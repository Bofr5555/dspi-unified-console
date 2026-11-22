//! The system measurement's plan (WP ATCORE, PLAN-AT 1.4): every unit's own
//! P5 plan (its outputs, the sweep, the levels' start and ceiling), the
//! preflight across units, and the timed runs: interleaved passes over the
//! group, split into runs chained through an anchor output when the group
//! does not fit one capture (at most 16 outputs, 120 s).

use dspi_measure::stimulus::{LogSweep, WalkSchedule};
use dspi_measure::system::messages::CAP;
use dspi_measure::system::{
    AnchorOutput, DomainId, InterleaveOptions, MAX_RUN_OUTPUTS, SystemSchedule, UnitIndex,
    UnitWalk, interleave_late, overlap_late,
};

use super::super::plan::{PREROLL_MS, Plan};
use super::super::{MeasurePlacement, MeasureRunKind, MeasureSetup};
use super::{
    Hub, MeasureSystemIssue, MeasureSystemPreflight, MeasureSystemSetup, MeasureSystemUnit,
    MeasureSystemUnitCheck, UnitContext,
};
use crate::{DspiError, Result};

/// The fewest units of a system measurement.
pub(crate) const MIN_UNITS: usize = 2;
/// The most (`dspi_measure::system::MAX_UNITS`).
pub(crate) const MAX_UNITS: usize = dspi_measure::system::MAX_UNITS;
/// The lead the plan sizes runs with before the units' arm leads are known,
/// ms (a K340: 150 ms arm lead + 100 ms).
pub(crate) const PLANNING_LEAD_MS: u32 = 250;
/// Added to the largest arm lead for the lead (S-AT3), ms.
pub(crate) const LEAD_MARGIN_MS: u32 = 100;
// -- WP ATOVL --
/// What a unit's play-out (its D and post-alink latency, read only when the
/// run starts) may add to the lateness the plan expects from its outputs'
/// `out.delay`, per segment, ms: the runs are sized with it, so that a group
/// that fits with its real lateness is not refused when the run is laid out.
pub(crate) const LATENESS_SLACK_MS: u32 = 10;

/// How a timed run was laid out (the result says which).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RunPlan {
    /// Overlapped starts, the units this far apart, ms.
    Overlapped(f64),
    /// Interleaved passes; why not overlapped (empty: not asked).
    Interleaved(String),
}

impl RunPlan {
    /// In words, for the result.
    pub(crate) fn words(&self) -> String {
        match self {
            RunPlan::Overlapped(ms) => format!("overlapped starts, {ms:.0} ms apart"),
            RunPlan::Interleaved(why) if why.is_empty() => "interleaved passes".into(),
            RunPlan::Interleaved(why) => format!("interleaved passes ({why})"),
        }
    }
}

fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

/// One unit's P5 plan: a walked run of its outputs at the listening
/// position, with the setup's sweep, microphone, ceiling and caps.
pub(crate) fn unit_plan(setup: &MeasureSystemSetup, unit: &MeasureSystemUnit) -> Result<Plan> {
    let outputs = unit
        .outputs
        .iter()
        .map(|o| super::super::MeasureOutput {
            group: 0,
            ..o.clone()
        })
        .collect();
    Plan::new(&MeasureSetup {
        kind: MeasureRunKind::Walked,
        outputs,
        placement: MeasurePlacement::ListeningPosition,
        count: 0,
        coupler: None,
        mic_document: setup.mic_document.clone(),
        sweep_ms: setup.sweep_ms,
        gap_ms: setup.gap_ms,
        // P5's walked plan wants 2 to 8; the system run stages one pass
        // per segment itself.
        passes: 2,
        pink_ms: 0,
        ceiling_db: setup.ceiling_db,
        spl_cap_db: setup.spl_cap_db,
        capture_rate_hz: setup.capture_rate_hz,
        name: setup.name.clone(),
        start_level_only: false,
        roles_from_layout: false, // -- WP ROLE0 --
    })
}

/// One unit of the plan.
#[derive(Debug, Clone)]
pub(crate) struct PlanUnit {
    pub(crate) serial: String,
    pub(crate) plan: Plan,
    pub(crate) ctx: UnitContext,
}

