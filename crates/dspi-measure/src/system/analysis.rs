//! The analyser over segments (WP ATSYS, PLAN-AT section 1.4): one timeline
//! across units.
//!
//! A run's segments are laid out on the domain's time base as **one walk**:
//! every cycle of every segment at its domain sample `T_s + start_c`, each
//! (unit, output) a *key* of the walk. P5's walked analyser
//! ([`crate::ir::WalkedAnalyser`]) then runs over that walk unchanged: the
//! first arrival in the window the time pin implies, every later cycle where
//! the timeline puts it, the noise-weighted envelope and the leave-one-out
//! refinement per cycle, **one common slope** (capture frames per domain
//! sample: the clock domain's ratio against the microphone) and **one
//! intercept per (unit, output)**, the ratio removed per cycle in the
//! frequency domain, the repeats averaged coherently, and P5's timing gate
//! (the residuals, the phase agreement, the wander and slip findings). A
//! single segment is that segment's own walk, so a single unit's result is
//! P5's, bit for bit.
//!
//! On top of P5's result this module reports what the timing can deliver
//! across units (review R8): an **uncertainty per (unit, output)** (the
//! spread of its per-pass intercepts about the fit, as a standard error) and
//! per cross-unit difference (both standard errors and the slope's), and
//! **offers a cross-unit delay only within [`MAX_OFFERED_UNCERTAINTY_US`]**.
//!
//! Limits, typed errors: at most [`MAX_RUN_OUTPUTS`] keys in one run (the
//! generator's mask is 16 bits; a larger group is split into runs chained
//! through an anchor output, [`super::chain`]), one sweep for every segment,
//! every key measured the same number of passes, the cycles of the timeline
//! at least one period (sweep plus gap) apart.
//!
//! Approximation: each segment is a new generator start, whose first cycle
//! carries the generator's 5 ms start fade; P5's analyser models the fade on
//! the run's first cycle only. On a later segment's first cycle the
//! difference is the first 5 ms of the sweep's lowest octave (about -35 dB
//! of it, below the trusted band's use).

use crate::ir::{
    AnalysisOptions, ClockFit, OutputIr, OverlapParts, OverlapSpec, StatusObservation, Step,
    TimingError, WalkedAnalyser, WalkedAnalysis, WalkedCapture,
};
use crate::stimulus::{Cycle, WalkSchedule};
use crate::types::{MeasureError, Response};

use super::{SystemSchedule, TimePin, UnitIndex};

/// The most (unit, output) keys one run analyses (the walk's mask is 16
/// bits wide).
pub const MAX_RUN_OUTPUTS: usize = 16;
/// The largest uncertainty of a cross-unit delay that is offered, µs (G5's
/// bound, review R8a).
pub const MAX_OFFERED_UNCERTAINTY_US: f64 = 10.0;
/// The arrival estimator's resolution, µs: added (in quadrature) to every
/// key's uncertainty, so that a run whose passes agree to a nanosecond (a
/// clean simulated world) still reports what one arrival can resolve. The
/// envelope peak of the averaged, drift-corrected response and the fit's
/// intercepts differ by a few hundredths of a µs in such a world.
pub const ARRIVAL_RESOLUTION_US: f64 = 0.25;
/// A transition band: where both outputs are within this of their own
/// passband levels, dB (review R8d).
pub const TRANSITION_WITHIN_DB: f64 = 10.0;
/// A transition band is at least this wide, octaves.
pub const TRANSITION_MIN_OCTAVES: f64 = 1.0;

/// One output of one unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnitOutput {
    /// The unit.
    pub unit: UnitIndex,
    /// Its output.
    pub output: u8,
}

/// A unit's `SIGGEN_GET_STATUS` reply (from `system.measure.status`) and the
/// capture frame the client had reached when it arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitStatusObservation {
    /// The unit that answered.
    pub unit: UnitIndex,
    /// What it said, and when.
    pub observation: StatusObservation,
}

/// The capture of one system run, as the session holds it.
#[derive(Debug, Clone, Copy)]
pub struct SystemCapture<'a> {
    /// Mono samples, full scale ±1, contiguous (checked with
    /// [`crate::ir::FrameContinuity`]), starting with the pre-roll.
    pub samples: &'a [f32],
    /// The frame index of `samples[0]`.
    pub first_frame: u64,
    /// The capture's sample rate, Hz.
    pub rate_hz: u32,
    /// The run (the analyser refuses a capture of another schedule).
    pub schedule: &'a SystemSchedule,
    /// Every time pin of the run; the one with the shortest round trip
    /// places the timeline on the capture.
    pub pins: &'a [TimePin],
    /// Every unit's status polls of the run.
    pub status: &'a [UnitStatusObservation],
    /// No more samples will come.
    pub complete: bool,
}

/// One key of a run: a (unit, output), or for a summed run the outputs that
/// played together at one place of the walk.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemOutput {
    /// Its label in [`SystemAnalysis::walked`] (`OutputIr::output`,
    /// `CycleTiming::output`, the typed findings): the output itself for a
    /// single segment, the key's index otherwise.
    pub key: u8,
    /// What played: one (unit, output), or several for a summed run.
    pub members: Vec<UnitOutput>,
    /// The fit's intercept: the capture frame (absolute) at which a cycle
    /// started at the run's first domain sample would arrive.
    pub intercept_frame: f64,
    /// The arrival (the peak of the noise-weighted envelope of the averaged,
    /// drift-corrected response) after the run's earliest key's, ms (P5's
    /// `OutputIr::arrival_ms`).
    pub arrival_ms: f64,
    /// The onset on the same scale, ms.
    pub onset_ms: f64,
    /// Passes averaged.
    pub passes: u16,
    /// The standard error of its intercept from the spread of its passes
    /// about the fit (at least what the run's pooled scatter gives a key of
    /// as many passes), with [`ARRIVAL_RESOLUTION_US`] in quadrature, µs;
    /// `None` with fewer than two clear passes. One standard error from few
    /// passes: an error of three of it is not rare with three passes.
    pub uncertainty_us: Option<f64>,
    // -- WP ATLATE --
    /// What [`SystemOutput::arrival_ms`] and [`SystemOutput::onset_ms`] hold
    /// beyond the analysed response's own timing, ms: the unit's expected
    /// lateness ([`SystemSchedule::with_lateness`]) as the analysis looked
    /// for it, less what the run's earliest output's arrival then held (so it
    /// may be negative). 0 without lateness. The response
    /// ([`SystemAnalysis::output_ir`]) is on the analysis' timeline, so it
    /// is this much early against the arrival.
    pub lateness_ms: f64,
    // -- WP ATOVL --
    /// [`SystemOutput::uncertainty_us`] from every cycle of the key against
    /// the absolute fit, the capture clock's wander left in: what a
    /// difference to an output it did not play together with carries (an
    /// output of its own unit). Equal to `uncertainty_us` for every run but
    /// an overlapped one.
    pub absolute_uncertainty_us: Option<f64>,
}

