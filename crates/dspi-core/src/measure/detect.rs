//! The speaker detection (WP ADET0, AUTODES sections 1 to 3; owner
//! 2026-11-23): on a device whose speakers nobody has set up, find which
//! outputs reach a driver and which drive a tweeter, by its roll-off, and
//! mark them in the speaker layout so the safety rules apply to them.
//!
//! **The principle** (AUTODES 1.1): an output with no trusted role is an
//! *unknown output*, played as a tweeter would be: nothing below its
//! **floor** (`max(1 kHz, the limits profile's highpass_min_hz for it, the
//! caller's floor)`) but the shaped probe, and never above the probe's
//! ceiling ([`PROBE_CEILING_DBFS`]). Below the floor it receives only a log
//! sweep of at most two octaves at [`PROBE_BELOW_FLOOR_DBFS`], and only while
//! the response measured so far says the driver is still playing healthily.
//! Whatever cannot be told apart is a tweeter.
//!
//! **The stages, one output at a time** (each an ordinary `liveSweep`
//! session, so every rule of the session holds):
//!
//! - **A and B** (one session): the ramp in the presence band, 4 to 16 kHz,
//!   from the start level up to the probe's ceiling, stopping at
//!   [`PROBE_SNR_DB`] of in-band SNR; then one 1.5 s sweep from the floor to
//!   20 kHz at that level: the top end's shape. **A2** when the top band is
//!   not heard: the same with the ramp from the floor to 4 kHz.
//! - **Top holds** (the 1.5–3 kHz mean less the 10–16 kHz mean under
//!   [`PROBE_TOP_FALL_DB`]): a tweeter, a dome midrange or a full-range
//!   driver; no descent.
//! - **C1, the shaped descent:** segments from `floor/√2^k`, k = 1..5, each
//!   two octaves up, at the shaped level, no ramp; stops when a segment's
//!   new half octave is not heard or falls [`PROBE_STOP_DB`] under the
//!   loudest point so far. **C2, the free descent** (125, 63, 32, 20 Hz, at
//!   the presence level, at most the ceiling) only for an output that
//!   reached 177 Hz (for a 1 kHz floor) in C1 without a stop: no tweeter does.
//! - The class from `f_low` (the −6 dB point) and the 1–2 kHz level.
//!
//! **The layout** is written as each output completes (its row `detected`);
//! from the first write the layout is [`LayoutState::AutoDetected`] and the
//! outputs still to come are `unknown`, role high, until their turn. A
//! detection never writes the device's configuration; outputs that are off
//! or muted are not probed ("off: not probed"). v1 refuses a detection on a
//! confirmed layout.
//!
//! **Driving it:** [`DspiCore::detect_start`], then
//! [`DspiCore::detect_tick`] about every 50 ms (instead of `measure_tick`)
//! while `running`, pushing capture with `measure_capture_push` whenever
//! `session.wantsCapture`.
//!
//! **Locks:** the detection's own mutex is taken before the session's slot,
//! never after it; what it shows is a leaf (`detect_view`).

use std::sync::atomic::Ordering;

use dspi_measure::doc::MeasurementDoc;
use serde::{Deserialize, Serialize};

use super::plan::{PROBE_SPAN, Plan, Probe};
use super::{
    MeasureCoupler, MeasureIssue, MeasureOutput, MeasurePhase, MeasurePlacement, MeasureRole,
    MeasureRunKind, MeasureSetup, MeasureState, PreflightOptions,
};
use crate::speaker_layout::{
    DetectClass, DetectConfidence, DetectHeard, DetectedOutput, Detection, LayoutState, RoleSource,
    SpeakerLayout, SpeakerLayoutOutput,
};
use crate::{DspiCore, DspiError, Result};

// ── The constants (AUTODES 1.2) ─────────────────────────────────────────

/// The ramp's first level, peak dBFS (as measuring).
pub const PROBE_START_DBFS: f32 = -50.0;
/// On a headphone coupler.
pub const PROBE_START_HEADPHONES_DBFS: f32 = -60.0;
/// The most any probe plays on an unknown output above its floor, peak dBFS.
pub const PROBE_CEILING_DBFS: f32 = -24.0;
/// On a headphone coupler.
pub const PROBE_CEILING_HEADPHONES_DBFS: f32 = -36.0;
/// The most any probe plays below an unknown output's floor, peak dBFS
/// (owner 2026-11-23: the damage oracle's confirmed-tweeter limit).
pub const PROBE_BELOW_FLOOR_DBFS: f32 = -50.0;
/// On a headphone coupler.
pub const PROBE_BELOW_FLOOR_HEADPHONES_DBFS: f32 = -60.0;
/// The ramp stops at this in-band SNR, dB.
pub const PROBE_SNR_DB: f64 = 20.0;
/// The classification sweep, ms (the ramp's steps are the session's 1 s).
pub const PROBE_SWEEP_MS: u32 = 1_500;
/// A descent segment's sweep, ms.
pub const PROBE_SEG_MS: u32 = 1_000;
/// The presence band, Hz (stage A).
pub const PROBE_TOP_BAND: (f32, f32) = (4_000.0, 16_000.0);
/// The top-end test's bands, Hz: A (raised with the floor) and C.
pub const PROBE_SHAPE_A: (f64, f64) = (1_500.0, 3_000.0);
pub const PROBE_SHAPE_C: (f64, f64) = (10_000.0, 16_000.0);
/// C under A by this much: the top falls (a non-tweeter candidate), dB.
pub const PROBE_TOP_FALL_DB: f64 = 10.0;
/// Under this the top holds plainly ("tweeter or full range"), dB; from
/// here to [`PROBE_TOP_FALL_DB`] it is ambiguous (a tweeter, low confidence).
pub const PROBE_TOP_UNSURE_DB: f64 = 6.0;
/// The roll-off seen inside the band above the floor: a tweeter of high
/// confidence, dB (owner 2026-11-23: 2.5).
pub const PROBE_LOW_TURN_DB: f64 = 2.5;
/// A descent segment's new band this far under the reference: stop, dB.
pub const PROBE_STOP_DB: f64 = 8.0;
/// `f_low`: where the response is this far under the reference, dB.
pub const PROBE_LOW_END_DB: f64 = 6.0;
/// `f_low` at or above this: a tweeter, Hz.
pub const PROBE_TWEETER_MIN_HZ: f64 = 200.0;
/// `f_low` from this to the tweeter's: a midrange; below, a woofer or a sub,
/// Hz.
pub const PROBE_MID_MIN_HZ: f64 = 70.0;
/// The 1–2 kHz level this far under the reference: a sub, dB.
pub const PROBE_SUB_TOP_DB: f64 = 20.0;
/// A stop in C1 at or above this: a tweeter of high confidence, Hz (for a
/// 1 kHz floor; scaled with the floor).
pub const PROBE_TWEETER_SURE_HZ: f64 = 354.0;
/// The shaped descent's steps below the floor: `floor/√2^k`, k = 1 to this.
pub const PROBE_SHAPED_STEPS: u32 = 5;
/// The free descent's steps, Hz.
pub const PROBE_FREE_STEPS: [f32; 4] = [125.0, 63.0, 32.0, 20.0];
/// The lowest floor, and the highest a caller may name, Hz.
pub const PROBE_FLOOR_MIN_HZ: f32 = 1_000.0;
/// The highest floor a caller may name, Hz. -- WP ADET1 -- (now exported)
pub const PROBE_FLOOR_MAX_HZ: f32 = 8_000.0;
/// The top of the A2 ramp's band, Hz.
const A2_TOP_HZ: f32 = 4_000.0;
/// The smoothing of a judged response, octaves (the roll-off search's).
const SMOOTH_OCT: f64 = 1.0 / 3.0;
/// A response is judged from this far inside its band's edges, octaves.
const EDGE_OCT: f64 = 1.0 / 6.0;
/// The roll-off seen at the band's bottom is read over this much above the
/// floor, octaves.
const LOW_TURN_OCT: f64 = 0.25;
/// Neighbouring segments are aligned by at most this, dB.
const ALIGN_MAX_DB: f64 = 6.0;

