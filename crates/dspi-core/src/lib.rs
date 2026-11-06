//! The DSPi embedder API: one object a GUI or a language binding drives.
//!
//! [`DspiCore`] is what the Qt desktop app reaches through the C ABI
//! (`dspi-capi`), and what the Android and iOS apps will reach through UniFFI
//! (WP F1). Its methods are the old desktop core's C functions, one for one,
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
//! against chunked bulk transfers. Listeners are called without the lock held.
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
//!
//! The skeleton methods marked `todo!("WP ..")` are filled in by the work
//! packages listed in `docs/core-merge/PLAN.md`.

// The core-merge quality bar (docs/core-merge/PLAN.md §1.7, §1.8): no
// panicking shortcuts on outside data. clippy.toml exempts test code from the
// first three; test modules allow arithmetic_side_effects themselves.
// missing_docs is turned on by WP F1, which documents every record.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
// Skeleton phase: the shared plumbing below is used only once the work
// packages fill the methods in. WP Q3 removes this allow.
#![allow(dead_code)]

use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use dspi_proto::wire_compat::WireRead;
use dspi_session::Session;
use dspi_session::ext_state::ExtState;
use dspi_transport::{Transport, TransportCaps};

pub mod callbacks;
pub mod consts;
pub mod error;
pub mod records;
pub mod state;

mod connect;
mod cs;
mod curves;
mod device;
mod ext;
pub mod files;
mod firmware;
mod hardware;
mod hotplug;
mod info;
#[cfg(feature = "link")]
pub mod link;
mod monitor;
mod notify;
mod params;
mod params_fx;
mod presets;
mod rta;
mod stats;

pub use callbacks::{DeviceEvents, LinkListener, NotifyListener, RtaListener};
pub use curves::*;
pub use device::MidiInputHandle;
pub use error::{DspiError, Result};
pub use firmware::*;
pub use records::*;
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

/// The embedder API. Create one per app with [`DspiCore::new`].
pub struct DspiCore {
    pub(crate) inner: Mutex<Inner>,
    /// Shared with the reader and worker threads, which wake the app.
    pub(crate) listeners: Arc<Listeners>,
}

/// Everything behind the lock.
pub(crate) struct Inner {
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
    /// The notification monitor's log (WP C7).
    pub(crate) monitor: monitor::MonitorLog,
    /// The analyser's state and worker (WP C9).
    pub(crate) rta: rta::RtaState,
    /// The control-surface editing state (WP C14).
    pub(crate) cs: cs::CsState,
    /// The DSPi Link session (WP C15).
    #[cfg(feature = "link")]
    pub(crate) link: Option<link::LinkSession>,
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
}

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
        Self {
            link: Wire::Raw(transport),
            serial,
            vid,
            caps,
            session_id,
            notifications: None,
            wire: None,
            ext: ExtState::default(),
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
    /// OUT transfer, errors typed.
    pub(crate) fn set(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        Ok(self.0.transport().control_out(opcode, value, data)?)
    }

    /// A write-as-read opcode (parameters in `wValue`, a status byte back):
    /// the status, 0 = applied.
    pub(crate) fn set_as_read(&mut self, opcode: u8, value: u16) -> Result<u8> {
        let b = self.0.transport().control_in(opcode, value, 1)?;
        b.first().copied().ok_or(DspiError::Transfer {
            msg: format!("empty status reply to opcode 0x{opcode:02X}"),
        })
    }

    /// The session, for `Session::send` (WP S1) and the session's write
    /// helpers.
    pub(crate) fn session(&mut self) -> Result<&mut Session> {
        self.0.session()
    }

    /// The connection, for reads in the middle of a write.
    pub(crate) fn conn(&mut self) -> &mut Connection {
        self.0
    }
}

impl Inner {
    fn new() -> Self {
        Self {
            devices: Vec::new(),
            inaccessible: 0,
            selected: None,
            conn: None,
            state: state::default_state(),
            baseline: None,
            monitor: monitor::MonitorLog::default(),
            rta: rta::RtaState::default(),
            cs: cs::CsState::default(),
            #[cfg(feature = "link")]
            link: None,
        }
    }

    /// The open connection, or [`DspiError::Disconnected`].
    pub(crate) fn conn(&mut self) -> Result<&mut Connection> {
        self.conn.as_mut().ok_or(DspiError::Disconnected)
    }

