//! The DSPi embedder API: one object a GUI or a language binding drives.
//!
//! [`DspiCore`] is what the Qt desktop app reaches through the C ABI
//! (`dspi-capi`), and what the Android and iOS apps reach through UniFFI (the
//! `uniffi` feature, for `crates/dspi-ffi`). Its methods are the old desktop core's C functions, one for one,
//! with typed errors ([`DspiError`]) and plain records ([`records`]); a few
//! are added for features the old core lacked (extension channel, names,
//! diagnostics, firmware files, more transports).
//!
//! Underneath it is the shared stack: `dspi-proto` (wire formats),
//! `dspi-transport` (USB, MIDI, Link, demo) and `dspi-session` (probe,
//! writes, notifications, RTA, control surfaces, presets).
//!
//! # Threads and locking
//!
//! Every method takes `&self`; one mutex ([`DspiCore::inner`]) guards the
//! connection and the mirror. The notification reader runs on its own thread
//! (it does not need the lock to read; it signals the [`NotifyListener`]),
//! and the RTA worker takes the lock for each poll, which also serialises it
//! against chunked bulk transfers (over DSPi Link it polls with the lock
//! released: no chunked bulk there, and a round trip must not stall the
//! core). Listeners are called without the lock held. The DSPi Link device
//! lock is never waited for with the core lock held.
//!
//! # The mirror
//!
//! [`Inner::state`] is the [`DspState`] the GUI reads. It is decoded from the
//! device's bulk image ([`state::decode_bulk`]); setters update the field and
//! the image ([`state::encode_bulk`]) after sending; notifications patch the
//! image and re-decode. See `docs/core-merge/PLAN.md`.
//!
//! # Panics
//!
//! No method may panic on device, network or caller data. The C ABI and the
//! bindings also catch unwinding at the boundary, but that is the last line,
//! not the policy.

// The core-merge quality bar (docs/core-merge/PLAN.md §1.7, §1.8): no
// panicking shortcuts on outside data. clippy.toml exempts test code from the
// first three; test modules allow arithmetic_side_effects themselves.
#![warn(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, Weak};

use dspi_proto::wire_compat::WireRead;
use dspi_session::Session;
use dspi_session::ext_state::ExtState;
use dspi_transport::{Transport, TransportCaps};

// The Kotlin and Swift binding (WP F1): the namespace `dspi_core`, whose
// symbols the library `crates/dspi-ffi` re-exports.
#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

pub mod callbacks;
pub mod consts;
pub mod error;
pub mod records;
pub mod state;

mod connect;
mod cs;
// -- WP CG20 --
mod cs_rules;
mod curves;
mod device;
mod ext;
// -- WP FWX --
mod ext_diag;
// -- WP SW1 -- the audio link (SYS2), not DSPi Link
mod alink;
// -- WP AUSB1 -- the alink's USB-direct units (extension_channel_spec.md 2.9)
mod alink_usb;
pub mod files;
// -- WP P1DF --
mod doc_diff;
mod firmware;
// -- WP CG8 --
mod firmware_guard;
// -- WP SECFWV --
mod firmware_manifest;
mod hardware;
mod hotplug;
// -- WP MD0 -- one core per open device (native only)
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod core_set;
// -- WP CG26 --
mod host_transport;
mod info;
#[cfg(feature = "link")]
pub mod link;
// -- WP CG2 --
#[cfg(feature = "mdns")]
pub mod link_browse;
mod monitor;
mod notify;
mod params;
mod params_fx;
mod presets;
// -- WP CG4 --
mod raw;
mod rta;
mod stats;
// -- WP SW-M1 -- the Mac hub's USB pipe (retires at SW-M3)
mod usb_pipe;
// -- WP CG11 --
mod vreg;
// -- WP CG6 --
pub mod autoeq;
// -- WP CG10 --
mod link_text;
// -- WP CG32 --
mod write_param;
// -- WP CG34 --
mod names;
// -- BUG1 --
mod output_labels;
// -- WP P0M --
mod events;
// -- WP P0W --
mod journal;
// -- WP P0E --
mod diag;
// -- WP P1NT --
pub mod notice_text;
// -- WP P1TK --
pub mod confirm;
// -- WP P1SN --
mod snapshot;
// -- WP P1OF --
pub mod offline;
// -- WP P1AB --
mod ab;
// -- WP P1LM --
pub mod limits;
// -- WP P1SA --
mod guarded;
// -- WP P1LK --
pub mod modes;
// -- WP HUBADMIN --
pub mod hub_admin;
// -- WP P2CC1 -- the hub's own families (inventory, identify, health, stats, history, snapshots)
pub mod hub_server;
// -- WP ROMODE --
pub mod read_only;
// -- WP CG-SWS-a --
#[cfg(feature = "link")]
mod link_txt;
mod platform;
// -- WP CG-SWS-b -- the pin map
pub mod pins;
// -- WP CLIP1 -- the channel clipboard (Copy / Paste Parameters)
mod channel_clip;
// -- WP P3PB -- the PICOBOOT writer (the hub's unattended flash)
mod picoboot;
// -- WP P5DOCK -- the measurement documents (PLAN-P5 T11) and the core's SectionModel
mod measure_doc;
// -- WP P5SES -- the measurement session (PLAN-P5)
pub mod measure;
// -- WP P5DEMO -- the demo's simulated microphone (PLAN-P5 OD-P5q)
mod measure_sim;
// -- WP MCP0 -- who writes: actors, the pre-write hook, the user-editing gate
pub mod actor;
// -- WP ROLE0 -- the speaker layout: output roles from the user's setup (MPLAN 2.2)
pub mod speaker_layout;
// -- WP FUNC2a -- the function test's config-time oracle (test-only)
#[cfg(feature = "sim-scenario")]
mod functest_oracle;

