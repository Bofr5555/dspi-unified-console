//! The USB MIDI control channel: vendor requests carried in SysEx
//! (Bofr5555/DSPi `usb_midi_control_spec.md`; `DSPI_USB_MIDI` firmware only).
//!
//! Three layers:
//! - [`MidiClient`]: sans-IO. Turns a request into SysEx bytes and incoming
//!   bytes into replies and notifications. No port, no thread, no clock:
//!   usable from wasm and from a foreign event loop.
//! - [`MidiPipe`]: what the app (Kotlin `MidiManager`, Swift CoreMIDI) or
//!   [`midir_port`] supplies: a way to send bytes. Incoming bytes are pushed
//!   with [`MidiInput::receive`].
//! - [`MidiTransport`]: the blocking [`Transport`] over a pipe: one request
//!   in flight, matched by tag, with the spec's retry rules.
//!
//! Owned by WP T2 (codec client and transport) and WP T3 (`midir_port`).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use dspi_proto::VENDOR_INTERFACE;
use dspi_proto::frame::{
    Frame, FrameError, Message, SysexDecoder, TAG_UNREADABLE, encode_sysex, status,
};
use dspi_proto::generated::opcodes as op;

use crate::{
    DeviceDescriptor, NotificationSource, Result, Transport, TransportCaps, TransportError,
};

/// Why a [`MidiPipe`] could not send.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PipeError {
    /// The port is closed or the device went away.
    #[error("the MIDI port is closed")]
    Closed,
    /// The platform refused the send; the text is the platform's.
    #[error("MIDI send failed: {0}")]
    Failed(String),
}

/// How the host sends bytes to the device's MIDI IN port. Called from the
/// thread that issues requests; must not block for long. Shared (`Arc`) so a
/// binding can hand in a foreign object (a Kotlin or Swift callback).
pub trait MidiPipe: Send + Sync {
    /// Send one complete SysEx message (`F0 .. F7`).
    fn send(&self, sysex: &[u8]) -> std::result::Result<(), PipeError>;
}

/// What a received message means to the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MidiEvent {
    /// The reply to the request with this tag (`0x7F`: to the request in
    /// flight, whose header the device could not read).
    Reply { tag: u8, frame: Frame },
    /// A notify packet (frame type `0x40`, tag 0), as the USB endpoint would
    /// deliver it.
    Notification(Vec<u8>),
    /// A message that matched our header but could not be read; fails the
    /// request in flight when the tag is `0x7F` or the request's own.
    Error { tag: u8, error: FrameError },
}

/// The tag after `tag`: host tags run 1..=0x7E (0 is the device's
/// notification tag, 0x7F its "unreadable header" tag, spec 3.1).
fn next_tag(tag: u8) -> u8 {
    if (1..0x7E).contains(&tag) {
        tag.wrapping_add(1)
    } else {
        1
    }
}

/// The sans-IO client: tags, encoding, decoding.
///
/// It keeps no notion of which request is in flight: that, the clock and the
/// retries belong to the driver ([`MidiTransport`], or a foreign event loop).
pub struct MidiClient {
    mfr: Vec<u8>,
    decoder: SysexDecoder,
    /// The tag of the last request; 0 before the first.
    tag: u8,
}

impl MidiClient {
    /// `mfr`: the manufacturer ID the firmware was built with
    /// ([`dspi_proto::frame::DEFAULT_MFR_ID`] unless a product changed it).
    pub fn new(mfr: &[u8]) -> Self {
        Self {
            mfr: mfr.to_vec(),
            decoder: SysexDecoder::new(mfr),
            tag: 0,
        }
    }

    /// Encode a request with the next tag (1..=0x7E). Returns the tag and the
    /// SysEx bytes to send.
    pub fn request(&mut self, frame: &Frame) -> (u8, Vec<u8>) {
        self.tag = next_tag(self.tag);
        (self.tag, encode_sysex(&self.mfr, self.tag, frame))
    }

    /// Feed received bytes (any split); returns the events they completed.
    /// Foreign SysEx, channel messages and real-time bytes are ignored
    /// ([`SysexDecoder`]).
    pub fn receive(&mut self, bytes: &[u8]) -> Vec<MidiEvent> {
        self.decoder
            .feed(bytes)
            .into_iter()
            .map(Self::classify)
            .collect()
    }

    fn classify(m: Message) -> MidiEvent {
        match m.frame {
            Ok(Frame::Notification { packet }) => MidiEvent::Notification(packet),
            Ok(frame) => MidiEvent::Reply { tag: m.tag, frame },
            Err(error) => MidiEvent::Error { tag: m.tag, error },
        }
    }
}

/// Notifications held for the reader before the oldest is dropped.
///
/// Policy: drop the **oldest**, keep the newest. A reader this far behind has
/// lost news either way; what matters is that the loss is seen. Every v2
/// packet carries a `seq` byte, so dropping packets here leaves a gap that
/// the session's reader (`dspi_session::notify::Notifications`) turns into
/// `lost` on the next packet it delivers, which forces a bulk re-read; the
/// packets kept are the most recent state. Same size as the session's queue
/// (`Notifications::QUEUE`).
const NOTIFY_QUEUE: usize = 1024;

/// Retries after `BUSY`, `BULK_LOCKED` or `CRC_ERROR`: each means the device
/// did not act on the request, so resending is safe for any opcode.
///
/// Combined back-off: a request still `BUSY` after these comes back as
/// [`TransportError::TimedOut`], which the session's
/// [`crate::with_busy_retry`] (idempotent reads only) retries in turn, up to
/// `BUSY_RETRIES` more times 150 ms apart. Worst case for such a read:
/// (1 + 4) x (1 + 4) = 25 transmissions over about 0.6 s + 5 x 0.2 s of
/// back-off, plus the device's own 50 ms per `BUSY` (spec 4.2). Writes and a
/// final `BULK_LOCKED` ([`TransportError::Locked`]) are not retried again.
const STATUS_RETRIES: u32 = crate::BUSY_RETRIES as u32;

/// Back-off before retry *n* is `n` times this. The firmware already retries
/// `BUSY` internally for 50 ms (spec 4.2), so a short pause is enough.
const STATUS_BACKOFF: Duration = Duration::from_millis(20);

/// State shared by the transport, its [`MidiInput`] and its notification
/// reader.
struct Shared {
    state: Mutex<State>,
    /// Signalled when the reply to the request in flight arrives.
    replied: Condvar,
    /// Signalled when a notification is queued or the transport closes.
    notified: Condvar,
}

