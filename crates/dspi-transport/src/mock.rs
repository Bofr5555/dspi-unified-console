//! A scripted device, for testing without hardware.
//!
//! This exists mainly to make the *awkward* paths testable: a flash blackout
//! that stalls for a few transfers and then recovers, a device that reports a
//! wire version we do not implement, a short read, a mid-session disconnect.
//! Those are exactly the behaviours that are hard to provoke on demand with real
//! hardware and expensive to get wrong in the field.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::{CmdId, DeviceDescriptor, NotificationSource, Result, Transport, TransportError};
use dspi_proto::VENDOR_INTERFACE;

/// Packets a test has queued for the notification endpoint.
pub type NotifyQueue = Arc<Mutex<VecDeque<Vec<u8>>>>;

/// The mock's notification endpoint: hands out queued packets, and an empty
/// read (a timeout) when the queue is dry. A `Disconnect` is modelled by
/// queuing an empty packet followed by nothing; tests that need an error push
/// the sentinel `MockNotifications::DISCONNECT`.
pub struct MockNotifications {
    queue: NotifyQueue,
    /// The beta4 firmware's keep-alive pacing: with nothing queued the
    /// endpoint NAKs, and an idle packet goes out only after this long
    /// (usb_audio.c:1064-1080). `None` models the old endpoint, which a
    /// read leaves empty-handed almost at once.
    pace: Option<Duration>,
}

impl MockNotifications {
    /// A packet that makes the next read report a disconnect.
    pub const DISCONNECT: &'static [u8] = &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
}

impl NotificationSource for MockNotifications {
    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        let next = self.queue.lock().unwrap().pop_front();
        match next {
            Some(p) if p == Self::DISCONNECT => Err(TransportError::Disconnected),
            Some(p) => Ok(p),
            None => match self.pace {
                // Paced: the read blocks until the keep-alive is due, and
                // returns the one-byte idle packet, unless the caller's
                // timeout runs out first, which reads as silence.
                Some(pace) => {
                    crate::clock::sleep(pace.min(timeout));
                    if pace <= timeout {
                        Ok(vec![0])
                    } else {
                        Ok(Vec::new())
                    }
                }
                // A real endpoint blocks for the timeout; give the reader
                // thread the same pause so tests do not spin a core.
                None => {
                    crate::clock::sleep(Duration::from_millis(5));
                    Ok(Vec::new())
                }
            },
        }
    }
}

/// What the mock should do for a given opcode.
#[derive(Debug, Clone)]
pub enum Reply {
    /// Answer with these bytes.
    Data(Vec<u8>),
    /// A window into a larger buffer, sliced at the byte offset carried in
    /// `wValue`. Models the chunked bulk opcodes (`0xA2` / `0xA3`), where the
    /// host walks a 6136-byte packet in transfer-sized pieces. Without this the
    /// mock would hand back the same prefix every time and a chunking bug would
    /// sail through the tests.
    Window(Vec<u8>),
    /// Refuse, as the firmware does for an unknown opcode or a bad index.
    Stall,
    /// Stall `n` more times, then fall through to the next reply. Models the
    /// flash blackout: the device is alive but its control IRQ is disabled.
    BusyThen(usize, Box<Reply>),
    /// Answer each reply in turn, then keep answering the last one.
    ///
    /// Models a value that changes between reads, which is what the write
    /// path's confirming readback actually sees: the old value on the way in,
    /// the new one on the way out. A single canned answer makes every verified
    /// write look like a silent rejection.
    Sequence(Vec<Reply>),
    /// The device went away.
    Disconnect,
    /// Fail with this error, on IN and OUT alike: a link's own failures
    /// (`Locked`, `Denied`, `TimedOut` after the transport's own retries,
    /// `Remote`, ...).
    Error(TransportError),
}

