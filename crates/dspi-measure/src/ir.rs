//! The timing core: deconvolution, cycle identity, the clock-drift fit and its
//! removal, coherent averaging, the noise floor, windows, and the timing
//! quality gate (WP P5TIME, PLAN-P5 section 1.1 steps 2 and 5 to 9).
//!
//! A walked run plays the generator's log sweep on each output in turn
//! ([`crate::stimulus::WalkSchedule`]); the host's microphone records it with
//! its own clock. Everything here works on that one capture:
//!
//! 1. **Frame continuity** ([`FrameContinuity`]): the client pushes blocks with
//!    the index of their first frame; any gap or repeat is a fault and the
//!    session aborts the run (S5). Pure and O(1) per block, so it runs in the
//!    push path before anything is stored (RC8).
//! 2. **Cycle identity** (RC5b): the first cycle must arrive in the window the
//!    START implies (the capture frame at which the START was acknowledged,
//!    `start_early_ms` before to `max_latency_ms` after), no sweep may be heard
//!    before it, every later cycle must be heard where the schedule puts it,
//!    and every polled status ([`StatusObservation`]) must agree with the cycle
//!    the capture is in. Any mismatch is [`TimingError::CycleIdentity`]: a
//!    missed first cycle can never shift every response onto the wrong output.
//!    [`crate::stimulus::WalkSchedule::status_consistent`] is the same check for
//!    the session's polls during the run.
//! 3. **The timing model** (RC5a): a coarse matched filter (deconvolution and
//!    the impulse response's peak) gives one arrival per cycle, refined against
//!    the same output's first cycle by an upsampled cross-correlation. The fit
//!    is **one common slope** (capture frames per device sample: the clock
//!    ratio) and **one intercept per output**, least squares over every cycle;
//!    it needs at least two passes.
//! 4. **Drift removal in the frequency domain** (S-e): each cycle is cut at its
//!    fitted position, deconvolved by regularised spectral division by the
//!    modelled sweep, and the ratio's error `e` is removed as the phase term of
//!    the delay `e·t(f)` the log sweep gives frequency `f` (`t(f)` from
//!    [`crate::stimulus::LogSweep::time_of_freq`]). No resampler; only per-cycle
//!    data is kept.
//! 5. **Coherent averaging** of each output's repeats on the common grid; every
//!    output's impulse response is referenced to the run's one timing origin
//!    (the earliest output's intercept), so relative delays and phases between
//!    outputs of one run are kept. Nothing compares arrivals across runs.
//! 6. **The timing quality gate** (RC5d): reliable only with every cycle's fit
//!    residual at most [`MAX_RESIDUAL_US`], the repeats of every output agreeing
//!    in phase within [`MAX_PHASE_SPREAD_DEG`] wherever they are loud enough to
//!    tell, a noise floor from the pre-roll, and at least
//!    [`AnalysisOptions::min_passes_reliable`] passes. Otherwise the result keeps
//!    its magnitudes and says why it is "timing unreliable".
//!
//! Analysis runs in slices ([`WalkedAnalyser::step`], one cycle's FFT each), so
//! the session can schedule it in the gaps or after the generator stopped; it
//! never runs between a push's safety check and the next block.
//!
//! Units: positions are capture frames (fractional), counted from the first
//! sample of the capture slice handed in; a [`StatusObservation`] and the START
//! frame carry the client's absolute frame indices.

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::stimulus::{GeneratorStatus, WalkSchedule};
use crate::types::{LogGrid, MeasureError};

/// A complex value (`rustfft`'s).
pub type C64 = Complex<f64>;

/// The most capture frames one run may hold: 8 Mi frames, 32 MiB of `f32`
/// (PLAN-P5 section 1.1 step 10).
pub const MAX_CAPTURE_SAMPLES: usize = 8 * 1024 * 1024;
/// The longest FFT the analysis computes (a 15 s sweep at 96 kHz with its gap
/// fits).
pub const MAX_FFT_LEN: usize = 1 << 21;
/// The frequency-dependent window for alignment, cycles (review S-c).
pub const ALIGNMENT_WINDOW_CYCLES: f64 = 5.0;
/// The frequency-dependent window for the EQ fit, cycles (review S-c).
pub const FIT_WINDOW_CYCLES: f64 = 15.0;
/// The timing quality gate: the largest fit residual of any cycle, µs.
pub const MAX_RESIDUAL_US: f64 = 10.0;
/// The timing quality gate: the largest phase disagreement between repeats, °.
pub const MAX_PHASE_SPREAD_DEG: f64 = 10.0;
/// A cycle is "heard" when its impulse response's peak is this many times the
/// median power of the response (30 dB).
const FOUND_RATIO: f64 = 1000.0;
/// A point is loud enough for the phase check at this SNR per repeat, dB. Above
/// the 20 dB of the response's own trust mask so that noise alone cannot fail
/// the gate (at 30 dB the noise moves a phase by about 1.3° rms).
const PHASE_CHECK_SNR_DB: f64 = 30.0;
/// The impulse-response excerpt around each cycle's peak used to refine its
/// arrival, ms either side.
const SNIPPET_MS: f64 = 10.0;
/// Upsampling of the refining cross-correlation.
const XCORR_UPSAMPLE: usize = 16;
/// Half-width of the band-limited interpolator of [`ImpulseResponse::peak_index`].
const INTERP_HALF: i64 = 16;
/// The most frequencies one windowed response is evaluated at.
const MAX_WINDOW_FREQS: usize = 4 * crate::types::MAX_GRID_POINTS;
/// The longest frequency-dependent window, samples (bounds its cost).
const MAX_FDW_LEN: usize = 1 << 17;

// ---------------------------------------------------------------------------
// Frame continuity (S5, RC5c)
// ---------------------------------------------------------------------------

/// A fault in the capture's frame indices. Every one aborts the run (S5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FrameFault {
    /// Frames are missing: a block started after the frame expected.
    #[error("capture gap: frame {got} where {expected} was expected")]
    Gap {
        /// The frame the next block had to start at.
        expected: u64,
        /// The frame it started at.
        got: u64,
    },
    /// Frames came twice: a block started before the frame expected.
    #[error("capture repeat: frame {got} where {expected} was expected")]
    Repeat {
        /// The frame the next block had to start at.
        expected: u64,
        /// The frame it started at.
        got: u64,
    },
    /// The capture passed [`MAX_CAPTURE_SAMPLES`].
    #[error("capture of {frames} frames is past the bound")]
    TooLong {
        /// Frames the capture would hold.
        frames: u64,
    },
}

/// Checks that pushed capture blocks follow each other frame for frame
/// (PLAN-P5 section 1.1 step 2: any gap or repeat aborts the run).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameContinuity {
    next: Option<u64>,
    frames: u64,
}

impl FrameContinuity {
    /// A tracker that accepts any first frame.
    pub fn new() -> FrameContinuity {
        FrameContinuity::default()
    }

    /// Accounts for a block of `len` frames starting at `first_frame`. On a
    /// fault nothing is accounted; the session discards the run. An empty block
    /// is ignored.
    pub fn push(&mut self, first_frame: u64, len: usize) -> Result<(), FrameFault> {
        if len == 0 {
            return Ok(());
        }
        if let Some(expected) = self.next {
            if first_frame > expected {
                return Err(FrameFault::Gap {
                    expected,
                    got: first_frame,
                });
            }
            if first_frame < expected {
                return Err(FrameFault::Repeat {
                    expected,
                    got: first_frame,
                });
            }
        }
        let len = len as u64;
        let frames = self.frames.saturating_add(len);
        if frames > MAX_CAPTURE_SAMPLES as u64 {
            return Err(FrameFault::TooLong { frames });
        }
        self.frames = frames;
        self.next = Some(first_frame.saturating_add(len));
        Ok(())
    }

    /// Frames accepted so far.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// The frame the next block must start at (none before the first block).
    pub fn next_frame(&self) -> Option<u64> {
        self.next
    }
}

// ---------------------------------------------------------------------------
// Inputs and results
// ---------------------------------------------------------------------------

/// Why a walked run could not be analysed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TimingError {
    /// The capture does not line up with the run the device played (RC5b): the
    /// session aborts and discards the run.
    #[error("cycle identity: {0}")]
    CycleIdentity(String),
    /// Bad input or no signal.
    #[error(transparent)]
    Measure(#[from] MeasureError),
}

/// A `SIGGEN_GET_STATUS` reply and the capture frame the client had reached
/// when it arrived. Only polls answered after the START belong here (before
/// it the status still shows the previous run).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusObservation {
    /// The capture's frame index when the reply arrived.
    pub capture_frame: u64,
    /// What the device said.
    pub status: GeneratorStatus,
}

/// The capture of one walked run, as the session holds it.
#[derive(Debug, Clone, Copy)]
pub struct WalkedCapture<'a> {
    /// Mono samples, full scale ±1, contiguous (the session checked the frame
    /// indices with [`FrameContinuity`]). Starts with the pre-roll.
    pub samples: &'a [f32],
    /// The frame index of `samples[0]`.
    pub first_frame: u64,
    /// The capture's sample rate, Hz (8 kHz to 192 kHz).
    pub rate_hz: u32,
    /// The capture frame at which the device acknowledged `SIGGEN_CTL_START`.
    pub start_frame: u64,
    /// The status polls of the run.
    pub status: &'a [StatusObservation],
    /// No more samples will come: data past the end reads as silence.
    pub complete: bool,
}

