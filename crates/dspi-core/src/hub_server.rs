//! Typed calls for the hub's own families past DSPi Link (WP P2CC1, PRO phase
//! 2): the inventory, Identify, health, statistics, the shared event history
//! and the hub's snapshot store, as `hub_admin.rs` does for the limits. The
//! wire contract is `docs/integrator/hub-api.md` (sections 3 to 6, 11, 15).
//!
//! - [`DspiCore::hub_caps`]: which of the families this console's hub has (its
//!   `hello.caps`). No I/O.
//! - [`DspiCore::hub_inventory`], [`DspiCore::hub_health`],
//!   [`DspiCore::hub_stats`], [`DspiCore::hub_events_read`]: reads.
//! - [`DspiCore::hub_identify`]: the hub's `device.identify` for the open
//!   device on a hub, and the device's own `EXT_IDENTIFY` when it is connected
//!   directly, so a client has one Identify.
//! - `hub_snapshots_*`: the hub's store (list, get, put, rename, note,
//!   delete, recall), "Save to the hub" ([`DspiCore::hub_snapshots_save`]) and
//!   "Copy here" ([`DspiCore::hub_snapshots_copy_here`]). The merged list of
//!   both sides is [`DspiCore::snapshot_list_merged`] (`snapshot.rs`).
//! - The hub's `event.added` (after [`DspiCore::hub_events_watch`]),
//!   `inventory.changed` and `snapshot.changed` reach the app through
//!   `dspi-link`'s message hook: a [`HubServerListener`], or
//!   [`DspiCore::hub_server_events`] where a callback cannot reach the app
//!   (the web).
//!
//! Every call answers [`DspiError::Unsupported`] and sends nothing when the
//! hub lacks the family's cap (or the console is on no hub), and
//! [`DspiError::Disconnected`] while the hub session is down. The hub's own
//! refusals come typed: `denied`, `unsupported`, `locked`, `bad_request`,
//! `limit_exceeded`, `installer_lock`, `image_refused` and
//! `confirmation_required` as the core's own variants; `not_found`,
//! `offline`, `conflict`, `rate_limited`, `busy` and `device` as
//! [`DspiError::Link`] with that code (the `HUB_ERR_*` constants), each with
//! its notice row `dlink.<code>`. A write past the client read-only mode is
//! [`DspiError::ReadOnly`], with nothing sent ([`HUB_SERVER_READS`] are the
//! reads it lets out). Each call is a round trip: off the UI thread.
//!
//! The second section (WP P2CC2, [`admin`]): alarms, schedules, notifications
//! out, MQTT and API keys, with `alarm.changed` and `schedule.ran` through
//! the same listener.
//!
//! The third section (WP P2CC3, [`manage`]): an Admin's inventory edits and
//! forget, renaming the hub, and Identify for any device on the hub.
//!
//! The fourth section (WP P3CC, [`fleet`]): groups, group writes, bulk
//! apply, the firmware image store and the rollout, with `group.changed` and
//! `fw.rollout.changed` through the same listener.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use serde_json::Value;

use crate::snapshot::SnapshotInfo;
use crate::{DspiCore, DspiError, IdentifyInfo, Result, Slot};

// -- WP P2CC2 -- the hub's admin families
pub mod admin;
pub use admin::{
    ALARM_CODES, CAP_ALARMS, CAP_API_KEYS, CAP_MQTT, CAP_OUTBOUND, CAP_REST, CAP_SCHEDULE,
    HubAlarm, HubAlarmRule, HubApiKey, HubApiKeyMinted, HubMqttSettings, HubMqttStatus, HubMqttTls,
    HubOutboundLast, HubOutboundSecret, HubOutboundSet, HubOutboundTarget, HubOutboundTest,
    HubScheduleAction, HubScheduleLast, HubScheduleResult, HubScheduleRule, HubScheduleRun,
    HubSchedules, HubSecret, NOTICE_ALARM_FAMILY, NOTICE_ALARM_UNKNOWN, hub_alarm_notice,
};

// -- WP P2CC3 -- the hub's device and name writes
pub mod manage;
pub use manage::HubInventoryForgot;

// -- WP P3CC -- the hub's fleet families (groups, group writes, bulk apply, rollout)
pub mod fleet;
pub use fleet::{
    CAP_GROUP_APPLY, CAP_GROUP_PARAMS, CAP_GROUPS, CAP_ROLLOUT, FIRMWARE_MAX_BYTES,
    GROUP_REQUEST_TIMEOUT_S, HUB_ERR_TLS_REQUIRED, HubFirmwareImage, HubFirmwareImages,
    HubFirmwarePut, HubFleetTarget, HubGroup, HubGroupParams, HubGroupRow, HubGroupRun,
    HubGroupSet, HubOffset, HubPresetPick, HubRollout, HubRolloutAfter, HubRolloutBefore,
    HubRolloutDevice, HubRolloutFailed, HubRolloutStep, HubRolloutVreg, NOTICE_ROLLOUT_FAMILY,
    NOTICE_ROLLOUT_UNKNOWN, ROLLOUT_STEPS, hub_rollout_step_notice,
};

// -- WP FEATUI -- who works on the hub's devices beside the people (AIVIS's web part)
pub mod presence;
pub use presence::{CAP_PRESENCE, HubPresence};

// -- WP P5HUB -- the hub's measurement documents
pub mod measure;
pub use measure::{
    CAP_HUB_MEASURE, HubMeasureDocPut, HubMeasureDocument, MEASURE_WIRE_PART_BYTES,
    MEASURE_WIRE_WHOLE_BYTES, MeasureDocsMerged, MergedMeasureDoc,
};

/// Cap: `inventory.*` (hub-api.md section 3).
pub const CAP_INVENTORY: &str = "inventory";
/// Cap: `device.identify` (section 4).
pub const CAP_IDENTIFY: &str = "identify";
/// Cap: `health.get` (section 5).
pub const CAP_HEALTH: &str = "health";
/// Cap: `event.*` (section 6).
pub const CAP_EVENTS: &str = "events";
/// Cap: the hub's snapshot store, `snapshot.*` (section 11; `snapshot` is
/// spec 7.5's device cache, a different thing).
pub const CAP_HUB_SNAPSHOTS: &str = "hub_snapshots";
/// Cap: the parameter API and presets by name (sections 9, 10).
pub const CAP_PARAM_API: &str = "param_api";
/// Cap: `hub.stats`, `hub.rename` (section 15).
pub const CAP_HUB_STATS: &str = "hub_stats";

/// A snapshot document is at most this many bytes on the DSPi Link, in its
/// compact JSON form (`max_frame` 65 536 less 4 KiB): the hub's
/// `MAX_WIRE_DOC_BYTES`. A larger one is refused here with the hub's own
/// words, and nothing is sent.
pub const SNAPSHOT_WIRE_MAX_BYTES: usize = 61_440;

/// The hub's code: an unknown serial, snapshot or record.
pub const HUB_ERR_NOT_FOUND: &str = "not_found";
/// The hub's code: the device is known but not attached; nothing was sent.
pub const HUB_ERR_OFFLINE: &str = "offline";
/// The hub's code: its state disagrees (a differing snapshot with the same
/// id, a store's cap, a pending guarded apply, a document too large).
pub const HUB_ERR_CONFLICT: &str = "conflict";
/// The hub's code: too many requests (Identify: one per device per 2 s).
pub const HUB_ERR_RATE_LIMITED: &str = "rate_limited";
/// The hub's code: the session's in-flight limit, or the hub's cores in use.
pub const HUB_ERR_BUSY: &str = "busy";
/// The hub's code: the device refused the command (a DSPi Link status).
pub const HUB_ERR_DEVICE: &str = "device";

