//! The extension channel: `REQ_EXTENSION` (0x00), with the sub-command in
//! `wValue`.
//!
//! Features developed outside Weeb Labs live behind this one opcode rather
//! than claiming opcodes upstream may assign later (the Bofr5555/DSPi fork's
//! `Documentation/Features/extension_channel_spec.md`). The direction comes
//! from the request type: an IN reads the sub-command's value, an OUT writes
//! it. Multi-byte values are little-endian.
//!
//! **A stall is an answer, not a fault.** Stock firmware has no 0x00 handler,
//! so `EXT_LIST` stalls there, and a sub-command the firmware lacks stalls
//! too. Both read as [`ExtError::Unsupported`]. Nothing here retries a stall:
//! that would only make "not supported" slow.

use dspi_proto::generated::ext as g;
use dspi_proto::generated::opcodes as op;
use dspi_transport::{Transport, TransportError};

/// Room for 32 sub-command ids (extension_channel_spec.md 2.1).
pub const LIST_READ_LEN: usize = 64;

/// What a sub-command is called, for listings.
pub fn name(id: u16) -> &'static str {
    match id {
        g::EXT_LIST => "list",
        g::EXT_CROSSFEED_PAIR_MAP => "crossfeed pair map",
        g::EXT_CORE_VREG => "core voltage",
        0x8000..=0xFFFF => "experimental",
        _ => "unknown",
    }
}

/// `EXT_LIST`'s reply: uint16 LE ids, ascending, 0 excluded. An odd trailing
/// byte (a truncated reply) is dropped.
pub fn decode_list(b: &[u8]) -> Vec<u16> {
    b.as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum ExtError {
    /// The device stalled a read: it has no extension channel, or not this
    /// sub-command.
    #[error("not supported by this firmware (extension 0x{id:04X} {}: the device stalled)", name(*id))]
    Unsupported { id: u16 },

    /// The device stalled a write: the firmware lacks the sub-command, or
    /// rejected the payload.
    #[error("the device refused extension 0x{id:04X} {} (unsupported, or an invalid request)", name(*id))]
    Refused { id: u16 },

    /// The write was accepted but the readback disagrees. The pair map does
    /// this on purpose: an invalid map is ACKed and ignored.
    #[error("{0}")]
    NotApplied(String),

    #[error("{0}")]
    BadArgument(String),

    #[error(transparent)]
    Transport(TransportError),
}

pub type Result<T> = std::result::Result<T, ExtError>;

fn read(t: &mut dyn Transport, id: u16, len: usize) -> Result<Vec<u8>> {
    t.control_in(op::REQ_EXTENSION, id, len as u16)
        .map_err(|e| match e {
            TransportError::Stalled { .. } => ExtError::Unsupported { id },
            e => ExtError::Transport(e),
        })
}

fn write(t: &mut dyn Transport, id: u16, payload: &[u8]) -> Result<()> {
    t.control_out(op::REQ_EXTENSION, id, payload)
        .map_err(|e| match e {
            TransportError::Stalled { .. } => ExtError::Refused { id },
            e => ExtError::Transport(e),
        })
}

/// The supported sub-commands. [`ExtError::Unsupported`] on firmware with no
/// extension channel, which is every stock build.
pub fn list(t: &mut dyn Transport) -> Result<Vec<u16>> {
    let b = t
        .control_in_upto(op::REQ_EXTENSION, g::EXT_LIST, LIST_READ_LEN as u16)
        .map_err(|e| match e {
            TransportError::Stalled { .. } => ExtError::Unsupported { id: g::EXT_LIST },
            e => ExtError::Transport(e),
        })?;
    Ok(decode_list(&b))
}

// ------------------------------------------------------------ pair map

/// `EXT_CROSSFEED_PAIR_MAP`: which two outputs each crossfeed pair processes.
///
/// Byte `p` is pair `p`. `0x00` is the default pairing (`2p`, `2p+1`); any
/// other value is `(right << 4) | left` (crossfeed_map.h).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairMap(pub [u8; 4]);

impl PairMap {
    pub const SIZE: usize = 4;

    /// The outputs of pair `p`, as `(left, right)`.
    pub fn pair(&self, p: usize) -> (u8, u8) {
        match self.0[p] {
            0 => (2 * p as u8, 2 * p as u8 + 1),
            b => (b & 0x0F, b >> 4),
        }
    }

    /// The firmware's normal form: an explicit default byte reads back as 0.
    pub fn normalized(&self) -> PairMap {
        let mut out = self.0;
        for (p, b) in out.iter_mut().enumerate() {
            if *b == ((2 * p as u8 + 1) << 4) | (2 * p as u8) {
                *b = 0;
            }
        }
        PairMap(out)
    }

    /// Parse up to four pairs: `default` (or `-`), `L:R` with output numbers,
    /// or a raw byte `0xNN`. Missing pairs are the default.
    pub fn parse(args: &[&str]) -> Result<PairMap> {
        if args.len() > 4 {
            return Err(ExtError::BadArgument(format!(
                "at most four pairs, got {}",
                args.len()
            )));
        }
        let mut map = [0u8; 4];
        for (p, a) in args.iter().enumerate() {
            map[p] = if *a == "default" || *a == "-" {
                0
            } else if let Some(hex) = a.strip_prefix("0x").or_else(|| a.strip_prefix("0X")) {
                u8::from_str_radix(hex, 16)
                    .map_err(|_| ExtError::BadArgument(format!("pair {p}: bad byte {a}")))?
            } else if let Some((l, r)) = a.split_once(':') {
                let out = |s: &str| {
                    s.parse::<u8>()
                        .ok()
                        .filter(|v| *v < 16)
                        .ok_or_else(|| ExtError::BadArgument(format!("pair {p}: bad output {s}")))
                };
                let (l, r) = (out(l)?, out(r)?);
                (r << 4) | l
            } else {
                return Err(ExtError::BadArgument(format!(
                    "pair {p}: expected L:R, default or 0xNN, got {a}"
                )));
            };
        }
        Ok(PairMap(map))
    }
}

impl std::fmt::Display for PairMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for p in 0..4 {
            let (l, r) = self.pair(p);
            let tag = if self.0[p] == 0 { " (default)" } else { "" };
            if p > 0 {
                writeln!(f)?;
            }
            write!(f, "pair {p}: L=out {l}  R=out {r}{tag}")?;
        }
        Ok(())
    }
}

