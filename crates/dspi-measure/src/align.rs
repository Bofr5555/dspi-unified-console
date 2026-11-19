//! Delay and polarity between ways (WP P5XO; PLAN-P5 section 1.4 step 4, T2).
//!
//! The ways of one **alignment group** are measured in one walked run, so their
//! responses share one timing reference: each response's phase is relative to
//! its own cycle start, and the common latency cancels between them
//! (section 1.1 step 6). The measured response of a way already holds its
//! measured chain (crossover, PEQ, output gain and **its current delay**), so
//! a new delay `d'` is the measured response times `exp(-j·2πf·(d' - d))`, and
//! the programme feed (crosspoint gain and polarity, review RC1) is added on
//! top: the generator bypasses the matrix, programme audio does not.
//!
//! [`align`] finds, per transition (two ways that cross over), the relative
//! delay and polarity at which the two ways' **programme** contributions
//! agree in phase through their overlap (one octave each side of the corner,
//! weighted where both contribute), starting from the coarse arrivals and
//! refining to a fraction of a microsecond; the flatness of the predicted sum
//! at that optimum is reported with it.
//!
//! Why phase agreement and not "the flattest summed magnitude" (PLAN-P5 T2's
//! wording; settled at the wave P5-A review on P5SIM's three-way room): for a
//! Linkwitz-Riley pair the right alignment and "the wrong polarity, half a
//! period of the corner later" both sum almost flat through the transition,
//! so the flatness of the magnitude is a weak judge; a driver resonance
//! inside the window then decides, and it decides for whichever
//! half-cancelled sum happens to hide it: a correctly wired tweeter was
//! flipped. Phase agreement tells the two apart by a wide margin and no
//! magnitude feature can buy a wrong timing. The measured responses hold the
//! crossover filters, so an LR2 or even-order Butterworth pair is judged with
//! the inversion it needs. It is the rule [`crate::xover::crossover_corner`]
//! uses per candidate, so "align only" gives the same answer either way.
//!
//! **Odd-order Butterworth pairs** (BW1, BW3, BW5, BW7 against each other)
//! are in quadrature by design: they sum flat with the ways 90° apart, in
//! either polarity. Pulling them into phase would put a +3 dB bump at the
//! corner and move the delay by a quarter period. For such a pair the delay
//! is the one that puts the ways in quadrature through the overlap, and
//! **its polarity is never changed** (neither polarity is flatter; which lobe
//! tilt the user chose is theirs), with a confidence of 0.
//!
//! A polarity flip is proposed only when it agrees clearly better
//! ([`FLIP_MARGIN`]): a near tie keeps the polarity there is. The pairs are
//! then composed over the group and **normalised so the group's smallest delay
//! keeps its current value** (review RC6, OD-P5j): a speaker's existing common
//! delay, left/right distance, sub or lip-sync alignment is never erased.
//! Polarity is the net invert of the output's feeding crosspoints, the device's
//! only polarity control. Nothing is offered for a run that is not "timing
//! reliable" (RC5d).
//!
//! Pure and deterministic: no hash maps, no clock, no panics on any input.

use crate::types::{ChannelChain, LogGrid, MeasureError, Response, WayRole};
use serde::{Deserialize, Serialize};

/// The most ways one alignment group (one walked run) holds: the run's bound
/// of 16 outputs (P5TIME).
pub const MAX_WAYS: usize = 16;
/// The fewest trusted points a transition's window must hold to be judged.
pub const MIN_WINDOW_POINTS: usize = 12;
/// Coarse delay steps each side of the start for [`align`]'s search.
const ALIGN_COARSE_STEPS: usize = 96;
/// Golden-section iterations of the fine delay search.
pub(crate) const REFINE_ITERS: usize = 24;
/// The search window each side of the coarse estimate, ms: one period of the
/// corner, but at least this.
const MIN_HALF_WINDOW_MS: f64 = 0.5;
/// ... and at most this (a 50 Hz corner).
const MAX_HALF_WINDOW_MS: f64 = 20.0;
/// The largest delay any option or chain may carry, ms (far above every
/// platform's maximum, 42 ms; a bound against absurd input).
pub const MAX_DELAY_BOUND_MS: f64 = 1000.0;
/// A polarity flip must lower the cost by at least this fraction of the
/// unflipped cost to be proposed; a near tie keeps the polarity there is.
pub const FLIP_MARGIN: f64 = 0.1;
/// The device's delay grid, ms (`dspi-core` `curves::quantize_delay`).
const DELAY_STEP_MS: f64 = 0.0001;
/// Slack when comparing a quantised delay with its bounds, ms.
const DELAY_SLACK_MS: f64 = 1e-6;