/// The hub requests of these families that only read: the client read-only
/// mode lets them out (`read_only.rs`), and refuses every other request of
/// them (`device.identify`, `snapshot.put`/`rename`/`note`/`delete`/`recall`,
/// `inventory.set`/`forget`, `hub.rename`; P2CC2: `alarm.ack`,
/// `alarm.rules.set`, `schedule.set`/`delete`/`run_now`, `outbound.set`/`test`,
/// `mqtt.set`, `auth.mint`/`revoke`; P3CC: `group.set`/`delete`,
/// `group.param.set`, `group.preset.load`, `group.apply`, `fw.image.put`/
/// `delete`, `fw.rollout.start`; `group.apply.confirm`/`cancel` and
/// `fw.rollout.cancel` pass as [`HUB_SERVER_SETTLES`], -- WP P3WGR --).
/// `event.watch` only asks for events.
pub const HUB_SERVER_READS: [&str; 21] = [
    "inventory.list",
    "health.get",
    "hub.stats",
    "event.list",
    "event.watch",
    "event.export",
    "snapshot.list",
    "snapshot.get",
    // -- WP P2CC2 -- reads only
    "alarm.list",
    "alarm.rules.get",
    "schedule.list",
    "outbound.get",
    "mqtt.get",
    "mqtt.status",
    "auth.list",
    // -- WP P3CC -- reads only
    "group.list",
    "group.param.get",
    "fw.image.list",
    "fw.rollout.status",
    // -- WP P5HUB -- reads only
    "measure.doc.list",
    "measure.doc.get",
];

// -- WP P3WGR --
/// The hub requests that settle a bulk apply's guarded run (`group.apply.confirm`,
/// `group.apply.cancel`): the client read-only mode lets them out too, as it
/// must let a run begun before the mode was turned on be kept or reverted
/// (settling never needs write access; Android's `WriteGuardedCore` passes
/// them the same way). They write nothing new: the hub keeps or reverts what
/// the run already wrote.
///
/// -- review p3d -- and `fw.rollout.cancel` (coordinator decision): it only
/// stops a rollout before its next bootloader entry and starts or writes
/// nothing (Android passes `hubRolloutCancel` the same way).
/// `fw.rollout.start` stays refused.
pub const HUB_SERVER_SETTLES: [&str; 3] = [
    "group.apply.confirm",
    "group.apply.cancel",
    "fw.rollout.cancel",
];

/// [`MergedSnapshot::location`]: only in this console's store.
pub const WHERE_LOCAL: &str = "local";
/// [`MergedSnapshot::location`]: only on the hub.
pub const WHERE_HUB: &str = "hub";
/// [`MergedSnapshot::location`]: on both sides, the same.
pub const WHERE_BOTH: &str = "both";

/// The events kept for [`DspiCore::hub_server_events`] at most; older ones go.
#[cfg_attr(not(feature = "link"), allow(dead_code))]
const QUEUE_MAX: usize = 64;
/// Longest text taken from the hub for one field, in characters.
const TEXT_MAX: usize = 300;
/// Longest note taken from the hub (its own cap is 4096).
const NOTE_MAX: usize = 4096;
/// Items taken from the hub for one list (tags, lines, notes).
const ITEMS_MAX: usize = 64;
/// Devices taken from the hub's inventory at most (its own cap).
#[cfg_attr(not(feature = "link"), allow(dead_code))]
const DEVICES_MAX: usize = 1024;
/// Pages of `inventory.list` read at most (500 a page).
#[cfg_attr(not(feature = "link"), allow(dead_code))]
const PAGES_MAX: usize = 8;
/// Records of one `event.list` at most (the hub's own cap).
#[cfg_attr(not(feature = "link"), allow(dead_code))]
const EVENTS_MAX: u32 = 500;

// ── Records ────────────────────────────────────────────────────────────────

/// Which of the hub's families this console's hub has ([`DspiCore::hub_caps`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubCaps {
    /// This console has a live DSPi Link session.
    pub on_hub: bool,
    /// `inventory.*` ([`DspiCore::hub_inventory`]).
    pub inventory: bool,
    /// `device.identify` ([`DspiCore::hub_identify`] on a hub).
    pub identify: bool,
    /// `health.get` ([`DspiCore::hub_health`]).
    pub health: bool,
    /// `event.*` ([`DspiCore::hub_events_read`]).
    pub events: bool,
    /// The hub's snapshot store (`hub_snapshots_*`).
    pub hub_snapshots: bool,
    /// The parameter API and presets by name.
    pub param_api: bool,
    /// `hub.stats` ([`DspiCore::hub_stats`]).
    pub hub_stats: bool,
    // -- WP P2CC2 --
    /// `alarm.*` ([`DspiCore::hub_alarms`]).
    pub alarms: bool,
    /// `outbound.*` ([`DspiCore::hub_outbound_get`]).
    pub outbound: bool,
    /// `schedule.*` ([`DspiCore::hub_schedules`]).
    pub schedule: bool,
    /// `mqtt.*` ([`DspiCore::hub_mqtt_get`]).
    pub mqtt: bool,
    /// API keys ([`DspiCore::hub_api_keys`]).
    pub api_keys: bool,
    /// The REST mirror is on (the hub runs with `--rest`).
    pub rest: bool,
    // -- WP P3CC --
    /// `group.*` ([`DspiCore::hub_groups`]).
    pub groups: bool,
    /// `group.param.*`, `group.preset.load` ([`DspiCore::hub_group_param_set`]).
    pub group_params: bool,
    /// `group.apply*` ([`DspiCore::hub_group_apply`]).
    pub group_apply: bool,
    /// `fw.*` ([`DspiCore::hub_rollout_start`]; over TLS only).
    pub rollout: bool,
    // -- WP P5HUB --
    /// `measure.doc.*` ([`DspiCore::hub_measure_docs`]).
    pub hub_measure: bool,
    /// Every cap of the hub's `hello`, as it sent them.
    pub caps: Vec<String>,
}

/// A reset as the hub saw it (`EXT_RESET_INFO`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubReset {
    /// `power_on`, `brownout`, `run_pin`, `watchdog`, `software`,
    /// `debugger`, `glitch` or `unknown`.
    pub reason: String,
    /// How long the run before it lasted, s; `None` when unknown.
    pub prev_uptime_s: Option<u32>,
    /// When it happened (ISO-8601 UTC); "" where the hub gives no time.
    pub at: String,
}

/// One device of the hub's inventory ([`DspiCore::hub_inventory`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubDevice {
    /// The key.
    pub serial: String,
    /// Its DSPi Link handle while online.
    pub handle: Option<u8>,
    /// Attached now.
    pub online: bool,
    /// Its name when last seen.
    pub name: String,
    /// `RP2040`, `RP2350` (`unknown platform N` for an id this build does
    /// not know); "" when unknown.
    pub platform: String,
    /// The firmware version.
    pub fw: String,
    /// A beta build; `None` when unknown.
    pub fw_beta: Option<bool>,
    /// Output channels.
    pub outputs: u8,
    /// The board the firmware was built for; "" when unknown.
    pub board: String,
    /// The hardware revision; "" when unknown or unset.
    pub hw_rev: String,
    /// The chip; "" when unknown.
    pub chip: String,
    /// The chip's revision, when known.
    pub chip_rev: Option<u8>,
    /// Set by an Admin; "" when unset.
    pub location: String,
    /// Set by an Admin; "" when unset.
    pub note: String,
    /// Set by an Admin.
    pub tags: Vec<String>,
    /// First seen (ISO-8601 UTC).
    pub first_seen: String,
    /// Last seen: now while online, else when it went offline.
    pub last_seen: String,
    /// Its last reset, when the hub read one.
    pub last_reset: Option<HubReset>,
}