pub fn read_pair_map(t: &mut dyn Transport) -> Result<PairMap> {
    let b = read(t, g::EXT_CROSSFEED_PAIR_MAP, PairMap::SIZE)?;
    Ok(PairMap([b[0], b[1], b[2], b[3]]))
}

/// Set the whole map, then read it back. The firmware ACKs an invalid map and
/// ignores it, so the readback is the only confirmation there is.
pub fn write_pair_map(t: &mut dyn Transport, map: PairMap) -> Result<PairMap> {
    write(t, g::EXT_CROSSFEED_PAIR_MAP, &map.0)?;
    let now = read_pair_map(t)?;
    if now != map.normalized() {
        return Err(ExtError::NotApplied(format!(
            "the device kept its map ({}): the requested one is invalid \
             (pairs need distinct stereo outputs, never the PDM sub, and no \
             output may be in two pairs)",
            hex(&now.0)
        )));
    }
    Ok(now)
}

// ------------------------------------------------------------ core voltage

/// `EXT_CORE_VREG`'s 16-byte status (core_vreg.h).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreVregStatus {
    pub live_vreg: u8,
    /// 0 = the build default.
    pub stored_sel: u8,
    pub default_vreg: u8,
    pub flags: u8,
    pub hang_count: u8,
    pub trial_active: bool,
    pub trial_vreg: u8,
    /// 0 = none.
    pub failed_trial_vreg: u8,
    pub failed_trial_uptime_s: u32,
    pub uptime_s: u32,
}

