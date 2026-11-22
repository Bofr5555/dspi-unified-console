//! A system measurement's outcome (WP ATCORE, PLAN-AT 1.4, 1.5, OD-ATl): the
//! responses and documents per unit, the arrivals with their uncertainty,
//! the one "may propose" rule, the inter-unit transitions and their summed
//! check (RC5f), the apply into the hub's model, the verify, Keep and Revert.
//!
//! **May propose** ([`may_propose`]): an arrival is offered only when its
//! run passed the gate G1 to G4 (`check_gate` empty) **and** its delay to
//! the group's latest output is offered: within one run that is the
//! analyser's [`CrossUnitDelay::offered`] (two outputs on different units)
//! or the run's G5 (two outputs of one unit); across chained runs it is both
//! outputs' [`ChainedOutput::offered`] against the anchor **and** the pair's
//! own uncertainty (the two against the anchor in quadrature) within 10 us:
//! two outputs each offered against the anchor at 10 us may differ by 14 us
//! (review AT-A/AT-B).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use dspi_measure::ir::FIT_WINDOW_CYCLES;
use dspi_measure::response::FLOOR_DB;
use dspi_measure::system::chain::{ChainOptions, ChainRun, ChainedOutput, chain_runs};
use dspi_measure::system::{
    CrossUnitDelay, GateFinding, MAX_OFFERED_UNCERTAINTY_US, SystemAnalysis, SystemSchedule,
    UnitIndex, UnitOutput, max_sum_spread_samples, transition_band,
};
use dspi_measure::types::Response;
use dspi_measure::xover::SUMMED_CHECK_MAX_RMS_DB;

use super::super::runs::{self, DocContext, OutputResult, RunResult};
use super::plan::SetupPlan;
use super::{
    Hub, MeasureSystemApplied, MeasureSystemApplyStage, MeasureSystemArrival, MeasureSystemDelay,
    MeasureSystemDocument, MeasureSystemIssue, MeasureSystemOutDelay, MeasureSystemProposal,
    MeasureSystemPurpose, MeasureSystemResult, MeasureSystemSnapshot, MeasureSystemTransition,
};
use crate::{DspiError, Result};

/// The fewest trusted points a summed check compares.
const MIN_SUM_POINTS: usize = 12;
/// Each doubling of the outputs a summed run starts together lowers its
/// level by this, dB (S-AT4).
pub(crate) const SUM_DOUBLING_DB: f32 = 6.0;
/// Of the fit window (`cycles / f`, three quarters after the onset), the
/// part a later arrival must fall in to be judged: the first half of the
/// window's fall.
const SUM_WINDOW_REACH: f64 = 0.375;
// -- review AT-D1 --
/// The least an output's analysed response must hold after its onset, ms. A
/// run's responses are kept from `search_ms` before its first arrival to the
/// gap's rest after (100 ms each side with the 300 ms gap): an output heard
/// later than that after the run's first (a unit whose play-out and output
/// delay put it some 100 ms later on the timeline) has its arrival clamped
/// to the end of that span and its response cut, and was offered all the
/// same (measured: 100 ms apart read as 99.98 ms with an uncertainty of
/// 0.35 us; 80 ms apart is still exact to 0.01 us).
const MIN_ROOM_AFTER_ONSET_MS: f64 = 15.0;
/// A predicted sum this far below its loudest part is a cancellation, dB.
const CANCELLATION_DB: f64 = 6.0;
/// The most run ids written with a speaker (the hub's `MAX_MEASURED_RUNS`).
const MAX_MEASURED_RUNS: usize = 16;

fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

/// The one rule (review AT-A/AT-B): a run that failed G1 to G4 proposes
/// nothing, whatever its timing; a passed run proposes only what its timing
/// offers.
pub(crate) fn may_propose(gate: &[GateFinding], offered: bool) -> bool {
    gate.is_empty() && offered
}

/// What the alignment engine adds to a unit's arrivals (WP ATHUBM).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Terms {
    /// The unit's play-out of a domain sample, us.
    pub(crate) playout_us: f64,
    /// `(output, out.delay ms)` in effect during the run.
    pub(crate) out_delay_ms: Vec<(u8, f32)>,
}

impl Terms {
    // -- WP ATLATE --
    /// What the engine adds to `output`'s sound after a START, us (the
    /// play-out and its `out.delay`).
    pub(crate) fn at_us(&self, output: u8) -> Option<f64> {
        self.of(output)
    }

    fn of(&self, output: u8) -> Option<f64> {
        self.out_delay_ms
            .iter()
            .find(|(o, _)| *o == output)
            .map(|(_, d)| self.playout_us + f64::from(*d) * 1000.0)
    }
}

/// Where a unit's play-out delay D comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlayOut {
    /// The unit's own word: `EXT_TIME_BASE.d` of the group's entry as the
    /// hub read it at `system.measure.open` (contract C2: a follower plays
    /// the sample carried as T exactly D samples after it arrived; a source
    /// plays T as it emits it, D = 0), at the units' rate, Hz.
    Own {
        /// D, samples.
        d: u16,
        /// The units' rate, Hz.
        rate_hz: u32,
    },
    /// The hub's alignment (`effective`), read with no session open right
    /// after `system.align` wrote it; `source`: the unit is the source of
    /// the group's time base (it plays T at once; its lag is in its output
    /// delays).
    Engine {
        /// The unit is the source of the group's time base.
        source: bool,
    },
}

/// A unit's play-out of a sample of the group's time base, us: T reaches
/// the unit `pathDelayFrames` after the source emits it, the unit plays it
/// its D later, and it is heard `latencyFrames` after that. The path and the
/// latency are the model's, from `system.status`'s alignment
/// (`alignment.units[]` by serial); D is `from`.
///
/// Review AT-C: during a measurement D is the unit's own
/// ([`PlayOut::Own`]), never the alignment's `effective`. The session's
/// device locks keep the hub from reading the units, so the alignment it
/// reports then is computed without their limits and need not be what they
/// run (an S/PDIF follower read 192 where it ran 150: its speakers were
/// placed 875 us early against the source's). The unit's own D is also
/// right for a unit not yet aligned as the model says, and for a line or
/// USB-direct unit that is an S/PDIF source besides (R7b: D of the group's
/// entry, 0 only on the S/PDIF time base).
pub(crate) fn playout_us(status: &Value, serial: &str, from: PlayOut) -> Option<f64> {
    let a = status.get("alignment")?;
    let rate = a
        .get("rateHz")
        .or_else(|| a.get("rate_hz"))
        .and_then(Value::as_f64)
        .filter(|r| *r > 0.0)?;
    let u = a
        .get("units")?
        .as_array()?
        .iter()
        .find(|u| u.get("serial").and_then(Value::as_str) == Some(serial))?;
    let num = |k: &str, s: &str| {
        u.get(k)
            .or_else(|| u.get(s))
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
    };
    let latency_us = num("latencyFrames", "latency_frames") * 1e6 / rate;
    let path = num("pathDelayFrames", "path_delay_frames");
    let us = match from {
        PlayOut::Own { d, rate_hz } => {
            if rate_hz == 0 {
                return None;
            }
            (path + f64::from(d)) * 1e6 / f64::from(rate_hz) + latency_us
        }
        PlayOut::Engine { source: true } => latency_us,
        PlayOut::Engine { source: false } => {
            (path + num("effective", "effective")) * 1e6 / rate + latency_us
        }
    };
    us.is_finite().then_some(us)
}

// -- WP ATDEMO --
/// The hub's `system.status`, waited for when it is rate limited (the hub
/// answers it once a second per connection, and another view of this
/// connection may just have read it): after an apply the engine's terms
/// must be read, never taken as unreadable (a verify would then predict no
/// change of delay and fail a good alignment).
pub(crate) fn system_status(hub: &mut dyn Hub) -> Option<Value> {
    for attempt in 0..3 {
        match hub.request("system.status", json!({})) {
            Ok(v) => return Some(v),
            Err(DspiError::Link { code, .. }) if code == "rate_limited" && attempt < 2 => {
                dspi_transport::clock::sleep(std::time::Duration::from_millis(1_050));
            }
            Err(_) => return None,
        }
    }
    None
}

/// `(output, ms)` from a `param.get_many` reply of `out.delay` per output.
pub(crate) fn out_delays_of(reply: &Value, outputs: &[u8]) -> Option<Vec<(u8, f32)>> {
    let values = reply.get("values")?.as_array()?;
    if values.len() != outputs.len() {
        return None;
    }
    values
        .iter()
        .zip(outputs)
        .map(|(v, o)| {
            if v.get("ok").and_then(Value::as_bool) == Some(false) {
                return None;
            }
            let ms = v.get("value").and_then(Value::as_f64)? as f32;
            (ms.is_finite() && ms >= 0.0).then_some((*o, ms))
        })
        .collect()
}