/// One online device's health ([`DspiCore::hub_health`]). A field the
/// firmware does not give, or the hub has not read yet, is `None`.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubHealth {
    /// The device.
    pub serial: String,
    /// Its handle.
    pub handle: u8,
    /// The newest read (ISO-8601 UTC); "" before the first.
    pub at: String,
    /// The busier core's load, %.
    pub cpu_pct: Option<u8>,
    /// The sticky clip latch, one bit per channel (never cleared by the hub).
    pub clip_mask: Option<u32>,
    /// °C, to 0.01.
    pub temp_c: Option<f64>,
    /// Uptime, s (from the reset-info read, every 60 s).
    pub uptime_s: Option<u32>,
    /// Watchdog hangs not yet acted on.
    pub hang_count: Option<u8>,
    /// No answer for 10 s while attached.
    pub unresponsive: bool,
    /// Since when, while [`HubHealth::unresponsive`]; "" otherwise.
    pub unresponsive_since: String,
    /// The reset seen at attach or since.
    pub reset: Option<HubReset>,
    // -- WP P3HB --
    /// The stream's underrun and starvation totals, once the hub read either
    /// (one read every 30 s, the four counters in turn).
    pub counters: Option<HubStreamCounters>,
    /// Clip events in the last minute; `None` on firmware with only the clip
    /// latch (no extension clip events).
    pub clip_per_min: Option<u32>,
    /// The device's newest change and who made it, from the hub's history.
    pub last_change: Option<HubLastChange>,
}

// -- WP P3HB --
/// A device's stream counters in [`HubHealth`]. Each is `None` until the hub
/// read it.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubStreamCounters {
    /// Output underruns (PDM ring, PDM DMA and S/PDIF added up), total since
    /// the device started.
    pub underruns: Option<u32>,
    /// Output starvations, total since the device started.
    pub starvation: Option<u32>,
}

/// A device's newest change in [`HubHealth`]: the newest history record of a
/// write, recall, import, schedule run or new firmware.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubLastChange {
    /// When (ISO-8601 UTC).
    pub at: String,
    /// Who made it (a client's name, a schedule); "" when the record names no
    /// one (a change at the device itself).
    pub by: String,
    /// The record's line.
    pub text: String,
}

/// One attached device's counts in [`HubStats`].
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubStatsDevice {
    /// Its handle.
    pub handle: u8,
    /// Its serial.
    pub serial: String,
    /// Commands that ran on it since it attached.
    pub cmds: u64,
    /// Those whose transfer failed.
    pub errors: u64,
    /// Their mean time on the transport, ms.
    pub avg_rtt_ms: f64,
    /// Its notifications a full session outbox dropped.
    pub notify_dropped: u64,
}

/// The hub's statistics ([`DspiCore::hub_stats`], Admin of the hub only).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubStats {
    /// Network sessions open now.
    pub sessions: u32,
    /// Seconds since the hub started.
    pub uptime_s: u64,
    /// One row per attached device.
    pub devices: Vec<HubStatsDevice>,
}

/// One record of a device's history on the hub: the event log's record
/// (`EventEntry`'s fields) and who caused it by name.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubEventRecord {
    /// Rising per device; numbering can start again (hub-api.md section 6).
    pub id: u64,
    /// When, ms since the Unix epoch.
    pub time_ms: i64,
    /// The record's kind (`notification`, `connected`, `recall`, `identify`,
    /// `alarm`, ...; `unknown` for a kind this build does not know).
    pub kind: String,
    /// The device's serial.
    pub serial: String,
    /// One readable line.
    pub text: String,
    /// Who or what caused it, as the log's `source`; "" when none.
    pub source: String,
    /// The DSPi Link session that caused it.
    pub session: Option<u32>,
    /// Who caused it, by name: the client, key, schedule or rule; "" when
    /// none.
    pub by: String,
    /// A wire channel, or a preset slot.
    pub channel: Option<u8>,
    /// A number: °C, a counter's total, Identify's seconds.
    pub value: Option<f64>,
    /// The firmware version, on connects and firmware changes.
    pub fw: Option<String>,
    /// The device's uptime, s.
    pub uptime_s: Option<u32>,
    /// Detail lines: what a recall or an import changed.
    pub lines: Vec<String>,
    /// Occurrences this record stands for.
    pub count: u32,
}

/// A page of a device's history on the hub ([`DspiCore::hub_events_read`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubEventPage {
    /// The device.
    pub serial: String,
    /// Oldest first.
    pub records: Vec<HubEventRecord>,
    /// More follow the last record's id.
    pub more: bool,
    /// The highest id the hub's log holds (0: empty). An `after_id` above
    /// it means the numbering started again: read from 0.
    pub newest_id: u64,
}

/// One of the hub's events of these families ([`HubServerListener`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubServerEvent {
    /// `event.added`, `inventory.changed` or `snapshot.changed`; P2CC2:
    /// `alarm.changed`, `schedule.ran`.
    pub kind: String,
    /// The device it concerns.
    pub serial: String,
    /// `event.added`'s record.
    pub record: Option<HubEventRecord>,
    /// `inventory.changed`'s record; `None` when the device was forgotten.
    pub device: Option<HubDevice>,
    /// `snapshot.changed`'s snapshot id.
    pub snapshot: String,
    /// `snapshot.changed`'s change: `put`, `rename`, `note`, `delete`.
    pub change: String,
    // -- WP P2CC2 --
    /// `alarm.changed`'s alarm (raised, raised again, acknowledged or
    /// cleared).
    pub alarm: Option<HubAlarm>,
    /// `schedule.ran`'s run.
    pub run: Option<HubScheduleRun>,
    // -- WP P3CC --
    /// `group.changed`'s group (as it is now; as it was, for a delete);
    /// [`HubServerEvent::change`] is `set`, `delete` or `forgot`.
    pub group: Option<HubGroup>,
    /// `fw.rollout.changed`'s whole record; [`HubServerEvent::serial`] is
    /// the device whose entry changed, where the hub says.
    pub rollout: Option<HubRollout>,
    // -- WP P5HUB --
    /// `measure.doc.changed`'s document id ([`HubServerEvent::change`] is
    /// `put`, `rename`, `note` or `delete`; [`HubServerEvent::serial`] the
    /// measured device, "" for a microphone or target).
    pub measure_doc: String,
    /// `measure.doc.changed`'s document kind: `measurement`, `mic` or `target`.
    pub measure_kind: String,
}

/// A snapshot on the hub with its document ([`DspiCore::hub_snapshots_get`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubSnapshotDocument {
    /// Its entry.
    pub snapshot: SnapshotInfo,
    /// The document's text (compact JSON).
    pub document: String,
}

/// A snapshot filed on the hub ([`DspiCore::hub_snapshots_put`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubSnapshotPut {
    /// Its entry on the hub.
    pub snapshot: SnapshotInfo,
    /// False: the hub already held this snapshot with the same content.
    pub created: bool,
}

/// What a recall on the hub did ([`DspiCore::hub_snapshots_recall`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubSnapshotRecall {
    /// The device.
    pub serial: String,
    /// The snapshot's id.
    pub snapshot: String,
    /// Its name.
    pub name: String,
    /// Whether the image was written.
    pub image: bool,
    /// What changed.
    pub lines: Vec<String>,
    /// What was not written, and why.
    pub notes: Vec<String>,
    /// The guarded apply's id with `guard`; "" without one. Confirm it as
    /// any guarded apply, or it reverts.
    pub guard: String,
}

/// One row of the merged snapshot list ([`DspiCore::snapshot_list_merged`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergedSnapshot {
    /// The entry: the hub's where it holds the id (the hub's wins).
    pub snapshot: SnapshotInfo,
    /// [`WHERE_LOCAL`], [`WHERE_HUB`] or [`WHERE_BOTH`] (`where` in JSON).
    #[serde(rename = "where")]
    pub location: String,
    /// This console's own copy of an id the hub holds with different content:
    /// listed beside the hub's, never overwritten by it.
    pub local_copy: bool,
}

