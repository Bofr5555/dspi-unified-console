//! The installation as the hub keeps it (WP SW2): rooms, zones, groups, sources,
//! the audio link's senders, fan-outs and units, and the scenes. What the hub
//! cannot learn from the stream is here: which alink output and which re-emitters
//! feed each unit (the stream's `hops` counts re-emitters, not frames), the slots
//! each unit takes, its measured or configured latency, where its speakers stand.
//!
//! Persisted in the hub's store as JSON (`system` in `hub-store.json`) and
//! exchanged as the same JSON over Link (`system.get`, `system.set`). Every
//! field is checked by [`SystemModel::validate`] before it is kept: ids,
//! references, the feed graph (it must end at a sender, without loops), ranges
//! and sizes, so a file or a client cannot hand the alignment engine a model it
//! cannot compute.
//!
//! -- WP INST0 -- Moved here from `dspi-hub` (`system/model.rs`, which
//! re-exports it, so the hub's paths stay): the clients build and check a
//! model with the hub's own types and validator (`dspi-core`'s Installation
//! setup, the Terminal's `usb-direct generate`). The hub keeps what is its
//! policy alone (which command a scene op stands for).
#![warn(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::{BTreeMap, BTreeSet};

use dspi_proto::ext::alink::{ALINK_MAX_CHANNELS, ALINK_SLOT_SILENT, ApplyOp, ApplyRequest};
use serde::{Deserialize, Serialize};

/// The model's format version.
pub const MODEL_VERSION: u32 = 1;
/// At most this many of each kind of element.
pub const MAX_ELEMENTS: usize = 256;
/// At most this many actions in a scene.
pub const MAX_SCENE_ACTIONS: usize = 32;
/// A unit's apply queue (`extension_channel_spec.md` 2.8.7): a scene may give
/// one unit at most this many ops.
pub const MAX_OPS_PER_UNIT: usize = 4;
/// Longest id and name, in characters.
pub const MAX_TEXT: usize = 64;
/// Latencies and the lip-sync offset are within one second.
pub const MAX_LATENCY_US: i32 = 1_000_000;
/// The furthest speaker, in metres.
pub const MAX_DISTANCE_M: f32 = 100.0;
/// The lead an apply gets by default: 2400 frames, 50 ms at 48 kHz (SYS2 3.3).
pub const DEFAULT_APPLY_LEAD: u32 = 2400;
/// The smallest lead: two superframes, so one corrupted superframe costs
/// nothing (`extension_channel_spec.md` 2.8.9).
pub const MIN_APPLY_LEAD: u32 = 64;
/// The largest lead: ten seconds at 96 kHz.
pub const MAX_APPLY_LEAD: u32 = 960_000;
// -- WP ATHUBM --
/// At most this many measurement runs named by one speaker's measured
/// arrival ([`Speaker::measured_runs`]).
pub const MAX_MEASURED_RUNS: usize = 16;

fn default_version() -> u32 {
    MODEL_VERSION
}

fn default_rate() -> u32 {
    48_000
}

fn default_lead() -> u32 {
    DEFAULT_APPLY_LEAD
}

/// The whole installation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SystemModel {
    /// [`MODEL_VERSION`].
    #[serde(default = "default_version")]
    pub version: u32,
    /// The system delay D in alink frames. `None`: the alignment engine picks the
    /// smallest that every unit accepts.
    #[serde(default)]
    pub d_frames: Option<u16>,
    /// The sample rate the microsecond figures convert at while no unit reports
    /// the stream's (44100, 48000 or 96000).
    #[serde(default = "default_rate")]
    pub rate_hz: u32,
    /// Lip sync: how much later than the video every room's audio plays, µs,
    /// unless the room has its own.
    #[serde(default)]
    pub lip_sync_us: u32,
    /// Frames between "now" and the frame a scene applies at.
    #[serde(default = "default_lead")]
    pub apply_lead_frames: u32,
    /// Rooms: one sound field, aligned to the sample.
    #[serde(default)]
    pub rooms: Vec<Room>,
    /// Zones: rooms that share a source.
    #[serde(default)]
    pub zones: Vec<Zone>,
    /// Groups: named sets of units, for scenes.
    #[serde(default)]
    pub groups: Vec<Group>,
    /// Sources: programmes on the alink.
    #[serde(default)]
    pub sources: Vec<Source>,
    /// The alink's senders (the master clock).
    #[serde(default)]
    pub senders: Vec<Sender>,
    /// Fan-outs: senders slaved to an alink input, re-emitting the stream.
    #[serde(default)]
    pub fan_outs: Vec<FanOut>,
    /// The receiving units.
    #[serde(default)]
    pub units: Vec<Unit>,
    /// Scenes: ops applied at one frame.
    #[serde(default)]
    pub scenes: Vec<Scene>,
    // -- WP FIXM1 --
    /// "Keep USB-direct units aligned" (Installer, off by default): when a
    /// USB-direct stream's start offset moves (the host reopened a stream, a
    /// unit anchored again), the hub writes the USB-direct units' output
    /// delays again by itself and records it (alarm `usb_realigned`); off,
    /// it raises the issue `usb_start_offsets_changed` instead.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep_usb_aligned: bool,
}

impl Default for SystemModel {
    fn default() -> Self {
        Self {
            version: MODEL_VERSION,
            d_frames: None,
            rate_hz: default_rate(),
            lip_sync_us: 0,
            apply_lead_frames: DEFAULT_APPLY_LEAD,
            rooms: Vec::new(),
            zones: Vec::new(),
            groups: Vec::new(),
            sources: Vec::new(),
            senders: Vec::new(),
            fan_outs: Vec::new(),
            units: Vec::new(),
            scenes: Vec::new(),
            keep_usb_aligned: false,
        }
    }
}

