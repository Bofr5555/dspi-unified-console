//! WP P5ARRIVAL: the arrival from the noise-weighted envelope, the
//! leave-one-out refinement, the phase gate per band and the wander
//! diagnostic, against P5TIME's test world and the first real captures
//! (P5SPIKE, `testdata/p5/spike/`).
#![allow(clippy::arithmetic_side_effects)]

use super::tests_wp_p5time::{Fault, Rendered, Way, World, analyse, render, run_capped, schedule};
use super::*;
use proptest::prelude::*;

const RATE: f64 = 48_000.0;

/// Two band-limited lobes of opposite sign `gap_us` apart, the second
/// `second` times the first.
fn two_lobes(gap_us: f64, second: f64) -> Vec<f64> {
    let taps = 160usize;
    let t1 = 12.0;
    let t2 = t1 + gap_us * 1e-6 * RATE;
    let fc = 16_000.0 / RATE;
    let bl = |x: f64| {
        let w = if x.abs() < 12.0 {
            0.5 + 0.5 * (std::f64::consts::PI * x / 12.0).cos()
        } else {
            0.0
        };
        2.0 * fc * sinc(2.0 * fc * x) * w
    };
    (0..taps)
        .map(|i| bl(i as f64 - t1) - second * bl(i as f64 - t2))
        .collect()
}

/// A tweeter-like way: [`two_lobes`] through a second-order high-pass at
/// 3.5 kHz.
fn tweeter_fir(gap_us: f64, second: f64) -> Vec<f64> {
    let raw = two_lobes(gap_us, second);
    // RBJ high-pass, 3.5 kHz, Q 0.707.
    let w0 = std::f64::consts::TAU * 3_500.0 / RATE;
    let alpha = w0.sin() / (2.0 * 0.707);
    let cw = w0.cos();
    let a0 = 1.0 + alpha;
    let (b0, b1, b2) = (
        (1.0 + cw) / 2.0 / a0,
        -(1.0 + cw) / a0,
        (1.0 + cw) / 2.0 / a0,
    );
    let (a1, a2) = (-2.0 * cw / a0, (1.0 - alpha) / a0);
    let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
    raw.iter()
        .map(|&x| {
            let y = b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
            x2 = x1;
            x1 = x;
            y2 = y1;
            y1 = y;
            y
        })
        .collect()
}

/// P5TIME's refinement, as it was before WP P5ARRIVAL: the coarse arrival
/// at the largest `|h|` of the full-band response, refined by a 16x
/// cross-correlation of ±10 ms excerpts against the output's **first**
/// cycle, then the common-slope fit. The cycles are cut where `a` heard
/// them. Returns each cycle's residual, µs.
fn old_residuals(r: &Rendered, sch: &WalkSchedule, a: &WalkedAnalysis) -> Vec<f64> {
    let rate = f64::from(a.capture_rate_hz);
    let pre = (0.05 * rate) as i64;
    let sn = (0.010 * rate) as i64;
    let sweep = &sch.sweep;
    let len = (f64::from(sweep.cycle_samples) * rate / f64::from(sweep.rate_hz)) as usize
        + (0.2 * rate) as usize;
    let band = a.band_hz;
    let mut found: Vec<(f64, i64, Vec<f64>)> = Vec::new();
    for (k, c) in a.cycles.iter().enumerate() {
        let centre = (c.arrival_frame - r.first_frame as f64).round() as i64;
        let start = centre - pre;
        let rec: Vec<f64> = (start..start + len as i64)
            .map(|i| f64::from(*r.samples.get(i.max(0) as usize).unwrap_or(&0.0)))
            .collect();
        let stim = sweep.render_at_rate(rate as u32, k == 0).unwrap();
        let h = deconvolve(&rec, &stim, rate, band).unwrap();
        let (idx, _) = h
            .iter()
            .enumerate()
            .take((2 * pre + 1) as usize)
            .map(|(i, v)| (i, v * v))
            .max_by(|x, y| x.1.total_cmp(&y.1))
            .unwrap();
        let i = idx as i64;
        let frac = parabolic(
            circ(&h, i - 1).abs(),
            circ(&h, i).abs(),
            circ(&h, i + 1).abs(),
        );
        let snip: Vec<f64> = (i - sn..=i + sn).map(|j| circ(&h, j)).collect();
        found.push((start as f64 + idx as f64 + frac, start + i - sn, snip));
    }
    let outputs = sch.outputs();
    let cycles = sch.cycles();
    let mut pts = Vec::new();
    for (k, c) in cycles.iter().enumerate() {
        let first = cycles.iter().position(|cc| cc.output == c.output).unwrap();
        let (coarse, ss, snip) = &found[k];
        let y = if first == k {
            *coarse
        } else {
            let (c0, ss0, snip0) = &found[first];
            let base = c0 + (*ss - *ss0) as f64;
            xcorr_lag(snip0, snip, sn as f64).map_or(*coarse, |lag| base + lag)
        };
        let o = outputs.iter().position(|&x| x == c.output).unwrap();
        pts.push((o, c.start as f64, y));
    }
    let fit = fit_common_slope(&pts, outputs.len()).unwrap();
    pts.iter()
        .map(|&(o, n, y)| (y - fit.intercepts[o].unwrap() - fit.slope * n) * 1e6 / rate)
        .collect()
}