/// Tuning of the analysis; the defaults are the plan's.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisOptions {
    /// How long before the START frame the first cycle may be heard (the
    /// capture's input latency), ms.
    pub start_early_ms: f64,
    /// How long after the START frame the first cycle must be heard, ms.
    pub max_latency_ms: f64,
    /// Each later cycle is searched this far either side of where the schedule
    /// puts it, and impulse responses keep this much before their timing
    /// origin, ms. Outputs of one run must arrive within this of each other.
    /// Limited to a third of the gap.
    pub search_ms: f64,
    /// The largest clock difference between the DSPi and the microphone, ppm.
    pub max_ratio_ppm: f64,
    /// How far a status poll may be from the capture's time, ms.
    pub status_slack_ms: f64,
    /// The timing gate's residual bound, µs ([`MAX_RESIDUAL_US`]).
    pub max_residual_us: f64,
    /// The timing gate's phase bound, ° ([`MAX_PHASE_SPREAD_DEG`]).
    pub max_phase_spread_deg: f64,
    /// Where the repeats' phases are compared (the transitions), Hz. Empty: 100
    /// Hz to 10 kHz.
    pub phase_check_bands: Vec<(f64, f64)>,
    /// The shortest pre-roll a noise floor is taken from, ms.
    pub min_preroll_ms: f64,
    /// Passes a run needs to be "timing reliable" (at least 2). Default 3: with
    /// two passes a capture slip between them is indistinguishable from a clock
    /// ratio (it fits with no residual), so only the summed check would catch
    /// it.
    pub min_passes_reliable: u16,
}

impl Default for AnalysisOptions {
    fn default() -> AnalysisOptions {
        AnalysisOptions {
            start_early_ms: 200.0,
            max_latency_ms: 500.0,
            search_ms: 100.0,
            max_ratio_ppm: 1000.0,
            status_slack_ms: 250.0,
            max_residual_us: MAX_RESIDUAL_US,
            max_phase_spread_deg: MAX_PHASE_SPREAD_DEG,
            phase_check_bands: Vec::new(),
            min_preroll_ms: 250.0,
            min_passes_reliable: 3,
        }
    }
}

/// An impulse response at the capture's rate.
#[derive(Debug, Clone, PartialEq)]
pub struct ImpulseResponse {
    /// Sample rate, Hz.
    pub rate_hz: u32,
    /// The response; `samples[zero_index]` is the timing origin.
    pub samples: Vec<f64>,
    /// The index of time zero (the run's timing reference).
    pub zero_index: usize,
}

/// The clock fit of a run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockFit {
    /// Capture frames per device sample.
    pub frames_per_device_sample: f64,
    /// The microphone's clock against the DSPi's, ppm (positive: the
    /// microphone counts more frames than nominal).
    pub ratio_ppm: f64,
    /// False for a single pass: the nominal ratio was assumed.
    pub fitted: bool,
}

/// The timing quality gate's verdict (RC5d).
#[derive(Debug, Clone, PartialEq)]
pub struct TimingQuality {
    /// Delay, polarity and crossover proposals may use this run.
    pub reliable: bool,
    /// The largest fit residual of any cycle, µs (0 without a fit).
    pub max_residual_us: f64,
    /// The residual bound this run was held to, µs: the options' bound,
    /// divided by the layout's [`slip_sensitivity`] where that is above 1, so
    /// that a single capture slip the gate lets through moves no delay by more
    /// than the options' bound.
    pub residual_bound_us: f64,
    /// The largest phase disagreement between repeats where checked, °.
    pub max_phase_spread_deg: f64,
    /// Passes in the run.
    pub passes: u16,
    /// Why not reliable, in words (empty when reliable).
    pub reasons: Vec<String>,
}

/// One cycle's place in the fit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CycleTiming {
    /// The cycle's index in the run.
    pub index: u32,
    /// Its output.
    pub output: u8,
    /// Where it was heard: the capture frame (absolute index) of its impulse
    /// response's peak.
    pub arrival_frame: f64,
    /// Its distance from the fit, µs.
    pub residual_us: f64,
}

/// One output's result.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputIr {
    /// The output.
    pub output: u8,
    /// The coherent average of its repeats, drift removed, referenced to the
    /// run's timing origin: `search_ms` before it, the gap's rest after.
    pub ir: ImpulseResponse,
    /// The response's peak, a fractional index into `ir.samples`.
    pub peak_index: f64,
    /// The peak's arrival after the run's earliest output, ms.
    pub arrival_ms: f64,
    /// Repeats averaged.
    pub repeats: u16,
    /// The noise floor on [`LogGrid::STANDARD`] (absent without a pre-roll), as
    /// the noise power per sample of the averaged response, dB: the DFT of the
    /// response under a window `w` holds noise of power `10^(noise_db/10) ·
    /// Σw²` (see [`WindowedResponse::sum_w2`]); the signal is `|H|²` of the same
    /// DFT.
    pub noise_db: Option<Vec<f64>>,
    /// The repeats' magnitude spread (largest minus smallest, 1/96 octave
    /// bands) on [`LogGrid::STANDARD`], dB; zeros for one repeat.
    pub spread_db: Vec<f64>,
    /// The largest phase disagreement between its repeats where checked, °.
    pub phase_spread_deg: f64,
    /// Points the phase check could use.
    pub phase_points_checked: usize,
}

/// The result of a walked run.
#[derive(Debug, Clone, PartialEq)]
pub struct WalkedAnalysis {
    /// The capture's rate, Hz.
    pub capture_rate_hz: u32,
    /// The clock fit.
    pub clock: ClockFit,
    /// The timing quality gate.
    pub quality: TimingQuality,
    /// Every cycle, in order.
    pub cycles: Vec<CycleTiming>,
    /// Every output, in walk order.
    pub outputs: Vec<OutputIr>,
    /// The deconvolution's trusted band, Hz.
    pub band_hz: (f64, f64),
}

/// What one analysis slice did.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// More slices to go.
    Progress {
        /// Slices done.
        done: usize,
        /// Slices in all.
        total: usize,
    },
    /// The next slice needs capture that has not arrived yet.
    NeedCapture,
    /// Finished.
    Done(Box<WalkedAnalysis>),
}

// ---------------------------------------------------------------------------
// Small numeric helpers (no operator on Complex: the crate's lints)
// ---------------------------------------------------------------------------

fn cmul(a: C64, b: C64) -> C64 {
    C64::new(a.re * b.re - a.im * b.im, a.re * b.im + a.im * b.re)
}

fn cadd(a: C64, b: C64) -> C64 {
    C64::new(a.re + b.re, a.im + b.im)
}

fn czero() -> C64 {
    C64::new(0.0, 0.0)
}

/// The smallest `2^a 3^b 5^c` at or above `min`, at most [`MAX_FFT_LEN`].
fn fft_len(min: usize) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut p2: usize = 1;
    while p2 <= MAX_FFT_LEN {
        let mut p3 = p2;
        while p3 <= MAX_FFT_LEN {
            let mut p5 = p3;
            while p5 <= MAX_FFT_LEN {
                if p5 >= min && best.is_none_or(|b| p5 < b) {
                    best = Some(p5);
                }
                p5 = match p5.checked_mul(5) {
                    Some(v) => v,
                    None => break,
                };
            }
            p3 = match p3.checked_mul(3) {
                Some(v) => v,
                None => break,
            };
        }
        p2 = match p2.checked_mul(2) {
            Some(v) => v,
            None => break,
        };
    }
    best
}

/// The signed frequency of bin `k` of an `n`-point FFT, Hz.
fn bin_freq(k: usize, n: usize, fs: f64) -> f64 {
    let kk = if k.saturating_mul(2) <= n {
        k as f64
    } else {
        k as f64 - n as f64
    };
    kk * fs / n as f64
}

fn parabolic(ym: f64, y0: f64, yp: f64) -> f64 {
    let d = ym - 2.0 * y0 + yp;
    if d.abs() < 1e-300 || !d.is_finite() {
        return 0.0;
    }
    (0.5 * (ym - yp) / d).clamp(-0.5, 0.5)
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mid = v.len() / 2;
    let (_, m, _) = v.select_nth_unstable_by(mid, f64::total_cmp);
    *m
}

/// Circular read.
fn circ(h: &[f64], i: i64) -> f64 {
    let n = h.len() as i64;
    if n == 0 {
        return 0.0;
    }
    h.get(i.rem_euclid(n) as usize).copied().unwrap_or(0.0)
}

struct Fourier {
    n: usize,
    fwd: Arc<dyn Fft<f64>>,
    inv: Arc<dyn Fft<f64>>,
    scratch: Vec<C64>,
}

