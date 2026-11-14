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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Speaker {
    /// The unit's output, from 0.
    pub output: u8,
    /// From the speaker to the listening position, metres.
    pub distance_m: f32,
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

// -- fixup sw2x (roles) --
/// The direct command a scene op stands for, as `(bRequest, dir)`: a caller may
/// recall the op only if the hub's policy lets its role send that command.
/// Mute and unmute are `SET_OUTPUT_MUTE`, a preset load `PRESET_LOAD`, the
/// master volume and output delay their setters; a click and a sync pulse
/// have no direct command but the alink's extension write (admin).
pub fn op_authority(op: ApplyOp) -> (u8, dspi_link::Dir) {
    use dspi_link::Dir;
    use dspi_proto::generated::opcodes as opc;
    match op {
        ApplyOp::Mute | ApplyOp::Unmute => (opc::REQ_SET_OUTPUT_MUTE, Dir::Set),
        ApplyOp::PresetLoad => (opc::REQ_PRESET_LOAD, Dir::Get),
        ApplyOp::MasterVolume => (opc::REQ_SET_MASTER_VOLUME, Dir::Set),
        ApplyOp::OutDelay => (opc::REQ_SET_OUTPUT_DELAY, Dir::Set),
        _ => (opc::REQ_EXTENSION, Dir::Set),
    }
}

/// The direct command the system mute stands for: muting every unit's outputs,
/// `SET_OUTPUT_MUTE`.
pub fn system_mute_authority() -> (u8, dspi_link::Dir) {
    op_authority(ApplyOp::Mute)
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
                    .filter(|u| !u.usb_direct())
                    .map(|u| u.upstream.as_str())
            })
    }

    // -- WP AUSB1 --
    /// The units on the audio link (not USB-direct), in model order.
    pub fn alink_units(&self) -> impl Iterator<Item = &Unit> {
        self.units.iter().filter(|u| !u.usb_direct())
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

    /// Check the whole model: what [`crate::system`] keeps is always valid.
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
                || self.unit(up).is_some_and(|u| u.repeater && !u.usb_direct())
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
            }
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

#[cfg(test)]
mod tests_wp_sw2x {
    use super::*;
    use crate::policy::Policy;
    use crate::session::Role;

    /// The coordinator's role rule, on the hub's own policy table: the ops a
    /// console's control role sends directly are control; a click and a sync
    /// pulse have only the alink's extension write, which is admin.
    #[test]
    fn wp_sw2x_ops_are_judged_as_their_direct_commands() {
        let p = Policy::embedded();
        let may = |role, op| {
            let (c, d) = op_authority(op);
            p.allows(role, c, d)
        };
        for op in [
            ApplyOp::Mute,
            ApplyOp::Unmute,
            ApplyOp::PresetLoad,
            ApplyOp::MasterVolume,
            ApplyOp::OutDelay,
        ] {
            assert!(may(Role::Control, op), "{op:?}");
            assert!(!may(Role::Viewer, op), "{op:?}");
        }
        for op in [ApplyOp::Click, ApplyOp::SyncPulse] {
            assert!(!may(Role::Control, op) && may(Role::Admin, op), "{op:?}");
        }
        let (c, d) = system_mute_authority();
        assert!(p.allows(Role::Control, c, d) && !p.allows(Role::Viewer, c, d));
    }
}
