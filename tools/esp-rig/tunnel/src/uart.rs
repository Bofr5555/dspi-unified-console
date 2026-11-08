//! The UART wire as the DSPi sees it: request frames out of the bytes the
//! bridge sends, the bytes the DSPi sends paced as an 8N1 line paces them,
//! and the faults a real wire adds.
//!
//! Spec: `control_interfaces_spec.md` (Bofr5555/DSPi `bofr-main`) section 3,
//! firmware `uart_control.c`. The frame codec is `dspi_proto::frame`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use dspi_proto::frame::{Frame, FrameError, SYNC, status};

/// Request header bytes after the sync byte: type, bRequest, wValue, wIndex,
/// wLength.
const HEADER: usize = 8;
/// The firmware drops a frame whose next byte is this late
/// (`uart_control.c` `FRAME_TIMEOUT_US`).
pub const FRAME_TIMEOUT: Duration = Duration::from_millis(100);
/// The longest SET payload taken: a bulk `0xA1` image is about 6 KB. A
/// longer announced length is not ours; the parser resyncs.
const MAX_PAYLOAD: usize = 16 * 1024;
/// Type bytes of the two requests.
const TY_SET: u8 = 0x01;
const TY_GET: u8 = 0x02;

/// What the parser made of the bytes so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// A whole request with a good CRC.
    Request(Frame),
    /// A whole request whose CRC failed: the firmware answers `CRC_ERROR`
    /// with the response type for `is_get`, and runs nothing.
    BadCrc { is_get: bool },
}

/// Splits the bridge's byte stream into requests, as `uart_control.c` does:
/// bytes outside a frame are noise until the next `0xA5`; a frame stalled
/// for [`FRAME_TIMEOUT`] is dropped without an answer.
#[derive(Debug, Default)]
pub struct RequestParser {
    /// The frame so far, without its sync byte.
    buf: Vec<u8>,
    in_frame: bool,
    last_byte: Option<Instant>,
    /// When the frame's first byte (its sync) arrived.
    started: Option<Instant>,
    /// Frames dropped by the inter-byte timeout.
    pub timed_out: u64,
    /// Bytes outside any frame.
    pub noise: u64,
}

impl RequestParser {
    /// Whether a frame is part way in: the DSPi holds notifications then.
    pub fn mid_frame(&self) -> bool {
        self.in_frame
    }

    /// When the current or last frame's sync byte arrived.
    pub fn started(&self) -> Option<Instant> {
        self.started
    }

    /// Feed one byte that arrived at `now`.
    pub fn push(&mut self, byte: u8, now: Instant) -> Option<Parsed> {
        if self.in_frame
            && self
                .last_byte
                .is_some_and(|t| now.saturating_duration_since(t) >= FRAME_TIMEOUT)
        {
            self.timed_out += 1;
            self.reset();
        }
        self.last_byte = Some(now);
        if !self.in_frame {
            if byte == SYNC {
                self.in_frame = true;
                self.started = Some(now);
            } else {
                self.noise += 1;
            }
            return None;
        }
        if self.buf.is_empty() && byte != TY_SET && byte != TY_GET {
            // Not a request type (a response type, or the reserved 0x40):
            // noise, resync. A second 0xA5 starts again.
            self.noise += 1;
            if byte != SYNC {
                self.in_frame = false;
            }
            return None;
        }
        self.buf.push(byte);
        if self.buf.len() < HEADER {
            return None;
        }
        let payload = if self.buf[0] == TY_SET {
            usize::from(u16::from_le_bytes([self.buf[6], self.buf[7]]))
        } else {
            0
        };
        if payload > MAX_PAYLOAD {
            self.noise += self.buf.len() as u64;
            self.reset();
            return None;
        }
        if self.buf.len() < HEADER + payload + 2 {
            return None;
        }
        let is_get = self.buf[0] == TY_GET;
        let result = match Frame::decode(&self.buf) {
            Ok(frame) => Some(Parsed::Request(frame)),
            Err(FrameError::Crc { .. }) => Some(Parsed::BadCrc { is_get }),
            Err(_) => None,
        };
        self.reset();
        result
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.in_frame = false;
    }
}

