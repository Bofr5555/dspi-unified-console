//! The control frame shared by the UART link and the USB MIDI channel, and
//! the SysEx envelope the MIDI channel puts around it.
//!
//! Pure codecs, no I/O: a transport turns a request into bytes with
//! [`encode_sysex`] (or [`Frame::encode_uart`]) and feeds whatever arrives to a
//! [`SysexDecoder`], which hands back whole messages.
//!
//! Specs (Bofr5555/DSPi `bofr-main`):
//! - `Documentation/Features/control_interfaces_spec.md` section 3: the
//!   frame (`0xA5` sync on UART, type byte, fields, CRC16-CCITT-FALSE over the
//!   type byte through the last payload byte, little-endian);
//! - `Documentation/Features/usb_midi_control_spec.md` section 3: one SysEx
//!   carries one frame without its sync byte,
//!   `F0 <mfr> 'D' 'S' 'P' <ver> <tag> <7-bit packed frame> F7`.
//!
//! The firmware's own codec is `firmware/DSPi/midi_frame.c`; the test vectors
//! in `tests/fixtures/midi/midi_vectors.json` are the ones both use.

/// UART sync byte. The MIDI channel leaves it out.
pub const SYNC: u8 = 0xA5;

/// The SysEx wire format version (the `ver` byte).
pub const SYSEX_VERSION: u8 = 1;

/// The development manufacturer ID (`DSPI_USB_MIDI_MFR_ID` default, the MIDI
/// 1.0 non-commercial ID).
pub const DEFAULT_MFR_ID: &[u8] = &[0x7D];

/// The tag the device uses on notifications.
pub const TAG_NOTIFY: u8 = 0x00;
/// The tag the device uses when the request's header could not be read.
pub const TAG_UNREADABLE: u8 = 0x7F;

/// Largest non-bulk SET payload over UART and MIDI.
pub const MAX_SET_PAYLOAD: usize = 64;

/// The longest SysEx message [`SysexDecoder`] accepts, counted from `F0`
/// through `F7` inclusive. Real-time bytes (`F8`..`FF`) inside a message are
/// not counted: the decoder passes over them and never buffers them. The
/// largest legal message, a whole bulk image in one `GET_RESPONSE`, is about
/// 7030 bytes; anything longer is not ours or is broken, and is dropped whole
/// rather than buffered (PLAN §1.7).
pub const MAX_SYSEX_LEN: usize = 8192;

/// The bytes between `F0` and `F7` that [`MAX_SYSEX_LEN`] leaves room for.
const MAX_SYSEX_BODY: usize = MAX_SYSEX_LEN - 2;

/// `CTRL_STATUS_*` (firmware `vendor_commands.h`): the status byte of a
/// response frame.
pub mod status {
    pub const OK: u8 = 0x00;
    /// Retry the whole request shortly.
    pub const BUSY: u8 = 0x01;
    /// Unknown command or bad parameter (USB answers a STALL).
    pub const ERROR: u8 = 0x02;
    /// USB-only command (interface self-configuration over UART/I2C).
    pub const BLOCKED: u8 = 0x03;
    /// The bulk buffer is busy; retry.
    pub const BULK_LOCKED: u8 = 0x04;
    pub const CRC_ERROR: u8 = 0x05;
    /// A non-bulk payload over 64 bytes.
    pub const OVERSIZE: u8 = 0x06;
    /// Malformed or truncated frame.
    pub const FRAME_ERROR: u8 = 0x07;

    pub fn name(s: u8) -> &'static str {
        match s {
            OK => "OK",
            BUSY => "BUSY",
            ERROR => "ERROR",
            BLOCKED => "BLOCKED",
            BULK_LOCKED => "BULK_LOCKED",
            CRC_ERROR => "CRC_ERROR",
            OVERSIZE => "OVERSIZE",
            FRAME_ERROR => "FRAME_ERROR",
            _ => "unknown status",
        }
    }

    /// Whether the spec says to retry (host guidance, MIDI spec section 5).
    pub fn is_retryable(s: u8) -> bool {
        matches!(s, BUSY | BULK_LOCKED | CRC_ERROR)
    }
}