impl CoreVregStatus {
    pub const SIZE: usize = 16;
    pub const FLAG_AUTO_RAISED: u8 = 0x01;
    pub const FLAG_AUTO_DISABLED: u8 = 0x02;

    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        Some(Self {
            live_vreg: b[0],
            stored_sel: b[1],
            default_vreg: b[2],
            flags: b[3],
            hang_count: b[4],
            trial_active: b[5] != 0,
            trial_vreg: b[6],
            failed_trial_vreg: b[7],
            failed_trial_uptime_s: u32_at(8),
            uptime_s: u32_at(12),
        })
    }

    /// Readable `key value` pairs, in wire order.
    pub fn fields(&self) -> Vec<(&'static str, String)> {
        let flag = |bit: u8| if self.flags & bit != 0 { "yes" } else { "no" };
        let mut v = vec![
            ("live", vreg_label(self.live_vreg)),
            (
                "stored",
                if self.stored_sel == 0 {
                    "build default".into()
                } else {
                    vreg_label(self.stored_sel)
                },
            ),
            ("default", vreg_label(self.default_vreg)),
            ("auto_raised", flag(Self::FLAG_AUTO_RAISED).into()),
            ("auto_disabled", flag(Self::FLAG_AUTO_DISABLED).into()),
            ("hang_count", self.hang_count.to_string()),
            (
                "trial",
                if self.trial_active {
                    vreg_label(self.trial_vreg)
                } else {
                    "none".into()
                },
            ),
        ];
        v.push((
            "failed_trial",
            if self.failed_trial_vreg == 0 {
                "none".into()
            } else {
                format!(
                    "{} after {} s",
                    vreg_label(self.failed_trial_vreg),
                    self.failed_trial_uptime_s
                )
            },
        ));
        v.push(("uptime_s", self.uptime_s.to_string()));
        v
    }
}

/// A vreg code in millivolts. The RP2040 and RP2350 share codes 0x06-0x0F:
/// 0.85 V in 50 mV steps, so 0x0C is 1.15 V and 0x0F is 1.30 V.
pub fn vreg_mv(code: u8) -> Option<u16> {
    (0x06..=0x0F)
        .contains(&code)
        .then(|| 850 + 50 * (code as u16 - 0x06))
}

pub fn vreg_label(code: u8) -> String {
    match vreg_mv(code) {
        Some(mv) => format!("{}.{:02} V (0x{code:02X})", mv / 1000, (mv % 1000) / 10),
        None => format!("0x{code:02X}"),
    }
}

/// A voltage as written: `1.20`, `1200`, `1.2V`, or a code `0x0D`.
pub fn parse_vreg(s: &str) -> Result<u8> {
    let bad = || ExtError::BadArgument(format!("not a core voltage: {s}"));
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return u8::from_str_radix(hex, 16).map_err(|_| bad());
    }
    let t = s.trim_end_matches(['V', 'v']);
    let v: f64 = t.parse().map_err(|_| bad())?;
    let mv = if v < 10.0 { v * 1000.0 } else { v };
    let mv = mv.round() as u16;
    (0x06..=0x0F)
        .find(|c| vreg_mv(*c) == Some(mv))
        .ok_or_else(bad)
}

/// `EXT_CORE_VREG`'s OUT `{op, value}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VregOp {
    /// Persist a voltage; 0 is the build default.
    Set(u8),
    /// RAM-only trial voltage.
    Trial(u8),
    TrialEnd,
    /// Automatic raising after repeated hangs.
    Auto(bool),
    /// Forget the hang count and the last trial failure.
    Clear,
}

impl VregOp {
    pub fn encode(self) -> [u8; 2] {
        match self {
            VregOp::Set(v) => [0, v],
            VregOp::Trial(v) => [1, v],
            VregOp::TrialEnd => [2, 0],
            VregOp::Auto(on) => [3, on as u8],
            VregOp::Clear => [4, 0],
        }
    }
}

pub fn read_core_vreg(t: &mut dyn Transport) -> Result<CoreVregStatus> {
    let b = read(t, g::EXT_CORE_VREG, CoreVregStatus::SIZE)?;
    CoreVregStatus::decode(&b).ok_or(ExtError::Transport(TransportError::ShortRead {
        opcode: op::REQ_EXTENSION,
        wanted: CoreVregStatus::SIZE,
        got: b.len(),
    }))
}