impl Fourier {
    fn new(n: usize) -> Fourier {
        let mut planner = FftPlanner::new();
        let fwd = planner.plan_fft_forward(n);
        let inv = planner.plan_fft_inverse(n);
        let len = fwd
            .get_inplace_scratch_len()
            .max(inv.get_inplace_scratch_len());
        Fourier {
            n,
            fwd,
            inv,
            scratch: vec![czero(); len],
        }
    }

    /// In place; `buf` must hold exactly `n` values (checked).
    fn forward(&mut self, buf: &mut [C64]) {
        if buf.len() == self.n {
            self.fwd.process_with_scratch(buf, &mut self.scratch);
        }
    }

    /// In place and scaled by `1/n`.
    fn inverse(&mut self, buf: &mut [C64]) {
        if buf.len() == self.n {
            self.inv.process_with_scratch(buf, &mut self.scratch);
            let s = 1.0 / self.n as f64;
            buf.iter_mut().for_each(|c| *c = c.scale(s));
        }
    }
}

/// `conj(S) / (|S|² + λ)` of a reference: λ is 10⁻⁹ of the largest `|S|²`
/// inside the band and the largest outside it (Kirkeby's regularisation).
fn inverse_filter(
    four: &mut Fourier,
    reference: &[f64],
    fs: f64,
    band: (f64, f64),
) -> Result<Vec<C64>, MeasureError> {
    let n = four.n;
    if reference.len() > n {
        return Err(MeasureError::Invalid(
            "reference longer than the FFT".into(),
        ));
    }
    let mut s = vec![czero(); n];
    for (d, &v) in s.iter_mut().zip(reference) {
        d.re = v;
    }
    four.forward(&mut s);
    let maxp = s.iter().map(|c| c.norm_sqr()).fold(0.0, f64::max);
    if !(maxp > 0.0 && maxp.is_finite()) {
        return Err(MeasureError::NoSignal(
            "the reference sweep is silent".into(),
        ));
    }
    for (k, c) in s.iter_mut().enumerate() {
        let f = bin_freq(k, n, fs).abs();
        let lam = if f >= band.0 && f <= band.1 {
            1e-9 * maxp
        } else {
            maxp
        };
        let p = c.norm_sqr();
        *c = c.conj().unscale(p + lam);
    }
    Ok(s)
}

/// Deconvolves a recording of a known stimulus: the recording's spectrum
/// divided by the stimulus's, regularised outside `band_hz`. The result is a
/// circular impulse response of the FFT's length (at least the recording's),
/// time zero at index 0 and negative times at the end. For an exact result the
/// recording must hold the whole response (`stimulus * h`).
///
/// Errors: [`MeasureError::Invalid`] for an empty, non-finite or too long input
/// or a bad rate or band; [`MeasureError::NoSignal`] for a silent stimulus.
pub fn deconvolve(
    recording: &[f64],
    stimulus: &[f64],
    rate_hz: f64,
    band_hz: (f64, f64),
) -> Result<Vec<f64>, MeasureError> {
    let ok = !recording.is_empty()
        && !stimulus.is_empty()
        && rate_hz.is_finite()
        && rate_hz > 0.0
        && band_hz.0.is_finite()
        && band_hz.1.is_finite()
        && band_hz.0 < band_hz.1
        && recording.iter().chain(stimulus).all(|v| v.is_finite());
    if !ok {
        return Err(MeasureError::Invalid("bad deconvolution input".into()));
    }
    let n = fft_len(recording.len().max(stimulus.len()))
        .ok_or_else(|| MeasureError::Invalid("recording too long".into()))?;
    let mut four = Fourier::new(n);
    let g = inverse_filter(&mut four, stimulus, rate_hz, band_hz)?;
    let mut y = vec![czero(); n];
    for (d, &v) in y.iter_mut().zip(recording) {
        d.re = v;
    }
    four.forward(&mut y);
    for (c, &gk) in y.iter_mut().zip(&g) {
        *c = cmul(*c, gk);
    }
    four.inverse(&mut y);
    Ok(y.iter().map(|c| c.re).collect())
}

// ---------------------------------------------------------------------------
// Band-limited interpolation, peaks, cross-correlation
// ---------------------------------------------------------------------------

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        let px = std::f64::consts::PI * x;
        px.sin() / px
    }
}

/// A Blackman-windowed sinc reconstruction of `h` at fractional `t`.
fn interp_at(h: &[f64], t: f64) -> f64 {
    let i0 = t.floor() as i64;
    let half = INTERP_HALF as f64;
    let mut acc = 0.0;
    for j in i0.saturating_sub(INTERP_HALF - 1)..=i0.saturating_add(INTERP_HALF) {
        let x = t - j as f64;
        if x.abs() >= half {
            continue;
        }
        let r = (x / half + 1.0) / 2.0; // 0..1 across the window
        let w = 0.42 - 0.5 * (std::f64::consts::TAU * r).cos()
            + 0.08 * (2.0 * std::f64::consts::TAU * r).cos();
        let v = if j < 0 {
            0.0
        } else {
            h.get(j as usize).copied().unwrap_or(0.0)
        };
        acc += v * sinc(x) * w;
    }
    acc
}

/// The fractional position of the largest `|h|`, by band-limited
/// interpolation (1/32 sample steps, then a parabola).
fn bl_peak(h: &[f64]) -> Option<f64> {
    let (idx, _) = h
        .iter()
        .enumerate()
        .map(|(i, v)| (i, v.abs()))
        .filter(|(_, v)| v.is_finite())
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    let base = idx as f64;
    let steps: i64 = 32;
    let at = |s: i64| interp_at(h, base + s as f64 / steps as f64).abs();
    let mut best = (0i64, at(0));
    for s in steps.saturating_neg()..=steps {
        let v = at(s);
        if v > best.1 {
            best = (s, v);
        }
    }
    let (s, y0) = best;
    let frac = parabolic(at(s.saturating_sub(1)), y0, at(s.saturating_add(1)));
    Some(base + (s as f64 + frac) / steps as f64)
}

/// How much later `b` is than `a`, samples (upsampled cross-correlation),
/// within `max_lag`; `None` without a clear peak.
fn xcorr_lag(a: &[f64], b: &[f64], max_lag: f64) -> Option<f64> {
    let m = fft_len(a.len().saturating_add(b.len()))?.next_power_of_two();
    let mu = m.checked_mul(XCORR_UPSAMPLE)?;
    if mu > MAX_FFT_LEN {
        return None;
    }
    let mut four = Fourier::new(m);
    let mut fa = vec![czero(); m];
    let mut fb = vec![czero(); m];
    for (d, &v) in fa.iter_mut().zip(a) {
        d.re = v;
    }
    for (d, &v) in fb.iter_mut().zip(b) {
        d.re = v;
    }
    four.forward(&mut fa);
    four.forward(&mut fb);
    // Zero-padded spectrum of conj(A)·B: positive bins first, negative last.
    let mut up = vec![czero(); mu];
    let half = m / 2;
    for (k, (x, y)) in fa.iter().zip(&fb).enumerate() {
        let v = cmul(x.conj(), *y);
        let dst = if k < half {
            Some(k)
        } else if k > half {
            mu.checked_sub(m.saturating_sub(k))
        } else {
            None
        };
        if let Some(slot) = dst.and_then(|d| up.get_mut(d)) {
            *slot = v;
        }
    }
    let mut big = Fourier::new(mu);
    big.inverse(&mut up);
    let r: Vec<f64> = up.iter().map(|c| c.re).collect();
    let limit = (max_lag * XCORR_UPSAMPLE as f64).floor() as i64;
    let mut best: Option<(i64, f64)> = None;
    for l in limit.saturating_neg()..=limit {
        let v = circ(&r, l);
        if best.is_none_or(|(_, bv)| v > bv) {
            best = Some((l, v));
        }
    }
    let (l, v) = best?;
    if v.is_nan() || v <= 0.0 {
        return None;
    }
    let frac = parabolic(
        circ(&r, l.saturating_sub(1)),
        v,
        circ(&r, l.saturating_add(1)),
    );
    Some((l as f64 + frac) / XCORR_UPSAMPLE as f64)
}

impl ImpulseResponse {
    /// The fractional index of the largest `|h|` (band-limited
    /// interpolation); `None` for an empty or silent response.
    pub fn peak_index(&self) -> Option<f64> {
        let p = bl_peak(&self.samples)?;
        let any = self.samples.iter().any(|v| *v != 0.0);
        (any && p.is_finite()).then_some(p)
    }
}

// ---------------------------------------------------------------------------
// Windows (fixed gate, frequency dependent)
// ---------------------------------------------------------------------------

/// A response evaluated at given frequencies from a windowed impulse response.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowedResponse {
    /// The frequencies, Hz.
    pub freqs: Vec<f64>,
    /// The DFT of the windowed response at each frequency, phase relative to
    /// the response's `zero_index`.
    pub values: Vec<C64>,
    /// The window's `Σw²` at each frequency: the noise of a value is
    /// `10^(noise_db/10) · sum_w2` ([`OutputIr::noise_db`]).
    pub sum_w2: Vec<f64>,
}