/// One recorded exchange, for asserting what the code under test actually sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exchange {
    pub direction: Direction,
    pub opcode: u8,
    pub value: u16,
    pub payload: Vec<u8>,
    // -- WP PGWIRE --
    /// `wIndex`: the vendor interface (2) for a page-0 request, the page in
    /// the high byte for a paged one ([`CmdId::w_index`]).
    pub index: u16,
    /// `wLength`: the length asked for an IN transfer, the payload's for an OUT.
    pub length: u16,
}

// -- WP PGWIRE --
impl Exchange {
    /// The page this request named: the high byte of `wIndex`.
    pub fn page(&self) -> u8 {
        self.index.to_be_bytes()[0]
    }

    /// The eight SETUP bytes this exchange went out as on USB
    /// (`bmRequestType`, `bRequest`, `wValue`, `wIndex`, `wLength`, little
    /// endian), for comparing with a protocol document's own.
    pub fn setup(&self) -> [u8; 8] {
        let request_type = match self.direction {
            Direction::In => 0xC1,
            Direction::Out => 0x41,
        };
        let [v0, v1] = self.value.to_le_bytes();
        let [i0, i1] = self.index.to_le_bytes();
        let [l0, l1] = self.length.to_le_bytes();
        [request_type, self.opcode, v0, v1, i0, i1, l0, l1]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
}

/// A shared handle to the recorded exchanges.
///
/// The log is shared rather than owned so a test can keep watching it after the
/// transport has been moved into a `Session`, which is exactly when asserting on
/// the wire matters most.
pub type LogHandle = Arc<Mutex<Vec<Exchange>>>;

#[derive(Default)]
pub struct MockTransport {
    /// Keyed by command id: a page-0 opcode is its own id (WP PGWIRE).
    replies: HashMap<u16, Reply>,
    /// What an opcode with no script answers. `None` stalls, as the firmware
    /// does for an opcode it does not implement.
    fallback: Option<Reply>,
    log: LogHandle,
    descriptor: Option<DeviceDescriptor>,
    max_transfer: Option<usize>,
    notify: NotifyQueue,
    notify_pace: Option<Duration>,
    // -- WP PGWIRE --
    /// [`Transport::carries_pages`]: off by default, so a mock scripted for
    /// 1.1.6 is never probed by `paged::open`.
    pages: bool,
}

impl MockTransport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reply(mut self, opcode: u8, reply: Reply) -> Self {
        self.replies.insert(u16::from(opcode), reply);
        self
    }

    // -- WP PGWIRE --
    /// A link that carries pages ([`Transport::carries_pages`]), as USB and
    /// USB MIDI do: `paged::open` probes it.
    pub fn carrying_pages(mut self) -> Self {
        self.pages = true;
        self
    }

    /// What a paged request for `id` answers. An unscripted paged id stalls;
    /// [`MockTransport::answering_everything`] answers page 0 only.
    pub fn paged(mut self, id: CmdId, reply: Reply) -> Self {
        self.replies.insert(id.0, reply);
        self
    }

    /// Answer an opcode with fixed bytes.
    pub fn data(self, opcode: u8, bytes: impl Into<Vec<u8>>) -> Self {
        self.reply(opcode, Reply::Data(bytes.into()))
    }

    /// Answer an opcode by slicing a larger buffer at the offset in `wValue`,
    /// the way the chunked bulk opcodes behave.
    pub fn window(self, opcode: u8, bytes: impl Into<Vec<u8>>) -> Self {
        self.reply(opcode, Reply::Window(bytes.into()))
    }

    /// Answer every unscripted opcode with these bytes.
    ///
    /// For a test that cares about one part of a long sequence: without it,
    /// every unscripted read stalls and the retry backoff turns a millisecond
    /// of assertions into tens of seconds of sleeping.
    pub fn answering_everything(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.fallback = Some(Reply::Data(bytes.into()));
        self
    }

