//! DSPi Link: one device on a hub, as a [`Transport`].
//!
//! Ported from the Android console's `dspi-ffi/src/link.rs`. Status mapping
//! (Link spec 8.1) onto [`TransportError`]: ERROR/BLOCKED -> `Stalled` (the
//! RTA bands-all fallback depends on it), TIMEOUT -> `TimedOut`, NO_DEVICE ->
//! `Disconnected`, DENIED -> `Denied`, LOCKED -> `Locked`, the rest ->
//! `Remote`. Bulk is single-shot (`caps().chunked_bulk == false`); the first
//! bulk read may come from the hub's snapshot. Notifications carry the
//! writer's session id as their origin.
//!
//! Busy window (spec 8.1): BUSY, BULK_LOCKED and CRC_ERROR are retried here,
//! up to [`BUSY_RETRIES`] times [`BUSY_BACKOFF`] apart, then reported as
//! [`TransportError::TimedOut`]. CRC_ERROR guarantees the command did not run,
//! and BUSY/BULK_LOCKED are refused before it does, so even a SET is safe to
//! repeat.
//!
//! Time bounds (§1.7): every call goes through `dspi-link`, which bounds a
//! command by its own timeouts (10 s, 15 s for bulk) and answers NO_DEVICE
//! once the session is closed, so a hub cannot block a caller forever.
//!
//! Owned by WP T5 (docs/core-merge/PLAN.md).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use dspi_link::frames::CmdResponse;
use dspi_link::{CmdSpec, Dir, LinkClient, LinkError, NotifyItem, status};
use dspi_proto::generated::opcodes as op;

use crate::{
    Attributed, BUSY_BACKOFF, BUSY_RETRIES, DeviceDescriptor, InRead, NotificationSource, Result,
    Transport, TransportCaps, TransportError,
};

/// The hub capability that offers `device.snapshot` (spec 7.5).
const CAP_SNAPSHOT: &str = "snapshot";
/// The hub capability that offers `lock.acquire` / `lock.release` (spec 7.7).
const CAP_LOCK: &str = "lock";
/// Longest lock a client may ask for (spec 7.7; `dspi-link` clamps too).
const LOCK_MAX_MS: u32 = 60_000;
/// What [`Transport::max_transfer`] reports when the hub's `max_payload` does
/// not fit a `usize` (spec 7.2's default).
const MAX_PAYLOAD_DEFAULT: usize = 8192;
/// Bounds for the hub's `max_payload`. Spec 7.2 has hubs advertise at least
/// 8192; a 0 or tiny value would make a chunked reader spin, and a vendor
/// request carries at most `u16::MAX` bytes.
const MAX_PAYLOAD_MIN: usize = 64;
const MAX_PAYLOAD_MAX: usize = 65_535;

/// Map a Link CMD status onto a transport error (`None` for OK).
///
/// `opcode` is the request's `bRequest`, kept in the errors that name one.
/// ERROR and BLOCKED are a STALL: capability probes and the analyser's
/// bands-all fallback rely on that (a bridge answers RTA bands-all ERROR and
/// the chunked bulk opcodes BLOCKED). TIMEOUT means the hub gave up on the
/// device; the command may have run, so it is never retried blindly here.
/// BUSY, BULK_LOCKED and CRC_ERROR map to `Remote` here; [`LinkTransport`]
/// retries them first (its busy window) and never returns them as such.
pub fn map_status(status: u8, opcode: u8) -> Option<TransportError> {
    Some(match status {
        status::OK => return None,
        status::ERROR | status::BLOCKED => TransportError::Stalled { opcode },
        status::TIMEOUT => TransportError::TimedOut { opcode, retries: 0 },
        status::NO_DEVICE => TransportError::Disconnected,
        status::DENIED => TransportError::Denied(format!(
            "opcode 0x{opcode:02X} is not allowed for this client"
        )),
        status::LOCKED => TransportError::Locked,
        s => TransportError::Remote(format!(
            "the hub answered {} (0x{s:02X}) to opcode 0x{opcode:02X}",
            status::name(s)
        )),
    })
}

/// Whether a Link status is a busy window to retry (spec 8.1).
fn is_busy(status: u8) -> bool {
    matches!(
        status,
        status::BUSY | status::BULK_LOCKED | status::CRC_ERROR
    )
}

/// Run `attempt` through the busy window: retry BUSY, BULK_LOCKED and
/// CRC_ERROR up to [`BUSY_RETRIES`] times, `backoff` apart; after that the
/// request is `TimedOut { retries: BUSY_RETRIES }`. Any other status is
/// mapped by [`map_status`]; OK yields the reply's payload.
fn busy_window(
    opcode: u8,
    backoff: Duration,
    mut attempt: impl FnMut() -> CmdResponse,
) -> Result<Vec<u8>> {
    for i in 0..=BUSY_RETRIES {
        let r = attempt();
        if !is_busy(r.status) {
            return match map_status(r.status, opcode) {
                Some(e) => Err(e),
                None => Ok(r.payload),
            };
        }
        if i < BUSY_RETRIES {
            crate::clock::sleep(backoff);
        }
    }
    Err(TransportError::TimedOut {
        opcode,
        retries: BUSY_RETRIES,
    })
}

