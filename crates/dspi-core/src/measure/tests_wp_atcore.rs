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
    // -- review AT-D1 --
    /// A hub from before WP ATSUMOFF: it does not know `summed` and refuses
    /// segments that overlap at different offsets (`bad_schedule`).
    old_hub: bool,
    /// Summed runs: the microphone's clock is this fast, ppm.
    sum_mic_ppm: Option<f64>,
    /// This unit's drivers are wired with inverted polarity.
    wired_inverted: Option<usize>,
    // -- WP ATDEMO --
    /// From this capture frame on, `status` answers `aborted` with these
    /// words (the hub stopped the session itself).
    status_aborted: Option<(u64, String)>,
    /// The simulated microphone: `Some(f)` serves `demo_mic` from the stub's
    /// capture, refusing it from frame `f` on (`u64::MAX`: never).
    demo_mic: Option<u64>,
    /// Blocks the room has heard and not served yet (the clock's pace).
    demo_ready: u32,
    /// `system.status` answers `rate_limited` this many times first.
    status_rate_limited: u32,
    // -- review AT-E --
    /// `(unit, ms)`: the hub reports this unit's `out.delay` that much
    /// larger than what it plays with (a wrong term).
    term_lie_ms: Option<(usize, f32)>,
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
    // -- WP ATGUARD -- the hub's guard of a system apply (with its cap)
    /// `(id, filed model, revision filed, timeout_ms)`.
    guard: Option<(u64, Value, u64, u64)>,
    guard_last: Option<msg::guard::Outcome>,
    guard_next: u64,
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
            guard: None,
            guard_last: None,
            guard_next: 0,
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
        if let (true, Some(ppm)) = (summed, self.faults.sum_mic_ppm) {
            world.clock.ppm = ppm;
        }
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
                if self.faults.wired_inverted == Some(u) {
                    world.outputs[self.global(u, o)].driver.inverted = true;
                }
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
                            // The D it runs (EXT_TIME_BASE.d).
                            d: u.effective as u16,
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
                // Review AT-D1: an older hub's `check_segments`.
                if self.faults.old_hub
                    && r.segments.windows(2).any(|w| w[0].offset != w[1].offset)
                    && r.segments.len() > 1
                    && self.units.iter().all(|u| u.staged.is_some())
                {
                    return Err(link("bad_schedule", "starts before the one before it ends"));
                }
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
                // WP ATSUMOFF: a summed run's parts may start apart.
                let summed = (r.summed && !self.faults.old_hub)
                    || (r.segments.len() > 1 && r.segments.iter().all(|s| s.offset == 0));
                self.render(&segs, summed);
                Ok(serde_json::to_value(RunReply {
                    t0,
                    segments: placed,
                })
                .unwrap())
            }
            // -- WP ATDEMO -- the demo hub's room: the stub's own capture
            msg::demo::T => {
                let Some(refuse_at) = self.faults.demo_mic else {
                    return Err(link("unsupported", "no room here"));
                };
                let req: msg::demo::DemoMicRequest = serde_json::from_value(body).unwrap();
                if self.frames >= refuse_at || req.units.iter().any(|u| u == "REAL") {
                    return Err(link("unsupported", "REAL is not a demo unit of this hub"));
                }
                if req.session.is_empty() {
                    return Ok(json!({"ok": true, "room": "the stub's room"}));
                }
                assert_eq!(req.from, self.frames, "read from where it was left");
                if self.faults.demo_ready == 0 {
                    return Ok(
                        json!({"ok": true, "first_frame": req.from, "rate_hz": RATE, "samples": []}),
                    );
                }
                self.faults.demo_ready -= 1;
                let block = self.block();
                let first = self.frames;
                self.frames += BLOCK as u64;
                let samples: Vec<i32> = block
                    .iter()
                    .map(|v| (v * msg::demo::FULL_SCALE) as i32)
                    .collect();
                Ok(
                    json!({"ok": true, "room": "the stub's room", "first_frame": first,
                    "rate_hz": RATE, "samples": samples}),
                )
            }
            msg::t::STATUS => {
                // -- WP ATDEMO --
                if let Some((f, words)) = self.faults.status_aborted.clone()
                    && self.frames >= f
                    && self.session_open
                {
                    return Err(link("aborted", &words));
                }
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
            // -- WP ATDEMO -- once a second per connection, as the hub's
            "system.status" if self.faults.status_rate_limited > 0 => {
                self.faults.status_rate_limited -= 1;
                Err(link(
                    "rate_limited",
                    "system.status at most once a second per session",
                ))
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
                let (ui, u) = self
                    .units
                    .iter()
                    .enumerate()
                    .find(|(_, u)| u.serial == serial)
                    .unwrap();
                // -- review AT-E --
                let lie = self
                    .faults
                    .term_lie_ms
                    .filter(|(x, _)| *x == ui)
                    .map_or(0.0, |(_, ms)| ms);
                let values: Vec<Value> = body["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|it| {
                        let o = it["idx"][0].as_u64().unwrap() as usize;
                        json!({"path": "out.delay", "idx": [o], "ok": true, "value": u.out_delay_ms[o] + lie})
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
            // -- WP ATGUARD -- as the hub's `system.guard.*`
            msg::guard::BEGIN => {
                if self.guard.is_some() {
                    return Err(link("busy", "a guard is pending"));
                }
                self.guard_next += 1;
                let ms = u64::from(msg::guard::clamp_timeout_ms(
                    body["timeout_ms"].as_u64().map(|v| v as u32),
                ));
                self.guard = Some((self.guard_next, self.model.clone(), self.revision, ms));
                Ok(json!({"guard": self.guard_next, "revision": self.revision, "timeout_ms": ms}))
            }
            msg::guard::CONFIRM => match self.guard.take() {
                Some(g) if body["guard"].as_u64() == Some(g.0) => {
                    let o = msg::guard::Outcome {
                        guard: g.0,
                        reason: "confirmed".into(),
                        result: "kept".into(),
                        revision_filed: g.2,
                        revision: self.revision,
                        ..Default::default()
                    };
                    self.guard_last = Some(o.clone());
                    Ok(serde_json::to_value(o).unwrap())
                }
                other => {
                    self.guard = other;
                    Err(link("conflict", "no such guard pending"))
                }
            },
            msg::guard::CANCEL => match &self.guard {
                Some(g) if body["guard"].as_u64() == Some(g.0) => {
                    Ok(serde_json::to_value(self.guard_revert("cancelled")).unwrap())
                }
                _ => Err(link("conflict", "no such guard pending")),
            },
            msg::guard::STATUS => Ok(json!({
                "active": self.guard.as_ref().map(|g| json!({
                    "guard": g.0, "session": 1, "revision_filed": g.2,
                    "revision_left": self.revision, "remaining_ms": g.3, "reverting": false
                })),
                "last": self.guard_last,
            })),
            _ => Err(link("unsupported", t)),
        }
    }

    // -- WP ATGUARD --
    /// The hub reverts its guard (`reason`: `cancelled`, `timeout`,
    /// `session_closed`): the filed model set at the revision now, aligned.
    fn guard_revert(&mut self, reason: &str) -> msg::guard::Outcome {
        let (id, model, filed, _) = self.guard.take().unwrap();
        let revision = self.revision;
        let set = self
            .handle("system.set", json!({"model": model, "revision": revision}))
            .unwrap();
        let report = self.handle("system.align", json!({"apply": true})).unwrap();
        let o = msg::guard::Outcome {
            guard: id,
            reason: reason.into(),
            result: "reverted".into(),
            revision_filed: filed,
            revision: set["revision"].as_u64().unwrap(),
            writes_ok: report["writes"].as_array().unwrap().len() as u32,
            message: "reverted: the filed model is set again and the units aligned".into(),
            ..Default::default()
        };
        self.guard_last = Some(o.clone());
        o
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
        roles: Vec::new(),
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

// -- WP ROLL1 -- one sweep from 20 Hz plays every unit: no tweeter in it
#[test]
fn wp_roll1_a_tweeter_is_refused_in_a_measurement_across_units() {
    let mut s = setup(&[("A", &[0, 1]), ("B", &[0])]);
    let mut hub = two_units();
    let checks = vec![check(&s, "A", Vec::new()), check(&s, "B", Vec::new())];
    let (pre, _) = plan::preflight(&s, &checks, &mut hub).unwrap();
    assert!(pre.clear, "{pre:?}");
    // Unit A's output 1 has a 2 kHz high-pass: a tweeter, called Full.
    let mut a = check(&s, "A", Vec::new());
    let mut ctx: UnitContext = serde_json::from_str(&a.context).unwrap();
    ctx.chains[1].1.crossover = vec![dspi_measure::types::BandSpec {
        filter_type: 35,
        bypass: false,
        freq: 2_000.0,
        q: 0.707,
        gain: 0.0,
        qp: 0.0,
    }];
    a.context = serde_json::to_string(&ctx).unwrap();
    let (pre, _) = plan::preflight(&s, &[a, check(&s, "B", Vec::new())], &mut hub).unwrap();
    assert!(!pre.clear);
    let b = pre
        .blockers
        .iter()
        .find(|b| b.key == "measure.tweeter_stimulus")
        .unwrap();
    assert_eq!((b.serial.as_str(), b.output), ("A", Some(1)));
    assert!(b.message.contains("Out 2 is a tweeter"), "{}", b.message);
    // -- WP RCFIX -- called Full, flat chain, but a tweeter on its own core
    // (its confirmed layout): refused.
    let mut b = check(&s, "B", Vec::new());
    let mut ctx: UnitContext = serde_json::from_str(&b.context).unwrap();
    ctx.roles = vec![(0, MeasureRole::High)];
    b.context = serde_json::to_string(&ctx).unwrap();
    let (pre, _) = plan::preflight(&s, &[check(&s, "A", Vec::new()), b], &mut hub).unwrap();
    let b = pre
        .blockers
        .iter()
        .find(|b| b.key == "measure.tweeter_stimulus")
        .unwrap();
    assert_eq!((b.serial.as_str(), b.output), ("B", Some(0)));
    // Called a tweeter, flat chain: refused too.
    s.units[1].outputs[0].role = MeasureRole::High;
    let checks = vec![check(&s, "A", Vec::new()), check(&s, "B", Vec::new())];
    let (pre, _) = plan::preflight(&s, &checks, &mut hub).unwrap();
    assert!(
        pre.blockers
            .iter()
            .any(|b| b.key == "measure.tweeter_stimulus" && b.serial == "B"),
        "{pre:?}"
    );
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
        // -- WP ATOVL -- overlapped starts by default: one interleaved pass
        // and three overlapped, two units: eight segments (six before).
        assert_eq!(o.result.plan, "overlapped starts, 50 ms apart");
        assert_eq!(b.run_ids.len(), 8);
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
        // -- WP ATOVL -- eight segments overlapped (six interleaved).
        assert_eq!(sp["measuredRuns"].as_array().unwrap().len(), 8);
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
    use proposal::PlayOut;
    let engine =
        |s: &str, source: bool| proposal::playout_us(&st, s, PlayOut::Engine { source }).unwrap();
    assert!((engine("L", false) - 240.0 / 48.0 * 1000.0).abs() < 1e-9);
    assert!((engine("U", false) - 5_000.0).abs() < 1e-9);
    assert!(
        (engine("S", true) - 500.0).abs() < 1e-9,
        "a source plays T at once"
    );
    assert!((engine("F", false) - 324.0 / 48.0 * 1000.0).abs() < 1e-9);
    let none = PlayOut::Engine { source: false };
    assert_eq!(proposal::playout_us(&st, "X", none), None);
    assert_eq!(proposal::playout_us(&json!({}), "L", none), None);
    // -- review AT-C -- during a session D is the unit's own word
    // (`EXT_TIME_BASE.d` at open), whatever the alignment says meanwhile:
    // the follower runs 150 where the hub, unable to read it, says 300.
    let own = |s: &str, d: u16| {
        proposal::playout_us(&st, s, PlayOut::Own { d, rate_hz: 48_000 }).unwrap()
    };
    assert!((own("F", 150) - 174.0 / 48.0 * 1000.0).abs() < 1e-9);
    assert!((own("S", 0) - 500.0).abs() < 1e-9, "a source: D = 0");
    assert!((own("L", 189) - 240.0 / 48.0 * 1000.0).abs() < 1e-9);
    // A line unit that is also an S/PDIF source (R7b), measured with its
    // line group: its line entry's D, not "a source plays at once".
    let st2 = json!({"alignment": {"rateHz": 48000, "units": [
        {"serial": "LS", "transport": "alink", "pathDelayFrames": 3, "effective": 125,
         "latencyFrames": 16, "spdif": {"mode": "source", "lagSamples": 192}}
    ]}});
    let us = proposal::playout_us(
        &st2,
        "LS",
        PlayOut::Own {
            d: 125,
            rate_hz: 48_000,
        },
    )
    .unwrap();
    assert!((us - 144.0 / 48.0 * 1000.0).abs() < 1e-9);
    assert_eq!(
        proposal::playout_us(&st, "L", PlayOut::Own { d: 1, rate_hz: 0 }),
        None
    );
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
        if let Some(us) = proposal::playout_us(&st, "A", proposal::PlayOut::Engine { source: false }) {
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

// -- review AT-C --
fn sum_session(
    o: &proposal::Outcome,
    hub: &mut StubHub,
    purpose: MeasureSystemPurpose,
) -> Vec<proposal::Verdict> {
    let jobs = o.sum_jobs(purpose).unwrap();
    let mut sc =
        SysSession::open(o.setup.clone(), purpose, Some(jobs), hub, Instant::now()).unwrap();
    drive(&mut sc, hub, 4_000);
    let Some(SessionOutput::Verdicts(v)) = sc.take_outcome() else {
        panic!("{:?}", sc.state());
    };
    v
}

/// At one T the sum check was judged only as far up as its window saw both
/// arrivals (703 Hz for two outputs 8 ms apart), where a walked delay wrong
/// by 100 us still agreed within 1.5 dB. WP ATSUMOFF: its parts now start
/// apart by their predicted arrivals, so it judges the whole transition
/// band and fails that delay. The verify must not be narrower either: after
/// the apply the arrivals coincide, the whole band is judged (up to a
/// quarter of the rate), and a wrong delay fails it.
#[test]
fn review_atc_the_verify_judges_the_whole_band_and_fails_a_wrong_delay() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(vec![
        unit("A", 1, 48, &[3.0]),
        unit("B", 1, 48, &[11.0]),
    ]))));
    let (_, out) = measure(&mut hub, &s);
    let mut o = out.unwrap();
    let top = o.result.transitions[0].band_hi_hz;
    assert!(top > 5_000.0, "the sum check's reach: {top}");
    let jobs = o.sum_jobs(MeasureSystemPurpose::SumCheck).unwrap();
    // A arrives 8 ms (384 samples) earlier: it starts that much later.
    assert_eq!(jobs[0].starts, (384, 0), "{:?}", jobs[0].starts);
    // B plays 0.1 ms later than the walked run measured it.
    hub.0.lock().unwrap().faults.sum_detune = Some((1, 0.1));
    let v = sum_session(&o, &mut hub, MeasureSystemPurpose::SumCheck);
    assert!(!v[0].passed, "100 us is told over the whole band: {v:?}");
    assert!(v[0].rms_db.is_some(), "{v:?}");
    // Right again: the sum check passes; the apply; then wrong again.
    hub.0.lock().unwrap().faults.sum_detune = None;
    let v = sum_session(&o, &mut hub, MeasureSystemPurpose::SumCheck);
    assert!(v[0].passed, "{v:?}");
    o.absorb_verdicts(MeasureSystemPurpose::SumCheck, v);
    let p = o.proposal("admin");
    assert!(p.offered, "{p:?}");
    o.apply(&mut hub, p.id).unwrap();
    hub.0.lock().unwrap().faults.sum_detune = Some((1, 0.1));
    let jobs = o.sum_jobs(MeasureSystemPurpose::Verify).unwrap();
    assert!(
        (jobs[0].band.1 - 12_000.0).abs() < 1.0,
        "the verify's band: {:?}",
        jobs[0].band
    );
    let v = sum_session(&o, &mut hub, MeasureSystemPurpose::Verify);
    assert!(!v[0].passed, "{v:?}");
    o.absorb_verdicts(MeasureSystemPurpose::Verify, v);
    assert_eq!(o.applied.verified, Some(false), "{}", o.applied.message);
    // As P5's Keep: after a verify that did not pass, the first Keep is
    // refused and says why; a second one right away (a double click) too;
    // one after the user had time to read it keeps.
    assert!(o.keep(&mut hub).is_err());
    assert_eq!(o.applied.stage, MeasureSystemApplyStage::Verified);
    assert!(
        o.applied.message.contains("did not confirm"),
        "{}",
        o.applied.message
    );
    assert!(o.keep(&mut hub).is_err(), "a double click");
    crate::guarded::skew::advance(Duration::from_millis(1_100));
    assert_eq!(
        o.keep(&mut hub).unwrap().stage,
        MeasureSystemApplyStage::Kept
    );
    // And a right delay passes it over the same band.
    let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(vec![
        unit("A", 1, 48, &[3.0]),
        unit("B", 1, 48, &[11.0]),
    ]))));
    let (_, out) = measure(&mut hub, &s);
    let mut o = out.unwrap();
    let v = sum_session(&o, &mut hub, MeasureSystemPurpose::SumCheck);
    o.absorb_verdicts(MeasureSystemPurpose::SumCheck, v);
    let p = o.proposal("admin");
    o.apply(&mut hub, p.id).unwrap();
    let v = sum_session(&o, &mut hub, MeasureSystemPurpose::Verify);
    assert!(v[0].passed, "{v:?}");
    o.absorb_verdicts(MeasureSystemPurpose::Verify, v);
    assert_eq!(
        o.keep(&mut hub).unwrap().stage,
        MeasureSystemApplyStage::Kept
    );
}