/// One way of a speaker as measured in a walked run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Way {
    /// The output it was measured on.
    pub output: u8,
    /// Its role in the layout (the tweeter rule depends on it).
    pub role: WayRole,
    /// The measured chain: what the output had while it was measured.
    pub chain: ChannelChain,
    /// The measured response, **with phase** on the run's common timing
    /// reference, and its `valid` mask (`None` reads as all trusted).
    pub response: Response,
    /// The measurement's signal-to-noise ratio per grid point, dB: the
    /// measured response (the chain's attenuation in it) over the noise floor
    /// from the pre-roll there. It is the de-embedding's validity mask as it
    /// is (review RC12, review P5-C2: no attenuation is subtracted again).
    pub snr_db: Vec<f64>,
    /// The coarse arrival (the impulse response's onset) relative to the
    /// way's own cycle start, ms; it includes the chain's current delay.
    pub arrival_ms: f64,
    /// The run passed the timing quality gate (PLAN-P5 section 1.1 step 7).
    pub timing_reliable: bool,
}

/// Two ways that cross over: `low` plays below the corner, `high` above it.
/// Indices into the ways (or drivers) passed with it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    /// Index of the lower way.
    pub low: usize,
    /// Index of the upper way.
    pub high: usize,
    /// The corner, Hz, when the chains have no crossover pair to read it
    /// from (a sub and a full-range main with PEQ filters); otherwise `None`
    /// and the corner is the geometric mean of the low way's low-pass and the
    /// high way's high-pass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_hz: Option<f64>,
}

/// The ways aligned to each other (PLAN-P5 section 1.4 step 4): by default one
/// speaker's ways; "align the whole system" puts both sides and the sub in it.
/// Ways not joined by a transition keep their delays relative to each other.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlignGroup {
    /// The transitions, each judged on its own window.
    pub transitions: Vec<Transition>,
}

/// Bounds and choices of an alignment.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlignOptions {
    /// The least delay a proposal may write, ms (the limits profile's
    /// `delay_min_ms`, else 0).
    pub min_delay_ms: f64,
    /// The most, ms: the platform's maximum (21 ms RP2040, 42 ms RP2350), or
    /// the limits profile's `delay_max_ms` when lower.
    pub max_delay_ms: f64,
    /// Keep the current delays (the user's choice); only polarity is judged.
    pub keep_delays: bool,
}

impl Default for AlignOptions {
    /// 0 to 21 ms (the smaller platform's range), delays free.
    fn default() -> Self {
        AlignOptions {
            min_delay_ms: 0.0,
            max_delay_ms: 21.0,
            keep_delays: false,
        }
    }
}

/// The proposal for one way.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WayAlignment {
    /// The output.
    pub output: u8,
    /// Its delay now, ms.
    pub delay_ms_before: f32,
    /// The proposed delay, ms, on the device's 0.0001 ms grid.
    pub delay_ms: f32,
    /// Its programme feed's net polarity now.
    pub inverted_before: bool,
    /// The proposed net polarity of its feeding crosspoints.
    pub inverted: bool,
}

/// What one transition's search found.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairAlignment {
    /// The lower way's output.
    pub low_output: u8,
    /// The upper way's output.
    pub high_output: u8,
    /// The corner the window was centred on, Hz.
    pub corner_hz: f64,
    /// The upper way's new delay minus the lower way's, ms.
    pub relative_delay_ms: f64,
    /// The upper way's polarity is flipped relative to the lower way's.
    pub flipped: bool,
    /// The flatness of the predicted sum at the optimum: RMS deviation of
    /// the summed magnitude from a straight line through the window, dB.
    pub cost_db: f64,
    /// 0 to 1: how clearly the optimum's phase agreement beats the best of
    /// the other polarity, times the share of the window's points that were
    /// trusted. 0 for an odd-order Butterworth pair, whose polarity is kept.
    pub confidence: f64,
}

/// The result of [`align`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlignResult {
    /// One entry per way, in the order given.
    pub ways: Vec<WayAlignment>,
    /// One entry per transition, in the order given.
    pub pairs: Vec<PairAlignment>,
    /// Remarks in words (a transition ignored, ways with no partner).
    pub notes: Vec<String>,
}

