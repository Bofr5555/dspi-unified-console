//! A transport the embedding app supplies (WP CG26).
//!
//! [`HostTransport`] is a foreign trait: the app carries the control
//! transfers, the core does everything above them. [`DspiCore::open_host`]
//! opens a device over one through the usual connect sequence
//! ([`DspiCore::open_transport`]), and [`DspiCore::host_notify`] feeds the
//! device's notification packets to that connection's reader.
//!
//! It lets the Mac's core reach the local device through the Swift hub's
//! session 1 until SW-M3 (`docs/SWIFT-SEAMS.md` section 5.4, option A), and
//! lets SW4's tests run with Swift's `FakeTransport` underneath. It retires at
//! SW-M3.
//!
//! A host that fails is contained: its errors are typed, a reply of the wrong
//! length is an error, and a call that comes back only after
//! [`HANG_LIMIT`] is taken for a hung host. The core cannot interrupt a
//! foreign call, so a host must bound each call itself; once it does come
//! back late, the connection ends as unplugged: that call and every later one
//! fail with [`DspiError::Disconnected`], and the notification stream ends, so
//! `process_notifications` reports `NOTIFY_DISCONNECTED`. A host that reports
//! `Disconnected` itself ends the same way.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::Duration;

use dspi_transport::clock::Instant;
use dspi_transport::{
    Attributed, DeviceDescriptor, NotificationSource, Transport, TransportCaps, TransportError,
    TransportKind,
};

use crate::{DspiCore, DspiError, Result};

/// How long one host call may take before the host counts as hung: twice the
/// longest timeout of the core's own transports (MIDI's bulk read, 5 s).
pub(crate) const HANG_LIMIT: Duration = Duration::from_secs(10);

/// What kind of link a [`HostTransport`] is. It decides the echo rule (which
/// notifications are this client's own writes, `dspi_transport::is_own_write`)
/// and how bulk transfers go.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostTransportKind {
    /// USB vendor control transfers: our writes echo as `HOST_SET`; chunked
    /// bulk.
    Usb,
    /// The USB MIDI control channel: our writes echo as `PARAM_SRC_MIDI`;
    /// single-shot bulk.
    Midi,
    /// A DSPi Link hub, where this client is `session` (the Mac's own hub:
    /// session 1). Our writes are the notifications whose origin is `session`;
    /// single-shot bulk.
    Link {
        /// This client's session id on the hub.
        session: u16,
    },
    /// A scripted or simulated device (Swift's `FakeTransport`): USB's rules.
    Fake,
}

/// Control transfers to a DSPi, carried by the embedding app (WP CG26).
///
/// Calls come one at a time (the core's lock serialises them), on the thread
/// of the core call that makes them (on the Mac, `coreQueue`) or on the
/// analyser's worker thread, so an implementation must be `Send + Sync` but
/// sees no overlap. Each call must not block for long: bound it by the
/// transport's own timeout (about 1 s for a control transfer). A call that
/// returns after [`HANG_LIMIT`] (10 s) ends the connection.
///
/// The core's lock is held during every call: an implementation must not
/// call the core from inside one, except [`DspiCore::host_notify`], or the
/// core deadlocks.
///
/// Errors: [`DspiError::Disconnected`] when the device is gone (the
/// connection then ends); [`DspiError::Refused`] or
/// [`DspiError::Unsupported`] when the device stalled the request;
/// [`DspiError::Locked`] or [`DspiError::Denied`] from a hub; anything else
/// (a timeout after the host's own retries, a USB error) as
/// [`DspiError::Transfer`] with the host's words.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
pub trait HostTransport: Send + Sync {
    /// IN transfer of up to `len` bytes; the reply as the device sent it.
    /// Also carries the write-as-read opcodes. A reply longer than `len` is
    /// an error; a shorter one is one only where the opcode's size is fixed.
    fn control_in(&self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>>;
    /// OUT transfer carrying `data`.
    fn control_out(&self, opcode: u8, value: u16, data: Vec<u8>) -> Result<()>;
    /// What kind of link this is. Read once, at [`DspiCore::open_host`].
    fn kind(&self) -> HostTransportKind;
    // -- WP CG26 -- (review c4i finding 2: the hub's device lock, WP CG24)
    /// Take the hub's device lock around a multi-step flow (a preset load, an
    /// IR learn, a firmware install): other sessions are refused until
    /// [`HostTransport::unlock`] or until `timeout_ms` (the lease, at most
    /// 60 s; not a wait) runs out. A link without locks (USB, MIDI, a fake
    /// device) succeeds at once. The core asks once per flow; its own flows
    /// nest inside a held lock without asking again.
    ///
    /// Errors: [`DspiError::Locked`] while another session holds it; others
    /// as for a transfer.
    fn lock(&self, reason: String, timeout_ms: u32) -> Result<()>;
    /// Give back the lock [`HostTransport::lock`] took. Also called when the
    /// connection closes with the lock still held.
    fn unlock(&self);
}

/// The sending side of the open host connection's notification feed.
#[derive(Default)]
pub(crate) struct FeedSlot(Mutex<Option<mpsc::Sender<Attributed>>>);

impl FeedSlot {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<mpsc::Sender<Attributed>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Open a device over `transport`, a link the app carries, and run the
    /// connect sequence; `serial` names it for the picker. Replaces any open
    /// device. From now on [`DspiCore::host_notify`] feeds this connection.
    /// Errors as for the connect sequence; a host that fails during it (an
    /// error, a hang, a reply of the wrong length) leaves nothing connected.
    pub fn open_host(&self, transport: Arc<dyn HostTransport>, serial: String) -> Result<()> {
        self.open_host_with(transport, serial, HANG_LIMIT)
    }

