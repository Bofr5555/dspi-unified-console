//! One attached device: the worker that owns its transport, and the reader that
//! owns its notification stream (docs/core-merge/HUB.md sections 2 and 3).
//!
//! The worker is the only code that touches the transport, so commands run one at
//! a time in arrival order across every session (spec 8.2), a chunked bulk session
//! cannot be torn by another request, and no lock is ever held across a transfer.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, PoisonError, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use dspi_link::frames::CmdRequest;
use dspi_link::{Dir, status};
use dspi_proto::VENDOR_INTERFACE;
use dspi_proto::generated::opcodes as op;
use dspi_proto::packets::PlatformInfo;
use dspi_transport::{
    NotificationSource, Transport, TransportError, TransportKind, with_busy_retry,
};

use crate::polls::Polls;
use crate::registry::DeviceIdentity;
use crate::relay::{self, Relay};
use crate::router::{ExecTicket, Router};
use crate::session::{Completion, NO_ORIGIN, SessionCtx};
use crate::snapshot::SnapshotCache;

/// How long the reader blocks on the device before looking at its stop flag.
const READ_TIMEOUT: Duration = Duration::from_millis(100);
/// Longest control transfer of a bulk chunk (`bulk_params_chunking.md`).
const BULK_CHUNK: usize = 2048;
/// `wLength` of the platform read.
const PLATFORM_LEN: u16 = {
    assert!(PlatformInfo::REQUEST_LEN <= u16::MAX as usize);
    PlatformInfo::REQUEST_LEN as u16
};

/// What the workers and readers tell the hub's supervisor.
#[derive(Debug)]
pub(crate) enum SupMsg {
    /// This attachment's device is gone.
    Lost { handle: u8, generation: u64 },
    /// Scan the bus now and answer.
    ScanNow(Sender<()>),
    /// End the supervisor.
    Stop,
}

/// A command for the worker.
enum Job {
    Cmd {
        ctx: SessionCtx,
        req: CmdRequest,
        done: Completion,
    },
    Stop,
}

/// What everyone may hold of an attachment. Not the transport.
pub(crate) struct DeviceShared {
    pub(crate) handle: u8,
    pub(crate) generation: u64,
    pub(crate) identity: RwLock<DeviceIdentity>,
    pub(crate) router: Router,
    pub(crate) snapshot: SnapshotCache,
    pub(crate) polls: Polls,
    tx: Sender<Job>,
    stopping: AtomicBool,
}

impl DeviceShared {
    /// Queue a command. Answered at once when it is not admitted or the worker has
    /// gone; otherwise answered by the worker, in queue order.
    pub(crate) fn submit(&self, ctx: SessionCtx, req: CmdRequest, done: Completion) {
        if let Err(refused) = self.router.admit(&ctx, &req, Instant::now()) {
            done.answer(refused, Vec::new());
            return;
        }
        // A send fails only once the worker has ended: dropping the job answers
        // NO_DEVICE (`Completion`).
        let _ = self.tx.send(Job::Cmd { ctx, req, done });
    }

    /// The device's serial.
    pub(crate) fn serial(&self) -> String {
        self.identity
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .serial
            .clone()
    }
}

/// One attachment: the threads, held by the hub until it is torn down.
pub(crate) struct Attachment {
    pub(crate) shared: Arc<DeviceShared>,
    pub(crate) from_bus: bool,
    worker: JoinHandle<()>,
    reader: Option<JoinHandle<()>>,
    reader_stop: Arc<AtomicBool>,
}

