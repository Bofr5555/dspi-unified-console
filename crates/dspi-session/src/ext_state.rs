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

use dspi_proto::ext::alink::{self, AlinkInfo};
use dspi_proto::ext::alink_usb; // -- WP AUSB1 --
use dspi_proto::ext::{CoreVregStatus, LIST_READ_LEN, PairMap, ResetInfo, decode_list};
use dspi_proto::generated::ext as g;
use dspi_proto::generated::opcodes as op;
use dspi_transport::{InRead, Transport, TransportError};

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
    /// `EXT_NOTIFY_FEATURES`, when supported and readable: the
    /// `NOTIFY_FEAT_*` flags. See [`ExtState::bulk_source`].
    pub notify_features: Option<u32>,
    // -- WP FWX --
    /// `EXT_RESET_INFO`, when supported and readable: why the device last
    /// reset and how long the run before lasted, as it was at connect (its
    /// uptime keeps counting; re-read with `ext::read_reset_info`).
    pub reset_info: Option<ResetInfo>,
    // -- WP SW1 --
    /// `EXT_ALINK_INFO` (0x8000), when listed and readable: the unit's role on
    /// the audio link. Read only when `EXT_LIST` names it, so firmware
    /// without the alink sends nothing more.
    pub alink_info: Option<AlinkInfo>,
}

/// `wLength` of the `EXT_LIST` read: 64 bytes, room for 32 ids. The
/// conversion is checked at compile time.
/// `NOTIFY_FEAT_BULK_SOURCE` (notify.h), a `(1u << 0)` the header generator
/// does not lift; this module's tests pin it to the header.
const NOTIFY_FEAT_BULK_SOURCE: u32 = 1 << 0;

/// The reply size of `EXT_NOTIFY_FEATURES`: a `uint32`.
const NOTIFY_FEATURES_LEN: u16 = 4;

const LIST_LEN: u16 = {
    assert!(LIST_READ_LEN <= u16::MAX as usize);
    LIST_READ_LEN as u16
};

impl ExtState {
    /// Probe: `EXT_LIST`, then each supported sub-command this build knows.
    /// A stall, a short reply or a DSPi Link hub refusing this client (DSPi Link
    /// `DENIED`: the hub classes `REQ_EXTENSION` as `config`, which viewer
    /// and control roles may not use) mean "absent": on `EXT_LIST` they give
    /// the empty state, never an error. Only a transport failure (disconnect,
    /// timeout, a lock, a transport fault) is returned.
    ///
    /// One transfer per read and no retries: a stall is the answer. A listed
    /// sub-command whose read is absent is left `None` (it stays in
    /// `supported`, which is what the device claimed).
    pub fn probe(t: &mut dyn Transport) -> Result<Self, TransportError> {
        let r = Self::list_read();
        let list = t.control_in_upto(r.opcode, r.value, r.len);
        let reads = Self::follow_up(&list);
        let answers = if reads.is_empty() {
            Vec::new()
        } else {
            t.control_in_batch(&reads)
        };
        Self::from_answers(list, answers)
    }

    // -- WP LK4 --
    /// The `EXT_LIST` read of [`ExtState::probe`], for a caller that sends it
    /// in a batch with others.
    pub fn list_read() -> InRead {
        InRead::upto(op::REQ_EXTENSION, g::EXT_LIST, LIST_LEN)
    }

    /// The reads that follow `EXT_LIST`'s answer `list`: none when it was
    /// absent or failed.
    pub fn follow_up(list: &Result<Vec<u8>, TransportError>) -> Vec<InRead> {
        let Ok(list) = list else {
            return Vec::new();
        };
        reads_for(&supported_from_list(list))
            .into_iter()
            .map(|(id, len)| InRead::exact(op::REQ_EXTENSION, id, len))
            .collect()
    }