fn half_hann_rise(x: f64) -> f64 {
    // 0 at x = 0, 1 at x = 1.
    let x = x.clamp(0.0, 1.0);
    0.5 * (1.0 - (std::f64::consts::PI * x).cos())
}

fn check_ir(ir: &ImpulseResponse, centre: f64) -> Result<(), MeasureError> {
    let ok = ir.rate_hz > 0
        && !ir.samples.is_empty()
        && ir.samples.len() <= MAX_FFT_LEN
        && ir.samples.iter().all(|v| v.is_finite())
        && centre.is_finite()
        && centre >= 0.0
        && centre < ir.samples.len() as f64;
    if ok {
        Ok(())
    } else {
        Err(MeasureError::Invalid(
            "bad impulse response or window centre".into(),
        ))
    }
}

/// A fixed gate around `centre` (an index, e.g. [`ImpulseResponse::peak_index`]):
/// `left_ms` before and `right_ms` after, the outer `taper_ms` of each side a
/// half-Hann taper, zero outside.
///
/// Errors: [`MeasureError::Invalid`] for a bad response, centre or lengths.
pub fn fixed_gate(
    ir: &ImpulseResponse,
    centre: f64,
    left_ms: f64,
    right_ms: f64,
    taper_ms: f64,
) -> Result<ImpulseResponse, MeasureError> {
    check_ir(ir, centre)?;
    let ok = [left_ms, right_ms, taper_ms]
        .iter()
        .all(|v| v.is_finite() && *v >= 0.0 && *v <= 60_000.0);
    if !ok {
        return Err(MeasureError::Invalid("bad gate lengths".into()));
    }
    let fs = f64::from(ir.rate_hz);
    let (l, r, t) = (
        left_ms * fs / 1000.0,
        right_ms * fs / 1000.0,
        taper_ms * fs / 1000.0,
    );
    let samples = ir
        .samples
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = i as f64 - centre;
            let w = if x < -l || x > r {
                0.0
            } else if x < 0.0 {
                let tl = t.min(l);
                if tl > 0.0 {
                    half_hann_rise((x + l) / tl)
                } else {
                    1.0
                }
            } else {
                let tr = t.min(r);
                if tr > 0.0 {
                    half_hann_rise((r - x) / tr)
                } else {
                    1.0
                }
            };
            v * w
        })
        .collect();
    Ok(ImpulseResponse {
        rate_hz: ir.rate_hz,
        samples,
        zero_index: ir.zero_index,
    })
}

/// The plain DFT of a whole (already gated) response at `freqs`, phase
/// relative to its `zero_index`; `sum_w2` is the response's length.
///
/// Errors: [`MeasureError::Invalid`] for a bad response or frequencies.
pub fn response_at(ir: &ImpulseResponse, freqs: &[f64]) -> Result<WindowedResponse, MeasureError> {
    check_ir(ir, 0.0)?;
    check_freqs(freqs, ir.rate_hz)?;
    if ir.samples.len().saturating_mul(freqs.len()) > MAX_FDW_LEN.saturating_mul(MAX_WINDOW_FREQS) {
        return Err(MeasureError::Invalid(
            "response too long to evaluate".into(),
        ));
    }
    let fs = f64::from(ir.rate_hz);
    let values = freqs
        .iter()
        .map(|&f| {
            dft_at(
                &ir.samples,
                0,
                ir.samples.len(),
                ir.zero_index,
                f / fs,
                |_| 1.0,
            )
        })
        .collect();
    Ok(WindowedResponse {
        freqs: freqs.to_vec(),
        values,
        sum_w2: vec![ir.samples.len() as f64; freqs.len()],
    })
}

fn check_freqs(freqs: &[f64], rate: u32) -> Result<(), MeasureError> {
    let nyq = f64::from(rate) / 2.0;
    let ok = !freqs.is_empty()
        && freqs.len() <= MAX_WINDOW_FREQS
        && freqs.iter().all(|f| f.is_finite() && *f > 0.0 && *f < nyq);
    if ok {
        Ok(())
    } else {
        Err(MeasureError::Invalid("bad frequencies".into()))
    }
}

/// `Σ w(n) h(n) e^{-j2π f (n - zero)}` over `n` in `[from, to)`, `f` in cycles
/// per sample.
fn dft_at(h: &[f64], from: usize, to: usize, zero: usize, f: f64, w: impl Fn(usize) -> f64) -> C64 {
    let omega = -std::f64::consts::TAU * f;
    let rot = C64::from_polar(1.0, omega);
    let mut ph = C64::from_polar(1.0, omega * (from as f64 - zero as f64));
    let mut acc = czero();
    for (i, &v) in h.iter().enumerate().take(to).skip(from) {
        let x = v * w(i);
        acc = cadd(acc, ph.scale(x));
        ph = cmul(ph, rot);
    }
    acc
}

/// The frequency-dependent window (FDW): at each frequency a window
/// `cycles / f` long around `centre`, a quarter of it before (half-Hann rise)
/// and three quarters after (half-Hann fall), at most the response and at least
/// 16 samples long. [`ALIGNMENT_WINDOW_CYCLES`] and [`FIT_WINDOW_CYCLES`] are
/// the plan's two uses. Phase is relative to the response's `zero_index`.
///
/// Errors: [`MeasureError::Invalid`] for a bad response, centre, frequencies or
/// a cycle count outside 1 to 1000.
pub fn fdw_response(
    ir: &ImpulseResponse,
    centre: f64,
    freqs: &[f64],
    cycles: f64,
) -> Result<WindowedResponse, MeasureError> {
    check_ir(ir, centre)?;
    check_freqs(freqs, ir.rate_hz)?;
    if !(cycles.is_finite() && (1.0..=1000.0).contains(&cycles)) {
        return Err(MeasureError::Invalid(format!("{cycles} window cycles")));
    }
    let fs = f64::from(ir.rate_hz);
    let len = ir.samples.len();
    let max_w = len.min(MAX_FDW_LEN) as f64;
    let mut values = Vec::with_capacity(freqs.len());
    let mut sum_w2 = Vec::with_capacity(freqs.len());
    for &f in freqs {
        let lw = (cycles * fs / f).clamp(16.0, max_w);
        let left = lw / 4.0;
        let right = lw - left;
        let from = (centre - left).floor().max(0.0) as usize;
        let to = ((centre + right).ceil() as usize).min(len);
        let w = |i: usize| -> f64 {
            let x = i as f64 - centre;
            if x < -left || x > right {
                0.0
            } else if x < 0.0 {
                half_hann_rise((x + left) / left)
            } else {
                half_hann_rise((right - x) / right)
            }
        };
        let s2: f64 = (from..to).map(|i| w(i) * w(i)).sum();
        values.push(dft_at(&ir.samples, from, to, ir.zero_index, f / fs, w));
        sum_w2.push(s2);
    }
    Ok(WindowedResponse {
        freqs: freqs.to_vec(),
        values,
        sum_w2,
    })
}

// ---------------------------------------------------------------------------
// The walked-run analyser
// ---------------------------------------------------------------------------

/// The common-slope, per-output-intercept fit.
#[derive(Debug, Clone, PartialEq)]
struct Fit {
    slope: f64,
    intercepts: Vec<Option<f64>>,
}

/// Least squares of `y = a[o] + b·n` with one `b`: `None` unless some output
/// has two points at different `n`.
fn fit_common_slope(points: &[(usize, f64, f64)], outputs: usize) -> Option<Fit> {
    let mut sum_n = vec![0.0; outputs];
    let mut sum_y = vec![0.0; outputs];
    let mut count = vec![0.0; outputs];
    for &(o, n, y) in points {
        if let (Some(sn), Some(sy), Some(c)) =
            (sum_n.get_mut(o), sum_y.get_mut(o), count.get_mut(o))
        {
            *sn += n;
            *sy += y;
            *c += 1.0;
        }
    }
    let mean = |s: &[f64], o: usize| -> f64 {
        let c = count.get(o).copied().unwrap_or(0.0);
        if c > 0.0 {
            s.get(o).copied().unwrap_or(0.0) / c
        } else {
            0.0
        }
    };
    let (mut sxx, mut sxy) = (0.0, 0.0);
    for &(o, n, y) in points {
        let dn = n - mean(&sum_n, o);
        sxx += dn * dn;
        sxy += dn * (y - mean(&sum_y, o));
    }
    if sxx.is_nan() || sxx <= 0.0 {
        return None;
    }
    let slope = sxy / sxx;
    let intercepts = (0..outputs)
        .map(|o| {
            (count.get(o).copied().unwrap_or(0.0) > 0.0)
                .then(|| mean(&sum_y, o) - slope * mean(&sum_n, o))
        })
        .collect();
    slope.is_finite().then_some(Fit { slope, intercepts })
}

#[derive(Debug, Clone)]
struct Found {
    /// Absolute capture position (relative to the slice's first sample) of
    /// the impulse response's peak.
    coarse: f64,
    snippet_start: i64,
    snippet: Vec<f64>,
}

#[derive(Debug, Clone, Default)]
struct RepeatStats {
    power: Vec<f64>,
    z: Vec<C64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Search,
    Pass1(usize),
    Fit,
    Pass2(usize),
    Finish,
    Done,
}