struct State {
    client: MidiClient,
    /// The tag of the request in flight.
    waiting: Option<u8>,
    /// Its answer, once it came.
    reply: Option<std::result::Result<Frame, FrameError>>,
    notes: VecDeque<Vec<u8>>,
    /// The transport was dropped, the pipe reported the port closed, or
    /// the app called [`MidiInput::close`].
    closed: bool,
    /// A [`MidiNotifications`] exists (one reader at a time).
    reader_taken: bool,
}

impl Shared {
    /// The state lock. A panic while holding it cannot leave the plain data
    /// inside inconsistent, so a poisoned lock is recovered.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn close(&self) {
        self.lock().closed = true;
        self.replied.notify_all();
        self.notified.notify_all();
    }
}

impl State {
    /// Whether a reply or error with `tag` answers the request in flight:
    /// its own tag, or `0x7F` (the device could not read the header).
    fn answers(&self, tag: u8) -> bool {
        match self.waiting {
            Some(w) => tag == w || tag == TAG_UNREADABLE,
            None => false,
        }
    }

    /// Record `answer` if it is for the request in flight; returns whether
    /// it was.
    fn settle(&mut self, tag: u8, answer: std::result::Result<Frame, FrameError>) -> bool {
        if !self.answers(tag) {
            // A late reply to a request that timed out, or noise.
            return false;
        }
        self.waiting = None;
        self.reply = Some(answer);
        true
    }
}

/// The receiving half of a [`MidiTransport`]: the app's MIDI callback calls
/// [`MidiInput::receive`] with whatever bytes arrived.
#[derive(Clone)]
pub struct MidiInput {
    shared: Arc<Shared>,
}

impl MidiInput {
    /// Feed bytes from the device's MIDI OUT port, in any split, from any
    /// thread. Replies wake the waiting request; notifications are queued for
    /// [`Transport::notifications`]; everything else is ignored. Never blocks
    /// beyond a short lock.
    pub fn receive(&self, bytes: &[u8]) {
        let (mut replied, mut notified) = (false, false);
        {
            let mut st = self.shared.lock();
            for event in st.client.receive(bytes) {
                match event {
                    MidiEvent::Notification(packet) => {
                        if st.notes.len() >= NOTIFY_QUEUE {
                            st.notes.pop_front();
                        }
                        st.notes.push_back(packet);
                        notified = true;
                    }
                    MidiEvent::Reply { tag, frame } => replied |= st.settle(tag, Ok(frame)),
                    MidiEvent::Error { tag, error } => replied |= st.settle(tag, Err(error)),
                }
            }
        }
        if replied {
            self.shared.replied.notify_all();
        }
        if notified {
            self.shared.notified.notify_all();
        }
    }

    /// The port went away (the platform reported the device unplugged or
    /// the port closed): fails the request in flight with
    /// [`TransportError::Disconnected`] at once, ends the notification
    /// reader (after the packets already queued), and makes every later
    /// request fail the same way. Idempotent.
    pub fn close(&self) {
        self.shared.close();
    }
}

/// A blocking transport over a [`MidiPipe`].
///
/// One request in flight (the `&mut self` methods), matched by tag. `BUSY`,
/// `BULK_LOCKED` and `CRC_ERROR` answers are retried; `ERROR` is
/// [`TransportError::Stalled`], as a STALL is on USB; a request without an
/// answer within its timeout is [`TransportError::TimedOut`] and is not
/// resent (a late reply to it is dropped by its tag).
pub struct MidiTransport {
    descriptor: DeviceDescriptor,
    pipe: Arc<dyn MidiPipe>,
    shared: Arc<Shared>,
    timeout: Duration,
    bulk_timeout: Duration,
}

impl MidiTransport {
    /// Default per-request timeout: flash writes blind the device for about
    /// 45 ms and SysEx pacing varies by host (spec 4.6, 5).
    pub const TIMEOUT: Duration = Duration::from_millis(1500);

    /// Default timeout for the bulk opcodes (`0xA0`..`0xA3`): the image is a
    /// 7 KB SysEx message, and some hosts pace SysEx at a few KB/s (spec 5;
    /// the Console's MIDITransport uses the same 5 s).
    pub const BULK_TIMEOUT: Duration = Duration::from_secs(5);

    /// A transport sending through `pipe`; feed received bytes to the
    /// returned [`MidiInput`]. `serial` names the device for the picker.
    pub fn new(pipe: Arc<dyn MidiPipe>, mfr: &[u8], serial: &str) -> (Self, MidiInput) {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                client: MidiClient::new(mfr),
                waiting: None,
                reply: None,
                notes: VecDeque::new(),
                closed: false,
                reader_taken: false,
            }),
            replied: Condvar::new(),
            notified: Condvar::new(),
        });
        let transport = Self {
            descriptor: DeviceDescriptor {
                serial: serial.to_owned(),
                bus_id: "midi".into(),
                address: 0,
            },
            pipe,
            shared: Arc::clone(&shared),
            timeout: Self::TIMEOUT,
            bulk_timeout: Self::BULK_TIMEOUT,
        };
        (transport, MidiInput { shared })
    }

    /// Replace the per-request timeouts: `request` for every opcode but the
    /// bulk ones, `bulk` for `0xA0`..`0xA3`.
    pub fn with_timeouts(mut self, request: Duration, bulk: Duration) -> Self {
        self.timeout = request;
        self.bulk_timeout = bulk;
        self
    }

    fn timeout_for(&self, opcode: u8) -> Duration {
        let bulk = matches!(
            opcode,
            op::REQ_GET_ALL_PARAMS
                | op::REQ_SET_ALL_PARAMS
                | op::REQ_GET_ALL_PARAMS_CHUNK
                | op::REQ_SET_ALL_PARAMS_CHUNK
        );
        if bulk {
            self.bulk_timeout
        } else {
            self.timeout
        }
    }

    /// Send `frame` once and wait for its answer.
    fn round_trip(&self, opcode: u8, frame: &Frame, timeout: Duration) -> Result<Frame> {
        let bytes = {
            let mut st = self.shared.lock();
            if st.closed {
                return Err(TransportError::Disconnected);
            }
            let (tag, bytes) = st.client.request(frame);
            // In flight before the send: the pipe may deliver the reply
            // from inside `send`.
            st.waiting = Some(tag);
            st.reply = None;
            bytes
        };
        if let Err(e) = self.pipe.send(&bytes) {
            self.shared.lock().waiting = None;
            return Err(match e {
                PipeError::Closed => {
                    self.shared.close();
                    TransportError::Disconnected
                }
                PipeError::Failed(_) => TransportError::Remote(e.to_string()),
            });
        }
        let started = Instant::now();
        let mut st = self.shared.lock();
        loop {
            if let Some(answer) = st.reply.take() {
                return answer.map_err(|e| {
                    TransportError::Remote(format!(
                        "opcode 0x{opcode:02X}: unreadable reply over MIDI: {e}"
                    ))
                });
            }
            if st.closed {
                st.waiting = None;
                return Err(TransportError::Disconnected);
            }
            let left = timeout.saturating_sub(started.elapsed());
            if left.is_zero() {
                st.waiting = None;
                return Err(TransportError::TimedOut { opcode, retries: 0 });
            }
            st = match self.shared.replied.wait_timeout(st, left) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }

    /// One request with the spec's retry rule. Returns the GET payload (empty
    /// for a SET).
    fn transact(&self, opcode: u8, frame: &Frame, is_get: bool) -> Result<Vec<u8>> {
        let timeout = self.timeout_for(opcode);
        let mut attempt: u32 = 0;
        loop {
            let (code, payload) = match self.round_trip(opcode, frame, timeout)? {
                Frame::GetResponse { status, payload } => (status, Some(payload)),
                Frame::SetResponse { status } => (status, None),
                other => return Err(unexpected(opcode, &other)),
            };
            if code == status::OK {
                // An error may come back in either response type (the device
                // answers 0x81 when it could not read the request's type),
                // but a success must match the request.
                return match (payload, is_get) {
                    (Some(p), true) => Ok(p),
                    (None, false) => Ok(Vec::new()),
                    (Some(p), false) => Err(unexpected(
                        opcode,
                        &Frame::GetResponse {
                            status: code,
                            payload: p,
                        },
                    )),
                    (None, true) => Err(unexpected(opcode, &Frame::SetResponse { status: code })),
                };
            }
            if status::is_retryable(code) && attempt < STATUS_RETRIES {
                attempt = attempt.saturating_add(1);
                std::thread::sleep(STATUS_BACKOFF.saturating_mul(attempt));
                continue;
            }
            return Err(status_error(opcode, code, attempt));
        }
    }
}

