//! WP C8: the extension channel through `DspiCore`, against `FakeDevice`.
//!
//! Refusals are proven by reading the device back, never by a setter's
//! result alone (wave-2b note: with `ack_unknown_sets` a refused SET looks
//! like success).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dspi_proto::generated::ext as g;
use dspi_proto::generated::opcodes as op;
use dspi_session::ext::{ExtError, PairMap};
use dspi_transport::{
    Capture, DeviceDescriptor, FakeDevice, FakeOptions, FakeRemote, NotificationSource, Transport,
    TransportCaps, TransportError,
};
use proptest::prelude::*;

use super::*;
use crate::consts::EXT_WRITE_REFUSED;

/// Faults a [`Counting`] transport injects in front of the fake.
#[derive(Default, Clone, Copy)]
pub(crate) struct Faults {
    /// Refuse `REQ_EXTENSION` reads with `Denied` (a DSPi Link viewer), for
    /// sub-commands at or above this id (0: every read, `EXT_LIST`
    /// included).
    pub deny_ext_reads_from: Option<u16>,
    /// Stall every `REQ_EXTENSION` OUT (the device refusing a request).
    pub stall_ext_writes: bool,
    /// ACK every `REQ_EXTENSION` OUT without applying it (the firmware
    /// ignoring an invalid pair map).
    pub ignore_ext_writes: bool,
    /// Stall every IN of these opcodes (firmware without the feature).
    pub stall_reads: &'static [u8],
}

/// A `FakeDevice` that counts its transfers, so a test can prove that a
/// refused call sent nothing, and injects [`Faults`].
pub(crate) struct Counting {
    dev: FakeDevice,
    faults: Faults,
    /// IN transfers so far.
    pub ins: Arc<AtomicUsize>,
    /// OUT transfers so far.
    pub outs: Arc<AtomicUsize>,
}

impl Counting {
    pub(crate) fn new(dev: FakeDevice, faults: Faults) -> Self {
        Self {
            dev,
            faults,
            ins: Arc::default(),
            outs: Arc::default(),
        }
    }

    fn refuse(&self, opcode: u8, value: u16) -> Option<TransportError> {
        if self.faults.stall_reads.contains(&opcode) {
            return Some(TransportError::Stalled { opcode });
        }
        self.denied(opcode, value)
            .then(|| TransportError::Denied("config".into()))
    }

    fn denied(&self, opcode: u8, value: u16) -> bool {
        opcode == op::REQ_EXTENSION && self.faults.deny_ext_reads_from.is_some_and(|f| value >= f)
    }
}

impl Transport for Counting {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.ins.fetch_add(1, Ordering::SeqCst);
        if let Some(e) = self.refuse(opcode, value) {
            return Err(e);
        }
        self.dev.control_in(opcode, value, len)
    }
    fn control_in_upto(
        &mut self,
        opcode: u8,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        self.ins.fetch_add(1, Ordering::SeqCst);
        if let Some(e) = self.refuse(opcode, value) {
            return Err(e);
        }
        self.dev.control_in_upto(opcode, value, max_len)
    }
    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        self.outs.fetch_add(1, Ordering::SeqCst);
        if opcode == op::REQ_EXTENSION && self.faults.stall_ext_writes {
            return Err(TransportError::Stalled { opcode });
        }
        if opcode == op::REQ_EXTENSION && self.faults.ignore_ext_writes {
            return Ok(());
        }
        self.dev.control_out(opcode, value, data)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        self.dev.descriptor()
    }
    fn max_transfer(&self) -> usize {
        self.dev.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.dev.notifications()
    }
    fn caps(&self) -> TransportCaps {
        self.dev.caps()
    }
}

/// A probed device: the demo capture with `options`, behind [`Counting`].
pub(crate) struct Probed {
    pub core: Arc<DspiCore>,
    pub remote: FakeRemote,
    pub ins: Arc<AtomicUsize>,
    pub outs: Arc<AtomicUsize>,
}

impl Probed {
    pub(crate) fn traffic(&self) -> (usize, usize) {
        (
            self.ins.load(Ordering::SeqCst),
            self.outs.load(Ordering::SeqCst),
        )
    }
}

pub(crate) fn probed(bulk: Vec<u8>, options: FakeOptions, faults: Faults) -> Probed {
    let capture = Capture {
        platform: Capture::demo().platform,
        bulk,
    };
    let dev = FakeDevice::new(capture, options);
    let remote = dev.remote();
    let t = Counting::new(dev, faults);
    let (ins, outs) = (t.ins.clone(), t.outs.clone());
    let core = DspiCore::new();
    core.attach_probed(Box::new(t), "C8".into(), 0).unwrap();
    Probed {
        core,
        remote,
        ins,
        outs,
    }
}