/// One analysed timed run of the measurement.
#[derive(Debug, Clone)]
pub(crate) struct RunDone {
    pub(crate) schedule: SystemSchedule,
    pub(crate) analysis: SystemAnalysis,
    /// `(unit, staged level)`.
    pub(crate) levels: Vec<(usize, f32)>,
    pub(crate) run_ids: Vec<u32>,
    // -- WP ATOVL -- how it was laid out, in words.
    pub(crate) plan: String,
}

/// One measured output.
#[derive(Debug, Clone)]
struct Measured {
    run: usize,
    unit: usize,
    output: u8,
    result: OutputResult,
    /// Its arrival on the common reference (run 0's, through the chain), ms.
    arrival_ms: f64,
    /// Its arrival in its own run, ms (the response's phase reference).
    own_ms: f64,
    inverted: bool,
}

/// A summed run to make: two outputs on different units at one T.
#[derive(Debug, Clone)]
pub(crate) struct SumJob {
    /// `(unit, output, inverted)`.
    pub(crate) a: (usize, u8, bool),
    pub(crate) b: (usize, u8, bool),
    /// The most both play at: 6 dB below the lower of the measurement's
    /// two ramp levels (S-AT4). The session ramps them again and plays 6 dB
    /// below what it finds, never above this.
    pub(crate) level_db: f32,
    /// The sum the walked run predicts.
    pub(crate) predicted: Response,
    /// The check band, Hz.
    pub(crate) band: (f64, f64),
    // -- WP ATSUMOFF --
    /// Where `a` and `b` start, samples after the run's first start
    /// ([`sum_starts`]): one of them 0.
    pub(crate) starts: (u64, u64),
}

// -- WP ATSUMOFF --
/// Where a summed run's two parts start, samples after its first start, and
/// how far apart their arrivals are then, ms (`a` less `b`): the part whose
/// output is predicted to arrive earlier starts later by the difference, in
/// whole samples, so both are heard together and the run is judged over its
/// whole band ([`window_reach`]). `pred_*_ms`: each output's arrival after
/// its own start as this run plays it (the walked arrival, with a verify's
/// change of delay): it holds the unit's play-out and `out.delay`, so on
/// USB-direct the streams' start offsets (tens of ms) are taken out here.
/// The offset is at most [`max_sum_spread_samples`]; a wider (or a
/// non-finite) difference is left in and narrows the band, failing closed.
pub(crate) fn sum_starts(pred_a_ms: f64, pred_b_ms: f64, rate_hz: u32) -> ((u64, u64), f64) {
    let diff = pred_a_ms - pred_b_ms;
    let per_ms = f64::from(rate_hz) / 1000.0;
    let cap = max_sum_spread_samples(rate_hz);
    let n = if diff.is_finite() && per_ms > 0.0 {
        // In range: finite, at most `cap` (u64 of a u32 rate in ms).
        let n = (diff.abs() * per_ms).round();
        if n >= cap as f64 { cap } else { n as u64 }
    } else {
        0
    };
    let n_ms = if per_ms > 0.0 { n as f64 / per_ms } else { 0.0 };
    if diff > 0.0 {
        ((0, n), diff - n_ms)
    } else {
        ((n, 0), diff + n_ms)
    }
}

/// Samples as ms at `rate_hz`.
fn samples_ms(n: u64, rate_hz: u32) -> f64 {
    if rate_hz == 0 {
        0.0
    } else {
        n as f64 * 1000.0 / f64::from(rate_hz)
    }
}

/// A summed run's verdict.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Verdict {
    pub(crate) job: usize,
    pub(crate) passed: bool,
    pub(crate) rms_db: Option<f64>,
    pub(crate) message: String,
}

impl Verdict {
    pub(crate) fn failed(job: usize, message: String) -> Verdict {
        Verdict {
            job,
            passed: false,
            rms_db: None,
            message,
        }
    }
}

/// The RMS difference of two magnitudes over `band`, on points both trust.
fn rms_over(measured: &Response, predicted: &Response, band: (f64, f64)) -> Option<(f64, usize)> {
    if measured.grid != predicted.grid {
        return None;
    }
    let freqs = measured.grid.freqs().ok()?;
    let ok = |r: &Response, i: usize| {
        r.valid
            .as_ref()
            .and_then(|v| v.get(i).copied())
            .unwrap_or(true)
            && r.mag_db
                .get(i)
                .is_some_and(|m| m.is_finite() && *m > FLOOR_DB)
    };
    let (mut ss, mut n) = (0.0, 0usize);
    for (i, f) in freqs.iter().enumerate() {
        if *f < band.0 || *f > band.1 || !ok(measured, i) || !ok(predicted, i) {
            continue;
        }
        let (Some(m), Some(p)) = (measured.mag_db.get(i), predicted.mag_db.get(i)) else {
            continue;
        };
        ss += (m - p) * (m - p);
        n = n.saturating_add(1);
    }
    (n >= MIN_SUM_POINTS).then(|| ((ss / n as f64).sqrt(), n))
}

impl SumJob {
    /// The summed run's verdict: its one key's response against the
    /// prediction, at most 1.5 dB RMS over the transition's band.
    /// `level_db`: the level the run played at.
    pub(crate) fn judge(
        &self,
        job: usize,
        a: &SystemAnalysis,
        plan: &SetupPlan,
        level_db: f32,
    ) -> Verdict {
        let Some(key) = a.outputs.first().filter(|o| o.members.len() == 2) else {
            return Verdict::failed(job, "the summed run has no sum".into());
        };
        let Some(ir) = a.output_ir(key.key) else {
            return Verdict::failed(job, "the summed run has no response".into());
        };
        let Some(pu) = plan.units.get(self.a.0) else {
            return Verdict::failed(job, "no such unit".into());
        };
        let measured = match runs::output_response(ir, a.walked.band_hz, level_db, &pu.plan) {
            Ok(r) => r.response,
            Err(e) => return Verdict::failed(job, format!("the summed run: {e}")),
        };
        match rms_over(&measured, &self.predicted, self.band) {
            Some((rms, _)) => {
                let passed = rms <= SUMMED_CHECK_MAX_RMS_DB;
                Verdict {
                    job,
                    passed,
                    rms_db: Some(rms),
                    message: if passed {
                        format!(
                            "the summed run agrees with the prediction ({rms:.2} dB RMS over {:.0} to {:.0} Hz)",
                            self.band.0, self.band.1
                        )
                    } else {
                        format!(
                            "the summed run differs from the prediction by {rms:.2} dB RMS over {:.0} to {:.0} Hz \
                             (at most 1.5 dB): every cross-unit delay is withdrawn",
                            self.band.0, self.band.1
                        )
                    },
                }
            }
            None => Verdict::failed(
                job,
                "too few trusted points in the transition's band to judge the summed run".into(),
            ),
        }
    }
}

// -- review AT-D1 --
/// Whether `sum` is a cancellation of `a` and `b` over most of `band`: more
/// than half of the band's points lie [`CANCELLATION_DB`] or more under the
/// louder part. With the parts of a summed run started so that they arrive
/// together (WP ATSUMOFF), two outputs of opposite polarity cancel over
/// their common band: the run is nearly silent at the microphone, its first
/// cycle is not found and the session fails on "cycle identity" (measured on
/// the stub; at one T their distance left a comb to judge, and the verify
/// after the apply met the same silence).
fn cancels(sum: &Response, a: &Response, b: &Response, band: (f64, f64)) -> bool {
    let Ok(freqs) = sum.grid.freqs() else {
        return false;
    };
    let (mut all, mut low) = (0usize, 0usize);
    for (i, f) in freqs.iter().enumerate() {
        if *f < band.0 || *f > band.1 {
            continue;
        }
        let (Some(s), Some(x), Some(y)) = (sum.mag_db.get(i), a.mag_db.get(i), b.mag_db.get(i))
        else {
            continue;
        };
        if !(x.is_finite() && y.is_finite()) {
            continue;
        }
        all = all.saturating_add(1);
        if !s.is_finite() || *s < x.max(*y) - CANCELLATION_DB {
            low = low.saturating_add(1);
        }
    }
    all > 0 && low.saturating_mul(2) > all
}

/// What the apply filed and did.
#[derive(Debug, Clone, Default)]
struct Filed {
    model: Value,
    /// The play-out and output delays after the apply, by unit.
    after: Vec<Option<Terms>>,
    // -- WP ATGUARD --
    /// The hub's guard of the apply, by id (`None`: an older hub, nothing
    /// reverts by itself).
    guard: Option<u64>,
}

// -- WP ATGUARD -- the hub's guard of a system apply (P5TRUST's decision
// (c) for its length).
/// The guard's time beyond the planned verify, s (as P5TRUST's).
pub(crate) const GUARD_MARGIN_S: u64 = 120;
/// The planned verify is taken this much longer, percent (as P5TRUST's).
pub(crate) const GUARD_SLACK_PCT: u64 = 25;
/// The guard's longest, s (as a single unit's guarded apply).
pub(crate) const GUARD_MAX_S: u32 = crate::guarded::APPLY_GUARD_MAX_TIMEOUT_S;
/// Per timed run in the plan beyond its sound: the hub's round trips, the
/// proofs, the analysis, ms.
const GUARD_RUN_SLACK_MS: u64 = 1_000;
/// A timed run's tail after its last sweep, ms (the session's).
const GUARD_TAIL_MS: u64 = 600;