/// A timed run of the plan: which outputs of which units.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RunPart {
    /// `(unit index, outputs)`, in the units' order.
    pub(crate) units: Vec<(usize, Vec<u8>)>,
}

/// The whole plan.
#[derive(Debug, Clone)]
pub(crate) struct SetupPlan {
    pub(crate) setup: MeasureSystemSetup,
    pub(crate) units: Vec<PlanUnit>,
    /// The units' (the domain's) rate, Hz.
    pub(crate) rate_hz: u32,
    pub(crate) capture_rate_hz: u32,
    pub(crate) passes: Option<u16>,
    pub(crate) anchor: AnchorOutput,
    /// The timed runs (one unless chained through the anchor).
    pub(crate) runs: Vec<RunPart>,
}

impl SetupPlan {
    /// The plan of `setup` over `units` (from the preflight).
    ///
    /// Errors: [`DspiError::InvalidArgument`] naming what does not fit.
    pub(crate) fn new(setup: &MeasureSystemSetup, units: Vec<PlanUnit>) -> Result<SetupPlan> {
        let Some(first) = units.first() else {
            return Err(invalid("no unit"));
        };
        let rate_hz = first.ctx.device_rate_hz;
        let capture_rate_hz = first.plan.capture_rate_hz;
        let passes = match setup.passes {
            0 => None,
            p @ 2..=8 => Some(u16::from(p)),
            p => return Err(invalid(format!("{p} passes (2 to 8)"))),
        };
        let anchor_unit = if setup.anchor_serial.is_empty() {
            0
        } else {
            units
                .iter()
                .position(|u| u.serial == setup.anchor_serial)
                .ok_or_else(|| {
                    invalid(format!(
                        "the anchor unit {} is not in the group",
                        setup.anchor_serial
                    ))
                })?
        };
        let anchor_outputs: Vec<u8> = units
            .get(anchor_unit)
            .map(|u| u.plan.outputs.iter().map(|o| o.output).collect())
            .unwrap_or_default();
        let anchor_output = match setup.anchor_output {
            Some(o) if anchor_outputs.contains(&o) => o,
            Some(o) => {
                return Err(invalid(format!(
                    "the anchor output {} is not measured on its unit",
                    u16::from(o).saturating_add(1)
                )));
            }
            None => *anchor_outputs
                .first()
                .ok_or_else(|| invalid("the anchor unit has no output"))?,
        };
        let anchor = AnchorOutput {
            unit: UnitIndex(u8::try_from(anchor_unit).unwrap_or(0)),
            output: anchor_output,
        };
        let mut plan = SetupPlan {
            setup: setup.clone(),
            units,
            rate_hz,
            capture_rate_hz,
            passes,
            anchor,
            runs: Vec::new(),
        };
        plan.runs = plan.split()?;
        Ok(plan)
    }

    /// The sweep every unit plays.
    pub(crate) fn sweep(&self) -> Result<LogSweep> {
        let p = self
            .units
            .first()
            .map(|u| &u.plan)
            .ok_or_else(|| invalid("no unit"))?;
        LogSweep::new(self.rate_hz, &p.sweep).map_err(|e| invalid(e.to_string()))
    }

    /// The lead in domain samples.
    pub(crate) fn lead_samples(&self, lead_ms: u32) -> u64 {
        u64::from(self.rate_hz).saturating_mul(u64::from(lead_ms)) / 1000
    }

    /// The interleaved schedule of run `part` from domain sample 0, with a
    /// lead of `lead_ms` between a unit's segments.
    #[cfg(test)]
    pub(crate) fn schedule(&self, part: &RunPart, lead_ms: u32) -> Result<SystemSchedule> {
        self.schedule_late(part, lead_ms, &[])
    }