impl Attachment {
    /// Start the worker and the reader. `source` was taken from `transport` before
    /// the call. The reader starts before anything can write, so no echo is missed.
    pub(crate) fn spawn(
        transport: Box<dyn Transport>,
        source: Option<Box<dyn NotificationSource>>,
        shared_parts: (u8, u64, DeviceIdentity, Router),
        from_bus: bool,
        relay: Arc<Relay>,
        sup: Sender<SupMsg>,
    ) -> std::io::Result<Self> {
        let (handle, generation, identity, router) = shared_parts;
        let kind = transport.caps().kind;
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(DeviceShared {
            handle,
            generation,
            identity: RwLock::new(identity),
            router,
            snapshot: SnapshotCache::default(),
            polls: Polls::default(),
            tx,
            stopping: AtomicBool::new(false),
        });
        let reader_stop = Arc::new(AtomicBool::new(false));
        let reader = match source {
            Some(source) => {
                let (shared, stop, sup) =
                    (Arc::clone(&shared), Arc::clone(&reader_stop), sup.clone());
                Some(
                    thread::Builder::new()
                        .name(format!("hub-rx-{handle}"))
                        .spawn(move || reader_loop(source, kind, &shared, &relay, &sup, &stop))?,
                )
            }
            None => None,
        };
        let worker = {
            let shared = Arc::clone(&shared);
            thread::Builder::new()
                .name(format!("hub-dev-{handle}"))
                .spawn(move || worker_loop(transport, &shared, &rx, sup))
        };
        let worker = match worker {
            Ok(w) => w,
            Err(e) => {
                reader_stop.store(true, Ordering::Relaxed);
                if let Some(r) = reader {
                    let _ = r.join();
                }
                return Err(e);
            }
        };
        Ok(Self {
            shared,
            from_bus,
            worker,
            reader,
            reader_stop,
        })
    }

    /// Stop and join, in the order that never reads as an unplug: reader first,
    /// then the worker, whose exit drops the transport and frees the USB interface.
    /// Queued commands answer `NO_DEVICE`; one inside the device finishes, bounded
    /// by the transport's own timeout. Never call this on the device's own threads
    /// or with a hub lock held.
    pub(crate) fn tear_down(self) {
        self.shared.router.shutdown();
        self.shared.stopping.store(true, Ordering::SeqCst);
        self.reader_stop.store(true, Ordering::Relaxed);
        if let Some(reader) = self.reader {
            let _ = reader.join();
        }
        let _ = self.shared.tx.send(Job::Stop);
        let _ = self.worker.join();
    }
}

/// Tells the supervisor when the worker ends, however it ends (a panicking
/// transport included). The supervisor ignores it for a torn-down attachment.
struct LostOnExit {
    sup: Sender<SupMsg>,
    handle: u8,
    generation: u64,
}

impl Drop for LostOnExit {
    fn drop(&mut self) {
        let _ = self.sup.send(SupMsg::Lost {
            handle: self.handle,
            generation: self.generation,
        });
    }
}

