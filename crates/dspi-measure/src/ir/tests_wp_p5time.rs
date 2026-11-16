//! WP P5TIME: the timing core against a small simulated world of its own
//! (P5SIM's world arrives in the same wave; these tests do not depend on it).
#![allow(clippy::arithmetic_side_effects)]

use super::*;
use crate::stimulus::{LogSweep, NO_CHANNEL, SweepConfig};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// A test world: ways with an impulse response and a delay, a microphone with
// latency, clock offset, noise and capture faults.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Way {
    output: u8,
    fir: Vec<f64>,
    delay_s: f64,
    gain: f64,
}

#[derive(Clone, Copy, Debug)]
enum Fault {
    /// The capture silently loses `frames` frames at a capture frame (its
    /// frame counter does not show it).
    StepSlip { at_frame: f64, frames: f64 },
    /// The device skips `samples` samples at a device sample.
    DeviceSlip { at_sample: f64, samples: f64 },
    /// The clock ratio wanders: a sinusoidal timing error.
    Wander { amp_us: f64, period_s: f64 },
}

#[derive(Clone)]
struct World {
    ways: Vec<Way>,
    capture_rate: u32,
    latency_s: f64,
    ppm: f64,
    noise_rms: f64,
    preroll_s: f64,
    faults: Vec<Fault>,
    seed: u64,
}

impl World {
    fn new(ways: Vec<Way>) -> World {
        World {
            ways,
            capture_rate: 48_000,
            latency_s: 0.05,
            ppm: 0.0,
            noise_rms: 1e-4,
            preroll_s: 1.0,
            faults: Vec::new(),
            seed: 7,
        }
    }
}

struct Rendered {
    samples: Vec<f32>,
    first_frame: u64,
    start_frame: u64,
    /// Capture frame (relative) at which device sample 0 is heard without
    /// acoustic delay.
    zero_at: f64,
}

const KHALF: usize = 24;
const KRES: usize = 512;

