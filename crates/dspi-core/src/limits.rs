//! Limits profiles in the core (WP P1LM, PRO1 S1).
//!
//! The profile itself, its document, the plain-value checks and the files
//! are `dspi_session::limits` (shared with the hub). This module holds the
//! open device's profile, judges every write against it, and holds what a
//! bulk write would send to it.
//!
//! - **No profile:** nothing changes, nothing is shown (owner, 2026-11-13).
//! - **A direct write** outside a limit is refused before anything is sent,
//!   with [`DspiError::LimitExceeded`] (notice key `limit_exceeded`): a typed
//!   setter, [`DspiCore::write_param`] and an undo or a redo alike, since
//!   they all reach the device through the setters' send paths
//!   (`params.rs`'s `send` and `send_band`, `params_fx.rs`'s `fx_write`).
//!   The check sits there and not in `journal_begin`, which never sees the
//!   value being written.
//! - **A bulk image** (an import, a snapshot recall, an A/B toggle, a
//!   template or a document applied) is held to the profile before it is
//!   sent, and what was held is reported in the import's words ("Held to
//!   the limits profile: out.gain[2] 6 → 3 dB, ...").
//! - **A profile applied** ([`DspiCore::limits_apply_profile`]) that narrows
//!   a limit the device is outside of clamps the device at once: ordinary
//!   writes, journalled (one undo group), reported. A **preset slot load**
//!   (the firmware loads values the host never held) is read back and held
//!   the same way (OD-P1j).
//! - **A corrupt or missing profile file** gives a standing notice; the last
//!   good copy is used when there is one (fails closed), otherwise the
//!   device runs unlimited (OD-P1k). Never a panic.
//! - **Reverts are never refused:** a revert of a guarded apply restores a
//!   state captured before, past any limit set since; it runs inside
//!   [`unlimited`].
//!
//! The UI says plainly that a limit the firmware does not know can be crossed
//! from a control surface, the UART or another console without the profile
//! (FW2 closes it later).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::cell::Cell;

use dspi_proto::generated::limiter;
use dspi_proto::value::Value;
use dspi_session::limits::{self as shared, BandView, LimitsProfile, LoadState, Violation};
use dspi_session::preset_file::OutOfRange;

use crate::records::{DspState, FilterParams};
use crate::{DspiCore, DspiError, Inner, Result};

/// The notice keys this module answers (`crate::notice_text`): a profile
/// file unreadable with a good copy in force, one with none, and the line a
/// Limits page shows on what a host-side limit cannot stop.
pub const NOTICE_KEYS: [&str; 3] = ["limits.fell_back", "limits.unreadable", "limits.scope"];

/// The undo group's label for the writes a profile or a slot load makes.
pub(crate) const HOLD_LABEL: &str = "Limits profile";

/// The first crossover band on the wire.
const XOVER_BASE: u8 = 20;

/// PEQ bands per channel.
const PEQ_BANDS: usize = 10;

thread_local! {
    /// Depth of [`unlimited`] on this thread.
    static UNLIMITED: Cell<u32> = const { Cell::new(0) };
}

/// Run `f` with no limit judged on this thread: a revert of a guarded apply
/// (P1SA) restores a state captured before, past any limit set since.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "P1SA's revert (guarded.rs, wave P1-C, built in parallel) is its caller"
    )
)]
pub(crate) fn unlimited<T>(f: impl FnOnce() -> T) -> T {
    struct Depth;
    impl Drop for Depth {
        fn drop(&mut self) {
            UNLIMITED.with(|d| d.set(d.get().saturating_sub(1)));
        }
    }
    UNLIMITED.with(|d| d.set(d.get().saturating_add(1)));
    let _depth = Depth;
    f()
}

fn suspended() -> bool {
    UNLIMITED.with(|d| d.get() > 0)
}

/// The open device's profile as the core holds it.
#[derive(Debug, Default)]
pub(crate) struct Limits {
    /// The folder the client named (none on the web).
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    dir: Option<std::path::PathBuf>,
    /// Profiles set with no folder named (the web), by serial, for the
    /// lifetime of this core.
    memory: Vec<LimitsProfile>,
    /// The profile loaded for a serial.
    current: Option<Current>,
    /// What the last clamp held, in the import's words.
    last_held: Vec<String>,
}