/// The K340 build: the extension channel on, unknown SETs ACKed as on USB.
pub(crate) fn k340_options() -> FakeOptions {
    FakeOptions {
        extensions: true,
        ack_unknown_sets: true,
        ..FakeOptions::default()
    }
}

pub(crate) fn k340() -> Probed {
    probed(Capture::demo().bulk, k340_options(), Faults::default())
}

/// The demo image relabelled as wire format `v`, cut or extended to `len`
/// bytes: another firmware's image, so the connection is read-only.
pub(crate) fn image_as(v: u8, len: usize) -> Vec<u8> {
    let mut b = Capture::demo().bulk;
    b.resize(len, 0);
    b[0] = v;
    b[6..8].copy_from_slice(&(len as u16).to_le_bytes());
    b
}

/// The pair map in the device's own bulk image.
fn device_map(remote: &FakeRemote) -> Vec<u8> {
    let mut s = state::default_state();
    state::decode_bulk(&remote.image(), &mut s).unwrap();
    s.crossfeed_pair_map
}

/// The pair map in the mirror's `bulk_raw`.
fn mirror_image_map(core: &DspiCore) -> Vec<u8> {
    let mut s = state::default_state();
    state::decode_bulk(&core.get_state().bulk_raw, &mut s).unwrap();
    s.crossfeed_pair_map
}

const K340_MAP: [u8; 4] = [0x02, 0x13, 0, 0];

// ------------------------------------------------------------ status

#[test]
fn wp_c8_ext_status_reports_the_connect_probe() {
    let p = k340();
    let before = p.traffic();
    let s = p.core.ext_status().unwrap();
    assert_eq!(
        s.supported,
        vec![g::EXT_CROSSFEED_PAIR_MAP, g::EXT_CORE_VREG]
    );
    assert!(s.has_pair_map && s.has_core_vreg);
    assert_eq!(p.traffic(), before, "ext_status sends nothing");

    let core = DspiCore::new();
    assert_eq!(core.ext_status(), Err(DspiError::Disconnected));
    assert_eq!(
        core.fetch_crossfeed_pair_map(),
        Err(DspiError::Disconnected)
    );
    assert_eq!(
        core.set_crossfeed_pair_map(K340_MAP.to_vec()),
        Err(DspiError::Disconnected)
    );
    assert_eq!(core.fetch_core_vreg(), Err(DspiError::Disconnected));
    assert_eq!(core.core_vreg_op(4, 0), Err(DspiError::Disconnected));
}

#[test]
fn wp_c8_stock_firmware_has_no_extensions_and_nothing_is_sent() {
    let p = probed(
        Capture::demo().bulk,
        FakeOptions {
            ack_unknown_sets: true,
            ..FakeOptions::default()
        },
        Faults::default(),
    );
    assert_eq!(p.core.ext_status().unwrap(), ExtStatus::default());
    let before = p.traffic();
    assert_eq!(
        p.core.fetch_crossfeed_pair_map(),
        Err(DspiError::Unsupported)
    );
    assert_eq!(
        p.core.set_crossfeed_pair_map(K340_MAP.to_vec()),
        Err(DspiError::Unsupported)
    );
    assert_eq!(p.core.fetch_core_vreg(), Err(DspiError::Unsupported));
    assert_eq!(p.core.core_vreg_op(0, 0x0E), Err(DspiError::Unsupported));
    assert_eq!(p.traffic(), before);
    // The mirror keeps the image's (default) map.
    assert_eq!(p.core.get_state().crossfeed_pair_map, vec![0; 4]);
}

#[test]
fn wp_c8_a_link_viewer_is_treated_as_having_no_extensions() {
    // The hub refuses REQ_EXTENSION to a viewer: absent, not an error.
    let p = probed(
        Capture::demo().bulk,
        k340_options(),
        Faults {
            deny_ext_reads_from: Some(0),
            ..Faults::default()
        },
    );
    assert_eq!(p.core.ext_status().unwrap(), ExtStatus::default());
    assert_eq!(p.core.fetch_core_vreg(), Err(DspiError::Unsupported));
}

