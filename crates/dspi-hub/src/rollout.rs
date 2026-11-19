//! The firmware rollout (WP P3RO, PLAN-P3 F6 and section 1.1, `docs/integrator/hub-api.md`
//! section 21): the hub updates the firmware of a DSPi on its own USB with nobody at the
//! rack, one device at a time, its settings exported first, the firmware guard asked with
//! nobody to type, the device verified after it restarts (the 1.20 V core setting read
//! back on a board that needs it), and an alarm when anything fails.
//!
//! | Message | Role | What |
//! |---|---|---|
//! | `fw.image.put {name, seq, data, last}` | Admin | a chunked upload into the image store ([`images`]) |
//! | `fw.image.list` | every role | the stored images |
//! | `fw.image.delete {image}` | Admin | remove one (`conflict` while a rollout uses it) |
//! | `fw.rollout.start {serial or handle or group, image, while_live?}` | Admin | steps 1 and 2 of the first device, then the rest on the rollout's thread |
//! | `fw.rollout.status {rollout?}` | every role | the records, newest first |
//! | `fw.rollout.cancel {rollout}` | Admin | before the bootloader entry of the device it is at; `conflict` from there on |
//!
//! **TLS only** (OD-P3n): every `fw.*` message over a connection that is not TLS answers
//! `tls_required` and does nothing, in every TLS mode, whatever the token or the key;
//! `fw.rollout.changed` goes to encrypted sessions only. The cap [`CAP`] is announced only
//! on a hub that serves TLS ([`RolloutConfig::serves_tls`], set by the hub).
//!
//! **Every device action is a principal's** (the session-id rule, OD-P3g): a rollout
//! opens one Admin principal with the starting Admin's client and name, and every step
//! goes through the parameter API's executor as that principal's child session, so the
//! role, the device lock and the voltage rule judge it as any console's write, the history
//! names the Admin, and the hub's own writes stay the closed list of three: nothing here
//! submits as `INTERNAL_SESSION_ID` (a test reads this file for it). The writer talks to
//! the boot ROM only ([`Writer`], over P3PB's `picoboot_flash_confirmed`), and the hub
//! never writes the core voltage: it reads it back.
//!
//! **The locks:** the installer lock (P1HL) judges the bootloader entry as any write, so
//! the Admin principal passes it, as an Admin's write does; a device under any session's
//! device lock is `locked`; any pending guarded apply is `conflict`. A rollout is never
//! retried and nothing is recalled after it.
//!
//! Records live in `<data>/rollout.json` (the newest [`MAX_RECORDS`]), atomic, 0600, the
//! corrupt-file rule (`rollout.json.bad-<time>`). The images: [`images`].
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

pub mod images;
mod steps;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use data_encoding::BASE64;
use dspi_core::{BootDevice, ConfirmOp, DspiCore, FirmwareGuardBoard, FlashReport};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::{ApiError, err, ok};
use crate::bus::{Bus, BusMessage, Subscription};
use crate::clock::HubClock;
use crate::hub::HubHandle;
use crate::inventory::iso8601;
use crate::service::{HubService, ServiceConfig, ServiceSetup};
use crate::session::{Outbound, Role, Session, SessionId};
use crate::snapshots::HubSnapshots;

use images::{ImageInfo, ImageStore};

/// The family's cap in `hello.caps`.
pub const CAP: &str = "rollout";
/// The records' file in the hub's data folder.
pub const FILE_NAME: &str = "rollout.json";
/// The event every encrypted session gets at each step.
pub const CHANGED_EVENT: &str = "fw.rollout.changed";
/// The messages of the family.
pub const MESSAGES: [&str; 6] = [
    "fw.image.put",
    "fw.image.list",
    "fw.image.delete",
    "fw.rollout.start",
    "fw.rollout.status",
    "fw.rollout.cancel",
];
/// Records kept.
pub const MAX_RECORDS: usize = 64;
/// The most base64 characters in one upload chunk (36 KiB of image).
pub const MAX_CHUNK_CHARS: usize = 49_152;
/// An upload not finished this long after its first chunk is dropped.
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(120);
/// Uploads open at once on the hub (each holds its image in memory).
pub const MAX_UPLOADS: usize = 2;
/// How often the BOOTSEL boards are looked for after the bootloader entry.
pub const FIND_EVERY: Duration = Duration::from_millis(500);
/// How long the board has to appear in BOOTSEL.
pub const FIND_FOR: Duration = Duration::from_secs(15);
/// How long the device has to come back after the flash.
pub const BACK_WITHIN: Duration = Duration::from_secs(60);
/// The records' file is never longer: the oldest records go first past it.
const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
/// A step's text, or any text from a device, is cut to this.
const MAX_TEXT_CHARS: usize = 256;
/// What the health subscription holds.
const BUS_CAPACITY: usize = 512;
/// `"v"` of the file, the answers and the event.
const VERSION: u8 = 1;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every section under these locks leaves its data whole before it could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn cut(text: &str) -> String {
    text.chars().take(MAX_TEXT_CHARS).collect()
}