fn max_abs(v: impl IntoIterator<Item = f64>) -> f64 {
    v.into_iter().fold(0.0, |m, x| m.max(x.abs()))
}

/// The tweeter world: one two-lobe way, five passes of the default 5.5 s
/// sweep, the microphone 9 ppm off (as P5SPIKE's), its clock wandering by
/// 75 ppm (±12 µs over 1 s periods, P5SPIKE saw 55 to 100 ppm).
fn tweeter_world(wander: bool) -> (World, WalkSchedule) {
    let sch = schedule(48_000, 5_500, 300, 1, 5);
    let mut world = World::new(vec![Way {
        output: 0,
        fir: tweeter_fir(47.0, 0.85),
        delay_s: 0.0004,
        gain: 0.5,
    }]);
    world.ppm = -9.0;
    world.noise_rms = 3e-4;
    world.seed = 5;
    if wander {
        world.faults = vec![Fault::Wander {
            amp_us: 12.0,
            period_s: 1.0,
        }];
    }
    (world, sch)
}

fn analyse_rendered(r: &Rendered, sch: &WalkSchedule, rate: u32) -> WalkedAnalysis {
    analyse_walked(
        sch,
        &WalkedCapture {
            samples: &r.samples,
            first_frame: r.first_frame,
            rate_hz: rate,
            start_frame: r.start_frame,
            status: &[],
            complete: true,
        },
        &AnalysisOptions::default(),
    )
    .unwrap()
}

#[test]
fn wp_p5arrival_tweeter_world_the_old_refinement_hops_a_carrier_period_the_new_does_not() {
    let (world, sch) = tweeter_world(true);
    let r = render(&world, &sch);
    let a = analyse_rendered(&r, &sch, 48_000);
    let new: Vec<f64> = a.cycles.iter().map(|c| c.residual_us).collect();
    let old = old_residuals(&r, &sch, &a);
    // The carrier: the lobes 47 µs apart put it near 10 kHz, ~100 µs.
    let spread = |v: &[f64]| {
        v.iter().copied().fold(f64::MIN, f64::max) - v.iter().copied().fold(f64::MAX, f64::min)
    };
    assert!(
        spread(&old) > 75.0,
        "the old refinement should hop a carrier period here: {old:.1?}"
    );
    assert!(
        max_abs(new.iter().copied()) < 10.0,
        "new {new:.1?} (old {old:.1?})"
    );
    assert!(
        a.cycles.iter().all(|c| !c.arrival_ambiguous),
        "{:?}",
        a.quality
    );
    // Not reliable all the same: the clock wandered, and the report says so.
    assert!(!a.quality.reliable);
    let wander: Vec<(f64, f64)> = a
        .quality
        .issues
        .iter()
        .filter_map(|i| match i {
            TimingIssue::ClockWander {
                wander_us,
                rate_ppm,
                ..
            } => Some((*wander_us, *rate_ppm)),
            _ => None,
        })
        .collect();
    assert!(!wander.is_empty(), "{:?}", a.quality);
    for (us, ppm) in &wander {
        assert!(*us > 10.0 && *us < 80.0, "{us} µs");
        assert!(*ppm > 30.0 && *ppm < 250.0, "{ppm} ppm");
    }
    assert!(
        a.quality.reasons.iter().any(|r| r.contains("wandered")),
        "{:?}",
        a.quality.reasons
    );
    assert!(
        !a.quality
            .issues
            .iter()
            .any(|i| matches!(i, TimingIssue::CaptureSlip { .. })),
        "{:?}",
        a.quality.issues
    );
}

