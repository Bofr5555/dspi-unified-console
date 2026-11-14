//! WP LKUI1: the audio link's installation on the hub, as the native System
//! screens show it (Qt, Android; Swift later). The hub's `system.*` requests
//! (`docs/integrator/audio-link.md`) over this session's own Link connection,
//! turned into rows a client draws as they are: units with their role and
//! alink state, the audio link delay and its range, the hop offset, the phase error,
//! the scenes, and what a recall did.
//!
//! The web console reads the same requests through its own generic call
//! ([`DspiCore::link_hub_request`], not exported); these are the typed calls
//! for exactly what the native screens need:
//! - [`DspiCore::alink_system_view`] reads `system.status`, plus `system.get`
//!   when the model's revision moved;
//! - [`DspiCore::alink_system_save`] sends the "Clock and audio links" edits as one
//!   `system.set`, guarded by the revision they were made on;
//! - [`DspiCore::alink_system_align`] sends `system.align` with `apply`;
//! - [`DspiCore::alink_system_recall`] sends `system.scene.apply`.
//!
//! The hub judges every request by this session's role; the view's `may_*`
//! flags only keep a client from offering what it would refuse. No clock is
//! read here: the words count milliseconds from the recall, never a time of
//! day.

use serde_json::{Map, Value};

use dspi_link::LinkClient;
use dspi_link::LinkError;

use super::{HUB_REQUEST_TIMEOUT, map_error};
use crate::{DspiCore, DspiError, Result};

/// The rate a model gives none for (48 kHz, the alink's usual one).
const DEFAULT_RATE_HZ: u32 = 48_000;

/// What the core keeps between calls on one Link session: the model as last
/// read and the view built from it. A new session starts empty.
#[derive(Debug, Default)]
pub(crate) struct Cache {
    /// The model and its revision, as `system.get` gave them.
    model: Option<(u64, Value)>,
    /// The last view built, given again while `system.status` is rate limited.
    view: Option<AlinkSystemView>,
    /// The frame of this session's own last scene taken through the sender.
    last_superframe: Option<u64>,
}

/// One element of the installation: the sender, a fan-out or a unit.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlinkSystemUnit {
    /// The model's id.
    pub id: String,
    /// Its name, or the id when it has none.
    pub name: String,
    /// The device's serial; "" when the model has none.
    pub serial: String,
    /// `sender`, `fan_out` or `unit`.
    pub kind: String,
    /// The role on the audio link in the glossary's words: "Sender",
    /// "Fan-out", "Repeater", "Receiver" (what the device says, else the model),
    /// or "No audio link" for a device whose firmware has none (WP HWLK1F).
    pub role: String,
    /// The room's name; "" for none.
    pub room: String,
    /// Attached to the hub now.
    pub connected: bool,
    /// `locked`, `searching`, `holdover`, `off`, `clock` (a sender),
    /// `offline` (not attached), `no_alink` (attached, its firmware has no
    /// audio link: WP HWLK1F) or `unknown`.
    pub state: String,
    /// [`Self::state`] in words: "Locked", "Holdover", ... "Not connected".
    pub state_word: String,
    /// The unit's audio link delay or hop offset is not the one the system delay
    /// asks for: writing the audio link delays ([`DspiCore::alink_system_align`])
    /// is pending.
    pub pending: bool,
    /// The audio link delay the alignment asks for, frames; else the unit's own.
    pub alink_delay_frames: Option<u32>,
    /// [`Self::alink_delay_frames`] in ms at the system's rate.
    pub alink_delay_ms: Option<f64>,
    /// The audio link delay the unit has now, frames, when it reported one.
    pub unit_alink_delay_frames: Option<u32>,
    /// The shortest audio link delay the unit takes, frames.
    pub alink_delay_min: Option<u32>,
    /// The longest.
    pub alink_delay_max: Option<u32>,
    /// The hop offset the alignment asks for, frames; else the unit's own.
    pub hop_offset: Option<i32>,
    /// The hop offset the unit has now.
    pub unit_hop_offset: Option<i32>,
    /// The phase error, frames (phase lock).
    pub phase_error_frames: Option<f32>,
    /// Fan-outs and repeaters between the sender and this unit.
    pub hops: Option<u32>,
    /// Frames a fan-out or repeater adds passing the stream on.
    pub hop_delay_frames: Option<u32>,
    /// The output latency the model counts, µs.
    pub latency_us: i64,
    /// The measured latency an installer entered, µs; it replaces
    /// [`Self::latency_us`] when set.
    pub measured_latency_us: Option<i64>,
    /// Its alarms still on.
    pub alarms: u32,
    // -- WP AUSB1 -- USB-direct units (`extension_channel_spec.md` 2.9)
    /// How the programme reaches it: `alink` (senders and fan-outs too) or
    /// `usb_direct`.
    pub transport: String,
    /// A USB-direct unit's mode as the unit runs it: `off`, `rate_lock`,
    /// `phase_lock`.
    pub usb_mode: Option<String>,
    /// The mode the installation asks for, when it names one.
    pub usb_mode_wanted: Option<String>,
    /// The delay D the alignment asks for, samples; else the unit's own.
    pub usb_delay_samples: Option<u32>,
    /// [`Self::usb_delay_samples`] in ms at the system's rate.
    pub usb_delay_ms: Option<f64>,
    /// The D the unit has now, samples.
    pub unit_usb_delay_samples: Option<u32>,
    /// The shortest D phase lock takes now, samples.
    pub usb_delay_min: Option<u32>,
    /// The longest.
    pub usb_delay_max: Option<u32>,
    /// The host's SOF rate against the unit's crystal, ppm.
    pub host_ppm: Option<f32>,
    /// The unit's soft VCXO correction, ppm.
    pub vcxo_ppm: Option<f32>,
    /// The phase error, samples.
    pub phase_error_samples: Option<f32>,
    /// Faded gaps since boot.
    pub gaps: Option<u32>,
    /// Silence inserted for missing packets, samples since boot.
    pub missed_samples: Option<u32>,
    /// Samples dropped as too late, since boot.
    pub late_samples: Option<u32>,
    /// Re-anchors since boot.
    pub reanchors: Option<u32>,
    /// Timeline ends counted by the unit's diagnostics, and the latest cause.
    pub timeline_ends: Option<u32>,
    /// The latest cause in words ("SOF lost", "bus reset"...); "" for none.
    pub last_end: String,
    /// How many samples earlier its stream plays than the latest unit's (the
    /// alignment delays its outputs by that).
    pub start_offset_samples: Option<i64>,
    /// The statistics in one line, for a details row: host and VCXO ppm,
    /// phase error, gaps, missed and late samples, re-anchors, timeline ends;
    /// -- WP AUSB2 -- lock errors, repaired SOF readings, the longest ride.
    pub usb_stats: String,
    // -- WP AUSB2 -- lock error (the fork's WP USB3b)
    /// The unit is in lock error: asked to phase-lock, it cannot be in step
    /// (its host out of range, a rate or D that cannot phase-lock, a phase
    /// error pending). [`Self::state_word`] then says what it does.
    pub usb_lock_error: bool,
    /// Why, in words, while it is ("the host's rate is out of range"); "".
    pub usb_lock_error_cause: String,
    /// What the unit does in lock error now: `mute` or `play_unaligned`;
    /// none when not read.
    pub usb_on_lock_error: Option<String>,
    /// What the installation asks it to do (the hub writes it): `mute` (the
    /// default) or `play_unaligned`; "" for a unit that is not USB-direct.
    pub usb_on_lock_error_wanted: String,
    /// The unit refused "play unaligned" (firmware before the fork's WP
    /// USB3b): it mutes in lock error whatever is asked.
    pub usb_play_unaligned_unsupported: bool,
}

/// A room, with its lip-sync offset.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlinkSystemRoom {
    /// The model's id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its own lip-sync offset, µs; none: the model's.
    pub lip_sync_us: Option<i64>,
}

/// A scene a recall can apply.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlinkSystemScene {
    /// The model's id: what [`DspiCore::alink_system_recall`] takes.
    pub id: String,
    /// Its name.
    pub name: String,
    /// The scene sets the system mute (true) or clears it (false); none:
    /// leaves it.
    pub system_mute: Option<bool>,
}

/// An alarm still on (raised or acknowledged).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlinkSystemAlarm {
    /// The hub's id for it.
    pub id: u64,
    /// `warning` or `critical`.
    pub severity: String,
    /// The element's name it is about.
    pub subject: String,
    /// The hub's sentence.
    pub message: String,
    /// `raised` or `acknowledged`.
    pub state: String,
}

