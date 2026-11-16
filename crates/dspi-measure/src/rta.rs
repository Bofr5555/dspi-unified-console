//! The microphone RTA: the microphone's real-time spectrum in fractional-octave
//! bands (WP P5RESP; PLAN-P5 T5).
//!
//! Not the **device analyser** (the firmware's, `dspi-core` `rta.rs`): this
//! one analyses the host microphone's samples, which the client pushes.
//!
//! - **Welch:** frames of `fft_len` samples, a Hann window, 50 % overlap;
//!   each frame's one-sided power spectrum is scaled so that the bins sum to
//!   the frame's mean square (Parseval, corrected for the window's power).
//! - **Fractional-octave bands, pink-referenced:** 1/1 to 1/48 octave bands
//!   centred on `1 kHz · 2^(k/N)` from 10 Hz up to the lower of 22 kHz and
//!   the band that still fits under the Nyquist frequency. A band's level is
//!   the **power summed over its bins** (a bin straddling an edge counts by
//!   its overlap), so pink noise, which has equal power in equal fractions
//!   of an octave, reads flat; white noise rises 3 dB an octave. A band
//!   narrower than two bins is marked untrusted (the FFT cannot resolve it).
//! - **Levels:** dBFS in the crate's one convention
//!   ([`FULL_SCALE_SINE_DB`]): `10·log10` of the band's mean square relative
//!   to a full scale of 1.0, plus 3.01 dB, so **a full-scale sine reads
//!   0 dBFS in its band**, exactly as [`crate::cal::level_dbfs`] reads it.
//!   With a microphone calibration ([`MicRta::new`]'s `mic`, a `mic`
//!   document's response on any grid) its deviation is subtracted at each
//!   band centre; with an SPL offset (P5CAL's sensitivity or calibrator
//!   step, [`crate::cal::spl_offset_from_tone`]: dB SPL = this level +
//!   offset) the result is dB SPL and [`Response::spl`] is set. A calibrator
//!   tone therefore reads its own level (94 or 114 dB SPL) in its band.
//! - **Averaging** ([`Averaging`]): exponential with a time constant, fixed
//!   over the last N frames, or infinite; presets [`Averaging::FAST`],
//!   [`Averaging::SLOW`], [`Averaging::INFINITE`]. All averaging is of band
//!   powers, so memory is bounded by the band count, not the FFT length.
//! - **Hold and compare:** [`MicRta::hold`] keeps a copy of the current
//!   spectrum; [`MicRta::compare`] (or [`compare`] for any two captures) gives
//!   the difference in dB.
//!
//! Pure and incremental: [`MicRta::push`] takes blocks of any length (up to
//! [`MAX_BLOCK`]) and analyses each frame as it completes; nothing waits.

use std::collections::VecDeque;
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlannerScalar};

use crate::response::{power_db, resample};
use crate::types::{FULL_SCALE_SINE_DB, GRID_MAX_HZ, GRID_MIN_HZ, LogGrid, MeasureError, Response};

/// The shortest FFT frame, samples.
pub const MIN_FFT_LEN: usize = 1024;
/// The longest FFT frame, samples.
pub const MAX_FFT_LEN: usize = 65_536;
/// The longest block [`MicRta::push`] takes at once, samples.
pub const MAX_BLOCK: usize = 1 << 20;
/// The most frames fixed averaging keeps.
pub const MAX_FIXED_FRAMES: u16 = 64;
/// The finest band resolution, bands per octave (1/48 octave).
pub const MAX_BANDS_PER_OCTAVE: u8 = 48;
/// Sample rates the RTA accepts, Hz.
pub const SAMPLE_RATE_RANGE: std::ops::RangeInclusive<f64> = 8_000.0..=384_000.0;
/// Time constants exponential averaging accepts, seconds.
pub const TIME_CONSTANT_RANGE: std::ops::RangeInclusive<f64> = 0.01..=60.0;
/// The largest SPL offset accepted, dB (either sign).
pub const MAX_SPL_OFFSET_DB: f64 = 300.0;