// ── The seams ───────────────────────────────────────────────────────────────

/// Where the boards in BOOTSEL are found: the core's [`dspi_core::picoboot_devices`]
/// ([`CoreScan`]), or a test's fake.
pub trait BootScan: Send + Sync {
    /// Every board in BOOTSEL mode on the hub's USB.
    fn boards(&self) -> Vec<BootDevice>;
}

/// What writes the image into a board in BOOTSEL: the core's PICOBOOT writer
/// ([`CoreWriter`]), or a test's fake. `board` is the guard's record read before the
/// board entered BOOTSEL; the writer refuses anything the guard would not clear with
/// nothing typed.
pub trait Writer: Send + Sync {
    /// Flash `image` into `target`; the error is one line for the record.
    fn flash(
        &self,
        image: &[u8],
        target: &BootDevice,
        board: &FirmwareGuardBoard,
    ) -> Result<FlashReport, String>;
}

/// [`BootScan`] over the core's PICOBOOT enumeration (`2E8A:0003`, `2E8A:000F`).
#[derive(Debug, Default, Clone, Copy)]
pub struct CoreScan;

impl BootScan for CoreScan {
    fn boards(&self) -> Vec<BootDevice> {
        dspi_core::picoboot_devices()
    }
}

/// [`Writer`] over the core's `picoboot_flash_confirmed`, on a core of its own with no
/// device open: the token is minted and spent inside, as the parameter API's
/// `"confirm": true` does (P1TK's discipline), and the guard runs again in the writer.
#[derive(Debug, Default, Clone, Copy)]
pub struct CoreWriter;

impl Writer for CoreWriter {
    fn flash(
        &self,
        image: &[u8],
        target: &BootDevice,
        board: &FirmwareGuardBoard,
    ) -> Result<FlashReport, String> {
        let core = DspiCore::new();
        let token = core.confirm_token(ConfirmOp::FirmwareFlash);
        core.picoboot_flash_confirmed(image.to_vec(), target.clone(), board.clone(), Some(token))
            .map_err(|e| e.to_string())
    }
}

/// How the rollout runs: [`crate::ServiceConfig::rollout`].
#[derive(Clone, Default)]
pub struct RolloutConfig {
    /// Flash images the release's manifest does not list (`--allow-unverified-firmware`,
    /// the owner's bench switch; OD-P3e). Off: only `verified` images go on a device.
    pub allow_unverified_firmware: bool,
    /// The BOOTSEL scan; `None` is [`CoreScan`].
    pub boot_scan: Option<Arc<dyn BootScan>>,
    /// The writer; `None` is [`CoreWriter`].
    pub writer: Option<Arc<dyn Writer>>,
    /// Whether the hub serves TLS. **Set by the hub** from its network configuration when
    /// it starts (whatever is given is replaced): without TLS there is no rollout.
    pub serves_tls: bool,
}

impl std::fmt::Debug for RolloutConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RolloutConfig")
            .field("allow_unverified_firmware", &self.allow_unverified_firmware)
            .field("boot_scan", &self.boot_scan.is_some())
            .field("writer", &self.writer.is_some())
            .field("serves_tls", &self.serves_tls)
            .finish()
    }
}

// ── Records ─────────────────────────────────────────────────────────────────

/// One step done (`steps` of a device's entry).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepDone {
    /// `check`, `guard`, `export`, `bootloader`, `find`, `flash`, `wait`, `verify`.
    pub step: String,
    /// When, ISO-8601.
    pub at: String,
    /// Whether it went through.
    pub ok: bool,
    /// One line for logs and people.
    pub text: String,
    /// The session the step's device request went out as (the bootloader entry: the
    /// principal's child, from the remote range).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<u32>,
}