impl SystemOutput {
    /// "unit 1 output 2", or the members joined with "+" for a sum.
    pub fn name(&self) -> String {
        self.members
            .iter()
            .map(|m| format!("unit {} output {}", m.unit.0, m.output))
            .collect::<Vec<_>>()
            .join(" + ")
    }
}

/// A delay between two outputs on different units.
#[derive(Debug, Clone, PartialEq)]
pub struct CrossUnitDelay {
    /// The earlier output in the run's key order.
    pub from: UnitOutput,
    /// The other.
    pub to: UnitOutput,
    /// `to`'s arrival minus `from`'s, ms.
    pub delay_ms: f64,
    /// The uncertainty of that difference (one standard error), µs; `None`
    /// when either output has fewer than two clear passes.
    pub uncertainty_us: Option<f64>,
    /// The run's timing is reliable (G5) and the uncertainty is at most
    /// [`MAX_OFFERED_UNCERTAINTY_US`]: a proposal may use it.
    pub offered: bool,
    /// Why not offered (empty when offered).
    pub reason: String,
}

/// The result of a system run.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemAnalysis {
    /// P5's analysis of the run's timeline walk: per key the impulse
    /// response, its noise floor, spread and phase check; per cycle its
    /// arrival and residual; P5's timing gate. Its outputs are labelled by
    /// [`SystemOutput::key`].
    pub walked: WalkedAnalysis,
    /// Every key, in [`SystemAnalysis::walked`]'s output order.
    pub outputs: Vec<SystemOutput>,
    /// A summed run.
    pub summed: bool,
    /// The domain sample every intercept refers to: the run's first segment
    /// start.
    pub t_ref: u64,
    /// The pin that placed the timeline.
    pub pin: TimePin,
    /// The clock domain's ratio against the microphone (one slope).
    pub clock: ClockFit,
    /// The standard error of the slope, µs per second of the domain; `None`
    /// without enough points.
    pub slope_uncertainty_us_per_s: Option<f64>,
    /// Every pair of keys on different units (none for a summed run).
    pub delays: Vec<CrossUnitDelay>,
    /// G5: P5's timing gate passed for the run.
    pub timing_reliable: bool,
    /// Why not reliable, and every delay not offered, in words naming units
    /// and outputs.
    pub reasons: Vec<String>,
    // -- WP ATOVL --
    /// The run's units started overlapped this far apart, ms (overlapped
    /// starts); `None`: interleaved passes (or a summed or single run).
    pub overlap_ms: Option<f64>,
    /// P5's gate on every cycle against the absolute fit (the wander left
    /// in): what a difference between outputs that did not play together
    /// (two of one unit, or two that never shared a group) may rely on.
    /// Equal to [`SystemAnalysis::timing_reliable`] for every run but an
    /// overlapped one, whose `timing_reliable` is the gate on what each
    /// group measures.
    pub absolute_reliable: bool,
    /// Why the absolute gate failed (empty when it passed).
    pub absolute_reasons: Vec<String>,
}

impl SystemAnalysis {
    /// The key of a unit's output.
    pub fn output(&self, unit: UnitIndex, output: u8) -> Option<&SystemOutput> {
        let want = UnitOutput { unit, output };
        self.outputs
            .iter()
            .find(|o| o.members.len() == 1 && o.members.first() == Some(&want))
    }

    /// P5's result of a key.
    pub fn output_ir(&self, key: u8) -> Option<&OutputIr> {
        self.walked.outputs.iter().find(|o| o.output == key)
    }

    /// The delays a proposal may use.
    pub fn offered(&self) -> impl Iterator<Item = &CrossUnitDelay> {
        self.delays.iter().filter(|d| d.offered)
    }

    // -- WP ATOVL --
    /// Whether a difference between two outputs of one unit may be used,
    /// and why not: the absolute gate (P5's), as an overlapped run never
    /// plays two outputs of one unit together.
    pub fn same_unit_offer(&self) -> (bool, String) {
        if self.absolute_reliable {
            (true, String::new())
        } else {
            (
                false,
                format!(
                    "timing unreliable: {}",
                    self.absolute_reasons
                        .first()
                        .map_or("the timing gate failed", String::as_str)
                ),
            )
        }
    }
}

/// What one analysis slice did.
#[derive(Debug, Clone, PartialEq)]
pub enum SystemStep {
    /// More slices to go.
    Progress {
        /// Slices done.
        done: usize,
        /// Slices in all.
        total: usize,
    },
    /// The next slice needs capture (or a time pin) that has not arrived yet.
    NeedCapture,
    /// Finished.
    Done(Box<SystemAnalysis>),
}

/// The run as one walk.
#[derive(Debug, Clone)]
struct Layout {
    walk: WalkSchedule,
    /// Per key, in walk order: its label and members.
    keys: Vec<(u8, Vec<UnitOutput>)>,
    t_ref: u64,
    /// A single segment: its walk is the run's (status polls go to P5).
    single: Option<UnitIndex>,
    // -- WP ATLATE --
    /// Per key (walk order): how much later than the run's first unit its
    /// unit is expected (negative: earlier), domain samples; its cycles are
    /// laid out that much later. The first unit's cycles are where they are
    /// played, so the walk's first cycle is at its START as without
    /// lateness (the time pin's search, the status polls).
    late: Vec<i64>,
    // -- review AT-E --
    /// How long after `t_ref` the walk's first cycle is expected, beyond
    /// what every unit shares, domain samples: the first unit's lateness
    /// over the run's earliest unit (WP ATLATE), or for a summed run how
    /// much later its last part starts (the parts are started apart so that
    /// they are heard together, WP ATSUMOFF). The first cycle is searched
    /// for, and the status polls are judged, that much later; without it a
    /// first unit (or a sum) some 0.5 s late was "not heard in the window
    /// the START implies" and one 0.3 s late failed its status polls.
    first_late: u64,
    // -- WP ATOVL -- overlapped starts: the groups and windows.
    overlap: Option<OverlapSpec>,
}

fn invalid(m: impl Into<String>) -> MeasureError {
    MeasureError::Invalid(m.into())
}