pub use callbacks::{DeviceEvents, LinkListener, MidiSender, NotifyListener, RtaListener};
// -- WP P5SES --
pub use callbacks::MeasureListener;
pub use curves::*;
pub use device::MidiInputHandle;
pub use error::{DspiError, Result};
pub use firmware::*;
pub use records::*;
// -- WP P0M --
pub use dspi_session::events::{EventKind, EventRecord, EventSink, ReadBatch};
pub use events::EVENT_READ_MAX;
// -- WP CG11 --
pub use vreg::{VregCalPhase, VregCalView, VregChoice, VregDemoEvent};
// -- WP P5DOCK --
pub use measure_doc::{
    MEASURE_DOC_REFUSAL,
    MeasureDocInfo,
    TargetInfo,
    TargetParam,
    measure_doc_info,
    // -- WP FEAT2 --
    mic_document_has_spl,
    mic_import,
    target_builtin_document,
    target_builtins,
    target_import,
};
// Work-package sections: add re-exports only inside your own section.
// -- WP C0 --
// -- WP C1 --
// -- WP C1b --
// -- WP C2 --
// -- WP C3 --
// -- WP C4 --
// -- WP C5 --
// -- WP C6 --
// -- WP C7 --
// -- WP C8 --
// -- WP C9 --
// -- WP C10 --
// -- WP C11 --
// -- WP C12 --
// -- WP C13 --
// -- WP C14 --
// -- WP C15 --
// -- WP F1 --
// -- WP CG6 --
pub use autoeq::*;
// -- WP CG8 --
pub use firmware_guard::*;
// -- WP SECFWV --
pub use firmware_manifest::NOTICE_NOT_VERIFIED as NOTICE_FIRMWARE_NOT_VERIFIED;
pub use firmware_manifest::{BundledFirmware, verify_bundled}; // -- WP SECFIX --
// -- WP PGFWUP -- the board rule and names (PLAN-PG 1.9), the manifest's lineage
pub use firmware_manifest::{BundledImage, FirmwareGeneration, FirmwareLineage, bundled_images};
pub use hardware::{
    PICO_PRODUCT_PREFIX, PICO_PRODUCTS, UNKNOWN_PRODUCT_BOARD, ZERO_BCD_DEVICE, ZERO_PRODUCT,
    board_from_descriptor, board_name, identify_board,
};
// -- WP CG10 --
pub use link_text::*;
// -- WP HB0 --
pub use hotplug::{BrokeredDevice, DeviceBroker};
// -- WP MD0 --
#[cfg(all(
    feature = "link",
    not(all(target_arch = "wasm32", target_os = "unknown"))
))]
pub use core_set::HubTarget;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub use core_set::{
    CORE_SET_CAP, CoreSet, CoreSetConfig, CoreSetError, CoreSetEvent, CoreSetEventKind,
    CoreSetListener, DeviceId, DeviceLiveness, DeviceTarget, DeviceTransport, OpenDevice,
};
// -- WP CG17 --
pub use dspi_proto::ext::PairMap;
// -- WP FWX --
pub use ext_diag::{DeviceResetInfo, IDENTIFY_TONE_DBFS, IdentifyInfo};
// -- WP SW1 --
pub use alink::{
    ALINK_SLOT_SILENT, APPLY_OP_MAX, AlinkFrameCounter, AlinkInfo, AlinkPhase, AlinkStatus,
    ApplyAtFrameStatus, INPUT_SOURCE_ALINK,
};
// -- WP AUSB1 --
pub use alink_usb::{
    AlinkTimeBaseCounter, AlinkUsbConfig, AlinkUsbDiag, AlinkUsbEnd, AlinkUsbStatus,
    TIME_BASE_ALINK, TIME_BASE_BUS_FRAME, TIME_BASE_USB_SAMPLE, USB_DEFAULT_D, USB_MODE_OFF,
    USB_MODE_PHASE_LOCK, USB_MODE_RATE_LOCK,
};
// -- WP CG23 --
pub use raw::RawNotification;
// -- WP P1TZF -- the opcodes a raw request refuses (for the Terminal's `raw`)
pub use raw::raw_gated_call;
// -- WP CG26 --
pub use host_transport::{HostTransport, HostTransportKind};
// -- WP SW-M1 --
pub use usb_pipe::{UsbPipe, usb_pipe_scan, usb_pipe_wait_change};
// -- WP CG32 --
pub use write_param::{ParamValue, write_refusal};
// -- WP CG34 --
pub use names::{FilterTypeName, filter_type_name, filter_type_names};
// -- BUG1 --
// -- WP ROLE0 --
pub use output_labels::{
    OutputCard, OutputLabel, OutputLabelInput, output_cards_for, output_labels_for,
};
pub use speaker_layout::{SpeakerLayout, SpeakerLayoutOutput};
// -- WP P0W --
pub use journal::{HistoryEntry, HistoryMove, HistorySource};
// -- WP P0E --
pub use diag::REMOVED as DIAG_REMOVED;
// -- WP P1NT --
pub use notice_text::{
    NOTICE_KEY_UNKNOWN, Notice, NoticeKind, notice_for, notice_for_key, notice_texts,
};
// -- WP P1TK --
pub use confirm::{CONFIRM_TOKEN_TTL_MS, ConfirmOp};
// -- WP P3PB --
pub use picoboot::{BootDevice, FlashReport, picoboot_devices};
// -- WP P1SN --
pub use snapshot::{SnapshotInfo, SnapshotRecall};
// -- WP P1OF --
pub use offline::{template_list, template_text};
// -- WP P1AB --
pub use ab::{AbCompare, AbSide};
// -- WP P1LM --
pub use limits::{LimitsApply, LimitsStatus};
// -- WP P1LK --
pub use limits::{InstallerPin, LockState};
// -- WP P1EFIX --
pub use limits::{LOCK_BANNER_NOTICE, LimitFieldRange, limits_field_ranges};
pub use modes::{
    ModeChoice, ModeEntry, ModeManifest, ModeOperation, ModeScreen, ModesLockedUser, mode_manifest,
};
// -- WP P1SA --
pub use guarded::{
    APPLY_GUARD_DEFAULT_TIMEOUT_S, APPLY_GUARD_MAX_TIMEOUT_S, GuardPhase, GuardedApply,
};
// -- WP P1DCFIX -- the countdown's notice key
pub use guarded::NOTICE_GUARD_WAITING;
// -- WP P1WB2 -- the hub's backstop sentence beside the countdown
pub use guarded::NOTICE_GUARD_HUB_BACKSTOP;
// -- WP HUBADMIN --
pub use hub_admin::{
    HubAdminEvent, HubAdminListener, HubAdminState, HubLimitsSet, HubModePin, HubPinSet,
    InstallerTarget,
};
// -- WP P2CC1 --
pub use hub_server::{
    HubCaps, HubDevice, HubEventPage, HubEventRecord, HubHealth, HubReset, HubServerEvent,
    HubServerListener, HubSnapshotDocument, HubSnapshotPut, HubSnapshotRecall, HubStats,
    HubStatsDevice, MergedSnapshot, SnapshotMergedList,
};
// -- WP FEATUI --
pub use hub_server::{CAP_PRESENCE, HubPresence};
// -- WP P3HB --
pub use hub_server::{HubLastChange, HubStreamCounters};
// -- WP P2CC2 --
pub use hub_server::{
    HubAlarm, HubAlarmRule, HubApiKey, HubApiKeyMinted, HubMqttSettings, HubMqttStatus, HubMqttTls,
    HubOutboundLast, HubOutboundSecret, HubOutboundSet, HubOutboundTarget, HubOutboundTest,
    HubScheduleAction, HubScheduleLast, HubScheduleResult, HubScheduleRule, HubScheduleRun,
    HubSchedules, HubSecret, hub_alarm_notice,
};
// -- WP P2CC3 --
pub use hub_server::HubInventoryForgot;
// -- WP P3CC --
pub use hub_server::{
    HubFirmwareImage, HubFirmwareImages, HubFirmwarePut, HubFleetTarget, HubGroup, HubGroupParams,
    HubGroupRow, HubGroupRun, HubGroupSet, HubOffset, HubPresetPick, HubRollout, HubRolloutAfter,
    HubRolloutBefore, HubRolloutDevice, HubRolloutFailed, HubRolloutStep, HubRolloutVreg,
    hub_rollout_step_notice,
};
// -- WP P5HUB --
pub use hub_server::{HubMeasureDocPut, HubMeasureDocument, MeasureDocsMerged, MergedMeasureDoc};
// -- WP ROMODE --
pub use read_only::{
    GUARD_PENDING, HUB_READS, NOTICE_READ_ONLY_CLIENT, NOTICE_READ_ONLY_GUARD_PENDING,
};
// -- WP CG-SWS-a --
pub use device::known_feature_names;
#[cfg(feature = "link")]
pub use link_txt::link_parse_txt;
pub use platform::{
    bulk_image_extras, default_channel_names, enum_value_labels, platform_shape, vreg_code_label,
    vreg_code_mv,
};
// -- WP CG-SWS-b --
pub use pins::{
    PIN_NONE, PinFunction, PinFunctionInfo, PinFunctionKind, PinHolder, PinHolderRole, PinMap,
    PinRefusal, PinRefusalReason,
};
// -- WP CLIP1 --
pub use channel_clip::{
    CHANNEL_CLIP_KIND, CHANNEL_CLIP_VERSION, ChannelClip, ChannelClipOutput, ChannelPasteOptions,
    PASTE_LABEL, channel_clip_from_json, channel_clip_to_json,
};
// -- WP P5SES --
pub use measure::{
    MeasureCoupler, MeasureIssue, MeasureLevel, MeasureOutput, MeasurePhase, MeasurePlacement,
    MeasurePreflight, MeasureRole, MeasureRunKind, MeasureSetup, MeasureState, MeasureTiming,
};
// -- WP P5LIVE --
pub use measure::{
    MeasureLiveAdjust, MeasureLiveBand, MeasureLiveChange, MeasureLiveCurve, MeasureLiveFrame,
    MeasureLiveMode, MeasureLiveProposal, MeasureLiveRta, MeasureLiveSetup, MeasureLiveState,
    MeasureRtaAveraging,
};
// -- WP P5DEMO --
pub use measure_sim::{MeasureSimInfo, SIM_MIC_LABEL, SIM_WORLD};
// -- WP P5TUNE --
pub use measure::{
    MeasureLayout, MeasureLayoutOutput, MeasureTuneCheck, MeasureTuneCurves, MeasureTuneKeep,
    MeasureTuneKind, MeasureTuneOptions, MeasureTuneParam, MeasureTuneProposal, MeasureTuneStage,
    MeasureTuneState, MeasureTuneTransition, MeasureTuneVerify, MeasureTuneVerifyOutput,
    MeasureTuneVerifySum, MeasureTuneWrite,
};
// -- WP P5CURV --
pub use measure::{
    MeasureDocCurve, MeasureDocCurves, MeasureDocCurvesOptions, MeasureRtaPassiveFrame,
    MeasureRtaPassiveSetup, MeasureRtaPassiveState, measure_doc_curves,
};
// -- WP ATCORE --
pub use measure::{
    MeasureSystemApplied, MeasureSystemApplyStage, MeasureSystemArrival, MeasureSystemDelay,
    MeasureSystemDocument, MeasureSystemIssue, MeasureSystemLevel, MeasureSystemOutDelay,
    MeasureSystemPhase, MeasureSystemPreflight, MeasureSystemProposal, MeasureSystemPurpose,
    MeasureSystemResult, MeasureSystemSetup, MeasureSystemSnapshot, MeasureSystemState,
    MeasureSystemTransition, MeasureSystemUnit, MeasureSystemUnitCheck,
};
// -- WP ATDEMO --
pub use measure::MeasureSystemSimInfo;
// -- WP ROLL1 -- the roll-off search
pub use measure::{
    MeasureRolloffEnd, MeasureRolloffResult, MeasureRolloffSetup, MeasureRolloffStage,
    MeasureRolloffState, ROLLOFF_START_HZ, ROLLOFF_START_MAX_HZ, ROLLOFF_STOP_DB,
};

