//! One write path through the session journal: undo, redo and history for
//! every binding (WP P0W).
//!
//! Every typed setter records the change it made in the session's journal
//! (`dspi_session::history`), the one the Terminal's `Session::write` fills:
//!
//! 1. under the core lock, before it sends, the setter notes what the mirror
//!    holds for its registry path ([`Inner::journal_begin`]), and what the
//!    device moves with it (`dspi_session::write::side_effects`: the tube's
//!    voicing, a limiter link group's settings);
//! 2. it sends and patches the mirror exactly as before: no read is added,
//!    so a slider still costs one transfer;
//! 3. once the mirror is patched it records the change with the mirror's new
//!    value ([`Pending::done`]). A change that moved nothing is not recorded.
//!
//! Undo and redo ([`DspiCore::undo`], [`DspiCore::redo`]) replay an entry
//! through [`DspiCore::write_param`], so a reversal takes the typed setter's
//! checks, clamps, device lock and mirror update, and is journalled in turn
//! (source `undo` or `redo`) without becoming a new step.
//!
//! What may be undone is the registry's word (`history::undoable`): only a
//! parameter with no hazard. A flash write, an audible one (an output enable,
//! the signal generator, a preset load), a reconfiguration (pins, clocks, the
//! input source, a bulk image), a deferred control-surface write and an
//! irreversible one are recorded and stepped over. A preset load, a bulk
//! image and a factory reset also end the history before them.
//!
//! The journal belongs to the connection's session: a reconnect (an unplug,
//! a DSPi Link re-attach, another device) starts with none, and a replay that
//! finds a different connection than the one its entry came from is refused
//! ([`DspiError::Disconnected`]) before anything is sent. Nothing is replayed
//! by itself, ever: a write lost with a link is gone, and undo only acts when
//! called.
//!
//! Over DSPi Link an undo is a write like any other: it goes to the hub on
//! the client's session, the hub orders and attributes it to that session as
//! it does every write, and other clients see the device's change as a
//! foreign one (their own journals do not hold it). The journal records this
//! client's changes only.
//!
//! When someone else has changed a value since (another Link client, a knob,
//! the UART, the computer's volume), the mirror no longer holds what this
//! client left there. Undo and redo then leave their value alone, drop the
//! entry and name the other writer in [`HistoryMove::note`], from the
//! notification's source and Link origin (owner, 2026-11-12). A value the
//! mirror holds no single copy of (a limiter write to every output) is not
//! judged.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::cell::Cell;

use dspi_proto::FilterType;
use dspi_proto::packets::MatrixRoutePacket;
use dspi_proto::registry::{ALL_OUTPUTS, Hazard, by_path};
use dspi_proto::value::Value;
use dspi_session::history::{Change, ChangeSource, hazard_of};
use dspi_session::write::{Companion, JournalEntry, side_effects};

use crate::records::{DspState, FilterParams};
use crate::write_param::{ParamValue, band, fx_mirror};
use crate::{DspiCore, DspiError, Inner, Result};

/// The journal's name for a whole band written at once (a type change, an
/// import): its value is [`band_bytes`]. The registry addresses a band's
/// fields one by one; a change of one field is journalled under that field's
/// path.
pub(crate) const EQ_BAND: &str = "eq.band";

/// The journal's name for the extension channel's crossfeed pair map
/// (`set_crossfeed_pair_map`): four bytes, as the binding takes them.
pub(crate) const EXT_PAIRMAP: &str = "ext.pairmap";

/// The first crossover band on the wire (`XOVER_BAND_BASE`, 20).
const XOVER_BASE: u8 = {
    assert!(dspi_proto::generated::filter::XOVER_BAND_BASE <= u8::MAX as u16);
    dspi_proto::generated::filter::XOVER_BAND_BASE as u8
};

/// The journal's own names for changes the registry has no path for, and
/// their hazards. Only a whole band is undone; the rest are recorded.
const OWN_PATHS: &[(&str, Hazard)] = &[
    (EQ_BAND, Hazard::None),
    // The extension channel's crossfeed pair map: live routing, as a
    // crossfeed setting is.
    (EXT_PAIRMAP, Hazard::None),
    // The core-voltage requests: never sent again.
    ("ext.vreg", Hazard::Irreversible),
    // A raw vendor request: nobody knows what it moved.
    ("raw", Hazard::Irreversible),
];