#[derive(Debug)]
struct Current {
    serial: String,
    profile: Option<LimitsProfile>,
    state: LoadState,
}

impl Limits {
    fn has_any(&self) -> bool {
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        if self.dir.is_some() {
            return true;
        }
        !self.memory.is_empty()
    }

    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    fn store(&self) -> Option<shared::LimitsStore> {
        self.dir.clone().map(shared::LimitsStore::new)
    }

    fn load(&self, serial: &str) -> Current {
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        if let Some(store) = self.store() {
            let l = store.load(serial);
            return Current {
                serial: serial.to_owned(),
                profile: l.profile.filter(worth),
                state: l.state,
            };
        }
        let profile = self.memory.iter().find(|p| p.serial == serial).cloned();
        Current {
            serial: serial.to_owned(),
            state: if profile.is_some() {
                LoadState::Ok
            } else {
                LoadState::None
            },
            profile,
        }
    }
}

/// A profile worth keeping: one that limits a value, or carries a note,
/// a contact or a locked page.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn worth(p: &LimitsProfile) -> bool {
    p.limits_values() || !p.note.is_empty() || !p.contact.is_empty() || !p.locked_pages.is_empty()
}

impl Inner {
    /// The open device's profile, loaded once per serial; `None` with no
    /// profile, no device, or inside [`unlimited`].
    pub(crate) fn limits_profile(&mut self) -> Option<&LimitsProfile> {
        if suspended() || !self.limits.has_any() {
            return None;
        }
        let serial = self.conn.as_ref()?.serial.clone();
        if self.limits.current.as_ref().map(|c| c.serial.as_str()) != Some(serial.as_str()) {
            self.limits.current = Some(self.limits.load(&serial));
        }
        self.limits.current.as_ref()?.profile.as_ref()
    }
}

fn refused(v: Violation) -> DspiError {
    DspiError::LimitExceeded {
        code: v.code.to_owned(),
        msg: v.msg,
    }
}

/// The bands of wire channel `ch` (PEQ, then crossover) as the rules see
/// them, with band `replace` set to `with` when given.
fn views(s: &DspState, ch: u8, replace: Option<(u8, FilterParams)>) -> Vec<BandView> {
    let c = usize::from(ch);
    let peq = s.filters.get(c).map(Vec::as_slice).unwrap_or_default();
    let xo = s.xover.get(c).map(Vec::as_slice).unwrap_or_default();
    let peq = peq.iter().zip(0u8..);
    let xo = xo.iter().zip(XOVER_BASE..);
    peq.chain(xo)
        .map(|(p, b)| match replace {
            Some((rb, with)) if rb == b => with,
            _ => *p,
        })
        .map(|p| BandView {
            filter_type: p.filter_type,
            bypass: p.bypass,
            freq: p.freq,
        })
        .collect()
}

/// The output whose wire channel is `ch`, if it is one.
fn output_of(s: &DspState, ch: u8) -> Option<u8> {
    ch.checked_sub(s.num_input_channels)
        .filter(|o| *o < s.num_output_channels)
}

/// Judge a write of `value` to `path` at `indices` (the setters' `send`).
pub(crate) fn check_write(
    inner: &mut Inner,
    path: &str,
    indices: &[u8],
    value: &Value,
) -> Result<()> {
    let Some(p) = inner.limits_profile() else {
        return Ok(());
    };
    let p = p.clone();
    let s = &inner.state;
    let i0 = indices.first().copied().unwrap_or(0);
    let num = match value {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f32),
        _ => None,
    };
    let r = match (path, num) {
        ("out.gain", Some(v)) => p.check_output_gain(i0, v),
        ("out.delay", Some(v)) => p.check_delay(i0, v),
        ("ch.delay", Some(v)) => match output_of(s, i0) {
            Some(o) => p.check_delay(o, v),
            None => Ok(()),
        },
        ("vol.master", Some(v)) => p.check_master_volume(v),
        ("eq.bypass", _) => {
            let (Some(&band), Value::Bool(bypass)) = (indices.get(1), value) else {
                return Ok(());
            };
            let Some(o) = output_of(s, i0) else {
                return Ok(());
            };
            let Some(mut b) = crate::write_param::band(s, i0, band) else {
                return Ok(());
            };
            b.bypass = *bypass;
            p.check_highpass(o, &views(s, i0, None), &views(s, i0, Some((band, b))))
        }
        _ => Ok(()),
    };
    r.map_err(refused)
}