/// Analyses one walked run in slices: [`WalkedAnalyser::step`] does about one
/// cycle's FFT per call. [`analyse_walked`] runs every slice at once.
pub struct WalkedAnalyser {
    schedule: WalkSchedule,
    options: AnalysisOptions,
    rate: f64,
    nominal: f64,
    pre: usize,
    tail: usize,
    cycle_cap: usize,
    first_frame: f64,
    fitted: bool,
    band: (f64, f64),
    n: usize,
    four: Option<Fourier>,
    g_first: Vec<C64>,
    g_regular: Vec<C64>,
    outputs: Vec<u8>,
    stage: Stage,
    arrival0: f64,
    found: Vec<Found>,
    arrivals: Vec<f64>,
    fit: Option<Fit>,
    origin: f64,
    output_offset: Vec<f64>,
    noise_psd: Option<(usize, Vec<f64>)>,
    grid: Vec<f64>,
    grid_bins: Vec<(usize, usize)>,
    stats: Vec<Vec<RepeatStats>>,
    acc: Vec<Vec<f64>>,
    reasons: Vec<String>,
}

impl WalkedAnalyser {
    /// Prepares the analysis of `schedule`'s run captured at `capture_rate_hz`.
    ///
    /// Errors: [`MeasureError::Invalid`] for a capture rate outside 8 kHz to
    /// 192 kHz, bad options, a gap too short to analyse (a third of it must
    /// hold at least 20 ms), or a sweep too long for [`MAX_FFT_LEN`].
    pub fn new(
        schedule: &WalkSchedule,
        capture_rate_hz: u32,
        options: &AnalysisOptions,
    ) -> Result<WalkedAnalyser, MeasureError> {
        if !(8_000..=192_000).contains(&capture_rate_hz) {
            return Err(MeasureError::Invalid(format!(
                "capture rate {capture_rate_hz} Hz"
            )));
        }
        let o = options;
        let finite_pos = [
            o.start_early_ms,
            o.max_latency_ms,
            o.search_ms,
            o.max_ratio_ppm,
            o.status_slack_ms,
            o.max_residual_us,
            o.max_phase_spread_deg,
            o.min_preroll_ms,
        ]
        .iter()
        .all(|v| v.is_finite() && *v >= 0.0 && *v <= 1e6);
        let bands_ok = o.phase_check_bands.len() <= 16
            && o.phase_check_bands
                .iter()
                .all(|(a, b)| a.is_finite() && b.is_finite() && a < b);
        if !finite_pos || !bands_ok || o.search_ms < 1.0 || o.min_passes_reliable < 2 {
            return Err(MeasureError::Invalid("bad analysis options".into()));
        }
        let sweep = &schedule.sweep;
        let rate = f64::from(capture_rate_hz);
        let nominal = rate / f64::from(sweep.rate_hz);
        let gap_cap = f64::from(sweep.gap_samples) * nominal;
        let pre_f = (o.search_ms * rate / 1000.0).min(gap_cap / 3.0).floor();
        if pre_f < 0.02 * rate {
            return Err(MeasureError::Invalid(
                "the gap between sweeps is too short to analyse (at least 60 ms)".into(),
            ));
        }
        let pre = pre_f as usize;
        let tail = (gap_cap - 2.0 * pre_f).floor().max(0.0) as usize;
        let cycle_cap = (f64::from(sweep.cycle_samples) * nominal).ceil() as usize;
        let seg1 = cycle_cap
            .saturating_add(tail)
            .saturating_add(pre.saturating_mul(2));
        let n = fft_len(seg1)
            .ok_or_else(|| MeasureError::Invalid("sweep too long to analyse".into()))?;
        let f_hi = sweep.end_freq_hz().min(0.42 * rate);
        let band = (sweep.start_freq_hz(), f_hi);
        if band.1.is_nan() || band.1 <= band.0 {
            return Err(MeasureError::Invalid(
                "no sweep band below the capture's Nyquist".into(),
            ));
        }
        let outputs = schedule.outputs();
        let grid = LogGrid::STANDARD.freqs()?;
        let n_out = outputs.len();
        Ok(WalkedAnalyser {
            schedule: schedule.clone(),
            options: options.clone(),
            rate,
            nominal,
            pre,
            tail,
            cycle_cap,
            first_frame: 0.0,
            fitted: false,
            band,
            n,
            four: None,
            g_first: Vec::new(),
            g_regular: Vec::new(),
            outputs,
            stage: Stage::Search,
            arrival0: 0.0,
            found: Vec::new(),
            arrivals: Vec::new(),
            fit: None,
            origin: 0.0,
            output_offset: vec![0.0; n_out],
            noise_psd: None,
            grid,
            grid_bins: Vec::new(),
            stats: vec![Vec::new(); n_out],
            acc: vec![vec![0.0; pre.saturating_add(tail)]; n_out],
            reasons: Vec::new(),
        })
    }

    fn total_slices(&self) -> usize {
        self.schedule
            .cycles()
            .len()
            .saturating_mul(2)
            .saturating_add(3)
    }

    fn slices_done(&self) -> usize {
        let n = self.schedule.cycles().len();
        match self.stage {
            Stage::Search => 0,
            Stage::Pass1(k) => k.saturating_add(1),
            Stage::Fit => n.saturating_add(1),
            Stage::Pass2(k) => n.saturating_add(2).saturating_add(k),
            Stage::Finish => n.saturating_mul(2).saturating_add(2),
            Stage::Done => self.total_slices(),
        }
    }

    fn output_index(&self, output: u8) -> usize {
        self.outputs.iter().position(|&o| o == output).unwrap_or(0)
    }

    /// Does the next slice. Errors end the analysis: a
    /// [`TimingError::CycleIdentity`] means the run must be discarded.
    pub fn step(&mut self, capture: &WalkedCapture<'_>) -> Result<Step, TimingError> {
        if capture.rate_hz as f64 != self.rate {
            return Err(MeasureError::Invalid("the capture's rate changed".into()).into());
        }
        if capture.samples.len() > MAX_CAPTURE_SAMPLES {
            return Err(MeasureError::Invalid("capture past the bound".into()).into());
        }
        let progressed = match self.stage {
            Stage::Search => self.search(capture)?,
            Stage::Pass1(k) => self.pass1(capture, k)?,
            Stage::Fit => {
                self.fit_and_check(capture)?;
                true
            }
            Stage::Pass2(k) => self.pass2(capture, k)?,
            Stage::Finish => {
                let r = self.finish()?;
                self.stage = Stage::Done;
                return Ok(Step::Done(Box::new(r)));
            }
            Stage::Done => {
                return Err(MeasureError::Invalid("the analysis is finished".into()).into());
            }
        };
        if !progressed {
            return Ok(Step::NeedCapture);
        }
        Ok(Step::Progress {
            done: self.slices_done(),
            total: self.total_slices(),
        })
    }

    fn ensure_filters(&mut self, first: &[f64], regular: &[f64]) -> Result<(), MeasureError> {
        if self.four.is_some() {
            return Ok(());
        }
        let mut four = Fourier::new(self.n);
        self.g_first = inverse_filter(&mut four, first, self.rate, self.band)?;
        self.g_regular = inverse_filter(&mut four, regular, self.rate, self.band)?;
        self.grid_bins = self
            .grid
            .iter()
            .map(|&f| {
                let r = 2f64.powf(1.0 / 192.0);
                let k = |x: f64| x * self.n as f64 / self.rate;
                let half = (self.n / 2) as f64;
                let lo = k(f / r).ceil().clamp(0.0, half);
                let hi = k(f * r).floor().clamp(0.0, half);
                if hi >= lo {
                    (lo as usize, hi as usize)
                } else {
                    let c = k(f).round().clamp(0.0, half) as usize;
                    (c, c)
                }
            })
            .collect();
        self.four = Some(four);
        Ok(())
    }

    /// Reads a segment of the capture into a complex buffer; `false` when it
    /// is not all there yet (and the capture is not complete).
    fn fill(buf: &mut [C64], capture: &WalkedCapture<'_>, start: i64, len: usize) -> bool {
        let end = start.saturating_add(len as i64);
        if !capture.complete && end > capture.samples.len() as i64 {
            return false;
        }
        for (i, c) in buf.iter_mut().enumerate() {
            let v = if i < len {
                let at = start.saturating_add(i as i64);
                if at < 0 {
                    0.0
                } else {
                    capture
                        .samples
                        .get(at as usize)
                        .map_or(0.0, |&s| f64::from(s))
                }
            } else {
                0.0
            };
            *c = C64::new(if v.is_finite() { v } else { 0.0 }, 0.0);
        }
        true
    }

    /// Deconvolves `len` samples from `start` into a time-domain response of
    /// FFT length `four.n`.
    fn deconvolve_at(
        four: &mut Fourier,
        g: &[C64],
        capture: &WalkedCapture<'_>,
        start: i64,
        len: usize,
    ) -> Option<Vec<f64>> {
        let mut buf = vec![czero(); four.n];
        if !Self::fill(&mut buf, capture, start, len) {
            return None;
        }
        four.forward(&mut buf);
        for (c, &gk) in buf.iter_mut().zip(g) {
            *c = cmul(*c, gk);
        }
        four.inverse(&mut buf);
        Some(buf.iter().map(|c| c.re).collect())
    }