fn layout(schedule: &SystemSchedule) -> Result<Layout, MeasureError> {
    let segments = schedule.segments();
    let Some(first) = segments.first() else {
        return Err(invalid("no segment"));
    };
    let t_ref = schedule.start();
    if segments
        .iter()
        .any(|s| s.schedule.sweep != first.schedule.sweep)
    {
        return Err(invalid("every segment of a run must play the same sweep"));
    }
    // -- WP ATOVL --
    if let Some(offset) = schedule.overlap_samples() {
        return layout_overlapped(schedule, offset);
    }
    if schedule.is_summed() {
        // Every part plays the same walk at once: place j of the walk is the
        // sum of every part's j-th output.
        let n = first.schedule.outputs().len();
        if segments
            .iter()
            .any(|s| s.schedule.outputs().len() != n || s.schedule.repeat != first.schedule.repeat)
        {
            return Err(invalid(
                "the parts of a summed run must walk as many outputs, as often",
            ));
        }
        let keys: Vec<(u8, Vec<UnitOutput>)> = (0..n)
            .map(|j| {
                let members = segments
                    .iter()
                    .filter_map(|s| {
                        s.schedule.outputs().get(j).map(|&output| UnitOutput {
                            unit: s.unit,
                            output,
                        })
                    })
                    .collect();
                (u8::try_from(j).unwrap_or(u8::MAX), members)
            })
            .collect();
        let outs = first.schedule.outputs();
        let cycles: Vec<Cycle> = first
            .schedule
            .cycles()
            .iter()
            .map(|c| Cycle {
                output: outs
                    .iter()
                    .position(|&o| o == c.output)
                    .and_then(|p| u8::try_from(p).ok())
                    .unwrap_or(0),
                ..*c
            })
            .collect();
        let mask = mask_of(n)?;
        return Ok(Layout {
            walk: WalkSchedule::from_cycles(
                first.schedule.sweep,
                mask,
                first.schedule.repeat,
                cycles,
            ),
            keys,
            t_ref,
            single: None,
            late: vec![0; n],
            overlap: None,
            first_late: segments
                .iter()
                .map(|s| s.start)
                .max()
                .unwrap_or(t_ref)
                .saturating_sub(t_ref),
        });
    }
    if let [only] = segments {
        let keys = only
            .schedule
            .outputs()
            .into_iter()
            .map(|output| {
                (
                    output,
                    vec![UnitOutput {
                        unit: only.unit,
                        output,
                    }],
                )
            })
            .collect();
        let n = only.schedule.outputs().len();
        return Ok(Layout {
            walk: only.schedule.clone(),
            keys,
            t_ref,
            single: Some(only.unit),
            late: vec![0; n],
            first_late: 0,
            overlap: None,
        });
    }
    let mut keys: Vec<(u8, Vec<UnitOutput>)> = Vec::new();
    let mut counts: Vec<u16> = Vec::new();
    let mut cycles: Vec<Cycle> = Vec::new();
    let period = first.schedule.period_samples();
    let mut last_start: Option<u64> = None;
    // -- WP ATLATE -- each unit's cycles where its sound is expected, against
    // the run's first unit.
    let rel = |u: UnitIndex| {
        i64::try_from(schedule.lateness_of(u))
            .unwrap_or(i64::MAX)
            .saturating_sub(i64::try_from(schedule.lateness_of(first.unit)).unwrap_or(i64::MAX))
    };
    for s in segments {
        let off = i128::from(s.start.saturating_sub(t_ref)).saturating_add(i128::from(rel(s.unit)));
        let off = u64::try_from(off).map_err(|_| {
            invalid(format!(
                "unit {} is expected so much earlier than the run's first unit that it would be \
                 heard before the run",
                s.unit.0
            ))
        })?;
        for c in s.schedule.cycles() {
            let uo = UnitOutput {
                unit: s.unit,
                output: c.output,
            };
            let k = match keys.iter().position(|(_, m)| m.first() == Some(&uo)) {
                Some(k) => k,
                None => {
                    if keys.len() >= MAX_RUN_OUTPUTS {
                        return Err(invalid(format!(
                            "more than {MAX_RUN_OUTPUTS} outputs in one run: split it into runs through an anchor output"
                        )));
                    }
                    keys.push((u8::try_from(keys.len()).unwrap_or(u8::MAX), vec![uo]));
                    counts.push(0);
                    keys.len().saturating_sub(1)
                }
            };
            let start = off.saturating_add(c.start);
            if let Some(prev) = last_start
                && start < prev.saturating_add(period)
            {
                return Err(invalid(
                    "two cycles of the run closer than one sweep and its gap",
                ));
            }
            last_start = Some(start);
            let pass = counts.get(k).copied().unwrap_or(0);
            if let Some(slot) = counts.get_mut(k) {
                *slot = pass.saturating_add(1);
            }
            cycles.push(Cycle {
                index: u32::try_from(cycles.len()).unwrap_or(u32::MAX),
                output: u8::try_from(k).unwrap_or(u8::MAX),
                pass,
                start,
            });
        }
    }
    let repeat = counts.first().copied().unwrap_or(0);
    if counts.iter().any(|&c| c != repeat) {
        return Err(invalid(
            "every output of a run must be measured the same number of passes",
        ));
    }
    let mask = mask_of(keys.len())?;
    let late = keys
        .iter()
        .map(|(_, m)| m.first().map_or(0, |uo| rel(uo.unit)))
        .collect();
    Ok(Layout {
        walk: WalkSchedule::from_cycles(first.schedule.sweep, mask, repeat, cycles),
        keys,
        t_ref,
        single: None,
        late,
        first_late: schedule.lateness_of(first.unit),
        overlap: None,
    })
}

