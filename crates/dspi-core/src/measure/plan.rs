//! The plan of a session (WP P5SES): the setup checked and completed with the
//! plan's defaults (PLAN-P5 section 1.1 step 1, section 1.2 S3 and S7), the
//! runs in order, and the bounds checked before anything plays (RC8: a setup
//! whose capture would pass `MAX_CAPTURE_SAMPLES` is refused here).

use dspi_measure::doc::MicDoc;
use dspi_measure::ir::MAX_CAPTURE_SAMPLES;
use dspi_measure::stimulus::{
    LogSweep, MAX_REPEAT, MAX_SWEEP_MS, MIN_SWEEP_MS, SUPPORTED_RATES, SweepConfig, WalkSchedule,
};
use dspi_measure::types::{Placement, Response};

use super::{
    MeasureIssue, MeasurePlacement, MeasureRole, MeasureRunKind, MeasureSetup, placement_of,
};
use crate::{DspiError, Result};

/// The generator idle before every ramp and run: the noise floor, ms.
pub(crate) const PREROLL_MS: u32 = 1_000;
/// The longest run, ms (S7).
pub(crate) const MAX_RUN_MS: u64 = 120_000;
/// The longest pink noise, ms (S7).
pub(crate) const PINK_MAX_MS: u32 = 60_000;
/// Pink noise when the setup names no length, ms.
pub(crate) const PINK_DEFAULT_MS: u32 = 10_000;
/// Capture after pink noise ends, ms.
pub(crate) const PINK_POST_MS: u32 = 500;
/// A sweep when the setup names no length, ms.
pub(crate) const SWEEP_DEFAULT_MS: u32 = 5_500;
/// A live sweep when the setup names no length, ms.
pub(crate) const LIVE_SWEEP_MS: u32 = 2_700;
/// The gap after a cycle for speakers and on a coupler, ms (review S-b).
pub(crate) const GAP_SPEAKERS_MS: u16 = 1_000;
pub(crate) const GAP_COUPLER_MS: u16 = 300;
/// The gaps a setup may ask for, ms.
pub(crate) const GAP_RANGE_MS: std::ops::RangeInclusive<u32> = 300..=2_000;
/// Passes of a walked run: the default and the least (two passes measure a
/// clock; three are needed for "timing reliable", review P5-A).
pub(crate) const PASSES_DEFAULT: u16 = 3;
pub(crate) const PASSES_MIN: u16 = 2;
/// The ramp's start levels, peak dBFS (S3).
pub(crate) const START_SPEAKERS_DB: f32 = -50.0;
pub(crate) const START_HEADPHONES_DB: f32 = -60.0;
/// The generator's default ceilings and hard caps, peak dBFS (S3).
pub(crate) const CEILING_SPEAKERS_DB: f32 = -12.0;
pub(crate) const CEILING_HEADPHONES_DB: f32 = -30.0;
pub(crate) const HARD_CAP_SPEAKERS_DB: f32 = -6.0;
pub(crate) const HARD_CAP_HEADPHONES_DB: f32 = -20.0;
/// The SPL caps with a calibrated microphone, dB SPL (S3).
pub(crate) const SPL_CAP_SPEAKERS: f32 = 95.0;
pub(crate) const SPL_CAP_HEADPHONES: f32 = 90.0;
pub(crate) const SPL_HARD_CAP_SPEAKERS: f32 = 105.0;
pub(crate) const SPL_HARD_CAP_HEADPHONES: f32 = 100.0;
/// The capture rate when the setup names none, Hz (RC8).
pub(crate) const CAPTURE_RATE_DEFAULT: u32 = 48_000;
/// The most outputs a session measures.
pub(crate) const MAX_OUTPUTS: usize = 16;

/// One output of the plan.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OutSpec {
    pub(crate) output: u8,
    pub(crate) role: MeasureRole,
    pub(crate) speaker: String,
    pub(crate) group: u8,
    pub(crate) inverted: bool,
}

/// One run: the outputs it plays, at which position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunSpec {
    /// The position (or reseat), 0-based.
    pub(crate) position: u8,
    /// The outputs, ascending.
    pub(crate) outputs: Vec<u8>,
    /// A summed run's polarities.
    pub(crate) invert_mask: u16,
}

