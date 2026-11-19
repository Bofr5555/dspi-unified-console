//! WP ATE2E (PLAN-AT, wave AT-D): simulated multi-unit worlds end to end.
//! WP ATCORE's session (`DspiCore::measure_system_*` on a DSPi Link session)
//! measures through the **real** in-process hub (WP ATHUB's
//! `system.measure.*`, WP ATHUBM's model and alignment) on WP ATFAKE's units,
//! and WP ATSIM's world is the room: every generator start the fakes record
//! is rendered by ATSIM (the room's modes and reflections, a typical
//! microphone, each output's driver and path) and laid onto **one capture**
//! on the microphone's own clock: a ratio against the units' clock and, when
//! asked, P5SPIKE's wander bursts (`WanderBursts`, `wander_error_us`). Grown
//! from the AT-C review's rig (`review_atc_core_on_real_hub.rs`), which has
//! neither a clock ratio, nor a room, nor wander.
//!
//! **What is covered** (the brief's list):
//! - line, USB-direct and S/PDIF worlds, two to four units, one behind a
//!   fan-out with a one-frame hop error: the arrivals within 2 µs of the
//!   world's and within three of their own uncertainties;
//! - a group of 17 outputs split into two runs chained through the anchor,
//!   captured as one stream (the microphone stays open);
//! - every injected fault (holdover, re-anchor, a correction with and
//!   without its event, an S/PDIF epoch change, lock error, the hub dying)
//!   ends the run with the unit named (the hub's death: every unit plays at
//!   most one more segment) and nothing proposed;
//! - P5SPIKE's microphone: "timing unreliable", nothing offered;
//! - a steady microphone: apply, verify and Revert through the hub's model;
//! - the summed check failing on a wrong delay (a speaker moved after the
//!   walk) and the verify failing on one (a unit's delay changed after the
//!   apply), Keep refused once.
//!
//! **Time.** The hub and the fakes run in real time, so each test plays its
//! sweeps (1 s, 300 ms gaps, one ramp step per output). The heavy tests take
//! one of [`SLOTS`] slots so a parallel run does not starve the renders.
//! The wall time is set by the chained test (about two and a half minutes).
//!
//! **How the room hears** (one capture, no gaps): each loop reads the
//! domain's T, services every fake (a start is worked out at its exact T),
//! renders every new start with ATSIM in a short world of its own (the clock
//! exact, then laid onto the capture through the microphone's clock map with
//! a windowed-sinc interpolation), and pushes the capture up to what was
//! known at that T less the microphone's latency, so nothing is pushed before
//! it is rendered. A unit plays T at its play-out (the engine's terms, as in
//! the AT-C rig: D and the latency for a follower, the latency for an S/PDIF
//! source) plus its output delay plus a fan-out's hop error.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::needless_range_loop
)]

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use dspi_core::{
    DspiCore, MeasureOutput, MeasureRole, MeasureSystemApplyStage, MeasureSystemPhase,
    MeasureSystemSetup, MeasureSystemState, MeasureSystemUnit, MeasureSystemUnitCheck,
};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig};
use dspi_measure::sim::system::{
    SimUnit, SystemWorld, UnitLink, WanderBurst, WanderBursts, render_system, wander_error_us,
};
use dspi_measure::sim::{Acoustics, Microphone, ProtoModel, World, flat_chains};
use dspi_measure::stimulus::{LogSweep, SweepConfig, WalkSchedule};
use dspi_measure::system::{DomainId, SystemSchedule, UnitIndex};
use dspi_proto::ext::alink::AlinkRole;
use dspi_transport::fake::{
    AlinkSetup, FakeGeneration, FakeTimeBase, FakeTimeBaseUnit, SpdifSetup, TimeBaseFault,
    UsbDirectSetup,
};
use dspi_transport::{FakeDevice, FakeRemote};
use serde_json::{Value, json};

const RATE: u32 = 48_000;
const FD: f64 = 48_000.0;
/// Frames per capture push (10 ms).
const BLOCK: u64 = 480;
/// The microphone's input latency, frames (150 ms).
const MIC_LAG: u64 = 7_200;
/// A local render starts this many frames before its start's T.
const MARGIN: u64 = 480;
/// Half-width of the interpolation kernel, taps.
const HALF: i64 = 24;
/// The noise at the capture, peak of a uniform distribution (-85 dBFS RMS).
const NOISE: f32 = 9.7e-5;
const G1B: FakeGeneration = FakeGeneration::Bofr116 { page_guard: true };
const G2B: FakeGeneration = FakeGeneration::Bofr117;
/// Heavy tests that run at once.
const SLOTS: usize = 4;

static RUNNING: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());

/// One of [`SLOTS`]: the renders are real-time work.
struct Slot;

fn slot() -> Slot {
    let (m, c) = &RUNNING;
    let mut n = m.lock().unwrap_or_else(|p| p.into_inner());
    while *n >= SLOTS {
        n = c.wait(n).unwrap_or_else(|p| p.into_inner());
    }
    *n += 1;
    Slot
}

impl Drop for Slot {
    fn drop(&mut self) {
        let (m, c) = &RUNNING;
        *m.lock().unwrap_or_else(|p| p.into_inner()) -= 1;
        c.notify_one();
    }
}