#[test]
fn wp_p5arrival_tweeter_world_without_wander_is_reliable() {
    let (world, sch) = tweeter_world(false);
    let r = render(&world, &sch);
    let a = analyse_rendered(&r, &sch, 48_000);
    assert!(a.quality.reliable, "{:?}", a.quality);
    assert!(a.quality.issues.is_empty(), "{:?}", a.quality.issues);
    assert!(a.quality.max_residual_us < 1.0, "{:?}", a.quality);
    assert!(a.cycles.iter().all(|c| c.wander_us < 2.0));
    // The phase is checked where the tweeter plays, band by band, the bound
    // growing with frequency above 5.6 kHz.
    let o = &a.outputs[0];
    assert!(!o.phase_bands.is_empty());
    for b in &o.phase_bands {
        let want = MAX_PHASE_SPREAD_DEG.max(360.0 * b.freq_hz * MAX_PHASE_SPREAD_US * 1e-6);
        assert!((b.bound_deg - want).abs() < 1e-9, "{b:?}");
        assert!(b.spread_deg < 2.0, "{b:?}");
    }
}

// ---------------------------------------------------------------------------
// The real captures (P5SPIKE, 2026-11-18): excerpts of the K340's tweeter and
// woofer as an iMM-6C heard them.
// ---------------------------------------------------------------------------

/// A capture rebuilt from an excerpt: the pre-roll and each cycle's band of
/// the sweep at their own frames, silence between.
struct RealRun {
    samples: Vec<f32>,
    schedule: WalkSchedule,
    start_frame: u64,
}

fn real_run(name: &str) -> RealRun {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/p5/spike");
    let man: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join(format!("{name}.json"))).unwrap()).unwrap();
    let bytes = std::fs::read(dir.join(format!("{name}.wav"))).unwrap();
    // 24-bit PCM mono: the data chunk's samples.
    let at = bytes.windows(4).position(|w| w == b"data").unwrap();
    let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
    let (frames, _) = bytes[at + 8..at + 8 + len].as_chunks::<3>();
    let pcm: Vec<f32> = frames
        .iter()
        .map(|b| {
            let v = i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8;
            v as f32 / 8_388_608.0
        })
        .collect();
    let origin = man["captureOrigin"].as_i64().unwrap();
    let total = man["captureFrames"].as_u64().unwrap() as usize;
    let taper = (man["taperMs"].as_f64().unwrap() * RATE / 1000.0) as usize;
    let mut samples = vec![0.0f32; total];
    let mut read = 0usize;
    for seg in man["segments"].as_array().unwrap() {
        let frames = seg["frames"].as_u64().unwrap() as usize;
        let at = (seg["sourceFrame"].as_i64().unwrap() - origin) as usize;
        let cycle = seg["kind"].as_str() == Some("cycle");
        for i in 0..frames {
            // Each cycle's band is faded in and out (a Hann taper).
            let w = if cycle {
                let d = i.min(frames - 1 - i);
                if d < taper {
                    0.5 - 0.5 * (std::f64::consts::PI * d as f64 / taper as f64).cos()
                } else {
                    1.0
                }
            } else {
                1.0
            };
            samples[at + i] = (f64::from(pcm[read + i]) * w) as f32;
        }
        read += frames;
    }
    assert_eq!(read, pcm.len());
    let sw = &man["sweep"];
    let cfg = crate::stimulus::SweepConfig {
        f1_hz: sw["f1Hz"].as_f64().unwrap() as f32,
        f2_hz: sw["f2Hz"].as_f64().unwrap() as f32,
        duration_ms: sw["durationMs"].as_u64().unwrap() as u32,
        gap_ms: sw["gapMs"].as_u64().unwrap() as u16,
    };
    let mask = man["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .fold(0u16, |m, o| m | 1 << o.as_u64().unwrap());
    let passes = man["passes"].as_u64().unwrap() as u16;
    let sweep = crate::stimulus::LogSweep::new(48_000, &cfg).unwrap();
    RealRun {
        samples,
        schedule: WalkSchedule::new(sweep, mask, passes).unwrap(),
        start_frame: man["startFrame"].as_u64().unwrap(),
    }
}