/// A well-formed reply of the wrong kind for the request.
fn unexpected(opcode: u8, frame: &Frame) -> TransportError {
    let kind = match frame {
        Frame::Set { .. } => "a SET request",
        Frame::Get { .. } => "a GET request",
        Frame::SetResponse { .. } => "a SET response",
        Frame::GetResponse { .. } => "a GET response",
        Frame::Notification { .. } => "a notification",
    };
    TransportError::Remote(format!(
        "opcode 0x{opcode:02X}: the device answered with {kind}"
    ))
}

/// The error for a final non-OK status.
fn status_error(opcode: u8, code: u8, retries: u32) -> TransportError {
    match code {
        // The device stayed busy through every retry: the same outcome as a
        // USB transfer that kept timing out in a flash blackout.
        status::BUSY => TransportError::TimedOut {
            opcode,
            retries: usize::try_from(retries).unwrap_or(usize::MAX),
        },
        // Another host (USB, UART, I2C) holds the bulk buffer: not a busy
        // window, so the session must not treat it as one.
        status::BULK_LOCKED => TransportError::Locked,
        // What USB answers with a STALL.
        status::ERROR => TransportError::Stalled { opcode },
        status::BLOCKED => TransportError::Denied(format!(
            "opcode 0x{opcode:02X} is refused on this link (BLOCKED)"
        )),
        other => TransportError::Remote(format!(
            "opcode 0x{opcode:02X}: the device answered {} (0x{other:02X})",
            status::name(other)
        )),
    }
}

impl Drop for MidiTransport {
    /// Ends the notification reader ([`TransportError::Disconnected`]).
    fn drop(&mut self) {
        self.shared.close();
    }
}

impl Transport for MidiTransport {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        let mut data = self.control_in_upto(opcode, value, len)?;
        let wanted = usize::from(len);
        if data.len() < wanted {
            return Err(TransportError::ShortRead {
                opcode,
                wanted,
                got: data.len(),
            });
        }
        data.truncate(wanted);
        Ok(data)
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> Result<Vec<u8>> {
        let frame = Frame::Get {
            request: opcode,
            value,
            // As on USB: wIndex is the vendor interface, never a parameter.
            index: VENDOR_INTERFACE,
            length: max_len,
        };
        let mut data = self.transact(opcode, &frame, true)?;
        // USB never returns more than wLength; keep that contract.
        data.truncate(usize::from(max_len));
        Ok(data)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        // The frame's length field is 16 bits; a longer payload would be
        // encoded with a wrapped length.
        if u16::try_from(data.len()).is_err() {
            return Err(TransportError::Remote(format!(
                "opcode 0x{opcode:02X}: {} bytes do not fit one MIDI frame",
                data.len()
            )));
        }
        let frame = Frame::Set {
            request: opcode,
            value,
            index: VENDOR_INTERFACE,
            payload: data.to_vec(),
        };
        self.transact(opcode, &frame, false).map(|_| ())
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    /// No transfer cap: SysEx has no length limit and the bulk image goes in
    /// one message (spec 3.4).
    fn max_transfer(&self) -> usize {
        usize::MAX
    }

    /// The notifications the device sends on the MIDI port (tag 0), as the
    /// USB endpoint would deliver them.
    ///
    /// **One reader at a time:** while a reader returned by this method is
    /// alive, another call returns `None` (two readers would split the
    /// queue between them). Dropping the reader frees the slot. The reader
    /// ends with [`TransportError::Disconnected`] once the
    /// transport is dropped, the pipe reports the port closed or
    /// [`MidiInput::close`] is called.
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        {
            let mut st = self.shared.lock();
            if st.reader_taken {
                return None;
            }
            st.reader_taken = true;
        }
        Some(Box::new(MidiNotifications {
            shared: Arc::clone(&self.shared),
        }))
    }

    /// `chunked_bulk: false`: the firmware serves `0xA2`/`0xA3` over MIDI
    /// (spec 4.2), but single-shot `0xA0`/`0xA1` are the recommended bulk
    /// commands there (one message, no chunk session to abort), so the
    /// session reads and writes the image in one request.
    fn caps(&self) -> TransportCaps {
        TransportCaps {
            kind: crate::TransportKind::Midi,
            chunked_bulk: false,
            remote: false,
        }
    }
}

/// The notification reader of a [`MidiTransport`].
struct MidiNotifications {
    shared: Arc<Shared>,
}

impl Drop for MidiNotifications {
    fn drop(&mut self) {
        self.shared.lock().reader_taken = false;
    }
}

impl NotificationSource for MidiNotifications {
    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        let started = Instant::now();
        let mut st = self.shared.lock();
        loop {
            if let Some(packet) = st.notes.pop_front() {
                return Ok(packet);
            }
            if st.closed {
                return Err(TransportError::Disconnected);
            }
            let left = timeout.saturating_sub(started.elapsed());
            if left.is_zero() {
                return Ok(Vec::new());
            }
            st = match self.shared.notified.wait_timeout(st, left) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }
}

