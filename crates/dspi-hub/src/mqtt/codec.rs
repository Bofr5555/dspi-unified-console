//! MQTT 3.1.1 packets, encoded and decoded without I/O (WP P2MQC).
//!
//! Only what the hub needs (PLAN-P2 OD-P2c): CONNECT with a will, CONNACK, PUBLISH at
//! QoS 0 and 1 with PUBACK, SUBSCRIBE and SUBACK, PINGREQ and PINGRESP, DISCONNECT.
//! Anything else a broker sends is a typed [`DecodeError::Unsupported`], never a panic.
//!
//! A broker is network data. [`decode`] takes the most a packet may be and refuses a
//! longer one from its fixed header alone, before its bytes arrive, so a reader never
//! buffers more than that limit; every length inside the packet is checked against
//! what is left of it.
//!
//! Section numbers are those of the OASIS MQTT 3.1.1 standard.

use std::fmt;

/// The largest remaining length the protocol can express (section 2.2.3).
pub const MAX_REMAINING_LENGTH: usize = 268_435_455;

/// The largest string or binary field: its length is a 16-bit prefix (section 1.5.3).
const MAX_FIELD: usize = 65_535;

/// The quality of service the hub uses. QoS 2 is not part of the hub's client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum QoS {
    /// At most once: no acknowledgement.
    AtMostOnce,
    /// At least once: the receiver answers PUBACK.
    AtLeastOnce,
}

impl QoS {
    fn bits(self) -> u8 {
        match self {
            QoS::AtMostOnce => 0,
            QoS::AtLeastOnce => 1,
        }
    }

    fn from_bits(bits: u8) -> Result<QoS, DecodeError> {
        match bits {
            0 => Ok(QoS::AtMostOnce),
            1 => Ok(QoS::AtLeastOnce),
            2 => Err(DecodeError::Unsupported("QoS 2")),
            _ => Err(DecodeError::Malformed("QoS 3")),
        }
    }
}

/// The message a broker publishes for a client that goes away without a DISCONNECT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Will {
    /// Its topic: a topic name, no wildcards.
    pub topic: String,
    /// Its payload.
    pub payload: Vec<u8>,
    /// Its quality of service.
    pub qos: QoS,
    /// Whether the broker keeps it as the topic's retained message.
    pub retain: bool,
}

/// CONNECT (section 3.1). Its `Debug` never shows the password.
#[derive(Clone, PartialEq, Eq)]
pub struct Connect {
    /// The client identifier.
    pub client_id: String,
    /// Keep-alive in seconds; 0 turns it off.
    pub keep_alive: u16,
    /// Start a new session, forgetting any the broker kept.
    pub clean_session: bool,
    /// The last will.
    pub will: Option<Will>,
    /// The user name.
    pub username: Option<String>,
    /// The password. MQTT 3.1.1 allows one only with a user name (section 3.1.2.9).
    pub password: Option<Vec<u8>>,
}

impl fmt::Debug for Connect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connect")
            .field("client_id", &self.client_id)
            .field("keep_alive", &self.keep_alive)
            .field("clean_session", &self.clean_session)
            .field("will", &self.will)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "set"))
            .finish()
    }
}

/// PUBLISH (section 3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publish {
    /// The topic name: no wildcards, not empty.
    pub topic: String,
    /// The payload.
    pub payload: Vec<u8>,
    /// Its quality of service.
    pub qos: QoS,
    /// Retain flag.
    pub retain: bool,
    /// A redelivery of a QoS 1 message. Always false at QoS 0.
    pub dup: bool,
    /// Packet identifier: 0 at QoS 0, never 0 at QoS 1.
    pub packet_id: u16,
}

/// One MQTT control packet the hub's client sends or receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    /// CONNECT.
    Connect(Connect),
    /// CONNACK: `session_present` and the return code (0 accepted; see [`connack_reason`]).
    Connack {
        /// The broker kept a session for this client.
        session_present: bool,
        /// 0 is accepted; 1 to 5 are the refusals of section 3.2.2.3.
        return_code: u8,
    },
    /// PUBLISH.
    Publish(Publish),
    /// PUBACK for a QoS 1 PUBLISH.
    Puback {
        /// The PUBLISH's packet identifier.
        packet_id: u16,
    },
    /// SUBSCRIBE: one or more topic filters with the QoS asked for.
    Subscribe {
        /// Packet identifier, not 0.
        packet_id: u16,
        /// Filters, at least one.
        filters: Vec<(String, QoS)>,
    },
    /// SUBACK: a return code per filter (0, 1, 2 granted; 0x80 refused).
    Suback {
        /// The SUBSCRIBE's packet identifier.
        packet_id: u16,
        /// One code per filter, in order.
        return_codes: Vec<u8>,
    },
    /// PINGREQ.
    Pingreq,
    /// PINGRESP.
    Pingresp,
    /// DISCONNECT.
    Disconnect,
}