mod ty {
    pub const SET: u8 = 0x01;
    pub const GET: u8 = 0x02;
    pub const SET_RESPONSE: u8 = 0x81;
    pub const GET_RESPONSE: u8 = 0x82;
    pub const NOTIFICATION: u8 = 0x40;
}

/// CRC16-CCITT-FALSE: polynomial 0x1021, initial value 0xFFFF, no reflection,
/// no final XOR. Check value 0x29B1 over `"123456789"`.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// One control frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// Host to device, OUT: `bRequest`, `wValue`, `wIndex` and the payload.
    Set {
        request: u8,
        value: u16,
        index: u16,
        payload: Vec<u8>,
    },
    /// Host to device, IN: `wLength` is the most the host wants back.
    Get {
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    },
    SetResponse {
        status: u8,
    },
    GetResponse {
        status: u8,
        payload: Vec<u8>,
    },
    /// A v2 notify packet, exactly as the USB endpoint would deliver it.
    Notification {
        packet: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("frame too short")]
    Truncated,
    #[error("unknown frame type 0x{0:02X}")]
    UnknownType(u8),
    #[error("frame length fields disagree with its size")]
    BadLength,
    #[error("CRC mismatch: frame says 0x{stated:04X}, computed 0x{computed:04X}")]
    Crc { stated: u16, computed: u16 },
    #[error("malformed 7-bit packing")]
    Packing,
    #[error("SysEx header: {0}")]
    Header(&'static str),
    /// A payload longer than the frame's 16-bit length field can state.
    #[error("payload of {len} bytes does not fit the 16-bit length field")]
    TooLong { len: usize },
}

impl Frame {
    /// The largest payload a frame can carry: its length field is 16 bits.
    pub const MAX_PAYLOAD: usize = u16::MAX as usize;

    /// Whether the frame can be encoded: its payload (or notify packet) is at
    /// most [`Frame::MAX_PAYLOAD`] bytes. `Err(FrameError::TooLong)`
    /// otherwise. A frame that came out of [`Frame::decode`] always passes.
    pub fn check_len(&self) -> Result<(), FrameError> {
        let len = match self {
            Frame::Set { payload, .. } | Frame::GetResponse { payload, .. } => payload.len(),
            Frame::Notification { packet } => packet.len(),
            Frame::Get { .. } | Frame::SetResponse { .. } => 0,
        };
        if len > Self::MAX_PAYLOAD {
            Err(FrameError::TooLong { len })
        } else {
            Ok(())
        }
    }

    /// The frame from the type byte through the CRC (the MIDI form).
    ///
    /// Infallible by design; it assumes [`Frame::check_len`] passed. A longer
    /// payload is not refused here: its length field would wrap, and the
    /// frame would not decode. Callers building frames from data they do not
    /// control call `check_len` first.
    pub fn encode(&self) -> Vec<u8> {
        let mut f = Vec::new();
        match self {
            Frame::Set {
                request,
                value,
                index,
                payload,
            } => {
                f.extend_from_slice(&[ty::SET, *request]);
                f.extend_from_slice(&value.to_le_bytes());
                f.extend_from_slice(&index.to_le_bytes());
                f.extend_from_slice(&(payload.len() as u16).to_le_bytes());
                f.extend_from_slice(payload);
            }
            Frame::Get {
                request,
                value,
                index,
                length,
            } => {
                f.extend_from_slice(&[ty::GET, *request]);
                f.extend_from_slice(&value.to_le_bytes());
                f.extend_from_slice(&index.to_le_bytes());
                f.extend_from_slice(&length.to_le_bytes());
            }
            Frame::SetResponse { status } => {
                f.extend_from_slice(&[ty::SET_RESPONSE, *status, 0, 0])
            }
            Frame::GetResponse { status, payload } => {
                f.extend_from_slice(&[ty::GET_RESPONSE, *status]);
                f.extend_from_slice(&(payload.len() as u16).to_le_bytes());
                f.extend_from_slice(payload);
            }
            Frame::Notification { packet } => {
                f.extend_from_slice(&[ty::NOTIFICATION, 0]);
                f.extend_from_slice(&(packet.len() as u16).to_le_bytes());
                f.extend_from_slice(packet);
            }
        }
        let crc = crc16(&f);
        f.extend_from_slice(&crc.to_le_bytes());
        f
    }