/// The hazard of a journal path: the registry's, or one of [`OWN_PATHS`].
/// The control surfaces' paths count as deferred whatever their row says:
/// their editor writes them (`cs_apply`), and `write_param` refuses them.
fn hazard(path: &str) -> Hazard {
    if path.starts_with("cs.") {
        return Hazard::Deferred;
    }
    OWN_PATHS
        .iter()
        .find(|(p, _)| *p == path)
        .map_or_else(|| hazard_of(path), |(_, h)| *h)
}

/// A replay in progress on this thread: undo or redo, and the connection
/// its entry came from.
#[derive(Debug, Clone, Copy)]
struct Replay {
    source: ChangeSource,
    generation: u64,
}

thread_local! {
    /// Set by [`DspiCore::undo`] and [`DspiCore::redo`] around the setter
    /// call that replays an entry. The setter runs on the same thread, so
    /// the record it makes knows itself for a replay, and a write from
    /// another thread meanwhile does not.
    static REPLAY: Cell<Option<Replay>> = const { Cell::new(None) };
}

/// The replay mark for the life of a replay step: cleared on drop, so a
/// panic inside the setter cannot leave the thread marked.
struct ReplayMark;

impl ReplayMark {
    fn set(r: Replay) -> Self {
        REPLAY.set(Some(r));
        Self
    }
}

impl Drop for ReplayMark {
    fn drop(&mut self) {
        REPLAY.set(None);
    }
}

/// Who last changed the device besides this client, for undo's
/// foreign-change rule (owner 2026-11-12: a value someone else has
/// overwritten since is left as they set it, the entry dropped and the
/// writer named): milliseconds since the Unix epoch and the writer in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForeignWrite {
    pub(crate) at_ms: u64,
    pub(crate) writer: String,
}

/// Note a change another writer made (a notification that was not this
/// client's echo), on the connection.
pub(crate) fn note_foreign(
    conn: &mut crate::Connection,
    source: dspi_session::Source,
    origin: Option<u16>,
) {
    use dspi_session::Source as S;
    let writer = match (origin, source) {
        (Some(sid), _) if sid != 0 => format!("DSPi Link session {sid}"),
        (_, S::Gpio) => "a control surface".into(),
        (_, S::Uac1) => "the computer's volume control".into(),
        (_, S::Uart) => "the UART".into(),
        (_, S::I2c) => "the I2C bus".into(),
        (_, S::Midi) => "MIDI".into(),
        (_, S::Preset) => "a preset load".into(),
        (_, S::Factory) => "a factory reset".into(),
        (_, S::HostSet | S::BulkSet) => "another program on the computer".into(),
        _ => "another writer".into(),
    };
    let at_ms = now_ms();
    conn.last_foreign = Some(ForeignWrite { at_ms, writer });
}

/// Whether someone else changed `entry`'s value since this client left it
/// at `expected`, and who, in words; `None` when the mirror still holds it
/// (or holds no single value for it).
fn overwritten(inner: &Inner, entry: &JournalEntry, expected: &Value) -> Option<String> {
    let now = mirror_value(inner, entry.path, &entry.indices)?;
    if &now == expected {
        return None;
    }
    // -- WP P0WECHO -- a value this session wrote itself not long ago: a late
    // own echo patched it back into the mirror, not another writer.
    if inner.conn.as_ref().is_some_and(|c| {
        c.own_writes
            .explains(entry.path, &entry.indices, &now, now_ms())
    }) {
        return None;
    }
    let who = inner
        .conn
        .as_ref()
        .and_then(|c| c.last_foreign.as_ref())
        .filter(|f| f.at_ms >= entry.at_ms)
        .map_or("another writer", |f| f.writer.as_str());
    Some(format!(
        "`{}` was changed by {who} since; it was left as they set it",
        entry.path
    ))
}

// -- WP P0WECHO --
/// How long a value this session wrote can still come back as its own echo.
/// Echoes come within milliseconds; a loaded host or a slow Link can hold
/// them for seconds, so this is generous.
const OWN_ECHO_MS: u64 = 30_000;

/// The most own writes remembered; the oldest go first.
const OWN_WRITES_CAP: usize = 64;

