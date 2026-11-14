//! AutoEQ preview and apply to any channel pair (WP P1AQ, PRO1 T6).
//!
//! [`DspiCore::autoeq_preview`] works out what a profile would do to a pair
//! of channels, from the mirror alone: the PEQ bank each channel would get,
//! its level, its crossover, the response curve, and what the limits profile
//! would hold. Nothing reaches the device.
//!
//! [`DspiCore::autoeq_apply`] writes the same thing, after capturing the
//! device's state as a snapshot (P1SN), so the state before is one recall
//! away (an A for an A/B compare). The writes are one undo group (P1UG)
//! labelled with the profile's name, and they are ordinary writes, so they
//! go through the limits profile (P1LM), the lock and every other check a
//! setter makes. A value the profile would take past a limit (a boost above
//! the profile's ceiling, a level above an output's maximum, a high-pass the
//! output must keep) is held to it before anything is sent and reported in
//! the import's words ("Held to the limits profile: eq.gain[4,0] 9 → 6 dB").
//!
//! Any pair works: two inputs or two outputs, by wire channel (inputs first,
//! then outputs). On inputs the profile's preamp is the inputs' preamp
//! (per input, as the Mac sets it); on outputs it is the outputs' gain,
//! the only level an output has. Every band of the bank is written: the
//! profile's bands first, every other band flat, so nothing stale stays
//! underneath the correction.

use dspi_proto::FilterType;
use dspi_session::autoeq::{self as session, Entry, Filter};

use crate::records::{DspState, FilterParams};
use crate::{DspiCore, DspiError, Inner, Result, compute_magnitude_curve, quantize_gain};

/// The undo step's label, before the profile's name.
pub const AUTOEQ_GROUP_LABEL: &str = "AutoEQ";

/// Where the profile's preamp goes on inputs ([`AutoEqPreview::level_target`]).
pub const AUTOEQ_LEVEL_INPUT_PREAMP: &str = "input_preamp";
/// Where the profile's preamp goes on outputs ([`AutoEqPreview::level_target`]).
pub const AUTOEQ_LEVEL_OUTPUT_GAIN: &str = "output_gain";

/// The lowest preamp and output gain the setters allow.
const LEVEL_MIN_DB: f32 = -60.0;
/// The highest output gain (`out.gain`).
const OUTPUT_GAIN_MAX_DB: f32 = 20.0;
/// What a cleared band is.
const FLAT: FilterParams = FilterParams {
    filter_type: 0,
    bypass: false,
    freq: 1000.0,
    q: 0.707,
    gain: 0.0,
    qp: 0.707,
};

/// One filter of a profile, as the AutoEQ database has it.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoEqFilter {
    /// The database's shape name (`PK`, `LSC`, `HSC`, `peaking`, …).
    pub kind: String,
    /// Centre or corner frequency in Hz.
    pub freq: f32,
    /// Quality factor.
    pub q: f32,
    /// Gain in dB.
    pub gain_db: f32,
}

/// A headphone profile to preview or apply: one entry of the AutoEQ
/// database. A client that holds the database as JSON passes an entry
/// through [`AutoEqProfile::from_entry_json`] (the C ABI and the web do).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoEqProfile {
    /// The database's id.
    pub id: String,
    /// The name to show, used in the undo step's label and the snapshot's.
    pub name: String,
    /// The trim the profile expects, almost always negative.
    pub preamp_db: f32,
    /// The filters, in the database's order: filter `i` goes to band `i`.
    pub filters: Vec<AutoEqFilter>,
}

impl AutoEqProfile {
    /// The profile of a database entry.
    pub fn from_entry(e: &Entry) -> Self {
        Self {
            id: e.id.clone(),
            name: e.display_name(),
            preamp_db: e.preamp,
            filters: e
                .filters
                .iter()
                .map(|f| AutoEqFilter {
                    kind: f.kind.clone(),
                    freq: f.freq,
                    q: f.q,
                    gain_db: f.gain,
                })
                .collect(),
        }
    }