    /// The UART form: [`SYNC`] then [`Frame::encode`]. Same contract as
    /// `encode`: call [`Frame::check_len`] first.
    pub fn encode_uart(&self) -> Vec<u8> {
        let mut v = vec![SYNC];
        v.extend(self.encode());
        v
    }

    /// Parse a whole frame (type byte through CRC, no sync byte).
    pub fn decode(f: &[u8]) -> Result<Frame, FrameError> {
        if f.len() < 3 {
            return Err(FrameError::Truncated);
        }
        let (body, crc) = f.split_at(f.len() - 2);
        let stated = u16::from_le_bytes([crc[0], crc[1]]);
        let le = |i: usize| u16::from_le_bytes([body[i], body[i + 1]]);
        let need = |n: usize| {
            if body.len() < n {
                Err(FrameError::Truncated)
            } else {
                Ok(())
            }
        };
        let frame = match body[0] {
            ty::SET => {
                need(8)?;
                let len = le(6) as usize;
                if body.len() != 8 + len {
                    return Err(FrameError::BadLength);
                }
                Frame::Set {
                    request: body[1],
                    value: le(2),
                    index: le(4),
                    payload: body[8..].to_vec(),
                }
            }
            ty::GET => {
                need(8)?;
                if body.len() != 8 {
                    return Err(FrameError::BadLength);
                }
                Frame::Get {
                    request: body[1],
                    value: le(2),
                    index: le(4),
                    length: le(6),
                }
            }
            ty::SET_RESPONSE => {
                need(4)?;
                if body.len() != 4 {
                    return Err(FrameError::BadLength);
                }
                Frame::SetResponse { status: body[1] }
            }
            ty::GET_RESPONSE | ty::NOTIFICATION => {
                need(4)?;
                let len = le(2) as usize;
                if body.len() != 4 + len {
                    return Err(FrameError::BadLength);
                }
                let data = body[4..].to_vec();
                if body[0] == ty::GET_RESPONSE {
                    Frame::GetResponse {
                        status: body[1],
                        payload: data,
                    }
                } else {
                    Frame::Notification { packet: data }
                }
            }
            other => return Err(FrameError::UnknownType(other)),
        };
        let computed = crc16(body);
        if computed != stated {
            return Err(FrameError::Crc { stated, computed });
        }
        Ok(frame)
    }
}

/// Pack 8-bit bytes into 7-bit SysEx data: each group of up to 7 bytes
/// becomes an MSB byte (bit *i* = bit 7 of byte *i*) followed by the low 7
/// bits of each byte.
pub fn pack7(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 7 * 8 + 8);
    for group in data.chunks(7) {
        let msb = group
            .iter()
            .enumerate()
            .fold(0u8, |m, (i, b)| m | ((b >> 7) << i));
        out.push(msb);
        out.extend(group.iter().map(|b| b & 0x7F));
    }
    out
}

/// Inverse of [`pack7`]. `None` for a lone MSB byte, a byte with bit 7 set,
/// or an MSB bit set for a byte that is not there.
pub fn unpack7(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len());
    for group in data.chunks(8) {
        if group.len() < 2 || group.iter().any(|b| b & 0x80 != 0) {
            return None;
        }
        let msb = group[0];
        let n = group.len() - 1;
        if msb >> n != 0 {
            return None;
        }
        out.extend(
            group[1..]
                .iter()
                .enumerate()
                .map(|(i, b)| b | (((msb >> i) & 1) << 7)),
        );
    }
    Some(out)
}

/// One SysEx message carrying `frame`, with the host's (or device's) `tag`.
///
/// Same contract as [`Frame::encode`]: the caller checks
/// [`Frame::check_len`] first, since an over-long payload would wrap the
/// frame's length field.
pub fn encode_sysex(mfr: &[u8], tag: u8, frame: &Frame) -> Vec<u8> {
    let packed = pack7(&frame.encode());
    let mut v = Vec::with_capacity(packed.len() + 10);
    v.push(0xF0);
    v.extend_from_slice(mfr);
    v.extend_from_slice(b"DSP");
    v.push(SYSEX_VERSION);
    v.push(tag);
    v.extend(packed);
    v.push(0xF7);
    v
}