/// The embedder API. Create one per app with [`DspiCore::new`].
///
/// Every call that touches the device blocks its thread for the transfers it
/// makes, so a binding calls these off its UI thread.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct DspiCore {
    pub(crate) inner: Mutex<Inner>,
    /// Shared with the reader and worker threads, which wake the app.
    pub(crate) listeners: Arc<Listeners>,
    /// This object, for threads a `&self` method starts (the RTA worker).
    /// Methods stay `&self`, which is what UniFFI exports.
    pub(crate) me: Weak<DspiCore>,
    /// What `link_close` needs without the lock (WP C15).
    #[cfg(feature = "link")]
    pub(crate) link_control: Mutex<Option<link::LinkControl>>,
    // -- WP CG11 --
    /// The core-voltage calibration flow. Taken before `inner`, never after.
    pub(crate) vreg_cal: Mutex<vreg::CalState>,
    // -- WP CG2 --
    /// The running hub browse.
    #[cfg(feature = "mdns")]
    pub(crate) browse: link_browse::BrowseSlot,
    // -- WP CG26 --
    /// Where `host_notify` sends: the newest `open_host` connection's feed.
    pub(crate) host_feed: host_transport::FeedSlot,
    // -- WP LK2 --
    /// The open connection's side channel and channel count, for
    /// `fetch_status` while another call holds `inner` (a start-up read, a
    /// re-read after a resync, `cs_fetch`). Set where the side is installed,
    /// refreshed by every `fetch_status` that gets the lock, cleared when the
    /// connection goes. Taken with or without `inner`, never `inner` after it.
    pub(crate) meter_path: Mutex<Option<MeterPath>>,
    // -- WP LK4 --
    /// Held while a pending probe is completed ([`DspiCore::ensure_probed`]),
    /// so it runs once. Taken without `inner`, never while holding it.
    #[cfg(feature = "link")]
    pub(crate) probe_lock: Mutex<()>,
    // -- WP LKFAST --
    /// What the core remembers about DSPi Link hubs between sessions (a file in
    /// the folder of [`DspiCore::link_set_dir`]).
    #[cfg(feature = "link")]
    pub(crate) hub_memory: Arc<link::hub_memory::Store>,
    // -- WP P0M --
    /// The meter ballistics and the event log; `inner.events` is the same.
    /// Its locks are taken with or without `inner`, never `inner` after.
    /// (Not "history": that is the undo journal's name.)
    pub(crate) events: Arc<events::Store>,
    // -- WP P1SN --
    /// The folder the client named for snapshots (none on the web).
    #[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]
    pub(crate) snapshots: snapshot::StoreSlot,
    // -- WP P1AB --
    /// The running A/B compare. Taken without `inner`, never while holding it.
    pub(crate) ab: ab::AbSlot,
    // -- WP P1SA --
    /// The guarded apply (`guarded.rs`). Taken before `inner`, never after.
    pub(crate) guarded: guarded::GuardSlot,
    // -- WP P5DOCK --
    /// The folder the client named for measurement documents (none on the
    /// web). Taken with or without `inner`, never `inner` after it.
    #[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]
    pub(crate) measure_dir: measure_doc::DirSlot,
    // -- WP P5SES --
    /// The measurement session (`measure.rs`). Taken before `inner`, never
    /// after it.
    pub(crate) measure: measure::MeasureSlot,
    // -- WP ROLE0 --
    /// The speaker layouts (`speaker_layout.rs`). Taken with or without
    /// `inner`, never `inner` after it.
    pub(crate) speaker_layouts: speaker_layout::LayoutSlot,
    // -- WP P5DEMO --
    /// The demo's simulated microphone (`measure_sim.rs`). Taken before the
    /// measurement slot and `inner`, never after them.
    pub(crate) measure_sim: measure_sim::SimSlot,
}

// -- WP LK4 --
/// A DSPi Link bind's part of `attach_opened_with`.
#[derive(Clone, Copy)]
pub(crate) struct LinkBind<'a> {
    /// The device's last probe, for a re-attach.
    pub(crate) reuse: Option<&'a dspi_session::probe::Reusable>,
    /// The identity the hub lists.
    pub(crate) listed: Option<&'a dspi_session::probe::Listed>,
}

/// What `attach_opened_with`'s probe read.
struct Attached {
    /// The probe to keep for a re-attach.
    reusable: Option<dspi_session::probe::Reusable>,
    ext: dspi_transport::Result<ExtState>,
    /// Complete, or provisional while `pending`.
    caps: dspi_session::probe::Capabilities,
    image: dspi_proto::wire_compat::Normalized,
    /// GET_PLATFORM's reply.
    platform: Vec<u8>,
    /// The rest of the probe, still to run.
    pending: Option<Box<dspi_session::probe::Quick>>,
}

// -- WP LK2 --
/// What [`DspiCore::meter_path`] holds.
#[derive(Clone)]
pub(crate) struct MeterPath {
    /// The mirror's `num_channels`.
    pub(crate) num_channels: u8,
    pub(crate) side: Arc<dyn SideChannel>,
}