    /// The state from `EXT_LIST`'s answer and the answers to
    /// [`ExtState::follow_up`]'s reads, in their order: as
    /// [`ExtState::probe`] would build it, with the same errors.
    pub fn from_answers(
        list: Result<Vec<u8>, TransportError>,
        answers: Vec<Result<Vec<u8>, TransportError>>,
    ) -> Result<Self, TransportError> {
        let list = match list {
            Ok(b) => b,
            Err(e) => {
                absent(e)?;
                return Ok(Self::default());
            }
        };
        let supported = supported_from_list(&list);
        let mut replies = Vec::new();
        for ((id, _), answer) in reads_for(&supported).into_iter().zip(answers) {
            match answer {
                Ok(b) => replies.push((id, b)),
                Err(e) => absent(e)?,
            }
        }
        Ok(from_replies(supported, &replies))
    }

    /// Whether `EXT_LIST` named sub-command `id`. This can be true while the
    /// matching field (`pair_map`, `core_vreg`) is `None`, when the listed
    /// read was absent (stalled, short or refused), so callers check the
    /// `Option` rather than relying on `has`.
    pub fn has(&self, id: u16) -> bool {
        self.supported.contains(&id)
    }

    /// Whether the firmware tags a bulk SET's `BULK_INVALIDATED` with its
    /// dispatch path (`NOTIFY_FEAT_BULK_SOURCE`). False when `0x0003` is not
    /// listed or its read was refused: a viewer on DSPi Link cannot read the
    /// extension channel at all.
    pub fn bulk_source(&self) -> bool {
        self.notify_features
            .is_some_and(|f| f & NOTIFY_FEAT_BULK_SOURCE != 0)
    }

    // -- WP FWX --
    /// Whether the device sends the clip event (`NOTIFY_FEAT_CLIP_EVENT`):
    /// then `dspi_proto::ext::clip_event` reads its 0xE0 packets. False
    /// without `EXT_NOTIFY_FEATURES`, as for [`ExtState::bulk_source`].
    pub fn clip_events(&self) -> bool {
        self.notify_features
            .is_some_and(|f| f & dspi_proto::ext::NOTIFY_FEAT_CLIP_EVENT != 0)
    }

    // -- WP SW1 --
    /// Whether the unit has the audio link: `EXT_ALINK_INFO` was listed and
    /// read, with a role and the protocol version this build speaks
    /// (`extension_channel_spec.md` 2.8.13). The other alink sub-commands are
    /// used only when this is true and `EXT_LIST` names them.
    pub fn alink(&self) -> bool {
        self.alink_info.is_some_and(|i| i.usable())
    }

    /// Whether the unit takes audio from an alink, so input source 7 (`INPUT_SOURCE_ALINK`)
    /// means the alink input.
    pub fn alink_input(&self) -> bool {
        self.alink_info
            .is_some_and(|i| i.usable() && i.role.receives())
    }

    /// Whether the device sends alink state events (0xE1, bit 2).
    pub fn alink_state_events(&self) -> bool {
        self.notify_features
            .is_some_and(|f| f & alink::NOTIFY_FEAT_ALINK_STATE != 0)
    }

    /// Whether the device sends apply-result events (0xE2, bit 3).
    pub fn apply_result_events(&self) -> bool {
        self.notify_features
            .is_some_and(|f| f & alink::NOTIFY_FEAT_APPLY_RESULT != 0)
    }

    // -- WP AUSB1 --
    /// Whether the unit has the alink's USB-direct block: `EXT_LIST` names
    /// `EXT_ALINK_USB` and `EXT_ALINK_USB_STATUS` (2.9, "Discovery"). Fork
    /// firmware without it and stock firmware send nothing more.
    pub fn usb_direct(&self) -> bool {
        self.has(alink_usb::EXT_ALINK_USB) && self.has(alink_usb::EXT_ALINK_USB_STATUS)
    }

    /// Whether the device sends USB-direct events (0xE3, bit 4).
    pub fn alink_usb_events(&self) -> bool {
        self.notify_features
            .is_some_and(|f| f & alink_usb::NOTIFY_FEAT_ALINK_USB != 0)
    }