/// Whether `hub` guards a system apply (the cap).
pub(crate) fn hub_guards(hub: &dyn Hub) -> bool {
    hub.caps()
        .is_some_and(|c| c.iter().any(|x| x == msg_guard::CAP))
}

use dspi_measure::system::messages::guard as msg_guard;

/// A measurement's outcome, kept for the proposal and its apply.
#[derive(Debug, Clone)]
pub(crate) struct Outcome {
    pub(crate) setup: SetupPlan,
    pub(crate) result: MeasureSystemResult,
    measured: Vec<Measured>,
    terms: Vec<Option<Terms>>,
    /// `(unit, output, level)` from the ramps.
    levels: Vec<(usize, u8, f32)>,
    /// Per transition: `(a index, b index)` into `measured`, its whole
    /// check band (PLAN-AT 1.4; a summed run judges it up to its
    /// [`window_reach`]).
    transitions: Vec<(usize, usize, (f64, f64))>,
    verdicts: Vec<Option<Verdict>>,
    proposal_id: u32,
    filed: Option<Filed>,
    pub(crate) applied: MeasureSystemApplied,
    /// Per unit: it is the source of the group's time base (for the terms
    /// read after an apply).
    sources: Vec<bool>,
    /// When a Keep after a verify that did not pass was first refused.
    keep_refused_at: Option<dspi_transport::clock::Instant>,
}

/// A transition's check band as far up as a summed run can be judged whose
/// two arrivals are `apart_ms` apart: its response is windowed about the
/// first arrival, and above `0.375 * cycles / apart` the later one falls
/// outside the window. Two arrivals that coincide (after an apply) are
/// judged over the whole band.
fn window_reach(band: (f64, f64), apart_ms: f64) -> (f64, f64) {
    let dt = (apart_ms.abs() / 1000.0).max(1e-9);
    (
        band.0,
        band.1.min(SUM_WINDOW_REACH * FIT_WINDOW_CYCLES / dt),
    )
}

/// The transition responses on one reference: `h` shifted by `ms` (its
/// phase only).
fn shifted(h: &Response, ms: f64) -> Response {
    let mut r = h.clone();
    if let (Some(ph), Ok(freqs)) = (r.phase_deg.as_mut(), h.grid.freqs()) {
        for (p, f) in ph.iter_mut().zip(freqs) {
            *p -= 360.0 * f * ms / 1000.0;
        }
    }
    r
}

/// The complex sum of `parts` (`response, sign`), magnitude and phase, on
/// their common grid; a point is trusted where every part is.
fn complex_sum(parts: &[(&Response, f64)]) -> Option<Response> {
    let first = parts.first()?.0;
    if parts.iter().any(|(r, _)| r.grid != first.grid) {
        return None;
    }
    let n = first.mag_db.len();
    let mut mag = Vec::with_capacity(n);
    let mut phase = Vec::with_capacity(n);
    let mut valid = Vec::with_capacity(n);
    for i in 0..n {
        let (mut re, mut im, mut ok) = (0.0f64, 0.0f64, true);
        let mut loudest = f64::NEG_INFINITY;
        for (r, s) in parts {
            let m = r.mag_db.get(i).copied().unwrap_or(f64::NAN);
            let p = r
                .phase_deg
                .as_ref()
                .and_then(|v| v.get(i).copied())
                .unwrap_or(f64::NAN);
            loudest = loudest.max(m);
            let a = 10f64.powf(m / 20.0) * s;
            re += a * p.to_radians().cos();
            im += a * p.to_radians().sin();
            ok = ok
                && r.valid
                    .as_ref()
                    .is_none_or(|v| v.get(i).copied().unwrap_or(false));
        }
        let db = 20.0 * (re * re + im * im).sqrt().max(1e-12).log10();
        let fin = db.is_finite();
        mag.push(if fin {
            db.clamp(FLOOR_DB, 400.0)
        } else {
            FLOOR_DB
        });
        phase.push(if fin { im.atan2(re).to_degrees() } else { 0.0 });
        // A cancellation notch is not judged: its depth depends on each
        // response's window, not on the timing; a wrong delay moves the
        // notches onto points that are judged.
        valid.push(ok && fin && db >= loudest - CANCELLATION_DB);
    }
    Some(Response {
        grid: first.grid,
        mag_db: mag,
        phase_deg: Some(phase),
        valid: Some(valid),
        spl: parts.iter().all(|(r, _)| r.spl),
    })
}

