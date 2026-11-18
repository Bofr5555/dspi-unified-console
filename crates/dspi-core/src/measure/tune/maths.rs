//! The pure part of the auto-tune (WP P5TUNE, PLAN-P5 section 1.4 steps 3 to
//! 8): from a `measurement` document, the confirmed layout and the device's
//! chains now, the proposal's chains, its writes in words and the curves.
//! No I/O, no clock, no hash-map order: identical input gives identical
//! quantised writes (RC10).
//!
//! Per speaker (the ways one layout name groups, ordered sub, low, mid,
//! high):
//! 1. **De-embedding** (step 3): each way's measurement divided by the chain
//!    it was measured with ([`de_embed`], its own validity mask from the
//!    document's per-point SNR, RC12); the driver is then put behind the
//!    device's chain **now** (OD-P5o: an EQ changed since is re-predicted, not
//!    refused).
//! 2. **Delay, polarity, corner and levels** (steps 4 and 5), only when the
//!    run passed the timing quality gate (RC5d) and the summed check has not
//!    failed for the speaker (RC5f): [`crossover_corner`] with the window, the
//!    protective high-passes of the role or the limits profile (S6), the delay
//!    range of the platform and the profile, cut-only levels (S9, S-f). Its
//!    corners, delays, gains and net polarities become the writes.
//! 3. **The target fit** (step 7, review P5-A): with timing, on the predicted
//!    programme sum of the speaker with the new crossover, its bands written
//!    to **every way that plays in the fit's range** (so the prediction is the
//!    sum times the bands, exactly); without timing, on the low way's own
//!    response below half its corner, its bands on the low way only. The
//!    range starts a third of an octave above the lowest trusted point
//!    (review P5-A N11), free bands only (OD-P5l), every band of
//!    `FitResult.bands` written. The fits' headroom is one common output-gain
//!    cut on every output of the layout, so no balance moves.

use std::collections::BTreeMap;

use dspi_measure::align::{AlignOptions, Transition, Way};
use dspi_measure::doc::{Curve, MeasurementDoc};
use dspi_measure::fit::{FitOptions, FitResult, fit};
use dspi_measure::types::{BandSpec, ChannelChain, LogGrid, ProgrammeFeed, Response, SectionModel};
use dspi_measure::xover::{
    DEEMBED_MIN_SNR_DB, Driver, HighpassOverride, SumMode, XoConstraints, crossover_corner,
    de_embed, predict_sum,
};

use super::{MeasureTuneKind, MeasureTuneOptions, MeasureTuneParam, MeasureTuneWrite};
use crate::measure::live::{band_words, chain_db, fit_span, levelled_target, repredict};
use crate::measure::runs::SNR_KEY;
use crate::measure::{MeasureIssue, MeasureRole};
use crate::records::FilterParams;
use crate::write_param::ParamValue;

/// The first crossover band on the wire.
pub(crate) const XOVER_BASE: u8 = {
    assert!(dspi_proto::generated::filter::XOVER_BAND_BASE <= u8::MAX as u16);
    dspi_proto::generated::filter::XOVER_BAND_BASE as u8
};

/// The speakers' fit range when the options name none, Hz (OD-P5l).
pub(crate) const FIT_SPEAKERS_HZ: (f64, f64) = (20.0, 300.0);
/// The headphones' fit range, Hz.
pub(crate) const FIT_HEADPHONES_HZ: (f64, f64) = (20.0, 10_000.0);
/// A way "plays" in the fit's range when its crossover passes this much of
/// its level somewhere in it, dB: its bands are written there too.
const PLAYS_DB: f64 = -30.0;
/// The lowest Q of a speaker's fitted band (see the fit's note in
/// `compute`).
pub(crate) const SPEAKERS_Q_MIN: f64 = 2.0;
/// A dip this far below the boost cap is not the fit's (see
/// [`mask_dips`]), dB.
const DIP_MARGIN_DB: f64 = 1.0;
/// A level a write must change by to be written, dB, ms, Hz.
const GAIN_EPS: f32 = 0.05;
const DELAY_EPS: f32 = 0.000_05;
const FREQ_EPS: f32 = 0.05;