    /// Constrain the transfer size, to exercise the chunked path that Windows
    /// forces on us without needing Windows.
    /// Pace the notification endpoint as beta4 firmware does: an idle
    /// packet only after `pace` without anything else to send, so an idle
    /// read blocks that long (usb_audio.c:1064-1080).
    pub fn with_notification_pace(mut self, pace: Duration) -> Self {
        self.notify_pace = Some(pace);
        self
    }

    pub fn with_max_transfer(mut self, max: usize) -> Self {
        self.max_transfer = Some(max);
        self
    }

    pub fn with_descriptor(mut self, d: DeviceDescriptor) -> Self {
        self.descriptor = Some(d);
        self
    }

    /// A handle that keeps working once this transport has been handed away.
    pub fn log_handle(&self) -> LogHandle {
        Arc::clone(&self.log)
    }

    /// The notification queue, to push packets into from a test.
    pub fn notify_queue(&self) -> NotifyQueue {
        Arc::clone(&self.notify)
    }

    /// Queue a notification packet for the reader to find.
    pub fn push_notification(&self, packet: impl Into<Vec<u8>>) {
        self.notify.lock().unwrap().push_back(packet.into());
    }

    /// Everything recorded so far.
    pub fn log(&self) -> Vec<Exchange> {
        self.log.lock().unwrap().clone()
    }

    /// Every opcode the code under test touched, in order.
    pub fn opcodes_seen(&self) -> Vec<u8> {
        self.log.lock().unwrap().iter().map(|e| e.opcode).collect()
    }

    /// What an OUT request for command id `key` answers (WP PGWIRE: shared by
    /// the page-0 and the paged OUT).
    fn out_reply(&self, key: u16) -> Result<()> {
        let opcode = CmdId(key).cmd();
        match self.replies.get(&key) {
            Some(Reply::Stall) => Err(TransportError::Stalled { opcode }),
            Some(Reply::Disconnect) => Err(TransportError::Disconnected),
            Some(Reply::Error(e)) => Err(e.clone()),
            _ => Ok(()),
        }
    }

    /// Resolve a reply that carries no state of its own.
    fn resolve_one(reply: &Reply, opcode: u8, value: u16) -> Result<Vec<u8>> {
        match reply {
            Reply::Data(d) => Ok(d.clone()),
            Reply::Window(all) => {
                let start = (value as usize).min(all.len());
                Ok(all[start..].to_vec())
            }
            Reply::Disconnect => Err(TransportError::Disconnected),
            Reply::Error(e) => Err(e.clone()),
            // A stall, and anything that would need its own bookkeeping to
            // nest, which nothing needs yet.
            _ => Err(TransportError::Stalled { opcode }),
        }
    }

    fn resolve(&mut self, opcode: u8, value: u16) -> Result<Vec<u8>> {
        self.resolve_id(u16::from(opcode), value)
    }

    /// [`MockTransport::resolve`] by command id (WP PGWIRE); `opcode` in an
    /// error is the id's command number.
    fn resolve_id(&mut self, key: u16, value: u16) -> Result<Vec<u8>> {
        let opcode = CmdId(key).cmd();
        if !self.replies.contains_key(&key)
            && CmdId(key).is_legacy()
            && let Some(fallback) = self.fallback.clone()
        {
            return Self::resolve_one(&fallback, opcode, value);
        }
        match self.replies.get_mut(&key) {
            None => Err(TransportError::Stalled { opcode }),
            Some(Reply::Stall) => Err(TransportError::Stalled { opcode }),
            Some(Reply::Disconnect) => Err(TransportError::Disconnected),
            Some(Reply::Error(e)) => Err(e.clone()),
            Some(Reply::Data(d)) => Ok(d.clone()),
            Some(Reply::Window(all)) => {
                let start = (value as usize).min(all.len());
                Ok(all[start..].to_vec())
            }
            Some(Reply::Sequence(items)) => {
                let next = match items.len() {
                    0 => return Err(TransportError::Stalled { opcode }),
                    // The last one persists, so a test only has to script the
                    // reads it cares about.
                    1 => items[0].clone(),
                    _ => items.remove(0),
                };
                Self::resolve_one(&next, opcode, value)
            }
            Some(Reply::BusyThen(remaining, inner)) => {
                if *remaining > 0 {
                    *remaining -= 1;
                    Err(TransportError::Stalled { opcode })
                } else {
                    let inner = (**inner).clone();
                    self.replies.insert(key, inner);
                    self.resolve_id(key, value)
                }
            }
        }
    }
}