impl Outcome {
    /// The outcome of the measurement's analysed runs.
    pub(crate) fn build(
        setup: SetupPlan,
        runs_done: Vec<RunDone>,
        terms: Vec<Option<Terms>>,
        levels: Vec<(usize, u8, f32)>,
        sources: Vec<bool>,
    ) -> Result<Outcome> {
        let mut reasons: Vec<String> = Vec::new();
        let serial = |u: usize| {
            setup
                .units
                .get(u)
                .map_or("", |p| p.serial.as_str())
                .to_owned()
        };
        // Every single-member key's response, on its run's reference.
        let mut measured: Vec<Measured> = Vec::new();
        let mut cut: Vec<String> = Vec::new();
        for (ri, r) in runs_done.iter().enumerate() {
            for o in &r.analysis.outputs {
                let [m] = o.members.as_slice() else {
                    continue;
                };
                let unit = usize::from(m.unit.0);
                let Some(pu) = setup.units.get(unit) else {
                    continue;
                };
                // The anchor once: from the first run.
                if measured
                    .iter()
                    .any(|x| x.unit == unit && x.output == m.output)
                {
                    continue;
                }
                let Some(ir) = r.analysis.output_ir(o.key) else {
                    continue;
                };
                // Review AT-D1: an arrival at the end of what the analysis
                // kept is not a measurement.
                let room_ms = (ir.ir.samples.len() as f64 - ir.onset_index) * 1000.0
                    / f64::from(ir.ir.rate_hz.max(1));
                if room_ms.is_nan() || room_ms < MIN_ROOM_AFTER_ONSET_MS {
                    cut.push(format!(
                        "{} output {} is heard too long after the run's first output for the gap \
                         between sweeps: its arrival cannot be measured in this run",
                        pu.serial, m.output
                    ));
                }
                let level = r
                    .levels
                    .iter()
                    .find(|(u, _)| *u == unit)
                    .map_or(pu.plan.start_db, |(_, l)| *l);
                let mut result =
                    runs::output_response(ir, r.analysis.walked.band_hz, level, &pu.plan)?;
                // The walk labels keys; the document wants the unit's output.
                result.output = m.output;
                let inverted = pu
                    .ctx
                    .chains
                    .iter()
                    .find(|(x, _)| *x == m.output)
                    .is_some_and(|(_, c)| c.feed.inverted);
                measured.push(Measured {
                    run: ri,
                    unit,
                    output: m.output,
                    result,
                    arrival_ms: o.arrival_ms,
                    // -- WP ATLATE -- the response is on the analysis'
                    // timeline: the planned lateness earlier.
                    own_ms: o.arrival_ms - o.lateness_ms,
                    inverted,
                });
            }
            reasons.extend(r.analysis.reasons.iter().cloned());
        }
        reasons.extend(cut.iter().cloned());
        // Chained runs: every arrival through the anchor.
        let mut chained: Option<Vec<ChainedOutput>> = None;
        let mut chain_refused: Option<String> = None;
        if runs_done.len() > 1 {
            let anchor = setup.anchor;
            let anchor_responses: Vec<Response> = runs_done
                .iter()
                .map(|r| {
                    r.analysis
                        .output(anchor.unit, anchor.output)
                        .and_then(|o| r.analysis.output_ir(o.key))
                        .and_then(|ir| {
                            let pu = setup.units.get(usize::from(anchor.unit.0))?;
                            let level = r
                                .levels
                                .iter()
                                .find(|(u, _)| *u == usize::from(anchor.unit.0))
                                .map_or(pu.plan.start_db, |(_, l)| *l);
                            runs::output_response(ir, r.analysis.walked.band_hz, level, &pu.plan)
                                .ok()
                        })
                        .map(|r| r.response)
                })
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| invalid("a run does not repeat the anchor output"))?;
            let chain_input: Vec<ChainRun<'_>> = runs_done
                .iter()
                .zip(&anchor_responses)
                .map(|(r, a)| ChainRun {
                    analysis: &r.analysis,
                    anchor_response: a,
                })
                .collect();
            // The microphone stays open across the runs: one capture clock.
            let options = ChainOptions {
                one_capture_clock: true,
                ..ChainOptions::default()
            };
            match chain_runs(anchor, &chain_input, &options) {
                Ok(c) => {
                    let base = runs_done
                        .first()
                        .and_then(|r| r.analysis.output(anchor.unit, anchor.output))
                        .map_or(0.0, |o| o.arrival_ms);
                    for m in &mut measured {
                        if let Some(c) = c.outputs.iter().find(|c| {
                            c.output.unit.0 == u8::try_from(m.unit).unwrap_or(u8::MAX)
                                && c.output.output == m.output
                        }) {
                            m.arrival_ms = base + c.relative_ms;
                        }
                    }
                    chained = Some(c.outputs);
                }
                Err(e) => {
                    chain_refused = Some(e.to_string());
                    reasons.push(e.to_string());
                }
            }
        }
        // The engine's terms out: the acoustic arrivals.
        let acoustic = |m: &Measured| -> Option<f64> {
            terms
                .get(m.unit)
                .and_then(|t| t.as_ref())
                .and_then(|t| t.of(m.output))
                .map(|e| m.arrival_ms * 1000.0 - e)
        };
        let latest = measured
            .iter()
            .enumerate()
            .filter_map(|(i, m)| acoustic(m).map(|a| (i, a)))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        let timing_reliable = runs_done.iter().all(|r| r.analysis.timing_reliable);
        let run_ids: Vec<u32> = runs_done.iter().flat_map(|r| r.run_ids.clone()).collect();
        let gate: Vec<GateFinding> = Vec::new();
        let mut arrivals = Vec::new();
        for m in &measured {
            let s = serial(m.unit);
            let (offered, unc, reason) = match (latest, acoustic(m)) {
                (Some((li, _)), Some(_)) => match measured.get(li) {
                    Some(l) => pair_offer(m, l, &runs_done, chained.as_deref(), &chain_refused),
                    None => (false, None, "no reference".into()),
                },
                _ => (
                    false,
                    None,
                    format!(
                        "{s}: the alignment's terms of this unit could not be read from the hub"
                    ),
                ),
            };
            let a = acoustic(m).unwrap_or(f64::NAN);
            let r = latest.map_or(f64::NAN, |l| l.1);
            let measured_ref = latest
                .and_then(|(li, _)| measured.get(li))
                .map_or(f64::NAN, |l| l.arrival_ms * 1000.0);
            let fin = |v: f64| if v.is_finite() { v } else { 0.0 };
            // Review AT-D1: the reference may be the output that was cut,
            // so nothing of the group is offered then.
            let (offered, reason) = match cut.first() {
                Some(why) => (false, why.clone()),
                None => (offered, reason),
            };
            let offered = may_propose(&gate, offered) && (a - r).is_finite();
            arrivals.push(MeasureSystemArrival {
                serial: s,
                output: m.output,
                arrival_us: fin(a - r),
                measured_us: fin(m.arrival_ms * 1000.0 - measured_ref),
                uncertainty_us: unc,
                offered,
                reason: if offered { String::new() } else { reason },
            });
        }
        let delays: Vec<MeasureSystemDelay> = runs_done
            .iter()
            .flat_map(|r| r.analysis.delays.iter())
            .map(|d| {
                // -- review AT-E -- as the arrivals (review AT-D1): with an
                // output cut at the end of what the analysis kept, no
                // cross-unit delay of the group is shown as offered either.
                let (offered, reason) = match cut.first() {
                    Some(why) => (false, why.clone()),
                    None => (d.offered, d.reason.clone()),
                };
                MeasureSystemDelay {
                    from_serial: serial(usize::from(d.from.unit.0)),
                    from_output: d.from.output,
                    to_serial: serial(usize::from(d.to.unit.0)),
                    to_output: d.to.output,
                    delay_us: d.delay_ms * 1000.0,
                    uncertainty_us: d.uncertainty_us,
                    offered,
                    reason,
                }
            })
            .collect();
        // The inter-unit transitions (R8d).
        let mut transitions = Vec::new();
        let mut rows = Vec::new();
        for (i, a) in measured.iter().enumerate() {
            for (j, b) in measured.iter().enumerate().skip(i.saturating_add(1)) {
                if a.unit == b.unit {
                    continue;
                }
                let u = pair_uncertainty(a, b, &runs_done, chained.as_deref());
                let ra = shifted(&a.result.response, a.arrival_ms - a.own_ms);
                let rb = shifted(&b.result.response, b.arrival_ms - b.own_ms);
                if let Some(whole) = transition_band(&ra, &rb, u) {
                    // Kept whole: each summed run is judged as far up as its
                    // own window reaches (`window_reach`); WP ATSUMOFF: with
                    // its parts started so the arrivals come together.
                    let (_, apart) = sum_starts(a.arrival_ms, b.arrival_ms, setup.rate_hz);
                    let band = window_reach(whole, apart);
                    transitions.push((i, j, whole));
                    rows.push(MeasureSystemTransition {
                        a_serial: serial(a.unit),
                        a_output: a.output,
                        b_serial: serial(b.unit),
                        b_output: b.output,
                        band_lo_hz: band.0,
                        band_hi_hz: band.1,
                        checked: false,
                        passed: false,
                        rms_db: None,
                        message: "the summed check has not run yet".into(),
                    });
                }
            }
        }
        // One document per unit.
        let documents = documents(&setup, &runs_done, &measured, &arrivals)?;
        let n = transitions.len();
        Ok(Outcome {
            result: MeasureSystemResult {
                documents,
                arrivals,
                delays,
                transitions: rows,
                timing_reliable,
                reasons,
                run_ids,
                runs: u16::try_from(runs_done.len()).unwrap_or(u16::MAX),
                // -- WP ATOVL -- which plan each run used.
                plan: plan_words(&runs_done),
            },
            setup,
            measured,
            terms,
            levels,
            transitions,
            verdicts: vec![None; n],
            proposal_id: 1,
            filed: None,
            applied: MeasureSystemApplied::default(),
            sources,
            keep_refused_at: None,
        })
    }

    /// The summed runs of `purpose`: every transition's (a sum check), or
    /// those again with the applied delays (a verify).
    pub(crate) fn sum_jobs(&self, purpose: MeasureSystemPurpose) -> Result<Vec<SumJob>> {
        if self.transitions.is_empty() {
            return Err(invalid("no inter-unit transition to check"));
        }
        let after = match purpose {
            MeasureSystemPurpose::Verify => {
                if !matches!(
                    self.applied.stage,
                    MeasureSystemApplyStage::Applied | MeasureSystemApplyStage::Verified
                ) {
                    return Err(invalid("nothing is applied to verify"));
                }
                self.filed.as_ref().map(|f| f.after.clone())
            }
            _ => None,
        };
        let mut jobs = Vec::new();
        for &(i, j, band) in &self.transitions {
            let (Some(a), Some(b)) = (self.measured.get(i), self.measured.get(j)) else {
                continue;
            };
            let level = |m: &Measured| {
                self.levels
                    .iter()
                    .find(|(u, o, _)| *u == m.unit && *o == m.output)
                    .map_or(-120.0, |(_, _, l)| *l)
            };
            let level_db = level(a).min(level(b)) - SUM_DOUBLING_DB;
            // A verify predicts with the change the apply made to each.
            let change = |m: &Measured| -> f64 {
                let before = self
                    .terms
                    .get(m.unit)
                    .and_then(|t| t.as_ref())
                    .and_then(|t| t.of(m.output));
                let now = after
                    .as_ref()
                    .and_then(|v| v.get(m.unit))
                    .and_then(|t| t.as_ref())
                    .and_then(|t| t.of(m.output));
                match (before, now) {
                    (Some(b), Some(n)) => (n - b) / 1000.0,
                    _ => 0.0,
                }
            };
            // WP ATSUMOFF: the part predicted to arrive earlier starts that
            // much later (whole samples), so the two are heard together as
            // the programme hears them although the units' play-out and
            // output delays differ (USB-direct: the streams' start offsets).
            // T is commanded and proven (G3), so the offset is exact and the
            // prediction moves each part by its own start.
            let rate = self.setup.rate_hz;
            let (starts, apart) =
                sum_starts(a.arrival_ms + change(a), b.arrival_ms + change(b), rate);
            let ra = shifted(
                &a.result.response,
                a.arrival_ms - a.own_ms + change(a) + samples_ms(starts.0, rate),
            );
            let rb = shifted(
                &b.result.response,
                b.arrival_ms - b.own_ms + change(b) + samples_ms(starts.1, rate),
            );
            // Review AT-C: the band follows the arrivals this run is predicted
            // with. With the starts above they coincide to half a sample, so
            // a sum check and a verify judge the whole transition band, and a
            // delay that was wrong by more than the band's top allows fails
            // them; at one T it would be judged only as far up as the walked
            // run's distance (or USB-direct's start offsets) allowed.
            let mut band = window_reach(band, apart);
            if after.is_some() {
                // A unit plays an output delay in whole samples: an applied
                // delay is uncertain by half a sample, 45 degrees at a
                // quarter of the rate (the plan's `1 / (8 u)`).
                band.1 = band.1.min(f64::from(self.setup.rate_hz) / 4.0);
            }
            let sign = |m: &Measured| if m.inverted { -1.0 } else { 1.0 };
            let predicted = complex_sum(&[(&ra, sign(a)), (&rb, sign(b))])
                .ok_or_else(|| invalid("the transition's responses are on different grids"))?;
            // Review AT-D1: a sum that cancels is not played (there would be
            // nothing to hear and nothing to judge): it is the polarity.
            if cancels(&predicted, &ra, &rb, band) {
                let name = |m: &Measured| {
                    let serial = self
                        .setup
                        .units
                        .get(m.unit)
                        .map_or("", |u| u.serial.as_str());
                    format!("{serial} output {}", m.output)
                };
                return Err(invalid(format!(
                    "{} and {} cancel over their common band when they arrive together (opposite \
                     polarity): the summed run cannot be judged and no cross-unit delay is \
                     offered; correct the polarity and measure again",
                    name(a),
                    name(b)
                )));
            }
            jobs.push(SumJob {
                a: (a.unit, a.output, a.inverted),
                b: (b.unit, b.output, b.inverted),
                level_db,
                predicted,
                band,
                starts,
            });
        }
        Ok(jobs)
    }

    /// A sum check's or a verify's verdicts.
    pub(crate) fn absorb_verdicts(&mut self, purpose: MeasureSystemPurpose, v: Vec<Verdict>) {
        match purpose {
            MeasureSystemPurpose::Verify => {
                let ok = !v.is_empty()
                    && v.iter().all(|x| x.passed)
                    && v.len() == self.transitions.len();
                self.applied.verified = Some(ok);
                self.keep_refused_at = None;
                self.applied.stage = MeasureSystemApplyStage::Verified;
                self.applied.message = v
                    .iter()
                    .map(|x| x.message.clone())
                    .collect::<Vec<_>>()
                    .join("; ");
            }
            _ => {
                for x in v {
                    if let Some(slot) = self.verdicts.get_mut(x.job) {
                        *slot = Some(x.clone());
                    }
                    if let Some(row) = self.result.transitions.get_mut(x.job) {
                        row.checked = true;
                        row.passed = x.passed;
                        row.rms_db = x.rms_db;
                        row.message = x.message;
                    }
                }
                self.proposal_id = self.proposal_id.saturating_add(1);
            }
        }
    }

    /// The proposal for a session of `role`.
    pub(crate) fn proposal(&self, role: &str) -> MeasureSystemProposal {
        let needed = self.verdicts.iter().any(Option::is_none);
        let withdrawn = self.verdicts.iter().flatten().any(|v| !v.passed);
        let mut arrivals = Vec::new();
        let mut withheld = Vec::new();
        for a in &self.result.arrivals {
            if a.offered && !withdrawn {
                arrivals.push(a.clone());
            } else {
                let mut w = a.clone();
                if withdrawn && a.offered {
                    w.offered = false;
                    w.reason = "a summed check failed: every cross-unit delay is withdrawn".into();
                }
                withheld.push(w);
            }
        }
        // The reference alone is not an alignment.
        let offered = !needed && !withdrawn && arrivals.len() >= 2;
        let may_apply = role == "admin";
        let notice = if withdrawn {
            Some(MeasureSystemIssue::new(
                "",
                "measure.system.sum_check_failed",
                None,
                "a summed run did not agree with the prediction: every cross-unit delay is withdrawn",
            ))
        } else if arrivals.len() < 2 {
            let unreliable = self
                .result
                .arrivals
                .iter()
                .find(|a| a.reason.contains("timing unreliable"));
            match unreliable {
                Some(a) => Some(MeasureSystemIssue::new(
                    &a.serial,
                    "measure.system.timing_unreliable",
                    Some(a.output),
                    a.reason.clone(),
                )),
                None => Some(MeasureSystemIssue::new(
                    "",
                    "measure.system.nothing_offered",
                    None,
                    "no delay across the units can be offered",
                )),
            }
        } else if needed {
            Some(MeasureSystemIssue::new(
                "",
                "measure.system.sum_check_needed",
                None,
                "run the summed check of every transition between the units first",
            ))
        } else if !may_apply {
            Some(MeasureSystemIssue::new(
                "",
                "measure.system.admin",
                None,
                "applying an alignment across units needs the Admin role on the hub",
            ))
        } else {
            None
        };
        MeasureSystemProposal {
            id: self.proposal_id,
            arrivals,
            withheld,
            transitions: self.result.transitions.clone(),
            sum_check_needed: needed,
            withdrawn,
            offered,
            may_apply,
            notice,
            // -- WP ATGUARD -- set by `proposal_for`
            guard_s: 0,
            guard_note: String::new(),
        }
    }

    fn snapshot(&self, hub: &mut dyn Hub) -> Result<Vec<MeasureSystemSnapshot>> {
        let mut out = Vec::new();
        for u in &self.setup.units {
            let outputs: Vec<u8> = u.plan.outputs.iter().map(|o| o.output).collect();
            let items: Vec<Value> = outputs
                .iter()
                .map(|o| json!({"path": "out.delay", "idx": [o]}))
                .collect();
            let got = hub.request(
                "param.get_many",
                json!({"serial": u.serial, "items": items}),
            )?;
            let delays = out_delays_of(&got, &outputs).ok_or_else(|| {
                invalid(format!("{}: its output delays could not be read", u.serial))
            })?;
            out.push(MeasureSystemSnapshot {
                serial: u.serial.clone(),
                out_delays_ms: delays
                    .into_iter()
                    .map(|(output, delay_ms)| MeasureSystemOutDelay { output, delay_ms })
                    .collect(),
            });
        }
        Ok(out)
    }

    fn read_after(&self, hub: &mut dyn Hub) -> Vec<Option<Terms>> {
        let status = system_status(hub); // -- WP ATDEMO --
        let snaps = self.snapshot(hub).ok();
        self.setup
            .units
            .iter()
            .enumerate()
            .map(|(i, u)| {
                let source = self.sources.get(i).copied().unwrap_or(false);
                let playout_us =
                    playout_us(status.as_ref()?, &u.serial, PlayOut::Engine { source })?;
                let s = snaps.as_ref()?.iter().find(|s| s.serial == u.serial)?;
                Some(Terms {
                    playout_us,
                    out_delay_ms: s
                        .out_delays_ms
                        .iter()
                        .map(|d| (d.output, d.delay_ms))
                        .collect(),
                })
            })
            .collect()
    }

    // -- WP ATGUARD --
    /// The verify's planned length, ms: one pre-roll, each output of a
    /// transition ramped again from its plan's start to its ceiling (every
    /// step a timed run), then each transition's summed run.
    pub(crate) fn verify_planned_ms(&self) -> u64 {
        let lead = u64::from(
            self.setup
                .setup
                .lead_ms
                .max(super::plan::PLANNING_LEAD_MS.saturating_add(super::plan::LEAD_MARGIN_MS)),
        );
        let mut ms = u64::from(super::super::plan::PREROLL_MS);
        let mut ramped: Vec<(usize, u8)> = Vec::new();
        for &(i, j, _) in &self.transitions {
            for k in [i, j] {
                let Some(m) = self.measured.get(k) else {
                    continue;
                };
                let Some(p) = self.setup.units.get(m.unit).map(|u| &u.plan) else {
                    continue;
                };
                let gap = u64::from(p.sweep.gap_ms);
                if !ramped.contains(&(m.unit, m.output)) {
                    ramped.push((m.unit, m.output));
                    let span = f64::from(p.ceiling_db - p.start_db).max(0.0);
                    let steps = ((span / f64::from(super::super::safety::RAMP_STEP_DB)).ceil()
                        as u64)
                        .saturating_add(1);
                    let step = lead
                        .saturating_add(u64::from(super::super::runs::RAMP_SWEEP_MS))
                        .saturating_add(gap)
                        .saturating_add(GUARD_TAIL_MS)
                        .saturating_add(GUARD_RUN_SLACK_MS);
                    ms = ms.saturating_add(steps.saturating_mul(step));
                }
                if k == j {
                    let passes = u64::from(super::session::SUM_PASSES);
                    let sweep = u64::from(p.sweep.duration_ms).saturating_add(gap);
                    ms = ms.saturating_add(
                        lead.saturating_add(u64::from(dspi_measure::system::MAX_SUM_SPREAD_MS))
                            .saturating_add(passes.saturating_mul(sweep))
                            .saturating_add(GUARD_TAIL_MS)
                            .saturating_add(GUARD_RUN_SLACK_MS),
                    );
                }
            }
        }
        ms
    }

    /// The guard's planned length before its cap, s (P5TRUST's (c)): the
    /// planned verify, [`GUARD_SLACK_PCT`] longer, plus [`GUARD_MARGIN_S`].
    pub(crate) fn guard_planned_s(&self) -> u64 {
        self.verify_planned_ms()
            .saturating_mul(100 + GUARD_SLACK_PCT)
            .div_ceil(100_000)
            .saturating_add(GUARD_MARGIN_S)
    }

    /// The guard's length, s: [`Self::guard_planned_s`], at most
    /// [`GUARD_MAX_S`].
    pub(crate) fn guard_s(&self) -> u32 {
        u32::try_from(self.guard_planned_s())
            .unwrap_or(u32::MAX)
            .min(GUARD_MAX_S)
    }

    /// The guard in words, for the proposal.
    fn guard_note(&self, guarded: bool) -> String {
        if !guarded {
            return "this hub has no guard for an alignment across units: nothing reverts by \
                    itself after the apply (not at a deadline, not when this console's \
                    connection is lost); verify it, then Keep or Revert"
                .into();
        }
        let s = self.guard_s();
        let planned = self.guard_planned_s();
        let mut note = format!(
            "the hub reverts the alignment by itself unless it is kept within {s} s of the \
             apply, and at once if this console's connection is lost"
        );
        if planned > u64::from(s) {
            note.push_str(&format!(
                "; the verify needs about {planned} s with its margin, longer than the guard's \
                 longest ({s} s): the hub may revert before the verify ends"
            ));
        }
        note
    }

    /// The proposal for `role` on a hub that does (`guarded`) or does not
    /// guard a system apply.
    pub(crate) fn proposal_for(&self, role: &str, guarded: bool) -> MeasureSystemProposal {
        let mut p = self.proposal(role);
        p.guard_s = if guarded { self.guard_s() } else { 0 };
        p.guard_note = self.guard_note(guarded);
        p
    }

    // -- review AT-D2 --
    /// An applied alignment waits for Keep or Revert.
    pub(crate) fn awaits_keep(&self) -> bool {
        matches!(
            self.applied.stage,
            MeasureSystemApplyStage::Applied | MeasureSystemApplyStage::Verified
        )
    }

    /// Whether the hub's guard of what is applied still runs (a verify
    /// runs inside it). Without a guard: yes. One the hub has ended is
    /// taken as its outcome says (reverted at its deadline: the stage is
    /// `Reverted`), and refused.
    pub(crate) fn guard_alive(&mut self, hub: &mut dyn Hub) -> Result<()> {
        let Some(g) = self.filed.as_ref().and_then(|f| f.guard) else {
            return Ok(());
        };
        if !matches!(
            self.applied.stage,
            MeasureSystemApplyStage::Applied | MeasureSystemApplyStage::Verified
        ) {
            return Ok(());
        }
        let st: msg_guard::StatusReply =
            serde_json::from_value(hub.request(msg_guard::STATUS, json!({}))?).map_err(|e| {
                DspiError::Link {
                    code: "protocol".into(),
                    msg: format!("system.guard.status: {e}"),
                }
            })?;
        if st
            .active
            .as_ref()
            .is_some_and(|a| a.guard == g && !a.reverting)
        {
            return Ok(());
        }
        let why = self.guard_ended(st.last.filter(|o| o.guard == g));
        Err(invalid(why))
    }

    // -- WP REVERTW --
    /// The revert did not put everything back: the stage says so, with the
    /// `revert.*` notice `key` (`message` names what).
    fn revert_incomplete(&mut self, key: &str) {
        self.applied.stage = MeasureSystemApplyStage::RevertIncomplete;
        self.applied.revert_notice = key.to_owned();
    }

    /// The hub ended the guard by itself: take its outcome. Its words.
    fn guard_ended(&mut self, last: Option<msg_guard::Outcome>) -> String {
        let why = match &last {
            Some(o) if o.result == "kept" => {
                self.applied.stage = MeasureSystemApplyStage::Kept;
                self.applied.message = "kept: the alignment stays".into();
                return format!("the hub kept the alignment ({})", o.message);
            }
            Some(o) if o.result == "reverted" => format!(
                "the hub ended the guard ({}) and reverted the alignment: {}",
                o.reason, o.message
            ),
            // -- review AT-D2 -- not put back (or not wholly): said first,
            // never shown as a clean revert
            Some(o) => format!(
                "the hub ended the guard ({}), but the alignment was NOT wholly put back ({}): {}; \
                 check the installation and the units' delays",
                o.reason, o.result, o.message
            ),
            None => "the hub's guard of this apply has ended; what the hub did could not be \
                     read: check the installation and the units' delays"
                .into(),
        };
        if let Some(o) = &last {
            self.applied.revision_after = o.revision;
            self.applied.writes_ok = o.writes_ok;
            self.applied.write_failures = o.write_failures.clone();
        }
        self.applied.stage = MeasureSystemApplyStage::Reverted;
        // -- WP REVERTW -- not put back (or not wholly, or not known): its
        // own stage and notice, never a clean revert
        match last.as_ref().map(|o| o.result.as_str()) {
            Some("reverted") => {}
            Some("not_written") => {
                self.revert_incomplete(crate::guarded::NOTICE_REVERT_NOT_WRITTEN);
            }
            Some(_) => self.revert_incomplete(crate::guarded::NOTICE_REVERT_FAILED),
            None => self.revert_incomplete(crate::guarded::NOTICE_REVERT_UNKNOWN),
        }
        self.applied.message = why.clone();
        why
    }

    /// Apply proposal `id` (Admin).
    pub(crate) fn apply(&mut self, hub: &mut dyn Hub, id: u32) -> Result<MeasureSystemApplied> {
        if hub.role() != "admin" {
            return Err(DspiError::Denied);
        }
        let p = self.proposal("admin");
        if id != p.id {
            return Err(invalid(format!(
                "proposal {id} is not the current one ({})",
                p.id
            )));
        }
        if !p.offered {
            return Err(invalid(
                p.notice
                    .map_or_else(|| "nothing to apply".to_owned(), |n| n.message),
            ));
        }
        // -- WP ATGUARD -- the hub's guard first (it files the model too);
        // an older hub has none: today's apply, nothing reverts by itself.
        let guard_s = if hub_guards(hub) { self.guard_s() } else { 0 };
        let guard = if guard_s > 0 {
            let req = msg_guard::BeginRequest {
                timeout_ms: Some(guard_s.saturating_mul(1000)),
            };
            let reply: msg_guard::BeginReply = serde_json::from_value(hub.request(
                msg_guard::BEGIN,
                serde_json::to_value(&req).unwrap_or_else(|_| json!({})),
            )?)
            .map_err(|e| DspiError::Link {
                code: "protocol".into(),
                msg: format!("system.guard.begin: {e}"),
            })?;
            Some(reply.guard)
        } else {
            None
        };
        // Nothing was changed: the guard ends kept (nothing to revert).
        let unguard = |hub: &mut dyn Hub| {
            if let Some(g) = guard {
                let _ = hub.request(msg_guard::CONFIRM, json!({ "guard": g }));
            }
        };
        let r = self.apply_guarded(hub, &p, guard, guard_s);
        // The model was set when the filed record names this guard.
        let set = guard.is_some() && self.filed.as_ref().and_then(|f| f.guard) == guard;
        if r.is_err() && !set {
            unguard(hub);
        }
        r
    }

    /// [`Self::apply`] within the hub's guard `guard` (if any).
    fn apply_guarded(
        &mut self,
        hub: &mut dyn Hub,
        p: &MeasureSystemProposal,
        guard: Option<u64>,
        guard_s: u32,
    ) -> Result<MeasureSystemApplied> {
        // File the model's revision and every unit's output delays first.
        let got = hub.request("system.get", json!({}))?;
        let revision = got.get("revision").and_then(Value::as_u64).unwrap_or(0);
        let model = got.get("model").cloned().unwrap_or(Value::Null);
        let snapshots = self.snapshot(hub)?;
        let mut edited = model.clone();
        let at = dspi_transport::clock::system_time()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let runs: Vec<u32> = self
            .result
            .run_ids
            .iter()
            .copied()
            .take(MAX_MEASURED_RUNS)
            .collect();
        let mut missing = Vec::new();
        for a in &p.arrivals {
            let speaker = edited
                .get_mut("units")
                .and_then(Value::as_array_mut)
                .and_then(|us| {
                    us.iter_mut()
                        .find(|u| u.get("serial").and_then(Value::as_str) == Some(&a.serial))
                })
                .and_then(|u| u.get_mut("speakers"))
                .and_then(Value::as_array_mut)
                .and_then(|ss| {
                    ss.iter_mut().find(|s| {
                        s.get("output").and_then(Value::as_u64) == Some(u64::from(a.output))
                    })
                })
                .and_then(Value::as_object_mut);
            match speaker {
                Some(s) => {
                    s.insert("measuredArrivalUs".into(), json!(a.arrival_us as f32));
                    s.insert("measuredRuns".into(), json!(runs));
                    s.insert("measuredAt".into(), json!(at));
                }
                None => missing.push(format!(
                    "{} output {}",
                    a.serial,
                    u16::from(a.output).saturating_add(1)
                )),
            }
        }
        if !missing.is_empty() {
            return Err(invalid(format!(
                "the hub's model has no speaker on {}: add them to the installation first",
                missing.join(", ")
            )));
        }
        let mut fields = Map::new();
        fields.insert("model".into(), edited);
        fields.insert("revision".into(), revision.into());
        let saved = hub.request("system.set", Value::Object(fields))?;
        let revision_after = saved
            .get("revision")
            .and_then(Value::as_u64)
            .unwrap_or(revision);
        let report = hub.request("system.align", json!({"apply": true}));
        self.filed = Some(Filed {
            model,
            after: Vec::new(),
            guard, // -- WP ATGUARD --
        });
        let (ok, failures) = report.as_ref().map_or((0, Vec::new()), writes_of);
        self.applied = MeasureSystemApplied {
            stage: MeasureSystemApplyStage::Applied,
            revision_before: revision,
            revision_after,
            snapshots,
            writes_ok: ok,
            write_failures: failures,
            verified: None,
            message: match (&report, guard) {
                (Ok(_), Some(_)) => format!(
                    "applied: verify it with the summed runs, then Keep or Revert; the hub \
                     reverts it by itself unless it is kept within {guard_s} s"
                ),
                (Ok(_), None) => "applied: verify it with the summed runs, then Keep or Revert \
                                  (this hub has no guard: nothing reverts by itself)"
                    .into(),
                (Err(e), _) => format!("the model was saved but the units were not written: {e}"),
            },
            guard_s,                      // -- WP ATGUARD --
            revert_notice: String::new(), // -- WP REVERTW --
        };
        let after = self.read_after(hub);
        if let Some(f) = self.filed.as_mut() {
            f.after = after;
        }
        report?;
        Ok(self.applied.clone())
    }

    /// Keep what is applied.
    ///
    /// Review AT-C, as P5's Keep (WP P5TRUST, coordinator decision (b)):
    /// **after a verify that did not pass** the first Keep is refused
    /// (nothing changes; `applied.message` says why) and only a second one,
    /// at least a second later on the user's explicit confirmation, keeps
    /// it. A new verify asks again.
    ///
    /// WP ATGUARD: a guarded apply is confirmed on the hub; one the hub
    /// ended by itself is taken as its outcome says.
    pub(crate) fn keep(&mut self, hub: &mut dyn Hub) -> Result<MeasureSystemApplied> {
        if !matches!(
            self.applied.stage,
            MeasureSystemApplyStage::Applied | MeasureSystemApplyStage::Verified
        ) {
            return Err(invalid("nothing is applied"));
        }
        if self.applied.verified == Some(false) {
            let now = crate::guarded::now();
            let min =
                std::time::Duration::from_millis(super::super::tune::TUNE_KEEP_CONFIRM_MIN_MS);
            let confirmed = self
                .keep_refused_at
                .is_some_and(|at| now.saturating_duration_since(at) >= min);
            if !confirmed {
                self.keep_refused_at.get_or_insert(now);
                let why = "the verify did not confirm the alignment; Keep it only if you are \
                           sure (call Keep again to keep it anyway, or Revert)";
                if !self.applied.message.contains(why) {
                    self.applied.message = if self.applied.message.is_empty() {
                        why.to_owned()
                    } else {
                        format!("{}; {why}", self.applied.message)
                    };
                }
                return Err(invalid(why));
            }
        }
        // -- WP ATGUARD --
        if let Some(g) = self.filed.as_ref().and_then(|f| f.guard) {
            match hub.request(msg_guard::CONFIRM, json!({ "guard": g })) {
                Ok(_) => {}
                Err(DspiError::Link { code, .. }) if code == "conflict" => {
                    let last = self.guard_last(hub, g);
                    let why = self.guard_ended(last);
                    if self.applied.stage == MeasureSystemApplyStage::Kept {
                        return Ok(self.applied.clone());
                    }
                    return Err(invalid(why));
                }
                Err(e) => return Err(e),
            }
        }
        self.applied.stage = MeasureSystemApplyStage::Kept;
        // -- review AT-D2 -- the apply's words ("... the hub reverts it by
        // itself unless it is kept within N s") are over
        self.applied.message = "kept: the alignment stays".into();
        Ok(self.applied.clone())
    }

    // -- WP ATGUARD --
    /// The hub's last guard outcome, if it is guard `g`'s.
    fn guard_last(&self, hub: &mut dyn Hub, g: u64) -> Option<msg_guard::Outcome> {
        hub.request(msg_guard::STATUS, json!({}))
            .ok()
            .and_then(|v| serde_json::from_value::<msg_guard::StatusReply>(v).ok())
            .and_then(|s| s.last)
            .filter(|o| o.guard == g)
    }

    /// Revert what is applied (Admin): the filed model again, aligned, and
    /// every unit's output delays read back against the filed ones.
    pub(crate) fn revert(&mut self, hub: &mut dyn Hub) -> Result<MeasureSystemApplied> {
        if !matches!(
            self.applied.stage,
            MeasureSystemApplyStage::Applied | MeasureSystemApplyStage::Verified
        ) {
            return Err(invalid("nothing is applied"));
        }
        if hub.role() != "admin" {
            return Err(DspiError::Denied);
        }
        let Some(f) = self.filed.clone() else {
            return Err(invalid("nothing was filed"));
        };
        // -- WP ATGUARD -- a guarded apply: the hub reverts it (the same
        // set and alignment) and the guard ends; one the hub has ended
        // already is read back as it left it.
        let (revision, ok, failures) = if let Some(g) = f.guard {
            let outcome = match hub.request(msg_guard::CANCEL, json!({ "guard": g })) {
                Ok(v) => serde_json::from_value::<msg_guard::Outcome>(v).map_err(|e| {
                    DspiError::Link {
                        code: "protocol".into(),
                        msg: format!("system.guard.cancel: {e}"),
                    }
                })?,
                Err(DspiError::Link { code, .. }) if code == "conflict" => {
                    match self.guard_last(hub, g) {
                        Some(o) if o.result == "kept" => {
                            let _ = self.guard_ended(Some(o));
                            return Err(invalid(
                                "the hub kept this alignment already: it cannot be reverted",
                            ));
                        }
                        Some(o) => o,
                        None => {
                            // -- WP REVERTW -- ended, how is not known
                            let why = self.guard_ended(None);
                            return Err(invalid(why));
                        }
                    }
                }
                Err(e) => return Err(e),
            };
            if outcome.result == "not_written" {
                // -- WP REVERTW -- the guard has ended with nothing set
                // back: the revert was not successful, and says so
                self.applied.revision_after = outcome.revision;
                self.applied.message = format!("not reverted: {}", outcome.message);
                self.revert_incomplete(crate::guarded::NOTICE_REVERT_NOT_WRITTEN);
                return Err(invalid(self.applied.message.clone()));
            }
            (
                Some(outcome.revision),
                outcome.writes_ok,
                outcome.write_failures,
            )
        } else {
            let mut fields = Map::new();
            fields.insert("model".into(), f.model.clone());
            fields.insert("revision".into(), self.applied.revision_after.into());
            let saved = hub.request("system.set", Value::Object(fields))?;
            let report = hub.request("system.align", json!({"apply": true}))?;
            let (ok, failures) = writes_of(&report);
            (saved.get("revision").and_then(Value::as_u64), ok, failures)
        };
        let now = self.snapshot(hub)?;
        let mut differ = Vec::new();
        for s in &self.applied.snapshots {
            let Some(n) = now.iter().find(|n| n.serial == s.serial) else {
                differ.push(format!("{}: not read back", s.serial));
                continue;
            };
            for d in &s.out_delays_ms {
                let back = n
                    .out_delays_ms
                    .iter()
                    .find(|x| x.output == d.output)
                    .map(|x| x.delay_ms);
                if back.is_none_or(|b| (b - d.delay_ms).abs() > 0.001) {
                    differ.push(format!(
                        "{} output {}: {} ms where {} ms was filed",
                        s.serial,
                        u16::from(d.output).saturating_add(1),
                        back.unwrap_or(f32::NAN),
                        d.delay_ms
                    ));
                }
            }
        }
        self.applied.stage = MeasureSystemApplyStage::Reverted;
        self.applied.revision_after = revision.unwrap_or(self.applied.revision_after);
        self.applied.writes_ok = ok;
        // -- WP REVERTW -- a failed write or a delay read back other than
        // filed: the revert was not fully successful
        let mut left: Vec<String> = failures
            .iter()
            .map(|f| format!("{f} (not written)"))
            .collect();
        left.extend(differ.iter().cloned());
        self.applied.write_failures = failures;
        self.applied.message = if left.is_empty() {
            "reverted: the model and every unit's output delays are as filed".into()
        } else {
            format!("reverted, but not wholly: {}", left.join("; "))
        };
        if !left.is_empty() {
            self.revert_incomplete(crate::guarded::NOTICE_REVERT_INCOMPLETE);
        }
        Ok(self.applied.clone())
    }
}