    /// Finds the largest peak of `h` at indices `lo..=hi`; `Some((index,
    /// fraction, power))` when it is at least [`FOUND_RATIO`] times the median.
    fn heard(h: &[f64], lo: usize, hi: usize) -> Option<(usize, f64, f64)> {
        let med = median(h.iter().map(|v| v * v).collect());
        let (idx, p) = h
            .iter()
            .enumerate()
            .take(hi.saturating_add(1))
            .skip(lo)
            .map(|(i, v)| (i, v * v))
            .max_by(|a, b| a.1.total_cmp(&b.1))?;
        if !(p > 0.0 && p.is_finite() && p >= FOUND_RATIO * med) {
            return None;
        }
        let i = idx as i64;
        let frac = parabolic(
            circ(h, i.saturating_sub(1)).abs(),
            circ(h, i).abs(),
            circ(h, i.saturating_add(1)).abs(),
        );
        Some((idx, frac, p))
    }

    fn search(&mut self, capture: &WalkedCapture<'_>) -> Result<bool, TimingError> {
        if capture.samples.iter().any(|v| !v.is_finite()) {
            return Err(MeasureError::Invalid("non-finite capture sample".into()).into());
        }
        let o = &self.options;
        let start_rel = capture.start_frame as f64 - capture.first_frame as f64;
        let w0 = (start_rel - o.start_early_ms * self.rate / 1000.0).floor() as i64;
        let w1 = (start_rel + o.max_latency_ms * self.rate / 1000.0).ceil() as i64;
        let window = w1.saturating_sub(w0).max(1) as usize;
        let len = window
            .saturating_add(self.cycle_cap)
            .saturating_add(self.tail);
        let n0 =
            fft_len(len).ok_or_else(|| MeasureError::Invalid("search window too long".into()))?;
        let mut four = Fourier::new(n0);
        let cap_rate = self.rate as u32;
        let first = self.schedule.sweep.render_at_rate(cap_rate, true)?;
        let regular = self.schedule.sweep.render_at_rate(cap_rate, false)?;
        let g0 = inverse_filter(&mut four, &first, self.rate, self.band)?;
        let Some(h) = Self::deconvolve_at(&mut four, &g0, capture, w0, len) else {
            return Ok(false);
        };
        let Some((idx, frac, p0)) = Self::heard(&h, 0, window) else {
            return Err(TimingError::CycleIdentity(
                "the first cycle was not heard in the window the START implies".into(),
            ));
        };
        let arrival0 = w0 as f64 + idx as f64 + frac;
        // Nothing sweep-like may come before it (a START noted a cycle late
        // would otherwise shift every response onto the next output).
        let before = w0.max(0) as usize;
        if before > 0 {
            let lb = before.min(capture.samples.len());
            if let Some(nb) = fft_len(lb.saturating_add(self.cycle_cap)) {
                let mut fb = Fourier::new(nb);
                let gb = inverse_filter(&mut fb, &regular, self.rate, self.band)?;
                if let Some(hb) = Self::deconvolve_at(&mut fb, &gb, capture, 0, lb) {
                    let last = hb.len().saturating_sub(1);
                    if Self::heard(&hb, 0, last).is_some_and(|(_, _, pb)| pb >= p0 * 1e-3) {
                        return Err(TimingError::CycleIdentity(
                            "a sweep was heard before the START's window".into(),
                        ));
                    }
                }
            }
        }
        self.arrival0 = arrival0;
        self.first_frame = capture.first_frame as f64;
        // The pre-roll's noise (Welch, Hann, 50 % overlap).
        let pre_end = (arrival0 - self.pre as f64 - 0.01 * self.rate).floor();
        let usable = if pre_end > 0.0 {
            (pre_end as usize).min(capture.samples.len())
        } else {
            0
        };
        if (usable as f64) >= self.options.min_preroll_ms * self.rate / 1000.0 && usable >= 256 {
            self.noise_psd = Self::welch(capture.samples.get(..usable).unwrap_or(&[]));
        }
        self.ensure_filters(&first, &regular)?;
        self.stage = Stage::Pass1(0);
        Ok(true)
    }

