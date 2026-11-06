//! What the extension channel says about the connected device, probed once
//! on connect and kept with the device state.
//!
//! Owned by WP S3 (docs/core-merge/PLAN.md).
//!
//! The probe is split sans-IO (plan S-l): [`supported_from_list`] turns an
//! `EXT_LIST` reply into the supported ids, [`reads_for`] decides which
//! sub-commands to read next, and [`from_replies`] builds the [`ExtState`]
//! from what came back. Only [`ExtState::probe`] touches the transport.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::ext::{CoreVregStatus, LIST_READ_LEN, PairMap, decode_list};
use dspi_proto::generated::ext as g;
use dspi_proto::generated::opcodes as op;
use dspi_transport::{Transport, TransportError};

/// The extension channel's view of a device. `supported` is empty on stock
/// firmware (`EXT_LIST` stalls there), and then everything else is `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtState {
    /// Sub-command ids from `EXT_LIST`, 0 excluded.
    pub supported: Vec<u16>,
    /// `EXT_CROSSFEED_PAIR_MAP`, when supported.
    pub pair_map: Option<PairMap>,
    /// `EXT_CORE_VREG`, when supported. Re-read on demand (it changes with
    /// uptime and the trial state).
    pub core_vreg: Option<CoreVregStatus>,
}

/// `wLength` of the `EXT_LIST` read: 64 bytes, room for 32 ids. The
/// conversion is checked at compile time.
const LIST_LEN: u16 = {
    assert!(LIST_READ_LEN <= u16::MAX as usize);
    LIST_READ_LEN as u16
};

impl ExtState {
    /// Probe: `EXT_LIST`, then each supported sub-command this build knows.
    /// A stall is "not supported" and gives the empty state, never an error;
    /// only a transport failure (disconnect, timeout) is returned.
    ///
    /// One transfer per read and no retries: a stall is the answer. A listed
    /// sub-command whose read stalls or comes back short is left `None` (it
    /// stays in `supported`, which is what the device claimed).
    pub fn probe(t: &mut dyn Transport) -> Result<Self, TransportError> {
        let list = match t.control_in_upto(op::REQ_EXTENSION, g::EXT_LIST, LIST_LEN) {
            Ok(b) => b,
            Err(e) => {
                absent(e)?;
                return Ok(Self::default());
            }
        };
        let supported = supported_from_list(&list);
        let mut replies = Vec::new();
        for (id, len) in reads_for(&supported) {
            match t.control_in(op::REQ_EXTENSION, id, len) {
                Ok(b) => replies.push((id, b)),
                Err(e) => absent(e)?,
            }
        }
        Ok(from_replies(supported, &replies))
    }

    /// Whether `EXT_LIST` named sub-command `id`.
    pub fn has(&self, id: u16) -> bool {
        self.supported.contains(&id)
    }
}

/// Sorts a failed read into "absent" (`Ok`) or a transport failure (`Err`).
/// A stall is the device saying "not supported"; a short reply is a device
/// answer too, just not a usable one. Everything else (disconnect, timeout, a
/// link refusing the client) is the transport's and goes back to the caller.
fn absent(e: TransportError) -> Result<(), TransportError> {
    match e {
        TransportError::Stalled { .. } | TransportError::ShortRead { .. } => Ok(()),
        e => Err(e),
    }
}