/// Milliseconds since the Unix epoch, from the clock seam.
fn now_ms() -> u64 {
    dspi_transport::clock::system_time()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The values this session wrote recently, per journal path and indices:
/// what its own echoes, which the mirror takes in on purpose (they correct a
/// value the firmware adjusted), can still bring back. A difference between
/// the mirror and the journal that one of these explains is not another
/// writer's (undo's foreign-change rule). Bounded by count and age; the
/// firmware merges echoes, so they are not counted off one by one.
#[derive(Debug, Default)]
pub(crate) struct OwnWrites(std::collections::VecDeque<OwnWrite>);

#[derive(Debug)]
struct OwnWrite {
    path: &'static str,
    indices: Vec<u8>,
    value: Value,
    at_ms: u64,
}

impl OwnWrites {
    /// Note that this session wrote `value` to `path` at `indices`.
    fn note(&mut self, path: &'static str, indices: &[u8], value: &Value, at_ms: u64) {
        if self.0.len() >= OWN_WRITES_CAP {
            self.0.pop_front();
        }
        self.0.push_back(OwnWrite {
            path,
            indices: indices.to_vec(),
            value: value.clone(),
            at_ms,
        });
    }

    /// Whether `value` at `path`/`indices` is one this session wrote within
    /// [`OWN_ECHO_MS`] of `now_ms`.
    fn explains(&self, path: &str, indices: &[u8], value: &Value, now_ms: u64) -> bool {
        self.0.iter().any(|w| {
            w.path == path
                && w.indices == indices
                && &w.value == value
                && now_ms.saturating_sub(w.at_ms) <= OWN_ECHO_MS
        })
    }
}

/// Refuse a replay on a connection other than the one its entry came from.
/// `Ok` outside a replay.
pub(crate) fn check_replay(inner: &Inner) -> Result<()> {
    let Some(r) = REPLAY.get() else {
        return Ok(());
    };
    match &inner.conn {
        Some(c) if c.generation == r.generation => Ok(()),
        _ => Err(DspiError::Disconnected),
    }
}

/// The source of the replay in progress on this thread, if any.
pub(crate) fn replay_source() -> Option<ChangeSource> {
    REPLAY.get().map(|r| r.source)
}

/// A change a setter is making: what the mirror held before.
#[must_use = "a change is recorded by `done`"]
pub(crate) struct Pending {
    path: &'static str,
    indices: Vec<u8>,
    before: Option<Value>,
    companions: Vec<Companion>,
}

impl Inner {
    /// Begin a journalled change of `path` at `indices`: note what the
    /// mirror holds for it and for what the device moves with it.
    /// [`DspiError::Disconnected`] for a replay whose connection has gone.
    pub(crate) fn journal_begin(&self, path: &'static str, indices: &[u8]) -> Result<Pending> {
        check_replay(self)?;
        let (inputs, outputs) = (
            self.state.num_input_channels,
            self.state.num_output_channels,
        );
        let companions = side_effects(path, indices, inputs, outputs)
            .into_iter()
            .filter_map(|(path, indices)| {
                let before = mirror_value(self, path, &indices)?;
                Some(Companion {
                    path,
                    indices,
                    before,
                })
            })
            .collect();
        Ok(Pending {
            path,
            indices: indices.to_vec(),
            before: mirror_value(self, path, indices),
            companions,
        })
    }
}

impl Pending {
    /// A change of `path` at `indices` whose value before the caller read
    /// itself, with nothing moving besides it.
    pub(crate) fn of(path: &'static str, indices: &[u8], before: Option<Value>) -> Self {
        Self {
            path,
            indices: indices.to_vec(),
            before,
            companions: Vec::new(),
        }
    }

    /// Record the change with the mirror's value now, or `after` where the
    /// mirror holds none.
    pub(crate) fn done(self, inner: &mut Inner, after: Value) {
        let after = mirror_value(inner, self.path, &self.indices).unwrap_or(after);
        self.done_with(inner, after);
    }

    /// Record the change with `after` as its new value. Nothing is recorded
    /// when nothing moved (neither the value nor what moves with it), except
    /// for an undo's or a redo's replay.
    pub(crate) fn done_with(self, inner: &mut Inner, after: Value) {
        let moved = self.before.as_ref() != Some(&after)
            || self
                .companions
                .iter()
                .any(|c| mirror_value(inner, c.path, &c.indices).as_ref() != Some(&c.before));
        // -- WP P0WECHO -- a replay is recorded even when the mirror already
        // showed its value (a late own echo can put it there first): it was
        // sent, and the history says so.
        if !moved && replay_source().is_none() {
            return;
        }
        // A drag is of a live value; a pin moved twice is two changes.
        let coalesce = hazard(self.path) == Hazard::None
            && (matches!(after, Value::Float(_) | Value::Int(_))
                || matches!(self.path, EQ_BAND | "mix"));
        record(
            inner,
            Change {
                path: self.path,
                indices: self.indices,
                before: self.before,
                after,
                companions: self.companions,
                hazard: hazard(self.path),
                source: replay_source().unwrap_or(ChangeSource::Local),
                coalesce,
            },
        );
    }
}

/// Record a change with no value before it to restore (an action, a flash
/// write, a reconfiguration): history, never undone.
pub(crate) fn journal(inner: &mut Inner, path: &'static str, indices: &[u8], after: Value) {
    record(
        inner,
        Change {
            path,
            indices: indices.to_vec(),
            before: None,
            after,
            companions: Vec::new(),
            hazard: hazard(path),
            source: replay_source().unwrap_or(ChangeSource::Local),
            coalesce: false,
        },
    );
}

/// Put `change` in the open session's journal; nowhere without one.
fn record(inner: &mut Inner, change: Change) {
    let Some(conn) = inner.conn.as_mut() else {
        return;
    };
    // -- WP P0WECHO -- its echo may still be on the way
    conn.own_writes
        .note(change.path, &change.indices, &change.after, now_ms());
    if let Ok(session) = conn.session() {
        session.record_change(change);
    }
}

/// The journal's path and values for a whole-band write, from the band as it
/// was and as it is: one field's path when only that field moved, otherwise
/// [`EQ_BAND`] with [`band_bytes`].
pub(crate) fn band_change(was: &FilterParams, now: &FilterParams) -> &'static str {
    let fields = [
        (was.bypass != now.bypass, "eq.bypass"),
        (was.freq != now.freq, "eq.freq"),
        (was.q != now.q, "eq.q"),
        (
            was.gain != now.gain,
            if FilterType::from_raw(now.filter_type).is_linkwitz() {
                "eq.fp"
            } else {
                "eq.gain"
            },
        ),
        (
            was.qp != now.qp && FilterType::from_raw(now.filter_type).is_linkwitz(),
            "eq.qp",
        ),
    ];
    let mut moved = fields.iter().filter(|(m, _)| *m).map(|(_, p)| *p);
    match (
        was.filter_type == now.filter_type,
        moved.next(),
        moved.next(),
    ) {
        (true, Some(one), None) => one,
        _ => EQ_BAND,
    }
}