fn kernel_table() -> Vec<f64> {
    let n = 2 * KHALF * KRES + 1;
    (0..n)
        .map(|i| {
            let x = i as f64 / KRES as f64 - KHALF as f64;
            let r = (x / KHALF as f64 + 1.0) / 2.0;
            let w = 0.42 - 0.5 * (std::f64::consts::TAU * r).cos()
                + 0.08 * (2.0 * std::f64::consts::TAU * r).cos();
            let s = if x.abs() < 1e-12 {
                1.0
            } else {
                (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
            };
            s * w
        })
        .collect()
}

fn interp(table: &[f64], sig: &[f64], t: f64) -> f64 {
    let i0 = t.floor() as i64;
    let mut acc = 0.0;
    for j in (i0 - KHALF as i64 + 1)..=(i0 + KHALF as i64) {
        if j < 0 || j as usize >= sig.len() {
            continue;
        }
        let x = t - j as f64;
        let p = (x + KHALF as f64) * KRES as f64;
        if p < 0.0 || p >= (table.len() - 1) as f64 {
            continue;
        }
        let k = p.floor() as usize;
        let f = p - k as f64;
        let kv = table[k] * (1.0 - f) + table[k + 1] * f;
        acc += sig[j as usize] * kv;
    }
    acc
}

fn conv(a: &[f64], b: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; a.len() + b.len() - 1];
    for (i, &x) in a.iter().enumerate() {
        if x == 0.0 {
            continue;
        }
        for (j, &y) in b.iter().enumerate() {
            out[i + j] += x * y;
        }
    }
    out
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn gauss(&mut self) -> f64 {
        let u1 = self.next().max(1e-300);
        let u2 = self.next();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

/// (output, first cycle filtered, other cycles filtered, delay in samples, gain)
type Filtered = (u8, Vec<f64>, Vec<f64>, f64, f64);

fn render(world: &World, schedule: &WalkSchedule) -> Rendered {
    let sweep = &schedule.sweep;
    let fs_d = f64::from(sweep.rate_hz);
    let fc = f64::from(world.capture_rate);
    let r0 = fc / fs_d;
    let table = kernel_table();
    let first = sweep.render_cycle(true);
    let regular = sweep.render_cycle(false);
    let filtered: Vec<Filtered> = world
        .ways
        .iter()
        .map(|w| {
            (
                w.output,
                conv(&first, &w.fir),
                conv(&regular, &w.fir),
                w.delay_s * fs_d,
                w.gain,
            )
        })
        .collect();
    let period = schedule.period_samples() as f64;
    let cycles = schedule.cycles();
    let pre = world.preroll_s * fc;
    let zero_at = pre + world.latency_s * fc;
    let dev_len = (schedule.total_samples() + u64::from(sweep.gap_samples)) as f64;
    let n = (zero_at + dev_len * r0 + 0.2 * fc).ceil() as usize;
    let mut rng = Rng(world.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let scale = r0 * (1.0 + world.ppm * 1e-6);
    let samples = (0..n)
        .map(|m| {
            let mf = m as f64;
            let mut d = (mf - zero_at) / scale;
            for f in &world.faults {
                match *f {
                    Fault::StepSlip { at_frame, frames } => {
                        if mf >= at_frame {
                            d += frames / r0;
                        }
                    }
                    Fault::DeviceSlip { at_sample, samples } => {
                        if d >= at_sample {
                            d += samples;
                        }
                    }
                    Fault::Wander { amp_us, period_s } => {
                        d += amp_us
                            * 1e-6
                            * fs_d
                            * (std::f64::consts::TAU * mf / (period_s * fc)).sin();
                    }
                }
            }
            let mut v = 0.0;
            if d > -1000.0 {
                let kmax = (d / period).floor() as i64;
                for k in (kmax - 1).max(0)..=kmax.max(0) {
                    let Some(c) = cycles.get(k as usize) else {
                        continue;
                    };
                    let Some((_, sf, sr, delay, gain)) = filtered.iter().find(|w| w.0 == c.output)
                    else {
                        continue;
                    };
                    let sig = if k == 0 { sf } else { sr };
                    let t = d - delay - c.start as f64;
                    if t > -(KHALF as f64) && t < sig.len() as f64 + KHALF as f64 {
                        v += gain * interp(&table, sig, t);
                    }
                }
            }
            (v + world.noise_rms * rng.gauss()) as f32
        })
        .collect();
    Rendered {
        samples,
        first_frame: 1000,
        start_frame: 1000 + pre as u64,
        zero_at,
    }
}

fn sweep_cfg(ms: u32, gap: u16) -> SweepConfig {
    SweepConfig {
        f1_hz: 20.0,
        f2_hz: 20_000.0,
        duration_ms: ms,
        gap_ms: gap,
    }
}

fn delta_way(output: u8, delay_s: f64) -> Way {
    Way {
        output,
        fir: vec![1.0],
        delay_s,
        gain: 0.5,
    }
}

fn analyse(
    world: &World,
    schedule: &WalkSchedule,
    opts: &AnalysisOptions,
) -> Result<WalkedAnalysis, TimingError> {
    let r = render(world, schedule);
    analyse_walked(
        schedule,
        &WalkedCapture {
            samples: &r.samples,
            first_frame: r.first_frame,
            rate_hz: world.capture_rate,
            start_frame: r.start_frame,
            status: &[],
            complete: true,
        },
        opts,
    )
}

fn schedule(rate: u32, ms: u32, gap: u16, mask: u16, repeat: u16) -> WalkSchedule {
    let s = LogSweep::new(rate, &sweep_cfg(ms, gap)).unwrap();
    WalkSchedule::new(s, mask, repeat).unwrap()
}

fn rel_arrival(a: &WalkedAnalysis, output: u8) -> f64 {
    a.outputs
        .iter()
        .find(|o| o.output == output)
        .unwrap()
        .arrival_ms
}

fn fft(x: &[f64], n: usize) -> Vec<C64> {
    let mut buf: Vec<C64> = (0..n)
        .map(|i| C64::new(*x.get(i).unwrap_or(&0.0), 0.0))
        .collect();
    FftPlanner::new().plan_fft_forward(n).process(&mut buf);
    buf
}

// ---------------------------------------------------------------------------
// Frame continuity
// ---------------------------------------------------------------------------

#[test]
fn wp_p5time_frame_continuity_gap_repeat_and_bound() {
    let mut c = FrameContinuity::new();
    assert_eq!(c.next_frame(), None);
    c.push(500, 960).unwrap();
    c.push(1460, 960).unwrap();
    c.push(2420, 0).unwrap();
    assert_eq!(c.frames(), 1920);
    assert_eq!(c.next_frame(), Some(2420));
    assert_eq!(
        c.clone().push(2421, 10),
        Err(FrameFault::Gap {
            expected: 2420,
            got: 2421
        })
    );
    assert_eq!(
        c.clone().push(1460, 960),
        Err(FrameFault::Repeat {
            expected: 2420,
            got: 1460
        })
    );
    let mut big = FrameContinuity::new();
    big.push(0, MAX_CAPTURE_SAMPLES).unwrap();
    assert!(matches!(
        big.push(MAX_CAPTURE_SAMPLES as u64, 1),
        Err(FrameFault::TooLong { .. })
    ));
}

/// The capture faults a client's frame indices show: a dropped block and a
/// repeated block (as a client would report them while recording a run).
#[test]
fn wp_p5time_dropped_and_repeated_blocks_are_seen_by_the_frame_check() {
    let block = 960usize;
    let blocks: Vec<u64> = (0..50).map(|i| 10_000 + i * block as u64).collect();
    // A dropped block: block 20 never arrives.
    let mut c = FrameContinuity::new();
    let mut fault = None;
    for (i, &f) in blocks.iter().enumerate() {
        if i == 20 {
            continue;
        }
        if let Err(e) = c.push(f, block) {
            fault = Some(e);
            break;
        }
    }
    assert!(matches!(fault, Some(FrameFault::Gap { .. })), "{fault:?}");
    // A repeated block: block 20 arrives twice.
    let mut c = FrameContinuity::new();
    let mut fault = None;
    for (i, &f) in blocks.iter().enumerate() {
        let r = c
            .push(f, block)
            .and_then(|()| if i == 20 { c.push(f, block) } else { Ok(()) });
        if let Err(e) = r {
            fault = Some(e);
            break;
        }
    }
    assert!(
        matches!(fault, Some(FrameFault::Repeat { .. })),
        "{fault:?}"
    );
}

// ---------------------------------------------------------------------------
// Deconvolution
// ---------------------------------------------------------------------------

#[test]
fn wp_p5time_known_impulse_response_is_recovered_below_minus_60_db() {
    let s = LogSweep::new(48_000, &sweep_cfg(1_000, 300)).unwrap();
    let x = s.render_cycle(false);
    let mut rng = Rng(99);
    let h: Vec<f64> = (0..256)
        .map(|i| rng.gauss() * (-(i as f64) / 40.0).exp())
        .collect();
    let y = conv(&x, &h);
    let est = deconvolve(&y, &x, 48_000.0, (s.start_freq_hz(), s.end_freq_hz())).unwrap();
    let n = est.len();
    let he = fft(&est, n);
    let ht = fft(&h, n);
    let (lo, hi) = (40.0, 16_000.0);
    let (mut err, mut sig) = (0.0, 0.0);
    for k in 0..n / 2 {
        let f = k as f64 * 48_000.0 / n as f64;
        if f < lo || f > hi {
            continue;
        }
        err += (he[k] - ht[k]).norm_sqr();
        sig += ht[k].norm_sqr();
    }
    let db = 10.0 * (err / sig).log10();
    assert!(db < -60.0, "error {db:.1} dB");
}

#[test]
fn wp_p5time_deconvolve_refuses_bad_input() {
    assert!(deconvolve(&[], &[1.0], 48_000.0, (20.0, 20_000.0)).is_err());
    assert!(deconvolve(&[1.0], &[f64::NAN], 48_000.0, (20.0, 20_000.0)).is_err());
    assert!(deconvolve(&[1.0], &[1.0], 0.0, (20.0, 20_000.0)).is_err());
    assert!(matches!(
        deconvolve(&[1.0], &[0.0], 48_000.0, (20.0, 20_000.0)),
        Err(MeasureError::NoSignal(_))
    ));
    assert!(
        deconvolve(
            &vec![0.0; MAX_FFT_LEN + 1],
            &[1.0],
            48_000.0,
            (20.0, 20_000.0)
        )
        .is_err()
    );
}

// ---------------------------------------------------------------------------
// The timing model
// ---------------------------------------------------------------------------

#[test]
fn wp_p5time_clock_100_ppm_measured_within_0_2_ppm_over_18_s_and_removed() {
    // 2 outputs x 3 passes x (2.7 s + 0.3 s): 18 s from the first cycle's start
    // to the run's end.
    let sch = schedule(48_000, 2_700, 300, 0b11, 3);
    assert_eq!(
        sch.total_samples() + u64::from(sch.sweep.gap_samples),
        18 * 48_000
    );
    let mut world = World::new(vec![delta_way(0, 0.001), delta_way(1, 0.0015)]);
    world.ppm = 100.0;
    let opts = AnalysisOptions::default();
    let a = analyse(&world, &sch, &opts).unwrap();
    assert!(a.clock.fitted);
    assert!(
        (a.clock.ratio_ppm - 100.0).abs() < 0.2,
        "{} ppm",
        a.clock.ratio_ppm
    );
    assert!(a.quality.reliable, "{:?}", a.quality);
    assert!(
        a.quality.max_residual_us < 1.0,
        "{}",
        a.quality.max_residual_us
    );
    // Removed: the drifting run's high frequencies equal a run without drift.
    world.ppm = 0.0;
    let b = analyse(&world, &sch, &opts).unwrap();
    let freqs = [1_000.0, 5_000.0, 10_000.0, 15_000.0];
    for (oa, ob) in a.outputs.iter().zip(&b.outputs) {
        let ra = fdw_response(&oa.ir, oa.peak_index, &freqs, ALIGNMENT_WINDOW_CYCLES).unwrap();
        let rb = fdw_response(&ob.ir, ob.peak_index, &freqs, ALIGNMENT_WINDOW_CYCLES).unwrap();
        for ((va, vb), f) in ra.values.iter().zip(&rb.values).zip(freqs) {
            let db = 20.0 * (va.norm() / vb.norm()).log10();
            let deg = (va * vb.conj()).arg().to_degrees();
            assert!(db.abs() < 0.1, "{f} Hz: {db} dB");
            assert!(deg.abs() < 3.0, "{f} Hz: {deg}°");
        }
    }
}

#[test]
fn wp_p5time_two_outputs_1_234_ms_apart_within_5_us_with_latency_and_80_ppm() {
    let sch = schedule(48_000, 2_000, 300, 0b101, 3);
    let mut world = World::new(vec![
        delta_way(0, 0.000_3),
        delta_way(2, 0.000_3 + 0.001_234),
    ]);
    world.ppm = 80.0;
    world.latency_s = 0.05;
    let a = analyse(&world, &sch, &AnalysisOptions::default()).unwrap();
    assert!(a.quality.reliable, "{:?}", a.quality);
    let d = rel_arrival(&a, 2) - rel_arrival(&a, 0);
    assert!((d - 1.234).abs() < 0.005, "{d} ms");
    assert!(
        (a.clock.ratio_ppm - 80.0).abs() < 1.0,
        "{}",
        a.clock.ratio_ppm
    );
    // The phase holds the same delay: at 2 kHz the two responses differ by
    // 2π·f·1.234 ms (mod 360°).
    let f = [2_000.0];
    let r0 = fdw_response(&a.outputs[0].ir, a.outputs[0].peak_index, &f, 5.0).unwrap();
    let r2 = fdw_response(&a.outputs[1].ir, a.outputs[1].peak_index, &f, 5.0).unwrap();
    let measured = (r0.values[0] * r2.values[0].conj()).arg().to_degrees();
    let expected = (360.0 * 2_000.0 * 0.001_234_f64).rem_euclid(360.0);
    let diff = (measured - expected + 540.0).rem_euclid(360.0) - 180.0;
    assert!(diff.abs() < 1.0, "phase {measured} vs {expected}");
}

#[test]
fn wp_p5time_three_repeats_raise_the_snr_by_4_8_db() {
    let noise = 0.02;
    let mut world = World::new(vec![delta_way(0, 0.001)]);
    world.noise_rms = noise;
    let noise_power = |a: &WalkedAnalysis| -> f64 {
        let o = &a.outputs[0];
        let start = o.ir.zero_index + (0.03 * 48_000.0) as usize;
        let tail = &o.ir.samples[start..];
        tail.iter().map(|v| v * v).sum::<f64>() / tail.len() as f64
    };
    let peak =
        |a: &WalkedAnalysis| interp_at(&a.outputs[0].ir.samples, a.outputs[0].peak_index).abs();
    let one = analyse(
        &world,
        &schedule(48_000, 1_000, 300, 1, 1),
        &AnalysisOptions::default(),
    )
    .unwrap();
    let three = analyse(
        &world,
        &schedule(48_000, 1_000, 300, 1, 3),
        &AnalysisOptions::default(),
    )
    .unwrap();
    let snr1 = 10.0 * (peak(&one).powi(2) / noise_power(&one)).log10();
    let snr3 = 10.0 * (peak(&three).powi(2) / noise_power(&three)).log10();
    let gain = snr3 - snr1;
    assert!(
        (gain - 4.77).abs() < 0.5,
        "{snr1:.2} -> {snr3:.2}: {gain:.2} dB"
    );
    assert!(!one.quality.reliable, "a single pass is magnitude only");
    assert!(!one.clock.fitted);
    // The noise floor estimate follows the averaging too.
    let n1 = one.outputs[0].noise_db.as_ref().unwrap()[500];
    let n3 = three.outputs[0].noise_db.as_ref().unwrap()[500];
    assert!(((n1 - n3) - 4.77).abs() < 1.0, "{n1} {n3}");
}

#[test]
fn wp_p5time_noise_floor_predicts_the_response_noise() {
    // The documented scale: noise power of a DFT under a window is
    // 10^(noise_db/10)·Σw². Check it against the tail of a noisy response.
    let mut world = World::new(vec![delta_way(0, 0.001)]);
    world.noise_rms = 0.01;
    let a = analyse(
        &world,
        &schedule(48_000, 1_000, 300, 1, 3),
        &AnalysisOptions::default(),
    )
    .unwrap();
    let o = &a.outputs[0];
    let start = o.ir.zero_index + (0.03 * 48_000.0) as usize;
    let tail = ImpulseResponse {
        rate_hz: 48_000,
        samples: o.ir.samples[start..].to_vec(),
        zero_index: 0,
    };
    let grid = LogGrid::STANDARD.freqs().unwrap();
    // Average over 1/3 octave around 1 kHz.
    let idx: Vec<usize> = (0..grid.len())
        .filter(|&i| grid[i] > 890.0 && grid[i] < 1120.0)
        .collect();
    let f: Vec<f64> = idx.iter().map(|&i| grid[i]).collect();
    let r = response_at(&tail, &f).unwrap();
    let measured: f64 = r.values.iter().map(|v| v.norm_sqr()).sum::<f64>() / f.len() as f64;
    let noise = o.noise_db.as_ref().unwrap();
    let predicted: f64 = idx
        .iter()
        .map(|&i| 10f64.powf(noise[i] / 10.0) * tail.samples.len() as f64)
        .sum::<f64>()
        / f.len() as f64;
    let db = 10.0 * (measured / predicted).log10();
    assert!(db.abs() < 1.5, "measured vs predicted noise: {db:.2} dB");
}

#[test]
fn wp_p5time_two_passes_are_unreliable_by_default_and_fit_when_allowed() {
    let sch = schedule(48_000, 1_000, 300, 0b11, 2);
    let world = World::new(vec![delta_way(0, 0.001), delta_way(1, 0.002)]);
    let a = analyse(&world, &sch, &AnalysisOptions::default()).unwrap();
    assert!(!a.quality.reliable);
    assert!(
        a.quality.reasons.iter().any(|r| r.contains("2 passes")),
        "{:?}",
        a.quality.reasons
    );
    let opts = AnalysisOptions {
        min_passes_reliable: 2,
        ..AnalysisOptions::default()
    };
    let a = analyse(&world, &sch, &opts).unwrap();
    assert!(a.quality.reliable, "{:?}", a.quality);
    assert!((rel_arrival(&a, 1) - 1.0).abs() < 0.005);
}

#[test]
fn wp_p5time_capture_at_another_rate_than_the_device() {
    let sch = schedule(48_000, 1_000, 300, 0b11, 3);
    let mut world = World::new(vec![delta_way(0, 0.0005), delta_way(1, 0.0015)]);
    world.capture_rate = 44_100;
    world.ppm = -30.0;
    let a = analyse(&world, &sch, &AnalysisOptions::default()).unwrap();
    assert!(a.quality.reliable, "{:?}", a.quality);
    assert!(
        (a.clock.ratio_ppm + 30.0).abs() < 1.0,
        "{}",
        a.clock.ratio_ppm
    );
    let d = rel_arrival(&a, 1) - rel_arrival(&a, 0);
    assert!((d - 1.0).abs() < 0.005, "{d}");
}

// ---------------------------------------------------------------------------
// Capture faults and the quality gate
// ---------------------------------------------------------------------------

/// Runs a three-output world with a fault; returns (reliable, the largest
/// delay error in µs).
fn faulty(fault: Fault) -> (bool, f64, WalkedAnalysis) {
    let sch = schedule(48_000, 1_000, 300, 0b111, 3);
    let delays = [0.0004, 0.0011, 0.0023];
    let mut world = World::new(vec![
        delta_way(0, delays[0]),
        delta_way(1, delays[1]),
        delta_way(2, delays[2]),
    ]);
    world.ppm = 40.0;
    world.faults = vec![fault];
    let a = analyse(&world, &sch, &AnalysisOptions::default()).unwrap();
    let mut worst: f64 = 0.0;
    for i in 0..3u8 {
        for j in 0..3u8 {
            let got = rel_arrival(&a, j) - rel_arrival(&a, i);
            let truth = (delays[j as usize] - delays[i as usize]) * 1000.0;
            worst = worst.max((got - truth).abs() * 1000.0);
        }
    }
    (a.quality.reliable, worst, a)
}

#[test]
fn wp_p5time_invisible_capture_faults_never_give_a_delay_off_by_10_us_unflagged() {
    // Cycle starts are every 1.3 s (62 400 frames at 48 kHz), after 1.05 s of
    // pre-roll and latency: probe slips between cycles and inside them.
    let zero = 1.05 * 48_000.0;
    let period = 62_400.0;
    let mut faults = Vec::new();
    for k in [1, 4, 7] {
        let gap_mid = zero + k as f64 * period - 0.15 * 48_000.0;
        let sweep_mid = zero + k as f64 * period + 0.5 * 48_000.0;
        for frames in [1.0, 3.0] {
            faults.push(Fault::StepSlip {
                at_frame: gap_mid,
                frames,
            });
            faults.push(Fault::StepSlip {
                at_frame: sweep_mid,
                frames,
            });
        }
        faults.push(Fault::DeviceSlip {
            at_sample: k as f64 * period - 0.15 * 48_000.0,
            samples: 48.0,
        });
        faults.push(Fault::DeviceSlip {
            at_sample: k as f64 * period + 0.5 * 48_000.0,
            samples: 32.0,
        });
    }
    for (amp, period_s) in [(2.0, 40.0), (20.0, 8.0), (60.0, 5.0)] {
        faults.push(Fault::Wander {
            amp_us: amp,
            period_s,
        });
    }
    for f in faults {
        let (reliable, worst, a) = faulty(f);
        assert!(
            !reliable || worst <= 10.0,
            "{f:?}: reliable with a delay {worst:.1} µs off ({:?})",
            a.quality
        );
    }
}

#[test]
fn wp_p5time_each_invisible_fault_kind_is_flagged_by_the_gate() {
    let zero = 1.05 * 48_000.0;
    let period = 62_400.0;
    for f in [
        Fault::StepSlip {
            at_frame: zero + 4.0 * period - 7_200.0,
            frames: 3.0,
        },
        Fault::StepSlip {
            at_frame: zero + 4.0 * period + 24_000.0,
            frames: 1.0,
        },
        Fault::DeviceSlip {
            at_sample: 4.0 * period - 7_200.0,
            samples: 48.0,
        },
        Fault::Wander {
            amp_us: 60.0,
            period_s: 5.0,
        },
    ] {
        let (reliable, _, a) = faulty(f);
        assert!(!reliable, "{f:?} was not flagged: {:?}", a.quality);
    }
}

#[test]
fn wp_p5time_the_residual_bound_keeps_any_single_slip_within_10_us() {
    // The fit itself: for every layout of 1 to 8 outputs and 3 to 8 passes and
    // every place a slip can fall between cycles, a slip just small enough to
    // pass the run's residual bound moves no delay by more than 10 µs.
    for outputs in 1..=8u32 {
        for repeat in 3..=8u16 {
            let mask = ((1u32 << outputs) - 1) as u16;
            let sch = schedule(48_000, 1_000, 300, mask, repeat);
            let kappa = slip_sensitivity(&sch);
            assert!(
                kappa.is_finite() && kappa < 1.6,
                "{outputs}x{repeat}: {kappa}"
            );
            let bound = MAX_RESIDUAL_US / kappa.max(1.0);
            let n = sch.cycles().len();
            for slip_at in 1..n {
                let step: Vec<(usize, f64, f64)> = sch
                    .cycles()
                    .iter()
                    .enumerate()
                    .map(|(k, c)| {
                        (
                            c.output as usize,
                            c.start as f64,
                            if k >= slip_at { 1.0 } else { 0.0 },
                        )
                    })
                    .collect();
                let fit = fit_common_slope(&step, outputs as usize).unwrap();
                let res = step
                    .iter()
                    .map(|&(o, x, y)| (y - fit.intercepts[o].unwrap() - fit.slope * x).abs())
                    .fold(0.0, f64::max);
                if res < 1e-12 {
                    continue;
                }
                let a: Vec<f64> = fit.intercepts.iter().map(|v| v.unwrap()).collect();
                let err = a.iter().copied().fold(f64::MIN, f64::max)
                    - a.iter().copied().fold(f64::MAX, f64::min);
                // Scale the slip so its largest residual sits on the bound.
                let err_us = err * bound / res;
                assert!(
                    err_us <= MAX_RESIDUAL_US + 1e-9,
                    "{outputs}x{repeat} at {slip_at}: {err_us} µs"
                );
            }
        }
    }
    // Two passes cannot tell a slip between them from a clock ratio.
    assert!(slip_sensitivity(&schedule(48_000, 1_000, 300, 0b111, 2)).is_infinite());
    let k = slip_sensitivity(&schedule(48_000, 1_000, 300, 0b1111, 3));
    assert!((k - 1.125).abs() < 1e-6, "{k}");
}

// ---------------------------------------------------------------------------
// Cycle identity
// ---------------------------------------------------------------------------

fn identity_case(
    start_shift_cycles: f64,
    status: &[StatusObservation],
) -> Result<WalkedAnalysis, TimingError> {
    let sch = schedule(48_000, 1_000, 300, 0b11, 3);
    let world = World::new(vec![delta_way(0, 0.001), delta_way(1, 0.002)]);
    let r = render(&world, &sch);
    let period_frames = sch.period_samples() as f64;
    let start = (r.start_frame as f64 + start_shift_cycles * period_frames) as u64;
    analyse_walked(
        &sch,
        &WalkedCapture {
            samples: &r.samples,
            first_frame: r.first_frame,
            rate_hz: 48_000,
            start_frame: start,
            status,
            complete: true,
        },
        &AnalysisOptions::default(),
    )
}

#[test]
fn wp_p5time_start_frame_one_cycle_off_is_a_cycle_identity_failure() {
    assert!(identity_case(0.0, &[]).is_ok());
    for shift in [1.0, -1.0] {
        let e = identity_case(shift, &[]).unwrap_err();
        assert!(matches!(e, TimingError::CycleIdentity(_)), "{shift}: {e:?}");
    }
}

#[test]
fn wp_p5time_status_polls_must_agree_with_the_capture() {
    let sch = schedule(48_000, 1_000, 300, 0b11, 3);
    let world = World::new(vec![delta_way(0, 0.001), delta_way(1, 0.002)]);
    let r = render(&world, &sch);
    let zero = r.first_frame as f64 + r.zero_at;
    let at = |dev: f64| (zero + dev) as u64;
    let period = sch.period_samples() as f64;
    let good = [
        StatusObservation {
            capture_frame: at(0.5 * period),
            status: GeneratorStatus {
                active_channel: 0,
                cycles_done: 0,
            },
        },
        StatusObservation {
            capture_frame: at(2.4 * period),
            status: GeneratorStatus {
                active_channel: 0,
                cycles_done: 2,
            },
        },
        StatusObservation {
            capture_frame: at(7.0 * period),
            status: GeneratorStatus {
                active_channel: NO_CHANNEL,
                cycles_done: 6,
            },
        },
    ];
    let cap = |status: &[StatusObservation]| {
        analyse_walked(
            &sch,
            &WalkedCapture {
                samples: &r.samples,
                first_frame: r.first_frame,
                rate_hz: 48_000,
                start_frame: r.start_frame,
                status,
                complete: true,
            },
            &AnalysisOptions::default(),
        )
    };
    assert!(cap(&good).is_ok());
    // The device says output 1 while the capture is in output 0's cycle.
    let bad = [StatusObservation {
        capture_frame: at(2.4 * period),
        status: GeneratorStatus {
            active_channel: 1,
            cycles_done: 3,
        },
    }];
    assert!(matches!(cap(&bad), Err(TimingError::CycleIdentity(_))));
}

#[test]
fn wp_p5time_a_silent_output_is_a_cycle_identity_failure() {
    let sch = schedule(48_000, 1_000, 300, 0b11, 3);
    let mut world = World::new(vec![delta_way(0, 0.001), delta_way(1, 0.002)]);
    world.ways[1].gain = 0.0;
    let e = analyse(&world, &sch, &AnalysisOptions::default()).unwrap_err();
    assert!(matches!(e, TimingError::CycleIdentity(_)), "{e:?}");
}

#[test]
fn wp_p5time_sliced_analysis_waits_for_capture_and_matches_the_whole() {
    let sch = schedule(48_000, 1_000, 300, 0b11, 3);
    let world = World::new(vec![delta_way(0, 0.001), delta_way(1, 0.0025)]);
    let r = render(&world, &sch);
    let mut an = WalkedAnalyser::new(&sch, 48_000, &AnalysisOptions::default()).unwrap();
    let mut have = 3 * 48_000usize;
    let mut waits = 0;
    let result = loop {
        let cap = WalkedCapture {
            samples: &r.samples[..have.min(r.samples.len())],
            first_frame: r.first_frame,
            rate_hz: 48_000,
            start_frame: r.start_frame,
            status: &[],
            complete: have >= r.samples.len(),
        };
        match an.step(&cap).unwrap() {
            Step::Done(a) => break *a,
            Step::NeedCapture => {
                waits += 1;
                have += 48_000;
            }
            Step::Progress { done, total } => assert!(done <= total),
        }
    };
    assert!(waits > 0);
    let whole = analyse(&world, &sch, &AnalysisOptions::default()).unwrap();
    assert_eq!(result, whole);
    assert!(
        an.step(&WalkedCapture {
            samples: &r.samples,
            first_frame: r.first_frame,
            rate_hz: 48_000,
            start_frame: r.start_frame,
            status: &[],
            complete: true,
        })
        .is_err()
    );
}

#[test]
fn wp_p5time_analyser_refuses_bad_setups() {
    let sch = schedule(48_000, 1_000, 300, 1, 3);
    assert!(WalkedAnalyser::new(&sch, 4_000, &AnalysisOptions::default()).is_err());
    let bad = AnalysisOptions {
        search_ms: f64::NAN,
        ..AnalysisOptions::default()
    };
    assert!(WalkedAnalyser::new(&sch, 48_000, &bad).is_err());
    let bad = AnalysisOptions {
        min_passes_reliable: 1,
        ..AnalysisOptions::default()
    };
    assert!(WalkedAnalyser::new(&sch, 48_000, &bad).is_err());
    let short_gap = schedule(48_000, 1_000, 50, 1, 3);
    assert!(WalkedAnalyser::new(&short_gap, 48_000, &AnalysisOptions::default()).is_err());
    let nan = vec![f32::NAN; 100_000];
    let e = analyse_walked(
        &sch,
        &WalkedCapture {
            samples: &nan,
            first_frame: 0,
            rate_hz: 48_000,
            start_frame: 48_000,
            status: &[],
            complete: true,
        },
        &AnalysisOptions::default(),
    );
    assert!(matches!(
        e,
        Err(TimingError::Measure(MeasureError::Invalid(_)))
    ));
}

// ---------------------------------------------------------------------------
// Windows and peaks
// ---------------------------------------------------------------------------

#[test]
fn wp_p5time_windows_of_a_delayed_impulse() {
    let mut samples = vec![0.0; 4_800];
    samples[1_000] = 1.0;
    let ir = ImpulseResponse {
        rate_hz: 48_000,
        samples,
        zero_index: 1_000,
    };
    assert_eq!(ir.peak_index(), Some(1_000.0));
    let f = [100.0, 1_000.0, 10_000.0];
    for cycles in [ALIGNMENT_WINDOW_CYCLES, FIT_WINDOW_CYCLES] {
        let r = fdw_response(&ir, 1_000.0, &f, cycles).unwrap();
        for (v, s2) in r.values.iter().zip(&r.sum_w2) {
            assert!((v.norm() - 1.0).abs() < 1e-9 && v.arg().abs() < 1e-9, "{v}");
            assert!(*s2 > 0.0);
        }
        // A wider window at low frequencies.
        assert!(r.sum_w2[0] > r.sum_w2[2]);
    }
    let g = fixed_gate(&ir, 1_000.0, 5.0, 20.0, 2.0).unwrap();
    assert_eq!(g.samples[1_000], 1.0);
    let g = fixed_gate(&ir, 2_000.0, 5.0, 10.0, 1.0).unwrap();
    assert_eq!(g.samples[1_000], 0.0, "outside the gate");
    let r = response_at(&ir, &f).unwrap();
    assert!(r.values.iter().all(|v| (v.norm() - 1.0).abs() < 1e-9));
    assert!(fdw_response(&ir, 1e9, &f, 5.0).is_err());
    assert!(fdw_response(&ir, 10.0, &[30_000.0], 5.0).is_err());
    assert!(fdw_response(&ir, 10.0, &f, 0.5).is_err());
    assert!(fixed_gate(&ir, 10.0, -1.0, 1.0, 1.0).is_err());
    let silent = ImpulseResponse {
        rate_hz: 48_000,
        samples: vec![0.0; 10],
        zero_index: 0,
    };
    assert_eq!(silent.peak_index(), None);
}

#[test]
fn wp_p5time_peak_index_finds_a_fractional_peak() {
    for frac in [0.0, 0.17, 0.5, 0.83] {
        let t0 = 200.0 + frac;
        let samples: Vec<f64> = (0..400)
            .map(|i| sinc((i as f64 - t0) * 0.9) * 0.9)
            .collect();
        let ir = ImpulseResponse {
            rate_hz: 48_000,
            samples,
            zero_index: 0,
        };
        let p = ir.peak_index().unwrap();
        assert!((p - t0).abs() < 0.01, "{p} vs {t0}");
    }
}

#[test]
fn wp_p5time_xcorr_lag_and_fft_len() {
    let a: Vec<f64> = (0..400).map(|i| sinc((i as f64 - 200.0) * 0.8)).collect();
    let b: Vec<f64> = (0..400).map(|i| sinc((i as f64 - 203.37) * 0.8)).collect();
    let lag = xcorr_lag(&a, &b, 20.0).unwrap();
    assert!((lag - 3.37).abs() < 0.01, "{lag}");
    assert_eq!(fft_len(1), Some(1));
    assert_eq!(fft_len(1001), Some(1024));
    assert_eq!(fft_len(140_000), Some(140_625));
    assert_eq!(fft_len(MAX_FFT_LEN + 1), None);
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn proptest_wp_p5time_frame_continuity_never_panics(blocks in proptest::collection::vec((any::<u64>(), 0usize..100_000), 0..50)) {
        let mut c = FrameContinuity::new();
        for (f, l) in blocks {
            let before = c.clone();
            if c.push(f, l).is_err() {
                prop_assert_eq!(&c, &before);
            }
        }
        prop_assert!(c.frames() <= MAX_CAPTURE_SAMPLES as u64);
    }

    #[test]
    fn proptest_wp_p5time_windows_never_panic(
        samples in proptest::collection::vec(-1e6f64..1e6, 1..600),
        centre in proptest::num::f64::ANY,
        freqs in proptest::collection::vec(proptest::num::f64::ANY, 0..20),
        cycles in proptest::num::f64::ANY,
        l in proptest::num::f64::ANY, r in proptest::num::f64::ANY, t in proptest::num::f64::ANY,
        zero in 0usize..1000,
    ) {
        let ir = ImpulseResponse { rate_hz: 48_000, samples, zero_index: zero };
        if let Some(p) = ir.peak_index() { prop_assert!(p.is_finite()); }
        if let Ok(w) = fdw_response(&ir, centre, &freqs, cycles) {
            prop_assert!(w.values.iter().all(|v| v.re.is_finite() && v.im.is_finite()));
            prop_assert!(w.sum_w2.iter().all(|v| v.is_finite()));
        }
        if let Ok(w) = response_at(&ir, &freqs) {
            prop_assert!(w.values.iter().all(|v| v.re.is_finite() && v.im.is_finite()));
        }
        if let Ok(g) = fixed_gate(&ir, centre, l, r, t) {
            prop_assert!(g.samples.iter().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn proptest_wp_p5time_deconvolve_and_xcorr_never_panic(
        rec in proptest::collection::vec(-10.0f64..10.0, 0..300),
        stim in proptest::collection::vec(-1.0f64..1.0, 0..300),
        lo in proptest::num::f64::ANY, hi in proptest::num::f64::ANY,
        lag in proptest::num::f64::ANY,
    ) {
        if let Ok(h) = deconvolve(&rec, &stim, 48_000.0, (lo, hi)) {
            prop_assert!(h.iter().all(|v| v.is_finite()));
        }
        if let Some(l) = xcorr_lag(&rec, &stim, lag.clamp(0.0, 100.0)) {
            prop_assert!(l.is_finite());
        }
        let pts: Vec<(usize, f64, f64)> = rec.iter().zip(&stim).enumerate().map(|(i, (a, b))| (i % 3, *a, *b)).collect();
        if let Some(f) = fit_common_slope(&pts, 3) {
            prop_assert!(f.slope.is_finite());
        }
    }
}

/// Runs a property test whose case is expensive: `PROPTEST_CASES` applies
/// (as `ci/check.sh --step proptest` sets it) but at most `max` cases
/// (PLAN.md section 1.9, as `cs_rules.rs`'s `walk_cases`).
fn run_capped<S: Strategy>(
    name: &'static str,
    default: u32,
    max: u32,
    strategy: S,
    test: impl Fn(S::Value) -> Result<(), TestCaseError>,
) {
    let mut config =
        proptest::test_runner::contextualize_config(ProptestConfig::with_cases(default));
    config.cases = config.cases.min(max);
    config.source_file = Some(file!());
    config.test_name = Some(name);
    let mut runner = proptest::test_runner::TestRunner::new(config);
    if let Err(e) = runner.run(&strategy, test) {
        panic!("{name}: {e}");
    }
}

#[test]
fn proptest_wp_p5time_analysis_never_panics_or_returns_non_finite() {
    // Each case renders and analyses a run: 6 cases, at most 40 under
    // `ci/check.sh --step proptest`.
    let strategy = (
        prop_oneof![Just(44_100u32), Just(48_000)],
        prop_oneof![Just(44_100u32), Just(48_000), Just(96_000)],
        1u16..4,
        1u16..3,
        -300.0f64..300.0,
        0.0f64..0.3,
        prop_oneof![Just(0.0f64), 1e-5f64..0.3],
        -60_000i64..60_000,
        any::<bool>(),
        any::<u64>(),
        prop_oneof![Just(0.0f64), 1e-4f64..2.0],
    );
    run_capped(
        "proptest_wp_p5time_analysis_never_panics_or_returns_non_finite",
        6,
        40,
        strategy,
        |(rate, cap_rate, mask, repeat, ppm, latency, noise, start_shift, garbage, seed, gain)| {
            let sch = schedule(rate, 1_000, 300, mask, repeat);
            let ways = sch
                .outputs()
                .iter()
                .map(|&o| Way {
                    output: o,
                    fir: vec![1.0, 0.3],
                    delay_s: 0.001 * f64::from(o),
                    gain,
                })
                .collect();
            let mut world = World::new(ways);
            world.capture_rate = cap_rate;
            world.ppm = ppm;
            world.latency_s = latency;
            world.noise_rms = noise;
            world.seed = seed;
            let mut r = render(&world, &sch);
            if garbage {
                let mut rng = Rng(seed | 1);
                for v in r.samples.iter_mut().step_by(7) {
                    *v = (rng.next() * 4.0 - 2.0) as f32;
                }
            }
            let start = (r.start_frame as i64 + start_shift).max(0) as u64;
            let out = analyse_walked(
                &sch,
                &WalkedCapture {
                    samples: &r.samples,
                    first_frame: r.first_frame,
                    rate_hz: cap_rate,
                    start_frame: start,
                    status: &[],
                    complete: true,
                },
                &AnalysisOptions::default(),
            );
            if let Ok(a) = out {
                prop_assert!(a.clock.ratio_ppm.is_finite());
                prop_assert!(a.quality.max_residual_us.is_finite());
                prop_assert!(a.quality.max_phase_spread_deg.is_finite());
                for o in &a.outputs {
                    prop_assert!(o.ir.samples.iter().all(|v| v.is_finite()));
                    prop_assert!(o.peak_index.is_finite() && o.arrival_ms.is_finite());
                    prop_assert!(o.spread_db.iter().all(|v| v.is_finite()));
                    if let Some(n) = &o.noise_db {
                        prop_assert!(n.iter().all(|v| v.is_finite()));
                    }
                }
                for c in &a.cycles {
                    prop_assert!(c.residual_us.is_finite() && c.arrival_frame.is_finite());
                }
            }
            Ok(())
        },
    );
}