/// Delay and polarity of the ways of one alignment group (T2).
///
/// Refused ([`MeasureError::Invalid`], the text starting "timing unreliable")
/// unless every way's run passed the timing quality gate. Every transition is
/// judged on the two ways' programme contributions through one octave each
/// side of its corner, on the points both ways trust (phase agreement, or
/// quadrature for an odd-order Butterworth pair: see the module's notes); the
/// delays are normalised so the
/// group's smallest delay keeps its current value and must stay within
/// `options`' range, else the alignment is refused with the delay it needs.
///
/// Errors: [`MeasureError::Invalid`] for a malformed input (no ways, more than
/// [`MAX_WAYS`], grids that differ, no phase, a transition out of range);
/// [`MeasureError::NoSignal`] when a transition's window holds fewer than
/// [`MIN_WINDOW_POINTS`] trusted points.
pub fn align(
    ways: &[Way],
    group: &AlignGroup,
    options: &AlignOptions,
) -> Result<AlignResult, MeasureError> {
    check_count(ways.len())?;
    check_options(options)?;
    if group.transitions.len() > MAX_WAYS.saturating_mul(2) {
        return Err(MeasureError::Invalid(format!(
            "{} transitions",
            group.transitions.len()
        )));
    }
    if let Some(w) = ways.iter().find(|w| !w.timing_reliable) {
        return Err(MeasureError::Invalid(format!(
            "timing unreliable: output {}'s run did not pass the timing quality gate; no delay or polarity is offered",
            w.output
        )));
    }
    let grid = common_grid(ways.iter().map(|w| &w.response))?;
    let freqs = grid.freqs()?;
    // Each way's programme contribution without its current delay.
    let mut prog: Vec<Vec<Cx>> = Vec::with_capacity(ways.len());
    let mut trust: Vec<Vec<bool>> = Vec::with_capacity(ways.len());
    for w in ways {
        check_chain(&w.chain)?;
        let (h, ok) = response_to_complex(&w.response, freqs.len())?;
        let d = f64::from(w.chain.delay_ms);
        let feed = feed_factor(&w.chain);
        let p: Vec<Cx> = h
            .iter()
            .zip(&freqs)
            .map(|(x, f)| x.mul(Cx::delay(*f, -d)).mul(feed))
            .collect();
        prog.push(p);
        trust.push(ok);
    }
    let mut edges = Vec::with_capacity(group.transitions.len());
    let mut pairs = Vec::with_capacity(group.transitions.len());
    let mut notes = Vec::new();
    for t in &group.transitions {
        let (wa, wb) = transition_ways(ways, t)?;
        let info = transition_corner(&wa.chain, &wb.chain, t.corner_hz)?;
        let corner = info.corner_hz;
        let (Some(pa), Some(pb), Some(ta), Some(tb)) = (
            prog.get(t.low),
            prog.get(t.high),
            trust.get(t.low),
            trust.get(t.high),
        ) else {
            return Err(MeasureError::Invalid("transition out of range".into()));
        };
        let lo = corner / 2.0;
        let hi = corner * 2.0;
        let mut pts = Vec::new();
        let mut total = 0usize;
        for (i, f) in freqs.iter().enumerate() {
            if *f < lo || *f > hi {
                continue;
            }
            total = total.saturating_add(1);
            let ok = ta.get(i).copied().unwrap_or(false) && tb.get(i).copied().unwrap_or(false);
            if let (true, Some(a), Some(b)) = (ok, pa.get(i), pb.get(i)) {
                pts.push(Point::new(*f, *a, *b, 0.0));
            }
        }
        if pts.len() < MIN_WINDOW_POINTS {
            return Err(MeasureError::NoSignal(format!(
                "outputs {} and {}: {} trusted points through the transition at {:.0} Hz (at least {MIN_WINDOW_POINTS})",
                wa.output,
                wb.output,
                pts.len(),
                corner
            )));
        }
        let da = f64::from(wa.chain.delay_ms);
        let db = f64::from(wb.chain.delay_ms);
        let quadrature = info.quadrature(&wa.chain, &wb.chain);
        if quadrature {
            notes.push(quadrature_note(wa.output, wb.output));
        }
        let ctx = PairCtx {
            points: pts,
            kind: if quadrature {
                CostKind::Quadrature
            } else {
                CostKind::Phase
            },
        };
        let fit = if options.keep_delays {
            ctx.search(db - da, 0.0, 0, 0)
        } else {
            let start = (wa.arrival_ms - da) - (wb.arrival_ms - db);
            let start = if start.is_finite() && start.abs() <= MAX_DELAY_BOUND_MS {
                start
            } else {
                db - da
            };
            ctx.search(
                start,
                half_window_ms(corner),
                ALIGN_COARSE_STEPS,
                REFINE_ITERS,
            )
        };
        let share = if total == 0 {
            0.0
        } else {
            ctx.points.len() as f64 / total as f64
        };
        // The sum's flatness at the phase optimum, for the report.
        let flatness = PairCtx {
            points: ctx.points.clone(),
            kind: CostKind::Flatness,
        }
        .cost(fit.delta_ms, fit.flip);
        pairs.push(PairAlignment {
            low_output: wa.output,
            high_output: wb.output,
            corner_hz: corner,
            relative_delay_ms: fit.delta_ms,
            flipped: fit.flip,
            cost_db: flatness,
            confidence: if quadrature {
                0.0
            } else {
                fit.confidence(share)
            },
        });
        edges.push(Edge {
            low: t.low,
            high: t.high,
            delta_ms: fit.delta_ms,
            flip: fit.flip,
        });
    }
    let current: Vec<f64> = ways.iter().map(|w| f64::from(w.chain.delay_ms)).collect();
    let (delays, flips) = compose(&current, &edges, options, &mut notes)?;
    let mut out = Vec::with_capacity(ways.len());
    for ((w, d), fl) in ways.iter().zip(&delays).zip(&flips) {
        out.push(WayAlignment {
            output: w.output,
            delay_ms_before: w.chain.delay_ms,
            delay_ms: if options.keep_delays {
                w.chain.delay_ms
            } else {
                *d
            },
            inverted_before: w.chain.feed.inverted,
            inverted: w.chain.feed.inverted ^ *fl,
        });
    }
    if group.transitions.is_empty() && ways.len() > 1 {
        notes.push("no transition joins these ways: their delays and polarities are kept".into());
    }
    Ok(AlignResult {
        ways: out,
        pairs,
        notes,
    })
}