/// Desktop MIDI ports through `midir` (ALSA, WinMM, CoreMIDI). WP T3.
#[cfg(feature = "midir")]
pub mod midir_port {
    /// A DSPi's MIDI port pair, by name ("DSPi Control").
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct PortInfo {
        pub name: String,
    }

    /// The DSPi control ports visible to this host.
    pub fn list_ports() -> Vec<PortInfo> {
        todo!("WP T3")
    }

    /// Open a port pair and return a ready transport.
    pub fn open(port: &PortInfo) -> crate::Result<super::MidiTransport> {
        let _ = port;
        todo!("WP T3")
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_t2 {
    use super::*;
    use crate::{TransportKind, is_own_write};
    use dspi_proto::frame::{DEFAULT_MFR_ID, MAX_SET_PAYLOAD, pack7};
    use dspi_proto::generated::BULK_SIZE;
    use dspi_proto::generated::notify as n;
    use proptest::prelude::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    type Inject = Box<dyn FnOnce(u8) -> Vec<u8> + Send>;

    /// The platform reply of the spec's worked example (RP2350, v1.1.6,
    /// 4 outputs).
    const PLATFORM: [u8; 7] = [0x01, 0x01, 0x16, 0x04, 0x01, 0x06, 0x00];

    /// The bulk offset the fake reports a master-volume change at. Any value
    /// does for the transport; the session maps offsets.
    const VOLUME_OFFSET: u16 = 8;

    /// The device state and scripted misbehaviour.
    struct Dev {
        decoder: SysexDecoder,
        volume: [u8; 4],
        image: Vec<u8>,
        seq: u8,
        /// Requests the device saw, in order.
        log: Vec<Frame>,
        /// Answer this many requests with `CRC_ERROR` first.
        crc_errors: usize,
        /// Answer this many requests with `BUSY` first.
        busy: usize,
        /// Never answer.
        silent: bool,
        /// Send a notification of another host's change ahead of each reply.
        notify_first: bool,
        /// Deliver the output in pieces of this size (0: whole).
        piece: usize,
        /// Put foreign SysEx, a note and real-time bytes around the output.
        noise: bool,
        /// Raw answers that replace the next replies (given the request tag).
        inject: VecDeque<Inject>,
    }

    /// An in-process DSPi that speaks the MIDI control channel, built on
    /// `dspi_proto::frame`. Changes made over MIDI notify with source
    /// `PARAM_SRC_MIDI` (10), bulk applies with `BULK_SET`, as the firmware
    /// does (spec 4.2, bulk_params.c).
    struct FakeMidiDevice {
        dev: Mutex<Dev>,
        input: Mutex<Option<MidiInput>>,
        /// Deliver from another thread after this pause, instead of from
        /// inside `send`.
        threaded: Option<Duration>,
        sends: AtomicUsize,
        pipe_error: Mutex<Option<PipeError>>,
    }

    impl FakeMidiDevice {
        fn new() -> Self {
            Self::with(None)
        }

        fn with(threaded: Option<Duration>) -> Self {
            Self {
                dev: Mutex::new(Dev {
                    decoder: SysexDecoder::new(DEFAULT_MFR_ID),
                    volume: (-20.0f32).to_le_bytes(),
                    image: (0..BULK_SIZE).map(|i| (i * 7 + 3) as u8).collect(),
                    seq: 0,
                    log: Vec::new(),
                    crc_errors: 0,
                    busy: 0,
                    silent: false,
                    notify_first: false,
                    piece: 0,
                    noise: false,
                    inject: VecDeque::new(),
                }),
                input: Mutex::new(None),
                threaded,
                sends: AtomicUsize::new(0),
                pipe_error: Mutex::new(None),
            }
        }

        fn dev(&self) -> MutexGuard<'_, Dev> {
            self.dev.lock().unwrap()
        }

        fn deliver(&self, bytes: Vec<u8>, piece: usize) {
            let Some(input) = self.input.lock().unwrap().clone() else {
                return;
            };
            let push = move || {
                if piece == 0 {
                    input.receive(&bytes);
                } else {
                    for c in bytes.chunks(piece) {
                        input.receive(c);
                    }
                }
            };
            match self.threaded {
                Some(pause) => {
                    std::thread::spawn(move || {
                        std::thread::sleep(pause);
                        push();
                    });
                }
                None => push(),
            }
        }
    }

    fn sysex(tag: u8, frame: &Frame) -> Vec<u8> {
        encode_sysex(DEFAULT_MFR_ID, tag, frame)
    }

    impl Dev {
        fn next_seq(&mut self) -> u8 {
            self.seq = self.seq.wrapping_add(1);
            self.seq
        }

        fn param_changed(&mut self, offset: u16, source: u16, bytes: &[u8]) -> Vec<u8> {
            let mut p = vec![2, n::NOTIFY_EVT_PARAM_CHANGED as u8, 0, self.next_seq()];
            p.extend(offset.to_le_bytes());
            p.extend((bytes.len() as u16).to_le_bytes());
            p.extend([source as u8, 0, 0, 0]);
            p.extend(bytes);
            sysex(0, &Frame::Notification { packet: p })
        }

        fn bulk_invalidated(&mut self, source: u16) -> Vec<u8> {
            let p = vec![
                2,
                n::NOTIFY_EVT_BULK_INVALIDATED as u8,
                0,
                self.next_seq(),
                source as u8,
                0,
                0,
                0,
            ];
            sysex(0, &Frame::Notification { packet: p })
        }

        /// The device's answer to one message: reply bytes, then
        /// notifications.
        fn handle(&mut self, tag: u8, frame: std::result::Result<Frame, FrameError>) -> Vec<u8> {
            let mut out = Vec::new();
            if self.notify_first {
                let other = self.param_changed(100, n::PARAM_SRC_HOST_SET, &[1, 2]);
                out.extend(other);
            }
            let frame = match frame {
                Ok(f) => f,
                Err(e) => {
                    let s = if matches!(e, FrameError::Crc { .. }) {
                        status::CRC_ERROR
                    } else {
                        status::FRAME_ERROR
                    };
                    out.extend(sysex(tag, &Frame::SetResponse { status: s }));
                    return out;
                }
            };
            self.log.push(frame.clone());
            if let Some(f) = self.inject.pop_front() {
                out.extend(f(tag));
                return out;
            }
            if self.silent {
                return out;
            }
            let is_get = matches!(frame, Frame::Get { .. });
            let refuse = |s: u8| {
                if is_get {
                    Frame::GetResponse {
                        status: s,
                        payload: Vec::new(),
                    }
                } else {
                    Frame::SetResponse { status: s }
                }
            };
            if self.crc_errors > 0 {
                self.crc_errors -= 1;
                out.extend(sysex(tag, &refuse(status::CRC_ERROR)));
                return out;
            }
            if self.busy > 0 {
                self.busy -= 1;
                out.extend(sysex(tag, &refuse(status::BUSY)));
                return out;
            }
            let ok = |payload: Vec<u8>| Frame::GetResponse {
                status: status::OK,
                payload,
            };
            let mut after = Vec::new();
            let reply = match frame {
                Frame::Get {
                    request, length, ..
                } => match request {
                    op::REQ_GET_PLATFORM => {
                        ok(PLATFORM[..PLATFORM.len().min(length as usize)].to_vec())
                    }
                    op::REQ_GET_MASTER_VOLUME => ok(self.volume.to_vec()),
                    op::REQ_GET_ALL_PARAMS => ok(self.image.clone()),
                    _ => refuse(status::ERROR),
                },
                Frame::Set {
                    request, payload, ..
                } => match request {
                    op::REQ_SET_MASTER_VOLUME if payload.len() == 4 => {
                        self.volume.copy_from_slice(&payload);
                        after = self.param_changed(VOLUME_OFFSET, n::PARAM_SRC_MIDI, &payload);
                        Frame::SetResponse { status: status::OK }
                    }
                    op::REQ_SET_ALL_PARAMS if payload.len() == BULK_SIZE => {
                        self.image = payload;
                        after = self.bulk_invalidated(n::PARAM_SRC_BULK_SET);
                        Frame::SetResponse { status: status::OK }
                    }
                    _ if payload.len() > MAX_SET_PAYLOAD => refuse(status::OVERSIZE),
                    _ => refuse(status::ERROR),
                },
                _ => refuse(status::FRAME_ERROR),
            };
            out.extend(sysex(tag, &reply));
            out.extend(after);
            out
        }
    }

    impl MidiPipe for FakeMidiDevice {
        fn send(&self, sysex_in: &[u8]) -> std::result::Result<(), PipeError> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            if let Some(e) = self.pipe_error.lock().unwrap().clone() {
                return Err(e);
            }
            let (mut out, piece, noise) = {
                let mut d = self.dev();
                let mut out = Vec::new();
                for m in d.decoder.feed(sysex_in) {
                    out.extend(d.handle(m.tag, m.frame));
                }
                (out, d.piece, d.noise)
            };
            if noise {
                let mut n = vec![0x90, 0x40, 0x7F, 0xF8]; // note on, clock
                n.extend([0xF0, 0x43, 0x10, 0x4C, 0xF7]); // Yamaha SysEx
                n.extend(&out);
                n.extend([0xF7, 0xFE]); // stray F7, active sensing
                out = n;
            }
            self.deliver(out, piece);
            Ok(())
        }
    }