    /// The profile of one database entry's JSON (`{"id", "manufacturer",
    /// "model", "preamp", "filters": [{"type", "freq", "q", "gain"}]}`).
    ///
    /// Errors: [`DspiError::InvalidArgument`] for text that is not one.
    pub fn from_entry_json(text: &str) -> Result<Self> {
        serde_json::from_str::<Entry>(text)
            .map(|e| Self::from_entry(&e))
            .map_err(|e| DspiError::InvalidArgument {
                msg: format!("not an AutoEQ profile: {e}"),
            })
    }

    fn entry(&self) -> Entry {
        Entry {
            id: self.id.clone(),
            manufacturer: self.name.clone(),
            model: String::new(),
            source: String::new(),
            form_factor: String::new(),
            preamp: self.preamp_db,
            filters: self
                .filters
                .iter()
                .map(|f| Filter {
                    kind: f.kind.clone(),
                    freq: f.freq,
                    q: f.q,
                    gain: f.gain_db,
                })
                .collect(),
        }
    }

    fn label(&self) -> String {
        let name = if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        };
        if name.is_empty() {
            AUTOEQ_GROUP_LABEL.to_owned()
        } else {
            format!("{AUTOEQ_GROUP_LABEL}: {name}")
        }
    }
}

/// One channel of a pair as a profile leaves it.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoEqChannel {
    /// The wire channel (inputs first, then outputs).
    pub channel: u8,
    /// Whether it is an output.
    pub is_output: bool,
    /// Its level after the apply: the input's preamp, or the output's gain.
    pub level_db: f32,
    /// Its PEQ bank after the apply, every band.
    pub bands: Vec<FilterParams>,
    /// Its crossover after the apply (changed only where the limits profile
    /// asks for a high-pass); empty on an input.
    pub crossover: Vec<FilterParams>,
    /// Its response after the apply, PEQ and crossover, in dB at each
    /// [`crate::curve_freq`] point: the graph's preview curve.
    pub curve: Vec<f64>,
}

/// What a profile would do to a channel pair ([`DspiCore::autoeq_preview`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoEqPreview {
    /// The profile's id.
    pub profile_id: String,
    /// The two channels, in the order given.
    pub channels: Vec<AutoEqChannel>,
    /// Where the profile's preamp goes: `input_preamp` or `output_gain`.
    pub level_target: String,
    /// What the limits profile held, one row each ("eq.gain[4,0] 9 → 6
    /// dB"); empty with no profile or nothing past a limit.
    pub held: Vec<String>,
    /// Filter shapes of the profile this build does not know, left flat.
    pub unsupported: Vec<String>,
    /// Filters past the end of the bank, left out.
    pub dropped: u32,
    /// The lines to show: what was held, dropped and left out, in words.
    pub notes: Vec<String>,
}

/// What [`DspiCore::autoeq_apply`] did.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoEqApply {
    /// What was written: the preview, as applied.
    pub preview: AutoEqPreview,
    /// The snapshot of the state before, as a document
    /// (`meta.kind = "snapshot"`): [`DspiCore::snapshot_recall_document`]
    /// puts it back; it is the A of an A/B compare
    /// ([`DspiCore::ab_start_documents`]).
    pub snapshot_document: String,
    /// The same snapshot's id in the store, "" when no snapshot folder is
    /// named (the web) or it could not be filed (then a line in `notes`
    /// says why).
    pub snapshot_id: String,
    /// What to tell the person beyond the preview's notes.
    pub notes: Vec<String>,
}

/// The pair `(left, right)` checked against `s`: both channels there,
/// distinct, both inputs or both outputs. Whether they are outputs.
fn check_pair(s: &DspState, left: u8, right: u8) -> Result<bool> {
    let invalid = |msg: String| DspiError::InvalidArgument { msg };
    for ch in [left, right] {
        if ch >= s.num_channels || s.filters.get(usize::from(ch)).is_none() {
            return Err(invalid(format!("channel {ch} (0..{})", s.num_channels)));
        }
    }
    if left == right {
        return Err(invalid(format!(
            "a channel pair is two channels, not {left} twice"
        )));
    }
    let out = |ch: u8| ch >= s.num_input_channels;
    if out(left) != out(right) {
        return Err(invalid(format!(
            "channels {left} and {right} are an input and an output; a pair is two inputs or two outputs"
        )));
    }
    Ok(out(left))
}