/// Notice keys of what is withdrawn.
pub(crate) const KEY_UNRELIABLE: &str = "measure.tune.timing_unreliable";
pub(crate) const KEY_SUM_FAILED: &str = "measure.tune.sum_check_failed";
pub(crate) const KEY_NO_TIMING: &str = "measure.tune.no_timing";
pub(crate) const KEY_LIMIT: &str = "measure.tune.limit";
pub(crate) const KEY_NO_FIT: &str = "measure.tune.no_fit";

/// The role's place in a speaker, lowest first.
fn rank(r: MeasureRole) -> u8 {
    match r {
        MeasureRole::Sub => 0,
        MeasureRole::Low => 1,
        MeasureRole::Mid => 2,
        MeasureRole::High => 3,
        MeasureRole::Full => 4,
    }
}

/// One way as the tune takes it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WayIn {
    pub(crate) output: u8,
    pub(crate) role: MeasureRole,
    pub(crate) speaker: String,
    /// The chain it was measured through (the document's).
    pub(crate) measured_chain: ChannelChain,
    /// The device's chain now.
    pub(crate) chain_now: ChannelChain,
    /// The measured response (with phase where measured).
    pub(crate) response: Response,
    /// The SNR per point, dB, when the document holds it.
    pub(crate) snr_db: Option<Vec<f64>>,
    /// The onset relative to its own cycle (timing-reliable runs).
    pub(crate) arrival_ms: Option<f64>,
    /// The spread across positions or reseats, dB.
    pub(crate) spread_db: Option<Vec<f64>>,
    /// The routed crosspoints feeding it now: input, gain, inverted.
    pub(crate) feeds: Vec<(u8, f32, bool)>,
    /// The wire channel of its PEQ and crossover banks.
    pub(crate) channel: u8,
}

/// The bounds a proposal is held to (S6, S9).
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Bounds {
    pub(crate) min_delay_ms: f64,
    pub(crate) max_delay_ms: f64,
    /// The limits profile's protective high-passes, per output.
    pub(crate) highpass: Vec<HighpassOverride>,
    /// The fit's largest boost: the placement's, lowered by the profile's.
    pub(crate) max_boost_db: f64,
    /// The boost cap came from the limits profile.
    pub(crate) boost_from_profile: bool,
    pub(crate) headphones: bool,
}

/// What a speaker may get.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Allow {
    /// Speakers whose timing writes are withdrawn, with the notice key and
    /// the reason.
    pub(crate) no_timing: BTreeMap<String, (String, String)>,
    /// Speakers whose fit is withdrawn, with the reason.
    pub(crate) no_fit: BTreeMap<String, String>,
}

/// One speaker's plan and result.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SpeakerOut {
    pub(crate) name: String,
    /// Indices into the ways, lowest way first.
    pub(crate) ways: Vec<usize>,
    /// Transitions, indices into `ways`' order (`low`, `high` of
    /// `drivers`/`ways` positions in this speaker).
    pub(crate) transitions: Vec<Transition>,
    /// De-embedded drivers (behind the chains now) when the timing path ran
    /// or could run: the summed check and the verify predict from them.
    pub(crate) drivers: Option<Vec<Driver>>,
    /// Timing writes were proposed for it.
    pub(crate) timing: bool,
    /// Per transition: corner before and after, relative delay, flip.
    pub(crate) xo: Vec<(f64, f64, f64, bool)>,
    /// The predicted sum (or the single way) with the chains now, and with
    /// the proposal.
    pub(crate) before: Option<Response>,
    pub(crate) predicted: Option<Response>,
    /// The curves are a coherent sum (with timing), else a power sum.
    pub(crate) coherent: bool,
    /// The fit's result, if one ran.
    pub(crate) fit: Option<FitResult>,
}

/// Everything a proposal is made of.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Computed {
    /// Every way's chain after the proposal, in the ways' order.
    pub(crate) chains: Vec<ChannelChain>,
    pub(crate) writes: Vec<MeasureTuneWrite>,
    pub(crate) withdrawn: Vec<MeasureIssue>,
    pub(crate) held: Vec<String>,
    pub(crate) notes: Vec<String>,
    pub(crate) speakers: Vec<SpeakerOut>,
}

/// The document's SNR curve of output measurement `o`, on its grid.
pub(crate) fn stored_snr(
    o: &dspi_measure::doc::OutputMeasurement,
    grid: &LogGrid,
    n: usize,
) -> Option<Vec<f64>> {
    let v = o.extra.get(SNR_KEY)?;
    let c: Curve = serde_json::from_value(v.clone()).ok()?;
    (c.grid == *grid && c.mag_db.len() == n).then_some(c.mag_db)
}

