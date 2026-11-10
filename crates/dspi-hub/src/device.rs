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
use dspi_link::messages::{PollGrant, PollSpec, Snapshot};
use dspi_link::{Dir, status};
use dspi_proto::generated::opcodes as op;
use dspi_proto::packets::PlatformInfo;
use dspi_transport::{
    NotificationSource, Transport, TransportError, TransportKind, with_busy_retry,
};

use crate::policy::Policy;
use crate::polls::{PollRefused, Polls};
use crate::registry::{self, DeviceIdentity};
use crate::relay::{self, Relay};
use crate::router::{ExecTicket, ExecUnit, Router};
use crate::session::{Completion, NO_ORIGIN, Session, SessionCtx};
use crate::snapshot::SnapshotCache;

/// How long the reader blocks on the device before looking at its stop flag.
const READ_TIMEOUT: Duration = Duration::from_millis(100);
/// How long after a write a stalled parameter read is taken for a blackout. Past the
/// firmware's own: two flash sectors at about 45 ms, and the direct path's 600 ms of
/// retries for a deferred write.
const BUSY_WINDOW: Duration = Duration::from_secs(1);
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
    // -- WP HB5 --
    /// The responder has started: look at it soon.
    AdvertisingStarted,
    /// This attachment's device is gone.
    Lost { handle: u8, generation: u64 },
    /// Scan the bus now and answer.
    ScanNow(Sender<()>),
    // -- WP HB3 --
    /// The board stored a name (`NOTIFY_EVT_DEVICE_NAME`).
    BoardName { serial: String, name: String },
    /// The board accepted the stored name the hub wrote to it.
    NameAdopted { serial: String },
    /// End the supervisor.
    Stop,
}

/// One command in the worker's queue.
struct CmdJob {
    ctx: SessionCtx,
    req: CmdRequest,
    done: Completion,
    /// When the caller gives up waiting. A command still queued past it does not run:
    /// its caller was told `TIMEOUT`, and a write must not land after that. `None` for a
    /// network client, whose own timeout the hub cannot see.
    deadline: Option<Instant>,
}

/// What the worker works through.
enum Job {
    Cmd(CmdJob),
    /// Read the image into the snapshot cache. At most one queued per device.
    SnapshotRead,
    // -- WP HB3 --
    /// Look at the polls again: the worker may be asleep with none, or till a later one.
    Wake,
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
    /// A [`Job::SnapshotRead`] is queued and has not started.
    snapshot_read_queued: AtomicBool,
}

impl DeviceShared {
    /// Queue a command. Answered at once when it is not admitted or the worker has
    /// gone; otherwise answered by the worker, in queue order. Past `deadline` it is
    /// answered `TIMEOUT` instead of running.
    pub(crate) fn submit(
        &self,
        ctx: SessionCtx,
        req: CmdRequest,
        done: Completion,
        deadline: Option<Instant>,
    ) {
        if let Err(refused) = self.router.admit(&ctx, &req, Instant::now()) {
            done.answer(refused, Vec::new());
            return;
        }
        // A send fails only once the worker has ended: dropping the job answers
        // NO_DEVICE (`Completion`).
        let _ = self.tx.send(Job::Cmd(CmdJob {
            ctx,
            req,
            done,
            deadline,
        }));
    }

    /// Have the worker read the image into the snapshot cache. Coalesced: while one is
    /// queued, more asks add nothing, and an ask after it has started gets a new one,
    /// so the last image read is never older than the last ask. Never blocks.
    pub(crate) fn queue_snapshot_read(&self) {
        if !self.snapshot_read_queued.swap(true, Ordering::SeqCst) {
            let _ = self.tx.send(Job::SnapshotRead);
        }
    }

    // -- WP HB3 --
    /// `poll.subscribe` on this device, waking the worker to it. Coalesced: however
    /// fast a client subscribes, one wake is queued.
    pub(crate) fn poll_subscribe(
        &self,
        session: &Session,
        policy: &Policy,
        specs: &[PollSpec],
    ) -> Result<Vec<PollGrant>, PollRefused> {
        let granted = self.polls.subscribe(
            self.handle,
            session.id(),
            session.outbox(),
            policy,
            specs,
            Instant::now(),
        )?;
        if self.polls.take_wake() {
            let _ = self.tx.send(Job::Wake);
        }
        Ok(granted)
    }