    /// Whether `EXT_FRAME_COUNTER` and `EXT_APPLY_AT_FRAME` may be used: on a
    /// unit with the alink or the USB-direct block, when `EXT_LIST` names `id`
    /// (a USB-direct build without the alink lists both, 2.9).
    pub fn time_base_id(&self, id: u16) -> bool {
        (self.alink() || self.usb_direct()) && self.has(id)
    }
}

/// Sorts a failed read into "absent" (`Ok`) or a transport failure (`Err`).
/// A stall, a short reply or a DSPi Link hub refusing this client mean "absent": a
/// stall is the device saying "not supported", a short reply is an answer
/// that is not usable, and a DSPi Link `DENIED` is the hub's policy for this
/// client's role (not a fault of the transport). Everything else (disconnect,
/// timeout, a lock, a transport fault) goes back to the caller.
fn absent(e: TransportError) -> Result<(), TransportError> {
    match e {
        TransportError::Stalled { .. }
        | TransportError::ShortRead { .. }
        | TransportError::Denied(_) => Ok(()),
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
    // The reply sizes as `wLength`; the conversions are checked at compile
    // time, as for `LIST_LEN`.
    const PAIR_MAP_LEN: u16 = {
        assert!(PairMap::SIZE <= u16::MAX as usize);
        PairMap::SIZE as u16
    };
    const CORE_VREG_LEN: u16 = {
        assert!(CoreVregStatus::SIZE <= u16::MAX as usize);
        CoreVregStatus::SIZE as u16
    };
    // -- WP FWX --
    const RESET_INFO_LEN: u16 = {
        assert!(ResetInfo::SIZE <= u16::MAX as usize);
        ResetInfo::SIZE as u16
    };
    // -- WP SW1 --
    const ALINK_INFO_LEN: u16 = {
        assert!(AlinkInfo::SIZE <= u16::MAX as usize);
        AlinkInfo::SIZE as u16
    };
    const KNOWN: [(u16, u16); 5] = [
        (g::EXT_CROSSFEED_PAIR_MAP, PAIR_MAP_LEN),
        (g::EXT_CORE_VREG, CORE_VREG_LEN),
        (g::EXT_NOTIFY_FEATURES, NOTIFY_FEATURES_LEN),
        (g::EXT_RESET_INFO, RESET_INFO_LEN),
        // -- WP SW1 --
        (alink::EXT_ALINK_INFO, ALINK_INFO_LEN),
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
    let notify_features = reply(g::EXT_NOTIFY_FEATURES)
        .and_then(|b| b.first_chunk::<4>())
        .map(|b| u32::from_le_bytes(*b));
    // -- WP FWX --
    let reset_info = reply(g::EXT_RESET_INFO).and_then(ResetInfo::decode);
    // -- WP SW1 --
    let alink_info = reply(alink::EXT_ALINK_INFO).and_then(AlinkInfo::decode);
    ExtState {
        supported,
        pair_map,
        core_vreg,
        notify_features,
        reset_info,
        alink_info,
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_s3 {
    use super::*;
    use dspi_transport::MockTransport;
    use dspi_transport::mock::{Direction, Reply};
    use proptest::prelude::*;

    #[test]
    fn wp_b2_the_feature_bit_is_the_headers() {
        let header = include_str!("../../dspi-proto/firmware/notify.h");
        assert!(header.contains("#define NOTIFY_FEAT_BULK_SOURCE      (1u << 0)"));
        assert_eq!(NOTIFY_FEAT_BULK_SOURCE, 1);
    }

    #[test]
    fn wp_b2_notify_features_are_probed_when_listed() {
        let list = [g::EXT_NOTIFY_FEATURES.to_le_bytes()].concat();
        let t = MockTransport::new().reply(
            op::REQ_EXTENSION,
            Reply::Sequence(vec![Reply::Data(list), Reply::Data(vec![1, 0, 0, 0])]),
        );
        let mut t = t;
        let e = ExtState::probe(&mut t).unwrap();
        assert_eq!(e.notify_features, Some(1));
        assert!(e.bulk_source());

        // Listed, flag clear; listed, refused; not listed.
        let list = [g::EXT_NOTIFY_FEATURES.to_le_bytes()].concat();
        let e = from_replies(
            supported_from_list(&list),
            &[(g::EXT_NOTIFY_FEATURES, vec![0, 0, 0, 0])],
        );
        assert!(!e.bulk_source());
        let e = from_replies(supported_from_list(&list), &[]);
        assert_eq!(e.notify_features, None);
        assert!(!e.bulk_source());
        let e = from_replies(Vec::new(), &[(g::EXT_NOTIFY_FEATURES, vec![1, 0, 0, 0])]);
        assert!(!e.bulk_source(), "a reply for an id not listed is ignored");
        // A short reply is absent, not a panic.
        let e = from_replies(
            supported_from_list(&list),
            &[(g::EXT_NOTIFY_FEATURES, vec![1])],
        );
        assert!(!e.bulk_source());
    }

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
        assert_eq!(reads_for(&[1, 2, 4, 0x8001]), vec![(1, 4), (2, 16)]);
        assert_eq!(reads_for(&[3]), vec![(3, 4)]);
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
        let mut t = MockTransport::new().data(op::REQ_EXTENSION, vec![4, 0, 1, 0x80]);
        let log = t.log_handle();
        let s = ExtState::probe(&mut t).unwrap();
        assert_eq!(s.supported, vec![4, 0x8001]);
        assert_eq!((s.pair_map, s.core_vreg), (None, None));
        assert_eq!(log.lock().unwrap().len(), 1);
    }

    /// Answers every IN request from a script of results, for errors
    /// `MockTransport` cannot produce (it has no reply carrying an arbitrary
    /// `TransportError`, such as `Denied`).
    struct Scripted {
        replies: std::collections::VecDeque<Result<Vec<u8>, TransportError>>,
        reads: usize,
        desc: dspi_transport::DeviceDescriptor,
    }

    impl Scripted {
        fn new(replies: Vec<Result<Vec<u8>, TransportError>>) -> Self {
            Self {
                replies: replies.into(),
                reads: 0,
                desc: dspi_transport::DeviceDescriptor {
                    serial: "S3TEST".into(),
                    bus_id: "0".into(),
                    address: 0,
                    ..Default::default()
                },
            }
        }
    }

    impl Transport for Scripted {
        fn control_in(&mut self, _: u8, _: u16, _: u16) -> dspi_transport::Result<Vec<u8>> {
            self.reads += 1;
            self.replies
                .pop_front()
                .unwrap_or(Err(TransportError::Disconnected))
        }
        fn control_out(&mut self, _: u8, _: u16, _: &[u8]) -> dspi_transport::Result<()> {
            Err(TransportError::Disconnected)
        }
        fn descriptor(&self) -> &dspi_transport::DeviceDescriptor {
            &self.desc
        }
    }

    #[test]
    fn wp_s3_a_denied_read_is_absent() {
        // A viewer on DSPi Link: the hub refuses REQ_EXTENSION outright.
        let mut t = Scripted::new(vec![Err(TransportError::Denied("config".into()))]);
        assert_eq!(ExtState::probe(&mut t).unwrap(), ExtState::default());
        assert_eq!(t.reads, 1);

        // Refused for one sub-command only: that field is absent, the rest
        // of the probe goes on.
        let mut t = Scripted::new(vec![
            Ok(vec![1, 0, 2, 0]),
            Err(TransportError::Denied("config".into())),
            Ok(vreg_reply()),
        ]);
        let s = ExtState::probe(&mut t).unwrap();
        assert_eq!(s.supported, vec![1, 2]);
        assert!(s.has(g::EXT_CROSSFEED_PAIR_MAP) && s.pair_map.is_none());
        assert!(s.core_vreg.is_some());
        assert_eq!(t.reads, 3);

        // A lock is not a refusal of this client's role: it is returned.
        let mut t = Scripted::new(vec![Err(TransportError::Locked)]);
        assert!(matches!(
            ExtState::probe(&mut t),
            Err(TransportError::Locked)
        ));
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

// -- WP FWX --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_fwx;

// -- WP SW1 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_sw1;