/// A `lock.acquire` failure as a transport error.
fn map_link_error(e: LinkError) -> TransportError {
    match e {
        LinkError::Rejected { code, .. } if code == "locked" => TransportError::Locked,
        LinkError::Rejected { code, msg } if code == "denied" => {
            TransportError::Denied(dspi_link::sanitize_hub_text(&msg))
        }
        LinkError::Closed { .. } => TransportError::Disconnected,
        // A control-plane request has no opcode
        LinkError::Timeout => TransportError::TimedOut {
            opcode: 0,
            retries: 0,
        },
        other => TransportError::Remote(other.to_string()),
    }
}

/// The hub's `max_payload` as a transfer size, clamped to
/// [`MAX_PAYLOAD_MIN`]..=[`MAX_PAYLOAD_MAX`].
fn clamp_max_payload(max_payload: u32) -> usize {
    usize::try_from(max_payload).map_or(MAX_PAYLOAD_DEFAULT, |n| {
        n.clamp(MAX_PAYLOAD_MIN, MAX_PAYLOAD_MAX)
    })
}

/// One device (a hub handle) on a DSPi Link session, as a [`Transport`].
///
/// Dropping it releases a lock it holds and stops the handle's notification
/// stream on the session (any [`NotificationSource`] it handed out then
/// reports [`TransportError::Disconnected`]). Open at most one per handle and
/// session at a time: the stream is per handle, so a second one would take it
/// over and the first one's drop would end it.
///
/// The notification stream is subscribed in [`LinkTransport::new`], so no
/// event is lost before the reader starts, and it is unbounded until read.
/// The embedder must therefore take [`Transport::notifications`] exactly once
/// per transport and drain it for as long as the transport lives; a transport
/// dropped without that logs a warning.
pub struct LinkTransport {
    client: LinkClient,
    handle: u8,
    descriptor: DeviceDescriptor,
    /// The notification stream subscribed at construction, so no event is
    /// lost between opening and starting the reader; the first
    /// [`Transport::notifications`] call takes it, later ones subscribe anew.
    notes: Mutex<Option<Receiver<NotifyItem>>>,
    /// The first bulk read comes from the hub's snapshot when it offers one
    /// (one round trip instead of a device read). Later reads go to the device.
    snapshot_unused: bool,
    /// Whether this transport holds the hub's device lock.
    locked: bool,
}

impl LinkTransport {
    /// Device `handle` on the hub `client` is connected to. Link handles are
    /// one byte on the wire (spec 7.4); the embedder API takes a `u32` for
    /// the bindings and refuses anything above 255 before calling this.
    ///
    /// `serial` is the device's serial from the hub's device list. The
    /// descriptor's `bus_id` names the hub (`link:<hub id>`) and its
    /// `address` is the handle.
    pub fn new(client: LinkClient, handle: u8, serial: &str) -> Self {
        let notes = Mutex::new(Some(client.notifications(handle)));
        let hello = client.hello();
        let snapshot_unused = hello.has_cap(CAP_SNAPSHOT);
        let descriptor = DeviceDescriptor {
            serial: serial.to_owned(),
            bus_id: format!("link:{}", hello.hub.id),
            address: handle,
        };
        Self {
            client,
            handle,
            descriptor,
            notes,
            snapshot_unused,
            locked: false,
        }
    }

    /// The bulk image from the hub's snapshot, cut to `len`, if it has one of
    /// at least `min_len` bytes. Any failure falls back to a device read.
    ///
    /// WP LK1: `age_ms` is not a reason to read the device instead. A hub keeps
    /// the blob current (spec 7.5), and the ESP32 bridge reports the time since
    /// it last read it, hours on a bridge up for hours; the macOS Console and
    /// the 0.10 app ignore it too.
    fn snapshot_bulk(&self, len: u16, min_len: u16) -> Option<Vec<u8>> {
        let mut blob = self.client.snapshot(self.handle).ok()?.bulk()?;
        if blob.is_empty() || blob.len() < usize::from(min_len) {
            return None;
        }
        blob.truncate(usize::from(len));
        Some(blob)
    }

