//! Shared helpers of the wave P5-A review's integration tests: P5SIM's world
//! driven with P5TIME's model of the generator, analysed with P5TIME's timing
//! core. Public API only, as `dspi-core`'s `measure` module will use it.
#![allow(dead_code)]

use dspi_measure::ir::{
    AnalysisOptions, C64, OutputIr, TimingError, WalkedAnalysis, WalkedCapture, analyse_walked,
    response_at,
};
use dspi_measure::sim::{Burst, Capture, ProtoModel, Stimulus, World, render};
use dspi_measure::stimulus::{LogSweep, SweepConfig, WalkSchedule, peak_linear};
use dspi_measure::types::ChannelChain;

/// The device's and the capture's nominal rate in these tests, Hz.
pub const RATE: u32 = 48_000;

/// One walked run rendered in a world.
pub struct Run {
    pub capture: Capture,
    pub schedule: WalkSchedule,
    /// The capture frame at which the generator's first sample left the
    /// device (what the session notes when the START is acknowledged).
    pub start_frame: u64,
    /// The device frame of the generator's first sample (the pre-roll).
    pub pre_frames: u64,
}

/// Plays the generator's walked log sweep (P5TIME's sample-exact model: the
/// first cycle with its start fade, no gap after the last cycle, a post-roll
/// of one gap) on the outputs in `mask` of `world`, through `chains`.
pub fn walked_run(
    world: &World,
    chains: &[ChannelChain],
    mask: u16,
    passes: u16,
    cfg: &SweepConfig,
    level_db: f32,
    pre_roll_ms: u32,
) -> Run {
    let sweep = LogSweep::new(RATE, cfg).unwrap();
    let amp = peak_linear(level_db);
    let scaled = |first: bool| -> Vec<f32> {
        sweep
            .render_cycle(first)
            .iter()
            .map(|v| *v as f32 * amp)
            .collect()
    };
    let (first, regular) = (scaled(true), scaled(false));
    let schedule = WalkSchedule::new(sweep, mask, passes).unwrap();
    let pre = u64::from(RATE) * u64::from(pre_roll_ms) / 1000;
    let bursts = schedule
        .cycles()
        .iter()
        .map(|c| Burst {
            output: usize::from(c.output),
            start_frame: pre + c.start,
            samples: if c.index == 0 { &first } else { &regular },
            gain: 1.0,
        })
        .collect();
    let stimulus = Stimulus {
        bursts,
        frames: pre + schedule.total_samples() + u64::from(sweep.gap_samples),
    };
    let capture = render(world, &stimulus, chains, &ProtoModel, RATE).unwrap();
    let start_frame = world.capture_position(pre as f64, RATE).unwrap().round() as u64;
    Run {
        capture,
        schedule,
        start_frame,
        pre_frames: pre,
    }
}

/// A summed run: one sweep on several outputs at once, each with its
/// polarity (the generator's `invert_mask`). To the analysis it is a walked
/// run of one output and one pass (magnitudes only: one pass measures no
/// clock).
pub fn summed_run(
    world: &World,
    chains: &[ChannelChain],
    outputs: &[(usize, bool)],
    cfg: &SweepConfig,
    level_db: f32,
    pre_roll_ms: u32,
) -> Run {
    let sweep = LogSweep::new(RATE, cfg).unwrap();
    let amp = peak_linear(level_db);
    let first: Vec<f32> = sweep
        .render_cycle(true)
        .iter()
        .map(|v| *v as f32 * amp)
        .collect();
    let pre = u64::from(RATE) * u64::from(pre_roll_ms) / 1000;
    let stimulus = Stimulus::summed(&first, outputs, pre, u64::from(sweep.gap_samples)).unwrap();
    let capture = render(world, &stimulus, chains, &ProtoModel, RATE).unwrap();
    let bit = outputs.first().map_or(0, |o| o.0);
    let schedule = WalkSchedule::new(sweep, 1 << bit, 1).unwrap();
    let start_frame = world.capture_position(pre as f64, RATE).unwrap().round() as u64;
    Run {
        capture,
        schedule,
        start_frame,
        pre_frames: pre,
    }
}

/// P5TIME's analysis of a run whose blocks arrived frame for frame.
pub fn analyse(
    run: &Run,
    world: &World,
    options: &AnalysisOptions,
) -> Result<WalkedAnalysis, TimingError> {
    let capture = WalkedCapture {
        samples: &run.capture.samples,
        first_frame: world.clock.first_frame,
        rate_hz: run.capture.rate_hz,
        start_frame: run.start_frame,
        status: &[],
        complete: true,
    };
    analyse_walked(&run.schedule, &capture, options)
}

/// `n` frequencies from `lo` to `hi`, log spaced.
pub fn log_space(lo: f64, hi: f64, n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| lo * (hi / lo).powf(i as f64 / (n - 1) as f64))
        .collect()
}