// -- WP ATSUMOFF --
/// USB-direct puts the streams' start offsets into `out.delay` (tens of
/// ms). At one T the two units of a summed run then sounded 30 ms apart, the
/// band's top (0.375 * 15 / dt, 188 Hz) fell under its floor and every
/// cross-unit delay was withdrawn. Now the part predicted to arrive earlier
/// starts that much later: the sum check judges the whole band and passes,
/// the apply goes through, the verify passes, and the summed run's starts
/// keep the lead (each at `t0 + offset`, the later at most 1 s after).
#[test]
fn wp_atsumoff_out_delays_tens_of_ms_apart_are_checked_over_the_whole_band() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let mut a = unit("A", 1, 48, &[3.0]);
    a.out_delay_ms = vec![30.0];
    let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(vec![
        a,
        unit("B", 1, 48, &[4.2]),
    ]))));
    let (_, out) = measure(&mut hub, &s);
    let mut o = out.unwrap();
    // The acoustic arrivals: the output delay is out.
    assert!((arrival(&o, "A", 0).arrival_us + 1_200.0).abs() < 3.0);
    assert!(o.result.transitions[0].band_hi_hz > 5_000.0);
    let jobs = o.sum_jobs(MeasureSystemPurpose::SumCheck).unwrap();
    // A is heard 28.8 ms (1382 samples) after B at one T: B starts later.
    assert_eq!(jobs[0].starts.0, 0, "{:?}", jobs[0].starts);
    assert!(
        (jobs[0].starts.1 as i64 - 1_382).abs() <= 1,
        "{:?}",
        jobs[0].starts
    );
    assert!(jobs[0].band.1 > 5_000.0, "{:?}", jobs[0].band);
    let v = sum_session(&o, &mut hub, MeasureSystemPurpose::SumCheck);
    assert!(v[0].passed, "{v:?}");
    {
        // The run asked for a summed run; B's start is its offset after A's.
        let st = hub.0.lock().unwrap();
        let (sa, sb) = (
            st.units[0].segs.last().unwrap().t,
            st.units[1].segs.last().unwrap().t,
        );
        assert_eq!(sb - sa, jobs[0].starts.1);
    }
    o.absorb_verdicts(MeasureSystemPurpose::SumCheck, v);
    let p = o.proposal("admin");
    assert!(p.offered && !p.withdrawn, "{p:?}");
    o.apply(&mut hub, p.id).unwrap();
    let v = sum_session(&o, &mut hub, MeasureSystemPurpose::Verify);
    assert!(v[0].passed, "{v:?}");
    o.absorb_verdicts(MeasureSystemPurpose::Verify, v);
    assert_eq!(o.applied.verified, Some(true), "{}", o.applied.message);
    // A delay wrong by 100 us (B later than measured) fails the next sum
    // check over that band: judged, not "too few points".
    hub.0.lock().unwrap().faults.sum_detune = Some((1, 0.1));
    let jobs = o.sum_jobs(MeasureSystemPurpose::Verify).unwrap();
    assert!(jobs[0].band.1 > 5_000.0, "{:?}", jobs[0].band);
    let v = sum_session(&o, &mut hub, MeasureSystemPurpose::Verify);
    assert!(!v[0].passed && v[0].rms_db.is_some(), "{v:?}");
}