    // -- WP ATOVL --
    /// Run `part`'s schedule as the session plays it: overlapped starts
    /// where they are safe (`overlap`, two units or more, and
    /// `overlap_late` lays them out), else interleaved passes
    /// ([`SetupPlan::schedule_late`]); and which, in words.
    ///
    /// Errors: as [`SetupPlan::schedule_late`] (the interleaved fallback's).
    pub(crate) fn schedule_run(
        &self,
        part: &RunPart,
        lead_ms: u32,
        lateness: &[(UnitIndex, u64)],
        overlap: bool,
    ) -> Result<(SystemSchedule, RunPlan)> {
        let why = if !overlap {
            String::new()
        } else if part.units.len() < 2 {
            "one unit".into()
        } else {
            match self.check_lateness(part, lateness).and_then(|()| {
                let walks = self.walks(part)?;
                overlap_late(&walks, &self.options(lead_ms)?, lateness)
                    .map_err(|e| invalid(e.to_string()))
            }) {
                Ok(s) => {
                    let ms = s
                        .overlap_samples()
                        .map_or(0.0, |o| o as f64 * 1000.0 / f64::from(self.rate_hz.max(1)));
                    return Ok((s, RunPlan::Overlapped(ms)));
                }
                Err(e) => {
                    let m = e.to_string();
                    let m = m.strip_prefix("invalid argument: ").unwrap_or(&m);
                    m.strip_prefix("invalid input: ").unwrap_or(m).to_owned()
                }
            }
        };
        let s = self.schedule_late(part, lead_ms, lateness)?;
        Ok((s, RunPlan::Interleaved(why)))
    }

    // -- WP ATOVL -- (moved out of `schedule_late`, unchanged)
    fn walks(&self, part: &RunPart) -> Result<Vec<UnitWalk>> {
        let sweep = self.sweep()?;
        part.units
            .iter()
            .map(|(u, outs)| {
                let mask = outs.iter().fold(0u16, |m, o| {
                    m | 1u16.checked_shl(u32::from(*o)).unwrap_or(0)
                });
                Ok(UnitWalk {
                    unit: UnitIndex(u8::try_from(*u).unwrap_or(u8::MAX)),
                    domain: DomainId {
                        time_base: 0,
                        id: 0,
                    },
                    walk: WalkSchedule::new(sweep, mask, 1).map_err(|e| invalid(e.to_string()))?,
                })
            })
            .collect()
    }

    // -- WP ATOVL --
    fn options(&self, lead_ms: u32) -> Result<InterleaveOptions> {
        let sweep = self.sweep()?;
        Ok(InterleaveOptions {
            passes: self.passes,
            gap_samples: u64::from(sweep.gap_samples),
            lead_samples: self.lead_samples(lead_ms),
            capture_rate_hz: self.capture_rate_hz,
            pre_roll_ms: PREROLL_MS,
        })
    }

    // -- WP ATOVL --
    /// What the plan expects of each unit's lateness in run `part` before
    /// the run (the engine's terms are read only then): the smallest
    /// `out.delay` of its outputs in the run, from its own check; the
    /// play-out is not known yet ([`LATENESS_SLACK_MS`] sizes for it).
    pub(crate) fn expected_lateness(&self, part: &RunPart) -> Vec<(UnitIndex, u64)> {
        part.units
            .iter()
            .filter_map(|(u, outs)| {
                let pu = self.units.get(*u)?;
                let ms = outs
                    .iter()
                    .filter_map(|o| {
                        pu.ctx
                            .chains
                            .iter()
                            .find(|(x, _)| x == o)
                            .map(|(_, c)| f64::from(c.delay_ms))
                    })
                    .fold(f64::INFINITY, f64::min);
                if !ms.is_finite() || ms <= 0.0 {
                    return None;
                }
                let samples = (ms * f64::from(self.rate_hz) / 1000.0).round();
                let samples = if samples >= 1.8e19 {
                    u64::MAX
                } else {
                    samples as u64
                };
                Some((UnitIndex(u8::try_from(*u).unwrap_or(u8::MAX)), samples))
            })
            .collect()
    }

