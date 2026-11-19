//! WP ATPROTO: the time-base helpers against the spec's examples on a
//! scripted transport, and through every firmware generation of the fake:
//! the same results on G1b (`REQ_EXTENSION`) and G2b (`Bofr117`, the tunnel),
//! a local refusal with nothing sent on G1s and G2s.
#![allow(clippy::arithmetic_side_effects)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use dspi_proto::ext::alink::ApplyResult;
use dspi_proto::ext::decode_list;
use dspi_proto::ext::timebase::*;
use dspi_proto::paged::{CmdId, EXT_TUNNEL};
use dspi_transport::fake::FakeGeneration;
use dspi_transport::mock::{Direction, Reply};
use dspi_transport::{
    DeviceDescriptor, FakeDevice, MockTransport, NotificationSource, TransportCaps, paged,
};

use super::*;

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// 2.11.1's example.
const TIME_BASE: &str = "01 02 00 00 00 e1 f5 05 00 3e 49 00 \
    01 00 03 01 00 00 00 01 00 b4 c4 04 00 00 00 00 01 00 c0 00 00 00 00 3e \
    03 01 03 0b 42 37 03 03 00 a6 0e 00 00 00 00 00 04 00 00 00 00 00 00 00 \
    00 00 00 00";
/// 2.11.2's example.
const TIMELINE: &str = "01 01 03 01 00 00 96 00 e0 e2 c4 04 00 00 00 00 00 00 00 00 00 00 00 00 \
    80 bb 00 00 00 00 00 00 07 00 00 00 00 00 00 00";
/// 2.10.2's example.
const STATUS: &str = "03 02 01 03 42 37 03 00 00 a6 0e 00 00 00 00 00 00 00 80 3e 00 00 48 41 \
    96 00 8f 01 60 00 00 00 02 00 00 00 01 00 00 00 05 00 00 00 01 00 00 00";
/// 2.10.1's follower example.
const SPDIF_FOLLOWER: &str = "01 01 c0 00 00 00 00 03";
/// 2.10.3's IN example.
const CTRL_IN: &str = "00 0a 00 00 40 42 0f 00 00 00 00 00 00 00 00 00";

/// Everything a full unit of the time-base blocks lists.
const ALL: [u16; 8] = [
    0x8000, 0x8006, 0x8010, 0x8020, 0x8021, 0x8022, 0x8030, 0x8031,
];

fn scripted(id_reply: &str) -> (MockTransport, dspi_transport::mock::LogHandle) {
    let t = MockTransport::new().data(op::REQ_EXTENSION, hex(id_reply));
    let log = t.log_handle();
    (t, log)
}

#[test]
fn wp_atproto_reads_decode_the_spec_examples() {
    let (mut t, log) = scripted(TIME_BASE);
    let r = read_time_base(&mut t, &ALL).unwrap();
    assert_eq!(r.output_frames, 4_800_000);
    assert_eq!(r.follower().unwrap().t, 80_000_000);
    assert_eq!(r.source().unwrap().domain, 0x0303_3742);
    {
        let l = log.lock().unwrap();
        assert_eq!(
            (l[0].direction, l[0].opcode, l[0].value, l[0].length),
            (Direction::In, 0x00, 0x8030, 64)
        );
    }

    let (mut t, log) = scripted(TIMELINE);
    let tl = read_siggen_timeline(&mut t, &ALL).unwrap();
    assert_eq!((tl.start, tl.run_id, tl.arm_lead_ms), (80_012_000, 7, 150));
    assert_eq!(tl.last_result, TimelineResult::Started);
    assert_eq!(log.lock().unwrap()[0].value, 0x8031);

    let (mut t, _) = scripted(STATUS);
    let s = read_alink_spdif_status(&mut t, &ALL).unwrap();
    assert_eq!((s.source_id, s.epoch, s.frame), (0x3742, 3, 960_000));

    let (mut t, _) = scripted(SPDIF_FOLLOWER);
    let c = read_alink_spdif(&mut t, &ALL).unwrap();
    assert_eq!(
        (c.mode, c.d, c.state),
        (SpdifMode::Follower, 192, ContractState::Locked)
    );

    let (mut t, _) = scripted(CTRL_IN);
    let q = read_alink_spdif_ctrl(&mut t, &ALL).unwrap();
    assert_eq!(
        (q.last_op, q.last_result, q.last_frame),
        (ApplyOp::SiggenStop, ApplyResult::Applied, 1_000_000)
    );
}

