//! A/B compare (WP P1AB, PRO1 W4): two snapshots as A and B, one toggle
//! between them.
//!
//! A compare holds two snapshot documents (P1SN's, `meta.kind =
//! "snapshot"`). [`DspiCore::ab_start`] takes them from the snapshot store,
//! [`DspiCore::ab_start_documents`] as text (the web, which has no store);
//! with no A the device's state is captured as A. A given A is recalled at
//! the start, so the device is on A when the compare begins.
//!
//! [`DspiCore::ab_toggle`] recalls the other side with the snapshot recall
//! (one `0xA1`, plus the settings outside the image that differ): it is
//! recorded in the event log as a recall and ends the undo history like any
//! bulk image (`docs/integrator/undo.md`); it is not an undo step. A toggle
//! is never guarded (no safe-apply timer, OD-P1g) and asks for no
//! confirmation token (P1TK): it must be instant.
//!
//! [`DspiCore::ab_diff`] is P1DF's document diff ([`crate::files::config_diff`])
//! from A to B. [`DspiCore::ab_keep`] ends the compare on the side named,
//! recalling it first when the device is on the other.
//!
//! A compare belongs to the connection it began on: a disconnect, a lost
//! device or a new connection ends it, and nothing is replayed; the next
//! call sees no compare. While a guarded apply (P1SA) waits for Keep or
//! Revert, [`DspiCore::ab_start`], [`DspiCore::ab_toggle`] and
//! [`DspiCore::ab_keep`] are refused ("finish the safe apply first"); a
//! guard that ends other than kept (reverted, failed, lost) ends the compare
//! (WP P1CFIX, F4). Edits made while on one side are kept: a toggle
//! first captures the side it leaves (reads only, no event), so tweaking B,
//! flipping to A and back finds B with the tweak. [`DspiCore::ab_diff`]
//! compares the sides as last left.
//!
//! -- WP UB34 -- (UXP B3) [`DspiCore::ab_start_saved`] compares "as saved on
//! the device" (A: the active preset as last loaded or saved, the baseline
//! of [`DspiCore::preset_changes`]) with "now" (B), so a top-bar A | B works
//! from the first edit with nothing captured by hand. Only what a preset
//! holds differs between its sides: the listening volume and mute, the
//! external mute, the master volume outside "with preset" mode, the output
//! configuration outside "with preset" mode and the settings outside the
//! image stay as they are live on every toggle (a toggle never jumps the
//! volume). It ends when its baseline is replaced (a load, a save, a revert,
//! a factory reset), except that a save or revert of the same preset while
//! on A keeps it, so the edits on B are not lost by a save on A.

use std::sync::Mutex;

use serde_json::Value;

use crate::records::DspState;
use crate::snapshot::SnapshotRecall;
use crate::{DspiCore, DspiError, Result};

/// The name a captured A is given, and its note.
const CAPTURED_A_NAME: &str = "A";
const CAPTURED_A_NOTE: &str = "Captured for an A/B compare";
/// The note of a side re-captured as a toggle leaves it.
const CAPTURED_NOTE: &str = "Captured by an A/B toggle";
// -- WP UB34 -- the sides of a compare of "as saved" against "now"
const SAVED_A_NAME: &str = "As saved";
const SAVED_A_NOTE: &str = "The active preset as saved on the device";
const SAVED_B_NAME: &str = "Now";
const SAVED_B_NOTE: &str = "The device's settings when the compare began";
/// The refusal of [`DspiCore::ab_start_saved`] with no saved state known.
pub(crate) const NO_SAVED: &str = "the settings saved on the device are not known yet";

/// One side of a compare.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AbSide {
    /// The first snapshot (the captured state when none was given).
    A,
    /// The second snapshot.
    B,
}

impl AbSide {
    /// The other side.
    pub fn other(self) -> AbSide {
        match self {
            AbSide::A => AbSide::B,
            AbSide::B => AbSide::A,
        }
    }

    /// `"a"` or `"b"`, as the C ABI and the web name it.
    pub fn key(self) -> &'static str {
        match self {
            AbSide::A => "a",
            AbSide::B => "b",
        }
    }

    /// The side with this key (`"a"`, `"b"`, any case).
    pub fn from_key(key: &str) -> Option<AbSide> {
        match key.trim().to_ascii_lowercase().as_str() {
            "a" => Some(AbSide::A),
            "b" => Some(AbSide::B),
            _ => None,
        }
    }
}