/// Send one core-voltage operation. The firmware refuses (stalls) an unknown
/// op, a voltage outside 1.15-1.30 V, or a SET below the build default.
pub fn core_vreg_op(t: &mut dyn Transport, op: VregOp) -> Result<()> {
    write(t, g::EXT_CORE_VREG, &op.encode())
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dspi_transport::MockTransport;
    use dspi_transport::mock::{Direction, Reply};

    #[test]
    fn the_ids_match_the_spec() {
        assert_eq!(op::REQ_EXTENSION, 0x00);
        assert_eq!(g::EXT_LIST, 0x0000);
        assert_eq!(g::EXT_CROSSFEED_PAIR_MAP, 0x0001);
        assert_eq!(g::EXT_CORE_VREG, 0x0002);
    }

    #[test]
    fn the_list_decodes_the_documented_reply() {
        // "Current firmware answers EXT_LIST with 01 00 02 00."
        let mut t = MockTransport::new().data(op::REQ_EXTENSION, vec![1, 0, 2, 0]);
        let log = t.log_handle();
        assert_eq!(list(&mut t).unwrap(), vec![1, 2]);
        let e = &log.lock().unwrap()[0];
        assert_eq!(
            (e.direction, e.opcode, e.value),
            (Direction::In, 0x00, 0x0000)
        );
    }

    #[test]
    fn stock_firmware_reads_as_unsupported_not_as_a_failure() {
        let mut t = MockTransport::new().reply(op::REQ_EXTENSION, Reply::Stall);
        let log = t.log_handle();
        assert!(matches!(list(&mut t), Err(ExtError::Unsupported { id: 0 })));
        assert!(matches!(
            read_core_vreg(&mut t),
            Err(ExtError::Unsupported { id: 2 })
        ));
        assert!(matches!(
            write_pair_map(&mut t, PairMap([0x02, 0x13, 0, 0])),
            Err(ExtError::Refused { id: 1 })
        ));
        // One transfer each: a stall is the answer, never retried.
        assert_eq!(log.lock().unwrap().len(), 3);
        let msg = list(&mut t).unwrap_err().to_string();
        assert!(msg.contains("not supported"), "{msg}");
    }

    #[test]
    fn the_k340_map_encodes_as_the_spec_example() {
        // Woofers L=2 / R=0 and tweeters L=3 / R=1 give 02 13 00 00.
        let m = PairMap::parse(&["2:0", "3:1"]).unwrap();
        assert_eq!(m.0, [0x02, 0x13, 0x00, 0x00]);
        assert_eq!(m.pair(0), (2, 0));
        assert_eq!(m.pair(1), (3, 1));
        assert_eq!(m.pair(2), (4, 5));
        assert_eq!(
            PairMap::parse(&["0x02", "0x13", "-", "default"]).unwrap(),
            m
        );
        assert!(PairMap::parse(&["2-0"]).is_err());
        assert!(PairMap::parse(&["16:0"]).is_err());
        assert!(PairMap::parse(&["-", "-", "-", "-", "-"]).is_err());
    }

    #[test]
    fn explicit_defaults_normalize_to_zero() {
        // Pair 1's default is outputs 2/3: (3 << 4) | 2.
        assert_eq!(PairMap([0x10, 0x32, 0, 0]).normalized(), PairMap([0; 4]));
        assert_eq!(
            PairMap([0x02, 0x13, 0, 0]).normalized().0,
            [0x02, 0x13, 0, 0]
        );
    }

    #[test]
    fn a_pair_map_write_sends_four_bytes_and_confirms() {
        let mut t = MockTransport::new().data(op::REQ_EXTENSION, vec![0x02, 0x13, 0, 0]);
        let log = t.log_handle();
        let m = write_pair_map(&mut t, PairMap([0x02, 0x13, 0, 0])).unwrap();
        assert_eq!(m.0, [0x02, 0x13, 0, 0]);
        let log = log.lock().unwrap();
        assert_eq!(log[0].direction, Direction::Out);
        assert_eq!((log[0].opcode, log[0].value), (0x00, 0x0001));
        assert_eq!(log[0].payload, vec![0x02, 0x13, 0, 0]);
        assert_eq!((log[1].direction, log[1].value), (Direction::In, 0x0001));
    }

    #[test]
    fn an_ignored_map_is_reported() {
        // The firmware ACKs an invalid map and keeps the old one.
        let mut t = MockTransport::new().data(op::REQ_EXTENSION, vec![0, 0, 0, 0]);
        let e = write_pair_map(&mut t, PairMap([0x00, 0x00, 0x00, 0x10])).unwrap_err();
        assert!(matches!(e, ExtError::NotApplied(_)), "{e}");
    }

    #[test]
    fn the_core_vreg_status_decodes_little_endian() {
        let mut b = vec![0x0D, 0x00, 0x0D, 0x03, 1, 1, 0x0E, 0x0C];
        b.extend_from_slice(&90u32.to_le_bytes());
        b.extend_from_slice(&3600u32.to_le_bytes());
        let mut t = MockTransport::new().data(op::REQ_EXTENSION, b);
        let log = t.log_handle();
        let s = read_core_vreg(&mut t).unwrap();
        assert_eq!(log.lock().unwrap()[0].value, 0x0002);
        assert_eq!(s.live_vreg, 0x0D);
        assert_eq!(s.stored_sel, 0);
        assert!(s.trial_active);
        assert_eq!(s.trial_vreg, 0x0E);
        assert_eq!(s.failed_trial_vreg, 0x0C);
        assert_eq!(s.failed_trial_uptime_s, 90);
        assert_eq!(s.uptime_s, 3600);
        let f = s.fields();
        assert!(f.contains(&("live", "1.20 V (0x0D)".into())));
        assert!(f.contains(&("stored", "build default".into())));
        assert!(f.contains(&("auto_raised", "yes".into())));
        assert!(f.contains(&("auto_disabled", "yes".into())));
        assert!(f.contains(&("failed_trial", "1.15 V (0x0C) after 90 s".into())));
    }

    #[test]
    fn a_short_vreg_reply_is_an_error_not_a_panic() {
        let mut t = MockTransport::new().data(op::REQ_EXTENSION, vec![0x0D; 8]);
        assert!(matches!(
            read_core_vreg(&mut t),
            Err(ExtError::Transport(TransportError::ShortRead { .. }))
        ));
    }

    #[test]
    fn vreg_ops_encode_as_op_and_value() {
        assert_eq!(VregOp::Set(0).encode(), [0, 0]);
        assert_eq!(VregOp::Set(0x0D).encode(), [0, 0x0D]);
        assert_eq!(VregOp::Trial(0x0E).encode(), [1, 0x0E]);
        assert_eq!(VregOp::TrialEnd.encode(), [2, 0]);
        assert_eq!(VregOp::Auto(false).encode(), [3, 0]);
        assert_eq!(VregOp::Clear.encode(), [4, 0]);

        let mut t = MockTransport::new();
        let log = t.log_handle();
        core_vreg_op(&mut t, VregOp::Trial(0x0E)).unwrap();
        let e = &log.lock().unwrap()[0];
        assert_eq!(
            (e.direction, e.opcode, e.value),
            (Direction::Out, 0x00, 0x0002)
        );
        assert_eq!(e.payload, vec![1, 0x0E]);
    }

    #[test]
    fn voltages_parse_in_volts_millivolts_or_codes() {
        assert_eq!(parse_vreg("1.20").unwrap(), 0x0D);
        assert_eq!(parse_vreg("1.2V").unwrap(), 0x0D);
        assert_eq!(parse_vreg("1250").unwrap(), 0x0E);
        assert_eq!(parse_vreg("0x0F").unwrap(), 0x0F);
        assert!(parse_vreg("1.17").is_err());
        assert_eq!(vreg_label(0x0C), "1.15 V (0x0C)");
        assert_eq!(vreg_mv(0x0F), Some(1300));
    }
}