/// The profile's numbers checked before anything is planned, so an apply
/// never stops half-way on a value the setters would refuse.
fn check_profile(p: &AutoEqProfile) -> Result<()> {
    let invalid = |msg: String| DspiError::InvalidArgument { msg };
    if !p.preamp_db.is_finite() {
        return Err(invalid(format!("the profile's preamp {}", p.preamp_db)));
    }
    for (i, f) in p.filters.iter().enumerate() {
        if session::filter_type(&f.kind).is_none() {
            continue;
        }
        let good = f.freq.is_finite() && f.freq > 0.0 && f.q.is_finite() && f.q > 0.0;
        if !good || !f.gain_db.is_finite() {
            return Err(invalid(format!(
                "the profile's filter {}: {} Hz, Q {}, {} dB",
                i.saturating_add(1),
                f.freq,
                f.q,
                f.gain_db
            )));
        }
    }
    Ok(())
}

/// Remove from `rows` one of each row in `base`: what remains is what the
/// profile's values brought, not what the device already had.
fn without(mut rows: Vec<String>, base: &[String]) -> Vec<String> {
    for b in base {
        if let Some(i) = rows.iter().position(|r| r == b) {
            rows.remove(i);
        }
    }
    rows
}

/// The plan: the preview, and the state the writes are taken from.
struct Plan {
    preview: AutoEqPreview,
    /// The mirror as the apply leaves it, for the pair's channels.
    target: DspState,
    /// The held rows, for the limits status.
    held: Vec<String>,
    /// Whether the pair are outputs.
    outputs: bool,
}

