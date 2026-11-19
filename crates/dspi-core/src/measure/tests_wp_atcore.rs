//! WP ATCORE: the client's system measurement session against a scripted
//! hub stub (the real hub joins in ATE2E). The stub plays the hub's
//! `system.measure.*` family over units on one time base: it keeps each
//! unit's staged config, places every START on the domain counter, renders
//! what the units play through P5SIM's flat world (each output at its
//! acoustic delay plus its unit's play-out and output delay) into the
//! microphone's future capture, and answers `status` with every unit's
//! `EXT_SIGGEN_TIMELINE` and `EXT_TIME_BASE` entry. Faults are injected per
//! test. The session is driven with a simulated clock (100 ms per block).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use proptest::prelude::*;
use serde_json::{Value, json};

use dspi_measure::sim::{Burst, ProtoModel, Stimulus, World, flat_chains, render};
use dspi_measure::stimulus::{LogSweep, SweepConfig, WalkSchedule, peak_linear};
use dspi_measure::system::chain::ChainedOutput;
use dspi_measure::system::messages::{
    self as msg, AbortReply, AbortUnit, AbortedEvent, OpenReply, OpenUnit, RunReply,
    RunSegmentReply, StatusReply, UnitEvent, UnitStatus,
};
use dspi_measure::system::{GateFinding, UnitIndex, UnitOutput};
use dspi_proto::ext::timebase::{
    ContractState, SiggenTimeline, TimeBaseEntry, TimeBaseReason, TimeBaseRole, TimelineResult,
    TimelineState,
};
use dspi_transport::clock::Instant;

use super::system::plan::{self, SetupPlan};
use super::system::proposal::{self, chained_pair, may_propose};
use super::system::session::{SessionOutput, SysSession, analysis_options, config_bytes};
use super::system::{Hub, UnitContext};
use super::*;

const RATE: u32 = 48_000;
const BLOCK: usize = 4_800;
const T_BASE: u64 = 1 << 32;
const DOMAIN: u32 = 0x0007_0000;
/// A K340's arm lead, ms.
const ARM_LEAD_MS: u16 = 150;

// ── The stub ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Seg {
    t: u64,
    total: u64,
    rid: u32,
}

#[derive(Debug, Clone)]
struct StubUnit {
    serial: String,
    outputs: u8,
    /// The engine's play-out of the unit (`effective`), frames.
    effective: i64,
    /// Its latency after the link, frames.
    latency: i64,
    /// Each output's acoustic delay to the microphone, ms.
    acoustic_ms: Vec<f64>,
    /// Each output's `out.delay`, ms.
    out_delay_ms: Vec<f32>,
    staged: Option<dspi_proto::packets::SiggenConfig>,
    segs: Vec<Seg>,
    run_id: u32,
    disturbances: u16,
}

#[derive(Default)]
struct Faults {
    /// `(unit, capture frame)`: its disturbance counter moves.
    disturb: Option<(usize, u64)>,
    /// `(unit, frame)`: a clip event (0xE0).
    clip: Option<(usize, u64)>,
    /// `(unit, frame)`: an output at full scale.
    peak: Option<(usize, u64)>,
    /// `(unit, frame)`: the hub aborts on its own.
    hub_abort: Option<(usize, u64)>,
    /// A unit whose timeline reports every start one sample late.
    proof_late: Option<usize>,
    /// Summed runs: this unit plays later by this much (the walked run
    /// mis-measured), ms.
    sum_detune: Option<(usize, f64)>,
    /// Walked runs: this unit's segments wander by up to this, us.
    wander_us: Option<(usize, f64)>,
}

struct Stub {
    units: Vec<StubUnit>,
    caps: Vec<String>,
    role: String,
    /// Capture frames pushed (the domain counts the same frames).
    frames: u64,
    future: Vec<f32>,
    requests: Vec<String>,
    aborted: Vec<AbortedEvent>,
    faults: Faults,
    model: Value,
    revision: u64,
    seed: u64,
    session_open: bool,
    /// Samples the hub adds to the next run's t0 (settling).
    settle: u64,
    settling_seen: bool,
}

impl Stub {
    fn new(units: Vec<StubUnit>) -> Stub {
        let model = json!({
            "units": units.iter().map(|u| json!({
                "id": u.serial, "serial": u.serial, "room": "R",
                "speakers": (0..u.outputs).map(|o| json!({"output": o, "distanceM": 2.0})).collect::<Vec<_>>()
            })).collect::<Vec<_>>()
        });
        Stub {
            units,
            caps: vec![msg::CAP.to_owned()],
            role: "admin".into(),
            frames: 0,
            future: Vec::new(),
            requests: Vec::new(),
            aborted: Vec::new(),
            faults: Faults::default(),
            model,
            revision: 7,
            seed: 1,
            session_open: false,
            settle: 0,
            settling_seen: false,
        }
    }

    fn t_now(&self) -> u64 {
        T_BASE + self.frames
    }

    fn entry(&self, u: usize) -> (TimeBaseEntry, u32) {
        let unit = &self.units[u];
        let t = self.t_now();
        let mut d = unit.disturbances;
        if let Some((x, f)) = self.faults.disturb
            && x == u
            && self.frames >= f
        {
            d = d.wrapping_add(1);
        }
        (
            TimeBaseEntry {
                time_base: 0,
                role: TimeBaseRole::from_raw(0),
                state: ContractState::from_raw(3),
                last_reason: TimeBaseReason::from_raw(0),
                domain: DOMAIN,
                t,
                disturbances: d,
                d: 192,
                phase_error: 0.0,
            },
            (t - 1_000) as u32,
        )
    }

    fn timeline(&self, u: usize) -> SiggenTimeline {
        let unit = &self.units[u];
        let now = self.t_now();
        let started: Vec<&Seg> = unit.segs.iter().filter(|s| s.t <= now).collect();
        let pending = unit.segs.iter().any(|s| s.t > now);
        let late = u64::from(self.faults.proof_late == Some(u));
        match started.last() {
            Some(s) => {
                let running = now < s.t + s.total;
                let state = if running {
                    3
                } else if pending {
                    1
                } else {
                    0
                };
                SiggenTimeline {
                    time_base: 0,
                    state: TimelineState::from_raw(state),
                    last_result: TimelineResult::from_raw(1),
                    discontinuities: 0,
                    arm_lead_ms: ARM_LEAD_MS,
                    start: s.t + late,
                    pending: 0,
                    samples: (now - s.t).min(s.total) as u32,
                    held: 0,
                    run_id: s.rid,
                }
            }
            None => SiggenTimeline {
                time_base: 0,
                state: TimelineState::from_raw(if pending { 1 } else { 0 }),
                last_result: TimelineResult::from_raw(0),
                discontinuities: 0,
                arm_lead_ms: ARM_LEAD_MS,
                start: 0,
                pending: 0,
                samples: 0,
                held: 0,
                run_id: unit.run_id - unit.segs.len() as u32,
            },
        }
    }

