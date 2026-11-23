//! The preflight (WP P5SES, PLAN-P5 section 1.2): every blocker in words before
//! anything plays, and again before a tune is applied (S10: P5TUNE calls it
//! with the chains its proposal was computed for). Reads only; writes nothing,
//! not even to clear what it finds (the user switches it off).

use dspi_measure::types::{BandSpec, ChannelChain, ProgrammeFeed};
use dspi_proto::ext::PairMap;
use dspi_session::limits::{LimitsProfile, highpass_slope};

use super::plan::Plan;
use super::{Chains, MeasureIssue, MeasurePreflight, MeasureRole, MeasureRunKind};
use crate::consts::{SUBHARM_PARAM_ENABLED, SUBHARM_PARAM_MASK};
use crate::guarded::GuardPhase;
use crate::records::{DspState, FilterParams};
use crate::{DspiCore, DspiError, Result};

/// The highest user volume a measurement starts at, dB: 80 % of the
/// registry's -60..0 dB range (S2).
pub(crate) const VOL_USER_MAX_DB: f32 = -12.0;
/// An input above this peak is programme audio, dBFS (S8).
pub(crate) const INPUT_SILENCE_DBFS: f32 = -70.0;
/// The tweeter rule without a limits profile (S6): corner, Hz, and slope,
/// dB/octave.
pub(crate) const HIGH_MIN_HZ: f32 = 1_000.0;
pub(crate) const MID_MIN_HZ: f32 = 80.0;
pub(crate) const MIN_SLOPE_DB: u8 = 12;
/// The screen id an installer may lock (assets/modes/modes.json).
pub(crate) const SCREEN_ID: &str = "measure";
/// The generator types a measurement plays (`siggen.h`).
pub(crate) const SIG_PINK: u8 = 3;
pub(crate) const SIG_LOG_SWEEP: u8 = 4;
/// A corner within this of the rule's passes (the registry's quantisation).
const HZ_SLACK: f32 = 0.5;

/// What a second preflight checks beyond the first (S10).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PreflightOptions {
    /// The id of a guard that may be waiting (P5TUNE's verify runs inside
    /// its own guard).
    pub(crate) allow_guard: Option<u64>,
    /// The chains a proposal was computed for: every output's must still be
    /// the device's.
    pub(crate) expect_chains: Option<Chains>,
}

fn band(f: &FilterParams) -> BandSpec {
    BandSpec {
        filter_type: f.filter_type,
        bypass: f.bypass,
        freq: f.freq,
        q: f.q,
        gain: f.gain,
        qp: f.qp,
    }
}

/// Output `output`'s measured chain and programme feed in `s` (PLAN-P5
/// section 1.1 step 1, RC1): its PEQ and crossover banks, gain and delay; the
/// feed is the routed input with the highest crosspoint gain.
pub(crate) fn chain_of(s: &DspState, output: u8) -> ChannelChain {
    let o = usize::from(output);
    let ch = usize::from(s.num_input_channels).saturating_add(o);
    let peq = s
        .filters
        .get(ch)
        .map(|b| b.iter().map(band).collect())
        .unwrap_or_default();
    let crossover = s
        .xover
        .get(ch)
        .map(|b| b.iter().map(band).collect())
        .unwrap_or_default();
    let mut feed: Option<ProgrammeFeed> = None;
    for (i, routes) in s.matrix_routing.iter().enumerate() {
        if !routes.get(o).copied().unwrap_or(false) {
            continue;
        }
        let gain_db = s
            .matrix_gain
            .get(i)
            .and_then(|g| g.get(o))
            .copied()
            .unwrap_or(0.0);
        let inverted = s
            .matrix_invert
            .get(i)
            .and_then(|g| g.get(o))
            .copied()
            .unwrap_or(false);
        if feed.is_none_or(|f| gain_db > f.gain_db) {
            feed = Some(ProgrammeFeed { gain_db, inverted });
        }
    }
    ChannelChain {
        peq,
        crossover,
        gain_db: s.output_gain_db.get(o).copied().unwrap_or(0.0),
        delay_ms: s.output_delay_ms.get(o).copied().unwrap_or(0.0),
        feed: feed.unwrap_or(ProgrammeFeed {
            gain_db: 0.0,
            inverted: false,
        }),
    }
}