/// The installation on the hub, as the System screen shows it.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlinkSystemView {
    /// The model's revision: what [`DspiCore::alink_system_save`] is given back.
    pub revision: u64,
    /// The hub has an installation (a model with at least one element).
    pub installed: bool,
    /// This session's role on the hub now: `admin`, `control` or `viewer`.
    pub role: String,
    /// The role may change the installation and write the audio link delays (admin).
    pub may_edit: bool,
    /// The role may recall scenes (control or admin).
    pub may_recall: bool,
    /// Why editing is not offered, or "".
    pub edit_note: String,
    /// Why recalling is not offered, or "".
    pub recall_note: String,
    /// The system delay the alignment uses now, frames.
    pub system_delay_frames: u32,
    /// [`Self::system_delay_frames`] in ms.
    pub system_delay_ms: f64,
    /// The model fixes no system delay: the hub takes the smallest every
    /// unit takes.
    pub system_delay_auto: bool,
    /// The alink's rate, Hz.
    pub rate_hz: u32,
    /// The model's lip-sync offset, µs.
    pub lip_sync_us: i64,
    /// Every element: senders, fan-outs, then units in the model's order.
    pub units: Vec<AlinkSystemUnit>,
    /// The rooms.
    pub rooms: Vec<AlinkSystemRoom>,
    /// The scenes.
    pub scenes: Vec<AlinkSystemScene>,
    /// Alarms still on, critical first.
    pub alarms: Vec<AlinkSystemAlarm>,
    /// A sender reports its system mute set (none: no sender reported).
    pub system_mute: Option<bool>,
    /// The frame a sender holds a scene for, when it holds one.
    pub pending_apply_frame: Option<u64>,
    /// Units (not senders or fan-outs) locked.
    pub locked: u32,
    /// Units in holdover.
    pub holdover: u32,
    /// Units whose audio link delay is still to be written.
    pub pending: u32,
    /// Units the model has, but those whose firmware has no audio link
    /// (WP HWLK1F): "N of M locked" counts the units that can lock.
    /// -- WP AUSB1 -- USB-direct units are counted apart.
    pub unit_count: u32,
    /// USB-direct units in the model (WP AUSB1).
    pub usb_unit_count: u32,
    /// Those locked as asked: phase-locked (rate-locked when the mode asks
    /// only that).
    pub usb_locked: u32,
    /// One line for a status indicator: "Audio link: 4 of 4 locked".
    pub summary: String,
    /// The summary's meaning for its colour: `ok`, `warning`, `problem`, or
    /// `none` without an installation.
    pub health: String,
    /// The alignment's issues, in the hub's words.
    pub issues: Vec<String>,
    // -- WP FIXM1 --
    /// "Keep USB-direct units aligned" (Installer): the hub writes the
    /// USB-direct units' output delays again by itself when their start
    /// offsets move. Off: it says "USB-direct start offsets changed — Align
    /// again" among the issues, and `align_needed` is set.
    pub keep_usb_aligned: bool,
    /// The hub says an alignment is due: USB-direct start offsets moved
    /// since it last wrote them (one-click Align, Installer).
    pub align_needed: bool,
    /// The hub limits how often the status is read; this is the last view
    /// again, not a fresh one.
    pub stale: bool,
}

/// One room's lip-sync offset to set.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AlinkRoomLipSync {
    /// The room's id.
    pub room_id: String,
    /// µs; none: the model's own.
    pub lip_sync_us: Option<i64>,
}

/// One unit's measured latency to set.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AlinkUnitLatency {
    /// The unit's id.
    pub unit_id: String,
    /// µs; none: the output latency counts.
    pub measured_latency_us: Option<i64>,
}

/// The "Clock and audio links" edits, sent as one change of the model.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AlinkSystemEdits {
    /// Change the system delay.
    pub set_system_delay: bool,
    /// The new system delay, frames; none: the smallest every unit takes.
    pub system_delay_frames: Option<u32>,
    /// Rooms' lip-sync offsets.
    pub rooms: Vec<AlinkRoomLipSync>,
    /// Units' measured latencies.
    pub units: Vec<AlinkUnitLatency>,
    // -- WP AUSB1 --
    /// USB-direct units' modes (Installer).
    pub usb_modes: Vec<AlinkUnitUsbMode>,
    // -- WP FIXM1 --
    /// "Keep USB-direct units aligned" (Installer); none: unchanged.
    pub keep_usb_aligned: Option<bool>,
}

// -- WP AUSB1 --
/// One USB-direct unit's mode to set ("Clock and audio links", Installer):
/// written with the audio link delays.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AlinkUnitUsbMode {
    /// The unit's id.
    pub unit_id: String,
    /// `off`, `rate_lock` or `phase_lock`; none: the unit's own is kept.
    pub mode: Option<String>,
    // -- WP AUSB2 --
    /// What it does in lock error: `mute` or `play_unaligned` (the hub writes
    /// it to the unit); none: unchanged.
    pub on_lock_error: Option<String>,
}

/// One write's outcome on one device.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlinkSystemWrite {
    /// The element's name.
    pub unit: String,
    /// What was written, in words ("Mute", "Audio link delay").
    pub what: String,
    /// The hub's result code (`applied`, `queued`, `refused`, ...).
    pub result: String,
    /// [`Self::result`] in words.
    pub result_word: String,
    /// `ok`, `warn` or `bad`.
    pub tone: String,
}

/// What a recall did.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlinkSceneRecall {
    /// The scene's id.
    pub scene: String,
    /// Its name.
    pub name: String,
    /// At least one device took it. False: refused, nothing changed, and
    /// [`Self::message`] says why.
    pub taken: bool,
    /// The alink frame it applies at.
    pub frame: u64,
    /// How far ahead it was scheduled, ms.
    pub lead_ms: f64,
    /// Every device applied it on time.
    pub on_time: bool,
    /// The frame the sender holds another scene for, when that is why it
    /// was refused.
    pub held_frame: Option<u64>,
    /// One sentence to show: the outcome, or the refusal and what to do.
    pub message: String,
    /// Each device's result.
    pub writes: Vec<AlinkSystemWrite>,
}

/// What writing the audio link delays did.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlinkSystemAlign {
    /// The hub wrote them (false: an issue stopped it).
    pub applied: bool,
    /// One sentence to show.
    pub message: String,
    /// Each write's result.
    pub writes: Vec<AlinkSystemWrite>,
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// The audio link's installation on the hub this session is on: its
    /// elements with their alink state, the rooms, the scenes, alarms and a
    /// one-line summary. Reads `system.status` (and `system.get` when the
    /// model changed). The hub answers the status at most once a second per
    /// session; asked sooner, this gives the last view again with `stale`.
    /// Blocks on the network: keep it off a UI thread.
    ///
    /// Errors: [`DspiError::Disconnected`] with no hub session,
    /// [`DspiError::Link`] with the hub's code or the client's.
    pub fn alink_system_view(&self) -> Result<AlinkSystemView> {
        let client = self.hub_client()?;
        let status = match request(&client, "system.status", Map::new()) {
            Ok(s) => s,
            Err(e) if is_rate_limited(&e) => {
                return self
                    .system_cache(|c| c.view.clone())
                    .map(|mut v| {
                        v.stale = true;
                        v
                    })
                    .ok_or_else(|| map_error(e));
            }
            Err(e) => return Err(map_error(e)),
        };
        let revision = status.get("revision").and_then(Value::as_u64).unwrap_or(0);
        let cached = self.system_cache(|c| c.model.clone());
        let model = match cached {
            Some((r, m)) if r == revision => m,
            _ => {
                let got = request(&client, "system.get", Map::new()).map_err(map_error)?;
                got.get("model").cloned().unwrap_or(Value::Null)
            }
        };
        let role = status
            .get("role")
            .and_then(Value::as_str)
            .map_or_else(|| client.role(), str::to_owned);
        let view = build_view(&model, revision, &status, &role);
        self.system_cache(|c| {
            c.model = Some((revision, model));
            c.view = Some(view.clone());
        });
        Ok(view)
    }

    /// Save the "Clock and audio links" edits made on the view of `revision`, as
    /// one `system.set` the hub takes only while its model is still at that
    /// revision. Returns the new revision. Admin only.
    ///
    /// Errors: [`DspiError::Link`] `conflict` when the installation changed on
    /// the hub meanwhile (nothing is saved; read the view again and edit
    /// again), `denied` for another role, `bad_request` for values the hub
    /// refuses, with its words; [`DspiError::Disconnected`] with no session.
    pub fn alink_system_save(&self, revision: u64, edits: AlinkSystemEdits) -> Result<u64> {
        // -- WP P1QT1 -- the body is alink_system_save_report's, which keeps the warnings too
        self.alink_system_save_report(revision, edits)
            .map(|saved| saved.revision)
    }

    /// Write the audio link delays and hop offsets the system delay asks for to
    /// every unit (`system.align` with `apply`). Each unit re-anchors: a
    /// short silence (2 to 6 ms). Admin only. Blocks while the hub writes
    /// every unit.
    ///
    /// Errors: as [`Self::alink_system_save`], without `conflict`.
    pub fn alink_system_align(&self) -> Result<AlinkSystemAlign> {
        let client = self.hub_client()?;
        let mut fields = Map::new();
        fields.insert("apply".into(), true.into());
        let report = request(&client, "system.align", fields).map_err(map_error)?;
        let model = self.system_cache(|c| c.model.clone().map(|(_, m)| m));
        Ok(align_outcome(
            &report,
            model.as_ref().unwrap_or(&Value::Null),
        ))
    }

    /// Recall scene `scene` (an id from the view) on every unit it names
    /// (`system.scene.apply`). A recall the hub took but every device
    /// refused (a sender holds one scene at a time) is answered with
    /// `taken` false and the words to show, naming the frame it holds.
    /// Control or admin.
    ///
    /// Errors: [`DspiError::Link`] `denied` when the role may not make one of
    /// the scene's operations (the hub's words name it), `bad_request` for a
    /// scene the model does not have; [`DspiError::Disconnected`].
    pub fn alink_system_recall(&self, scene: String) -> Result<AlinkSceneRecall> {
        let client = self.hub_client()?;
        let mut fields = Map::new();
        fields.insert("scene".into(), Value::from(scene.as_str()));
        let report = request(&client, "system.scene.apply", fields).map_err(map_error)?;
        let (model, held, own) = self.system_cache(|c| {
            (
                c.model.clone().map(|(_, m)| m),
                c.view.as_ref().and_then(|v| v.pending_apply_frame),
                c.last_superframe,
            )
        });
        let model = model.unwrap_or(Value::Null);
        let rate = rate_of(&model, None);
        let outcome = recall_outcome(&scene, &report, &model, held.or(own), rate);
        if outcome.taken && str_at(&report, &["plan", "path"]) == "superframe" {
            self.system_cache(|c| c.last_superframe = Some(outcome.frame));
        }
        Ok(outcome)
    }
}