    fn connect(dev: FakeMidiDevice) -> (MidiTransport, Arc<FakeMidiDevice>, MidiInput) {
        let dev = Arc::new(dev);
        let pipe: Arc<dyn MidiPipe> = dev.clone();
        let (t, input) = MidiTransport::new(pipe, DEFAULT_MFR_ID, "MIDI-FAKE-0001");
        *dev.input.lock().unwrap() = Some(input.clone());
        (t, dev, input)
    }

    /// A packet's source byte: PARAM_CHANGED byte 8, BULK_INVALIDATED byte 4.
    fn source_of(p: &[u8]) -> u8 {
        match p[1] as u16 {
            n::NOTIFY_EVT_PARAM_CHANGED => p[8],
            n::NOTIFY_EVT_BULK_INVALIDATED => p[4],
            other => panic!("event {other}"),
        }
    }

    // -- the sans-IO client ------------------------------------------------

    #[test]
    fn a_get_round_trips_through_the_client() {
        let mut c = MidiClient::new(DEFAULT_MFR_ID);
        let get = Frame::Get {
            request: 0x7F,
            value: 0,
            index: 0,
            length: 0,
        };
        // Spec 3.3, worked example.
        let (tag, bytes) = c.request(&get);
        assert_eq!(tag, 1);
        assert_eq!(
            bytes,
            [
                0xF0, 0x7D, 0x44, 0x53, 0x50, 0x01, 0x01, 0x00, 0x02, 0x7F, 0, 0, 0, 0, 0, 0x04,
                0x00, 0x10, 0x30, 0xF7
            ]
        );
        let reply = [
            0xF0, 0x7D, 0x44, 0x53, 0x50, 0x01, 0x01, 0x01, 0x02, 0x00, 0x07, 0x00, 0x01, 0x01,
            0x16, 0x10, 0x04, 0x01, 0x06, 0x00, 0x44, 0x2F, 0xF7,
        ];
        let mut events = Vec::new();
        for b in reply {
            events.extend(c.receive(&[b]));
        }
        assert_eq!(
            events,
            [MidiEvent::Reply {
                tag: 1,
                frame: Frame::GetResponse {
                    status: 0,
                    payload: PLATFORM.to_vec()
                }
            }]
        );
    }

    #[test]
    fn wp_t2_tags_run_1_to_7e_and_wrap() {
        let mut c = MidiClient::new(DEFAULT_MFR_ID);
        let f = Frame::SetResponse { status: 0 };
        let tags: Vec<u8> = (0..0x7E * 2 + 1).map(|_| c.request(&f).0).collect();
        assert_eq!(tags[0], 1);
        assert_eq!(tags[0x7D], 0x7E);
        assert_eq!(tags[0x7E], 1, "wraps past 0x7E, skipping 0x7F and 0");
        assert!(tags.iter().all(|t| (1..=0x7E).contains(t)));
    }

    #[test]
    fn wp_t2_client_classifies_notifications_and_errors() {
        let mut c = MidiClient::new(DEFAULT_MFR_ID);
        let note = sysex(
            0,
            &Frame::Notification {
                packet: vec![2, 3, 0, 1, 10, 0, 0, 0],
            },
        );
        assert_eq!(
            c.receive(&note),
            [MidiEvent::Notification(vec![2, 3, 0, 1, 10, 0, 0, 0])]
        );
        // A frame whose CRC does not match: an error with its tag.
        let mut f = Frame::SetResponse { status: 0 }.encode();
        f[1] = 2;
        let mut m = vec![0xF0, 0x7D, b'D', b'S', b'P', 1, 9];
        m.extend(pack7(&f));
        m.push(0xF7);
        let ev = c.receive(&m);
        assert!(
            matches!(
                ev[..],
                [MidiEvent::Error {
                    tag: 9,
                    error: FrameError::Crc { .. }
                }]
            ),
            "{ev:?}"
        );
        // Another manufacturer: nothing.
        assert!(c.receive(&[0xF0, 0x43, 0x10, 0x4C, 0xF7]).is_empty());
    }