/// Judge a whole band written to wire channel `ch`, band `band` (the
/// setters' `send_band`): the boost and the output's high-pass.
pub(crate) fn check_band(inner: &mut Inner, ch: u8, band: u8, params: &FilterParams) -> Result<()> {
    let Some(p) = inner.limits_profile() else {
        return Ok(());
    };
    let p = p.clone();
    let s = &inner.state;
    p.check_boost(params.filter_type, params.gain)
        .map_err(refused)?;
    if let Some(o) = output_of(s, ch) {
        p.check_highpass(o, &views(s, ch, None), &views(s, ch, Some((band, *params))))
            .map_err(refused)?;
    }
    Ok(())
}

/// Judge the limiters a write leaves (`next`, the mirror after it): a write
/// may not take an output's limiter outside its ceiling, or further outside
/// (switched off, or its threshold raised, while already outside); one that
/// brings it closer passes (switching on with a high threshold, lowering a
/// threshold while off), so a limiter can be put right one step at a time.
pub(crate) fn check_limiters(inner: &mut Inner, next: &DspState) -> Result<()> {
    let Some(p) = inner.limits_profile() else {
        return Ok(());
    };
    let p = p.clone();
    let s = &inner.state;
    for o in 0..s.num_output_channels {
        let k = usize::from(o);
        let at = |st: &DspState| {
            (
                st.limiter_enabled.get(k).copied().unwrap_or(false),
                st.limiter_threshold_db.get(k).copied().unwrap_or(0.0),
            )
        };
        let (was, now) = (at(s), at(next));
        if was == now {
            continue;
        }
        let Err(v) = p.check_limiter(o, now.0, now.1) else {
            continue;
        };
        let worse = p.check_limiter(o, was.0, was.1).is_ok() || (was.0 && !now.0) || now.1 > was.1;
        if worse {
            return Err(refused(v));
        }
    }
    Ok(())
}

/// Whether a limits profile would judge the write of `path` (the effects
/// writes simulate only then).
pub(crate) fn judges_fx(inner: &mut Inner, path: &str) -> bool {
    path.starts_with("limit.") && inner.limits_profile().is_some()
}

fn row(path: String, value: f32, applied: f32) -> String {
    OutOfRange {
        path,
        value: f64::from(value),
        applied: Some(f64::from(applied)),
    }
    .line()
}

fn put(v: &mut [f32], k: usize, to: f32, path: String, rows: &mut Vec<String>) {
    if let Some(x) = v.get_mut(k)
        && *x != to
    {
        rows.push(row(path, *x, to));
        *x = to;
    }
}