#[test]
fn wp_atsumoff_sum_starts_bring_the_arrivals_together_within_the_spread() {
    use proposal::sum_starts;
    // A later by 8 ms: B starts 384 samples later; together to half a sample.
    let ((a, b), apart) = sum_starts(11.0, 3.0, RATE);
    assert_eq!((a, b), (0, 384));
    assert!(apart.abs() < 1e-9, "{apart}");
    let ((a, b), apart) = sum_starts(3.0, 3.0101, RATE);
    assert_eq!((a, b), (0, 0));
    assert!((apart + 0.0101).abs() < 1e-9, "{apart}");
    let ((a, b), apart) = sum_starts(3.0, 3.03, RATE);
    assert_eq!((a, b), (1, 0));
    assert!(apart.abs() < 0.5 / 48.0, "{apart}");
    // Wider than 1 s: capped, the rest left in (the band narrows).
    let ((a, b), apart) = sum_starts(0.0, 1_500.0, RATE);
    assert_eq!((a, b), (48_000, 0));
    assert!((apart + 500.0).abs() < 1e-9, "{apart}");
    // Nothing finite: no offset, the difference left in.
    assert_eq!(sum_starts(f64::NAN, 0.0, RATE).0, (0, 0));
    assert_eq!(sum_starts(1.0, 0.0, 0).0, (0, 0));
}

// -- review AT-D1 --
/// A hub from before WP ATSUMOFF refuses a summed run whose parts start
/// apart (`bad_schedule`: its segments overlap at different offsets). The
/// sum check then ends with no verdict, nothing is offered and an apply is
/// refused: it fails closed.
#[test]
fn review_atd1_an_older_hub_refuses_staggered_starts_and_nothing_is_offered() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(vec![
        unit("A", 1, 48, &[3.0]),
        unit("B", 1, 48, &[11.0]),
    ]))));
    let (_, out) = measure(&mut hub, &s);
    let mut o = out.unwrap();
    hub.0.lock().unwrap().faults.old_hub = true;
    let jobs = o.sum_jobs(MeasureSystemPurpose::SumCheck).unwrap();
    assert_eq!(jobs[0].starts, (384, 0));
    let mut sc = SysSession::open(
        o.setup.clone(),
        MeasureSystemPurpose::SumCheck,
        Some(jobs),
        &mut hub,
        Instant::now(),
    )
    .unwrap();
    drive(&mut sc, &mut hub, 4_000);
    let st = sc.state();
    assert!(
        st.issue
            .as_ref()
            .is_some_and(|i| i.key == "measure.system.hub_refused"),
        "{st:?}"
    );
    assert!(
        !matches!(sc.take_outcome(), Some(SessionOutput::Verdicts(_))),
        "no verdict from a run the hub refused"
    );
    let p = o.proposal("admin");
    assert!(!p.offered, "{p:?}");
    assert!(o.apply(&mut hub, p.id).is_err());
}

/// One row of the table: the sum check's verdict for a pair `gap_ms` apart
/// (`out.delay` on A beyond 8 ms, the acoustics below) when B plays
/// `detune_us` later than the walked run measured it.
fn sum_check_cell(
    o: &proposal::Outcome,
    hub: &mut StubHub,
    detune_us: f64,
    ppm: Option<f64>,
) -> (bool, Option<f64>) {
    {
        let mut st = hub.0.lock().unwrap();
        st.faults.sum_detune = (detune_us != 0.0).then_some((1, detune_us / 1000.0));
        st.faults.sum_mic_ppm = ppm;
    }
    let jobs = o.sum_jobs(MeasureSystemPurpose::SumCheck).unwrap();
    let mut sc = SysSession::open(
        o.setup.clone(),
        MeasureSystemPurpose::SumCheck,
        Some(jobs),
        hub,
        Instant::now(),
    )
    .unwrap();
    drive(&mut sc, hub, 4_000);
    match sc.take_outcome() {
        Some(SessionOutput::Verdicts(v)) => (v[0].passed, v[0].rms_db),
        _ => {
            eprintln!("the summed run gave no verdict: {:?}", sc.state().issue);
            (false, None)
        }
    }
}

fn pair_apart(gap_ms: f64) -> StubHub {
    let (mut a, b) = if gap_ms <= 8.0 {
        (unit("A", 1, 48, &[3.0 + gap_ms]), unit("B", 1, 48, &[3.0]))
    } else {
        (unit("A", 1, 48, &[3.0]), unit("B", 1, 48, &[3.0]))
    };
    if gap_ms > 8.0 {
        a.out_delay_ms = vec![gap_ms as f32];
    }
    StubHub(Arc::new(Mutex::new(Stub::new(vec![a, b]))))
}

/// The table's gaps, ms; a negative one: that far apart, unit B wired inverted.
const GAPS: [f64; 12] = [
    0.2, 2.0, 8.0, 30.0, 45.0, 60.0, 80.0, 100.0, 150.0, 200.0, -2.0, -30.0,
];

/// The AT-C review's F3 table again, with WP ATSUMOFF's staggered starts:
/// the sum check over arrival gaps and wrong delays (`cargo test -p
/// dspi-core --lib review_atd1_sum_check_table -- --ignored --nocapture`).
#[test]
#[ignore = "review AT-D1: the table, a minute or two"]
fn review_atd1_sum_check_table() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let detunes = [
        0.0, 10.0, -10.0, 20.0, -20.0, 30.0, -30.0, 50.0, -50.0, 100.0, 200.0, 500.0, 5_000.0,
        100_000.0,
    ];
    println!("gap ms | starts | band top Hz | detune us: verdict (dB RMS) ...");
    // A negative gap: that far apart, and unit B wired with inverted polarity.
    for gap in GAPS {
        let mut hub = pair_apart(f64::abs(gap));
        hub.0.lock().unwrap().faults.wired_inverted = (gap < 0.0).then_some(1);
        let (sess, out) = measure(&mut hub, &s);
        let Some(o) = out else {
            println!(
                "{gap:>6} | the walked run gave no result: {:?}",
                sess.state().issue
            );
            continue;
        };
        let jobs = match o.sum_jobs(MeasureSystemPurpose::SumCheck) {
            Ok(j) => j,
            Err(e) => {
                println!("{gap:>6} | no sum check: {e}; {:?}", o.result.arrivals);
                continue;
            }
        };
        let mut row = format!(
            "{gap:>6} | offered {} | {:?} | {:>7.0} |",
            o.result.arrivals.iter().all(|a| a.offered),
            jobs[0].starts,
            jobs[0].band.1
        );
        if std::env::var_os("REVIEW_ATD1_DEBUG").is_some() {
            println!("arrivals {:?}", o.result.arrivals);
            let f = jobs[0].predicted.grid.freqs().unwrap();
            for i in (0..f.len()).step_by(f.len() / 12) {
                println!(
                    "  predicted {:>8.1} Hz {:>7.2} dB",
                    f[i], jobs[0].predicted.mag_db[i]
                );
            }
        }
        for d in detunes {
            let (pass, rms) = sum_check_cell(&o, &mut hub, d, None);
            row += &format!(
                " {d}: {} ({})",
                if pass { "PASS" } else { "fail" },
                rms.map_or("-".into(), |r| format!("{r:.2}"))
            );
        }
        for ppm in [31.0, 100.0] {
            for d in [0.0, 50.0] {
                let (pass, rms) = sum_check_cell(&o, &mut hub, d, Some(ppm));
                row += &format!(
                    " | {ppm} ppm {d}: {} ({})",
                    if pass { "PASS" } else { "fail" },
                    rms.map_or("-".into(), |r| format!("{r:.2}"))
                );
            }
        }
        println!("{row}");
    }
}