// -- WP ATOVL --
/// The layout of an overlapped run (overlapped starts): every cycle where
/// it is heard (its segment's start plus its unit's expected lateness, from
/// the run's first heard sound), in time order; each unit's first segment
/// (the interleaved first pass) in groups of one, and in every later pass
/// the units' `j`-th cycles one group. Refused (before anything plays, by
/// [`overlap_layout_check`]) when the analysis could not tell the units of
/// a group apart: windows shorter than
/// [`super::plan::OVERLAP_MIN_TAIL_MS`], an impulse or harmonic image of
/// one unit in another's window, or a group's cut reaching another group's
/// sweep.
fn layout_overlapped(schedule: &SystemSchedule, _offset: u64) -> Result<Layout, MeasureError> {
    use super::plan::{
        OVERLAP_GUARD_MS, OVERLAP_HARMONICS, OVERLAP_MIN_TAIL_MS, OVERLAP_SPREAD_MS,
    };
    let segments = schedule.segments();
    let Some(first) = segments.first() else {
        return Err(invalid("no segment"));
    };
    if segments
        .iter()
        .any(|s| s.schedule.sweep != first.schedule.sweep)
    {
        return Err(invalid("every segment of a run must play the same sweep"));
    }
    let sweep = first.schedule.sweep;
    let t_ref = schedule.start();
    let heard = |s: &super::Segment| s.start.saturating_add(schedule.lateness_of(s.unit));
    let base = segments.iter().map(heard).min().unwrap_or(t_ref);
    let mut keys: Vec<(u8, Vec<UnitOutput>)> = Vec::new();
    let mut pass_of: Vec<(UnitIndex, u32)> = Vec::new();
    // (heard position, key, group)
    let mut raw: Vec<(u64, usize, u32)> = Vec::new();
    for s in segments {
        let pass = match pass_of.iter_mut().find(|(u, _)| *u == s.unit) {
            Some(slot) => {
                slot.1 = slot.1.saturating_add(1);
                slot.1
            }
            None => {
                pass_of.push((s.unit, 0));
                0
            }
        };
        let off = heard(s).saturating_sub(base);
        for (j, c) in s.schedule.cycles().iter().enumerate() {
            let uo = UnitOutput {
                unit: s.unit,
                output: c.output,
            };
            let k = match keys.iter().position(|(_, m)| m.first() == Some(&uo)) {
                Some(k) => k,
                None => {
                    if keys.len() >= MAX_RUN_OUTPUTS {
                        return Err(invalid(format!(
                            "more than {MAX_RUN_OUTPUTS} outputs in one run: split it into runs through an anchor output"
                        )));
                    }
                    keys.push((u8::try_from(keys.len()).unwrap_or(u8::MAX), vec![uo]));
                    keys.len().saturating_sub(1)
                }
            };
            let group = if pass == 0 {
                (1u32 << 20).saturating_add(u32::try_from(raw.len()).unwrap_or(u32::MAX))
            } else {
                pass.saturating_mul(64)
                    .saturating_add(u32::try_from(j).unwrap_or(63))
            };
            raw.push((off.saturating_add(c.start), k, group));
        }
    }
    raw.sort_by_key(|r| r.0);
    let mut counts = vec![0u16; keys.len()];
    let mut cycles = Vec::with_capacity(raw.len());
    let mut group = Vec::with_capacity(raw.len());
    for (i, &(pos, k, g)) in raw.iter().enumerate() {
        let pass = counts.get(k).copied().unwrap_or(0);
        if let Some(slot) = counts.get_mut(k) {
            *slot = pass.saturating_add(1);
        }
        cycles.push(Cycle {
            index: u32::try_from(i).unwrap_or(u32::MAX),
            output: u8::try_from(k).unwrap_or(u8::MAX),
            pass,
            start: pos,
        });
        group.push(g);
    }
    let repeat = counts.first().copied().unwrap_or(0);
    if counts.iter().any(|&c| c != repeat) {
        return Err(invalid(
            "every output of a run must be measured the same number of passes",
        ));
    }
    // The windows: the closest members of any group, less the guard.
    let rate = f64::from(sweep.rate_hz.max(1));
    let ms = |x: f64| x * 1000.0 / rate;
    let mut ids: Vec<u32> = group.clone();
    ids.sort_unstable();
    ids.dedup();
    let members = |g: u32| -> Vec<u64> {
        let mut v: Vec<u64> = cycles
            .iter()
            .zip(&group)
            .filter(|(_, x)| **x == g)
            .map(|(c, _)| c.start)
            .collect();
        v.sort_unstable();
        v
    };
    let closest = ids
        .iter()
        .map(|&g| members(g))
        .filter(|v| v.len() >= 2)
        .flat_map(|v| {
            v.windows(2)
                .map(|w| {
                    w.get(1)
                        .copied()
                        .unwrap_or(0)
                        .saturating_sub(w.first().copied().unwrap_or(0))
                })
                .collect::<Vec<u64>>()
        })
        .min()
        .ok_or_else(|| invalid("an overlapped run without a group of two units"))?;
    let (pre, p5_tail) = super::plan::overlap_window_ms(&sweep, closest);
    let tail = p5_tail;
    let (spread, guard) = (OVERLAP_SPREAD_MS, OVERLAP_GUARD_MS);
    if tail < OVERLAP_MIN_TAIL_MS || ms(closest as f64) <= pre + spread + guard {
        return Err(invalid(format!(
            "units {:.1} ms apart in an overlapped group: too close to tell apart",
            ms(closest as f64)
        )));
    }
    // Every impulse and harmonic image clear of every other member's window.
    let ratio = sweep.end_freq_hz() / sweep.start_freq_hz();
    let leads: Vec<f64> = (2..=OVERLAP_HARMONICS)
        .map(|k| ms(f64::from(sweep.cycle_samples) * f64::from(k).ln() / ratio.ln()))
        .collect();
    for &g in &ids {
        let v = members(g);
        for (i, &a) in v.iter().enumerate() {
            for &b in v.iter().skip(i.saturating_add(1)) {
                let d = ms(b.saturating_sub(a) as f64);
                for h in &leads {
                    let (lo, hi) = (d - h, d - h + spread);
                    if hi > -pre - guard && lo < tail + guard {
                        return Err(invalid(format!(
                            "a harmonic image of a unit {d:.0} ms later in an overlapped group \
                             falls {lo:.0} ms from an arrival, inside its window"
                        )));
                    }
                }
            }
        }
    }
    // A group's cut (its members' sweeps whole, the windows, the spread)
    // never reaches another group's sweep; lone cycles keep P5's rule.
    let cycle = f64::from(sweep.cycle_samples);
    let mut spans: Vec<(u64, u64, bool)> = ids
        .iter()
        .map(|&g| {
            let v = members(g);
            (
                v.first().copied().unwrap_or(0),
                v.last().copied().unwrap_or(0),
                v.len() >= 2,
            )
        })
        .collect();
    spans.sort_by_key(|x| x.0);
    let period = f64::from(sweep.cycle_samples) + f64::from(sweep.gap_samples);
    for w in spans.windows(2) {
        let (Some(a), Some(b)) = (w.first(), w.get(1)) else {
            continue;
        };
        let apart = b.0.saturating_sub(a.1) as f64;
        let need = if a.2 || b.2 {
            cycle + (pre.max(tail) + spread + 2.0 * guard) * rate / 1000.0
        } else {
            period
        };
        if apart + 0.5 < need {
            return Err(invalid(
                "two groups of an overlapped run closer than their sweeps and windows",
            ));
        }
    }
    let mask = mask_of(keys.len())?;
    let late: Vec<i64> = keys
        .iter()
        .map(|(_, m)| {
            m.first().map_or(0, |uo| {
                i64::try_from(schedule.lateness_of(uo.unit)).unwrap_or(0)
            })
        })
        .collect();
    Ok(Layout {
        walk: WalkSchedule::from_cycles(sweep, mask, repeat, cycles),
        keys,
        t_ref,
        single: None,
        late,
        first_late: base.saturating_sub(t_ref),
        overlap: Some(OverlapSpec {
            group,
            pre_ms: pre,
            tail_ms: tail,
            spread_ms: spread,
            guard_ms: guard,
            harmonics: OVERLAP_HARMONICS,
        }),
    })
}

// -- WP ATOVL --
/// Whether the analysis can lay out the overlapped run `schedule`
/// ([`super::SystemSchedule::overlapped`]); the planner refuses what it
/// cannot, before anything plays.
///
/// Errors: [`MeasureError::Invalid`] naming why.
pub(crate) fn overlap_layout_check(schedule: &SystemSchedule) -> Result<(), MeasureError> {
    match schedule.overlap_samples() {
        Some(o) => layout_overlapped(schedule, o).map(|_| ()),
        None => Err(invalid("not an overlapped run")),
    }
}

fn mask_of(n: usize) -> Result<u16, MeasureError> {
    if n == 0 || n > MAX_RUN_OUTPUTS {
        return Err(invalid(format!("{n} outputs in a run (1 to 16)")));
    }
    let bits = 1u32
        .checked_shl(u32::try_from(n).unwrap_or(32))
        .unwrap_or(0);
    u16::try_from(bits.wrapping_sub(1)).map_err(|_| invalid("mask"))
}