/// Everything behind the lock.
pub(crate) struct Inner {
    // -- WP QD6 --
    /// Writes refused because the connection is read-only: a client's read-only
    /// mode should never have let one get this far.
    pub(crate) read_only_refusals: u32,
    /// DSPis found by the last scan or hotplug poll.
    pub(crate) devices: Vec<DeviceInfo>,
    /// DSPis on the bus that could not be opened (permission, driver, busy).
    pub(crate) inaccessible: u32,
    /// The device the user chose; kept across a disconnect for reconnection.
    pub(crate) selected: Option<String>,
    /// The open device, if any.
    pub(crate) conn: Option<Connection>,
    /// The mirror the GUI reads (`get_state`). Defaults while disconnected.
    pub(crate) state: DspState,
    /// Bulk image captured by `capture_baseline`, for `preset_dirty` and
    /// `preset_changes` (WP C5).
    pub(crate) baseline: Option<Vec<u8>>,
    // -- WP SWAPFIX --
    /// The device (serial) and active preset slot `baseline` was captured
    /// for: a re-read of the same device on the same preset keeps it.
    pub(crate) baseline_of: Option<(String, u8)>,
    /// The notification monitor's log (WP C7).
    pub(crate) monitor: monitor::MonitorLog,
    /// The analyser's state and worker (WP C9).
    pub(crate) rta: rta::RtaState,
    /// The control-surface editing state (WP C14).
    pub(crate) cs: cs::CsState,
    /// The DSPi Link session (WP C15).
    #[cfg(feature = "link")]
    pub(crate) link: Option<link::LinkSession>,
    // -- WP C1b --
    /// The hotplug poll's own view of the bus and its pending reconnect
    /// (WP C1b). Kept apart from `devices`, which the picker's scan
    /// filters to the openable units.
    pub(crate) hotplug: hotplug::HotplugState,
    // -- WP CG23 --
    /// The raw notification tap (`raw.rs`).
    pub(crate) notify_tap: raw::NotifyTap,
    // -- WP P0M --
    /// [`DspiCore::events`], for code that holds only the lock.
    pub(crate) events: Arc<events::Store>,
    // -- WP P0W --
    /// The history as `process_notifications` last reported it
    /// (`journal::history_mark`), for `NOTIFY_HISTORY`.
    pub(crate) history_seen: Option<(u64, u64)>,
    // -- WP QTGAP1 --
    /// A client's continuous drag is under way ([`DspiCore::set_dragging`]):
    /// only then do writes join one undo step. -- WP P0W2 (fixup p0w2b) --
    /// The generation of the connection it was marked on: a drag ends with
    /// its connection (a disconnect, another device opened, a re-attach), so
    /// a stale mark never joins a later write ([`Inner::dragging`]).
    pub(crate) dragging: Option<u64>,
    // -- WP P1TK --
    /// Confirmation tokens and whether gated calls need one (`confirm.rs`).
    pub(crate) confirm: confirm::Confirm,
    // -- WP P1LM --
    /// The open device's limits profile (`limits.rs`).
    pub(crate) limits: limits::Limits,
    // -- WP ROMODE --
    /// The client read-only mode (`read_only.rs`).
    pub(crate) client_read_only: read_only::ClientReadOnly,
    // -- WP P5SES --
    /// What the notification path saw that a running measurement judges.
    pub(crate) measure_tap: measure::Tap,
    // -- WP PGFLASH --
    /// The update target this core last read from the open device
    /// ([`DspiCore::firmware_target`], or the bootloader entry's own read):
    /// what the write gate (`flash_confirmed`, `picoboot_flash_confirmed`)
    /// judges by, never a caller's. Cleared when another device is opened
    /// and after a write; kept when the device leaves for its bootloader.
    pub(crate) flash_target: Option<crate::FirmwareTarget>,
    // -- WP PGQT --
    /// The fake opened by [`DspiCore::open_demo_generation`], for its
    /// violation count; `None` for any other device.
    pub(crate) demo_generation: Option<dspi_transport::FakeRemote>,
    // -- WP MCP0 --
    /// What an [`actor::ActorScope`] names this core by.
    pub(crate) actor_key: u64,
    /// What other actors changed since the last `process_notifications`.
    pub(crate) foreign: actor::Foreign,
    // -- WP AIVIS --
    /// What the user changed, for the other actors (`actor::user`).
    pub(crate) user_log: actor::user::UserLog,
}

/// An open device.
pub(crate) struct Connection {
    /// The transport, until the device has been probed; then the session,
    /// which owns it.
    pub(crate) link: Wire,
    /// Serial of the device (the picker's identity).
    pub(crate) serial: String,
    /// USB vendor id it was found under (current or legacy firmware), 0 for
    /// a non-USB transport.
    pub(crate) vid: u16,
    /// What the transport can do (chunked bulk, remote).
    pub(crate) caps: TransportCaps,
    /// Our session id on a shared link (echo attribution).
    pub(crate) session_id: Option<u16>,
    /// The notification reader, once started.
    pub(crate) notifications: Option<dspi_session::Notifications>,
    /// How the last bulk image was read; `None` before the first read.
    pub(crate) wire: Option<WireRead>,
    /// The extension channel, probed on connect (WP S3).
    pub(crate) ext: ExtState,
    /// Recognises the invalidation of our own bulk writes (PLAN §1.2):
    /// `apply_all_params` (WP C2) and any other `0xA1`/`0xA3` writer call
    /// `bulk_write_started(Instant::now())` / `bulk_write_finished(Instant::now(), ok)`;
    /// `process_notifications` (WP C7) classifies with it.
    pub(crate) echo: dspi_session::notify::EchoFilter,
    /// Set by `attach_probed` (WP C1) when it decoded the mirror from this
    /// connect's own image; consumed by `fetch_all` (WP C2), which then
    /// skips re-reading the platform and the image but still does the
    /// per-item reads. False after any later change could have made the
    /// image stale.
    pub(crate) fresh_image: bool,
    /// Unique per connection, never reused: a flow that drops the core lock
    /// on the way uses it to tell its connection from one that replaced it
    /// (the device lock's release, WP C15).
    pub(crate) generation: u64,
    /// The way to a remote device that needs no core lock (DSPi Link, set by
    /// `link_select_device`): the device lock, the analyser and the meters
    /// use it, so a slow link cannot stall the core. `None` on USB, MIDI and
    /// the demo device.
    pub(crate) side: Option<Arc<dyn SideChannel>>,
    /// A re-read after a notification failed and is to be tried again
    /// (`notify.rs`, WP C15).
    pub(crate) refresh_pending: Option<notify::RefreshPending>,
    /// The next `process_notifications` reports a full refresh: the core read
    /// the device again by itself (a Link re-attach, WP C15).
    pub(crate) announce_refresh: bool,
    // -- WP P1EFIX2 --
    /// What a settle before a whole image consumed (`notify.rs`,
    /// `settle_notifications`): its flags and channels, ORed into the next
    /// `process_notifications` result so nothing it took is lost to the client.
    pub(crate) settled: NotifyResult,
    // -- WP CG13 --
    /// The user's input source names, "" for unnamed: read by `fetch_all`,
    /// kept current by `NOTIFY_EVT_INPUT_SOURCE_NAME` and
    /// `set_input_source_name`, so `preset_changes` needs no I/O. On the
    /// connection, so one device's names never reach the next.
    pub(crate) source_names: info::SourceNames,
    /// A lost notification may have been a rename: the next re-read after
    /// notifications reads the names too.
    pub(crate) source_names_stale: bool,
    // -- WP QD4 --
    /// The device's name, "" for unnamed, kept as `source_names` is.
    pub(crate) device_name: String,
    // -- WP CG21 --
    /// The fill target `fetch_stats_tick` judges by. On the connection, so a
    /// change of device starts over.
    pub(crate) ring_target: dspi_session::ring_health::RingTargetCache,
    // -- WP CG24 --
    /// The device lock held past one call (`device_lock_acquire`, an IR
    /// learn window, the bootloader restart). On the connection, so it goes
    /// with it: a closed or replaced connection gives it back.
    pub(crate) held_lock: Option<presets::HeldLock>,
    // -- WP LK4 --
    /// A DSPi Link bind installed the device from its identity and image
    /// alone: the rest of its probe (features, control-surface and generator
    /// caps, band count, extension channel) and the per-item reads still
    /// have to run ([`DspiCore::ensure_probed`]; the Link supervisor runs
    /// them in the background).
    pub(crate) pending_probe: Option<Box<dspi_session::probe::Quick>>,
    // -- WP P0W --
    /// The last change another writer made, for undo's foreign-change rule
    /// (`journal::ForeignChange`). On the connection, as its journal is.
    pub(crate) last_foreign: Option<journal::ForeignWrite>,
    // -- WP P0WECHO --
    /// The values this session wrote recently, which its own late echoes can
    /// bring back into the mirror (`journal::OwnWrites`).
    pub(crate) own_writes: journal::OwnWrites,
    // -- WP P1UG --
    /// The undo group open on this connection ([`DspiCore::begin_group`]):
    /// a group ends with its connection, as a drag does.
    pub(crate) undo_group: journal::UndoGroup,
    // -- WP BULKFIX -- (fixup bf F1, F2, F4)
    /// The bulk dB bug's state on this connection: whether the firmware has
    /// the fix, and which outputs may hold the series' gains behind their
    /// mute. Its transport is wrapped by it, so every unmute is checked.
    pub(crate) gain_guard: dspi_session::bulk_gain::GainGuard,
    /// The last bulk write left the device with its outputs muted (its
    /// repair failed after the muted image was sent).
    pub(crate) bulk_left_muted: bool,
    // -- WP FWX --
    /// Channels named by clip events since `take_clip_mask` (`ext_diag.rs`).
    pub(crate) clip_mask: u32,
    // -- WP P1OF --
    /// The document this connection was opened from (`offline.rs`); `None`
    /// for a device.
    pub(crate) document: Option<offline::Document>,
    // -- WP PGFIX --
    /// What the device is (`DeviceProtocol::device_kind`): the alink sender
    /// has no session (no image, no DSP model); its setters answer
    /// `Unsupported`.
    pub(crate) kind: DeviceKind,
    // -- WP LKFAST --
    /// The board, once read on this connection or taken from a DSPi Link
    /// bind's own platform read: `device_protocol` reads it once, not on
    /// every call (the board does not change within a connection).
    pub(crate) board_seen: Option<dspi_proto::packets::Board>,
}