/// `system.align`'s writes: how many `ok` (or `unchanged`), the others in
/// words.
fn writes_of(report: &Value) -> (u32, Vec<String>) {
    let mut ok = 0u32;
    let mut bad = Vec::new();
    for w in report
        .get("writes")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let r = w.get("result").and_then(Value::as_str).unwrap_or("");
        if r == "ok" || r == "unchanged" {
            ok = ok.saturating_add(1);
        } else {
            bad.push(format!(
                "{} {}: {r}",
                w.get("serial").and_then(Value::as_str).unwrap_or("?"),
                w.get("what").and_then(Value::as_str).unwrap_or("?")
            ));
        }
    }
    (ok, bad)
}

// -- WP ATOVL --
/// Which plan each run used, in words ("run 2: ..." when more than one).
fn plan_words(runs: &[RunDone]) -> String {
    let words: Vec<String> = runs
        .iter()
        .map(|r| {
            if r.plan.is_empty() {
                r.analysis.overlap_ms.map_or_else(
                    || "interleaved passes".to_string(),
                    |ms| format!("overlapped starts, {ms:.0} ms apart"),
                )
            } else {
                r.plan.clone()
            }
        })
        .collect();
    match words.as_slice() {
        [one] => one.clone(),
        _ => words
            .iter()
            .enumerate()
            .map(|(i, w)| format!("run {}: {w}", i.saturating_add(1)))
            .collect::<Vec<_>>()
            .join("; "),
    }
}