    /// A notification packet from the device of the [`DspiCore::open_host`]
    /// connection, as its endpoint delivered it, with the hub's `origin` (the
    /// session whose write caused it, 0 for a change at the device; `None`
    /// over USB and MIDI). From any thread; never blocks beyond a short lock.
    /// Ignored without an open host connection, for an empty packet, and
    /// after the connection ended. Packets fed while a later `open_host`
    /// connects go to the new connection: stop feeding the old device first.
    pub fn host_notify(&self, packet: Vec<u8>, origin: Option<u16>) {
        if packet.is_empty() {
            return;
        }
        if let Some(feed) = self.host_feed.lock().as_ref() {
            // A send fails only when the connection's reader has gone: the
            // packet then has nobody to go to.
            let _ = feed.send(Attributed { packet, origin });
        }
    }
}

impl DspiCore {
    /// [`DspiCore::open_host`] with the hang limit given (tests shorten it).
    pub(crate) fn open_host_with(
        &self,
        transport: Arc<dyn HostTransport>,
        serial: String,
        hang_limit: Duration,
    ) -> Result<()> {
        let (adapter, feed) = Adapter::new(transport, serial.clone(), hang_limit);
        // Before the connect, so packets that come while it probes are read.
        *self.host_feed.lock() = Some(feed);
        // -- WP PGWIRE -- the generation wrapper; a host's adapter carries no
        // page, so it is legacy without a probe (OD-PGk)
        let adapter = dspi_transport::paged::open(Box::new(adapter))?;
        self.open_transport(adapter, serial, 0)
    }
}

/// A [`HostTransport`] as a [`Transport`].
struct Adapter {
    host: Arc<dyn HostTransport>,
    kind: HostTransportKind,
    descriptor: DeviceDescriptor,
    hang_limit: Duration,
    /// Set when the host is gone or hung; shared with the notification source.
    gone: Arc<AtomicBool>,
    /// The notification source's receiving half, until the reader takes it.
    notifications: Mutex<Option<mpsc::Receiver<Attributed>>>,
    /// Kept so the stream ends only with the transport, never because the
    /// core replaced its sending half.
    _feed: mpsc::Sender<Attributed>,
    // -- WP CG26 --
    /// The host's device lock is held: given back when the transport drops.
    locked: bool,
}

impl Adapter {
    fn new(
        host: Arc<dyn HostTransport>,
        serial: String,
        hang_limit: Duration,
    ) -> (Self, mpsc::Sender<Attributed>) {
        let (feed, rx) = mpsc::channel();
        let kind = host.kind();
        let adapter = Self {
            host,
            kind,
            descriptor: DeviceDescriptor {
                serial,
                bus_id: "host".into(),
                address: 0,
            },
            hang_limit,
            gone: Arc::new(AtomicBool::new(false)),
            notifications: Mutex::new(Some(rx)),
            _feed: feed.clone(),
            locked: false,
        };
        (adapter, feed)
    }

    /// Make one host call: refused at once once the host is gone, and the
    /// end of the connection when it fails with `Disconnected` or comes back
    /// too late.
    fn call<T>(&self, op: Op, f: impl FnOnce(&dyn HostTransport) -> Result<T>) -> TResult<T> {
        if self.gone.load(Ordering::Acquire) {
            return Err(TransportError::Disconnected);
        }
        let started = Instant::now();
        let r = f(&*self.host);
        if started.elapsed() > self.hang_limit {
            log::warn!(
                "the host took {:?} over {op}; taken as unplugged",
                started.elapsed()
            );
            self.gone.store(true, Ordering::Release);
            return Err(TransportError::Disconnected);
        }
        r.map_err(|e| {
            let e = host_error(e, op);
            if matches!(e, TransportError::Disconnected) {
                self.gone.store(true, Ordering::Release);
            }
            e
        })
    }