/// The output's name in words ("Output 3 (L high)").
pub(crate) fn who(w: &WayIn) -> String {
    let role = match w.role {
        MeasureRole::Full => "full range",
        MeasureRole::Sub => "sub",
        MeasureRole::Low => "low",
        MeasureRole::Mid => "mid",
        MeasureRole::High => "high",
    };
    let n = u16::from(w.output).saturating_add(1);
    if w.speaker.is_empty() {
        format!("Output {n} ({role})")
    } else {
        format!("Output {n} ({} {role})", w.speaker)
    }
}

fn pass_of(b: &BandSpec) -> Option<bool> {
    if b.bypass || b.filter_type == 0 {
        return None;
    }
    dspi_proto::xover::meta(b.filter_type).map(|m| m.high_pass)
}

/// The active crossover low-pass's corner of `c`, if any.
pub(crate) fn lowpass_corner(c: &ChannelChain) -> Option<f32> {
    c.crossover
        .iter()
        .find(|b| pass_of(b) == Some(false))
        .map(|b| b.freq)
        .or_else(|| {
            c.peq
                .iter()
                .find(|b| !b.bypass && matches!(b.filter_type, 4 | 12))
                .map(|b| b.freq)
        })
}

fn has_xover(c: &ChannelChain, high_pass: bool) -> bool {
    c.crossover.iter().any(|b| pass_of(b) == Some(high_pass))
}

/// The speakers of `ways` in order of first appearance, each with its ways
/// lowest first and the transitions between consecutive crossed ways.
pub(crate) fn speakers_of(ways: &[WayIn], notes: &mut Vec<String>) -> Vec<SpeakerOut> {
    let mut names: Vec<String> = Vec::new();
    for w in ways {
        if !names.contains(&w.speaker) {
            names.push(w.speaker.clone());
        }
    }
    names
        .into_iter()
        .map(|name| {
            let mut idx: Vec<usize> = ways
                .iter()
                .enumerate()
                .filter(|(_, w)| w.speaker == name)
                .map(|(i, _)| i)
                .collect();
            idx.sort_by_key(|i| ways.get(*i).map(|w| (rank(w.role), w.output)));
            let mut transitions = Vec::new();
            let crossed: Vec<usize> = (0..idx.len())
                .filter(|k| {
                    idx.get(*k)
                        .and_then(|i| ways.get(*i))
                        .is_some_and(|w| w.role != MeasureRole::Full)
                })
                .collect();
            for pair in crossed.windows(2) {
                let (Some(&a), Some(&b)) = (pair.first(), pair.get(1)) else {
                    continue;
                };
                let (Some(wa), Some(wb)) = (
                    idx.get(a).and_then(|i| ways.get(*i)),
                    idx.get(b).and_then(|i| ways.get(*i)),
                ) else {
                    continue;
                };
                if has_xover(&wa.chain_now, false) && has_xover(&wb.chain_now, true) {
                    transitions.push(Transition {
                        low: a,
                        high: b,
                        corner_hz: None,
                    });
                } else {
                    notes.push(format!(
                        "{} and {} have no crossover pair: the transition between them is not tuned",
                        who(wa),
                        who(wb)
                    ));
                }
            }
            SpeakerOut {
                name,
                ways: idx,
                transitions,
                drivers: None,
                timing: false,
                xo: Vec::new(),
                before: None,
                predicted: None,
                coherent: false,
                fit: None,
            }
        })
        .collect()
}

/// The way `w` as the alignment takes it (its measured chain).
fn way_of(w: &WayIn) -> Option<Way> {
    let n = w.response.mag_db.len();
    w.response.phase_deg.as_ref()?;
    let snr = match &w.snr_db {
        Some(s) if s.len() == n => s.clone(),
        // Filed without its SNR: only the trusted points count, and as
        // barely trusted (the de-embedding then trusts only the passband).
        _ => w.response.valid.as_ref().map_or_else(
            || vec![DEEMBED_MIN_SNR_DB; n],
            |v| {
                v.iter()
                    .map(|ok| if *ok { DEEMBED_MIN_SNR_DB } else { 0.0 })
                    .collect()
            },
        ),
    };
    Some(Way {
        output: w.output,
        role: w.role.way(),
        chain: w.measured_chain.clone(),
        response: w.response.clone(),
        snr_db: snr,
        arrival_ms: w.arrival_ms?,
        timing_reliable: true,
    })
}

