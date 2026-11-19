//! Review AT-C: WP ATCORE's system measurement session (`DspiCore::
//! measure_system_*` on a DSPi Link session) against the **real** hub family
//! `system.measure.*` (WP ATHUB) and the real system model (WP ATHUBM), with
//! two ATFAKE units on one line time base. ATCORE's own tests play the hub
//! with a stub; this is the two sides of the contract meeting: the status
//! body under `status`, the codes, the settle wait, `run_id` and the
//! `EXT_SIGGEN_TIMELINE` proofs as the fakes (the firmware's model) give
//! them, `system.status` and `param.get_many` for the engine's terms, and
//! the apply, verify and Revert through `system.set` and `system.align`.
//!
//! The test is the microphone and the room. It runs in real time (the hub
//! and the fakes do): it reads every generator start the fakes record and
//! renders the staged sweep into the capture at
//! `T + play-out + out.delay + acoustic delay`, the play-out being what the
//! hub's alignment reports for the unit (`pathDelayFrames + effective +
//! latencyFrames`) and `out.delay` what the unit holds, so an applied
//! alignment is heard. About 40 s.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::needless_range_loop
)]

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dspi_core::{
    DspiCore, MeasureOutput, MeasureRole, MeasureSystemApplyStage, MeasureSystemPhase,
    MeasureSystemSetup, MeasureSystemState, MeasureSystemUnit,
};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig};
use dspi_measure::stimulus::{LogSweep, SweepConfig, WalkSchedule, peak_linear};
use dspi_proto::ext::alink::AlinkRole;
use dspi_transport::fake::{
    AlinkSetup, FakeGeneration, FakeTimeBase, FakeTimeBaseUnit, SpdifSetup,
};
use dspi_transport::{FakeDevice, FakeRemote, Transport};
use serde_json::{Value, json};

const RATE: u32 = 48_000;
const BLOCK: usize = 2_400;
/// The microphone's input latency, frames: the capture reaches the session
/// this long after the air (and the test learns of a start within it).
const MIC_LAG: u64 = 7_200;
/// Each unit's speaker to the microphone, samples: 3 ms and 5 ms.
const ACOUSTIC: [u64; 2] = [144, 240];
const SERIALS: [&str; 2] = ["ATC-A", "ATC-B"];