// ---------------------------------------------------------------------------
// Shared with `xover` (pub(crate)).
// ---------------------------------------------------------------------------

/// A complex number. Methods instead of operators so the crate's
/// `arithmetic_side_effects` lint sees plain float arithmetic only.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Cx {
    pub(crate) re: f64,
    pub(crate) im: f64,
}

impl Cx {
    pub(crate) const ZERO: Cx = Cx { re: 0.0, im: 0.0 };
    pub(crate) const ONE: Cx = Cx { re: 1.0, im: 0.0 };

    pub(crate) fn polar(mag: f64, rad: f64) -> Cx {
        Cx {
            re: mag * rad.cos(),
            im: mag * rad.sin(),
        }
    }
    /// `exp(-j·2π·f·ms/1000)`: a delay of `ms` at `f` Hz.
    pub(crate) fn delay(f: f64, ms: f64) -> Cx {
        Cx::polar(1.0, -std::f64::consts::TAU * f * ms / 1000.0)
    }
    pub(crate) fn add(self, o: Cx) -> Cx {
        Cx {
            re: self.re + o.re,
            im: self.im + o.im,
        }
    }
    pub(crate) fn mul(self, o: Cx) -> Cx {
        Cx {
            re: self.re * o.re - self.im * o.im,
            im: self.re * o.im + self.im * o.re,
        }
    }
    pub(crate) fn scale(self, k: f64) -> Cx {
        Cx {
            re: self.re * k,
            im: self.im * k,
        }
    }
    /// `self / o`, or `None` when `o` is (nearly) zero or the result is not
    /// finite.
    pub(crate) fn div(self, o: Cx) -> Option<Cx> {
        let d = o.norm_sqr();
        if d.is_nan() || d <= 1e-24 {
            return None;
        }
        let r = Cx {
            re: (self.re * o.re + self.im * o.im) / d,
            im: (self.im * o.re - self.re * o.im) / d,
        };
        r.is_finite().then_some(r)
    }
    pub(crate) fn norm_sqr(self) -> f64 {
        self.re * self.re + self.im * self.im
    }
    pub(crate) fn arg(self) -> f64 {
        self.im.atan2(self.re)
    }
    pub(crate) fn is_finite(self) -> bool {
        self.re.is_finite() && self.im.is_finite()
    }
    /// Magnitude in dB, floored at -200 dB.
    pub(crate) fn db(self) -> f64 {
        let p = self.norm_sqr();
        if p.is_finite() {
            10.0 * p.max(1e-20).log10()
        } else {
            400.0
        }
    }
}

/// The programme feed as a factor: crosspoint gain and net polarity.
pub(crate) fn feed_factor(chain: &ChannelChain) -> Cx {
    let g = db_to_lin(f64::from(chain.feed.gain_db));
    Cx {
        re: if chain.feed.inverted { -g } else { g },
        im: 0.0,
    }
}

/// `10^(db/20)`, 0 for a non-finite value; clamped to ±400 dB.
pub(crate) fn db_to_lin(db: f64) -> f64 {
    if db.is_finite() {
        10f64.powf(db.clamp(-400.0, 400.0) / 20.0)
    } else {
        0.0
    }
}

pub(crate) fn check_count(n: usize) -> Result<(), MeasureError> {
    if n == 0 || n > MAX_WAYS {
        return Err(MeasureError::Invalid(format!("{n} ways (1 to {MAX_WAYS})")));
    }
    Ok(())
}

pub(crate) fn check_options(o: &AlignOptions) -> Result<(), MeasureError> {
    let ok = o.min_delay_ms.is_finite()
        && o.max_delay_ms.is_finite()
        && o.min_delay_ms >= 0.0
        && o.max_delay_ms >= o.min_delay_ms
        && o.max_delay_ms <= MAX_DELAY_BOUND_MS;
    if !ok {
        return Err(MeasureError::Invalid(format!(
            "delay range {} to {} ms",
            o.min_delay_ms, o.max_delay_ms
        )));
    }
    Ok(())
}