/// The guard's board record, as kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardRecord {
    /// The lowest core voltage the board may be left on, mV (0 unknown).
    pub required_mv: u16,
    /// Where it came from (`extCoreVreg`, `statusBlock`, ...).
    pub source: String,
}

/// `EXT_CORE_VREG` as read, mV.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VregRecord {
    /// The live setting.
    pub live_mv: Option<u16>,
    /// The stored selection (the build default when none is stored).
    pub stored_mv: Option<u16>,
    /// The build default.
    pub default_mv: Option<u16>,
}

/// What a device was before the flash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Before {
    /// Its firmware version.
    pub fw: String,
    /// `EXT_LIST`.
    pub ext_list: Vec<u16>,
    /// The guard's board record.
    pub board: BoardRecord,
    /// `EXT_CORE_VREG`, where it has one.
    pub vreg: Option<VregRecord>,
    /// The export's snapshot id in the hub's store.
    pub export: Option<String>,
}

/// What a device was after it came back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct After {
    /// Its firmware version.
    pub fw: String,
    /// `EXT_LIST`.
    pub ext_list: Vec<u16>,
    /// `EXT_CORE_VREG`, where it has one.
    pub vreg: Option<VregRecord>,
    /// Against the export: `unchanged`, `changed` or `unknown`.
    pub settings: String,
}

/// Why a device's part ended badly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failed {
    /// The step.
    pub step: String,
    /// The code, where a refusal had one.
    pub code: Option<String>,
    /// One line.
    pub text: String,
}

/// One device of a rollout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceEntry {
    /// The device.
    pub serial: String,
    /// `pending`, `running`, `ok`, `failed`, `cancelled`, `not_tried`; -- WP PGFWUP --
    /// `excluded`: a group member without BOOTSEL (a DSPi-Zero, an unknown board), never
    /// tried, its reason in `failed` (code `no_bootsel`).
    pub state: String,
    /// The step it is at, or ended at ("" before its first).
    pub step: String,
    /// The steps done.
    pub steps: Vec<StepDone>,
    /// Before the flash (from step 3).
    pub before: Option<Before>,
    /// After it came back (step 8).
    pub after: Option<After>,
    /// Why it failed.
    pub failed: Option<Failed>,
}

impl DeviceEntry {
    fn pending(serial: &str) -> Self {
        Self {
            serial: serial.to_owned(),
            state: "pending".into(),
            step: String::new(),
            steps: Vec::new(),
            before: None,
            after: None,
            failed: None,
        }
    }
}

/// The image a rollout puts on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRef {
    /// Its name in the store.
    pub name: String,
    /// Its SHA-256.
    pub sha256: String,
    /// Its DSPi version.
    pub firmware_version: String,
}

/// A rollout, as `fw.rollout.status` answers it and `rollout.json` keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// `ro1`, `ro2`, ...; never reused.
    pub id: String,
    /// The serial, or `group:<id>`.
    pub target: String,
    /// The image.
    pub image: ImageRef,
    /// The Admin's client name.
    pub by: String,
    /// As asked.
    pub while_live: bool,
    /// `running`, `ok`, `failed`, `cancelled`.
    pub state: String,
    /// ISO-8601.
    pub started_at: String,
    /// ISO-8601; `None` while running.
    pub ended_at: Option<String>,
    /// One entry per device, in order.
    pub devices: Vec<DeviceEntry>,
}

impl Record {
    fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({"id": self.id}))
    }

    // -- WP P3CC --
    /// The device a change is about: the one running, else the last one that ran
    /// (the members after a failure or a cancel are `not_tried`).
    fn current(&self) -> Option<&DeviceEntry> {
        self.devices
            .iter()
            .find(|d| d.state == "running")
            .or_else(|| {
                self.devices
                    .iter()
                    .rev()
                    .find(|d| !matches!(d.state.as_str(), "pending" | "not_tried"))
            })
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    v: u8,
    #[serde(default)]
    next_id: u64,
    #[serde(default)]
    rollouts: Vec<Value>,
}

// ── State ───────────────────────────────────────────────────────────────────

/// An upload in progress.
struct Upload {
    name: String,
    next_seq: u64,
    data: Vec<u8>,
    started: Instant,
    session: Weak<Session>,
}