#[test]
fn wp_atproto_writes_send_the_spec_bytes() {
    let mut t = MockTransport::new();
    let log = t.log_handle();
    // 2.10.1: the follower write of the host recipe (state sent as 0).
    let cfg = AlinkSpdif::decode(&hex(SPDIF_FOLLOWER)).unwrap();
    write_alink_spdif(&mut t, &ALL, cfg).unwrap();
    // 2.10.3: a broadcast SIGGEN_STOP now at F = 1 000 000.
    let stop = ApplyRequest {
        frame: 1_000_000,
        op: ApplyOp::SiggenStop,
        arg: 1,
        value: 0,
    };
    write_spdif_ctrl(&mut t, &ALL, stop).unwrap();
    // 2.8.7.1: op 9 on the USB time base at S = 80 012 000.
    siggen_start_at(&mut t, &ALL, TB_USB_SAMPLE, 80_012_000).unwrap();
    // 2.8.7: MASTER_VOLUME -20 dB at frame 1 002 400 on the line.
    let vol = ApplyRequest {
        frame: 1_002_400,
        op: ApplyOp::MasterVolume,
        arg: 0,
        value: -2000,
    };
    apply_at(&mut t, &ALL, vol, TB_ALINK_LINE).unwrap();
    // 2.9.4: the same on the USB bus, bus frame 1100, sample 0.
    apply_at(
        &mut t,
        &ALL,
        ApplyRequest { frame: 1100, ..vol },
        TB_USB_BUS_FRAME,
    )
    .unwrap();
    // Op 10 at once on the S/PDIF time base.
    siggen_stop_at(&mut t, &ALL, TB_SPDIF, 1_000_000, true).unwrap();
    let l = log.lock().unwrap();
    let wire: Vec<(u16, Vec<u8>)> = l.iter().map(|e| (e.value, e.payload.clone())).collect();
    assert!(
        l.iter()
            .all(|e| e.direction == Direction::Out && e.opcode == 0x00)
    );
    assert_eq!(
        wire,
        vec![
            (0x8020, hex("01 01 c0 00 00 00 00 00")),
            (
                0x8022,
                hex("40 42 0f 00 00 00 00 00 0a 01 00 00 03 00 00 00")
            ),
            (
                0x8006,
                hex("e0 e2 c4 04 00 00 00 00 09 00 00 00 01 00 00 00")
            ),
            (
                0x8006,
                hex("a0 4b 0f 00 00 00 00 00 04 00 30 f8 00 00 00 00")
            ),
            (
                0x8006,
                hex("4c 04 00 00 00 00 00 00 04 00 30 f8 02 00 00 00")
            ),
            (
                0x8006,
                hex("40 42 0f 00 00 00 00 00 0a 01 00 00 03 00 00 00")
            ),
        ]
    );
}