    /// Welch's power spectral density as noise power per sample per bin
    /// (`mean |X|² / Σw²`), bins `0..=L/2` of an `L`-point FFT.
    fn welch(x: &[f32]) -> Option<(usize, Vec<f64>)> {
        let mut l = 8192usize;
        while l > x.len() && l > 256 {
            l /= 2;
        }
        if x.len() < l {
            return None;
        }
        let hop = l / 2;
        let w: Vec<f64> = (0..l)
            .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / l as f64).cos())
            .collect();
        let sw2: f64 = w.iter().map(|v| v * v).sum();
        let mut four = Fourier::new(l);
        let mut acc = vec![0.0; (l / 2).saturating_add(1)];
        let mut count = 0.0;
        let mut buf = vec![czero(); l];
        let mut start = 0usize;
        while start.saturating_add(l) <= x.len() {
            for ((c, &v), &wi) in buf.iter_mut().zip(x.iter().skip(start)).zip(&w) {
                *c = C64::new(f64::from(v) * wi, 0.0);
            }
            four.forward(&mut buf);
            for (a, c) in acc.iter_mut().zip(&buf) {
                *a += c.norm_sqr();
            }
            count += 1.0;
            start = start.saturating_add(hop);
        }
        if !(count > 0.0 && sw2 > 0.0) {
            return None;
        }
        acc.iter_mut().for_each(|a| *a /= count * sw2);
        Some((l, acc))
    }

    fn noise_at(&self, f: f64) -> Option<f64> {
        let (l, psd) = self.noise_psd.as_ref()?;
        let x = f * *l as f64 / self.rate;
        let i = x.floor().max(0.0) as usize;
        let a = psd.get(i).copied()?;
        let b = psd.get(i.saturating_add(1)).copied().unwrap_or(a);
        let t = x - i as f64;
        Some(a + (b - a) * t)
    }

    fn slope_estimate(&self) -> f64 {
        let pts = self.points(false);
        fit_common_slope(&pts, self.outputs.len()).map_or(self.nominal, |f| f.slope)
    }

    /// (output index, device start, arrival) of every cycle found so far.
    fn points(&self, refined: bool) -> Vec<(usize, f64, f64)> {
        self.schedule
            .cycles()
            .iter()
            .zip(&self.found)
            .enumerate()
            .map(|(k, (c, f))| {
                let y = if refined {
                    self.arrivals.get(k).copied().unwrap_or(f.coarse)
                } else {
                    f.coarse
                };
                (self.output_index(c.output), c.start as f64, y)
            })
            .collect()
    }

    fn predict(&self, k: usize) -> f64 {
        let cycles = self.schedule.cycles();
        let Some(c) = cycles.get(k) else {
            return self.arrival0;
        };
        if k == 0 {
            return self.arrival0;
        }
        let b = self.slope_estimate();
        let last_same = cycles
            .iter()
            .zip(&self.found)
            .rev()
            .find(|(cc, _)| cc.output == c.output);
        if let Some((cc, f)) = last_same {
            return f.coarse + (c.start as f64 - cc.start as f64) * b;
        }
        let first = self.found.first().map_or(self.arrival0, |f| f.coarse);
        first + c.start as f64 * b
    }

    fn pass1(&mut self, capture: &WalkedCapture<'_>, k: usize) -> Result<bool, TimingError> {
        let Some(c) = self.schedule.cycles().get(k).copied() else {
            self.stage = Stage::Fit;
            return Ok(true);
        };
        let expected = self.predict(k);
        let start = (expected.round() as i64).saturating_sub(self.pre as i64);
        let len = self
            .cycle_cap
            .saturating_add(self.tail)
            .saturating_add(self.pre.saturating_mul(2));
        let Some(four) = self.four.as_mut() else {
            return Err(MeasureError::Invalid("analysis not prepared".into()).into());
        };
        let g = if k == 0 {
            &self.g_first
        } else {
            &self.g_regular
        };
        let Some(h) = Self::deconvolve_at(four, g, capture, start, len) else {
            return Ok(false);
        };
        let Some((idx, frac, _)) = Self::heard(&h, 0, self.pre.saturating_mul(2)) else {
            return Err(TimingError::CycleIdentity(format!(
                "cycle {} (output {}) was not heard where the schedule puts it",
                c.index, c.output
            )));
        };
        let sn = (SNIPPET_MS * self.rate / 1000.0).round() as i64;
        let i = idx as i64;
        let snippet: Vec<f64> = (i.saturating_sub(sn)..=i.saturating_add(sn))
            .map(|j| circ(&h, j))
            .collect();
        self.found.push(Found {
            coarse: start as f64 + idx as f64 + frac,
            snippet_start: start.saturating_add(i).saturating_sub(sn),
            snippet,
        });
        self.stage = if k.saturating_add(1) < self.schedule.cycles().len() {
            Stage::Pass1(k.saturating_add(1))
        } else {
            Stage::Fit
        };
        Ok(true)
    }

    fn fit_and_check(&mut self, capture: &WalkedCapture<'_>) -> Result<(), TimingError> {
        let cycles = self.schedule.cycles().to_vec();
        let sn = (SNIPPET_MS * self.rate / 1000.0).round();
        // Refine each cycle against its output's first cycle.
        let mut arrivals = Vec::with_capacity(cycles.len());
        for (c, f) in cycles.iter().zip(&self.found) {
            let first = cycles
                .iter()
                .zip(&self.found)
                .find(|(cc, _)| cc.output == c.output)
                .map(|(_, ff)| ff);
            let y = match first {
                Some(r) if !std::ptr::eq(r, f) => {
                    let base = r.coarse + (f.snippet_start as f64 - r.snippet_start as f64);
                    xcorr_lag(&r.snippet, &f.snippet, sn).map_or(f.coarse, |lag| base + lag)
                }
                _ => f.coarse,
            };
            arrivals.push(y);
        }
        self.arrivals = arrivals;
        let pts = self.points(true);
        let fit = fit_common_slope(&pts, self.outputs.len());
        let (slope, fitted) = fit
            .as_ref()
            .map_or((self.nominal, false), |f| (f.slope, true));
        let ppm = (slope / self.nominal - 1.0) * 1e6;
        if ppm.is_nan() || ppm.abs() > self.options.max_ratio_ppm {
            return Err(TimingError::CycleIdentity(format!(
                "the cycles' spacing implies a clock difference of {ppm:.0} ppm"
            )));
        }
        let fit = match fit {
            Some(f) => f,
            None => {
                // One pass: each output's intercept is its one arrival.
                let mut intercepts = vec![None; self.outputs.len()];
                for &(o, n, y) in &pts {
                    if let Some(slot) = intercepts.get_mut(o).filter(|s| s.is_none()) {
                        *slot = Some(y - slope * n);
                    }
                }
                Fit { slope, intercepts }
            }
        };
        let origin = fit
            .intercepts
            .iter()
            .flatten()
            .copied()
            .fold(f64::INFINITY, f64::min);
        if !origin.is_finite() {
            return Err(MeasureError::NoSignal("no output was heard".into()).into());
        }
        // Every status poll must agree with the cycle the capture was in.
        let slack = self.options.status_slack_ms * f64::from(self.schedule.sweep.rate_hz) / 1000.0;
        for obs in capture.status {
            let rel = obs.capture_frame as f64 - capture.first_frame as f64;
            let d = (rel - origin) / slope;
            if !self
                .schedule
                .status_consistent(d - slack, d + slack, obs.status)
            {
                let expected = self.schedule.status_at(d);
                return Err(TimingError::CycleIdentity(format!(
                    "the device said output {} after {} cycles at frame {}, the capture was at output {} after {}",
                    obs.status.active_channel,
                    obs.status.cycles_done,
                    obs.capture_frame,
                    expected.active_channel,
                    expected.cycles_done
                )));
            }
        }
        self.output_offset = fit
            .intercepts
            .iter()
            .map(|a| a.map_or(0.0, |a| a - origin))
            .collect();
        self.origin = origin;
        self.fitted = fitted;
        if !fitted {
            self.reasons
                .push("a single pass: the clock was not measured".to_string());
        }
        self.fit = Some(fit);
        self.stage = Stage::Pass2(0);
        Ok(())
    }

    fn pass2(&mut self, capture: &WalkedCapture<'_>, k: usize) -> Result<bool, TimingError> {
        let Some(c) = self.schedule.cycles().get(k).copied() else {
            self.stage = Stage::Finish;
            return Ok(true);
        };
        let Some(fit) = self.fit.as_ref() else {
            return Err(MeasureError::Invalid("analysis not fitted".into()).into());
        };
        let slope = fit.slope;
        let e = slope / self.nominal - 1.0;
        let p = self.origin + slope * c.start as f64;
        let base = p.floor();
        let frac = p - base;
        let start = (base as i64).saturating_sub(self.pre as i64);
        let len = self
            .cycle_cap
            .saturating_add(self.tail)
            .saturating_add(self.pre);
        let n = self.n;
        let mut buf = vec![czero(); n];
        if !Self::fill(&mut buf, capture, start, len) {
            return Ok(false);
        }
        let oi = self.output_index(c.output);
        let offset = self.output_offset.get(oi).copied().unwrap_or(0.0);
        let Some(four) = self.four.as_mut() else {
            return Err(MeasureError::Invalid("analysis not prepared".into()).into());
        };
        four.forward(&mut buf);
        let g = if k == 0 {
            &self.g_first
        } else {
            &self.g_regular
        };
        let shift = self.pre as f64 + frac;
        let sweep = &self.schedule.sweep;
        for (bin, (x, &gk)) in buf.iter_mut().zip(g).enumerate() {
            let f = bin_freq(bin, n, self.rate);
            // Odd in f: t(|f|) is even, so conjugate bins stay conjugate.
            let t = sweep.time_of_freq(f.abs());
            let theta = std::f64::consts::TAU * (f * shift / self.rate + f * e * t);
            *x = cmul(cmul(*x, gk), C64::from_polar(1.0, theta));
        }
        // Per-repeat statistics on the standard grid: band power, and the band
        // phasor with the output's own delay taken out (the phase check).
        let mut st = RepeatStats {
            power: Vec::with_capacity(self.grid_bins.len()),
            z: Vec::with_capacity(self.grid_bins.len()),
        };
        for &(lo, hi) in &self.grid_bins {
            let mut pw = 0.0;
            let mut z = czero();
            let mut cnt = 0.0;
            for (bin, x) in buf.iter().enumerate().take(hi.saturating_add(1)).skip(lo) {
                let f = bin_freq(bin, n, self.rate);
                pw += x.norm_sqr();
                let rot = C64::from_polar(1.0, std::f64::consts::TAU * f * offset / self.rate);
                z = cadd(z, cmul(*x, rot));
                cnt += 1.0;
            }
            st.power.push(if cnt > 0.0 { pw / cnt } else { 0.0 });
            st.z.push(z);
        }
        if let Some(s) = self.stats.get_mut(oi) {
            s.push(st);
        }
        four.inverse(&mut buf);
        if let Some(acc) = self.acc.get_mut(oi) {
            let pre = self.pre as i64;
            for (i, a) in acc.iter_mut().enumerate() {
                let j = (i as i64).saturating_sub(pre);
                let idx = j.rem_euclid(n as i64) as usize;
                *a += buf.get(idx).map_or(0.0, |c| c.re);
            }
        }
        self.stage = if k.saturating_add(1) < self.schedule.cycles().len() {
            Stage::Pass2(k.saturating_add(1))
        } else {
            Stage::Finish
        };
        Ok(true)
    }

    fn finish(&mut self) -> Result<WalkedAnalysis, TimingError> {
        let Some(fit) = self.fit.clone() else {
            return Err(MeasureError::Invalid("analysis not fitted".into()).into());
        };
        let cycles = self.schedule.cycles().to_vec();
        let us = 1e6 / self.rate;
        let mut cycle_timing = Vec::with_capacity(cycles.len());
        let mut max_res: f64 = 0.0;
        for (k, c) in cycles.iter().enumerate() {
            let oi = self.output_index(c.output);
            let y = self.arrivals.get(k).copied().unwrap_or(0.0);
            let a = fit.intercepts.get(oi).copied().flatten().unwrap_or(y);
            let res = (y - a - fit.slope * c.start as f64) * us;
            max_res = max_res.max(res.abs());
            cycle_timing.push(CycleTiming {
                index: c.index,
                output: c.output,
                arrival_frame: y + self.first_frame,
                residual_us: res,
            });
        }
        let repeats = self.schedule.repeat;
        let r = f64::from(repeats);
        let seg = self
            .cycle_cap
            .saturating_add(self.tail)
            .saturating_add(self.pre) as f64;
        let n = self.n as f64;
        // Per grid point: Σ|G|² and the bin count of its band.
        let g_sums: Vec<(f64, f64)> = self
            .grid_bins
            .iter()
            .map(|&(lo, hi)| {
                let s: f64 = self
                    .g_regular
                    .iter()
                    .take(hi.saturating_add(1))
                    .skip(lo)
                    .map(|g| g.norm_sqr())
                    .sum();
                (s, (hi.saturating_sub(lo)).saturating_add(1) as f64)
            })
            .collect();
        let bands = if self.options.phase_check_bands.is_empty() {
            vec![(100.0, 10_000.0)]
        } else {
            self.options.phase_check_bands.clone()
        };
        let snr_min = 10f64.powf(PHASE_CHECK_SNR_DB / 10.0);
        let mut outputs = Vec::with_capacity(self.outputs.len());
        let mut max_phase: f64 = 0.0;
        let noise_measured = self.noise_psd.is_some();
        if !noise_measured {
            self.reasons
                .push("no pre-roll: the noise floor is not known".to_string());
        }
        for (oi, &output) in self.outputs.iter().enumerate() {
            let acc = self.acc.get(oi).cloned().unwrap_or_default();
            let samples: Vec<f64> = acc.iter().map(|v| v / r).collect();
            let ir = ImpulseResponse {
                rate_hz: self.rate as u32,
                samples,
                zero_index: self.pre,
            };
            let peak = ir.peak_index().unwrap_or(self.pre as f64);
            let stats = self.stats.get(oi).cloned().unwrap_or_default();
            let noise_db = noise_measured.then(|| {
                self.grid
                    .iter()
                    .zip(&g_sums)
                    .map(|(&f, &(s, cnt))| {
                        let p = self.noise_at(f).unwrap_or(0.0);
                        let rho = p * seg * (s / cnt) / (n * r);
                        10.0 * rho.max(1e-40).log10()
                    })
                    .collect::<Vec<f64>>()
            });
            let spread_db: Vec<f64> = (0..self.grid.len())
                .map(|g| {
                    if stats.len() < 2 {
                        return 0.0;
                    }
                    let vals = stats.iter().filter_map(|s| s.power.get(g).copied());
                    let (mn, mx) =
                        vals.fold((f64::INFINITY, 0.0f64), |(a, b), v| (a.min(v), b.max(v)));
                    if mx <= 0.0 {
                        0.0
                    } else if mn <= 0.0 {
                        200.0
                    } else {
                        (10.0 * (mx / mn).log10()).clamp(0.0, 200.0)
                    }
                })
                .collect();
            // The phase check: points in the check bands and the sweep's band,
            // loud enough per repeat.
            let mut spread: f64 = 0.0;
            let mut checked = 0usize;
            if stats.len() >= 2 && noise_measured {
                for (g, &f) in self.grid.iter().enumerate() {
                    let in_band = f >= self.band.0
                        && f <= self.band.1
                        && bands.iter().any(|&(a, b)| f >= a && f <= b);
                    if !in_band {
                        continue;
                    }
                    let zs: Vec<C64> = stats.iter().filter_map(|s| s.z.get(g).copied()).collect();
                    let mean = zs
                        .iter()
                        .fold(czero(), |a, z| cadd(a, *z))
                        .unscale(zs.len() as f64);
                    let (s, _) = g_sums.get(g).copied().unwrap_or((0.0, 1.0));
                    let noise_z = self.noise_at(f).unwrap_or(f64::INFINITY) * seg * s;
                    let loud = mean.norm_sqr() >= snr_min * noise_z;
                    if !loud {
                        continue;
                    }
                    checked = checked.saturating_add(1);
                    for z in &zs {
                        let d = cmul(*z, mean.conj()).arg().to_degrees().abs();
                        spread = spread.max(d);
                    }
                }
            }
            if stats.len() >= 2 && noise_measured && checked == 0 {
                self.reasons.push(format!(
                    "output {output}: too quiet where the phase is checked"
                ));
            }
            if spread > self.options.max_phase_spread_deg {
                self.reasons.push(format!(
                    "output {output}: its repeats disagree in phase by {spread:.1}°"
                ));
            }
            max_phase = max_phase.max(spread);
            outputs.push(OutputIr {
                output,
                ir,
                peak_index: peak,
                arrival_ms: 0.0,
                repeats,
                noise_db,
                spread_db,
                phase_spread_deg: spread,
                phase_points_checked: checked,
            });
        }
        let earliest = outputs
            .iter()
            .map(|o| o.peak_index)
            .fold(f64::INFINITY, f64::min);
        // Put the timing origin on the earliest output's drift-corrected peak
        // (the fit's intercepts come from the uncorrected first pass, whose
        // peaks the drift smears by up to e·T). One common shift: relative
        // timing between outputs is unchanged.
        let delta = earliest - self.pre as f64;
        let shift_ok = delta.is_finite() && delta.abs() <= self.pre as f64 / 2.0;
        for o in &mut outputs {
            if earliest.is_finite() {
                o.arrival_ms = (o.peak_index - earliest) * 1000.0 / self.rate;
            }
            if shift_ok {
                o.ir.samples = shift_left(&o.ir.samples, delta);
                o.peak_index -= delta;
            }
        }
        if repeats < self.options.min_passes_reliable {
            self.reasons.push(format!(
                "{repeats} passes: timing needs at least {}",
                self.options.min_passes_reliable
            ));
        }
        let kappa = slip_sensitivity(&self.schedule);
        let bound = if kappa.is_finite() && kappa > 1.0 {
            self.options.max_residual_us / kappa
        } else {
            self.options.max_residual_us
        };
        if max_res > bound {
            self.reasons.push(format!(
                "a cycle is {max_res:.1} µs off the clock fit, more than {bound:.1} µs (a capture slip or clock wander)"
            ));
        }
        let fitted = self.fitted;
        let quality = TimingQuality {
            reliable: self.reasons.is_empty(),
            max_residual_us: max_res,
            residual_bound_us: bound,
            max_phase_spread_deg: max_phase,
            passes: repeats,
            reasons: self.reasons.clone(),
        };
        Ok(WalkedAnalysis {
            capture_rate_hz: self.rate as u32,
            clock: ClockFit {
                frames_per_device_sample: fit.slope,
                ratio_ppm: (fit.slope / self.nominal - 1.0) * 1e6,
                fitted,
            },
            quality,
            cycles: cycle_timing,
            outputs,
            band_hz: self.band,
        })
    }
}