/// Hold `s` to `p`: every number past a limit to the limit, a limiter the
/// profile wants on switched on, a high-pass it wants raised or added. The
/// rows say what moved, one each.
pub(crate) fn hold_state(p: &LimitsProfile, s: &mut DspState) -> Vec<String> {
    let mut rows = Vec::new();
    let n_in = s.num_input_channels;
    for o in 0..s.num_output_channels {
        let k = usize::from(o);
        let Some(lim) = p.output(o) else {
            continue;
        };
        if let Some(&g) = s.output_gain_db.get(k) {
            put(
                &mut s.output_gain_db,
                k,
                p.clamp_output_gain(o, g),
                format!("out.gain[{o}]"),
                &mut rows,
            );
        }
        if let Some(&d) = s.output_delay_ms.get(k) {
            let to = p.clamp_delay(o, d);
            put(
                &mut s.output_delay_ms,
                k,
                to,
                format!("out.delay[{o}]"),
                &mut rows,
            );
            let ch = usize::from(n_in).saturating_add(k);
            if let Some(x) = s.channel_delays.get_mut(ch) {
                *x = to;
            }
        }
        if let Some(ceiling) = lim.limiter_ceiling_db {
            if let Some(e) = s.limiter_enabled.get_mut(k)
                && !*e
            {
                *e = true;
                rows.push(format!("limit.on[{o}] off → on"));
            }
            if let Some(&t) = s.limiter_threshold_db.get(k)
                && t > ceiling
            {
                put(
                    &mut s.limiter_threshold_db,
                    k,
                    ceiling,
                    format!("limit.threshold[{o}]"),
                    &mut rows,
                );
            }
        }
        if let Some(ch) = n_in.checked_add(o) {
            hold_highpass(p, s, o, ch, &mut rows);
        }
    }
    if p.max_peq_boost_db.is_some() {
        for (ch, bank) in (0u8..).zip(s.filters.iter_mut()) {
            for (b, band) in (0u8..).zip(bank.iter_mut().take(PEQ_BANDS)) {
                let to = p.clamp_boost(band.filter_type, band.gain);
                if to != band.gain {
                    rows.push(row(format!("eq.gain[{ch},{b}]"), band.gain, to));
                    band.gain = to;
                }
            }
        }
    }
    let m = s.master_volume_db;
    let to = p.clamp_master_volume(m);
    if to != m && m > -128.0 {
        rows.push(row("vol.master".into(), m, to));
        s.master_volume_db = to;
    }
    rows
}

/// Give output `o` (wire channel `ch`) the high-pass `p` asks of it: raise
/// an active one that is steep enough, else make a crossover high-pass (one
/// already there, else a flat slot) steep enough at the corner, else say it
/// could not.
fn hold_highpass(p: &LimitsProfile, s: &mut DspState, o: u8, ch: u8, rows: &mut Vec<String>) {
    if p.highpass_kept(o, &views(s, ch, None)) {
        return;
    }
    let Some(lim) = p.output(o) else {
        return;
    };
    let Some(min_hz) = lim.highpass_min_hz else {
        return;
    };
    let slope = lim.highpass_min_slope_db.unwrap_or(6);
    let c = usize::from(ch);
    let steep = |t: u8| shared::highpass_slope(t).is_some_and(|x| x >= slope);
    let name = |b: u8| -> String {
        match b.checked_sub(XOVER_BASE) {
            Some(x) => format!("crossover {}", u16::from(x).saturating_add(1)),
            None => format!("band {}", u16::from(b).saturating_add(1)),
        }
    };
    // An active high-pass steep enough: raise its corner.
    let bands: Vec<(u8, FilterParams)> = views_full(s, ch);
    if let Some((b, mut f)) = bands
        .iter()
        .copied()
        .find(|(_, f)| !f.bypass && steep(f.filter_type))
    {
        rows.push(row(format!("eq.freq[{ch},{b}]"), f.freq, min_hz));
        f.freq = min_hz;
        set_band(s, c, b, f);
        return;
    }
    // A crossover high-pass (any), else a flat crossover slot.
    let slot = bands
        .iter()
        .copied()
        .filter(|(b, _)| *b >= XOVER_BASE)
        .find(|(_, f)| shared::highpass_slope(f.filter_type).is_some())
        .or_else(|| {
            bands
                .iter()
                .copied()
                .filter(|(b, _)| *b >= XOVER_BASE)
                .find(|(_, f)| f.filter_type == 0)
        });
    match slot {
        Some((b, f)) => {
            let filter_type = if steep(f.filter_type) {
                f.filter_type
            } else {
                shared::lr_highpass_for(slope)
            };
            let freq = f.freq.max(min_hz);
            let hp = FilterParams {
                filter_type,
                bypass: false,
                freq,
                q: 0.707,
                gain: 0.0,
                qp: f.qp,
            };
            set_band(s, c, b, hp);
            rows.push(format!(
                "output {}: a {} dB/oct high-pass at {} Hz in {}",
                u16::from(o).saturating_add(1),
                shared::highpass_slope(filter_type).unwrap_or(slope),
                freq.round(),
                name(b)
            ));
        }
        None => rows.push(format!(
            "output {}: no free crossover slot for the high-pass the profile asks for; not changed",
            u16::from(o).saturating_add(1)
        )),
    }
}