/// The rollout that runs.
#[derive(Debug, Clone)]
struct Running {
    id: String,
    image: String,
    /// Cancelled before the bootloader entry of the device it is at.
    cancelled: bool,
    /// The device it is at has been sent to its bootloader: no cancel from here on.
    past_entry: bool,
}

#[derive(Default)]
struct State {
    /// Oldest first.
    records: Vec<Record>,
    next_id: u64,
    /// The steps 1 and 2 of a start are being run (nothing is recorded yet).
    starting: bool,
    running: Option<Running>,
    /// Devices the health poll last saw silent.
    unresponsive: BTreeSet<String>,
    uploads: BTreeMap<SessionId, Upload>,
}

/// What the service and its rollout thread share.
pub(crate) struct Shared {
    state: Mutex<State>,
    images: ImageStore,
    file: PathBuf,
    snapshots: HubSnapshots,
    exec: crate::api::exec::Executor,
    clock: Arc<dyn HubClock>,
    bus: Arc<Bus>,
    scan: Arc<dyn BootScan>,
    writer: Arc<dyn Writer>,
    allow_unverified: bool,
    serves_tls: bool,
    health: Subscription,
    /// `group:<id>` targets (P3GR); `None` on a hub without groups.
    groups: Option<Arc<dyn crate::groups::GroupResolver>>,
    stop: AtomicBool,
    worker: Mutex<Option<JoinHandle<()>>>,
}

/// The `fw.*` family as a hub service.
pub struct RolloutService {
    sh: Arc<Shared>,
}

/// The service, when the config asks for it and the hub has a data folder (the one
/// `registered` line).
pub fn service(config: &ServiceConfig, setup: &ServiceSetup<'_>) -> Option<Arc<dyn HubService>> {
    let rc = config.rollout.as_ref()?;
    let Some(dir) = setup.data_dir else {
        log::warn!("hub rollout: no data folder, so no rollout");
        return None;
    };
    Some(Arc::new(RolloutService::new(rc, dir, setup)) as Arc<dyn HubService>)
}

/// What the health subscription takes.
fn wanted(m: &BusMessage) -> bool {
    matches!(
        m,
        BusMessage::Unresponsive { .. }
            | BusMessage::Health { .. }
            | BusMessage::DeviceOnline { .. }
            | BusMessage::DeviceOffline { .. }
    )
}

impl RolloutService {
    /// The service over `<dir>/firmware/` and `<dir>/rollout.json`.
    pub fn new(rc: &RolloutConfig, dir: &Path, setup: &ServiceSetup<'_>) -> Self {
        let clock = Arc::clone(setup.clock);
        let now_s = clock.unix_seconds();
        let file = dir.join(FILE_NAME);
        let (records, next_id, ended) = load(&file, now_s);
        let sh = Arc::new(Shared {
            state: Mutex::new(State {
                records,
                next_id,
                ..State::default()
            }),
            images: ImageStore::open(dir, now_s),
            file,
            snapshots: HubSnapshots::open(dir),
            exec: crate::api::exec::Executor::new(Arc::clone(&clock), setup.wake.clone()),
            clock,
            bus: Arc::clone(setup.bus),
            scan: rc
                .boot_scan
                .clone()
                .unwrap_or_else(|| Arc::new(CoreScan) as Arc<dyn BootScan>),
            writer: rc
                .writer
                .clone()
                .unwrap_or_else(|| Arc::new(CoreWriter) as Arc<dyn Writer>),
            allow_unverified: rc.allow_unverified_firmware,
            serves_tls: rc.serves_tls,
            health: setup
                .bus
                .subscribe_filtered("rollout", BUS_CAPACITY, wanted),
            groups: setup.group_resolver(),
            stop: AtomicBool::new(false),
            worker: Mutex::new(None),
        });
        if !ended.is_empty() {
            sh.persist();
        }
        // A rollout the hub stopped in the middle of: failed, alarmed.
        for (serial, step, text, fw) in ended {
            sh.bus.publish(BusMessage::RolloutEnded {
                serial,
                ok: false,
                step,
                text,
                firmware_version: fw,
            });
        }
        if rc.allow_unverified_firmware {
            log::warn!(
                "hub rollout: images the release does not list may be flashed \
                 (--allow-unverified-firmware)"
            );
        }
        Self { sh }
    }
}