/// The table's claims, kept in the suite: at every gap the starts can bring
/// together, a delay wrong by 50 us or more (either way) fails the sum check
/// as a judged failure and a right one passes, with the microphone's clock
/// off by 100 ppm too.
#[test]
fn review_atd1_staggered_starts_hide_no_wrong_delay() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    for gap in [2.0, 30.0, 80.0] {
        let mut hub = pair_apart(gap);
        let (_, out) = measure(&mut hub, &s);
        let o = out.unwrap();
        assert!(o.result.arrivals.iter().all(|a| a.offered), "{gap} ms");
        for ppm in [None, Some(100.0)] {
            let (pass, rms) = sum_check_cell(&o, &mut hub, 0.0, ppm);
            assert!(pass, "{gap} ms, right, {ppm:?}: {rms:?}");
            for d in [50.0, -50.0, 500.0, 5_000.0] {
                let (pass, rms) = sum_check_cell(&o, &mut hub, d, ppm);
                assert!(!pass && rms.is_some(), "{gap} ms, {d} us wrong: {rms:?}");
            }
        }
    }
}

/// An output heard 100 ms after the run's first lies at the end of what the
/// walked analysis keeps (100 ms with the 300 ms gap): its arrival came out
/// clamped (99.98 ms, "uncertainty" 0.35 us), its response cut, and it was
/// offered. Now nothing of such a group is offered, and the reason says why.
///
/// WP ATLATE: the review made A late by its `out.delay`, which the planner
/// now plans for (`wp_atlate_*` below). The guard is for what the planner
/// cannot know: here the 100 ms are acoustic (or a wrong term).
#[test]
fn review_atd1_an_arrival_at_the_end_of_the_analysed_span_is_not_offered() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(vec![
        unit("A", 1, 48, &[103.0]),
        unit("B", 1, 48, &[3.0]),
    ]))));
    let (_, out) = measure(&mut hub, &s);
    let o = out.unwrap();
    assert!(
        o.result
            .arrivals
            .iter()
            .all(|a| !a.offered && a.reason.contains("too long after the run's first output")),
        "{:?}",
        o.result.arrivals
    );
    assert!(
        o.result.reasons.iter().any(|r| r.contains("A output 0")),
        "{:?}",
        o.result.reasons
    );
    let p = o.proposal("admin");
    assert!(!p.offered && p.arrivals.is_empty(), "{p:?}");
    // -- review AT-E -- nor is the cross-unit delay shown as offered.
    assert!(
        o.result
            .delays
            .iter()
            .all(|d| !d.offered && d.reason.contains("too long after")),
        "{:?}",
        o.result.delays
    );
}

/// Two outputs of opposite polarity, their arrivals brought together by the
/// summed run's starts (WP ATSUMOFF), cancel over the whole band. Played,
/// the run was nearly silent and the session failed on "cycle identity: the
/// first cycle was not heard" (at one T, 2 ms apart, the comb left points
/// to judge). Now the sum check is refused before anything plays, naming
/// the pair and the polarity, and nothing is offered.
#[test]
fn review_atd1_outputs_of_opposite_polarity_are_not_played_as_a_sum() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let mut hub = pair_apart(2.0);
    hub.0.lock().unwrap().faults.wired_inverted = Some(1);
    let (_, out) = measure(&mut hub, &s);
    let o = out.unwrap();
    let e = o.sum_jobs(MeasureSystemPurpose::SumCheck).unwrap_err();
    let e = e.to_string();
    assert!(
        e.contains("opposite polarity") && e.contains("A output 0") && e.contains("B output 0"),
        "{e}"
    );
    let p = o.proposal("admin");
    assert!(p.sum_check_needed && !p.offered, "{p:?}");
    // The same pair in phase is played and judged.
    let mut hub = pair_apart(2.0);
    let (_, out) = measure(&mut hub, &s);
    assert!(
        out.unwrap()
            .sum_jobs(MeasureSystemPurpose::SumCheck)
            .is_ok()
    );
}

// ── WP ATGUARD: the hub's guard of a system apply ──────────────────────

/// Two units measured and summed-checked on a hub with (or without) the
/// guard's cap: the outcome, ready to apply.
fn atguard_checked(guarded: bool) -> (StubHub, proposal::Outcome, MeasureSystemSetup) {
    let s = setup(&[("A", &[0, 1]), ("B", &[0, 1])]);
    let mut hub = two_units();
    if guarded {
        hub.0.lock().unwrap().caps.push(msg::guard::CAP.to_owned());
    }
    let (_, out) = measure(&mut hub, &s);
    let mut o = out.unwrap();
    let v = sum_session(&o, &mut hub, MeasureSystemPurpose::SumCheck);
    assert!(v.iter().all(|x| x.passed), "{v:?}");
    o.absorb_verdicts(MeasureSystemPurpose::SumCheck, v);
    (hub, o, s)
}

fn requests_since(hub: &StubHub, from: usize) -> Vec<String> {
    hub.0.lock().unwrap().requests[from..].to_vec()
}

/// The proposal names the guard and its length (P5TRUST's (c): the planned
/// verify, 25 % longer, plus 120 s, at most 590 s); the apply begins the
/// hub's guard before it files anything; Keep confirms it on the hub.
#[test]
fn wp_atguard_a_guarded_apply_begins_the_hubs_guard_and_keep_confirms_it() {
    let (mut hub, mut o, _) = atguard_checked(true);
    let planned = o.verify_planned_ms();
    // Four outputs ramped again (each from its start to its ceiling) and
    // the transitions' summed runs: tens of seconds, not nothing.
    assert!(planned > 20_000 && planned < 400_000, "{planned} ms");
    let want = ((planned * 125).div_ceil(100_000) + 120).min(590) as u32;
    let p = o.proposal_for("admin", true);
    assert!(p.offered, "{p:?}");
    assert_eq!(p.guard_s, want);
    assert!(p.guard_s >= 120 && p.guard_s <= 590);
    assert!(
        p.guard_note.contains(&format!("within {want} s")),
        "{}",
        p.guard_note
    );
    let from = hub.0.lock().unwrap().requests.len();
    let applied = o.apply(&mut hub, p.id).unwrap();
    let sent = requests_since(&hub, from);
    let begin = sent.iter().position(|r| r == msg::guard::BEGIN).unwrap();
    let get = sent.iter().position(|r| r == "system.get").unwrap();
    let set = sent.iter().position(|r| r == "system.set").unwrap();
    assert!(begin < get && get < set, "{sent:?}");
    assert_eq!(applied.guard_s, want);
    assert!(
        applied.message.contains("reverts it by itself"),
        "{}",
        applied.message
    );
    {
        let st = hub.0.lock().unwrap();
        let g = st.guard.as_ref().unwrap();
        assert_eq!(g.3, u64::from(want) * 1000, "the countdown asked for");
        assert_eq!(g.2, 7, "the revision filed");
    }
    // The verify runs inside the guard.
    assert!(o.guard_alive(&mut hub).is_ok());
    let v = sum_session(&o, &mut hub, MeasureSystemPurpose::Verify);
    o.absorb_verdicts(MeasureSystemPurpose::Verify, v);
    assert_eq!(o.applied.verified, Some(true), "{}", o.applied.message);
    // Keep, before the deadline: confirmed on the hub, the delays stay.
    let k = o.keep(&mut hub).unwrap();
    assert_eq!(k.stage, MeasureSystemApplyStage::Kept);
    let st = hub.0.lock().unwrap();
    assert!(st.guard.is_none());
    let last = st.guard_last.as_ref().unwrap();
    assert_eq!(
        (last.reason.as_str(), last.result.as_str()),
        ("confirmed", "kept")
    );
    assert!((st.units[0].out_delay_ms[0] - 1.1).abs() < 0.005);
}