fn views_full(s: &DspState, ch: u8) -> Vec<(u8, FilterParams)> {
    let c = usize::from(ch);
    let peq = s.filters.get(c).map(Vec::as_slice).unwrap_or_default();
    let xo = s.xover.get(c).map(Vec::as_slice).unwrap_or_default();
    (0u8..)
        .zip(peq.iter().copied())
        .chain((XOVER_BASE..).zip(xo.iter().copied()))
        .collect()
}

fn set_band(s: &mut DspState, c: usize, b: u8, f: FilterParams) {
    let slot = match b.checked_sub(XOVER_BASE) {
        Some(x) => s.xover.get_mut(c).and_then(|r| r.get_mut(usize::from(x))),
        None => s.filters.get_mut(c).and_then(|r| r.get_mut(usize::from(b))),
    };
    if let Some(slot) = slot {
        *slot = f;
    }
}

/// The report line for held rows, in the import's words; `None` for none.
pub(crate) fn held_line(rows: &[String]) -> Option<String> {
    (!rows.is_empty()).then(|| format!("Held to the limits profile: {}", rows.join(", ")))
}

/// Hold a state about to be sent as a bulk image to the open device's
/// profile (an import, a recall). The rows, also kept for
/// [`DspiCore::limits_status`]. Nothing with no profile or in [`unlimited`].
pub(crate) fn hold(inner: &mut Inner, s: &mut DspState) -> Vec<String> {
    let Some(p) = inner.limits_profile() else {
        return Vec::new();
    };
    let p = p.clone();
    let rows = hold_state(&p, s);
    if !rows.is_empty() {
        inner.limits.last_held.clone_from(&rows);
    }
    rows
}

// ── The bindings' view ──────────────────────────────────────────────────

/// The open device's limits ([`DspiCore::limits_status`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitsStatus {
    /// A profile is in force for the open device.
    pub active: bool,
    /// The open device's serial ("" with none).
    pub serial: String,
    /// How the profile was found: `none`, `ok`, `fell_back` (the profile
    /// file is missing or unreadable; the last good copy is in force) or
    /// `unreadable` (no good copy: running unlimited).
    pub state: String,
    /// The standing notice's key (`limits.fell_back`, `limits.unreadable`;
    /// `crate::notice_text`), "" when there is none to show.
    pub notice_key: String,
    /// Why, for the notice's detail line.
    pub detail: String,
    /// What the last clamp held (an import, a recall, a profile applied, a
    /// slot load), one row each, in the import's words.
    pub last_held: Vec<String>,
}