/// The records of `path`, the next id, and what a rollout running when the hub stopped
/// left: `(serial, step, text, firmware)` of each device it was at, now failed.
#[allow(clippy::type_complexity)]
fn load(path: &Path, now_s: u64) -> (Vec<Record>, u64, Vec<(String, String, String, String)>) {
    let bytes = match images::read_limited(path, MAX_FILE_BYTES as u64) {
        Ok(Some(b)) => b,
        Ok(None) => {
            log::error!("hub rollout: {} is too long; kept aside", path.display());
            images::set_aside(path, now_s);
            return (Vec::new(), 1, Vec::new());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), 1, Vec::new()),
        Err(e) => {
            log::error!("hub rollout: cannot read {}: {e}", path.display());
            return (Vec::new(), 1, Vec::new());
        }
    };
    let Ok(file) = serde_json::from_slice::<File>(&bytes) else {
        log::error!("hub rollout: {} does not read; kept aside", path.display());
        images::set_aside(path, now_s);
        return (Vec::new(), 1, Vec::new());
    };
    let mut whole = true;
    let mut records: Vec<Record> = Vec::new();
    for v in file.rollouts {
        match serde_json::from_value::<Record>(v) {
            Ok(r) if records.len() < MAX_RECORDS => records.push(r),
            Ok(_) => {}
            Err(e) => {
                log::warn!("hub rollout: a record of {FILE_NAME} does not read: {e}");
                whole = false;
            }
        }
    }
    if !whole {
        // Keep the file as it was beside the records that read.
        let mut aside = path.as_os_str().to_owned();
        aside.push(format!(".bad-{now_s}"));
        if let Err(e) = std::fs::copy(path, PathBuf::from(&aside)) {
            log::error!("hub rollout: cannot keep {} aside: {e}", path.display());
        }
    }
    let highest = records
        .iter()
        .filter_map(|r| r.id.strip_prefix("ro")?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    let next_id = file.next_id.max(highest.saturating_add(1)).max(1);
    let mut ended = Vec::new();
    for r in &mut records {
        if r.state != "running" {
            continue;
        }
        let fw = r.image.firmware_version.clone();
        let mut stop = false;
        for d in &mut r.devices {
            if stop {
                d.state = "not_tried".into();
                continue;
            }
            match d.state.as_str() {
                "running" => {
                    let text = "The hub stopped during this device's rollout".to_owned();
                    d.state = "failed".into();
                    d.failed = Some(Failed {
                        step: d.step.clone(),
                        code: None,
                        text: text.clone(),
                    });
                    ended.push((d.serial.clone(), d.step.clone(), text, fw.clone()));
                    stop = true;
                }
                "pending" => {
                    d.state = "not_tried".into();
                    stop = true;
                }
                _ => {}
            }
        }
        r.state = "failed".into();
        r.ended_at = Some(iso8601(now_s));
    }
    (records, next_id, ended)
}

// ── Shared state, files, events ─────────────────────────────────────────────

impl Shared {
    fn now_iso(&self) -> String {
        iso8601(self.clock.unix_seconds())
    }

    /// Write `rollout.json` (the newest records that fit [`MAX_FILE_BYTES`]).
    fn persist(&self) {
        let st = lock(&self.state);
        self.persist_locked(&st);
    }

    fn persist_locked(&self, st: &State) {
        let mut keep: &[Record] = &st.records;
        loop {
            let text = serde_json::to_string_pretty(&json!({
                "v": VERSION, "next_id": st.next_id, "rollouts": keep,
            }))
            .unwrap_or_default();
            if text.len() <= MAX_FILE_BYTES || keep.is_empty() {
                if let Err(e) = images::write_atomic(&self.file, text.as_bytes()) {
                    log::error!("hub rollout: cannot write {}: {e}", self.file.display());
                }
                return;
            }
            keep = keep.get(1..).unwrap_or_default();
        }
    }

    /// `fw.rollout.changed` to every encrypted session.
    fn announce(&self, hub: &HubHandle, record: &Record) {
        let mut event = json!({"t": CHANGED_EVENT, "v": VERSION, "hub": hub.hub_id(),
            "rollout": record.to_json()});
        // -- WP P3CC -- the delta: the device whose entry changed, and its step
        if let (Some(d), Some(m)) = (record.current(), event.as_object_mut()) {
            m.insert("serial".into(), json!(d.serial));
            m.insert("step".into(), json!(d.step));
        }
        let text = event.to_string();
        for s in hub.encrypted_sessions() {
            s.outbox().push_reliable(Outbound::Event(text.clone()));
        }
    }