/// Revert ends the guard: the hub sets the filed model back and aligns
/// (the console sends no `system.set` of its own), and the delays are read
/// back as filed.
#[test]
fn wp_atguard_revert_ends_the_guard_through_the_hub() {
    let (mut hub, mut o, _) = atguard_checked(true);
    let p = o.proposal_for("admin", true);
    o.apply(&mut hub, p.id).unwrap();
    let from = hub.0.lock().unwrap().requests.len();
    let r = o.revert(&mut hub).unwrap();
    assert_eq!(r.stage, MeasureSystemApplyStage::Reverted);
    assert!(r.message.contains("as filed"), "{}", r.message);
    let sent = requests_since(&hub, from);
    assert_eq!(
        sent.first().map(String::as_str),
        Some(msg::guard::CANCEL),
        "{sent:?}"
    );
    // The two after the cancel are the stub's own revert (the hub's).
    assert_eq!(
        sent.iter().filter(|x| *x == "system.set").count(),
        1,
        "only the hub's: {sent:?}"
    );
    let st = hub.0.lock().unwrap();
    assert!(st.guard.is_none());
    assert_eq!(st.guard_last.as_ref().unwrap().reason, "cancelled");
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
    assert_eq!(r.revision_after, st.revision);
}

/// The hub reverted by itself (its deadline passed, or the connection was
/// lost and the console came back): the verify is refused, Keep says what
/// the hub did, and the stage is `Reverted`.
#[test]
fn wp_atguard_a_guard_the_hub_ended_is_shown_as_reverted() {
    let (mut hub, mut o, _) = atguard_checked(true);
    let p = o.proposal_for("admin", true);
    o.apply(&mut hub, p.id).unwrap();
    hub.0.lock().unwrap().guard_revert("timeout");
    let e = o.guard_alive(&mut hub).unwrap_err().to_string();
    assert!(e.contains("timeout") && e.contains("reverted"), "{e}");
    assert_eq!(o.applied.stage, MeasureSystemApplyStage::Reverted);
    assert!(o.keep(&mut hub).is_err(), "nothing is applied any more");
    // Keep itself learns it too.
    let (mut hub, mut o, _) = atguard_checked(true);
    let p = o.proposal_for("admin", true);
    o.apply(&mut hub, p.id).unwrap();
    hub.0.lock().unwrap().guard_revert("session_closed");
    let e = o.keep(&mut hub).unwrap_err().to_string();
    assert!(e.contains("session_closed"), "{e}");
    assert_eq!(o.applied.stage, MeasureSystemApplyStage::Reverted);
    assert!(
        hub.0
            .lock()
            .unwrap()
            .units
            .iter()
            .all(|u| u.out_delay_ms.iter().all(|d| *d == 0.0))
    );
}

// -- WP RCFIX -- (review RC, RC1)
/// An agent never keeps an alignment whose verify failed, and does not spend
/// the user's second Keep.
#[test]
fn wp_rcfix_an_agent_never_keeps_an_alignment_whose_verify_failed() {
    let (mut hub, mut o, _) = atguard_checked(true);
    let p = o.proposal_for("admin", true);
    o.apply(&mut hub, p.id).unwrap();
    o.applied.verified = Some(false);
    let core = crate::DspiCore::new();
    core.measure_system_set_hub(Box::new(hub.clone()));
    core.system_slot().outcome = Some(o);
    let agent = crate::actor::register_actor("RCFIX agent");
    let agent_keeps = || {
        let _scope = crate::actor::ActorScope::enter(&core, agent, None);
        for _ in 0..2 {
            let e = core.measure_system_keep().unwrap_err().to_string();
            assert!(e.contains("only the user"), "{e}");
            crate::guarded::skew::advance(Duration::from_millis(1_100));
        }
        assert_eq!(
            core.measure_system_applied().stage,
            MeasureSystemApplyStage::Applied
        );
    };
    agent_keeps();
    assert!(core.measure_system_keep().is_err(), "the user's first");
    agent_keeps();
    let kept = core.measure_system_keep().unwrap();
    assert_eq!(kept.stage, MeasureSystemApplyStage::Kept);
}

/// An older hub without the cap: today's apply (no `system.guard.*` sent),
/// and the proposal and the applied record say nothing reverts by itself.
#[test]
fn wp_atguard_an_old_hub_offers_no_guard_and_says_so() {
    let (mut hub, o, _) = atguard_checked(false);
    let core = crate::DspiCore::new();
    core.measure_system_set_hub(Box::new(hub.clone()));
    core.system_slot().outcome = Some(o);
    let p = core.measure_system_proposal().unwrap();
    assert!(p.offered, "{p:?}");
    assert_eq!(p.guard_s, 0);
    assert!(
        p.guard_note.contains("nothing reverts by itself"),
        "{}",
        p.guard_note
    );
    let from = hub.0.lock().unwrap().requests.len();
    let applied = core.measure_system_apply(p.id).unwrap();
    assert_eq!(applied.guard_s, 0);
    assert!(
        applied.message.contains("nothing reverts by itself"),
        "{}",
        applied.message
    );
    let kept = core.measure_system_keep().unwrap();
    assert_eq!(kept.stage, MeasureSystemApplyStage::Kept);
    let sent = requests_since(&hub, from);
    assert!(
        !sent.iter().any(|r| r.starts_with("system.guard.")),
        "{sent:?}"
    );
    // And with the cap, the core's own path names the guard.
    let (hub2, o, _) = atguard_checked(true);
    let core = crate::DspiCore::new();
    core.measure_system_set_hub(Box::new(hub2.clone()));
    core.system_slot().outcome = Some(o);
    let p = core.measure_system_proposal().unwrap();
    assert!(p.guard_s >= 120, "{p:?}");
    let _ = &mut hub;
}

// ── review AT-D2 ────────────────────────────────────────────────────────

/// While an applied alignment waits for Keep or Revert no new measurement
/// starts (it would replace what was filed and the guard's id): nothing is
/// sent. After the Revert a start is judged as before.
#[test]
fn review_atd2_no_new_measurement_while_an_applied_alignment_waits() {
    let (hub, o, setup) = atguard_checked(true);
    let core = crate::DspiCore::new();
    core.measure_system_set_hub(Box::new(hub.clone()));
    core.system_slot().outcome = Some(o);
    let p = core.measure_system_proposal().unwrap();
    core.measure_system_apply(p.id).unwrap();
    let from = hub.0.lock().unwrap().requests.len();
    let e = core
        .measure_system_start(setup.clone(), Vec::new())
        .unwrap_err()
        .to_string();
    assert!(e.contains("waits for Keep or Revert"), "{e}");
    assert!(requests_since(&hub, from).is_empty(), "nothing sent");
    assert_eq!(
        core.measure_system_applied().stage,
        MeasureSystemApplyStage::Applied
    );
    assert!(
        hub.0.lock().unwrap().guard.is_some(),
        "the guard still runs"
    );
    core.measure_system_revert().unwrap();
    let e = core
        .measure_system_start(setup, Vec::new())
        .map(|_| String::new())
        .unwrap_or_else(|e| e.to_string());
    assert!(!e.contains("waits for Keep or Revert"), "{e}");
}

/// A guard the hub ended without putting everything back is never shown as
/// a clean revert: the words say so first. Keep leaves its own message.
#[test]
fn review_atd2_a_guard_ended_not_written_says_so_and_keep_has_its_own_words() {
    let (mut hub, mut o, _) = atguard_checked(true);
    let p = o.proposal_for("admin", true);
    o.apply(&mut hub, p.id).unwrap();
    {
        let mut st = hub.0.lock().unwrap();
        let (id, _, filed, _) = st.guard.take().unwrap();
        st.guard_last = Some(msg::guard::Outcome {
            guard: id,
            reason: "timeout".into(),
            result: "not_written".into(),
            revision_filed: filed,
            message: "nothing was set back: the model changed".into(),
            ..msg::guard::Outcome::default()
        });
    }
    let e = o.keep(&mut hub).unwrap_err().to_string();
    assert!(
        e.contains("NOT wholly put back (not_written)") && e.contains("the model changed"),
        "{e}"
    );
    assert_eq!(
        o.applied.message,
        e.trim_start_matches("invalid argument: ")
    );
    // Keep's own words replace the apply's "... unless it is kept within N s".
    let (mut hub, mut o, _) = atguard_checked(true);
    let p = o.proposal_for("admin", true);
    o.apply(&mut hub, p.id).unwrap();
    let k = o.keep(&mut hub).unwrap();
    assert_eq!(k.stage, MeasureSystemApplyStage::Kept);
    assert_eq!(k.message, "kept: the alignment stays");
}

// ── WP ATDEMO ───────────────────────────────────────────────────────────

/// The hub answered `status` with `aborted` (it stopped the session
/// itself): the core reports why by the hub's words, the unit named, never
/// "the hub does not answer".
#[test]
fn wp_atdemo_a_status_the_hub_aborted_says_why_not_disconnected() {
    let cases = [
        (
            "unit B: its time base was disturbed (5 -> 6, last reanchor)",
            "measure.system.abort.disturbance",
            "B",
        ),
        (
            "unit B: its timeline moved against its outputs",
            "measure.system.abort.disturbance",
            "B",
        ),
        ("disturbance", "measure.system.abort.disturbance", ""),
        (
            "unit B: its time base is holdover",
            "measure.system.abort.time_base",
            "B",
        ),
        ("lease", "measure.system.abort.hub", ""),
    ];
    for (words, key, serial) in cases {
        let (sess, hub) = faulted(|f| f.status_aborted = Some((96_000, words.to_owned())));
        let st = sess.state();
        assert_eq!(st.phase, MeasureSystemPhase::Aborted, "{words}: {st:?}");
        let i = st.issue.unwrap();
        assert_eq!(i.key, key, "{words}: {i:?}");
        assert_eq!(i.serial, serial, "{words}: {i:?}");
        assert!(!i.message.contains("does not answer"), "{i:?}");
        if !serial.is_empty() {
            assert!(i.message.starts_with("B: "), "{i:?}");
        }
        let reqs = hub.0.lock().unwrap().requests.clone();
        assert_eq!(reqs.last().map(String::as_str), Some(msg::t::CLOSE));
    }
    // The hub's own event for a disturbance: the unit lost its lock too.
    let (sess, _) = faulted(|f| f.hub_abort = Some((1, 96_000)));
    assert_eq!(sess.state().issue.unwrap().key, "measure.system.abort.hub");
}