/// Analyses one system run in slices: [`SystemAnalyser::step`] does about
/// one cycle's FFT per call, as [`WalkedAnalyser`] does.
pub struct SystemAnalyser {
    schedule: SystemSchedule,
    layout: Layout,
    inner: WalkedAnalyser,
    options: AnalysisOptions,
    rate: f64,
    nominal: f64,
    pin: Option<TimePin>,
    status_checked: bool,
}

impl SystemAnalyser {
    /// Prepares the analysis of `schedule` captured at `capture_rate_hz`.
    ///
    /// Errors: [`MeasureError::Invalid`] for a run the analyser cannot lay
    /// out as one walk (see the module), or what [`WalkedAnalyser::new`]
    /// refuses.
    pub fn new(
        schedule: &SystemSchedule,
        capture_rate_hz: u32,
        options: &AnalysisOptions,
    ) -> Result<SystemAnalyser, MeasureError> {
        let layout = layout(schedule)?;
        // -- WP ATOVL --
        let inner = match &layout.overlap {
            Some(spec) => WalkedAnalyser::new_overlapped(
                &layout.walk,
                capture_rate_hz,
                options,
                spec.clone(),
            )?,
            None => WalkedAnalyser::new(&layout.walk, capture_rate_hz, options)?,
        };
        let rate = f64::from(capture_rate_hz);
        let nominal = rate / f64::from(schedule.rate_hz.max(1));
        Ok(SystemAnalyser {
            schedule: schedule.clone(),
            layout,
            inner,
            options: options.clone(),
            rate,
            nominal,
            pin: None,
            status_checked: false,
        })
    }

    /// The pin a capture's pins give: the shortest round trip, at most the
    /// capture latency P5 allows before a START (`start_early_ms`).
    fn best_pin(&self, pins: &[TimePin]) -> Option<TimePin> {
        let bound = self.options.start_early_ms * self.rate / 1000.0;
        pins.iter()
            .filter(|p| (p.round_trip_frames as f64) <= bound)
            .min_by_key(|p| p.round_trip_frames)
            .copied()
    }

    /// The capture frame of the run's first domain sample, from a pin.
    fn start_frame(&self, pin: &TimePin) -> Result<u64, TimingError> {
        // Two u64 as i128: the difference cannot overflow.
        // -- review AT-E -- where the first cycle is expected (`first_late`).
        let dt = i128::from(self.layout.t_ref)
            .saturating_add(i128::from(self.layout.first_late))
            .saturating_sub(i128::from(pin.t));
        let f = pin.capture_frame as f64 + dt as f64 * self.nominal;
        if !f.is_finite() || f < 0.0 || f > 1.8e19 {
            return Err(TimingError::CycleIdentity(
                "the time pin puts the run's start outside the capture".into(),
            ));
        }
        Ok(f.round() as u64)
    }

    /// Does the next slice. Errors end the analysis: a
    /// [`TimingError::CycleIdentity`] means the run must be discarded.
    pub fn step(&mut self, capture: &SystemCapture<'_>) -> Result<SystemStep, TimingError> {
        if *capture.schedule != self.schedule {
            return Err(MeasureError::Invalid("a capture of another run".into()).into());
        }
        // The pin is fixed by the first slice that needs it (the search).
        let pin = match self.pin {
            Some(p) => p,
            None => match self.best_pin(capture.pins) {
                Some(p) => p,
                None if capture.complete => {
                    return Err(TimingError::CycleIdentity(
                        "no time pin places the run on the capture (none with a round trip under the capture latency allowed)".into(),
                    ));
                }
                None => return Ok(SystemStep::NeedCapture),
            },
        };
        let start_frame = self.start_frame(&pin)?;
        let status: Vec<StatusObservation> = match self.layout.single {
            Some(unit) => capture
                .status
                .iter()
                .filter(|s| s.unit == unit)
                .map(|s| s.observation)
                .collect(),
            None => Vec::new(),
        };
        let walked = WalkedCapture {
            samples: capture.samples,
            first_frame: capture.first_frame,
            rate_hz: capture.rate_hz,
            start_frame,
            status: &status,
            complete: capture.complete,
        };
        let step = self.inner.step(&walked)?;
        if !matches!(step, Step::NeedCapture) {
            self.pin = Some(pin);
        }
        if self.layout.single.is_none()
            && !self.status_checked
            && let Some(fit) = self.inner.fit_parts()
        {
            self.check_status(capture, &fit)?;
            self.status_checked = true;
        }
        Ok(match step {
            Step::Progress { done, total } => SystemStep::Progress { done, total },
            Step::NeedCapture => SystemStep::NeedCapture,
            Step::Done(w) => SystemStep::Done(Box::new(self.finish(*w, pin)?)),
        })
    }

    /// Every unit's polls against its own segments (the per-unit status
    /// cross-check, PLAN-AT section 1.4). A poll before the unit's first
    /// segment is not checked: its generator has not started (the timeline
    /// read-back, G3, proves the start).
    fn check_status(
        &self,
        capture: &SystemCapture<'_>,
        fit: &crate::ir::FitParts,
    ) -> Result<(), TimingError> {
        let slack = self.options.status_slack_ms * f64::from(self.schedule.rate_hz) / 1000.0;
        for obs in capture.status {
            let rel = obs.observation.capture_frame as f64 - fit.first_frame;
            // -- review AT-E -- the fit's origin is where the walk's first
            // cycle is heard: `first_late` after `t_ref` on the timeline.
            let d = (rel - fit.origin) / fit.slope + self.layout.first_late as f64;
            if !d.is_finite() {
                continue;
            }
            let mine: Vec<_> = self
                .schedule
                .segments()
                .iter()
                .filter(|s| s.unit == obs.unit)
                .collect();
            let off = |s: &super::Segment| s.start.saturating_sub(self.layout.t_ref) as f64;
            let Some(first) = mine.first() else {
                return Err(TimingError::CycleIdentity(format!(
                    "a status poll of unit {}, which plays no segment",
                    obs.unit.0
                )));
            };
            if d + slack < off(first) {
                continue;
            }
            let ok = mine.iter().any(|s| {
                let o = off(s);
                s.schedule
                    .status_consistent(d - o - slack, d - o + slack, obs.observation.status)
            });
            if !ok {
                return Err(TimingError::CycleIdentity(format!(
                    "unit {} said output {} after {} cycles at frame {}, which none of its segments plays there",
                    obs.unit.0,
                    obs.observation.status.active_channel,
                    obs.observation.status.cycles_done,
                    obs.observation.capture_frame
                )));
            }
        }
        Ok(())
    }

    fn finish(&self, walked: WalkedAnalysis, pin: TimePin) -> Result<SystemAnalysis, TimingError> {
        let Some(fit) = self.inner.fit_parts() else {
            return Err(MeasureError::Invalid("analysis not fitted".into()).into());
        };
        let parts = self.inner.overlap_parts();
        Ok(build(
            &self.layout,
            walked,
            &fit,
            pin,
            self.rate,
            parts.as_ref(),
            self.schedule.overlap_samples(),
        ))
    }
}