/// Moves a response earlier by `delta` samples (a fractional delay in the
/// frequency domain, zero-padded so nothing wraps into the start).
fn shift_left(x: &[f64], delta: f64) -> Vec<f64> {
    let Some(n) = fft_len(x.len().saturating_add(x.len() / 8).saturating_add(64)) else {
        return x.to_vec();
    };
    let mut four = Fourier::new(n);
    let mut buf = vec![czero(); n];
    for (d, &v) in buf.iter_mut().zip(x) {
        d.re = v;
    }
    four.forward(&mut buf);
    for (k, c) in buf.iter_mut().enumerate() {
        let f = bin_freq(k, n, 1.0);
        *c = cmul(*c, C64::from_polar(1.0, std::f64::consts::TAU * f * delta));
    }
    four.inverse(&mut buf);
    buf.iter().take(x.len()).map(|c| c.re).collect()
}

/// How much a single capture or device slip between two cycles can move the
/// fitted delays, relative to the largest fit residual it leaves, over every
/// place it can fall in this schedule: the worst ratio. Below 1 the residual
/// gate alone keeps a slip's delay error under the residual bound; three
/// passes over many outputs reach about 1.4; two passes are infinite (a slip
/// between them fits with no residual), which is why
/// [`AnalysisOptions::min_passes_reliable`] defaults to 3.
pub fn slip_sensitivity(schedule: &WalkSchedule) -> f64 {
    let cycles = schedule.cycles();
    let outputs = schedule.outputs();
    let index = |o: u8| outputs.iter().position(|&x| x == o).unwrap_or(0);
    let mut worst: f64 = 0.0;
    for at in 1..cycles.len() {
        let pts: Vec<(usize, f64, f64)> = cycles
            .iter()
            .enumerate()
            .map(|(k, c)| {
                (
                    index(c.output),
                    c.start as f64,
                    if k >= at { 1.0 } else { 0.0 },
                )
            })
            .collect();
        let Some(fit) = fit_common_slope(&pts, outputs.len()) else {
            continue;
        };
        let res = pts
            .iter()
            .map(|&(o, n, y)| {
                let a = fit.intercepts.get(o).copied().flatten().unwrap_or(y);
                (y - a - fit.slope * n).abs()
            })
            .fold(0.0, f64::max);
        let a: Vec<f64> = fit.intercepts.iter().flatten().copied().collect();
        let lo = a.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = a.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let err = if a.is_empty() { 0.0 } else { hi - lo };
        if err <= 1e-9 {
            continue;
        }
        if res <= 1e-9 {
            return f64::INFINITY;
        }
        worst = worst.max(err / res);
    }
    worst
}

/// Analyses a complete walked run: every slice of [`WalkedAnalyser`] at once.
/// The capture is treated as complete.
///
/// Errors: [`TimingError::CycleIdentity`] when the capture does not line up
/// with the run (the run is discarded); [`TimingError::Measure`] for bad input
/// or no signal.
pub fn analyse_walked(
    schedule: &WalkSchedule,
    capture: &WalkedCapture<'_>,
    options: &AnalysisOptions,
) -> Result<WalkedAnalysis, TimingError> {
    let capture = WalkedCapture {
        complete: true,
        ..*capture
    };
    let mut a = WalkedAnalyser::new(schedule, capture.rate_hz, options)?;
    let bound = a.total_slices().saturating_add(2);
    for _ in 0..bound {
        match a.step(&capture)? {
            Step::Done(r) => return Ok(*r),
            Step::NeedCapture => {
                return Err(
                    MeasureError::NoSignal("the capture ends before the run".into()).into(),
                );
            }
            Step::Progress { .. } => {}
        }
    }
    Err(MeasureError::Invalid("the analysis did not finish".into()).into())
}

#[cfg(test)]
mod tests_wp_p5time;