    fn global(&self, u: usize, o: u8) -> usize {
        self.units[..u]
            .iter()
            .map(|x| usize::from(x.outputs))
            .sum::<usize>()
            + usize::from(o)
    }

    /// Render what the segments play into the future capture.
    fn render(&mut self, segs: &[(usize, u64)], summed: bool) {
        let now = self.t_now();
        let n: usize = self.units.iter().map(|u| usize::from(u.outputs)).sum();
        let mut world = World::flat(n, RATE);
        world.clock.latency_ms = 5.0;
        for (u, unit) in self.units.iter().enumerate() {
            for o in 0..unit.outputs {
                let mut d = unit.acoustic_ms[usize::from(o)]
                    + (unit.effective + unit.latency) as f64 * 1000.0 / f64::from(RATE)
                    + f64::from(unit.out_delay_ms[usize::from(o)]);
                if summed
                    && let Some((x, ms)) = self.faults.sum_detune
                    && x == u
                {
                    d += ms;
                }
                world.outputs[self.global(u, o)].delay_ms = d;
            }
        }
        let mut signals: Vec<(usize, u64, Vec<f32>, f32)> = Vec::new();
        let mut end = 0u64;
        for &(u, t) in segs {
            let cfg = self.units[u].staged.clone().unwrap();
            let sweep = LogSweep::new(
                RATE,
                &SweepConfig {
                    f1_hz: cfg.p1,
                    f2_hz: cfg.p2,
                    duration_ms: cfg.duration_ms,
                    gap_ms: cfg.gap_ms,
                },
            )
            .unwrap();
            let walk = WalkSchedule::new(sweep, cfg.channel_mask, cfg.repeat_count).unwrap();
            let amp = peak_linear(cfg.level_db);
            for c in walk.cycles() {
                let mut at = t - now + c.start;
                if let Some((x, us)) = self.faults.wander_us
                    && x == u
                    && !summed
                {
                    self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let r = ((self.seed >> 33) as f64 / (1u64 << 31) as f64) * 2.0 - 1.0;
                    at = (at as i64 + (r * us * 48e-3).round() as i64) as u64;
                }
                let gain = if cfg.invert_mask & (1 << c.output) != 0 {
                    -1.0
                } else {
                    1.0
                };
                let samples: Vec<f32> = sweep
                    .render_cycle(c.index == 0)
                    .iter()
                    .map(|v| *v as f32 * amp)
                    .collect();
                end = end.max(at + samples.len() as u64);
                signals.push((self.global(u, c.output), at, samples, gain));
            }
        }
        let bursts: Vec<Burst<'_>> = signals
            .iter()
            .map(|(o, at, s, g)| Burst {
                output: *o,
                start_frame: *at,
                samples: s,
                gain: *g,
            })
            .collect();
        let stimulus = Stimulus {
            bursts,
            frames: end + 48_000,
        };
        let cap = render(&world, &stimulus, &flat_chains(n), &ProtoModel, RATE).unwrap();
        let base = self.frames as usize;
        if self.future.len() < base + cap.samples.len() {
            self.future.resize(base + cap.samples.len(), 0.0);
        }
        for (i, v) in cap.samples.iter().enumerate() {
            self.future[base + i] += *v;
        }
    }