fn analyse_real(run: &RealRun) -> WalkedAnalysis {
    analyse_walked(
        &run.schedule,
        &WalkedCapture {
            samples: &run.samples,
            first_frame: 0,
            rate_hz: 48_000,
            start_frame: run.start_frame,
            status: &[],
            complete: true,
        },
        &AnalysisOptions::default(),
    )
    .unwrap()
}

#[test]
fn wp_p5arrival_real_tweeter_alone_has_no_lobe_hop_and_its_clock_wandered() {
    // P5SPIKE's output 3 alone, 5 passes: the analyser said [18.6 18.2 -81.6
    // 34.2 10.6] µs (a lobe hop of one carrier period on cycle 2); the
    // offline noise-weighted, leave-one-out estimate [2.6 2.5 -1.8 -14.2
    // 10.9] µs, ratio -9.06 ppm (p5spike-analysis/REPORT.md section 4).
    let run = real_run("only3_p5_excerpt");
    let a = analyse_real(&run);
    let res: Vec<f64> = a.cycles.iter().map(|c| c.residual_us).collect();
    println!(
        "only3: {res:.1?} ratio {:.2} ppm {:?}",
        a.clock.ratio_ppm, a.quality
    );
    assert!(
        a.cycles.iter().all(|c| !c.arrival_ambiguous),
        "{:?}",
        a.quality
    );
    let worst = max_abs(res.iter().copied());
    assert!(worst < 20.0, "{res:.1?}");
    assert!(worst > 5.0, "the wander is real: {res:.1?}");
    assert!(
        (a.clock.ratio_ppm + 9.06).abs() < 0.5,
        "{}",
        a.clock.ratio_ppm
    );
    // The time base moved inside the sweeps: said so, as wander.
    assert!(!a.quality.reliable);
    let wander: Vec<&TimingIssue> = a
        .quality
        .issues
        .iter()
        .filter(|i| matches!(i, TimingIssue::ClockWander { .. }))
        .collect();
    assert!(!wander.is_empty(), "{:?}", a.quality);
    let fastest = a.cycles.iter().map(|c| c.wander_ppm).fold(0.0, f64::max);
    assert!(fastest > 30.0 && fastest < 300.0, "{fastest} ppm");
}

#[test]
fn wp_p5arrival_real_woofer_and_tweeter_walked_residuals() {
    // P5SPIKE's walked run of outputs 2 and 3, 3 passes, ramp to -30 dBFS:
    // offline estimate max 4.4 µs, ratio -11.47 ppm.
    let run = real_run("ramp30_excerpt");
    let a = analyse_real(&run);
    let res: Vec<f64> = a.cycles.iter().map(|c| c.residual_us).collect();
    println!(
        "ramp30: {res:.1?} ratio {:.2} ppm {:?}",
        a.clock.ratio_ppm, a.quality
    );
    assert!(
        a.cycles.iter().all(|c| !c.arrival_ambiguous),
        "{:?}",
        a.quality
    );
    assert!(max_abs(res.iter().copied()) < 8.0, "{res:.1?}");
    assert!(
        (a.clock.ratio_ppm + 11.47).abs() < 0.5,
        "{}",
        a.clock.ratio_ppm
    );
    assert_eq!(a.outputs.len(), 2);
    // The woofer arrives later: its 3.5 kHz low-pass alone is about 64 µs
    // (offline: 70.9 µs between the two).
    let lag = (a.outputs[0].arrival_ms - a.outputs[1].arrival_ms) * 1000.0;
    println!("woofer after tweeter: {lag:.1} µs");
}

// ---------------------------------------------------------------------------
// The pieces
// ---------------------------------------------------------------------------