/// A band as the journal holds it: type, bypass, then freq, Q, gain and the
/// Linkwitz target Q as little-endian `f32`s (18 bytes).
pub(crate) fn band_bytes(p: &FilterParams) -> Vec<u8> {
    let mut v = vec![p.filter_type, u8::from(p.bypass)];
    for f in [p.freq, p.q, p.gain, p.qp] {
        v.extend_from_slice(&f.to_le_bytes());
    }
    v
}

/// [`band_bytes`] read back; `None` for anything else.
pub(crate) fn band_from_bytes(b: &[u8]) -> Option<FilterParams> {
    let f = |k: usize| -> Option<f32> {
        let at = k.checked_mul(4)?.checked_add(2)?;
        Some(f32::from_le_bytes(
            b.get(at..at.checked_add(4)?)?.try_into().ok()?,
        ))
    };
    (b.len() == 18).then_some(())?;
    Some(FilterParams {
        filter_type: *b.first()?,
        bypass: *b.get(1)? != 0,
        freq: f(0)?,
        q: f(1)?,
        gain: f(2)?,
        qp: f(3)?,
    })
}

fn at<T: Clone>(v: &[T], i: u8) -> Option<T> {
    v.get(usize::from(i)).cloned()
}

/// What the mirror holds for journal `path` at `indices`, as the registry's
/// kind; `None` for an action, a value the mirror does not hold, or an index
/// it does not have. "Every output" (`ALL_OUTPUTS`) has no single value.
#[allow(clippy::too_many_lines)] // one arm per path, nothing to factor
pub(crate) fn mirror_value(inner: &Inner, path: &str, ix: &[u8]) -> Option<Value> {
    let s: &DspState = &inner.state;
    let i0 = ix.first().copied().unwrap_or(0);
    let i1 = ix.get(1).copied().unwrap_or(0);
    let f = |x: f32| Some(Value::Float(x));
    let b = |x: bool| Some(Value::Bool(x));
    let c = |x: u8| Some(Value::Choice(x));
    let n = |x: u8| Some(Value::Int(x.into()));
    let m = |x: u32| Some(Value::Mask(x));
    if let Some((fx, id)) = crate::params_fx::fx_param(path) {
        if i0 == ALL_OUTPUTS && path.starts_with("limit.") {
            return None;
        }
        let x = fx_mirror(s, fx, id, i0)?;
        let kind = by_path(path)?.kind;
        return Some(crate::write_param::as_kind(kind, x).into());
    }
    match path {
        "vol.user" => f(s.user_volume_db),
        "vol.mute" => b(s.user_mute),
        "vol.master" => f(s.master_volume_db),
        "vol.master.mode" => c(s.master_volume_mode),
        "pre" => at(&s.input_preamp_db, i0).map(Value::Float),
        "pre.all" => f(s.preamp_db),
        "bypass" => b(s.bypass),
        "eq.type" => band(s, i0, i1).map(|p| Value::Choice(p.filter_type)),
        "eq.freq" => band(s, i0, i1).map(|p| Value::Float(p.freq)),
        "eq.q" => band(s, i0, i1).map(|p| Value::Float(p.q)),
        "eq.gain" | "eq.fp" => band(s, i0, i1).map(|p| Value::Float(p.gain)),
        "eq.qp" => band(s, i0, i1).map(|p| Value::Float(p.qp)),
        "eq.bypass" => band(s, i0, i1).map(|p| Value::Bool(p.bypass)),
        EQ_BAND => band(s, i0, i1).map(|p| Value::Bytes(band_bytes(&p))),
        "ch.delay" => at(&s.channel_delays, i0).map(Value::Float),
        "ch.name" => at(&s.channel_names, i0).map(Value::Text),
        "mix" => {
            let cell = |v: &Vec<Vec<bool>>| v.get(usize::from(i0))?.get(usize::from(i1)).copied();
            let gain = s
                .matrix_gain
                .get(usize::from(i0))?
                .get(usize::from(i1))
                .copied()?;
            let p = MatrixRoutePacket {
                input: i0,
                output: i1,
                enabled: cell(&s.matrix_routing)?,
                phase_invert: cell(&s.matrix_invert)?,
                gain_db: gain,
            };
            Some(Value::Bytes(p.encode().to_vec()))
        }
        "out.enable" => at(&s.output_enabled, i0).map(Value::Bool),
        "out.gain" => at(&s.output_gain_db, i0).map(Value::Float),
        "out.mute" => at(&s.output_muted, i0).map(Value::Bool),
        "out.delay" => at(&s.output_delay_ms, i0).map(Value::Float),
        "out.pin" => at(&s.output_pins, i0).map(|p| Value::Int(p.into())),
        "out.type" => at(&s.output_types, i0).map(Value::Choice),
        "loud.on" => b(s.loudness_enabled),
        "loud.ref" => f(s.loudness_ref_spl),
        "loud.intensity" => f(s.loudness_intensity),
        "loud.mask" => m(s.loudness_output_mask.into()),
        "lev.on" => b(s.leveller_enabled),
        "lev.amount" => f(s.leveller_amount),
        "lev.speed" => c(s.leveller_speed),
        "lev.maxgain" => f(s.leveller_max_gain_db),
        "lev.lookahead" => b(s.leveller_lookahead),
        "lev.gate" => f(s.leveller_gate_db),
        "lev.masks" => {
            m(u32::from(s.leveller_detector_mask)
                | u32::from(s.leveller_apply_mask).wrapping_shl(8))
        }
        "cf.on" => b(s.crossfeed_enabled),
        "cf.preset" => c(s.crossfeed_preset),
        "cf.freq" => f(s.crossfeed_freq),
        "cf.feed" => f(s.crossfeed_feed),
        "cf.itd" => b(s.crossfeed_itd),
        "cf.outputs" => m(s.crossfeed_output_pair_mask.into()),
        "bass.on" => b(s.psybass_enabled),
        "bass.mask" => m(s.psybass_output_mask.into()),
        "sub.solo" => b(s.subharm_solo),
        "in.source" => c(s.input_source),
        "in.name" => inner
            .conn
            .as_ref()
            .and_then(|c| at(&c.source_names, i0))
            .map(Value::Text),
        "in.rate" => Some(Value::Int(match s.i2s_input_rate {
            0 => 44_100,
            2 => 96_000,
            _ => 48_000,
        })),
        "in.spdif.pin" => at(&s.spdif_rx_pins, i0).map(|p| Value::Int(p.into())),
        "in.spdif.enable" => {
            let bit = 1u8.checked_shl(u32::from(i0))?;
            b(s.spdif_inputs_enabled & bit != 0)
        }
        "in.i2s.pin" => at(&s.i2s_rx_pins, i0).map(|p| Value::Int(p.into())),
        "in.i2s.channels" => n(s.i2s_input_channels),
        "in.i2s.clock" => c(s.i2s_clock_mode),
        "in.adat.enable" => b(s.adat_input_enabled),
        "in.adat.pin" => n(s.adat_input_pin),
        "in.adat.clock" => c(s.adat_input_clock_mode),
        "in.lg" => b(s.lg_sound_sync_enabled),
        "i2s.bck" => n(s.i2s_bck_pin),
        "i2s.bck.slave" => n(s.i2s_bck_pin_slave),
        "i2s.clockpins" => c(s.i2s_clock_pin_mode),
        "i2s.mck" => b(s.mck_enabled),
        "i2s.mck.pin" => n(s.mck_pin),
        "i2s.mck.mult" => c(s.mck_multiplier),
        "adat.enable" => b(s.adat_out_enabled),
        "adat.pin" => n(s.adat_out_pin),
        "dev.name" => inner
            .conn
            .as_ref()
            .map(|c| Value::Text(c.device_name.clone())),
        "preset.name" => at(&s.preset_names, i0).map(Value::Text),
        "preset.startup" => Some(Value::Bytes(vec![
            s.preset_startup_mode,
            s.preset_default_slot,
        ])),
        "preset.iomode" => c(s.output_config_mode),
        EXT_PAIRMAP => inner
            .conn
            .as_ref()
            .and_then(|c| c.ext.pair_map)
            .map(|m| Value::Bytes(m.0.to_vec())),
        _ => None,
    }
}