    /// The next block of the microphone: the future capture plus noise at
    /// about -80 dBFS.
    fn block(&mut self) -> Vec<f32> {
        let base = self.frames as usize;
        (0..BLOCK)
            .map(|i| {
                self.seed = self
                    .seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let r = ((self.seed >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0;
                self.future.get(base + i).copied().unwrap_or(0.0) + r * 1.7e-4
            })
            .collect()
    }

    fn handle(&mut self, t: &str, body: Value) -> Result<Value> {
        self.requests.push(t.to_owned());
        let link = |code: &str, m: &str| DspiError::Link {
            code: code.into(),
            msg: m.into(),
        };
        match t {
            msg::t::OPEN => {
                self.session_open = true;
                let units = self
                    .units
                    .iter()
                    .enumerate()
                    .map(|(i, u)| {
                        let (e, of) = self.entry(i);
                        OpenUnit {
                            serial: u.serial.clone(),
                            time_base: 0,
                            role: 0,
                            state: 3,
                            d: 192,
                            arm_lead_ms: ARM_LEAD_MS,
                            outputs: u.outputs,
                            t: e.t,
                            origin_offset: 0,
                            t_minus_output: e.t_minus_output(of),
                            disturbances: e.disturbances,
                        }
                    })
                    .collect();
                Ok(serde_json::to_value(OpenReply {
                    session: "s1".into(),
                    time_base: 0,
                    domain: DOMAIN,
                    lease_ms: 2000,
                    units,
                })
                .unwrap())
            }
            msg::t::STAGE => {
                let r: msg::StageRequest = serde_json::from_value(body).unwrap();
                let c = dspi_proto::packets::SiggenConfig::decode(&r.config)
                    .map_err(|_| link("bad_config", "short"))?;
                dspi_measure::stimulus::check_timed_config(&c, 0)
                    .map_err(|e| link("bad_config", &e.to_string()))?;
                let u = self
                    .units
                    .iter_mut()
                    .find(|u| u.serial == r.serial)
                    .unwrap();
                u.staged = Some(c);
                Ok(json!({"v": 1, "serial": r.serial, "samples": 0}))
            }
            msg::t::RUN => {
                let r: msg::RunRequest = serde_json::from_value(body).unwrap();
                let lead = u64::from(r.lead_ms) * u64::from(RATE) / 1000;
                // The hub holds the first run back until the clocks settle.
                let t0 = self.t_now() + lead + std::mem::take(&mut self.settle);
                self.settling_seen = true;
                let mut placed = Vec::new();
                let mut segs = Vec::new();
                for s in &r.segments {
                    let u = usize::from(s.unit);
                    let cfg = self.units[u]
                        .staged
                        .clone()
                        .ok_or(link("bad_schedule", "unstaged"))?;
                    let sweep = LogSweep::new(
                        RATE,
                        &SweepConfig {
                            f1_hz: cfg.p1,
                            f2_hz: cfg.p2,
                            duration_ms: cfg.duration_ms,
                            gap_ms: cfg.gap_ms,
                        },
                    )
                    .unwrap();
                    let total = WalkSchedule::new(sweep, cfg.channel_mask, cfg.repeat_count)
                        .unwrap()
                        .total_samples();
                    let unit = &mut self.units[u];
                    unit.run_id += 1;
                    unit.segs.push(Seg {
                        t: t0 + s.offset,
                        total,
                        rid: unit.run_id,
                    });
                    placed.push(RunSegmentReply {
                        unit: s.unit,
                        t: t0 + s.offset,
                        t_unit: t0 + s.offset,
                    });
                    segs.push((u, t0 + s.offset));
                }
                let summed = r.segments.len() > 1 && r.segments.iter().all(|s| s.offset == 0);
                self.render(&segs, summed);
                Ok(serde_json::to_value(RunReply {
                    t0,
                    segments: placed,
                })
                .unwrap())
            }
            msg::t::STATUS => {
                if let Some((u, f)) = self.faults.hub_abort
                    && self.frames >= f
                    && self.session_open
                {
                    self.faults.hub_abort = None;
                    self.aborted.push(AbortedEvent {
                        session: "s1".into(),
                        unit: Some(u as u8),
                        reason: "foreign".into(),
                    });
                }
                let units = (0..self.units.len())
                    .map(|i| {
                        let (e, of) = self.entry(i);
                        let mut events = Vec::new();
                        if let Some((x, f)) = self.faults.clip
                            && x == i
                            && self.frames >= f
                        {
                            events.push(UnitEvent {
                                event: 0xE0,
                                raw: vec![2, 0xE0, 0, 1, 1, 0, 0, 0],
                            });
                        }
                        let mut peaks = vec![-60.0f32; 8];
                        if let Some((x, f)) = self.faults.peak
                            && x == i
                            && self.frames >= f
                        {
                            peaks[0] = 0.0;
                        }
                        UnitStatus {
                            unit: i as u8,
                            siggen: dspi_proto::packets::SiggenStatus::default()
                                .encode()
                                .to_vec(),
                            timeline: self.timeline(i).encode().to_vec(),
                            time_base_entry: e.encode().to_vec(),
                            output_frames: of,
                            output_peaks_dbfs: peaks,
                            clipped: false,
                            limiter_gr_db: vec![0.0; 8],
                            events,
                        }
                    })
                    .collect();
                // As ATHUB answers: the body under "status", `v` beside it.
                let body = serde_json::to_value(StatusReply {
                    t: self.t_now(),
                    hub_us: self.frames * 1_000_000 / u64::from(RATE),
                    units,
                })
                .unwrap();
                Ok(json!({"v": 1, "status": body}))
            }
            msg::t::ABORT => {
                let now = self.t_now();
                for u in &mut self.units {
                    u.segs.retain(|s| s.t <= now);
                }
                self.future.truncate(self.frames as usize);
                let mut v = serde_json::to_value(AbortReply {
                    units: (0..self.units.len())
                        .map(|i| AbortUnit {
                            unit: i as u8,
                            stopped: true,
                            error: None,
                        })
                        .collect(),
                })
                .unwrap();
                v["broadcast"] = json!("none");
                v["v"] = json!(1);
                Ok(v)
            }
            msg::t::CLOSE => {
                self.session_open = false;
                Ok(json!({}))
            }
            "system.status" => Ok(json!({
                "alignment": {
                    "rateHz": RATE,
                    "units": self.units.iter().map(|u| json!({
                        "serial": u.serial, "transport": "alink", "pathDelayFrames": 0,
                        "effective": u.effective, "latencyFrames": u.latency
                    })).collect::<Vec<_>>()
                }
            })),
            "param.get_many" => {
                let serial = body["serial"].as_str().unwrap_or("").to_owned();
                let u = self.units.iter().find(|u| u.serial == serial).unwrap();
                let values: Vec<Value> = body["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|it| {
                        let o = it["idx"][0].as_u64().unwrap() as usize;
                        json!({"path": "out.delay", "idx": [o], "ok": true, "value": u.out_delay_ms[o]})
                    })
                    .collect();
                Ok(json!({"serial": serial, "values": values}))
            }
            "system.get" => Ok(json!({"revision": self.revision, "model": self.model})),
            "system.set" => {
                if body["revision"].as_u64() != Some(self.revision) {
                    return Err(link("conflict", "the model changed"));
                }
                self.model = body["model"].clone();
                self.revision += 1;
                Ok(json!({"revision": self.revision, "warnings": []}))
            }
            "system.align" => {
                // The engine: a room measured whole gets m_latest - m; a
                // speaker without a measurement keeps its delay at 0.
                let mut writes = Vec::new();
                let all: Vec<f64> = self.model["units"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .flat_map(|u| u["speakers"].as_array().unwrap().clone())
                    .filter_map(|s| s["measuredArrivalUs"].as_f64())
                    .collect();
                let latest = all.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                for mu in self.model["units"].as_array().unwrap().clone() {
                    let serial = mu["serial"].as_str().unwrap().to_owned();
                    let unit = self.units.iter_mut().find(|u| u.serial == serial).unwrap();
                    for s in mu["speakers"].as_array().unwrap() {
                        let o = s["output"].as_u64().unwrap() as usize;
                        unit.out_delay_ms[o] = match s["measuredArrivalUs"].as_f64() {
                            Some(m) => ((latest - m) / 1000.0) as f32,
                            None => 0.0,
                        };
                    }
                    writes.push(json!({"serial": serial, "what": "out.delay", "result": "ok"}));
                }
                Ok(json!({"applied": true, "writes": writes}))
            }
            _ => Err(link("unsupported", t)),
        }
    }
}

#[derive(Clone)]
struct StubHub(Arc<Mutex<Stub>>);

impl Hub for StubHub {
    fn caps(&self) -> Option<Vec<String>> {
        Some(self.0.lock().unwrap().caps.clone())
    }
    fn role(&self) -> String {
        self.0.lock().unwrap().role.clone()
    }
    fn request(&mut self, t: &str, body: Value) -> Result<Value> {
        self.0.lock().unwrap().handle(t, body)
    }
    fn aborted(&mut self) -> Vec<AbortedEvent> {
        std::mem::take(&mut self.0.lock().unwrap().aborted)
    }
    fn stop_now_fallback(&mut self, _serials: &[String]) -> bool {
        false
    }
}

// ── Building blocks ─────────────────────────────────────────────────────

fn unit(serial: &str, outputs: u8, effective: i64, acoustic_ms: &[f64]) -> StubUnit {
    StubUnit {
        serial: serial.into(),
        outputs,
        effective,
        latency: 0,
        acoustic_ms: acoustic_ms.to_vec(),
        out_delay_ms: vec![0.0; usize::from(outputs)],
        staged: None,
        segs: Vec::new(),
        run_id: 40,
        disturbances: 5,
    }
}

fn setup(units: &[(&str, &[u8])]) -> MeasureSystemSetup {
    MeasureSystemSetup {
        units: units
            .iter()
            .map(|(s, outs)| MeasureSystemUnit {
                serial: (*s).into(),
                outputs: outs
                    .iter()
                    .map(|o| MeasureOutput {
                        output: *o,
                        role: MeasureRole::Full,
                        speaker: format!("{s}-{o}"),
                        ..MeasureOutput::default()
                    })
                    .collect(),
            })
            .collect(),
        passes: 3,
        sweep_ms: 1_000,
        gap_ms: 300,
        capture_rate_hz: RATE,
        ..MeasureSystemSetup::default()
    }
}

fn check(
    s: &MeasureSystemSetup,
    serial: &str,
    blockers: Vec<MeasureIssue>,
) -> MeasureSystemUnitCheck {
    let u = s.units.iter().find(|u| u.serial == serial).unwrap();
    let ctx = UnitContext {
        serial: serial.into(),
        platform: "RP2350".into(),
        device_rate_hz: RATE,
        num_inputs: 2,
        limiter_outputs: Vec::new(),
        chains: u
            .outputs
            .iter()
            .map(|o| (o.output, flat_chains(1).remove(0)))
            .collect(),
    };
    MeasureSystemUnitCheck {
        serial: serial.into(),
        preflight: MeasurePreflight {
            clear: blockers.is_empty(),
            blockers,
            warnings: Vec::new(),
        },
        context: serde_json::to_string(&ctx).unwrap(),
    }
}

fn plan_of(s: &MeasureSystemSetup, hub: &mut StubHub) -> SetupPlan {
    let checks: Vec<_> = s
        .units
        .iter()
        .map(|u| check(s, &u.serial, Vec::new()))
        .collect();
    let (pre, units) = plan::preflight(s, &checks, hub).unwrap();
    assert!(pre.clear, "{pre:?}");
    SetupPlan::new(s, units).unwrap()
}

/// Push, tick and advance the clock until the session ends.
fn drive(s: &mut SysSession, hub: &mut StubHub, max_ticks: usize) {
    let mut now = Instant::now();
    for _ in 0..max_ticks {
        if s.finished() {
            return;
        }
        let (frames, block) = {
            let mut st = hub.0.lock().unwrap();
            (st.frames, st.block())
        };
        s.push(hub, frames, &block, RATE, false);
        hub.0.lock().unwrap().frames += BLOCK as u64;
        now += Duration::from_millis(100);
        s.tick(hub, now);
    }
    panic!("the session did not end: {:?}", s.state());
}

fn two_units() -> StubHub {
    // An aligned pair as the engine leaves it: unit A's effective delay is
    // 48 frames with 72 of latency after the link, unit B's 120 with none,
    // so both play a domain sample 2.5 ms after it; the outputs' acoustic
    // delays differ.
    let mut a = unit("A", 2, 48, &[3.0, 3.4]);
    a.latency = 72;
    StubHub(Arc::new(Mutex::new(Stub::new(vec![
        a,
        unit("B", 2, 120, &[2.2, 4.1]),
    ]))))
}

fn measure(hub: &mut StubHub, s: &MeasureSystemSetup) -> (SysSession, Option<proposal::Outcome>) {
    let p = plan_of(s, hub);
    let mut sess =
        SysSession::open(p, MeasureSystemPurpose::Measure, None, hub, Instant::now()).unwrap();
    drive(&mut sess, hub, 4_000);
    let out = match sess.take_outcome() {
        Some(SessionOutput::Measured(o)) => Some(*o),
        _ => None,
    };
    (sess, out)
}

fn arrival<'a>(o: &'a proposal::Outcome, serial: &str, out: u8) -> &'a MeasureSystemArrival {
    o.result
        .arrivals
        .iter()
        .find(|a| a.serial == serial && a.output == out)
        .unwrap()
}