/// A remote device reached without the [`Inner`] lock (WP C15).
///
/// Everything here may wait for the network. Call it with the lock released;
/// that is the point of it (PLAN §1.2, Wave 3 notes).
pub(crate) trait SideChannel: Send + Sync {
    /// Take the hub's device lock. Same errors as [`Transport::lock`].
    fn lock(&self, reason: &str, timeout: std::time::Duration) -> Result<()>;
    /// Release the lock taken by [`SideChannel::lock`].
    fn unlock(&self);
    /// A transport for one analyser poll, for the channels in `rta_mask`
    /// (0 = none wanted any more).
    fn rta_transport(self: Arc<Self>, rta_mask: u16) -> Box<dyn Transport>;
    /// The latest meter block of `len` bytes, without a round trip when the
    /// hub is feeding it.
    fn meters(&self, len: u16) -> Result<MeterRead>;
    // -- WP LK2 --
    /// Subscribe, renew or drop the meter feed as [`SideChannel::meters`]'s
    /// callers need it; may do a round trip. The Link supervisor calls it
    /// before it waits for the core lock, so the first meter does not wait
    /// for whatever holds it.
    // -- WP WB4b -- only the Link supervisor calls it (wasm-core builds without `link`)
    #[cfg_attr(not(feature = "link"), allow(dead_code))]
    fn maintain_meters(&self) {}
    // -- WP LK4 --
    /// A read-only transport for background reads that needs no core lock
    /// and leaves room for the commands a user waits for; `None` where there
    /// is none.
    #[cfg_attr(not(feature = "link"), allow(dead_code))]
    fn background(self: Arc<Self>) -> Option<Box<dyn Transport>> {
        None
    }
}

/// What [`SideChannel::meters`] has.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct MeterRead {
    /// The block; `None`: no recent frame (just subscribed, or the feed stalled).
    pub(crate) data: Option<Vec<u8>>,
    /// Not a fresh reading.
    pub(crate) stale: bool,
    /// Its age; 0 when nothing was ever read.
    pub(crate) age_ms: u32,
}

/// Source of [`Connection::generation`]; 0 is never handed out.
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

/// A placeholder transport for a `Connection` built before its session
/// (`attach_probed` replaces it at once). Every call is `Disconnected`.
struct NoTransport;

impl Transport for NoTransport {
    fn control_in(&mut self, _: u8, _: u16, _: u16) -> dspi_transport::Result<Vec<u8>> {
        Err(dspi_transport::TransportError::Disconnected)
    }

    fn control_out(&mut self, _: u8, _: u16, _: &[u8]) -> dspi_transport::Result<()> {
        Err(dspi_transport::TransportError::Disconnected)
    }

    fn descriptor(&self) -> &dspi_transport::DeviceDescriptor {
        static NONE: std::sync::OnceLock<dspi_transport::DeviceDescriptor> =
            std::sync::OnceLock::new();
        NONE.get_or_init(|| dspi_transport::DeviceDescriptor {
            serial: String::new(),
            bus_id: String::new(),
            address: 0,
            ..Default::default()
        })
    }
}

/// The transport, raw or inside a session.
pub(crate) enum Wire {
    Raw(Box<dyn Transport>),
    Session(Box<Session>),
}

impl Connection {
    pub(crate) fn new(transport: Box<dyn Transport>, serial: String, vid: u16) -> Self {
        let caps = transport.caps();
        let session_id = transport.session_id();
        // -- WP BULKFIX --
        let (transport, gain_guard) = dspi_session::bulk_gain::GainGuard::wrap(transport, false);
        Self {
            link: Wire::Raw(transport),
            serial,
            vid,
            caps,
            session_id,
            notifications: None,
            wire: None,
            ext: ExtState::default(),
            echo: dspi_session::notify::EchoFilter::default(),
            fresh_image: false,
            generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
            side: None,
            refresh_pending: None,
            announce_refresh: false,
            settled: NotifyResult::default(), // -- WP P1EFIX2 --
            // -- WP CG13 --
            source_names: info::SourceNames::default(),
            source_names_stale: false,
            // -- WP QD4 --
            device_name: String::new(),
            // -- WP CG21 --
            ring_target: Default::default(),
            // -- WP CG24 --
            held_lock: None,
            // -- WP LK4 --
            pending_probe: None,
            // -- WP P0W --
            last_foreign: None,
            // -- WP P0WECHO --
            own_writes: journal::OwnWrites::default(),
            // -- WP P1UG --
            undo_group: journal::UndoGroup::default(),
            // -- WP BULKFIX --
            gain_guard,
            bulk_left_muted: false,
            // -- WP FWX --
            clip_mask: 0,
            // -- WP P1OF --
            document: None,
            // -- WP PGFIX --
            kind: DeviceKind::Dsp,
            board_seen: None, // -- WP LKFAST --
        }
    }

    /// The transport, whichever form the connection is in. For reads; a
    /// write goes through [`Writable`].
    pub(crate) fn transport(&mut self) -> &mut dyn Transport {
        match &mut self.link {
            Wire::Raw(t) => &mut **t,
            Wire::Session(s) => s.transport_mut(),
        }
    }

    /// The session, once the device has been probed. For reads; writes go
    /// through [`Writable::session`].
    pub(crate) fn session(&mut self) -> Result<&mut Session> {
        match &mut self.link {
            Wire::Session(s) => Ok(s),
            // -- WP PGFIX -- the alink sender has no DSP model
            Wire::Raw(_) if self.kind == DeviceKind::AlinkSender => Err(DspiError::Unsupported),
            Wire::Raw(_) => Err(DspiError::Disconnected),
        }
    }

    /// Whether the mirror may be written back: the image was read in this
    /// build's own wire format.
    pub(crate) fn writable(&self) -> bool {
        self.wire == Some(WireRead::Exact)
    }

    /// IN transfer, errors typed.
    pub(crate) fn get(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        Ok(self.transport().control_in(opcode, value, len)?)
    }

    /// IN transfer whose reply may be shorter than `max_len` (replies sized
    /// by the device), errors typed.
    pub(crate) fn get_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> Result<Vec<u8>> {
        Ok(self.transport().control_in_upto(opcode, value, max_len)?)
    }
}

/// [`DspiError::Unsupported`] unless the connect probe found `feature`.
pub(crate) fn require_feature(session: &Session, feature: &str) -> Result<()> {
    let present = session
        .capabilities()
        .features
        .iter()
        .any(|f| f.name == feature && f.present);
    if present {
        Ok(())
    } else {
        Err(DspiError::Unsupported)
    }
}

/// A connection whose state may be written: the only way to send a change.
///
/// Returned by [`Inner::writable_conn`], which refuses a read-only
/// connection (another wire format, §1.5 of the plan). A [`Connection`]
/// alone offers reads only (`get`, `get_upto`, session reads); every
/// setter goes through this type, so a write on a read-only connection is a
/// compile error rather than a forgotten check. Rule for reviewers:
/// `Session::send`/`write`/`write_band`, `control_out` and write-as-read
/// opcodes appear only on a `Writable`.
pub(crate) struct Writable<'a>(&'a mut Connection);

