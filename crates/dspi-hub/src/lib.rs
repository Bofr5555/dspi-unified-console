//! The DSPi Link hub: it owns local devices and serves them to the app, in-process,
//! and to clients on the network.
//!
//! The design is `docs/core-merge/HUB.md`: threads, ownership, lock order, the app's
//! session and the echo rule, and what happens on stop, unplug and a slow client.
//!
//! | Module | What | Owner |
//! |---|---|---|
//! | [`hub`] | lifecycle, the supervisor, attach and detach, sessions | HB0 |
//! | `device` (private) | the worker that owns a transport, the reader | HB0 |
//! | [`local`] | the app's in-process transport | HB0 |
//! | [`session`] | sessions, outboxes, completions | HB0 |
//! | [`usb`] | the bus seam, and USB behind it | HB0 |
//! | `app` (feature) | a `DspiCore` as the hub's local session | HB0 |
//! | [`router`], [`policy`], [`lock`], [`relay`] | admission, attribution, fan-out | HB2 |
//! | [`registry`], [`snapshot`], [`polls`] | handles and names, the cache, polls | HB3 |
//! | [`server`], [`auth`], [`store`] | the socket, pairing, what is kept on disk | HB1 |
//! | [`advertise`], [`web`] | Bonjour, the bundled web UI | HB5 |
//! | [`system`] | the audio link's system model: installation, alignment, scenes, alarms | SW2 |
//! | [`apply_guard`] | guarded apply: capture, confirm or revert by the hub | P1HA |
//! | [`limits`] | limits profiles, the installer lock and pinned modes | P1HL |
//! | [`service`], [`bus`], [`clock`] | the phase-2 service seam, its bus and time | P2SV |
//! | [`snapshots`] | the hub's shared copy of each device's snapshots | P2SS |
//! | [`health`] | the health poll and device facts (`health.get`) | P2HP |
//! | [`inventory`] | every DSPi the hub has seen, `inventory.*` | P2IN |
//! | [`identify`] | `device.identify`: blink, and the quiet tone when asked | P2ID |
//! | `stats` (private) | `hub.stats` and `hub.rename` | P2HS |
//! | [`history`] | the shared event history and the watchdog events | P2EV |
//! | [`api`] | the parameter API, the action executor, hub-side recall (feature `api`) | P2PA |
//! | [`outbound`] | notifications out: webhooks and push | P2WH |
//! | [`tls_identity`] | the hub's TLS key, certificate, pin and rotation | SECKEY |
//! | [`rest`] | the REST mirror under `/api/v1/` and API keys (`auth.mint`) | P2RS |
//! | `ffi` (feature `uniffi`) | the Swift binding: the app hosts a hub | SW-M3 |
//! | [`groups`] | named sets of inventory serials, `group.*`, `group:<id>` targets | P3GR |
#![warn(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

// -- WP SW-M3 -- The Swift binding (feature `uniffi`): the namespace `dspi_hub`,
// whose symbols the library `crates/dspi-ffi` re-exports with its feature `hub`.
#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

pub mod advertise;
// -- WP P1HA --
#[cfg(feature = "app")]
pub mod app;
pub mod apply_guard;
// -- WP P1CFIX -- F6: the settings outside the image a guarded apply's revert puts back
pub mod apply_extras;
pub mod auth;
// -- WP BULKFIX --
mod bulk_gain;
mod device;
pub mod error;
// -- WP SW-M3 --
#[cfg(feature = "uniffi")]
pub mod ffi;
pub mod hub;
pub mod local;
// -- WP P1HL --
pub mod limits;
pub mod lock;
// -- WP P2MQC --
pub mod mqtt;
// -- WP P2WH -- notifications out: webhooks and push
pub mod outbound;
pub mod policy;
pub mod polls;
pub mod registry;
pub mod relay;
pub mod router;
pub mod server;
pub mod session;
pub mod snapshot;
// -- WP P2SS -- the hub's shared snapshot store (not yet registered as a service)
pub mod snapshots;
// -- WP P2HS -- `hub.stats` and `hub.rename`
mod stats;
pub mod store;
// -- WP SW2 --
pub mod system;
pub mod usb;
pub mod web;
// -- WP P2SV --
pub mod bus;
pub mod clock;
pub mod service;
// -- WP P2HP -- the health poll and device facts
pub mod health;
// -- WP P2IN --
pub mod inventory;
// -- WP P2ID --
pub mod identify;
// -- WP P2EV --
pub mod history;
// -- WP P2PA -- the snapshot family always; the parameter API with feature `api`
pub mod api;
// -- WP SECKEY -- the hub's TLS key, certificate, pin and rotation
pub mod tls_identity;
// -- WP P2RS --
pub mod rest;
// -- WP SECWEB -- the `uconsole web` pinning proxy
pub mod proxy;
// -- WP P2AL -- device alarms
pub mod device_alarms;
// -- WP P2SC -- schedules (they run with feature `api`)
pub mod schedule;
// -- WP P3GR -- groups
pub mod groups;
// -- WP P5HUB -- the measurement documents (`measure.doc.*`, hub-api.md section 22)
pub mod measurements;
// -- WP P3RO -- the firmware rollout (feature `rollout`)
#[cfg(feature = "rollout")]
pub mod rollout;

pub use auth::AccessMode;
pub use error::HubError;
pub use hub::{Hub, HubConfig, HubEvent, HubHandle, HubListener};
pub use local::LocalTransport;
pub use registry::{DeviceIdentity, DeviceState, RegisteredDevice};
// -- WP HB3 --
pub use polls::{MAX_TRANSFERS_PER_SEC, POLL_BUDGET_BPS, POLL_MAX_HZ, PollRefused};
pub use registry::NameStore;
pub use server::NetworkConfig;
pub use session::{Completion, LOCAL_SESSION_ID, Outbound, Outbox, Pop, Role, Session, SessionId};
pub use snapshot::SnapshotRefused;
pub use usb::{BusDevice, DeviceBus};
// -- WP HB6 --
pub use auth::{PairedClient, SavedHub};
// -- WP P2RS --
pub use auth::{ClientKind, MintedKey};
pub use dspi_link::DEFAULT_PORT;
pub use server::NetworkControl;
// -- WP SECSRV --
pub use server::{TlsInfo, TlsMode, TlsPolicy, downgrade_warning};
#[cfg(all(feature = "usb", not(target_os = "android")))]
pub use usb::UsbBus;
// -- WP SW2 --
pub use system::{
    AlignReport, ModelRefused, SceneReport, SystemRefused, SystemStatus, SystemStore, WriteResult,
};
// -- WP P2SV --
pub use bus::{Bus, BusMessage, Subscription, TargetResult};
pub use clock::{HubClock, SystemClock, TestClock};
pub use service::{DeviceEvent, HubService, ServiceConfig, SessionLabel};
// -- WP P2HP --
pub use health::HealthConfig;
// -- WP P3HB --
pub use bus::{LastChange, StreamCounters};
// -- WP P2EV --
pub use history::HistoryConfig;
// -- WP P2SC --
pub use schedule::ScheduleConfig;
// -- WP P3RO --
#[cfg(feature = "rollout")]
pub use rollout::RolloutConfig;