impl RunSpec {
    /// The generator's channel mask.
    pub(crate) fn mask(&self) -> u16 {
        self.outputs.iter().fold(0u16, |m, o| {
            m | 1u16.checked_shl(u32::from(*o)).unwrap_or(0)
        })
    }
}

/// The microphone a session measures with.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MicInfo {
    pub(crate) doc: MicDoc,
    /// What to subtract from a measurement.
    pub(crate) correction: Response,
    /// `dB SPL = dBFS + offset`, when known.
    pub(crate) spl_offset_db: Option<f64>,
}

/// A checked setup with its defaults filled in.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Plan {
    pub(crate) kind: MeasureRunKind,
    pub(crate) placement: Placement,
    pub(crate) headphones: bool,
    pub(crate) outputs: Vec<OutSpec>,
    pub(crate) runs: Vec<RunSpec>,
    pub(crate) positions: u8,
    pub(crate) sweep: SweepConfig,
    pub(crate) passes: u16,
    pub(crate) pink_ms: u32,
    pub(crate) start_db: f32,
    pub(crate) ceiling_db: f32,
    /// The SPL cap, only with a microphone whose SPL offset is known.
    pub(crate) spl_cap_db: Option<f32>,
    pub(crate) capture_rate_hz: u32,
    pub(crate) mic: Option<MicInfo>,
    pub(crate) name: String,
}

fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

