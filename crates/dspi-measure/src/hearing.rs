//! Does the microphone hear what the generator plays? (WP P5RAMPFIX; PLAN-P5
//! S3, the ramp's "heard" and "follows" checks.)
//!
//! The first real-microphone spike (P5SPIKE, 2026-11-18) found that a
//! broadband level cannot answer this: the microphone's floor was -51 dBFS
//! broadband but nearly all of it low-frequency rumble (around 50 Hz), while
//! the floor at 1 kHz to 3 kHz lay far lower. A ramp step that was clearly
//! audible in its band was invisible against the rumble, so the ramp judged
//! every step "not heard". A [`Listener`] compares like with like instead:
//!
//! - **The band is what is played:** the sweep's own band within 100 Hz to
//!   10 kHz ([`hearing_band`]); pink noise uses 100 Hz to 10 kHz. The band's
//!   edges are tapered inside it, nothing outside counts, so rumble below
//!   100 Hz is never taken for signal.
//! - **A sweep: the matched filter of the known stimulus, whitened by the
//!   pre-roll's noise.** The capture is correlated with the sweep (P5TIME's
//!   firmware-exact model, rendered at the capture rate), each frequency
//!   weighted by the sweep over the pre-roll's noise power there, the
//!   optimal detector of a known signal in coloured noise: a quiet 3 kHz
//!   region counts for much, a rumbling 120 Hz region for little. The
//!   statistic is the **loudest 2 ms of the filter's output** (the
//!   arrival is unknown); the noise is the same statistic of the same
//!   output away from that peak (outside 0.25 s before it, where a sweep's
//!   harmonics fall, and 0.5 s after it, the room's tail), so the two
//!   compare like with like and the noise is the step's own, even if the
//!   room got louder since the pre-roll. The pre-roll gives the whitening.
//! - **Pink noise: band-limited power.** The capture's third-octave bands
//!   in Hann frames of about 85 ms; each band's level is its loudest
//!   frame's power, the pre-roll's alike; the bands' excesses (below) are
//!   averaged over the louder half of the bands.
//! - **One number, the in-band SNR:** the excess `step / noise - 1` (the
//!   signal's power over the noise's, the noise taken out) in dB. Noise
//!   alone reads well below 0 dB; it rises 6 dB a 6 dB step once the signal
//!   is above the noise, so the ramp's rules (S3) apply to it unchanged.
//!
//! Pure and bounded ([`MAX_SAMPLES`]); any input gives a finite answer or a
//! typed error, never a panic.

use rustfft::FftPlannerScalar;
use rustfft::num_complex::Complex;

use crate::types::MeasureError;

/// The lowest frequency the check listens to, Hz (S3: 100 Hz to 10 kHz).
pub const BAND_MIN_HZ: f64 = 100.0;
/// The highest, Hz.
pub const BAND_MAX_HZ: f64 = 10_000.0;
/// Bands per octave of the band-power check.
pub const BANDS_PER_OCTAVE: u8 = 3;
/// The longest capture, pre-roll or stimulus analysed, samples.
pub const MAX_SAMPLES: usize = 1 << 20;
/// The capture rates accepted, Hz.
pub const RATE_RANGE: std::ops::RangeInclusive<u32> = 8_000..=384_000;
/// The SNR reported when nothing is heard at all (no excess), dB.
pub const NO_SNR_DB: f64 = -60.0;
/// The highest SNR reported (a noise-free capture), dB.
pub const MAX_SNR_DB: f64 = 120.0;
/// A band-power frame (pink noise), seconds (rounded up to a power of two
/// in samples): fine enough below 100 Hz that rumble stays out of the bands.
const BAND_FRAME_S: f64 = 0.17;
/// A frame of the noise spectrum that whitens the matched filter, seconds.
const PSD_FRAME_S: f64 = 0.085;
/// The matched filter's statistic: its output's energy over this, seconds
/// (long enough that a noise window's energy varies little, so noise alone
/// seldom peaks high; short against a sweep's compact peak).
const PEAK_WINDOW_S: f64 = 0.002;
/// The fade at each end of a buffer the matched filter takes, seconds (at
/// most a tenth of the buffer).
const FADE_S: f64 = 0.1;
/// Around the matched filter's peak, the output that is not noise: the
/// sweep's harmonics come before it (the third's 0.16 s for a 1 s sweep of
/// ten octaves), the room's tail after it, seconds.
const EXCLUDE_BEFORE_S: f64 = 0.25;
const EXCLUDE_AFTER_S: f64 = 0.5;
/// The least capture beside the peak to take the noise from, seconds.
const MIN_NOISE_S: f64 = 0.25;