// ── The plan ────────────────────────────────────────────────────────────

#[test]
fn wp_atcore_two_and_four_unit_plans_interleave_with_the_k340_lead() {
    for n in [2usize, 4] {
        let names: Vec<String> = (0..n).map(|i| format!("U{i}")).collect();
        let outs: &[u8] = &[0, 1];
        let units: Vec<(&str, &[u8])> = names.iter().map(|s| (s.as_str(), outs)).collect();
        let s = setup(&units);
        let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(
            names.iter().map(|s| unit(s, 2, 48, &[1.0, 1.0])).collect(),
        ))));
        let p = plan_of(&s, &mut hub);
        assert_eq!(p.runs.len(), 1, "fits one run");
        let sess = SysSession::open(
            p.clone(),
            MeasureSystemPurpose::Measure,
            None,
            &mut hub,
            Instant::now(),
        )
        .unwrap();
        // A K340's arm lead of 150 ms plus 100 ms (S-AT3).
        assert_eq!(sess.lead(), 250);
        let sch = p.schedule(&p.runs[0], sess.lead()).unwrap();
        assert_eq!(sch.segments().len(), 3 * n, "three passes over {n} units");
        let lead = 250 * u64::from(RATE) / 1000;
        dspi_measure::system::check_lead(&sch, lead).unwrap();
        for (i, seg) in sch.segments().iter().enumerate() {
            assert_eq!(usize::from(seg.unit.0), i % n, "interleaved");
            assert_eq!(seg.schedule.repeat, 1);
            assert_eq!(seg.schedule.outputs(), vec![0, 1]);
        }
        assert_eq!(sch.start(), 0, "offsets from the run's T0");
        for w in sch.segments().windows(2) {
            assert!(
                w[1].start >= w[0].end() + 300 * 48,
                "the gap between segments"
            );
        }
    }
    // A group past 16 outputs is split into runs that each repeat the anchor.
    let outs: &[u8] = &[0, 1, 2, 3, 4, 5, 6, 7];
    let s = setup(&[("A", outs), ("B", outs), ("C", outs)]);
    let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(vec![
        unit("A", 8, 0, &[1.0; 8]),
        unit("B", 8, 0, &[1.0; 8]),
        unit("C", 8, 0, &[1.0; 8]),
    ]))));
    let p = plan_of(&s, &mut hub);
    assert!(p.runs.len() >= 2, "{:?}", p.runs);
    for r in &p.runs {
        assert!(
            r.units.iter().any(|(u, o)| *u == 0 && o.contains(&0)),
            "the anchor in {r:?}"
        );
        assert!(r.units.iter().map(|(_, o)| o.len()).sum::<usize>() <= 16);
    }
    let covered: usize = p
        .runs
        .iter()
        .map(|r| r.units.iter().map(|(_, o)| o.len()).sum::<usize>() - 1)
        .sum();
    assert_eq!(covered + 1, 24, "every output once besides the anchor");
}