impl DspiCore {
    /// This session's hub connection, if it is up.
    fn hub_client(&self) -> Result<LinkClient> {
        self.lock()
            .link
            .as_ref()
            .and_then(|s| s.client.clone())
            .filter(|c| !c.is_closed())
            .ok_or(DspiError::Disconnected)
    }

    /// `f` on the session's cache, under the core lock (briefly; never across
    /// a request). A fresh cache when there is no session.
    fn system_cache<T>(&self, f: impl FnOnce(&mut Cache) -> T) -> T {
        let mut inner = self.lock();
        match inner.link.as_mut() {
            Some(s) => f(&mut s.system),
            None => f(&mut Cache::default()),
        }
    }
}

fn request(
    client: &LinkClient,
    t: &str,
    fields: Map<String, Value>,
) -> std::result::Result<Value, LinkError> {
    client.request_json(t, fields, HUB_REQUEST_TIMEOUT)
}

fn is_rate_limited(e: &LinkError) -> bool {
    match e {
        LinkError::RateLimited { .. } => true,
        LinkError::Rejected { code, .. } => code == "rate_limited",
        _ => false,
    }
}

fn conflict(revision: u64) -> DspiError {
    DspiError::Link {
        code: "conflict".into(),
        msg: format!(
            "The installation changed on the hub while it was being edited (revision {revision}). \
             Reload it and make the changes again."
        ),
    }
}

// ---------------------------------------------------------------- reading JSON

fn at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(v, |v, k| v.get(*k))
}

fn str_at<'a>(v: &'a Value, path: &[&str]) -> &'a str {
    at(v, path).and_then(Value::as_str).unwrap_or("")
}

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn u32_of(v: Option<&Value>) -> Option<u32> {
    v.and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
}

fn i32_of(v: Option<&Value>) -> Option<i32> {
    v.and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
}

/// Frames as ms at `rate_hz`.
fn frames_ms(frames: u32, rate_hz: u32) -> f64 {
    if rate_hz == 0 {
        return 0.0;
    }
    f64::from(frames) * 1000.0 / f64::from(rate_hz)
}

/// The rate: the alignment's, else the model's, else 48 kHz.
fn rate_of(model: &Value, status: Option<&Value>) -> u32 {
    status
        .and_then(|s| u32_of(at(s, &["alignment", "rateHz"])))
        .or_else(|| u32_of(model.get("rateHz")))
        .filter(|r| *r > 0)
        .unwrap_or(DEFAULT_RATE_HZ)
}

// ---------------------------------------------------------------- words

/// The alink's state in the glossary's words (3.1).
fn state_word(state: &str) -> &'static str {
    match state {
        "locked" => "Locked",
        "searching" => "Searching",
        "holdover" => "Holdover",
        "off" => "Off",
        "clock" => "Clock",
        "offline" => "Not connected",
        // -- WP HWLK1F --
        "no_alink" => "No audio link (firmware)",
        _ => "Unknown",
    }
}

/// The role on the audio link in the glossary's words.
fn role_word(role: &str) -> &'static str {
    match role {
        "sender" => "Sender",
        "fan_out" | "fanout" => "Fan-out",
        "repeater" => "Repeater",
        "receiver" | "unit" => "Receiver",
        _ => "No audio link",
    }
}

/// One write's result in words, as the web console says it.
fn result_word(result: &str) -> String {
    let w = match result {
        "ok" => "Done",
        "unchanged" => "Unchanged",
        "queued" => "Queued",
        "queued_unverified" => "Queued, not verified",
        "applied" => "Applied",
        "late" => "Late",
        "cancelled" => "Cancelled",
        "refused_at_apply" | "refused" => "Refused",
        "skipped" => "Skipped",
        "locked" => "Locked by a client",
        "denied" => "Denied",
        "timeout" => "No answer",
        "no_device" => "Not connected",
        // -- WP HWLK1F --
        "applied_now" => "Applied now",
        "unsupported" => "Needs the audio link",
        "no_frame" => "No audio link frame",
        r if r.starts_with("clamped") => "Clamped",
        r => r,
    };
    w.to_owned()
}

fn result_tone(result: &str) -> &'static str {
    match result {
        "ok" | "unchanged" | "queued" | "applied" | "applied_now" => "ok",
        "late" | "queued_unverified" | "skipped" | "cancelled" => "warn",
        r if r.starts_with("clamped") => "warn",
        _ => "bad",
    }
}

/// Results that mean a device took the op.
fn taken(result: &str) -> bool {
    matches!(
        result,
        "ok" | "unchanged" | "queued" | "queued_unverified" | "applied" | "late" | "applied_now"
    ) || result.starts_with("clamped")
}

/// What a write wrote, in the glossary's words.
fn what_word(what: &str) -> String {
    match what {
        "phase" => "Audio link delay".to_owned(),
        // -- WP AUSB1 --
        "usb_direct" => "USB-direct mode and delay".to_owned(),
        "slots" => "Audio link slots".to_owned(),
        "mute" => "Mute".to_owned(),
        "unmute" => "Unmute".to_owned(),
        "master_volume" => "Master volume".to_owned(),
        "preset_load" => "Preset".to_owned(),
        "click" => "Test click".to_owned(),
        "sync_pulse" => "Sync pulse".to_owned(),
        "superframe" | "system_mute" => "System mute".to_owned(),
        w => match w
            .strip_prefix("out_delay:")
            .and_then(|n| n.parse::<u32>().ok())
        {
            Some(n) => format!("Delay OUT{}", n.saturating_add(1)),
            None => w.to_owned(),
        },
    }
}

/// The element's name for an id or serial.
fn element_name(model: &Value, key: &str) -> String {
    ["units", "fanOuts", "senders"]
        .iter()
        .flat_map(|k| list(model, k))
        .find(|e| str_at(e, &["id"]) == key || str_at(e, &["serial"]) == key)
        .map(|e| {
            let n = str_at(e, &["name"]);
            if n.is_empty() { str_at(e, &["id"]) } else { n }.to_owned()
        })
        .unwrap_or_else(|| key.to_owned())
}