fn invalid(msg: impl Into<String>) -> MeasureError {
    MeasureError::Invalid(msg.into())
}

fn no_signal(msg: impl Into<String>) -> MeasureError {
    MeasureError::NoSignal(msg.into())
}

/// The band the check listens to for a sweep from `f1_hz` to `f2_hz` at
/// `rate_hz`: the sweep's band within [`BAND_MIN_HZ`] to [`BAND_MAX_HZ`] and
/// below 0.42 of the rate. A sweep (nearly) wholly outside that range is
/// heard in its own band. Non-finite or non-positive frequencies give the
/// default band.
pub fn hearing_band(f1_hz: f64, f2_hz: f64, rate_hz: u32) -> (f64, f64) {
    let top = 0.42 * f64::from(rate_hz.max(1));
    let (a, b) = if f1_hz.is_finite() && f2_hz.is_finite() && f1_hz > 0.0 && f2_hz > 0.0 {
        (f1_hz.min(f2_hz), f1_hz.max(f2_hz))
    } else {
        (BAND_MIN_HZ, BAND_MAX_HZ)
    };
    let lo = a.max(BAND_MIN_HZ);
    let hi = b.min(BAND_MAX_HZ).min(top);
    if hi >= lo * 2.0 {
        (lo, hi)
    } else {
        // Less than an octave inside 100 Hz to 10 kHz: the sweep's own band.
        let lo = a.max(10.0);
        let hi = b.min(top);
        if hi > lo {
            (lo, hi)
        } else {
            (BAND_MIN_HZ.min(top / 2.0), top)
        }
    }
}

/// A band's weight at `f`: 1 inside, a raised cosine over the half octave
/// inside each edge, 0 outside.
fn taper(f: f64, band: (f64, f64)) -> f64 {
    let (lo, hi) = band;
    if !(f > lo && f < hi) {
        return 0.0;
    }
    let edge = std::f64::consts::SQRT_2.min((hi / lo).sqrt());
    let rise = |x: f64| 0.5 - 0.5 * (std::f64::consts::PI * x.clamp(0.0, 1.0)).cos();
    let mut w = 1.0;
    if f < lo * edge {
        w *= rise((f / lo).ln() / edge.ln());
    }
    if f > hi / edge {
        w *= rise((hi / f).ln() / edge.ln());
    }
    w
}

fn check_rate(rate_hz: u32) -> Result<(), MeasureError> {
    if RATE_RANGE.contains(&rate_hz) {
        Ok(())
    } else {
        Err(invalid(format!("a capture rate of {rate_hz} Hz")))
    }
}

fn check_band(band_hz: (f64, f64)) -> Result<(), MeasureError> {
    let (lo, hi) = band_hz;
    if lo.is_finite() && hi.is_finite() && lo > 0.0 && hi > lo {
        Ok(())
    } else {
        Err(invalid(format!("a band of {lo} to {hi} Hz")))
    }
}

fn check_len(what: &str, n: usize) -> Result<(), MeasureError> {
    if n > MAX_SAMPLES {
        Err(invalid(format!(
            "a {what} of {n} samples (at most {MAX_SAMPLES})"
        )))
    } else {
        Ok(())
    }
}

fn finite(x: &f32) -> f64 {
    if x.is_finite() { f64::from(*x) } else { 0.0 }
}

/// A frame of about `secs` at `rate_hz`, a power of two.
fn frame_len(rate_hz: u32, secs: f64) -> usize {
    let n = (f64::from(rate_hz) * secs).ceil();
    let n = if n.is_finite() && n >= 1.0 {
        n as usize
    } else {
        4096
    };
    n.clamp(512, 65_536).next_power_of_two()
}