/// The response frame carrying only `status`, of the type that answers a
/// GET or a SET.
pub fn status_reply(is_get: bool, code: u8) -> Frame {
    if is_get {
        Frame::GetResponse {
            status: code,
            payload: Vec::new(),
        }
    } else {
        Frame::SetResponse { status: code }
    }
}

/// `CTRL_STATUS_OK` with nothing else, for a SET.
pub fn set_ok() -> Frame {
    status_reply(false, status::OK)
}

/// How long one byte takes on an 8N1 line: start bit, 8 data bits, stop bit.
pub fn byte_time(baud: u32) -> Duration {
    Duration::from_nanos(10_000_000_000 / u64::from(baud.max(1)))
}

/// The DSPi's transmit line: frames queue in order, and each byte becomes
/// visible to the bridge when its stop bit ends.
#[derive(Debug)]
pub struct Line {
    byte_time: Duration,
    /// When the last queued byte's stop bit ends.
    free_at: Instant,
    pending: VecDeque<(Instant, u8)>,
}

impl Line {
    pub fn new(baud: u32, now: Instant) -> Self {
        Self {
            byte_time: byte_time(baud),
            free_at: now,
            pending: VecDeque::new(),
        }
    }

    /// Queue `bytes` behind whatever is still going out.
    pub fn send(&mut self, bytes: &[u8], now: Instant) {
        let mut t = self.free_at.max(now);
        for &b in bytes {
            t += self.byte_time;
            self.pending.push_back((t, b));
        }
        self.free_at = t;
    }

    /// Nothing queued and the last stop bit has ended.
    pub fn idle(&self, now: Instant) -> bool {
        self.pending.is_empty() && now >= self.free_at
    }

    /// The bytes whose stop bit has ended by `now`, oldest first.
    pub fn due(&mut self, now: Instant, out: &mut Vec<u8>) {
        while let Some(&(t, b)) = self.pending.front() {
            if t > now {
                break;
            }
            out.push(b);
            self.pending.pop_front();
        }
    }

    /// When the next byte is due, if any is queued.
    pub fn next_due(&self) -> Option<Instant> {
        self.pending.front().map(|&(t, _)| t)
    }
}