/// How successive frames are averaged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Averaging {
    /// Exponential: each frame moves the average by `1 - exp(-hop / tau)`
    /// of the way toward it (the first frame is taken as is).
    Exponential {
        /// The time constant, seconds ([`TIME_CONSTANT_RANGE`]).
        time_constant_s: f64,
    },
    /// The plain mean of the last `frames` frames (1 to
    /// [`MAX_FIXED_FRAMES`]).
    Fixed {
        /// Frames averaged.
        frames: u16,
    },
    /// The mean of every frame since the start or the last reset.
    Infinite,
}

impl Averaging {
    /// "Fast": exponential, 125 ms (a sound level meter's fast).
    pub const FAST: Averaging = Averaging::Exponential {
        time_constant_s: 0.125,
    };
    /// "Slow": exponential, 1 s (a sound level meter's slow).
    pub const SLOW: Averaging = Averaging::Exponential {
        time_constant_s: 1.0,
    };
    /// "Infinite": every frame since the start, equally.
    pub const INFINITE: Averaging = Averaging::Infinite;
}

/// What the RTA analyses and how.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RtaConfig {
    /// The capture's sample rate, Hz ([`SAMPLE_RATE_RANGE`]).
    pub sample_rate: f64,
    /// The FFT frame, a power of two from [`MIN_FFT_LEN`] to [`MAX_FFT_LEN`].
    pub fft_len: usize,
    /// Bands per octave: 1 (1/1 octave) to 48 (1/48 octave).
    pub bands_per_octave: u8,
    /// How frames are averaged.
    pub averaging: Averaging,
}

impl RtaConfig {
    /// 48 kHz, 16384-point frames (2.9 Hz bins), 1/3 octave, slow.
    pub const DEFAULT: RtaConfig = RtaConfig {
        sample_rate: 48_000.0,
        fft_len: 16_384,
        bands_per_octave: 3,
        averaging: Averaging::SLOW,
    };
}

impl Default for RtaConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

fn invalid(msg: impl Into<String>) -> MeasureError {
    MeasureError::Invalid(msg.into())
}

/// The band-centre grid of an RTA at `sample_rate` with `bands_per_octave`:
/// centres `1 kHz · 2^(k/N)` from the first at or above 10 Hz to the last at
/// or below 22 kHz whose upper edge is under the Nyquist frequency.
///
/// Errors: [`MeasureError::Invalid`] for a rate outside
/// [`SAMPLE_RATE_RANGE`] or a band count outside 1..=48.
pub fn band_grid(sample_rate: f64, bands_per_octave: u8) -> Result<LogGrid, MeasureError> {
    if !SAMPLE_RATE_RANGE.contains(&sample_rate) {
        return Err(invalid(format!("sample rate {sample_rate} Hz")));
    }
    if !(1..=MAX_BANDS_PER_OCTAVE).contains(&bands_per_octave) {
        return Err(invalid(format!(
            "{bands_per_octave} bands an octave (1 to {MAX_BANDS_PER_OCTAVE})"
        )));
    }
    let n = f64::from(bands_per_octave);
    let k_min = (n * (GRID_MIN_HZ / 1000.0).log2()).ceil();
    let top = GRID_MAX_HZ.min(sample_rate / 2.0 / (0.5 / n).exp2());
    let k_max = (n * (top / 1000.0).log2()).floor();
    if !(k_min.is_finite() && k_max.is_finite() && k_max >= k_min) {
        return Err(invalid("no band fits the sample rate"));
    }
    let grid = LogGrid {
        min_hz: 1000.0 * (k_min / n).exp2(),
        max_hz: 1000.0 * ((k_max + 0.25) / n).exp2(),
        points_per_octave: u16::from(bands_per_octave),
    };
    grid.freqs()?;
    Ok(grid)
}