/// A message the decoder recognised as ours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub tag: u8,
    /// The frame, or why it could not be read. A broken message that matched
    /// our header is still reported, so the request in flight can be failed
    /// at once instead of timing out.
    pub frame: Result<Frame, FrameError>,
}

/// Collects SysEx messages out of a raw MIDI byte stream.
///
/// Ignores everything that is not ours: other manufacturers' SysEx, other
/// products under the same ID, channel messages, real-time bytes (also inside
/// a message) and stray `F7`s. Any other status byte inside a message ends
/// it; `F0` also starts a new one.
///
/// A message longer than [`MAX_SYSEX_LEN`] is dropped whole: the decoder
/// stops buffering at the cap, ignores the rest, and resynchronises on the
/// next `F0`. Its memory use is therefore bounded whatever the stream holds.
#[derive(Debug, Clone)]
pub struct SysexDecoder {
    mfr: Vec<u8>,
    buf: Vec<u8>,
    in_sysex: bool,
}

impl SysexDecoder {
    pub fn new(mfr: &[u8]) -> Self {
        Self {
            mfr: mfr.to_vec(),
            buf: Vec::new(),
            in_sysex: false,
        }
    }

    /// Feed bytes as they arrive; returns the messages they completed.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Message> {
        let mut out = Vec::new();
        for &b in bytes {
            match b {
                0xF8..=0xFF => {} // real-time: transparent
                0xF0 => {
                    self.buf.clear();
                    self.in_sysex = true;
                }
                0xF7 => {
                    if self.in_sysex {
                        self.in_sysex = false;
                        if let Some(m) = self.finish() {
                            out.push(m);
                        }
                    }
                }
                0x80..=0xF6 => {
                    // Another status byte aborts the message.
                    self.in_sysex = false;
                    self.buf.clear();
                }
                _ => {
                    if self.in_sysex {
                        if self.buf.len() < MAX_SYSEX_BODY {
                            self.buf.push(b);
                        } else {
                            // Over the cap: drop the message, wait for an F0.
                            self.in_sysex = false;
                            self.buf.clear();
                        }
                    }
                }
            }
        }
        out
    }

    fn finish(&mut self) -> Option<Message> {
        let b = std::mem::take(&mut self.buf);
        let m = self.mfr.len();
        // Not ours: another ID or another product.
        if b.len() < m + 3 || b[..m] != self.mfr[..] || &b[m..m + 3] != b"DSP" {
            return None;
        }
        let rest = &b[m + 3..];
        if rest.len() < 2 {
            return Some(Message {
                tag: TAG_UNREADABLE,
                frame: Err(FrameError::Header("truncated")),
            });
        }
        let (ver, tag) = (rest[0], rest[1]);
        if ver != SYSEX_VERSION {
            return Some(Message {
                tag,
                frame: Err(FrameError::Header("unknown version")),
            });
        }
        let frame = unpack7(&rest[2..])
            .ok_or(FrameError::Packing)
            .and_then(|f| Frame::decode(&f));
        Some(Message { tag, frame })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_check_value() {
        assert_eq!(crc16(b"123456789"), 0x29B1);
    }

    #[test]
    fn packing_matches_the_spec_example() {
        let d = [0x81, 0x02, 0x83, 0x04, 0x85, 0x06, 0x87, 0xFF];
        let p = pack7(&d);
        assert_eq!(p, [0x55, 1, 2, 3, 4, 5, 6, 7, 0x01, 0x7F]);
        assert_eq!(unpack7(&p).unwrap(), d);
    }

    #[test]
    fn packing_round_trips_every_length() {
        for n in (0..300).chain([6146, 6147]) {
            let d: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            let p = pack7(&d);
            assert_eq!(p.len(), 8 * (n / 7) + if n % 7 > 0 { n % 7 + 1 } else { 0 });
            assert_eq!(unpack7(&p).unwrap(), d, "length {n}");
        }
    }

    #[test]
    fn bad_packing_is_refused() {
        assert_eq!(unpack7(&[0x00]), None, "lone MSB byte");
        assert_eq!(unpack7(&[0x02, 0x10]), None, "MSB bit for a missing byte");
    }

    #[test]
    fn the_worked_example_encodes() {
        let get = Frame::Get {
            request: 0x7F,
            value: 0,
            index: 0,
            length: 0,
        };
        assert_eq!(get.encode(), [0x02, 0x7F, 0, 0, 0, 0, 0, 0, 0x10, 0xB0]);
        assert_eq!(
            encode_sysex(DEFAULT_MFR_ID, 1, &get),
            [
                0xF0, 0x7D, 0x44, 0x53, 0x50, 0x01, 0x01, 0x00, 0x02, 0x7F, 0, 0, 0, 0, 0, 0x04,
                0x00, 0x10, 0x30, 0xF7
            ]
        );
    }

    #[test]
    fn foreign_and_noise_are_ignored() {
        let mut d = SysexDecoder::new(DEFAULT_MFR_ID);
        let ours = encode_sysex(DEFAULT_MFR_ID, 2, &Frame::SetResponse { status: 0 });
        let mut stream = vec![0x90, 0x40, 0x7F, 0xF7, 0xF8]; // note on, stray F7, clock
        stream.extend([0xF0, 0x43, 0x10, 0x4C, 0xF7]); // Yamaha
        stream.extend([0xF0, 0x7D, b'X', b'Y', b'Z', 1, 1, 0, 0xF7]); // other product
        stream.extend(&ours[..5]);
        stream.push(0xFE); // active sensing inside the message
        stream.extend(&ours[5..]);
        let got = d.feed(&stream);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].tag, 2);
        assert_eq!(got[0].frame, Ok(Frame::SetResponse { status: 0 }));
    }

    #[test]
    fn a_message_split_anywhere_still_decodes() {
        let msg = encode_sysex(
            DEFAULT_MFR_ID,
            9,
            &Frame::GetResponse {
                status: 0,
                payload: (0..200).map(|i| i as u8).collect(),
            },
        );
        for piece in 1..40 {
            let mut d = SysexDecoder::new(DEFAULT_MFR_ID);
            let mut got = Vec::new();
            for c in msg.chunks(piece) {
                got.extend(d.feed(c));
            }
            assert_eq!(got.len(), 1, "piece size {piece}");
            assert!(got[0].frame.is_ok());
        }
    }

    #[test]
    fn a_status_byte_aborts_and_f0_restarts() {
        let mut d = SysexDecoder::new(DEFAULT_MFR_ID);
        let ok = encode_sysex(DEFAULT_MFR_ID, 3, &Frame::SetResponse { status: 0 });
        let mut s = ok[..8].to_vec();
        s.push(0xF2); // song position: aborts
        s.extend(&ok[8..]);
        assert!(d.feed(&s).is_empty());
        let mut s = ok[..8].to_vec();
        s.extend(&ok); // F0 inside: restarts
        assert_eq!(d.feed(&s).len(), 1);
    }

    #[test]
    fn crc_and_version_errors_are_reported_with_the_tag() {
        let mut d = SysexDecoder::new(DEFAULT_MFR_ID);
        let mut f = Frame::SetResponse { status: 0 }.encode();
        f[1] = 2; // status changed after the CRC was computed
        let mut m = vec![0xF0, 0x7D, b'D', b'S', b'P', 1, 4];
        m.extend(pack7(&f));
        m.push(0xF7);
        let got = d.feed(&m);
        assert_eq!(got[0].tag, 4);
        assert!(matches!(got[0].frame, Err(FrameError::Crc { .. })));
        m[5] = 2;
        assert!(matches!(d.feed(&m)[0].frame, Err(FrameError::Header(_))));
    }
}