/// What a CONNACK return code means (section 3.2.2.3).
pub fn connack_reason(code: u8) -> &'static str {
    match code {
        0 => "accepted",
        1 => "unacceptable protocol version",
        2 => "client identifier rejected",
        3 => "server unavailable",
        4 => "bad user name or password",
        5 => "not authorised",
        _ => "unknown return code",
    }
}

/// Why bytes are not a packet the hub takes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// The packet's fixed header says it is longer than the limit.
    #[error("packet of {len} bytes is over the limit of {max}")]
    TooLarge {
        /// Its whole length, fixed header included.
        len: usize,
        /// The limit.
        max: usize,
    },
    /// Not well-formed MQTT 3.1.1.
    #[error("malformed packet: {0}")]
    Malformed(&'static str),
    /// Valid MQTT, but not a packet or feature the hub's client handles.
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
}

/// Why a packet cannot be encoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    /// A string or binary field is over 65 535 bytes, or the packet over the protocol's limit.
    #[error("too long: {0}")]
    TooLong(&'static str),
    /// A field breaks a rule of the protocol (an empty topic, a wildcard in a topic name,
    /// packet identifier 0 at QoS 1, a password without a user name, ...).
    #[error("invalid: {0}")]
    Invalid(&'static str),
}

/// Whether `topic` is a valid topic name to publish to: not empty, no wildcards, no NUL.
pub fn valid_topic(topic: &str) -> bool {
    !topic.is_empty() && topic.len() <= MAX_FIELD && !topic.contains(['+', '#', '\0'])
}

/// Whether `filter` is a valid topic filter: not empty, no NUL, `+` alone in its level,
/// `#` alone in the last level (section 4.7.1).
pub fn valid_filter(filter: &str) -> bool {
    if filter.is_empty() || filter.len() > MAX_FIELD || filter.contains('\0') {
        return false;
    }
    let mut levels = filter.split('/').peekable();
    while let Some(level) = levels.next() {
        let last = levels.peek().is_none();
        if level.contains('#') && (level != "#" || !last) {
            return false;
        }
        if level.contains('+') && level != "+" {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------- encoding

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put_bin(out: &mut Vec<u8>, b: &[u8], what: &'static str) -> Result<(), EncodeError> {
    let len = u16::try_from(b.len()).map_err(|_| EncodeError::TooLong(what))?;
    put_u16(out, len);
    out.extend_from_slice(b);
    Ok(())
}

fn put_str(out: &mut Vec<u8>, s: &str, what: &'static str) -> Result<(), EncodeError> {
    if s.contains('\0') {
        return Err(EncodeError::Invalid(what));
    }
    put_bin(out, s.as_bytes(), what)
}

/// Encodes one packet into its bytes.
pub fn encode(packet: &Packet) -> Result<Vec<u8>, EncodeError> {
    let mut body = Vec::new();
    let first: u8 = match packet {
        Packet::Connect(c) => {
            put_str(&mut body, "MQTT", "protocol name")?;
            body.push(4);
            let mut flags = 0u8;
            if c.clean_session {
                flags |= 0x02;
            }
            if let Some(w) = &c.will {
                if !valid_topic(&w.topic) {
                    return Err(EncodeError::Invalid("will topic"));
                }
                flags |= 0x04 | (w.qos.bits() << 3);
                if w.retain {
                    flags |= 0x20;
                }
            }
            if c.password.is_some() {
                if c.username.is_none() {
                    return Err(EncodeError::Invalid("password without a user name"));
                }
                flags |= 0x40;
            }
            if c.username.is_some() {
                flags |= 0x80;
            }
            body.push(flags);
            put_u16(&mut body, c.keep_alive);
            put_str(&mut body, &c.client_id, "client identifier")?;
            if let Some(w) = &c.will {
                put_str(&mut body, &w.topic, "will topic")?;
                put_bin(&mut body, &w.payload, "will payload")?;
            }
            if let Some(u) = &c.username {
                put_str(&mut body, u, "user name")?;
            }
            if let Some(p) = &c.password {
                put_bin(&mut body, p, "password")?;
            }
            0x10
        }
        Packet::Connack {
            session_present,
            return_code,
        } => {
            body.push(u8::from(*session_present));
            body.push(*return_code);
            0x20
        }
        Packet::Publish(p) => {
            if !valid_topic(&p.topic) {
                return Err(EncodeError::Invalid("topic name"));
            }
            put_str(&mut body, &p.topic, "topic name")?;
            match p.qos {
                QoS::AtMostOnce => {
                    if p.packet_id != 0 || p.dup {
                        return Err(EncodeError::Invalid("packet id or dup at QoS 0"));
                    }
                }
                QoS::AtLeastOnce => {
                    if p.packet_id == 0 {
                        return Err(EncodeError::Invalid("packet id 0 at QoS 1"));
                    }
                    put_u16(&mut body, p.packet_id);
                }
            }
            body.extend_from_slice(&p.payload);
            0x30 | (u8::from(p.dup) << 3) | (p.qos.bits() << 1) | u8::from(p.retain)
        }
        Packet::Puback { packet_id } => {
            put_u16(&mut body, *packet_id);
            0x40
        }
        Packet::Subscribe { packet_id, filters } => {
            if *packet_id == 0 {
                return Err(EncodeError::Invalid("packet id 0"));
            }
            if filters.is_empty() {
                return Err(EncodeError::Invalid("no topic filter"));
            }
            put_u16(&mut body, *packet_id);
            for (filter, qos) in filters {
                if !valid_filter(filter) {
                    return Err(EncodeError::Invalid("topic filter"));
                }
                put_str(&mut body, filter, "topic filter")?;
                body.push(qos.bits());
            }
            0x82
        }
        Packet::Suback {
            packet_id,
            return_codes,
        } => {
            if return_codes.is_empty() {
                return Err(EncodeError::Invalid("no return code"));
            }
            put_u16(&mut body, *packet_id);
            body.extend_from_slice(return_codes);
            0x90
        }
        Packet::Pingreq => 0xC0,
        Packet::Pingresp => 0xD0,
        Packet::Disconnect => 0xE0,
    };
    if body.len() > MAX_REMAINING_LENGTH {
        return Err(EncodeError::TooLong("packet"));
    }
    let mut out = Vec::with_capacity(body.len().saturating_add(5));
    out.push(first);
    let mut rem = body.len();
    loop {
        // Seven bits per byte; the cast keeps exactly those bits.
        let mut byte = (rem & 0x7F) as u8;
        rem >>= 7;
        if rem > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if rem == 0 {
            break;
        }
    }
    out.extend_from_slice(&body);
    Ok(out)
}

// ---------------------------------------------------------------- decoding

/// A cursor over one packet's body; every read is checked against what is left.
struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let (head, tail) = self
            .rest
            .split_at_checked(n)
            .ok_or(DecodeError::Malformed("field runs past the packet"))?;
        self.rest = tail;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        let (&b, tail) = self
            .rest
            .split_first()
            .ok_or(DecodeError::Malformed("field runs past the packet"))?;
        self.rest = tail;
        Ok(b)
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        let hi = self.u8()?;
        let lo = self.u8()?;
        Ok(u16::from_be_bytes([hi, lo]))
    }

    fn bin(&mut self) -> Result<&'a [u8], DecodeError> {
        let len = self.u16()?;
        self.take(usize::from(len))
    }

    fn string(&mut self) -> Result<String, DecodeError> {
        let raw = self.bin()?;
        let s =
            std::str::from_utf8(raw).map_err(|_| DecodeError::Malformed("string is not UTF-8"))?;
        if s.contains('\0') {
            return Err(DecodeError::Malformed("string contains NUL"));
        }
        Ok(s.to_owned())
    }

    fn end(&self) -> Result<(), DecodeError> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(DecodeError::Malformed("bytes after the packet's fields"))
        }
    }
}