/// What the preflight read from the device, beside the mirror.
#[derive(Debug, Clone, Default)]
pub(crate) struct Facts {
    pub(crate) generator: bool,
    pub(crate) read_only_client: bool,
    pub(crate) read_only_connection: bool,
    pub(crate) viewer: bool,
    pub(crate) page_locked: bool,
    pub(crate) guard_waiting: bool,
    pub(crate) ab_running: bool,
    /// Every input's peak, linear full scale.
    pub(crate) input_peaks: Vec<f32>,
    pub(crate) device_rate_hz: u32,
}

/// Whether `bands` hold an active high-pass at or above `min_hz`, at least
/// `min_slope` dB/octave steep.
fn has_highpass(bands: &[BandSpec], min_hz: f32, min_slope: u8) -> bool {
    bands.iter().any(|b| {
        !b.bypass
            && b.freq >= min_hz - HZ_SLACK
            && highpass_slope(b.filter_type).is_some_and(|s| s >= min_slope)
    })
}

/// The tweeter rule's corner and slope for `role` (S6): the plan's numbers,
/// or the limits profile's for that output where it names them, a high way
/// never below [`tweeter_floor`] (-- WP ROLE0 --).
pub(crate) fn tweeter_rule(
    role: MeasureRole,
    output: u8,
    profile: Option<&LimitsProfile>,
) -> Option<(f32, u8)> {
    let (hz, slope) = match role {
        MeasureRole::High => (HIGH_MIN_HZ, MIN_SLOPE_DB),
        MeasureRole::Mid => (MID_MIN_HZ, MIN_SLOPE_DB),
        _ => return None,
    };
    let o = profile.and_then(|p| p.output(output));
    let hz = o.and_then(|o| o.highpass_min_hz).unwrap_or(hz);
    let slope = o.and_then(|o| o.highpass_min_slope_db).unwrap_or(slope);
    Some(tweeter_floor(role, hz, slope))
}

// -- WP ROLE0 -- owner 2026-11-22: never below 1 kHz on a tweeter
/// The hard floor of a high way's rule: a corner of at least 1 kHz and a
/// slope of at least 12 dB/octave, whatever a limits profile says (a profile
/// may only raise them). `role` is the effective role. Other ways keep
/// `hz` and `slope` as they are.
pub(crate) fn tweeter_floor(role: MeasureRole, hz: f32, slope: u8) -> (f32, u8) {
    if role == MeasureRole::High {
        (
            if hz.is_nan() {
                HIGH_MIN_HZ
            } else {
                hz.max(HIGH_MIN_HZ)
            },
            slope.max(MIN_SLOPE_DB),
        )
    } else {
        (hz, slope)
    }
}