#[test]
fn wp_atdemo_a_hub_event_for_a_disturbance_is_the_unit_losing_its_lock() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(vec![
        unit("A", 1, 48, &[3.0]),
        unit("B", 1, 48, &[3.2]),
    ]))));
    let p = plan_of(&s, &mut hub);
    let mut sess = SysSession::open(
        p,
        MeasureSystemPurpose::Measure,
        None,
        &mut hub,
        Instant::now(),
    )
    .unwrap();
    hub.0.lock().unwrap().aborted.push(AbortedEvent {
        session: "s1".into(),
        unit: Some(1),
        reason: "disturbance".into(),
    });
    sess.tick(&mut hub, Instant::now());
    let i = sess.state().issue.unwrap();
    assert_eq!(i.key, "measure.system.abort.disturbance", "{i:?}");
    assert_eq!(i.serial, "B");
}

/// Drive a session from the simulated microphone only: the core reads the
/// hub's room on every tick (nothing pushed by a client).
fn drive_sim(s: &mut SysSession, hub: &mut StubHub, max_ticks: usize) {
    let mut sim = super::system::sim::SystemSim::default();
    let mut now = Instant::now();
    for _ in 0..max_ticks {
        if s.finished() {
            return;
        }
        hub.0.lock().unwrap().faults.demo_ready = 1;
        sim.feed(hub, s);
        now += Duration::from_millis(100);
        s.tick(hub, now);
    }
    panic!("the session did not end: {:?}", s.state());
}

/// The simulated microphone's capture measures as a client's would: a
/// result with every arrival offered, read in order from frame 0.
#[test]
fn wp_atdemo_the_simulated_microphone_feeds_the_session() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(vec![
        unit("A", 1, 48, &[3.0]),
        unit("B", 1, 48, &[3.6]),
    ]))));
    hub.0.lock().unwrap().faults.demo_mic = Some(u64::MAX);
    let p = plan_of(&s, &mut hub);
    let mut sess = SysSession::open(
        p,
        MeasureSystemPurpose::Measure,
        None,
        &mut hub,
        Instant::now(),
    )
    .unwrap();
    drive_sim(&mut sess, &mut hub, 4_000);
    let st = sess.state();
    assert_eq!(st.phase, MeasureSystemPhase::Done, "{st:?}");
    let Some(SessionOutput::Measured(o)) = sess.take_outcome() else {
        panic!("a result")
    };
    assert!(
        o.result.arrivals.iter().all(|a| a.offered),
        "{:?}",
        o.result
    );
    let a = arrival(&o, "A", 0).arrival_us;
    assert!((a + 600.0).abs() < 2.0, "A arrives 600 us before B: {a}");
}

/// The room refusing during a session (a unit that is not the demo's)
/// aborts it at once, the abort sent.
#[test]
fn wp_atdemo_a_refused_simulated_microphone_aborts_the_session() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(vec![
        unit("A", 1, 48, &[3.0]),
        unit("B", 1, 48, &[3.6]),
    ]))));
    hub.0.lock().unwrap().faults.demo_mic = Some(48_000);
    let p = plan_of(&s, &mut hub);
    let mut sess = SysSession::open(
        p,
        MeasureSystemPurpose::Measure,
        None,
        &mut hub,
        Instant::now(),
    )
    .unwrap();
    drive_sim(&mut sess, &mut hub, 4_000);
    let i = sess.state().issue.unwrap();
    assert_eq!(i.key, super::system::sim::KEY_REFUSED, "{i:?}");
    assert!(i.message.contains("not a demo unit"), "{i:?}");
    let reqs = hub.0.lock().unwrap().requests.clone();
    assert!(reqs.iter().any(|r| r == msg::t::ABORT), "abort sent");
}

/// Fail closed through the core's calls: no simulated microphone on a hub
/// without the demo room; with it, a group with any unit the room does not
/// hear does not start (nothing opened, nothing staged), and a client's
/// pushes are ignored while it is on.
#[test]
fn wp_atdemo_the_simulated_microphone_is_never_offered_beyond_the_demo_units() {
    let core = crate::DspiCore::new();
    let hub = two_units();
    core.measure_system_set_hub(Box::new(hub.clone()));
    // A hub without the room (any hub but the demo's).
    let info = core.measure_system_sim_info();
    assert!(!info.available && !info.on, "{info:?}");
    assert!(matches!(
        core.measure_system_sim_set(true),
        Err(DspiError::Link { ref code, .. }) if code == "unsupported"
    ));
    assert!(!core.measure_system_sim_info().on);
    // The demo hub's room: on, labelled.
    hub.0.lock().unwrap().caps.push(msg::demo::CAP.into());
    hub.0.lock().unwrap().faults.demo_mic = Some(u64::MAX);
    let info = core.measure_system_sim_set(true).unwrap();
    assert!(info.available && info.on, "{info:?}");
    assert_eq!(info.label, crate::SIM_MIC_LABEL);
    // A real unit in the group: refused before anything is opened.
    let s = setup(&[("A", &[0]), ("REAL", &[0])]);
    let checks = vec![check(&s, "A", Vec::new()), check(&s, "REAL", Vec::new())];
    let r = core.measure_system_start(s, checks);
    assert!(
        matches!(r, Err(DspiError::Link { ref code, ref msg }) if code == "unsupported" && msg.contains("REAL")),
        "{r:?}"
    );
    let reqs = hub.0.lock().unwrap().requests.clone();
    assert!(reqs.iter().any(|r| r == msg::demo::T), "{reqs:?}");
    assert!(
        !reqs.iter().any(|r| r == msg::t::OPEN || r == msg::t::STAGE),
        "nothing opened or staged: {reqs:?}"
    );
    assert!(core.measure_system_sim_info().issue.is_some());
    // The hub loses the room (another hub on this connection): off.
    hub.0.lock().unwrap().caps.retain(|c| c != msg::demo::CAP);
    assert!(!core.measure_system_sim_info().on);
    // Off: turned off always works.
    assert!(!core.measure_system_sim_set(false).unwrap().on);
}

/// `system.status` rate limited when the engine's terms are read (another
/// view of the connection read it just before): read again during the run,
/// every arrival still offered.
#[test]
fn wp_atdemo_a_rate_limited_system_status_is_read_again() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(vec![
        unit("A", 1, 48, &[3.0]),
        unit("B", 1, 48, &[3.6]),
    ]))));
    hub.0.lock().unwrap().faults.status_rate_limited = 2;
    let (sess, out) = measure(&mut hub, &s);
    assert_eq!(sess.state().phase, MeasureSystemPhase::Done);
    let o = out.unwrap();
    assert!(
        o.result.arrivals.iter().all(|a| a.offered),
        "{:?}",
        o.result
    );
    let n = hub
        .0
        .lock()
        .unwrap()
        .requests
        .iter()
        .filter(|r| *r == "system.status")
        .count();
    assert_eq!(n, 3, "read again until answered");
}

/// After an apply the engine's terms are read even when `system.status` is
/// rate limited at first (the verify's prediction needs them).
#[test]
fn wp_atdemo_system_status_after_an_apply_waits_out_the_rate_limit() {
    let mut hub = two_units();
    hub.0.lock().unwrap().faults.status_rate_limited = 1;
    let v = super::system::proposal::system_status(&mut hub);
    assert!(v.is_some_and(|v| v.get("alignment").is_some()));
    assert_eq!(hub.0.lock().unwrap().requests.len(), 2);
}

// -- WP ATLATE --
/// The planner's expected lateness (review AT-D1, "For the coordinator"): A
/// output 0 heard 100 ms and 150 ms after B's through its `out.delay` (a
/// USB-direct unit's stream start offset is there, some 21 ms a unit). The
/// planner reads the engine's terms before the run, starts the parts with
/// room for it and the analysis looks for A that much later: both arrivals
/// are measured whole and offered (before: 100 ms was cut and not offered,
/// 150 ms "ambiguous" or not found), the acoustic arrivals within 3 us.
#[test]
fn wp_atlate_an_output_delay_past_the_analysed_span_is_planned_for() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    for gap in [100.0, 150.0, 400.0] {
        let mut hub = pair_apart(gap);
        let (sess, out) = measure(&mut hub, &s);
        assert_eq!(
            sess.state().phase,
            MeasureSystemPhase::Done,
            "{gap} ms: {:?}",
            sess.state().issue
        );
        let o = out.unwrap();
        for a in &o.result.arrivals {
            // Both 3.0 ms acoustic: the latest's at 0, so both at 0.
            assert!(a.offered, "{gap} ms: {a:?}");
            assert!(a.arrival_us.abs() < 3.0, "{gap} ms: {a:?}");
        }
        let (a, b) = (arrival(&o, "A", 0), arrival(&o, "B", 0));
        assert!(
            (a.measured_us - b.measured_us - gap * 1000.0).abs() < 3.0,
            "{gap} ms: {a:?} {b:?}"
        );
        assert!(o.result.delays.iter().all(|d| d.offered), "{gap} ms");
        assert!(
            !o.result
                .reasons
                .iter()
                .any(|r| r.contains("too long after")),
            "{gap} ms: {:?}",
            o.result.reasons
        );
    }
}