/// One band's bins and their weights (the share of each bin inside it).
struct BandBins {
    bins: Vec<(usize, f64)>,
    resolved: bool,
}

/// The averaging state, over band powers.
struct Averager {
    mode: Averaging,
    alpha: f64,
    state: Vec<f64>,
    history: VecDeque<Vec<f64>>,
    count: u64,
}

impl Averager {
    fn new(mode: Averaging, hop_s: f64, bands: usize) -> Self {
        let alpha = match mode {
            Averaging::Exponential { time_constant_s } => 1.0 - (-hop_s / time_constant_s).exp(),
            _ => 1.0,
        };
        Self {
            mode,
            alpha: if alpha.is_finite() {
                alpha.clamp(0.0, 1.0)
            } else {
                1.0
            },
            state: vec![0.0; bands],
            history: VecDeque::new(),
            count: 0,
        }
    }

    fn reset(&mut self) {
        self.state.iter_mut().for_each(|v| *v = 0.0);
        self.history.clear();
        self.count = 0;
    }

    fn add(&mut self, frame: &[f64]) {
        let first = self.count == 0;
        self.count = self.count.saturating_add(1);
        match self.mode {
            Averaging::Exponential { .. } => {
                for (s, p) in self.state.iter_mut().zip(frame) {
                    *s = if first {
                        *p
                    } else {
                        *s + self.alpha * (p - *s)
                    };
                }
            }
            Averaging::Infinite => {
                for (s, p) in self.state.iter_mut().zip(frame) {
                    *s += p;
                }
            }
            Averaging::Fixed { frames } => {
                if self.history.len() >= usize::from(frames.max(1)) {
                    self.history.pop_front();
                }
                self.history.push_back(frame.to_vec());
            }
        }
    }

    fn current(&self) -> Option<Vec<f64>> {
        if self.count == 0 {
            return None;
        }
        Some(match self.mode {
            Averaging::Exponential { .. } => self.state.clone(),
            Averaging::Infinite => {
                let c = self.count as f64;
                self.state.iter().map(|s| s / c).collect()
            }
            Averaging::Fixed { .. } => {
                let mut sum = vec![0.0; self.state.len()];
                for f in &self.history {
                    for (s, p) in sum.iter_mut().zip(f) {
                        *s += p;
                    }
                }
                let c = self.history.len().max(1) as f64;
                sum.iter().map(|s| s / c).collect()
            }
        })
    }
}

/// The microphone RTA (see the module documentation).
pub struct MicRta {
    config: RtaConfig,
    grid: LogGrid,
    bands: Vec<BandBins>,
    mic_db: Vec<f64>,
    mic_ok: Vec<bool>,
    spl_offset_db: Option<f64>,
    window: Vec<f64>,
    norm: f64,
    fft: Arc<dyn Fft<f64>>,
    buf: Vec<Complex<f64>>,
    scratch: Vec<Complex<f64>>,
    pending: Vec<f64>,
    band_power: Vec<f64>,
    avg: Averager,
    frames: u64,
    nonfinite: u64,
    held: Option<Response>,
}

