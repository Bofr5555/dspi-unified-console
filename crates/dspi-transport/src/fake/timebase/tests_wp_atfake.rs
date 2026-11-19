//! WP ATFAKE: fake units on one time base. The replies are decoded here by
//! hand from the fork's `extension_channel_spec.md` 2.10 and 2.11 (bofr-main
//! dd4ce4b) and the seam's constants, never with WP ATPROTO's codecs. Every
//! scenario that touches the wire runs on G1b (`REQ_EXTENSION` 0x00) and on
//! G2b (the tunnel `0xE010`, whose refusal codes are checked too).

use std::time::Duration;

use dspi_proto::ext::alink::{
    self as alink, AlinkInfo, AlinkRole, ApplyOp, ApplyRequest, EXT_ALINK_CTRL_MSG, EXT_ALINK_INFO,
    EXT_APPLY_AT_FRAME,
};
use dspi_proto::ext::timebase::{self as tbc, reason, timeline_result as tr};
use dspi_proto::generated::{ext as gext, opcodes as op};
use dspi_proto::packets::{SiggenConfig, SiggenStatus};
use dspi_proto::paged::legacy as lm;
use dspi_proto::paged::{EXT_TUNNEL, cmds};
use proptest::prelude::*;

use super::*;
use crate::fake::{ALINK_SIM_LOSE, tests_wp_t4::drain};
use crate::{NotificationSource, Transport};

// ------------------------------------------------------------ helpers

#[derive(Debug, Clone, Copy)]
enum Env {
    /// G1b: `bofr-main` 1.1.6 with PAGEGUARD, `REQ_EXTENSION` 0x00.
    G1b,
    /// G2b: the dual-stack port, the tunnel `0xE010`.
    G2b,
}

const ENVS: [Env; 2] = [Env::G1b, Env::G2b];

impl Env {
    fn generation(self) -> FakeGeneration {
        match self {
            Env::G1b => FakeGeneration::Bofr116 { page_guard: true },
            Env::G2b => FakeGeneration::Bofr117,
        }
    }
}

fn unit(tb: &FakeTimeBase, u: FakeTimeBaseUnit, env: Env) -> FakeDevice {
    let d = FakeDevice::demo_on_time_base_as(tb, u, env.generation());
    d.remote().time_base_set_arm_lead_ms(50);
    d
}

fn line(tb: &FakeTimeBase, env: Env) -> FakeDevice {
    unit(tb, FakeTimeBaseUnit::Line(AlinkSetup::default()), env)
}

fn usb(tb: &FakeTimeBase, origin_wraps: u32, env: Env) -> FakeDevice {
    let setup = UsbDirectSetup {
        origin_wraps,
        ..UsbDirectSetup::default()
    };
    unit(tb, FakeTimeBaseUnit::UsbDirect(setup), env)
}

fn get(d: &mut FakeDevice, env: Env, id: u16, len: u16) -> Vec<u8> {
    match env {
        Env::G1b => d.control_in(op::REQ_EXTENSION, id, len).unwrap(),
        Env::G2b => d.control_in_paged(EXT_TUNNEL, id, len).unwrap(),
    }
}

/// A write: `Err(None)` a STALL on G1b, `Err(Some(code))` the tunnel's code.
fn set(d: &mut FakeDevice, env: Env, id: u16, data: &[u8]) -> Result<(), Option<u8>> {
    match env {
        Env::G1b => d.control_out(op::REQ_EXTENSION, id, data).map_err(|_| None),
        Env::G2b => d.control_out_paged(EXT_TUNNEL, id, data).map_err(|_| {
            let last = d.control_in_paged(cmds::SYS_LAST_RESULT, 0, 8).unwrap();
            Some(last[3])
        }),
    }
}