    /// One GET; the reply may be shorter than `len`, never longer. The first
    /// bulk read comes from the snapshot when it has at least `min_len` bytes.
    fn get(&mut self, opcode: u8, value: u16, len: u16, min_len: u16) -> Result<Vec<u8>> {
        if opcode == op::REQ_GET_ALL_PARAMS
            && std::mem::take(&mut self.snapshot_unused)
            && let Some(blob) = self.snapshot_bulk(len, min_len)
        {
            return Ok(blob);
        }
        let mut payload = busy_window(opcode, BUSY_BACKOFF, || {
            self.client.command(
                self.handle,
                Dir::Get,
                opcode,
                value,
                dspi_proto::VENDOR_INTERFACE,
                len,
                &[],
            )
        })?;
        payload.truncate(usize::from(len));
        Ok(payload)
    }
}

// -- WP LK2 --
/// One answer of a pipelined batch, as [`Transport::control_in`] or
/// [`Transport::control_in_upto`] would give it. A busy status, or
/// RATE_LIMITED (a slot the hub still counts for a request of ours that timed
/// out), means the read did not run: it goes again on its own by `again`.
fn batch_answer(
    r: &InRead,
    a: CmdResponse,
    again: impl FnOnce() -> Result<Vec<u8>>,
) -> Result<Vec<u8>> {
    if is_busy(a.status) || a.status == status::RATE_LIMITED {
        return again();
    }
    if let Some(e) = map_status(a.status, r.opcode) {
        return Err(e);
    }
    let mut payload = a.payload;
    payload.truncate(usize::from(r.len));
    if r.exact {
        exact_reply(r.opcode, r.len, payload)
    } else {
        Ok(payload)
    }
}

/// `payload` as [`Transport::control_in`] answers it: refused when short.
fn exact_reply(opcode: u8, len: u16, payload: Vec<u8>) -> Result<Vec<u8>> {
    if payload.len() < usize::from(len) {
        return Err(TransportError::ShortRead {
            opcode,
            wanted: usize::from(len),
            got: payload.len(),
        });
    }
    Ok(payload)
}

impl Transport for LinkTransport {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        let payload = self.get(opcode, value, len, len)?;
        exact_reply(opcode, len, payload)
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> Result<Vec<u8>> {
        // WP LK1: `max_len` is a cap (the probe asks for `max_payload`), so any
        // snapshot serves, as a device reply of that size would
        self.get(opcode, value, max_len, 1)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        let Ok(len) = u16::try_from(data.len()) else {
            return Err(TransportError::Remote(format!(
                "{} bytes for opcode 0x{opcode:02X} exceed a vendor request",
                data.len()
            )));
        };
        busy_window(opcode, BUSY_BACKOFF, || {
            self.client.command(
                self.handle,
                Dir::Set,
                opcode,
                value,
                dspi_proto::VENDOR_INTERFACE,
                len,
                data,
            )
        })
        .map(drop)
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    /// The hub's `max_payload` (spec 7.2; 8192 when it does not say),
    /// clamped to 64..=65535 so a bogus advertisement cannot stall a chunked
    /// reader or exceed a vendor request.
    fn max_transfer(&self) -> usize {
        clamp_max_payload(self.client.hello().limits.max_payload)
    }

    /// The device's notification stream. Take it exactly once and drain it
    /// (see [`LinkTransport`]): the first call returns the stream subscribed
    /// in [`LinkTransport::new`]. Any further call subscribes anew, which
    /// replaces the handle's sink on the session: the earlier source then
    /// reports [`TransportError::Disconnected`].
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        let first = self
            .notes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let rx = first.unwrap_or_else(|| self.client.notifications(self.handle));
        Some(Box::new(LinkNotifications { rx }))
    }

    fn caps(&self) -> TransportCaps {
        TransportCaps {
            kind: crate::TransportKind::Link,
            chunked_bulk: false,
            remote: true,
        }
    }

    fn session_id(&self) -> Option<u16> {
        self.client.session_id()
    }

    /// Spec 7.7: a hub without the `lock` capability serialises commands
    /// itself, so this succeeds at once. Another session's lock is
    /// [`TransportError::Locked`], a viewer's request `Denied`.
    fn lock(&mut self, reason: &str, timeout: Duration) -> Result<()> {
        if !self.client.hello().has_cap(CAP_LOCK) {
            return Ok(());
        }
        let ms = u32::try_from(timeout.as_millis())
            .unwrap_or(LOCK_MAX_MS)
            .min(LOCK_MAX_MS);
        self.client
            .lock_acquire(self.handle, reason, ms)
            .map_err(map_link_error)?;
        self.locked = true;
        Ok(())
    }

    /// Fire and forget: a lost release expires on the hub.
    fn unlock(&mut self) {
        if std::mem::take(&mut self.locked) {
            self.client.lock_release(self.handle);
        }
    }