fn writes_of(report: &Value, model: &Value) -> Vec<AlinkSystemWrite> {
    list(report, "writes")
        .iter()
        .map(|w| {
            let result = str_at(w, &["result"]);
            let element = str_at(w, &["element"]);
            let key = if element.is_empty() {
                str_at(w, &["serial"])
            } else {
                element
            };
            AlinkSystemWrite {
                unit: element_name(model, key),
                what: what_word(str_at(w, &["what"])),
                result: result.to_owned(),
                result_word: result_word(result),
                tone: result_tone(result).to_owned(),
            }
        })
        .collect()
}

// ---------------------------------------------------------------- the view

/// The view of `model` at `revision`, with the hub's `status` and this
/// session's `role`.
pub(crate) fn build_view(
    model: &Value,
    revision: u64,
    status: &Value,
    role: &str,
) -> AlinkSystemView {
    let rate = rate_of(model, Some(status));
    let observed = |serial: &str| -> Option<&Value> {
        if serial.is_empty() {
            None
        } else {
            at(status, &["observed", serial])
        }
    };
    let alarms_on: Vec<&Value> = list(status, "alarms")
        .iter()
        .filter(|a| str_at(a, &["state"]) != "cleared")
        .collect();
    let alarms_for = |id: &str| -> u32 {
        let n = alarms_on
            .iter()
            .filter(|a| str_at(a, &["subject"]) == id)
            .count();
        u32::try_from(n).unwrap_or(u32::MAX)
    };
    let room_name = |id: &str| -> String {
        list(model, "rooms")
            .iter()
            .find(|r| str_at(r, &["id"]) == id)
            .map(|r| str_at(r, &["name"]).to_owned())
            .unwrap_or_default()
    };
    let plans = list(at(status, &["alignment"]).unwrap_or(&Value::Null), "units");
    let plan_of = |id: &str| plans.iter().find(|p| str_at(p, &["unit"]) == id);

    let mut units = Vec::new();
    let base = |e: &Value, kind: &str| -> AlinkSystemUnit {
        let id = str_at(e, &["id"]);
        let name = str_at(e, &["name"]);
        let serial = str_at(e, &["serial"]);
        let o = observed(serial);
        let connected = o.and_then(|o| o.get("online")).and_then(Value::as_bool) == Some(true);
        let seen_role = o
            .map(|o| str_at(o, &["role"]))
            .filter(|r| !r.is_empty() && *r != "none");
        let model_role = match kind {
            "unit" if e.get("repeater").and_then(Value::as_bool) == Some(true) => "repeater",
            k => k,
        };
        // -- WP HWLK1F -- a firmware without the alink is said as such, never "Unknown"
        let no_alink = connected && o.map(|o| str_at(o, &["alink"])) == Some("none");
        let state = if !connected {
            "offline"
        } else if no_alink {
            "no_alink"
        } else if kind == "sender" {
            "clock"
        } else {
            match o.map(|o| str_at(o, &["state"])).unwrap_or("") {
                s @ ("locked" | "searching" | "holdover" | "off") => s,
                _ => "unknown",
            }
        };
        let phase = o.and_then(|o| o.get("phase")).and_then(Value::as_array);
        AlinkSystemUnit {
            id: id.to_owned(),
            name: if name.is_empty() { id } else { name }.to_owned(),
            serial: serial.to_owned(),
            kind: kind.to_owned(),
            // -- WP HWLK1F -- never a role from the model alone for a unit without the alink
            role: if no_alink {
                "No audio link"
            } else {
                role_word(seen_role.unwrap_or(model_role))
            }
            .to_owned(),
            room: room_name(str_at(e, &["room"])),
            connected,
            state: state.to_owned(),
            state_word: state_word(state).to_owned(),
            unit_alink_delay_frames: u32_of(phase.and_then(|p| p.get(1))),
            unit_hop_offset: i32_of(phase.and_then(|p| p.get(2))),
            alink_delay_min: u32_of(o.and_then(|o| o.get("dLocalMin"))),
            alink_delay_max: u32_of(o.and_then(|o| o.get("dLocalMax"))),
            phase_error_frames: o
                .and_then(|o| o.get("phaseErrorFrames"))
                .and_then(Value::as_f64)
                .map(|f| f as f32),
            hops: u32_of(o.and_then(|o| o.get("hops"))),
            hop_delay_frames: u32_of(o.and_then(|o| o.get("hopDelayFrames")))
                .filter(|_| kind == "fan_out" || model_role == "repeater")
                .or_else(|| u32_of(e.get("hopDelayFrames"))),
            latency_us: e.get("latencyUs").and_then(Value::as_i64).unwrap_or(0),
            measured_latency_us: e.get("measuredLatencyUs").and_then(Value::as_i64),
            alarms: alarms_for(id),
            transport: "alink".into(),
            ..AlinkSystemUnit::default()
        }
    };
    for s in list(model, "senders") {
        units.push(base(s, "sender"));
    }
    for f in list(model, "fanOuts") {
        units.push(base(f, "fan_out"));
    }
    let (mut locked, mut holdover, mut pending, mut count) = (0u32, 0u32, 0u32, 0u32);
    let mut without = 0u32;
    // -- WP AUSB1 --
    let (mut usb_count, mut usb_locked, mut usb_trouble) = (0u32, 0u32, 0u32);
    for u in list(model, "units") {
        let mut row = base(u, "unit");
        // -- WP AUSB1 -- a USB-direct unit: its own state, delay and statistics
        if str_at(u, &["transport"]) == "usb_direct" {
            let plan = plan_of(&row.id);
            usb_row(&mut row, u, observed(str_at(u, &["serial"])), plan, rate);
            usb_count = usb_count.saturating_add(1);
            if usb_locked_as_asked(&row) {
                usb_locked = usb_locked.saturating_add(1);
            }
            if matches!(row.state.as_str(), "no_sof" | "rejected") || row.usb_lock_error {
                usb_trouble = usb_trouble.saturating_add(1);
            }
            if row.pending {
                pending = pending.saturating_add(1);
            }
            units.push(row);
            continue;
        }
        // -- WP HWLK1F -- no audio link delay, nothing pending, not counted among those that lock
        if row.state == "no_alink" {
            without = without.saturating_add(1);
            units.push(row);
            continue;
        }
        let plan = plan_of(&row.id);
        let planned_delay = plan.and_then(|p| u32_of(p.get("dLocal")));
        let planned_offset = plan.and_then(|p| i32_of(p.get("hopsOffset")));
        let writable = plan
            .and_then(|p| p.get("writable"))
            .and_then(Value::as_bool)
            != Some(false);
        row.alink_delay_frames = planned_delay.or(row.unit_alink_delay_frames);
        row.hop_offset = planned_offset.or(row.unit_hop_offset);
        row.alink_delay_ms = row.alink_delay_frames.map(|f| frames_ms(f, rate));
        row.pending = row.connected
            && writable
            && planned_delay.is_some()
            && (planned_delay != row.unit_alink_delay_frames
                || planned_offset != row.unit_hop_offset);
        count = count.saturating_add(1);
        match row.state.as_str() {
            "locked" => locked = locked.saturating_add(1),
            "holdover" => holdover = holdover.saturating_add(1),
            _ => {}
        }
        if row.pending {
            pending = pending.saturating_add(1);
        }
        units.push(row);
    }

    let mut alarms: Vec<AlinkSystemAlarm> = alarms_on
        .iter()
        .map(|a| AlinkSystemAlarm {
            id: a.get("id").and_then(Value::as_u64).unwrap_or(0),
            severity: str_at(a, &["severity"]).to_owned(),
            subject: element_name(model, str_at(a, &["subject"])),
            message: str_at(a, &["message"]).to_owned(),
            state: str_at(a, &["state"]).to_owned(),
        })
        .collect();
    alarms.sort_by_key(|a| a.severity != "critical");

    let senders: Vec<&Value> = list(model, "senders")
        .iter()
        .filter_map(|s| observed(str_at(s, &["serial"])))
        .collect();
    let mutes: Vec<bool> = senders
        .iter()
        .filter_map(|o| o.get("systemMute").and_then(Value::as_bool))
        .collect();
    let system_mute = (!mutes.is_empty()).then(|| mutes.iter().any(|m| *m));
    let pending_apply_frame = senders
        .iter()
        .find_map(|o| o.get("pendingApplyFrame").and_then(Value::as_u64));

    let installed = !units.is_empty();
    let (summary, health) = summary(
        installed,
        Counts {
            count,
            locked,
            holdover,
            pending,
            without,
            usb_count,
            usb_locked,
            usb_trouble,
        },
        &units,
    );
    let may_edit = role == "admin";
    let may_recall = role == "admin" || role == "control";
    let delay = u32_of(at(status, &["alignment", "dFrames"])).unwrap_or(0);
    AlinkSystemView {
        revision,
        installed,
        role: role.to_owned(),
        may_edit,
        may_recall,
        edit_note: if may_edit {
            String::new()
        } else {
            "Changing the installation needs the hub's Admin role.".into()
        },
        recall_note: if may_recall {
            String::new()
        } else {
            "Recalling a scene needs the hub's Control role.".into()
        },
        system_delay_frames: delay,
        system_delay_ms: frames_ms(delay, rate),
        system_delay_auto: model.get("dFrames").is_none_or(Value::is_null),
        rate_hz: rate,
        lip_sync_us: model.get("lipSyncUs").and_then(Value::as_i64).unwrap_or(0),
        units,
        rooms: list(model, "rooms")
            .iter()
            .map(|r| AlinkSystemRoom {
                id: str_at(r, &["id"]).to_owned(),
                name: str_at(r, &["name"]).to_owned(),
                lip_sync_us: r.get("lipSyncUs").and_then(Value::as_i64),
            })
            .collect(),
        scenes: list(model, "scenes")
            .iter()
            .map(|s| AlinkSystemScene {
                id: str_at(s, &["id"]).to_owned(),
                name: str_at(s, &["name"]).to_owned(),
                system_mute: s.get("systemMute").and_then(Value::as_bool),
            })
            .collect(),
        alarms,
        system_mute,
        pending_apply_frame,
        locked,
        holdover,
        pending,
        unit_count: count,
        usb_unit_count: usb_count,
        usb_locked,
        summary,
        health: health.to_owned(),
        issues: list(at(status, &["alignment"]).unwrap_or(&Value::Null), "issues")
            .iter()
            .map(|i| str_at(i, &["message"]).to_owned())
            .filter(|m| !m.is_empty())
            .collect(),
        // -- WP FIXM1 --
        keep_usb_aligned: model
            .get("keepUsbAligned")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        align_needed: list(at(status, &["alignment"]).unwrap_or(&Value::Null), "issues")
            .iter()
            .any(|i| str_at(i, &["code"]) == "usb_start_offsets_changed"),
        stale: false,
    }
}