/// A room.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Room {
    /// Unique among every element.
    pub id: String,
    /// What people call it.
    #[serde(default)]
    pub name: String,
    /// This room's lip-sync offset, µs, instead of the system's.
    #[serde(default)]
    pub lip_sync_us: Option<u32>,
}

/// A zone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Zone {
    /// Unique among every element.
    pub id: String,
    /// What people call it.
    #[serde(default)]
    pub name: String,
    /// Its rooms.
    #[serde(default)]
    pub rooms: Vec<String>,
    /// What it plays.
    #[serde(default)]
    pub source: Option<String>,
}

/// A group of units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Group {
    /// Unique among every element.
    pub id: String,
    /// What people call it.
    #[serde(default)]
    pub name: String,
    /// Its units.
    #[serde(default)]
    pub units: Vec<String>,
    /// The installer's note.
    #[serde(default)]
    pub note: String,
}

/// A programme carried on audio link channels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Source {
    /// Unique among every element.
    pub id: String,
    /// What people call it.
    #[serde(default)]
    pub name: String,
    /// The sender whose stream carries it.
    #[serde(default)]
    pub sender: Option<String>,
    /// The audio link channels it occupies.
    #[serde(default)]
    pub channels: Vec<u8>,
}

/// A sender: the master clock and its alink outputs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Sender {
    /// Unique among every element.
    pub id: String,
    /// What people call it.
    #[serde(default)]
    pub name: String,
    /// Its USB serial when the hub can reach it; the superframe path needs it.
    #[serde(default)]
    pub serial: Option<String>,
    /// The id it puts in the superframe (the low byte of its serial), to check
    /// that each unit hears it.
    #[serde(default)]
    pub sender_id: Option<u8>,
}

/// A fan-out: a sender slaved to an alink input, re-emitting the stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FanOut {
    /// Unique among every element.
    pub id: String,
    /// What people call it.
    #[serde(default)]
    pub name: String,
    /// Its USB serial when the hub can reach it, to read its hop delay.
    #[serde(default)]
    pub serial: Option<String>,
    /// What feeds it: a sender, a fan-out or a repeating unit.
    pub upstream: String,
    /// Which output of the upstream element, for the record.
    #[serde(default)]
    pub output: Option<u8>,
    /// Its hop delay in frames when it cannot be read.
    #[serde(default)]
    pub hop_delay_frames: Option<u8>,
}

/// A speaker on one of a unit's outputs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)] // Default: WP ATHUBM
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Speaker {
    /// The unit's output, from 0.
    pub output: u8,
    /// From the speaker to the listening position, metres.
    pub distance_m: f32,
    // -- WP ATHUBM --
    /// The measured arrival at the listening position, µs, relative to the
    /// latest output of the group it was measured with (0 for that one,
    /// negative for an earlier one; PLAN-AT 1.5, OD-ATl). The acoustic part
    /// only: the measuring client takes out what the alignment engine adds
    /// itself (the unit's play-out, its latency and lip-sync terms and the
    /// output delay in effect while it measured). Where present the engine
    /// uses it instead of the distance (`system::align`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measured_arrival_us: Option<f32>,
    /// The measurement runs it came from (`EXT_SIGGEN_TIMELINE.run_id`s),
    /// at most [`MAX_MEASURED_RUNS`]. Speakers with the same runs were
    /// measured together and share one reference.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub measured_runs: Vec<u32>,
    /// When it was measured, Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measured_at: Option<u64>,
}

/// A receiving unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Unit {
    /// Unique among every element.
    pub id: String,
    /// What people call it.
    #[serde(default)]
    pub name: String,
    /// Its USB serial: how the hub finds it.
    pub serial: String,
    /// Where it stands.
    #[serde(default)]
    pub room: Option<String>,
    /// What feeds it: a sender, a fan-out or a repeating unit. -- WP AUSB1 --
    /// Empty for a USB-direct unit, which the host's USB bus feeds.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub upstream: String,
    // -- WP AUSB1 --
    /// How the programme reaches it: the audio link (the default) or USB
    /// directly from the host (`extension_channel_spec.md` 2.9).
    #[serde(default, skip_serializing_if = "UnitTransport::is_alink")]
    pub transport: UnitTransport,
    /// A USB-direct unit's mode, written with the alignment: `off`,
    /// `rate_lock` or `phase_lock`. `None`: the unit's own mode is kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usb_mode: Option<String>,
    // -- WP AUSB2 --
    /// What a USB-direct unit in phase lock does when it cannot be in step
    /// (*lock error*, the fork's WP USB3b): mute (the default) or play
    /// unaligned. The hub writes it to the unit (`EXT_ALINK_USB` byte 4).
    #[serde(default, skip_serializing_if = "OnLockError::is_mute")]
    pub on_lock_error: OnLockError,
    /// Which output of the upstream element, for the record.
    #[serde(default)]
    pub output: Option<u8>,
    /// It re-emits the stream (a repeater) to the units that name it upstream.
    #[serde(default)]
    pub repeater: bool,
    /// A repeater's hop delay in frames when it cannot be read.
    #[serde(default)]
    pub hop_delay_frames: Option<u8>,
    /// The audio link channel for each of its 8 matrix inputs, 255 for silence;
    /// `None` leaves the unit's own table alone.
    #[serde(default)]
    pub slots: Option<Vec<u8>>,
    /// Latency after the link (DAC, amplifier, a limiter's lookahead) as
    /// configured, µs.
    #[serde(default)]
    pub latency_us: i32,
    /// The same, measured: wins over `latency_us`.
    #[serde(default)]
    pub measured_latency_us: Option<i32>,
    /// Speakers by output, for the distance delays.
    #[serde(default)]
    pub speakers: Vec<Speaker>,
    // -- WP ATHUBM --
    /// It is an S/PDIF time-base source (`EXT_ALINK_SPDIF` mode 2): it stamps
    /// its own counter on its S/PDIF outputs, and the units with
    /// `"transport": "spdif"` that name it `upstream` follow it. A source may
    /// itself be fed by the audio link or USB-direct, or (`"transport":
    /// "spdif"` with no `upstream`) by anything the model does not hold.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub spdif_source: bool,
    /// A source's S/PDIF output slots that carry the time base
    /// (`EXT_ALINK_SPDIF.tx_mask`, bit n = output slot n = outputs 2n and
    /// 2n + 1). A source must name them (never 0, "every S/PDIF slot"), so
    /// the hub keeps them out of every measurement (PLAN-AT S-AT6).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub spdif_tx_mask: u8,
    /// A source's S/PDIF domain delay D in samples, the base of every
    /// follower's D. `None`: the alignment engine picks the smallest every
    /// follower takes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spdif_d: Option<u16>,
}