/// Excerpts of a two-lobe response delayed by `lags` (samples), each with
/// its envelope arrival off by `bias` (samples).
fn two_lobe_excerpts(lags: &[f64], bias: &[f64]) -> Vec<(i64, Vec<f64>, f64)> {
    let fir = tweeter_fir(47.0, 0.85);
    lags.iter()
        .zip(bias)
        .enumerate()
        .map(|(k, (&lag, &b))| {
            let start = 10_000 * k as i64;
            let t0 = 600.0 + lag;
            let samples: Vec<f64> = (0..1200)
                .map(|i| {
                    fir.iter()
                        .enumerate()
                        .map(|(j, h)| h * sinc(i as f64 - t0 - j as f64))
                        .sum()
                })
                .collect();
            (start, samples, start as f64 + t0 + 14.0 + b)
        })
        .collect()
}

fn refine(ex: &[(i64, Vec<f64>, f64)]) -> Vec<Refined> {
    let e: Vec<Excerpt<'_>> = ex
        .iter()
        .map(|(start, s, env)| Excerpt {
            start: *start,
            samples: s,
            envelope: *env,
        })
        .collect();
    refine_arrivals(&e, RATE, (20.0, 20_000.0), None)
}

#[test]
fn wp_p5arrival_refinement_recovers_lags_through_a_biased_envelope() {
    // The carrier period of this response is about 4.5 samples (~10.6
    // kHz): envelopes off by up to a third of it either way still choose the
    // right carrier peak.
    let lags = [0.0, 0.31, -0.42, 0.18, -0.05];
    let bias = [1.4, -1.5, 0.9, -1.2, 0.0];
    let ex = two_lobe_excerpts(&lags, &bias);
    let r = refine(&ex);
    let mean_env = ex.iter().map(|e| e.2).sum::<f64>() / 5.0;
    let mean_arr = r.iter().map(|x| x.arrival).sum::<f64>() / 5.0;
    assert!(
        (mean_arr - mean_env).abs() < 1e-6,
        "the envelopes' mean stays"
    );
    // The same shape in every excerpt: arrival minus true lag is one value.
    let rel: Vec<f64> = r
        .iter()
        .zip(&ex)
        .zip(&lags)
        .map(|((x, e), lag)| x.arrival - e.0 as f64 - lag)
        .collect();
    for (k, x) in r.iter().enumerate() {
        assert!(x.ambiguity.is_none(), "{k}: {x:?}");
        assert!((rel[k] - rel[0]).abs() * 1e6 / RATE < 0.3, "{k}: {rel:?}");
    }
}

#[test]
fn wp_p5arrival_an_envelope_half_a_carrier_period_off_is_an_ambiguous_arrival() {
    let lags = [0.0, 0.0, 0.0, 0.0];
    // Cycle 2's envelope is 3.2 samples (~0.7 carrier period) late: the
    // window holds the wrong carrier peak or none, and it is said.
    let bias = [0.0, 0.0, 3.2, 0.0];
    let r = refine(&two_lobe_excerpts(&lags, &bias));
    assert!(r[2].ambiguity.is_some(), "{r:?}");
    assert!(
        r[0].ambiguity.is_none() && r[1].ambiguity.is_none(),
        "{r:?}"
    );
    // One repeat alone keeps its envelope.
    let one = refine(&two_lobe_excerpts(&[0.0], &[0.7]));
    assert_eq!(one[0].ambiguity, None);
    assert!((one[0].arrival - (614.7)).abs() < 1e-9);
}

#[test]
fn wp_p5arrival_onset_from_the_envelope_does_not_move_when_the_lobes_trade_heights() {
    // A tweeter's two lobes trade heights between passes (P5SPIKE: 30 % of
    // |h| moved by 60 µs). The envelope's onset moves far less.
    let sch = schedule(48_000, 1_000, 300, 1, 3);
    let an = WalkedAnalyser::new(&sch, 48_000, &AnalysisOptions::default()).unwrap();
    let response = |second: f64| -> Vec<f64> {
        let fir = two_lobes(70.0, second);
        let mut h = vec![0.0; 4_800];
        for (i, v) in fir.iter().enumerate() {
            h[2_000 + i] = *v;
        }
        h
    };
    let onset_env = |h: &[f64]| {
        let a = an.envelope(h, None).unwrap();
        let (p, _) = env_peak(&a, 0, h.len() - 1).unwrap();
        let mag: Vec<f64> = a.iter().map(|c| c.norm()).collect();
        env_onset(&mag, p).unwrap()
    };
    let onset_raw = |h: &[f64]| {
        ImpulseResponse {
            rate_hz: 48_000,
            samples: h.to_vec(),
            zero_index: 0,
        }
        .onset_index()
        .unwrap()
    };
    let (a, b) = (response(2.5), response(4.5));
    let raw_move = (onset_raw(&a) - onset_raw(&b)).abs() * 1e6 / RATE;
    let env_move = (onset_env(&a) - onset_env(&b)).abs() * 1e6 / RATE;
    // The first lobe drops under 30 % of |h|'s peak: the raw onset jumps by
    // most of the lobes' 70 µs; the envelope's moves by a third of that.
    assert!(raw_move > 40.0, "{raw_move} µs");
    assert!(env_move < 0.4 * raw_move, "{env_move} µs (raw {raw_move})");
}