fn hann(n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos())
        .collect()
}

/// The four-term Blackman-Harris window: its sidelobes lie 92 dB down, so
/// rumble a few bins below a band does not leak into it.
fn blackman_harris(n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| {
            let x = std::f64::consts::TAU * i as f64 / n as f64;
            0.358_75 - 0.488_29 * x.cos() + 0.141_28 * (2.0 * x).cos() - 0.011_68 * (3.0 * x).cos()
        })
        .collect()
}

/// Each frame's one-sided power spectrum (bins 0..=n/2), unscaled: frames of
/// `window`'s length, half overlapping.
fn frames_power(samples: &[f32], window: &[f64], mut each: impl FnMut(&[f64])) -> usize {
    let n = window.len().max(1);
    let half = n / 2;
    let fft = FftPlannerScalar::<f64>::new().plan_fft_forward(n);
    let mut scratch = vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len()];
    let mut buf = vec![Complex::new(0.0, 0.0); n];
    let mut power = vec![0.0f64; half.saturating_add(1)];
    let mut frames = 0usize;
    let mut start = 0usize;
    while let Some(frame) = start.checked_add(n).and_then(|end| samples.get(start..end)) {
        for ((b, x), w) in buf.iter_mut().zip(frame).zip(window) {
            *b = Complex::new(finite(x) * w, 0.0);
        }
        fft.process_with_scratch(&mut buf, &mut scratch);
        for (p, c) in power.iter_mut().zip(&buf) {
            *p = c.norm_sqr();
        }
        each(&power);
        frames = frames.saturating_add(1);
        start = start.saturating_add(half.max(1));
    }
    frames
}

// ── Pink noise: band levels ────────────────────────────────────────────

/// The third-octave bands of `band_hz` at `bin_hz`: (first bin, last bin).
fn bands(band_hz: (f64, f64), bin_hz: f64, half: usize) -> Vec<(usize, usize)> {
    let per = f64::from(BANDS_PER_OCTAVE);
    let step = (1.0 / per).exp2();
    let edge = step.sqrt();
    let k_lo = (per * (band_hz.0 / 1000.0).log2()).ceil();
    let k_hi = (per * (band_hz.1 / 1000.0).log2()).floor();
    let mut out = Vec::new();
    if !(k_lo.is_finite() && k_hi.is_finite()) || k_hi < k_lo || k_hi - k_lo > 200.0 {
        return out;
    }
    let mut k = k_lo;
    while k <= k_hi {
        let fc = 1000.0 * (k / per).exp2();
        let lo = (fc / edge).max(band_hz.0);
        let hi = (fc * edge).min(band_hz.1);
        let first = ((lo / bin_hz).round().max(1.0) as usize).min(half);
        let last = ((hi / bin_hz).round().max(1.0) as usize).min(half);
        if last >= first {
            out.push((first, last));
        }
        k += 1.0;
    }
    out
}

/// Each band's loudest frame's power.
fn band_levels(
    samples: &[f32],
    rate_hz: u32,
    band_hz: (f64, f64),
) -> Result<Vec<f64>, MeasureError> {
    let n = frame_len(rate_hz, BAND_FRAME_S);
    if samples.len() < n {
        return Err(no_signal(format!(
            "{} samples, less than one frame of {n}",
            samples.len()
        )));
    }
    let half = n / 2;
    let bands = bands(band_hz, f64::from(rate_hz) / n as f64, half);
    if bands.is_empty() {
        return Err(no_signal("no band to listen to"));
    }
    let mut peak = vec![0.0f64; bands.len()];
    frames_power(samples, &blackman_harris(n), |power| {
        for ((first, last), p) in bands.iter().zip(peak.iter_mut()) {
            let v: f64 = power.get(*first..=*last).map_or(0.0, |s| s.iter().sum());
            if v.is_finite() && v > *p {
                *p = v;
            }
        }
    });
    Ok(peak)
}

// ── A sweep: the whitened matched filter ───────────────────────────────