#[test]
fn wp_atproto_refusals_are_local_and_send_nothing() {
    let mut t = MockTransport::new();
    let log = t.log_handle();
    let none: [u16; 0] = [];
    let unsupported =
        |r: Result<()>, id: u16| matches!(r, Err(ExtError::Unsupported { id: i }) if i == id);
    // Not listed: nothing sent.
    assert!(matches!(
        read_time_base(&mut t, &none),
        Err(ExtError::Unsupported { id: 0x8030 })
    ));
    assert!(matches!(
        read_siggen_timeline(&mut t, &none),
        Err(ExtError::Unsupported { id: 0x8031 })
    ));
    assert!(matches!(
        read_alink_spdif(&mut t, &none),
        Err(ExtError::Unsupported { id: 0x8020 })
    ));
    assert!(matches!(
        read_alink_spdif_status(&mut t, &none),
        Err(ExtError::Unsupported { id: 0x8021 })
    ));
    assert!(matches!(
        read_alink_spdif_ctrl(&mut t, &none),
        Err(ExtError::Unsupported { id: 0x8022 })
    ));
    assert!(unsupported(
        write_alink_spdif(&mut t, &none, AlinkSpdif::DEFAULT),
        0x8020
    ));
    // Ops 9 and 10 need EXT_SIGGEN_TIMELINE listed (a build with the ops).
    let no_ops = [0x8000u16, 0x8006, 0x8010];
    assert!(unsupported(
        siggen_start_at(&mut t, &no_ops, TB_USB_SAMPLE, 5),
        0x8031
    ));
    assert!(unsupported(
        siggen_stop_at(&mut t, &no_ops, TB_ALINK_LINE, 5, false),
        0x8031
    ));
    // Time base 3 needs the S/PDIF block, 1 the USB block.
    assert!(unsupported(
        siggen_start_at(&mut t, &[0x8006, 0x8031], TB_SPDIF, 5),
        0x8020
    ));
    assert!(unsupported(
        siggen_start_at(&mut t, &[0x8006, 0x8031], TB_USB_SAMPLE, 5),
        0x8010
    ));
    // By arguments: op 9 as a bus frame (R5), bad args, CANCEL_ALL broadcast.
    let bad = |r: Result<()>| matches!(r, Err(ExtError::BadArgument(_)));
    assert!(bad(siggen_start_at(&mut t, &ALL, TB_USB_BUS_FRAME, 5)));
    let start = ApplyRequest {
        frame: 5,
        op: ApplyOp::SiggenStart,
        arg: 1,
        value: 0,
    };
    assert!(bad(apply_at(&mut t, &ALL, start, TB_USB_SAMPLE)));
    assert!(bad(apply_at(
        &mut t,
        &ALL,
        ApplyRequest { arg: 0, ..start },
        4
    )));
    let cancel = ApplyRequest {
        frame: 0,
        op: ApplyOp::CancelAll,
        arg: 0,
        value: 0,
    };
    assert!(bad(write_spdif_ctrl(&mut t, &ALL, cancel)));
    assert!(bad(write_alink_spdif(
        &mut t,
        &ALL,
        AlinkSpdif {
            mode: SpdifMode::Other(3),
            ..AlinkSpdif::DEFAULT
        }
    )));
    assert!(log.lock().unwrap().is_empty(), "nothing on the wire");
    // CANCEL_ALL per unit is fine on every time base.
    apply_at(&mut t, &ALL, cancel, TB_SPDIF).unwrap();
    assert_eq!(log.lock().unwrap().len(), 1);
}

#[test]
fn wp_atproto_short_stalled_and_foreign_replies() {
    // Short: a ShortRead, never a panic.
    let mut t = MockTransport::new().data(op::REQ_EXTENSION, vec![1; 39]);
    assert!(matches!(
        read_siggen_timeline(&mut t, &ALL),
        Err(ExtError::Transport(TransportError::ShortRead { .. }))
    ));
    // Another layout version: Malformed.
    let mut v2 = hex(TIMELINE);
    v2[0] = 2;
    let mut t = MockTransport::new().data(op::REQ_EXTENSION, v2);
    assert!(matches!(
        read_siggen_timeline(&mut t, &ALL),
        Err(ExtError::Transport(TransportError::Malformed(_)))
    ));
    // Extra bytes are ignored.
    let mut long = hex(TIME_BASE);
    long.extend_from_slice(&[0xEE; 8]);
    let mut t = MockTransport::new().data(op::REQ_EXTENSION, long);
    assert_eq!(read_time_base(&mut t, &ALL).unwrap().entries.len(), 2);
    // A stall: listed, but the unit says no.
    let mut t = MockTransport::new().reply(op::REQ_EXTENSION, Reply::Stall);
    assert!(matches!(
        read_time_base(&mut t, &ALL),
        Err(ExtError::Unsupported { id: 0x8030 })
    ));
    assert!(matches!(
        siggen_start_at(&mut t, &ALL, TB_USB_SAMPLE, 5),
        Err(ExtError::Refused { id: 0x8006 })
    ));
}

#[test]
fn wp_atproto_the_event_behind_bit_5() {
    let p = hex("03 03 01 00 00 a6 0e 00");
    let e = time_base_event(Some(1 << 5), 0xE4, &p).unwrap();
    assert_eq!(
        (e.time_base, e.state, e.t_low),
        (3, ContractState::Locked, 960_000)
    );
    assert_eq!(time_base_event(Some(1 << 4), 0xE4, &p), None);
}