#[test]
fn wp_atcore_each_unit_preflight_refusal_names_its_unit() {
    let s = setup(&[("A", &[0, 1]), ("B", &[0])]);
    let mut hub = two_units();
    for (key, out) in [
        ("measure.volume_high", None),         // S2
        ("measure.tweeter_highpass", Some(1)), // S6
        ("measure.input_audio", None),         // S8
        ("measure.crossfeed", None),           // S13
    ] {
        let checks = vec![
            check(&s, "A", Vec::new()),
            check(&s, "B", vec![MeasureIssue::new(key, out, "the rule")]),
        ];
        let (pre, _) = plan::preflight(&s, &checks, &mut hub).unwrap();
        assert!(!pre.clear);
        let b = pre.blockers.iter().find(|b| b.key == key).unwrap();
        assert_eq!(b.serial, "B");
        assert!(b.message.starts_with("B: "), "{b:?}");
        assert_eq!(b.output, out);
        assert!(
            plan::refusal(&pre)
                .unwrap()
                .to_string()
                .contains("B: the rule")
        );
    }
    // A unit not checked on its own connection is a blocker, named.
    let (pre, _) = plan::preflight(&s, &[check(&s, "A", Vec::new())], &mut hub).unwrap();
    let b = pre
        .blockers
        .iter()
        .find(|b| b.key == "measure.system.unit_check")
        .unwrap();
    assert_eq!(b.serial, "B");
    // A Viewer is refused.
    hub.0.lock().unwrap().role = "viewer".into();
    let checks = vec![check(&s, "A", Vec::new()), check(&s, "B", Vec::new())];
    let (pre, _) = plan::preflight(&s, &checks, &mut hub).unwrap();
    assert!(matches!(plan::refusal(&pre), Some(DspiError::Denied)));
}

#[test]
fn wp_atcore_a_hub_without_the_cap_gives_the_notice() {
    let core = crate::DspiCore::new();
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let hub = two_units();
    hub.0.lock().unwrap().caps = vec!["limits".into()];
    core.measure_system_set_hub(Box::new(hub.clone()));
    let checks = vec![check(&s, "A", Vec::new()), check(&s, "B", Vec::new())];
    let pre = core
        .measure_system_preflight(s.clone(), checks.clone())
        .unwrap();
    let b = &pre.blockers[0];
    assert_eq!(b.key, "measure.system.needs_hub");
    assert!(b.message.contains("needs the DSPi hub"));
    assert!(matches!(
        core.measure_system_start(s, checks),
        Err(DspiError::Unsupported)
    ));
    assert!(hub.0.lock().unwrap().requests.is_empty(), "nothing sent");
}

// ── A whole run ─────────────────────────────────────────────────────────

#[test]
fn wp_atcore_two_units_measure_propose_check_apply_verify_and_revert() {
    let s = setup(&[("A", &[0, 1]), ("B", &[0, 1])]);
    let mut hub = two_units();
    let (sess, out) = measure(&mut hub, &s);
    assert_eq!(
        sess.state().phase,
        MeasureSystemPhase::Done,
        "{:?}",
        sess.state().issue
    );
    let mut o = out.unwrap();
    // Every unit ramped, then three interleaved passes.
    assert_eq!(sess.state().ramp_levels.len(), 4);
    // The arrivals are acoustic, relative to the latest output (B out 1,
    // 4.1 ms): the play-out and the output delays are out.
    for (serial, out, ac) in [("A", 0u8, 3.0), ("A", 1, 3.4), ("B", 0, 2.2), ("B", 1, 4.1)] {
        let a = arrival(&o, serial, out);
        let want = (ac - 4.1) * 1000.0;
        assert!(
            (a.arrival_us - want).abs() < 3.0,
            "{serial}{out}: {a:?} want {want}"
        );
        assert!(a.offered, "{a:?}");
        assert!(a.uncertainty_us.is_some_and(|u| u < 10.0), "{a:?}");
    }
    assert_eq!(arrival(&o, "B", 1).arrival_us, 0.0);
    // The play-out (effective plus latency) is the same for both units:
    // the measured and the acoustic arrivals differ by one constant.
    for a in &o.result.arrivals {
        assert!((a.measured_us - a.arrival_us).abs() < 3.0, "{a:?}");
    }
    assert!(!o.result.delays.is_empty() && o.result.delays.iter().all(|d| d.offered));
    // One document per unit, each with its system block.
    assert_eq!(o.result.documents.len(), 2);
    for d in &o.result.documents {
        let doc = dspi_measure::doc::MeasurementDoc::from_text(&d.document).unwrap();
        let b = doc.measurement.system().unwrap();
        assert_eq!(b.group, vec!["A".to_owned(), "B".to_owned()]);
        assert_eq!(b.domain, DOMAIN);
        assert_eq!(b.run_ids.len(), 6);
        assert_eq!(doc.measurement.serial, d.serial);
    }
    // Full-range outputs on two units: transitions; the summed check first.
    assert!(!o.result.transitions.is_empty());
    let p = o.proposal("admin");
    assert!(p.sum_check_needed && !p.offered, "{p:?}");
    assert_eq!(p.notice.unwrap().key, "measure.system.sum_check_needed");
    assert!(
        o.apply(&mut hub, p.id).is_err(),
        "not before the summed check"
    );
    // The summed check: each transition's units at one T.
    let jobs = o.sum_jobs(MeasureSystemPurpose::SumCheck).unwrap();
    assert_eq!(jobs.len(), o.result.transitions.len());
    for j in &jobs {
        assert!(
            j.level_db <= -6.0,
            "6 dB below the lower of the two (S-AT4)"
        );
    }
    let mut sc = SysSession::open(
        o.setup.clone(),
        MeasureSystemPurpose::SumCheck,
        Some(jobs),
        &mut hub,
        Instant::now(),
    )
    .unwrap();
    drive(&mut sc, &mut hub, 4_000);
    let Some(SessionOutput::Verdicts(v)) = sc.take_outcome() else {
        panic!("{:?}", sc.state());
    };
    assert!(v.iter().all(|x| x.passed), "{v:?}");
    o.absorb_verdicts(MeasureSystemPurpose::SumCheck, v);
    // A Control role sees it and cannot apply.
    let p = o.proposal("control");
    assert!(p.offered && !p.may_apply, "{p:?}");
    assert_eq!(p.notice.unwrap().key, "measure.system.admin");
    hub.0.lock().unwrap().role = "control".into();
    assert!(matches!(o.apply(&mut hub, p.id), Err(DspiError::Denied)));
    assert!(
        !hub.0
            .lock()
            .unwrap()
            .requests
            .iter()
            .any(|r| r == "system.set")
    );
    // Admin applies: the revision and every output delay filed, the model
    // written, the units aligned.
    hub.0.lock().unwrap().role = "admin".into();
    let p = o.proposal("admin");
    assert!(p.offered && p.may_apply && p.notice.is_none(), "{p:?}");
    let applied = o.apply(&mut hub, p.id).unwrap();
    assert_eq!(applied.stage, MeasureSystemApplyStage::Applied);
    assert_eq!(applied.revision_before, 7);
    assert_eq!(applied.revision_after, 8);
    assert_eq!(applied.snapshots.len(), 2);
    assert!(
        applied
            .snapshots
            .iter()
            .all(|s| s.out_delays_ms.iter().all(|d| d.delay_ms == 0.0))
    );
    {
        let st = hub.0.lock().unwrap();
        let sp = &st.model["units"][1]["speakers"][1];
        assert_eq!(sp["measuredArrivalUs"].as_f64(), Some(0.0));
        assert_eq!(sp["measuredRuns"].as_array().unwrap().len(), 6);
        // A out 0 now waits 1.1 ms: the room plays aligned.
        assert!(
            (st.units[0].out_delay_ms[0] - 1.1).abs() < 0.005,
            "{:?}",
            st.units[0].out_delay_ms
        );
    }
    // The verify: the summed runs again, predicted with the new delays.
    let jobs = o.sum_jobs(MeasureSystemPurpose::Verify).unwrap();
    let mut vf = SysSession::open(
        o.setup.clone(),
        MeasureSystemPurpose::Verify,
        Some(jobs),
        &mut hub,
        Instant::now(),
    )
    .unwrap();
    drive(&mut vf, &mut hub, 4_000);
    let Some(SessionOutput::Verdicts(v)) = vf.take_outcome() else {
        panic!("{:?}", vf.state());
    };
    o.absorb_verdicts(MeasureSystemPurpose::Verify, v);
    assert_eq!(o.applied.verified, Some(true), "{}", o.applied.message);
    // Revert: the filed model again, the units back, read back.
    let r = o.revert(&mut hub).unwrap();
    assert_eq!(r.stage, MeasureSystemApplyStage::Reverted);
    assert!(r.message.contains("as filed"), "{}", r.message);
    let st = hub.0.lock().unwrap();
    assert!(
        st.units
            .iter()
            .all(|u| u.out_delay_ms.iter().all(|d| *d == 0.0))
    );
    assert!(
        st.model["units"][1]["speakers"][1]
            .get("measuredArrivalUs")
            .is_none()
    );
}