/// The pre-roll's noise power by frequency (Welch, Hann frames).
#[derive(Debug, Clone, PartialEq)]
struct NoisePsd {
    bin_hz: f64,
    power: Vec<f64>,
}

impl NoisePsd {
    fn new(samples: &[f32], rate_hz: u32) -> Result<NoisePsd, MeasureError> {
        let n = frame_len(rate_hz, PSD_FRAME_S);
        if samples.len() < n {
            return Err(no_signal("a pre-roll shorter than one frame"));
        }
        let mut sum = vec![0.0f64; (n / 2).saturating_add(1)];
        let frames = frames_power(samples, &hann(n), |power| {
            for (s, p) in sum.iter_mut().zip(power) {
                *s += p;
            }
        });
        let frames = frames.max(1) as f64;
        let mut power: Vec<f64> = sum.iter().map(|s| s / frames).collect();
        // Never divide by nothing: a floor 120 dB under the loudest bin.
        let top = power.iter().copied().fold(0.0f64, f64::max);
        let floor = if top > 0.0 { top * 1e-12 } else { 1.0 };
        for p in &mut power {
            if !(p.is_finite() && *p > floor) {
                *p = floor;
            }
        }
        Ok(NoisePsd {
            bin_hz: f64::from(rate_hz) / n as f64,
            power,
        })
    }

    /// The power at `f`, interpolated.
    fn at(&self, f: f64) -> f64 {
        let x = (f / self.bin_hz).max(0.0);
        let i = x.floor();
        let u = x - i;
        let i = i as usize;
        let a = self
            .power
            .get(i)
            .or(self.power.last())
            .copied()
            .unwrap_or(1.0);
        let b = self
            .power
            .get(i.saturating_add(1))
            .or(self.power.last())
            .copied()
            .unwrap_or(a);
        a + (b - a) * u
    }
}

/// The matched filter's statistic of `x`: the loudest [`PEAK_WINDOW_S`] of
/// its output (a circular correlation over `x`'s own length, both ends
/// faded, so noise meets the whole stimulus at every lag), and that
/// window's start.
fn matched_peak(
    x: &[f32],
    template: &[f64],
    rate_hz: u32,
    band: (f64, f64),
    psd: &NoisePsd,
) -> Result<Peak, MeasureError> {
    let n = x.len();
    if n < template.len() || template.is_empty() {
        return Err(no_signal(format!(
            "{n} samples, shorter than the stimulus of {}",
            template.len()
        )));
    }
    let mut planner = FftPlannerScalar::<f64>::new();
    let fwd = planner.plan_fft_forward(n);
    let inv = planner.plan_fft_inverse(n);
    let mut scratch = vec![
        Complex::new(0.0, 0.0);
        fwd.get_inplace_scratch_len()
            .max(inv.get_inplace_scratch_len())
    ];
    // Both ends faded (a Tukey window): the correlation is circular, and
    // rumble's jump from the last sample to the first would be a click.
    let fade = ((f64::from(rate_hz) * FADE_S) as usize).min(n / 10).max(1);
    let gain = |i: usize| -> f64 {
        let d = i.min(n.saturating_sub(1).saturating_sub(i));
        if d >= fade {
            1.0
        } else {
            0.5 - 0.5 * (std::f64::consts::PI * d as f64 / fade as f64).cos()
        }
    };
    let mut xs: Vec<Complex<f64>> = x
        .iter()
        .enumerate()
        .map(|(i, v)| Complex::new(finite(v) * gain(i), 0.0))
        .collect();
    let mut ss: Vec<Complex<f64>> = template
        .iter()
        .map(|v| Complex::new(if v.is_finite() { *v } else { 0.0 }, 0.0))
        .chain(std::iter::repeat(Complex::new(0.0, 0.0)))
        .take(n)
        .collect();
    fwd.process_with_scratch(&mut xs, &mut scratch);
    fwd.process_with_scratch(&mut ss, &mut scratch);
    let bin_hz = f64::from(rate_hz) / n as f64;
    for (k, (xv, sv)) in xs.iter_mut().zip(&ss).enumerate() {
        let f = (k.min(n.saturating_sub(k))) as f64 * bin_hz;
        let w = taper(f, band) / psd.at(f);
        // x · conj(s) · w, written out (no operator on a non-primitive).
        let (a, b, c, d) = (xv.re, xv.im, sv.re, sv.im);
        *xv = Complex::new((a * c + b * d) * w, (b * c - a * d) * w);
    }
    inv.process_with_scratch(&mut xs, &mut scratch);
    // Each window's energy, circularly (window i starts at sample i).
    let w = ((f64::from(rate_hz) * PEAK_WINDOW_S).round() as usize).clamp(1, n);
    let e: Vec<f64> = xs.iter().map(|c| c.re * c.re).collect();
    let mut acc: f64 = e.iter().take(w).sum();
    let mut win = Vec::with_capacity(n);
    for i in 0..n {
        win.push(acc.max(0.0));
        let out = e.get(i).copied().unwrap_or(0.0);
        let inn = e
            .get(i.saturating_add(w).checked_rem(n).unwrap_or(0))
            .copied()
            .unwrap_or(0.0);
        acc += inn - out;
    }
    let (at, peak) = win
        .iter()
        .copied()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap_or((0, 0.0));
    // The noise: the loudest window away from the peak, outside where a
    // sweep's harmonics (before it) and the room's tail (after it) lie.
    let before = (f64::from(rate_hz) * EXCLUDE_BEFORE_S) as usize;
    let after = (f64::from(rate_hz) * EXCLUDE_AFTER_S) as usize;
    let mut noise: Option<f64> = None;
    let mut outside = 0usize;
    for (j, v) in win.iter().enumerate() {
        // The circular distance from the peak forward to `j`.
        let d = if j >= at {
            j.saturating_sub(at)
        } else {
            n.saturating_sub(at.saturating_sub(j))
        };
        if d <= after || n.saturating_sub(d) <= before {
            continue;
        }
        outside = outside.saturating_add(1);
        noise = Some(noise.map_or(*v, |m: f64| m.max(*v)));
    }
    let min_outside = (f64::from(rate_hz) * MIN_NOISE_S) as usize;
    match noise {
        Some(noise) if outside >= min_outside.max(1) => Ok(Peak { peak, noise, at }),
        _ => Err(no_signal(format!(
            "{n} samples leave too little capture beside the sweep's peak"
        ))),
    }
}