/// The power sum of magnitudes (no phase: an incoherent estimate), on the
/// first response's grid; trusted where every part is.
fn power_sum(parts: &[Response]) -> Option<Response> {
    let first = parts.first()?;
    let n = first.mag_db.len();
    let mut mag = vec![0.0f64; n];
    let mut valid = vec![true; n];
    for p in parts {
        if p.grid != first.grid || p.mag_db.len() != n {
            return None;
        }
        for (i, m) in p.mag_db.iter().enumerate() {
            if let Some(x) = mag.get_mut(i) {
                *x += 10f64.powf(m / 10.0);
            }
            let ok = p
                .valid
                .as_ref()
                .is_none_or(|v| v.get(i).copied().unwrap_or(false));
            if let Some(x) = valid.get_mut(i) {
                *x = *x && ok;
            }
        }
    }
    Some(Response {
        grid: first.grid,
        mag_db: mag
            .iter()
            .map(|p| {
                let d = 10.0 * p.max(1e-30).log10();
                if d.is_finite() { d } else { -300.0 }
            })
            .collect(),
        phase_deg: None,
        valid: Some(valid),
        spl: first.spl,
    })
}

/// The speaker's response through `chains` (one per way of `sp`): the
/// coherent programme sum from the drivers, else the power sum of the ways'
/// re-predicted magnitudes.
pub(crate) fn speaker_response(
    sp: &SpeakerOut,
    ways: &[WayIn],
    chains: &[ChannelChain],
    model: &dyn SectionModel,
) -> Option<Response> {
    let mine: Vec<ChannelChain> = sp
        .ways
        .iter()
        .filter_map(|i| chains.get(*i).cloned())
        .collect();
    if let Some(d) = &sp.drivers {
        return predict_sum(d, &mine, SumMode::Programme, model).ok();
    }
    let parts: Vec<Response> = sp
        .ways
        .iter()
        .zip(&mine)
        .filter_map(|(i, c)| {
            let w = ways.get(*i)?;
            repredict(&w.response, &w.measured_chain, c, model).ok()
        })
        .collect();
    if parts.len() != sp.ways.len() {
        return None;
    }
    power_sum(&parts)
}

/// The crossover bank of `c` alone, at 0 dB.
fn xover_only(c: &ChannelChain) -> ChannelChain {
    ChannelChain {
        peq: Vec::new(),
        crossover: c.crossover.clone(),
        gain_db: 0.0,
        delay_ms: 0.0,
        feed: ProgrammeFeed {
            gain_db: 0.0,
            inverted: false,
        },
    }
}

/// Whether chain `c`'s crossover passes at least [`PLAYS_DB`] somewhere in
/// `lo..=hi`.
fn plays_in(c: &ChannelChain, lo: f64, hi: f64, model: &dyn SectionModel) -> bool {
    let Ok(freqs) = LogGrid::STANDARD.freqs() else {
        return false;
    };
    let f: Vec<f64> = freqs.into_iter().filter(|f| *f >= lo && *f <= hi).collect();
    chain_db(&xover_only(c), model, &f)
        .iter()
        .any(|d| *d >= PLAYS_DB)
}

fn free(b: &BandSpec) -> bool {
    b.filter_type == 0 || b.bypass
}

pub(crate) fn params_of(b: &BandSpec) -> FilterParams {
    FilterParams {
        filter_type: b.filter_type,
        bypass: b.bypass,
        freq: b.freq,
        q: b.q,
        gain: b.gain,
        qp: b.qp,
    }
}

fn param(path: &str, indices: Vec<u8>, value: ParamValue) -> MeasureTuneParam {
    MeasureTuneParam {
        path: path.to_owned(),
        indices,
        value,
    }
}

fn hz(f: f64) -> String {
    if f >= 1_000.0 {
        format!("{:.2} kHz", f / 1_000.0)
    } else {
        format!("{f:.0} Hz")
    }
}