    // -- WP LK2 --
    /// Up to the hub's `max_inflight` of the reads on the wire at once
    /// ([`LinkClient::commands`]): one round trip for each window of them
    /// instead of one each. A busy answer (spec 8.1) is retried on its own
    /// through the busy window afterwards; a bulk read goes one at a time (it
    /// may come from the snapshot).
    fn control_in_batch(&mut self, reads: &[InRead]) -> Vec<Result<Vec<u8>>> {
        let one = |t: &mut Self, r: &InRead| {
            if r.exact {
                t.control_in(r.opcode, r.value, r.len)
            } else {
                t.control_in_upto(r.opcode, r.value, r.len)
            }
        };
        if reads.len() < 2
            || self.client.max_inflight() < 2
            || reads.iter().any(|r| r.opcode == op::REQ_GET_ALL_PARAMS)
        {
            return reads.iter().map(|r| one(self, r)).collect();
        }
        let specs: Vec<CmdSpec> = reads
            .iter()
            .map(|r| CmdSpec {
                handle: self.handle,
                dir: Dir::Get,
                b_request: r.opcode,
                w_value: r.value,
                w_index: dspi_proto::VENDOR_INTERFACE,
                w_length: r.len,
                payload: Vec::new(),
            })
            .collect();
        let answers = self.client.commands(&specs);
        reads
            .iter()
            .zip(answers)
            .map(|(r, a)| batch_answer(r, a, || one(self, r)))
            .collect()
    }
}

impl Drop for LinkTransport {
    fn drop(&mut self) {
        let untaken = self
            .notes
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some();
        if untaken {
            log::warn!(
                "DSPi Link handle {}: transport dropped without its notifications ever being taken",
                self.handle
            );
        }
        // A closed session holds no lock, and the send could wait on a half-open
        // socket on the caller's thread.
        if !self.client.is_closed() {
            self.unlock();
        }
        self.client.stop_notifications(self.handle);
    }
}

/// The notification stream of one Link device.
struct LinkNotifications {
    rx: Receiver<NotifyItem>,
}