/// The per-key uncertainty and the slope's, from the cycles' residuals.
struct Spread {
    /// Per key (walk order): standard error µs, mean device sample.
    per_key: Vec<(Option<f64>, f64)>,
    /// The slope's standard error, µs per device sample.
    slope_us_per_sample: Option<f64>,
}

fn spread(layout: &Layout, walked: &WalkedAnalysis) -> Spread {
    // -- WP ATOVL -- (the residuals given: as before, every clear cycle's)
    let res: Vec<Option<f64>> = walked
        .cycles
        .iter()
        .map(|t| (!t.arrival_ambiguous && t.residual_us.is_finite()).then_some(t.residual_us))
        .collect();
    spread_of(layout, walked, &res)
}

// -- WP ATOVL --
/// [`spread`] of the residuals `residual` (µs per cycle of the walk; `None`:
/// left out).
fn spread_of(layout: &Layout, walked: &WalkedAnalysis, residual: &[Option<f64>]) -> Spread {
    let cycles = layout.walk.cycles();
    let n_keys = layout.keys.len();
    let mut res: Vec<Vec<(f64, f64)>> = vec![Vec::new(); n_keys];
    for ((c, t), r) in cycles.iter().zip(&walked.cycles).zip(residual) {
        let Some(r) = r else {
            continue;
        };
        if let Some(k) = layout.keys.iter().position(|(key, _)| *key == t.output)
            && let Some(v) = res.get_mut(k)
        {
            v.push((c.start as f64, *r));
        }
    }
    // Per key: its passes, their mean position, the sum of its squared
    // residuals.
    let mut sum_r2 = 0.0;
    let mut sxx = 0.0;
    let mut points = 0usize;
    let mut used_keys = 0usize;
    let stats: Vec<(usize, f64, f64)> = res
        .iter()
        .map(|v| {
            let n = v.len();
            if n == 0 {
                return (0, 0.0, 0.0);
            }
            used_keys = used_keys.saturating_add(1);
            points = points.saturating_add(n);
            let mean_n = v.iter().map(|p| p.0).sum::<f64>() / n as f64;
            let r2: f64 = v.iter().map(|p| p.1 * p.1).sum();
            sum_r2 += r2;
            sxx += v.iter().map(|p| (p.0 - mean_n).powi(2)).sum::<f64>();
            (n, mean_n, r2)
        })
        .collect();
    // The fit took one intercept per key and one slope from the points.
    let dof = points.saturating_sub(used_keys).saturating_sub(1);
    // -- review AT-A/AT-B -- the variance of one cycle's arrival, pooled
    // over every key of the run. The scatter is mostly the capture clock's
    // wander, which every key shares, and a key's own two or three passes
    // say little of it (a variance from three points is within a factor of
    // three of the truth only two times in three, and its `n - 1` forgets
    // that the slope was fitted from the same points). So a key's
    // uncertainty is never below what the whole run's scatter gives it; a
    // key that scatters more than the run (a poor signal-to-noise ratio)
    // keeps its own.
    let pooled = (dof >= 1).then(|| sum_r2 / dof as f64);
    let per_key = stats
        .iter()
        .map(|&(n, mean_n, r2)| {
            let se = (n >= 2).then(|| {
                let nf = n as f64;
                let own = r2 / (nf - 1.0);
                let var = pooled.map_or(own, |p| own.max(p));
                (var / nf + ARRIVAL_RESOLUTION_US * ARRIVAL_RESOLUTION_US).sqrt()
            });
            (se.filter(|s| s.is_finite()), mean_n)
        })
        .collect();
    let slope = (dof >= 1 && sxx > 0.0)
        .then(|| (sum_r2 / dof as f64 / sxx).sqrt())
        .filter(|s| s.is_finite());
    Spread {
        per_key,
        slope_us_per_sample: slope,
    }
}