fn store_dir() -> PathBuf {
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join("wp-ate2e").join(format!(
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
        "wp-ate2e".into(),
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

fn out_delay_ms(core: &DspiCore, serial: &str, output: u8) -> f64 {
    let v = hub_request(
        core,
        "param.get_many",
        json!({"serial": serial, "items": [{"path": "out.delay", "idx": [output]}]}),
    );
    v["values"][0]["value"].as_f64().unwrap()
}

// ── The world ───────────────────────────────────────────────────────────

/// The installation's transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Receivers on one audio-link sender.
    Line,
    /// USB-direct units on one controller.
    Usb,
    /// An S/PDIF source (unit 0, slot 1 carries the time base) and its
    /// followers.
    Spdif,
}

/// One unit of a world.
#[derive(Debug, Clone)]
struct UnitSpec {
    /// The model's `latencyUs` (a delay after the link).
    latency_us: u32,
    /// Its measured outputs and each one's acoustic delay to the
    /// microphone, ms.
    outputs: Vec<(u8, f64)>,
    /// A fan-out's hop error, frames (line only).
    hop_error: i32,
}

fn unit_spec(latency_us: u32, outputs: &[(u8, f64)]) -> UnitSpec {
    UnitSpec {
        latency_us,
        outputs: outputs.to_vec(),
        hop_error: 0,
    }
}

#[derive(Debug, Clone)]
struct Spec {
    kind: Kind,
    generation: FakeGeneration,
    units: Vec<UnitSpec>,
    passes: u8,
    /// The microphone's clock against the units', ppm.
    ppm: f64,
    wander: Option<WanderBursts>,
    seed: u64,
    lease_ms: u32,
}

impl Spec {
    fn new(kind: Kind, generation: FakeGeneration, units: Vec<UnitSpec>) -> Spec {
        Spec {
            kind,
            generation,
            units,
            passes: 3,
            ppm: 31.0,
            wander: None,
            seed: 1,
            lease_ms: 5_000,
        }
    }
}

fn serial(i: usize) -> String {
    format!("E2E-{i}")
}

/// One unit as the room knows it.
struct Unit {
    serial: String,
    remote: FakeRemote,
    /// Its own T minus the domain's (USB: its S origin), samples.
    off: i64,
    /// Where it plays T, frames after T (the engine's terms).
    playout: f64,
    hop_error: i32,
    /// Measured outputs, acoustic delay ms.
    outputs: Vec<(u8, f64)>,
    /// Each measured output's `out.delay`, ms.
    out_delay_ms: Vec<f64>,
    /// Starts already rendered.
    seen: usize,
}

/// What a start sounded like: `(unit, its T, level dBFS, outputs walked)`.
type Heard = (usize, u64, f32, u16);

/// The room, the microphone and its clock: one capture.
struct Room {
    tb: FakeTimeBase,
    /// Domain T of reference device frame 0.
    t0: u64,
    units: Vec<Unit>,
    acoustics: Acoustics,
    eps: f64,
    bursts: Vec<WanderBurst>,
    /// The capture not pushed yet, from `air_base`.
    air: Vec<f32>,
    air_base: u64,
    pushed: u64,
    /// The domain T at the last look.
    t_seen: u64,
    seed: u64,
    heard: Vec<Heard>,
    words: Vec<String>,
    /// Rendering the last look took this long (the next tick waits).
    rendered: bool,
}

impl Room {
    /// The microphone's wander at reference frame `q`, frames.
    fn wander(&self, q: f64) -> f64 {
        if self.bursts.is_empty() {
            return 0.0;
        }
        wander_error_us(&self.bursts, q / FD) * 1e-6 * FD
    }

    /// The capture frame at which the sound at the microphone at reference
    /// frame `p` is heard.
    fn capture_of(&self, p: f64) -> f64 {
        let mut q = p;
        for _ in 0..6 {
            q = p - self.wander(q);
        }
        q * (1.0 + self.eps)
    }

    /// The reference frame heard at capture frame `k`.
    fn heard_at(&self, k: f64) -> f64 {
        let q = k / (1.0 + self.eps);
        q + self.wander(q)
    }

    /// The domain's T now.
    fn t_now(&self) -> u64 {
        self.tb.t_now()
    }

    /// Render every start the fakes recorded since the last look.
    fn listen(&mut self) {
        self.t_seen = self.t_now();
        self.rendered = false;
        for u in 0..self.units.len() {
            let _ = self.units[u].remote.time_base_t();
            let starts = self.units[u].remote.measure_timed_starts();
            for s in starts
                .iter()
                .skip(self.units[u].seen)
                .cloned()
                .collect::<Vec<_>>()
            {
                assert!(s.timed, "only op 9 starts a generator here: {s:?}");
                let t = (i128::from(s.t) - i128::from(self.units[u].off)) as u64;
                self.render(u, t, &s.config);
                self.rendered = true;
                self.heard.push((
                    u,
                    t,
                    s.config.level_db,
                    s.config.channel_mask.count_ones() as u16,
                ));
            }
            self.units[u].seen = starts.len();
        }
    }

    /// Render unit `u`'s generator started at domain T `t` with `c`, and lay
    /// it onto the capture.
    fn render(&mut self, u: usize, t: u64, c: &dspi_proto::packets::SiggenConfig) {
        let unit = &self.units[u];
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
        let n = walk
            .outputs()
            .iter()
            .chain(unit.outputs.iter().map(|(o, _)| o))
            .map(|o| usize::from(*o) + 1)
            .max()
            .unwrap_or(1);
        let mut sim = SimUnit::flat(&unit.serial, n);
        for (o, p) in sim.outputs.iter_mut().enumerate() {
            match unit.outputs.iter().position(|(x, _)| usize::from(*x) == o) {
                Some(i) => {
                    p.delay_ms = unit.outputs[i].1 + unit.out_delay_ms[i];
                    p.gain_db = -6.0;
                }
                // An output nobody measures stays silent.
                None => p.gain_db = -200.0,
            }
        }
        sim.chains = flat_chains(n);
        sim.playout_frames = unit.playout;
        sim.link = UnitLink::Line {
            hops: 0,
            hop_error_frames: unit.hop_error,
        };
        sim.level_db = c.level_db;
        sim.invert_mask = c.invert_mask;
        let d = DomainId {
            time_base: 0,
            id: 0xE2E,
        };
        let mut world = SystemWorld::free_field(vec![sim], d, 0, RATE);
        world.acoustics = self.acoustics.clone();
        world.mic = Microphone::typical();
        world.post_roll_ms = 600;
        let schedule = SystemSchedule::single(d, UnitIndex(0), MARGIN, walk).unwrap();
        let r = render_system(&world, &schedule, &ProtoModel).unwrap();
        let local = &r.capture.samples;
        assert_eq!(r.capture.blocks.first().map_or(0, |b| b.first_frame), 0);

        // Lay it onto the capture through the microphone's clock.
        let p0 = (t - self.t0) as f64 - MARGIN as f64;
        let k_lo = self.capture_of(p0).floor() as u64;
        let k_hi = self.capture_of(p0 + local.len() as f64).ceil() as u64;
        let sound = self.capture_of((t - self.t0) as f64);
        assert!(
            sound as u64 >= self.pushed,
            "unit {u}'s start at T {t} was learnt too late (capture frame {sound}, pushed {})",
            self.pushed
        );
        let from = k_lo.max(self.pushed);
        let need = (k_hi - self.air_base) as usize + 1;
        if self.air.len() < need {
            self.air.resize(need, 0.0);
        }
        let cos_tab: Vec<(f64, f64)> = (-HALF + 1..=HALF)
            .map(|j| {
                let a = std::f64::consts::PI * j as f64 / HALF as f64;
                (a.cos(), a.sin())
            })
            .collect();
        for k in from..=k_hi {
            let x = self.heard_at(k as f64) - p0;
            let i0 = x.floor();
            let f = x - i0;
            let i0 = i0 as i64;
            if i0 - HALF < 0 || i0 + HALF >= local.len() as i64 {
                continue;
            }
            let v = if f < 1e-9 {
                f64::from(local[i0 as usize])
            } else {
                let pi = std::f64::consts::PI;
                let s = (pi * f).sin();
                let (ca, sa) = ((pi * f / HALF as f64).cos(), (pi * f / HALF as f64).sin());
                let mut acc = 0.0;
                for (jj, j) in (-HALF + 1..=HALF).enumerate() {
                    let dx = f - j as f64;
                    let sign = if j % 2 == 0 { 1.0 } else { -1.0 };
                    let sinc = s * sign / (pi * dx);
                    // cos(pi (f - j) / HALF), by the angle difference.
                    let (cb, sb) = cos_tab[jj];
                    let win = 0.5 * (1.0 + (ca * cb + sa * sb));
                    acc += f64::from(local[(i0 + j) as usize]) * sinc * win;
                }
                acc
            };
            self.air[(k - self.air_base) as usize] += v as f32;
        }
    }

    /// Push the capture up to what was known at the last look.
    fn capture(&mut self, core: &DspiCore) {
        let have = (self.capture_of((self.t_seen - self.t0) as f64) as u64).saturating_sub(MIC_LAG);
        while self.pushed + BLOCK <= have {
            let base = (self.pushed - self.air_base) as usize;
            let block: Vec<f32> = (0..BLOCK as usize)
                .map(|i| {
                    self.seed = self
                        .seed
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    let r = ((self.seed >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0;
                    self.air.get(base + i).copied().unwrap_or(0.0) + r * NOISE
                })
                .collect();
            core.measure_system_capture_push(self.pushed, block, RATE, false)
                .unwrap();
            self.pushed += BLOCK;
        }
        // Drop what is pushed.
        let done = ((self.pushed - self.air_base) as usize).min(self.air.len());
        if done > 48_000 {
            self.air.drain(..done);
            self.air_base += done as u64;
        }
    }

    /// One step of the session: look, push, tick (unless the look took a
    /// render: then look again first, so the tick's time pin is current).
    fn step(&mut self, core: &DspiCore) -> Option<MeasureSystemState> {
        self.listen();
        self.capture(core);
        if self.rendered {
            return None;
        }
        Some(core.measure_system_tick())
    }

    /// Drive the core's session to its end.
    fn drive(&mut self, core: &DspiCore, what: &str) -> MeasureSystemState {
        self.drive_until(core, what, |_, _| false)
    }

    /// Drive it until it ends or `stop(state, room)` says so.
    fn drive_until(
        &mut self,
        core: &DspiCore,
        what: &str,
        mut stop: impl FnMut(&MeasureSystemState, &Room) -> bool,
    ) -> MeasureSystemState {
        let start = Instant::now();
        loop {
            let Some(st) = self.step(core) else {
                continue;
            };
            if !st.words.is_empty() && !self.words.contains(&st.words) {
                eprintln!("{what}: {}", st.words);
                self.words.push(st.words.clone());
            }
            if !st.active || stop(&st, self) {
                return st;
            }
            assert!(
                start.elapsed() < Duration::from_secs(400),
                "{what} did not end: {st:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Re-read every measured output's `out.delay`.
    fn read_delays(&mut self, core: &DspiCore) {
        for u in &mut self.units {
            u.out_delay_ms = u
                .outputs
                .iter()
                .map(|(o, _)| out_delay_ms(core, &u.serial, *o))
                .collect();
        }
    }

    /// The truth of what the session reports as an output's arrival (the
    /// acoustic part, a fan-out's hop error included: the engine's terms do
    /// not know it), µs.
    fn truth_us(&self, serial: &str, output: u8) -> f64 {
        let u = self.units.iter().find(|u| u.serial == serial).unwrap();
        let (_, ms) = u.outputs.iter().find(|(o, _)| *o == output).unwrap();
        ms * 1000.0 + f64::from(u.hop_error) * 1e6 / FD
    }
}

/// The hub, its units, one core per unit, the checked setup and the room.
struct Rig {
    hub: Hub,
    cores: Vec<Arc<DspiCore>>,
    setup: MeasureSystemSetup,
    room: Room,
    /// Each unit's measured outputs' `out.delay` before anything, ms.
    before: Vec<Vec<f64>>,
}

impl Rig {
    /// The core on the hub session (unit 0's, Admin).
    fn core(&self) -> &DspiCore {
        &self.cores[0]
    }

    /// Each unit judged by its own core, now.
    fn checks(&self) -> Vec<MeasureSystemUnitCheck> {
        self.cores
            .iter()
            .zip(&self.setup.units)
            .map(|(c, u)| {
                let k = c
                    .measure_system_unit_check(self.setup.clone(), u.clone())
                    .unwrap();
                assert!(k.preflight.clear, "{}: {:?}", k.serial, k.preflight);
                k
            })
            .collect()
    }
}

fn rig(spec: &Spec) -> Rig {
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(store_dir())
        }),
        initial_scan: false,
        ..HubConfig::in_process("wp-ate2e hub")
    })
    .unwrap();
    hub.handle().set_system_measure_settle(Duration::ZERO);
    hub.handle()
        .set_system_poll_interval(Duration::from_secs(3600));

    let tb = match spec.kind {
        Kind::Line => FakeTimeBase::line(0x2E),
        Kind::Usb => FakeTimeBase::usb_direct(),
        Kind::Spdif => FakeTimeBase::spdif(0x2E2E),
    };
    let mut remotes = Vec::new();
    for i in 0..spec.units.len() {
        let unit = match (spec.kind, i) {
            (Kind::Line, _) => FakeTimeBaseUnit::Line(AlinkSetup::default()),
            (Kind::Usb, _) => FakeTimeBaseUnit::UsbDirect(UsbDirectSetup {
                origin_wraps: 1 + 2 * i as u32,
                ..UsbDirectSetup::default()
            }),
            (Kind::Spdif, 0) => FakeTimeBaseUnit::Spdif(SpdifSetup {
                tx_mask: 0b10,
                ..SpdifSetup::source()
            }),
            (Kind::Spdif, _) => FakeTimeBaseUnit::Spdif(SpdifSetup::follower(192)),
        };
        let d =
            FakeDevice::demo_on_time_base_as(&tb, unit, spec.generation).with_serial(&serial(i));
        remotes.push(d.remote());
        hub.attach(Box::new(d), 0).unwrap();
    }
    if spec.kind == Kind::Line {
        hub.attach(
            Box::new(
                FakeDevice::demo_alink_as(AlinkSetup {
                    role: AlinkRole::Sender,
                    ..AlinkSetup::default()
                })
                .with_serial("E2E-TX"),
            ),
            0,
        )
        .unwrap();
    }
    let units: Vec<Value> = spec
        .units
        .iter()
        .enumerate()
        .map(|(i, u)| {
            let speakers: Vec<Value> = u
                .outputs
                .iter()
                .map(|(o, _)| json!({"output": o, "distanceM": 2.0}))
                .collect();
            let mut v = json!({"id": format!("U{i}"), "serial": serial(i), "room": "main",
                "latencyUs": u.latency_us, "speakers": speakers});
            match (spec.kind, i) {
                (Kind::Line, _) => v["upstream"] = json!("tx"),
                (Kind::Usb, _) => {
                    v["transport"] = json!("usb_direct");
                    v["usbMode"] = json!("phase_lock");
                }
                (Kind::Spdif, 0) => {
                    v["transport"] = json!("spdif");
                    v["spdifSource"] = json!(true);
                    v["spdifTxMask"] = json!(2);
                }
                (Kind::Spdif, _) => {
                    v["transport"] = json!("spdif");
                    v["upstream"] = json!("U0");
                }
            }
            v
        })
        .collect();
    let mut model = json!({"rooms": [{"id": "main", "name": "Main"}], "units": units});
    if spec.kind == Kind::Line {
        model["senders"] = json!([{"id": "tx", "serial": "E2E-TX"}]);
    }
    hub.handle()
        .system_set_model(serde_json::from_value(model).unwrap())
        .unwrap();
    let first = hub.handle().system_align(true);
    assert!(
        first.writes.iter().all(|w| w.result != "failed"),
        "{:?}",
        first.writes
    );

    let cores: Vec<Arc<DspiCore>> = (0..spec.units.len())
        .map(|i| connect(&hub, &serial(i)))
        .collect();
    assert_eq!(cores[0].link_hub_role(), "admin");
    for c in &cores {
        c.set_user_volume(-20.0).unwrap();
    }
    let setup = MeasureSystemSetup {
        units: spec
            .units
            .iter()
            .enumerate()
            .map(|(i, u)| MeasureSystemUnit {
                serial: serial(i),
                outputs: u
                    .outputs
                    .iter()
                    .map(|(o, _)| MeasureOutput {
                        output: *o,
                        role: MeasureRole::Full,
                        speaker: format!("{}/{o}", serial(i)),
                        ..MeasureOutput::default()
                    })
                    .collect(),
            })
            .collect(),
        passes: spec.passes,
        sweep_ms: 1_000,
        gap_ms: 300,
        capture_rate_hz: RATE,
        lease_ms: spec.lease_ms,
        ..MeasureSystemSetup::default()
    };

    // The room: what the engine says each unit's play-out is.
    let status = hub.handle().system_status();
    let mut room_units = Vec::new();
    for (i, (u, remote)) in spec.units.iter().zip(remotes).enumerate() {
        let s = serial(i);
        let p = status
            .alignment
            .units
            .iter()
            .find(|x| x.serial == s)
            .unwrap();
        // C2: a source plays T as it emits it (its lag is in its output
        // delays); a follower its D after T arrives.
        let source = p.spdif.as_ref().is_some_and(|x| x.mode == "source");
        let playout = if source {
            p.latency_frames as f64
        } else {
            (i64::from(p.path_delay_frames) + p.effective + p.latency_frames) as f64
        };
        // The unit's own numbering against the domain's, read at one T.
        let off = (0..100_000)
            .find_map(|_| {
                let a = tb.t_now();
                let (_, own) = remote.time_base_t().unwrap();
                (tb.t_now() == a).then_some(own as i64 - a as i64)
            })
            .expect("one T for the unit's read");
        room_units.push(Unit {
            serial: s.clone(),
            seen: remote.measure_timed_starts().len(),
            remote,
            off,
            playout,
            hop_error: u.hop_error,
            outputs: u.outputs.clone(),
            out_delay_ms: u
                .outputs
                .iter()
                .map(|(o, _)| out_delay_ms(&cores[0], &s, *o))
                .collect(),
        });
    }
    eprintln!(
        "{:?}: play-outs {:?}, offsets {:?}, issues {:?}",
        spec.kind,
        room_units.iter().map(|u| u.playout).collect::<Vec<_>>(),
        room_units.iter().map(|u| u.off).collect::<Vec<_>>(),
        status.alignment.issues
    );
    let before = room_units.iter().map(|u| u.out_delay_ms.clone()).collect();
    let t0 = tb.t_now();
    let room = Room {
        tb,
        t0,
        units: room_units,
        acoustics: World::three_way_room(spec.seed).acoustics,
        eps: spec.ppm * 1e-6,
        bursts: spec.wander.map(|w| w.bursts(900.0)).unwrap_or_default(),
        air: Vec::new(),
        air_base: 0,
        pushed: 0,
        t_seen: t0,
        seed: spec.seed ^ 0xA7E2,
        heard: Vec::new(),
        words: Vec::new(),
        rendered: false,
    };
    Rig {
        hub,
        cores,
        setup,
        room,
        before,
    }
}

/// Measure: the session from start to its result.
fn measure(rig: &mut Rig) -> MeasureSystemState {
    let checks = rig.checks();
    let st = rig
        .core()
        .measure_system_start(rig.setup.clone(), checks)
        .unwrap();
    assert!(st.active && st.wants_capture, "{st:?}");
    assert_eq!(rig.hub.handle().system_measure_sessions(), 1);
    let core = rig.cores[0].clone();
    let st = rig.room.drive(&core, "measure");
    assert_eq!(rig.hub.handle().system_measure_sessions(), 0, "closed");
    st
}

/// Every arrival within 2 µs of the world's and within three of its own
/// uncertainties, offered; the latest at 0.
fn check_arrivals(rig: &Rig) {
    let result = rig.core().measure_system_result().unwrap();
    eprintln!("arrivals {:?}", result.arrivals);
    eprintln!("reasons {:?}", result.reasons);
    assert!(result.timing_reliable, "{:?}", result.reasons);
    let truths: Vec<f64> = result
        .arrivals
        .iter()
        .map(|a| rig.room.truth_us(&a.serial, a.output))
        .collect();
    let latest = truths.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert_eq!(
        result.arrivals.len(),
        rig.setup
            .units
            .iter()
            .map(|u| u.outputs.len())
            .sum::<usize>()
    );
    for (a, t) in result.arrivals.iter().zip(&truths) {
        let err = a.arrival_us - (t - latest);
        let u = a.uncertainty_us.unwrap_or(0.0);
        eprintln!(
            "{} out {}: {:.3} µs, truth {:.3}, error {err:.3} µs, u {u:.3}",
            a.serial,
            a.output,
            a.arrival_us,
            t - latest
        );
        assert!(a.offered, "{a:?}");
        assert!(err.abs() <= 2.0, "{a:?}: {err} µs off");
        assert!(err.abs() <= 3.0 * u.max(0.25), "{a:?}: {err} µs, u {u}");
    }
    assert_eq!(result.documents.len(), rig.setup.units.len());
}

/// The summed check (fresh checks), driven to its end.
fn sum_check(rig: &mut Rig) -> dspi_core::MeasureSystemProposal {
    let checks = rig.checks();
    rig.core().measure_system_sum_check(checks).unwrap();
    let core = rig.cores[0].clone();
    let st = rig.room.drive(&core, "sum check");
    assert_eq!(st.phase, MeasureSystemPhase::Done, "{st:?}");
    let p = rig.core().measure_system_proposal().unwrap();
    eprintln!("transitions {:?}", p.transitions);
    p
}

/// Apply (Admin), then the verify's summed runs with the delays the units
/// now hold in the room.
fn apply_and_verify(rig: &mut Rig, p: &dspi_core::MeasureSystemProposal) -> Option<bool> {
    let applied = rig.core().measure_system_apply(p.id).unwrap();
    eprintln!("applied {applied:?}");
    assert_eq!(applied.stage, MeasureSystemApplyStage::Applied);
    assert!(applied.write_failures.is_empty(), "{applied:?}");
    assert!(applied.revision_after > applied.revision_before);
    let core = rig.cores[0].clone();
    rig.room.read_delays(&core);
    let checks = rig.checks();
    rig.core().measure_system_verify(checks).unwrap();
    let st = rig.room.drive(&core, "verify");
    assert_eq!(st.phase, MeasureSystemPhase::Done, "{st:?}");
    let v = rig.core().measure_system_applied();
    assert_eq!(v.stage, MeasureSystemApplyStage::Verified, "{v:?}");
    v.verified
}

/// Revert: the model and every measured output's delay as filed.
fn revert(rig: &mut Rig) {
    let r = rig.core().measure_system_revert().unwrap();
    assert_eq!(r.stage, MeasureSystemApplyStage::Reverted, "{r:?}");
    let (model, _) = rig.hub.handle().system_model();
    assert!(
        model
            .units
            .iter()
            .all(|u| u.speakers.iter().all(|s| s.measured_arrival_us.is_none()))
    );
    let core = rig.cores[0].clone();
    rig.room.read_delays(&core);
    for (u, before) in rig.room.units.iter().zip(&rig.before) {
        for (now, was) in u.out_delay_ms.iter().zip(before) {
            assert!(
                (now - was).abs() < 0.0015,
                "{}: {now} ms where {was} ms was filed",
                u.serial
            );
        }
    }
}

// ── Worlds measured ─────────────────────────────────────────────────────

/// Two receivers on the line, a steady microphone 31 ppm off in a room:
/// measured within 2 µs, the summed check passes, applied through the hub's
/// model, verified, reverted.
#[test]
fn wp_ate2e_line_two_units_measured_applied_verified_and_reverted() {
    let _slot = slot();
    let spec = Spec::new(
        Kind::Line,
        G1B,
        vec![unit_spec(0, &[(0, 3.0)]), unit_spec(1_000, &[(0, 5.37)])],
    );
    let mut rig = rig(&spec);
    let st = measure(&mut rig);
    assert_eq!(st.phase, MeasureSystemPhase::Done, "{st:?}");
    check_arrivals(&rig);
    let p = rig.core().measure_system_proposal().unwrap();
    assert!(p.sum_check_needed && !p.offered, "{p:?}");
    let p = sum_check(&mut rig);
    assert!(p.transitions.iter().all(|t| t.checked && t.passed), "{p:?}");
    assert!(p.offered && p.may_apply && !p.withdrawn, "{p:?}");
    // The model carries the measured arrivals (relative to the latest).
    let verified = apply_and_verify(&mut rig, &p);
    let (model, _) = rig.hub.handle().system_model();
    let m = model.units[0].speakers[0].measured_arrival_us.unwrap();
    assert!((m + 2_370.0).abs() < 2.0, "{m}");
    assert_eq!(verified, Some(true));
    revert(&mut rig);
}

/// Two receivers on the line as G2b firmware: measured, the summed check
/// passes, applied; then unit 1's delay is changed by 5 samples (104 µs)
/// behind the session's back: the verify's summed runs fail, the first Keep
/// is refused with the reason, and Revert restores the filed delays.
#[test]
fn wp_ate2e_a_wrong_delay_after_the_apply_fails_the_verify() {
    let _slot = slot();
    let spec = Spec {
        seed: 6,
        ppm: -18.0,
        ..Spec::new(
            Kind::Line,
            G2B,
            vec![unit_spec(500, &[(0, 4.1)]), unit_spec(0, &[(0, 2.6)])],
        )
    };
    let mut rig = rig(&spec);
    let st = measure(&mut rig);
    assert_eq!(st.phase, MeasureSystemPhase::Done, "{st:?}");
    check_arrivals(&rig);
    let p = sum_check(&mut rig);
    assert!(p.transitions.iter().all(|t| t.checked && t.passed), "{p:?}");
    assert!(p.offered, "{p:?}");
    let applied = rig.core().measure_system_apply(p.id).unwrap();
    assert_eq!(
        applied.stage,
        MeasureSystemApplyStage::Applied,
        "{applied:?}"
    );
    let s1 = serial(1);
    let now = out_delay_ms(rig.core(), &s1, 0);
    let v = hub_request(
        rig.core(),
        "param.set",
        json!({"serial": s1, "path": "out.delay", "idx": [0], "value": now + 5.0 / 48.0}),
    );
    assert_eq!(v["t"], "ok", "{v}");
    let core = rig.cores[0].clone();
    rig.room.read_delays(&core);
    let checks = rig.checks();
    rig.core().measure_system_verify(checks).unwrap();
    let st = rig.room.drive(&core, "verify");
    assert_eq!(st.phase, MeasureSystemPhase::Done, "{st:?}");
    let v = rig.core().measure_system_applied();
    eprintln!("verify {v:?}");
    assert_eq!(
        v.verified,
        Some(false),
        "a wrong delay passed the verify: {v:?}"
    );
    // Keep after a failed verify: refused once, with the reason.
    let k = rig.core().measure_system_keep();
    assert!(
        k.is_err(),
        "the first Keep after a failed verify kept: {k:?}"
    );
    let a = rig.core().measure_system_applied();
    assert_ne!(a.stage, MeasureSystemApplyStage::Kept, "{a:?}");
    eprintln!("keep refused: {k:?}; {}", a.message);
    revert(&mut rig);
}

/// Three USB-direct units on one controller as G2b firmware, each with its
/// own S origin (whole 2048-frame wraps apart) and its own latency after
/// the link: every arrival within 2 µs of the world's, although the
/// engine's output delays (the USB streams' start offsets, which the
/// units' anchors stagger by the hub's write time) put the units tens of
/// milliseconds apart on the timeline. The summed check then either passes
/// or, when those offsets push its band below its floor, cannot judge and
/// withdraws every delay (fail closed; WP ATE2E's finding for ATCORE).
#[test]
fn wp_ate2e_usb_direct_three_units() {
    let _slot = slot();
    let spec = Spec {
        seed: 2,
        ppm: -18.0,
        ..Spec::new(
            Kind::Usb,
            G2B,
            vec![
                unit_spec(0, &[(0, 4.1)]),
                unit_spec(500, &[(0, 2.6)]),
                unit_spec(1_500, &[(0, 3.35)]),
            ],
        )
    };
    let mut rig = rig(&spec);
    let st = measure(&mut rig);
    assert_eq!(st.phase, MeasureSystemPhase::Done, "{st:?}");
    check_arrivals(&rig);
    let result = rig.core().measure_system_result().unwrap();
    let on_timeline: Vec<f64> = result.arrivals.iter().map(|a| a.measured_us).collect();
    let spread_ms = (on_timeline
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max)
        - on_timeline.iter().copied().fold(f64::INFINITY, f64::min))
        / 1000.0;
    let p = sum_check(&mut rig);
    assert_eq!(p.transitions.len(), 3, "{p:?}");
    if p.transitions.iter().all(|t| t.checked && t.passed) {
        assert!(p.offered && !p.withdrawn, "{p:?}");
    } else {
        eprintln!(
            "FINDING: the units are {spread_ms:.1} ms apart on the timeline; the summed check \
             cannot judge: {:?}",
            p.transitions.iter().map(|t| &t.message).collect::<Vec<_>>()
        );
        for t in p.transitions.iter().filter(|t| !t.passed) {
            assert!(t.rms_db.is_none(), "a judged summed run failed: {t:?}");
            assert!(t.message.contains("too few trusted points"), "{t:?}");
        }
        assert!(spread_ms > 5.0, "{spread_ms} ms");
        assert!(p.withdrawn && !p.offered && p.arrivals.is_empty(), "{p:?}");
    }
}

/// An S/PDIF source and two followers with different D: measured within
/// 2 µs; then follower 2's speaker is moved 150 µs farther (the measured
/// delay is now wrong by that): the summed check fails over its transitions
/// and every cross-unit delay is withdrawn.
#[test]
fn wp_ate2e_spdif_source_and_two_followers_and_a_wrong_delay_fails_the_sum_check() {
    let _slot = slot();
    let spec = Spec {
        seed: 3,
        ppm: 12.5,
        ..Spec::new(
            Kind::Spdif,
            G1B,
            vec![
                unit_spec(0, &[(0, 3.0)]),
                unit_spec(1_000, &[(0, 5.0)]),
                unit_spec(250, &[(0, 4.4)]),
            ],
        )
    };
    let mut rig = rig(&spec);
    let st = measure(&mut rig);
    assert_eq!(st.phase, MeasureSystemPhase::Done, "{st:?}");
    check_arrivals(&rig);
    rig.room.units[2].outputs[0].1 += 0.150;
    let p = sum_check(&mut rig);
    let bad: Vec<_> = p
        .transitions
        .iter()
        .filter(|t| t.a_serial == serial(2) || t.b_serial == serial(2))
        .collect();
    assert!(!bad.is_empty(), "{p:?}");
    assert!(bad.iter().all(|t| t.checked && !t.passed), "{p:?}");
    assert!(p.withdrawn && !p.offered, "{p:?}");
}

/// Four receivers on the line as G2b firmware, one of them behind a fan-out
/// that plays it one frame later than its `hops_offset` assumes: every
/// arrival within 2 µs of the world's, the hop error measured as the
/// 20.8 µs it is.
#[test]
fn wp_ate2e_four_units_on_the_line_one_behind_a_fan_out() {
    let _slot = slot();
    let mut units = vec![
        unit_spec(0, &[(0, 3.0)]),
        unit_spec(300, &[(0, 3.6)]),
        unit_spec(0, &[(0, 4.2)]),
        unit_spec(700, &[(0, 3.3)]),
    ];
    units[2].hop_error = 1;
    let spec = Spec {
        seed: 4,
        ..Spec::new(Kind::Line, G2B, units)
    };
    let mut rig = rig(&spec);
    let st = measure(&mut rig);
    assert_eq!(st.phase, MeasureSystemPhase::Done, "{st:?}");
    check_arrivals(&rig);
}

/// A group of 17 outputs (more than one run's 16): two runs chained through
/// the anchor (unit 0's output 0), the microphone open across both (one
/// capture), every output within 2 µs of the world's.
#[test]
fn wp_ate2e_seventeen_outputs_in_two_runs_chained_through_the_anchor() {
    let _slot = slot();
    let mut units = Vec::new();
    for i in 0..9u8 {
        let outs: Vec<(u8, f64)> = (0..if i == 8 { 1 } else { 2 })
            .map(|o| (o, 2.5 + 0.21 * f64::from(i) + 0.37 * f64::from(o)))
            .collect();
        units.push(unit_spec(100 * u32::from(i), &outs));
    }
    let spec = Spec {
        seed: 5,
        ..Spec::new(Kind::Line, G1B, units)
    };
    let mut rig = rig(&spec);
    let t = Instant::now();
    let st = measure(&mut rig);
    eprintln!("17 outputs measured in {:?}", t.elapsed());
    assert_eq!(st.phase, MeasureSystemPhase::Done, "{st:?}");
    assert_eq!(st.runs, 2, "{st:?}");
    let result = rig.core().measure_system_result().unwrap();
    assert_eq!(result.runs, 2);
    check_arrivals(&rig);
    // One capture: the frame count ran on through both runs.
    assert_eq!(st.frames, rig.room.pushed);
}

// ── Faults ──────────────────────────────────────────────────────────────

/// Measure two units and play `fault` on unit `victim` as pass 2 of the
/// timed run starts: the run ends, the unit is named, nothing is proposed.
fn fault_in_pass_two(spec: &Spec, victim: usize, fault: TimeBaseFault, named: Option<usize>) {
    let _slot = slot();
    let mut rig = rig(spec);
    let checks = rig.checks();
    rig.core()
        .measure_system_start(rig.setup.clone(), checks)
        .unwrap();
    let core = rig.cores[0].clone();
    let n = rig.setup.units.len();
    let mut run_from: Option<usize> = None;
    let mut played = false;
    let remote = rig.room.units[victim].remote.clone();
    let st = rig.room.drive_until(&core, "fault", |st, room| {
        if st.phase == MeasureSystemPhase::Run && st.words.is_empty() && run_from.is_none() {
            run_from = Some(room.heard.len());
        }
        if let Some(r) = run_from
            && !played
            && room.heard.len() > r + n
        {
            assert!(remote.time_base_fault(fault), "{fault:?}");
            played = true;
        }
        false
    });
    assert!(played, "the run ended before pass 2: {st:?}");
    eprintln!("{fault:?}: {:?}", st.issue);
    assert_eq!(st.phase, MeasureSystemPhase::Aborted, "{st:?}");
    let issue = st.issue.clone().unwrap();
    match named {
        Some(u) => assert_eq!(issue.serial, serial(u), "{issue:?}"),
        None => assert!(
            (0..n).any(|u| issue.serial == serial(u)),
            "no unit named: {issue:?}"
        ),
    }
    assert!(issue.message.contains(&issue.serial), "{issue:?}");
    assert!(rig.core().measure_system_result().is_err());
    assert!(rig.core().measure_system_proposal().is_err());
    let start = Instant::now();
    while rig.hub.handle().system_measure_sessions() != 0 {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the session stayed"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // Nothing starts afterwards.
    let heard = rig.room.heard.len();
    std::thread::sleep(Duration::from_secs(2));
    rig.room.listen();
    assert_eq!(
        rig.room.heard.len(),
        heard,
        "a unit started after the abort"
    );
}

fn two_line(g: FakeGeneration) -> Spec {
    Spec::new(
        Kind::Line,
        g,
        vec![unit_spec(0, &[(0, 3.0)]), unit_spec(0, &[(0, 4.0)])],
    )
}

fn two_usb(g: FakeGeneration) -> Spec {
    Spec::new(
        Kind::Usb,
        g,
        vec![unit_spec(0, &[(0, 3.0)]), unit_spec(0, &[(0, 4.0)])],
    )
}

fn two_spdif(g: FakeGeneration) -> Spec {
    Spec::new(
        Kind::Spdif,
        g,
        vec![unit_spec(0, &[(0, 3.0)]), unit_spec(500, &[(0, 4.0)])],
    )
}

#[test]
fn wp_ate2e_a_holdover_in_pass_two_ends_the_run_naming_the_unit() {
    fault_in_pass_two(&two_line(G1B), 1, TimeBaseFault::Holdover, Some(1));
}

#[test]
fn wp_ate2e_a_reanchor_in_pass_two_ends_the_run_naming_the_unit() {
    fault_in_pass_two(&two_usb(G2B), 0, TimeBaseFault::Reanchor, Some(0));
}

#[test]
fn wp_ate2e_a_reported_correction_in_pass_two_ends_the_run_naming_the_unit() {
    fault_in_pass_two(
        &two_line(G2B),
        1,
        TimeBaseFault::Correction {
            samples: 2,
            event: true,
        },
        Some(1),
    );
}

/// A whole-sample correction the unit does not report (no event, no
/// counter): only `t - output_frames` shows it (G2's read).
#[test]
fn wp_ate2e_a_silent_correction_in_pass_two_is_caught_by_the_timeline_read() {
    fault_in_pass_two(
        &two_usb(G1B),
        1,
        TimeBaseFault::Correction {
            samples: 3,
            event: false,
        },
        Some(1),
    );
}

/// The S/PDIF source restarts (a new epoch): every follower re-anchors.
#[test]
fn wp_ate2e_an_spdif_epoch_change_in_pass_two_ends_the_run_naming_a_unit() {
    fault_in_pass_two(&two_spdif(G2B), 0, TimeBaseFault::Epoch, None);
}

#[test]
fn wp_ate2e_lock_error_in_pass_two_ends_the_run_naming_the_unit() {
    fault_in_pass_two(&two_spdif(G1B), 1, TimeBaseFault::LockError(true), Some(1));
}

/// The hub dies in the middle of the timed run: the console's session
/// ends, and each unit plays at most one more segment (the one START the
/// hub keeps pending per unit, R6a).
#[test]
fn wp_ate2e_a_hub_that_dies_mid_run_leaves_at_most_one_segment_per_unit() {
    let _slot = slot();
    let spec = two_line(G1B);
    let mut rig = rig(&spec);
    let checks = rig.checks();
    rig.core()
        .measure_system_start(rig.setup.clone(), checks)
        .unwrap();
    let core = rig.cores[0].clone();
    let mut run_from: Option<usize> = None;
    let st = rig.room.drive_until(&core, "hub dies", |st, room| {
        if st.phase == MeasureSystemPhase::Run && st.words.is_empty() && run_from.is_none() {
            run_from = Some(room.heard.len());
        }
        run_from.is_some_and(|r| room.heard.len() > r)
    });
    assert!(st.active, "{st:?}");
    let at_stop: Vec<usize> = rig
        .room
        .units
        .iter()
        .map(|u| u.remote.measure_timed_starts().len())
        .collect();
    rig.hub.stop();
    // Long enough for the whole run (3 passes of 1.3 s per unit).
    let start = Instant::now();
    let mut ended = None;
    while start.elapsed() < Duration::from_secs(10) {
        if ended.is_none() {
            let st = core.measure_system_tick();
            if !st.active {
                ended = Some(st);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let st = ended.expect("the session noticed the hub's death");
    eprintln!("hub died: {:?}", st.issue);
    assert_eq!(st.phase, MeasureSystemPhase::Aborted, "{st:?}");
    assert!(core.measure_system_proposal().is_err());
    for (u, before) in rig.room.units.iter().zip(at_stop) {
        let _ = u.remote.time_base_t();
        let after = u.remote.measure_timed_starts().len();
        assert!(
            after <= before + 1,
            "{} played {} more segments",
            u.serial,
            after - before
        );
    }
}

// ── The microphone ──────────────────────────────────────────────────────

/// P5SPIKE's microphone (55-100 ppm bursts of 0.25-1.5 s within ±50 µs):
/// the run is "timing unreliable" and nothing is offered.
#[test]
fn wp_ate2e_a_p5spike_microphone_is_timing_unreliable_and_nothing_is_offered() {
    let _slot = slot();
    let spec = Spec {
        wander: Some(WanderBursts::p5spike(7)),
        seed: 7,
        ..two_line(G2B)
    };
    let mut rig = rig(&spec);
    let st = measure(&mut rig);
    assert_eq!(st.phase, MeasureSystemPhase::Done, "{st:?}");
    assert_eq!(st.timing_reliable, Some(false), "{st:?}");
    let result = rig.core().measure_system_result().unwrap();
    eprintln!("reasons {:?}", result.reasons);
    eprintln!("arrivals {:?}", result.arrivals);
    assert!(!result.timing_reliable);
    assert!(
        result.arrivals.iter().all(|a| !a.offered),
        "{:?}",
        result.arrivals
    );
    assert!(
        result
            .arrivals
            .iter()
            .any(|a| a.reason.starts_with("timing unreliable")),
        "{:?}",
        result.arrivals
    );
    let p = rig.core().measure_system_proposal().unwrap();
    assert!(p.arrivals.is_empty() && !p.offered, "{p:?}");
    assert!(!p.withheld.is_empty(), "{p:?}");
    let notice = p.notice.unwrap();
    assert_eq!(notice.key, "measure.system.timing_unreliable", "{notice:?}");
    assert!(
        notice.message.starts_with("timing unreliable"),
        "{notice:?}"
    );
}