// -- WP ATHUBM --
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(v: &u8) -> bool {
    *v == 0
}

impl Unit {
    /// The latency the engine uses: measured, else configured.
    pub fn latency(&self) -> i32 {
        self.measured_latency_us.unwrap_or(self.latency_us)
    }

    // -- WP AUSB1 --
    /// A USB-direct unit.
    pub fn usb_direct(&self) -> bool {
        self.transport == UnitTransport::UsbDirect
    }

    // -- WP ATHUBM --
    /// A unit on the audio link (fed by a sender, a fan-out or a repeater).
    pub fn on_alink(&self) -> bool {
        self.transport == UnitTransport::Alink
    }

    /// A unit whose transport is S/PDIF: a follower, or a source fed by
    /// nothing the model holds.
    pub fn spdif(&self) -> bool {
        self.transport == UnitTransport::Spdif
    }

    /// An S/PDIF follower: it plays its `upstream` source's time base.
    pub fn spdif_follower(&self) -> bool {
        self.spdif() && !self.spdif_source
    }

    /// The S/PDIF output slots of a source that carry the time base, as
    /// output numbers (`[2n, 2n + 1]` for slot n).
    pub fn spdif_tx_outputs(&self) -> Vec<u8> {
        if !self.spdif_source {
            return Vec::new();
        }
        (0..8u8)
            .filter(|n| self.spdif_tx_mask & (1 << n) != 0)
            .flat_map(|n| {
                let o = n.saturating_mul(2);
                [o, o.saturating_add(1)]
            })
            .collect()
    }
}

// -- WP AUSB1 --
/// How the programme reaches a unit ([`Unit::transport`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitTransport {
    /// The audio link, from a sender (through fan-outs and repeaters).
    #[default]
    Alink,
    /// USB from the host, the unit locked to its Start of Frame.
    UsbDirect,
    // -- WP ATHUBM --
    /// S/PDIF from a source unit (`upstream`) that stamps its time base in
    /// the user bits (`extension_channel_spec.md` 2.10); or, with
    /// `spdifSource` and no `upstream`, a source fed by anything else.
    Spdif,
}

impl UnitTransport {
    /// The default, left out of the JSON.
    pub fn is_alink(&self) -> bool {
        *self == Self::Alink
    }
}

// -- WP AUSB2 --
/// What a USB-direct unit does in lock error ([`Unit::on_lock_error`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnLockError {
    /// Stay muted until it is in step again (the firmware's default).
    #[default]
    Mute,
    /// Play, unaligned (rate-free, rate-locked or with its phase error).
    PlayUnaligned,
}

impl OnLockError {
    /// The default, left out of the JSON.
    pub fn is_mute(&self) -> bool {
        *self == Self::Mute
    }

    /// `EXT_ALINK_USB` byte 4.
    pub fn code(self) -> u8 {
        match self {
            Self::Mute => dspi_proto::ext::alink_usb::ON_LOCK_ERROR_MUTE,
            Self::PlayUnaligned => dspi_proto::ext::alink_usb::ON_LOCK_ERROR_PLAY,
        }
    }

    /// `mute` or `play_unaligned`, as in the JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mute => "mute",
            Self::PlayUnaligned => "play_unaligned",
        }
    }
}

// -- WP AUSB1 --
/// The USB-direct modes a model may name ([`Unit::usb_mode`]).
pub const USB_MODES: [&str; 3] = ["off", "rate_lock", "phase_lock"];

/// A scene: ops that take effect at the same alink frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scene {
    /// Unique among every element.
    pub id: String,
    /// What people call it.
    #[serde(default)]
    pub name: String,
    /// What it does, in order.
    #[serde(default)]
    pub actions: Vec<SceneAction>,
    // -- fixup sw2x (F1) --
    /// Set (`true`) or clear (`false`) the system mute on the sender with the
    /// scene. `None`, the default: the scene leaves it as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_mute: Option<bool>,
}

/// One op of a scene, on a set of units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SceneAction {
    /// `all`, or `room:<id>`, `zone:<id>`, `group:<id>`, `unit:<id>`.
    pub target: String,
    /// `mute`, `unmute`, `preset_load`, `master_volume`, `out_delay`,
    /// `click` or `sync_pulse` (`extension_channel_spec.md` 2.8.7).
    pub op: String,
    /// The op's `arg`: a preset slot, an output.
    #[serde(default)]
    pub arg: u8,
    /// The op's `value`: dB x 100, samples.
    #[serde(default)]
    pub value: i16,
}

/// The apply op a scene action names; `None` for an unknown one, `cancel_all`
/// and `rate` (a scene never changes the rate: only the sender schedules it).
pub fn scene_op(name: &str) -> Option<ApplyOp> {
    Some(match name {
        "mute" => ApplyOp::Mute,
        "unmute" => ApplyOp::Unmute,
        "preset_load" => ApplyOp::PresetLoad,
        "master_volume" => ApplyOp::MasterVolume,
        "out_delay" => ApplyOp::OutDelay,
        "click" => ApplyOp::Click,
        "sync_pulse" => ApplyOp::SyncPulse,
        _ => return None,
    })
}