fn worker_loop(
    mut transport: Box<dyn Transport>,
    shared: &DeviceShared,
    rx: &Receiver<Job>,
    sup: Sender<SupMsg>,
) {
    let _lost = LostOnExit {
        sup: sup.clone(),
        handle: shared.handle,
        generation: shared.generation,
    };
    loop {
        let next = match shared.polls.next_due() {
            Some(due) => rx.recv_timeout(due.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match next {
            Ok(Job::Cmd { ctx, req, done }) => {
                run_command(&mut *transport, shared, &sup, ctx, &req, done);
            }
            Ok(Job::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
        for job in shared.polls.take_due(Instant::now()) {
            let reply = transport.control_in_upto(job.req, job.val, job.len).ok();
            shared.polls.finished(&job, reply);
        }
    }
}

fn run_command(
    t: &mut dyn Transport,
    shared: &DeviceShared,
    sup: &Sender<SupMsg>,
    ctx: SessionCtx,
    req: &CmdRequest,
    done: Completion,
) {
    if shared.stopping.load(Ordering::SeqCst) {
        shared.router.after_exec(&ctx, ExecTicket::none(), false);
        done.answer(status::NO_DEVICE, Vec::new());
        return;
    }
    let ticket = shared.router.before_exec(&ctx, req, Instant::now());
    let result = execute(t, req, shared.router.is_idempotent(req));
    shared
        .router
        .after_exec(&ctx, ticket, result.as_ref().is_ok());
    match result {
        Ok(payload) => done.answer(status::OK, payload),
        Err(e) => {
            if matches!(e, TransportError::Disconnected) {
                let _ = sup.send(SupMsg::Lost {
                    handle: shared.handle,
                    generation: shared.generation,
                });
            }
            done.answer(status_of(&e), Vec::new());
        }
    }
}

/// A transport's failure as a Link status (spec 8.1, 9.1).
pub(crate) fn status_of(e: &TransportError) -> u8 {
    match e {
        TransportError::TimedOut { .. } => status::TIMEOUT,
        TransportError::Disconnected
        | TransportError::NotFound
        | TransportError::SerialNotFound(_)
        | TransportError::PermissionDenied => status::NO_DEVICE,
        TransportError::Denied(_) => status::DENIED,
        TransportError::Locked => status::LOCKED,
        TransportError::Stalled { .. }
        | TransportError::ShortRead { .. }
        | TransportError::Usb(_)
        | TransportError::Incompatible(_)
        | TransportError::Malformed(_)
        | TransportError::Remote(_) => status::ERROR,
    }
}

/// Run one tunnelled request on the transport.
///
/// `0xA0` and `0xA1` become chunked sessions where the transport has them (WinUSB
/// cannot carry the image in one transfer, spec 9.1.1), inside this one call, so
/// nothing can land between the chunks. The hub repeats only `idempotent` reads
/// through the busy window, as the direct path does (PLAN section 1.2): a stall in
/// a flash blackout is not yet the device's answer. That leaves nothing for the
/// client to retry, since over Link a stall is final.
pub(crate) fn execute(
    t: &mut dyn Transport,
    req: &CmdRequest,
    idempotent: bool,
) -> Result<Vec<u8>, TransportError> {
    // The transport has no wIndex: it is always the vendor interface. Forwarding
    // anything else cannot be done, so it is refused rather than misdelivered.
    if req.w_index != VENDOR_INTERFACE {
        return Err(TransportError::Stalled {
            opcode: req.b_request,
        });
    }
    let opcode = req.b_request;
    let caps = t.caps();
    match req.dir {
        Dir::Get if opcode == op::REQ_GET_ALL_PARAMS && caps.chunked_bulk => {
            read_chunked(t, req.w_length)
        }
        Dir::Get if idempotent => with_busy_retry(
            caps.kind,
            || t.control_in_upto(opcode, req.w_value, req.w_length),
            opcode,
        ),
        Dir::Get => t.control_in_upto(opcode, req.w_value, req.w_length),
        Dir::Set if opcode == op::REQ_SET_ALL_PARAMS && caps.chunked_bulk => {
            write_chunked(t, &req.payload).map(|()| Vec::new())
        }
        Dir::Set => t
            .control_out(opcode, req.w_value, &req.payload)
            .map(|()| Vec::new()),
    }
}

/// A single-shot read served in chunks: at most `want` bytes, as the firmware
/// answers `min(wLength, image)`. A short chunk is the end of the image.
fn read_chunked(t: &mut dyn Transport, want: u16) -> Result<Vec<u8>, TransportError> {
    let opcode = op::REQ_GET_ALL_PARAMS_CHUNK;
    let kind = t.caps().kind;
    let chunk = BULK_CHUNK.min(t.max_transfer()).max(1);
    let want = usize::from(want);
    let mut image = Vec::with_capacity(want.min(BULK_CHUNK));
    while image.len() < want {
        let ask = chunk.min(want.saturating_sub(image.len()));
        let (Ok(offset), Ok(len)) = (u16::try_from(image.len()), u16::try_from(ask)) else {
            break;
        };
        match with_busy_retry(kind, || t.control_in_upto(opcode, offset, len), opcode) {
            Ok(part) => {
                let got = part.len();
                image.extend_from_slice(&part);
                if got < ask {
                    break;
                }
            }
            // An image of a whole number of chunks: past its end the firmware stalls.
            Err(TransportError::Stalled { .. }) if !image.is_empty() => break,
            Err(e) => return Err(e),
        }
    }
    Ok(image)
}

/// A single-shot write served in chunks. Not retried: a failed chunk closes the
/// device's chunk session, and a repeat could apply twice.
fn write_chunked(t: &mut dyn Transport, image: &[u8]) -> Result<(), TransportError> {
    let size = BULK_CHUNK.min(t.max_transfer()).max(1);
    let mut offset = 0usize;
    for part in image.chunks(size) {
        let value = u16::try_from(offset).map_err(|_| {
            TransportError::Remote(format!(
                "a bulk image of {} bytes does not fit wValue",
                image.len()
            ))
        })?;
        t.control_out(op::REQ_SET_ALL_PARAMS_CHUNK, value, part)?;
        offset = offset.saturating_add(part.len());
    }
    Ok(())
}

fn reader_loop(
    mut source: Box<dyn NotificationSource>,
    kind: TransportKind,
    shared: &DeviceShared,
    relay: &Relay,
    sup: &Sender<SupMsg>,
    stop: &AtomicBool,
) {
    while !stop.load(Ordering::Relaxed) {
        match source.read(READ_TIMEOUT) {
            Ok(packet) => ingest(&packet, kind, shared, relay),
            Err(TransportError::Disconnected) => {
                let _ = sup.send(SupMsg::Lost {
                    handle: shared.handle,
                    generation: shared.generation,
                });
                return;
            }
            // A stall or a transient error: give the bus a moment.
            Err(e) => {
                log::debug!("device {}: notification read: {e}", shared.handle);
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// One packet from the device: attribute it, show it to the snapshot, fan it out.
fn ingest(packet: &[u8], kind: TransportKind, shared: &DeviceShared, relay: &Relay) {
    if !relay::is_relayable(packet) {
        return;
    }
    let origin = match relay::packet_source(packet) {
        Some((source, bulk)) if relay::is_hub_path(kind, source) => {
            shared.router.consume_attribution(bulk, Instant::now())
        }
        _ => NO_ORIGIN,
    };
    shared.snapshot.note_packet(packet);
    relay.publish_notify(shared.handle, origin, packet);
}

/// Read a device's identity for the registry. `serial` is its USB serial.
pub(crate) fn read_identity(
    t: &mut dyn Transport,
    vid: u16,
    location_id: u32,
) -> Result<DeviceIdentity, TransportError> {
    let reply = t.control_in_upto(op::REQ_GET_PLATFORM, 0, PLATFORM_LEN)?;
    let info = PlatformInfo::decode(&reply)
        .map_err(|e| TransportError::Malformed(format!("GET_PLATFORM: {e}")))?;
    Ok(DeviceIdentity {
        serial: t.descriptor().serial.clone(),
        vid,
        location_id,
        platform: info.platform_id,
        firmware: info.firmware(),
        firmware_beta: info.version.beta(),
        outputs: info.num_output_channels,
        board_name: None,
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_hb0 {
    use dspi_proto::generated::notify as nfy;
    use dspi_transport::mock::{MockTransport, Reply};
    use dspi_transport::{FakeDevice, FakeOptions};

    use super::*;

    fn get(opcode: u8, len: u16) -> CmdRequest {
        CmdRequest {
            tag: 1,
            handle: 0,
            dir: Dir::Get,
            b_request: opcode,
            w_value: 0,
            w_index: VENDOR_INTERFACE,
            w_length: len,
            payload: Vec::new(),
        }
    }

    fn set(opcode: u8, payload: Vec<u8>) -> CmdRequest {
        CmdRequest {
            dir: Dir::Set,
            b_request: opcode,
            w_length: payload.len() as u16,
            payload,
            ..get(opcode, 0)
        }
    }

    /// A device that, like a WinUSB one, cannot carry the image in one transfer.
    fn small_transfers() -> FakeDevice {
        FakeDevice::new(
            dspi_transport::Capture::demo(),
            FakeOptions {
                max_transfer: Some(4096),
                ..FakeOptions::default()
            },
        )
    }

    #[test]
    fn wp_hb0_a_single_shot_read_is_served_in_chunks() {
        let mut d = small_transfers();
        let image = d.remote().image();
        assert!(image.len() > 4096);
        let got = execute(&mut d, &get(op::REQ_GET_ALL_PARAMS, u16::MAX), true).unwrap();
        assert_eq!(got, image);
    }

    #[test]
    fn wp_hb0_a_short_bulk_read_stops_at_what_was_asked() {
        let mut d = small_transfers();
        let image = d.remote().image();
        let got = execute(&mut d, &get(op::REQ_GET_ALL_PARAMS, 8), true).unwrap();
        assert_eq!(got, image[..8]);
    }

    #[test]
    fn wp_hb0_a_single_shot_write_is_served_in_chunks() {
        let mut d = small_transfers();
        let remote = d.remote();
        let mut image = remote.image();
        // The demo's master volume, in the bytes the fake keeps.
        let at = 4700;
        image[at..at + 4].copy_from_slice(&(-12.0f32).to_le_bytes());
        execute(&mut d, &set(op::REQ_SET_ALL_PARAMS, image.clone()), false).unwrap();
        assert_eq!(remote.image()[at..at + 4], image[at..at + 4]);
    }

    #[test]
    fn wp_hb0_a_read_is_retried_through_the_busy_window_and_a_write_is_not() {
        let mut m = MockTransport::new().reply(op::REQ_GET_PLATFORM, Reply::Stall);
        let e = execute(&mut m, &get(op::REQ_GET_PLATFORM, 7), true).unwrap_err();
        assert!(matches!(e, TransportError::Stalled { .. }));
        assert_eq!(m.log().len(), dspi_transport::BUSY_RETRIES + 1);

        let mut m = MockTransport::new().reply(0x77, Reply::Stall);
        let e = execute(&mut m, &set(0x77, vec![1]), false).unwrap_err();
        assert!(matches!(e, TransportError::Stalled { .. }));
        assert_eq!(m.log().len(), 1);
    }

    #[test]
    fn wp_hb0_a_foreign_windex_is_refused() {
        let mut d = FakeDevice::demo();
        let req = CmdRequest {
            w_index: 0,
            ..get(op::REQ_GET_PLATFORM, 7)
        };
        let e = execute(&mut d, &req, true).unwrap_err();
        assert_eq!(status_of(&e), status::ERROR);
    }

    #[test]
    fn wp_hb0_statuses_follow_the_spec() {
        for (e, s) in [
            (TransportError::Stalled { opcode: 1 }, status::ERROR),
            (
                TransportError::TimedOut {
                    opcode: 1,
                    retries: 0,
                },
                status::TIMEOUT,
            ),
            (TransportError::Disconnected, status::NO_DEVICE),
        ] {
            assert_eq!(status_of(&e), s);
        }
    }

    #[test]
    fn wp_hb0_identity_comes_from_the_platform_read() {
        let mut d = FakeDevice::demo();
        let id = read_identity(&mut d, 0x2E8B, 7).unwrap();
        assert_eq!(id.platform, 1, "the demo is an RP2350");
        assert_eq!((id.vid, id.location_id), (0x2E8B, 7));
        assert_eq!(id.serial, d.descriptor().serial);
    }

    #[test]
    fn wp_hb0_a_short_platform_reply_is_an_error_not_a_panic() {
        let mut m = MockTransport::new().reply(op::REQ_GET_PLATFORM, Reply::Data(vec![1]));
        assert!(read_identity(&mut m, 0, 0).is_err());
    }

    /// Attribution follows the transport's path: a USB hub takes `HOST_SET` as a
    /// session's write, never `GPIO`.
    #[test]
    fn wp_hb0_ingest_attributes_only_the_hub_path() {
        use crate::policy::Policy;
        use crate::session::{Outbound, Outbox, Pop, Role};

        let (tx, _rx) = mpsc::channel();
        let shared = DeviceShared {
            handle: 4,
            generation: 1,
            identity: RwLock::new(read_identity(&mut FakeDevice::demo(), 0, 0).unwrap()),
            router: Router::new(Arc::new(Policy::embedded()), 8),
            snapshot: SnapshotCache::default(),
            polls: Polls::default(),
            tx,
            stopping: AtomicBool::new(false),
        };
        let ctx = SessionCtx {
            id: 9,
            role: Role::Admin,
            exempt_inflight: false,
        };
        let write = set(op::REQ_SET_MASTER_VOLUME, vec![0; 4]);
        shared.router.admit(&ctx, &write, Instant::now()).unwrap();
        let t = shared.router.before_exec(&ctx, &write, Instant::now());
        shared.router.after_exec(&ctx, t, true);

        let relay = Relay::default();
        let out = Arc::new(Outbox::new());
        relay.add(2, crate::relay::Scope::AllDevices, Arc::clone(&out));
        let param = |source: u32| {
            let mut p = vec![
                2,
                nfy::NOTIFY_EVT_PARAM_CHANGED as u8,
                0,
                0,
                0,
                0,
                1,
                0,
                source as u8,
                0,
                0,
                0,
                0,
            ];
            p.push(0);
            p
        };
        ingest(
            &param(nfy::PARAM_SRC_GPIO as u32),
            TransportKind::Usb,
            &shared,
            &relay,
        );
        ingest(
            &param(nfy::PARAM_SRC_HOST_SET as u32),
            TransportKind::Usb,
            &shared,
            &relay,
        );
        let origin = |p: Pop| match p {
            Pop::Item(Outbound::Notify { origin, handle, .. }) => {
                assert_eq!(handle, 4);
                origin
            }
            other => panic!("{other:?}"),
        };
        assert_eq!(origin(out.pop(Duration::ZERO)), NO_ORIGIN, "a knob");
        assert_eq!(origin(out.pop(Duration::ZERO)), 9, "the session's write");
    }
}