    // -- WP ATOVL --
    /// Whether run `part` fits one capture as the session will lay it out:
    /// interleaved (the longer plan, and the fallback) with the lateness
    /// the plan expects and [`LATENESS_SLACK_MS`] per segment for the
    /// play-out not yet known.
    fn fits(&self, part: &RunPart) -> bool {
        let lateness = self.expected_lateness(part);
        let Ok(s) = self.schedule_late(part, PLANNING_LEAD_MS, &lateness) else {
            return false;
        };
        let Ok(options) = self.options(PLANNING_LEAD_MS) else {
            return false;
        };
        let per = self.lead_samples(LATENESS_SLACK_MS);
        let slack = per.saturating_mul(s.segments().len() as u64);
        dspi_measure::system::plan::run_fits(&s, &options, slack)
    }

    // -- WP ATLATE --
    /// [`SetupPlan::schedule`] with each unit's expected lateness (domain
    /// samples after its START that its sound is expected, see
    /// [`SetupPlan::lateness`]): the starts leave room for it and the
    /// analysis looks for each unit that much later.
    ///
    /// Errors: as [`SetupPlan::schedule`], and a lateness spread over
    /// `dspi_measure::system::MAX_LATENESS_MS`.
    pub(crate) fn schedule_late(
        &self,
        part: &RunPart,
        lead_ms: u32,
        lateness: &[(UnitIndex, u64)],
    ) -> Result<SystemSchedule> {
        self.check_lateness(part, lateness)?;
        let walks = self.walks(part)?;
        interleave_late(&walks, &self.options(lead_ms)?, lateness)
            .map_err(|e| invalid(e.to_string()))
    }

    // -- WP ATOVL -- (moved out of `schedule_late`, unchanged)
    fn check_lateness(&self, part: &RunPart, lateness: &[(UnitIndex, u64)]) -> Result<()> {
        // Fail closed, the unit named: more than a run plans for is a model
        // or alignment fault, refused before anything plays.
        let of = |u: usize| {
            lateness
                .iter()
                .find(|(x, _)| usize::from(x.0) == u)
                .map_or(0, |(_, l)| *l)
        };
        let min = part.units.iter().map(|(u, _)| of(*u)).min().unwrap_or(0);
        let bound = self.lead_samples(dspi_measure::system::MAX_LATENESS_MS);
        if let Some((u, l)) = part
            .units
            .iter()
            .map(|(u, _)| (*u, of(*u).saturating_sub(min)))
            .find(|(_, l)| *l > bound)
        {
            let serial = self.units.get(u).map_or("", |p| p.serial.as_str());
            return Err(invalid(format!(
                "{serial} is expected {:.0} ms after the group's earliest unit (its play-out and \
                 output delays), more than a measurement plans for ({} ms): check its alignment",
                l as f64 * 1000.0 / f64::from(self.rate_hz.max(1)),
                dspi_measure::system::MAX_LATENESS_MS
            )));
        }
        Ok(())
    }

    // -- WP ATLATE --
    /// Each unit's expected lateness in run `part`, domain samples: what the
    /// alignment engine adds to the unit's sound after its START (its
    /// play-out of a domain sample and the smallest `out.delay` of its
    /// outputs in the run: on USB-direct the streams' start offsets, some
    /// 21 ms a unit), from `terms` read before the run. A unit without terms
    /// is expected at 0 (as before this WP): its arrivals are not offered
    /// anyway, and the analysis' own bound (an arrival cut at the end of
    /// what it keeps is not offered) still holds for every unit.
    pub(crate) fn lateness(
        &self,
        part: &RunPart,
        terms: &[Option<super::proposal::Terms>],
    ) -> Vec<(UnitIndex, u64)> {
        part.units
            .iter()
            .filter_map(|(u, outs)| {
                let t = terms.get(*u)?.as_ref()?;
                let us = outs
                    .iter()
                    .filter_map(|o| t.at_us(*o))
                    .fold(f64::INFINITY, f64::min);
                if !us.is_finite() || us <= 0.0 {
                    return None;
                }
                let samples = (us * f64::from(self.rate_hz) / 1e6).round();
                // A lateness past the counter is refused by the schedule.
                let samples = if samples >= 1.8e19 {
                    u64::MAX
                } else {
                    samples as u64
                };
                Some((UnitIndex(u8::try_from(*u).unwrap_or(u8::MAX)), samples))
            })
            .collect()
    }