/// The run waits for the engine's terms while `system.status` is rate
/// limited (another view read it just before), so a late unit is planned for
/// (before, the run started at once and the terms were read during it).
#[test]
fn wp_atlate_the_run_waits_for_the_terms_it_plans_with() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let mut hub = pair_apart(150.0);
    hub.0.lock().unwrap().faults.status_rate_limited = 2;
    let (sess, out) = measure(&mut hub, &s);
    assert_eq!(sess.state().phase, MeasureSystemPhase::Done);
    let o = out.unwrap();
    assert!(
        o.result.arrivals.iter().all(|a| a.offered),
        "{:?}",
        o.result
    );
    // Every read of the terms before the walked run (the last RUN; the
    // ramps' steps are runs too): none during it is needed.
    let reqs = hub.0.lock().unwrap().requests.clone();
    let run = reqs.iter().rposition(|r| r == msg::t::RUN).unwrap();
    let statuses: Vec<usize> = reqs
        .iter()
        .enumerate()
        .filter(|(_, r)| *r == "system.status")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(statuses.len(), 3, "read again until answered: {reqs:?}");
    assert!(statuses.iter().all(|&i| i < run), "{reqs:?}");
}

/// Fail closed: a unit the terms put more than a run plans for (1 s) after
/// the others is a model or alignment fault: the plan of the run is refused,
/// the unit named (the session then fails `measure.system.plan` before it
/// stages anything). Within it, the run is planned with room for it.
#[test]
fn wp_atlate_a_lateness_past_what_a_run_plans_for_is_refused_before_it_plays() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let mut hub = pair_apart(2.0);
    let p = plan_of(&s, &mut hub);
    let part = p.runs[0].clone();
    let terms = |a_us: f64| {
        vec![
            Some(proposal::Terms {
                playout_us: a_us,
                out_delay_ms: vec![(0, 0.0)],
            }),
            Some(proposal::Terms {
                playout_us: 2_500.0,
                out_delay_ms: vec![(0, 0.0)],
            }),
        ]
    };
    // A: 1.2 s of play-out (and B 2.5 ms).
    let late = p.lateness(&part, &terms(1_200_000.0));
    assert_eq!(late, [(UnitIndex(0), 57_600), (UnitIndex(1), 120)]);
    let e = p
        .schedule_late(&part, plan::PLANNING_LEAD_MS, &late)
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("A is expected") && e.contains("more than a measurement plans for"),
        "{e}"
    );
    // 150 ms: planned, B's segments started that much later after A's.
    let late = p.lateness(&part, &terms(152_500.0));
    let sched = p
        .schedule_late(&part, plan::PLANNING_LEAD_MS, &late)
        .unwrap();
    let plain = p.schedule(&part, plan::PLANNING_LEAD_MS).unwrap();
    assert_eq!(sched.lateness_of(UnitIndex(0)), 7_200);
    assert_eq!(sched.lateness_of(UnitIndex(1)), 0);
    let (seg, was) = (sched.segments(), plain.segments());
    assert_eq!(
        seg[1].start - seg[0].start,
        was[1].start - was[0].start + 7_200
    );
    // No terms: as before this WP.
    assert!(p.lateness(&part, &[None, None]).is_empty());
}

// -- review AT-E --
/// The lateness is planned from the engine's terms, so a wrong term moves
/// where the analysis looks. It must never make a wrong arrival offered:
/// with either unit's `out.delay` reported 60 ms to 900 ms larger than what
/// it plays with, the run either fails, or offers nothing, or its measured
/// arrivals (on the timeline, before any term is taken off) are right.
/// 60 ms is inside the analysed span: found where it is, so the test is seen
/// to measure; from 100 ms on the AT-D1 guard or the cycle search refuses;
/// between them (the guard's 15 ms) either, never a wrong arrival offered.
#[test]
fn review_ate_a_lateness_planned_from_a_wrong_term_never_offers_a_wrong_arrival() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    for which in [0usize, 1] {
        for lie in [60.0f32, 86.0, 93.0, 100.0, 150.0, 400.0, 900.0] {
            // A is 2 ms of air further than B.
            let mut hub = pair_apart(2.0);
            hub.0.lock().unwrap().faults.term_lie_ms = Some((which, lie));
            let (sess, out) = measure(&mut hub, &s);
            let Some(o) = out else {
                let st = sess.state();
                eprintln!("unit {which} +{lie} ms: no result: {:?}", st.issue);
                assert_ne!(st.phase, MeasureSystemPhase::Done, "unit {which} +{lie} ms");
                assert!(lie >= 100.0, "unit {which} +{lie} ms: {:?}", st.issue);
                continue;
            };
            let (a, b) = (arrival(&o, "A", 0), arrival(&o, "B", 0));
            let right = (a.measured_us - b.measured_us - 2_000.0).abs() < 3.0;
            let offered = o.result.arrivals.iter().any(|x| x.offered)
                || o.result.delays.iter().any(|d| d.offered);
            eprintln!(
                "unit {which} +{lie} ms: offered {offered}, right {right}: A {:.1} us, B {:.1} us, {:?}",
                a.measured_us, b.measured_us, o.result.reasons
            );
            assert!(right || !offered, "unit {which} +{lie} ms: {a:?} {b:?}");
            if lie <= 60.0 {
                assert!(right && offered, "unit {which} +{lie} ms: {a:?} {b:?}");
            } else if lie >= 100.0 {
                // Found at the span's end or not at all: nothing is offered,
                // right or not.
                assert!(!offered, "unit {which} +{lie} ms: {:?}", o.result);
                let p = o.proposal("admin");
                assert!(!p.offered && p.arrivals.is_empty(), "{p:?}");
            }
        }
    }
}

/// The run's **last** unit the late one, by 400 ms and 900 ms (WP ATLATE's
/// tests made the first one late, by up to 400 ms): both arrivals are
/// offered and right, the capture kept its lateness longer, and the summed
/// check judges them. Before this review the summed check of the pair
/// 900 ms apart failed "the first cycle was not heard in the window the
/// START implies" (its earlier part starts 900 ms before the sum is heard,
/// the search window ends 500 ms after it), so such a proposal could never
/// be checked or applied.
#[test]
fn review_ate_the_runs_last_unit_late_is_captured_whole() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    for gap in [400.0f32, 900.0] {
        let (a, mut b) = (unit("A", 1, 48, &[3.0]), unit("B", 1, 48, &[3.0]));
        b.out_delay_ms = vec![gap];
        let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(vec![a, b]))));
        let (sess, out) = measure(&mut hub, &s);
        assert_eq!(
            sess.state().phase,
            MeasureSystemPhase::Done,
            "{gap} ms: {:?}",
            sess.state().issue
        );
        let o = out.unwrap();
        let (a, b) = (arrival(&o, "A", 0), arrival(&o, "B", 0));
        assert!(a.offered && b.offered, "{gap} ms: {a:?} {b:?}");
        assert!(
            (b.measured_us - a.measured_us - f64::from(gap) * 1000.0).abs() < 3.0,
            "{gap} ms: {a:?} {b:?}"
        );
        assert!(a.arrival_us.abs() < 3.0 && b.arrival_us.abs() < 3.0);
        // The summed check on responses taken with a planned lateness (the
        // response is on the analysis' timeline, `own_ms`): right passes, a
        // delay 50 us wrong fails as a judged failure.
        let (pass, rms) = sum_check_cell(&o, &mut hub, 0.0, None);
        assert!(pass, "{gap} ms, right: {rms:?}");
        let (pass, rms) = sum_check_cell(&o, &mut hub, 50.0, None);
        assert!(!pass && rms.is_some(), "{gap} ms, 50 us wrong: {rms:?}");
    }
}

// ── WP REVERTW ──────────────────────────────────────────────────────────