/// Where a compare stands ([`DspiCore::ab_state`] and every A/B call).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbCompare {
    /// Whether a compare is running.
    pub active: bool,
    /// The side on the device: while a compare runs, the one last recalled;
    /// after [`DspiCore::ab_keep`], the one kept. `None` with no compare.
    pub side: Option<AbSide>,
    /// A's snapshot name ("" with no compare).
    pub a_name: String,
    /// B's snapshot name ("" with no compare).
    pub b_name: String,
    /// What this call recalled, if it recalled anything.
    pub recall: Option<SnapshotRecall>,
}

// -- WP UB34 --
/// What a compare of "as saved" against "now" was made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SavedMark {
    /// The baseline's epoch (`Inner::baseline_epoch`).
    epoch: u64,
    /// The active preset slot it was the baseline of.
    slot: Option<u8>,
}

impl AbCompare {
    fn none() -> Self {
        Self {
            active: false,
            side: None,
            a_name: String::new(),
            b_name: String::new(),
            recall: None,
        }
    }
}

/// A running compare.
#[derive(Debug, Clone)]
struct Compare {
    /// Told apart from a compare started since (a toggle drops the lock
    /// while it recalls).
    epoch: u64,
    /// The connection it began on.
    generation: u64,
    a: String,
    b: String,
    a_name: String,
    b_name: String,
    side: AbSide,
    // -- WP UB34 -- Some for a compare of "as saved" against "now"
    saved: Option<SavedMark>,
}

impl Compare {
    fn text(&self, side: AbSide) -> &str {
        match side {
            AbSide::A => &self.a,
            AbSide::B => &self.b,
        }
    }

    fn text_mut(&mut self, side: AbSide) -> &mut String {
        match side {
            AbSide::A => &mut self.a,
            AbSide::B => &mut self.b,
        }
    }

    fn name(&self, side: AbSide) -> &str {
        match side {
            AbSide::A => &self.a_name,
            AbSide::B => &self.b_name,
        }
    }

    fn state(&self, recall: Option<SnapshotRecall>) -> AbCompare {
        AbCompare {
            active: true,
            side: Some(self.side),
            a_name: self.a_name.clone(),
            b_name: self.b_name.clone(),
            recall,
        }
    }
}

/// The running compare, if any. Never held across a device call (a
/// listener the recall wakes may ask for the state).
#[derive(Debug, Default)]
pub(crate) struct AbSlot {
    compare: Mutex<Option<Compare>>,
    epochs: Mutex<u64>,
}