/// The rules over the mirror and the facts (pure: the tests call it).
pub(crate) fn judge(
    plan: &Plan,
    s: &DspState,
    profile: Option<&LimitsProfile>,
    facts: &Facts,
    options: &PreflightOptions,
) -> MeasurePreflight {
    let mut blockers = Vec::new();
    let mut warnings = Vec::new();
    let mut block = |key: &str, output: Option<u8>, msg: String| {
        blockers.push(MeasureIssue::new(key, output, msg));
    };
    if !facts.generator {
        block(
            "measure.no_generator",
            None,
            "the DSPi's firmware has no signal generator (v1.1.5 or later has one)".into(),
        );
    }
    if facts.read_only_client {
        block(
            "read_only.client",
            None,
            "this console is in read-only mode".into(),
        );
    }
    if facts.read_only_connection {
        block(
            "read_only",
            None,
            "this firmware's settings cannot be changed by this version".into(),
        );
    }
    if facts.viewer {
        block(
            "measure.viewer",
            None,
            "a Viewer on the hub may not play the generator".into(),
        );
    }
    if facts.page_locked {
        block(
            "measure.page_locked",
            None,
            "the installer has locked measuring on this DSPi".into(),
        );
    }
    if facts.guard_waiting {
        block(
            "measure.guard_pending",
            None,
            "a safe apply is waiting for Keep or Revert".into(),
        );
    }
    if facts.ab_running {
        block(
            "measure.ab_pending",
            None,
            "an A/B compare is running".into(),
        );
    }
    // S2: the volume.
    if s.user_volume_db.is_nan() || s.user_volume_db > VOL_USER_MAX_DB {
        block(
            "measure.volume_high",
            None,
            format!(
                "the volume is {:.1} dB; lower it to {VOL_USER_MAX_DB} dB or below",
                s.user_volume_db
            ),
        );
    }
    if let Some(p) = profile
        && let Err(v) = p.check_master_volume(s.master_volume_db)
    {
        block("measure.master_volume", None, v.msg);
    }
    // S8: silence on the inputs.
    let silence = 10f32.powf(INPUT_SILENCE_DBFS / 20.0);
    if let Some((i, peak)) = facts
        .input_peaks
        .iter()
        .enumerate()
        .find(|(_, p)| p.is_nan() || **p > silence)
    {
        block(
            "measure.input_audio",
            None,
            format!(
                "In {} plays at {:.0} dBFS: stop playback to measure",
                i.saturating_add(1),
                20.0 * peak.max(1e-9).log10()
            ),
        );
    }
    if let Err(issue) = plan.check_bounds(facts.device_rate_hz) {
        blockers.push(issue);
    }
    let mut block = |key: &str, output: Option<u8>, msg: String| {
        blockers.push(MeasureIssue::new(key, output, msg));
    };
    let pairs = {
        let mut m = [0u8; 4];
        for (d, v) in m.iter_mut().zip(&s.crossfeed_pair_map) {
            *d = *v;
        }
        PairMap(m)
    };
    for spec in &plan.outputs {
        let o = spec.output;
        let n = o.saturating_add(1);
        let bit = 1u16.checked_shl(u32::from(o)).unwrap_or(0);
        if o >= s.num_output_channels {
            block(
                "measure.no_output",
                Some(o),
                format!("the DSPi has no Out {n}"),
            );
            continue;
        }
        let idx = usize::from(o);
        if !s.output_enabled.get(idx).copied().unwrap_or(false) {
            block(
                "measure.output_off",
                Some(o),
                format!("Out {n} is switched off"),
            );
        }
        if s.output_muted.get(idx).copied().unwrap_or(false) {
            block(
                "measure.output_muted",
                Some(o),
                format!("Out {n} is muted"),
            );
        }
        // S6: the tweeter rule, on the effective role (-- WP ROLE0 --): a
        // high-pass at 1 kHz or above makes it a high way whatever its label.
        let chain = chain_of(s, o);
        let role = super::effective_role(spec.role, &chain);
        if let Some((hz, slope)) = tweeter_rule(role, o, profile) {
            let bands: Vec<BandSpec> = chain.peq.iter().chain(&chain.crossover).copied().collect();
            if !has_highpass(&bands, hz, slope) {
                let way = if role == MeasureRole::High {
                    "high"
                } else {
                    "middle"
                };
                // -- WP ROLL1 -- the caller's role (the plan's is resolved)
                let why = if role == spec.declared {
                    format!("Out {n} is a {way} way")
                } else if super::effective_role(spec.declared, &chain) != role {
                    // -- WP RCFIX -- raised by the user's confirmed layout
                    format!("Out {n} is a {way} way in the speaker layout")
                } else {
                    format!("Out {n} has a high-pass at 1 kHz or above, so it is a {way} way,")
                };
                block(
                    "measure.tweeter_highpass",
                    Some(o),
                    format!(
                        "{why} and has no active high-pass of at least {slope} dB/octave at \
                         {hz:.0} Hz or above"
                    ),
                );
            }
        }
        // S13: processing the maths does not model.
        if s.crossfeed_enabled {
            let on_pair = (0..PairMap::PAIRS).any(|p| {
                let pair_on = u32::try_from(p)
                    .ok()
                    .and_then(|p| 1u8.checked_shl(p))
                    .is_some_and(|b| s.crossfeed_output_pair_mask & b != 0);
                let (l, r) = pairs.pair(p);
                pair_on && (l == o || r == o)
            });
            if on_pair {
                block(
                    "measure.crossfeed",
                    Some(o),
                    format!("crossfeed is on for Out {n}: switch it off to measure"),
                );
            }
        }
        if s.psybass_enabled && s.psybass_output_mask & bit != 0 {
            block(
                "measure.bass",
                Some(o),
                format!("psychoacoustic bass is on for Out {n}: switch it off to measure"),
            );
        }
        let tube_on = s
            .tube
            .get(usize::from(dspi_proto::generated::tube::TUBE_PARAM_ENABLED))
            .is_some_and(|v| *v != 0.0);
        let tube_mask = s
            .tube
            .get(usize::from(
                dspi_proto::generated::tube::TUBE_PARAM_OUTPUT_MASK,
            ))
            .map_or(0u32, |v| {
                if v.is_finite() && *v >= 0.0 {
                    *v as u32
                } else {
                    0
                }
            });
        if tube_on && tube_mask & u32::from(bit) != 0 {
            block(
                "measure.tube",
                Some(o),
                format!("the tube preamp is on for Out {n}: switch it off to measure"),
            );
        }
        let sub_on = s
            .subharm
            .get(usize::from(SUBHARM_PARAM_ENABLED))
            .is_some_and(|v| *v != 0.0);
        let sub_mask = s
            .subharm
            .get(usize::from(SUBHARM_PARAM_MASK))
            .map_or(0u32, |v| {
                if v.is_finite() && *v >= 0.0 {
                    *v as u32
                } else {
                    0
                }
            });
        if sub_on && sub_mask & u32::from(bit) != 0 {
            block(
                "measure.subharmonic",
                Some(o),
                format!(
                    "the subharmonic synthesizer is on for Out {n}: switch it off to measure"
                ),
            );
        }
        if s.loudness_enabled && s.loudness_output_mask & bit != 0 {
            block(
                "measure.loudness",
                Some(o),
                format!("loudness is on for Out {n}: switch it off to measure"),
            );
        }
        let limiter_on = s.limiter_enabled.get(idx).copied().unwrap_or(false);
        let threshold = s.limiter_threshold_db.get(idx).copied().unwrap_or(0.0);
        if limiter_on && (threshold.is_nan() || threshold <= plan.ceiling_db) {
            block(
                "measure.limiter",
                Some(o),
                format!(
                    "Out {n}'s limiter acts at {threshold:.1} dBFS, at or below the run's \
                     ceiling of {:.1} dBFS: switch it off or raise its threshold",
                    plan.ceiling_db
                ),
            );
        }
        if let Some(expect) = &options.expect_chains
            && let Some((_, c)) = expect.iter().find(|(out, _)| *out == o)
            && *c != chain
        {
            block(
                "measure.chain_changed",
                Some(o),
                format!("Out {n}'s settings changed since it was measured"),
            );
        }
    }
    // Warnings.
    let input_eq = s
        .filters
        .iter()
        .take(usize::from(s.num_input_channels))
        .flatten()
        .any(|b| !b.bypass && b.filter_type != 0 && !(b.filter_type <= 3 && b.gain == 0.0));
    if input_eq {
        warnings.push(MeasureIssue::new(
            "measure.input_eq",
            None,
            "an input has EQ: it is outside what is measured",
        ));
    }
    if plan.headphones {
        warnings.push(MeasureIssue::new(
            "measure.coupler",
            None,
            "measure on the coupler, not on a head",
        ));
    }
    if plan.mic.is_none() {
        warnings.push(MeasureIssue::new(
            "measure.uncalibrated",
            None,
            "no microphone calibration: levels are relative and there is no SPL cap",
        ));
    }
    if s.user_mute {
        warnings.push(MeasureIssue::new(
            "measure.muted",
            None,
            "the volume is muted: the microphone will hear nothing",
        ));
    }
    if plan.kind == MeasureRunKind::Pink {
        // Nothing more: pink noise ramps itself the same way (S3).
    }
    MeasurePreflight {
        clear: blockers.is_empty(),
        blockers,
        warnings,
    }
}