    fn read(&self, opcode: u8, value: u16, len: u16, exact: bool) -> TResult<Vec<u8>> {
        let reply = self.call(Op::Transfer(opcode), |h| h.control_in(opcode, value, len))?;
        let wanted = usize::from(len);
        if reply.len() > wanted {
            return Err(TransportError::Malformed(format!(
                "the host returned {} bytes for opcode 0x{opcode:02X}, {wanted} asked",
                reply.len()
            )));
        }
        if exact && reply.len() < wanted {
            return Err(TransportError::ShortRead {
                opcode,
                wanted,
                got: reply.len(),
            });
        }
        Ok(reply)
    }
}

type TResult<T> = dspi_transport::Result<T>;

/// What a host call is for, for its error and its log line.
#[derive(Debug, Clone, Copy)]
enum Op {
    /// A control transfer with this opcode.
    Transfer(u8),
    /// `lock` or `unlock`.
    Lock,
}

impl std::fmt::Display for Op {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Op::Transfer(opcode) => write!(f, "opcode 0x{opcode:02X}"),
            Op::Lock => f.write_str("the device lock"),
        }
    }
}

/// A host's error as the transport error the layers above expect. A
/// refusal of a transfer is a stall; of the lock, an error with its words.
fn host_error(e: DspiError, op: Op) -> TransportError {
    match (e, op) {
        (DspiError::Disconnected, _) => TransportError::Disconnected,
        (DspiError::Refused { .. } | DspiError::Unsupported, Op::Transfer(opcode)) => {
            TransportError::Stalled { opcode }
        }
        (DspiError::Locked, _) => TransportError::Locked,
        (e @ DspiError::Denied, _) => TransportError::Denied(e.to_string()),
        (DspiError::Transfer { msg }, _) => TransportError::Remote(msg),
        (other, _) => TransportError::Remote(other.to_string()),
    }
}

impl Transport for Adapter {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> TResult<Vec<u8>> {
        self.read(opcode, value, len, true)
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> TResult<Vec<u8>> {
        self.read(opcode, value, max_len, false)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> TResult<()> {
        self.call(Op::Transfer(opcode), |h| {
            h.control_out(opcode, value, data.to_vec())
        })
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    /// USB's safe size over USB and the fake device; over MIDI and a hub the
    /// host carries a whole bulk transfer (a hub chunks it itself), up to
    /// `wLength`'s limit.
    fn max_transfer(&self) -> usize {
        match self.kind {
            HostTransportKind::Usb | HostTransportKind::Fake => 4096,
            HostTransportKind::Midi | HostTransportKind::Link { .. } => usize::from(u16::MAX),
        }
    }

    /// The feed, once: the connection has one reader.
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        let rx = self
            .notifications
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()?;
        Some(Box::new(Feed {
            rx,
            gone: Arc::clone(&self.gone),
        }))
    }

    fn caps(&self) -> TransportCaps {
        let (kind, chunked_bulk, remote) = match self.kind {
            HostTransportKind::Usb => (TransportKind::Usb, true, false),
            HostTransportKind::Midi => (TransportKind::Midi, false, false),
            HostTransportKind::Link { .. } => (TransportKind::Link, false, true),
            HostTransportKind::Fake => (TransportKind::Fake, true, false),
        };
        TransportCaps {
            kind,
            chunked_bulk,
            remote,
        }
    }

    fn session_id(&self) -> Option<u16> {
        match self.kind {
            HostTransportKind::Link { session } => Some(session),
            _ => None,
        }
    }

    // -- WP CG26 --
    fn lock(&mut self, reason: &str, timeout: Duration) -> TResult<()> {
        let timeout_ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        self.call(Op::Lock, |h| h.lock(reason.to_owned(), timeout_ms))?;
        self.locked = true;
        Ok(())
    }

    fn unlock(&mut self) {
        if std::mem::take(&mut self.locked) {
            let _ = self.call(Op::Lock, |h| {
                h.unlock();
                Ok(())
            });
        }
    }
}

// -- WP CG26 --
impl Drop for Adapter {
    /// A connection closed with the hub's lock held gives it back, so other
    /// sessions are not refused until its lease runs out. Not to a host that
    /// is gone.
    fn drop(&mut self) {
        Transport::unlock(self);
    }
}

/// The notification stream [`DspiCore::host_notify`] feeds.
struct Feed {
    rx: mpsc::Receiver<Attributed>,
    gone: Arc<AtomicBool>,
}

impl NotificationSource for Feed {
    fn read(&mut self, timeout: Duration) -> TResult<Vec<u8>> {
        Ok(self.read_attributed(timeout)?.packet)
    }

    fn read_attributed(&mut self, timeout: Duration) -> TResult<Attributed> {
        if self.gone.load(Ordering::Acquire) {
            return Err(TransportError::Disconnected);
        }
        match self.rx.recv_timeout(timeout) {
            Ok(a) => Ok(a),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(Attributed {
                packet: Vec::new(),
                origin: None,
            }),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(TransportError::Disconnected),
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "host_transport/tests_wp_cg26.rs"]
mod tests_wp_cg26;