/// The supported ids in an `EXT_LIST` reply: 0 dropped (the list excludes
/// itself), sorted and deduplicated, so a sloppy reply cannot make the probe
/// read a sub-command twice. An odd trailing byte is ignored.
fn supported_from_list(reply: &[u8]) -> Vec<u16> {
    let mut ids: Vec<u16> = decode_list(reply)
        .into_iter()
        .filter(|id| *id != g::EXT_LIST)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// The sub-commands to read after `EXT_LIST`, with each reply's size: the
/// ones this build knows, in id order. Unknown and experimental ids are
/// listed in `supported` but never read.
fn reads_for(supported: &[u16]) -> Vec<(u16, u16)> {
    const KNOWN: [(u16, u16); 2] = [
        (g::EXT_CROSSFEED_PAIR_MAP, PairMap::SIZE as u16),
        (g::EXT_CORE_VREG, CoreVregStatus::SIZE as u16),
    ];
    KNOWN
        .into_iter()
        .filter(|(id, _)| supported.contains(id))
        .collect()
}

/// Build the state from the supported ids and the replies read for them. A
/// reply too short to decode leaves its field `None`; extra bytes are
/// ignored; replies for ids not in `supported` are ignored.
fn from_replies(supported: Vec<u16>, replies: &[(u16, Vec<u8>)]) -> ExtState {
    let reply = |id: u16| {
        if !supported.contains(&id) {
            return None;
        }
        replies
            .iter()
            .find(|(r, _)| *r == id)
            .map(|(_, b)| b.as_slice())
    };
    let pair_map = reply(g::EXT_CROSSFEED_PAIR_MAP).and_then(crate::ext::pair_map_from);
    let core_vreg = reply(g::EXT_CORE_VREG).and_then(CoreVregStatus::decode);
    ExtState {
        supported,
        pair_map,
        core_vreg,
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_s3 {
    use super::*;
    use dspi_transport::MockTransport;
    use dspi_transport::mock::{Direction, Reply};
    use proptest::prelude::*;

    /// A K340's vreg status: live 1.20 V, auto-raise on, 1 h uptime.
    fn vreg_reply() -> Vec<u8> {
        let mut b = vec![0x0D, 0x00, 0x0D, 0x01, 0, 0, 0, 0];
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&3600u32.to_le_bytes());
        b
    }

    // ---------------------------------------------------------- sans-IO

    #[test]
    fn wp_s3_the_list_drops_zero_duplicates_and_an_odd_byte() {
        assert_eq!(supported_from_list(&[1, 0, 2, 0]), vec![1, 2]);
        assert_eq!(
            supported_from_list(&[2, 0, 0, 0, 1, 0, 2, 0, 0x7F]),
            vec![1, 2]
        );
        assert!(supported_from_list(&[]).is_empty());
        assert!(supported_from_list(&[0, 0, 0, 0]).is_empty());
    }

    #[test]
    fn wp_s3_only_known_listed_sub_commands_are_read() {
        assert!(reads_for(&[]).is_empty());
        assert_eq!(reads_for(&[2]), vec![(2, 16)]);
        assert_eq!(reads_for(&[1, 2, 3, 0x8001]), vec![(1, 4), (2, 16)]);
    }

    #[test]
    fn wp_s3_short_replies_are_absent_and_unlisted_ones_ignored() {
        let s = from_replies(vec![1, 2], &[(1, vec![2, 0x13, 0]), (2, vec![0x0D; 15])]);
        assert_eq!(s.supported, vec![1, 2]);
        assert_eq!((s.pair_map, s.core_vreg), (None, None));

        let s = from_replies(vec![2], &[(1, vec![2, 0x13, 0, 0])]);
        assert_eq!(s.pair_map, None);

        let s = from_replies(vec![1], &[(1, vec![2, 0x13, 0, 0, 0xFF])]);
        assert_eq!(s.pair_map, Some(PairMap([2, 0x13, 0, 0])));
    }

    proptest! {
        #[test]
        fn proptest_wp_s3_building_the_state_never_panics(
            list in proptest::collection::vec(any::<u8>(), 0..80),
            pm in proptest::collection::vec(any::<u8>(), 0..8),
            vr in proptest::collection::vec(any::<u8>(), 0..24),
        ) {
            let supported = supported_from_list(&list);
            prop_assert!(!supported.contains(&0));
            prop_assert!(supported.windows(2).all(|w| w[0] < w[1]));
            let s = from_replies(supported.clone(), &[(1, pm.clone()), (2, vr.clone())]);
            prop_assert_eq!(s.pair_map.is_some(), supported.contains(&1) && pm.len() >= 4);
            prop_assert_eq!(s.core_vreg.is_some(), supported.contains(&2) && vr.len() >= 16);
        }
    }

    // ---------------------------------------------------------- probe

    #[test]
    fn wp_s3_stock_firmware_probes_as_empty() {
        let mut t = MockTransport::new().reply(op::REQ_EXTENSION, Reply::Stall);
        let log = t.log_handle();
        let s = ExtState::probe(&mut t).unwrap();
        assert_eq!(s, ExtState::default());
        assert!(!s.has(g::EXT_CROSSFEED_PAIR_MAP));
        // One EXT_LIST read, never retried.
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(
            (log[0].direction, log[0].opcode, log[0].value),
            (Direction::In, 0x00, 0x0000)
        );
    }

    #[test]
    fn wp_s3_a_k340_probes_the_pair_map_and_core_voltage() {
        let mut t = MockTransport::new().reply(
            op::REQ_EXTENSION,
            Reply::Sequence(vec![
                Reply::Data(vec![1, 0, 2, 0]),
                Reply::Data(vec![0x02, 0x13, 0, 0]),
                Reply::Data(vreg_reply()),
            ]),
        );
        let log = t.log_handle();
        let s = ExtState::probe(&mut t).unwrap();
        assert_eq!(s.supported, vec![1, 2]);
        assert!(s.has(g::EXT_CROSSFEED_PAIR_MAP) && s.has(g::EXT_CORE_VREG));
        assert!(!s.has(3));
        assert_eq!(s.pair_map, Some(PairMap([0x02, 0x13, 0, 0])));
        let v = s.core_vreg.unwrap();
        assert_eq!((v.live_vreg, v.flags, v.uptime_s), (0x0D, 0x01, 3600));
        let seen: Vec<_> = log
            .lock()
            .unwrap()
            .iter()
            .map(|e| (e.direction, e.opcode, e.value))
            .collect();
        assert_eq!(
            seen,
            vec![
                (Direction::In, 0x00, 0x0000),
                (Direction::In, 0x00, 0x0001),
                (Direction::In, 0x00, 0x0002),
            ]
        );
    }

    #[test]
    fn wp_s3_a_stalled_sub_command_is_absent_not_an_error() {
        // Listed, but the read stalls: the device's word is kept in
        // `supported`, the value is absent, and the next read still runs.
        let mut t = MockTransport::new().reply(
            op::REQ_EXTENSION,
            Reply::Sequence(vec![
                Reply::Data(vec![1, 0, 2, 0]),
                Reply::Stall,
                Reply::Data(vreg_reply()),
            ]),
        );
        let s = ExtState::probe(&mut t).unwrap();
        assert_eq!(s.supported, vec![1, 2]);
        assert_eq!(s.pair_map, None);
        assert!(s.core_vreg.is_some());
    }

    #[test]
    fn wp_s3_unknown_ids_are_listed_but_not_read() {
        let mut t = MockTransport::new().data(op::REQ_EXTENSION, vec![3, 0, 1, 0x80]);
        let log = t.log_handle();
        let s = ExtState::probe(&mut t).unwrap();
        assert_eq!(s.supported, vec![3, 0x8001]);
        assert_eq!((s.pair_map, s.core_vreg), (None, None));
        assert_eq!(log.lock().unwrap().len(), 1);
    }

    #[test]
    fn wp_s3_a_disconnect_is_an_error() {
        let mut t = MockTransport::new().reply(op::REQ_EXTENSION, Reply::Disconnect);
        assert!(matches!(
            ExtState::probe(&mut t),
            Err(TransportError::Disconnected)
        ));

        // Mid-probe too: the list answers, then the device goes away.
        let mut t = MockTransport::new().reply(
            op::REQ_EXTENSION,
            Reply::Sequence(vec![Reply::Data(vec![1, 0, 2, 0]), Reply::Disconnect]),
        );
        assert!(matches!(
            ExtState::probe(&mut t),
            Err(TransportError::Disconnected)
        ));
    }
}