/// A small deterministic generator (xorshift64*), so a run with the same
/// seed injects the same faults.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// True with probability `p` (0 to 1).
    pub fn chance(&mut self, p: f64) -> bool {
        p > 0.0 && (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// Faults on one direction of the wire, per byte: a flipped bit (noise), a
/// lost byte (an overrun, a glitch).
#[derive(Debug, Clone, Default)]
pub struct Faults {
    pub flip: f64,
    pub drop: f64,
    pub flipped: u64,
    pub dropped: u64,
}

impl Faults {
    /// Pass `bytes` through the faults into `out`.
    pub fn apply(&mut self, rng: &mut Rng, bytes: &[u8], out: &mut Vec<u8>) {
        for &b in bytes {
            if rng.chance(self.drop) {
                self.dropped += 1;
                continue;
            }
            if rng.chance(self.flip) {
                self.flipped += 1;
                out.push(b ^ (1 << rng.below(8)));
            } else {
                out.push(b);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(p: &mut RequestParser, bytes: &[u8], t: Instant) -> Vec<Parsed> {
        bytes.iter().filter_map(|&b| p.push(b, t)).collect()
    }

    #[test]
    fn wp_lk3_a_get_and_a_set_parse_out_of_noise() {
        let t = Instant::now();
        let get = Frame::Get {
            request: 0x7E,
            value: 0,
            index: 0,
            length: 32,
        };
        let set = Frame::Set {
            request: 0x56,
            value: 0,
            index: 0,
            payload: vec![b'x'; 32],
        };
        let mut bytes = vec![0x00, 0xFF];
        bytes.extend(get.encode_uart());
        bytes.push(0x42);
        bytes.extend(set.encode_uart());
        let mut p = RequestParser::default();
        let got = feed(&mut p, &bytes, t);
        assert_eq!(got, vec![Parsed::Request(get), Parsed::Request(set)]);
        assert_eq!(p.noise, 3);
    }

    #[test]
    fn wp_lk3_a_bad_crc_is_answered_with_the_requests_type() {
        let t = Instant::now();
        let mut bytes = Frame::Get {
            request: 0x50,
            value: 9,
            index: 0,
            length: 41,
        }
        .encode_uart();
        let n = bytes.len();
        bytes[n - 1] ^= 0x10;
        let mut p = RequestParser::default();
        assert_eq!(
            feed(&mut p, &bytes, t),
            vec![Parsed::BadCrc { is_get: true }]
        );
    }

    #[test]
    fn wp_lk3_a_stalled_frame_is_dropped_without_an_answer() {
        let t = Instant::now();
        let bytes = Frame::Get {
            request: 0x7F,
            value: 0,
            index: 0,
            length: 7,
        }
        .encode_uart();
        let mut p = RequestParser::default();
        assert!(feed(&mut p, &bytes[..5], t).is_empty());
        // The rest 150 ms later: the firmware dropped the first part, so the
        // tail is noise and no request comes out.
        let later = t + Duration::from_millis(150);
        assert!(feed(&mut p, &bytes[5..], later).is_empty());
        assert_eq!(p.timed_out, 1);
        // A whole frame after that parses again.
        let again = feed(&mut p, &bytes, later);
        assert_eq!(again.len(), 1);
    }

    #[test]
    fn wp_lk3_the_line_paces_ten_bits_a_byte() {
        let t = Instant::now();
        let mut line = Line::new(115_200, t);
        line.send(&[1, 2, 3], t);
        let bt = byte_time(115_200);
        assert_eq!(bt.as_nanos(), 86_805);
        let mut out = Vec::new();
        line.due(t + bt - Duration::from_nanos(1), &mut out);
        assert!(out.is_empty());
        line.due(t + bt * 2, &mut out);
        assert_eq!(out, vec![1, 2]);
        assert!(!line.idle(t + bt * 2));
        // A frame queued now goes out behind the one still on the wire.
        line.send(&[4], t + bt * 2);
        line.due(t + bt * 4, &mut out);
        assert_eq!(out, vec![1, 2, 3, 4]);
        assert!(line.idle(t + bt * 4));
    }

    #[test]
    fn wp_lk3_a_6136_byte_bulk_reply_takes_its_wire_time() {
        // control_interfaces_spec.md 3.7 and the bridge spec's "530 ms at
        // 115200" for the bulk image plus framing.
        let t = Instant::now();
        let mut line = Line::new(115_200, t);
        let reply = Frame::GetResponse {
            status: status::OK,
            payload: vec![0; 6136],
        }
        .encode_uart();
        line.send(&reply, t);
        let end = line.next_due().unwrap();
        assert!(end > t);
        let mut out = Vec::new();
        line.due(t + Duration::from_millis(535), &mut out);
        assert_eq!(out.len(), reply.len());
        let mut fast = Line::new(1_000_000, t);
        fast.send(&reply, t);
        let mut out = Vec::new();
        fast.due(t + Duration::from_millis(62), &mut out);
        assert_eq!(out.len(), reply.len());
    }

    #[test]
    fn wp_lk3_faults_are_repeatable_and_counted() {
        let bytes = vec![0x55u8; 10_000];
        let run = || {
            let mut rng = Rng::new(7);
            let mut f = Faults {
                flip: 0.01,
                drop: 0.01,
                ..Faults::default()
            };
            let mut out = Vec::new();
            f.apply(&mut rng, &bytes, &mut out);
            (out, f.flipped, f.dropped)
        };
        let (a, flipped, dropped) = run();
        assert_eq!(run().0, a);
        assert!((50..200).contains(&flipped), "{flipped}");
        assert!((50..200).contains(&dropped), "{dropped}");
        assert_eq!(a.len() as u64, 10_000 - dropped);
        let mut rng = Rng::new(7);
        let mut none = Faults::default();
        let mut out = Vec::new();
        none.apply(&mut rng, &bytes, &mut out);
        assert_eq!(out, bytes);
    }
}