// -- WP AUSB1 --
/// A USB-direct state in words.
fn usb_state_word(state: &str) -> &'static str {
    match state {
        "phase_locked" => "Phase-locked (USB)",
        "rate_locked" => "Rate-locked (USB)",
        "pull_in" => "Pulling in (USB)",
        "anchoring" => "Anchoring (USB)",
        "no_sof" => "No USB time base",
        "rejected" => "Host out of range (USB)",
        "usb_off" => "USB-direct off",
        "no_usb_direct" => "No USB-direct (firmware)",
        "offline" => "Not connected",
        _ => "Unknown",
    }
}

// -- WP AUSB2 --
/// A lock error's cause (the hub's `lockErrorCause`) in words.
fn lock_error_words(cause: &str) -> &'static str {
    match cause {
        "host_rejected" => "the host's rate is out of range",
        "pending" => "a phase error is still being pulled in",
        "config" => "its rate or D cannot phase-lock",
        _ => "",
    }
}

// -- WP AUSB1 --
/// A timeline end's cause in words.
fn end_word(cause: &str) -> &'static str {
    match cause {
        "sof_lost" => "SOF lost",
        "long_gap" => "a gap over 1 s",
        "discontinuity" => "a SOF discontinuity",
        "host_rejected" => "host out of range",
        "rate_or_config" => "rate or configuration",
        "reanchor_phase_error" => "re-anchor (phase error)",
        "reanchor_output_reset" => "re-anchor (output reset)",
        "reanchor_other" => "re-anchor",
        "stop" => "switched off",
        _ => "unknown",
    }
}

// -- WP AUSB1 --
/// Locked as the unit's mode asks: phase-locked, or rate-locked when the mode
/// is rate lock.
fn usb_locked_as_asked(row: &AlinkSystemUnit) -> bool {
    match row.usb_mode.as_deref() {
        Some("rate_lock") => matches!(row.state.as_str(), "rate_locked" | "phase_locked"),
        _ => row.state == "phase_locked",
    }
}