    /// Read the image now, so the first `device.snapshot` finds it.
    pub(crate) fn warm_snapshot(&self) {
        if self.snapshot.ask() {
            self.queue_snapshot_read();
        }
    }

    /// The image for `device.snapshot`, read first if need be. Blocks for that, at most
    /// `timeout`: on a connection's reader, never a device thread.
    pub(crate) fn snapshot_wait(&self, timeout: Duration) -> Option<Snapshot> {
        self.warm_snapshot();
        self.snapshot.wait(self.handle, timeout)
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
            snapshot_read_queued: AtomicBool::new(false),
        });
        // -- WP HB3 --
        if source.is_none() {
            shared.snapshot.distrust();
        }
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
        // -- WP HB3 --
        self.shared.snapshot.close();
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
    let mut busy = BusyWindow::default();
    loop {
        let next = match shared.polls.next_due() {
            Some(due) => rx.recv_timeout(due.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match next {
            Ok(Job::Cmd(job)) => {
                // -- WP HB3 --
                if is_chunk(job.req.b_request) {
                    shared.polls.hold_for_chunks(Instant::now());
                }
                run_command(&mut *transport, shared, &sup, &mut busy, job);
            }
            Ok(Job::SnapshotRead) => read_snapshot(&mut *transport, shared),
            Ok(Job::Wake) | Err(RecvTimeoutError::Timeout) => {}
            Ok(Job::Stop) | Err(RecvTimeoutError::Disconnected) => return,
        }
        for job in shared.polls.take_due(Instant::now()) {
            let reply = transport.control_in_upto(job.req, job.val, job.len);
            if matches!(reply, Err(TransportError::Disconnected)) {
                let _ = sup.send(SupMsg::Lost {
                    handle: shared.handle,
                    generation: shared.generation,
                });
            }
            shared.polls.finished(&job, reply, Instant::now());
        }
    }
}

/// A client's own bulk chunk: part of a session any other vendor request would tear.
fn is_chunk(opcode: u8) -> bool {
    opcode == op::REQ_GET_ALL_PARAMS_CHUNK || opcode == op::REQ_SET_ALL_PARAMS_CHUNK
}

/// The queued re-read: in the device's order, needing no role or inflight slot. Lock
/// or not, a read changes nothing a lock protects, and a failed one leaves no image,
/// since the cache dropped it when it asked.
fn read_snapshot(t: &mut dyn Transport, shared: &DeviceShared) {
    shared.snapshot_read_queued.store(false, Ordering::SeqCst);
    if shared.stopping.load(Ordering::SeqCst) {
        // -- WP HB3 --
        shared.snapshot.read_failed();
        return;
    }
    // -- WP HB3 --
    shared.snapshot.read_started();
    let read = if t.caps().chunked_bulk {
        read_chunked(t, u16::MAX)
    } else {
        t.control_in_upto(op::REQ_GET_ALL_PARAMS, 0, u16::MAX)
    };
    match read {
        Ok(image) => shared.snapshot.set_bulk(image),
        Err(e) => {
            log::debug!("device {}: snapshot read: {e}", shared.handle);
            // -- WP HB3 --
            shared.snapshot.read_failed();
        }
    }
}

/// Whether the device may be in a blackout it caused itself: a command that can write
/// flash or restart the pipeline has just run on it. A stall outside this window is the
/// firmware's answer (HUB.md section 4), and repeating it would hold the queue.
#[derive(Debug, Default)]
struct BusyWindow {
    last_write: Option<Instant>,
}

impl BusyWindow {
    fn note_write(&mut self, now: Instant) {
        self.last_write = Some(now);
    }

    fn is_open(&self, now: Instant) -> bool {
        self.last_write
            .is_some_and(|at| now.saturating_duration_since(at) < BUSY_WINDOW)
    }
}

fn run_command(
    t: &mut dyn Transport,
    shared: &DeviceShared,
    sup: &Sender<SupMsg>,
    busy: &mut BusyWindow,
    job: CmdJob,
) {
    let CmdJob {
        ctx,
        req,
        done,
        deadline,
    } = job;
    let req = &req;
    let now = Instant::now();
    let early = if shared.stopping.load(Ordering::SeqCst) {
        Some(status::NO_DEVICE)
    } else if deadline.is_some_and(|d| now >= d) {
        Some(status::TIMEOUT)
    } else {
        None
    };
    if let Some(early) = early {
        shared.router.after_exec(&ctx, ExecTicket::none(), false);
        done.answer(early, Vec::new());
        return;
    }
    // -- WP HB2 --
    let unit = ExecUnit {
        others_poll: &|codes, me| shared.polls.others_poll(codes, me),
        read_vreg: &mut || dspi_session::ext::read_core_vreg(&mut *t).ok(),
    };
    let ticket = match shared.router.before_exec(&ctx, req, now, unit) {
        Ok(ticket) => ticket,
        Err((answer, payload)) => {
            shared.router.after_exec(&ctx, ExecTicket::none(), true);
            done.answer(answer, payload);
            return;
        }
    };
    let busy_retry = shared.router.retries_stall(req) && busy.is_open(now);
    let result = execute(t, req, busy_retry);
    if shared.router.mutates(req) {
        busy.note_write(Instant::now());
    }
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
/// nothing can land between the chunks. `busy_retry` repeats a read through the busy
/// window, as the direct path does for the same reads (HUB.md section 4). Everything
/// else gets one transfer and its answer, since a stall may simply be the firmware
/// saying it lacks the command.
///
/// `wIndex` is not passed on: the firmware ignores it and `Transport` has no such
/// parameter (real clients send 0 for much of the surface, `NetworkTransport.swift`).
pub(crate) fn execute(
    t: &mut dyn Transport,
    req: &CmdRequest,
    busy_retry: bool,
) -> Result<Vec<u8>, TransportError> {
    let opcode = req.b_request;
    let caps = t.caps();
    match req.dir {
        Dir::Get if opcode == op::REQ_GET_ALL_PARAMS && caps.chunked_bulk => {
            read_chunked(t, req.w_length)
        }
        Dir::Get if busy_retry => with_busy_retry(
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
    // -- WP UPW2 --
    let mut failures = dspi_transport::ReadFailures::default();
    while !stop.load(Ordering::Relaxed) {
        match source.read(READ_TIMEOUT) {
            Ok(packet) => {
                failures.worked();
                ingest(&packet, kind, shared, relay);
                // -- WP HB3 --
                // The supervisor takes it: the registry's lock and the name store are
                // no places for the reader. The packet has gone out first.
                if relay::is_relayable(&packet)
                    && let Some(name) = registry::board_name_event(&packet)
                {
                    let _ = sup.send(SupMsg::BoardName {
                        serial: shared.serial(),
                        name,
                    });
                }
            }
            Err(TransportError::Disconnected) => {
                let _ = sup.send(SupMsg::Lost {
                    handle: shared.handle,
                    generation: shared.generation,
                });
                return;
            }
            // A stall or a transient error: give the bus a moment. A run of
            // them is a dead handle, given up as an unplug is, so the next scan
            // reattaches a board still listed (WP UPW2).
            Err(e) => {
                log::debug!("device {}: notification read: {e}", shared.handle);
                if failures.failed() {
                    log::warn!(
                        "device {}: notifications keep failing, giving the handle up: {e}",
                        shared.handle
                    );
                    let _ = sup.send(SupMsg::Lost {
                        handle: shared.handle,
                        generation: shared.generation,
                    });
                    return;
                }
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
        Some((source, answers)) if relay::is_hub_path(kind, source) => {
            shared.router.consume_attribution(answers, Instant::now())
        }
        _ => NO_ORIGIN,
    };
    if shared.snapshot.note_packet(packet) {
        shared.queue_snapshot_read();
    }
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
        // -- WP HB3 --
        board_name: read_board_name(t)?,
    })
}

// -- WP HB3 --
/// `REQ_GET_DEVICE_NAME`: firmware that stores a name answers 32 bytes. A stall or
/// anything shorter is firmware that cannot (it may be a legacy gain or mute on that
/// opcode), so a `0x56` must never reach it. A second try for a read that failed some
/// other way, as a board that can store a name must not pass for one that cannot.
fn read_board_name(t: &mut dyn Transport) -> Result<Option<String>, TransportError> {
    const LEN: u16 = 32;
    for _ in 0..2 {
        match t.control_in_upto(op::REQ_GET_DEVICE_NAME, 0, LEN) {
            Ok(reply) if reply.len() == usize::from(LEN) => {
                return Ok(Some(registry::decode_board_name(&reply)));
            }
            Ok(_) | Err(TransportError::Stalled { .. }) => return Ok(None),
            Err(e @ TransportError::Disconnected) => return Err(e),
            Err(e) => log::debug!("device name read: {e}"),
        }
    }
    Ok(None)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_hb0 {
    use dspi_proto::VENDOR_INTERFACE;
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
    fn wp_hb0_busy_retry_repeats_a_read_and_nothing_else() {
        let mut m = MockTransport::new().reply(op::REQ_GET_PLATFORM, Reply::Stall);
        let e = execute(&mut m, &get(op::REQ_GET_PLATFORM, 7), true).unwrap_err();
        assert!(matches!(e, TransportError::Stalled { .. }));
        assert_eq!(m.log().len(), dspi_transport::BUSY_RETRIES + 1);

        let mut m = MockTransport::new().reply(op::REQ_GET_CLOCK_DIAG, Reply::Stall);
        let e = execute(&mut m, &get(op::REQ_GET_CLOCK_DIAG, 7), false).unwrap_err();
        assert!(matches!(e, TransportError::Stalled { .. }));
        assert_eq!(m.log().len(), 1, "a stall is the answer");

        let mut m = MockTransport::new().reply(0x77, Reply::Stall);
        let e = execute(&mut m, &set(0x77, vec![1]), false).unwrap_err();
        assert!(matches!(e, TransportError::Stalled { .. }));
        assert_eq!(m.log().len(), 1);
    }

    /// `NetworkTransport.swift:341-363` sends `WINDEX_GLOBAL = 0` for EQ, volume and
    /// more, and the firmware ignores it.
    #[test]
    fn wp_hb0_windex_is_ignored() {
        let mut d = FakeDevice::demo();
        let remote = d.remote();
        let volume = (-17.5f32).to_le_bytes().to_vec();
        for w_index in [0, 2, 0xFFFF] {
            let write = CmdRequest {
                w_index,
                ..set(op::REQ_SET_MASTER_VOLUME, volume.clone())
            };
            execute(&mut d, &write, false).unwrap();
            let read = CmdRequest {
                w_index,
                ..get(op::REQ_GET_MASTER_VOLUME, 4)
            };
            assert_eq!(execute(&mut d, &read, true).unwrap(), volume);
        }
        assert!(remote.image().windows(4).any(|w| w == volume));
    }

    // -- WP UPW1 --
    /// The macOS Console names its audio interface (0) in wIndex, also for the whole
    /// image (upstream's Windows hub, 3972302e). The chunked read and write still go
    /// to the vendor interface: no transport call carries wIndex at all.
    #[test]
    fn wp_upw1_the_macs_windex_does_not_reach_the_chunked_bulk_transfers() {
        let mut d = small_transfers();
        let remote = d.remote();
        let mut image = remote.image();
        let read = CmdRequest {
            w_index: 0,
            ..get(op::REQ_GET_ALL_PARAMS, u16::MAX)
        };
        assert_eq!(execute(&mut d, &read, true).unwrap(), image);
        let at = 4700;
        image[at..at + 4].copy_from_slice(&(-9.0f32).to_le_bytes());
        let write = CmdRequest {
            w_index: 0,
            ..set(op::REQ_SET_ALL_PARAMS, image.clone())
        };
        execute(&mut d, &write, false).unwrap();
        assert_eq!(remote.image()[at..at + 4], image[at..at + 4]);
    }

    #[test]
    fn wp_hb0_the_busy_window_follows_a_write() {
        let t = Instant::now();
        let mut b = BusyWindow::default();
        assert!(!b.is_open(t), "no write yet");
        b.note_write(t);
        assert!(b.is_open(t));
        assert!(b.is_open(t + BUSY_WINDOW / 2));
        assert!(!b.is_open(t + BUSY_WINDOW));
    }

    /// The re-read HB3 asks for: however many asks, one queued, and one more only for
    /// an ask made after the read has started.
    #[test]
    fn wp_hb0_snapshot_reads_are_coalesced() {
        use crate::policy::Policy;

        let (tx, rx) = mpsc::channel();
        let shared = DeviceShared {
            handle: 0,
            generation: 1,
            identity: RwLock::new(read_identity(&mut FakeDevice::demo(), 0, 0).unwrap()),
            router: Router::new(Arc::new(Policy::embedded()), 8),
            snapshot: SnapshotCache::default(),
            polls: Polls::default(),
            tx,
            stopping: AtomicBool::new(false),
            snapshot_read_queued: AtomicBool::new(false),
        };
        for _ in 0..1000 {
            shared.queue_snapshot_read();
        }
        assert!(matches!(rx.try_recv(), Ok(Job::SnapshotRead)));
        assert!(rx.try_recv().is_err(), "one queued, however many asks");
        assert!(shared.snapshot.bulk().is_none());

        let mut d = FakeDevice::demo();
        read_snapshot(&mut d, &shared);
        assert_eq!(shared.snapshot.bulk(), Some(d.remote().image()));

        shared.queue_snapshot_read();
        assert!(matches!(rx.try_recv(), Ok(Job::SnapshotRead)), "a new ask");
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
            snapshot_read_queued: AtomicBool::new(false),
        };
        let ctx = SessionCtx {
            id: 9,
            role: Role::Admin,
            exempt_inflight: false,
        };
        let write = set(op::REQ_SET_MASTER_VOLUME, vec![0; 4]);
        shared.router.admit(&ctx, &write, Instant::now()).unwrap();
        let unit = ExecUnit {
            others_poll: &|_, _| false,
            read_vreg: &mut || None,
        };
        let t = shared
            .router
            .before_exec(&ctx, &write, Instant::now(), unit)
            .unwrap();
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

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_hb3 {
    use dspi_transport::mock::{MockTransport, Reply};

    use super::*;

    fn mock(reply: Reply) -> MockTransport {
        MockTransport::new().reply(op::REQ_GET_DEVICE_NAME, reply)
    }

    #[test]
    fn wp_hb3_32_bytes_is_a_board_that_stores_names() {
        let mut field = b"Den".to_vec();
        field.resize(32, 0);
        let mut m = mock(Reply::Data(field));
        assert_eq!(read_board_name(&mut m).unwrap().as_deref(), Some("Den"));
        let mut m = mock(Reply::Data(vec![0; 32]));
        assert_eq!(
            read_board_name(&mut m).unwrap().as_deref(),
            Some(""),
            "unnamed"
        );
    }

    /// A stall or a short reply is the firmware's answer, so one read.
    #[test]
    fn wp_hb3_anything_else_is_firmware_that_cannot() {
        for reply in [
            Reply::Stall,
            Reply::Data(vec![1; 4]),
            Reply::Data(Vec::new()),
        ] {
            let mut m = mock(reply);
            assert!(matches!(read_board_name(&mut m), Ok(None)));
            assert_eq!(m.log().len(), 1);
        }
    }

    #[test]
    fn wp_hb3_a_failed_name_read_is_tried_once_more() {
        let timeout = TransportError::TimedOut {
            opcode: op::REQ_GET_DEVICE_NAME,
            retries: 0,
        };
        let mut m = mock(Reply::Sequence(vec![
            Reply::Error(timeout.clone()),
            Reply::Data(vec![0; 32]),
        ]));
        assert_eq!(read_board_name(&mut m).unwrap().as_deref(), Some(""));

        let mut m = mock(Reply::Error(timeout));
        assert!(matches!(read_board_name(&mut m), Ok(None)));
        assert_eq!(m.log().len(), 2);

        let mut m = mock(Reply::Disconnect);
        assert!(matches!(
            read_board_name(&mut m),
            Err(TransportError::Disconnected)
        ));
    }
}