impl Transport for MockTransport {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        self.log.lock().unwrap().push(Exchange {
            direction: Direction::In,
            opcode,
            value,
            payload: Vec::new(),
            index: VENDOR_INTERFACE,
            length: len,
        });

        let mut data = self.resolve(opcode, value)?;
        // Mirror the real backend: answering fewer bytes than asked is an error,
        // not something to pad over.
        if data.len() < len as usize {
            return Err(TransportError::ShortRead {
                opcode,
                wanted: len as usize,
                got: data.len(),
            });
        }
        data.truncate(len as usize);
        Ok(data)
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> Result<Vec<u8>> {
        self.log.lock().unwrap().push(Exchange {
            direction: Direction::In,
            opcode,
            value,
            payload: Vec::new(),
            index: VENDOR_INTERFACE,
            length: max_len,
        });
        let mut data = self.resolve(opcode, value)?;
        data.truncate(max_len as usize);
        Ok(data)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        self.log.lock().unwrap().push(Exchange {
            direction: Direction::Out,
            opcode,
            value,
            payload: data.to_vec(),
            index: VENDOR_INTERFACE,
            length: u16::try_from(data.len()).unwrap_or(u16::MAX),
        });
        self.out_reply(u16::from(opcode))
    }

    // -- WP PGWIRE --
    fn carries_pages(&self) -> bool {
        self.pages
    }

    fn control_in_paged(&mut self, id: CmdId, value: u16, max_len: u16) -> Result<Vec<u8>> {
        self.log.lock().unwrap().push(Exchange {
            direction: Direction::In,
            opcode: id.cmd(),
            value,
            payload: Vec::new(),
            index: id.w_index(),
            length: max_len,
        });
        let mut data = self.resolve_id(id.0, value)?;
        data.truncate(usize::from(max_len));
        Ok(data)
    }

    fn control_out_paged(&mut self, id: CmdId, value: u16, data: &[u8]) -> Result<()> {
        self.log.lock().unwrap().push(Exchange {
            direction: Direction::Out,
            opcode: id.cmd(),
            value,
            payload: data.to_vec(),
            index: id.w_index(),
            length: u16::try_from(data.len()).unwrap_or(u16::MAX),
        });
        self.out_reply(id.0)
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        static FALLBACK: std::sync::OnceLock<DeviceDescriptor> = std::sync::OnceLock::new();
        self.descriptor.as_ref().unwrap_or_else(|| {
            FALLBACK.get_or_init(|| DeviceDescriptor {
                serial: "MOCK000000000000".into(),
                bus_id: "mock".into(),
                address: 0,
            })
        })
    }

