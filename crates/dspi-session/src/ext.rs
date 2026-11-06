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

pub use dspi_proto::ext::{
    BadArgument, CoreVregStatus, LIST_READ_LEN, PairMap, VregOp, decode_list, name, parse_vreg,
    vreg_label, vreg_mv,
};

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

    /// WP CG14: this voltage is above 1.20 V and above what the unit reports
    /// now, and the confirmation word was not given. Nothing was written.
    #[error("{target} needs the confirmation word: type {word}", target = vreg_label(*code), word = crate::overvolt::WORD)]
    WordRequired {
        /// The voltage asked for.
        code: u8,
    },

    #[error(transparent)]
    Transport(TransportError),
}

pub type Result<T> = std::result::Result<T, ExtError>;

impl From<BadArgument> for ExtError {
    fn from(e: BadArgument) -> Self {
        ExtError::BadArgument(e.0)
    }
}

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

/// The live pair map. [`ExtError::Unsupported`] when the firmware lacks it; a
/// reply shorter than four bytes is a `ShortRead` transport error.
pub fn read_pair_map(t: &mut dyn Transport) -> Result<PairMap> {
    let b = read(t, g::EXT_CROSSFEED_PAIR_MAP, PairMap::SIZE)?;
    pair_map_from(&b).ok_or(ExtError::Transport(TransportError::ShortRead {
        opcode: op::REQ_EXTENSION,
        wanted: PairMap::SIZE,
        got: b.len(),
    }))
}

/// The pair map in a reply's first four bytes; `None` when it is shorter.
pub(crate) fn pair_map_from(b: &[u8]) -> Option<PairMap> {
    b.get(..PairMap::SIZE)?.try_into().ok().map(PairMap)
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
///
/// WP CG14: a `SET` or `TRIAL` first reads the unit's status, and with no
/// [`crate::overvolt::WORD`] in `word` a raise the rule covers is refused with
/// [`ExtError::WordRequired`] and nothing written. A failed read counts as an
/// unreadable unit. The other requests read nothing.
pub fn core_vreg_op(t: &mut dyn Transport, op: VregOp, word: Option<&str>) -> Result<()> {
    if let VregOp::Set(code) | VregOp::Trial(code) = op {
        let status = read_core_vreg(t).ok();
        if crate::overvolt::op_needs_word(op, status.as_ref())
            && !crate::overvolt::word_matches(word)
        {
            return Err(ExtError::WordRequired { code });
        }
    }
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
    fn wp_s3_a_short_pair_map_reply_is_an_error_not_a_panic() {
        let mut t = MockTransport::new().reply(
            op::REQ_EXTENSION,
            Reply::Sequence(vec![Reply::Data(vec![0x02, 0x13, 0])]),
        );
        assert!(matches!(
            read_pair_map(&mut t),
            Err(ExtError::Transport(TransportError::ShortRead { .. }))
        ));
        assert_eq!(pair_map_from(&[1, 2, 3]), None);
        assert_eq!(pair_map_from(&[1, 2, 3, 4, 5]), Some(PairMap([1, 2, 3, 4])));
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
        core_vreg_op(&mut t, VregOp::Trial(0x0E), Some("OVERVOLT")).unwrap();
        let log = log.lock().unwrap();
        let e = log.last().unwrap();
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