    /// Change record `id` with `f` while it runs; written and announced. False (and
    /// nothing changed) when it no longer runs: it was cancelled.
    fn update(&self, hub: &HubHandle, id: &str, f: impl FnOnce(&mut Record)) -> bool {
        let record = {
            let mut st = lock(&self.state);
            let Some(r) = st.records.iter_mut().find(|r| r.id == id) else {
                return false;
            };
            if r.state != "running" {
                return false;
            }
            f(r);
            let r = r.clone();
            self.persist_locked(&st);
            r
        };
        self.announce(hub, &record);
        true
    }

    // -- review p3d --
    /// A rollout's worker ends: as [`Self::update`], and the hub's one slot is freed
    /// under the same lock, so that a console which starts the next rollout the moment
    /// it is told this one ended is not answered `busy`. The slot is freed whether or
    /// not the record still ran (a cancelled one ended when it was cancelled; its
    /// worker ends here).
    fn end(&self, hub: &HubHandle, id: &str, f: impl FnOnce(&mut Record)) {
        let record = {
            let mut st = lock(&self.state);
            let record = st
                .records
                .iter_mut()
                .find(|r| r.id == id && r.state == "running")
                .map(|r| {
                    f(r);
                    r.clone()
                });
            if st.running.as_ref().is_some_and(|r| r.id == id) {
                st.running = None;
            }
            if record.is_some() {
                self.persist_locked(&st);
            }
            record
        };
        if let Some(record) = record {
            self.announce(hub, &record);
        }
    }

    /// Catch up with the health poll: who is silent.
    fn refresh_health(&self) {
        let messages = self.health.drain();
        let mut st = lock(&self.state);
        for m in messages {
            match &*m {
                BusMessage::Unresponsive { serial, .. } => {
                    st.unresponsive.insert(serial.clone());
                }
                BusMessage::Health { serial, .. }
                | BusMessage::DeviceOnline { serial, .. }
                | BusMessage::DeviceOffline { serial, .. } => {
                    st.unresponsive.remove(serial);
                }
                _ => {}
            }
        }
    }

    fn is_unresponsive(&self, serial: &str) -> bool {
        lock(&self.state).unresponsive.contains(serial)
    }

    /// Drop uploads past their time or of a session that closed.
    fn expire_uploads(&self, now: Instant) {
        lock(&self.state).uploads.retain(|_, u| {
            let alive = u.session.strong_count() > 0;
            let in_time = now.saturating_duration_since(u.started) < UPLOAD_TIMEOUT;
            if !(alive && in_time) {
                log::info!("hub rollout: the upload of {} was dropped", u.name);
            }
            alive && in_time
        });
    }
}

// ── The messages ────────────────────────────────────────────────────────────

fn refused(e: images::ImageError) -> ApiError {
    ApiError::new(e.code, e.msg)
}

fn require(session: &Session, need: Role) -> Result<(), ApiError> {
    if session.role() < need {
        let who = if need == Role::Admin {
            "an admin's"
        } else {
            "a viewer's or above"
        };
        return Err(ApiError::new(
            "denied",
            format!("this is {who}: this role may not do that"),
        ));
    }
    Ok(())
}