/// Two ways crossing at 2 kHz: a low-pass and its complement.
fn crossover_ways(delay_s: f64) -> Vec<Way> {
    let taps = 129usize;
    let mid = 64.0;
    let fc = 2_000.0 / RATE;
    let lp: Vec<f64> = (0..taps)
        .map(|i| {
            let x = i as f64 - mid;
            let w = 0.42
                + 0.5 * (std::f64::consts::PI * x / mid).cos()
                + 0.08 * (2.0 * std::f64::consts::PI * x / mid).cos();
            2.0 * fc * sinc(2.0 * fc * x) * w
        })
        .collect();
    let hp: Vec<f64> = lp
        .iter()
        .enumerate()
        .map(|(i, v)| if i == 64 { 1.0 - v } else { -v })
        .collect();
    vec![
        Way {
            output: 0,
            fir: lp,
            delay_s,
            gain: 0.5,
        },
        Way {
            output: 1,
            fir: hp,
            delay_s: delay_s + 0.0002,
            gain: 0.5,
        },
    ]
}

#[test]
fn wp_p5arrival_the_phase_check_defaults_to_the_transitions() {
    let sch = schedule(48_000, 1_000, 300, 0b11, 3);
    let world = World::new(crossover_ways(0.0005));
    let a = analyse(&world, &sch, &AnalysisOptions::default()).unwrap();
    assert!(a.quality.reliable, "{:?}", a.quality);
    for o in &a.outputs {
        assert!(!o.phase_bands.is_empty(), "output {}", o.output);
        for b in &o.phase_bands {
            assert!(
                b.freq_hz > 700.0 && b.freq_hz < 6_000.0,
                "output {}: {b:?}",
                o.output
            );
        }
    }
    // A run of one output has no transition: 100 Hz to 10 kHz.
    let one = schedule(48_000, 1_000, 300, 0b01, 3);
    let world = World::new(vec![super::tests_wp_p5time::delta_way(0, 0.0005)]);
    let a = analyse(&world, &one, &AnalysisOptions::default()).unwrap();
    let f: Vec<f64> = a.outputs[0].phase_bands.iter().map(|b| b.freq_hz).collect();
    assert!(f.first().is_some_and(|x| *x < 130.0) && f.last().is_some_and(|x| *x > 7_000.0));
    assert!(f.iter().all(|x| (99.0..=10_100.0).contains(x)), "{f:?}");
    // Given bands replace the default.
    let opts = AnalysisOptions {
        phase_check_bands: vec![(900.0, 1_100.0)],
        ..AnalysisOptions::default()
    };
    let a = analyse(&world, &one, &opts).unwrap();
    let f: Vec<f64> = a.outputs[0].phase_bands.iter().map(|b| b.freq_hz).collect();
    assert_eq!(f.len(), 1, "{f:?}");
    assert!((f[0] - 1_000.0).abs() < 1.0);
}