fn unit_output(m: &Measured) -> UnitOutput {
    UnitOutput {
        unit: UnitIndex(u8::try_from(m.unit).unwrap_or(u8::MAX)),
        output: m.output,
    }
}

fn delay_of<'a>(r: &'a RunDone, a: &Measured, b: &Measured) -> Option<&'a CrossUnitDelay> {
    let (x, y) = (unit_output(a), unit_output(b));
    r.analysis
        .delays
        .iter()
        .find(|d| (d.from == x && d.to == y) || (d.from == y && d.to == x))
}

fn chained_of<'a>(c: &'a [ChainedOutput], m: &Measured) -> Option<&'a ChainedOutput> {
    c.iter().find(|x| x.output == unit_output(m))
}

/// The uncertainty of `a` against `b`, us.
fn pair_uncertainty(
    a: &Measured,
    b: &Measured,
    runs: &[RunDone],
    chained: Option<&[ChainedOutput]>,
) -> Option<f64> {
    if a.run == b.run {
        let r = runs.get(a.run)?;
        if a.unit == b.unit {
            // -- WP ATOVL -- two outputs of one unit never play together:
            // the absolute uncertainties (equal to the plain ones for
            // interleaved passes).
            let ua = r
                .analysis
                .output(unit_output(a).unit, a.output)?
                .absolute_uncertainty_us?;
            let ub = r
                .analysis
                .output(unit_output(b).unit, b.output)?
                .absolute_uncertainty_us?;
            return Some((ua * ua + ub * ub).sqrt());
        }
        return delay_of(r, a, b)?.uncertainty_us;
    }
    let c = chained?;
    let ua = chained_of(c, a)?.uncertainty_us?;
    let ub = chained_of(c, b)?.uncertainty_us?;
    Some((ua * ua + ub * ub).sqrt())
}