// ------------------------------------------------------------ the generations

/// The writes a [`SpecUnit`] took: id and payload.
type Writes = Arc<Mutex<Vec<(u16, Vec<u8>)>>>;

/// The new ids this stand-in answers, on top of what the fake lists.
const EXTRA: [u16; 7] = [0x8006, 0x8010, 0x8020, 0x8021, 0x8022, 0x8030, 0x8031];

/// A fake unit with the time-base blocks: the generation's [`FakeDevice`]
/// for everything else, and the spec's examples for the new ids, on page 0
/// (`REQ_EXTENSION`) and in the tunnel alike. ATFAKE's fakes replace it; this
/// is only the envelope's witness. It records every request for those ids.
struct SpecUnit {
    dev: FakeDevice,
    spdif: Vec<u8>,
    seen: Arc<AtomicUsize>,
    writes: Writes,
}

impl SpecUnit {
    fn answer(&mut self, value: u16, len: u16) -> Option<Vec<u8>> {
        let mut b = match value {
            0x8030 => hex(TIME_BASE),
            0x8031 => hex(TIMELINE),
            0x8020 => self.spdif.clone(),
            0x8021 => hex(STATUS),
            0x8022 => hex(CTRL_IN),
            _ => return None,
        };
        self.seen.fetch_add(1, Ordering::SeqCst);
        b.truncate(usize::from(len));
        Some(b)
    }

    fn list(b: Vec<u8>) -> Vec<u8> {
        let mut ids = decode_list(&b);
        ids.extend(EXTRA);
        ids.sort_unstable();
        ids.dedup();
        ids.iter().flat_map(|i| i.to_le_bytes()).collect()
    }

    fn take(&mut self, value: u16, data: &[u8]) -> bool {
        if !EXTRA.contains(&value) {
            return false;
        }
        self.seen.fetch_add(1, Ordering::SeqCst);
        if value == 0x8020 {
            let mut c = data.to_vec();
            if let Some(s) = c.get_mut(7) {
                *s = 3;
            }
            self.spdif = c;
        }
        self.writes.lock().unwrap().push((value, data.to_vec()));
        true
    }

    fn get(&mut self, value: u16, len: u16, upto: bool) -> dspi_transport::Result<Vec<u8>> {
        if value == 0 {
            let b = self.dev.control_in_upto(op::REQ_EXTENSION, 0, len)?;
            return Ok(Self::list(b));
        }
        if let Some(b) = self.answer(value, len) {
            return Ok(b);
        }
        if upto {
            self.dev.control_in_upto(op::REQ_EXTENSION, value, len)
        } else {
            self.dev.control_in(op::REQ_EXTENSION, value, len)
        }
    }
}

impl Transport for SpecUnit {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        if opcode == op::REQ_EXTENSION {
            return self.get(value, len, false);
        }
        self.dev.control_in(opcode, value, len)
    }
    fn control_in_upto(
        &mut self,
        opcode: u8,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        if opcode == op::REQ_EXTENSION {
            return self.get(value, max_len, true);
        }
        self.dev.control_in_upto(opcode, value, max_len)
    }
    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        if opcode == op::REQ_EXTENSION && self.take(value, data) {
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
    fn carries_pages(&self) -> bool {
        self.dev.carries_pages()
    }
    fn control_in_paged(
        &mut self,
        id: CmdId,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        if id == EXT_TUNNEL {
            if value == 0 {
                let b = self.dev.control_in_paged(id, 0, max_len)?;
                return Ok(Self::list(b));
            }
            if let Some(b) = self.answer(value, max_len) {
                return Ok(b);
            }
        }
        self.dev.control_in_paged(id, value, max_len)
    }
    fn control_out_paged(
        &mut self,
        id: CmdId,
        value: u16,
        data: &[u8],
    ) -> dspi_transport::Result<()> {
        if id == EXT_TUNNEL && self.take(value, data) {
            return Ok(());
        }
        self.dev.control_out_paged(id, value, data)
    }
}

/// What one generation answers to every helper, as text, and the writes it
/// saw.
struct Run {
    listed: Vec<u16>,
    results: Vec<String>,
    seen: usize,
    writes: Vec<(u16, Vec<u8>)>,
}