/// The snapshots of both sides ([`DspiCore::snapshot_list_merged`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotMergedList {
    /// Oldest first; on an id both sides hold with different content the
    /// hub's row comes first, then the local copy.
    pub entries: Vec<MergedSnapshot>,
    /// The hub's list is in it (a hub with `hub_snapshots` answered).
    pub hub_listed: bool,
    /// Why the hub's list is not in it, as a notice row; "" when it is, or
    /// when there is no such hub.
    pub hub_notice_key: String,
}

// ── Events ─────────────────────────────────────────────────────────────────

/// The hub's events of these families reach the app here (WP P2CC1): set with
/// [`DspiCore::set_hub_server_listener`].
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
pub trait HubServerListener: Send + Sync {
    /// A record was added to a watched history, the inventory or the hub's
    /// snapshots changed. Called on the Link session's supervisor, no core
    /// lock held: post the work to the app's own thread and return.
    fn on_hub_server_event(&self, event: HubServerEvent);
}

/// The listener and the queue of [`DspiCore::hub_server_events`].
#[derive(Default)]
pub(crate) struct Events {
    listener: Slot<dyn HubServerListener>,
    queue: Mutex<VecDeque<HubServerEvent>>,
}

/// The kinds of [`HubServerEvent`].
#[cfg_attr(not(feature = "link"), allow(dead_code))]
const KINDS: [&str; 9] = [
    "event.added",
    "inventory.changed",
    "snapshot.changed",
    // -- WP P2CC2 --
    "alarm.changed",
    "schedule.ran",
    // -- WP P3CC --
    "group.changed",
    "fw.rollout.changed",
    // -- WP P5HUB --
    "measure.doc.changed",
    // -- WP FEATUI -- who works on the devices beside the people (`presence`)
    "presence.changed",
];

impl HubServerEvent {
    /// The event in a hub message of type `t`; `None` for any other message.
    #[cfg_attr(not(feature = "link"), allow(dead_code))]
    pub(crate) fn from_hub(t: &str, body: &Value) -> Option<Self> {
        if !KINDS.contains(&t) {
            return None;
        }
        // -- WP P2CC2 --
        let alarm = (t == "alarm.changed")
            .then(|| {
                body.get("alarm")
                    .filter(|a| a.is_object())
                    .map(admin::alarm_from)
            })
            .flatten();
        let run = (t == "schedule.ran").then(|| admin::run_from(body));
        let serial = match &alarm {
            Some(a) => a.serial.clone(),
            None => text(body.get("serial")),
        };
        Some(Self {
            kind: t.to_owned(),
            serial,
            alarm,
            run,
            record: body
                .get("record")
                .filter(|r| r.is_object())
                .map(record_from),
            device: body
                .get("device")
                .filter(|d| d.is_object())
                .map(device_from),
            snapshot: text(body.get("snapshot")),
            change: text(body.get("change")),
            // -- WP P3CC --
            group: (t == "group.changed")
                .then(|| {
                    body.get("group")
                        .filter(|g| g.is_object())
                        .map(fleet::group_from)
                })
                .flatten(),
            rollout: (t == "fw.rollout.changed")
                .then(|| {
                    body.get("rollout")
                        .filter(|r| r.is_object())
                        .map(fleet::rollout_from)
                })
                .flatten(),
            // -- WP P5HUB --
            measure_doc: if t == "measure.doc.changed" {
                text(body.get("doc"))
            } else {
                String::new()
            },
            measure_kind: if t == "measure.doc.changed" {
                text(body.get("kind"))
            } else {
                String::new()
            },
        })
    }
}

// ── Reading the hub's answers ─────────────────────────────────────────────

fn cut(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn text(v: Option<&Value>) -> String {
    v.and_then(Value::as_str)
        .map(|t| cut(t, TEXT_MAX))
        .unwrap_or_default()
}

fn texts(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .take(ITEMS_MAX)
                .filter_map(Value::as_str)
                .map(|t| cut(t, TEXT_MAX))
                .collect()
        })
        .unwrap_or_default()
}

fn small<T: TryFrom<u64>>(v: Option<&Value>) -> Option<T> {
    v.and_then(Value::as_u64).and_then(|n| T::try_from(n).ok())
}

/// The platform as a name: the hub sends `REQ_GET_PLATFORM`'s id (a number);
/// a name is taken as it is.
fn platform(v: Option<&Value>) -> String {
    match v.and_then(Value::as_u64) {
        Some(id) => u8::try_from(id)
            .map(|id| dspi_proto::Platform::from_id(id).name())
            .unwrap_or_default(),
        None => text(v),
    }
}

fn reset_from(v: &Value) -> HubReset {
    HubReset {
        reason: text(v.get("reason")),
        prev_uptime_s: small(v.get("prev_uptime_s")),
        at: text(v.get("at")),
    }
}

pub(crate) fn device_from(v: &Value) -> HubDevice {
    HubDevice {
        serial: text(v.get("serial")),
        handle: small(v.get("handle")),
        online: v.get("online").and_then(Value::as_bool).unwrap_or(false),
        name: text(v.get("name")),
        platform: platform(v.get("platform")),
        fw: text(v.get("fw")),
        fw_beta: v.get("fw_beta").and_then(Value::as_bool),
        outputs: small(v.get("outputs")).unwrap_or(0),
        board: text(v.get("board")),
        hw_rev: text(v.get("hw_rev")),
        chip: text(v.get("chip")),
        chip_rev: small(v.get("chip_rev")),
        location: text(v.get("location")),
        note: v
            .get("note")
            .and_then(Value::as_str)
            .map(|t| cut(t, NOTE_MAX))
            .unwrap_or_default(),
        tags: texts(v.get("tags")),
        first_seen: text(v.get("first_seen")),
        last_seen: text(v.get("last_seen")),
        last_reset: v
            .get("last_reset")
            .filter(|r| r.is_object())
            .map(reset_from),
    }
}

#[cfg_attr(not(feature = "link"), allow(dead_code))]
fn health_from(v: &Value) -> HubHealth {
    HubHealth {
        serial: text(v.get("serial")),
        handle: small(v.get("handle")).unwrap_or(0),
        at: text(v.get("at")),
        cpu_pct: small(v.get("cpu_pct")),
        clip_mask: small(v.get("clip_mask")),
        temp_c: v.get("temp_c").and_then(Value::as_f64),
        uptime_s: small(v.get("uptime_s")),
        hang_count: small(v.get("hang_count")),
        unresponsive: v
            .get("unresponsive")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        unresponsive_since: text(v.get("unresponsive_since")),
        reset: v.get("reset").filter(|r| r.is_object()).map(reset_from),
        // -- WP P3HB --
        counters: v
            .get("counters")
            .filter(|c| c.is_object())
            .map(|c| HubStreamCounters {
                underruns: small(c.get("underruns")),
                starvation: small(c.get("starvation")),
            }),
        clip_per_min: small(v.get("clip_per_min")),
        last_change: v
            .get("last_change")
            .filter(|c| c.is_object())
            .map(|c| HubLastChange {
                at: text(c.get("at")),
                by: text(c.get("by")),
                text: text(c.get("text")),
            }),
    }
}

#[cfg_attr(not(feature = "link"), allow(dead_code))]
fn stats_from(v: &Value) -> HubStats {
    let row = |d: &Value| HubStatsDevice {
        handle: small(d.get("handle")).unwrap_or(0),
        serial: text(d.get("serial")),
        cmds: d.get("cmds").and_then(Value::as_u64).unwrap_or(0),
        errors: d.get("errors").and_then(Value::as_u64).unwrap_or(0),
        avg_rtt_ms: d.get("avg_rtt_ms").and_then(Value::as_f64).unwrap_or(0.0),
        notify_dropped: d.get("notify_dropped").and_then(Value::as_u64).unwrap_or(0),
    };
    HubStats {
        sessions: small(v.get("sessions")).unwrap_or(0),
        uptime_s: v.get("uptime_s").and_then(Value::as_u64).unwrap_or(0),
        devices: v
            .get("devices")
            .and_then(Value::as_array)
            .map(|a| a.iter().take(256).map(row).collect())
            .unwrap_or_default(),
    }
}