// ── The bindings' view ──────────────────────────────────────────────

/// Who made a change in the history.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HistorySource {
    /// A write this program asked for.
    Local,
    /// The reversal of an earlier change, sent by [`DspiCore::undo`].
    Undo,
    /// The repeat of an undone change, sent by [`DspiCore::redo`].
    Redo,
}

impl From<ChangeSource> for HistorySource {
    fn from(s: ChangeSource) -> Self {
        match s {
            ChangeSource::Local => Self::Local,
            ChangeSource::Undo => Self::Undo,
            ChangeSource::Redo => Self::Redo,
        }
    }
}

/// One change in the history ([`DspiCore::history`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    /// The registry path (`out.gain`, `eq.freq`, ...), or one of the
    /// journal's own: `eq.band` (a whole band: type, bypass, then freq, Q,
    /// gain and the Linkwitz target Q as little-endian `f32`s), `ext.pairmap`
    /// (the crossfeed pair map's four bytes), `ext.vreg` (a core-voltage
    /// request: op and value), `raw` (a raw request: opcode and `wValue` as
    /// indices, the payload as the value). A control-surface operation is
    /// under its `cs.*` path with the operation's JSON as text.
    pub path: String,
    /// The path's indices (an output, a channel and a band, ...).
    pub indices: Vec<u8>,
    /// The value before, when known; undo writes it back.
    pub before: Option<ParamValue>,
    /// The value after (`trigger` for an action).
    pub after: ParamValue,
    /// Who made it.
    pub source: HistorySource,
    /// When, in milliseconds since the Unix epoch (a drag: its last step).
    pub at_ms: u64,
    /// Whether undo may reverse it. The registry decides: only a parameter
    /// with no hazard (not a flash write, an audible one, a reconfiguration,
    /// a deferred or an irreversible one).
    pub undoable: bool,
}