impl MicRta {
    /// A new RTA.
    ///
    /// `mic` is the microphone's calibration response (its deviation from
    /// flat, dB, on any grid; P5CAL's parsed file or a `mic` document's
    /// response), subtracted at each band centre; band centres outside it
    /// are marked untrusted. `spl_offset_db` turns the levels into dB SPL
    /// (dB SPL = level + offset).
    ///
    /// Errors: [`MeasureError::Invalid`] for a configuration outside the
    /// bounds above, a malformed `mic` response, or a non-finite or too large
    /// offset.
    pub fn new(
        config: RtaConfig,
        mic: Option<&Response>,
        spl_offset_db: Option<f64>,
    ) -> Result<Self, MeasureError> {
        let n = config.fft_len;
        if !(MIN_FFT_LEN..=MAX_FFT_LEN).contains(&n) || !n.is_power_of_two() {
            return Err(invalid(format!(
                "FFT of {n} points (a power of two, {MIN_FFT_LEN} to {MAX_FFT_LEN})"
            )));
        }
        let grid = band_grid(config.sample_rate, config.bands_per_octave)?;
        match config.averaging {
            Averaging::Exponential { time_constant_s } => {
                if !TIME_CONSTANT_RANGE.contains(&time_constant_s) {
                    return Err(invalid(format!("time constant {time_constant_s} s")));
                }
            }
            Averaging::Fixed { frames } => {
                if !(1..=MAX_FIXED_FRAMES).contains(&frames) {
                    return Err(invalid(format!(
                        "{frames} frames (1 to {MAX_FIXED_FRAMES})"
                    )));
                }
            }
            Averaging::Infinite => {}
        }
        if let Some(o) = spl_offset_db
            && (!o.is_finite() || o.abs() > MAX_SPL_OFFSET_DB)
        {
            return Err(invalid(format!("SPL offset {o} dB")));
        }
        let centres = grid.freqs()?;
        let (mic_db, mic_ok) = match mic {
            Some(m) => {
                let r = resample(m, &grid)?;
                let ok = r.valid.unwrap_or_else(|| vec![true; centres.len()]);
                (r.mag_db, ok)
            }
            None => (vec![0.0; centres.len()], vec![true; centres.len()]),
        };
        let bin_hz = config.sample_rate / n as f64;
        let half = n / 2;
        let half_step = (0.5 / f64::from(config.bands_per_octave)).exp2();
        let bands = centres
            .iter()
            .map(|fc| band_bins(fc / half_step, fc * half_step, bin_hz, half))
            .collect::<Vec<_>>();
        // Hann (periodic), and 1/(N·Σw²) to turn |X|² into a mean square.
        let window: Vec<f64> = (0..n)
            .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos())
            .collect();
        let wpow: f64 = window.iter().map(|w| w * w).sum();
        let norm = 1.0 / (n as f64 * wpow);
        let fft = FftPlannerScalar::<f64>::new().plan_fft_forward(n);
        let scratch = vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len()];
        let hop_s = half as f64 / config.sample_rate;
        let nb = centres.len();
        Ok(Self {
            config,
            grid,
            bands,
            mic_db,
            mic_ok,
            spl_offset_db,
            window,
            norm,
            fft,
            buf: vec![Complex::new(0.0, 0.0); n],
            scratch,
            pending: Vec::with_capacity(n),
            band_power: vec![0.0; nb],
            avg: Averager::new(config.averaging, hop_s, nb),
            frames: 0,
            nonfinite: 0,
            held: None,
        })
    }

    /// The configuration.
    pub fn config(&self) -> RtaConfig {
        self.config
    }

    /// The band-centre grid every spectrum is on.
    pub fn grid(&self) -> LogGrid {
        self.grid
    }

    /// Frames analysed since the start or the last [`MicRta::reset`].
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Non-finite samples seen (each analysed as silence).
    pub fn nonfinite_samples(&self) -> u64 {
        self.nonfinite
    }

    /// Analyses a block of samples (mono, at the configured rate, any length
    /// up to [`MAX_BLOCK`]); each frame completed is averaged in. A
    /// non-finite sample is analysed as silence and counted. Returns the
    /// number of frames completed.
    ///
    /// Errors: [`MeasureError::Invalid`] for a block above [`MAX_BLOCK`]
    /// (nothing is analysed).
    pub fn push(&mut self, samples: &[f32]) -> Result<usize, MeasureError> {
        if samples.len() > MAX_BLOCK {
            return Err(invalid(format!(
                "block of {} samples (at most {MAX_BLOCK})",
                samples.len()
            )));
        }
        let n = self.config.fft_len;
        let hop = n / 2;
        let mut done = 0usize;
        let mut rest = samples;
        while !rest.is_empty() {
            let want = n.saturating_sub(self.pending.len()).min(rest.len());
            let (take, tail) = rest.split_at(want);
            for s in take {
                if s.is_finite() {
                    self.pending.push(f64::from(*s));
                } else {
                    self.nonfinite = self.nonfinite.saturating_add(1);
                    self.pending.push(0.0);
                }
            }
            rest = tail;
            if self.pending.len() >= n {
                self.analyse_frame();
                self.pending.drain(..hop);
                done = done.saturating_add(1);
            }
        }
        Ok(done)
    }

    fn analyse_frame(&mut self) {
        for ((b, x), w) in self.buf.iter_mut().zip(&self.pending).zip(&self.window) {
            *b = Complex::new(x * w, 0.0);
        }
        self.fft
            .process_with_scratch(&mut self.buf, &mut self.scratch);
        let half = self.config.fft_len / 2;
        for (band, out) in self.bands.iter().zip(self.band_power.iter_mut()) {
            let mut p = 0.0;
            for (k, wgt) in &band.bins {
                if let Some(c) = self.buf.get(*k) {
                    // One-sided: every bin but the Nyquist bin counts twice.
                    let side = if *k == half { 1.0 } else { 2.0 };
                    p += wgt * side * c.norm_sqr();
                }
            }
            *out = p * self.norm;
        }
        self.avg.add(&self.band_power);
        self.frames = self.frames.saturating_add(1);
    }

    /// The current averaged spectrum on [`MicRta::grid`]: dBFS (a full-scale
    /// sine reads 0, [`FULL_SCALE_SINE_DB`]), or dB SPL with an offset,
    /// magnitude only; `valid` false where a band
    /// is narrower than two bins or outside the microphone's calibration.
    /// `None` before the first frame.
    pub fn spectrum(&self) -> Option<Response> {
        let avg = self.avg.current()?;
        let offset = self.spl_offset_db.unwrap_or(0.0);
        let mag = avg
            .iter()
            .zip(&self.mic_db)
            .map(|(p, m)| power_db(*p) + FULL_SCALE_SINE_DB - m + offset)
            .collect();
        let valid = self
            .bands
            .iter()
            .zip(&self.mic_ok)
            .map(|(b, ok)| b.resolved && *ok)
            .collect();
        Some(Response {
            grid: self.grid,
            mag_db: mag,
            phase_deg: None,
            valid: Some(valid),
            spl: self.spl_offset_db.is_some(),
        })
    }

    /// Forgets the samples and the average (not the held copy).
    pub fn reset(&mut self) {
        self.pending.clear();
        self.avg.reset();
        self.frames = 0;
    }

    /// Keeps a copy of the current spectrum for [`MicRta::compare`]. Returns
    /// false (and keeps the previous copy) before the first frame.
    pub fn hold(&mut self) -> bool {
        match self.spectrum() {
            Some(s) => {
                self.held = Some(s);
                true
            }
            None => false,
        }
    }

    /// The held copy.
    pub fn held(&self) -> Option<&Response> {
        self.held.as_ref()
    }

    /// Drops the held copy.
    pub fn clear_hold(&mut self) {
        self.held = None;
    }

    /// The current spectrum minus the held copy, dB; `None` without both.
    pub fn compare(&self) -> Option<Response> {
        let held = self.held.as_ref()?;
        compare(&self.spectrum()?, held).ok()
    }
}