// ── The notices (assets/notices/notices.json, family `detect`) ──────────

pub(crate) const KEY_STARTED: &str = "detect.started";
pub(crate) const KEY_DONE: &str = "detect.done";
pub(crate) const KEY_STOPPED: &str = "detect.stopped";
pub(crate) const KEY_NOTHING: &str = "detect.nothing";
pub(crate) const KEY_OFF: &str = "detect.off";
pub(crate) const KEY_UNKNOWN: &str = "detect.unknown";
pub(crate) const KEY_TWEETER: &str = "detect.tweeter";
pub(crate) const KEY_TWEETER_UNSURE: &str = "detect.tweeter_unsure";
pub(crate) const KEY_TWEETER_UNCERTAIN: &str = "detect.tweeter_uncertain";
pub(crate) const KEY_MIDRANGE: &str = "detect.midrange";
pub(crate) const KEY_WOOFER: &str = "detect.woofer";
pub(crate) const KEY_SUB: &str = "detect.sub";
pub(crate) const KEY_CONFIRMED: &str = "detect.confirmed_layout";

/// Every notice key of the detection (its rows in `notices.json`).
pub const DETECT_NOTICE_KEYS: &[&str] = &[
    KEY_STARTED,
    KEY_DONE,
    KEY_STOPPED,
    KEY_NOTHING,
    KEY_OFF,
    KEY_UNKNOWN,
    KEY_TWEETER,
    KEY_TWEETER_UNSURE,
    KEY_TWEETER_UNCERTAIN,
    KEY_MIDRANGE,
    KEY_WOOFER,
    KEY_SUB,
    KEY_CONFIRMED,
];

fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

// ── Records ─────────────────────────────────────────────────────────────

/// What to detect (`detect_start`). Every field may be left out.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DetectSetup {
    /// The outputs, 0-based; empty for every output of the device.
    pub outputs: Vec<u8>,
    /// Where the microphone is: the listening position (default), near
    /// field, or a headphone coupler (the headphones' levels).
    pub placement: MeasurePlacement,
    /// The coupler, for headphones.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub coupler: Option<MeasureCoupler>,
    /// The floor for every output, Hz: 0 for 1 kHz; may only raise it (1000
    /// to 8000). The limits profile's `highpass_min_hz` raises an output's
    /// own further.
    pub floor_hz: f32,
    /// The microphone's `mic` document's text ("" uncalibrated).
    pub mic_document: String,
    /// The SPL cap with a calibrated microphone (as `MeasureSetup`).
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub spl_cap_db: Option<f32>,
    /// The capture's rate, Hz (0: 48000).
    pub capture_rate_hz: u32,
}

/// Where the detection stands.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DetectStage {
    /// No detection on this core.
    #[default]
    Idle,
    /// An output's presence ramp (stage A or A2).
    Ramp,
    /// Its shape above the floor (stage B).
    Shape,
    /// Its descent below the floor (C1 or C2).
    Descent,
    /// Every output was probed.
    Done,
    /// It stopped before the end (`issue` says why).
    Stopped,
}

/// The detection's state.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectState {
    /// Where it stands.
    pub stage: DetectStage,
    /// Tick it (`detect_tick`): a session runs or is next.
    pub running: bool,
    /// The output probed now (or last).
    pub output: Option<u8>,
    /// Outputs finished so far.
    pub outputs_done: u16,
    /// Outputs to probe (those off or muted are not).
    pub outputs_total: u16,
    /// The output's floor, Hz.
    pub floor_hz: f32,
    /// The segment's start (or the band's bottom), Hz.
    pub segment_hz: f32,
    /// The level it plays at (the ramp's ceiling while ramping), peak dBFS.
    pub level_dbfs: f32,
    /// The session (push capture while it wants it).
    pub session: MeasureState,
    /// The detection so far: each output's class and evidence, the summary.
    pub detection: Option<Detection>,
    /// Why it stopped, or what the last session asked.
    pub issue: Option<MeasureIssue>,
    /// Who started it: 0 the app, else an agent's actor.
    pub actor: u32,
}

// ── The judgement (pure) ────────────────────────────────────────────────

/// A response's trusted points between `lo` and `hi` (inside their edges),
/// smoothed to a third of an octave: `(Hz, dB)`, rising.
pub(crate) fn smoothed(
    freqs: &[f64],
    mag: &[f64],
    valid: Option<&[bool]>,
    lo: f64,
    hi: f64,
) -> Vec<(f64, f64)> {
    smooth(&trusted(freqs, mag, valid, lo, hi))
}

/// A response's trusted points between `lo` and `hi`, inside their edges:
/// `(Hz, dB)`, rising.
pub(crate) fn trusted(
    freqs: &[f64],
    mag: &[f64],
    valid: Option<&[bool]>,
    lo: f64,
    hi: f64,
) -> Vec<(f64, f64)> {
    let from = lo * 2f64.powf(EDGE_OCT);
    let to = hi / 2f64.powf(EDGE_OCT);
    freqs
        .iter()
        .zip(mag)
        .enumerate()
        .filter(|(i, (f, m))| {
            **f >= from
                && **f <= to
                && f.is_finite()
                && m.is_finite()
                && valid.is_none_or(|v| v.get(*i).copied().unwrap_or(false))
        })
        .map(|(_, (f, m))| (*f, *m))
        .collect()
}

/// `pts` smoothed to a third of an octave.
pub(crate) fn smooth(pts: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let half = SMOOTH_OCT / 2.0;
    pts.iter()
        .map(|(f, _)| {
            let (sum, n) = pts
                .iter()
                .filter(|(g, _)| (g / f).log2().abs() <= half)
                .fold((0.0, 0u32), |(s, n), (_, m)| (s + m, n.saturating_add(1)));
            (*f, sum / f64::from(n.max(1)))
        })
        .collect()
}

/// The mean of `pts` from `lo` to `hi`; `None` with fewer than two points.
pub(crate) fn band_mean(pts: &[(f64, f64)], lo: f64, hi: f64) -> Option<f64> {
    let (sum, n) = pts
        .iter()
        .filter(|(f, _)| *f >= lo && *f <= hi)
        .fold((0.0, 0u32), |(s, n), (_, m)| (s + m, n.saturating_add(1)));
    (n >= 2).then(|| sum / f64::from(n))
}

/// The loudest of `pts`, dB.
fn loudest(pts: &[(f64, f64)]) -> Option<f64> {
    pts.iter().map(|p| p.1).reduce(f64::max)
}