/// A record in the event log's own file format (the hub writes it so).
pub(crate) fn record_from(v: &Value) -> HubEventRecord {
    let r: dspi_session::events::EventRecord =
        serde_json::from_value(v.clone()).unwrap_or_else(|_| {
            dspi_session::events::EventRecord::new(
                dspi_session::events::EventKind::Unknown,
                0,
                text(v.get("text")),
            )
        });
    HubEventRecord {
        id: r.id,
        time_ms: r.t,
        kind: r.kind.as_str().to_owned(),
        serial: cut(&r.serial, TEXT_MAX),
        text: cut(&r.text, NOTE_MAX),
        source: r.source.map(|s| cut(&s, TEXT_MAX)).unwrap_or_default(),
        session: r.session,
        by: r.by.map(|s| cut(&s, TEXT_MAX)).unwrap_or_default(),
        channel: r.channel,
        value: r.value,
        fw: r.fw.map(|s| cut(&s, TEXT_MAX)),
        uptime_s: r.uptime_s,
        lines: r
            .lines
            .into_iter()
            .take(ITEMS_MAX)
            .map(|l| cut(&l, TEXT_MAX))
            .collect(),
        count: r.count,
    }
}

/// A snapshot index entry as the hub sends it (the index's own camelCase).
#[cfg_attr(not(feature = "link"), allow(dead_code))]
pub(crate) fn entry_from(v: &Value) -> SnapshotInfo {
    let e: dspi_session::snapshot_store::SnapshotEntry =
        serde_json::from_value(v.clone()).unwrap_or_default();
    SnapshotInfo {
        id: cut(&e.id, TEXT_MAX),
        name: cut(&e.name, TEXT_MAX),
        note: cut(&e.note, NOTE_MAX),
        serial: cut(&e.serial, TEXT_MAX),
        saved_ms: e.saved_ms,
        platform: cut(&e.platform, TEXT_MAX),
        input_channels: e.input_channels,
        output_channels: e.output_channels,
        wire_format: e.wire_format,
    }
}

#[cfg_attr(not(feature = "link"), allow(dead_code))]
fn recall_from(v: &Value) -> HubSnapshotRecall {
    let guard = match v.get("guard") {
        Some(Value::String(s)) => cut(s, TEXT_MAX),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    HubSnapshotRecall {
        serial: text(v.get("serial")),
        snapshot: text(v.get("snapshot")),
        name: text(v.get("name")),
        image: v.get("image").and_then(Value::as_bool).unwrap_or(false),
        lines: texts(v.get("lines")),
        notes: texts(v.get("notes")),
        guard,
    }
}

/// The local refusal of a document over [`SNAPSHOT_WIRE_MAX_BYTES`]: the
/// hub's `conflict`, in the hub's own words.
pub(crate) fn over_wire(len: usize) -> DspiError {
    DspiError::Link {
        code: HUB_ERR_CONFLICT.into(),
        msg: format!(
            "a snapshot document is at most {SNAPSHOT_WIRE_MAX_BYTES} bytes on the DSPi Link \
             (compact JSON); this one is {len}"
        ),
    }
}

/// `text` as a snapshot document in its compact form, measured against the
/// wire cap before anything is sent.
pub(crate) fn wire_document(text: &str) -> Result<Value> {
    let doc: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| {
        DspiError::InvalidArgument {
            msg: format!("not a snapshot document: {e}"),
        }
    })?;
    if !doc.is_object() {
        return Err(DspiError::InvalidArgument {
            msg: "a snapshot document is a JSON object".into(),
        });
    }
    let len = serde_json::to_string(&doc).map(|t| t.len()).unwrap_or(0);
    if len > SNAPSHOT_WIRE_MAX_BYTES {
        return Err(over_wire(len));
    }
    Ok(doc)
}

// -- WP P2WB2 --
/// `doc` with a store id in its snapshot block: a document captured with no
/// store (the web's `snapshot_capture_document`) has none, and the hub files
/// a snapshot only under one. It gets the id a store would give it first,
/// from its capture time (`snapshot_store::id_for`); one that has an id
/// keeps it.
#[cfg_attr(not(feature = "link"), allow(dead_code))]
pub(crate) fn with_store_id(mut doc: Value) -> Value {
    use dspi_session::snapshot_store::{BLOCK, id_for, valid_id};
    if let Some(b) = doc.get_mut(BLOCK).and_then(Value::as_object_mut) {
        let has = b.get("id").and_then(Value::as_str).is_some_and(valid_id);
        if !has {
            let ms = b.get("savedMs").and_then(Value::as_i64).unwrap_or(0);
            b.insert("id".into(), id_for(ms, 0).into());
        }
    }
    doc
}

/// The hub's refusals as the core's kinds (module docs).
#[cfg_attr(not(feature = "link"), allow(dead_code))]
pub(crate) fn hub_error(e: DspiError) -> DspiError {
    match e {
        DspiError::Link { code, msg } => {
            // The hub's own words, as it sent them (the client's text puts
            // "hub refused: <code>" before them).
            let msg = msg
                .strip_prefix(&format!("hub refused: {code} "))
                .map_or_else(|| msg.clone(), str::to_owned);
            hub_code(code, msg)
        }
        other => other,
    }
}

/// The hub's refusal `code` with its words `msg`, typed.
fn hub_code(code: String, msg: String) -> DspiError {
    match code.as_str() {
        "denied" => DspiError::Denied,
        "unsupported" | "unknown_type" => DspiError::Unsupported,
        "locked" => DspiError::Locked,
        "bad_request" => DspiError::InvalidArgument { msg },
        "no_device" => DspiError::Disconnected,
        "limit_exceeded" => DspiError::LimitExceeded {
            code: "limit".into(),
            msg,
        },
        "installer_lock" => DspiError::InstallerLock {
            code: "hub".into(),
            msg,
        },
        "image_refused" => DspiError::ImageRefused { msg },
        "limits_unknown" => DspiError::LimitsUnknown { msg },
        "confirmation_required" => DspiError::ConfirmationRequired {
            code: "hub".into(),
            msg,
        },
        _ => DspiError::Link { code, msg },
    }
}

/// Which caps of [`HubCaps`] a list of caps has.
fn caps_of(on_hub: bool, caps: Vec<String>) -> HubCaps {
    let has = |c: &str| caps.iter().any(|x| x == c);
    HubCaps {
        on_hub,
        inventory: has(CAP_INVENTORY),
        identify: has(CAP_IDENTIFY),
        health: has(CAP_HEALTH),
        events: has(CAP_EVENTS),
        hub_snapshots: has(CAP_HUB_SNAPSHOTS),
        param_api: has(CAP_PARAM_API),
        hub_stats: has(CAP_HUB_STATS),
        // -- WP P2CC2 --
        alarms: has(CAP_ALARMS),
        outbound: has(CAP_OUTBOUND),
        schedule: has(CAP_SCHEDULE),
        mqtt: has(CAP_MQTT),
        api_keys: has(CAP_API_KEYS),
        rest: has(CAP_REST),
        // -- WP P3CC --
        groups: has(CAP_GROUPS),
        group_params: has(CAP_GROUP_PARAMS),
        group_apply: has(CAP_GROUP_APPLY),
        rollout: has(CAP_ROLLOUT),
        // -- WP P5HUB --
        hub_measure: has(CAP_HUB_MEASURE),
        caps,
    }
}