impl Writable<'_> {
    // -- WP ECHO1 -- (review batch1 F1)
    /// A write of ours goes out now: from here an own echo may be its echo,
    /// not one older than the last whole image (`EchoFilter::param_write_sent`).
    /// Stamped by the send paths below, never by taking a `Writable` (the
    /// analyser takes one every tick and sends no parameter). Whole images
    /// (the bulk SET, a preset load, a factory reset) are not stamped: they
    /// mark the image themselves.
    fn stamp(&mut self, opcode: u8) {
        if !matches!(
            opcode,
            dspi_proto::generated::opcodes::REQ_SET_ALL_PARAMS
                | dspi_proto::generated::opcodes::REQ_SET_ALL_PARAMS_CHUNK
                | dspi_proto::generated::opcodes::REQ_PRESET_LOAD
                | dspi_proto::generated::opcodes::REQ_FACTORY_RESET
        ) {
            self.0
                .echo
                .param_write_sent(dspi_transport::clock::Instant::now());
        }
    }

    /// OUT transfer, errors typed.
    pub(crate) fn set(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        self.stamp(opcode); // -- WP ECHO1 --
        Ok(self.0.transport().control_out(opcode, value, data)?)
    }

    /// A write-as-read opcode (parameters in `wValue`, a status byte back):
    /// the status, 0 = applied.
    pub(crate) fn set_as_read(&mut self, opcode: u8, value: u16) -> Result<u8> {
        self.stamp(opcode); // -- WP ECHO1 --
        let b = self.0.transport().control_in(opcode, value, 1)?;
        b.first().copied().ok_or(DspiError::Transfer {
            msg: format!("empty status reply to opcode 0x{opcode:02X}"),
        })
    }

    /// The session, for `Session::send` (WP S1) and the session's write
    /// helpers.
    pub(crate) fn session(&mut self) -> Result<&mut Session> {
        // -- WP ECHO1 -- the session is only taken to write through it
        self.0
            .echo
            .param_write_sent(dspi_transport::clock::Instant::now());
        self.0.session()
    }

    // -- WP ECHO1 --
    /// The raw transport, to send a parameter or configuration write on it
    /// (an extension request, a raw write): stamped as our write. Reads, the
    /// analyser's requests and the demo device's simulated events use
    /// [`Writable::conn`] instead.
    pub(crate) fn sending(&mut self) -> &mut dyn Transport {
        self.0
            .echo
            .param_write_sent(dspi_transport::clock::Instant::now());
        self.0.transport()
    }

    /// The connection, for reads in the middle of a write. Not stamped as a
    /// write (WP ECHO1): send a write with `set`, `set_as_read`, `session` or
    /// `sending`.
    pub(crate) fn conn(&mut self) -> &mut Connection {
        self.0
    }
}

impl Inner {
    fn new() -> Self {
        Self {
            // -- WP QD6 --
            read_only_refusals: 0,
            devices: Vec::new(),
            inaccessible: 0,
            selected: None,
            conn: None,
            state: state::default_state(),
            baseline: None,
            baseline_of: None,
            monitor: monitor::MonitorLog::default(),
            rta: rta::RtaState::default(),
            cs: cs::CsState::default(),
            #[cfg(feature = "link")]
            link: None,
            // -- WP C1b --
            hotplug: hotplug::HotplugState::default(),
            // -- WP CG23 --
            notify_tap: raw::NotifyTap::default(),
            // -- WP P0M --
            events: Arc::default(),
            // -- WP P0W --
            history_seen: None,
            dragging: None,                                         // -- WP QTGAP1 --
            confirm: confirm::Confirm::default(),                   // -- WP P1TK --
            limits: limits::Limits::default(),                      // -- WP P1LM --
            client_read_only: read_only::ClientReadOnly::default(), // -- WP ROMODE --
            measure_tap: measure::Tap::default(),                   // -- WP P5SES --
            flash_target: None,                                     // -- WP PGFLASH --
            demo_generation: None,                                  // -- WP PGQT --
            actor_key: actor::next_core_key(),                      // -- WP MCP0 --
            foreign: actor::Foreign::default(),                     // -- WP MCP0 --
            user_log: actor::user::UserLog::default(),              // -- WP AIVIS --
        }
    }

    /// The open connection, or [`DspiError::Disconnected`].
    pub(crate) fn conn(&mut self) -> Result<&mut Connection> {
        self.conn.as_mut().ok_or(DspiError::Disconnected)
    }

    /// The open connection as a [`Writable`], if its state may be written
    /// ([`DspiError::ReadOnly`] for another wire format).
    pub(crate) fn writable_conn(&mut self) -> Result<Writable<'_>> {
        // -- WP PGFIX -- nothing of the DSP model goes to the alink sender
        if self.conn()?.kind == DeviceKind::AlinkSender {
            return Err(DspiError::Unsupported);
        }
        read_only::check(self)?; // -- WP ROMODE -- the client read-only mode
        if !self.conn()?.writable() {
            // -- WP QD6 --
            self.read_only_refusals = self.read_only_refusals.saturating_add(1);
            return Err(DspiError::ReadOnly);
        }
        crate::limits::lock_check(self)?; // -- WP P1LK -- the installer's lock
        actor::gate(self)?; // -- WP MCP0 -- another actor: the user-editing gate
        Ok(Writable(self.conn()?))
    }

    // -- WP PGFIX --
    /// [`Inner::writable_conn`] for an extension-channel write the alink
    /// sender answers too (its alink block): on the sender, which has no
    /// image to judge, only the client read-only mode and the installer's
    /// lock apply.
    pub(crate) fn ext_writable_conn(&mut self) -> Result<Writable<'_>> {
        if self.conn()?.kind != DeviceKind::AlinkSender {
            return self.writable_conn();
        }
        read_only::check(self)?;
        crate::limits::lock_check(self)?;
        actor::gate(self)?; // -- WP MCP0 --
        Ok(Writable(self.conn()?))
    }
}

/// A listener slot: set, replaced, cleared and called from any thread. A call
/// runs with the slot's read lock held, so clearing or replacing a listener
/// (`None` or `Some` alike) waits for a call in flight. For the same reason a
/// listener must not call a `set_*_listener` from inside its own callback.
pub(crate) struct Slot<T: ?Sized>(RwLock<Option<Arc<T>>>);

impl<T: ?Sized> Default for Slot<T> {
    fn default() -> Self {
        Self(RwLock::new(None))
    }
}

impl<T: ?Sized> Slot<T> {
    pub(crate) fn set(&self, listener: Option<Arc<T>>) {
        *self.0.write().unwrap_or_else(|p| p.into_inner()) = listener;
    }

    /// Call the listener if one is set. A panicking listener is contained
    /// and logged, naming the listener's interface.
    pub(crate) fn call(&self, f: impl FnOnce(&T)) {
        let guard = self.0.read().unwrap_or_else(|p| p.into_inner());
        if let Some(l) = guard.as_deref()
            && let Err(panic) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(l)))
        {
            log::error!(
                "listener {} panicked: {}",
                std::any::type_name::<T>(),
                panic_text(panic.as_ref())
            );
        }
    }
}