/// What applying a profile did ([`DspiCore::limits_apply_profile`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitsApply {
    /// The report: "Held to the limits profile: ..." when the device was
    /// outside a narrowed limit, nothing otherwise.
    pub lines: Vec<String>,
    /// What moved, one row each.
    pub held: Vec<String>,
    /// Writes that failed, and why (the profile is kept all the same).
    pub notes: Vec<String>,
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Keep limits profiles (and their PIN hashes) in files under `dir`, the
    /// app's own data folder, one subfolder per device serial (created when
    /// first written; "" forgets the folder). A profile set before a folder
    /// was named is kept for this core's lifetime only.
    ///
    /// Errors: [`DspiError::Unsupported`] on the web (no files: use
    /// [`DspiCore::limits_apply_profile`] with the document the page keeps).
    pub fn limits_set_dir(&self, dir: String) -> Result<()> {
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            let mut inner = self.lock();
            inner.limits.dir = (!dir.trim().is_empty()).then(|| dir.into());
            inner.limits.current = None;
            Ok(())
        }
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            let _ = dir;
            Err(DspiError::Unsupported)
        }
    }

    /// The open device's profile as a document (`meta.kind = "limits"`;
    /// never a PIN), "" when it has none.
    pub fn limits_profile_document(&self) -> String {
        let mut inner = self.lock();
        inner
            .limits_profile()
            .map(LimitsProfile::to_document)
            .unwrap_or_default()
    }

    /// Set the open device's profile from `document` (a limits document, or
    /// any document carrying `meta.limits`), keep it (files when a folder is
    /// named), and hold the device to it at once: a value outside a narrowed
    /// limit is written to the limit (journalled, one undo group), and
    /// reported. The profile's serial is the open device's.
    ///
    /// Errors: [`DspiError::Disconnected`] with no device;
    /// [`DspiError::InvalidArgument`] for a document with no readable
    /// profile; [`DspiError::Io`] when the file could not be written
    /// (nothing changed).
    pub fn limits_apply_profile(&self, document: String) -> Result<LimitsApply> {
        let mut profile = shared::from_document(&document)
            .map_err(|e| DspiError::InvalidArgument { msg: e.to_string() })?;
        {
            let mut inner = self.lock();
            let serial = inner.conn()?.serial.clone();
            profile.serial.clone_from(&serial);
            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
            if let Some(store) = inner.limits.store() {
                store.save(&profile).map_err(|e| DspiError::Io {
                    msg: format!("the limits profile could not be written: {e}"),
                })?;
            }
            inner.limits.memory.retain(|p| p.serial != serial);
            inner.limits.memory.push(profile.clone());
            inner.limits.current = Some(Current {
                serial,
                profile: Some(profile),
                state: LoadState::Ok,
            });
        }
        let (held, notes) = self.limits_enforce();
        Ok(LimitsApply {
            lines: held_line(&held).into_iter().collect(),
            held,
            notes,
        })
    }

    /// Remove the open device's profile (its files too): the device runs as
    /// it does with none. The PIN, if one is set, stays (P1LK).
    ///
    /// Errors: [`DspiError::Disconnected`] with no device; [`DspiError::Io`]
    /// when a file could not be removed.
    pub fn limits_clear(&self) -> Result<()> {
        let mut inner = self.lock();
        let serial = inner.conn()?.serial.clone();
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        if let Some(store) = inner.limits.store() {
            store.clear(&serial).map_err(|e| DspiError::Io {
                msg: format!("the limits profile could not be removed: {e}"),
            })?;
        }
        inner.limits.memory.retain(|p| p.serial != serial);
        inner.limits.current = None;
        inner.limits.last_held.clear();
        Ok(())
    }

    /// The open device's limits: whether a profile is in force, how it was
    /// found, the standing notice when its file is unreadable, and what the
    /// last clamp held. Reads the profile's file once per device.
    pub fn limits_status(&self) -> LimitsStatus {
        let mut inner = self.lock();
        let active = inner.limits_profile().is_some();
        let serial = inner
            .conn
            .as_ref()
            .map(|c| c.serial.clone())
            .unwrap_or_default();
        let current = inner
            .limits
            .current
            .as_ref()
            .filter(|c| c.serial == serial && !serial.is_empty());
        let (state, key, detail) = match current.map(|c| &c.state) {
            Some(LoadState::Ok) => ("ok", "", String::new()),
            Some(LoadState::FellBack { why }) => ("fell_back", "limits.fell_back", why.clone()),
            Some(LoadState::Unreadable { why }) => ("unreadable", "limits.unreadable", why.clone()),
            Some(LoadState::None) | None => ("none", "", String::new()),
        };
        LimitsStatus {
            active,
            serial,
            state: state.into(),
            notice_key: key.into(),
            detail,
            last_held: if active {
                inner.limits.last_held.clone()
            } else {
                Vec::new()
            },
        }
    }
}