/// WP P1: the decoder's size cap (PLAN §1.7, gate 1 S-d).
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p1 {
    use super::*;

    /// `F0 7D 'D' 'S' 'P' 01 05` then `body` 7-bit bytes, then `F7`:
    /// `7 + body + 1` bytes in all.
    fn ours_with_body(body: usize) -> Vec<u8> {
        let mut m = vec![0xF0, 0x7D, b'D', b'S', b'P', SYSEX_VERSION, 5];
        m.extend((0..body).map(|i| (i % 0x80) as u8));
        m.push(0xF7);
        m
    }

    #[test]
    fn wp_p1_a_message_at_the_cap_is_kept_one_over_is_dropped() {
        let at_cap = ours_with_body(MAX_SYSEX_LEN - 8);
        assert_eq!(at_cap.len(), MAX_SYSEX_LEN);
        let mut d = SysexDecoder::new(DEFAULT_MFR_ID);
        let got = d.feed(&at_cap);
        assert_eq!(got.len(), 1, "a message of exactly the cap is reported");
        assert_eq!(got[0].tag, 5);

        let over = ours_with_body(MAX_SYSEX_LEN - 7);
        assert_eq!(over.len(), MAX_SYSEX_LEN + 1);
        assert!(d.feed(&over).is_empty(), "one byte over the cap is dropped");
    }

    #[test]
    fn wp_p1_an_oversized_message_is_not_buffered_and_resyncs_on_f0() {
        let mut d = SysexDecoder::new(DEFAULT_MFR_ID);
        let mut s = vec![0xF0];
        s.extend(std::iter::repeat_n(0x11, 100_000));
        assert!(d.feed(&s).is_empty());
        assert!(d.buf.len() <= MAX_SYSEX_BODY, "buffer stays bounded");
        assert!(!d.in_sysex, "the message was abandoned");
        // The rest of the oversized message, then its F7: nothing.
        assert!(d.feed(&[0x22; 50]).is_empty());
        assert!(d.feed(&[0xF7]).is_empty());
        // The next message is found.
        let ok = encode_sysex(DEFAULT_MFR_ID, 6, &Frame::SetResponse { status: 0 });
        let got = d.feed(&ok);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].frame, Ok(Frame::SetResponse { status: 0 }));
    }

    #[test]
    fn wp_p1_check_len_refuses_a_payload_the_length_field_cannot_state() {
        let max = Frame::MAX_PAYLOAD;
        let ok = [
            Frame::Set {
                request: 1,
                value: 0,
                index: 0,
                payload: vec![0; max],
            },
            Frame::GetResponse {
                status: 0,
                payload: vec![0; max],
            },
            Frame::Notification {
                packet: vec![0; max],
            },
            Frame::Get {
                request: 1,
                value: 0,
                index: 0,
                length: u16::MAX,
            },
            Frame::SetResponse { status: 0 },
        ];
        for f in &ok {
            assert_eq!(f.check_len(), Ok(()));
            assert_eq!(
                Frame::decode(&f.encode()).as_ref(),
                Ok(f),
                "the limit round-trips"
            );
        }
        let too_long = [
            Frame::Set {
                request: 1,
                value: 0,
                index: 0,
                payload: vec![0; max + 1],
            },
            Frame::GetResponse {
                status: 0,
                payload: vec![0; max + 1],
            },
            Frame::Notification {
                packet: vec![0; max + 1],
            },
        ];
        for f in &too_long {
            assert_eq!(f.check_len(), Err(FrameError::TooLong { len: max + 1 }));
            // What check_len guards against: the length field wraps.
            assert!(Frame::decode(&f.encode()).is_err());
        }
    }

    #[test]
    fn wp_p1_the_largest_legal_message_fits_under_the_cap() {
        // A whole bulk image in one GET_RESPONSE, with a 3-byte manufacturer id.
        let mfr = [0x00, 0x21, 0x7E];
        let frame = Frame::GetResponse {
            status: 0,
            payload: vec![0xA5; crate::generated::BULK_SIZE],
        };
        let m = encode_sysex(&mfr, 9, &frame);
        assert!(m.len() <= MAX_SYSEX_LEN, "{} bytes", m.len());
        let got = SysexDecoder::new(&mfr).feed(&m);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].frame, Ok(frame));
    }
}