impl RolloutService {
    fn image_put(
        &self,
        session: &Arc<Session>,
        hub: &HubHandle,
        msg: &Value,
    ) -> Result<Value, ApiError> {
        require(session, Role::Admin)?;
        let sh = &self.sh;
        let name = msg
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| images::valid_name(n))
            .ok_or_else(|| {
                ApiError::bad("name is 1 to 64 characters of A-Z a-z 0-9 . - _ + ending in .uf2")
            })?;
        let seq = msg
            .get("seq")
            .and_then(Value::as_u64)
            .ok_or_else(|| ApiError::bad("seq is a whole number from 0"))?;
        let data = msg
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::bad("data is the chunk in base64"))?;
        let last = match msg.get("last") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(ApiError::bad("last is true or false")),
        };
        let now = sh.clock.now();
        sh.expire_uploads(now);
        let sid = session.id();
        let drop_it = |why: ApiError| {
            lock(&sh.state).uploads.remove(&sid);
            why
        };
        if data.len() > MAX_CHUNK_CHARS {
            return Err(drop_it(ApiError::bad(format!(
                "a chunk is at most {MAX_CHUNK_CHARS} base64 characters"
            ))));
        }
        let bytes = BASE64
            .decode(data.as_bytes())
            .map_err(|_| drop_it(ApiError::bad("data is not base64 (RFC 4648, padded)")))?;
        let received = {
            let mut st = lock(&sh.state);
            if seq == 0 {
                // -- WP P3WRO -- (review p3c N2) a seq 0 starts this session's upload
                // again: the one it had open (a client whose middle chunk failed) is
                // dropped rather than holding the session busy for UPLOAD_TIMEOUT.
                if let Some(old) = st.uploads.remove(&sid) {
                    log::info!(
                        "hub rollout: the upload of {} was dropped for a new one",
                        old.name
                    );
                }
                if st.uploads.len() >= MAX_UPLOADS {
                    return Err(ApiError::new(
                        "busy",
                        format!("{MAX_UPLOADS} uploads are open on the hub; try again later"),
                    ));
                }
                st.uploads.insert(
                    sid,
                    Upload {
                        name: name.to_owned(),
                        next_seq: 0,
                        data: Vec::new(),
                        started: now,
                        session: Arc::downgrade(session),
                    },
                );
            }
            let Some(u) = st.uploads.get_mut(&sid) else {
                return Err(ApiError::new(
                    "not_found",
                    "no upload is open in this session (it may have timed out)",
                ));
            };
            let wrong = if u.name != name {
                Some("the chunk names another image than the upload".to_owned())
            } else if u.next_seq != seq {
                Some(format!(
                    "seq {seq} is out of order: {} was next",
                    u.next_seq
                ))
            } else if u.data.len().saturating_add(bytes.len()) > images::MAX_IMAGE_BYTES {
                Some(format!(
                    "an image is at most {} bytes",
                    images::MAX_IMAGE_BYTES
                ))
            } else {
                None
            };
            if let Some(why) = wrong {
                st.uploads.remove(&sid);
                return Err(ApiError::bad(format!("{why}; the upload was dropped")));
            }
            u.data.extend_from_slice(&bytes);
            u.next_seq = seq.saturating_add(1);
            u.data.len()
        };
        if !last {
            return Ok(json!({"name": name, "seq": seq, "received": received}));
        }
        let Some(upload) = lock(&sh.state).uploads.remove(&sid) else {
            return Err(ApiError::new("not_found", "the upload is gone"));
        };
        let by = hub
            .session_label(sid)
            .by()
            .unwrap_or_else(|| format!("Session {sid}"));
        let added = sh
            .images
            .add(name, &upload.data, &by, sh.clock.unix_seconds())
            .map_err(refused)?;
        Ok(json!({"name": name, "seq": seq, "received": received,
            "image": added.image.to_json(), "created": added.created}))
    }

    fn image_list(&self) -> Value {
        let list: Vec<Value> = self
            .sh
            .images
            .list(self.sh.clock.unix_seconds())
            .iter()
            .map(ImageInfo::to_json)
            .collect();
        json!({"images": list, "max_images": images::MAX_IMAGES,
            "max_bytes": images::MAX_IMAGE_BYTES})
    }

    fn image_delete(&self, session: &Session, msg: &Value) -> Result<Value, ApiError> {
        require(session, Role::Admin)?;
        let name = msg
            .get("image")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::bad("image is a stored image's name"))?;
        {
            let st = lock(&self.sh.state);
            if st.running.as_ref().is_some_and(|r| r.image == name) {
                return Err(ApiError::new(
                    "conflict",
                    format!("a running rollout uses {name}"),
                ));
            }
        }
        self.sh
            .images
            .delete(name, self.sh.clock.unix_seconds())
            .map_err(refused)?;
        Ok(json!({"image": name}))
    }

    fn status(&self, msg: &Value) -> Result<Value, ApiError> {
        let st = lock(&self.sh.state);
        match msg.get("rollout") {
            None | Some(Value::Null) => {
                let all: Vec<Value> = st.records.iter().rev().map(Record::to_json).collect();
                Ok(json!({"rollouts": all}))
            }
            Some(Value::String(id)) => st
                .records
                .iter()
                .find(|r| &r.id == id)
                .map(|r| json!({"rollouts": [r.to_json()]}))
                .ok_or_else(|| ApiError::new("not_found", format!("no rollout {id}"))),
            Some(_) => Err(ApiError::bad("rollout is a rollout's id (\"ro1\")")),
        }
    }

    fn cancel(&self, session: &Session, hub: &HubHandle, msg: &Value) -> Result<Value, ApiError> {
        require(session, Role::Admin)?;
        let id = msg
            .get("rollout")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::bad("rollout is a rollout's id (\"ro1\")"))?;
        let now = self.sh.now_iso();
        let record = {
            let mut st = lock(&self.sh.state);
            let running = st.running.clone();
            let Some(r) = st.records.iter_mut().find(|r| r.id == id) else {
                return Err(ApiError::new("not_found", format!("no rollout {id}")));
            };
            let this = running
                .as_ref()
                .filter(|run| run.id == id && r.state == "running");
            let Some(run) = this else {
                return Err(ApiError::new(
                    "conflict",
                    format!("rollout {id} has ended ({})", r.state),
                ));
            };
            if run.past_entry {
                return Err(ApiError::new(
                    "conflict",
                    "the device is past its bootloader entry: a flash is never interrupted",
                ));
            }
            let mut stopped = false;
            for d in &mut r.devices {
                match d.state.as_str() {
                    "running" => {
                        d.state = "cancelled".into();
                        stopped = true;
                    }
                    "pending" if stopped => d.state = "not_tried".into(),
                    "pending" => {
                        d.state = "cancelled".into();
                        stopped = true;
                    }
                    _ => {}
                }
            }
            r.state = "cancelled".into();
            r.ended_at = Some(now);
            let r = r.clone();
            if let Some(run) = st.running.as_mut() {
                run.cancelled = true;
            }
            self.sh.persist_locked(&st);
            r
        };
        log::warn!("hub rollout: {id} cancelled");
        self.sh.announce(hub, &record);
        Ok(json!({"rollout": record.to_json()}))
    }
}