impl From<&JournalEntry> for HistoryEntry {
    fn from(e: &JournalEntry) -> Self {
        Self {
            path: e.path.into(),
            indices: e.indices.clone(),
            before: e.before.clone().map(Into::into),
            after: e.after.clone().into(),
            source: e.source.into(),
            at_ms: e.at_ms,
            undoable: e.undoable,
        }
    }
}

/// What [`DspiCore::undo`] or [`DspiCore::redo`] did.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryMove {
    /// The change reversed (undo) or applied again (redo); `None` when
    /// nothing moved.
    pub entry: Option<HistoryEntry>,
    /// The changes stepped over on the way, newest first, which cannot be
    /// undone (see [`HistoryEntry::undoable`]). An entry here that is
    /// undoable had no value before to restore, or someone else has changed
    /// it since (`note` says who); the walk stopped there.
    pub skipped: Vec<HistoryEntry>,
    /// Why nothing moved, in words, when another writer changed the value
    /// since (only where the core leaves such a change alone); `None`
    /// otherwise.
    pub note: Option<String>,
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Step back: write back the value the newest undoable change replaced
    /// (then what the device moved with it), through the typed setter for
    /// its path. Changes that cannot be undone are stepped over and named in
    /// [`HistoryMove::skipped`]. Nothing to undo is not an error: `entry` is
    /// then `None`.
    ///
    /// Errors: [`DspiError::Disconnected`] with no device, or when the
    /// connection changed during the call (nothing is sent to another
    /// device); the setter's errors, after which the change stays to be
    /// undone.
    pub fn undo(&self) -> Result<HistoryMove> {
        self.undo_step()
    }
    /// Step forward: write again the change [`DspiCore::undo`] most recently
    /// reversed. A new change since then ends what can be redone.
    ///
    /// Errors: as [`DspiCore::undo`]; after a setter's error the change
    /// stays to be redone.
    pub fn redo(&self) -> Result<HistoryMove> {
        self.redo_step()
    }

    /// Whether [`DspiCore::undo`] has a change to reverse. False with no
    /// device.
    pub fn can_undo(&self) -> bool {
        self.lock()
            .conn
            .as_mut()
            .and_then(|c| c.session().ok())
            .is_some_and(|s| s.can_undo())
    }

    /// Whether [`DspiCore::redo`] has a change to apply again. False with no
    /// device.
    pub fn can_redo(&self) -> bool {
        self.lock()
            .conn
            .as_mut()
            .and_then(|c| c.session().ok())
            .is_some_and(|s| s.can_redo())
    }

    /// The newest `n` changes of this connection, newest first: every typed
    /// setter's, undo's and redo's, with the value before and after, who
    /// made it and when. The connection's session keeps the newest 1000; a
    /// reconnect starts with none. Empty with no device.
    pub fn history(&self, n: u32) -> Vec<HistoryEntry> {
        let n = usize::try_from(n).unwrap_or(usize::MAX);
        self.lock()
            .conn
            .as_mut()
            .and_then(|c| c.session().ok())
            .map(|s| s.history(n).iter().map(HistoryEntry::from).collect())
            .unwrap_or_default()
    }
}