#[allow(clippy::too_many_arguments)]
fn build(
    layout: &Layout,
    walked: WalkedAnalysis,
    fit: &crate::ir::FitParts,
    pin: TimePin,
    rate: f64,
    parts: Option<&OverlapParts>,
    overlap: Option<u64>,
) -> SystemAnalysis {
    // -- WP ATOVL -- an overlapped run: the differences each group measures
    // (its cycles played together) and, beside them, every cycle against
    // the absolute fit.
    let sp = match parts {
        Some(p) => {
            let res: Vec<Option<f64>> = walked
                .cycles
                .iter()
                .enumerate()
                .map(|(k, t)| {
                    let used = p.used.get(k).copied().unwrap_or(false) && multi_at(p, k);
                    (used && !t.arrival_ambiguous && t.residual_us.is_finite())
                        .then_some(t.residual_us)
                })
                .collect();
            spread_of(layout, &walked, &res)
        }
        None => spread(layout, &walked),
    };
    let sp_abs = parts.map(|p| {
        let res: Vec<Option<f64>> = walked
            .cycles
            .iter()
            .enumerate()
            .map(|(k, t)| {
                let r = p.absolute_residual_us.get(k).copied().unwrap_or(f64::NAN);
                (!t.arrival_ambiguous && r.is_finite()).then_some(r)
            })
            .collect();
        spread_of(layout, &walked, &res)
    });
    let ms = 1000.0 / rate;
    let earliest = walked
        .outputs
        .iter()
        .map(|o| o.envelope_index)
        .fold(f64::INFINITY, f64::min);
    // -- WP ATLATE -- each key's lateness as laid out, domain samples and ms.
    let device_ms = 1000.0 / f64::from(layout.walk.sweep.rate_hz.max(1));
    let late_ms = |i: usize| layout.late.get(i).copied().unwrap_or(0) as f64 * device_ms;
    let any_late = layout.late.iter().any(|&l| l != 0);
    let mut outputs: Vec<SystemOutput> = layout
        .keys
        .iter()
        .enumerate()
        .map(|(i, (key, members))| {
            let ir = walked.outputs.iter().find(|o| o.output == *key);
            let arrival_ms = ir.map_or(0.0, |o| o.arrival_ms);
            let onset_ms = ir.map_or(arrival_ms, |o| {
                if earliest.is_finite() {
                    (o.onset_index - earliest) * ms
                } else {
                    arrival_ms
                }
            });
            let late = layout.late.get(i).copied().unwrap_or(0) as f64;
            let uncertainty_us = sp.per_key.get(i).and_then(|p| p.0);
            SystemOutput {
                key: *key,
                members: members.clone(),
                intercept_frame: fit
                    .intercepts
                    .get(i)
                    .copied()
                    .flatten()
                    .map_or(f64::NAN, |a| a + fit.first_frame + late * fit.slope),
                arrival_ms: arrival_ms + late_ms(i),
                onset_ms: onset_ms + late_ms(i),
                passes: ir.map_or(0, |o| o.repeats),
                uncertainty_us,
                lateness_ms: late_ms(i),
                absolute_uncertainty_us: match &sp_abs {
                    Some(a) => a.per_key.get(i).and_then(|p| p.0),
                    None => uncertainty_us,
                },
            }
        })
        .collect();
    // Arrivals after the run's earliest output, as without lateness.
    if any_late {
        let first = outputs
            .iter()
            .filter(|o| o.passes > 0)
            .map(|o| o.arrival_ms)
            .fold(f64::INFINITY, f64::min);
        if first.is_finite() {
            for o in &mut outputs {
                o.arrival_ms -= first;
                o.onset_ms -= first;
                o.lateness_ms -= first;
            }
        }
    }
    let timing_reliable = walked.quality.reliable;
    let mut reasons: Vec<String> = walked
        .quality
        .reasons
        .iter()
        .map(|r| name_keys(r, &outputs))
        .collect();
    // -- WP ATOVL --
    let (absolute_reliable, absolute_reasons) = match parts.and_then(|p| p.absolute.as_ref()) {
        Some(g) => (
            g.reliable,
            g.reasons.iter().map(|r| name_keys(r, &outputs)).collect(),
        ),
        None if parts.is_some() => (false, vec!["the absolute gate did not run".to_string()]),
        None => (timing_reliable, reasons.clone()),
    };
    let mut delays = Vec::new();
    let summed = layout.keys.iter().any(|(_, m)| m.len() > 1);
    if !summed {
        for (a, oa) in outputs.iter().enumerate() {
            for (b, ob) in outputs.iter().enumerate().skip(a.saturating_add(1)) {
                let (Some(&ma), Some(&mb)) = (oa.members.first(), ob.members.first()) else {
                    continue;
                };
                if ma.unit == mb.unit {
                    continue;
                }
                let mean = |sp: &Spread, k: usize| sp.per_key.get(k).map_or(0.0, |p| p.1);
                let plain_u = |sp: &Spread, x: Option<f64>, y: Option<f64>| match (x, y) {
                    (Some(x), Some(y)) => {
                        let s = sp.slope_us_per_sample.unwrap_or(0.0) * (mean(sp, a) - mean(sp, b));
                        Some((x * x + y * y + s * s).sqrt()).filter(|v| v.is_finite())
                    }
                    _ => None,
                };
                // -- WP ATOVL -- played together: what their groups measure;
                // else as interleaved passes (the absolute gate).
                let together = parts.and_then(|p| pair_together(layout, &walked, p, a, b));
                let (u, ok, first_reason) = match (parts, together) {
                    (Some(_), Some(u)) => (Some(u), timing_reliable, reasons.first().cloned()),
                    (Some(_), None) => (
                        sp_abs.as_ref().and_then(|s| {
                            plain_u(s, oa.absolute_uncertainty_us, ob.absolute_uncertainty_us)
                        }),
                        absolute_reliable,
                        absolute_reasons.first().cloned(),
                    ),
                    (None, _) => (
                        plain_u(&sp, oa.uncertainty_us, ob.uncertainty_us),
                        timing_reliable,
                        reasons.first().cloned(),
                    ),
                };
                let (offered, reason) =
                    offer(ok, u, first_reason.as_deref(), &oa.name(), &ob.name());
                if !offered && ok {
                    reasons.push(reason.clone());
                }
                delays.push(CrossUnitDelay {
                    from: ma,
                    to: mb,
                    delay_ms: ob.arrival_ms - oa.arrival_ms,
                    uncertainty_us: u,
                    offered,
                    reason,
                });
            }
        }
    }
    let per_s = f64::from(layout.walk.sweep.rate_hz);
    SystemAnalysis {
        clock: walked.clock,
        slope_uncertainty_us_per_s: sp.slope_us_per_sample.map(|s| s * per_s),
        walked,
        outputs,
        summed,
        t_ref: layout.t_ref,
        pin,
        delays,
        timing_reliable,
        reasons,
        overlap_ms: overlap.map(|o| o as f64 * device_ms),
        absolute_reliable,
        absolute_reasons,
    }
}

// -- WP ATOVL --
/// Cycle `k` of an overlapped run plays with its group (a group of two or
/// more).
fn multi_at(p: &OverlapParts, k: usize) -> bool {
    let Some(g) = p.group.get(k) else {
        return false;
    };
    p.group.iter().filter(|x| *x == g).count() >= 2
}

// -- WP ATOVL --
/// The uncertainty of the difference of keys `a` and `b` of an overlapped
/// run from the groups both played in (`None`: fewer than two such groups,
/// so the pair did not play together): the spread of their residual
/// differences over those groups (the common delay cancels in each), never
/// below what the run's pooled scatter gives a pair, with the arrival
/// estimator's resolution of each in quadrature, µs.
fn pair_together(
    layout: &Layout,
    walked: &WalkedAnalysis,
    p: &OverlapParts,
    a: usize,
    b: usize,
) -> Option<f64> {
    let cycles = layout.walk.cycles();
    let key_of = |k: usize| -> Option<usize> {
        let out = cycles.get(k)?.output;
        layout.keys.iter().position(|(key, _)| *key == out)
    };
    let clear = |k: usize| {
        p.used.get(k).copied().unwrap_or(false)
            && multi_at(p, k)
            && walked
                .cycles
                .get(k)
                .is_some_and(|t| !t.arrival_ambiguous && t.residual_us.is_finite())
    };
    let res = |k: usize| walked.cycles.get(k).map_or(f64::NAN, |t| t.residual_us);
    let mut diffs = Vec::new();
    for ka in (0..cycles.len()).filter(|&k| key_of(k) == Some(a) && clear(k)) {
        let g = p.group.get(ka);
        if let Some(kb) =
            (0..cycles.len()).find(|&k| key_of(k) == Some(b) && p.group.get(k) == g && clear(k))
        {
            diffs.push(res(ka) - res(kb));
        }
    }
    let n = diffs.len();
    if n < 2 {
        return None;
    }
    let nf = n as f64;
    let mean = diffs.iter().sum::<f64>() / nf;
    let own = diffs.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / (nf - 1.0);
    // The pooled scatter of one cycle against its group: every clear cycle
    // played together, less the groups' and the keys' parameters.
    let used: Vec<usize> = (0..cycles.len()).filter(|&k| clear(k)).collect();
    let mut groups: Vec<u32> = used
        .iter()
        .filter_map(|&k| p.group.get(k).copied())
        .collect();
    groups.sort_unstable();
    groups.dedup();
    let mut keys: Vec<usize> = used.iter().filter_map(|&k| key_of(k)).collect();
    keys.sort_unstable();
    keys.dedup();
    let dof = used
        .len()
        .saturating_sub(groups.len())
        .saturating_sub(keys.len());
    let pooled =
        (dof >= 1).then(|| 2.0 * used.iter().map(|&k| res(k).powi(2)).sum::<f64>() / dof as f64);
    let var = pooled.map_or(own, |p| own.max(p));
    let u = (var / nf + 2.0 * ARRIVAL_RESOLUTION_US * ARRIVAL_RESOLUTION_US).sqrt();
    u.is_finite().then_some(u)
}