impl AbSlot {
    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Compare>> {
        self.compare.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn next_epoch(&self) -> u64 {
        let mut e = self.epochs.lock().unwrap_or_else(|p| p.into_inner());
        *e = e.wrapping_add(1);
        *e
    }
}

fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

/// The refusal of an A/B call with no compare running.
pub(crate) const NO_COMPARE: &str = "no A/B compare is running";

// -- WP P1CFIX -- F4
/// The refusal of an A/B call while a guarded apply waits for Keep or Revert
/// (P1SA): a toggle then would capture the guarded change into a side, and
/// bring it back after the guard reverted it.
pub(crate) const GUARD_WAITING: &str = "finish the safe apply first: Keep or Revert";

/// What a snapshot document says about itself, checked before anything is
/// sent: `(name, platform, inputs, outputs)`.
fn described(text: &str, side: AbSide) -> Result<(String, String, Option<u64>, Option<u64>)> {
    let label = side.key().to_ascii_uppercase();
    let doc = crate::files::parse_config(text).map_err(|e| invalid(format!("{label}: {e}")))?;
    let kind = crate::files::doc_kind(&doc);
    if kind != dspi_session::preset_file::MetaKind::Snapshot
        || !doc.get("snapshot").is_some_and(Value::is_object)
    {
        return Err(invalid(format!(
            "{label}: {} (a {} document)",
            crate::snapshot::NOT_A_SNAPSHOT,
            kind.word()
        )));
    }
    let meta = doc.get("meta");
    let s = |k: &str| {
        meta.and_then(|m| m.get(k))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let n = |k: &str| meta.and_then(|m| m.get(k)).and_then(Value::as_u64);
    Ok((
        s("name"),
        s("platform"),
        n("inputChannelCount"),
        n("outputChannelCount"),
    ))
}

impl DspiCore {
    // -- WP P1CFIX -- F4
    /// Refuse an A/B call while a guarded apply waits (nothing sent).
    fn ab_guard_free(&self) -> Result<()> {
        if self.apply_status().phase == crate::GuardPhase::Waiting {
            return Err(invalid(GUARD_WAITING));
        }
        Ok(())
    }

    // -- WP P1CFIX -- F4
    /// End a running compare with nothing sent: a guarded apply's revert (or
    /// its lost connection) has put the device somewhere neither side was
    /// captured from.
    pub(crate) fn ab_end(&self) {
        *self.ab.slot() = None;
    }

    /// The open connection's generation.
    fn ab_generation(&self) -> Result<u64> {
        self.lock()
            .conn
            .as_ref()
            .map(|c| c.generation)
            .ok_or(DspiError::Disconnected)
    }

    /// The running compare, ended first when its connection is gone.
    fn ab_current(&self) -> Option<Compare> {
        let (generation, base) = {
            let inner = self.lock();
            let generation = inner.conn.as_ref().map(|c| c.generation);
            (generation, saved_mark(&inner))
        };
        let mut slot = self.ab.slot();
        if slot
            .as_ref()
            .is_some_and(|c| Some(c.generation) != generation)
        {
            *slot = None;
        }
        // -- WP UB34 -- "as saved" is no more what the device has saved
        if let Some(c) = slot.as_mut()
            && let Some(mark) = c.saved
            && mark.epoch != base.epoch
        {
            if c.side == AbSide::A && mark.slot == base.slot {
                // A save or revert of this preset while on A: A is still
                // what is saved, and B keeps its edits.
                c.saved = Some(base);
            } else {
                *slot = None;
            }
        }
        slot.clone()
    }

    /// Recall `side` of `c`; on success the compare (if still the same one)
    /// is on `side`. A lost connection ends the compare.
    fn ab_recall(&self, c: &Compare, side: AbSide) -> Result<SnapshotRecall> {
        // -- WP UB34 -- a side of "as saved" against "now" is laid onto the
        // live settings first, so only what a preset holds changes
        let text = if c.saved.is_some() {
            self.ab_on_live(c.text(side), c.name(side))
                .map_err(|e| self.ab_failed(c, e))?
        } else {
            c.text(side).to_owned()
        };
        match self.snapshot_recall_document(text) {
            Ok(r) => {
                if let Some(now) = self.ab.slot().as_mut().filter(|n| n.epoch == c.epoch) {
                    now.side = side;
                }
                Ok(r)
            }
            Err(e) => Err(self.ab_failed(c, e)),
        }
    }

    /// `e` from a call for compare `c`: a lost connection ends `c`.
    fn ab_failed(&self, c: &Compare, e: DspiError) -> DspiError {
        let gone = e == DspiError::Disconnected || self.ab_generation().ok() != Some(c.generation);
        if gone {
            let mut slot = self.ab.slot();
            if slot.as_ref().is_some_and(|n| n.epoch == c.epoch) {
                *slot = None;
            }
        }
        e
    }

    /// Capture the device as the side `c` is on, so edits made there are
    /// kept (P1AB round 2): reads only, no event, no write.
    ///
    /// Under a limits profile (WP P1CFIX, F3) the side keeps its stored value
    /// for every value its recall had to hold and that has not been edited
    /// since: only unheld values are re-captured, so the compare still shows
    /// the value above the limit and every recall of the side reports the
    /// hold again.
    fn ab_recapture(&self, c: &mut Compare) -> Result<()> {
        let side = c.side;
        let name = c.name(side).to_owned();
        let held = self.ab_held(c.text(side));
        let text = self
            .snapshot_capture_document_adjusted(name, CAPTURED_NOTE.into(), &|now| {
                if let Some((stored, held)) = &held {
                    crate::limits::keep_held(stored, held, now);
                }
            })
            .map_err(|e| self.ab_failed(c, e))?;
        *c.text_mut(side) = text.clone();
        if let Some(now) = self.ab.slot().as_mut().filter(|n| n.epoch == c.epoch) {
            *now.text_mut(side) = text;
        }
        Ok(())
    }

    // -- WP P1CFIX -- F3
    /// A side's stored settings and what its recall held them to, when the
    /// open device's profile holds any of them; `None` otherwise (no profile,
    /// a document that cannot be read). No I/O.
    fn ab_held(&self, text: &str) -> Option<(DspState, DspState)> {
        let doc = crate::files::parse_config(text).ok()?;
        let mut inner = self.lock();
        let stored = crate::snapshot::state_of_document(&inner.state, &doc)?;
        let held = crate::limits::held_side(&mut inner, &stored)?;
        Some((stored, held))
    }

    /// Start a compare of `a` and `b` (documents; `a` `None`: capture it).
    fn ab_begin(
        &self,
        a: Option<String>,
        b: String,
        saved: Option<SavedMark>,
    ) -> Result<AbCompare> {
        self.ab_guard_free()?; // -- WP P1CFIX -- F4
        let generation = self.ab_generation()?;
        // A compare already running is replaced, whatever comes of this one.
        *self.ab.slot() = None;
        let (b_name, b_platform, b_ins, b_outs) = described(&b, AbSide::B)?;
        let captured = a.is_none();
        let a = match a {
            Some(a) => a,
            None => {
                // -- WP P1EFIX2 -- (review p1efix F3) A is the device now,
                // what was queued before the compare included
                self.settle_notifications();
                self.snapshot_capture_document(CAPTURED_A_NAME.into(), CAPTURED_A_NOTE.into())?
            }
        };
        let (a_name, a_platform, a_ins, a_outs) = described(&a, AbSide::A)?;
        // Both of one device's shape, or a toggle would be refused half-way.
        let platform_differs = !a_platform.is_empty()
            && !b_platform.is_empty()
            && !a_platform.eq_ignore_ascii_case(&b_platform);
        let differ = |x: Option<u64>, y: Option<u64>| x.zip(y).is_some_and(|(x, y)| x != y);
        if platform_differs || differ(a_ins, b_ins) || differ(a_outs, b_outs) {
            return Err(invalid(
                "A and B were taken on devices of another platform or channel count, so nothing was sent",
            ));
        }
        let c = Compare {
            epoch: self.ab.next_epoch(),
            generation,
            a,
            b,
            a_name,
            b_name,
            side: AbSide::A,
            saved,
        };
        // A given A is recalled first, so the device is on A. A refusal
        // leaves no compare.
        let recall = if captured {
            None
        } else {
            Some(self.snapshot_recall_document(c.a.clone())?)
        };
        if self.ab_generation().ok() != Some(generation) {
            return Err(DspiError::Disconnected);
        }
        let state = c.state(recall);
        *self.ab.slot() = Some(c);
        Ok(state)
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Start an A/B compare of two snapshots in the store (WP P1AB, PRO1
    /// W4). `a_id` "" captures the device's state as A (named "A", not
    /// filed); otherwise snapshot `a_id` is recalled first, so the device is
    /// on A. `b_id` is B. A compare already running is replaced.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an unknown id, a document
    /// that is not a snapshot, or A and B of different platforms or channel
    /// counts (nothing sent); [`DspiError::Disconnected`]; the capture's and
    /// the recall's errors; [`DspiError::Io`] for an unreadable store or no
    /// folder named; [`DspiError::Unsupported`] on the web (use
    /// [`DspiCore::ab_start_documents`]).
    pub fn ab_start(&self, a_id: String, b_id: String) -> Result<AbCompare> {
        self.client_write_check()?; // -- WP ROMODE --
        self.ab_guard_free()?; // -- WP P1CFIX -- F4
        let b = self.snapshot_export(b_id)?;
        let a = if a_id.is_empty() {
            None
        } else {
            Some(self.snapshot_export(a_id)?)
        };
        self.ab_begin(a, b, None)
    }

    /// [`DspiCore::ab_start`] with snapshot documents' text (the web, and a
    /// client's own store): `a_text` "" captures the device as A.
    pub fn ab_start_documents(&self, a_text: String, b_text: String) -> Result<AbCompare> {
        self.client_write_check()?; // -- WP ROMODE --
        let a = (!a_text.is_empty()).then_some(a_text);
        self.ab_begin(a, b_text, None)
    }

    /// Recall the other side: one bulk write (`0xA1`) plus the settings
    /// outside the image that differ, recorded in the event log as a recall.
    /// The side left is captured first (reads only), so edits made on it are
    /// kept and come back on the next toggle, as in a DAW's A/B.
    /// It ends the undo history; it is not an undo step. Never guarded and
    /// never token-gated. Returns the compare, on the new side, with the
    /// recall's changes.
    ///
    /// Errors: [`DspiError::InvalidArgument`] with no compare running; the
    /// recall's errors. A lost connection ends the compare
    /// ([`DspiError::Disconnected`]); another refusal leaves it on its side.
    pub fn ab_toggle(&self) -> Result<AbCompare> {
        self.client_write_check()?; // -- WP ROMODE --
        self.ab_guard_free()?; // -- WP P1CFIX -- F4
        let mut c = self.ab_current().ok_or_else(|| invalid(NO_COMPARE))?;
        let to = c.side.other();
        // -- WP P1AB round 2 -- the side left keeps its edits
        self.ab_recapture(&mut c)?;
        let recall = self.ab_recall(&c, to)?;
        let mut now = c;
        now.side = to;
        Ok(now.state(Some(recall)))
    }

    /// What differs from A to B: [`crate::files::config_diff`]'s JSON for
    /// the pair (P1DF's rows, blocks and summary). No I/O.
    ///
    /// Errors: [`DspiError::InvalidArgument`] with no compare running.
    pub fn ab_diff(&self) -> Result<String> {
        let c = self.ab_current().ok_or_else(|| invalid(NO_COMPARE))?;
        Ok(crate::files::config_diff(c.a, c.b))
    }

    /// End the compare on `side`. Keeping the live side keeps the device's
    /// state as it is, edits included (nothing sent); the other side is
    /// recalled as it was left (a recall, as a toggle is). Returns `active: false`,
    /// `side` the one kept, and the recall if one was made.
    ///
    /// Errors: [`DspiError::InvalidArgument`] with no compare running; the
    /// recall's errors (the compare then runs on, unless the connection is
    /// gone).
    pub fn ab_keep(&self, side: AbSide) -> Result<AbCompare> {
        self.ab_guard_free()?; // -- WP P1CFIX -- F4
        let c = self.ab_current().ok_or_else(|| invalid(NO_COMPARE))?;
        let recall = if c.side == side {
            None
        } else {
            Some(self.ab_recall(&c, side)?)
        };
        {
            let mut slot = self.ab.slot();
            if slot.as_ref().is_some_and(|n| n.epoch == c.epoch) {
                *slot = None;
            }
        }
        Ok(AbCompare {
            active: false,
            side: Some(side),
            a_name: c.a_name,
            b_name: c.b_name,
            recall,
        })
    }

    /// The running compare, or `active: false` with none (a disconnect
    /// ends one). Never fails; no I/O.
    pub fn ab_state(&self) -> AbCompare {
        self.ab_current()
            .map_or_else(AbCompare::none, |c| c.state(None))
    }
}

// -- WP UB34 -- (UXP B3) "as saved" against "now"

/// The baseline a compare of "as saved" against "now" stands on now.
fn saved_mark(inner: &crate::Inner) -> SavedMark {
    SavedMark {
        epoch: inner.baseline_epoch,
        slot: inner.baseline_of.as_ref().map(|(_, slot)| *slot),
    }
}

/// `preset`'s settings where a preset holds them, `live`'s everywhere else:
/// the listening volume and mute, the external mute, the sub solo, the
/// persistence modes, the master volume unless it travels with presets and
/// the output configuration unless it does (what a preset load leaves alone,
/// and what [`DspiCore::preset_changes`] does not count).
pub(crate) fn preset_on_live(preset: &DspState, live: &DspState) -> DspState {
    use crate::presets::WITH_PRESET;
    let mut s = preset.clone();
    s.user_volume_db = live.user_volume_db;
    s.user_mute = live.user_mute;
    s.dac_mute_enabled = live.dac_mute_enabled;
    s.dac_mute_active_low = live.dac_mute_active_low;
    s.dac_mute_pin = live.dac_mute_pin;
    s.dac_mute_hold_ms = live.dac_mute_hold_ms;
    s.dac_mute_release_ms = live.dac_mute_release_ms;
    s.subharm_solo = live.subharm_solo;
    s.master_volume_mode = live.master_volume_mode;
    s.output_config_mode = live.output_config_mode;
    if live.master_volume_mode != WITH_PRESET {
        s.master_volume_db = live.master_volume_db;
    }
    if live.output_config_mode != WITH_PRESET {
        s.output_pins.clone_from(&live.output_pins);
        s.output_types.clone_from(&live.output_types);
        s.i2s_bck_pin = live.i2s_bck_pin;
        s.i2s_bck_pin_slave = live.i2s_bck_pin_slave;
        s.i2s_clock_pin_mode = live.i2s_clock_pin_mode;
        s.mck_enabled = live.mck_enabled;
        s.mck_pin = live.mck_pin;
        s.mck_multiplier = live.mck_multiplier;
        s.adat_out_enabled = live.adat_out_enabled;
        s.adat_out_pin = live.adat_out_pin;
        s.limiter_enabled.clone_from(&live.limiter_enabled);
        s.limiter_link_group.clone_from(&live.limiter_link_group);
        s.limiter_threshold_db
            .clone_from(&live.limiter_threshold_db);
        s.limiter_release_ms.clone_from(&live.limiter_release_ms);
        // The input block but the source itself
        s.spdif_inputs_enabled = live.spdif_inputs_enabled;
        s.spdif_rx_pins.clone_from(&live.spdif_rx_pins);
        s.i2s_rx_pins.clone_from(&live.i2s_rx_pins);
        s.i2s_input_channels = live.i2s_input_channels;
        s.i2s_input_rate = live.i2s_input_rate;
        s.i2s_clock_mode = live.i2s_clock_mode;
        s.adat_input_enabled = live.adat_input_enabled;
        s.adat_input_pin = live.adat_input_pin;
        s.adat_input_clock_mode = live.adat_input_clock_mode;
    }
    s
}

impl DspiCore {
    /// Snapshot document `text`'s preset laid onto the live settings
    /// ([`preset_on_live`]), as a snapshot document named `name`. Reads only.
    fn ab_on_live(&self, text: &str, name: &str) -> Result<String> {
        let doc = crate::files::parse_config(text).map_err(invalid)?;
        let base = self.lock().state.clone();
        let preset = crate::snapshot::state_of_document(&base, &doc)
            .ok_or_else(|| invalid(format!("{name}: the snapshot's settings cannot be read")))?;
        self.snapshot_capture_document_adjusted(name.to_owned(), CAPTURED_NOTE.into(), &|live| {
            *live = preset_on_live(&preset, live);
        })
    }
}

// Not exported to the bindings: the desktop app (C ABI) and the web (wasm)
// call it; Android has no top-bar A/B.
impl DspiCore {
    /// Start an A/B compare of the active preset as saved on the device (A,
    /// the baseline [`DspiCore::preset_changes`] compares with) against the
    /// device now (B), and recall A: the top bar's A | B from the first edit
    /// (UXP B3, owner D7). Only what a preset holds differs between the
    /// sides (see [`preset_on_live`]), on every toggle too. It ends by
    /// itself when its baseline is replaced, but a save or revert of the
    /// same preset while on A keeps it. A compare already running is
    /// replaced.
    ///
    /// Errors: [`DspiError::InvalidArgument`] ([`NO_SAVED`]) when the core
    /// has no baseline (no image in this build's format), or while a
    /// guarded apply waits; [`DspiError::Disconnected`]; the capture's and
    /// the recall's errors.
    pub fn ab_start_saved(&self) -> Result<AbCompare> {
        self.client_write_check()?; // -- WP ROMODE --
        self.ab_guard_free()?;
        // What was queued before the compare is part of "now".
        self.settle_notifications();
        let (saved, mark) = {
            let mut inner = self.lock();
            inner.conn()?;
            let base = inner.baseline.as_deref().ok_or_else(|| invalid(NO_SAVED))?;
            let mut saved = inner.state.clone();
            crate::state::decode_bulk(base, &mut saved).map_err(|_| invalid(NO_SAVED))?;
            (saved, saved_mark(&inner))
        };
        let b = self.snapshot_capture_document(SAVED_B_NAME.into(), SAVED_B_NOTE.into())?;
        let a = self.snapshot_capture_document_adjusted(
            SAVED_A_NAME.into(),
            SAVED_A_NOTE.into(),
            &|live| *live = preset_on_live(&saved, live),
        )?;
        self.ab_begin(Some(a), b, Some(mark))
    }

    /// Whether the running compare is one of "as saved" against "now"
    /// ([`DspiCore::ab_start_saved`]). No I/O.
    pub fn ab_is_saved(&self) -> bool {
        self.ab_current().is_some_and(|c| c.saved.is_some())
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p1ab;

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_ub34;