    fn max_transfer(&self) -> usize {
        self.max_transfer.unwrap_or(4096)
    }

    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        Some(Box::new(MockNotifications {
            queue: Arc::clone(&self.notify),
            pace: self.notify_pace,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::with_busy_retry;

    #[test]
    fn answers_scripted_data() {
        let mut t = MockTransport::new().data(0x7F, vec![1, 1, 5, 9]);
        assert_eq!(t.control_in(0x7F, 0, 4).unwrap(), vec![1, 1, 5, 9]);
    }

    #[test]
    fn an_unscripted_opcode_stalls_like_the_firmware() {
        let mut t = MockTransport::new();
        assert!(matches!(
            t.control_in(0xFF, 0, 1),
            Err(TransportError::Stalled { opcode: 0xFF })
        ));
    }

    /// The flash blackout, which is the behaviour most likely to be got wrong.
    #[test]
    fn a_busy_window_recovers_after_backoff() {
        let mut t =
            MockTransport::new().reply(0x90, Reply::BusyThen(2, Box::new(Reply::Data(vec![0x00]))));

        let r = with_busy_retry(crate::TransportKind::Usb, || t.control_in(0x90, 3, 1), 0x90);
        assert_eq!(r.unwrap(), vec![0x00]);
        // Two refusals plus the successful third.
        assert_eq!(t.opcodes_seen().len(), 3);
    }

    #[test]
    fn records_what_was_actually_sent() {
        let mut t = MockTransport::new();
        let _ = t.control_out(0x42, 0, &[1, 2, 3]);
        assert_eq!(
            t.log()[0],
            Exchange {
                direction: Direction::Out,
                opcode: 0x42,
                value: 0,
                payload: vec![1, 2, 3],
                index: 2,
                length: 3,
            }
        );
    }

    /// A reply sized by the device, such as GET_PLATFORM from older firmware,
    /// arrives short and is not an error on the path that expects it.
    #[test]
    fn a_short_answer_is_accepted_where_the_device_sizes_it() {
        let mut t = MockTransport::new().data(0x7F, vec![1, 1, 0x16, 9]);
        assert_eq!(t.control_in_upto(0x7F, 0, 7).unwrap(), vec![1, 1, 0x16, 9]);
        let mut t = MockTransport::new().data(0x7F, vec![1, 1, 0x16, 9, 1, 6, 4, 0xEE]);
        assert_eq!(t.control_in_upto(0x7F, 0, 7).unwrap().len(), 7);
        assert!(
            t.control_in_upto(0x7E, 0, 7).is_err(),
            "a stall is still a stall"
        );
    }

    /// A paced endpoint holds an idle read for the keep-alive interval and
    /// then answers the one-byte idle packet; a queued event is immediate.
    #[test]
    fn a_paced_endpoint_blocks_until_the_keep_alive() {
        let t = MockTransport::new().with_notification_pace(Duration::from_millis(100));
        let mut n = t.notifications().unwrap();
        let start = std::time::Instant::now();
        assert_eq!(n.read(Duration::from_millis(250)).unwrap(), vec![0]);
        assert!(start.elapsed() >= Duration::from_millis(100));

        t.push_notification(vec![2, 3, 0, 1, 1, 0, 0, 0]);
        let start = std::time::Instant::now();
        assert_eq!(n.read(Duration::from_millis(250)).unwrap().len(), 8);
        assert!(start.elapsed() < Duration::from_millis(50));

        // A read shorter than the pace times out empty-handed.
        assert!(n.read(Duration::from_millis(20)).unwrap().is_empty());
    }

    #[test]
    fn a_short_answer_is_an_error_not_silent_padding() {
        let mut t = MockTransport::new().data(0x50, vec![1, 2]);
        assert!(matches!(
            t.control_in(0x50, 9, 26),
            Err(TransportError::ShortRead {
                wanted: 26,
                got: 2,
                ..
            })
        ));
    }

    #[test]
    fn an_error_reply_returns_that_error_both_ways() {
        let mut t = MockTransport::new()
            .reply(0x90, Reply::Error(TransportError::Locked))
            .reply(0x91, Reply::Error(TransportError::Remote("BUSY".into())));
        assert!(matches!(
            t.control_in(0x90, 0, 1),
            Err(TransportError::Locked)
        ));
        assert!(matches!(
            t.control_out(0x90, 0, &[1]),
            Err(TransportError::Locked)
        ));
        assert!(matches!(t.control_in(0x91, 0, 1), Err(TransportError::Remote(m)) if m == "BUSY"));
    }
}