fn run(g: FakeGeneration) -> Run {
    let seen = Arc::new(AtomicUsize::new(0));
    let writes = Arc::new(Mutex::new(Vec::new()));
    let unit = SpecUnit {
        dev: FakeDevice::demo_generation(g),
        spdif: hex("00 01 c0 00 00 00 00 00"),
        seen: Arc::clone(&seen),
        writes: Arc::clone(&writes),
    };
    let mut t = paged::open(Box::new(unit)).unwrap();
    let t = &mut *t;
    let supported = crate::ext::list(t).unwrap_or_default();
    let listed: Vec<u16> = EXTRA
        .iter()
        .copied()
        .filter(|i| supported.contains(i))
        .collect();
    let s = &supported;
    let follower = AlinkSpdif {
        mode: SpdifMode::Follower,
        ..AlinkSpdif::DEFAULT
    };
    let stop = ApplyRequest {
        frame: 1_000_000,
        op: ApplyOp::SiggenStop,
        arg: 1,
        value: 0,
    };
    let results = vec![
        format!("{:?}", read_time_base(t, s)),
        format!("{:?}", read_siggen_timeline(t, s)),
        format!("{:?}", read_alink_spdif(t, s)),
        format!("{:?}", write_alink_spdif(t, s, follower)),
        format!("{:?}", read_alink_spdif(t, s)),
        format!("{:?}", read_alink_spdif_status(t, s)),
        format!("{:?}", read_alink_spdif_ctrl(t, s)),
        format!("{:?}", write_spdif_ctrl(t, s, stop)),
        format!("{:?}", siggen_start_at(t, s, TB_USB_SAMPLE, 80_012_000)),
        format!("{:?}", siggen_stop_at(t, s, TB_SPDIF, 1_000_000, false)),
        format!("{:?}", siggen_start_at(t, s, TB_USB_BUS_FRAME, 5)),
    ];
    let writes = writes.lock().unwrap().clone();
    Run {
        listed,
        results,
        seen: seen.load(Ordering::SeqCst),
        writes,
    }
}

#[test]
fn wp_atproto_both_envelopes_give_the_same_results() {
    let g1b = run(FakeGeneration::Bofr116 { page_guard: true });
    let g1b_old = run(FakeGeneration::Bofr116 { page_guard: false });
    let g2b = run(FakeGeneration::Bofr117);
    assert_eq!(g1b.listed, EXTRA.to_vec(), "the stand-in lists its ids");
    for r in [&g1b_old, &g2b] {
        assert_eq!(r.listed, g1b.listed);
        assert_eq!(r.results, g1b.results);
        assert_eq!(r.writes, g1b.writes);
        assert_eq!(r.seen, g1b.seen);
    }
    // Everything answered as the spec says, the bus-frame op 9 refused here.
    let ok = g1b.results.iter().filter(|r| r.starts_with("Ok(")).count();
    assert_eq!(ok, 10, "{:#?}", g1b.results);
    assert!(g1b.results[10].contains("BadArgument"));
    assert!(g1b.results[4].contains("Follower"), "{}", g1b.results[4]);
    assert_eq!(
        g1b.writes,
        vec![
            (0x8020, hex("01 01 c0 00 00 00 00 00")),
            (
                0x8022,
                hex("40 42 0f 00 00 00 00 00 0a 01 00 00 03 00 00 00")
            ),
            (
                0x8006,
                hex("e0 e2 c4 04 00 00 00 00 09 00 00 00 01 00 00 00")
            ),
            (
                0x8006,
                hex("40 42 0f 00 00 00 00 00 0a 00 00 00 03 00 00 00")
            ),
        ]
    );
}

#[test]
fn wp_atproto_stock_generations_refuse_locally_with_nothing_sent() {
    for g in [FakeGeneration::Stock116, FakeGeneration::STOCK117] {
        let r = run(g);
        assert!(r.listed.is_empty(), "{g:?}");
        assert_eq!(r.seen, 0, "{g:?}: nothing reached the unit");
        assert!(r.writes.is_empty());
        for (i, res) in r.results.iter().enumerate() {
            let want = if i == 10 {
                "BadArgument"
            } else {
                "Unsupported"
            };
            assert!(res.contains(want), "{g:?} #{i}: {res}");
        }
    }
}