/// The message of a caught panic, when it has one.
fn panic_text(panic: &(dyn std::any::Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("no message")
}

#[derive(Default)]
pub(crate) struct Listeners {
    pub(crate) notify: Slot<dyn NotifyListener>,
    pub(crate) rta: Slot<dyn RtaListener>,
    pub(crate) device: Slot<dyn DeviceEvents>,
    // Read by the Link reader only (src/link.rs behind feature `link`).
    #[cfg_attr(not(feature = "link"), allow(dead_code))]
    pub(crate) link: Slot<dyn LinkListener>,
    // -- WP HUBADMIN -- the hub's admin events: listener and queue
    pub(crate) hub_admin: hub_admin::Events,
    // -- WP P2CC1 -- the hub's inventory, history and snapshot events
    pub(crate) hub_server: hub_server::Events,
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// A core with no device open. No I/O.
    #[cfg_attr(feature = "uniffi", uniffi::constructor)]
    pub fn new() -> Arc<Self> {
        // -- WP P0M --
        let events = Arc::new(events::Store::default());
        let mut inner = Inner::new();
        inner.events = Arc::clone(&events);
        Arc::new_cyclic(|me| Self {
            inner: Mutex::new(inner),
            listeners: Arc::new(Listeners::default()),
            me: me.clone(),
            #[cfg(feature = "link")]
            link_control: Mutex::new(None),
            // -- WP CG11 --
            vreg_cal: Mutex::new(vreg::CalState::new()),
            // -- WP CG2 --
            #[cfg(feature = "mdns")]
            browse: link_browse::BrowseSlot::default(),
            // -- WP CG26 --
            host_feed: host_transport::FeedSlot::default(),
            // -- WP LK2 --
            meter_path: Mutex::new(None),
            #[cfg(feature = "link")]
            probe_lock: Mutex::new(()),
            #[cfg(feature = "link")]
            hub_memory: Arc::default(), // -- WP LKFAST --
            // -- WP P0M --
            events,
            // -- WP P1SN --
            snapshots: snapshot::StoreSlot::default(),
            // -- WP P1AB --
            ab: ab::AbSlot::default(),
            // -- WP P1SA --
            guarded: guarded::GuardSlot::default(),
            // -- WP P5DOCK --
            measure_dir: measure_doc::DirSlot::default(),
            // -- WP P5SES --
            measure: measure::MeasureSlot::default(),
            // -- WP P5DEMO --
            measure_sim: measure_sim::SimSlot::default(),
            // -- WP ROLE0 --
            speaker_layouts: speaker_layout::LayoutSlot::default(),
        })
    }
}

impl DspiCore {
    /// Make `transport` the open device, replacing any other, without
    /// probing or reading it: the bare minimum the work packages' tests use
    /// with a `FakeDevice`. WP C1's `open_transport` builds on it (probe,
    /// notifications, then `fetch_all`).
    #[cfg(test)]
    pub(crate) fn attach(&self, transport: Box<dyn Transport>, serial: String, vid: u16) {
        let mut inner = self.lock();
        let conn = Connection::new(transport, serial.clone(), vid);
        inner.confirm.connected(conn.generation); // -- WP P1BFIX -- F5
        inner.flash_target = None; // -- WP PGFLASH -- another device's story
        inner.demo_generation = None; // -- WP PGQT -- the fake of another opening
        inner.conn = Some(conn);
        inner.selected = Some(serial);
    }

    /// The connect sequence every `open_*` ends in (owned by WP C1):
    ///
    /// 1. take the old connection out under the lock and close it with the
    ///    lock released (`device::shut`: its reader is stopped and joined
    ///    before its transport is dropped); the mirror resets to defaults;
    /// 2. take the transport's notification source, exactly once (Link and
    ///    MIDI hand out one reader), and start the reader, waking the
    ///    [`NotifyListener`], before anything else can fail: a later failure
    ///    drops the reader with everything else, never orphaning a
    ///    subscription;
    /// 3. probe whatever the wire format (`probe_any_with_image`, WP S2): one
    ///    bulk read serves the probe and the mirror; `wire` is its read mode;
    /// 4. probe the extension channel (`ExtState::probe`, WP S3); WP LK4: 3
    ///    and 4 are one `probe_connect`, batched;
    /// 5. build the [`Session`]; apply the probe's `GET_PLATFORM` reply to a
    ///    fresh mirror ([`state::apply_platform`]: platform, firmware version) and decode
    ///    the image into it ([`state::decode_bulk`], WP C0). An image of another wire format is
    ///    decoded read-only: `bulk_valid` false, `compat` too old / too new;
    /// 6. install the connection (with a new echo filter, `fresh_image`
    ///    set) and the mirror, stop a displaced connection's RTA worker and
    ///    reader with the lock released, and wake the [`NotifyListener`]
    ///    once for packets that arrived during the connect. `selected`
    ///    becomes `serial` for a USB device (`vid != 0`), and is cleared for
    ///    any other transport.
    ///
    /// A device that answers `GET_PLATFORM` but whose image cannot be mapped
    /// (pre-V16, an implausible newer image, or a legacy-VID unit whose bulk
    /// read fails) is installed read-only instead of failing (§1.5,
    /// "Unmappable images"): `Wire::Raw`, `wire` `None`, the mirror from
    /// `state::apply_platform` only, `compat` too old / too new from the
    /// image's version byte, `bulk_valid` false.
    ///
    /// Errors from the extension probe other than `Disconnected` (a timeout,
    /// say during a flash blackout) leave the device connected without the
    /// extension channel for this session.
    ///
    /// Errors: a probe failure (typed from the transport), an extension
    /// probe that lost the device ([`DspiError::Disconnected`]), a device
    /// reporting an impossible channel layout, or a reader thread that
    /// could not be spawned ([`DspiError::Io`]). On any error nothing is
    /// connected.
    pub(crate) fn attach_probed(
        &self,
        transport: Box<dyn Transport>,
        serial: String,
        vid: u16,
    ) -> Result<()> {
        self.close_connection();
        self.attach_opened(transport, serial, vid)
    }

    /// [`DspiCore::attach_probed`] from step 2, the old connection being
    /// closed already. The mirror changes only when the new one is installed
    /// (a Link re-attach keeps the last known state until then).
    pub(crate) fn attach_opened(
        &self,
        transport: Box<dyn Transport>,
        serial: String,
        vid: u16,
    ) -> Result<()> {
        self.attach_opened_with(transport, serial, vid, &mut |_| {}, None)
            .map(drop)
    }

    // -- WP LK4 --
    /// [`DspiCore::attach_opened`], calling `on_channels` with the image's
    /// channel count as soon as the image is in (before the feature reads),
    /// and describing the device from `reuse` when it is the same device and
    /// firmware ([`dspi_session::probe::probe_connect`]). Returns the probe
    /// for a later re-attach (`None` for an unmappable device or one whose
    /// extension channel could not be read).
    ///
    /// With `bind` (a DSPi Link bind), only the identity (unless the hub
    /// listed it) and the image are read before the device is installed; the
    /// rest of the probe and the per-item reads are left for
    /// [`DspiCore::ensure_probed`] (`Connection::pending_probe`), unless its
    /// `reuse` already describes the device.
    pub(crate) fn attach_opened_with(
        &self,
        mut transport: Box<dyn Transport>,
        serial: String,
        vid: u16,
        on_channels: &mut dyn FnMut(u8),
        bind: Option<LinkBind<'_>>,
    ) -> Result<Option<dspi_session::probe::Reusable>> {
        let listeners = Arc::clone(&self.listeners);
        let waker: dspi_session::notify::Waker =
            Arc::new(move || listeners.notify.call(|l| l.on_notifications()));
        // Declared before anything that can fail, so an early return drops
        // (stops and joins) the reader.
        let notifications = match transport.notifications() {
            Some(source) => Some(
                dspi_session::Notifications::start_with_waker(source, Some(waker)).map_err(
                    |e| DspiError::Io {
                        msg: format!("cannot start the notification reader: {e}"),
                    },
                )?,
            ),
            None => None,
        };

        // WP LK4: one probe for the device and its extension channel, in as
        // few round trips as the transport allows, handing back GET_PLATFORM's
        // reply for the mirror (no second read).
        let mut on_image = |image: &dspi_proto::wire_compat::Normalized| {
            on_channels(image.packet.header().num_channels);
        };
        let probed = if let Some(LinkBind { reuse, listed }) = bind {
            dspi_session::probe::probe_quick(&mut *transport, &mut on_image, reuse, listed).map(
                |quick| {
                    if quick.is_complete() {
                        let ext = quick.reused_ext().cloned().unwrap_or_default();
                        Attached {
                            reusable: quick.reusable(),
                            ext: Ok(ext),
                            caps: quick.caps,
                            image: quick.image,
                            platform: quick.platform,
                            pending: None,
                        }
                    } else {
                        Attached {
                            reusable: None,
                            ext: Ok(ExtState::default()),
                            caps: quick.caps.clone(),
                            image: quick.image.clone(),
                            platform: quick.platform.clone(),
                            pending: Some(Box::new(quick)),
                        }
                    }
                },
            )
        } else {
            dspi_session::probe::probe_connect(&mut *transport, &mut on_image, None).map(|p| {
                Attached {
                    reusable: p.reusable(),
                    ext: p.ext,
                    caps: p.caps,
                    image: p.image,
                    platform: p.platform,
                    pending: None,
                }
            })
        };
        let probed = match probed {
            Ok(probed) => probed,
            Err(e) => {
                // -- WP PGFIX -- (PGMAT F1) the alink sender: no image, its
                // own commands only
                if let Some(sender) = device::sender_mirror(&mut *transport, &e) {
                    let (mirror, ext) = sender?;
                    let mut conn = Connection::new(transport, serial.clone(), vid);
                    conn.notifications = notifications;
                    conn.ext = ext;
                    conn.kind = DeviceKind::AlinkSender;
                    self.install(conn, mirror, serial, vid);
                    return Ok(None);
                }
                // §1.5 "Unmappable images": a device that identifies itself
                // but whose image cannot be read is still shown, read-only.
                let Some(mirror) = device::unmappable_mirror(&mut *transport, &e, vid) else {
                    return Err(DspiError::from(e));
                };
                let mut conn = Connection::new(transport, serial.clone(), vid);
                conn.notifications = notifications;
                self.install(conn, mirror, serial, vid);
                return Ok(None);
            }
        };
        let Attached {
            reusable,
            ext,
            caps,
            image,
            platform,
            pending,
        } = probed;
        let ext = match ext {
            Ok(ext) => ext,
            Err(dspi_transport::TransportError::Disconnected) => {
                return Err(DspiError::Disconnected);
            }
            // The extension channel is optional: a device that answers
            // everything else but times out here still connects, without it.
            Err(e) => {
                log::warn!("extension probe failed, continuing without it: {e}");
                ExtState::default()
            }
        };

        // The firmware identity is not in the image: GET_PLATFORM's reply
        // (the probe's own) fills it, before the decode, as `fetch_all` does,
        // so the mirror is complete and `fresh_image` may let `fetch_all`
        // skip both reads.
        let mut mirror = state::default_state();
        let platform_known = match state::apply_platform(&platform, &mut mirror) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("GET_PLATFORM reply not applied: {e}");
                false
            }
        };
        state::decode_bulk(image.packet.as_bytes(), &mut mirror).map_err(|e| {
            DspiError::Transfer {
                msg: format!("the bulk image could not be decoded: {e}"),
            }
        })?;
        // The decoded header is the mapped V32 one; report the device's own.
        mirror.format_version = image.device_version;
        mirror.bulk_valid = image.is_writable();
        mirror.compat = match image.read {
            WireRead::Exact => consts::COMPAT_OK,
            WireRead::Older => consts::COMPAT_FIRMWARE_TOO_OLD,
            WireRead::Newer => consts::COMPAT_FIRMWARE_TOO_NEW,
        };

        let conn_caps = transport.caps();
        let session_id = transport.session_id();
        // -- WP BULKFIX --
        let (transport, gain_guard) = dspi_session::bulk_gain::GainGuard::wrap(transport, false);
        let mut session = Session::new(transport, caps).ok_or(DspiError::InvalidArgument {
            msg: "the device reported an impossible channel layout".into(),
        })?;
        // -- WP LK4 -- writes that depend on the rest of the probe wait for it
        session.set_provisional(pending.is_some());

        let mut conn = Connection::new(Box::new(NoTransport), serial.clone(), vid);
        conn.caps = conn_caps;
        conn.session_id = session_id;
        conn.link = Wire::Session(Box::new(session));
        conn.notifications = notifications;
        conn.wire = Some(image.read);
        // The firmware's own word on bulk echoes; without it, the time window.
        conn.echo.set_bulk_source(ext.bulk_source());
        conn.ext = ext;
        // -- WP BULKFIX -- Outputs muted now may be a cut-short repair's.
        conn.gain_guard = gain_guard;
        let muted = dspi_proto::bulk_gain::muted_mask(
            image.packet.as_bytes(),
            usize::from(mirror.num_output_channels),
        );
        if pending.is_some() {
            conn.gain_guard.suspect(muted);
        } else {
            let supported = conn.ext.supported.clone();
            let guard = conn.gain_guard.clone();
            let fixed = guard.fixed(conn.transport(), Some(&supported));
            guard.seed(fixed, muted);
        }
        // Without the platform reply the mirror lacks the firmware identity:
        // let `fetch_all` read everything again.
        conn.fresh_image = platform_known;
        conn.pending_probe = pending; // -- WP LK4 --

        self.install(conn, mirror, serial, vid);
        Ok(reusable)
    }

    // -- WP LK4 --
    /// Complete the open device's probe if a DSPi Link bind left it pending
    /// (`Connection::pending_probe`): what needs the features, the
    /// control-surface or generator caps or the extension channel calls this
    /// first. Waits for a completion in progress. Call with the lock
    /// released. A no-op otherwise.
    pub(crate) fn ensure_probed(&self) {
        #[cfg(feature = "link")]
        self.ensure_probed_link();
    }

    /// Tell the Link supervisor the open connection changed, so that it sees a
    /// bound device closed or displaced at once. Call with the lock released.
    pub(crate) fn kick_link(&self) {
        #[cfg(feature = "link")]
        link::kick(self);
    }

    /// The lock. A panic while it was held does not poison the API.
    pub(crate) fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    // -- WP LK2 --
    /// The lock if nobody holds it now, else `None`.
    pub(crate) fn try_lock(&self) -> Option<MutexGuard<'_, Inner>> {
        match self.inner.try_lock() {
            Ok(g) => Some(g),
            Err(std::sync::TryLockError::Poisoned(p)) => Some(p.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    }

    /// Replace [`DspiCore::meter_path`]. Returns the old one: drop it with the
    /// core lock released (a Link side's drop may write to the socket).
    #[must_use]
    pub(crate) fn set_meter_path(&self, path: Option<MeterPath>) -> Option<MeterPath> {
        std::mem::replace(
            &mut *self.meter_path.lock().unwrap_or_else(|p| p.into_inner()),
            path,
        )
    }

    /// A copy of [`DspiCore::meter_path`].
    pub(crate) fn meter_path(&self) -> Option<MeterPath> {
        self.meter_path
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    use super::*;

    #[test]
    fn a_new_core_is_disconnected() {
        let core = DspiCore::new();
        assert!(matches!(core.lock().conn(), Err(DspiError::Disconnected)));
    }

    #[test]
    fn wp_c1_attach_probed_opens_the_demo_device() {
        let core = DspiCore::new();
        let demo = dspi_transport::FakeDevice::demo();
        core.attach_probed(Box::new(demo), "DEMO".into(), 0)
            .unwrap();
        let mut inner = core.lock();
        assert!(inner.conn().is_ok());
        assert_eq!(inner.state.bulk_raw.len(), 6136);
    }

    #[test]
    fn wp_f1_a_panics_text_is_read() {
        let text = std::panic::catch_unwind(|| panic!("static")).unwrap_err();
        assert_eq!(panic_text(text.as_ref()), "static");
        let text = std::panic::catch_unwind(|| panic!("{}", 7)).unwrap_err();
        assert_eq!(panic_text(text.as_ref()), "7");
        let text = std::panic::catch_unwind(|| std::panic::panic_any(7)).unwrap_err();
        assert_eq!(panic_text(text.as_ref()), "no message");
    }

    #[test]
    fn wp_f1_replacing_a_listener_waits_for_a_call_in_flight() {
        use std::sync::atomic::{AtomicBool, Ordering};

        struct Slow(std::sync::mpsc::Sender<()>, Arc<AtomicBool>);
        impl NotifyListener for Slow {
            fn on_notifications(&self) {
                self.0.send(()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(100));
                self.1.store(true, Ordering::SeqCst);
            }
        }
        struct Idle;
        impl NotifyListener for Idle {
            fn on_notifications(&self) {}
        }
        let slot: Arc<Slot<dyn NotifyListener>> = Arc::new(Slot::default());
        let (tx, rx) = std::sync::mpsc::channel();
        let done = Arc::new(AtomicBool::new(false));
        slot.set(Some(Arc::new(Slow(tx, Arc::clone(&done)))));
        let caller = Arc::clone(&slot);
        let call = std::thread::spawn(move || caller.call(|l| l.on_notifications()));
        rx.recv().unwrap();
        slot.set(Some(Arc::new(Idle)));
        assert!(done.load(Ordering::SeqCst), "set returned during the call");
        call.join().unwrap();
    }

    #[test]
    fn a_panicking_listener_is_contained() {
        struct Boom;
        impl NotifyListener for Boom {
            fn on_notifications(&self) {
                panic!("listener");
            }
        }
        let core = DspiCore::new();
        core.listeners.notify.set(Some(Arc::new(Boom)));
        core.listeners.notify.call(|l| l.on_notifications());
    }
}