#[test]
fn wp_c8_a_listed_sub_command_whose_read_failed_is_absent() {
    // EXT_LIST answers, the sub-command reads are refused: `has` is true but
    // the values are None, and the Option decides.
    let p = probed(
        Capture::demo().bulk,
        k340_options(),
        Faults {
            deny_ext_reads_from: Some(g::EXT_CROSSFEED_PAIR_MAP),
            ..Faults::default()
        },
    );
    {
        let mut inner = p.core.lock();
        let ext = &inner.conn().unwrap().ext;
        assert!(ext.has(g::EXT_CROSSFEED_PAIR_MAP) && ext.pair_map.is_none());
    }
    let s = p.core.ext_status().unwrap();
    assert_eq!(s.supported, vec![1, 2]);
    assert!(!s.has_pair_map && !s.has_core_vreg);
    let before = p.traffic();
    assert_eq!(
        p.core.fetch_crossfeed_pair_map(),
        Err(DspiError::Unsupported)
    );
    assert_eq!(
        p.core.set_crossfeed_pair_map(K340_MAP.to_vec()),
        Err(DspiError::Unsupported)
    );
    assert_eq!(p.core.fetch_core_vreg(), Err(DspiError::Unsupported));
    assert_eq!(p.core.core_vreg_op(4, 0), Err(DspiError::Unsupported));
    assert_eq!(p.traffic(), before);
    assert!(!pair_map_supported(&mut p.core.lock()));
}

// ------------------------------------------------------------ pair map

#[test]
fn wp_c8_the_k340_pair_map_round_trips_into_the_mirror() {
    let p = k340();
    assert!(pair_map_supported(&mut p.core.lock()));
    assert_eq!(p.core.fetch_crossfeed_pair_map().unwrap(), vec![0; 4]);

    p.core.set_crossfeed_pair_map(K340_MAP.to_vec()).unwrap();
    assert_eq!(device_map(&p.remote), K340_MAP);
    assert_eq!(p.core.get_state().crossfeed_pair_map, K340_MAP);
    assert_eq!(mirror_image_map(&p.core), K340_MAP, "bulk_raw patched");
    assert_eq!(p.core.fetch_crossfeed_pair_map().unwrap(), K340_MAP);
    {
        let mut inner = p.core.lock();
        assert_eq!(inner.conn().unwrap().ext.pair_map, Some(PairMap(K340_MAP)));
    }

    // Explicit default bytes go out normalized and read back as zeros.
    p.core
        .set_crossfeed_pair_map(vec![0x10, 0x32, 0, 0])
        .unwrap();
    assert_eq!(device_map(&p.remote), vec![0; 4]);
    assert_eq!(p.core.get_state().crossfeed_pair_map, vec![0; 4]);
}

#[test]
fn wp_c8_an_invalid_pair_map_is_refused_before_any_traffic() {
    let p = k340();
    p.core.set_crossfeed_pair_map(K340_MAP.to_vec()).unwrap();
    let before = p.traffic();
    for bad in [
        vec![0x10, 0x10, 0, 0],       // output 0 and 1 in two pairs
        vec![0x00, 0x00, 0x00, 0x88], // the PDM sub (output 8)
        vec![0x11, 0, 0, 0],          // one output twice in a pair
        vec![0x02, 0x13, 0],          // three bytes
        vec![0, 0, 0, 0, 0],          // five bytes
    ] {
        assert!(
            matches!(
                p.core.set_crossfeed_pair_map(bad.clone()),
                Err(DspiError::InvalidArgument { .. })
            ),
            "{bad:02x?}"
        );
    }
    assert_eq!(p.traffic(), before, "nothing sent");
    assert_eq!(device_map(&p.remote), K340_MAP);
    assert_eq!(p.core.get_state().crossfeed_pair_map, K340_MAP);
}