impl HubService for RolloutService {
    fn name(&self) -> &'static str {
        "rollout"
    }

    fn caps(&self) -> Vec<&'static str> {
        if self.sh.serves_tls {
            vec![CAP]
        } else {
            Vec::new()
        }
    }

    fn handle_message(
        &self,
        hub: &HubHandle,
        session: &Arc<Session>,
        msg: &Value,
    ) -> Option<String> {
        let t = msg.get("t")?.as_str()?;
        if !MESSAGES.contains(&t) {
            return None;
        }
        let Some(id) = msg.get("id").filter(|id| id.is_u64() || id.is_i64()) else {
            return Some(err(&Value::Null, &ApiError::bad("id is required")));
        };
        // OD-P3n: never over plaintext, the reads included, in every mode.
        if !session.encrypted() {
            return Some(err(
                id,
                &ApiError::new(
                    "tls_required",
                    "the firmware rollout is served over TLS only; connect with TLS",
                ),
            ));
        }
        let result = match t {
            "fw.image.put" => self.image_put(session, hub, msg),
            "fw.image.list" => Ok(self.image_list()),
            "fw.image.delete" => self.image_delete(session, msg),
            "fw.rollout.start" => steps::start(&self.sh, hub, session, msg),
            "fw.rollout.status" => self.status(msg),
            // "fw.rollout.cancel"
            _ => self.cancel(session, hub, msg),
        };
        Some(match result {
            Ok(body) => ok(id, body),
            Err(e) => err(id, &e),
        })
    }

    fn next_due(&self) -> Option<Instant> {
        let upload = lock(&self.sh.state)
            .uploads
            .values()
            .map(|u| u.started.checked_add(UPLOAD_TIMEOUT).unwrap_or(u.started))
            .min();
        [upload, self.sh.exec.next_due()]
            .into_iter()
            .flatten()
            .min()
    }

    fn run_due(&self, hub: &HubHandle, now: Instant) {
        self.sh.expire_uploads(now);
        if self.sh.exec.next_due().is_some_and(|d| d <= now) {
            self.sh.exec.sweep(hub, now);
        }
    }

    fn stop(&self) {
        self.sh.stop.store(true, Ordering::SeqCst);
        let worker = lock(&self.sh.worker).take();
        if let Some(w) = worker {
            let _ = w.join();
        }
        self.sh.exec.close_all();
    }
}

#[cfg(test)]
#[path = "rollout/tests_wp_p3ro.rs"]
mod tests_wp_p3ro;