#[test]
fn wp_atcore_a_summed_run_that_disagrees_withdraws_every_cross_unit_delay() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(vec![
        unit("A", 1, 48, &[3.0]),
        unit("B", 1, 48, &[3.2]),
    ]))));
    let (_, out) = measure(&mut hub, &s);
    let mut o = out.unwrap();
    // The units' sum at the transition: B plays 0.5 ms later than the walked
    // run measured it (a delay the walked timing got wrong).
    hub.0.lock().unwrap().faults.sum_detune = Some((1, 0.5));
    let jobs = o.sum_jobs(MeasureSystemPurpose::SumCheck).unwrap();
    let mut sc = SysSession::open(
        o.setup.clone(),
        MeasureSystemPurpose::SumCheck,
        Some(jobs),
        &mut hub,
        Instant::now(),
    )
    .unwrap();
    drive(&mut sc, &mut hub, 4_000);
    let Some(SessionOutput::Verdicts(v)) = sc.take_outcome() else {
        panic!("{:?}", sc.state());
    };
    assert!(v.iter().any(|x| !x.passed), "{v:?}");
    o.absorb_verdicts(MeasureSystemPurpose::SumCheck, v);
    let p = o.proposal("admin");
    assert!(p.withdrawn && !p.offered && p.arrivals.is_empty(), "{p:?}");
    assert_eq!(p.notice.unwrap().key, "measure.system.sum_check_failed");
    assert!(p.withheld.iter().all(|a| !a.offered));
    assert!(o.apply(&mut hub, p.id).is_err());
}

#[test]
fn wp_atcore_a_wandering_delay_is_timing_unreliable_and_not_offered() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(vec![
        unit("A", 1, 48, &[3.0]),
        unit("B", 1, 48, &[3.2]),
    ]))));
    // B's segments land up to 60 us apart: the capture clock "wandered".
    hub.0.lock().unwrap().faults.wander_us = Some((1, 60.0));
    let (sess, out) = measure(&mut hub, &s);
    let o = out.unwrap_or_else(|| panic!("{:?}", sess.state()));
    let a = arrival(&o, "A", 0);
    let b = arrival(&o, "B", 0);
    let not = if a.arrival_us == 0.0 { b } else { a };
    assert!(!not.offered, "{not:?}");
    assert!(not.reason.contains("timing unreliable"), "{not:?}");
    let p = o.proposal("admin");
    assert!(!p.offered);
    let n = p.notice.unwrap();
    assert_eq!(n.key, "measure.system.timing_unreliable", "{n:?}");
    assert!(n.message.contains("timing unreliable"));
}

// ── Faults end the run, naming the unit ─────────────────────────────────

fn faulted(set: impl FnOnce(&mut Faults)) -> (SysSession, StubHub) {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(vec![
        unit("A", 1, 48, &[3.0]),
        unit("B", 1, 48, &[3.2]),
    ]))));
    set(&mut hub.0.lock().unwrap().faults);
    let (sess, out) = measure(&mut hub, &s);
    assert!(out.is_none(), "nothing kept");
    (sess, hub)
}

type SetFault = Box<dyn FnOnce(&mut Faults)>;

#[test]
fn wp_atcore_a_clip_a_peak_a_disturbance_and_a_hub_abort_end_the_run_naming_the_unit() {
    let cases: Vec<(&str, SetFault)> = vec![
        (
            "measure.system.abort.clip",
            Box::new(|f: &mut Faults| f.clip = Some((1, 96_000))),
        ),
        (
            "measure.system.abort.output_peak",
            Box::new(|f: &mut Faults| f.peak = Some((1, 96_000))),
        ),
        (
            "measure.system.abort.disturbance",
            Box::new(|f: &mut Faults| f.disturb = Some((1, 96_000))),
        ),
        (
            "measure.system.abort.hub",
            Box::new(|f: &mut Faults| f.hub_abort = Some((1, 96_000))),
        ),
    ];
    for (key, set) in cases {
        let (sess, hub) = faulted(set);
        let st = sess.state();
        assert_eq!(st.phase, MeasureSystemPhase::Aborted, "{key}: {st:?}");
        let i = st.issue.unwrap();
        assert_eq!(i.key, key);
        assert_eq!(i.serial, "B", "{i:?}");
        assert!(i.message.contains('B'), "{i:?}");
        let reqs = hub.0.lock().unwrap().requests.clone();
        assert!(reqs.iter().any(|r| r == msg::t::ABORT), "{key}: abort sent");
        assert_eq!(reqs.last().map(String::as_str), Some(msg::t::CLOSE));
        assert!(!hub.0.lock().unwrap().session_open);
    }
}