/// Two entries of one id are the same snapshot: every field the index keeps
/// agrees (the hub compares whole documents; a rename or a new note on one
/// side is a difference, and capture-time ids make the rest the same
/// capture).
fn same(a: &SnapshotInfo, b: &SnapshotInfo) -> bool {
    a == b
}

/// Both sides' lists as one (module docs): the hub's wins on an id both hold.
pub(crate) fn merge(local: Vec<SnapshotInfo>, hub: Vec<SnapshotInfo>) -> Vec<MergedSnapshot> {
    let mut out: Vec<MergedSnapshot> = Vec::with_capacity(local.len().saturating_add(hub.len()));
    let mut local: Vec<Option<SnapshotInfo>> = local.into_iter().map(Some).collect();
    for h in hub {
        let mine = local
            .iter_mut()
            .find(|l| l.as_ref().is_some_and(|l| l.id == h.id))
            .and_then(Option::take);
        match mine {
            Some(l) if same(&l, &h) => out.push(MergedSnapshot {
                snapshot: h,
                location: WHERE_BOTH.into(),
                local_copy: false,
            }),
            Some(l) => {
                out.push(MergedSnapshot {
                    snapshot: h,
                    location: WHERE_HUB.into(),
                    local_copy: false,
                });
                out.push(MergedSnapshot {
                    snapshot: l,
                    location: WHERE_LOCAL.into(),
                    local_copy: true,
                });
            }
            None => out.push(MergedSnapshot {
                snapshot: h,
                location: WHERE_HUB.into(),
                local_copy: false,
            }),
        }
    }
    out.extend(local.into_iter().flatten().map(|l| MergedSnapshot {
        snapshot: l,
        location: WHERE_LOCAL.into(),
        local_copy: false,
    }));
    // Stable: the hub's row stays before its local copy.
    out.sort_by(|a, b| {
        (a.snapshot.saved_ms, &a.snapshot.id).cmp(&(b.snapshot.saved_ms, &b.snapshot.id))
    });
    out
}