impl Plan {
    /// Checks `setup` and fills in the defaults.
    ///
    /// Errors: [`DspiError::InvalidArgument`] naming what is wrong.
    pub(crate) fn new(setup: &MeasureSetup) -> Result<Plan> {
        let kind = setup.kind;
        let n = setup.outputs.len();
        if n == 0 || n > MAX_OUTPUTS {
            return Err(invalid(format!("{n} outputs to measure (1 to 16)")));
        }
        let mut seen = 0u32;
        for o in &setup.outputs {
            let bit = 1u32
                .checked_shl(u32::from(o.output))
                .filter(|_| usize::from(o.output) < MAX_OUTPUTS)
                .ok_or_else(|| invalid(format!("output {} (0 to 15)", o.output)))?;
            if seen & bit != 0 {
                return Err(invalid(format!("output {} twice", o.output)));
            }
            seen |= bit;
        }
        match kind {
            MeasureRunKind::Summed if n < 2 => {
                return Err(invalid("a summed run plays at least two outputs"));
            }
            MeasureRunKind::LiveSweep | MeasureRunKind::Pink if n != 1 => {
                return Err(invalid("a live sweep or pink noise plays one output"));
            }
            _ => {}
        }
        let mut setup = setup.clone();
        if setup.placement == MeasurePlacement::Headphones && setup.count == 0 {
            setup.count = 1;
        }
        let placement = placement_of(&setup);
        dspi_measure::position::validate_placement(&placement)
            .map_err(|e| invalid(e.to_string()))?;
        let headphones = setup.placement == MeasurePlacement::Headphones;
        // Single runs (summed, live, pink) are of one position.
        let positions = match kind {
            MeasureRunKind::Walked => match placement {
                Placement::MultiPosition { count } => count,
                Placement::Headphones { reseats, .. } => reseats,
                Placement::ListeningPosition => 1,
                // One position per output, below.
                Placement::NearField => u8::try_from(n).unwrap_or(u8::MAX),
            },
            _ => 1,
        };
        let sweep_ms = match (setup.sweep_ms, kind) {
            (0, MeasureRunKind::LiveSweep) => LIVE_SWEEP_MS,
            (0, _) => SWEEP_DEFAULT_MS,
            (ms, _) if (MIN_SWEEP_MS..=MAX_SWEEP_MS).contains(&ms) => ms,
            (ms, _) => return Err(invalid(format!("a sweep of {ms} ms (1000 to 15000)"))),
        };
        let gap_ms = match setup.gap_ms {
            0 if headphones => GAP_COUPLER_MS,
            0 => GAP_SPEAKERS_MS,
            g if GAP_RANGE_MS.contains(&g) => u16::try_from(g).unwrap_or(GAP_SPEAKERS_MS),
            g => return Err(invalid(format!("a gap of {g} ms (300 to 2000)"))),
        };
        let passes = match (kind, setup.passes) {
            (MeasureRunKind::Walked, 0) => PASSES_DEFAULT,
            (MeasureRunKind::Walked, p) if (PASSES_MIN..=MAX_REPEAT).contains(&u16::from(p)) => {
                u16::from(p)
            }
            (MeasureRunKind::Walked, p) => {
                return Err(invalid(format!("{p} passes (2 to 8)")));
            }
            _ => 1,
        };
        let pink_ms = match setup.pink_ms {
            0 => PINK_DEFAULT_MS,
            ms if (1_000..=PINK_MAX_MS).contains(&ms) => ms,
            ms => return Err(invalid(format!("pink noise of {ms} ms (1000 to 60000)"))),
        };
        let (start, default_ceiling, hard_cap) = if headphones {
            (
                START_HEADPHONES_DB,
                CEILING_HEADPHONES_DB,
                HARD_CAP_HEADPHONES_DB,
            )
        } else {
            (START_SPEAKERS_DB, CEILING_SPEAKERS_DB, HARD_CAP_SPEAKERS_DB)
        };
        let ceiling_db = match setup.ceiling_db {
            None => default_ceiling,
            Some(c) if c.is_finite() && (-120.0..=hard_cap).contains(&c) => c,
            Some(c) => {
                return Err(invalid(format!(
                    "a ceiling of {c} dBFS (at most {hard_cap} dBFS)"
                )));
            }
        };
        let (default_spl, hard_spl) = if headphones {
            (SPL_CAP_HEADPHONES, SPL_HARD_CAP_HEADPHONES)
        } else {
            (SPL_CAP_SPEAKERS, SPL_HARD_CAP_SPEAKERS)
        };
        let spl_cap = match setup.spl_cap_db {
            None => default_spl,
            Some(c) if c.is_finite() && (40.0..=hard_spl).contains(&c) => c,
            Some(c) => {
                return Err(invalid(format!(
                    "an SPL cap of {c} dB (40 to {hard_spl} dB SPL)"
                )));
            }
        };
        let capture_rate_hz = match setup.capture_rate_hz {
            0 => CAPTURE_RATE_DEFAULT,
            r if (8_000..=192_000).contains(&r) => r,
            r => return Err(invalid(format!("a capture rate of {r} Hz"))),
        };
        let mic = if setup.mic_document.trim().is_empty() {
            None
        } else {
            let doc = MicDoc::from_text(&setup.mic_document)
                .map_err(|e| invalid(format!("the microphone document: {e}")))?;
            let correction = doc.mic.correction();
            let spl_offset_db = doc.mic.spl_scale().offset_db;
            Some(MicInfo {
                doc,
                correction,
                spl_offset_db,
            })
        };
        if setup.name.chars().count() > dspi_measure::doc::MAX_NAME_CHARS {
            return Err(invalid("the name is longer than 256 characters"));
        }
        let mut outputs: Vec<OutSpec> = setup
            .outputs
            .iter()
            .map(|o| OutSpec {
                output: o.output,
                role: o.role,
                speaker: o.speaker.chars().take(64).collect(),
                group: o.group,
                inverted: o.inverted,
            })
            .collect();
        outputs.sort_by_key(|o| o.output);
        let runs = runs_of(kind, &placement, &outputs, positions);
        let spl_cap_db = mic.as_ref().and_then(|m| m.spl_offset_db).map(|_| spl_cap);
        Ok(Plan {
            kind,
            placement,
            headphones,
            outputs,
            runs,
            positions,
            sweep: SweepConfig {
                f1_hz: 20.0,
                f2_hz: 20_000.0,
                duration_ms: sweep_ms,
                gap_ms,
            },
            passes,
            pink_ms,
            start_db: start.min(ceiling_db),
            ceiling_db,
            spl_cap_db,
            capture_rate_hz,
            mic,
            name: setup.name.clone(),
        })
    }

    /// The output's spec.
    pub(crate) fn out(&self, output: u8) -> Option<&OutSpec> {
        self.outputs.iter().find(|o| o.output == output)
    }

    /// A walked or single-sweep run's schedule at the device's rate: what the
    /// analysis models (a summed run is one output's single pass to it).
    pub(crate) fn schedule(&self, run: &RunSpec, device_rate: u32) -> Result<WalkSchedule> {
        let sweep = LogSweep::new(device_rate, &self.sweep).map_err(|e| invalid(e.to_string()))?;
        let mask = match self.kind {
            MeasureRunKind::Walked => run.mask(),
            _ => run
                .outputs
                .first()
                .and_then(|o| 1u16.checked_shl(u32::from(*o)))
                .unwrap_or(1),
        };
        WalkSchedule::new(sweep, mask, self.passes).map_err(|e| invalid(e.to_string()))
    }

