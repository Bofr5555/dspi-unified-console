//! Review AT-A/AT-B: WP ATPROTO's helpers against WP ATFAKE's real fakes.
//!
//! The two were written independently from the firmware's spec (PLAN-AT
//! R9a): ATFAKE lays its replies out byte by byte, ATPROTO decodes them. Here
//! they meet, through `paged::open`, on G1b (`REQ_EXTENSION`) and on G2b (the
//! tunnel): every field one side writes must read as the same value on the
//! other, on the line, USB-direct and S/PDIF. `tests_wp_atproto.rs` keeps its
//! `SpecUnit` stand-in for the spec's byte examples (the fakes cannot be
//! made to answer those exact bytes).
#![allow(clippy::arithmetic_side_effects)]

use std::time::Duration;

use dspi_proto::ext::alink::ApplyResult;
use dspi_proto::ext::timebase::*;
use dspi_proto::packets::SiggenConfig;
use dspi_transport::fake::{
    AlinkSetup, FakeGeneration, FakeRemote, FakeTimeBase, FakeTimeBaseUnit, SpdifSetup,
    TimeBaseFault, UsbDirectSetup,
};
use dspi_transport::{FakeDevice, NotificationSource, paged};

use super::*;

const GENERATIONS: [FakeGeneration; 2] = [
    FakeGeneration::Bofr116 { page_guard: true },
    FakeGeneration::Bofr117,
];
/// The arm lead the tests set, ms, and half a second of lead in samples
/// (the fakes run on real time: a busy host must not eat the lead).
const ARM_MS: u16 = 50;
const LEAD: u64 = 24_000;

struct Unit {
    t: Box<dyn Transport>,
    remote: FakeRemote,
    notes: Box<dyn NotificationSource>,
    listed: Vec<u16>,
}

fn unit(tb: &FakeTimeBase, u: FakeTimeBaseUnit, g: FakeGeneration) -> Unit {
    let dev = FakeDevice::demo_on_time_base_as(tb, u, g);
    let remote = dev.remote();
    remote.time_base_set_arm_lead_ms(ARM_MS);
    let notes = dev.notifications().unwrap();
    let mut t = paged::open(Box::new(dev)).unwrap();
    let listed = crate::ext::list(&mut *t).unwrap();
    Unit {
        t,
        remote,
        notes,
        listed,
    }
}

fn sweep(ms: u32) -> SiggenConfig {
    SiggenConfig {
        signal_type: 4,
        channel_mask: 1,
        level_db: -40.0,
        duration_ms: ms,
        repeat_count: 1,
        p1: 20.0,
        p2: 20_000.0,
        ..SiggenConfig::default()
    }
}

fn stage(u: &mut Unit, cfg: &SiggenConfig) {
    u.t.control_out(op::REQ_SIGGEN_SET_CONFIG, 0, &cfg.encode())
        .unwrap();
}