impl DspiCore {
    /// [`DspiCore::undo`]. A change someone else has overwritten since (the
    /// mirror no longer holds the value this client left) is not reversed:
    /// theirs stays, the entry is dropped and [`HistoryMove::note`] names
    /// them (owner 2026-11-12).
    fn undo_step(&self) -> Result<HistoryMove> {
        let (step, generation, note) = {
            let mut inner = self.lock();
            let conn = inner.conn()?;
            let generation = conn.generation;
            let mut step = conn.session()?.take_undo();
            let note = step
                .entry
                .as_ref()
                .and_then(|e| overwritten(&inner, e, &e.after));
            if note.is_some() {
                // Theirs now: the entry describes a value that is gone.
                step.skipped.extend(step.entry.take());
            }
            (step, generation, note)
        };
        let skipped = step.skipped.iter().map(HistoryEntry::from).collect();
        let Some(entry) = step.entry else {
            return Ok(HistoryMove {
                entry: None,
                skipped,
                note,
            });
        };
        let steps = entry
            .before
            .iter()
            .map(|v| (entry.path, entry.indices.clone(), v.clone()))
            .chain(
                entry
                    .companions
                    .iter()
                    .map(|c| (c.path, c.indices.clone(), c.before.clone())),
            );
        let replay = Replay {
            source: ChangeSource::Undo,
            generation,
        };
        for (path, ix, value) in steps {
            if let Err(e) = self.replay(replay, path, &ix, value) {
                self.with_session(generation, |s| s.restore_undo(entry));
                return Err(e);
            }
        }
        let moved = HistoryEntry::from(&entry);
        self.with_session(generation, |s| s.finish_undo(entry));
        Ok(HistoryMove {
            entry: Some(moved),
            skipped,
            note: None,
        })
    }