    /// A run's capture, frames at the capture rate (pre-roll, run, post-roll).
    pub(crate) fn run_frames(&self, run: &RunSpec, device_rate: u32) -> Result<u64> {
        if self.kind == MeasureRunKind::Pink {
            let ms = u64::from(PREROLL_MS)
                .saturating_add(u64::from(self.pink_ms))
                .saturating_add(u64::from(PINK_POST_MS));
            return Ok(ms.saturating_mul(u64::from(self.capture_rate_hz)) / 1000);
        }
        Ok(self
            .schedule(run, device_rate)?
            .planned_capture_frames(self.capture_rate_hz, PREROLL_MS))
    }

    /// A run's length, ms (its capture at the capture rate).
    pub(crate) fn run_ms(&self, run: &RunSpec, device_rate: u32) -> Result<u64> {
        let frames = self.run_frames(run, device_rate)?;
        Ok(frames.saturating_mul(1000) / u64::from(self.capture_rate_hz.max(1)))
    }

    /// The plan's bounds at the device's rate (S7, RC8): the generator's
    /// rate, each run within 120 s and [`MAX_CAPTURE_SAMPLES`].
    pub(crate) fn check_bounds(&self, device_rate: u32) -> std::result::Result<(), MeasureIssue> {
        if self.kind != MeasureRunKind::Pink && !SUPPORTED_RATES.contains(&device_rate) {
            return Err(MeasureIssue::new(
                "measure.device_rate",
                None,
                format!(
                    "the DSPi runs at {device_rate} Hz; a measurement needs 44.1, 48 or 96 kHz"
                ),
            ));
        }
        for run in &self.runs {
            let frames = self
                .run_frames(run, device_rate)
                .map_err(|e| MeasureIssue::new("measure.too_long", None, e.to_string()))?;
            let ms = self
                .run_ms(run, device_rate)
                .map_err(|e| MeasureIssue::new("measure.too_long", None, e.to_string()))?;
            if frames > MAX_CAPTURE_SAMPLES as u64 || ms > MAX_RUN_MS {
                return Err(MeasureIssue::new(
                    "measure.too_long",
                    None,
                    format!(
                        "a run of {} s and {frames} frames is past the bound (120 s, {} frames): \
                         use 48 kHz, shorter sweeps or fewer outputs a run",
                        ms / 1000,
                        MAX_CAPTURE_SAMPLES
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// The runs in order: walked runs per position and alignment group, near
/// field one per output, the single runs one.
fn runs_of(
    kind: MeasureRunKind,
    placement: &Placement,
    outputs: &[OutSpec],
    positions: u8,
) -> Vec<RunSpec> {
    let all: Vec<u8> = outputs.iter().map(|o| o.output).collect();
    let invert_mask = outputs.iter().filter(|o| o.inverted).fold(0u16, |m, o| {
        m | 1u16.checked_shl(u32::from(o.output)).unwrap_or(0)
    });
    match kind {
        MeasureRunKind::Walked => {
            if *placement == Placement::NearField {
                return (0u8..)
                    .zip(&all)
                    .map(|(position, o)| RunSpec {
                        position,
                        outputs: vec![*o],
                        invert_mask: 0,
                    })
                    .collect();
            }
            let mut groups: Vec<u8> = outputs.iter().map(|o| o.group).collect();
            groups.sort_unstable();
            groups.dedup();
            let mut runs = Vec::new();
            for position in 0..positions {
                for g in &groups {
                    runs.push(RunSpec {
                        position,
                        outputs: outputs
                            .iter()
                            .filter(|o| o.group == *g)
                            .map(|o| o.output)
                            .collect(),
                        invert_mask: 0,
                    });
                }
            }
            runs
        }
        MeasureRunKind::Summed => vec![RunSpec {
            position: 0,
            outputs: all,
            invert_mask,
        }],
        MeasureRunKind::LiveSweep | MeasureRunKind::Pink => vec![RunSpec {
            position: 0,
            outputs: all,
            invert_mask: 0,
        }],
    }
}