    /// The group as runs: one when it fits (16 outputs, one capture), else
    /// runs that each repeat the anchor output, as few as fit (OD-ATk).
    fn split(&self) -> Result<Vec<RunPart>> {
        let all: Vec<(usize, u8)> = self
            .units
            .iter()
            .enumerate()
            .flat_map(|(i, u)| u.plan.outputs.iter().map(move |o| (i, o.output)))
            .collect();
        let whole = part_of(&all);
        // -- WP ATOVL -- sized with the lateness expected (review AT-E 2).
        if all.len() <= MAX_RUN_OUTPUTS && self.fits(&whole) {
            return Ok(vec![whole]);
        }
        let anchor = (usize::from(self.anchor.unit.0), self.anchor.output);
        let rest: Vec<(usize, u8)> = all.iter().copied().filter(|x| *x != anchor).collect();
        let mut runs = Vec::new();
        let mut i = 0usize;
        while i < rest.len() {
            // As many as fit with the anchor, at least one.
            let mut take = 0usize;
            for n in 1..=rest.len().saturating_sub(i) {
                let mut outs = vec![anchor];
                outs.extend(rest.iter().skip(i).take(n).copied());
                if outs.len() > MAX_RUN_OUTPUTS {
                    break;
                }
                if self.fits(&part_of(&outs)) {
                    take = n;
                } else {
                    break;
                }
            }
            if take == 0 {
                return Err(invalid(
                    "one output with the anchor does not fit one capture: shorten the sweep",
                ));
            }
            let mut outs = vec![anchor];
            outs.extend(rest.iter().skip(i).take(take).copied());
            runs.push(part_of(&outs));
            i = i.saturating_add(take);
        }
        if runs.len() > dspi_measure::system::chain::MAX_CHAIN_RUNS {
            return Err(invalid(format!(
                "the group needs {} runs (at most 16)",
                runs.len()
            )));
        }
        Ok(runs)
    }
}

/// `(unit, output)` pairs as a run, units in order.
fn part_of(outs: &[(usize, u8)]) -> RunPart {
    let mut units: Vec<(usize, Vec<u8>)> = Vec::new();
    for &(u, o) in outs {
        match units.iter_mut().find(|(x, _)| *x == u) {
            Some((_, v)) => v.push(o),
            None => units.push((u, vec![o])),
        }
    }
    units.sort_by_key(|(u, _)| *u);
    for (_, v) in &mut units {
        v.sort_unstable();
    }
    RunPart { units }
}