impl DspiCore {
    /// Hold the live device to its profile: every value outside a limit
    /// written to it through the typed setters (journalled, one undo group,
    /// each judged like any write), after a profile is applied or a preset
    /// slot is loaded. The rows held, and the writes that failed.
    pub(crate) fn limits_enforce(&self) -> (Vec<String>, Vec<String>) {
        let (before, after, rows) = {
            let mut inner = self.lock();
            let Some(p) = inner.limits_profile().cloned() else {
                return (Vec::new(), Vec::new());
            };
            let before = inner.state.clone();
            let mut after = before.clone();
            let rows = hold_state(&p, &mut after);
            (before, after, rows)
        };
        if rows.is_empty() {
            return (rows, Vec::new());
        }
        let mut notes = Vec::new();
        let mut note = |what: String, r: Result<()>| {
            if let Err(e) = r {
                notes.push(format!("{what}: not written ({e})"));
            }
        };
        self.begin_group(HOLD_LABEL.into());
        let (n_in, n_out) = (after.num_input_channels, after.num_output_channels);
        // The enables first: switching a limiter is audible, so no undo
        // switches one back off, and it stands apart; the values after it
        // are then one undo step (as P1PR's protect-all does).
        for o in 0..n_out {
            let k = usize::from(o);
            if after.limiter_enabled.get(k) != before.limiter_enabled.get(k) {
                note(
                    format!("limit.on[{o}]"),
                    self.set_limiter_param(o, limiter::LIMITER_PARAM_ENABLED as u8, 1.0),
                );
            }
        }
        for o in 0..n_out {
            let k = usize::from(o);
            let differs = |a: &[f32], b: &[f32]| a.get(k) != b.get(k);
            if let Some(&v) = after.output_gain_db.get(k)
                && differs(&before.output_gain_db, &after.output_gain_db)
            {
                note(format!("out.gain[{o}]"), self.set_output_gain(o, v));
            }
            if let Some(&v) = after.output_delay_ms.get(k)
                && differs(&before.output_delay_ms, &after.output_delay_ms)
            {
                note(format!("out.delay[{o}]"), self.set_output_delay(o, v));
            }
            if let Some(&v) = after.limiter_threshold_db.get(k)
                && differs(&before.limiter_threshold_db, &after.limiter_threshold_db)
            {
                note(
                    format!("limit.threshold[{o}]"),
                    self.set_limiter_param(o, limiter::LIMITER_PARAM_THRESHOLD_DB as u8, v),
                );
            }
        }
        for (ch, (was, now)) in (0u8..).zip(before.filters.iter().zip(&after.filters)) {
            for (b, (x, y)) in (0u8..).zip(was.iter().zip(now)) {
                if x != y {
                    note(format!("eq[{ch},{b}]"), self.set_filter(ch, b, *y));
                }
            }
        }
        for (ch, (was, now)) in (0u8..).zip(before.xover.iter().zip(&after.xover)) {
            if ch < n_in {
                continue;
            }
            for (x, (a, b)) in (0u8..).zip(was.iter().zip(now)) {
                if a != b {
                    note(
                        format!("crossover[{ch},{x}]"),
                        self.set_crossover(ch, x, *b),
                    );
                }
            }
        }
        if after.master_volume_db != before.master_volume_db {
            note(
                "vol.master".into(),
                self.set_master_volume(after.master_volume_db),
            );
        }
        self.end_group();
        self.lock().limits.last_held.clone_from(&rows);
        (rows, notes)
    }

    /// After a preset slot load (OD-P1j): read back already, hold what is
    /// outside the profile and report it in [`DspiCore::limits_status`].
    pub(crate) fn limits_after_load(&self) {
        let _ = self.limits_enforce();
    }

    /// `text`, an export, with the open device's profile in `meta.limits`
    /// (never a PIN); unchanged with no profile.
    pub(crate) fn with_limits_meta(&self, text: String) -> String {
        let Some(p) = self.lock().limits_profile().cloned() else {
            return text;
        };
        let Ok(mut doc) = serde_json::from_str::<serde_json::Value>(&text) else {
            return text;
        };
        p.embed(&mut doc);
        serde_json::to_string_pretty(&doc).unwrap_or(text)
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p1lm;