/// The error `measure_start` answers for a preflight with blockers: the
/// existing codes where one fits (read-only, a Viewer), else the blockers'
/// words.
pub(crate) fn refusal(pre: &MeasurePreflight) -> Option<DspiError> {
    if pre.clear {
        return None;
    }
    let has = |k: &str| pre.blockers.iter().any(|b| b.key == k);
    if has("read_only.client") || has("read_only") {
        return Some(DspiError::ReadOnly);
    }
    if has("measure.viewer") {
        return Some(DspiError::Denied);
    }
    let words: Vec<&str> = pre.blockers.iter().map(|b| b.message.as_str()).collect();
    Some(DspiError::InvalidArgument {
        msg: format!("cannot measure: {}", words.join("; ")),
    })
}

impl DspiCore {
    /// The preflight of `plan` (see [`judge`]): reads the generator's caps,
    /// the meters and the device's rate, then judges the mirror.
    pub(crate) fn measure_preflight_plan(
        &self,
        plan: &Plan,
        options: &PreflightOptions,
    ) -> Result<MeasurePreflight> {
        self.lock().conn()?;
        let caps = self.siggen_fetch_caps()?;
        let generator = caps.supported
            && caps.type_count > SIG_LOG_SWEEP
            && caps
                .types
                .get(usize::from(SIG_LOG_SWEEP))
                .is_some_and(|t| t.id == SIG_LOG_SWEEP)
            && caps
                .types
                .get(usize::from(SIG_PINK))
                .is_some_and(|t| t.id == SIG_PINK);
        let device_rate_hz = self
            .fetch_stat(crate::consts::STAT_SAMPLE_RATE)
            .unwrap_or(0);
        let meters = self.fetch_status()?;
        let guard = self.apply_status();
        let guard_waiting = matches!(guard.phase, GuardPhase::Waiting | GuardPhase::Lost)
            && options.allow_guard != Some(guard.id);
        let ab_running = self.ab_state().active;
        #[cfg(feature = "link")]
        let viewer = self.link_admin_target().is_some_and(|t| t.role == "viewer");
        #[cfg(not(feature = "link"))]
        let viewer = false;
        let read_only_client = self.client_read_only();
        let mut inner = self.lock();
        let read_only_connection = !inner.conn()?.writable();
        let page_locked = inner.installer_locked()
            && inner
                .limits_profile()
                .is_some_and(|p| p.locked_pages.iter().any(|x| x == SCREEN_ID));
        let n_in = usize::from(inner.state.num_input_channels);
        let facts = Facts {
            generator,
            read_only_client,
            read_only_connection,
            viewer,
            page_locked,
            guard_waiting,
            ab_running,
            input_peaks: meters.peaks.iter().take(n_in).copied().collect(),
            device_rate_hz,
        };
        let profile = inner.limits_profile().cloned();
        Ok(judge(plan, &inner.state, profile.as_ref(), &facts, options))
    }
}