/// A refusal, with the tunnel's code on G2b.
fn refused(r: Result<(), Option<u8>>, env: Env, code: u8) {
    match env {
        Env::G1b => assert_eq!(r, Err(None)),
        Env::G2b => assert_eq!(r, Err(Some(code))),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct E {
    tb: u8,
    role: u8,
    state: u8,
    reason: u8,
    domain: u32,
    t: u64,
    dist: u16,
    d: u16,
    pe: f32,
}

#[derive(Debug)]
struct Tb {
    out: u32,
    entries: Vec<E>,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// `EXT_TIME_BASE`, 2.11.1.
fn time_base(d: &mut FakeDevice, env: Env) -> Tb {
    let b = get(d, env, tbc::EXT_TIME_BASE, 64);
    assert_eq!(b.len(), 64);
    assert_eq!(b[0], 1, "version");
    assert_eq!(&b[2..4], &[0, 0]);
    assert_eq!(&b[60..64], &[0; 4]);
    let n = usize::from(b[1]);
    assert!(n <= 2);
    let entries = (0..n)
        .map(|i| {
            let e = 12 + 24 * i;
            E {
                tb: b[e],
                role: b[e + 1],
                state: b[e + 2],
                reason: b[e + 3],
                domain: u32_at(&b, e + 4),
                t: u64_at(&b, e + 8),
                dist: u16_at(&b, e + 16),
                d: u16_at(&b, e + 18),
                pe: f32::from_le_bytes(b[e + 20..e + 24].try_into().unwrap()),
            }
        })
        .collect();
    Tb {
        out: u32_at(&b, 8),
        entries,
    }
}

fn entry(d: &mut FakeDevice, env: Env) -> E {
    time_base(d, env).entries[0]
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Tl {
    tb: u8,
    state: u8,
    last: u8,
    disc: u16,
    arm_ms: u16,
    start: u64,
    pending: u64,
    samples: u32,
    run_id: u32,
}

/// `EXT_SIGGEN_TIMELINE`, 2.11.2.
fn timeline(d: &mut FakeDevice, env: Env) -> Tl {
    let b = get(d, env, tbc::EXT_SIGGEN_TIMELINE, 40);
    assert_eq!(b.len(), 40);
    assert_eq!(b[0], 1);
    assert_eq!(u32_at(&b, 28), 0, "held");
    assert_eq!(u32_at(&b, 36), 0, "reserved");
    Tl {
        tb: b[1],
        state: b[2],
        last: b[3],
        disc: u16_at(&b, 4),
        arm_ms: u16_at(&b, 6),
        start: u64_at(&b, 8),
        pending: u64_at(&b, 16),
        samples: u32_at(&b, 24),
        run_id: u32_at(&b, 32),
    }
}

/// A 200 ms log sweep on output 0, once.
fn sweep() -> SiggenConfig {
    sweep_ms(200)
}

fn sweep_ms(ms: u32) -> SiggenConfig {
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

fn stage(d: &mut FakeDevice, cfg: &SiggenConfig) {
    d.control_out(op::REQ_SIGGEN_SET_CONFIG, 0, &cfg.encode())
        .unwrap();
}

fn control(d: &mut FakeDevice, ctl: u16) {
    d.control_in(op::REQ_SIGGEN_CONTROL, ctl, 1).unwrap();
}

fn status(d: &mut FakeDevice) -> SiggenStatus {
    SiggenStatus::decode(&d.control_in(op::REQ_SIGGEN_GET_STATUS, 0, 16).unwrap()).unwrap()
}

fn req(t: u64, o: ApplyOp, arg: u8, tb: u8) -> [u8; 16] {
    let mut b = ApplyRequest {
        frame: t,
        op: o,
        arg,
        value: 0,
    }
    .encode();
    b[12] = tb;
    b
}

fn op9(t: u64, tb: u8) -> [u8; 16] {
    req(t, ApplyOp::SiggenStart, 0, tb)
}

fn op10(t: u64, arg: u8, tb: u8) -> [u8; 16] {
    req(t, ApplyOp::SiggenStop, arg, tb)
}

/// The arm lead the tests set (50 ms), in samples.
const ARM: u64 = 2400;
/// 100 ms.
const LEAD: u64 = 4800;

fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

/// Poll `f` every 10 ms for up to 5 s.
fn wait_for(mut f: impl FnMut() -> bool) -> bool {
    for _ in 0..500 {
        if f() {
            return true;
        }
        sleep_ms(10);
    }
    false
}

/// `(tb, state, reason)` of every 0xE4 in `ps`.
fn e4s(ps: &[Vec<u8>]) -> Vec<(u8, u8, u8)> {
    ps.iter()
        .filter(|p| p.len() == 12 && p[1] == tbc::NOTIFY_EVT_TIME_BASE)
        .map(|p| (p[4], p[5], p[6]))
        .collect()
}

/// `(op, result)` of every 0xE2 in `ps`.
fn e2s(ps: &[Vec<u8>]) -> Vec<(u8, u8)> {
    ps.iter()
        .filter(|p| p.len() == 12 && p[1] == alink::NOTIFY_EVT_APPLY_RESULT)
        .map(|p| (p[4], p[5]))
        .collect()
}

fn list(d: &mut FakeDevice, env: Env) -> Vec<u16> {
    let b = match env {
        Env::G1b => d
            .control_in_upto(op::REQ_EXTENSION, gext::EXT_LIST, 64)
            .unwrap(),
        Env::G2b => get(d, env, gext::EXT_LIST, 64),
    };
    b.chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

// ------------------------------------------------------------ the domain

#[test]
fn wp_atfake_three_units_agree_on_t_within_the_clock_tick() {
    for env in ENVS {
        // The line: three receivers of sender 0x42.
        let tb = FakeTimeBase::line(0x42);
        let mut us: Vec<_> = (0..3).map(|_| line(&tb, env)).collect();
        let before = tb.t_now();
        let es: Vec<E> = us.iter_mut().map(|d| entry(d, env)).collect();
        let after = tb.t_now();
        for e in &es {
            assert!(
                before <= e.t && e.t <= after,
                "{e:?} outside {before}..{after}"
            );
            assert_eq!(
                (e.tb, e.role, e.state, e.domain),
                (0, 0, tbc::TB_STATE_LOCKED, 0x0000_0042)
            );
            assert_eq!(e.d, 300, "the line's effective D");
            assert!(e.pe > 0.0 && e.pe < 0.1);
        }
        let spread = es.iter().map(|e| e.t).max().unwrap() - es.iter().map(|e| e.t).min().unwrap();
        assert!(spread <= after - before);

        // USB-direct: three origins, mapped by multiples of 2048 * 48.
        let tb = FakeTimeBase::usb_direct();
        let origins = [1u32, 3, 7];
        let mut us: Vec<_> = origins.iter().map(|o| usb(&tb, *o, env)).collect();
        let before = tb.t_now();
        let es: Vec<E> = us.iter_mut().map(|d| entry(d, env)).collect();
        let after = tb.t_now();
        for (e, o) in es.iter().zip(origins) {
            assert_eq!(
                (e.tb, e.state, e.domain),
                (1, tbc::TB_STATE_LOCKED, 0x0100_0000)
            );
            let mapped = e.t - u64::from(o) * 2048 * 48;
            assert!(
                before <= mapped && mapped <= after,
                "{mapped} {before}..{after}"
            );
        }

        // S/PDIF: a source and two followers.
        let tb = FakeTimeBase::spdif(0x3742);
        let mut src = unit(&tb, FakeTimeBaseUnit::Spdif(SpdifSetup::source()), env);
        let mut f1 = unit(&tb, FakeTimeBaseUnit::Spdif(SpdifSetup::follower(192)), env);
        let mut f2 = unit(&tb, FakeTimeBaseUnit::Spdif(SpdifSetup::follower(192)), env);
        let before = tb.t_now();
        let (s, a, b) = (
            entry(&mut src, env),
            entry(&mut f1, env),
            entry(&mut f2, env),
        );
        let after = tb.t_now();
        assert_eq!(tb.domain_id(), 0x0301_3742);
        assert_eq!(
            (s.tb, s.role, s.state, s.domain, s.d),
            (3, 1, 3, 0x0301_3742, 0)
        );
        for f in [a, b] {
            assert_eq!(
                (f.tb, f.role, f.state, f.domain, f.d),
                (3, 0, 3, 0x0301_3742, 192)
            );
        }
        for t in [s.t, a.t, b.t] {
            assert!(before <= t && t <= after);
        }
    }
}

#[test]
fn wp_atfake_t_minus_output_frames_is_constant_while_locked() {
    for env in ENVS {
        let tb = FakeTimeBase::line(7);
        let mut d = line(&tb, env);
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..200 {
            let r = time_base(&mut d, env);
            let e = r.entries[0];
            seen.insert(low32(e.t).wrapping_sub(r.out));
            assert_eq!(e.dist, 0);
        }
        // One instant of the clock for both: exact, not +-1.
        assert_eq!(seen.len(), 1, "{seen:?}");
        // EXT_FRAME_COUNTER agrees: frame - output_frames is the same constant.
        let fc = get(&mut d, env, alink::EXT_FRAME_COUNTER, 16);
        let diff = low32(u64_at(&fc, 0)).wrapping_sub(u32_at(&fc, 12));
        assert!(seen.contains(&diff));
    }
}

#[test]
fn wp_atfake_discovery_lists_the_contract_on_time_base_firmware_only() {
    for env in ENVS {
        let tb = FakeTimeBase::line(1);
        let mut d = line(&tb, env);
        let ids = list(&mut d, env);
        assert!(ids.contains(&tbc::EXT_TIME_BASE) && ids.contains(&tbc::EXT_SIGGEN_TIMELINE));
        assert!(!ids.contains(&tbc::EXT_ALINK_SPDIF));
        let f = u32_at(&get(&mut d, env, gext::EXT_NOTIFY_FEATURES, 4), 0);
        assert_ne!(f & tbc::NOTIFY_FEAT_TIME_BASE, 0);
        let info =
            AlinkInfo::decode(&get(&mut d, env, EXT_ALINK_INFO, AlinkInfo::SIZE as u16)).unwrap();
        assert_ne!(info.feature_bits & tbc::FEATURE_SIGGEN_SYNC, 0);
        let tl = timeline(&mut d, env);
        assert_eq!((tl.tb, tl.state, tl.last, tl.arm_ms), (0xFF, 0, 0, 50));
        d.remote().time_base_set_arm_lead_ms(DEFAULT_ARM_LEAD_MS);
        assert_eq!(timeline(&mut d, env).arm_ms, 150);

        // An S/PDIF unit also lists the apply and its block.
        let tb = FakeTimeBase::spdif(9);
        let mut s = unit(&tb, FakeTimeBaseUnit::Spdif(SpdifSetup::source()), env);
        let ids = list(&mut s, env);
        for id in [EXT_APPLY_AT_FRAME, 0x8020, 0x8021, 0x8022, 0x8030, 0x8031] {
            assert!(ids.contains(&id), "{id:#x} in {ids:x?}");
        }
        assert!(ids.windows(2).all(|w| w[0] < w[1]), "ascending once each");
    }
    // A fake without a time base is firmware before FWTB1.
    let mut old = FakeDevice::demo_alink();
    let ids: Vec<u16> = old
        .control_in_upto(op::REQ_EXTENSION, gext::EXT_LIST, 64)
        .unwrap()
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    assert!(!ids.contains(&tbc::EXT_TIME_BASE));
    let info = AlinkInfo::decode(
        &old.control_in(op::REQ_EXTENSION, EXT_ALINK_INFO, AlinkInfo::SIZE as u16)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(info.feature_bits & tbc::FEATURE_SIGGEN_SYNC, 0);
    let fc = old
        .control_in(op::REQ_EXTENSION, alink::EXT_FRAME_COUNTER, 16)
        .unwrap();
    let t = u64_at(&fc, 0) + 48_000;
    assert!(
        old.control_out(op::REQ_EXTENSION, EXT_APPLY_AT_FRAME, &op9(t, 0))
            .is_err()
    );
    assert!(
        old.control_in(op::REQ_EXTENSION, tbc::EXT_TIME_BASE, 64)
            .is_err()
    );
}

// ------------------------------------------------------------ op 9 on its T

/// Stage, write op 9 at `T + LEAD`, check it pending, wait for the run:
/// started on T, every sample rendered, the start recorded.
fn starts_on_t(mut d: FakeDevice, env: Env) {
    let mut n = d.notifications().unwrap();
    stage(&mut d, &sweep());
    let e = entry(&mut d, env);
    let t = e.t + LEAD;
    set(&mut d, env, EXT_APPLY_AT_FRAME, &op9(t, e.tb)).unwrap();
    let tl = timeline(&mut d, env);
    assert_eq!((tl.tb, tl.pending, tl.run_id), (e.tb, t, 0));
    assert!(
        matches!(tl.state, tbc::TL_PENDING | tbc::TL_ARMED),
        "{tl:?}"
    );
    assert!(wait_for(|| timeline(&mut d, env).last == tr::STARTED));
    let tl = timeline(&mut d, env);
    assert_eq!(
        (tl.tb, tl.start, tl.run_id, tl.disc, tl.pending),
        (e.tb, t, 1, 0, 0)
    );
    // Done: 200 ms is 9600 samples.
    assert!(wait_for(|| timeline(&mut d, env).state == tbc::TL_IDLE));
    let tl = timeline(&mut d, env);
    assert_eq!((tl.samples, tl.start, tl.last), (9600, t, tr::STARTED));
    let s = status(&mut d);
    assert_eq!((s.state, s.stop_reason), (0, 2), "completed");
    let starts = d.remote().measure_timed_starts();
    assert_eq!(starts.len(), 1);
    assert_eq!(
        (
            starts[0].t,
            starts[0].time_base,
            starts[0].timed,
            starts[0].run_id
        ),
        (t, e.tb, true, 1)
    );
    assert_eq!(starts[0].config.duration_ms, 200);
    let ps = drain(&mut n);
    assert!(e2s(&ps).contains(&(9, 0)), "0xE2 applied: {ps:x?}");
}

#[test]
fn wp_atfake_op9_starts_the_generator_on_its_t_on_every_transport() {
    for env in ENVS {
        let tb = FakeTimeBase::line(0x42);
        starts_on_t(line(&tb, env), env);
        let tb = FakeTimeBase::usb_direct();
        starts_on_t(usb(&tb, 3, env), env);
        let tb = FakeTimeBase::spdif(0x1234);
        let src = unit(&tb, FakeTimeBaseUnit::Spdif(SpdifSetup::source()), env);
        starts_on_t(
            unit(&tb, FakeTimeBaseUnit::Spdif(SpdifSetup::follower(192)), env),
            env,
        );
        starts_on_t(src, env);
    }
}

#[test]
fn wp_atfake_three_units_start_at_their_scheduled_ts() {
    let env = Env::G1b;
    let tb = FakeTimeBase::usb_direct();
    let origins = [1u32, 2, 5];
    let mut us: Vec<_> = origins.iter().map(|o| usb(&tb, *o, env)).collect();
    let t0 = tb.t_now() + LEAD;
    let unit_t = |i: usize| t0 + 480 * i as u64 + u64::from(origins[i]) * 2048 * 48;
    for (i, d) in us.iter_mut().enumerate() {
        stage(d, &sweep_ms(50));
        set(d, env, EXT_APPLY_AT_FRAME, &op9(unit_t(i), 1)).unwrap();
    }
    for (i, d) in us.iter_mut().enumerate() {
        assert!(wait_for(|| timeline(d, env).last == tr::STARTED));
        assert_eq!(timeline(d, env).start, unit_t(i));
        let s = d.remote().measure_timed_starts();
        assert_eq!((s[0].t, s[0].time_base), (unit_t(i), 1));
    }
}

// ------------------------------------------------------------ refusals at the write

#[test]
fn wp_atfake_op9_write_refusals() {
    for env in ENVS {
        let tb = FakeTimeBase::line(0x42);
        let mut d = line(&tb, env);
        stage(&mut d, &sweep());
        let now = entry(&mut d, env).t;
        // Less than arm_lead ahead, or in the past.
        refused(
            set(&mut d, env, EXT_APPLY_AT_FRAME, &op9(now + 100, 0)),
            env,
            lm::VC_ERR_STATE,
        );
        refused(
            set(&mut d, env, EXT_APPLY_AT_FRAME, &op9(now - 10, 0)),
            env,
            lm::VC_ERR_STATE,
        );
        // More than 120 s ahead (FWTB1).
        let far = now + 121 * 48_000;
        refused(
            set(&mut d, env, EXT_APPLY_AT_FRAME, &op9(far, 0)),
            env,
            lm::VC_ERR_STATE,
        );
        // Arguments: op 9 arg 0 value 0; op 10 arg 0..1.
        refused(
            set(
                &mut d,
                env,
                EXT_APPLY_AT_FRAME,
                &req(now + LEAD, ApplyOp::SiggenStart, 1, 0),
            ),
            env,
            lm::VC_ERR_VALUE,
        );
        refused(
            set(&mut d, env, EXT_APPLY_AT_FRAME, &op10(now + LEAD, 2, 0)),
            env,
            lm::VC_ERR_VALUE,
        );
        // One op 9 at a time.
        set(&mut d, env, EXT_APPLY_AT_FRAME, &op9(now + 10 * LEAD, 0)).unwrap();
        refused(
            set(&mut d, env, EXT_APPLY_AT_FRAME, &op9(now + 11 * LEAD, 0)),
            env,
            lm::VC_ERR_STATE,
        );
        // The time-base reads are IN only.
        refused(
            set(&mut d, env, tbc::EXT_TIME_BASE, &[0; 4]),
            env,
            lm::VC_ERR_DIRECTION,
        );
        refused(
            set(&mut d, env, tbc::EXT_SIGGEN_TIMELINE, &[0; 4]),
            env,
            lm::VC_ERR_DIRECTION,
        );

        // A full queue refuses op 9 too.
        let mut d = line(&tb, env);
        stage(&mut d, &sweep());
        let now = entry(&mut d, env).t;
        for i in 0..4 {
            set(
                &mut d,
                env,
                EXT_APPLY_AT_FRAME,
                &req(now + 48_000 + i, ApplyOp::Mute, 0, 0),
            )
            .unwrap();
        }
        refused(
            set(&mut d, env, EXT_APPLY_AT_FRAME, &op9(now + LEAD, 0)),
            env,
            lm::VC_ERR_STATE,
        );

        // USB: never in the bus-frame form (time base 2); STATE when not locked.
        let tb = FakeTimeBase::usb_direct();
        let mut u = usb(&tb, 3, env);
        stage(&mut u, &sweep());
        let s = entry(&mut u, env).t;
        let bus = (s / 48 + 200) % 2048;
        refused(
            set(&mut u, env, EXT_APPLY_AT_FRAME, &op9(bus, 2)),
            env,
            lm::VC_ERR_VALUE,
        );
        // Op 10 may use it.
        set(&mut u, env, EXT_APPLY_AT_FRAME, &op10(bus, 1, 2)).unwrap();
        assert!(u.remote().time_base_fault(TimeBaseFault::Holdover));
        refused(
            set(&mut u, env, EXT_APPLY_AT_FRAME, &op9(s + 10 * LEAD, 1)),
            env,
            lm::VC_ERR_STATE,
        );
    }
}

// ------------------------------------------------------------ preconditions

/// A unit in `prepare`d state; op 9 just beyond the arming edge; it must
/// complete refused (not ready) at arming, with `0xE2` refused and nothing
/// started.
fn refused_not_ready(env: Env, prepare: impl FnOnce(&mut FakeDevice, Env)) {
    let tb = FakeTimeBase::line(0x42);
    let mut d = line(&tb, env);
    prepare(&mut d, env);
    let mut n = d.notifications().unwrap();
    let t = entry(&mut d, env).t + ARM + 240;
    set(&mut d, env, EXT_APPLY_AT_FRAME, &op9(t, 0)).unwrap();
    assert!(wait_for(
        || timeline(&mut d, env).last == tr::REFUSED_NOT_READY
    ));
    let tl = timeline(&mut d, env);
    assert_eq!(
        (tl.state, tl.pending, tl.run_id),
        (tbc::TL_IDLE, 0, 0),
        "{tl:?}"
    );
    assert!(e2s(&drain(&mut n)).contains(&(9, 3)));
    sleep_ms(30);
    assert_eq!(status(&mut d).state, 0, "never started");
}

#[test]
fn wp_atfake_op9_needs_a_finite_freshly_staged_config() {
    for env in ENVS {
        // Nothing staged since boot.
        refused_not_ready(env, |_, _| {});
        // A continuous type, RAW, a sweep without a repeat count.
        refused_not_ready(env, |d, _| {
            stage(
                d,
                &SiggenConfig {
                    signal_type: 0,
                    channel_mask: 1,
                    level_db: -40.0,
                    p1: 1000.0,
                    repeat_count: 1,
                    ..SiggenConfig::default()
                },
            )
        });
        refused_not_ready(env, |d, _| {
            stage(
                d,
                &SiggenConfig {
                    flags: SiggenConfig::FLAG_RAW,
                    ..sweep()
                },
            )
        });
        refused_not_ready(env, |d, _| {
            stage(
                d,
                &SiggenConfig {
                    repeat_count: 0,
                    ..sweep()
                },
            )
        });
        // Fresh, then a host STOP or STOP_NOW, an applied op 10, CANCEL_ALL,
        // a preset load, a loss.
        for ctl in [0u16, 2] {
            refused_not_ready(env, move |d, _| {
                stage(d, &sweep());
                control(d, ctl);
            });
        }
        refused_not_ready(env, |d, env| {
            stage(d, &sweep());
            let now = entry(d, env).t;
            set(d, env, EXT_APPLY_AT_FRAME, &op10(now - 1, 1, 0)).unwrap();
        });
        refused_not_ready(env, |d, env| {
            stage(d, &sweep());
            set(
                d,
                env,
                EXT_APPLY_AT_FRAME,
                &req(0, ApplyOp::CancelAll, 0, 0),
            )
            .unwrap();
        });
        refused_not_ready(env, |d, _| {
            stage(d, &sweep());
            d.control_in(op::REQ_PRESET_LOAD, 0, 1).unwrap();
        });
        refused_not_ready(env, |d, _| {
            stage(d, &sweep());
            assert!(d.remote().time_base_fault(TimeBaseFault::Holdover));
            assert!(d.remote().time_base_fault(TimeBaseFault::Resume));
        });
        // A run that ended by itself keeps the flag: the next pass starts.
        let tb = FakeTimeBase::line(0x42);
        let mut d = line(&tb, env);
        stage(&mut d, &sweep_ms(30));
        for pass in 1..=2 {
            let t = entry(&mut d, env).t + ARM + 240;
            set(&mut d, env, EXT_APPLY_AT_FRAME, &op9(t, 0)).unwrap();
            assert!(wait_for(|| {
                let tl = timeline(&mut d, env);
                tl.run_id == pass && tl.state == tbc::TL_IDLE
            }));
            assert_eq!(timeline(&mut d, env).last, tr::STARTED);
        }
    }
}

#[test]
fn wp_atfake_op9_is_refused_when_not_ready_at_t() {
    for env in ENVS {
        // The generator still runs at T (a plain START): refused, the run untouched.
        let tb = FakeTimeBase::line(0x42);
        let mut d = line(&tb, env);
        stage(&mut d, &sweep_ms(5_000));
        control(&mut d, 1);
        let plain = timeline(&mut d, env);
        assert_eq!(plain.run_id, 1, "a plain START is a run");
        assert_eq!(plain.tb, 0, "stamped on the line");
        assert!(plain.start > 0);
        let t = entry(&mut d, env).t + ARM + 240;
        set(&mut d, env, EXT_APPLY_AT_FRAME, &op9(t, 0)).unwrap();
        assert!(wait_for(
            || timeline(&mut d, env).last == tr::REFUSED_NOT_READY
        ));
        let tl = timeline(&mut d, env);
        assert_eq!(
            (tl.run_id, tl.start, tl.state),
            (1, plain.start, tbc::TL_RUNNING)
        );
        assert_ne!(status(&mut d).state, 0);
        control(&mut d, 2);

        // Muted by REQ_SET_USER_MUTE at T.
        let mut d = line(&tb, env);
        stage(&mut d, &sweep());
        d.control_out(op::REQ_SET_USER_MUTE, 0, &[1]).unwrap();
        let t = entry(&mut d, env).t + ARM + 240;
        set(&mut d, env, EXT_APPLY_AT_FRAME, &op9(t, 0)).unwrap();
        assert!(wait_for(
            || timeline(&mut d, env).last == tr::REFUSED_NOT_READY
        ));
        assert_eq!(status(&mut d).state, 0);

        // T skipped by a whole-sample correction: refused late.
        let mut d = line(&tb, env);
        stage(&mut d, &sweep());
        let mut n = d.notifications().unwrap();
        let t = entry(&mut d, env).t + 10 * LEAD;
        set(&mut d, env, EXT_APPLY_AT_FRAME, &op9(t, 0)).unwrap();
        assert!(d.remote().time_base_fault(TimeBaseFault::Correction {
            samples: 20 * LEAD as i32,
            event: true,
        }));
        let tl = timeline(&mut d, env);
        assert_eq!(
            (tl.last, tl.state, tl.pending),
            (tr::REFUSED_LATE, tbc::TL_IDLE, 0)
        );
        let ps = drain(&mut n);
        assert!(e2s(&ps).contains(&(9, 3)), "refused, never late: {ps:x?}");
        assert!(!e2s(&ps).contains(&(9, 1)));
    }
}

// ------------------------------------------------------------ cancels

fn pending_unit(env: Env) -> (FakeDevice, Box<dyn NotificationSource>) {
    let tb = FakeTimeBase::line(0x42);
    let mut d = line(&tb, env);
    stage(&mut d, &sweep());
    let n = d.notifications().unwrap();
    let t = entry(&mut d, env).t + 20 * LEAD;
    set(&mut d, env, EXT_APPLY_AT_FRAME, &op9(t, 0)).unwrap();
    assert_eq!(timeline(&mut d, env).state, tbc::TL_PENDING);
    (d, n)
}

fn cancelled(mut d: FakeDevice, mut n: Box<dyn NotificationSource>, env: Env, result: u8) {
    let tl = timeline(&mut d, env);
    assert_eq!(
        (tl.last, tl.state, tl.pending),
        (result, tbc::TL_IDLE, 0),
        "{tl:?}"
    );
    assert!(e2s(&drain(&mut n)).contains(&(9, 2)), "0xE2 cancelled");
    let q = get(&mut d, env, EXT_APPLY_AT_FRAME, 16);
    assert_eq!(q[0], 0, "out of the queue");
}

#[test]
fn wp_atfake_every_cancel_of_a_pending_op9() {
    for env in ENVS {
        let (mut d, n) = pending_unit(env);
        stage(&mut d, &sweep());
        cancelled(d, n, env, tr::CANCELLED_CONFIG);
        for ctl in [0u16, 2] {
            let (mut d, n) = pending_unit(env);
            control(&mut d, ctl);
            cancelled(d, n, env, tr::CANCELLED_HOST);
        }
        let (mut d, n) = pending_unit(env);
        set(
            &mut d,
            env,
            EXT_APPLY_AT_FRAME,
            &req(0, ApplyOp::CancelAll, 0, 0),
        )
        .unwrap();
        cancelled(d, n, env, tr::CANCELLED_HOST);
        for f in [
            TimeBaseFault::Holdover,
            TimeBaseFault::Reanchor,
            TimeBaseFault::LockError(true),
        ] {
            let (d, n) = pending_unit(env);
            assert!(d.remote().time_base_fault(f));
            cancelled(d, n, env, tr::CANCELLED_LOSS);
        }
        // USB: a lost bus cancels too.
        let tb = FakeTimeBase::usb_direct();
        let mut u = usb(&tb, 2, env);
        stage(&mut u, &sweep());
        let mut n = u.notifications().unwrap();
        let t = entry(&mut u, env).t + 20 * LEAD;
        set(&mut u, env, EXT_APPLY_AT_FRAME, &op9(t, 1)).unwrap();
        assert!(u.remote().usb_direct_sim(crate::fake::USB_SIM_SOF_LOST));
        let tl = timeline(&mut u, env);
        assert_eq!((tl.last, tl.pending), (tr::CANCELLED_LOSS, 0));
        assert!(e2s(&drain(&mut n)).contains(&(9, 2)));
    }
}

#[test]
fn wp_atfake_a_loss_stops_an_op9_run_but_not_a_plain_one() {
    for env in ENVS {
        let tb = FakeTimeBase::line(0x42);
        let mut d = line(&tb, env);
        stage(&mut d, &sweep_ms(5_000));
        let t = entry(&mut d, env).t + ARM + 240;
        set(&mut d, env, EXT_APPLY_AT_FRAME, &op9(t, 0)).unwrap();
        assert!(wait_for(|| timeline(&mut d, env).state == tbc::TL_RUNNING));
        assert!(d.remote().alink_sim(ALINK_SIM_LOSE));
        let tl = timeline(&mut d, env);
        assert_eq!(
            (tl.last, tl.state, tl.disc),
            (tr::STOPPED_LOSS, tbc::TL_IDLE, 1)
        );
        let s = status(&mut d);
        assert_eq!((s.state, s.stop_reason), (0, 1), "as STOP_NOW (HOST)");

        // A plain START plays on through the loss; the holdover is counted.
        let mut d = line(&tb, env);
        stage(&mut d, &sweep_ms(5_000));
        control(&mut d, 1);
        assert!(d.remote().time_base_fault(TimeBaseFault::Holdover));
        let tl = timeline(&mut d, env);
        assert_eq!((tl.state, tl.disc, tl.last), (tbc::TL_RUNNING, 1, tr::NONE));
        assert_ne!(status(&mut d).state, 0);
        control(&mut d, 2);
    }
}

// ------------------------------------------------------------ op 10

#[test]
fn wp_atfake_op10_stops_at_t_and_a_late_one_at_once() {
    for env in ENVS {
        let tb = FakeTimeBase::line(0x42);
        let mut d = line(&tb, env);
        stage(&mut d, &sweep_ms(5_000));
        let mut n = d.notifications().unwrap();
        let now = entry(&mut d, env).t;
        set(&mut d, env, EXT_APPLY_AT_FRAME, &op9(now + ARM + 240, 0)).unwrap();
        set(
            &mut d,
            env,
            EXT_APPLY_AT_FRAME,
            &op10(now + ARM + 240 + 4800, 1, 0),
        )
        .unwrap();
        // Started, then stopped at its T, 100 ms in: 4800 samples.
        assert!(wait_for(|| timeline(&mut d, env).run_id == 1));
        assert!(wait_for(|| status(&mut d).state == 0));
        let tl = timeline(&mut d, env);
        assert_eq!(tl.samples, 4800, "{tl:?}");
        assert!(e2s(&drain(&mut n)).contains(&(10, 0)));
        // A late op 10 is applied at once and reported late.
        stage(&mut d, &sweep_ms(5_000));
        control(&mut d, 1);
        let now = entry(&mut d, env).t;
        set(&mut d, env, EXT_APPLY_AT_FRAME, &op10(now - 48, 1, 0)).unwrap();
        assert_eq!(status(&mut d).state, 0);
        assert!(e2s(&drain(&mut n)).contains(&(10, 1)));
    }
}

// ------------------------------------------------------------ faults

#[test]
fn wp_atfake_every_fault_moves_disturbances_and_sends_0xe4() {
    for env in ENVS {
        let line_faults = [
            TimeBaseFault::Holdover,
            TimeBaseFault::Resume,
            TimeBaseFault::Reanchor,
            TimeBaseFault::Correction {
                samples: 1,
                event: true,
            },
            TimeBaseFault::LockError(true),
            TimeBaseFault::LockError(false),
        ];
        let mut spdif_faults = line_faults.to_vec();
        spdif_faults.insert(4, TimeBaseFault::Epoch);
        let cases: [(TimeBaseKind, Vec<TimeBaseFault>); 3] = [
            (TimeBaseKind::Line { sender_id: 0x42 }, line_faults.to_vec()),
            (TimeBaseKind::UsbDirect, line_faults.to_vec()),
            (TimeBaseKind::Spdif { source_id: 0x55 }, spdif_faults),
        ];
        for (kind, faults) in cases {
            let tb = FakeTimeBase::new(kind);
            let u = match kind {
                TimeBaseKind::Line { .. } => FakeTimeBaseUnit::Line(AlinkSetup::default()),
                TimeBaseKind::UsbDirect => FakeTimeBaseUnit::UsbDirect(UsbDirectSetup::default()),
                TimeBaseKind::Spdif { .. } => FakeTimeBaseUnit::Spdif(SpdifSetup::follower(192)),
            };
            let mut d = unit(&tb, u, env);
            let mut n = d.notifications().unwrap();
            for f in &faults {
                let before = entry(&mut d, env).dist;
                drain(&mut n);
                assert!(d.remote().time_base_fault(*f), "{kind:?} {f:?}");
                let e = entry(&mut d, env);
                let ev = e4s(&drain(&mut n));
                // Leaving lock error is no disturbance (entering it left LOCKED).
                if *f == TimeBaseFault::LockError(false) {
                    assert_eq!(e.dist, before, "{kind:?} {f:?}");
                } else {
                    assert!(e.dist > before, "{kind:?} {f:?}: {} -> {}", before, e.dist);
                }
                assert!(!ev.is_empty(), "{kind:?} {f:?}: no 0xE4");
                assert!(ev.iter().all(|x| x.0 == kind.time_base()));
                let want = match f {
                    TimeBaseFault::Holdover => (tbc::TB_STATE_HOLDOVER, reason::HOLDOVER),
                    TimeBaseFault::Resume => (tbc::TB_STATE_LOCKED, reason::RESUMED),
                    TimeBaseFault::Reanchor => (tbc::TB_STATE_ANCHORING, reason::REANCHOR),
                    TimeBaseFault::Correction { .. } => (tbc::TB_STATE_LOCKED, reason::CORRECTION),
                    TimeBaseFault::Epoch => (tbc::TB_STATE_ANCHORING, reason::EPOCH),
                    TimeBaseFault::LockError(true) => {
                        (tbc::TB_STATE_LOCK_ERROR, reason::LOCK_ERROR)
                    }
                    TimeBaseFault::LockError(false) => (tbc::TB_STATE_LOCKED, reason::LOCK_OK),
                };
                assert_eq!((ev[0].1, ev[0].2), want, "{kind:?} {f:?}: {ev:?}");
                assert_eq!(e.reason, ev.last().unwrap().2, "last_reason");
                assert_eq!(e.state, ev.last().unwrap().1);
            }
            // An epoch is the S/PDIF source's only.
            if !matches!(kind, TimeBaseKind::Spdif { .. }) {
                assert!(!d.remote().time_base_fault(TimeBaseFault::Epoch));
            }
        }
    }
}

#[test]
fn wp_atfake_a_silent_correction_only_moves_t_minus_output_frames() {
    for env in ENVS {
        let tb = FakeTimeBase::usb_direct();
        let mut d = usb(&tb, 2, env);
        let mut n = d.notifications().unwrap();
        let r0 = time_base(&mut d, env);
        let inv0 = low32(r0.entries[0].t).wrapping_sub(r0.out);
        assert!(d.remote().time_base_fault(TimeBaseFault::Correction {
            samples: 3,
            event: false
        }));
        let r1 = time_base(&mut d, env);
        let inv1 = low32(r1.entries[0].t).wrapping_sub(r1.out);
        assert_eq!(inv1.wrapping_sub(inv0), 3);
        assert_eq!(r1.entries[0].dist, r0.entries[0].dist);
        assert!(e4s(&drain(&mut n)).is_empty());
    }
}

#[test]
fn wp_atfake_an_epoch_moves_the_domain_and_every_follower_reanchors() {
    for env in ENVS {
        let tb = FakeTimeBase::spdif(0x3742);
        let mut src = unit(&tb, FakeTimeBaseUnit::Spdif(SpdifSetup::source()), env);
        let mut f = unit(&tb, FakeTimeBaseUnit::Spdif(SpdifSetup::follower(200)), env);
        let mut n = f.notifications().unwrap();
        let before = entry(&mut f, env);
        stage(&mut f, &sweep());
        let tt = before.t + 20 * LEAD;
        set(&mut f, env, EXT_APPLY_AT_FRAME, &op9(tt, 3)).unwrap();
        // The source's outputs restart: a preset load is one.
        src.control_in(op::REQ_PRESET_LOAD, 0, 1).unwrap();
        assert_eq!(tb.epoch(), 2);
        let s = entry(&mut src, env);
        let a = entry(&mut f, env);
        assert_eq!(a.domain, 0x0302_3742);
        assert_eq!(s.domain, a.domain);
        assert!(a.t > before.t + 192_000);
        assert_eq!(a.dist, before.dist + 1);
        assert_eq!(a.reason, reason::LOCKED);
        let ev = e4s(&drain(&mut n));
        assert_eq!(ev[0], (3, tbc::TB_STATE_ANCHORING, reason::EPOCH), "{ev:?}");
        // The pending START did not survive the re-anchor.
        assert_eq!(timeline(&mut f, env).last, tr::CANCELLED_LOSS);
        // A source's t - output_frames is its origin, a multiple of 192.
        let r = time_base(&mut src, env);
        let origin = r.entries[0].t - u64::from(r.out);
        assert_eq!(origin % 192, 0, "{origin}");
        // EXT_ALINK_SPDIF_STATUS on the follower: source id, epoch, F.
        let st = get(&mut f, env, tbc::EXT_ALINK_SPDIF_STATUS, 48);
        assert_eq!((st[0], st[1], st[2], st[3] & 0b11), (3, 2, 1, 0b11));
        assert_eq!((u16_at(&st, 4), st[6]), (0x3742, 2));
        assert!(u64_at(&st, 8) >= a.t);
        assert_eq!(
            (u16_at(&st, 24), u16_at(&st, 26)),
            (SPDIF_D_MIN, SPDIF_D_MAX)
        );
        assert_eq!(u32_at(&st, 44), 1, "one re-anchor");
    }
}

// ------------------------------------------------------------ broadcasts

#[test]
fn wp_atfake_the_line_superframe_stop_reaches_every_receiver() {
    for env in ENVS {
        let tb = FakeTimeBase::line(0x42);
        let mut sender = unit(
            &tb,
            FakeTimeBaseUnit::Line(AlinkSetup {
                role: AlinkRole::Sender,
                ..AlinkSetup::default()
            }),
            env,
        );
        let mut rs: Vec<_> = (0..2).map(|_| line(&tb, env)).collect();
        let t = entry(&mut rs[0], env).t + ARM + 480;
        for d in rs.iter_mut() {
            stage(d, &sweep_ms(5_000));
            set(d, env, EXT_APPLY_AT_FRAME, &op9(t, 0)).unwrap();
        }
        for d in rs.iter_mut() {
            assert!(wait_for(|| timeline(d, env).state == tbc::TL_RUNNING));
        }
        // The hub's abort: op 10 "now" in the superframe.
        let now = tb.t_now();
        let stop = alink::ctrl_msg_min_frame(now).unwrap() + 32;
        set(&mut sender, env, EXT_ALINK_CTRL_MSG, &op10(stop, 1, 0)).unwrap();
        for d in rs.iter_mut() {
            assert!(wait_for(|| status(d).state == 0));
            let tl = timeline(d, env);
            assert_eq!(
                (tl.state, tl.start, tl.last),
                (tbc::TL_IDLE, t, tr::STARTED)
            );
        }
    }
    // A sender without a time base refuses ops 9 and 10.
    let mut old = FakeDevice::demo_alink_as(AlinkSetup {
        role: AlinkRole::Sender,
        ..AlinkSetup::default()
    });
    let f = old
        .control_in(op::REQ_EXTENSION, alink::EXT_FRAME_COUNTER, 16)
        .unwrap();
    let stop = alink::ctrl_msg_min_frame(u64_at(&f, 0)).unwrap() + 4800;
    let m = op10(stop, 1, 0);
    assert!(
        old.control_out(op::REQ_EXTENSION, EXT_ALINK_CTRL_MSG, &m)
            .is_err()
    );
}

#[test]
fn wp_atfake_the_spdif_broadcast_reaches_every_follower_and_the_source() {
    for env in ENVS {
        let tb = FakeTimeBase::spdif(0x0042);
        let mut src = unit(&tb, FakeTimeBaseUnit::Spdif(SpdifSetup::source()), env);
        let mut fs: Vec<_> = (0..2)
            .map(|_| unit(&tb, FakeTimeBaseUnit::Spdif(SpdifSetup::follower(192)), env))
            .collect();
        let now = entry(&mut src, env).t;
        let ctrl = tbc::EXT_ALINK_SPDIF_CTRL;
        // Below the 896-frame lead, the wrong time base, ops never carried.
        refused(
            set(&mut src, env, ctrl, &op10(now + 800, 1, 3)),
            env,
            lm::VC_ERR_STATE,
        );
        refused(
            set(&mut src, env, ctrl, &op10(now + 4800, 1, 0)),
            env,
            lm::VC_ERR_VALUE,
        );
        refused(
            set(
                &mut src,
                env,
                ctrl,
                &req(now + 4800, ApplyOp::PresetLoad, 1, 3),
            ),
            env,
            lm::VC_ERR_VALUE,
        );
        refused(
            set(&mut src, env, ctrl, &req(0, ApplyOp::CancelAll, 0, 3)),
            env,
            lm::VC_ERR_VALUE,
        );
        // A follower is no source.
        refused(
            set(&mut fs[0], env, ctrl, &op10(now + 4800, 1, 3)),
            env,
            lm::VC_ERR_STATE,
        );
        // Every unit runs; one broadcast stop ends all three runs.
        let t = now + ARM + 480;
        for d in fs.iter_mut().chain(std::iter::once(&mut src)) {
            stage(d, &sweep_ms(5_000));
            set(d, env, EXT_APPLY_AT_FRAME, &op9(t, 3)).unwrap();
        }
        for d in fs.iter_mut().chain(std::iter::once(&mut src)) {
            assert!(wait_for(|| timeline(d, env).state == tbc::TL_RUNNING));
        }
        let stop = entry(&mut src, env).t + 1000;
        set(&mut src, env, ctrl, &op10(stop, 1, 3)).unwrap();
        // One at a time.
        refused(
            set(&mut src, env, ctrl, &op10(stop + 1000, 1, 3)),
            env,
            lm::VC_ERR_STATE,
        );
        let q = get(&mut src, env, ctrl, 16);
        assert_eq!(q[0], 1, "pending");
        for d in fs.iter_mut().chain(std::iter::once(&mut src)) {
            assert!(wait_for(|| status(d).state == 0));
        }
        let q = get(&mut src, env, ctrl, 16);
        assert_eq!((q[0], q[1], q[2], u64_at(&q, 4)), (0, 10, 0, stop));
        assert_eq!(
            get(&mut fs[0], env, ctrl, 16),
            vec![0; 16],
            "a follower reads zero"
        );
        // A broadcast op 9 inside the arm lead is refused on every follower.
        for d in fs.iter_mut() {
            stage(d, &sweep());
        }
        let short = entry(&mut src, env).t + 1000;
        set(&mut src, env, ctrl, &op9(short, 3)).unwrap();
        for d in fs.iter_mut() {
            assert_eq!(timeline(d, env).last, tr::REFUSED_NOT_READY);
        }
    }
}

// ------------------------------------------------------------ the S/PDIF block

#[test]
fn wp_atfake_alink_spdif_config_refusals_and_round_trip() {
    for env in ENVS {
        let tb = FakeTimeBase::spdif(0x3742);
        let mut f = unit(&tb, FakeTimeBaseUnit::Spdif(SpdifSetup::follower(192)), env);
        // 2.10.1's example: follower, AUTO_REANCHOR, D 192, mute, locked.
        assert_eq!(
            get(&mut f, env, tbc::EXT_ALINK_SPDIF, 8),
            vec![1, 1, 0xc0, 0, 0, 0, 0, 3]
        );
        for bad in [
            [3u8, 1, 0xc0, 0, 0, 0, 0, 0], // mode above 2
            [1, 3, 0xc0, 0, 0, 0, 0, 0],   // undefined flag
            [1, 1, 0xc0, 0, 0, 1, 0, 0],   // reserved
            [1, 1, 10, 0, 0, 0, 0, 0],     // D below d_min
            [2, 1, 0, 0, 0, 0, 0x10, 0],   // not an S/PDIF slot
        ] {
            refused(
                set(&mut f, env, tbc::EXT_ALINK_SPDIF, &bad),
                env,
                lm::VC_ERR_VALUE,
            );
        }
        let mut n = f.notifications().unwrap();
        // A new D re-anchors.
        set(
            &mut f,
            env,
            tbc::EXT_ALINK_SPDIF,
            &[1, 1, 0xc8, 0, 1, 0, 0, 0],
        )
        .unwrap();
        assert_eq!(
            e4s(&drain(&mut n)).first(),
            Some(&(3, tbc::TB_STATE_ANCHORING, reason::REANCHOR))
        );
        assert!(wait_for(|| entry(&mut f, env).state == tbc::TB_STATE_LOCKED));
        assert_eq!(
            get(&mut f, env, tbc::EXT_ALINK_SPDIF, 8),
            vec![1, 1, 0xc8, 0, 1, 0, 0, 3]
        );
        // Mode 0: no entry, DISABLED, its queue cancelled.
        let t = entry(&mut f, env).t + 48_000;
        set(
            &mut f,
            env,
            EXT_APPLY_AT_FRAME,
            &req(t, ApplyOp::Mute, 0, 3),
        )
        .unwrap();
        set(
            &mut f,
            env,
            tbc::EXT_ALINK_SPDIF,
            &[0, 1, 0xc8, 0, 0, 0, 0, 0],
        )
        .unwrap();
        let ps = drain(&mut n);
        assert!(e4s(&ps).contains(&(3, tbc::TB_STATE_NONE, reason::DISABLED)));
        assert!(e2s(&ps).contains(&(1, 2)));
        assert!(time_base(&mut f, env).entries.is_empty());
        refused(
            set(
                &mut f,
                env,
                EXT_APPLY_AT_FRAME,
                &req(t, ApplyOp::Mute, 0, 3),
            ),
            env,
            lm::VC_ERR_STATE,
        );
        // The source's example: slot 0 only.
        let mut s = unit(&tb, FakeTimeBaseUnit::Spdif(SpdifSetup::source()), env);
        set(&mut s, env, tbc::EXT_ALINK_SPDIF, &[2, 1, 0, 0, 0, 0, 1, 0]).unwrap();
        assert!(wait_for(|| entry(&mut s, env).state == tbc::TB_STATE_LOCKED));
        assert_eq!(
            get(&mut s, env, tbc::EXT_ALINK_SPDIF, 8),
            vec![2, 1, 0, 0, 0, 0, 1, 3]
        );
        let st = get(&mut s, env, tbc::EXT_ALINK_SPDIF_STATUS, 48);
        assert_eq!(st[3] & 0x10, 0x10, "SOURCE");
        assert_eq!(
            (u16_at(&st, 24), u32_at(&st, 40)),
            (0, 0),
            "follower-only fields"
        );
        // STATUS is IN only.
        refused(
            set(&mut s, env, tbc::EXT_ALINK_SPDIF_STATUS, &[0; 4]),
            env,
            lm::VC_ERR_DIRECTION,
        );
    }
}

#[test]
fn wp_atfake_spdif_queue_ops_and_late() {
    for env in ENVS {
        let tb = FakeTimeBase::spdif(1);
        let mut f = unit(&tb, FakeTimeBaseUnit::Spdif(SpdifSetup::follower(192)), env);
        let mut n = f.notifications().unwrap();
        let now = entry(&mut f, env).t;
        set(
            &mut f,
            env,
            EXT_APPLY_AT_FRAME,
            &req(now - 1, ApplyOp::Mute, 0, 3),
        )
        .unwrap();
        set(
            &mut f,
            env,
            EXT_APPLY_AT_FRAME,
            &req(now + 480, ApplyOp::Unmute, 0, 3),
        )
        .unwrap();
        // RATE is never queued here; reserved bytes must be 0.
        refused(
            set(
                &mut f,
                env,
                EXT_APPLY_AT_FRAME,
                &req(now + 480, ApplyOp::Rate, 0, 3),
            ),
            env,
            lm::VC_ERR_VALUE,
        );
        let mut b = req(now + 480, ApplyOp::Mute, 0, 3);
        b[14] = 1;
        refused(
            set(&mut f, env, EXT_APPLY_AT_FRAME, &b),
            env,
            lm::VC_ERR_VALUE,
        );
        assert!(wait_for(|| get(&mut f, env, EXT_APPLY_AT_FRAME, 16)[0] == 0));
        let q = get(&mut f, env, EXT_APPLY_AT_FRAME, 16);
        assert_eq!((q[1], q[2], u64_at(&q, 4)), (2, 0, now + 480));
        let ev = drain(&mut n);
        assert!(e2s(&ev).contains(&(1, 1)) && e2s(&ev).contains(&(2, 0)));
        // 0xE2 of time base 3 carries F's low 32 bits.
        let p = ev.iter().find(|p| p[1] == 0xE2 && p[4] == 2).unwrap();
        assert_eq!(u32_at(p, 8), low32(now + 480));
    }
}

// ------------------------------------------------------------ properties

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Untrusted writes to every id of the block never panic, and a refused
    /// write changes nothing a read shows.
    #[test]
    fn proptest_wp_atfake_arbitrary_writes_never_panic(
        id in prop::sample::select(vec![0x8006u16, 0x8020, 0x8021, 0x8022, 0x8030, 0x8031]),
        data in prop::collection::vec(any::<u8>(), 0..24),
        tb3 in any::<bool>(),
    ) {
        let tb = FakeTimeBase::spdif(7);
        let mut d = FakeDevice::demo_on_time_base(
            &tb,
            FakeTimeBaseUnit::Spdif(SpdifSetup::source()),
        );
        let mut data = data;
        if tb3 && data.len() > 12 {
            data[12] = 3;
        }
        let before = d.control_in(op::REQ_EXTENSION, tbc::EXT_ALINK_SPDIF, 8).unwrap();
        let r = d.control_out(op::REQ_EXTENSION, id, &data);
        if r.is_err() {
            let after = d.control_in(op::REQ_EXTENSION, tbc::EXT_ALINK_SPDIF, 8).unwrap();
            prop_assert_eq!(before, after);
        }
        let tbr = d.control_in(op::REQ_EXTENSION, tbc::EXT_TIME_BASE, 64).unwrap();
        prop_assert_eq!(tbr.len(), 64);
        let tl = d.control_in(op::REQ_EXTENSION, tbc::EXT_SIGGEN_TIMELINE, 40).unwrap();
        prop_assert_eq!(tl.len(), 40);
    }

    /// T maps to an instant and back to the same T, at any distance.
    #[test]
    fn proptest_wp_atfake_t_and_instant_round_trip(ahead in 0u64..(48_000 * 3600)) {
        let tb = FakeTimeBase::line(1);
        let t = tb.t_now() + ahead;
        let at = tb.instant_of(t);
        let d = lock(&tb.domain);
        prop_assert_eq!(d.t_at(at), t);
    }
}
