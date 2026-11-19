//! The app's way to a device: a [`Transport`] inside the hub's process.
//!
//! [`LocalTransport`] is what the app hands to `DspiCore::open_transport`. Commands
//! go through the same router and worker as a network client's, as session
//! [`LOCAL_SESSION_ID`]. Its kind is [`TransportKind::Link`] and its session id is
//! real, so the core's echo rule applies unchanged: a change is ours iff the hub's
//! `origin` is our id (`dspi_transport::is_own_write`). See HUB.md section 4.

use std::sync::mpsc;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use dspi_link::frames::{CmdRequest, CmdResponse};
use dspi_link::{Dir, status};
use dspi_proto::VENDOR_INTERFACE;
use dspi_proto::generated::notify as nfy;
use dspi_transport::link::map_status;
use dspi_transport::{
    Attributed, BUSY_RETRIES, DeviceDescriptor, NotificationSource, Result, Transport,
    TransportCaps, TransportError, TransportKind,
};

use crate::device::DeviceShared;
use crate::hub::{HubHandle, HubInner};
use crate::relay::Relay;
use crate::session::{Completion, LOCAL_SESSION_ID, NO_ORIGIN, Outbound, Outbox, Pop, Session};

/// The longest a caller waits for an answer. Every command is answered (the transport
/// bounds each one, and `Completion` answers the unrun), so this only guards a wedged
/// transport, as `HubTransport.swift`'s `syncWaitTimeout` does. The core may hold its
/// lock for this long, the same order as a Link command's own 10 s.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);
/// A vendor request's most (`wLength` is a u16). No wire limit applies in-process.
const MAX_TRANSFER: usize = u16::MAX as usize;

/// A v2 `BULK_INVALIDATED` with source `UNKNOWN`: what a RESYNC reads as. The core
/// re-reads on it, which is the whole recovery. Reference: `HubTransport.swift`.
fn synthetic_invalidation() -> Vec<u8> {
    vec![
        2,
        nfy::NOTIFY_EVT_BULK_INVALIDATED as u8,
        0,
        0,
        nfy::PARAM_SRC_UNKNOWN as u8,
        0,
        0,
        0,
    ]
}

/// One device of the hub, from the app's side.
///
/// It is bound to one attachment: once that device goes, every call fails with
/// [`TransportError::Disconnected`], even if the same board comes back. The core
/// closes it and opens a new one (HUB.md section 5).
pub struct LocalTransport {
    device: Arc<DeviceShared>,
    session: Arc<Session>,
    relay: Arc<Relay>,
    subscriber: u64,
    outbox: Arc<Outbox>,
    descriptor: DeviceDescriptor,
    locked: bool,
    answer_timeout: Duration,
    /// The hub, to end another session's guarded apply when this takes the lock
    /// (-- WP P1BFIX -- F2). Weak: a transport never keeps a stopped hub alive.
    hub: Weak<HubInner>,
}

impl LocalTransport {
    /// `subscriber` already follows the device into `outbox`, so nothing is lost
    /// between here and the core starting its reader.
    pub(crate) fn new(
        device: Arc<DeviceShared>,
        session: Arc<Session>,
        relay: Arc<Relay>,
        subscriber: u64,
        outbox: Arc<Outbox>,
    ) -> Self {
        let descriptor = DeviceDescriptor {
            serial: device.serial(),
            bus_id: "hub".into(),
            address: device.handle,
            ..Default::default()
        };
        Self {
            device,
            session,
            relay,
            subscriber,
            outbox,
            descriptor,
            locked: false,
            answer_timeout: ANSWER_TIMEOUT,
            hub: Weak::new(),
        }
    }

    // -- WP P1BFIX -- F2
    /// The hub this transport's device is on.
    pub(crate) fn with_hub(mut self, hub: Weak<HubInner>) -> Self {
        self.hub = hub;
        self
    }

    /// Wait this long for an answer instead of the standard 10 s.
    pub fn with_answer_timeout(mut self, timeout: Duration) -> Self {
        self.answer_timeout = timeout;
        self
    }

    fn command(&self, dir: Dir, opcode: u8, value: u16, len: u16, payload: &[u8]) -> CmdResponse {
        let (tx, rx) = mpsc::sync_channel(1);
        let req = CmdRequest {
            tag: 0,
            handle: self.device.handle,
            dir,
            b_request: opcode,
            w_value: value,
            w_index: VENDOR_INTERFACE,
            w_length: len,
            payload: payload.to_vec(),
        };
        let done = Completion::new(0, move |r| {
            let _ = tx.send(r);
        });
        let deadline = Instant::now().checked_add(self.answer_timeout);
        self.device.submit(self.session.ctx(), req, done, deadline);
        rx.recv_timeout(self.answer_timeout)
            .unwrap_or_else(|e| CmdResponse {
                tag: 0,
                status: match e {
                    mpsc::RecvTimeoutError::Timeout => status::TIMEOUT,
                    mpsc::RecvTimeoutError::Disconnected => status::NO_DEVICE,
                },
                payload: Vec::new(),
            })
    }

    /// One GET; the reply may be shorter than `len`, never longer.
    fn get(&self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        let mut r = self.command(Dir::Get, opcode, value, len, &[]);
        check(r.status, opcode)?;
        r.payload.truncate(usize::from(len));
        Ok(r.payload)
    }
}