    /// The open connection as a [`Writable`], if its state may be written
    /// ([`DspiError::ReadOnly`] for another wire format).
    pub(crate) fn writable_conn(&mut self) -> Result<Writable<'_>> {
        let c = self.conn()?;
        if c.writable() {
            Ok(Writable(c))
        } else {
            Err(DspiError::ReadOnly)
        }
    }
}

/// A listener slot: set, cleared and called from any thread. A call runs
/// with the slot's read lock held, so clearing waits for a call in flight;
/// a listener must not set listeners from inside its own callback.
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

    /// Call the listener if one is set. A panicking listener is contained.
    pub(crate) fn call(&self, f: impl FnOnce(&T)) {
        let guard = self.0.read().unwrap_or_else(|p| p.into_inner());
        if let Some(l) = guard.as_deref() {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(l)));
        }
    }
}

#[derive(Default)]
pub(crate) struct Listeners {
    pub(crate) notify: Slot<dyn NotifyListener>,
    pub(crate) rta: Slot<dyn RtaListener>,
    pub(crate) device: Slot<dyn DeviceEvents>,
    pub(crate) link: Slot<dyn LinkListener>,
}

impl DspiCore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner::new()),
            listeners: Arc::new(Listeners::default()),
        })
    }

    /// Make `transport` the open device, replacing any other, without
    /// probing or reading it: the bare minimum the work packages' tests use
    /// with a `FakeDevice`. WP C1's `open_transport` builds on it (probe,
    /// notifications, then `fetch_all`).
    pub(crate) fn attach(&self, transport: Box<dyn Transport>, serial: String, vid: u16) {
        let mut inner = self.lock();
        inner.conn = Some(Connection::new(transport, serial.clone(), vid));
        inner.selected = Some(serial);
    }

    /// The connect sequence every `open_*` ends in (WP C1 wraps it in
    /// `open_transport`):
    ///
    /// 1. take the old connection out under the lock and drop it with the
    ///    lock released (its reader thread joins);
    /// 2. probe the device and build the [`Session`];
    /// 3. read the bulk image into the mirror;
    /// 4. start the notification reader, waking the [`NotifyListener`];
    /// 5. install the connection.
    ///
    /// Base version: strict V32 probe and bulk read, the image kept in
    /// `bulk_raw` only. As their WPs land, C1 switches it to S2's
    /// `probe_any` and `compat::read_bulk_any` (setting `wire` from the read
    /// mode), C0's `state::decode_bulk`, and S3's `ExtState::probe`; C1 owns
    /// this function from wave 2 on.
    pub(crate) fn attach_probed(
        &self,
        mut transport: Box<dyn Transport>,
        serial: String,
        vid: u16,
    ) -> Result<()> {
        let old = self.lock().conn.take();
        drop(old);

        let caps = dspi_session::probe(&mut *transport).map_err(DspiError::from)?;
        let mut session = Session::new(transport, caps).ok_or(DspiError::InvalidArgument {
            msg: "the device reported an impossible channel layout".into(),
        })?;
        let bulk = dspi_session::read_bulk(session.transport_mut()).map_err(DspiError::from)?;

        let listeners = Arc::clone(&self.listeners);
        let waker: dspi_session::notify::Waker =
            Arc::new(move || listeners.notify.call(|l| l.on_notifications()));
        let notifications = match session.transport_mut().notifications() {
            Some(source) => Some(
                dspi_session::Notifications::start_with_waker(source, Some(waker)).map_err(
                    |e| DspiError::Io {
                        msg: format!("cannot start the notification reader: {e}"),
                    },
                )?,
            ),
            None => None,
        };

        let mut conn = Connection::new(Box::new(NoTransport), serial.clone(), vid);
        conn.caps = session.transport_mut().caps();
        conn.session_id = session.transport_mut().session_id();
        conn.link = Wire::Session(Box::new(session));
        conn.notifications = notifications;
        conn.wire = Some(WireRead::Exact);

        let mut inner = self.lock();
        inner.state.bulk_raw = bulk.as_bytes().to_vec();
        inner.state.bulk_valid = true;
        inner.conn = Some(conn);
        inner.selected = Some(serial);
        Ok(())
    }

    /// The lock. A panic while it was held does not poison the API.
    pub(crate) fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "WP C0: needs state::default_state"]
    fn a_new_core_is_disconnected() {
        let core = DspiCore::new();
        assert!(matches!(core.lock().conn(), Err(DspiError::Disconnected)));
    }

    #[test]
    #[ignore = "WP C1: needs C0's default_state and T4's FakeDevice::demo"]
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
    #[ignore = "WP C0: needs state::default_state"]
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