/// Decodes the first packet in `buf`.
///
/// `Ok(None)`: not a whole packet yet; read more and call again. `Ok(Some((packet, used)))`:
/// one packet and how many bytes of `buf` it took. A packet longer than `max_packet` bytes
/// (fixed header included) is refused as soon as its fixed header is in, so a reader that
/// keeps only an unfinished packet never holds more than `max_packet` bytes of it.
pub fn decode(buf: &[u8], max_packet: usize) -> Result<Option<(Packet, usize)>, DecodeError> {
    let Some((&first, after_first)) = buf.split_first() else {
        return Ok(None);
    };
    // The remaining length: at most four bytes of seven bits (section 2.2.3).
    let mut rem: usize = 0;
    let mut header_len: usize = 1;
    let mut complete = false;
    // Four bytes at most, seven bits each: the value fits the 28 bits of the limit.
    for ((&byte, shift), len) in after_first.iter().zip([0u32, 7, 14, 21]).zip(2usize..) {
        rem |= usize::from(byte & 0x7F).checked_shl(shift).unwrap_or(0);
        header_len = len;
        if byte & 0x80 == 0 {
            complete = true;
            break;
        }
    }
    if !complete {
        if after_first.len() >= 4 {
            return Err(DecodeError::Malformed("remaining length over four bytes"));
        }
        return Ok(None);
    }
    let total = header_len.saturating_add(rem);
    if total > max_packet {
        return Err(DecodeError::TooLarge {
            len: total,
            max: max_packet,
        });
    }
    let Some(packet_bytes) = buf.get(..total) else {
        return Ok(None);
    };
    let body = packet_bytes
        .get(header_len..)
        .ok_or(DecodeError::Malformed("remaining length"))?;
    let packet = decode_body(first, body)?;
    Ok(Some((packet, total)))
}