/// Every number of a chain finite and the delay within the absolute bound.
pub(crate) fn check_chain(c: &ChannelChain) -> Result<(), MeasureError> {
    let bands_ok =
        c.peq.iter().chain(&c.crossover).all(|b| {
            b.freq.is_finite() && b.q.is_finite() && b.gain.is_finite() && b.qp.is_finite()
        });
    let ok = bands_ok
        && c.gain_db.is_finite()
        && c.gain_db.abs() <= 200.0
        && c.delay_ms.is_finite()
        && c.feed.gain_db.is_finite()
        && c.feed.gain_db.abs() <= 200.0
        && f64::from(c.delay_ms).abs() <= MAX_DELAY_BOUND_MS
        && c.peq.len() <= 64
        && c.crossover.len() <= 64;
    if !ok {
        return Err(MeasureError::Invalid(
            "a chain with a non-finite or absurd value, or too many bands".into(),
        ));
    }
    Ok(())
}

/// The one grid every response shares.
pub(crate) fn common_grid<'a>(
    mut rs: impl Iterator<Item = &'a Response>,
) -> Result<LogGrid, MeasureError> {
    let first = rs
        .next()
        .ok_or_else(|| MeasureError::Invalid("no responses".into()))?;
    let g = first.grid;
    if rs.any(|r| r.grid != g) {
        return Err(MeasureError::Invalid(
            "the ways' responses are on different grids".into(),
        ));
    }
    g.freqs()?;
    Ok(g)
}

/// A response with phase as complex values, and per point whether it is
/// trusted (its `valid` flag and finite values).
pub(crate) fn response_to_complex(
    r: &Response,
    n: usize,
) -> Result<(Vec<Cx>, Vec<bool>), MeasureError> {
    let phase = r
        .phase_deg
        .as_ref()
        .ok_or_else(|| MeasureError::Invalid("a response without phase".into()))?;
    if r.mag_db.len() != n || phase.len() != n {
        return Err(MeasureError::Invalid(format!(
            "a response of {} points with {} phases on a grid of {n}",
            r.mag_db.len(),
            phase.len()
        )));
    }
    if let Some(v) = &r.valid
        && v.len() != n
    {
        return Err(MeasureError::Invalid(format!(
            "a valid mask of {} points on a grid of {n}",
            v.len()
        )));
    }
    let mut h = Vec::with_capacity(n);
    let mut ok = Vec::with_capacity(n);
    for (i, (m, p)) in r.mag_db.iter().zip(phase).enumerate() {
        let valid = r
            .valid
            .as_ref()
            .and_then(|v| v.get(i).copied())
            .unwrap_or(true);
        // A phase past a billion degrees has lost its precision; it is
        // untrusted rather than guessed at.
        let finite = m.is_finite() && p.is_finite() && m.abs() < 400.0 && p.abs() < 1e9;
        if finite {
            h.push(Cx::polar(db_to_lin(*m), p.to_radians()));
        } else {
            h.push(Cx::ZERO);
        }
        ok.push(valid && finite);
    }
    Ok((h, ok))
}

pub(crate) fn transition_ways<'a, T>(
    ways: &'a [T],
    t: &Transition,
) -> Result<(&'a T, &'a T), MeasureError> {
    match (ways.get(t.low), ways.get(t.high)) {
        (Some(a), Some(b)) if t.low != t.high => Ok((a, b)),
        _ => Err(MeasureError::Invalid(format!(
            "transition {} to {} with {} ways",
            t.low,
            t.high,
            ways.len()
        ))),
    }
}

/// Where a transition's crossover pair sits in its chains.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CornerInfo {
    /// The geometric mean of the low-pass and high-pass corners (or the
    /// override), Hz.
    pub(crate) corner_hz: f64,
    /// The low way's low-pass slot and the high way's high-pass slot in their
    /// crossover banks, when both exist.
    pub(crate) slots: Option<(usize, usize)>,
}

impl CornerInfo {
    /// Whether the transition's crossover pair is in quadrature by design: an
    /// odd-order Butterworth low-pass against an odd-order Butterworth
    /// high-pass. Such a pair sums flat with the ways 90° apart in either
    /// polarity, so it is aligned to quadrature and its polarity is kept.
    pub(crate) fn quadrature(&self, low: &ChannelChain, high: &ChannelChain) -> bool {
        let Some((ls, hs)) = self.slots else {
            return false;
        };
        let odd_butterworth = |b: Option<&crate::types::BandSpec>| {
            b.and_then(|b| dspi_proto::xover::meta(b.filter_type))
                .is_some_and(|m| {
                    m.family == dspi_proto::xover::Family::Butterworth && m.order & 1 == 1
                })
        };
        odd_butterworth(low.crossover.get(ls)) && odd_butterworth(high.crossover.get(hs))
    }
}

/// The note a quadrature pair gets.
pub(crate) fn quadrature_note(low_output: u8, high_output: u8) -> String {
    format!(
        "outputs {low_output} and {high_output}: an odd-order Butterworth pair sums flat in either polarity; its polarity is kept and its delay puts the ways in quadrature"
    )
}