    /// [`DspiCore::redo`]: as [`DspiCore::undo`], a change someone else has
    /// overwritten since undo put it back is left as they set it.
    fn redo_step(&self) -> Result<HistoryMove> {
        let (entry, generation) = {
            let mut inner = self.lock();
            let conn = inner.conn()?;
            let generation = conn.generation;
            let entry = conn.session()?.take_redo();
            let note = entry
                .as_ref()
                .and_then(|e| e.before.as_ref().and_then(|b| overwritten(&inner, e, b)));
            if let Some(note) = note {
                let skipped = entry.iter().map(HistoryEntry::from).collect();
                return Ok(HistoryMove {
                    entry: None,
                    skipped,
                    note: Some(note),
                });
            }
            (entry, generation)
        };
        let Some(entry) = entry else {
            return Ok(HistoryMove::default());
        };
        let replay = Replay {
            source: ChangeSource::Redo,
            generation,
        };
        if let Err(e) = self.replay(replay, entry.path, &entry.indices, entry.after.clone()) {
            self.with_session(generation, |s| s.restore_redo(entry));
            return Err(e);
        }
        let moved = HistoryEntry::from(&entry);
        self.with_session(generation, |s| s.finish_redo(entry));
        Ok(HistoryMove {
            entry: Some(moved),
            skipped: Vec::new(),
            note: None,
        })
    }

    /// Write one step of a replay through the typed setter for `path`, the
    /// thread marked as replaying so the record knows it for one.
    fn replay(&self, replay: Replay, path: &str, ix: &[u8], value: Value) -> Result<()> {
        let _mark = ReplayMark::set(replay);
        self.write_journal_value(path, ix, value)
    }

    /// A journal path's value through its writer: a whole band through
    /// `set_filter` or `set_crossover`, anything else through
    /// [`DspiCore::write_param`].
    fn write_journal_value(&self, path: &str, ix: &[u8], value: Value) -> Result<()> {
        if path == EXT_PAIRMAP {
            return match value {
                Value::Bytes(map) => self.set_crossfeed_pair_map(map),
                _ => Err(DspiError::InvalidArgument {
                    msg: format!("{EXT_PAIRMAP}: not a pair map"),
                }),
            };
        }
        if path != EQ_BAND {
            return self
                .write_param(path.into(), ix.to_vec(), value.into())
                .map(drop);
        }
        let (Some(&ch), Some(&b)) = (ix.first(), ix.get(1)) else {
            return Err(DspiError::InvalidArgument {
                msg: format!("{EQ_BAND} takes a channel and a band, not {ix:?}"),
            });
        };
        let p = match &value {
            Value::Bytes(bytes) => band_from_bytes(bytes),
            _ => None,
        }
        .ok_or_else(|| DspiError::InvalidArgument {
            msg: format!("{EQ_BAND} [{ch}, {b}]: not a band"),
        })?;
        match b.checked_sub(XOVER_BASE) {
            Some(x) => self.set_crossover(ch, x, p),
            None => self.set_filter(ch, b, p),
        }
    }

    /// Run `f` on the session of connection `generation`, if it is still
    /// the open one.
    fn with_session(&self, generation: u64, f: impl FnOnce(&mut dspi_session::Session)) {
        let mut inner = self.lock();
        if let Some(c) = inner.conn.as_mut().filter(|c| c.generation == generation)
            && let Ok(s) = c.session()
        {
            f(s);
        }
    }
}

/// The history's state for `NOTIFY_HISTORY`: the connection and its
/// journal's version, `None` while there is no history at all.
pub(crate) fn history_mark(inner: &mut Inner) -> Option<(u64, u64)> {
    let conn = inner.conn.as_mut()?;
    let generation = conn.generation;
    let version = conn.session().ok()?.history_version();
    (version != 0).then_some((generation, version))
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "journal/tests_wp_p0w.rs"]
mod tests_wp_p0w;