fn decode_body(first: u8, body: &[u8]) -> Result<Packet, DecodeError> {
    let kind = first >> 4;
    let flags = first & 0x0F;
    let mut r = Reader { rest: body };
    let fixed_flags = |want: u8| {
        if flags == want {
            Ok(())
        } else {
            Err(DecodeError::Malformed("reserved fixed-header flags"))
        }
    };
    let packet = match kind {
        1 => {
            fixed_flags(0)?;
            let name = r.bin()?;
            let level = r.u8()?;
            if name != b"MQTT" || level != 4 {
                return Err(DecodeError::Unsupported("protocol other than MQTT 3.1.1"));
            }
            let cf = r.u8()?;
            if cf & 0x01 != 0 {
                return Err(DecodeError::Malformed("reserved connect flag"));
            }
            let keep_alive = r.u16()?;
            let client_id = r.string()?;
            let will_flag = cf & 0x04 != 0;
            let will_qos = (cf >> 3) & 0x03;
            let will_retain = cf & 0x20 != 0;
            let will = if will_flag {
                let qos = QoS::from_bits(will_qos)?;
                let topic = r.string()?;
                if !valid_topic(&topic) {
                    return Err(DecodeError::Malformed("will topic"));
                }
                let payload = r.bin()?.to_vec();
                Some(Will {
                    topic,
                    payload,
                    qos,
                    retain: will_retain,
                })
            } else {
                if will_qos != 0 || will_retain {
                    return Err(DecodeError::Malformed("will QoS or retain without a will"));
                }
                None
            };
            let has_user = cf & 0x80 != 0;
            let has_pass = cf & 0x40 != 0;
            if has_pass && !has_user {
                return Err(DecodeError::Malformed("password without a user name"));
            }
            let username = if has_user { Some(r.string()?) } else { None };
            let password = if has_pass {
                Some(r.bin()?.to_vec())
            } else {
                None
            };
            Packet::Connect(Connect {
                client_id,
                keep_alive,
                clean_session: cf & 0x02 != 0,
                will,
                username,
                password,
            })
        }
        2 => {
            fixed_flags(0)?;
            let ack = r.u8()?;
            if ack & 0xFE != 0 {
                return Err(DecodeError::Malformed("reserved acknowledge flags"));
            }
            let return_code = r.u8()?;
            Packet::Connack {
                session_present: ack & 0x01 != 0,
                return_code,
            }
        }
        3 => {
            let dup = flags & 0x08 != 0;
            let qos = QoS::from_bits((flags >> 1) & 0x03)?;
            let retain = flags & 0x01 != 0;
            let topic = r.string()?;
            if !valid_topic(&topic) {
                return Err(DecodeError::Malformed("topic name"));
            }
            let packet_id = match qos {
                QoS::AtMostOnce => {
                    if dup {
                        return Err(DecodeError::Malformed("dup at QoS 0"));
                    }
                    0
                }
                QoS::AtLeastOnce => {
                    let id = r.u16()?;
                    if id == 0 {
                        return Err(DecodeError::Malformed("packet id 0"));
                    }
                    id
                }
            };
            let payload = r.rest.to_vec();
            r.rest = &[];
            Packet::Publish(Publish {
                topic,
                payload,
                qos,
                retain,
                dup,
                packet_id,
            })
        }
        4 => {
            fixed_flags(0)?;
            Packet::Puback {
                packet_id: r.u16()?,
            }
        }
        8 => {
            fixed_flags(2)?;
            let packet_id = r.u16()?;
            if packet_id == 0 {
                return Err(DecodeError::Malformed("packet id 0"));
            }
            let mut filters = Vec::new();
            while !r.rest.is_empty() {
                let filter = r.string()?;
                if !valid_filter(&filter) {
                    return Err(DecodeError::Malformed("topic filter"));
                }
                let q = r.u8()?;
                if q & 0xFC != 0 {
                    return Err(DecodeError::Malformed("reserved subscription bits"));
                }
                filters.push((filter, QoS::from_bits(q)?));
            }
            if filters.is_empty() {
                return Err(DecodeError::Malformed("no topic filter"));
            }
            Packet::Subscribe { packet_id, filters }
        }
        9 => {
            fixed_flags(0)?;
            let packet_id = r.u16()?;
            let return_codes = r.rest.to_vec();
            r.rest = &[];
            if return_codes.is_empty() {
                return Err(DecodeError::Malformed("no return code"));
            }
            if return_codes.iter().any(|c| !matches!(c, 0 | 1 | 2 | 0x80)) {
                return Err(DecodeError::Malformed("return code"));
            }
            Packet::Suback {
                packet_id,
                return_codes,
            }
        }
        12 => {
            fixed_flags(0)?;
            Packet::Pingreq
        }
        13 => {
            fixed_flags(0)?;
            Packet::Pingresp
        }
        14 => {
            fixed_flags(0)?;
            Packet::Disconnect
        }
        5..=7 => return Err(DecodeError::Unsupported("QoS 2 flow")),
        10 | 11 => return Err(DecodeError::Unsupported("UNSUBSCRIBE")),
        _ => return Err(DecodeError::Malformed("reserved packet type")),
    };
    r.end()?;
    Ok(packet)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p2mqc {
    use super::*;
    use proptest::prelude::*;

    const MAX: usize = 1 << 20;

    fn one(bytes: &[u8]) -> Packet {
        let (p, used) = decode(bytes, MAX).unwrap().unwrap();
        assert_eq!(used, bytes.len());
        p
    }

    #[test]
    fn wp_p2mqc_connect_vector() {
        let c = Packet::Connect(Connect {
            client_id: "hub".into(),
            keep_alive: 60,
            clean_session: true,
            will: Some(Will {
                topic: "d/s".into(),
                payload: b"off".to_vec(),
                qos: QoS::AtLeastOnce,
                retain: true,
            }),
            username: Some("u".into()),
            password: Some(b"pw".to_vec()),
        });
        #[rustfmt::skip]
        let want: Vec<u8> = vec![
            0x10, 32,
            0, 4, b'M', b'Q', b'T', b'T', 4,
            0b1110_1110, // user, password, will retain, will QoS 1, will, clean
            0, 60,
            0, 3, b'h', b'u', b'b',
            0, 3, b'd', b'/', b's',
            0, 3, b'o', b'f', b'f',
            0, 1, b'u',
            0, 2, b'p', b'w',
        ];
        assert_eq!(encode(&c).unwrap(), want);
        assert_eq!(one(&want), c);
    }

    #[test]
    fn wp_p2mqc_minimal_connect_vector() {
        let c = Packet::Connect(Connect {
            client_id: String::new(),
            keep_alive: 0,
            clean_session: true,
            will: None,
            username: None,
            password: None,
        });
        let want = [0x10, 12, 0, 4, b'M', b'Q', b'T', b'T', 4, 0x02, 0, 0, 0, 0];
        assert_eq!(encode(&c).unwrap(), want);
        assert_eq!(one(&want), c);
    }

    #[test]
    fn wp_p2mqc_debug_hides_password() {
        let c = Connect {
            client_id: "x".into(),
            keep_alive: 1,
            clean_session: true,
            will: None,
            username: Some("user".into()),
            password: Some(b"hunter2".to_vec()),
        };
        let d = format!("{c:?} {:?}", Packet::Connect(c.clone()));
        assert!(!d.contains("hunter2"));
        assert!(!d.contains("104, 117")); // the bytes as a list
        assert!(d.contains("\"set\""));
    }

    #[test]
    fn wp_p2mqc_small_packet_vectors() {
        let cases: Vec<(Packet, Vec<u8>)> = vec![
            (
                Packet::Connack {
                    session_present: false,
                    return_code: 0,
                },
                vec![0x20, 2, 0, 0],
            ),
            (
                Packet::Connack {
                    session_present: true,
                    return_code: 5,
                },
                vec![0x20, 2, 1, 5],
            ),
            (
                Packet::Publish(Publish {
                    topic: "a/b".into(),
                    payload: b"hi".to_vec(),
                    qos: QoS::AtMostOnce,
                    retain: true,
                    dup: false,
                    packet_id: 0,
                }),
                vec![0x31, 7, 0, 3, b'a', b'/', b'b', b'h', b'i'],
            ),
            (
                Packet::Publish(Publish {
                    topic: "t".into(),
                    payload: vec![],
                    qos: QoS::AtLeastOnce,
                    retain: false,
                    dup: true,
                    packet_id: 0x1234,
                }),
                vec![0x3A, 5, 0, 1, b't', 0x12, 0x34],
            ),
            (Packet::Puback { packet_id: 7 }, vec![0x40, 2, 0, 7]),
            (
                Packet::Subscribe {
                    packet_id: 10,
                    filters: vec![
                        ("a/+".into(), QoS::AtLeastOnce),
                        ("#".into(), QoS::AtMostOnce),
                    ],
                },
                vec![0x82, 12, 0, 10, 0, 3, b'a', b'/', b'+', 1, 0, 1, b'#', 0],
            ),
            (
                Packet::Suback {
                    packet_id: 10,
                    return_codes: vec![1, 0x80],
                },
                vec![0x90, 4, 0, 10, 1, 0x80],
            ),
            (Packet::Pingreq, vec![0xC0, 0]),
            (Packet::Pingresp, vec![0xD0, 0]),
            (Packet::Disconnect, vec![0xE0, 0]),
        ];
        for (p, bytes) in cases {
            assert_eq!(encode(&p).unwrap(), bytes, "{p:?}");
            assert_eq!(one(&bytes), p);
        }
    }

    #[test]
    fn wp_p2mqc_remaining_length_vectors() {
        // 127 is one byte, 128 two (0x80 0x01), 16 384 three (section 2.2.3's table).
        for (payload_len, header) in [
            (123usize, vec![0x30, 0x7F]),
            (124, vec![0x30, 0x80, 0x01]),
            (16_380, vec![0x30, 0x80, 0x80, 0x01]),
        ] {
            let p = Packet::Publish(Publish {
                topic: "ab".into(),
                payload: vec![0x55; payload_len],
                qos: QoS::AtMostOnce,
                retain: false,
                dup: false,
                packet_id: 0,
            });
            let bytes = encode(&p).unwrap();
            assert_eq!(&bytes[..header.len()], &header[..]);
            assert_eq!(one(&bytes), p);
        }
    }

    #[test]
    fn wp_p2mqc_partial_and_limits() {
        assert_eq!(decode(&[], MAX), Ok(None));
        assert_eq!(decode(&[0x30], MAX), Ok(None));
        assert_eq!(decode(&[0x30, 0x80, 0x80], MAX), Ok(None));
        assert_eq!(decode(&[0x30, 5, 0, 1], MAX), Ok(None));
        assert!(matches!(
            decode(&[0x30, 0xFF, 0xFF, 0xFF, 0xFF], MAX),
            Err(DecodeError::Malformed(_))
        ));
        // The limit refuses from the fixed header alone.
        assert_eq!(
            decode(&[0x30, 0xFF, 0xFF, 0xFF, 0x7F], MAX),
            Err(DecodeError::TooLarge {
                len: 5 + MAX_REMAINING_LENGTH,
                max: MAX
            })
        );
        assert_eq!(
            decode(&[0x30, 0x0A], 11),
            Err(DecodeError::TooLarge { len: 12, max: 11 })
        );
        // Two packets back to back: the first, and how much it used.
        let (p, used) = decode(&[0xD0, 0, 0xC0, 0], MAX).unwrap().unwrap();
        assert_eq!((p, used), (Packet::Pingresp, 2));
    }

    #[test]
    fn wp_p2mqc_malformed_refused() {
        let bad: &[&[u8]] = &[
            &[0xD1, 0],                                                     // reserved flags
            &[0xD0, 1, 0],                                                  // trailing byte
            &[0x36, 3, 0, 1, b't'],                                         // QoS 3
            &[0x34, 5, 0, 1, b't', 0, 1],                                   // QoS 2
            &[0x38, 3, 0, 1, b't'],                                         // dup at QoS 0
            &[0x32, 5, 0, 1, b't', 0, 0],                                   // packet id 0
            &[0x30, 3, 0, 1, b'#'],                // wildcard in a topic name
            &[0x30, 2, 0, 0],                      // empty topic
            &[0x30, 4, 0, 2, 0xC3, 0x28],          // not UTF-8
            &[0x30, 3, 0, 1, 0],                   // NUL
            &[0x30, 3, 0, 9, b't'],                // string past the packet
            &[0x82, 2, 0, 1],                      // no filter
            &[0x82, 6, 0, 1, 0, 1, b'#', 4],       // reserved subscription bits
            &[0x82, 7, 0, 1, 0, 2, b'#', b'a', 0], // # not alone
            &[0x90, 3, 0, 1, 3],                   // bad suback code
            &[0x20, 2, 2, 0],                      // reserved ack flags
            &[0x00, 0],                            // reserved type 0
            &[0xF0, 0],                            // reserved type 15
            &[0x50, 2, 0, 1],                      // PUBREC
            &[0xA2, 2, 0, 1],                      // UNSUBSCRIBE
            &[0x10, 12, 0, 4, b'M', b'Q', b'T', b'T', 3, 2, 0, 0, 0, 0], // 3.1
            &[0x10, 12, 0, 4, b'M', b'Q', b'T', b'T', 4, 3, 0, 0, 0, 0], // reserved flag
            &[0x10, 12, 0, 4, b'M', b'Q', b'T', b'T', 4, 0x42, 0, 0, 0, 0], // password alone
            &[0x10, 12, 0, 4, b'M', b'Q', b'T', b'T', 4, 0x22, 0, 0, 0, 0], // will retain, no will
        ];
        for b in bad {
            assert!(decode(b, MAX).is_err(), "{b:?} decoded");
        }
    }

    #[test]
    fn wp_p2mqc_encode_refusals() {
        let publish = |topic: &str, qos, packet_id, dup| {
            encode(&Packet::Publish(Publish {
                topic: topic.into(),
                payload: vec![],
                qos,
                retain: false,
                dup,
                packet_id,
            }))
        };
        assert!(publish("", QoS::AtMostOnce, 0, false).is_err());
        assert!(publish("a/+", QoS::AtMostOnce, 0, false).is_err());
        assert!(publish("a", QoS::AtLeastOnce, 0, false).is_err());
        assert!(publish("a", QoS::AtMostOnce, 3, false).is_err());
        assert!(publish("a", QoS::AtMostOnce, 0, true).is_err());
        assert!(publish(&"x".repeat(70_000), QoS::AtMostOnce, 0, false).is_err());
        let c = Connect {
            client_id: "x".into(),
            keep_alive: 1,
            clean_session: true,
            will: None,
            username: None,
            password: Some(vec![1]),
        };
        assert!(encode(&Packet::Connect(c)).is_err());
        assert!(
            encode(&Packet::Subscribe {
                packet_id: 1,
                filters: vec![("a/#/b".into(), QoS::AtMostOnce)]
            })
            .is_err()
        );
        assert!(
            encode(&Packet::Subscribe {
                packet_id: 1,
                filters: vec![]
            })
            .is_err()
        );
    }

    #[test]
    fn wp_p2mqc_filters() {
        for ok in ["#", "+", "a/#", "a/+/b", "+/+", "/", "a//b", "dspi/+/set"] {
            assert!(valid_filter(ok), "{ok}");
        }
        for bad in ["", "a#", "#/a", "a/b+", "a/#/b", "++", "a\0"] {
            assert!(!valid_filter(bad), "{bad}");
        }
    }

    fn arb_qos() -> impl Strategy<Value = QoS> {
        prop_oneof![Just(QoS::AtMostOnce), Just(QoS::AtLeastOnce)]
    }

    fn arb_topic() -> impl Strategy<Value = String> {
        "[a-z0-9/_é]{1,40}"
    }

    fn arb_packet() -> impl Strategy<Value = Packet> {
        let publish = (
            arb_topic(),
            proptest::collection::vec(any::<u8>(), 0..300),
            arb_qos(),
            any::<bool>(),
            any::<bool>(),
            1u16..,
        )
            .prop_map(|(topic, payload, qos, retain, dup, id)| {
                let (dup, packet_id) = match qos {
                    QoS::AtMostOnce => (false, 0),
                    QoS::AtLeastOnce => (dup, id),
                };
                Packet::Publish(Publish {
                    topic,
                    payload,
                    qos,
                    retain,
                    dup,
                    packet_id,
                })
            });
        let will = proptest::option::of(
            (
                arb_topic(),
                proptest::collection::vec(any::<u8>(), 0..50),
                arb_qos(),
                any::<bool>(),
            )
                .prop_map(|(topic, payload, qos, retain)| Will {
                    topic,
                    payload,
                    qos,
                    retain,
                }),
        );
        let connect = (
            "[ -~]{0,30}",
            any::<u16>(),
            any::<bool>(),
            will,
            proptest::option::of("[ -~]{0,20}"),
            proptest::option::of(proptest::collection::vec(any::<u8>(), 0..20)),
        )
            .prop_map(
                |(client_id, keep_alive, clean_session, will, username, password)| {
                    let password = if username.is_some() { password } else { None };
                    Packet::Connect(Connect {
                        client_id,
                        keep_alive,
                        clean_session,
                        will,
                        username,
                        password,
                    })
                },
            );
        prop_oneof![
            publish,
            connect,
            (any::<bool>(), 0u8..6).prop_map(|(session_present, return_code)| Packet::Connack {
                session_present,
                return_code
            }),
            any::<u16>().prop_map(|packet_id| Packet::Puback { packet_id }),
            (
                1u16..,
                proptest::collection::vec(("[a-z]{1,8}(/\\+)?(/#)?", arb_qos()), 1..5)
            )
                .prop_map(|(packet_id, filters)| Packet::Subscribe { packet_id, filters }),
            (
                any::<u16>(),
                proptest::collection::vec(
                    prop_oneof![Just(0u8), Just(1), Just(2), Just(0x80)],
                    1..5
                )
            )
                .prop_map(|(packet_id, return_codes)| Packet::Suback {
                    packet_id,
                    return_codes
                }),
            Just(Packet::Pingreq),
            Just(Packet::Pingresp),
            Just(Packet::Disconnect),
        ]
    }

    proptest! {
        /// Whatever a broker sends, decoding never panics, never claims more bytes than it
        /// was given, and never accepts a packet over the limit.
        #[test]
        fn proptest_wp_p2mqc_decode_never_panics(
            bytes in proptest::collection::vec(any::<u8>(), 0..600),
            max in 0usize..700,
        ) {
            if let Ok(Some((_, used))) = decode(&bytes, max) {
                prop_assert!(used <= bytes.len());
                prop_assert!(used <= max);
            }
        }

        /// A valid packet's first byte with a fuzzed remaining length and body: no panic,
        /// and a length over the limit is refused before the body is there.
        #[test]
        fn proptest_wp_p2mqc_fuzzed_lengths(
            first in prop_oneof![Just(0x10u8), Just(0x20), Just(0x30), Just(0x32), Just(0x3B), Just(0x40), Just(0x82), Just(0x90), Just(0xC0), Just(0xD0), Just(0xE0)],
            rem in 0usize..=MAX_REMAINING_LENGTH,
            body in proptest::collection::vec(any::<u8>(), 0..300),
            max in 0usize..400,
        ) {
            let mut bytes = vec![first];
            let mut r = rem;
            loop {
                let mut b = (r & 0x7F) as u8;
                r >>= 7;
                if r > 0 { b |= 0x80; }
                bytes.push(b);
                if r == 0 { break; }
            }
            let header = bytes.len();
            bytes.extend_from_slice(&body);
            match decode(&bytes, max) {
                Err(DecodeError::TooLarge { len, .. }) => prop_assert_eq!(len, header + rem),
                Ok(Some((_, used))) => prop_assert_eq!(used, header + rem),
                Ok(None) => prop_assert!(bytes.len() < header + rem),
                Err(_) => {}
            }
        }

        /// Every packet the hub can encode decodes to itself, also when the bytes arrive in
        /// two pieces, and a corrupted copy never panics.
        #[test]
        fn proptest_wp_p2mqc_round_trip(p in arb_packet(), cut in any::<prop::sample::Index>(), flip in any::<(prop::sample::Index, u8)>()) {
            let bytes = encode(&p).unwrap();
            prop_assert_eq!(decode(&bytes, MAX), Ok(Some((p.clone(), bytes.len()))));
            let at = cut.index(bytes.len());
            if at < bytes.len() {
                prop_assert_eq!(decode(&bytes[..at], MAX), Ok(None));
            }
            let mut bad = bytes.clone();
            let i = flip.0.index(bad.len());
            bad[i] ^= flip.1;
            let _ = decode(&bad, MAX);
        }
    }
}