/// The matched filter's peak, the noise's loudest window beside it, and
/// where the peak lies.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Peak {
    peak: f64,
    noise: f64,
    at: usize,
}

// ── The listener ───────────────────────────────────────────────────────

/// What the generator plays during the steps a [`Listener`] judges.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Played<'a> {
    /// A log sweep: one cycle at unit amplitude as the capture hears it
    /// ([`crate::stimulus::LogSweep::render_at_rate`]).
    Sweep(&'a [f64]),
    /// Pink noise.
    Pink,
}

#[derive(Debug, Clone, PartialEq)]
enum Detector {
    Sweep { template: Vec<f64>, psd: NoisePsd },
    Pink { noise: Vec<f64> },
}

/// The noise floor of a pre-roll, ready to judge the steps that follow it.
#[derive(Debug, Clone, PartialEq)]
pub struct Listener {
    rate_hz: u32,
    band_hz: (f64, f64),
    detector: Detector,
}

/// How well a step is heard against its pre-roll.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hearing {
    /// The in-band SNR, dB: the step's excess over the pre-roll's noise,
    /// [`NO_SNR_DB`] to [`MAX_SNR_DB`].
    pub snr_db: f64,
    /// The step's in-band level, dB on the listener's own scale (only
    /// differences between steps of one listener mean anything): the
    /// matched filter's peak, or pink noise's louder half of the bands. It
    /// follows the signal where the SNR may stop rising (a sweep's own
    /// sidelobes bound its SNR at about 45 dB).
    pub level_db: f64,
    /// A sweep: where in the step its matched filter peaked, seconds from
    /// the step's first sample (circular). `None` for pink noise.
    pub peak_s: Option<f64>,
}

fn level_of(power: f64) -> f64 {
    if power.is_finite() && power > 1e-300 {
        10.0 * power.log10()
    } else {
        -3000.0
    }
}