/// A status as the transport's error. The hub has already run any busy window this
/// command gets and bounded the wait, so a timeout says so and the core does not run
/// its own. Reads only, in any case: a write over the hub is never retried.
fn check(st: u8, opcode: u8) -> Result<()> {
    if st == status::TIMEOUT {
        return Err(TransportError::TimedOut {
            opcode,
            retries: BUSY_RETRIES,
        });
    }
    match map_status(st, opcode) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

impl Transport for LocalTransport {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        let payload = self.get(opcode, value, len)?;
        if payload.len() < usize::from(len) {
            return Err(TransportError::ShortRead {
                opcode,
                wanted: usize::from(len),
                got: payload.len(),
            });
        }
        Ok(payload)
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> Result<Vec<u8>> {
        self.get(opcode, value, max_len)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        let Ok(len) = u16::try_from(data.len()) else {
            return Err(TransportError::Remote(format!(
                "{} bytes for opcode 0x{opcode:02X} exceed a vendor request",
                data.len()
            )));
        };
        check(
            self.command(Dir::Set, opcode, value, len, data).status,
            opcode,
        )
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    fn max_transfer(&self) -> usize {
        MAX_TRANSFER
    }

    /// Take it once, as with `LinkTransport`: a second source would split the
    /// stream between the two.
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        Some(Box::new(LocalNotifications {
            outbox: Arc::clone(&self.outbox),
        }))
    }

    /// `chunked_bulk` is false as for any hub client: the worker chunks a
    /// single-shot request inside one command, so no other session can land between
    /// the chunks. `remote` is false: no network between the app and the device.
    fn caps(&self) -> TransportCaps {
        TransportCaps {
            kind: TransportKind::Link,
            chunked_bulk: false,
            remote: false,
        }
    }

    fn session_id(&self) -> Option<u16> {
        Some(LOCAL_SESSION_ID)
    }

    fn lock(&mut self, _reason: &str, timeout: Duration) -> Result<()> {
        let id = self.session.id();
        if !self.device.router.lock_acquire(id, timeout, Instant::now()) {
            return Err(TransportError::Locked);
        }
        self.locked = true;
        // -- WP P1BFIX -- F2: another session's guarded apply here ends, kept.
        if let Some(inner) = self.hub.upgrade() {
            HubHandle { inner }.apply_lock_taken(&self.descriptor.serial, id);
        }
        Ok(())
    }

    fn unlock(&mut self) {
        if std::mem::take(&mut self.locked) {
            self.device.router.lock_release(self.session.id());
        }
    }
}

impl Drop for LocalTransport {
    fn drop(&mut self) {
        self.unlock();
        self.relay.remove(self.subscriber);
        self.outbox.close();
    }
}

/// The app's notification stream. Ends with `Disconnected` when the device or the
/// hub goes, which is what `poll_hotplug` acts on, as for a USB unplug.
struct LocalNotifications {
    outbox: Arc<Outbox>,
}

impl NotificationSource for LocalNotifications {
    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        self.read_attributed(timeout).map(|a| a.packet)
    }

    fn read_attributed(&mut self, timeout: Duration) -> Result<Attributed> {
        let attributed = |packet, origin| Attributed {
            packet,
            origin: Some(origin),
        };
        match self.outbox.pop(timeout) {
            Pop::Item(Outbound::Notify { packet, origin, .. }) => {
                Ok(attributed(packet.to_vec(), origin))
            }
            Pop::Item(Outbound::Resync { .. }) => {
                Ok(attributed(synthetic_invalidation(), NO_ORIGIN))
            }
            Pop::Closed => Err(TransportError::Disconnected),
            // A device-bound outbox gets nothing else.
            Pop::Item(_) | Pop::Idle => Ok(attributed(Vec::new(), NO_ORIGIN)),
        }
    }
}

#[cfg(test)]
mod tests_wp_hb0 {
    use super::*;

    #[test]
    fn wp_hb0_a_resync_reads_as_a_foreign_invalidation() {
        let outbox = Arc::new(Outbox::new());
        outbox.push_resync(0, 1);
        let mut n = LocalNotifications {
            outbox: Arc::clone(&outbox),
        };
        let a = n.read_attributed(Duration::ZERO).unwrap();
        assert_eq!(a.origin, Some(NO_ORIGIN));
        assert!(matches!(
            crate::relay::packet_source(&a.packet),
            Some((src, crate::relay::PacketKind::Bulk)) if u16::from(src) == nfy::PARAM_SRC_UNKNOWN
        ));
    }

    #[test]
    fn wp_hb0_silence_is_empty_and_a_closed_outbox_is_the_end() {
        let outbox = Arc::new(Outbox::new());
        let mut n = LocalNotifications {
            outbox: Arc::clone(&outbox),
        };
        assert!(n.read(Duration::ZERO).unwrap().is_empty());
        outbox.close();
        assert!(matches!(
            n.read(Duration::ZERO),
            Err(TransportError::Disconnected)
        ));
    }

    #[test]
    fn wp_hb0_a_timeout_is_final_for_the_core_and_a_stall_is_a_stall() {
        assert!(matches!(
            check(status::TIMEOUT, 1),
            Err(TransportError::TimedOut { retries, .. }) if retries > 0
        ));
        assert!(matches!(
            check(status::ERROR, 1),
            Err(TransportError::Stalled { opcode: 1 })
        ));
        assert!(matches!(
            check(status::NO_DEVICE, 1),
            Err(TransportError::Disconnected)
        ));
        assert!(matches!(
            check(status::LOCKED, 1),
            Err(TransportError::Locked)
        ));
        assert!(check(status::OK, 1).is_ok());
    }
}