/// The bins of a band `[lo, hi)`: bin `k` covers `[(k-½)·Δ, (k+½)·Δ)`, and
/// counts by the share of it inside the band. DC is never counted.
fn band_bins(lo: f64, hi: f64, bin_hz: f64, half: usize) -> BandBins {
    let first = ((lo / bin_hz - 0.5).floor().max(1.0) as usize).min(half);
    let last = ((hi / bin_hz + 0.5).ceil().max(1.0) as usize).min(half);
    let bins = (first..=last)
        .filter_map(|k| {
            let c = k as f64 * bin_hz;
            let a = lo.max(c - bin_hz / 2.0);
            let b = hi.min(c + bin_hz / 2.0);
            let w = (b - a) / bin_hz;
            (w > 0.0).then_some((k, w))
        })
        .collect();
    BandBins {
        bins,
        resolved: hi - lo >= 2.0 * bin_hz,
    }
}

/// `current - reference`, dB, point by point (hold and compare of two
/// captures). Trusted where both are; not dB SPL; no phase.
///
/// Errors: [`MeasureError::Invalid`] for a malformed response or two grids.
pub fn compare(current: &Response, reference: &Response) -> Result<Response, MeasureError> {
    crate::response::check(current)?;
    crate::response::check(reference)?;
    if current.grid != reference.grid {
        return Err(invalid("captures on different grids"));
    }
    let n = current.mag_db.len();
    let flag = |r: &Response, i: usize| {
        r.valid
            .as_ref()
            .is_none_or(|v| v.get(i).copied().unwrap_or(false))
    };
    Ok(Response {
        grid: current.grid,
        mag_db: current
            .mag_db
            .iter()
            .zip(&reference.mag_db)
            .map(|(a, b)| a - b)
            .collect(),
        phase_deg: None,
        valid: Some(
            (0..n)
                .map(|i| flag(current, i) && flag(reference, i))
                .collect(),
        ),
        spl: false,
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5resp {
    use super::*;
    use proptest::prelude::*;

    /// xorshift64*, uniform in [-1, 1).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            let v = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
            (v >> 11) as f64 / (1u64 << 52) as f64 - 1.0
        }
    }

    /// Paul Kellet's refined pink-noise filter (±0.05 dB above 9.2 Hz at
    /// 44.1 kHz), driven by uniform white noise.
    fn pink(len: usize, seed: u64) -> Vec<f32> {
        let mut r = Rng(seed);
        let mut b = [0.0f64; 7];
        (0..len)
            .map(|_| {
                let w = r.next();
                b[0] = 0.99886 * b[0] + w * 0.0555179;
                b[1] = 0.99332 * b[1] + w * 0.0750759;
                b[2] = 0.96900 * b[2] + w * 0.1538520;
                b[3] = 0.86650 * b[3] + w * 0.3104856;
                b[4] = 0.55000 * b[4] + w * 0.5329522;
                b[5] = -0.7616 * b[5] - w * 0.0168980;
                let p = b.iter().sum::<f64>() + w * 0.5362;
                b[6] = w * 0.115926;
                (p * 0.05) as f32
            })
            .collect()
    }

    #[test]
    fn wp_p5resp_pink_noise_reads_flat_within_0_5_db_per_third_octave() {
        let cfg = RtaConfig {
            sample_rate: 44_100.0,
            fft_len: 16_384,
            bands_per_octave: 3,
            averaging: Averaging::INFINITE,
        };
        let mut rta = MicRta::new(cfg, None, None).unwrap();
        let x = pink(44_100 * 40, 0x5eed_1234);
        // Pushed in 20 ms blocks, as a client does.
        for block in x.chunks(882) {
            rta.push(block).unwrap();
        }
        let s = rta.spectrum().unwrap();
        let f = rta.grid().freqs().unwrap();
        let sel: Vec<(f64, f64)> = f
            .iter()
            .zip(&s.mag_db)
            .filter(|(f, _)| **f >= 99.0 && **f <= 10_100.0)
            .map(|(f, m)| (*f, *m))
            .collect();
        assert_eq!(sel.len(), 21, "100 Hz to 10 kHz in 1/3 octave");
        let mean = sel.iter().map(|(_, m)| m).sum::<f64>() / sel.len() as f64;
        for (f, m) in &sel {
            assert!((m - mean).abs() <= 0.5, "{f} Hz: {m} vs mean {mean}");
        }
        assert!(
            s.valid
                .unwrap()
                .iter()
                .zip(&f)
                .all(|(v, f)| *f < 99.0 || *v)
        );
        assert!(!s.spl);
    }

    #[test]
    fn wp_p5resp_sine_level_calibration_and_spl() {
        let fs = 48_000.0;
        let amp = 0.5f64;
        let x: Vec<f32> = (0..48_000)
            .map(|i| (amp * (std::f64::consts::TAU * 1000.0 * i as f64 / fs).sin()) as f32)
            .collect();
        let cfg = RtaConfig {
            averaging: Averaging::INFINITE,
            ..RtaConfig::DEFAULT
        };
        let grid = band_grid(fs, 3).unwrap();
        let k = grid
            .freqs()
            .unwrap()
            .iter()
            .position(|f| (f - 1000.0).abs() < 1e-6)
            .unwrap();
        // A sine of peak 0.5 is -6.02 dBFS (a full-scale sine reads 0).
        let want = 20.0 * amp.log10();
        let mut plain = MicRta::new(cfg, None, None).unwrap();
        plain.push(&x).unwrap();
        let s = plain.spectrum().unwrap();
        assert!(
            (s.mag_db[k] - want).abs() < 0.05,
            "{} vs {want}",
            s.mag_db[k]
        );
        // A microphone 2 dB hot everywhere, and an SPL offset of 120 dB.
        let mic = Response {
            grid: LogGrid::STANDARD,
            mag_db: vec![2.0; LogGrid::STANDARD.freqs().unwrap().len()],
            phase_deg: None,
            valid: None,
            spl: false,
        };
        let mut cal = MicRta::new(cfg, Some(&mic), Some(120.0)).unwrap();
        cal.push(&x).unwrap();
        let c = cal.spectrum().unwrap();
        assert!(c.spl);
        assert!((c.mag_db[k] - (want - 2.0 + 120.0)).abs() < 0.05);
    }

    #[test]
    fn wp_p5resp_averaging_modes() {
        let hop_s = 0.1;
        let mut e = Averager::new(
            Averaging::Exponential {
                time_constant_s: 0.1,
            },
            hop_s,
            1,
        );
        assert!(e.current().is_none());
        e.add(&[1.0]);
        assert_eq!(e.current().unwrap(), vec![1.0]);
        e.add(&[2.0]);
        let a = 1.0 - (-1.0f64).exp();
        assert!((e.current().unwrap()[0] - (1.0 + a)).abs() < 1e-12);

        let mut f = Averager::new(Averaging::Fixed { frames: 2 }, hop_s, 1);
        for v in [1.0, 5.0, 3.0] {
            f.add(&[v]);
        }
        assert_eq!(f.current().unwrap(), vec![4.0]);

        let mut i = Averager::new(Averaging::Infinite, hop_s, 1);
        for v in [1.0, 5.0, 3.0] {
            i.add(&[v]);
        }
        assert_eq!(i.current().unwrap(), vec![3.0]);
        i.reset();
        assert!(i.current().is_none());
    }

    #[test]
    fn wp_p5resp_hold_and_compare() {
        let cfg = RtaConfig {
            fft_len: 4096,
            averaging: Averaging::Fixed { frames: 4 },
            ..RtaConfig::DEFAULT
        };
        let mut rta = MicRta::new(cfg, None, None).unwrap();
        assert!(!rta.hold());
        assert!(rta.compare().is_none());
        let quiet: Vec<f32> = pink(48_000, 1).iter().map(|v| v * 0.5).collect();
        rta.push(&quiet).unwrap();
        assert!(rta.hold());
        rta.reset();
        assert_eq!(rta.frames(), 0);
        // The same noise 6.02 dB louder.
        let loud: Vec<f32> = pink(48_000, 1);
        rta.push(&loud).unwrap();
        let d = rta.compare().unwrap();
        let v = d.valid.as_ref().unwrap();
        for (i, m) in d.mag_db.iter().enumerate().filter(|(i, _)| v[*i]) {
            assert!((m - 6.0206).abs() < 0.01, "band {i}: {m}");
        }
        assert!(rta.held().is_some());
        rta.clear_hold();
        assert!(rta.compare().is_none());
    }

    #[test]
    fn wp_p5resp_bad_configs_are_refused() {
        let ok = RtaConfig::DEFAULT;
        for bad in [
            RtaConfig {
                fft_len: 1000,
                ..ok
            },
            RtaConfig { fft_len: 512, ..ok },
            RtaConfig {
                fft_len: 1 << 17,
                ..ok
            },
            RtaConfig {
                sample_rate: f64::NAN,
                ..ok
            },
            RtaConfig {
                bands_per_octave: 0,
                ..ok
            },
            RtaConfig {
                bands_per_octave: 49,
                ..ok
            },
            RtaConfig {
                averaging: Averaging::Fixed { frames: 0 },
                ..ok
            },
            RtaConfig {
                averaging: Averaging::Fixed { frames: 65 },
                ..ok
            },
            RtaConfig {
                averaging: Averaging::Exponential {
                    time_constant_s: f64::INFINITY,
                },
                ..ok
            },
        ] {
            assert!(MicRta::new(bad, None, None).is_err(), "{bad:?}");
        }
        assert!(MicRta::new(ok, None, Some(f64::NAN)).is_err());
        let mut r = MicRta::new(ok, None, None).unwrap();
        assert!(r.push(&vec![0.0; MAX_BLOCK + 1]).is_err());
        r.push(&[f32::NAN, f32::INFINITY, 0.0]).unwrap();
        assert_eq!(r.nonfinite_samples(), 2);
    }

    #[test]
    fn wp_p5resp_band_grid_is_1khz_aligned_and_under_nyquist() {
        for n in [1u8, 3, 6, 12, 24, 48] {
            for fs in [44_100.0, 48_000.0, 96_000.0, 8_000.0] {
                let f = band_grid(fs, n).unwrap().freqs().unwrap();
                assert!(f[0] >= 10.0 - 1e-9);
                let top = *f.last().unwrap();
                assert!(top <= 22_000.0 && top * (0.5 / f64::from(n)).exp2() <= fs / 2.0);
                let k = f64::from(n) * (f[0] / 1000.0).log2();
                assert!((k - k.round()).abs() < 1e-9);
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        #[test]
        fn proptest_wp_p5resp_rta_never_panics(
            fs in prop_oneof![Just(44_100.0f64), Just(48_000.0), proptest::num::f64::ANY],
            fft_pow in 8u32..18,
            bpo in 0u8..60,
            mode in 0u8..3,
            param in proptest::num::f64::ANY,
            blocks in proptest::collection::vec(proptest::collection::vec(proptest::num::f32::ANY, 0..3000), 0..6),
            offset in proptest::option::of(proptest::num::f64::ANY),
        ) {
            let averaging = match mode {
                0 => Averaging::Exponential { time_constant_s: param },
                1 => Averaging::Fixed { frames: (param.abs() % 100.0) as u16 },
                _ => Averaging::Infinite,
            };
            let cfg = RtaConfig { sample_rate: fs, fft_len: 1usize << fft_pow, bands_per_octave: bpo, averaging };
            if let Ok(mut rta) = MicRta::new(cfg, None, offset) {
                for b in &blocks {
                    rta.push(b).unwrap();
                }
                if let Some(s) = rta.spectrum() {
                    prop_assert!(s.mag_db.iter().all(|v| v.is_finite()));
                    rta.hold();
                    if let Some(c) = rta.compare() {
                        prop_assert!(c.mag_db.iter().all(|v| v.is_finite()));
                    }
                }
            }
        }
    }
}