fn xover_words(b: &BandSpec) -> String {
    dspi_proto::xover::meta(b.filter_type).map_or_else(
        || format!("type {}", b.filter_type),
        |m| {
            format!(
                "{}{} {}",
                match m.family {
                    dspi_proto::xover::Family::LinkwitzRiley => "LR",
                    dspi_proto::xover::Family::Butterworth => "BW",
                    dspi_proto::xover::Family::Bessel => "Bessel ",
                },
                m.order,
                if m.high_pass { "high-pass" } else { "low-pass" }
            )
        },
    )
}

/// The writes that take way `w` from `before` to `after`, in the order they
/// are applied: a gain cut first, then the crossover, the delay, the
/// polarity, the PEQ bands.
fn writes_of(
    w: &WayIn,
    before: &ChannelChain,
    after: &ChannelChain,
    level_db: f32,
    headroom_db: f32,
) -> Vec<MeasureTuneWrite> {
    let name = who(w);
    let ch = w.channel;
    let o = w.output;
    let mut out = Vec::new();
    if (after.gain_db - before.gain_db).abs() >= GAIN_EPS {
        let mut why = Vec::new();
        if level_db.abs() >= GAIN_EPS {
            why.push(format!("level {level_db:+.1} dB"));
        }
        if headroom_db.abs() >= GAIN_EPS {
            why.push(format!("headroom for the EQ {headroom_db:+.1} dB"));
        }
        out.push(MeasureTuneWrite {
            output: o,
            kind: if level_db.abs() >= GAIN_EPS {
                MeasureTuneKind::Level
            } else {
                MeasureTuneKind::Headroom
            },
            timing: level_db.abs() >= GAIN_EPS,
            words: format!(
                "{name}: output gain {:.1} → {:.1} dB ({})",
                before.gain_db,
                after.gain_db,
                why.join(", ")
            ),
            params: vec![param("out.gain", vec![o], ParamValue::Float(after.gain_db))],
        });
    }
    for (slot, (b, a)) in (0u8..).zip(before.crossover.iter().zip(&after.crossover)) {
        if (a.freq - b.freq).abs() >= FREQ_EPS {
            out.push(MeasureTuneWrite {
                output: o,
                kind: MeasureTuneKind::Crossover,
                timing: true,
                words: format!(
                    "{name}: crossover {} {} → {}",
                    xover_words(b),
                    hz(f64::from(b.freq)),
                    hz(f64::from(a.freq))
                ),
                params: vec![param(
                    "eq.freq",
                    vec![ch, XOVER_BASE.saturating_add(slot)],
                    ParamValue::Float(a.freq),
                )],
            });
        }
    }
    if (after.delay_ms - before.delay_ms).abs() >= DELAY_EPS {
        out.push(MeasureTuneWrite {
            output: o,
            kind: MeasureTuneKind::Delay,
            timing: true,
            words: format!(
                "{name}: delay {:.4} → {:.4} ms",
                before.delay_ms, after.delay_ms
            ),
            params: vec![param(
                "out.delay",
                vec![o],
                ParamValue::Float(after.delay_ms),
            )],
        });
    }
    if after.feed.inverted != before.feed.inverted {
        let word = |inv: bool| if inv { "inverted" } else { "normal" };
        let params = w
            .feeds
            .iter()
            .map(|(input, gain, inv)| {
                let p = dspi_proto::packets::MatrixRoutePacket {
                    input: *input,
                    output: o,
                    enabled: true,
                    phase_invert: !inv,
                    gain_db: *gain,
                };
                param(
                    "mix",
                    vec![*input, o],
                    ParamValue::Bytes(p.encode().to_vec()),
                )
            })
            .collect();
        out.push(MeasureTuneWrite {
            output: o,
            kind: MeasureTuneKind::Polarity,
            timing: true,
            words: format!(
                "{name}: polarity {} → {}",
                word(before.feed.inverted),
                word(after.feed.inverted)
            ),
            params,
        });
    }
    for (band, (b, a)) in (0u8..).zip(before.peq.iter().zip(&after.peq)) {
        if a == b {
            continue;
        }
        let mut params = Vec::new();
        let idx = vec![ch, band];
        if a.filter_type != b.filter_type {
            params.push(param(
                "eq.type",
                idx.clone(),
                ParamValue::Choice(a.filter_type),
            ));
        }
        if a.filter_type != 0 {
            params.push(param("eq.freq", idx.clone(), ParamValue::Float(a.freq)));
            params.push(param("eq.q", idx.clone(), ParamValue::Float(a.q)));
            params.push(param("eq.gain", idx.clone(), ParamValue::Float(a.gain)));
        }
        if a.bypass != b.bypass {
            params.push(param("eq.bypass", idx, ParamValue::Bool(a.bypass)));
        }
        out.push(MeasureTuneWrite {
            output: o,
            kind: MeasureTuneKind::Eq,
            timing: false,
            words: format!(
                "{name}: PEQ band {}: {} → {}",
                u16::from(band).saturating_add(1),
                band_words(&params_of(b)),
                band_words(&params_of(a))
            ),
            params,
        });
    }
    out
}