fn snr_of(step: f64, noise: f64) -> f64 {
    let ratio = if noise > 1e-300 {
        step / noise
    } else if step > 1e-300 {
        f64::INFINITY
    } else {
        1.0
    };
    let excess = ratio - 1.0;
    if !excess.is_finite() {
        return if excess > 0.0 { MAX_SNR_DB } else { NO_SNR_DB };
    }
    if excess > 10f64.powf(NO_SNR_DB / 10.0) {
        (10.0 * excess.log10()).clamp(NO_SNR_DB, MAX_SNR_DB)
    } else {
        NO_SNR_DB
    }
}

impl Listener {
    /// The listener of `pre_roll` (the generator idle, at `rate_hz`) for
    /// steps that play `played` in `band_hz` (from [`hearing_band`]). A
    /// sweep's step must be as long as the sweep and at least 1 s (its peak
    /// and the noise beside it).
    ///
    /// Errors: [`MeasureError::Invalid`] for a rate outside [`RATE_RANGE`], a
    /// band that is not positive and rising, or a buffer past
    /// [`MAX_SAMPLES`]; [`MeasureError::NoSignal`] for a pre-roll too short.
    pub fn new(
        pre_roll: &[f32],
        rate_hz: u32,
        band_hz: (f64, f64),
        played: Played<'_>,
    ) -> Result<Listener, MeasureError> {
        check_rate(rate_hz)?;
        check_band(band_hz)?;
        check_len("pre-roll", pre_roll.len())?;
        let detector = match played {
            Played::Sweep(template) => {
                check_len("stimulus", template.len())?;
                if template.is_empty() {
                    return Err(invalid("an empty stimulus"));
                }
                Detector::Sweep {
                    template: template.to_vec(),
                    psd: NoisePsd::new(pre_roll, rate_hz)?,
                }
            }
            Played::Pink => Detector::Pink {
                noise: band_levels(pre_roll, rate_hz, band_hz)?,
            },
        };
        Ok(Listener {
            rate_hz,
            band_hz,
            detector,
        })
    }

    /// The band listened to, Hz.
    pub fn band_hz(&self) -> (f64, f64) {
        self.band_hz
    }

    /// How well `step` (the capture of one step, from its START to its
    /// tail) is heard.
    ///
    /// Errors: [`MeasureError::Invalid`] for a step past [`MAX_SAMPLES`];
    /// [`MeasureError::NoSignal`] for one too short (a sweep's step: the
    /// sweep, and 1 s; pink noise: a frame).
    pub fn hear(&self, step: &[f32]) -> Result<Hearing, MeasureError> {
        check_len("step", step.len())?;
        match &self.detector {
            Detector::Sweep { template, psd } => {
                let p = matched_peak(step, template, self.rate_hz, self.band_hz, psd)?;
                Ok(Hearing {
                    snr_db: snr_of(p.peak, p.noise),
                    level_db: level_of(p.peak),
                    peak_s: Some(p.at as f64 / f64::from(self.rate_hz)),
                })
            }
            Detector::Pink { noise } => {
                let levels = band_levels(step, self.rate_hz, self.band_hz)?;
                let mut loud = levels.clone();
                loud.sort_by(|a, b| b.total_cmp(a));
                let half = loud.len().div_ceil(2).max(1);
                let level = loud.iter().take(half).sum::<f64>();
                let mut excess: Vec<f64> = levels
                    .iter()
                    .zip(noise)
                    .map(|(s, n)| {
                        let db = snr_of(*s, *n);
                        if db <= NO_SNR_DB {
                            0.0
                        } else {
                            10f64.powf(db / 10.0)
                        }
                    })
                    .collect();
                excess.sort_by(|a, b| b.total_cmp(a));
                let louder = excess.len().div_ceil(2).max(1);
                let mean = excess.iter().take(louder).sum::<f64>() / louder as f64;
                Ok(Hearing {
                    snr_db: snr_of(1.0 + mean, 1.0),
                    level_db: level_of(level),
                    peak_s: None,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests_wp_p5rampfix;