impl NotificationSource for LinkNotifications {
    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        self.read_attributed(timeout).map(|a| a.packet)
    }

    /// The packet with the hub's origin (spec 8.3): the writer's session id,
    /// 0 for a change made at the device. Silence is an empty packet; the
    /// session ending (or the transport dropped) is `Disconnected`.
    fn read_attributed(&mut self, timeout: Duration) -> Result<Attributed> {
        match self.rx.recv_timeout(timeout) {
            Ok(item) => Ok(Attributed {
                packet: item.packet,
                origin: Some(item.origin),
            }),
            Err(RecvTimeoutError::Timeout) => Ok(Attributed {
                packet: Vec::new(),
                origin: None,
            }),
            Err(RecvTimeoutError::Disconnected) => Err(TransportError::Disconnected),
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_t5 {
    use super::*;
    use crate::{TransportKind, is_own_write};
    use dspi_link::fakehub::{FakeHub, HubConfig, OFF_MASTER_VOLUME, req};
    use dspi_link::socket::TcpConnector;
    use dspi_link::{AuthState, ConnectOptions};

    const WAIT: Duration = Duration::from_secs(3);

    fn connect(hub: &FakeHub) -> LinkClient {
        let opts = ConnectOptions {
            client_name: "wp-t5".into(),
            app: "dspi-transport tests".into(),
            app_version: "test".into(),
            token: None,
        };
        let (c, state) = LinkClient::connect(&TcpConnector::default(), &hub.url(), &opts, None)
            .expect("connect to the fake hub");
        assert_eq!(state, AuthState::Ready);
        c
    }

    fn open(hub: &FakeHub) -> LinkTransport {
        let c = connect(hub);
        let serial = c.devices()[0].serial.clone();
        LinkTransport::new(c, 0, &serial)
    }

    fn bulk_reads(hub: &FakeHub) -> usize {
        hub.log()
            .iter()
            .filter(|e| e.2 == req::GET_ALL_PARAMS)
            .count()
    }

    fn wait(mut f: impl FnMut() -> bool) -> bool {
        for _ in 0..300 {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// The next non-empty notification, with its origin.
    fn next_note(src: &mut dyn NotificationSource) -> Attributed {
        for _ in 0..30 {
            let a = src.read_attributed(Duration::from_millis(100)).unwrap();
            if !a.packet.is_empty() {
                return a;
            }
        }
        panic!("no notification within 3 s");
    }

    #[test]
    fn link_statuses_map_onto_transport_errors() {
        use TransportError as E;
        let m = |s| map_status(s, 0x0F);
        assert!(m(status::OK).is_none());
        assert!(
            matches!(m(status::ERROR), Some(E::Stalled { opcode: 0x0F })),
            "RTA bands-all fallback"
        );
        assert!(matches!(
            m(status::BLOCKED),
            Some(E::Stalled { opcode: 0x0F })
        ));
        assert!(matches!(
            m(status::TIMEOUT),
            Some(E::TimedOut { opcode: 0x0F, .. })
        ));
        assert!(matches!(m(status::NO_DEVICE), Some(E::Disconnected)));
        assert!(matches!(m(status::DENIED), Some(E::Denied(_))));
        assert!(matches!(m(status::LOCKED), Some(E::Locked)));
        for s in [
            status::BUSY,
            status::BULK_LOCKED,
            status::CRC_ERROR,
            status::OVERSIZE,
            status::FRAME_ERROR,
            status::TOO_LARGE,
            status::RATE_LIMITED,
            status::UNSUPPORTED,
            status::BAD_FRAME,
            0x42,
        ] {
            match m(s) {
                Some(E::Remote(msg)) => assert!(msg.contains(status::name(s)), "{msg}"),
                other => panic!("status 0x{s:02X} -> {other:?}"),
            }
        }
    }

    #[test]
    fn a_device_on_the_fake_hub_reads_its_bulk_image() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let mut t = open(&hub);
        let len = u16::try_from(hub.image().len()).unwrap();
        assert_eq!(
            t.control_in(req::GET_ALL_PARAMS, 0, len).unwrap(),
            hub.image()
        );
        assert_eq!(bulk_reads(&hub), 0, "the first bulk read is the snapshot");
        assert_eq!(
            t.control_in(req::GET_ALL_PARAMS, 0, len).unwrap(),
            hub.image()
        );
        assert_eq!(bulk_reads(&hub), 1, "later reads go to the device");
    }

    // -- WP LK1 --
    /// The probe's read asks for `max_payload`, more than the image: the
    /// snapshot still serves it, as on the 0.10 app.
    #[test]
    fn wp_lk1_the_probes_capped_bulk_read_comes_from_the_snapshot() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let mut t = open(&hub);
        let max = u16::try_from(t.max_transfer()).unwrap();
        assert!(usize::from(max) > hub.image().len());
        assert_eq!(
            t.control_in_upto(req::GET_ALL_PARAMS, 0, max).unwrap(),
            hub.image()
        );
        assert_eq!(bulk_reads(&hub), 0, "no second bulk transfer");
    }

    /// An ESP32 bridge's snapshot is as old as its last read of the blob, which
    /// its notifications keep current: it is used whatever its age.
    #[test]
    fn wp_lk1_an_old_snapshot_is_used() {
        let hub = FakeHub::start(HubConfig {
            snapshot_age_ms: 3_600_000,
            auth: dspi_link::fakehub::Auth::None,
            ..HubConfig::bridge()
        })
        .unwrap();
        let mut t = open(&hub);
        let len = u16::try_from(hub.image().len()).unwrap();
        assert_eq!(
            t.control_in(req::GET_ALL_PARAMS, 0, len).unwrap(),
            hub.image()
        );
        assert_eq!(bulk_reads(&hub), 0);
    }

    /// An exact read the snapshot is too short for goes to the device.
    #[test]
    fn wp_lk1_an_exact_read_longer_than_the_snapshot_reads_the_device() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let mut t = open(&hub);
        let len = u16::try_from(hub.image().len() + 1).unwrap();
        assert!(matches!(
            t.control_in(req::GET_ALL_PARAMS, 0, len),
            Err(TransportError::ShortRead { .. })
        ));
        assert_eq!(
            bulk_reads(&hub),
            1,
            "read from the device after the snapshot"
        );
    }

    #[test]
    fn wp_t5_bulk_reads_the_device_on_a_hub_without_snapshots() {
        let mut cfg = HubConfig::default();
        cfg.caps.retain(|c| c != CAP_SNAPSHOT);
        let hub = FakeHub::start(cfg).unwrap();
        let mut t = open(&hub);
        let len = u16::try_from(hub.image().len()).unwrap();
        assert_eq!(
            t.control_in(req::GET_ALL_PARAMS, 0, len).unwrap(),
            hub.image()
        );
        assert_eq!(bulk_reads(&hub), 1);
    }

    #[test]
    fn wp_t5_identity_caps_and_limits() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let t = open(&hub);
        assert_eq!(t.descriptor().serial, "0123456789ABCDEF");
        assert_eq!(t.descriptor().address, 0);
        assert!(t.descriptor().bus_id.starts_with("link:"));
        assert_eq!(t.max_transfer(), 8192);
        let caps = t.caps();
        assert_eq!(caps.kind, TransportKind::Link);
        assert!(!caps.chunked_bulk && caps.remote);
        assert_eq!(t.session_id(), Some(12));
    }

    #[test]
    fn wp_t5_short_replies_and_stalls() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let mut t = open(&hub);
        // GET_PLATFORM answers 7 bytes, whatever is asked
        assert_eq!(
            t.control_in_upto(req::GET_PLATFORM, 0, 16).unwrap().len(),
            7
        );
        assert!(matches!(
            t.control_in(req::GET_PLATFORM, 0, 16),
            Err(TransportError::ShortRead {
                wanted: 16,
                got: 7,
                ..
            })
        ));
        assert_eq!(t.control_in(req::GET_PLATFORM, 0, 4).unwrap().len(), 4);
        assert!(matches!(
            t.control_in(0x99, 0, 4),
            Err(TransportError::Stalled { opcode: 0x99 })
        ));
        let mut gone = LinkTransport::new(t.client.clone(), 3, "none");
        assert!(matches!(
            gone.control_in(req::GET_PLATFORM, 0, 7),
            Err(TransportError::Disconnected)
        ));
    }

    #[test]
    fn wp_t5_a_bridge_stalls_chunked_bulk_and_a_hub_timeout_is_timed_out() {
        let hub = FakeHub::start(HubConfig {
            bridge_limits: true,
            timeout_requests: [req::GET_STATUS].into(),
            ..HubConfig::default()
        })
        .unwrap();
        let mut t = open(&hub);
        assert!(matches!(
            t.control_in(req::GET_ALL_PARAMS_CHUNK, 0, 64),
            Err(TransportError::Stalled { .. })
        ));
        assert!(matches!(
            t.control_in(req::RTA_GET_BANDS_ALL, 0, 64),
            Err(TransportError::Stalled { .. })
        ));
        assert!(matches!(
            t.control_in(req::GET_STATUS, 9, 4),
            Err(TransportError::TimedOut { .. })
        ));
        assert!(matches!(
            t.control_out(0x42, 0, &[0; 65]),
            Err(TransportError::Remote(_))
        ));
    }

    /// The hub's text must not reach a terminal or a log with its escapes intact.
    #[test]
    fn wp_l1_a_denied_message_from_the_hub_is_sanitised() {
        let e = map_link_error(LinkError::Rejected {
            code: "denied".into(),
            msg: "no\u{1b}[2J\u{9b}31m\nway".into(),
        });
        match e {
            TransportError::Denied(m) => assert_eq!(m, "no\u{fffd}[2J\u{fffd}31m\u{fffd}way"),
            other => panic!("{other:?}"),
        }
    }

    /// A session past `max_inflight` gets RATE_LIMITED. The transport does not
    /// retry it (the command did not run, and only time frees a slot): it is a
    /// typed error naming the status, at once, and the next command works.
    #[test]
    fn wp_l1_rate_limited_surfaces_as_a_remote_error() {
        let hub = FakeHub::start(HubConfig {
            max_inflight: 1,
            ..HubConfig::default()
        })
        .unwrap();
        let cfg = dspi_link::SessionConfig {
            timing: dspi_link::Timing {
                request_timeout: Duration::from_millis(500),
                ..dspi_link::Timing::default()
            },
            ..dspi_link::SessionConfig::default()
        };
        let opts = ConnectOptions {
            client_name: "wp-l1".into(),
            app: "dspi-transport tests".into(),
            app_version: "test".into(),
            token: None,
        };
        let (c, _) =
            LinkClient::connect_with(&TcpConnector::default(), &hub.url(), &opts, None, &cfg)
                .unwrap();
        let serial = c.devices()[0].serial.clone();
        let mut t = LinkTransport::new(c, 0, &serial);
        hub.hold_replies();
        assert!(matches!(
            t.control_in(req::GET_PLATFORM, 0, 7),
            Err(TransportError::TimedOut { .. })
        ));
        let ran = hub.log().len();
        match t.control_in(req::GET_PLATFORM, 0, 7) {
            Err(TransportError::Remote(m)) => assert!(m.contains("RATE_LIMITED"), "{m}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(hub.log().len(), ran, "not retried, and it did not run");
        hub.release_replies();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(t.control_in(req::GET_PLATFORM, 0, 7).unwrap().len(), 7);
    }

    #[test]
    fn wp_t5_a_set_lands_and_notifies_with_our_origin() {
        use dspi_proto::generated::notify as n;
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let mut t = open(&hub);
        let mut notes = t.notifications().unwrap();
        let session = t.session_id();
        let v = (-12.0f32).to_le_bytes();
        t.control_out(req::SET_MASTER_VOLUME, 0, &v).unwrap();
        let off = usize::from(OFF_MASTER_VOLUME);
        assert_eq!(&hub.image()[off..off + 4], &v);

        // Our own write: the echo carries our session id
        let own = next_note(notes.as_mut());
        assert_eq!(&own.packet[..2], &[0x02, 0x02], "PARAM_CHANGED");
        assert_eq!(own.origin, session);
        assert!(is_own_write(
            TransportKind::Link,
            own.packet[8],
            own.origin,
            session,
            false
        ));

        // Another client's write, and a change at the device (origin 0)
        let host = n::PARAM_SRC_HOST_SET as u8;
        hub.external_change(OFF_MASTER_VOLUME, &(-6.0f32).to_le_bytes(), host, 40);
        let other = next_note(notes.as_mut());
        assert_eq!(other.origin, Some(40));
        assert!(!is_own_write(
            TransportKind::Link,
            other.packet[8],
            other.origin,
            session,
            false
        ));
        hub.external_change(OFF_MASTER_VOLUME, &(-3.0f32).to_le_bytes(), host, 0);
        let device = next_note(notes.as_mut());
        assert_eq!(device.origin, Some(0));
        assert!(!is_own_write(
            TransportKind::Link,
            device.packet[8],
            device.origin,
            session,
            false
        ));

        // Silence is an empty packet, the session ending is Disconnected
        assert!(notes.read(Duration::from_millis(50)).unwrap().is_empty());
        t.client.close();
        assert!(matches!(
            notes.read(WAIT),
            Err(TransportError::Disconnected)
        ));
        assert!(matches!(
            t.control_out(req::SET_MASTER_VOLUME, 0, &v),
            Err(TransportError::Disconnected)
        ));
    }

    #[test]
    fn wp_t5_dropping_the_transport_ends_its_notifications() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let t = open(&hub);
        let mut notes = t.notifications().unwrap();
        drop(t);
        assert!(matches!(
            notes.read(WAIT),
            Err(TransportError::Disconnected)
        ));
    }

    #[test]
    fn wp_t5_the_device_lock() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let mut t = open(&hub);
        let sid = t.session_id().unwrap();
        let v = (-20.0f32).to_le_bytes();

        // Another console holds it: our writes and our lock are refused
        hub.hold_lock(99, 10_000);
        assert!(matches!(
            t.control_out(req::SET_MASTER_VOLUME, 0, &v),
            Err(TransportError::Locked)
        ));
        assert!(matches!(
            t.lock("Saving preset", Duration::from_secs(5)),
            Err(TransportError::Locked)
        ));
        hub.release_lock();

        t.lock("Saving preset", Duration::from_secs(5)).unwrap();
        assert_eq!(hub.lock_holder(), Some(sid));
        t.control_out(req::SET_MASTER_VOLUME, 0, &v).unwrap();
        t.unlock();
        assert!(wait(|| hub.lock_holder().is_none()), "unlock releases");

        // A dropped transport gives its lock back
        t.lock("Restoring", Duration::from_secs(120)).unwrap();
        assert_eq!(hub.lock_holder(), Some(sid));
        drop(t);
        assert!(wait(|| hub.lock_holder().is_none()), "drop releases");
    }

    #[test]
    fn wp_t5_a_hub_without_locks_grants_at_once() {
        let mut cfg = HubConfig::default();
        cfg.caps.retain(|c| c != CAP_LOCK);
        let hub = FakeHub::start(cfg).unwrap();
        let mut t = open(&hub);
        t.lock("Saving preset", Duration::from_secs(5)).unwrap();
        assert_eq!(hub.lock_holder(), None);
        t.unlock();
    }

    /// A scripted status sequence through the busy window; `calls` counts
    /// the attempts.
    fn scripted(statuses: &[u8]) -> (Result<Vec<u8>>, usize) {
        let mut calls = 0;
        let r = busy_window(0xA0, Duration::ZERO, || {
            let status = statuses.get(calls).copied().unwrap_or(status::OK);
            calls += 1;
            CmdResponse {
                tag: 1,
                status,
                payload: vec![7; 3],
            }
        });
        (r, calls)
    }

    // -- WP LK2 --
    #[test]
    fn wp_lk2_a_batch_answers_as_one_read_at_a_time_does() {
        let hub = FakeHub::start(HubConfig {
            max_inflight: 4,
            ..HubConfig::default()
        })
        .unwrap();
        let reads = [
            InRead::exact(req::GET_PLATFORM, 0, 7),
            InRead::upto(req::GET_PLATFORM, 0, 64),
            InRead::exact(req::GET_PLATFORM, 0, 64),
            InRead::exact(0xEE, 0, 4),
            InRead::upto(req::GET_STATUS, 9, 41),
            InRead::exact(req::GET_PLATFORM, 0, 4),
        ];
        let mut t = open(&hub);
        let one: Vec<String> = reads
            .iter()
            .map(|r| {
                let a = if r.exact {
                    t.control_in(r.opcode, r.value, r.len)
                } else {
                    t.control_in_upto(r.opcode, r.value, r.len)
                };
                format!("{a:?}")
            })
            .collect();
        let batch: Vec<String> = t
            .control_in_batch(&reads)
            .iter()
            .map(|a| format!("{a:?}"))
            .collect();
        assert_eq!(batch, one);
        assert!(batch[2].contains("ShortRead"), "{}", batch[2]);
        assert!(batch[3].contains("Stalled"), "{}", batch[3]);
    }

    #[test]
    fn wp_lk2_a_batch_keeps_at_most_max_inflight_on_the_wire() {
        let hub = FakeHub::start(HubConfig {
            max_inflight: 4,
            ..HubConfig::default()
        })
        .unwrap();
        let mut t = open(&hub);
        let base = hub.log().len();
        hub.hold_replies();
        let reads = vec![InRead::exact(req::GET_PLATFORM, 0, 7); 10];
        let batch = std::thread::spawn(move || t.control_in_batch(&reads));
        assert!(wait(|| hub.log().len() == base + 4), "four out at once");
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(hub.log().len(), base + 4, "and no more (no RATE_LIMITED)");
        hub.release_replies();
        let answers = batch.join().unwrap();
        assert_eq!(answers.len(), 10);
        assert!(
            answers
                .iter()
                .all(|a| a.as_ref().is_ok_and(|d| d.len() == 7))
        );
        assert_eq!(hub.log().len(), base + 10);
    }

    #[test]
    fn wp_lk2_a_batch_retries_what_did_not_run_and_nothing_else() {
        let r = InRead::exact(0x7F, 0, 3);
        let answer = |status, payload: Vec<u8>| CmdResponse {
            tag: 1,
            status,
            payload,
        };
        for s in [
            status::BUSY,
            status::BULK_LOCKED,
            status::CRC_ERROR,
            status::RATE_LIMITED,
        ] {
            let again = batch_answer(&r, answer(s, Vec::new()), || Ok(vec![1, 2, 3]));
            assert_eq!(again.unwrap(), vec![1, 2, 3], "0x{s:02X} goes again");
        }
        for s in [status::TIMEOUT, status::ERROR, status::NO_DEVICE] {
            let mut retried = false;
            let e = batch_answer(&r, answer(s, Vec::new()), || {
                retried = true;
                Ok(Vec::new())
            });
            assert!(
                e.is_err() && !retried,
                "0x{s:02X} is not repeated: it may have run"
            );
        }
        assert!(matches!(
            batch_answer(&r, answer(status::OK, vec![9; 2]), || unreachable!()),
            Err(TransportError::ShortRead { got: 2, .. })
        ));
        assert_eq!(
            batch_answer(
                &InRead::upto(0x7F, 0, 3),
                answer(status::OK, vec![9; 5]),
                || unreachable!()
            )
            .unwrap(),
            vec![9; 3]
        );
    }

    #[test]
    fn wp_t5_busy_statuses_are_retried_then_time_out() {
        for busy in [status::BUSY, status::BULK_LOCKED, status::CRC_ERROR] {
            let (r, calls) = scripted(&[busy, busy]);
            assert_eq!(r.unwrap(), vec![7; 3], "status 0x{busy:02X} recovers");
            assert_eq!(calls, 3);
        }
        let (r, calls) = scripted(&[status::BUSY; BUSY_RETRIES + 1]);
        assert!(matches!(
            r,
            Err(TransportError::TimedOut {
                opcode: 0xA0,
                retries: BUSY_RETRIES
            })
        ));
        assert_eq!(calls, BUSY_RETRIES + 1, "gives up rather than looping");
        let (r, calls) = scripted(&[status::BUSY, status::ERROR]);
        assert!(matches!(r, Err(TransportError::Stalled { opcode: 0xA0 })));
        assert_eq!(calls, 2);
        let (r, calls) = scripted(&[status::TIMEOUT]);
        assert!(
            matches!(r, Err(TransportError::TimedOut { retries: 0, .. })),
            "a hub TIMEOUT may have run: not retried"
        );
        assert_eq!(calls, 1);
    }

    #[test]
    fn wp_t5_max_payload_is_clamped() {
        assert_eq!(clamp_max_payload(0), 64);
        assert_eq!(clamp_max_payload(1), 64);
        assert_eq!(clamp_max_payload(8192), 8192);
        assert_eq!(clamp_max_payload(u32::MAX), 65_535);
    }

    #[test]
    fn wp_t5_a_second_notifications_call_ends_the_first() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let mut t = open(&hub);
        let mut first = t.notifications().unwrap();
        let mut second = t.notifications().unwrap();
        assert!(matches!(
            first.read(WAIT),
            Err(TransportError::Disconnected)
        ));
        t.control_out(req::SET_MASTER_VOLUME, 0, &(-9.0f32).to_le_bytes())
            .unwrap();
        assert_eq!(next_note(second.as_mut()).origin, t.session_id());
    }
}