/// Stage B's measures of a response above the floor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Shape {
    pub(crate) reference_db: f64,
    /// A − C, dB; `None` when C was not heard (it fell into the noise).
    pub(crate) top_fall_db: Option<f64>,
    /// The reference less the half octave above the floor, dB.
    pub(crate) low_turn_db: Option<f64>,
    /// The top falls: a descent may follow.
    pub(crate) falls: bool,
}

/// Judge stage B: `raw` the trusted response from `floor` up.
pub(crate) fn judge_shape(raw: &[(f64, f64)], floor: f64) -> Option<Shape> {
    let smoothed = smooth(raw);
    let pts = smoothed.as_slice();
    let reference_db = loudest(pts)?;
    let a_lo = PROBE_SHAPE_A.0.max(floor);
    let a_hi = PROBE_SHAPE_A.1.max(floor * std::f64::consts::SQRT_2);
    // A band the floor leaves no points in: the lowest third of an octave
    // that has some.
    let a = band_mean(pts, a_lo, a_hi).or_else(|| {
        let first = pts.first()?.0;
        band_mean(pts, first, first * 2f64.powf(SMOOTH_OCT))
    });
    let c = band_mean(pts, PROBE_SHAPE_C.0, PROBE_SHAPE_C.1);
    let top_fall_db = a.zip(c).map(|(a, c)| a - c);
    let falls = match (a, c) {
        (Some(_), Some(_)) => top_fall_db.is_some_and(|d| d >= PROBE_TOP_FALL_DB),
        // The top fell into the noise.
        (Some(_), None) => true,
        // Only the top heard: it rises.
        _ => false,
    };
    // The response at the bottom of the band, unsmoothed (a third of an
    // octave there reaches into the band): the lowest quarter octave above
    // the floor (its trusted part starts a sixth of an octave up).
    let low_turn_db = band_mean(raw, floor, floor * 2f64.powf(LOW_TURN_OCT))
        .or_else(|| raw.first().map(|p| p.1))
        .map(|low| (reference_db - low).max(0.0));
    Some(Shape {
        reference_db,
        top_fall_db,
        low_turn_db,
        falls,
    })
}

/// A tweeter's confidence from a top that holds.
pub(crate) fn top_holds_confidence(s: &Shape) -> DetectConfidence {
    if s.top_fall_db.is_some_and(|d| d >= PROBE_TOP_UNSURE_DB) {
        DetectConfidence::Low
    } else if s.low_turn_db.is_some_and(|d| d >= PROBE_LOW_TURN_DB) {
        DetectConfidence::High
    } else {
        DetectConfidence::Medium
    }
}

/// The descent's segment starts below `floor`: the shaped ones, then the
/// free ones under the last shaped one.
pub(crate) fn shaped_steps(floor: f32) -> Vec<f32> {
    (1..=PROBE_SHAPED_STEPS)
        .map(|k| floor / std::f32::consts::SQRT_2.powi(i32::try_from(k).unwrap_or(1)))
        .collect()
}

/// The sweep of the descent segment that judges the half octave above
/// `f_k`: from a sixth of an octave under it (the analysis tapers a band's
/// edge), two octaves up (the interlock's widest).
pub(crate) fn segment_band(f_k: f32) -> (f32, f32) {
    let f1 = (f_k / 2f32.powf(EDGE_OCT as f32)).max(20.0);
    (f1, (f1 * PROBE_SPAN).min(20_000.0))
}

pub(crate) fn free_steps(shaped_last: f32) -> Vec<f32> {
    PROBE_FREE_STEPS
        .iter()
        .copied()
        .filter(|f| *f < shaped_last / 2f32.powf(EDGE_OCT as f32))
        .collect()
}

/// One judged band of the descent (or B's half octave above the floor).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Band {
    pub(crate) lo: f64,
    pub(crate) hi: f64,
    /// Its mean, dB; `None` not heard.
    pub(crate) mean: Option<f64>,
    /// The reference when it was judged (the loudest point so far), dB: a
    /// room mode found lower down does not move where the band stood.
    pub(crate) reference: f64,
}

/// `f_low`: where the low end's roll-off crosses 6 dB under the reference
/// each band was judged against, found from the lowest band up (a ripple
/// higher up that recovered is not the roll-off), on log frequency between
/// the bands' centres; an unheard band counts as under. `None` when the
/// lowest band is not under.
pub(crate) fn f_low(bands: &[Band]) -> Option<f64> {
    let centre = |b: &Band| (b.lo * b.hi).sqrt();
    // A band's level re its own limit (under it: negative).
    let over = |b: &Band| b.mean.map(|m| m - (b.reference - PROBE_LOW_END_DB));
    let mut below: Option<&Band> = None;
    for b in bands.iter().rev() {
        match over(b) {
            Some(d) if d > 0.0 => {
                let under = below?;
                let Some(du) = over(under) else {
                    // Unheard below: the crossing at its top.
                    return Some(under.hi);
                };
                let t = (d / (d - du)).clamp(0.0, 1.0);
                return Some((centre(b).ln() + t * (centre(under).ln() - centre(b).ln())).exp());
            }
            _ => below = Some(b),
        }
    }
    // Under all the way up: the top band's centre.
    below.map(centre)
}

/// The class of a descent that ended (AUTODES 1.6): `f_low` (`None` not
/// reached), the 1–2 kHz level, the reference, where C1 stopped, the floor.
pub(crate) fn classify_descent(
    f_low_hz: Option<f64>,
    level_1k2k: Option<f64>,
    reference_db: f64,
    stopped_at_hz: Option<f64>,
    floor: f64,
) -> (DetectClass, DetectConfidence) {
    let scale = floor / f64::from(PROBE_FLOOR_MIN_HZ);
    match f_low_hz {
        Some(f) if f >= PROBE_TWEETER_MIN_HZ => {
            let sure = stopped_at_hz.is_some_and(|s| s >= PROBE_TWEETER_SURE_HZ * scale * 0.99);
            (
                DetectClass::Tweeter,
                if sure {
                    DetectConfidence::High
                } else {
                    DetectConfidence::Medium
                },
            )
        }
        Some(f) if f >= PROBE_MID_MIN_HZ => (DetectClass::Midrange, DetectConfidence::High),
        _ => match level_1k2k {
            Some(l) if l < reference_db - PROBE_SUB_TOP_DB => {
                (DetectClass::Sub, DetectConfidence::High)
            }
            Some(_) => (DetectClass::Woofer, DetectConfidence::High),
            // Nothing heard at 1–2 kHz at all: far under the reference.
            None => (DetectClass::Sub, DetectConfidence::Medium),
        },
    }
}

/// The role a class gives.
pub(crate) fn role_of(class: DetectClass) -> Option<MeasureRole> {
    match class {
        DetectClass::Tweeter | DetectClass::Unknown => Some(MeasureRole::High),
        DetectClass::Midrange => Some(MeasureRole::Mid),
        DetectClass::Woofer => Some(MeasureRole::Low),
        DetectClass::Sub => Some(MeasureRole::Sub),
        DetectClass::FullRange => Some(MeasureRole::Full),
        DetectClass::Nothing | DetectClass::Off => None,
    }
}

/// The notice of an output's class, where the user should read one.
fn note_of(class: DetectClass, confidence: DetectConfidence) -> Option<&'static str> {
    match (class, confidence) {
        (DetectClass::Nothing, _) => Some(KEY_NOTHING),
        (DetectClass::Off, _) => Some(KEY_OFF),
        (DetectClass::Unknown, _) => Some(KEY_UNKNOWN),
        (DetectClass::Tweeter, DetectConfidence::Medium) => Some(KEY_TWEETER_UNSURE),
        (DetectClass::Tweeter, DetectConfidence::Low) => Some(KEY_TWEETER_UNCERTAIN),
        _ => None,
    }
}