#[test]
fn wp_atcore_a_failed_proof_discards_the_run_naming_the_unit() {
    // B's timeline says every start was one sample after its T (G3).
    let (sess, _) = faulted(|f| f.proof_late = Some(1));
    let st = sess.state();
    // The ramp's own proof (last_result started) does not check the sample;
    // the run's gate does.
    assert_eq!(st.phase, MeasureSystemPhase::Failed, "{st:?}");
    let i = st.issue.unwrap();
    assert_eq!(i.key, "measure.system.gate");
    assert_eq!(i.serial, "B");
    assert!(i.message.contains("started at"), "{i:?}");
}

#[test]
fn wp_atcore_a_frame_gap_aborts_and_a_stop_aborts() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let mut hub = two_units();
    let p = plan_of(&s, &mut hub);
    let mut sess = SysSession::open(
        p,
        MeasureSystemPurpose::Measure,
        None,
        &mut hub,
        Instant::now(),
    )
    .unwrap();
    sess.push(&mut hub, 0, &[0.0; 480], RATE, false);
    sess.push(&mut hub, 960, &[0.0; 480], RATE, false);
    let st = sess.state();
    assert_eq!(st.phase, MeasureSystemPhase::Aborted);
    assert_eq!(st.issue.unwrap().key, "measure.abort.frame_gap");
    let mut hub = two_units();
    let p = plan_of(&s, &mut hub);
    let mut sess = SysSession::open(
        p,
        MeasureSystemPurpose::Measure,
        None,
        &mut hub,
        Instant::now(),
    )
    .unwrap();
    sess.stop_by_user(&mut hub);
    assert_eq!(sess.state().phase, MeasureSystemPhase::Aborted);
    assert!(
        hub.0
            .lock()
            .unwrap()
            .requests
            .iter()
            .any(|r| r == msg::t::ABORT)
    );
}

// ── Through the core ────────────────────────────────────────────────────