#[test]
fn wp_p5arrival_a_slip_between_cycles_and_wander_inside_a_sweep_are_told_apart() {
    let sch = schedule(48_000, 1_000, 300, 0b111, 3);
    let ways = vec![
        super::tests_wp_p5time::delta_way(0, 0.0004),
        super::tests_wp_p5time::delta_way(1, 0.0011),
        super::tests_wp_p5time::delta_way(2, 0.0023),
    ];
    let zero = 1.05 * RATE;
    let period = 62_400.0;
    let run = |fault: Fault| {
        let mut world = World::new(ways.clone());
        world.ppm = 40.0;
        world.faults = vec![fault];
        analyse(&world, &sch, &AnalysisOptions::default()).unwrap()
    };
    let slip = run(Fault::StepSlip {
        at_frame: zero + 4.0 * period - 7_200.0,
        frames: 3.0,
    });
    assert!(!slip.quality.reliable);
    assert!(
        slip.quality
            .issues
            .iter()
            .any(|i| matches!(i, TimingIssue::CaptureSlip { .. })),
        "{:?}",
        slip.quality
    );
    assert!(
        !slip
            .quality
            .issues
            .iter()
            .any(|i| matches!(i, TimingIssue::ClockWander { .. })),
        "{:?}",
        slip.quality
    );
    let wander = run(Fault::Wander {
        amp_us: 12.0,
        period_s: 0.8,
    });
    assert!(!wander.quality.reliable);
    assert!(
        wander
            .quality
            .issues
            .iter()
            .any(|i| matches!(i, TimingIssue::ClockWander { rate_ppm, .. } if *rate_ppm > 30.0)),
        "{:?}",
        wander.quality
    );
}

#[test]
fn proptest_wp_p5arrival_refinement_never_panics_or_returns_non_finite() {
    let strategy = (
        1usize..6,
        prop_oneof![Just(0usize), 1usize..64, 64usize..3_000],
        any::<u64>(),
        prop_oneof![Just(0.0f64), 1e-12f64..1e6],
        -1e6f64..1e6,
        any::<bool>(),
        prop_oneof![
            Just((20.0f64, 20_000.0f64)),
            Just((5_000.0, 5_001.0)),
            Just((30_000.0, 40_000.0))
        ],
    );
    run_capped(
        "proptest_wp_p5arrival_refinement_never_panics_or_returns_non_finite",
        64,
        2_000,
        strategy,
        |(m, len, seed, scale, env, with_noise, band)| {
            let mut x = seed | 1;
            let mut next = || {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 11) as f64 / (1u64 << 53) as f64 - 0.5
            };
            let data: Vec<(i64, Vec<f64>, f64)> = (0..m)
                .map(|k| {
                    let s: Vec<f64> = (0..len).map(|_| next() * scale).collect();
                    (k as i64 * 1_000 - 7, s, env + next() * 10.0)
                })
                .collect();
            let ex: Vec<Excerpt<'_>> = data
                .iter()
                .map(|(start, s, e)| Excerpt {
                    start: *start,
                    samples: s,
                    envelope: *e,
                })
                .collect();
            let noise = |f: f64| f * 1e-9 + 1e-12;
            let r = refine_arrivals(
                &ex,
                RATE,
                band,
                if with_noise { Some(&noise) } else { None },
            );
            prop_assert_eq!(r.len(), m);
            for v in &r {
                prop_assert!(v.arrival.is_finite());
                prop_assert!(v.ambiguity.is_none_or(|a| a.is_finite() && a >= 0.0));
            }
            Ok(())
        },
    );
}

#[test]
fn proptest_wp_p5arrival_envelope_helpers_never_panic() {
    let strategy = (
        proptest::collection::vec(-1e3f64..1e3, 0..400),
        -10.0f64..500.0,
        0.0f64..30_000.0,
    );
    run_capped(
        "proptest_wp_p5arrival_envelope_helpers_never_panic",
        256,
        5_000,
        strategy,
        |(v, peak, f)| {
            let a: Vec<C64> = v.iter().map(|x| C64::new(*x, x * 0.5)).collect();
            if let Some((p, pw)) = env_peak(&a, 0, a.len()) {
                prop_assert!(p.is_finite() && pw.is_finite() && pw >= 0.0);
            }
            let mag: Vec<f64> = v.iter().map(|x| x.abs()).collect();
            if let Some(o) = env_onset(&mag, peak) {
                prop_assert!(o.is_finite() && o <= peak.max(0.0) + 1.0);
            }
            let t = band_taper(f, (20.0, 20_000.0));
            prop_assert!((0.0..=1.0).contains(&t));
            let _ = heard_env(&a, 0, a.len());
            Ok(())
        },
    );
}