/// The hub ended the guard and put nothing back (`not_written`) or not all
/// (`revert_failed`): the stage is `RevertIncomplete` with its notice, never
/// `Reverted`; a clean one has none.
#[test]
fn wp_revertw_a_guard_the_hub_ended_not_wholly_has_its_own_stage() {
    for (result, failures, key) in [
        (
            "not_written",
            vec![],
            crate::guarded::NOTICE_REVERT_NOT_WRITTEN,
        ),
        (
            "revert_failed",
            vec!["UNIT-2 delays: refused".to_owned()],
            crate::guarded::NOTICE_REVERT_FAILED,
        ),
    ] {
        let (mut hub, mut o, _) = atguard_checked(true);
        let p = o.proposal_for("admin", true);
        o.apply(&mut hub, p.id).unwrap();
        {
            let mut st = hub.0.lock().unwrap();
            let (id, _, filed, _) = st.guard.take().unwrap();
            st.guard_last = Some(msg::guard::Outcome {
                guard: id,
                reason: "timeout".into(),
                result: result.into(),
                revision_filed: filed,
                write_failures: failures,
                message: "not all set back".into(),
                ..msg::guard::Outcome::default()
            });
        }
        assert!(o.guard_alive(&mut hub).is_err());
        assert_eq!(
            o.applied.stage,
            MeasureSystemApplyStage::RevertIncomplete,
            "{result}"
        );
        assert_eq!(o.applied.revert_notice, key, "{result}");
        assert!(o.keep(&mut hub).is_err(), "nothing waits any more");
    }
    let (mut hub, mut o, _) = atguard_checked(true);
    let p = o.proposal_for("admin", true);
    o.apply(&mut hub, p.id).unwrap();
    hub.0.lock().unwrap().guard_revert("timeout");
    assert!(o.guard_alive(&mut hub).is_err());
    assert_eq!(o.applied.stage, MeasureSystemApplyStage::Reverted);
    assert!(o.applied.revert_notice.is_empty());
    assert_eq!(
        serde_json::to_value(MeasureSystemApplyStage::RevertIncomplete).unwrap(),
        json!("revert_incomplete")
    );
}

/// The user's Revert: a guard the hub ended with nothing set back is
/// refused and says so (`revert.not_written`); one whose writes failed is
/// `RevertIncomplete` (`revert.incomplete`) with the failed writes named.
#[test]
fn wp_revertw_a_revert_whose_writes_fail_is_incomplete() {
    let (mut hub, mut o, _) = atguard_checked(true);
    let p = o.proposal_for("admin", true);
    o.apply(&mut hub, p.id).unwrap();
    {
        let mut st = hub.0.lock().unwrap();
        let (id, _, filed, _) = st.guard.take().unwrap();
        st.guard_last = Some(msg::guard::Outcome {
            guard: id,
            reason: "session_closed".into(),
            result: "not_written".into(),
            revision_filed: filed,
            message: "nothing was set back: the model changed".into(),
            ..msg::guard::Outcome::default()
        });
    }
    let e = o.revert(&mut hub).unwrap_err().to_string();
    assert!(
        e.contains("not reverted") && e.contains("the model changed"),
        "{e}"
    );
    assert_eq!(o.applied.stage, MeasureSystemApplyStage::RevertIncomplete);
    assert_eq!(
        o.applied.revert_notice,
        crate::guarded::NOTICE_REVERT_NOT_WRITTEN
    );
    // The hub ended it with a unit's write failed: the revert reads the
    // rest back and names the failure.
    let (mut hub, mut o, _) = atguard_checked(true);
    let p = o.proposal_for("admin", true);
    o.apply(&mut hub, p.id).unwrap();
    {
        let mut st = hub.0.lock().unwrap();
        let mut out = st.guard_revert("timeout");
        out.result = "revert_failed".into();
        out.write_failures = vec!["UNIT-2 out.delay: refused".into()];
        st.guard_last = Some(out);
    }
    let r = o.revert(&mut hub).unwrap();
    assert_eq!(r.stage, MeasureSystemApplyStage::RevertIncomplete, "{r:?}");
    assert_eq!(r.revert_notice, crate::guarded::NOTICE_REVERT_INCOMPLETE);
    assert!(
        r.message.contains("UNIT-2 out.delay: refused") && r.message.contains("not wholly"),
        "{}",
        r.message
    );
}

// -- WP ATOVL -- overlapped starts (owner 2026-11-19).

/// Two units: overlapped starts by default, the run asks the hub for an
/// overlapped schedule, the arrivals are offered, and the result says which
/// plan it used.
#[test]
fn wp_atovl_two_units_start_overlapped_by_default_and_the_result_says_so() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let mut hub = pair_apart(2.0);
    let (_, out) = measure(&mut hub, &s);
    let o = out.expect("an outcome");
    assert_eq!(
        o.result.plan, "overlapped starts, 50 ms apart",
        "{:?}",
        o.result
    );
    assert!(o.result.timing_reliable, "{:?}", o.result.reasons);
    assert!(
        o.result.arrivals.iter().all(|a| a.offered),
        "{:?}",
        o.result.arrivals
    );
    let a = arrival(&o, "A", 0).arrival_us - arrival(&o, "B", 0).arrival_us;
    assert!((a - 2_000.0).abs() < 2.0, "{a}");
}

/// Two outputs a unit: what played together is offered, and so are two
/// outputs of one unit with a steady microphone (the absolute gate passes).
#[test]
fn wp_atovl_two_outputs_a_unit_overlapped_offer_every_arrival() {
    let s = setup(&[("A", &[0, 1]), ("B", &[0, 1])]);
    let mut hub = two_units();
    let (_, out) = measure(&mut hub, &s);
    let o = out.expect("an outcome");
    assert_eq!(o.result.plan, "overlapped starts, 50 ms apart");
    assert!(
        o.result.arrivals.iter().all(|a| a.offered),
        "{:?} {:?}",
        o.result.reasons,
        o.result.arrivals
    );
}

/// Three units of a 1 s sweep cannot start overlapped (a harmonic image of
/// the third would fall on the first's arrival): interleaved, saying why.
#[test]
fn wp_atovl_three_units_of_a_short_sweep_interleave_and_say_why() {
    let s = setup(&[("A", &[0]), ("B", &[0]), ("C", &[0])]);
    let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(vec![
        unit("A", 1, 48, &[3.0]),
        unit("B", 1, 48, &[3.5]),
        unit("C", 1, 48, &[4.0]),
    ]))));
    let (_, out) = measure(&mut hub, &s);
    let o = out.expect("an outcome");
    assert!(
        o.result
            .plan
            .starts_with("interleaved passes (overlapped starts do not fit"),
        "{}",
        o.result.plan
    );
    assert!(o.result.plan.contains("harmonic"), "{}", o.result.plan);
    assert!(
        o.result.arrivals.iter().all(|a| a.offered),
        "{:?}",
        o.result.arrivals
    );
}

/// A unit heard 30 ms further than the plan knows (its path, not in its
/// terms): the overlapped run cannot tell the units apart, so the session
/// measures the same run again interleaved, and the result says why.
#[test]
fn wp_atovl_units_not_separable_are_measured_again_interleaved() {
    let s = setup(&[("A", &[0]), ("B", &[0])]);
    let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(vec![
        unit("A", 1, 48, &[33.0]),
        unit("B", 1, 48, &[3.0]),
    ]))));
    let (_, out) = measure(&mut hub, &s);
    let o = out.expect("an outcome");
    assert!(
        o.result
            .plan
            .starts_with("interleaved passes (overlapped starts could not tell the units apart"),
        "{}",
        o.result.plan
    );
    let a = arrival(&o, "A", 0).arrival_us - arrival(&o, "B", 0).arrival_us;
    assert!((a - 30_000.0).abs() < 2.0, "{a}");
    assert!(
        o.result.arrivals.iter().all(|a| a.offered),
        "{:?}",
        o.result.arrivals
    );
}

/// Review AT-E 2: the runs are sized with the lateness the plan expects
/// (each unit's smallest `out.delay`, from its own check), so a group that
/// fits one capture without it but not with it is split through the anchor
/// at the plan, and every run then fits with that lateness; before, the one
/// run was refused only when it was laid out with the terms.
#[test]
fn wp_atovl_split_sizes_the_runs_with_the_expected_lateness() {
    let names: Vec<String> = (0..16).map(|i| format!("U{i:02}")).collect();
    let outs: &[u8] = &[0];
    let units: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), outs)).collect();
    let mut s = setup(&units);
    s.passes = 5;
    let mut hub = StubHub(Arc::new(Mutex::new(Stub::new(
        names.iter().map(|n| unit(n, 1, 48, &[3.0])).collect(),
    ))));
    // Every other unit's output delayed 0.9 s (a model's out.delay).
    let checks: Vec<_> = s
        .units
        .iter()
        .enumerate()
        .map(|(i, u)| {
            let mut c = check(&s, &u.serial, Vec::new());
            let mut ctx: UnitContext = serde_json::from_str(&c.context).unwrap();
            if i % 2 == 1 {
                for (_, ch) in &mut ctx.chains {
                    ch.delay_ms = 900.0;
                }
            }
            c.context = serde_json::to_string(&ctx).unwrap();
            c
        })
        .collect();
    let (pre, plan_units) = plan::preflight(&s, &checks, &mut hub).unwrap();
    assert!(pre.clear, "{pre:?}");
    let p = SetupPlan::new(&s, plan_units).unwrap();
    let whole = p.runs.iter().map(|r| r.units.len()).sum::<usize>();
    assert!(p.runs.len() >= 2, "split: {:?}", p.runs);
    assert!(whole >= 16);
    for r in &p.runs {
        let late = p.expected_lateness(r);
        assert!(!late.is_empty());
        p.schedule_late(r, plan::PLANNING_LEAD_MS, &late)
            .unwrap_or_else(|e| panic!("{r:?}: {e}"));
    }
    // Without the delays the whole group is one run.
    let plain = plan_of(&s, &mut hub);
    assert_eq!(plain.runs.len(), 1, "{:?}", plain.runs);
    let late = plain.expected_lateness(&plain.runs[0]);
    assert!(late.is_empty(), "the flat chains: {late:?}");
}
