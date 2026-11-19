//! A [`Transport`] that submits as a principal's child session (WP P2PA).
//!
//! Modelled on [`crate::local::LocalTransport`], the app's in-process transport, with the
//! one difference the session-id rule (PLAN-P2 M2) asks for: every command goes in as
//! the child session ([`ChildSession`]), and [`Transport::session_id`] answers **that
//! session's own id**, never `LOCAL_SESSION_ID`. So the router judges each command by
//! the child's role, the limits judge it (`limits::admit` exempts only the app's session
//! and the guard's write-back), the voltage rule judges it (the router skips only the
//! app's session), the lock is the child's own, and a core on top tells its own echoes
//! by the child's id while every other session sees a foreign write.
//!
//! A hub refusal comes back as the core reads one from a network hub: a `DENIED` with a
//! named payload (`limit_exceeded`, `installer_lock`, `image_refused`) is mapped by
//! [`dspi_transport::link::map_response`], so the core answers its typed error.

use std::sync::mpsc;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use dspi_link::frames::{CmdRequest, CmdResponse};
use dspi_link::{Dir, status};
use dspi_proto::VENDOR_INTERFACE;
use dspi_proto::generated::notify as nfy;
use dspi_transport::link::map_response;
use dspi_transport::{
    Attributed, BUSY_RETRIES, DeviceDescriptor, NotificationSource, Result, Transport,
    TransportCaps, TransportError, TransportKind,
};

use crate::device::DeviceShared;
use crate::error::HubError;
use crate::hub::{HubHandle, HubInner};
use crate::relay::{Relay, Scope};
use crate::session::{
    ChildSession, Completion, NO_ORIGIN, Outbound, Outbox, Pop, Session, SessionId,
};

/// The longest a caller waits for an answer, as the app's transport: the worker answers
/// every command, so this only guards a wedged transport.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);
/// A network session's most per command (the router refuses larger with `TOO_LARGE`):
/// the core and the reader chunk to it, as over DSPi Link.
const MAX_TRANSFER: usize = crate::router::MAX_PAYLOAD;

/// A v2 `BULK_INVALIDATED` with source `UNKNOWN`: what a RESYNC reads as (the core reads
/// the image again on it), as for the app's transport.
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

/// One device of the hub, as a principal's child session sees it. Bound to one
/// attachment: once that device goes, every call fails with
/// [`TransportError::Disconnected`].
pub struct PrincipalTransport {
    device: Arc<DeviceShared>,
    session: Arc<Session>,
    /// The principal it acts for: a guarded apply of its own is not ended by its lock.
    parent: SessionId,
    relay: Arc<Relay>,
    /// The notification subscription, for a transport that has one.
    follow: Option<(u64, Arc<Outbox>)>,
    descriptor: DeviceDescriptor,
    locked: bool,
    answer_timeout: Duration,
    hub: Weak<HubInner>,
}

impl HubHandle {
    /// A transport to the online device with `serial` that submits as `child`. With
    /// `notifications`, it follows the device's notifications from here on (for a core
    /// on top); without, it has none (a reader).
    ///
    /// Errors: [`HubError::Stopped`]; [`HubError::NoDevice`] when no device with that
    /// serial is online.
    pub fn open_as(
        &self,
        serial: &str,
        child: &ChildSession,
        notifications: bool,
    ) -> std::result::Result<PrincipalTransport, HubError> {
        if self.is_stopped() {
            return Err(HubError::Stopped);
        }
        let device = self.device_by_serial(serial).ok_or(HubError::NoDevice)?;
        // The child acts for its parent's device lock here (`ChildSession`), while the
        // parent is open: looked at again after, as its close may have come between.
        device.router.set_acts_for(child.id(), child.parent_id());
        if child.parent_gone(self) {
            device.router.clear_acts_for(child.id());
            return Err(HubError::Stopped);
        }
        let follow = notifications.then(|| {
            let outbox = Arc::new(Outbox::new());
            let id = self.inner.relay.add(
                child.id(),
                Scope::Attachment(device.handle),
                Arc::clone(&outbox),
            );
            (id, outbox)
        });
        let transport = PrincipalTransport {
            descriptor: DeviceDescriptor {
                serial: device.serial(),
                bus_id: "hub".into(),
                address: device.handle,
                ..Default::default()
            },
            device: Arc::clone(&device),
            session: Arc::clone(child.session()),
            parent: child.parent_id(),
            relay: Arc::clone(&self.inner.relay),
            follow,
            locked: false,
            answer_timeout: ANSWER_TIMEOUT,
            hub: Arc::downgrade(&self.inner),
        };
        // Subscribed after the look-up: a detach in between would never close the new
        // outbox, so look again (the transport's drop removes the subscription).
        match self.device_by_serial(serial) {
            Some(d) if Arc::ptr_eq(&d, &device) => Ok(transport),
            _ => Err(HubError::NoDevice),
        }
    }
}

