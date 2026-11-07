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
#![warn(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

pub mod advertise;
#[cfg(feature = "app")]
pub mod app;
pub mod auth;
mod device;
pub mod error;
pub mod hub;
pub mod local;
pub mod lock;
pub mod policy;
pub mod polls;
pub mod registry;
pub mod relay;
pub mod router;
pub mod server;
pub mod session;
pub mod snapshot;
pub mod store;
pub mod usb;
pub mod web;

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