/// Whether `m`'s delay to the reference `l` is offered, its uncertainty and
/// why not.
fn pair_offer(
    m: &Measured,
    l: &Measured,
    runs: &[RunDone],
    chained: Option<&[ChainedOutput]>,
    chain_refused: &Option<String>,
) -> (bool, Option<f64>, String) {
    if let Some(why) = chain_refused {
        return (false, None, why.clone());
    }
    let same = m.unit == l.unit && m.output == l.output;
    let u = if same {
        Some(0.0)
    } else {
        pair_uncertainty(m, l, runs, chained)
    };
    if m.run == l.run {
        let Some(r) = runs.get(m.run) else {
            return (false, u, "no run".into());
        };
        if same {
            let ok = r.analysis.timing_reliable;
            let why = if ok {
                String::new()
            } else {
                format!(
                    "timing unreliable: {}",
                    r.analysis.reasons.first().cloned().unwrap_or_default()
                )
            };
            return (ok, u, why);
        }
        if m.unit == l.unit {
            // -- WP ATOVL -- the absolute gate (the run's own for
            // interleaved passes): an overlapped run never plays two
            // outputs of one unit together.
            let (ok, why) = r.analysis.same_unit_offer();
            return (ok, u, why);
        }
        return match delay_of(r, m, l) {
            Some(d) => (d.offered, d.uncertainty_us, d.reason.clone()),
            None => (false, u, "no delay to the reference".into()),
        };
    }
    let Some(c) = chained else {
        return (false, u, "the runs are not chained".into());
    };
    let (Some(cm), Some(cl)) = (chained_of(c, m), chained_of(c, l)) else {
        return (false, u, "not in the chain".into());
    };
    chained_pair(cm, cl)
}