fn plan(inner: &mut Inner, profile: &AutoEqProfile, left: u8, right: u8) -> Result<Plan> {
    inner.conn()?;
    check_profile(profile)?;
    let s = inner.state.clone();
    let outputs = check_pair(&s, left, right)?;
    let entry = profile.entry();
    let pair = [left, right];

    let mut target = s.clone();
    let mut unsupported = Vec::new();
    let mut dropped = 0usize;
    let platform_max = if s.platform_id == 1 {
        crate::device::PREAMP_MAX_DB_RP2350
    } else {
        crate::device::PREAMP_MAX_DB_RP2040
    };
    for ch in pair {
        let c = usize::from(ch);
        let n = s.filters.get(c).map_or(0, Vec::len);
        let max = u8::try_from(n).unwrap_or(u8::MAX);
        let planned = entry.plan(max);
        let mut bank = vec![FLAT; n];
        for b in &planned.bands {
            if let Some(slot) = bank.get_mut(usize::from(b.band)) {
                let filter_type = b.filter_type.to_raw();
                *slot = FilterParams {
                    filter_type,
                    bypass: false,
                    freq: b.freq,
                    q: b.q,
                    gain: if FilterType::from_raw(filter_type).is_linkwitz() {
                        b.gain_db
                    } else {
                        quantize_gain(b.gain_db)
                    },
                    qp: FLAT.qp,
                };
            }
        }
        if let Some(slot) = target.filters.get_mut(c) {
            *slot = bank;
        }
        for u in planned.unsupported {
            if !unsupported.contains(&u) {
                unsupported.push(u);
            }
        }
        dropped = dropped.max(planned.dropped);
        if outputs {
            let o = usize::from(ch.saturating_sub(s.num_input_channels));
            let v = quantize_gain(profile.preamp_db.clamp(LEVEL_MIN_DB, OUTPUT_GAIN_MAX_DB));
            if let Some(g) = target.output_gain_db.get_mut(o) {
                *g = v;
            }
        } else {
            let v = quantize_gain(profile.preamp_db.clamp(LEVEL_MIN_DB, platform_max));
            if let Some(g) = target.input_preamp_db.get_mut(c) {
                *g = v;
            }
            if ch == 0 {
                target.preamp_db = v;
            }
        }
    }

    // The limits profile: hold the target, keep what moved on the pair, and
    // report what the profile's values brought (not what the device was
    // already outside of before).
    let mut held = Vec::new();
    if let Some(p) = crate::limits::profile(inner) {
        let mut base = s.clone();
        let base_rows = crate::limits::hold_state(&p, &mut base);
        let mut all = target.clone();
        let rows = crate::limits::hold_state(&p, &mut all);
        held = without(rows, &base_rows);
        for ch in pair {
            let c = usize::from(ch);
            if let (Some(t), Some(h)) = (target.filters.get_mut(c), all.filters.get(c)) {
                t.clone_from(h);
            }
            if let (Some(t), Some(h)) = (target.xover.get_mut(c), all.xover.get(c)) {
                t.clone_from(h);
            }
            if outputs {
                let o = usize::from(ch.saturating_sub(s.num_input_channels));
                if let (Some(t), Some(h)) =
                    (target.output_gain_db.get_mut(o), all.output_gain_db.get(o))
                {
                    *t = *h;
                }
            }
        }
    }

    let channels = pair
        .iter()
        .map(|&ch| {
            let c = usize::from(ch);
            let bands = target.filters.get(c).cloned().unwrap_or_default();
            let crossover = if outputs {
                target.xover.get(c).cloned().unwrap_or_default()
            } else {
                Vec::new()
            };
            let level_db = if outputs {
                let o = usize::from(ch.saturating_sub(s.num_input_channels));
                target.output_gain_db.get(o).copied().unwrap_or_default()
            } else {
                target.input_preamp_db.get(c).copied().unwrap_or_default()
            };
            let curve = compute_magnitude_curve(bands.iter().chain(&crossover).copied().collect());
            AutoEqChannel {
                channel: ch,
                is_output: outputs,
                level_db,
                bands,
                crossover,
                curve,
            }
        })
        .collect();

    let mut notes = Vec::new();
    if let Some(line) = crate::limits::held_line(&held) {
        notes.push(line);
    }
    if dropped > 0 {
        notes.push(format!(
            "{dropped} of the profile's filters are past the end of the bank and are left out"
        ));
    }
    if !unsupported.is_empty() {
        notes.push(format!(
            "Filter shapes this build does not know are left flat: {}",
            unsupported.join(", ")
        ));
    }
    Ok(Plan {
        preview: AutoEqPreview {
            profile_id: profile.id.clone(),
            channels,
            level_target: if outputs {
                AUTOEQ_LEVEL_OUTPUT_GAIN
            } else {
                AUTOEQ_LEVEL_INPUT_PREAMP
            }
            .to_owned(),
            held: held.clone(),
            unsupported,
            dropped: u32::try_from(dropped).unwrap_or(u32::MAX),
            notes,
        },
        target,
        held,
        outputs,
    })
}