/// The transition's corner: the low way's active crossover low-pass and the
/// high way's active crossover high-pass nearest to each other, or the
/// override.
pub(crate) fn transition_corner(
    low: &ChannelChain,
    high: &ChannelChain,
    override_hz: Option<f64>,
) -> Result<CornerInfo, MeasureError> {
    let pick = |c: &ChannelChain, hp: bool| -> Vec<(usize, f64)> {
        c.crossover
            .iter()
            .enumerate()
            .filter(|(_, b)| !b.bypass && b.freq.is_finite() && b.freq > 0.0)
            .filter(|(_, b)| {
                dspi_proto::xover::meta(b.filter_type).is_some_and(|m| m.high_pass == hp)
            })
            .map(|(i, b)| (i, f64::from(b.freq)))
            .collect()
    };
    let lps = pick(low, false);
    let hps = pick(high, true);
    let mut best: Option<(usize, usize, f64, f64)> = None;
    for (i, fl) in &lps {
        for (j, fh) in &hps {
            let dist = (fl / fh).log2().abs();
            if best.is_none_or(|b| dist < b.2) {
                best = Some((*i, *j, dist, (fl * fh).sqrt()));
            }
        }
    }
    match (override_hz, best) {
        (Some(c), _) if c.is_finite() && c > 0.0 => Ok(CornerInfo {
            corner_hz: c,
            slots: best.map(|b| (b.0, b.1)),
        }),
        (Some(c), _) => Err(MeasureError::Invalid(format!("corner {c} Hz"))),
        (None, Some(b)) => Ok(CornerInfo {
            corner_hz: b.3,
            slots: Some((b.0, b.1)),
        }),
        (None, None) => Err(MeasureError::Invalid(
            "a transition without a crossover low-pass and high-pass pair and no corner given"
                .into(),
        )),
    }
}

/// The delay search's half window, ms: one period of the corner, bounded.
pub(crate) fn half_window_ms(corner_hz: f64) -> f64 {
    let p = 1000.0 / corner_hz;
    if p.is_finite() {
        p.clamp(MIN_HALF_WINDOW_MS, MAX_HALF_WINDOW_MS)
    } else {
        MIN_HALF_WINDOW_MS
    }
}

/// One point of a transition's window: the two ways' contributions and the
/// reference the sum is judged against.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Point {
    /// `2π·f/1000`: phase per ms of delay.
    w_ms: f64,
    /// log2 f, for the flatness line.
    x: f64,
    a: Cx,
    b: Cx,
    /// The reference (target) level, dB.
    r: f64,
}

impl Point {
    pub(crate) fn new(f: f64, a: Cx, b: Cx, r: f64) -> Point {
        Point {
            w_ms: std::f64::consts::TAU * f / 1000.0,
            x: f.log2(),
            a,
            b,
            r,
        }
    }
}

/// How a sum is judged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum CostKind {
    /// RMS deviation from the best straight line in log frequency (no target).
    Flatness,
    /// RMS deviation from the reference after removing the mean offset.
    Target,
    /// Phase disagreement of the two contributions through their overlap:
    /// `sqrt(Σ w·(1 - cos Δφ) / Σ w)` with `w = |a||b| / (|a|² + |b|²)`, so
    /// points where one way dominates do not count. Used by the crossover
    /// search's per-candidate timing (see `xover`) and by [`align`].
    Phase,
    /// As `Phase`, for a pair that is in quadrature by design (odd-order
    /// Butterworth): `sqrt(Σ w·cos² Δφ / Σ w)`, zero when the ways are 90°
    /// apart whichever leads. The polarity does not enter it.
    Quadrature,
}

/// A transition's window, ready to judge relative delays and flips.
pub(crate) struct PairCtx {
    pub(crate) points: Vec<Point>,
    pub(crate) kind: CostKind,
}

/// The result of a pair search.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PairFit {
    pub(crate) delta_ms: f64,
    pub(crate) flip: bool,
    pub(crate) cost: f64,
    /// The best cost of the other polarity.
    pub(crate) other: f64,
    /// Cost evaluations used.
    pub(crate) evals: u32,
}