/// What kind of element an id names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A [`Room`].
    Room,
    /// A [`Zone`].
    Zone,
    /// A [`Group`].
    Group,
    /// A [`Source`].
    Source,
    /// A [`Sender`].
    Sender,
    /// A [`FanOut`].
    FanOut,
    /// A [`Unit`].
    Unit,
    /// A [`Scene`].
    Scene,
}

/// Why a model was not taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid(pub String);

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// -- WP INST0 --
impl Invalid {
    /// The id of the element the refusal is about, as its words name it
    /// (`unit A: ...`, `id "A" is used twice`, `A: its feed ...`); `None`
    /// for the model as a whole (its version, rate, sizes, a serial).
    pub fn subject(&self) -> Option<String> {
        const KINDS: [&str; 6] = ["unit ", "zone ", "group ", "source ", "fan-out ", "scene "];
        let m = self.0.as_str();
        if let Some(rest) = m.strip_prefix("id \"") {
            return rest.split('"').next().map(str::to_owned);
        }
        for k in KINDS {
            if let Some((id, _)) = m.strip_prefix(k).and_then(|r| r.split_once(':')) {
                return Some(id.to_owned());
            }
        }
        m.split_once(": its feed ").map(|(id, _)| id.to_owned())
    }
}

fn bad<T>(msg: impl Into<String>) -> Result<T, Invalid> {
    Err(Invalid(msg.into()))
}

fn text_ok(what: &str, t: &str) -> Result<(), Invalid> {
    if t.chars().count() > MAX_TEXT || t.chars().any(char::is_control) {
        return bad(format!(
            "{what}: at most {MAX_TEXT} characters, none of them control characters"
        ));
    }
    Ok(())
}

fn count_ok<T>(what: &str, v: &[T]) -> Result<(), Invalid> {
    if v.len() > MAX_ELEMENTS {
        return bad(format!("{what}: at most {MAX_ELEMENTS}"));
    }
    Ok(())
}

fn latency_ok(what: &str, us: i64) -> Result<(), Invalid> {
    if us.abs() > i64::from(MAX_LATENCY_US) {
        return bad(format!("{what}: within {MAX_LATENCY_US} µs"));
    }
    Ok(())
}

impl SystemModel {
    /// What each id names. Assumes [`Self::validate`] passed (ids unique).
    pub fn kinds(&self) -> BTreeMap<&str, Kind> {
        let mut m = BTreeMap::new();
        for x in &self.rooms {
            m.insert(x.id.as_str(), Kind::Room);
        }
        for x in &self.zones {
            m.insert(x.id.as_str(), Kind::Zone);
        }
        for x in &self.groups {
            m.insert(x.id.as_str(), Kind::Group);
        }
        for x in &self.sources {
            m.insert(x.id.as_str(), Kind::Source);
        }
        for x in &self.senders {
            m.insert(x.id.as_str(), Kind::Sender);
        }
        for x in &self.fan_outs {
            m.insert(x.id.as_str(), Kind::FanOut);
        }
        for x in &self.units {
            m.insert(x.id.as_str(), Kind::Unit);
        }
        for x in &self.scenes {
            m.insert(x.id.as_str(), Kind::Scene);
        }
        m
    }

    /// The unit with this id.
    pub fn unit(&self, id: &str) -> Option<&Unit> {
        self.units.iter().find(|u| u.id == id)
    }

    /// The scene with this id.
    pub fn scene(&self, id: &str) -> Option<&Scene> {
        self.scenes.iter().find(|s| s.id == id)
    }

    /// The element that feeds `id` (a fan-out or unit), if any.
    fn upstream_of(&self, id: &str) -> Option<&str> {
        self.fan_outs
            .iter()
            .find(|f| f.id == id)
            .map(|f| f.upstream.as_str())
            .or_else(|| {
                self.unit(id)
                    .filter(|u| u.on_alink()) // -- WP ATHUBM -- (was: not USB-direct)
                    .map(|u| u.upstream.as_str())
            })
    }

    // -- WP AUSB1 --
    /// The units on the audio link (not USB-direct), in model order.
    pub fn alink_units(&self) -> impl Iterator<Item = &Unit> {
        // -- WP ATHUBM -- S/PDIF units are not on the audio link either
        self.units.iter().filter(|u| u.on_alink())
    }

    /// The path from a sender to `id`: the sender, then the re-emitters in
    /// order, not `id` itself. `None` when it does not end at a sender within
    /// [`MAX_ELEMENTS`] steps (a loop, a dangling reference).
    pub fn path_to(&self, id: &str) -> Option<(String, Vec<String>)> {
        let mut reemitters = Vec::new();
        let mut at = self.upstream_of(id)?;
        for _ in 0..=MAX_ELEMENTS.saturating_mul(2) {
            if self.senders.iter().any(|s| s.id == at) {
                reemitters.reverse();
                return Some((at.to_owned(), reemitters));
            }
            reemitters.push(at.to_owned());
            at = self.upstream_of(at)?;
        }
        None
    }

    /// The units an action's target names, in model order. `None` for a
    /// target that names nothing in the model.
    pub fn resolve_target(&self, target: &str) -> Option<Vec<&Unit>> {
        if target == "all" {
            return Some(self.units.iter().collect());
        }
        let (kind, id) = target.split_once(':')?;
        let pick = |f: &dyn Fn(&Unit) -> bool| self.units.iter().filter(|u| f(u)).collect();
        Some(match kind {
            "unit" => vec![self.unit(id)?],
            "room" => {
                self.rooms.iter().find(|r| r.id == id)?;
                pick(&|u| u.room.as_deref() == Some(id))
            }
            "zone" => {
                let z = self.zones.iter().find(|z| z.id == id)?;
                pick(&|u| u.room.as_ref().is_some_and(|r| z.rooms.contains(r)))
            }
            "group" => {
                let g = self.groups.iter().find(|g| g.id == id)?;
                pick(&|u| g.units.contains(&u.id))
            }
            _ => return None,
        })
    }