// -- WP AUSB1 --
/// Fill in a USB-direct unit's row: `u` the model's unit, `o` what the hub
/// read (`observed[serial]`), `plan` its line of the alignment.
fn usb_row(
    row: &mut AlinkSystemUnit,
    u: &Value,
    o: Option<&Value>,
    plan: Option<&Value>,
    rate: u32,
) {
    row.transport = "usb_direct".into();
    row.role = "USB-direct".into();
    // -- WP AUSB2 --
    row.usb_on_lock_error_wanted = match str_at(u, &["onLockError"]) {
        "play_unaligned" => "play_unaligned",
        _ => "mute",
    }
    .into();
    let wanted = str_at(u, &["usbMode"]);
    row.usb_mode_wanted = (!wanted.is_empty()).then(|| wanted.to_owned());
    let seen = o.and_then(|o| o.get("usb")).filter(|v| v.is_object());
    let no_block = o.map(|o| str_at(o, &["usbDirect"])) == Some("none");
    let state = if !row.connected {
        "offline"
    } else if no_block {
        "no_usb_direct"
    } else {
        match seen.map(|s| str_at(s, &["state"])).unwrap_or("") {
            s @ ("phase_locked" | "rate_locked" | "pull_in" | "anchoring" | "no_sof"
            | "rejected") => s,
            "off" => "usb_off",
            _ => "unknown",
        }
    };
    row.state = state.into();
    row.state_word = usb_state_word(state).into();
    // -- WP AUSB2 -- in lock error: what it does says more than its state
    let flag = |k: &str| {
        seen.and_then(|s| s.get(k))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    row.usb_on_lock_error = seen
        .map(|s| str_at(s, &["onLockError"]))
        .filter(|a| !a.is_empty())
        .map(str::to_owned);
    row.usb_play_unaligned_unsupported = flag("playUnalignedUnsupported");
    row.usb_lock_error = row.connected && flag("lockError");
    if row.usb_lock_error {
        let playing = row.usb_on_lock_error.as_deref() == Some("play_unaligned");
        row.state_word = if playing {
            "Can't lock (USB): playing unaligned"
        } else {
            "Can't lock (USB): muted"
        }
        .into();
        row.usb_lock_error_cause = lock_error_words(
            seen.map(|s| str_at(s, &["lockErrorCause"]))
                .unwrap_or_default(),
        )
        .into();
    }
    row.phase_error_frames = None;
    let f32_of = |k: &str| {
        seen.and_then(|s| s.get(k))
            .and_then(Value::as_f64)
            .map(|f| f as f32)
    };
    let u32_at = |k: &str| u32_of(seen.and_then(|s| s.get(k)));
    let mode = seen.map(|s| str_at(s, &["mode"])).filter(|m| !m.is_empty());
    row.usb_mode = mode.map(str::to_owned);
    row.unit_usb_delay_samples = u32_at("d");
    row.usb_delay_min = u32_at("dMin");
    row.usb_delay_max = u32_at("dMax");
    row.host_ppm = f32_of("hostPpm");
    row.vcxo_ppm = f32_of("vcxoPpm");
    row.phase_error_samples = f32_of("phaseError").filter(|_| state == "phase_locked");
    row.gaps = u32_at("gapCount");
    row.missed_samples = u32_at("missedSamples");
    row.late_samples = u32_at("lateSamples");
    row.reanchors = u32_at("reanchors");
    let diag = seen.and_then(|s| s.get("diag")).filter(|d| d.is_object());
    row.timeline_ends = diag.and_then(|d| u32_of(d.get("ends")));
    row.last_end = diag
        .map(|d| str_at(d, &["lastCause"]))
        .filter(|c| !c.is_empty())
        .map(|c| end_word(c).to_owned())
        .unwrap_or_default();
    let planned = plan.and_then(|p| u32_of(p.get("usbD")));
    let planned_mode = plan
        .map(|p| str_at(p, &["usbMode"]))
        .filter(|m| !m.is_empty());
    let writable = plan
        .and_then(|p| p.get("writable"))
        .and_then(Value::as_bool)
        != Some(false);
    row.start_offset_samples = plan
        .and_then(|p| p.get("startOffsetSamples"))
        .and_then(Value::as_i64);
    row.usb_delay_samples = planned.or(row.unit_usb_delay_samples);
    row.usb_delay_ms = row.usb_delay_samples.map(|f| frames_ms(f, rate));
    row.pending = row.connected
        && writable
        && seen.is_some()
        && ((planned.is_some() && planned != row.unit_usb_delay_samples)
            || (planned_mode.is_some() && planned_mode != mode));
    if seen.is_some() {
        let mut parts = Vec::new();
        if let Some(h) = row.host_ppm {
            parts.push(format!("host {h:+.1} ppm"));
        }
        if let Some(v) = row.vcxo_ppm {
            parts.push(format!("VCXO {v:+.1} ppm"));
        }
        if let Some(e) = row.phase_error_samples {
            parts.push(format!("phase error {e:+.2} samples"));
        }
        parts.push(format!(
            "gaps {}, missed {}, late {} samples",
            row.gaps.unwrap_or(0),
            row.missed_samples.unwrap_or(0),
            row.late_samples.unwrap_or(0)
        ));
        parts.push(format!("re-anchors {}", row.reanchors.unwrap_or(0)));
        if let Some(n) = row.timeline_ends {
            if row.last_end.is_empty() {
                parts.push(format!("timeline ends {n}"));
            } else {
                parts.push(format!("timeline ends {n} (last: {})", row.last_end));
            }
        }
        if let Some(o) = row.start_offset_samples.filter(|o| *o != 0) {
            parts.push(format!("stream {o} samples early"));
        }
        // -- WP AUSB2 -- DIAG layout 2's counters, when there is something to say
        let diag_u = |k: &str| u32_of(diag.and_then(|d| d.get(k))).unwrap_or(0);
        if diag.is_some_and(|d| u32_of(d.get("version")) == Some(2)) {
            parts.push(format!("lock errors {}", diag_u("lockErrors")));
            let bad = diag_u("badSofReadings");
            if bad > 0 {
                parts.push(format!("SOF readings repaired {bad}"));
            }
        }
        let ride = diag_u("longestRideMs");
        if ride > 0 {
            parts.push(format!("longest ride {ride} ms"));
        }
        row.usb_stats = parts.join(" · ");
    }
}

/// The units counted for the summary.
struct Counts {
    /// Units that can lock (not those without the alink).
    count: u32,
    locked: u32,
    holdover: u32,
    pending: u32,
    // -- WP HWLK1F --
    /// Units whose firmware has no audio link.
    without: u32,
    // -- WP AUSB1 --
    /// USB-direct units.
    usb_count: u32,
    /// Those locked as their mode asks.
    usb_locked: u32,
    /// Those without SOF or with the host out of range.
    usb_trouble: u32,
}

/// The status line and its meaning.
fn summary(installed: bool, c: Counts, units: &[AlinkSystemUnit]) -> (String, &'static str) {
    let Counts {
        count,
        locked,
        holdover,
        pending,
        without,
        usb_count,
        usb_locked,
        usb_trouble,
    } = c;
    if !installed {
        return ("No audio link installation on this hub".into(), "none");
    }
    let offline = units
        .iter()
        .filter(|u| u.kind == "unit" && !u.connected)
        .count();
    // -- WP AUSB1 -- USB-direct units after the audio link's, or alone
    let mut line = if count == 0 && without == 0 && usb_count > 0 {
        format!("Audio link USB-direct: {usb_locked} of {usb_count} locked")
    } else {
        let mut l = format!("Audio link: {locked} of {count} locked");
        if usb_count > 0 {
            l.push_str(&format!(", USB-direct {usb_locked} of {usb_count} locked"));
        }
        l
    };
    // -- WP HWLK1F --
    if without > 0 {
        line.push_str(&format!(", {without} without the audio link (firmware)"));
    }
    if holdover > 0 {
        line.push_str(&format!(", {holdover} in holdover"));
    }
    if offline > 0 {
        line.push_str(&format!(", {offline} not connected"));
    }
    if pending > 0 {
        line.push_str(&format!(", audio link delay to write on {pending}"));
    }
    let health = if holdover > 0 || offline > 0 || usb_trouble > 0 {
        "problem"
    } else if locked < count || pending > 0 || without > 0 || usb_locked < usb_count {
        "warning"
    } else {
        "ok"
    };
    (line, health)
}

// ---------------------------------------------------------------- edits

/// `edits` made on `model` (the hub's JSON, camelCase). Ids the model does
/// not have are refused, so nothing is half applied.
pub(crate) fn apply_edits(model: &mut Value, edits: &AlinkSystemEdits) -> Result<()> {
    let Some(m) = model.as_object_mut() else {
        return Err(DspiError::InvalidArgument {
            msg: "the hub has no installation to edit".into(),
        });
    };
    if edits.set_system_delay {
        m.insert(
            "dFrames".into(),
            edits.system_delay_frames.map_or(Value::Null, Value::from),
        );
    }
    for r in &edits.rooms {
        let room = m
            .get_mut("rooms")
            .and_then(Value::as_array_mut)
            .and_then(|rooms| rooms.iter_mut().find(|x| str_at(x, &["id"]) == r.room_id))
            .and_then(Value::as_object_mut)
            .ok_or_else(|| DspiError::InvalidArgument {
                msg: format!("no room {}", r.room_id),
            })?;
        room.insert(
            "lipSyncUs".into(),
            r.lip_sync_us.map_or(Value::Null, Value::from),
        );
    }
    for u in &edits.units {
        let unit = m
            .get_mut("units")
            .and_then(Value::as_array_mut)
            .and_then(|units| units.iter_mut().find(|x| str_at(x, &["id"]) == u.unit_id))
            .and_then(Value::as_object_mut)
            .ok_or_else(|| DspiError::InvalidArgument {
                msg: format!("no unit {}", u.unit_id),
            })?;
        unit.insert(
            "measuredLatencyUs".into(),
            u.measured_latency_us.map_or(Value::Null, Value::from),
        );
    }
    // -- WP AUSB1 --
    for u in &edits.usb_modes {
        if let Some(m) = u.mode.as_deref()
            && !matches!(m, "off" | "rate_lock" | "phase_lock")
        {
            return Err(DspiError::InvalidArgument {
                msg: format!("USB-direct mode {m:?}: off, rate_lock or phase_lock"),
            });
        }
        // -- WP AUSB2 --
        if let Some(a) = u.on_lock_error.as_deref()
            && !matches!(a, "mute" | "play_unaligned")
        {
            return Err(DspiError::InvalidArgument {
                msg: format!("USB-direct lock-error action {a:?}: mute or play_unaligned"),
            });
        }
        let unit = m
            .get_mut("units")
            .and_then(Value::as_array_mut)
            .and_then(|units| units.iter_mut().find(|x| str_at(x, &["id"]) == u.unit_id))
            .and_then(Value::as_object_mut)
            .filter(|x| x.get("transport").and_then(Value::as_str) == Some("usb_direct"))
            .ok_or_else(|| DspiError::InvalidArgument {
                msg: format!("no USB-direct unit {}", u.unit_id),
            })?;
        match &u.mode {
            Some(mode) => {
                unit.insert("usbMode".into(), Value::from(mode.as_str()));
            }
            None => {
                unit.remove("usbMode");
            }
        }
        // -- WP AUSB2 -- mute is the hub's default: not written
        match u.on_lock_error.as_deref() {
            Some("play_unaligned") => {
                unit.insert("onLockError".into(), Value::from("play_unaligned"));
            }
            Some(_) => {
                unit.remove("onLockError");
            }
            None => {}
        }
    }
    // -- WP FIXM1 -- off is the hub's default: not written
    match edits.keep_usb_aligned {
        Some(true) => {
            m.insert("keepUsbAligned".into(), Value::Bool(true));
        }
        Some(false) => {
            m.remove("keepUsbAligned");
        }
        None => {}
    }
    Ok(())
}

// ---------------------------------------------------------------- outcomes

/// What a recall did, from the hub's report. `held`: the frame the sender
/// was last seen holding, for the words of a refusal.
pub(crate) fn recall_outcome(
    scene: &str,
    report: &Value,
    model: &Value,
    held: Option<u64>,
    rate_hz: u32,
) -> AlinkSceneRecall {
    let name = list(model, "scenes")
        .iter()
        .find(|s| str_at(s, &["id"]) == scene)
        .map(|s| str_at(s, &["name"]))
        .filter(|n| !n.is_empty())
        .unwrap_or(scene)
        .to_owned();
    let plan = report.get("plan").unwrap_or(&Value::Null);
    let frame = plan.get("frame").and_then(Value::as_u64).unwrap_or(0);
    let lead = u32_of(plan.get("leadFrames")).unwrap_or(0);
    let path = str_at(plan, &["path"]);
    let on_time = report.get("onTime").and_then(Value::as_bool) == Some(true);
    let mut writes = writes_of(report, model);
    // -- fixup lk1 -- through the sender the one write carries the whole scene, so it
    // is named as the scene: "Sender · Scene Closed: Refused", never a refused "Mute"
    // -- WP HWLK1F -- not a unit's own result, which the hub gives a reason (applied
    // now without the audio link, not connected): it keeps its op
    if path == "superframe" {
        // -- WP AUSB1 -- a USB-direct unit's own write keeps its op too
        let usb: Vec<&str> = list(plan, "usbOps")
            .iter()
            .map(|o| str_at(o, &["serial"]))
            .collect();
        for (w, raw) in writes.iter_mut().zip(list(report, "writes")) {
            if str_at(raw, &["reason"]).is_empty() && !usb.contains(&str_at(raw, &["serial"])) {
                w.what = format!("Scene {name}");
            }
        }
    }
    let was_taken = list(report, "writes")
        .iter()
        .any(|w| taken(str_at(w, &["result"])));
    // The sender's answer, when nothing was taken through it
    let sender_result = list(report, "writes")
        .iter()
        .map(|w| str_at(w, &["result"]))
        .find(|r| !taken(r))
        .unwrap_or("refused");
    // -- WP HWLK1F -- units without the audio link took it at once, not at the frame
    let mixed = recall_parts(report, model);
    let (mut message, held_frame) = if was_taken && !mixed.is_plain() {
        (mixed.sentence(&name, path, frame, plan), None)
    } else if was_taken {
        let path_word = match path {
            "superframe" => "through the sender",
            "per_unit" => "to each unit",
            _ => "system mute only",
        };
        let mut m = format!("{name} recalled {path_word}, at audio link frame {frame}");
        match plan.get("systemMute").and_then(Value::as_bool) {
            Some(true) => m.push_str(", system mute on"),
            Some(false) => m.push_str(", system mute off"),
            None => {}
        }
        m.push_str(if on_time {
            "."
        } else {
            ": not every unit on time."
        });
        (m, None)
    } else if path != "superframe" {
        (
            format!("{name} was refused by every unit; nothing changed."),
            None,
        )
    } else {
        match sender_result {
            // -- fixup lk1 -- the held-scene words only for the sender's own refusal (2.8.9)
            "refused" | "refused_at_apply" => match held {
                Some(pending) => {
                    let after = report
                        .get("frameAfterWrites")
                        .and_then(Value::as_u64)
                        .unwrap_or_else(|| frame.saturating_sub(u64::from(lead)));
                    let frames = pending.saturating_sub(after);
                    let ms = u32::try_from(frames)
                        .map_or(f64::MAX, |f| frames_ms(f, rate_hz))
                        .round();
                    (
                        format!(
                            "{name} was refused: a scene is already scheduled on the sender for audio \
                             link frame {pending}, {ms:.0} ms after this recall. The sender takes one scene \
                             at a time; recall {name} again after that."
                        ),
                        Some(pending),
                    )
                }
                None => (
                    format!(
                        "{name} was refused: the sender takes one scene at a time and still holds \
                         one. Recall it again in a moment."
                    ),
                    None,
                ),
            },
            "timeout" => (
                format!(
                    "{name} was not recalled: the sender did not answer in time. Check the sender \
                     and recall {name} again."
                ),
                None,
            ),
            "locked" => (
                format!(
                    "{name} was not recalled: another console is applying changes to the sender. \
                     Recall {name} again when it is done."
                ),
                None,
            ),
            other => (
                format!(
                    "{name} was not recalled: the sender's answer was \"{}\".",
                    result_word(other)
                ),
                None,
            ),
        }
    };
    // -- fixup lk1 -- a scene that sets the system mute and was not taken leaves it as it was
    if !was_taken && plan.get("systemMute").and_then(Value::as_bool).is_some() {
        message.push_str(" System mute unchanged.");
    }
    AlinkSceneRecall {
        scene: scene.to_owned(),
        name,
        taken: was_taken,
        frame,
        lead_ms: frames_ms(lead, rate_hz),
        on_time,
        held_frame,
        message,
        writes,
    }
}

// -- WP HWLK1F --
/// A recall's writes sorted by what happened to each unit, for its sentence
/// when it was not simply "at the frame on every unit".
#[derive(Debug, Default)]
struct RecallParts {
    /// Units without the audio link that took it at once.
    now: Vec<String>,
    /// Units without the audio link that did not, with the result in words.
    now_failed: Vec<String>,
    /// Units with the alink that no frame could be read for.
    no_frame: Vec<String>,
    /// Units of the scene that are not attached.
    absent: Vec<String>,
    // -- fixup hwlk1f (F1) --
    /// Units with the alink (or not told) that the sender's write reaches, but
    /// whose own frame could not be read: not known to have taken it.
    unverified: Vec<String>,
    /// Every write at the frame was taken in time.
    frame_ok: bool,
    /// Any write went at the frame.
    at_frame: bool,
    // -- WP AUSB1 --
    /// USB-direct units that took it at the bus frame.
    usb_at: Vec<String>,
    /// The bus frame.
    usb_frame: Option<u64>,
    /// USB-direct units that are not phase-locked and took it at once.
    now_usb: Vec<String>,
}

impl RecallParts {
    /// Nothing to say beyond the frame and "on time".
    fn is_plain(&self) -> bool {
        self.now.is_empty()
            && self.usb_at.is_empty()
            && self.now_usb.is_empty()
            && self.now_failed.is_empty()
            && self.no_frame.is_empty()
            && self.absent.is_empty()
            && self.unverified.is_empty()
    }

    fn sentence(&self, name: &str, path: &str, frame: u64, plan: &Value) -> String {
        let mut parts = Vec::new();
        // -- WP AUSB1 -- with USB-direct units: at the USB frame, at the audio
        // link frame, applied now, in that order
        let usb = !self.usb_at.is_empty();
        if usb {
            parts.push(format!(
                "at USB frame {} on {}",
                self.usb_frame.unwrap_or(0),
                self.usb_at.join(", ")
            ));
            if self.at_frame {
                let path_word = if path == "superframe" {
                    "through the sender"
                } else {
                    "to each unit"
                };
                parts.push(format!(
                    "at audio link frame {frame} {path_word} on the others"
                ));
            }
        }
        let now: Vec<String> = self
            .now
            .iter()
            .map(|n| format!("{n} (no audio link)"))
            .chain(
                self.now_usb
                    .iter()
                    .map(|n| format!("{n} (USB-direct, not phase-locked)")),
            )
            .collect();
        if usb {
            if !now.is_empty() {
                parts.push(format!("applied now on {}", now.join(", ")));
            }
        } else if !self.now_usb.is_empty() {
            parts.push(format!("applied now on {}", now.join(", ")));
        } else if !self.now.is_empty() {
            parts.push(format!(
                "applied now on {} (no audio link)",
                self.now.join(", ")
            ));
        }
        if self.at_frame && !usb {
            let path_word = if path == "superframe" {
                "through the sender"
            } else {
                "to each unit"
            };
            let whom =
                if self.now.is_empty() && self.now_usb.is_empty() && self.now_failed.is_empty() {
                    ""
                } else {
                    " on the others"
                };
            parts.push(format!("at audio link frame {frame} {path_word}{whom}"));
        }
        if !self.now_failed.is_empty() {
            parts.push(format!("not applied on {}", self.now_failed.join(", ")));
        }
        if !self.no_frame.is_empty() {
            parts.push(format!(
                "not applied on {} (no audio link frame)",
                self.no_frame.join(", ")
            ));
        }
        if !self.absent.is_empty() {
            parts.push(format!("{} not connected", self.absent.join(", ")));
        }
        if !self.unverified.is_empty() {
            parts.push(format!(
                "not known to have taken it on {} (no audio link frame read)",
                self.unverified.join(", ")
            ));
        }
        let mut m = format!("{name} recalled: {}", parts.join("; "));
        match plan.get("systemMute").and_then(Value::as_bool) {
            Some(true) => m.push_str("; system mute on"),
            Some(false) => m.push_str("; system mute off"),
            None => {}
        }
        m.push('.');
        if (self.at_frame || usb) && !self.frame_ok {
            m.push_str(" Not every unit at the frame on time.");
        }
        m
    }
}

/// [`RecallParts`] of a hub's `system.scene.apply` report.
fn recall_parts(report: &Value, model: &Value) -> RecallParts {
    let mut p = RecallParts {
        frame_ok: true,
        ..RecallParts::default()
    };
    let push = |v: &mut Vec<String>, n: String| {
        if !v.contains(&n) {
            v.push(n);
        }
    };
    // -- WP AUSB1 -- the writes to USB-direct units at the bus frame
    let plan = report.get("plan").unwrap_or(&Value::Null);
    p.usb_frame = plan.get("usbFrame").and_then(Value::as_u64);
    let usb_serials: Vec<&str> = list(plan, "usbOps")
        .iter()
        .map(|o| str_at(o, &["serial"]))
        .collect();
    for w in list(report, "writes") {
        let result = str_at(w, &["result"]);
        let element = str_at(w, &["element"]);
        let unit = element_name(model, element);
        let reason = str_at(w, &["reason"]);
        let direct = matches!(
            reason,
            "no audio link (firmware)" | "not phase-locked (USB-direct)"
        );
        // -- WP AUSB1 --
        if reason.is_empty() && usb_serials.contains(&str_at(w, &["serial"])) {
            match result {
                "no_device" => push(&mut p.absent, unit),
                r => {
                    if !matches!(r, "queued" | "applied" | "ok" | "ok_unverified") {
                        p.frame_ok = false;
                    }
                    if taken(r) {
                        push(&mut p.usb_at, unit);
                    } else {
                        push(&mut p.now_failed, format!("{unit} ({})", result_word(r)));
                    }
                }
            }
            continue;
        }
        match result {
            // -- WP AUSB1 -- held back with a refused audio link part: nothing changed there
            _ if reason == "the audio link part was refused" => push(
                &mut p.now_failed,
                format!("{unit} (held back with the audio link)"),
            ),
            "applied_now" if reason == "not phase-locked (USB-direct)" => {
                push(&mut p.now_usb, unit)
            }
            "applied_now" => push(&mut p.now, unit),
            _ if direct => push(
                &mut p.now_failed,
                format!("{unit} ({})", result_word(result)),
            ),
            // -- WP ALINK1 -- an older hub's reason too ("no link frame read")
            "no_frame"
                if matches!(
                    str_at(w, &["reason"]),
                    "no audio link frame read" | "no link frame read"
                ) =>
            {
                push(&mut p.unverified, unit);
            }
            "no_frame" => push(&mut p.no_frame, unit),
            "no_device" => push(&mut p.absent, unit),
            r => {
                if matches!(str_at(w, &["what"]), "system_mute" | "system_unmute") {
                    continue;
                }
                p.at_frame = true;
                if !matches!(r, "queued" | "applied" | "ok" | "ok_unverified") {
                    p.frame_ok = false;
                }
            }
        }
    }
    p
}

// -- WP WB8 --
/// What a recall did, for a client that sent `system.scene.apply` itself (the
/// web's `hubSystem`): [`DspiCore::alink_system_recall`]'s outcome and words from
/// the hub's `report` and the `model` of `system.get`. `held`: the frame the
/// sender was last seen holding (its `pendingApplyFrame` in `system.status`);
/// `rate_hz`: the alignment's rate, else the model's.
pub fn scene_recall_outcome(
    scene: &str,
    report: &Value,
    model: &Value,
    held: Option<u64>,
    rate_hz: Option<u32>,
) -> AlinkSceneRecall {
    let rate = rate_hz
        .filter(|r| *r > 0)
        .unwrap_or_else(|| rate_of(model, None));
    recall_outcome(scene, report, model, held, rate)
}

// -- WP HWLK1F --
/// What writing the audio link delays did, for a client that sent `system.align`
/// itself (the web's `hubSystem`): [`DspiCore::alink_system_align`]'s outcome
/// and words from the hub's `report` and the `model` of `system.get`.
pub fn system_align_outcome(report: &Value, model: &Value) -> AlinkSystemAlign {
    align_outcome(report, model)
}

/// What writing the audio link delays did, from the hub's report. -- WP HWLK1F --
/// Said as it was: nothing written (and why) when every write was skipped or
/// already in place, and the units written and skipped when only some were.
pub(crate) fn align_outcome(report: &Value, model: &Value) -> AlinkSystemAlign {
    let applied = report.get("applied").and_then(Value::as_bool) == Some(true);
    let writes = writes_of(report, model);
    let failed = writes.iter().filter(|w| w.tone == "bad").count();
    // By unit, in the report's order: written, already set, skipped (and why).
    let (mut written, mut kept, mut skipped) = (Vec::new(), Vec::new(), Vec::new());
    for (w, raw) in writes.iter().zip(list(report, "writes")) {
        let unit = &w.unit;
        if w.result == "ok" || w.result.starts_with("clamped") {
            kept.retain(|k| k != unit);
            if !written.contains(unit) {
                written.push(unit.clone());
            }
        } else if w.result == "unchanged" {
            if !written.contains(unit) && !kept.contains(unit) {
                kept.push(unit.clone());
            }
        } else if w.result == "skipped" {
            let reason = str_at(raw, &["reason"]);
            skipped.push(if reason.is_empty() {
                unit.clone()
            } else {
                format!("{unit} ({reason})")
            });
        }
    }
    let mut parts = Vec::new();
    if !skipped.is_empty() {
        parts.push(format!("skipped on {}", skipped.join(", ")));
    }
    let message = if !applied {
        let issue = list(at(report, &["alignment"]).unwrap_or(&Value::Null), "issues")
            .iter()
            .map(|i| str_at(i, &["message"]))
            .find(|m| !m.is_empty())
            .unwrap_or("the alignment has an issue");
        format!("The audio link delays were not written: {issue}.")
    } else if written.is_empty() && failed == 0 {
        if !kept.is_empty() {
            parts.insert(0, format!("already set on {}", kept.join(", ")));
        }
        if skipped.is_empty() {
            "Nothing was written: every unit already has its audio link delays.".to_owned()
        } else {
            format!("Nothing was written: {}.", parts.join("; "))
        }
    } else if failed > 0 && written.is_empty() {
        // -- fixup hwlk1f (F3) -- every write failed: nothing was written, said so
        let mut m = format!("Nothing was written: {failed} writes failed");
        if !parts.is_empty() {
            m.push_str(&format!("; {}", parts.join("; ")));
        }
        m.push('.');
        m
    } else if failed > 0 {
        let mut m = format!("The audio link delays were written; {failed} writes failed");
        if !parts.is_empty() {
            m.push_str(&format!("; {}", parts.join("; ")));
        }
        m.push('.');
        m
    } else if parts.is_empty() {
        "The audio link delays were written. Each unit re-anchored with a short silence.".to_owned()
    } else {
        format!(
            "The audio link delays were written on {} (each re-anchored with a short silence); {}.",
            written.join(", "),
            parts.join("; ")
        )
    };
    AlinkSystemAlign {
        applied,
        message,
        writes,
    }
}

// -- WP P1QT1 -- `system.set`'s whole answer for the C ABI (HWLK1F's note: the
// System window shows the hub's warnings after Save). Not a UniFFI export:
// Android's `alink_system_save` keeps returning the revision until its own WP
// asks for this.

/// What saving the "Clock and audio links" edits did.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlinkSystemSaved {
    /// The model's new revision.
    pub revision: u64,
    /// The hub's warnings about the model it kept (a unit whose firmware has
    /// no audio link, or no USB-direct block): one sentence each, the hub's
    /// words. Empty when there are none.
    pub warnings: Vec<String>,
}