// ── The calls ──────────────────────────────────────────────────────────────

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Which of the hub's families this console's hub has, from its `hello`.
    /// All false with no live DSPi Link session. No I/O.
    pub fn hub_caps(&self) -> HubCaps {
        #[cfg(feature = "link")]
        if let Some(Some(caps)) = self.link_hub_caps() {
            return caps_of(true, caps);
        }
        caps_of(false, Vec::new())
    }

    /// Every DSPi the hub has seen, online or not: online first, then by
    /// last seen, newest first (`inventory.list`, read page by page, at most
    /// 1024). Every role. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Unsupported`] without the `inventory` cap (nothing
    /// sent); [`DspiError::Disconnected`]; the hub's refusal (module docs).
    pub fn hub_inventory(&self) -> Result<Vec<HubDevice>> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_INVENTORY)?;
            let mut out: Vec<HubDevice> = Vec::new();
            let mut offset: u64 = 0;
            for _ in 0..PAGES_MAX {
                let mut f = serde_json::Map::new();
                f.insert("offset".into(), offset.into());
                f.insert("limit".into(), 500.into());
                let page = self.hub_ask("inventory.list", f)?;
                if let Some(a) = page.get("devices").and_then(Value::as_array) {
                    out.extend(
                        a.iter()
                            .take(DEVICES_MAX.saturating_sub(out.len()))
                            .map(device_from),
                    );
                }
                match page.get("next_offset").and_then(Value::as_u64) {
                    Some(next) if next > offset && out.len() < DEVICES_MAX => offset = next,
                    _ => break,
                }
            }
            Ok(out)
        }
        #[cfg(not(feature = "link"))]
        Err(DspiError::Unsupported)
    }

    /// Identify the open device: blink its LED for `seconds` (0 stops it;
    /// at most 120) and, with `tone_output`, play its quiet identify tone
    /// once on that output (leave it empty unless the user asked for sound).
    /// On a hub with the `identify` cap this is the hub's `device.identify`
    /// (Control or Admin; the hub sends it, at most once per device per 2 s);
    /// connected directly it is the device's own `EXT_IDENTIFY`
    /// ([`DspiCore::identify`]). The answer is the device's state read after
    /// the write; `None` when that read failed (the write went out). Slow:
    /// off the UI thread.
    ///
    /// Errors: [`DspiError::Unsupported`] (nothing sent) on a hub without the
    /// cap and on firmware without `EXT_IDENTIFY`;
    /// [`DspiError::InvalidArgument`] for a tone output the device does not
    /// have; [`DspiError::Locked`] for a tone under another session's lock;
    /// [`DspiError::Link`] `rate_limited`, `offline`, `not_found`, `device`;
    /// [`DspiError::ReadOnly`] in read-only mode; as
    /// [`DspiCore::identify`] directly.
    pub fn hub_identify(
        &self,
        seconds: u8,
        tone_output: Option<u8>,
    ) -> Result<Option<IdentifyInfo>> {
        #[cfg(feature = "link")]
        if let Some(t) = self.link_admin_target() {
            if !t.live {
                return Err(DspiError::Disconnected);
            }
            self.hub_cap(CAP_IDENTIFY)?;
            // -- WP P2CC3 -- shared with `hub_identify_serial`
            return self.hub_identify_on(t.serial, seconds, tone_output);
        }
        self.identify(seconds, tone_output)?;
        Ok(self.fetch_identify().ok())
    }

    /// One health record per online device of the hub (`health.get`):
    /// CPU, the clip latch, temperature, uptime, hangs, whether it answers;
    /// -- WP P3HB -- the stream counters, clip events a minute and the last
    /// change where the hub has them.
    /// Every role. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Unsupported`] without the `health` cap;
    /// [`DspiError::Disconnected`]; the hub's refusal.
    pub fn hub_health(&self) -> Result<Vec<HubHealth>> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_HEALTH)?;
            let reply = self.hub_ask("health.get", serde_json::Map::new())?;
            Ok(reply
                .get("devices")
                .and_then(Value::as_array)
                .map(|a| a.iter().take(DEVICES_MAX).map(health_from).collect())
                .unwrap_or_default())
        }
        #[cfg(not(feature = "link"))]
        Err(DspiError::Unsupported)
    }

    /// The hub's statistics (`hub.stats`): sessions, uptime, and per attached
    /// device its commands, errors, mean time and dropped notifications.
    /// Admin of the hub only. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Unsupported`] without the `hub_stats` cap;
    /// [`DspiError::Denied`] below Admin; [`DspiError::Disconnected`].
    pub fn hub_stats(&self) -> Result<HubStats> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_HUB_STATS)?;
            let reply = self.hub_ask("hub.stats", serde_json::Map::new())?;
            Ok(stats_from(&reply))
        }
        #[cfg(not(feature = "link"))]
        Err(DspiError::Unsupported)
    }

    /// Device `serial`'s history on the hub ("" the open device), with ids
    /// above `after_id`, oldest first, at most `limit` (capped at 500; 0
    /// is the hub's 100): with `after_id` 0 the newest page
    /// (`event.list`). Every role. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Unsupported`] without the `events` cap;
    /// [`DspiError::Link`] `not_found` for a device with no log on the hub;
    /// [`DspiError::Disconnected`].
    pub fn hub_events_read(
        &self,
        serial: String,
        after_id: u64,
        limit: u32,
    ) -> Result<HubEventPage> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_EVENTS)?;
            let serial = self.hub_serial(serial)?;
            let mut f = serde_json::Map::new();
            f.insert("serial".into(), serial.clone().into());
            f.insert("after_id".into(), after_id.into());
            if limit > 0 {
                f.insert("limit".into(), limit.min(EVENTS_MAX).into());
            }
            let reply = self.hub_ask("event.list", f)?;
            Ok(HubEventPage {
                serial: reply
                    .get("serial")
                    .and_then(Value::as_str)
                    .map_or(serial, |s| cut(s, TEXT_MAX)),
                records: reply
                    .get("records")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .take(usize::try_from(EVENTS_MAX).unwrap_or(500))
                            .map(record_from)
                            .collect()
                    })
                    .unwrap_or_default(),
                more: reply.get("more").and_then(Value::as_bool).unwrap_or(false),
                newest_id: reply.get("newest_id").and_then(Value::as_u64).unwrap_or(0),
            })
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = (serial, after_id, limit);
            Err(DspiError::Unsupported)
        }
    }

    /// Have the hub send each new record of device `serial`'s history ("":
    /// every device) as `event.added` (`on`), or stop (`event.watch`). The
    /// records reach [`HubServerListener`] and [`DspiCore::hub_server_events`].
    /// A watch ends with the hub session: ask again after a reconnect. Every
    /// role. Slow: off the UI thread.
    ///
    /// Errors: as [`DspiCore::hub_events_read`].
    pub fn hub_events_watch(&self, on: bool, serial: String) -> Result<()> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_EVENTS)?;
            let mut f = serde_json::Map::new();
            f.insert("on".into(), on.into());
            if !serial.is_empty() {
                f.insert("serial".into(), serial.into());
            }
            self.hub_ask("event.watch", f).map(|_| ())
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = (on, serial);
            Err(DspiError::Unsupported)
        }
    }

    /// Register (`Some`) or clear (`None`) the listener for the hub's
    /// `event.added`, `inventory.changed` and `snapshot.changed`.
    pub fn set_hub_server_listener(&self, listener: Option<Arc<dyn HubServerListener>>) {
        self.listeners.hub_server.listener.set(listener);
    }

    /// The hub's events of these families since the last call, oldest first
    /// (at most 64 are kept), for a binding that polls instead of a listener
    /// (the web). No I/O.
    pub fn hub_server_events(&self) -> Vec<HubServerEvent> {
        self.listeners
            .hub_server
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }

    /// Device `serial`'s snapshots on the hub ("" the open device), oldest
    /// first (`snapshot.list`). Every role. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Unsupported`] without the `hub_snapshots` cap;
    /// [`DspiError::Disconnected`]; the hub's refusal.
    pub fn hub_snapshots_list(&self, serial: String) -> Result<Vec<SnapshotInfo>> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_HUB_SNAPSHOTS)?;
            let serial = self.hub_serial(serial)?;
            let mut f = serde_json::Map::new();
            f.insert("serial".into(), serial.into());
            let reply = self.hub_ask("snapshot.list", f)?;
            Ok(reply
                .get("snapshots")
                .and_then(Value::as_array)
                .map(|a| a.iter().take(1024).map(entry_from).collect())
                .unwrap_or_default())
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = serial;
            Err(DspiError::Unsupported)
        }
    }

    /// Snapshot `id` on the hub with its document (`snapshot.get`). Every
    /// role. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Link`] `not_found`; `conflict` for a document
    /// over 61 440 bytes compact (the hub's words); as
    /// [`DspiCore::hub_snapshots_list`].
    pub fn hub_snapshots_get(&self, id: String) -> Result<HubSnapshotDocument> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_HUB_SNAPSHOTS)?;
            let mut f = serde_json::Map::new();
            f.insert("snapshot".into(), id.into());
            let reply = self.hub_ask("snapshot.get", f)?;
            let document = match reply.get("document") {
                Some(Value::String(s)) => s.clone(),
                Some(v @ Value::Object(_)) => v.to_string(),
                _ => {
                    return Err(DspiError::Link {
                        code: "protocol".into(),
                        msg: "the hub's answer has no document".into(),
                    });
                }
            };
            // Measured as the hub measures it: a larger one never reaches a client.
            wire_document(&document)?;
            Ok(HubSnapshotDocument {
                snapshot: reply
                    .get("snapshot")
                    .map(entry_from)
                    .unwrap_or_else(|| entry_from(&Value::Null)),
                document,
            })
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = id;
            Err(DspiError::Unsupported)
        }
    }

    /// File snapshot `document` (its text: a snapshot document of device
    /// `serial`, "" for the serial its `snapshot` block names) on the hub
    /// (`snapshot.put`), under the id the document carries. The same document
    /// again is `created: false`. Control or Admin of the hub. Slow: off the
    /// UI thread.
    ///
    /// A document over 61 440 bytes in its compact JSON form is refused here,
    /// with nothing sent: [`DspiError::Link`] `conflict`, "a snapshot document
    /// is at most 61440 bytes on the DSPi Link (compact JSON); this one is N",
    /// as the hub would answer.
    ///
    /// Errors: that; [`DspiError::InvalidArgument`] for text that is not a
    /// JSON object, or the hub's `bad_request` (not a snapshot of that
    /// device); [`DspiError::Link`] `conflict` (the hub holds a different
    /// snapshot with this id, or a cap is reached); [`DspiError::Denied`]
    /// for a Viewer; [`DspiError::ReadOnly`]; as
    /// [`DspiCore::hub_snapshots_list`].
    pub fn hub_snapshots_put(&self, serial: String, document: String) -> Result<HubSnapshotPut> {
        #[cfg(feature = "link")]
        {
            // Measured first: too large is refused here whatever the hub.
            let doc = wire_document(&document)?;
            self.hub_cap(CAP_HUB_SNAPSHOTS)?;
            let doc = with_store_id(doc); // -- WP P2WB2 --
            let serial = if serial.is_empty() {
                doc.get("snapshot")
                    .and_then(|b| b.get("serial"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            } else {
                serial
            };
            self.hub_put(serial, doc)
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = serial;
            wire_document(&document)?;
            Err(DspiError::Unsupported)
        }
    }

    /// Rename snapshot `id` on the hub (`snapshot.rename`; at most 256
    /// characters). Control or Admin. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Link`] `not_found`; as
    /// [`DspiCore::hub_snapshots_put`].
    pub fn hub_snapshots_rename(&self, id: String, name: String) -> Result<SnapshotInfo> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_HUB_SNAPSHOTS)?;
            let mut f = serde_json::Map::new();
            f.insert("snapshot".into(), id.into());
            f.insert("name".into(), name.into());
            let reply = self.hub_ask("snapshot.rename", f)?;
            Ok(reply
                .get("snapshot")
                .map(entry_from)
                .unwrap_or_else(|| entry_from(&Value::Null)))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = (id, name);
            Err(DspiError::Unsupported)
        }
    }

    /// Set snapshot `id`'s note on the hub (`snapshot.note`; at most 4096
    /// characters). Control or Admin. Slow: off the UI thread.
    ///
    /// Errors: as [`DspiCore::hub_snapshots_rename`].
    pub fn hub_snapshots_note(&self, id: String, note: String) -> Result<SnapshotInfo> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_HUB_SNAPSHOTS)?;
            let mut f = serde_json::Map::new();
            f.insert("snapshot".into(), id.into());
            f.insert("note".into(), note.into());
            let reply = self.hub_ask("snapshot.note", f)?;
            Ok(reply
                .get("snapshot")
                .map(entry_from)
                .unwrap_or_else(|| entry_from(&Value::Null)))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = (id, note);
            Err(DspiError::Unsupported)
        }
    }

    /// Delete snapshot `id` from the hub (`snapshot.delete`). Admin of the
    /// hub only. This console's own copy, if any, stays. Slow: off the UI
    /// thread.
    ///
    /// Errors: [`DspiError::Denied`] below Admin; as
    /// [`DspiCore::hub_snapshots_rename`].
    pub fn hub_snapshots_delete(&self, id: String) -> Result<()> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_HUB_SNAPSHOTS)?;
            let mut f = serde_json::Map::new();
            f.insert("snapshot".into(), id.into());
            self.hub_ask("snapshot.delete", f).map(|_| ())
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = id;
            Err(DspiError::Unsupported)
        }
    }

    /// Recall snapshot `id` from the hub's store on device `serial` ("" the
    /// open device), written by the hub as this console's session
    /// (`snapshot.recall`). With `guard` a guarded apply begins first
    /// (`timeout_ms`, 0 for the hub's 30 s): confirm it, or it reverts.
    /// Control or Admin. Slow: off the UI thread.
    ///
    /// WP P2CC3: a guarded recall is this core's guarded apply, as one begun
    /// here: [`DspiCore::apply_status`] reports it (`hub: true`, the
    /// recall's changes), [`DspiCore::apply_confirm`] keeps it and
    /// [`DspiCore::apply_revert`] (or the timeout, through
    /// [`DspiCore::apply_poll`]) has the hub put back what it captured, as
    /// this session. While another guarded apply waits it is refused with
    /// [`DspiError::InvalidArgument`], nothing sent.
    ///
    /// Errors: [`DspiError::ImageRefused`] for Control on a device with a
    /// limits profile or a lock (nothing written); [`DspiError::Link`]
    /// `offline`, `not_found`, `conflict` (another session's guarded apply);
    /// as [`DspiCore::hub_snapshots_put`].
    pub fn hub_snapshots_recall(
        &self,
        serial: String,
        id: String,
        guard: bool,
        timeout_ms: u32,
    ) -> Result<HubSnapshotRecall> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_HUB_SNAPSHOTS)?;
            let serial = self.hub_serial(serial)?;
            let mut f = serde_json::Map::new();
            f.insert("serial".into(), serial.into());
            f.insert("snapshot".into(), id.into());
            if guard {
                f.insert("guard".into(), true.into());
                if timeout_ms > 0 {
                    f.insert("timeout_ms".into(), timeout_ms.into());
                }
            }
            if !guard {
                let reply = self.hub_ask("snapshot.recall", f)?;
                return Ok(recall_from(&reply));
            }
            // -- WP P2CC3 -- the hub's guard is this core's guarded apply:
            // apply_status shows it, apply_confirm / apply_revert end it
            self.guard_adopt_recall(timeout_ms, || {
                self.hub_ask("snapshot.recall", f).map(|r| recall_from(&r))
            })
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = (serial, id, guard, timeout_ms);
            Err(DspiError::Unsupported)
        }
    }

    /// "Save to the hub": file this console's snapshot `id` (its own store,
    /// [`DspiCore::snapshot_set_dir`]) on the hub, under the same id and the
    /// device it was taken from. Errors: as [`DspiCore::hub_snapshots_put`],
    /// and as [`DspiCore::snapshot_export`] for the local read.
    pub fn hub_snapshots_save(&self, id: String) -> Result<HubSnapshotPut> {
        #[cfg(feature = "link")]
        self.hub_cap(CAP_HUB_SNAPSHOTS)?;
        let text = self.snapshot_export(id)?;
        self.hub_snapshots_put(String::new(), text)
    }

    /// "Copy here": file the hub's snapshot `id` in this console's own store.
    /// Kept as the hub has it, under its capture-time id unless this store
    /// already holds that id with other content (then the copy gets the
    /// next free one: nothing is overwritten); the same snapshot already here
    /// is answered as it is. Errors: as [`DspiCore::hub_snapshots_get`], and
    /// [`DspiError::Io`] with no folder named for snapshots.
    pub fn hub_snapshots_copy_here(&self, id: String) -> Result<SnapshotInfo> {
        let got = self.hub_snapshots_get(id)?;
        let serial = got.snapshot.serial.clone();
        if let Some(mine) = self
            .snapshot_list(serial.clone())
            .into_iter()
            .find(|l| l.id == got.snapshot.id)
            && same(&mine, &got.snapshot)
        {
            return Ok(mine);
        }
        self.snapshot_file_copy(&serial, &got.document)
    }
}