/// The preflight across units: the hub, the role, one check per unit (each
/// unit's blockers named with it), one rate, a plan that fits. Also the
/// units for [`SetupPlan::new`] when nothing blocks.
pub(crate) fn preflight(
    setup: &MeasureSystemSetup,
    checks: &[MeasureSystemUnitCheck],
    hub: &mut dyn Hub,
) -> Result<(MeasureSystemPreflight, Vec<PlanUnit>)> {
    let mut blockers = Vec::new();
    let mut warnings = Vec::new();
    let n = setup.units.len();
    if !(MIN_UNITS..=MAX_UNITS).contains(&n) {
        return Err(invalid(format!("{n} units (2 to {MAX_UNITS})")));
    }
    for (i, u) in setup.units.iter().enumerate() {
        if setup
            .units
            .iter()
            .skip(i.saturating_add(1))
            .any(|v| v.serial == u.serial)
        {
            return Err(invalid(format!("unit {} twice", u.serial)));
        }
    }
    match hub.caps() {
        Some(caps) if caps.iter().any(|c| c == CAP) => {}
        _ => blockers.push(MeasureSystemIssue::new(
            "",
            "measure.system.needs_hub",
            None,
            "measuring several units together needs the DSPi hub",
        )),
    }
    if hub.role() == "viewer" {
        blockers.push(MeasureSystemIssue::new(
            "",
            "measure.viewer",
            None,
            "a Viewer cannot measure: the hub gave this console the Viewer role",
        ));
    }
    let mut units = Vec::new();
    for u in &setup.units {
        let plan = unit_plan(setup, u)?;
        let Some(check) = checks.iter().find(|c| c.serial == u.serial) else {
            blockers.push(MeasureSystemIssue::new(
                &u.serial,
                "measure.system.unit_check",
                None,
                format!(
                    "{}: the unit was not checked on its own connection",
                    u.serial
                ),
            ));
            continue;
        };
        for b in &check.preflight.blockers {
            blockers.push(MeasureSystemIssue::of(&u.serial, b));
        }
        for w in &check.preflight.warnings {
            warnings.push(MeasureSystemIssue::of(&u.serial, w));
        }
        let ctx: UnitContext = match serde_json::from_str(&check.context) {
            Ok(c) => c,
            Err(e) => {
                blockers.push(MeasureSystemIssue::new(
                    &u.serial,
                    "measure.system.unit_check",
                    None,
                    format!("{}: its check cannot be read ({e})", u.serial),
                ));
                continue;
            }
        };
        if ctx.serial != u.serial {
            blockers.push(MeasureSystemIssue::new(
                &u.serial,
                "measure.system.unit_check",
                None,
                format!("{}: its check is of another unit", u.serial),
            ));
            continue;
        }
        // -- WP ROLL1 -- one sweep plays every unit from the setup's start
        // (20 Hz): a tweeter (its effective role on the unit's own chain) is
        // never played below 1 kHz, so it cannot be in a system run.
        // -- WP RCFIX -- or its role on its own core, from its confirmed layout.
        for o in &plan.outputs {
            let Some((_, chain)) = ctx.chains.iter().find(|(x, _)| *x == o.output) else {
                continue;
            };
            let role = ctx
                .roles
                .iter()
                .find(|(x, _)| *x == o.output)
                .map_or(o.role, |(_, r)| super::super::stricter_role(o.role, *r));
            if super::super::effective_role(role, chain) == super::super::MeasureRole::High {
                blockers.push(MeasureSystemIssue::new(
                    &u.serial,
                    super::super::plan::KEY_TWEETER_STIMULUS,
                    Some(o.output),
                    format!(
                        "{}: output {} is a tweeter, and a measurement across units sweeps \
                         every output from 20 Hz: measure the tweeter on its own unit",
                        u.serial,
                        u16::from(o.output).saturating_add(1)
                    ),
                ));
            }
        }
        units.push(PlanUnit {
            serial: u.serial.clone(),
            plan,
            ctx,
        });
    }
    if units.len() == n {
        let rate = units.first().map_or(0, |u| u.ctx.device_rate_hz);
        if rate == 0 || units.iter().any(|u| u.ctx.device_rate_hz != rate) {
            let rates: Vec<String> = units
                .iter()
                .map(|u| format!("{} at {} Hz", u.serial, u.ctx.device_rate_hz))
                .collect();
            blockers.push(MeasureSystemIssue::new(
                "",
                "measure.system.rates",
                None,
                format!(
                    "the units run at different rates ({}): one time base has one rate",
                    rates.join(", ")
                ),
            ));
        } else if let Err(e) = SetupPlan::new(setup, units.clone()) {
            blockers.push(MeasureSystemIssue::new(
                "",
                "measure.system.plan",
                None,
                e.to_string(),
            ));
        }
    }
    Ok((
        MeasureSystemPreflight {
            clear: blockers.is_empty(),
            blockers,
            warnings,
        },
        units,
    ))
}

/// The error a start answers for a preflight with blockers.
pub(crate) fn refusal(pre: &MeasureSystemPreflight) -> Option<DspiError> {
    if pre.clear {
        return None;
    }
    let has = |k: &str| pre.blockers.iter().any(|b| b.key == k);
    if has("measure.system.needs_hub") {
        return Some(DspiError::Unsupported);
    }
    if has("measure.viewer") {
        return Some(DspiError::Denied);
    }
    if has("read_only.client") || has("read_only") {
        return Some(DspiError::ReadOnly);
    }
    let words: Vec<&str> = pre.blockers.iter().map(|b| b.message.as_str()).collect();
    Some(invalid(format!("cannot measure: {}", words.join("; "))))
}