#[test]
fn wp_c8_an_rp2040_takes_only_two_pairs() {
    let p = k340();
    // The mirror says RP2040: pairs 2 and 3 must stay 0.
    p.core.lock().state.platform_id = 0;
    let before = p.traffic();
    assert!(matches!(
        p.core.set_crossfeed_pair_map(vec![0x02, 0x13, 0x54, 0]),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert_eq!(p.traffic(), before);
    p.core
        .set_crossfeed_pair_map(vec![0x01, 0x23, 0, 0])
        .unwrap();
    assert_eq!(device_map(&p.remote), vec![0x01, 0x23, 0, 0]);
}

#[test]
fn wp_c8_a_map_the_device_ignores_is_reported_and_the_mirror_follows_the_device() {
    // The firmware ACKs a map it will not take and keeps the old one
    // (crossfeed_map.h): the read-back catches it.
    let p = probed(
        Capture::demo().bulk,
        k340_options(),
        Faults {
            ignore_ext_writes: true,
            ..Faults::default()
        },
    );
    p.core.lock().state.crossfeed_pair_map = K340_MAP.to_vec();
    assert!(matches!(
        p.core.set_crossfeed_pair_map(K340_MAP.to_vec()),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert_eq!(device_map(&p.remote), vec![0; 4]);
    assert_eq!(
        p.core.get_state().crossfeed_pair_map,
        vec![0; 4],
        "the device's map"
    );
}

#[test]
fn wp_c8_a_pair_map_write_the_device_stalls_is_refused() {
    let p = probed(
        Capture::demo().bulk,
        k340_options(),
        Faults {
            stall_ext_writes: true,
            ..Faults::default()
        },
    );
    assert_eq!(
        p.core.set_crossfeed_pair_map(K340_MAP.to_vec()),
        Err(DspiError::Refused {
            status: EXT_WRITE_REFUSED
        })
    );
    // Read back: unchanged on the device and in the mirror.
    assert_eq!(device_map(&p.remote), vec![0; 4]);
    assert_eq!(p.core.fetch_crossfeed_pair_map().unwrap(), vec![0; 4]);
    assert_eq!(p.core.get_state().crossfeed_pair_map, vec![0; 4]);
}

#[test]
fn wp_c8_a_read_only_connection_never_writes_the_extension_channel() {
    let p = probed(image_as(33, 6200), k340_options(), Faults::default());
    assert!(!p.core.lock().conn().unwrap().writable());
    // Reads work.
    assert!(p.core.ext_status().unwrap().has_pair_map);
    assert_eq!(p.core.fetch_crossfeed_pair_map().unwrap(), vec![0; 4]);
    let vreg = p.core.fetch_core_vreg().unwrap();

    let outs = p.traffic().1;
    assert_eq!(
        p.core.set_crossfeed_pair_map(K340_MAP.to_vec()),
        Err(DspiError::ReadOnly)
    );
    assert_eq!(p.core.core_vreg_op(0, 0x0E), Err(DspiError::ReadOnly));
    assert_eq!(p.core.core_vreg_op(3, 0), Err(DspiError::ReadOnly));
    assert_eq!(p.traffic().1, outs, "no OUT transfer");
    // Proven by read-back.
    assert_eq!(p.core.fetch_crossfeed_pair_map().unwrap(), vec![0; 4]);
    assert_eq!(p.core.fetch_core_vreg().unwrap(), vreg);
}

// ------------------------------------------------------------ core voltage

#[test]
fn wp_c8_core_voltage_ops_apply_and_read_back() {
    let p = k340();
    let v = p.core.fetch_core_vreg().unwrap();
    assert_eq!((v.live_vreg, v.default_vreg, v.stored_sel), (0x0D, 0x0D, 0));
    assert_eq!(v.millivolts, 1200);

    p.core.core_vreg_op(0, 0x0E).unwrap();
    let v = p.core.fetch_core_vreg().unwrap();
    assert_eq!(
        (v.stored_sel, v.live_vreg, v.millivolts),
        (0x0E, 0x0E, 1250)
    );

    p.core.core_vreg_op(1, 0x0F).unwrap();
    let v = p.core.fetch_core_vreg().unwrap();
    assert!(v.trial_active);
    assert_eq!((v.trial_vreg, v.live_vreg), (0x0F, 0x0F));
    p.core.core_vreg_op(2, 0).unwrap();
    let v = p.core.fetch_core_vreg().unwrap();
    assert!(!v.trial_active);
    assert_eq!(v.live_vreg, 0x0E);

    p.core.core_vreg_op(3, 0).unwrap();
    assert!(p.core.fetch_core_vreg().unwrap().auto_disabled);
    p.core.core_vreg_op(3, 1).unwrap();
    assert!(!p.core.fetch_core_vreg().unwrap().auto_disabled);

    // Back to the build default.
    p.core.core_vreg_op(0, 0).unwrap();
    let v = p.core.fetch_core_vreg().unwrap();
    assert_eq!((v.stored_sel, v.live_vreg), (0, 0x0D));
    p.core.core_vreg_op(4, 0).unwrap();
    // The connection's copy follows the last read.
    let mut inner = p.core.lock();
    let ext = &inner.conn().unwrap().ext;
    assert_eq!(ext.core_vreg.as_ref().map(|s| s.stored_sel), Some(0));
}

#[test]
fn wp_c8_core_voltage_requests_outside_the_limits_are_refused_before_any_traffic() {
    let p = k340();
    p.core.core_vreg_op(0, 0x0E).unwrap();
    let before = p.traffic();
    for (op, value) in [
        (0, 0x0C), // below the build default (0x0D), the floor
        (0, 0x10), // above 1.30 V
        (1, 0x0B), // trial below 1.15 V
        (1, 0x10),
        (3, 2), // auto takes 0/1
        (5, 0), // unknown op
        (0xFF, 0),
    ] {
        assert!(
            matches!(
                p.core.core_vreg_op(op, value),
                Err(DspiError::InvalidArgument { .. })
            ),
            "op {op} value {value:#x}"
        );
    }
    assert_eq!(p.traffic(), before, "nothing sent");
    let v = p.core.fetch_core_vreg().unwrap();
    assert_eq!(
        (v.stored_sel, v.live_vreg, v.trial_active),
        (0x0E, 0x0E, false)
    );
    assert!(!v.auto_disabled);
}

#[test]
fn wp_c8_a_core_voltage_request_the_device_stalls_is_refused() {
    let p = probed(
        Capture::demo().bulk,
        k340_options(),
        Faults {
            stall_ext_writes: true,
            ..Faults::default()
        },
    );
    assert_eq!(
        p.core.core_vreg_op(0, 0x0E),
        Err(DspiError::Refused {
            status: EXT_WRITE_REFUSED
        })
    );
    assert_eq!(p.core.fetch_core_vreg().unwrap().stored_sel, 0);
}

// ------------------------------------------------------------ pure parts

#[test]
fn wp_c8_extension_errors_map_to_typed_errors() {
    let denied = || ExtError::Transport(TransportError::Denied("config".into()));
    assert_eq!(
        read_error(ExtError::Unsupported { id: 1 }),
        DspiError::Unsupported
    );
    assert_eq!(read_error(denied()), DspiError::Unsupported);
    assert_eq!(
        read_error(ExtError::Transport(TransportError::Disconnected)),
        DspiError::Disconnected
    );
    assert!(matches!(
        read_error(ExtError::Transport(TransportError::ShortRead {
            opcode: 0,
            wanted: 4,
            got: 3
        })),
        DspiError::Transfer { .. }
    ));
    assert_eq!(write_error(denied()), DspiError::Denied);
    assert_eq!(
        write_error(ExtError::Refused { id: 2 }),
        DspiError::Refused {
            status: EXT_WRITE_REFUSED
        }
    );
    assert_eq!(
        write_error(ExtError::Transport(TransportError::Locked)),
        DspiError::Locked
    );
}

#[test]
fn wp_c8_vreg_requests_encode_as_the_spec_says() {
    assert_eq!(vreg_request(0, 0, 0x0D).unwrap(), VregOp::Set(0));
    assert_eq!(vreg_request(0, 0x0D, 0x0D).unwrap(), VregOp::Set(0x0D));
    assert_eq!(vreg_request(0, 0x0C, 0x0C).unwrap(), VregOp::Set(0x0C));
    assert_eq!(vreg_request(1, 0x0C, 0x0D).unwrap(), VregOp::Trial(0x0C));
    assert_eq!(vreg_request(2, 0x55, 0x0D).unwrap(), VregOp::TrialEnd);
    assert_eq!(vreg_request(3, 1, 0x0D).unwrap(), VregOp::Auto(true));
    assert_eq!(vreg_request(4, 9, 0x0D).unwrap(), VregOp::Clear);
}

proptest! {
    #[test]
    fn proptest_wp_c8_vreg_requests_stay_in_the_spec_limits(
        op in any::<u8>(), value in any::<u8>(), floor in any::<u8>()
    ) {
        let r = vreg_request(op, value, floor);
        if op > 4 {
            prop_assert!(r.is_err());
        }
        if let Ok(r) = r {
            match r {
                VregOp::Set(v) => prop_assert!(v == 0 || (v >= floor && v <= 0x0F)),
                VregOp::Trial(v) => prop_assert!((0x0C..=0x0F).contains(&v)),
                VregOp::Auto(_) => prop_assert!(value <= 1),
                VregOp::TrialEnd | VregOp::Clear => prop_assert!(op == 2 || op == 4),
            }
        }
    }
}