/// Whether a cross-unit delay is offered (G5 and review R8a): only from a
/// run whose timing is reliable, and only with an uncertainty of at most
/// [`MAX_OFFERED_UNCERTAINTY_US`]; else why not.
pub(crate) fn offer(
    timing_reliable: bool,
    uncertainty_us: Option<f64>,
    first_reason: Option<&str>,
    from: &str,
    to: &str,
) -> (bool, String) {
    match uncertainty_us {
        _ if !timing_reliable => (
            false,
            format!(
                "timing unreliable: {}",
                first_reason.unwrap_or("the timing gate failed")
            ),
        ),
        Some(u) if u <= MAX_OFFERED_UNCERTAINTY_US => (true, String::new()),
        Some(u) => (
            false,
            format!(
                "timing unreliable: the capture clock wandered {u:.1} µs between {from} and {to}"
            ),
        ),
        None => (
            false,
            format!("timing unreliable: fewer than two clear passes of {from} or {to}"),
        ),
    }
}

/// P5's reasons name a key as "output N": name the unit and output instead.
pub(crate) fn name_keys(reason: &str, outputs: &[SystemOutput]) -> String {
    const TOKEN: &str = "output ";
    let mut out = String::with_capacity(reason.len().saturating_add(16));
    let mut rest = reason;
    while let Some(i) = rest.find(TOKEN) {
        let Some((head, tail)) = rest.split_at_checked(i.saturating_add(TOKEN.len())) else {
            break;
        };
        let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
        let named = digits
            .parse::<u8>()
            .ok()
            .and_then(|k| outputs.iter().find(|o| o.key == k));
        match named {
            Some(o) => {
                out.push_str(head.get(..i).unwrap_or(""));
                out.push_str(&o.name());
            }
            None => {
                out.push_str(head);
                out.push_str(&digits);
            }
        }
        rest = tail.get(digits.len()..).unwrap_or("");
    }
    out.push_str(rest);
    out
}

/// Analyses a complete system run: every slice of [`SystemAnalyser`] at
/// once. The capture is treated as complete.
///
/// Errors: [`TimingError::CycleIdentity`] when the capture does not line up
/// with the run (the run is discarded); [`TimingError::Measure`] for a run
/// the analyser cannot lay out, bad input or no signal.
pub fn analyse_system(
    capture: &SystemCapture<'_>,
    options: &AnalysisOptions,
) -> Result<SystemAnalysis, TimingError> {
    let capture = SystemCapture {
        complete: true,
        ..*capture
    };
    let mut a = SystemAnalyser::new(capture.schedule, capture.rate_hz, options)?;
    let bound = a.inner.slices_total().saturating_add(2);
    for _ in 0..bound {
        match a.step(&capture)? {
            SystemStep::Done(r) => return Ok(*r),
            SystemStep::NeedCapture => {
                return Err(
                    MeasureError::NoSignal("the capture ends before the run".into()).into(),
                );
            }
            SystemStep::Progress { .. } => {}
        }
    }
    Err(MeasureError::Invalid("the analysis did not finish".into()).into())
}

/// A response's passband level: the median of its trusted points within
/// 10 dB of its loudest, dB.
pub fn passband_level_db(response: &Response) -> Option<f64> {
    let pts = trusted(response);
    let max = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    if !max.is_finite() {
        return None;
    }
    let mut top: Vec<f64> = pts
        .iter()
        .map(|p| p.1)
        .filter(|&m| m >= max - TRANSITION_WITHIN_DB)
        .collect();
    top.sort_by(f64::total_cmp);
    top.get(top.len() / 2).copied()
}

fn trusted(r: &Response) -> Vec<(f64, f64)> {
    let Ok(freqs) = r.grid.freqs() else {
        return Vec::new();
    };
    freqs
        .iter()
        .zip(&r.mag_db)
        .enumerate()
        .filter(|(i, (_, m))| {
            m.is_finite()
                && r.valid
                    .as_ref()
                    .is_none_or(|v| v.get(*i).copied().unwrap_or(false))
        })
        .map(|(_, (&f, &m))| (f, m))
        .collect()
}

/// The band where a response is within [`TRANSITION_WITHIN_DB`] of its own
/// passband level: the widest contiguous run of such trusted points, Hz.
pub fn passband(response: &Response) -> Option<(f64, f64)> {
    let level = passband_level_db(response)?;
    let freqs = response.grid.freqs().ok()?;
    let ok: Vec<bool> = mask_within(response, level, freqs.len());
    widest(&freqs, &ok)
}

fn mask_within(r: &Response, level: f64, n: usize) -> Vec<bool> {
    (0..n)
        .map(|i| {
            let m = r.mag_db.get(i).copied().unwrap_or(f64::NAN);
            let trusted = r
                .valid
                .as_ref()
                .is_none_or(|v| v.get(i).copied().unwrap_or(false));
            trusted && m.is_finite() && m >= level - TRANSITION_WITHIN_DB
        })
        .collect()
}

fn widest(freqs: &[f64], ok: &[bool]) -> Option<(f64, f64)> {
    let mut best: Option<(f64, f64)> = None;
    let mut run: Option<(f64, f64)> = None;
    for (&f, &good) in freqs.iter().zip(ok) {
        if good {
            run = Some(run.map_or((f, f), |(a, _)| (a, f)));
            if let Some((a, b)) = run
                && best.is_none_or(|(x, y)| b / a > y / x)
            {
                best = Some((a, b));
            }
        } else {
            run = None;
        }
    }
    best
}

/// An inter-unit transition's check band (review R8d): where both outputs
/// are within [`TRANSITION_WITHIN_DB`] of their own passband levels (the
/// widest contiguous band), limited above to where the delay's uncertainty
/// `uncertainty_us` is 45° of phase (`1 / (8 u)`); `None` when that is less
/// than [`TRANSITION_MIN_OCTAVES`] wide (no transition) or the responses are
/// on different grids.
pub fn transition_band(
    a: &Response,
    b: &Response,
    uncertainty_us: Option<f64>,
) -> Option<(f64, f64)> {
    if a.grid != b.grid {
        return None;
    }
    let freqs = a.grid.freqs().ok()?;
    let ma = mask_within(a, passband_level_db(a)?, freqs.len());
    let mb = mask_within(b, passband_level_db(b)?, freqs.len());
    let both: Vec<bool> = ma.iter().zip(&mb).map(|(x, y)| *x && *y).collect();
    let (lo, mut hi) = widest(&freqs, &both)?;
    if let Some(u) = uncertainty_us
        && u.is_finite()
        && u > 0.0
    {
        hi = hi.min(1.0 / (8.0 * u * 1e-6));
    }
    (hi > 0.0 && lo > 0.0 && (hi / lo).log2() >= TRANSITION_MIN_OCTAVES).then_some((lo, hi))
}