/// Mark the points of `base` that lie more than `max_boost_db` (and a
/// margin) below `target` levelled to it in the reference band, smoothed to
/// 1/12 octave, as untrusted: a fit never pulls its level down to meet a dip
/// it may not fill. Returns how many points were marked.
fn mask_dips(base: &mut Response, target: &Response, max_boost_db: f64) -> usize {
    let level = levelled_target(target, base);
    let Ok(level) = dspi_measure::response::resample(&level, &base.grid) else {
        return 0;
    };
    let Ok(sm) = dspi_measure::response::smooth(base, 12) else {
        return 0;
    };
    let n = base.mag_db.len();
    let valid = base.valid.get_or_insert_with(|| vec![true; n]);
    let mut marked = 0usize;
    for (i, ok) in valid.iter_mut().enumerate() {
        let (Some(m), Some(t)) = (sm.mag_db.get(i), level.mag_db.get(i)) else {
            continue;
        };
        if *ok && *m < t - max_boost_db - DIP_MARGIN_DB {
            *ok = false;
            marked = marked.saturating_add(1);
        }
    }
    marked
}

/// The proposal (see the module docs). `target` on the standard grid;
/// `allow` withdraws timing or the fit per speaker.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compute(
    doc: &MeasurementDoc,
    ways: &[WayIn],
    options: &MeasureTuneOptions,
    bounds: &Bounds,
    target: &Response,
    allow: &Allow,
    model: &dyn SectionModel,
) -> Computed {
    let mut notes = Vec::new();
    let mut withdrawn = Vec::new();
    let mut held = Vec::new();
    let mut speakers = speakers_of(ways, &mut notes);
    let mut chains: Vec<ChannelChain> = ways.iter().map(|w| w.chain_now.clone()).collect();
    let mut level: Vec<f32> = vec![0.0; ways.len()];
    let reliable = doc.measurement.capture.timing_reliable;
    held.push(format!(
        "delays within {:.1} to {:.1} ms",
        bounds.min_delay_ms, bounds.max_delay_ms
    ));
    held.push("levels cut only; no output gain raised".into());
    for h in &bounds.highpass {
        held.push(format!(
            "output {}: a high-pass at or above {:.0} Hz, {} dB/octave (limits profile)",
            u16::from(h.output).saturating_add(1),
            h.rule.min_hz,
            h.rule.min_slope_db
        ));
    }
    // ── Steps 3 to 5: de-embedding, delay, polarity, corner, levels ──
    for sp in &mut speakers {
        if sp.transitions.is_empty() {
            continue;
        }
        let mut withdraw = |key: &str, why: String| {
            withdrawn.push(MeasureIssue::new(key, None, format!("{}: {why}", sp.name)));
        };
        let way_list: Vec<&WayIn> = sp.ways.iter().filter_map(|i| ways.get(*i)).collect();
        let drivers: Option<Vec<Driver>> = way_list
            .iter()
            .map(|w| {
                let way = way_of(w)?;
                let mut d = de_embed(&way, model).ok()?;
                d.chain = w.chain_now.clone();
                Some(d)
            })
            .collect();
        if !reliable {
            withdraw(
                KEY_UNRELIABLE,
                "the measurement is timing unreliable: no delay, polarity or crossover change is \
                 offered; measure again for those"
                    .into(),
            );
            continue;
        }
        let Some(drivers) = drivers else {
            withdraw(
                KEY_NO_TIMING,
                "the measurement has no phase or arrival for every way (several positions \
                 averaged?): no delay, polarity or crossover change is offered"
                    .into(),
            );
            continue;
        };
        if way_list.iter().any(|w| w.snr_db.is_none()) {
            notes.push(format!(
                "{}: the measurement holds no per-point SNR; only each driver's passband is \
                 trusted",
                sp.name
            ));
        }
        if let Some((key, why)) = allow.no_timing.get(&sp.name) {
            withdraw(key, why.clone());
            continue;
        }
        sp.drivers = Some(drivers.clone());
        let constraints = XoConstraints {
            window_octaves: if options.window_octaves > 0.0 {
                options.window_octaves
            } else {
                dspi_measure::xover::DEFAULT_WINDOW_OCTAVES
            },
            keep_corners: options.keep_crossover,
            keep_levels: options.keep_levels,
            highpass: bounds.highpass.clone(),
            align: AlignOptions {
                min_delay_ms: bounds.min_delay_ms,
                max_delay_ms: bounds.max_delay_ms,
                keep_delays: options.keep_delays,
            },
            ..XoConstraints::default()
        };
        match crossover_corner(&drivers, &sp.transitions, target, &constraints, model) {
            Ok(p) => {
                for (k, o) in p.outputs.iter().enumerate() {
                    if let Some(i) = sp.ways.get(k) {
                        if let Some(slot) = chains.get_mut(*i) {
                            *slot = o.chain.clone();
                        }
                        if let Some(l) = level.get_mut(*i) {
                            *l = o.level_change_db as f32;
                        }
                    }
                }
                sp.xo = p
                    .transitions
                    .iter()
                    .map(|t| {
                        (
                            t.corner_before_hz,
                            t.corner_hz,
                            t.relative_delay_ms,
                            t.flipped,
                        )
                    })
                    .collect();
                for t in &p.transitions {
                    if let Some(r) = &t.kept_reason {
                        notes.push(format!("{}: {r}", sp.name));
                    }
                }
                notes.extend(p.notes.iter().map(|n| format!("{}: {n}", sp.name)));
                sp.timing = true;
            }
            Err(e) => withdraw(
                KEY_NO_TIMING,
                format!("no delay, polarity or crossover change could be found: {e}"),
            ),
        }
    }
    // ── Step 7: the target fit ──
    let (def_lo, def_hi) = if bounds.headphones {
        FIT_HEADPHONES_HZ
    } else {
        FIT_SPEAKERS_HZ
    };
    let lo = if options.fit_min_hz > 0.0 {
        options.fit_min_hz.max(10.0)
    } else {
        def_lo
    };
    let hi = if options.fit_max_hz > 0.0 {
        options.fit_max_hz.min(22_000.0)
    } else {
        def_hi
    };
    held.push(format!(
        "PEQ boost at most {:+.1} dB{}, cuts at most -15 dB, Q 0.3 to 10, free bands only",
        bounds.max_boost_db,
        if bounds.boost_from_profile {
            " (limits profile)"
        } else {
            ""
        }
    ));
    let mut cuts: Vec<f32> = Vec::new();
    for sp in &mut speakers {
        if options.keep_eq {
            break;
        }
        if let Some(why) = allow.no_fit.get(&sp.name) {
            withdrawn.push(MeasureIssue::new(
                KEY_NO_FIT,
                None,
                format!("{}: {why}", sp.name),
            ));
            continue;
        }
        let Some(&first) = sp.ways.first() else {
            continue;
        };
        let (base, targets, top) = if sp.drivers.is_some() && sp.timing {
            let base = speaker_response(sp, ways, &chains, model);
            let targets: Vec<usize> = sp
                .ways
                .iter()
                .copied()
                .filter(|i| chains.get(*i).is_some_and(|c| plays_in(c, lo, hi, model)))
                .collect();
            (base, targets, hi)
        } else {
            let base = ways.get(first).and_then(|w| {
                repredict(&w.response, &w.measured_chain, chains.get(first)?, model).ok()
            });
            let top = if sp.ways.len() > 1 {
                chains
                    .get(first)
                    .and_then(lowpass_corner)
                    .map_or(hi, |c| hi.min(f64::from(c) / 2.0))
            } else {
                hi
            };
            (base, vec![first], top)
        };
        let Some(mut base) = base else {
            notes.push(format!("{}: no response to fit", sp.name));
            continue;
        };
        // The whole speaker (or a single way) holds the reference band: a dip
        // the boost cap cannot fill there is left alone, so the fit's free
        // level is not pulled down to meet it (which would move the bass
        // against the rest of the speaker).
        let whole = sp.ways.len() == 1 || (sp.drivers.is_some() && sp.timing);
        if whole {
            let left = mask_dips(&mut base, target, bounds.max_boost_db);
            if left > 0 {
                notes.push(format!(
                    "{}: {left} points of dips deeper than the boost cap are left alone",
                    sp.name
                ));
            }
        }
        let Some((f1, f2)) = fit_span(&base, lo, top) else {
            notes.push(format!(
                "{}: the measurement is not trusted in the fit's range",
                sp.name
            ));
            continue;
        };
        let n_peq = targets
            .iter()
            .filter_map(|i| chains.get(*i).map(|c| c.peq.len()))
            .min()
            .unwrap_or(0);
        let allowed: Vec<u8> = (0u8..)
            .take(n_peq)
            .filter(|b| {
                targets.iter().all(|i| {
                    chains
                        .get(*i)
                        .and_then(|c| c.peq.get(usize::from(*b)))
                        .is_some_and(free)
                })
            })
            .collect();
        if allowed.is_empty() {
            notes.push(format!(
                "{}: no PEQ band is free on every way that plays there: no EQ fit",
                sp.name
            ));
            continue;
        }
        let mut fo = if bounds.headphones {
            FitOptions::headphones()
        } else {
            FitOptions::speakers()
        };
        fo.min_hz = f1;
        fo.max_hz = f2;
        fo.max_boost_db = bounds.max_boost_db.clamp(0.0, fo.max_boost_db.max(0.0));
        // The fit's level is free within its range, so a broad band there
        // would move the bass against the rest of the speaker (a level the
        // fit does not see), and a shelf at the top of the range would tilt
        // every way above it: for speakers, peaks of Q 2 or more only (room
        // modes are resonances); the bass level is the crossover's.
        fo.shelves = bounds.headphones;
        if !bounds.headphones {
            fo.q_min = fo.q_min.max(SPEAKERS_Q_MIN);
        }
        fo.bands = Some(allowed);
        fo.spread_db = ways
            .get(first)
            .and_then(|w| w.spread_db.clone())
            .filter(|s| s.len() == base.mag_db.len());
        let Some(chain0) = targets.first().and_then(|i| chains.get(*i)).cloned() else {
            continue;
        };
        match fit(&base, target, &chain0, model, &fo) {
            Ok(r) => {
                for b in &r.bands {
                    for i in &targets {
                        if let Some(slot) = chains
                            .get_mut(*i)
                            .and_then(|c| c.peq.get_mut(usize::from(b.index)))
                        {
                            let mut band = b.band;
                            band.qp = slot.qp;
                            *slot = band;
                        }
                    }
                }
                if !r.bands.is_empty() {
                    cuts.push(r.gain_cut_db.min(0.0));
                }
                notes.extend(r.notes.iter().map(|n| format!("{}: {n}", sp.name)));
                sp.fit = Some(r);
            }
            Err(e) => notes.push(format!("{}: no EQ fit: {e}", sp.name)),
        }
    }
    // One headroom cut for every output, so no balance moves.
    let headroom = crate::curves::quantize_gain(cuts.iter().copied().fold(0.0f32, f32::min));
    if headroom < 0.0 {
        for c in &mut chains {
            c.gain_db = crate::curves::quantize_gain(c.gain_db + headroom);
        }
        notes.push(format!(
            "every output's gain is cut by {:.1} dB for the EQ's boost (headroom)",
            -headroom
        ));
    }
    // ── The curves ──
    for sp in &mut speakers {
        let now: Vec<ChannelChain> = ways.iter().map(|w| w.chain_now.clone()).collect();
        sp.coherent = sp.drivers.is_some();
        sp.before = speaker_response(sp, ways, &now, model);
        sp.predicted = speaker_response(sp, ways, &chains, model);
    }
    // ── Step 8: the writes in words ──
    let mut writes = Vec::new();
    for (i, w) in ways.iter().enumerate() {
        let (Some(after), Some(l)) = (chains.get(i), level.get(i)) else {
            continue;
        };
        writes.extend(writes_of(w, &w.chain_now, after, *l, headroom));
    }
    Computed {
        chains,
        writes,
        withdrawn,
        held,
        notes,
        speakers,
    }
}