fn store_dir() -> PathBuf {
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join("review-atc").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn connect(hub: &Hub, serial: &str) -> Arc<DspiCore> {
    let port = hub.network().unwrap().port();
    let core = DspiCore::new();
    core.link_connect(
        format!("ws://127.0.0.1:{port}/dspi/v1"),
        "review-atc".into(),
        "0".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    let d = core
        .link_devices()
        .into_iter()
        .find(|d| d.serial == serial)
        .unwrap();
    core.link_select_device(d.handle, d.serial).unwrap();
    core.fetch_all().unwrap();
    core
}

fn hub_request(core: &DspiCore, t: &str, body: Value) -> Value {
    let Value::Object(m) = body else {
        panic!("an object")
    };
    core.link_hub_request(t, m)
        .unwrap_or_else(|e| panic!("{t}: {e}"))
}

/// The room and the microphone.
struct Room {
    tb: FakeTimeBase,
    remotes: Vec<FakeRemote>,
    /// Starts already rendered, per unit.
    seen: Vec<usize>,
    /// Domain T of capture frame 0.
    t0: u64,
    /// Frames pushed.
    pushed: u64,
    air: Vec<f32>,
    /// Per unit: its play-out, samples, and its output 0's `out.delay`, samples.
    playout: Vec<u64>,
    out_delay: Vec<u64>,
    seed: u64,
    /// Every start heard: `(unit, T, level dBFS, outputs at once)`.
    heard: Vec<(usize, u64, f32, u16)>,
    /// What the session said it waits for.
    words: Vec<String>,
}

impl Room {
    /// Render every start the fakes recorded since the last look.
    fn listen(&mut self) {
        for u in 0..self.remotes.len() {
            let _ = self.remotes[u].time_base_t();
            let starts = self.remotes[u].measure_timed_starts();
            for s in starts.iter().skip(self.seen[u]) {
                assert!(s.timed, "only op 9 starts a generator here: {s:?}");
                let c = &s.config;
                let sweep = LogSweep::new(
                    RATE,
                    &SweepConfig {
                        f1_hz: c.p1,
                        f2_hz: c.p2,
                        duration_ms: c.duration_ms,
                        gap_ms: c.gap_ms,
                    },
                )
                .unwrap();
                let walk = WalkSchedule::new(sweep, c.channel_mask, c.repeat_count).unwrap();
                let amp = peak_linear(c.level_db);
                let delay = self.playout[u] + self.out_delay[u] + ACOUSTIC[u];
                for cy in walk.cycles() {
                    assert_eq!(cy.output, 0, "only output 0 is measured");
                    let sign = if c.invert_mask & 1 != 0 { -1.0 } else { 1.0 };
                    let at = (s.t - self.t0 + cy.start + delay) as usize;
                    assert!(
                        at as u64 >= self.pushed,
                        "a start at frame {at} was learnt of too late (pushed {})",
                        self.pushed
                    );
                    let samples = sweep.render_cycle(cy.index == 0);
                    if self.air.len() < at + samples.len() {
                        self.air.resize(at + samples.len(), 0.0);
                    }
                    for (i, v) in samples.iter().enumerate() {
                        self.air[at + i] += *v as f32 * amp * sign;
                    }
                }
                self.heard
                    .push((u, s.t, c.level_db, c.channel_mask.count_ones() as u16));
            }
            self.seen[u] = starts.len();
        }
    }

    /// The capture the microphone has by now, in blocks.
    fn capture(&mut self, core: &DspiCore) {
        let have = (self.tb.t_now() - self.t0).saturating_sub(MIC_LAG);
        while self.pushed + BLOCK as u64 <= have {
            let base = self.pushed as usize;
            let block: Vec<f32> = (0..BLOCK)
                .map(|i| {
                    self.seed = self
                        .seed
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    let r = ((self.seed >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0;
                    self.air.get(base + i).copied().unwrap_or(0.0) + r * 3e-5
                })
                .collect();
            core.measure_system_capture_push(self.pushed, block, RATE, false)
                .unwrap();
            self.pushed += BLOCK as u64;
        }
    }

    /// Drive the core's session to its end.
    fn drive(&mut self, core: &DspiCore, what: &str) -> MeasureSystemState {
        let start = Instant::now();
        loop {
            self.listen();
            self.capture(core);
            let st = core.measure_system_tick();
            if !st.words.is_empty() && !self.words.contains(&st.words) {
                eprintln!("{what}: {}", st.words);
                self.words.push(st.words.clone());
            }
            if !st.active {
                return st;
            }
            assert!(
                start.elapsed() < Duration::from_secs(120),
                "{what} did not end: {st:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn out_delay_ms(core: &DspiCore, serial: &str) -> f64 {
    let v = hub_request(
        core,
        "param.get_many",
        json!({"serial": serial, "items": [{"path": "out.delay", "idx": [0]}]}),
    );
    v["values"][0]["value"].as_f64().unwrap()
}

/// The hub, two units and a sender, their cores, the checked setup and the room.
struct Rig {
    hub: Hub,
    a: Arc<DspiCore>,
    b: Arc<DspiCore>,
    setup: MeasureSystemSetup,
    checks: Vec<dspi_core::MeasureSystemUnitCheck>,
    room: Room,
    /// Each unit's `out.delay` before anything is measured, ms.
    before: [f64; 2],
}

fn delays(core: &DspiCore) -> Vec<u64> {
    SERIALS
        .iter()
        .map(|s| (out_delay_ms(core, s) * f64::from(RATE) / 1000.0).round() as u64)
        .collect()
}

/// The installation under test.
#[derive(Clone, Copy, PartialEq, Eq)]
enum World {
    /// Two receivers on one audio-link sender.
    Line,
    /// An S/PDIF source (unit A, slot 1 carries the time base) and its
    /// follower (unit B).
    Spdif,
}

fn rig(world: World, settle: Duration, sweep_ms: u32) -> Rig {
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(store_dir())
        }),
        initial_scan: false,
        ..HubConfig::in_process("review-atc hub")
    })
    .unwrap();
    // The tests decide when the system model is read.
    hub.handle().set_system_measure_settle(settle);
    hub.handle()
        .set_system_poll_interval(Duration::from_secs(3600));

    let tb = match world {
        World::Line => FakeTimeBase::line(0x21),
        World::Spdif => FakeTimeBase::spdif(0x1234),
    };
    let mut remotes = Vec::new();
    for (i, serial) in SERIALS.into_iter().enumerate() {
        let unit = match (world, i) {
            (World::Line, _) => FakeTimeBaseUnit::Line(AlinkSetup::default()),
            (World::Spdif, 0) => FakeTimeBaseUnit::Spdif(SpdifSetup {
                tx_mask: 0b10,
                ..SpdifSetup::source()
            }),
            (World::Spdif, _) => FakeTimeBaseUnit::Spdif(SpdifSetup::follower(192)),
        };
        let d = FakeDevice::demo_on_time_base_as(
            &tb,
            unit,
            FakeGeneration::Bofr116 { page_guard: true },
        )
        .with_serial(serial);
        assert_eq!(d.descriptor().serial, serial);
        remotes.push(d.remote());
        hub.attach(Box::new(d), 0).unwrap();
    }
    if world == World::Line {
        hub.attach(
            Box::new(
                FakeDevice::demo_alink_as(AlinkSetup {
                    role: AlinkRole::Sender,
                    ..AlinkSetup::default()
                })
                .with_serial("ATC-TX"),
            ),
            0,
        )
        .unwrap();
    }
    // Unit B has a millisecond of latency after the link: the engine gives
    // unit A that much more delay, so their play-outs are equal but their
    // terms are not.
    let model = match world {
        World::Line => json!({
            "rooms": [{"id": "main", "name": "Main"}],
            "senders": [{"id": "tx", "serial": "ATC-TX"}],
            "units": [
                {"id": "A", "serial": SERIALS[0], "room": "main", "upstream": "tx",
                 "speakers": [{"output": 0, "distanceM": 2.0}]},
                {"id": "B", "serial": SERIALS[1], "room": "main", "upstream": "tx",
                 "latencyUs": 1000, "speakers": [{"output": 0, "distanceM": 2.0}]}
            ]
        }),
        World::Spdif => json!({
            "rooms": [{"id": "main", "name": "Main"}],
            "units": [
                {"id": "A", "serial": SERIALS[0], "room": "main", "transport": "spdif",
                 "spdifSource": true, "spdifTxMask": 2,
                 "speakers": [{"output": 0, "distanceM": 2.0}]},
                {"id": "B", "serial": SERIALS[1], "room": "main", "transport": "spdif",
                 "upstream": "A", "latencyUs": 1000,
                 "speakers": [{"output": 0, "distanceM": 2.0}]}
            ]
        }),
    };
    hub.handle()
        .system_set_model(serde_json::from_value(model).unwrap())
        .unwrap();
    // The installation as the engine leaves it, before anything is measured.
    let first = hub.handle().system_align(true);
    eprintln!("first align: {:?}", first.writes);

    let a = connect(&hub, SERIALS[0]);
    let b = connect(&hub, SERIALS[1]);
    assert_eq!(a.link_hub_role(), "admin");

    let unit = |serial: &str| MeasureSystemUnit {
        serial: serial.into(),
        outputs: vec![MeasureOutput {
            output: 0,
            role: MeasureRole::Full,
            speaker: serial.into(),
            ..MeasureOutput::default()
        }],
    };
    let setup = MeasureSystemSetup {
        units: vec![unit(SERIALS[0]), unit(SERIALS[1])],
        passes: 3,
        sweep_ms,
        gap_ms: 300,
        capture_rate_hz: RATE,
        lease_ms: 5_000,
        ..MeasureSystemSetup::default()
    };
    // Each unit judged by its own core (P5's preflight on its own mirror):
    // S2 refuses a unit at full volume, naming it.
    let loud = a
        .measure_system_unit_check(setup.clone(), unit(SERIALS[0]))
        .unwrap();
    assert!(
        loud.preflight
            .blockers
            .iter()
            .any(|i| i.key == "measure.volume_high"),
        "{:?}",
        loud.preflight
    );
    let refused = a
        .measure_system_preflight(setup.clone(), vec![loud])
        .unwrap();
    assert!(!refused.clear);
    assert!(
        refused
            .blockers
            .iter()
            .any(|i| i.key == "measure.volume_high" && i.serial == SERIALS[0]),
        "{refused:?}"
    );
    a.set_user_volume(-20.0).unwrap();
    b.set_user_volume(-20.0).unwrap();
    let checks = vec![
        a.measure_system_unit_check(setup.clone(), unit(SERIALS[0]))
            .unwrap(),
        b.measure_system_unit_check(setup.clone(), unit(SERIALS[1]))
            .unwrap(),
    ];
    for c in &checks {
        assert!(c.preflight.clear, "{}: {:?}", c.serial, c.preflight);
    }
    let pre = a
        .measure_system_preflight(setup.clone(), checks.clone())
        .unwrap();
    assert!(pre.clear, "{pre:?}");

    // The room: what the hub's engine says each unit's play-out is.
    let status = hub.handle().system_status();
    let playout: Vec<u64> = SERIALS
        .iter()
        .map(|s| {
            let p = status
                .alignment
                .units
                .iter()
                .find(|u| u.serial == *s)
                .unwrap();
            // C2: a source plays T as it emits it (its lag is in its output
            // delays); a follower its D after T arrives.
            let source = p.spdif.as_ref().is_some_and(|s| s.mode == "source");
            if source {
                p.latency_frames as u64
            } else {
                (i64::from(p.path_delay_frames) + p.effective + p.latency_frames) as u64
            }
        })
        .collect();
    eprintln!(
        "play-outs {playout:?} samples, issues {:?}",
        status.alignment.issues
    );
    if world == World::Line {
        assert_eq!(playout[0], playout[1], "the engine equalises the play-outs");
    }
    let before = [out_delay_ms(&a, SERIALS[0]), out_delay_ms(&a, SERIALS[1])];
    let room = Room {
        t0: tb.t_now(),
        tb: tb.clone(),
        seen: remotes
            .iter()
            .map(|r| r.measure_timed_starts().len())
            .collect(),
        remotes,
        pushed: 0,
        air: Vec::new(),
        playout,
        out_delay: delays(&a),
        seed: 7,
        heard: Vec::new(),
        words: Vec::new(),
    };
    Rig {
        hub,
        a,
        b,
        setup,
        checks,
        room,
        before,
    }
}

/// Two receivers on one sender: measured, checked, applied (Admin, through
/// the hub's model), verified and reverted.
#[test]
fn review_atc_two_units_measured_applied_verified_and_reverted_on_the_real_hub() {
    // The first alignment wrote each unit's phase: the hub sees them locked
    // anew at `open` and holds the first START back until they have settled.
    whole(World::Line, Duration::from_secs(3));
}

/// An S/PDIF source and its follower, measured on the S/PDIF time base, the
/// same way. The source plays T as it emits it and waits the domain's D in
/// its output delays; the follower plays T its own D after it arrives. The
/// session takes each unit's play-out from the D the unit itself reports
/// (`EXT_TIME_BASE.d`, 0 for the source), never from an alignment the hub
/// computes while the session's locks keep it from reading the units: the
/// follower runs the engine's D of 150 samples, where the hub, unable to
/// read its limits during the session, would say the default 192 (the
/// source's speaker would be placed 875 us early).
#[test]
fn review_atc_an_spdif_source_and_its_follower_on_the_real_hub() {
    whole(World::Spdif, Duration::ZERO);
}

fn whole(world: World, settle: Duration) {
    let Rig {
        hub,
        a,
        b,
        setup,
        checks,
        mut room,
        before,
    } = rig(world, settle, 1_000);
    // Each unit judged anew by its own core, for a session that plays.
    let again = setup.clone();
    let fresh = || {
        vec![
            a.measure_system_unit_check(again.clone(), again.units[0].clone())
                .unwrap(),
            b.measure_system_unit_check(again.clone(), again.units[1].clone())
                .unwrap(),
        ]
    };
    if world == World::Spdif {
        eprintln!("out.delay {:?} samples", room.out_delay);
        assert!(room.out_delay[0] > 0, "the source waits the domain's D");
        assert_eq!(room.playout[0], 0, "and plays T as it emits it");
        assert_ne!(
            room.playout[1],
            192 + 48,
            "the follower's D is the engine's choice, not the default"
        );
    }

    // ── The measurement ────────────────────────────────────────────────
    let st = a.measure_system_start(setup, checks).unwrap();
    assert!(st.active && st.wants_capture, "{st:?}");
    assert_eq!(hub.handle().system_measure_sessions(), 1);
    for s in SERIALS {
        assert!(hub.handle().system_measure_lock_exempt(s));
    }
    let st = room.drive(&a, "measure");
    assert_eq!(st.phase, MeasureSystemPhase::Done, "{st:?}");
    assert!(st.result_ready);
    if !settle.is_zero() {
        assert!(
            room.words.iter().any(|w| w.contains("settle")),
            "the settle wait was said: {:?}",
            room.words
        );
    }
    assert_eq!(hub.handle().system_measure_sessions(), 0, "closed");
    for s in SERIALS {
        assert!(
            !hub.handle().system_measure_lock_exempt(s),
            "locks released"
        );
    }
    let result = a.measure_system_result().unwrap();
    eprintln!("arrivals {:?}", result.arrivals);
    eprintln!("reasons {:?}", result.reasons);
    assert!(result.timing_reliable, "{:?}", result.reasons);
    let arr = |serial: &str| {
        result
            .arrivals
            .iter()
            .find(|x| x.serial == serial)
            .unwrap()
            .clone()
    };
    // B is 2 ms further: the latest; A's acoustic arrival is 2 ms before it.
    assert!(
        arr(SERIALS[1]).arrival_us.abs() < 0.5,
        "{:?}",
        arr(SERIALS[1])
    );
    assert!(
        (arr(SERIALS[0]).arrival_us + 2_000.0).abs() < 5.0,
        "{:?}",
        arr(SERIALS[0])
    );
    assert!(arr(SERIALS[0]).offered && arr(SERIALS[1]).offered);
    assert_eq!(result.documents.len(), 2);
    // S-AT4, as heard: one generator at a time in the walked run and ramps.
    let walked = room.heard.len();
    for w in room.heard.windows(2) {
        assert!(w[1].1 > w[0].1, "starts in time order, never together");
    }

    // ── The summed check ───────────────────────────────────────────────
    let p = a.measure_system_proposal().unwrap();
    assert!(p.sum_check_needed && !p.offered, "{p:?}");
    assert_eq!(p.transitions.len(), 1);
    let measured_min = a
        .measure_system_state()
        .ramp_levels
        .iter()
        .map(|l| l.level_db)
        .fold(f32::INFINITY, f32::min);
    // A summed session is under every S-rule of its own: a unit turned up
    // since the measurement is refused, named, and nothing plays (S2).
    b.set_user_volume(0.0).unwrap();
    let heard = room.heard.len();
    let loud = a.measure_system_sum_check(fresh());
    assert!(loud.is_err(), "{loud:?}");
    assert!(
        format!("{loud:?}").contains(SERIALS[1]),
        "the unit is named: {loud:?}"
    );
    assert!(!a.measure_system_state().active);
    assert_eq!(hub.handle().system_measure_sessions(), 0);
    room.listen();
    assert_eq!(room.heard.len(), heard, "nothing played");
    b.set_user_volume(-20.0).unwrap();
    // A stale or missing check is refused too.
    assert!(a.measure_system_sum_check(Vec::new()).is_err());
    a.measure_system_sum_check(fresh()).unwrap();
    let st = room.drive(&a, "sum check");
    assert_eq!(st.phase, MeasureSystemPhase::Done, "{st:?}");
    // It ramped its two outputs again (S3).
    let ramp_min = st
        .ramp_levels
        .iter()
        .map(|l| l.level_db)
        .fold(f32::INFINITY, f32::min);
    assert_eq!(st.ramp_levels.len(), 2, "{st:?}");
    let p = a.measure_system_proposal().unwrap();
    eprintln!("transitions {:?}", p.transitions);
    assert!(p.transitions[0].checked && p.transitions[0].passed, "{p:?}");
    assert!(p.offered && p.may_apply && !p.withdrawn, "{p:?}");
    // S-AT4: after its own ramp steps (one generator at a time), the two
    // units started together, each 6 dB under the lower ramp result, never
    // above the measurement's. WP ATSUMOFF: the unit heard 2 ms (96
    // samples) earlier starts that much later, so both arrive together.
    let played = &room.heard[walked..];
    let (ramps, summed) = played.split_at(played.len() - 2);
    assert!(ramps.len() >= 2, "{played:?}");
    for w in played[..=ramps.len()].windows(2) {
        assert!(w[1].1 > w[0].1, "the ramp steps never together: {played:?}");
    }
    assert_eq!(summed[0].1.abs_diff(summed[1].1), 96, "{summed:?}");
    assert_ne!(summed[0].0, summed[1].0, "two units");
    for s in summed {
        assert!(
            s.2 <= ramp_min - 6.0 + 0.01 && s.2 <= measured_min - 6.0 + 0.01,
            "a summed run at {} dBFS, the ramps found {ramp_min} (the measurement's {measured_min})",
            s.2
        );
    }

    // ── Apply (Admin), through the hub's model ─────────────────────────
    let applied = a.measure_system_apply(p.id).unwrap();
    eprintln!("applied {applied:?}");
    assert_eq!(applied.stage, MeasureSystemApplyStage::Applied);
    assert!(applied.write_failures.is_empty(), "{applied:?}");
    assert!(applied.revision_after > applied.revision_before);
    let (model, _) = hub.handle().system_model();
    let m = |i: usize| model.units[i].speakers[0].measured_arrival_us;
    assert!((m(0).unwrap() + 2_000.0).abs() < 5.0 && m(1).unwrap().abs() < 0.5);
    let now = [out_delay_ms(&a, SERIALS[0]), out_delay_ms(&a, SERIALS[1])];
    assert!((now[0] - before[0] - 2.0).abs() < 0.006, "{now:?}");
    assert!((now[1] - before[1]).abs() < 0.0015, "{now:?}");

    // ── Verify: the summed runs again, with the new delays in the room ──
    room.out_delay = delays(&a);
    a.measure_system_verify(fresh()).unwrap();
    let st = room.drive(&a, "verify");
    assert_eq!(st.phase, MeasureSystemPhase::Done, "{st:?}");
    let v = a.measure_system_applied();
    assert_eq!(v.stage, MeasureSystemApplyStage::Verified, "{v:?}");
    assert_eq!(v.verified, Some(true), "{v:?}");

    // ── Revert: the model and the delays as filed ──────────────────────
    let r = a.measure_system_revert().unwrap();
    assert_eq!(r.stage, MeasureSystemApplyStage::Reverted);
    assert!(r.message.starts_with("reverted: "), "{r:?}");
    let (model, _) = hub.handle().system_model();
    assert!(
        model
            .units
            .iter()
            .all(|u| u.speakers[0].measured_arrival_us.is_none())
    );
    let back = [out_delay_ms(&a, SERIALS[0]), out_delay_ms(&a, SERIALS[1])];
    assert!(
        (back[0] - before[0]).abs() < 0.0015 && (back[1] - before[1]).abs() < 0.0015,
        "{back:?} where {before:?} was filed"
    );
}

/// S-AT1 over a real connection: the console vanishes in the middle of the
/// timed run (its Link connection closes, no abort sent). The hub's
/// dead-man stops the unit that plays, cancels the STARTs still queued and
/// releases the locks; nothing starts afterwards.
#[test]
fn review_atc_a_dropped_connection_stops_every_unit() {
    let Rig {
        hub,
        a,
        setup,
        checks,
        mut room,
        ..
    } = rig(World::Line, Duration::ZERO, 3_000);
    a.measure_system_start(setup, checks).unwrap();
    // Into the timed run: its first segment plays, the others wait.
    let start = Instant::now();
    let ramps = loop {
        room.listen();
        room.capture(&a);
        let st = a.measure_system_tick();
        assert!(st.active, "{st:?}");
        if st.phase == MeasureSystemPhase::Run && st.words.is_empty() {
            break room.heard.len();
        }
        assert!(start.elapsed() < Duration::from_secs(60), "{st:?}");
        std::thread::sleep(Duration::from_millis(10));
    };
    let start = Instant::now();
    while room.heard.len() == ramps {
        room.listen();
        room.capture(&a);
        a.measure_system_tick();
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "no start of the run"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(hub.handle().system_measure_sessions(), 1);
    // The run's first segment is unit A's: a 3 s sweep, just begun.
    let (unit, t, ..) = room.heard[ramps];
    assert_eq!(unit, 0);
    // Gone, without a word.
    a.link_close();
    let start = Instant::now();
    while hub.handle().system_measure_sessions() != 0 {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the hub kept the session of a closed connection"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    eprintln!(
        "the hub dropped the session {:?} after the close",
        start.elapsed()
    );
    for s in SERIALS {
        assert!(!hub.handle().system_measure_lock_exempt(s));
    }
    // Unit A's generator was stopped, well before its sweep's end.
    let c = connect(&hub, SERIALS[0]);
    let gen_state = c.siggen_get_status().unwrap();
    let played = room.tb.t_now() - t;
    assert!(
        played < 2 * u64::from(RATE),
        "read {played} samples after its start"
    );
    assert_eq!(gen_state.state, 0, "{gen_state:?}");
    // The run had five more segments: none of them starts.
    room.listen();
    let heard = room.heard.len();
    std::thread::sleep(Duration::from_secs(4));
    for (u, r) in room.remotes.iter().enumerate() {
        let _ = r.time_base_t();
        assert_eq!(
            r.measure_timed_starts().len(),
            room.seen[u],
            "unit {u} started after the abort"
        );
    }
    assert_eq!(room.heard.len(), heard);
}

// -- WP ATGUARD --
/// The core's apply on the real hub runs inside the hub's guard: measured,
/// summed-checked, applied by `measure_system_apply` (which begins the
/// guard, sized from the planned verify), verified inside it; then the
/// console vanishes without Keep. The hub reverts by itself: the filed
/// model, every delay as before.
#[test]
fn wp_atguard_the_cores_apply_reverts_when_the_console_drops() {
    use dspi_measure::system::messages::guard as wire;
    let Rig {
        hub,
        a,
        b,
        setup,
        checks,
        mut room,
        before,
    } = rig(World::Line, Duration::ZERO, 1_000);
    let again = setup.clone();
    let fresh = || {
        vec![
            a.measure_system_unit_check(again.clone(), again.units[0].clone())
                .unwrap(),
            b.measure_system_unit_check(again.clone(), again.units[1].clone())
                .unwrap(),
        ]
    };
    a.measure_system_start(setup, checks).unwrap();
    let st = room.drive(&a, "measure");
    assert_eq!(st.phase, MeasureSystemPhase::Done, "{st:?}");
    a.measure_system_sum_check(fresh()).unwrap();
    let st = room.drive(&a, "sum check");
    assert_eq!(st.phase, MeasureSystemPhase::Done, "{st:?}");
    let p = a.measure_system_proposal().unwrap();
    assert!(p.offered, "{p:?}");
    assert!(p.guard_s >= 120 && p.guard_s <= 590, "{p:?}");
    assert!(
        p.guard_note.contains("reverts the alignment by itself"),
        "{p:?}"
    );
    let applied = a.measure_system_apply(p.id).unwrap();
    assert_eq!(applied.stage, MeasureSystemApplyStage::Applied);
    assert_eq!(applied.guard_s, p.guard_s);
    let st = hub_request(&b, wire::STATUS, json!({}));
    let active = &st["active"];
    assert!(active.is_object(), "{st}");
    let remaining = active["remaining_ms"].as_u64().unwrap();
    assert!(
        remaining <= u64::from(p.guard_s) * 1000
            && remaining > u64::from(p.guard_s) * 1000 - 30_000,
        "{st}"
    );
    let now = [out_delay_ms(&b, SERIALS[0]), out_delay_ms(&b, SERIALS[1])];
    assert!((now[0] - before[0] - 2.0).abs() < 0.006, "{now:?}");
    // The verify, inside the guard.
    room.out_delay = delays(&a);
    a.measure_system_verify(fresh()).unwrap();
    let st = room.drive(&a, "verify");
    assert_eq!(st.phase, MeasureSystemPhase::Done, "{st:?}");
    assert_eq!(a.measure_system_applied().verified, Some(true));
    // Gone, without Keep.
    a.link_close();
    let start = Instant::now();
    let last = loop {
        let st = hub_request(&b, wire::STATUS, json!({}));
        if st["active"].is_null() && st["last"].is_object() {
            break st["last"].clone();
        }
        assert!(start.elapsed() < Duration::from_secs(10), "{st}");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(last["reason"], "session_closed", "{last}");
    assert_eq!(last["result"], "reverted", "{last}");
    let back = [out_delay_ms(&b, SERIALS[0]), out_delay_ms(&b, SERIALS[1])];
    assert!(
        (back[0] - before[0]).abs() < 0.0015 && (back[1] - before[1]).abs() < 0.0015,
        "{back:?} where {before:?} was filed"
    );
    let (model, _) = hub.handle().system_model();
    assert!(
        model
            .units
            .iter()
            .all(|u| u.speakers[0].measured_arrival_us.is_none())
    );
}