impl DspiCore {
    /// [`DspiCore::alink_system_save`] with the hub's whole answer: the new
    /// revision and the warnings `system.set` gave about the model it kept.
    /// Errors as there.
    pub fn alink_system_save_report(
        &self,
        revision: u64,
        edits: AlinkSystemEdits,
    ) -> Result<AlinkSystemSaved> {
        let client = self.hub_client()?;
        let cached = self
            .system_cache(|c| c.model.clone())
            .filter(|(r, _)| *r == revision);
        let mut model = match cached {
            Some((_, m)) => m,
            None => {
                let got = request(&client, "system.get", Map::new()).map_err(map_error)?;
                let now = got.get("revision").and_then(Value::as_u64).unwrap_or(0);
                if now != revision {
                    return Err(conflict(now));
                }
                got.get("model").cloned().unwrap_or(Value::Null)
            }
        };
        apply_edits(&mut model, &edits)?;
        let mut fields = Map::new();
        fields.insert("model".into(), model);
        fields.insert("revision".into(), revision.into());
        let saved = request(&client, "system.set", fields);
        // Whatever the answer, the next view reads the model again.
        self.system_cache(|c| c.model = None);
        match saved {
            Ok(v) => Ok(AlinkSystemSaved {
                revision: v
                    .get("revision")
                    .and_then(Value::as_u64)
                    .unwrap_or(revision),
                warnings: v
                    .get("warnings")
                    .and_then(Value::as_array)
                    .map(|w| {
                        w.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
            }),
            Err(LinkError::Rejected { code, .. }) if code == "conflict" => Err(conflict(revision)),
            Err(e) => Err(map_error(e)),
        }
    }
}
