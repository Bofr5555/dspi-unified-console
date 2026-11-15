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
//! | [`identify`] | `device.identify`: blink, and the quiet tone when asked | P2ID |
#![warn(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

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
pub mod hub;
pub mod local;
// -- WP P1HL --
pub mod limits;
pub mod lock;
// -- WP P2MQC --
pub mod mqtt;
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
pub mod store;
// -- WP SW2 --
pub mod system;
pub mod usb;
pub mod web;
// -- WP P2SV --
pub mod bus;
pub mod clock;
pub mod service;
// -- WP P2ID --
pub mod identify;

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
pub use dspi_link::DEFAULT_PORT;
pub use server::NetworkControl;
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