/// Out N, Out N–M, Out N, M: outputs in the user's numbering.
fn outs_words(outs: &[u8]) -> String {
    let mut v: Vec<u16> = outs
        .iter()
        .map(|o| u16::from(*o).saturating_add(1))
        .collect();
    v.sort_unstable();
    v.dedup();
    let contiguous = v
        .windows(2)
        .all(|w| w.get(1).copied() == w.first().map(|a| a.saturating_add(1)));
    match (v.first(), v.last()) {
        (Some(a), Some(b)) if v.len() > 2 && contiguous => format!("Out {a}–{b}"),
        _ => format!(
            "Out {}",
            v.iter().map(u16::to_string).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// The detection in words (AUTODES 2): "2 tweeters, 2 woofers, 1 midrange?,
/// 1 subwoofer; nothing heard on Out 4; Out 7–8 off, not probed."
pub(crate) fn summary_of(d: &Detection, probed: usize) -> String {
    let count = |class: DetectClass| {
        let all: Vec<&DetectedOutput> = d.outputs.iter().filter(|o| o.class == class).collect();
        let unsure = all.iter().any(|o| o.confidence != DetectConfidence::High);
        (all.len(), unsure)
    };
    let mut parts = Vec::new();
    for (class, one, many) in [
        (DetectClass::Tweeter, "tweeter", "tweeters"),
        (DetectClass::Woofer, "woofer", "woofers"),
        (DetectClass::Midrange, "midrange", "midranges"),
        (DetectClass::Sub, "subwoofer", "subwoofers"),
    ] {
        let (n, unsure) = count(class);
        if n > 0 {
            parts.push(format!(
                "{n} {}{}",
                if n == 1 { one } else { many },
                if unsure { "?" } else { "" }
            ));
        }
    }
    let of = |class: DetectClass| -> Vec<u8> {
        d.outputs
            .iter()
            .filter(|o| o.class == class)
            .map(|o| o.output)
            .collect()
    };
    let mut text = parts.join(", ");
    let mut more = Vec::new();
    let unknown = of(DetectClass::Unknown);
    if !unknown.is_empty() {
        more.push(format!(
            "{} unknown, treated as tweeters",
            outs_words(&unknown)
        ));
    }
    let nothing = of(DetectClass::Nothing);
    if !nothing.is_empty() {
        more.push(format!("nothing heard on {}", outs_words(&nothing)));
    }
    let off = of(DetectClass::Off);
    if !off.is_empty() {
        more.push(format!("{} off, not probed", outs_words(&off)));
    }
    if d.stopped {
        let total = d
            .outputs
            .iter()
            .filter(|o| o.class != DetectClass::Off)
            .count();
        more.push(format!("stopped after {probed} of {total} outputs"));
    }
    if text.is_empty() {
        text = more.join("; ");
    } else if !more.is_empty() {
        text = format!("{text}; {}", more.join("; "));
    }
    if text.is_empty() {
        "no outputs".to_owned()
    } else {
        format!("{text}.")
    }
}

/// `trusted` roles of `layout`: each output's role, by output, when its
/// state is confirmed or auto-detected; `None` when it is neither (nobody's
/// word: no roles reach the safety rules). AUTODES 3.2.
pub fn trusted_roles(layout: &SpeakerLayout) -> Option<Vec<Option<MeasureRole>>> {
    if layout.state() == LayoutState::Unconfirmed {
        return None;
    }
    let mut out: Vec<Option<MeasureRole>> = Vec::new();
    for o in &layout.outputs {
        let at = usize::from(o.output);
        if out.len() <= at {
            out.resize(at.saturating_add(1), None);
        }
        if let Some(slot) = out.get_mut(at) {
            *slot = Some(o.role);
        }
    }
    Some(out)
}

// ── The driver ──────────────────────────────────────────────────────────

/// Where an output's probe stands.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Step {
    /// Stages A and B (the ramp in the presence band).
    Top,
    /// Stages A2 and B (the ramp from the floor to 4 kHz).
    Band,
    /// A descent segment, an index into `Work::steps`.
    Descent(usize),
}

/// One output's probe.
#[derive(Debug, Clone)]
struct Work {
    output: u8,
    floor: f32,
    step: Step,
    /// The descent's starts: the shaped ones, then (once entered) the free.
    steps: Vec<f32>,
    /// The shaped steps' count (C2 starts after them).
    shaped: usize,
    heard: DetectHeard,
    /// The presence level, peak dBFS.
    level: Option<f32>,
    /// Every trusted, smoothed point so far, aligned.
    pts: Vec<(f64, f64)>,
    bands: Vec<Band>,
    shape: Option<Shape>,
    lowest_hz: Option<f32>,
    /// The descent's verdicts: the −6 dB point, the segment it stopped at.
    f_low_hz: Option<f64>,
    stopped_at: Option<f32>,
}

impl Work {
    fn new(output: u8, floor: f32) -> Work {
        let steps = shaped_steps(floor);
        Work {
            output,
            floor,
            step: Step::Top,
            shaped: steps.len(),
            steps,
            heard: DetectHeard::Never,
            level: None,
            pts: Vec::new(),
            bands: Vec::new(),
            shape: None,
            lowest_hz: None,
            f_low_hz: None,
            stopped_at: None,
        }
    }

    fn played(&mut self, hz: f32) {
        self.lowest_hz = Some(self.lowest_hz.map_or(hz, |l| l.min(hz)));
    }

    fn record(&self, class: DetectClass, confidence: DetectConfidence) -> DetectedOutput {
        DetectedOutput {
            output: self.output,
            class,
            confidence,
            role: role_of(class),
            floor_hz: self.floor,
            heard: self.heard,
            level_dbfs: self.level,
            top_fall_db: self
                .shape
                .and_then(|s| s.top_fall_db)
                .map(|d| round1(d) as f32),
            low_turn_db: self
                .shape
                .and_then(|s| s.low_turn_db)
                .map(|d| round1(d) as f32),
            f_low_hz: self.f_low_hz.map(|f| f as f32),
            lowest_hz: self.lowest_hz,
            stopped_at_hz: self.stopped_at,
            note: note_of(class, confidence).map(str::to_owned),
            probed_ms: crate::events::now_ms(),
        }
    }
}

/// What follows a judged session.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Next {
    /// The work's next session (its step moved on).
    Session,
    /// The output is classified.
    Done(DetectClass, DetectConfidence),
}

impl Work {
    /// The presence ramp heard nothing: the ramp from the floor to 4 kHz,
    /// or (after it) the shaped probe below the floor.
    fn not_heard_above(&mut self) -> Next {
        self.step = if self.step == Step::Top && self.floor * 2f32.powf(EDGE_OCT as f32) < A2_TOP_HZ
        {
            Step::Band
        } else {
            Step::Descent(0)
        };
        Next::Session
    }

    /// Stage B judged from `pts` (smoothed, from the floor up): a tweeter
    /// when the top holds, else the descent. `None` when nothing could be
    /// judged.
    fn after_shape(&mut self, raw: Vec<(f64, f64)>) -> Option<Next> {
        let floor = f64::from(self.floor);
        let shape = judge_shape(&raw, floor)?;
        let pts = smooth(&raw);
        self.heard = if self.step == Step::Top {
            DetectHeard::Top
        } else {
            DetectHeard::AboveFloor
        };
        self.shape = Some(shape);
        let hi = floor * std::f64::consts::SQRT_2;
        self.bands.push(Band {
            lo: floor,
            hi,
            mean: band_mean(&pts, floor, hi),
            reference: shape.reference_db,
        });
        self.pts = pts;
        if !shape.falls {
            return Some(Next::Done(
                DetectClass::Tweeter,
                top_holds_confidence(&shape),
            ));
        }
        self.step = Step::Descent(0);
        Some(Next::Session)
    }

    // -- WP ADETS --
    /// Stage B, or (no trusted point above the floor in its sweep: the
    /// ramp heard only an edge, as a sub's tail at 1 kHz) silent above the
    /// floor: AUTODES 1.3 step 3, the next ramp or the shaped descent.
    fn after_top(&mut self, raw: Vec<(f64, f64)>) -> Next {
        match self.after_shape(raw) {
            Some(next) => next,
            None => self.not_heard_above(),
        }
    }

    /// A descent segment judged from `seg` (smoothed over its band, empty
    /// when not heard): on down, or the class.
    fn after_segment(&mut self, seg: Vec<(f64, f64)>) -> Next {
        let Step::Descent(k) = self.step else {
            return Next::Done(DetectClass::Unknown, DetectConfidence::Low);
        };
        let f_k = self.steps.get(k).copied().unwrap_or(self.floor);
        let upper = match k {
            0 => self.floor,
            _ => self
                .steps
                .get(k.saturating_sub(1))
                .copied()
                .unwrap_or(self.floor),
        };
        // Aligned on the band it shares with what came before.
        let seg = align(&self.pts, seg);
        let mut band = Band {
            lo: f64::from(f_k),
            hi: f64::from(upper),
            mean: band_mean(&seg, f64::from(f_k), f64::from(upper)),
            reference: f64::NEG_INFINITY,
        };
        let silent_above = self.heard == DetectHeard::Never;
        self.pts.extend(seg);
        self.pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let next = k.saturating_add(1);
        if silent_above {
            if band.mean.is_none() {
                // Not heard yet: on down the shaped descent; nothing reached
                // the microphone when it ends.
                if next < self.shaped {
                    self.step = Step::Descent(next);
                    return Next::Session;
                }
                return Next::Done(DetectClass::Nothing, DetectConfidence::High);
            }
            self.heard = DetectHeard::BelowFloor;
        }
        let reference = loudest(&self.pts).unwrap_or(f64::NEG_INFINITY);
        band.reference = reference;
        self.bands.push(band);
        let stop = band.mean.is_none_or(|m| m <= reference - PROBE_STOP_DB);
        if !stop {
            if next == self.shaped && self.steps.len() == self.shaped {
                // Flat (or rising) 2.5 octaves under the floor at the shaped
                // level: no tweeter. The free descent.
                self.steps.extend(free_steps(f_k));
            }
            if next < self.steps.len() {
                self.step = Step::Descent(next);
                return Next::Session;
            }
        }
        // Stopped, or the free descent's end (f_low below 20 Hz).
        self.f_low_hz = if stop { f_low(&self.bands) } else { None };
        // Past the shaped descent with no stop it is no tweeter: a room's
        // ripple above its roll-off does not make it one.
        if k >= self.shaped
            && let Some(last) = self.steps.get(self.shaped.saturating_sub(1))
        {
            self.f_low_hz = self.f_low_hz.map(|f| f.min(f64::from(*last)));
        }
        self.stopped_at = stop.then_some(f_k);
        let level_1k2k = band_mean(&self.pts, 1_000.0, 2_000.0)
            .or_else(|| self.bands.first().and_then(|b| b.mean));
        let in_shaped = (stop && k < self.shaped).then_some(f64::from(f_k));
        let (class, confidence) = classify_descent(
            self.f_low_hz,
            level_1k2k,
            reference,
            in_shaped,
            f64::from(self.floor),
        );
        Next::Done(class, confidence)
    }
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// The detection of one core.
#[derive(Debug, Default)]
pub(crate) struct Detect {
    setup: Option<DetectSetup>,
    stage: DetectStage,
    /// A session runs.
    running: bool,
    /// The outputs to probe, in order, and the one now.
    queue: Vec<(u8, f32)>,
    at: usize,
    work: Option<Work>,
    detection: Detection,
    issue: Option<MeasureIssue>,
    actor: u32,
    headphones: bool,
    /// The level the session plays at, peak dBFS.
    level_now: f32,
    segment_hz: f32,
    /// The layout when it started: the user's rows, names and groups kept.
    base: Option<SpeakerLayout>,
}

impl Detect {
    fn state(&self, session: MeasureState) -> DetectState {
        let ramp_or_shape = if session.phase == MeasurePhase::Ramp
            || session.phase == MeasurePhase::PreRoll && session.run_level_db.is_none()
        {
            DetectStage::Ramp
        } else {
            DetectStage::Shape
        };
        let stage = match (self.stage, self.work.as_ref().map(|w| w.step)) {
            (DetectStage::Ramp | DetectStage::Shape, Some(Step::Descent(_))) => {
                DetectStage::Descent
            }
            (DetectStage::Ramp | DetectStage::Shape, Some(_)) => ramp_or_shape,
            (s, _) => s,
        };
        let running = matches!(
            self.stage,
            DetectStage::Ramp | DetectStage::Shape | DetectStage::Descent
        );
        DetectState {
            stage,
            running,
            output: self.work.as_ref().map(|w| w.output),
            outputs_done: u16::try_from(self.at.min(self.queue.len())).unwrap_or(u16::MAX),
            outputs_total: u16::try_from(self.queue.len()).unwrap_or(u16::MAX),
            floor_hz: self.work.as_ref().map_or(0.0, |w| w.floor),
            segment_hz: self.segment_hz,
            level_dbfs: self.level_now,
            session,
            detection: (self.setup.is_some()).then(|| self.detection.clone()),
            issue: self.issue.clone(),
            actor: self.actor,
        }
    }

    /// The detection's records of this run's outputs only (RGFIX): those
    /// carried from the detection before are not counted in its summary.
    fn this_run(&self) -> Detection {
        let asked = self
            .setup
            .as_ref()
            .map(|s| s.outputs.as_slice())
            .unwrap_or_default();
        let mut run = self.detection.clone();
        if !asked.is_empty() {
            run.outputs.retain(|o| asked.contains(&o.output));
        }
        run
    }

    fn levels(&self) -> (f32, f32, f32) {
        if self.headphones {
            (
                PROBE_START_HEADPHONES_DBFS,
                PROBE_CEILING_HEADPHONES_DBFS,
                PROBE_BELOW_FLOOR_HEADPHONES_DBFS,
            )
        } else {
            (PROBE_START_DBFS, PROBE_CEILING_DBFS, PROBE_BELOW_FLOOR_DBFS)
        }
    }
}

impl super::MeasureSlot {
    fn detect_lock(&self) -> std::sync::MutexGuard<'_, Detect> {
        self.detect.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// A detection runs on this core.
    pub(crate) fn detect_on(&self) -> bool {
        self.detect_view
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .running
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Start the speaker detection (module docs): each output probed in
    /// turn, quietly, from the top down; the speaker layout written as each
    /// output completes. Outputs that are off or muted are not probed.
    ///
    /// Errors: as `measure_start` (a blocker, read-only, a Viewer, the device
    /// lock); [`DspiError::InvalidArgument`] for a setup that is not one, a
    /// confirmed speaker layout (`detect.confirmed_layout`: v1 never detects
    /// over the user's word), or while a detection, a session, a roll-off
    /// search, a tune's run or the live view runs on this core.
    pub fn detect_start(&self, setup: DetectSetup) -> Result<DetectState> {
        let actor = crate::actor::JobActor::capture(self).actor();
        let mut d = self.measure.detect_lock();
        if matches!(
            d.stage,
            DetectStage::Ramp | DetectStage::Shape | DetectStage::Descent
        ) {
            return Err(invalid("busy: a speaker detection is already running"));
        }
        if self.measure_rolloff_state().running {
            return Err(invalid("busy: a roll-off search is running on this device"));
        }
        if self.measure_tune_state().running {
            return Err(invalid("busy: a tune's run is going on this device"));
        }
        if self.measure.live_on() {
            return Err(invalid("busy: the live view is running on this device"));
        }
        if self.measure_state().active {
            return Err(invalid(
                "busy: a measurement is already running on this device",
            ));
        }
        let caller_floor = if setup.floor_hz == 0.0 {
            PROBE_FLOOR_MIN_HZ
        } else if setup.floor_hz.is_finite()
            && (PROBE_FLOOR_MIN_HZ..=PROBE_FLOOR_MAX_HZ).contains(&setup.floor_hz)
        {
            setup.floor_hz
        } else {
            return Err(invalid(format!(
                "a floor of {} Hz ({PROBE_FLOOR_MIN_HZ} to {PROBE_FLOOR_MAX_HZ} Hz; it may only \
                 raise the 1 kHz floor)",
                setup.floor_hz
            )));
        };
        let layout = self.speaker_layout()?;
        if layout
            .as_ref()
            .is_some_and(|l| l.state() == LayoutState::Confirmed)
        {
            return Err(invalid(format!(
                "{KEY_CONFIRMED}: the speaker layout is confirmed: the user corrects it in \
                 Settings › Speakers"
            )));
        }
        let (n_out, enabled, muted, profile) = {
            let mut inner = self.lock();
            inner.conn()?;
            let s = &inner.state;
            let n = s.num_output_channels;
            let enabled = s.output_enabled.clone();
            let muted = s.output_muted.clone();
            (n, enabled, muted, crate::limits::profile(&mut inner))
        };
        let mut outs = if setup.outputs.is_empty() {
            (0..n_out).collect::<Vec<u8>>()
        } else {
            setup.outputs.clone()
        };
        outs.sort_unstable();
        outs.dedup();
        if let Some(bad) = outs.iter().find(|o| **o >= n_out) {
            return Err(invalid(format!(
                "the DSPi has no output {}",
                u16::from(*bad).saturating_add(1)
            )));
        }
        let microphone = if setup.mic_document.trim().is_empty() {
            String::new()
        } else {
            dspi_measure::doc::MicDoc::from_text(&setup.mic_document)
                .map_err(|e| invalid(format!("the microphone document: {e}")))?
                .mic
                .name
        };
        let mut detection = Detection {
            at_ms: crate::events::now_ms(),
            by: crate::actor::actor_label(actor).unwrap_or_default(),
            placement: setup.placement,
            floor_hz: caller_floor,
            microphone,
            outputs: Vec::new(),
            stopped: false,
            summary: String::new(),
            device_outputs: n_out,
        };
        let mut queue = Vec::new();
        for o in &outs {
            let at = usize::from(*o);
            let on = enabled.get(at).copied().unwrap_or(false)
                && !muted.get(at).copied().unwrap_or(false);
            let profile_floor = profile
                .as_ref()
                .and_then(|p| p.output(*o))
                .and_then(|l| l.highpass_min_hz)
                .filter(|f| f.is_finite())
                .unwrap_or(0.0);
            let floor = caller_floor
                .max(profile_floor)
                .clamp(PROBE_FLOOR_MIN_HZ, PROBE_FLOOR_MAX_HZ);
            if on {
                queue.push((*o, floor));
                detection.outputs.push(DetectedOutput {
                    output: *o,
                    class: DetectClass::Unknown,
                    confidence: DetectConfidence::Low,
                    role: Some(MeasureRole::High),
                    floor_hz: floor,
                    note: Some(KEY_UNKNOWN.to_owned()),
                    ..DetectedOutput::default()
                });
            } else {
                detection.outputs.push(DetectedOutput {
                    output: *o,
                    class: DetectClass::Off,
                    floor_hz: floor,
                    note: Some(KEY_OFF.to_owned()),
                    ..DetectedOutput::default()
                });
            }
        }
        // -- RGFIX -- A detection of some outputs keeps what the one before
        // found of the others (their roles, the `nothing` records).
        if let Some(prev) = layout
            .as_ref()
            .and_then(|l| l.detected.as_ref())
            .filter(|p| p.device_outputs == n_out)
        {
            detection.outputs.extend(
                prev.outputs
                    .iter()
                    .filter(|r| !outs.contains(&r.output))
                    .cloned(),
            );
            detection.outputs.sort_by_key(|r| r.output);
        }
        *d = Detect {
            setup: Some(setup.clone()),
            stage: DetectStage::Ramp,
            running: false,
            queue,
            at: 0,
            work: None,
            detection,
            issue: None,
            actor,
            headphones: setup.placement == MeasurePlacement::Headphones,
            level_now: 0.0,
            segment_hz: 0.0,
            base: layout,
        };
        if d.queue.is_empty() {
            self.detect_finish(&mut d);
            return Ok(self.detect_publish(&d));
        }
        let first = d.queue.first().copied();
        d.work = first.map(|(o, f)| Work::new(o, f));
        if let Err(e) = self.detect_session(&mut d) {
            *d = Detect::default();
            self.detect_publish(&d);
            return Err(e);
        }
        // From the first sound on, the layout says what is known: every
        // output still to come is unknown, a tweeter until its turn.
        self.detect_write_layout(&d);
        Ok(self.detect_publish(&d))
    }

    /// Drive the detection: its session (`measure_tick`), then the verdict
    /// and the next session, output or the end. Call it about every 50 ms
    /// off the UI thread while `running`, instead of `measure_tick`. Never
    /// fails.
    pub fn detect_tick(&self) -> DetectState {
        let mut d = self.measure.detect_lock();
        if !d.running {
            return self.detect_publish(&d);
        }
        let st = self.measure_tick();
        match st.phase {
            MeasurePhase::Done => {
                let text = self.measure_result().ok();
                let _ = self.measure_clear();
                d.running = false;
                self.detect_judge(&mut d, text.as_deref(), None, st.run_level_db);
            }
            MeasurePhase::Failed => {
                let _ = self.measure_clear();
                d.running = false;
                self.detect_judge(&mut d, None, st.issue.clone(), st.run_level_db);
            }
            MeasurePhase::Aborted | MeasurePhase::Idle => {
                let _ = self.measure_clear();
                d.running = false;
                self.detect_stopped(&mut d, st.issue.clone());
            }
            _ => {}
        }
        self.detect_publish(&d)
    }

    /// The detection's state, without I/O (safe from the measurement
    /// listener).
    pub fn detect_state(&self) -> DetectState {
        self.measure
            .detect_view
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// The user's Stop: the session's generator stopped at once
    /// (`SIGGEN_CTL_STOP_NOW`), the output being probed left unknown (a
    /// tweeter), the rows already written kept. Idempotent; never fails.
    pub fn detect_stop(&self) -> DetectState {
        let mut d = self.measure.detect_lock();
        if d.running
            || matches!(
                d.stage,
                DetectStage::Ramp | DetectStage::Shape | DetectStage::Descent
            )
        {
            crate::actor::user::did(self, "measure_stop");
            if d.running {
                self.measure_stop_session();
                let _ = self.measure_clear();
                d.running = false;
            }
            self.detect_stopped(
                &mut d,
                Some(MeasureIssue::new(
                    "measure.stopped",
                    None,
                    "the speaker detection was stopped",
                )),
            );
        }
        self.detect_publish(&d)
    }

    /// Forget a finished detection's state (the layout keeps what it found).
    /// Refused while one runs.
    ///
    /// Errors: [`DspiError::InvalidArgument`] while a detection runs.
    pub fn detect_clear(&self) -> Result<()> {
        let mut d = self.measure.detect_lock();
        if d.running {
            return Err(invalid("a speaker detection is running: stop it first"));
        }
        *d = Detect::default();
        self.detect_publish(&d);
        Ok(())
    }

    /// Whether AI agents may detect the speakers under the "never ask"
    /// setting (Settings › AI Agents › Agents may detect speakers; off by
    /// default). The app sets it from its settings at start; the agents'
    /// policy reads it. Never fails.
    pub fn detect_agents_allowed(&self) -> bool {
        self.measure.detect_agents.load(Ordering::SeqCst)
    }

    /// Set [`DspiCore::detect_agents_allowed`] (the user's switch only).
    ///
    /// Errors: [`DspiError::ConfirmationRequired`] `user_only` inside
    /// another actor's scope.
    pub fn detect_set_agents_allowed(&self, allowed: bool) -> Result<()> {
        crate::actor::user_only(self, "The speaker detection's setting")?;
        self.measure.detect_agents.store(allowed, Ordering::SeqCst);
        Ok(())
    }
}

impl DspiCore {
    fn detect_publish(&self, d: &Detect) -> DetectState {
        let session = if d.running {
            self.measure_state()
        } else {
            MeasureState::default()
        };
        let st = d.state(session);
        *self
            .measure
            .detect_view
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = st.clone();
        st
    }

    /// The plan of the work's next session.
    fn detect_plan(&self, d: &Detect, w: &Work) -> Result<Plan> {
        let setup = d.setup.as_ref().ok_or_else(|| invalid("no detection"))?;
        let (start, ceiling, below) = d.levels();
        let (f1, f2, ms) = match w.step {
            Step::Top | Step::Band => (w.floor, 20_000.0, PROBE_SWEEP_MS),
            Step::Descent(k) => {
                let f = w
                    .steps
                    .get(k)
                    .copied()
                    .ok_or_else(|| invalid("no segment left"))?;
                let (f1, f2) = segment_band(f);
                (f1, f2, PROBE_SEG_MS)
            }
        };
        let base = MeasureSetup {
            kind: MeasureRunKind::LiveSweep,
            outputs: vec![MeasureOutput {
                output: w.output,
                role: MeasureRole::Full,
                speaker: String::new(),
                group: 0,
                inverted: false,
            }],
            placement: match setup.placement {
                MeasurePlacement::NearField => MeasurePlacement::NearField,
                MeasurePlacement::Headphones => MeasurePlacement::Headphones,
                _ => MeasurePlacement::ListeningPosition,
            },
            coupler: setup.coupler,
            mic_document: setup.mic_document.clone(),
            sweep_ms: ms,
            ceiling_db: Some(ceiling),
            spl_cap_db: setup.spl_cap_db,
            capture_rate_hz: setup.capture_rate_hz,
            name: "Speaker detection".into(),
            ..MeasureSetup::default()
        };
        let mut plan = Plan::new(&base)?;
        // The free descent is for an output the shaped one showed is no
        // tweeter: its floor is the plan's 20 Hz, the ceiling still holds.
        let free = matches!(w.step, Step::Descent(k) if k >= w.shaped);
        plan.probe = Some(Probe {
            floor_hz: if free { 20.0 } else { w.floor },
            below_dbfs: below,
            ceiling_dbfs: ceiling,
        });
        match w.step {
            Step::Top | Step::Band => {
                plan.start_db = start;
                plan.ceiling_db = ceiling;
                plan.ramp_snr_db = Some(PROBE_SNR_DB);
                plan.ramp_band = Some(if w.step == Step::Top {
                    (PROBE_TOP_BAND.0.max(w.floor), PROBE_TOP_BAND.1)
                } else {
                    (w.floor, A2_TOP_HZ)
                });
            }
            Step::Descent(_) => {
                let level = if free {
                    w.level.unwrap_or(ceiling).min(ceiling)
                } else {
                    w.level.unwrap_or(below).min(below)
                };
                plan.start_db = level;
                plan.ceiling_db = level;
                plan.fixed_level = true;
            }
        }
        plan.set_sweep_band(f1, f2)?;
        Ok(plan)
    }

    /// Start the work's next session (refused like any start).
    fn detect_session(&self, d: &mut Detect) -> Result<()> {
        let w = d
            .work
            .clone()
            .ok_or_else(|| invalid("no output to probe"))?;
        let plan = self.detect_plan(d, &w)?;
        if self.measure.live_on() {
            return Err(invalid("busy: the live view is running on this device"));
        }
        let level = plan.start_db;
        let ceiling = plan.ceiling_db;
        let f1 = match w.step {
            Step::Top => plan.ramp_band.map_or(w.floor, |b| b.0),
            _ => plan.sweep.f1_hz,
        };
        // -- WP ADETS -- the function test's oracle: the segment and its floor
        #[cfg(feature = "sim-scenario")]
        if let Some(serial) = self.lock().conn.as_ref().map(|c| c.serial.clone()) {
            let free = matches!(w.step, Step::Descent(k) if k >= w.shaped);
            let stage = match w.step {
                Step::Top => "top",
                Step::Band => "band",
                Step::Descent(_) if free => "free",
                Step::Descent(_) => "shaped",
            };
            crate::measure_sim::scenario::probe_segment(
                &serial,
                w.output,
                stage,
                plan.probe.as_ref().map_or(w.floor, |p| p.floor_hz),
                f1,
                plan.sweep.f2_hz,
                ceiling,
            );
        }
        self.measure_start_plan(plan, &PreflightOptions::default())?;
        self.measure_set_actor(d.actor);
        d.running = true;
        d.level_now = if w.step == Step::Top || w.step == Step::Band {
            ceiling
        } else {
            level
        };
        d.segment_hz = f1;
        let low = match w.step {
            Step::Descent(_) => f1,
            _ => w.floor,
        };
        if let Some(work) = d.work.as_mut() {
            work.played(low);
        }
        d.stage = match w.step {
            Step::Descent(_) => DetectStage::Descent,
            _ => DetectStage::Ramp,
        };
        Ok(())
    }

    /// A session ended: `text` its document, or `issue` why it failed;
    /// `level` the level it played at.
    fn detect_judge(
        &self,
        d: &mut Detect,
        text: Option<&str>,
        issue: Option<MeasureIssue>,
        level: Option<f32>,
    ) {
        let Some(mut w) = d.work.clone() else {
            self.detect_stopped(d, issue);
            return;
        };
        let key = issue.as_ref().map(|i| i.key.as_str());
        let response = text.and_then(|t| {
            let doc = MeasurementDoc::from_text(t).ok()?;
            let o = doc.measurement.outputs.first()?.clone();
            let freqs = o.response.grid.freqs().ok()?;
            let played = doc.measurement.stimulus.level_db as f32;
            Some((freqs, o.response.mag_db, o.response.valid, played))
        });
        let next = match w.step {
            Step::Top | Step::Band => {
                if matches!(
                    key,
                    Some("measure.mic_not_hearing" | "measure.start_level_quiet")
                ) {
                    Some(w.not_heard_above())
                } else {
                    response.map(|(freqs, mag, valid, played)| {
                        w.level = level.or(played.is_finite().then_some(played));
                        let raw =
                            trusted(&freqs, &mag, valid.as_deref(), f64::from(w.floor), 20_000.0);
                        w.after_top(raw) // -- WP ADETS --
                    })
                }
            }
            Step::Descent(k) => {
                if response.is_none() && !matches!(key, Some("measure.no_signal") | None) {
                    // The microphone asked something: unknown.
                    None
                } else {
                    let (f1, f2) = segment_band(w.steps.get(k).copied().unwrap_or(w.floor));
                    let seg = response
                        .map(|(freqs, mag, valid, _)| {
                            smoothed(&freqs, &mag, valid.as_deref(), f64::from(f1), f64::from(f2))
                        })
                        .unwrap_or_default();
                    Some(w.after_segment(seg))
                }
            }
        };
        match next {
            Some(Next::Session) => self.detect_next(d, w),
            Some(Next::Done(class, confidence)) => {
                self.detect_output_done(d, &w, class, confidence);
            }
            None => self.detect_output_unknown(d, w, issue),
        }
    }

    /// Go on with `w`'s next session, or end the output unknown when it
    /// cannot start.
    fn detect_next(&self, d: &mut Detect, w: Work) {
        d.work = Some(w.clone());
        if let Err(e) = self.detect_session(d) {
            let issue = MeasureIssue::new(
                "measure.stopped",
                Some(w.output),
                format!("the next probe could not start: {e}"),
            );
            self.detect_stopped(d, Some(issue));
        }
    }

    fn detect_output_unknown(&self, d: &mut Detect, w: Work, issue: Option<MeasureIssue>) {
        d.issue = issue;
        self.detect_output_done(d, &w, DetectClass::Unknown, DetectConfidence::Low);
    }

    /// The output is classified: its row written, the next output started.
    fn detect_output_done(
        &self,
        d: &mut Detect,
        w: &Work,
        class: DetectClass,
        confidence: DetectConfidence,
    ) {
        let rec = w.record(class, confidence);
        if let Some(slot) = d
            .detection
            .outputs
            .iter_mut()
            .find(|o| o.output == w.output)
        {
            *slot = rec;
        }
        d.at = d.at.saturating_add(1);
        self.detect_write_layout(d);
        match d.queue.get(d.at).copied() {
            Some((o, f)) => self.detect_next(d, Work::new(o, f)),
            None => self.detect_finish(d),
        }
    }

    fn detect_finish(&self, d: &mut Detect) {
        d.work = None;
        d.stage = DetectStage::Done;
        d.running = false;
        d.detection.summary = summary_of(&d.this_run(), d.at);
        self.detect_write_layout(d);
        crate::actor::user::did_as(self, d.actor, "speakers_detected", &d.detection.summary);
    }

    fn detect_stopped(&self, d: &mut Detect, issue: Option<MeasureIssue>) {
        d.stage = DetectStage::Stopped;
        d.running = false;
        d.detection.stopped = true;
        d.issue = issue.or_else(|| {
            Some(MeasureIssue::new(
                KEY_STOPPED,
                None,
                "the speaker detection ended before every output was probed",
            ))
        });
        // The output being probed stays unknown (a tweeter), as do those
        // still to come.
        if let Some(w) = d.work.take()
            && let Some(slot) = d
                .detection
                .outputs
                .iter_mut()
                .find(|o| o.output == w.output)
        {
            *slot = w.record(DetectClass::Unknown, DetectConfidence::Low);
        }
        d.detection.summary = summary_of(&d.this_run(), d.at);
        if d.setup.is_some() {
            self.detect_write_layout(d);
            crate::actor::user::did_as(self, d.actor, "speakers_detected", &d.detection.summary);
        }
    }

    /// The layout as the detection knows it now: every probed output's row
    /// from its class (an output detected as nothing left out), the user's
    /// own rows kept as they are, suggestions dropped.
    fn detect_write_layout(&self, d: &Detect) {
        let before = d.base.clone().unwrap_or_default();
        // The user may have corrected a row meanwhile: their rows stand.
        let now = self.speaker_layout().ok().flatten().unwrap_or_default();
        if before.state() == LayoutState::Confirmed || now.state() == LayoutState::Confirmed {
            return;
        }
        let mut rows: Vec<SpeakerLayoutOutput> = now
            .outputs
            .iter()
            .filter(|o| o.source == RoleSource::User)
            .cloned()
            .collect();
        for o in before
            .outputs
            .iter()
            .filter(|o| o.source == RoleSource::User)
        {
            if !rows.iter().any(|r| r.output == o.output) {
                rows.push(o.clone());
            }
        }
        for rec in &d.detection.outputs {
            if rows.iter().any(|r| r.output == rec.output) {
                continue;
            }
            let Some(role) = rec.role else {
                continue;
            };
            let old = before.output(rec.output);
            rows.push(SpeakerLayoutOutput {
                output: rec.output,
                role,
                speaker: old.map(|o| o.speaker.clone()).unwrap_or_default(),
                group: old.map_or(0, |o| o.group),
                source: RoleSource::Detected,
            });
        }
        rows.sort_by_key(|r| r.output);
        let layout = SpeakerLayout {
            serial: before.serial.clone(),
            outputs: rows,
            confirmed_by_user: false,
            updated_ms: 0,
            notes: before.notes.clone(),
            detected: Some(d.detection.clone()),
        };
        if let Err(e) = self.speaker_layout_keep_detected(&layout) {
            log::warn!("the speaker detection's layout could not be kept: {e}");
        }
    }
}

/// `seg` shifted onto `pts` by their mean difference over the band both
/// cover (at most [`ALIGN_MAX_DB`]): the neighbouring segments' levels
/// agree before they are judged (magnitude only). Unchanged when they share
/// too little.
pub(crate) fn align(pts: &[(f64, f64)], seg: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    let (Some(lo), Some(hi)) = (seg.first().map(|p| p.0), seg.last().map(|p| p.0)) else {
        return seg;
    };
    let (Some(p_lo), Some(p_hi)) = (pts.first().map(|p| p.0), pts.last().map(|p| p.0)) else {
        return seg;
    };
    let (from, to) = (lo.max(p_lo), hi.min(p_hi));
    let (Some(old), Some(new)) = (band_mean(pts, from, to), band_mean(&seg, from, to)) else {
        return seg;
    };
    let shift = (old - new).clamp(-ALIGN_MAX_DB, ALIGN_MAX_DB);
    seg.into_iter().map(|(f, m)| (f, m + shift)).collect()
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_adet0;