    // -- the transport against the fake device ------------------------------

    #[test]
    fn wp_t2_get_and_set_through_the_transport() {
        let (mut t, dev, _) = connect(FakeMidiDevice::new());
        assert_eq!(
            t.control_in_upto(op::REQ_GET_PLATFORM, 0, 7).unwrap(),
            PLATFORM
        );
        t.control_out(op::REQ_SET_MASTER_VOLUME, 0, &(-6.5f32).to_le_bytes())
            .unwrap();
        assert_eq!(
            t.control_in(op::REQ_GET_MASTER_VOLUME, 0, 4).unwrap(),
            (-6.5f32).to_le_bytes()
        );
        // wIndex is the vendor interface, as on USB.
        let log = dev.dev().log.clone();
        assert_eq!(log.len(), 3);
        assert!(
            log.iter()
                .all(|f| matches!(f, Frame::Get { index: 2, .. } | Frame::Set { index: 2, .. }))
        );
        assert_eq!(t.caps().kind, TransportKind::Midi);
        assert!(!t.caps().chunked_bulk);
        assert_eq!(t.descriptor().serial, "MIDI-FAKE-0001");
    }

    #[test]
    fn notifications_between_replies_are_delivered() {
        let (mut t, dev, _) = connect(FakeMidiDevice::new());
        dev.dev().notify_first = true;
        let mut notes = t.notifications().unwrap();
        t.control_out(op::REQ_SET_MASTER_VOLUME, 0, &(-3.0f32).to_le_bytes())
            .unwrap();
        assert_eq!(
            t.control_in_upto(op::REQ_GET_PLATFORM, 0, 7).unwrap(),
            PLATFORM
        );
        let got: Vec<Vec<u8>> = (0..3)
            .map(|_| notes.read(Duration::from_millis(200)).unwrap())
            .collect();
        // Another host's USB write, our own MIDI write, another USB write.
        let sources: Vec<u8> = got.iter().map(|p| source_of(p)).collect();
        assert_eq!(
            sources,
            [
                n::PARAM_SRC_HOST_SET as u8,
                n::PARAM_SRC_MIDI as u8,
                n::PARAM_SRC_HOST_SET as u8
            ]
        );
        let seqs: Vec<u8> = got.iter().map(|p| p[3]).collect();
        assert_eq!(seqs, [1, 2, 3], "in order, none lost");
        // The echo rule (§1.2): over MIDI our writes come back as MIDI.
        let own: Vec<bool> = sources
            .iter()
            .map(|&s| is_own_write(TransportKind::Midi, s, None, t.session_id()))
            .collect();
        assert_eq!(own, [false, true, false]);
        assert_eq!(&got[1][12..16], &(-3.0f32).to_le_bytes());
        assert_eq!(
            notes.read(Duration::from_millis(10)).unwrap(),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn wp_t2_bulk_goes_single_shot() {
        let (mut t, dev, _) = connect(FakeMidiDevice::new());
        let mut notes = t.notifications().unwrap();
        let image = t
            .control_in(op::REQ_GET_ALL_PARAMS, 0, BULK_SIZE as u16)
            .unwrap();
        assert_eq!(image, dev.dev().image);
        let new: Vec<u8> = image.iter().map(|b| b ^ 0x5A).collect();
        t.control_out(op::REQ_SET_ALL_PARAMS, 0, &new).unwrap();
        assert_eq!(dev.dev().image, new);
        // One message each way.
        assert_eq!(dev.sends.load(Ordering::SeqCst), 2);
        let p = notes.read(Duration::from_millis(200)).unwrap();
        assert_eq!(p[1] as u16, n::NOTIFY_EVT_BULK_INVALIDATED);
        assert!(is_own_write(TransportKind::Midi, source_of(&p), None, None));
        assert_eq!(t.max_transfer(), usize::MAX);
    }

    #[test]
    fn wp_t2_crc_error_is_retried() {
        let (mut t, dev, _) = connect(FakeMidiDevice::new());
        dev.dev().crc_errors = 2;
        t.control_out(op::REQ_SET_MASTER_VOLUME, 0, &1.0f32.to_le_bytes())
            .unwrap();
        assert_eq!(dev.dev().log.len(), 3, "two CRC errors, then applied");
        assert_eq!(dev.dev().volume, 1.0f32.to_le_bytes());
    }

    #[test]
    fn wp_t2_busy_is_retried_then_gives_up() {
        let (mut t, dev, _) = connect(FakeMidiDevice::new());
        dev.dev().busy = 1;
        assert_eq!(
            t.control_in_upto(op::REQ_GET_PLATFORM, 0, 7).unwrap(),
            PLATFORM
        );
        dev.dev().busy = 100;
        let e = t.control_in_upto(op::REQ_GET_PLATFORM, 0, 7).unwrap_err();
        assert!(
            matches!(e, TransportError::TimedOut { opcode: 0x7F, retries } if retries == crate::BUSY_RETRIES),
            "{e:?}"
        );
        assert_eq!(dev.dev().log.len(), 2 + crate::BUSY_RETRIES + 1);
    }

    #[test]
    fn wp_t2_a_timeout_fails_and_a_late_reply_is_dropped() {
        let (t, dev, input) = connect(FakeMidiDevice::new());
        let mut t = t.with_timeouts(Duration::from_millis(50), Duration::from_millis(50));
        dev.dev().silent = true;
        let started = Instant::now();
        let e = t.control_in_upto(op::REQ_GET_PLATFORM, 0, 7).unwrap_err();
        assert!(
            matches!(e, TransportError::TimedOut { opcode: 0x7F, .. }),
            "{e:?}"
        );
        assert!(started.elapsed() >= Duration::from_millis(50));
        // The answer to that request (tag 1) arrives late, then the device
        // recovers: the next request (tag 2) must get its own reply, not the
        // stale one.
        input.receive(&sysex(
            1,
            &Frame::GetResponse {
                status: 0,
                payload: vec![9; 7],
            },
        ));
        dev.dev().silent = false;
        assert_eq!(
            t.control_in_upto(op::REQ_GET_PLATFORM, 0, 7).unwrap(),
            PLATFORM
        );
    }

    #[test]
    fn wp_t2_a_7f_error_fails_the_request_in_flight() {
        let (mut t, dev, _) = connect(FakeMidiDevice::new());
        dev.dev().inject.push_back(Box::new(|_| {
            sysex(
                TAG_UNREADABLE,
                &Frame::SetResponse {
                    status: status::FRAME_ERROR,
                },
            )
        }));
        let e = t.control_in_upto(op::REQ_GET_PLATFORM, 0, 7).unwrap_err();
        assert!(
            matches!(e, TransportError::Remote(ref m) if m.contains("FRAME_ERROR")),
            "{e:?}"
        );
        assert_eq!(dev.dev().log.len(), 1, "not retried");
    }

    #[test]
    fn wp_t2_an_unreadable_reply_fails_at_once() {
        let (mut t, dev, _) = connect(FakeMidiDevice::new());
        // A reply under our tag whose CRC is wrong: fails now, not after
        // the (default, 1.5 s) timeout, and is not resent (the device may
        // have acted).
        dev.dev().inject.push_back(Box::new(|tag| {
            let mut f = Frame::SetResponse { status: 0 }.encode();
            f[2] = 1;
            let mut m = vec![0xF0, 0x7D, b'D', b'S', b'P', 1, tag];
            m.extend(pack7(&f));
            m.push(0xF7);
            m
        }));
        let started = Instant::now();
        let e = t
            .control_out(op::REQ_SET_MASTER_VOLUME, 0, &0.0f32.to_le_bytes())
            .unwrap_err();
        assert!(matches!(e, TransportError::Remote(_)), "{e:?}");
        assert!(started.elapsed() < Duration::from_millis(500));
        assert_eq!(dev.dev().log.len(), 1);
    }

    #[test]
    fn wp_t2_statuses_map_to_transport_errors() {
        let (mut t, dev, _) = connect(FakeMidiDevice::new());
        let e = t.control_in_upto(0x55, 0, 4).unwrap_err();
        assert!(
            matches!(e, TransportError::Stalled { opcode: 0x55 }),
            "{e:?}"
        );
        let e = t.control_out(0x56, 0, &[0; 65]).unwrap_err();
        assert!(
            matches!(e, TransportError::Remote(ref m) if m.contains("OVERSIZE")),
            "{e:?}"
        );
        dev.dev().inject.push_back(Box::new(|tag| {
            sysex(
                tag,
                &Frame::SetResponse {
                    status: status::BLOCKED,
                },
            )
        }));
        let e = t.control_out(0xF5, 0, &[1]).unwrap_err();
        assert!(matches!(e, TransportError::Denied(_)), "{e:?}");
        // A success of the wrong kind is a protocol error, not data.
        dev.dev().inject.push_back(Box::new(|tag| {
            sysex(tag, &Frame::SetResponse { status: 0 })
        }));
        let e = t.control_in_upto(op::REQ_GET_PLATFORM, 0, 7).unwrap_err();
        assert!(matches!(e, TransportError::Remote(_)), "{e:?}");
    }

    #[test]
    fn wp_t2_a_short_reply_is_a_short_read_except_upto() {
        let (mut t, _, _) = connect(FakeMidiDevice::new());
        let e = t.control_in(op::REQ_GET_PLATFORM, 0, 8).unwrap_err();
        assert!(
            matches!(
                e,
                TransportError::ShortRead {
                    opcode: 0x7F,
                    wanted: 8,
                    got: 7
                }
            ),
            "{e:?}"
        );
        assert_eq!(
            t.control_in_upto(op::REQ_GET_PLATFORM, 0, 8).unwrap(),
            PLATFORM
        );
        // Never more than asked for.
        assert_eq!(
            t.control_in(op::REQ_GET_MASTER_VOLUME, 0, 2).unwrap().len(),
            2
        );
    }

    #[test]
    fn wp_t2_any_split_and_noise_are_tolerated() {
        for piece in [1, 2, 3, 7, 64] {
            let (mut t, dev, _) = connect(FakeMidiDevice::new());
            {
                let mut d = dev.dev();
                d.piece = piece;
                d.noise = true;
                d.notify_first = true;
            }
            let mut notes = t.notifications().unwrap();
            assert_eq!(
                t.control_in_upto(op::REQ_GET_PLATFORM, 0, 7).unwrap(),
                PLATFORM
            );
            let image = t
                .control_in(op::REQ_GET_ALL_PARAMS, 0, BULK_SIZE as u16)
                .unwrap();
            assert_eq!(image.len(), BULK_SIZE, "piece {piece}");
            assert_eq!(notes.read(Duration::ZERO).unwrap()[3], 1);
            assert_eq!(notes.read(Duration::ZERO).unwrap()[3], 2);
        }
    }

    #[test]
    fn wp_t2_replies_from_another_thread() {
        let (mut t, dev, _) = connect(FakeMidiDevice::with(Some(Duration::from_millis(5))));
        let mut notes = t.notifications().unwrap();
        let reader = std::thread::spawn(move || notes.read(Duration::from_secs(2)));
        for _ in 0..20 {
            assert_eq!(
                t.control_in_upto(op::REQ_GET_PLATFORM, 0, 7).unwrap(),
                PLATFORM
            );
        }
        t.control_out(op::REQ_SET_MASTER_VOLUME, 0, &2.0f32.to_le_bytes())
            .unwrap();
        let p = reader.join().unwrap().unwrap();
        assert_eq!(source_of(&p), n::PARAM_SRC_MIDI as u8);
        assert_eq!(dev.sends.load(Ordering::SeqCst), 21);
    }

    #[test]
    fn wp_t2_pipe_errors_are_typed() {
        let (mut t, dev, _) = connect(FakeMidiDevice::new());
        let mut notes = t.notifications().unwrap();
        *dev.pipe_error.lock().unwrap() = Some(PipeError::Failed("no route".into()));
        let e = t.control_in_upto(op::REQ_GET_PLATFORM, 0, 7).unwrap_err();
        assert!(
            matches!(e, TransportError::Remote(ref m) if m.contains("no route")),
            "{e:?}"
        );
        assert_eq!(notes.read(Duration::ZERO).unwrap(), Vec::<u8>::new());
        *dev.pipe_error.lock().unwrap() = Some(PipeError::Closed);
        let e = t.control_in_upto(op::REQ_GET_PLATFORM, 0, 7).unwrap_err();
        assert!(matches!(e, TransportError::Disconnected), "{e:?}");
        // A closed port ends the notification reader and every later request.
        assert!(matches!(
            notes.read(Duration::from_secs(1)),
            Err(TransportError::Disconnected)
        ));
        *dev.pipe_error.lock().unwrap() = None;
        assert!(matches!(
            t.control_in_upto(op::REQ_GET_PLATFORM, 0, 7),
            Err(TransportError::Disconnected)
        ));
    }

    #[test]
    fn wp_t2_dropping_the_transport_ends_the_reader() {
        let (t, _, input) = connect(FakeMidiDevice::new());
        let mut notes = t.notifications().unwrap();
        let reader = std::thread::spawn(move || {
            let mut got = Vec::new();
            loop {
                match notes.read(Duration::from_secs(5)) {
                    Ok(p) if p.is_empty() => {}
                    Ok(p) => got.push(p),
                    Err(e) => return (got, e),
                }
            }
        });
        input.receive(&sysex(
            0,
            &Frame::Notification {
                packet: vec![2, 3, 0, 1, 1, 0, 0, 0],
            },
        ));
        std::thread::sleep(Duration::from_millis(20));
        drop(t);
        let (got, e) = reader.join().unwrap();
        assert_eq!(got.len(), 1);
        assert!(matches!(e, TransportError::Disconnected));
    }

    #[test]
    fn wp_t2_the_notification_queue_is_bounded() {
        let (t, _, input) = connect(FakeMidiDevice::new());
        let mut notes = t.notifications().unwrap();
        for i in 0..NOTIFY_QUEUE + 10 {
            input.receive(&sysex(
                0,
                &Frame::Notification {
                    packet: vec![2, 3, 0, i as u8, 1, 0, 0, 0],
                },
            ));
        }
        let first = notes.read(Duration::ZERO).unwrap();
        assert_eq!(first[3], 10, "the oldest were dropped; seq shows the gap");
        let mut count = 1;
        while !notes.read(Duration::ZERO).unwrap().is_empty() {
            count += 1;
        }
        assert_eq!(count, NOTIFY_QUEUE);
    }

    #[test]
    fn wp_t2_an_oversized_payload_is_refused_locally() {
        let (mut t, dev, _) = connect(FakeMidiDevice::new());
        let e = t
            .control_out(op::REQ_SET_ALL_PARAMS, 0, &vec![0; 70_000])
            .unwrap_err();
        assert!(matches!(e, TransportError::Remote(_)), "{e:?}");
        assert_eq!(dev.sends.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn wp_t2_closing_the_input_fails_the_request_and_ends_the_reader() {
        let (mut t, dev, input) = connect(FakeMidiDevice::new());
        dev.dev().silent = true;
        let mut notes = t.notifications().unwrap();
        let reader = std::thread::spawn(move || notes.read(Duration::from_secs(5)));
        let closer = {
            let input = input.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(50));
                input.close();
            })
        };
        let started = Instant::now();
        // Default timeout 1.5 s: the close must end the wait well before.
        let e = t.control_in_upto(op::REQ_GET_PLATFORM, 0, 7).unwrap_err();
        assert!(matches!(e, TransportError::Disconnected), "{e:?}");
        assert!(started.elapsed() < Duration::from_millis(1000));
        closer.join().unwrap();
        assert!(matches!(
            reader.join().unwrap(),
            Err(TransportError::Disconnected)
        ));
        let sends = dev.sends.load(Ordering::SeqCst);
        assert!(matches!(
            t.control_out(op::REQ_SET_MASTER_VOLUME, 0, &[0; 4]),
            Err(TransportError::Disconnected)
        ));
        assert_eq!(
            dev.sends.load(Ordering::SeqCst),
            sends,
            "nothing sent after close"
        );
        input.close(); // idempotent
    }

    #[test]
    fn wp_t2_one_notification_reader_at_a_time() {
        let (t, _, _) = connect(FakeMidiDevice::new());
        let first = t.notifications().unwrap();
        assert!(
            t.notifications().is_none(),
            "a second reader would split the queue"
        );
        drop(first);
        assert!(
            t.notifications().is_some(),
            "dropping the reader frees the slot"
        );
    }

    #[test]
    fn wp_t2_a_final_bulk_locked_is_locked() {
        let (mut t, dev, _) = connect(FakeMidiDevice::new());
        for _ in 0..=STATUS_RETRIES {
            dev.dev().inject.push_back(Box::new(|tag| {
                sysex(
                    tag,
                    &Frame::GetResponse {
                        status: status::BULK_LOCKED,
                        payload: Vec::new(),
                    },
                )
            }));
        }
        let e = t
            .control_in(op::REQ_GET_ALL_PARAMS, 0, BULK_SIZE as u16)
            .unwrap_err();
        assert!(matches!(e, TransportError::Locked), "{e:?}");
        assert_eq!(
            dev.dev().log.len(),
            1 + STATUS_RETRIES as usize,
            "retried first"
        );
        // Once the lock is released the read goes through.
        let image = t
            .control_in(op::REQ_GET_ALL_PARAMS, 0, BULK_SIZE as u16)
            .unwrap();
        assert_eq!(image.len(), BULK_SIZE);
    }

    // -- property tests (§1.9) -----------------------------------------------

    fn any_frame() -> impl Strategy<Value = Frame> {
        prop_oneof![
            any::<u8>().prop_map(|status| Frame::SetResponse { status }),
            (any::<u8>(), prop::collection::vec(any::<u8>(), 0..300))
                .prop_map(|(status, payload)| Frame::GetResponse { status, payload }),
            (any::<u8>(), any::<u16>(), any::<u16>(), any::<u16>()).prop_map(
                |(request, value, index, length)| Frame::Get {
                    request,
                    value,
                    index,
                    length
                }
            ),
        ]
    }

    proptest! {
        #[test]
        fn proptest_midi_client_receive_never_panics(
            bytes in prop::collection::vec(any::<u8>(), 0..2048),
            piece in 1usize..64,
        ) {
            let mut c = MidiClient::new(DEFAULT_MFR_ID);
            for chunk in bytes.chunks(piece) {
                for e in c.receive(chunk) {
                    let is_notify_reply = matches!(
                        e,
                        MidiEvent::Reply { frame: Frame::Notification { .. }, .. }
                    );
                    prop_assert!(!is_notify_reply, "a notification classified as a reply");
                }
            }
        }

        #[test]
        fn proptest_midi_client_replies_match_their_tags(
            tag in 1u8..=0x7E,
            frame in any_frame(),
            noise in prop::collection::vec(any::<u8>(), 0..64),
            piece in 1usize..32,
        ) {
            let mut c = MidiClient::new(DEFAULT_MFR_ID);
            // Noise first, then an F7 so a message left open in it ends;
            // the decoder must then find ours exactly once.
            let mut stream = noise;
            stream.push(0xF7);
            stream.extend(sysex(tag, &frame));
            let mut events = Vec::new();
            for chunk in stream.chunks(piece) {
                events.extend(c.receive(chunk));
            }
            let want = MidiEvent::Reply { tag, frame };
            prop_assert_eq!(events.iter().filter(|e| **e == want).count(), 1);
            prop_assert_eq!(events.last(), Some(&want));
        }
    }
}