#[test]
fn wp_atcore_a_control_role_can_measure_but_not_apply_through_the_core() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(vec![
        unit("A", 1, 48, &[3.0]),
        unit("B", 1, 48, &[3.2]),
    ]))));
    hub.0.lock().unwrap().role = "control".into();
    let (_, out) = measure(&mut hub, &s);
    let mut o = out.expect("a Control role measures");
    for i in 0..o.result.transitions.len() {
        o.absorb_verdicts(
            MeasureSystemPurpose::SumCheck,
            vec![proposal::Verdict {
                job: i,
                passed: true,
                rms_db: Some(0.2),
                message: String::new(),
            }],
        );
    }
    let core = crate::DspiCore::new();
    core.measure_system_set_hub(Box::new(hub.clone()));
    core.system_slot().outcome = Some(o);
    let r = core.measure_system_result().unwrap();
    assert_eq!(r.documents.len(), 2);
    let p = core.measure_system_proposal().unwrap();
    assert!(p.offered && !p.may_apply, "{p:?}");
    assert!(matches!(
        core.measure_system_apply(p.id),
        Err(DspiError::Denied)
    ));
    assert!(matches!(
        core.measure_system_revert(),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert!(
        !hub.0
            .lock()
            .unwrap()
            .requests
            .iter()
            .any(|r| r == "system.set" || r == "system.get")
    );
}

// ── Pure parts ──────────────────────────────────────────────────────────

#[test]
fn wp_atcore_may_propose_needs_the_gate_and_the_offer() {
    assert!(may_propose(&[], true));
    assert!(!may_propose(&[], false));
    let g = [GateFinding::Disturbed {
        unit: UnitIndex(1),
        before: 1,
        after: 2,
    }];
    assert!(!may_propose(&g, true), "a failed gate proposes nothing");
    // Never P5's widened gate options for a system run.
    assert_eq!(
        analysis_options(),
        dspi_measure::ir::AnalysisOptions::default()
    );
}

#[test]
fn wp_atcore_two_outputs_of_chained_runs_are_judged_as_a_pair() {
    let c = |unit: u8, u: Option<f64>, offered: bool| ChainedOutput {
        run: usize::from(unit),
        output: UnitOutput {
            unit: UnitIndex(unit),
            output: 0,
        },
        relative_ms: 1.0,
        uncertainty_us: u,
        offered,
    };
    // Each offered against the anchor at 8 us: 11.3 us between them.
    let (ok, u, why) = chained_pair(&c(1, Some(8.0), true), &c(2, Some(8.0), true));
    assert!(!ok && (u.unwrap() - 128f64.sqrt()).abs() < 1e-9, "{why}");
    assert!(why.contains("timing unreliable"));
    let (ok, u, _) = chained_pair(&c(1, Some(6.0), true), &c(2, Some(7.0), true));
    assert!(ok && u.unwrap() < 10.0);
    // The anchor itself (0 us) against an offered output.
    assert!(chained_pair(&c(0, Some(0.0), true), &c(2, Some(9.9), true)).0);
    assert!(!chained_pair(&c(1, Some(1.0), false), &c(2, Some(1.0), true)).0);
    assert!(!chained_pair(&c(1, None, true), &c(2, Some(1.0), true)).0);
}

#[test]
fn wp_atcore_the_engines_terms_are_read_per_transport() {
    let st = json!({"alignment": {"rateHz": 48000, "units": [
        {"serial": "L", "transport": "alink", "pathDelayFrames": 3, "effective": 189, "latencyFrames": 48},
        {"serial": "U", "transport": "usb_direct", "pathDelayFrames": 0, "effective": 240, "latencyFrames": 0},
        {"serial": "S", "transport": "spdif", "effective": 77, "latencyFrames": 24, "spdif": {"mode": "source"}},
        {"serial": "F", "transport": "spdif", "effective": 300, "latencyFrames": 24, "spdif": {"mode": "follower"}}
    ]}});
    let us = |s: &str| proposal::playout_us(&st, s).unwrap();
    assert!((us("L") - 240.0 / 48.0 * 1000.0).abs() < 1e-9);
    assert!((us("U") - 5_000.0).abs() < 1e-9);
    assert!((us("S") - 500.0).abs() < 1e-9, "a source plays T at once");
    assert!((us("F") - 324.0 / 48.0 * 1000.0).abs() < 1e-9);
    assert_eq!(proposal::playout_us(&st, "X"), None);
    assert_eq!(proposal::playout_us(&json!({}), "L"), None);
    let r = json!({"values": [{"ok": true, "value": 1.25}, {"ok": true, "value": 0}]});
    assert_eq!(
        proposal::out_delays_of(&r, &[2, 5]),
        Some(vec![(2, 1.25), (5, 0.0)])
    );
    let r = json!({"values": [{"ok": false, "code": "bad_request"}]});
    assert_eq!(proposal::out_delays_of(&r, &[2]), None);
}

#[test]
fn wp_atcore_only_finite_log_sweeps_are_staged() {
    let mut c = crate::records::SiggenConfig {
        signal_type: super::preflight::SIG_LOG_SWEEP,
        channel_mask: 1,
        invert_mask: 0,
        flags: 0,
        level_db: -30.0,
        duration_ms: 1_000,
        repeat: 1,
        gap_ms: 300,
        p: vec![20.0, 20_000.0, 0.0, 0.0],
    };
    assert_eq!(config_bytes(&c).unwrap().len(), 36);
    c.signal_type = super::preflight::SIG_PINK;
    assert!(config_bytes(&c).is_err(), "pink is continuous to op 9");
    c.signal_type = super::preflight::SIG_LOG_SWEEP;
    c.flags = crate::consts::SIGGEN_FLAG_RAW;
    assert!(config_bytes(&c).is_err());
}

#[test]
fn wp_atcore_the_time_base_event_is_decoded_and_logged() {
    use dspi_session::events::{ExtEvent, alink_record, decode_ext_event};
    // 0xE4 payload from byte 4: time base, state, reason, reserved, T low.
    let p = [0u8, 4, 3, 0, 0x78, 0x56, 0x34, 0x12];
    let Some(ExtEvent::TimeBase(e)) = decode_ext_event(Some(1 << 5), 0xE4, &p) else {
        panic!("not decoded");
    };
    assert_eq!(e.t_low, 0x1234_5678);
    assert_eq!(
        decode_ext_event(Some(1 << 4), 0xE4, &p),
        None,
        "only behind bit 5"
    );
    let r = alink_record(&ExtEvent::TimeBase(e), 1).unwrap();
    assert_eq!(r.source.as_deref(), Some("holdover"));
    assert!(r.text.starts_with("Time base"), "{}", r.text);
}

#[test]
fn wp_atcore_every_notice_key_has_a_row() {
    let sources = [
        include_str!("system.rs"),
        include_str!("system/plan.rs"),
        include_str!("system/session.rs"),
        include_str!("system/proposal.rs"),
    ];
    let mut keys = Vec::new();
    for src in sources {
        for part in src.split('"').skip(1).step_by(2) {
            if part.starts_with("measure.")
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_')
            {
                keys.push(part.to_owned());
            }
        }
    }
    keys.sort();
    keys.dedup();
    assert!(keys.len() > 20, "{keys:?}");
    for k in &keys {
        assert!(
            super::NOTICE_KEYS.contains(&k.as_str()),
            "{k} is in no NOTICE_KEYS"
        );
        assert!(crate::notice_text::has_row(k), "no notice row for {k}");
    }
}

proptest! {
    #[test]
    fn proptest_wp_atcore_the_terms_parsers_never_panic(
        rate in proptest::num::f64::ANY,
        eff in proptest::num::f64::ANY,
        lat in proptest::num::i64::ANY,
        vals in proptest::collection::vec(proptest::num::f64::ANY, 0..6),
        n in 0usize..6,
    ) {
        let st = json!({"alignment": {"rateHz": rate, "units": [
            {"serial": "A", "effective": eff, "latencyFrames": lat, "pathDelayFrames": lat}
        ]}});
        if let Some(us) = proposal::playout_us(&st, "A") {
            prop_assert!(us.is_finite());
        }
        let r = json!({"values": vals.iter().map(|v| json!({"ok": true, "value": v})).collect::<Vec<_>>()});
        let outs: Vec<u8> = (0..n as u8).collect();
        if let Some(d) = proposal::out_delays_of(&r, &outs) {
            prop_assert_eq!(d.len(), n);
            prop_assert!(d.iter().all(|(_, ms)| ms.is_finite() && *ms >= 0.0));
        }
    }

    #[test]
    fn proptest_wp_atcore_config_bytes_never_panics(
        t in 0u8..8, mask in proptest::num::u16::ANY, inv in proptest::num::u16::ANY,
        flags in proptest::num::u8::ANY, level in proptest::num::f32::ANY,
        ms in proptest::num::u32::ANY, repeat in proptest::num::u16::ANY,
    ) {
        let c = crate::records::SiggenConfig {
            signal_type: t, channel_mask: mask, invert_mask: inv, flags, level_db: level,
            duration_ms: ms, repeat, gap_ms: 300, p: vec![20.0, 20_000.0],
        };
        if config_bytes(&c).is_ok() {
            prop_assert_eq!(c.signal_type, super::preflight::SIG_LOG_SWEEP);
            prop_assert!(c.repeat >= 1 && c.channel_mask != 0);
        }
    }
}

#[test]
fn wp_atcore_a_run_the_hub_holds_back_for_settling_says_so_and_finishes() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(vec![
        unit("A", 1, 48, &[3.0]),
        unit("B", 1, 48, &[3.2]),
    ]))));
    // The first START waits 8 s more than its lead (ATHUB settles up to 40 s).
    hub.0.lock().unwrap().settle = 8 * u64::from(RATE);
    let p = plan_of(&s, &mut hub);
    let mut sess = SysSession::open(
        p,
        MeasureSystemPurpose::Measure,
        None,
        &mut hub,
        Instant::now(),
    )
    .unwrap();
    let mut now = Instant::now();
    let mut said = false;
    for _ in 0..4_000 {
        if sess.finished() {
            break;
        }
        let (frames, block) = {
            let mut st = hub.0.lock().unwrap();
            (st.frames, st.block())
        };
        sess.push(&mut hub, frames, &block, RATE, false);
        hub.0.lock().unwrap().frames += BLOCK as u64;
        now += Duration::from_millis(100);
        sess.tick(&mut hub, now);
        said |= sess.state().words.contains("settle");
    }
    assert!(hub.0.lock().unwrap().settling_seen);
    assert!(said, "the wait is said in words");
    assert_eq!(
        sess.state().phase,
        MeasureSystemPhase::Done,
        "{:?}",
        sess.state()
    );
    assert!(sess.state().words.is_empty());
}