impl DspiCore {
    /// An event of the hub's: kept and told. On the Link supervisor, with no
    /// core lock held.
    #[cfg_attr(not(feature = "link"), allow(dead_code))]
    pub(crate) fn hub_server_event(&self, event: HubServerEvent) {
        {
            let mut q = self
                .listeners
                .hub_server
                .queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if q.len() >= QUEUE_MAX {
                q.pop_front();
            }
            q.push_back(event.clone());
        }
        self.listeners
            .hub_server
            .listener
            .call(|l| l.on_hub_server_event(event));
    }

    /// The hub's snapshot list of `serial` for the merged list: `Ok(None)`
    /// when there is no hub with the store (nothing sent).
    pub(crate) fn hub_snapshots_for_merge(
        &self,
        serial: &str,
    ) -> Result<Option<Vec<SnapshotInfo>>> {
        match self.hub_snapshots_list(serial.to_owned()) {
            Ok(list) => Ok(Some(list)),
            Err(DspiError::Unsupported) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(feature = "link")]
impl DspiCore {
    /// Ok when this console's live hub announces `cap`; nothing is sent.
    fn hub_cap(&self, cap: &str) -> Result<()> {
        match self.link_hub_caps() {
            None => Err(DspiError::Unsupported),
            Some(None) => Err(DspiError::Disconnected),
            Some(Some(caps)) if caps.iter().any(|c| c == cap) => Ok(()),
            Some(Some(_)) => Err(DspiError::Unsupported),
        }
    }

    /// `t` with `fields` to the hub; its `ok` whole, or its refusal typed.
    fn hub_ask(&self, t: &str, fields: serde_json::Map<String, Value>) -> Result<Value> {
        self.link_hub_request(t, fields).map_err(hub_error)
    }

    /// `serial`, or the open device's for "".
    fn hub_serial(&self, serial: String) -> Result<String> {
        if !serial.is_empty() {
            return Ok(serial);
        }
        self.lock()
            .conn
            .as_ref()
            .map(|c| c.serial.clone())
            .filter(|s| !s.is_empty())
            .ok_or(DspiError::Disconnected)
    }

    // -- WP P2CC3 --
    /// `device.identify` of `serial` (the cap checked by the caller): the
    /// status read after the write, `None` when that read failed.
    fn hub_identify_on(
        &self,
        serial: String,
        seconds: u8,
        tone_output: Option<u8>,
    ) -> Result<Option<IdentifyInfo>> {
        let mut f = serde_json::Map::new();
        f.insert("serial".into(), serial.into());
        f.insert("seconds".into(), seconds.into());
        if let Some(o) = tone_output {
            f.insert("tone_output".into(), o.into());
        }
        let reply = self.hub_ask("device.identify", f)?;
        Ok(reply
            .get("status")
            .filter(|s| s.is_object())
            .map(|s| IdentifyInfo {
                remaining_s: small(s.get("remaining_s")).unwrap_or(0),
                last_tone_output: small(s.get("last_tone")),
                chip: text(s.get("chip")),
                chip_rev: small(s.get("chip_rev")).unwrap_or(0),
                chip_id: 0,
                board: text(s.get("board")),
                hw_rev: text(s.get("hw_rev")),
            }))
    }

    /// `snapshot.put` of a measured document.
    fn hub_put(&self, serial: String, doc: Value) -> Result<HubSnapshotPut> {
        let mut f = serde_json::Map::new();
        f.insert("serial".into(), serial.into());
        f.insert("document".into(), doc);
        let reply = self.hub_ask("snapshot.put", f)?;
        Ok(HubSnapshotPut {
            snapshot: reply
                .get("snapshot")
                .map(entry_from)
                .unwrap_or_else(|| entry_from(&Value::Null)),
            created: reply
                .get("created")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        })
    }
}

#[cfg(test)]
#[path = "hub_server/tests_wp_p2cc1.rs"]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p2cc1;
// -- WP P2WB2 --
#[cfg(test)]
#[path = "hub_server/tests_wp_p2wb2.rs"]
mod tests_wp_p2wb2;