    /// Check the whole model: what the hub keeps is always valid.
    pub fn validate(&self) -> Result<(), Invalid> {
        if self.version != MODEL_VERSION {
            return bad(format!(
                "version {} (this hub speaks {MODEL_VERSION})",
                self.version
            ));
        }
        if !matches!(self.rate_hz, 44_100 | 48_000 | 96_000) {
            return bad("rateHz: 44100, 48000 or 96000");
        }
        latency_ok("lipSyncUs", i64::from(self.lip_sync_us))?;
        if !(MIN_APPLY_LEAD..=MAX_APPLY_LEAD).contains(&self.apply_lead_frames) {
            return bad(format!(
                "applyLeadFrames: {MIN_APPLY_LEAD}..={MAX_APPLY_LEAD}"
            ));
        }
        count_ok("rooms", &self.rooms)?;
        count_ok("zones", &self.zones)?;
        count_ok("groups", &self.groups)?;
        count_ok("sources", &self.sources)?;
        count_ok("senders", &self.senders)?;
        count_ok("fanOuts", &self.fan_outs)?;
        count_ok("units", &self.units)?;
        count_ok("scenes", &self.scenes)?;

        let mut ids = BTreeSet::new();
        let all_ids = self
            .rooms
            .iter()
            .map(|x| (&x.id, &x.name))
            .chain(self.zones.iter().map(|x| (&x.id, &x.name)))
            .chain(self.groups.iter().map(|x| (&x.id, &x.name)))
            .chain(self.sources.iter().map(|x| (&x.id, &x.name)))
            .chain(self.senders.iter().map(|x| (&x.id, &x.name)))
            .chain(self.fan_outs.iter().map(|x| (&x.id, &x.name)))
            .chain(self.units.iter().map(|x| (&x.id, &x.name)))
            .chain(self.scenes.iter().map(|x| (&x.id, &x.name)));
        for (id, name) in all_ids {
            if id.is_empty() || id.contains(':') || id == "all" {
                return bad(format!("id {id:?}: not empty, no ':', not \"all\""));
            }
            text_ok("an id", id)?;
            text_ok("a name", name)?;
            if !ids.insert(id.as_str()) {
                return bad(format!("id {id:?} is used twice"));
            }
        }
        let kinds = self.kinds();
        let is = |id: &str, k: Kind| kinds.get(id) == Some(&k);

        for r in &self.rooms {
            if let Some(us) = r.lip_sync_us {
                latency_ok("a room's lipSyncUs", i64::from(us))?;
            }
        }
        for z in &self.zones {
            if let Some(r) = z.rooms.iter().find(|r| !is(r, Kind::Room)) {
                return bad(format!("zone {}: no room {r:?}", z.id));
            }
            if let Some(s) = z.source.as_deref().filter(|s| !is(s, Kind::Source)) {
                return bad(format!("zone {}: no source {s:?}", z.id));
            }
        }
        for g in &self.groups {
            if g.note.chars().count() > 1024 {
                return bad(format!("group {}: a note of at most 1024 characters", g.id));
            }
            if let Some(u) = g.units.iter().find(|u| !is(u, Kind::Unit)) {
                return bad(format!("group {}: no unit {u:?}", g.id));
            }
        }
        for s in &self.sources {
            if let Some(x) = s.sender.as_deref().filter(|x| !is(x, Kind::Sender)) {
                return bad(format!("source {}: no sender {x:?}", s.id));
            }
            if s.channels.len() > usize::from(ALINK_MAX_CHANNELS)
                || s.channels.iter().any(|c| *c >= ALINK_MAX_CHANNELS)
            {
                return bad(format!(
                    "source {}: channels below {ALINK_MAX_CHANNELS}",
                    s.id
                ));
            }
        }
        let mut serials = BTreeSet::new();
        let serial_list = self
            .senders
            .iter()
            .filter_map(|s| s.serial.as_ref())
            .chain(self.fan_outs.iter().filter_map(|f| f.serial.as_ref()))
            .chain(self.units.iter().map(|u| &u.serial));
        for serial in serial_list {
            if serial.is_empty() {
                return bad("a serial is empty");
            }
            text_ok("a serial", serial)?;
            if !serials.insert(serial.as_str()) {
                return bad(format!("serial {serial:?} is used twice"));
            }
        }
        let feeds = |up: &str| {
            is(up, Kind::Sender)
                || is(up, Kind::FanOut)
                || self.unit(up).is_some_and(|u| u.repeater && u.on_alink())
        };
        for f in &self.fan_outs {
            if !feeds(&f.upstream) {
                return bad(format!(
                    "fan-out {}: upstream {:?} is not a sender, fan-out or repeater",
                    f.id, f.upstream
                ));
            }
            if f.hop_delay_frames == Some(0) {
                return bad(format!("fan-out {}: a hop delay is at least 1 frame", f.id));
            }
        }
        for u in &self.units {
            // -- WP AUSB1 -- a USB-direct unit has no feed on the audio link
            if u.usb_direct() {
                if !u.upstream.is_empty()
                    || u.repeater
                    || u.slots.is_some()
                    || u.hop_delay_frames.is_some()
                {
                    return bad(format!(
                        "unit {}: a USB-direct unit has no upstream, slots or hop delay and repeats nothing",
                        u.id
                    ));
                }
            } else if u.spdif() {
                // -- WP ATHUBM --
                self.validate_spdif_unit(u)?;
            } else if u.usb_mode.is_some() || !u.on_lock_error.is_mute() {
                return bad(format!(
                    "unit {}: a USB mode or lock-error action is for a USB-direct unit",
                    u.id
                ));
            } else if !feeds(&u.upstream) {
                return bad(format!(
                    "unit {}: upstream {:?} is not a sender, fan-out or repeater",
                    u.id, u.upstream
                ));
            }
            if let Some(m) = u.usb_mode.as_deref().filter(|m| !USB_MODES.contains(m)) {
                return bad(format!(
                    "unit {}: USB mode {m:?} (off, rate_lock or phase_lock)",
                    u.id
                ));
            }
            if let Some(r) = u.room.as_deref().filter(|r| !is(r, Kind::Room)) {
                return bad(format!("unit {}: no room {r:?}", u.id));
            }
            if u.hop_delay_frames == Some(0) {
                return bad(format!("unit {}: a hop delay is at least 1 frame", u.id));
            }
            if let Some(slots) = &u.slots {
                let ok = slots.len() == 8
                    && slots
                        .iter()
                        .all(|s| *s < ALINK_MAX_CHANNELS || *s == ALINK_SLOT_SILENT);
                if !ok {
                    return bad(format!(
                        "unit {}: 8 slots, each a channel below {ALINK_MAX_CHANNELS} or 255",
                        u.id
                    ));
                }
            }
            latency_ok("latencyUs", i64::from(u.latency_us))?;
            if let Some(m) = u.measured_latency_us {
                latency_ok("measuredLatencyUs", i64::from(m))?;
            }
            if u.speakers.len() > 32 {
                return bad(format!("unit {}: at most 32 speakers", u.id));
            }
            let mut outs = BTreeSet::new();
            for s in &u.speakers {
                if !(0.0..=MAX_DISTANCE_M).contains(&s.distance_m) {
                    return bad(format!(
                        "unit {}: a distance of 0..={MAX_DISTANCE_M} m",
                        u.id
                    ));
                }
                if s.output >= 32 || !outs.insert(s.output) {
                    return bad(format!(
                        "unit {}: output {} below 32, once each",
                        u.id, s.output
                    ));
                }
                // -- WP ATHUBM --
                speaker_measurement_ok(&u.id, s)?;
            }
            // -- WP ATHUBM --
            self.validate_spdif_source(u)?;
        }
        for id in self
            .fan_outs
            .iter()
            .map(|f| &f.id)
            .chain(self.alink_units().map(|u| &u.id))
        {
            if self.path_to(id).is_none() {
                return bad(format!(
                    "{id}: its feed does not lead to a sender (a loop?)"
                ));
            }
        }
        for s in &self.scenes {
            if s.actions.len() > MAX_SCENE_ACTIONS {
                return bad(format!(
                    "scene {}: at most {MAX_SCENE_ACTIONS} actions",
                    s.id
                ));
            }
            let mut per_unit: BTreeMap<&str, usize> = BTreeMap::new();
            for a in &s.actions {
                let Some(op) = scene_op(&a.op) else {
                    return bad(format!("scene {}: op {:?}", s.id, a.op));
                };
                let req = ApplyRequest {
                    frame: 0,
                    op,
                    arg: a.arg,
                    value: a.value,
                };
                if !req.args_valid() {
                    return bad(format!(
                        "scene {}: {} with arg {} and value {} (extension_channel_spec.md 2.8.7)",
                        s.id, a.op, a.arg, a.value
                    ));
                }
                let Some(units) = self.resolve_target(&a.target) else {
                    return bad(format!("scene {}: target {:?}", s.id, a.target));
                };
                for u in units {
                    let n = per_unit.entry(&u.id).or_default();
                    *n = n.saturating_add(1);
                    if *n > MAX_OPS_PER_UNIT {
                        return bad(format!(
                            "scene {}: unit {} gets more than {MAX_OPS_PER_UNIT} ops (its queue)",
                            s.id, u.id
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

// -- WP ATHUBM --
fn speaker_measurement_ok(unit: &str, s: &Speaker) -> Result<(), Invalid> {
    if let Some(a) = s.measured_arrival_us
        && !(a.is_finite() && f64::from(a.abs()) <= f64::from(MAX_LATENCY_US))
    {
        return bad(format!(
            "unit {unit}: output {}'s measured arrival within {MAX_LATENCY_US} µs",
            s.output
        ));
    }
    if s.measured_runs.len() > MAX_MEASURED_RUNS {
        return bad(format!(
            "unit {unit}: output {} names at most {MAX_MEASURED_RUNS} runs",
            s.output
        ));
    }
    if s.measured_arrival_us.is_none() && (!s.measured_runs.is_empty() || s.measured_at.is_some()) {
        return bad(format!(
            "unit {unit}: output {} has runs or a time but no measured arrival",
            s.output
        ));
    }
    Ok(())
}

// -- WP ATHUBM --
impl SystemModel {
    /// The S/PDIF source with this id.
    pub fn spdif_source(&self, id: &str) -> Option<&Unit> {
        self.unit(id).filter(|u| u.spdif_source)
    }

    /// The S/PDIF followers of source `id`, in model order.
    pub fn spdif_followers<'a>(&'a self, id: &'a str) -> impl Iterator<Item = &'a Unit> {
        self.units
            .iter()
            .filter(move |u| u.spdif_follower() && u.upstream == id)
    }

    /// A unit with `"transport": "spdif"`: a follower names a source as its
    /// `upstream` (one role per unit, so no follower is a source and no feed
    /// loops); a source with that transport has no `upstream`. Neither has
    /// slots, a hop delay, a USB mode or repeats the audio link.
    fn validate_spdif_unit(&self, u: &Unit) -> Result<(), Invalid> {
        if u.repeater || u.slots.is_some() || u.hop_delay_frames.is_some() || u.usb_mode.is_some() {
            return bad(format!(
                "unit {}: an S/PDIF unit has no slots, hop delay or USB mode and repeats nothing",
                u.id
            ));
        }
        if u.spdif_source {
            if !u.upstream.is_empty() {
                return bad(format!(
                    "unit {}: one S/PDIF role per unit: a source follows no upstream (a relay is not supported)",
                    u.id
                ));
            }
            return Ok(());
        }
        if u.upstream == u.id {
            return bad(format!("unit {}: it cannot follow itself (a loop)", u.id));
        }
        let Some(src) = self.unit(&u.upstream) else {
            return bad(format!(
                "unit {}: upstream {:?} is not a unit (an S/PDIF follower follows a source unit)",
                u.id, u.upstream
            ));
        };
        if !src.spdif_source {
            return bad(format!(
                "unit {}: upstream {:?} is not an S/PDIF source{}",
                u.id,
                u.upstream,
                if src.spdif_follower() && src.upstream == u.id {
                    " (the two follow each other: a loop)"
                } else {
                    ""
                }
            ));
        }
        Ok(())
    }

    /// Any unit's S/PDIF source fields: only on a source, which names its
    /// time-base slots; none of those outputs is a speaker (a measured
    /// output would feed the stimulus to every follower as programme, S-AT6).
    fn validate_spdif_source(&self, u: &Unit) -> Result<(), Invalid> {
        if !u.spdif_source {
            if u.spdif_tx_mask != 0 || u.spdif_d.is_some() {
                return bad(format!(
                    "unit {}: spdifTxMask and spdifD are for an S/PDIF source",
                    u.id
                ));
            }
            return Ok(());
        }
        if u.spdif_tx_mask == 0 {
            return bad(format!(
                "unit {}: an S/PDIF source names the output slots that carry its time base (spdifTxMask, not 0)",
                u.id
            ));
        }
        let tx = u.spdif_tx_outputs();
        if let Some(s) = u.speakers.iter().find(|s| tx.contains(&s.output)) {
            return bad(format!(
                "unit {}: output {} is on S/PDIF slot {}, which carries the time base: it is never a speaker or a measured output",
                u.id,
                s.output,
                s.output / 2
            ));
        }
        Ok(())
    }
}

// -- WP ATHUBM --
#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests_wp_athubm {
    use super::*;
    use serde_json::json;

    fn model(units: serde_json::Value) -> Result<SystemModel, String> {
        let m: SystemModel = serde_json::from_value(json!({
            "rooms": [{"id": "main"}],
            "senders": [{"id": "tx", "serial": "S-tx"}],
            "units": units,
        }))
        .map_err(|e| e.to_string())?;
        m.validate().map_err(|e| e.0)?;
        Ok(m)
    }

    fn src() -> serde_json::Value {
        json!({"id": "SRC", "serial": "S-SRC", "room": "main", "transport": "spdif",
               "spdifSource": true, "spdifTxMask": 1,
               "speakers": [{"output": 2, "distanceM": 2.0}]})
    }

    fn follower(id: &str, up: &str) -> serde_json::Value {
        json!({"id": id, "serial": format!("S-{id}"), "room": "main", "transport": "spdif",
               "upstream": up, "onLockError": "play_unaligned",
               "speakers": [{"output": 0, "distanceM": 3.0}]})
    }

    /// A source fed by nothing the model holds with two followers; a source
    /// on the audio link with one: taken, and kept as written.
    #[test]
    fn wp_athubm_a_source_with_followers_is_taken() {
        let m = model(json!([src(), follower("F1", "SRC"), follower("F2", "SRC")])).unwrap();
        assert!(m.units[0].spdif_source && m.units[0].spdif());
        assert!(m.units[1].spdif_follower() && !m.units[1].on_alink());
        assert_eq!(m.units[0].spdif_tx_outputs(), vec![0, 1]);
        assert_eq!(m.spdif_followers("SRC").count(), 2);
        assert_eq!(m.alink_units().count(), 0);
        let back: SystemModel = serde_json::from_value(serde_json::to_value(&m).unwrap()).unwrap();
        assert_eq!(back, m);

        let on_line = json!({"id": "A", "serial": "S-A", "room": "main", "upstream": "tx",
                             "spdifSource": true, "spdifTxMask": 2, "spdifD": 200});
        let m = model(json!([on_line, follower("F1", "A")])).unwrap();
        assert_eq!(m.alink_units().count(), 1);
        assert_eq!(m.path_to("F1"), None, "a follower has no audio link path");
        assert_eq!(m.units[0].spdif_tx_outputs(), vec![2, 3]);
        // A USB-direct unit may be a source too.
        let usb = json!({"id": "U", "serial": "S-U", "transport": "usb_direct",
                         "usbMode": "phase_lock", "spdifSource": true, "spdifTxMask": 1});
        model(json!([usb, follower("F1", "U")])).unwrap();
    }

    #[test]
    fn wp_athubm_a_follower_names_a_source_without_loops() {
        let plain = json!({"id": "A", "serial": "S-A", "upstream": "tx"});
        let e = model(json!([plain, follower("F1", "A")])).unwrap_err();
        assert!(e.contains("not an S/PDIF source"), "{e}");
        let e = model(json!([src(), follower("F1", "nobody")])).unwrap_err();
        assert!(e.contains("is not a unit"), "{e}");
        let e = model(json!([src(), follower("F1", "F1")])).unwrap_err();
        assert!(e.contains("itself"), "{e}");
        // Two followers of each other: a loop, said as one.
        let e = model(json!([follower("F1", "F2"), follower("F2", "F1")])).unwrap_err();
        assert!(e.contains("a loop"), "{e}");
        // A follower of a follower.
        let e = model(json!([src(), follower("F1", "SRC"), follower("F2", "F1")])).unwrap_err();
        assert!(e.contains("not an S/PDIF source"), "{e}");
        // One role per unit: a source that follows (a relay) is refused.
        let mut relay = src();
        relay["upstream"] = json!("SRC2");
        let mut other = src();
        other["id"] = json!("SRC2");
        other["serial"] = json!("S-SRC2");
        let e = model(json!([relay, other])).unwrap_err();
        assert!(e.contains("one S/PDIF role"), "{e}");
        // An S/PDIF unit has no audio link slots or hop delay.
        let mut f = follower("F1", "SRC");
        f["slots"] = json!([0, 1, 2, 3, 4, 5, 6, 7]);
        assert!(model(json!([src(), f])).is_err());
        // An audio link unit cannot have the S/PDIF lock-error action.
        let mut a = json!({"id": "A", "serial": "S-A", "upstream": "tx"});
        a["onLockError"] = json!("play_unaligned");
        assert!(model(json!([a])).is_err());
    }

    /// S-AT6: the source names its time-base slots, and none of their
    /// outputs is a speaker (a measured output).
    #[test]
    fn wp_athubm_a_source_slot_is_never_a_speaker() {
        let mut s = src();
        s["speakers"] = json!([{"output": 1, "distanceM": 2.0}]);
        let e = model(json!([s])).unwrap_err();
        assert!(e.contains("slot 0, which carries the time base"), "{e}");
        let mut s = src();
        s["spdifTxMask"] = json!(0);
        let e = model(json!([s])).unwrap_err();
        assert!(e.contains("names the output slots"), "{e}");
        let f = json!({"id": "F", "serial": "S-F", "upstream": "tx", "spdifTxMask": 1});
        let e = model(json!([f])).unwrap_err();
        assert!(e.contains("for an S/PDIF source"), "{e}");
        let f = json!({"id": "F", "serial": "S-F", "upstream": "tx", "spdifD": 150});
        assert!(model(json!([f])).is_err());
    }

    #[test]
    fn wp_athubm_measured_arrivals_are_checked() {
        let speaker = |extra: serde_json::Value| {
            let mut s = json!({"output": 0, "distanceM": 3.0});
            for (k, v) in extra.as_object().unwrap() {
                s[k] = v.clone();
            }
            json!([{"id": "A", "serial": "S-A", "upstream": "tx", "speakers": [s]}])
        };
        let m = model(speaker(
            json!({"measuredArrivalUs": -250.5, "measuredRuns": [7, 8],
                                     "measuredAt": 1_763_500_000u64}),
        ))
        .unwrap();
        let s = &m.units[0].speakers[0];
        assert_eq!(s.measured_arrival_us, Some(-250.5));
        assert_eq!(s.measured_runs, vec![7, 8]);
        assert!(model(speaker(json!({"measuredArrivalUs": 2.0e6}))).is_err());
        assert!(model(speaker(json!({"measuredRuns": [1]}))).is_err());
        let runs: Vec<u32> = (0..17).collect();
        assert!(
            model(speaker(
                json!({"measuredArrivalUs": 0.0, "measuredRuns": runs})
            ))
            .is_err()
        );
        // NaN cannot come through JSON; checked anyway.
        let mut m = model(speaker(json!({"measuredArrivalUs": 0.0}))).unwrap();
        m.units[0].speakers[0].measured_arrival_us = Some(f32::NAN);
        assert!(m.validate().is_err());
    }

    /// A model of before this WP loads unchanged and writes back the same JSON
    /// (no new key appears).
    #[test]
    fn wp_athubm_an_existing_model_json_loads_unchanged() {
        let old = json!({
            "version": 1, "dFrames": null, "rateHz": 48000, "lipSyncUs": 0,
            "applyLeadFrames": 2400,
            "rooms": [{"id": "main", "name": "Main", "lipSyncUs": null}],
            "zones": [], "groups": [], "sources": [],
            "senders": [{"id": "tx", "name": "", "serial": "S-tx", "senderId": 66}],
            "fanOuts": [],
            "units": [
                {"id": "A", "name": "", "serial": "S-A", "room": "main", "upstream": "tx",
                 "output": null, "repeater": false, "hopDelayFrames": null, "slots": null,
                 "latencyUs": 0, "measuredLatencyUs": null,
                 "speakers": [{"output": 0, "distanceM": 3.0}]},
                {"id": "U", "name": "", "serial": "S-U", "room": "main", "transport": "usb_direct",
                 "usbMode": "phase_lock", "output": null, "repeater": false,
                 "hopDelayFrames": null, "slots": null, "latencyUs": 0,
                 "measuredLatencyUs": null, "speakers": []}
            ],
            "scenes": []
        });
        let m: SystemModel = serde_json::from_value(old.clone()).unwrap();
        m.validate().unwrap();
        assert_eq!(serde_json::to_value(&m).unwrap(), old);
    }
}

// -- WP INST0 --
#[cfg(test)]
mod tests_wp_inst0 {
    use super::*;

    /// Every refusal that is about one element names it; the rest are about
    /// the model as a whole.
    #[test]
    fn wp_inst0_a_refusal_names_its_element() {
        let subject = |m: &str| Invalid(m.into()).subject();
        for (msg, want) in [
            (
                "unit A: upstream \"x\" is not a sender, fan-out or repeater",
                Some("A"),
            ),
            ("unit Main left: at most 32 speakers", Some("Main left")),
            ("zone z: no room \"r\"", Some("z")),
            ("group g: no unit \"u\"", Some("g")),
            ("source s: channels below 64", Some("s")),
            ("fan-out f: a hop delay is at least 1 frame", Some("f")),
            ("scene evening: op \"x\"", Some("evening")),
            ("id \"tx\" is used twice", Some("tx")),
            ("id \"a:b\": not empty, no ':', not \"all\"", Some("a:b")),
            ("B: its feed does not lead to a sender (a loop?)", Some("B")),
            ("rateHz: 44100, 48000 or 96000", None),
            ("serial \"S\" is used twice", None),
            (
                "a name: at most 64 characters, none of them control characters",
                None,
            ),
        ] {
            assert_eq!(subject(msg).as_deref(), want, "{msg}");
        }
        // And from the validator itself.
        let m: SystemModel = serde_json::from_value(serde_json::json!({
            "senders": [{"id": "tx"}],
            "units": [{"id": "A", "serial": "S-A", "upstream": "A", "repeater": true}]
        }))
        .unwrap();
        assert_eq!(m.validate().unwrap_err().subject().as_deref(), Some("A"));
    }
}