/// How much later `a` is than `b`, µs, over `band`: the delay that best
/// explains the phase between the two impulse responses (both referred to
/// their own run's timing origin).
pub fn lag_us(a: &OutputIr, b: &OutputIr, band: (f64, f64)) -> f64 {
    let freqs = log_space(band.0, band.1, 48);
    let ha = response_at(&a.ir, &freqs).unwrap();
    let hb = response_at(&b.ir, &freqs).unwrap();
    let x: Vec<C64> = ha
        .values
        .iter()
        .zip(&hb.values)
        .map(|(p, q)| p * q.conj())
        .collect();
    let score = |tau_us: f64| -> f64 {
        x.iter()
            .zip(&freqs)
            .map(|(v, f)| (v * C64::from_polar(1.0, std::f64::consts::TAU * f * tau_us * 1e-6)).re)
            .sum()
    };
    let mut best = (0.0, f64::NEG_INFINITY);
    let mut t = -400.0;
    while t <= 400.0 {
        let s = score(t);
        if s > best.1 {
            best = (t, s);
        }
        t += 1.0;
    }
    let (ym, y0, yp) = (score(best.0 - 1.0), best.1, score(best.0 + 1.0));
    let d = ym - 2.0 * y0 + yp;
    let frac = if d.abs() > 0.0 {
        0.5 * (ym - yp) / d
    } else {
        0.0
    };
    best.0 + frac.clamp(-0.5, 0.5)
}

// ---------------------------------------------------------------------------
// From P5TIME's impulse responses to the ways P5XO and P5FIT take.
// ---------------------------------------------------------------------------

use dspi_measure::align::Way;
use dspi_measure::cal::apply_correction;
use dspi_measure::ir::fdw_response;
use dspi_measure::response::{TrustRule, from_grid_values, trust_mask};
use dspi_measure::types::{LogGrid, Response, WayRole};

/// One output's response from its impulse response: P5TIME's
/// frequency-dependent window of `cycles` about the response's peak, onto the
/// standard grid with unwrapped phase (P5RESP), the trusted mask from the
/// pre-roll's noise floor and the repeats' spread (P5RESP), then the
/// microphone's calibration taken out (P5CAL). Returns the response and the
/// SNR per point, dB.
pub fn output_response(
    o: &OutputIr,
    band_hz: (f64, f64),
    cycles: f64,
    mic: Option<&Response>,
) -> (Response, Vec<f64>) {
    let grid = LogGrid::STANDARD;
    let freqs = grid.freqs().unwrap();
    // The onset, not the peak: the peak of a band-limited way jumps between
    // lobes from run to run, and the window with it.
    let centre = o.onset_index;
    let w = fdw_response(&o.ir, centre, &freqs, cycles).unwrap();
    let values: Vec<[f64; 2]> = w.values.iter().map(|c| [c.re, c.im]).collect();
    let delay_s = (centre - o.ir.zero_index as f64) / f64::from(o.ir.rate_hz);
    let mut resp = from_grid_values(&values, &grid, delay_s).unwrap();
    // The noise of a windowed value is the noise per sample times Σw².
    let noise_db: Vec<f64> = o
        .noise_db
        .as_ref()
        .expect("a pre-roll")
        .iter()
        .zip(&w.sum_w2)
        .map(|(n, s)| n + 10.0 * s.max(1e-30).log10())
        .collect();
    let snr_db: Vec<f64> = resp
        .mag_db
        .iter()
        .zip(&noise_db)
        .map(|(s, n)| s - n)
        .collect();
    let noise = Response {
        grid,
        mag_db: noise_db,
        phase_deg: None,
        valid: None,
        spl: false,
    };
    let spread = (o.repeats > 1).then_some(o.spread_db.as_slice());
    // Outside the sweep's band the deconvolution is regularised: nothing
    // there is a measurement, whatever its SNR reads.
    if let Some(v) = resp.valid.as_mut() {
        for (ok, f) in v.iter_mut().zip(&freqs) {
            *ok = *ok && *f >= band_hz.0 && *f <= band_hz.1;
        }
    }
    resp.valid = Some(trust_mask(&resp, Some(&noise), spread, &TrustRule::DEFAULT).unwrap());
    let resp = match mic {
        Some(m) => apply_correction(&resp, m).unwrap(),
        None => resp,
    };
    (resp, snr_db)
}

/// The ways of a walked run, as P5XO takes them.
pub fn ways_of(
    analysis: &WalkedAnalysis,
    chains: &[ChannelChain],
    roles: &[WayRole],
    cycles: f64,
    mic: Option<&Response>,
) -> Vec<Way> {
    analysis
        .outputs
        .iter()
        .zip(roles)
        .map(|(o, role)| {
            let (response, snr_db) = output_response(o, analysis.band_hz, cycles, mic);
            Way {
                output: o.output,
                role: *role,
                chain: chains[usize::from(o.output)].clone(),
                response,
                snr_db,
                // The onset (what P5XO asks for), on the run's timing origin.
                arrival_ms: (o.onset_index - o.ir.zero_index as f64) * 1000.0
                    / f64::from(o.ir.rate_hz),
                timing_reliable: analysis.quality.reliable,
            }
        })
        .collect()
}

/// RMS of `a - b` over the points both trust between `lo` and `hi` Hz, after
/// removing nothing: levels count.
pub fn rms_difference_db(a: &Response, b: &Response, lo: f64, hi: f64) -> (f64, usize) {
    let freqs = a.grid.freqs().unwrap();
    let ok = |r: &Response, i: usize| r.valid.as_ref().is_none_or(|v| v[i]);
    let mut ss = 0.0;
    let mut n = 0usize;
    for (i, f) in freqs.iter().enumerate() {
        if *f < lo || *f > hi || !ok(a, i) || !ok(b, i) {
            continue;
        }
        let e = a.mag_db[i] - b.mag_db[i];
        ss += e * e;
        n += 1;
    }
    ((ss / n.max(1) as f64).sqrt(), n)
}