impl PairFit {
    pub(crate) fn confidence(&self, share: f64) -> f64 {
        let c = if self.other.is_finite() && self.other > 1e-12 && self.cost.is_finite() {
            ((self.other - self.cost) / self.other).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let s = if share.is_finite() {
            share.clamp(0.0, 1.0)
        } else {
            0.0
        };
        c * s
    }
}

/// A cost that never compares as better than a real one.
const WORST: f64 = 1e12;

impl PairCtx {
    /// The cost of the high way delayed by `delta_ms` relative to the low way
    /// and flipped or not, dB; [`WORST`] for anything non-finite.
    pub(crate) fn cost(&self, delta_ms: f64, flip: bool) -> f64 {
        let n = self.points.len() as f64;
        if self.points.is_empty() || !delta_ms.is_finite() {
            return WORST;
        }
        let sign = if flip { -1.0 } else { 1.0 };
        if matches!(self.kind, CostKind::Phase | CostKind::Quadrature) {
            let (mut num, mut den) = (0.0, 0.0);
            for p in &self.points {
                let b = p.b.scale(sign).mul(Cx::polar(1.0, -p.w_ms * delta_ms));
                let (ma, mb) = (p.a.norm_sqr().sqrt(), b.norm_sqr().sqrt());
                let pw = ma * ma + mb * mb;
                if !(pw > 1e-30 && pw.is_finite()) {
                    continue;
                }
                let w = ma * mb / pw;
                // cos Δφ = Re(a·conj(b)) / (|a||b|).
                let re = p.a.re * b.re + p.a.im * b.im;
                let cos = if ma * mb > 1e-30 { re / (ma * mb) } else { 1.0 };
                let cos = cos.clamp(-1.0, 1.0);
                num += w * if self.kind == CostKind::Quadrature {
                    cos * cos
                } else {
                    1.0 - cos
                };
                den += w;
            }
            let c = if den > 1e-30 { (num / den).sqrt() } else { 0.0 };
            return if c.is_finite() { c.min(WORST) } else { WORST };
        }
        let mut r = Vec::with_capacity(self.points.len());
        for p in &self.points {
            let s =
                p.a.add(p.b.scale(sign).mul(Cx::polar(1.0, -p.w_ms * delta_ms)));
            r.push(s.db() - p.r);
        }
        let mean = r.iter().sum::<f64>() / n;
        let c = match self.kind {
            CostKind::Target => (r.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n).sqrt(),
            CostKind::Phase | CostKind::Quadrature | CostKind::Flatness => {
                let mx = self.points.iter().map(|p| p.x).sum::<f64>() / n;
                let mut sxx = 0.0;
                let mut sxy = 0.0;
                for (p, v) in self.points.iter().zip(&r) {
                    sxx += (p.x - mx) * (p.x - mx);
                    sxy += (p.x - mx) * (v - mean);
                }
                let slope = if sxx > 1e-18 { sxy / sxx } else { 0.0 };
                let ss: f64 = self
                    .points
                    .iter()
                    .zip(&r)
                    .map(|(p, v)| {
                        let e = v - mean - slope * (p.x - mx);
                        e * e
                    })
                    .sum();
                (ss / n).sqrt()
            }
        };
        if c.is_finite() { c.min(WORST) } else { WORST }
    }

    /// Search the relative delay in `start ± half` with `steps` coarse steps
    /// each side for each polarity, then refine the best by golden section
    /// (`refine` evaluations). `steps == 0` judges `start` alone, both
    /// polarities. Uses `2·(2·steps + 1) + refine` evaluations at most.
    pub(crate) fn search(&self, start: f64, half: f64, steps: usize, refine: usize) -> PairFit {
        let mut evals = 0u32;
        // The best delay and cost of each polarity: [unflipped, flipped].
        let mut per = [(start, f64::INFINITY), (start, f64::INFINITY)];
        let step = if steps == 0 || half.is_nan() || half <= 0.0 {
            0.0
        } else {
            half / steps as f64
        };
        let count = if step > 0.0 {
            steps.saturating_mul(2).saturating_add(1)
        } else {
            1
        };
        // A quadrature pair's cost does not depend on the polarity: it is
        // never flipped.
        let polarities: &[bool] = if self.kind == CostKind::Quadrature {
            &[false]
        } else {
            &[false, true]
        };
        for &flip in polarities {
            for k in 0..count {
                let d = if step > 0.0 {
                    start - half + step * k as f64
                } else {
                    start
                };
                let c = self.cost(d, flip);
                evals = evals.saturating_add(1);
                if let Some(b) = per.get_mut(usize::from(flip))
                    && c < b.1
                {
                    *b = (d, c);
                }
            }
        }
        // A flip must be clearly better (FLIP_MARGIN); a near tie keeps the
        // polarity there is.
        let [plain, flipped] = per;
        let flip = polarities.len() > 1 && flipped.1 < plain.1 * (1.0 - FLIP_MARGIN);
        let chosen = if flip { flipped } else { plain };
        let mut best = (chosen.0, flip, chosen.1);
        let best_other = [plain.1, flipped.1];
        // Golden-section refinement in the best coarse step's neighbourhood.
        if step > 0.0 && refine >= 2 {
            let inv_phi = (5f64.sqrt() - 1.0) / 2.0;
            let (mut lo, mut hi) = (best.0 - step, best.0 + step);
            let mut x1 = hi - inv_phi * (hi - lo);
            let mut x2 = lo + inv_phi * (hi - lo);
            let mut c1 = self.cost(x1, best.1);
            let mut c2 = self.cost(x2, best.1);
            evals = evals.saturating_add(2);
            for _ in 2..refine {
                if c1 <= c2 {
                    hi = x2;
                    x2 = x1;
                    c2 = c1;
                    x1 = hi - inv_phi * (hi - lo);
                    c1 = self.cost(x1, best.1);
                } else {
                    lo = x1;
                    x1 = x2;
                    c1 = c2;
                    x2 = lo + inv_phi * (hi - lo);
                    c2 = self.cost(x2, best.1);
                }
                evals = evals.saturating_add(1);
            }
            let (x, c) = if c1 <= c2 { (x1, c1) } else { (x2, c2) };
            if c < best.2 {
                best = (x, best.1, c);
            }
        }
        let other = if polarities.len() > 1 {
            best_other
                .get(usize::from(!best.1))
                .copied()
                .unwrap_or(WORST)
        } else {
            // No other polarity was judged: no margin to report.
            best.2
        };
        PairFit {
            delta_ms: best.0,
            flip: best.1,
            cost: best.2.min(WORST),
            other: other.min(WORST),
            evals,
        }
    }
}

/// A judged transition: `high`'s new delay minus `low`'s and the relative
/// flip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Edge {
    pub(crate) low: usize,
    pub(crate) high: usize,
    pub(crate) delta_ms: f64,
    pub(crate) flip: bool,
}