/// Two outputs of different chained runs (review AT-A/AT-B): each offered
/// against the anchor **and** their own difference within 10 us (the two
/// uncertainties against the anchor in quadrature: two outputs each at
/// 10 us may differ by 14 us). The anchor itself counts 0 us.
pub(crate) fn chained_pair(cm: &ChainedOutput, cl: &ChainedOutput) -> (bool, Option<f64>, String) {
    let u = match (cm.uncertainty_us, cl.uncertainty_us) {
        (Some(a), Some(b)) => Some((a * a + b * b).sqrt()),
        _ => None,
    };
    let within = u.is_some_and(|u| u <= MAX_OFFERED_UNCERTAINTY_US);
    let ok = cm.offered && cl.offered && within;
    let why = if ok {
        String::new()
    } else if !(cm.offered && cl.offered) {
        "timing unreliable against the anchor output".into()
    } else {
        format!(
            "timing unreliable: {:.1} us against the latest output across the chained runs (at most 10)",
            u.unwrap_or(f64::NAN)
        )
    };
    (ok, u, why)
}

/// The additive `system` block of a unit's document (PLAN-AT 1.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SystemOut {
    output: u8,
    arrival_us: f64,
    uncertainty_us: Option<f64>,
    offered: bool,
}

fn documents(
    setup: &SetupPlan,
    runs_done: &[RunDone],
    measured: &[Measured],
    arrivals: &[MeasureSystemArrival],
) -> Result<Vec<MeasureSystemDocument>> {
    let secs = dspi_transport::clock::system_time()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let timing_reliable = runs_done.iter().all(|r| r.analysis.timing_reliable);
    let group: Vec<String> = setup.units.iter().map(|u| u.serial.clone()).collect();
    let anchor_serial = setup
        .units
        .get(usize::from(setup.anchor.unit.0))
        .map_or(String::new(), |u| u.serial.clone());
    let domain = runs_done.first().map(|r| r.schedule.domain);
    let run_ids: Vec<u32> = runs_done.iter().flat_map(|r| r.run_ids.clone()).collect();
    let mut docs = Vec::new();
    for (i, u) in setup.units.iter().enumerate() {
        let outputs: Vec<OutputResult> = measured
            .iter()
            .filter(|m| m.unit == i)
            .map(|m| m.result.clone())
            .collect();
        if outputs.is_empty() {
            continue;
        }
        let level = runs_done
            .iter()
            .flat_map(|r| r.levels.iter())
            .filter(|(x, _)| *x == i)
            .map(|(_, l)| *l)
            .fold(f32::INFINITY, f32::min);
        let rr = RunResult {
            position: 0,
            outputs,
            level_db: if level.is_finite() {
                level
            } else {
                u.plan.start_db
            },
            timing_reliable,
            max_residual_us: None,
            ratio_ppm: 0.0,
            timing: None,
            ramp_steps: Vec::new(),
            // -- WP ROLL1 --
            f1_hz: u.plan.sweep.f1_hz,
            aligns: true,
        };
        let chains = u.ctx.chains.clone();
        let ctx = DocContext {
            serial: &u.serial,
            platform: &u.ctx.platform,
            device_rate_hz: u.ctx.device_rate_hz,
            chains: &chains,
            saved_ms: i64::try_from(secs.saturating_mul(1000)).unwrap_or(i64::MAX),
            saved_utc: runs::utc_text(secs),
        };
        let text = runs::document(&u.plan, &[rr], &ctx)?;
        let e = |m: dspi_measure::types::MeasureError| invalid(m.to_string());
        let mut doc = dspi_measure::doc::MeasurementDoc::from_text(&text).map_err(e)?;
        let outs: Vec<SystemOut> = arrivals
            .iter()
            .filter(|a| a.serial == u.serial)
            .map(|a| SystemOut {
                output: a.output,
                arrival_us: a.arrival_us,
                uncertainty_us: a.uncertainty_us,
                offered: a.offered,
            })
            .collect();
        let block = dspi_measure::doc::SystemBlock {
            group: group.clone(),
            time_base: domain.map_or(0, |d| d.time_base),
            domain: domain.map_or(0, |d| d.id),
            anchor_serial: anchor_serial.clone(),
            anchor_output: setup.anchor.output,
            run_ids: run_ids.clone(),
            outputs: serde_json::to_value(&outs).unwrap_or(Value::Null),
        };
        doc.measurement.set_system(&block);
        docs.push(MeasureSystemDocument {
            serial: u.serial.clone(),
            document: doc.to_text().map_err(e)?,
        });
    }
    Ok(docs)
}