fn wait_for(mut f: impl FnMut() -> bool) -> bool {
    for _ in 0..500 {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// Every 0xE4 queued so far, decoded with ATPROTO's event codec.
fn events(u: &mut Unit) -> Vec<TimeBaseEvent> {
    let mut out = Vec::new();
    loop {
        let p = u.notes.read(Duration::ZERO).unwrap();
        if p.is_empty() {
            return out;
        }
        if p.len() == 12
            && p[0] == 2
            && let Some(e) = time_base_event(Some(NOTIFY_FEAT_TIME_BASE), p[1], &p[4..])
        {
            out.push(e);
        }
    }
}

fn timeline(u: &mut Unit) -> SiggenTimeline {
    read_siggen_timeline(&mut *u.t, &u.listed).unwrap()
}

fn first(u: &mut Unit) -> (TimeBaseRead, TimeBaseEntry) {
    let r = read_time_base(&mut *u.t, &u.listed).unwrap();
    let e = r.entries[0];
    (r, e)
}

/// The contract read, an op 9 start with its proof, and a holdover, on one
/// unit of time base `tb`.
fn start_and_fault(mut u: Unit, tb: u8, domain: u32, d: u16) {
    assert!(u.listed.contains(&EXT_TIME_BASE) && u.listed.contains(&EXT_SIGGEN_TIMELINE));
    let (r, e) = first(&mut u);
    assert_eq!(r.entries.len(), 1);
    assert_eq!(
        (e.time_base, e.role, e.state, e.domain, e.d),
        (tb, TimeBaseRole::Follower, ContractState::Locked, domain, d)
    );
    assert_eq!(e.domain_time_base(), tb);
    assert_eq!(e.last_reason, TimeBaseReason::Locked);
    assert!(e.phase_error.is_finite() && e.phase_error.abs() < 1.0);
    let inv = r.t_minus_output(&e);

    let tl = timeline(&mut u);
    assert_eq!(
        (tl.state, tl.last_result, tl.arm_lead_ms, tl.run_id),
        (TimelineState::Idle, TimelineResult::None, ARM_MS, 0)
    );
    assert_eq!((tl.time_base, tl.start, tl.pending), (TB_NONE, 0, 0));

    // Op 9 at T: pending, then started on T, then the run's length.
    stage(&mut u, &sweep(200));
    let (_, e) = first(&mut u);
    let t0 = e.t + LEAD;
    assert!(siggen_start_lead_ok(e.t, t0, tl.arm_lead_ms, 48_000));
    siggen_start_at(&mut *u.t, &u.listed, tb, t0).unwrap();
    let tl = timeline(&mut u);
    assert!(
        matches!(tl.state, TimelineState::Pending | TimelineState::Armed),
        "{tl:?}"
    );
    assert_eq!((tl.time_base, tl.pending), (tb, t0));
    // A second START while one is pending: the unit refuses.
    let again = siggen_start_at(&mut *u.t, &u.listed, tb, t0 + LEAD);
    assert!(matches!(again, Err(ExtError::Refused { .. })), "{again:?}");
    assert!(wait_for(|| timeline(&mut u).state == TimelineState::Running));
    assert!(wait_for(|| timeline(&mut u).state == TimelineState::Idle));
    let tl = timeline(&mut u);
    assert_eq!(
        (
            tl.last_result,
            tl.start,
            tl.pending,
            tl.run_id,
            tl.discontinuities
        ),
        (TimelineResult::Started, t0, 0, 1, 0)
    );
    assert_eq!((tl.time_base, tl.samples, tl.held), (tb, 9600, 0));
    let starts = u.remote.measure_timed_starts();
    assert_eq!((starts[0].t, starts[0].time_base), (t0, tb));

    // Nothing moved: the invariant and the counter (PLAN-AT G2).
    let (r, e) = first(&mut u);
    assert_eq!(r.t_minus_output(&e), inv);
    assert_eq!(e.disturbances, 0);
    assert!(events(&mut u).is_empty());

    // A late op 10 is applied at once (a stop), an op 9 in the past refused.
    siggen_stop_at(&mut *u.t, &u.listed, tb, e.t.saturating_sub(10), true).unwrap();
    let late = siggen_start_at(&mut *u.t, &u.listed, tb, e.t.saturating_sub(10));
    assert!(matches!(late, Err(ExtError::Refused { .. })), "{late:?}");

    // A holdover: the entry, the counter and the event agree. (Staged again
    // first: the applied op 10 cleared the fresh-config flag.)
    stage(&mut u, &sweep(200));
    assert!(u.remote.time_base_fault(TimeBaseFault::Holdover));
    let (_, e) = first(&mut u);
    assert_eq!(
        (e.state, e.last_reason, e.disturbances),
        (ContractState::Holdover, TimeBaseReason::Holdover, 1)
    );
    let ev = events(&mut u);
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert_eq!(
        (ev[0].time_base, ev[0].state, ev[0].reason),
        (tb, ContractState::Holdover, TimeBaseReason::Holdover)
    );
    assert!(ev[0].is_loss());
    // Only a LOCKED T may be scheduled on: the unit refuses the START.
    let r = siggen_start_at(&mut *u.t, &u.listed, tb, e.t + LEAD);
    assert!(matches!(r, Err(ExtError::Refused { .. })), "{r:?}");
    assert!(u.remote.time_base_fault(TimeBaseFault::Resume));
    let (_, e) = first(&mut u);
    assert_eq!(
        (e.state, e.last_reason, e.disturbances),
        (ContractState::Locked, TimeBaseReason::Resumed, 2)
    );
    // The loss cleared the fresh-config flag: a START completes refused.
    siggen_start_at(&mut *u.t, &u.listed, tb, e.t + LEAD).unwrap();
    assert!(wait_for(
        || timeline(&mut u).last_result == TimelineResult::RefusedNotReady
    ));
    assert_eq!(timeline(&mut u).run_id, 1);
}

#[test]
fn wp_atproto_atfake_line_and_usb_agree_on_every_field() {
    for g in GENERATIONS {
        let tb = FakeTimeBase::line(0x42);
        let u = unit(&tb, FakeTimeBaseUnit::Line(AlinkSetup::default()), g);
        let info = crate::ext::read_alink_info(
            &mut *unit(&tb, FakeTimeBaseUnit::Line(AlinkSetup::default()), g).t,
        )
        .unwrap();
        assert_ne!(
            info.feature_bits & dspi_proto::ext::alink::feature::SIGGEN_SYNC,
            0
        );
        start_and_fault(u, TB_ALINK_LINE, 0x0000_0042, 300);

        let tb = FakeTimeBase::usb_direct();
        let setup = UsbDirectSetup {
            origin_wraps: 3,
            ..UsbDirectSetup::default()
        };
        let mut u = unit(&tb, FakeTimeBaseUnit::UsbDirect(setup), g);
        // The bus-frame form of op 9 never leaves the host.
        let r = siggen_start_at(&mut *u.t, &u.listed, TB_USB_BUS_FRAME, 5);
        assert!(matches!(r, Err(ExtError::BadArgument(_))), "{r:?}");
        let (_, e) = first(&mut u);
        let before = tb.t_now();
        let mapped = e.t - 3 * 2048 * 48;
        assert!(
            mapped <= before && before - mapped < 48_000,
            "{mapped} {before}"
        );
        let d = e.d;
        start_and_fault(u, TB_USB_SAMPLE, 0x0100_0000, d);
    }
}

#[test]
fn wp_atproto_atfake_spdif_block_agrees_on_every_field() {
    for g in GENERATIONS {
        let tb = FakeTimeBase::spdif(0x3742);
        let mut src = unit(&tb, FakeTimeBaseUnit::Spdif(SpdifSetup::source()), g);
        let mut fol = unit(&tb, FakeTimeBaseUnit::Spdif(SpdifSetup::follower(192)), g);
        for u in [&src, &fol] {
            for id in ALINK_SPDIF_IDS {
                assert!(u.listed.contains(&id), "{id:#06x}");
            }
        }

        // 2.10.1 and 2.10.2 on both roles.
        let c = read_alink_spdif(&mut *fol.t, &fol.listed).unwrap();
        assert_eq!(
            (c.mode, c.flags, c.d, c.on_lock_error, c.tx_mask, c.state),
            (
                SpdifMode::Follower,
                SPDIF_FLAG_AUTO_REANCHOR,
                192,
                0,
                0,
                ContractState::Locked
            )
        );
        assert!(c.is_valid());
        let c = read_alink_spdif(&mut *src.t, &src.listed).unwrap();
        assert_eq!(
            (c.mode, c.state),
            (SpdifMode::Source, ContractState::Locked)
        );
        let s = read_alink_spdif_status(&mut *fol.t, &fol.listed).unwrap();
        assert_eq!(
            (s.state, s.phase_state, s.rate_code, s.source_id, s.epoch),
            (ContractState::Locked, 2, 1, 0x3742, tb.epoch())
        );
        assert!(s.has(spdif_flag::SUPERFRAME) && s.has(spdif_flag::TIME_BASE));
        assert!(!s.has(spdif_flag::SOURCE) && !s.has(spdif_flag::HOLDOVER));
        assert!(s.d_min <= 192 && 192 <= s.d_max && s.fill > 0);
        assert_eq!((s.lock_count, s.loss_count, s.reanchors), (1, 0, 0));
        let ss = read_alink_spdif_status(&mut *src.t, &src.listed).unwrap();
        assert!(ss.has(spdif_flag::SOURCE));
        assert_eq!((ss.source_id, ss.d_min, ss.d_max), (0x3742, 0, 0));
        assert!(ss.frame.abs_diff(s.frame) < 48_000, "one counter");

        // 2.11.1: the domain of C5, the source's D = 0.
        let (_, ef) = first(&mut fol);
        let (_, es) = first(&mut src);
        assert_eq!(
            (ef.time_base, ef.role, ef.d),
            (TB_SPDIF, TimeBaseRole::Follower, 192)
        );
        assert_eq!((es.role, es.d), (TimeBaseRole::Source, 0));
        assert_eq!(ef.domain, es.domain);
        assert_eq!(
            (ef.domain_time_base(), ef.domain_source(), ef.domain_epoch()),
            (TB_SPDIF, 0x3742, tb.epoch())
        );

        // A new D re-anchors the follower (2.10.1 "Changes").
        let moved = AlinkSpdif {
            d: 200,
            ..read_alink_spdif(&mut *fol.t, &fol.listed).unwrap()
        };
        write_alink_spdif(&mut *fol.t, &fol.listed, moved).unwrap();
        let ev = events(&mut fol);
        assert_eq!(
            (ev[0].time_base, ev[0].state, ev[0].reason),
            (TB_SPDIF, ContractState::Anchoring, TimeBaseReason::Reanchor)
        );
        assert!(wait_for(|| first(&mut fol).1.state == ContractState::Locked));
        let (_, ef) = first(&mut fol);
        assert_eq!((ef.d, ef.disturbances), (200, 1));
        assert_eq!(
            read_alink_spdif_status(&mut *fol.t, &fol.listed)
                .unwrap()
                .reanchors,
            1
        );
        // A D outside d_min..d_max is the unit's to refuse.
        let out = AlinkSpdif { d: 10, ..moved };
        let r = write_alink_spdif(&mut *fol.t, &fol.listed, out);
        assert!(matches!(r, Err(ExtError::Refused { .. })), "{r:?}");

        // 2.10.3: the broadcast stop reaches the follower's generator.
        stage(&mut fol, &sweep(5_000));
        let (_, ef) = first(&mut fol);
        siggen_start_at(&mut *fol.t, &fol.listed, TB_SPDIF, ef.t + LEAD).unwrap();
        assert!(wait_for(
            || timeline(&mut fol).state == TimelineState::Running
        ));
        let (_, es) = first(&mut src);
        let short = ApplyRequest {
            frame: es.t + 800,
            op: ApplyOp::SiggenStop,
            arg: SIGGEN_STOP_NOW,
            value: 0,
        };
        assert!(!spdif_ctrl_lead_ok(es.t, short.frame));
        let r = write_spdif_ctrl(&mut *src.t, &src.listed, short);
        assert!(matches!(r, Err(ExtError::Refused { .. })), "{r:?}");
        let (_, es) = first(&mut src);
        let stop = ApplyRequest {
            frame: es.t + 9600,
            ..short
        };
        assert!(spdif_ctrl_lead_ok(es.t, stop.frame));
        write_spdif_ctrl(&mut *src.t, &src.listed, stop).unwrap();
        let q = read_alink_spdif_ctrl(&mut *src.t, &src.listed).unwrap();
        assert_eq!(q.pending, 1);
        assert!(wait_for(|| timeline(&mut fol).state == TimelineState::Idle));
        assert!(wait_for(|| {
            read_alink_spdif_ctrl(&mut *src.t, &src.listed)
                .unwrap()
                .pending
                == 0
        }));
        let q = read_alink_spdif_ctrl(&mut *src.t, &src.listed).unwrap();
        assert_eq!(
            (q.last_op, q.last_result, q.last_frame),
            (ApplyOp::SiggenStop, ApplyResult::Applied, stop.frame)
        );
        let tl = timeline(&mut fol);
        assert!(tl.samples > 0 && tl.samples < 240_000, "{tl:?}");

        // An epoch: the follower re-anchors with the reason, the domain moves.
        let _ = events(&mut fol);
        let (_, before) = first(&mut fol);
        assert!(src.remote.time_base_fault(TimeBaseFault::Epoch));
        assert!(wait_for(|| first(&mut fol).1.domain != before.domain));
        assert!(wait_for(|| first(&mut fol).1.state == ContractState::Locked));
        let (_, after) = first(&mut fol);
        assert_eq!(after.domain_epoch(), before.domain_epoch().wrapping_add(1));
        assert!(after.disturbances > before.disturbances);
        assert!(
            events(&mut fol)
                .iter()
                .any(|e| e.reason == TimeBaseReason::Epoch),
            "0xE4 EPOCH"
        );
        let (_, es) = first(&mut src);
        assert_eq!(es.domain, after.domain);
        // The source counted it too. (Its `last_reason` reads `LOCKED` on the
        // fake, which passes through an anchor; the spec's 2.11.1 example
        // shows `EPOCH` on a source. FWSP2 decides; the gate reads the
        // counter.)
        assert!(es.disturbances >= 1);
        assert!(
            events(&mut src)
                .iter()
                .any(|e| e.reason == TimeBaseReason::Epoch)
        );
    }
}