/// Compose the transitions' relative delays and flips over the group, then
/// normalise each joined set of ways so its smallest delay keeps its current
/// value (RC6), quantise to the device's grid and check the range.
///
/// The first way of each joined set keeps its polarity; a way joined to no
/// other keeps its delay. A transition that would close a loop is ignored
/// with a note.
pub(crate) fn compose(
    current: &[f64],
    edges: &[Edge],
    options: &AlignOptions,
    notes: &mut Vec<String>,
) -> Result<(Vec<f32>, Vec<bool>), MeasureError> {
    let n = current.len();
    let mut rel = vec![0f64; n];
    let mut flips = vec![false; n];
    let mut comp: Vec<Option<usize>> = vec![None; n];
    let mut used = vec![false; edges.len()];
    let mut components = 0usize;
    for root in 0..n {
        if comp.get(root).copied().flatten().is_some() {
            continue;
        }
        if let Some(c) = comp.get_mut(root) {
            *c = Some(components);
        }
        let mut queue = vec![root];
        let mut head = 0usize;
        while let Some(&u) = queue.get(head) {
            head = head.saturating_add(1);
            for (ei, e) in edges.iter().enumerate() {
                if used.get(ei).copied().unwrap_or(true) {
                    continue;
                }
                let (v, delta) = if e.low == u {
                    (e.high, e.delta_ms)
                } else if e.high == u {
                    (e.low, -e.delta_ms)
                } else {
                    continue;
                };
                if let Some(x) = used.get_mut(ei) {
                    *x = true;
                }
                if v >= n || comp.get(v).copied().flatten().is_some() {
                    notes.push(format!(
                        "the transition between ways {} and {} closes a loop and was not used",
                        e.low, e.high
                    ));
                    continue;
                }
                let ru = rel.get(u).copied().unwrap_or(0.0);
                let fu = flips.get(u).copied().unwrap_or(false);
                if let Some(r) = rel.get_mut(v) {
                    *r = ru + delta;
                }
                if let Some(f) = flips.get_mut(v) {
                    *f = fu ^ e.flip;
                }
                if let Some(c) = comp.get_mut(v) {
                    *c = Some(components);
                }
                queue.push(v);
            }
        }
        components = components.saturating_add(1);
    }
    let mut out = vec![0f32; n];
    for k in 0..components {
        let members: Vec<usize> = (0..n)
            .filter(|i| comp.get(*i).copied().flatten() == Some(k))
            .collect();
        let cur_min = members
            .iter()
            .filter_map(|i| current.get(*i).copied())
            .fold(f64::INFINITY, f64::min);
        let rel_min = members
            .iter()
            .filter_map(|i| rel.get(*i).copied())
            .fold(f64::INFINITY, f64::min);
        for i in members {
            let (Some(r), Some(slot)) = (rel.get(i).copied(), out.get_mut(i)) else {
                continue;
            };
            let d = r - rel_min + cur_min;
            let q = quantize_delay(d);
            if !(q.is_finite()
                && q >= options.min_delay_ms - DELAY_SLACK_MS
                && q <= options.max_delay_ms + DELAY_SLACK_MS)
            {
                return Err(MeasureError::Invalid(format!(
                    "the alignment needs a delay of {d:.4} ms on way {i}, outside {} to {} ms",
                    options.min_delay_ms, options.max_delay_ms
                )));
            }
            // Within the options' range (at most MAX_DELAY_BOUND_MS) by the
            // check above, so the narrowing is exact to f32's precision.
            *slot = q as f32;
        }
    }
    Ok((out, flips))
}

/// A delay on the device's 0.0001 ms grid (halves away from zero, as
/// `dspi-core`'s `quantize_delay`), as the f32 the setter sends.
pub(crate) fn quantize_delay(ms: f64) -> f64 {
    let v = (ms / DELAY_STEP_MS).round() * DELAY_STEP_MS;
    let v = f64::from(v as f32);
    if v == 0.0 { 0.0 } else { v }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
pub(crate) mod tests_wp_p5xo;