impl PrincipalTransport {
    /// The session it submits as.
    pub fn session_id_of(&self) -> SessionId {
        self.session.id()
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

    fn get(&self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        let mut r = self.command(Dir::Get, opcode, value, len, &[]);
        check(&r, opcode)?;
        r.payload.truncate(usize::from(len));
        Ok(r.payload)
    }
}

/// A response as the transport's error. A hub's named refusal keeps its name; a timeout
/// is final (the hub ran the busy window and bounded the wait).
fn check(r: &CmdResponse, opcode: u8) -> Result<()> {
    if r.status == status::TIMEOUT {
        return Err(TransportError::TimedOut {
            opcode,
            retries: BUSY_RETRIES,
        });
    }
    match map_response(r.status, opcode, &r.payload) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

impl Transport for PrincipalTransport {
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
        check(&self.command(Dir::Set, opcode, value, len, data), opcode)
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    fn max_transfer(&self) -> usize {
        MAX_TRANSFER
    }

    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.follow.as_ref().map(|(_, outbox)| {
            Box::new(ChildNotifications {
                outbox: Arc::clone(outbox),
            }) as Box<dyn NotificationSource>
        })
    }

    /// A hub client's: the worker chunks a single-shot bulk request inside one command;
    /// no network between it and the device.
    fn caps(&self) -> TransportCaps {
        TransportCaps {
            kind: TransportKind::Link,
            chunked_bulk: false,
            remote: false,
        }
    }

    /// The child session's own id: the session-id rule (never `LOCAL_SESSION_ID`).
    fn session_id(&self) -> Option<u16> {
        Some(self.session.id())
    }

    fn lock(&mut self, _reason: &str, timeout: Duration) -> Result<()> {
        let id = self.session.id();
        if !self.device.router.lock_acquire(id, timeout, Instant::now()) {
            return Err(TransportError::Locked);
        }
        self.locked = true;
        // Another session's guarded apply here ends, kept (P1BFIX F2), as when the app
        // takes the lock; the principal's own guard (a guarded recall) goes on.
        if let Some(inner) = self.hub.upgrade() {
            HubHandle { inner }.apply_lock_taken(&self.descriptor.serial, self.parent);
        }
        Ok(())
    }

    fn unlock(&mut self) {
        if std::mem::take(&mut self.locked) {
            self.device.router.lock_release(self.session.id());
        }
    }
}

impl Drop for PrincipalTransport {
    fn drop(&mut self) {
        self.unlock();
        if let Some((id, outbox)) = self.follow.take() {
            self.relay.remove(id);
            outbox.close();
        }
    }
}

/// The child's notification stream; ends with `Disconnected` when the device or the
/// hub goes.
struct ChildNotifications {
    outbox: Arc<Outbox>,
}

impl NotificationSource for ChildNotifications {
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
            Pop::Item(_) | Pop::Idle => Ok(attributed(Vec::new(), NO_ORIGIN)),
        }
    }
}

#[cfg(test)]
mod tests_wp_p2pa {
    use super::*;

    #[test]
    fn wp_p2pa_a_named_hub_refusal_keeps_its_name() {
        let r = CmdResponse {
            tag: 0,
            status: status::DENIED,
            payload: br#"{"refused":"limit_exceeded","limit":"out_gain","msg":"over"}"#.to_vec(),
        };
        let e = check(&r, 1).unwrap_err();
        assert_eq!(e.hub_refusal().map(|(k, _)| k), Some("limit_exceeded"));
        let plain = CmdResponse {
            tag: 0,
            status: status::DENIED,
            payload: Vec::new(),
        };
        assert!(matches!(check(&plain, 1), Err(TransportError::Denied(_))));
        let late = CmdResponse {
            tag: 0,
            status: status::TIMEOUT,
            payload: Vec::new(),
        };
        assert!(matches!(
            check(&late, 1),
            Err(TransportError::TimedOut { .. })
        ));
    }
}