/// Whether a failed filing is only "no folder named" (the web, or a client
/// that keeps no store): then the document alone is the snapshot, no note.
fn no_store(e: &DspiError) -> bool {
    match e {
        DspiError::Unsupported => true,
        DspiError::Io { msg } => msg.contains("no folder has been named"),
        _ => false,
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// What applying `profile` to wire channels `left` and `right` (two
    /// inputs or two outputs) would do: each channel's PEQ bank, level,
    /// crossover and response curve, what the limits profile would hold, and
    /// the lines to show. Reads the mirror only: nothing is sent.
    ///
    /// Errors: [`DspiError::Disconnected`]; [`DspiError::InvalidArgument`]
    /// for a channel out of range, the same channel twice, an input with an
    /// output, or a profile value no setter would take (a non-finite preamp,
    /// a known filter with a frequency or Q not above 0).
    pub fn autoeq_preview(
        &self,
        profile: AutoEqProfile,
        left: u8,
        right: u8,
    ) -> Result<AutoEqPreview> {
        let mut inner = self.lock();
        plan(&mut inner, &profile, left, right).map(|p| p.preview)
    }

    /// Apply `profile` to wire channels `left` and `right` as
    /// [`DspiCore::autoeq_preview`] shows it. First the device's state is
    /// captured as a snapshot, returned as a document and filed in the
    /// snapshot store when a folder is named ([`DspiCore::snapshot_set_dir`]);
    /// then every write is one undo step labelled "AutoEQ: <name>": the
    /// level (inputs' preamp or outputs' gain), any crossover band the
    /// limits profile asks for, then every PEQ band of both channels.
    /// Values past the limits profile are held to it and reported
    /// ([`AutoEqPreview::held`]); nothing outside the pair changes.
    ///
    /// Errors: those of [`DspiCore::autoeq_preview`], then
    /// [`DspiError::ReadOnly`], all before anything is sent; those of the
    /// snapshot capture, before any write; then the errors of every setter
    /// ([`DspiError::LimitExceeded`] for a high-pass the limits profile
    /// cannot place, [`DspiError::Transfer`] and the rest). A failure part
    /// way leaves the writes made so far as the one undo step, and the
    /// snapshot filed (with a folder named).
    pub fn autoeq_apply(&self, profile: AutoEqProfile, left: u8, right: u8) -> Result<AutoEqApply> {
        // Every check before anything is sent.
        {
            let mut inner = self.writing()?;
            plan(&mut inner, &profile, left, right)?;
        }
        let name = format!("Before {}", profile.label());
        let note = format!("Captured before AutoEQ was applied to channels {left} and {right}");
        let snapshot_document = self.snapshot_capture_document(name.clone(), note.clone())?;
        let mut notes = Vec::new();
        let snapshot_id = match self.snapshot_import(snapshot_document.clone(), name, note) {
            Ok(info) => info.id,
            Err(e) if no_store(&e) => String::new(),
            Err(e) => {
                notes.push(format!("The snapshot before could not be filed: {e}"));
                String::new()
            }
        };

        // Planned again: the capture let go of the lock, and the plan is
        // taken from the mirror as it is now.
        let plan = {
            let mut inner = self.writing()?;
            plan(&mut inner, &profile, left, right)?
        };
        self.begin_group(profile.label());
        let written = self.write_plan(&plan, [left, right]);
        self.end_group();
        written?;
        crate::limits::note_held(&mut self.lock(), &plan.held);
        Ok(AutoEqApply {
            preview: plan.preview,
            snapshot_document,
            snapshot_id,
            notes,
        })
    }
}

impl DspiCore {
    /// The writes of `plan` on `pair`: level, crossover bands that change,
    /// then the PEQ banks. Lowering a level before raising a band keeps the
    /// headroom while the bands arrive.
    fn write_plan(&self, plan: &Plan, pair: [u8; 2]) -> Result<()> {
        let t = &plan.target;
        let now = self.get_state();
        for ch in pair {
            let c = usize::from(ch);
            if plan.outputs {
                let o = ch.saturating_sub(t.num_input_channels);
                if let Some(&g) = t.output_gain_db.get(usize::from(o)) {
                    self.set_output_gain(o, g)?;
                }
            } else if let Some(&g) = t.input_preamp_db.get(c) {
                self.set_input_preamp(ch, g)?;
            }
        }
        if plan.outputs {
            for ch in pair {
                let c = usize::from(ch);
                let want = t.xover.get(c).map(Vec::as_slice).unwrap_or_default();
                let have = now.xover.get(c).map(Vec::as_slice).unwrap_or_default();
                for (x, (w, h)) in (0u8..).zip(want.iter().zip(have)) {
                    if w != h {
                        self.set_crossover(ch, x, *w)?;
                    }
                }
            }
        }
        for ch in pair {
            let bank = t
                .filters
                .get(usize::from(ch))
                .map(Vec::as_slice)
                .unwrap_or_default();
            for (b, f) in (0u8..).zip(bank) {
                self.set_filter(ch, b, *f)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::arithmetic_side_effects)]
mod tests_wp_p1aq;
