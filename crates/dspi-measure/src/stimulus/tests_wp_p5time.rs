//! WP P5TIME: the sweep model against an independent transcription of
//! `siggen.c`, the walk schedule and the status model.
#![allow(clippy::arithmetic_side_effects)]

use super::*;
use proptest::prelude::*;

/// An independent transcription of DSPi v1.1.5 `siggen.c`, quoted line by line.
/// Written separately from the model (C-like integer code, `i128` products,
/// no shared helpers) so the two can be compared.
mod siggen_c {
    pub struct Derived {
        pub cycle_samples: u32,
        pub sweep_inc0: u64,
        pub sweep_eps_q31: i32,
    }

    // siggen.c:926-937 (inside compute_derived, case SIGGEN_SWEEP_LOG):
    //     if (f1 < 1.0f) f1 = 1.0f;
    //     if (f2 > nyq) f2 = nyq;
    //     if (f2 <= f1) f2 = f1 + 1.0f;
    //     uint32_t dur = (uint32_t)((uint64_t)fs * cfg.duration_ms / 1000u);
    //     if (dur < d->fade_samples * 2) dur = d->fade_samples * 2;
    //     d->cycle_samples = dur;
    //     d->sweep_inc0 = (uint64_t)((double)f1 / fs * two48);
    //     if (cfg.signal_type == SIGGEN_SWEEP_LOG) {
    //         double eps = log((double)f2 / f1) / (double)dur;   // per-sample growth
    //         d->sweep_eps_q31 = (int32_t)(eps * 2147483648.0);
    // with, earlier in compute_derived:
    //     const double two48 = 281474976710656.0;
    //     float nyq = 0.45f * (float)fs;
    //     float f1 = cfg.p1, f2 = cfg.p2;
    //     d->fade_samples = (uint32_t)((uint64_t)fs * SIGGEN_FADE_MS / 1000u);
    //     if (d->fade_samples < 8) d->fade_samples = 8;
    pub fn compute_derived(fs: u32, p1: f32, p2: f32, duration_ms: u32) -> Derived {
        let two48: f64 = 281474976710656.0;
        let nyq: f32 = 0.45f32 * (fs as f32);
        let mut f1: f32 = p1;
        let mut f2: f32 = p2;
        let mut fade_samples: u32 = ((fs as u64) * 5u64 / 1000u64) as u32;
        if fade_samples < 8 {
            fade_samples = 8;
        }
        if f1 < 1.0 {
            f1 = 1.0;
        }
        if f2 > nyq {
            f2 = nyq;
        }
        if f2 <= f1 {
            f2 = f1 + 1.0;
        }
        let mut dur: u32 = ((fs as u64) * (duration_ms as u64) / 1000u64) as u32;
        if dur < fade_samples * 2 {
            dur = fade_samples * 2;
        }
        let sweep_inc0 = ((f1 as f64) / (fs as f64) * two48) as u64;
        let eps: f64 = ((f2 as f64) / (f1 as f64)).ln() / (dur as f64);
        let sweep_eps_q31 = (eps * 2147483648.0) as i32;
        Derived {
            cycle_samples: dur,
            sweep_inc0,
            sweep_eps_q31,
        }
    }

    // siggen.c:360-372 (synth_sweep), after synth_reset_cycle's
    //     sweep_phase = 0; sweep_inc = d->sweep_inc0;
    //     uint64_t ph = sweep_phase, inc = sweep_inc;
    //     for (uint32_t i = 0; i < n; i++) {
    //         ph += inc;
    //         dst[i] = osc_sin((uint32_t)(ph >> 16)) * amp;
    //         if (type == SIGGEN_SWEEP_LOG) {
    //             inc += (uint64_t)(((int64_t)(inc >> 16) * d->sweep_eps_q31) >> 15);
    // Returns the argument of osc_sin per sample.
    pub fn sweep_phases(d: &Derived) -> Vec<u32> {
        let mut ph: u64 = 0;
        let mut inc: u64 = d.sweep_inc0;
        let mut out = Vec::with_capacity(d.cycle_samples as usize);
        for _ in 0..d.cycle_samples {
            ph = ph.wrapping_add(inc);
            out.push((ph >> 16) as u32);
            let p: i128 = ((inc >> 16) as i64 as i128) * (d.sweep_eps_q31 as i128);
            inc = inc.wrapping_add(((p as i64) >> 15) as u64);
        }
        out
    }
}

fn sweep(rate: u32, cfg: &SweepConfig) -> LogSweep {
    LogSweep::new(rate, cfg).unwrap()
}

#[test]
fn wp_p5time_sweep_matches_siggen_c_first_and_last_4096_samples_at_each_rate() {
    for &rate in &SUPPORTED_RATES {
        for cfg in [
            SweepConfig::SPEAKERS,
            SweepConfig {
                duration_ms: 2_700,
                ..SweepConfig::COUPLER
            },
            SweepConfig {
                f1_hz: 10.0,
                f2_hz: 40_000.0,
                duration_ms: 15_000,
                gap_ms: 0,
            },
        ] {
            let m = sweep(rate, &cfg);
            let d = siggen_c::compute_derived(rate, cfg.f1_hz, cfg.f2_hz, cfg.duration_ms);
            assert_eq!(m.cycle_samples, d.cycle_samples, "{rate} {cfg:?}");
            assert_eq!(m.inc0_q48, d.sweep_inc0, "{rate} {cfg:?}");
            assert_eq!(m.eps_q31, d.sweep_eps_q31, "{rate} {cfg:?}");
            let model = m.phases_q32();
            let reference = siggen_c::sweep_phases(&d);
            assert_eq!(model.len(), reference.len());
            let n = model.len();
            assert_eq!(model[..4096], reference[..4096], "first 4096 at {rate}");
            assert_eq!(
                model[n - 4096..],
                reference[n - 4096..],
                "last 4096 at {rate}"
            );
        }
    }
}

#[test]
fn wp_p5time_quantised_sweep_differs_from_the_analytic_one() {
    // The reason the model exists: the Q31 growth ends a 5.5 s sweep about
    // 3 Hz below 20 kHz, and the phase is then turns away from the analytic
    // log sweep's.
    let m = sweep(48_000, &SweepConfig::SPEAKERS);
    let fs = 48_000.0;
    let (f1, f2) = (20.0f64, 20_000.0f64);
    let t_len = f64::from(m.cycle_samples) / fs;
    let k = t_len / (f2 / f1).ln();
    let analytic_turns = f1 * k * ((t_len / k).exp() - 1.0);
    let mut st = m.initial_state();
    let mut turns: u128 = 0;
    for _ in 0..m.cycle_samples {
        turns += u128::from(st.inc_q48);
        st.step(m.eps_q31);
    }
    let model_turns = turns as f64 / 281_474_976_710_656.0;
    let drift = (analytic_turns - model_turns).abs();
    assert!(drift > 1.0, "{drift} turns");
    let end = m.end_freq_hz();
    assert!((1.0..10.0).contains(&(f2 - end)), "{end} Hz");
}

#[test]
fn wp_p5time_derived_numbers_per_rate() {
    let m = sweep(48_000, &SweepConfig::SPEAKERS);
    assert_eq!(m.cycle_samples, 264_000);
    assert_eq!(m.gap_samples, 48_000);
    assert_eq!(m.fade_samples, 240);
    let m = sweep(44_100, &SweepConfig::SPEAKERS);
    assert_eq!(m.fade_samples, 220);
    assert_eq!(m.cycle_samples, 242_550);
    assert!(m.f2_hz <= 0.45 * 44_100.0 + 0.01, "f2 clamped to 0.45 fs");
    let m = sweep(96_000, &SweepConfig::COUPLER);
    assert_eq!(m.gap_samples, 28_800);
    assert_eq!(m.fade_samples, 480);
    // The end frequency of the quantised sweep is close to f2.
    let m = sweep(48_000, &SweepConfig::SPEAKERS);
    assert!(
        (m.end_freq_hz() / 20_000.0 - 1.0).abs() < 1e-3,
        "{}",
        m.end_freq_hz()
    );
    assert!((m.time_of_freq(20_000.0) - 5.5).abs() < 0.01);
    assert!((m.time_of_freq(20.0)).abs() < 0.01);
}

#[test]
fn wp_p5time_bad_sweeps_are_refused() {
    let bad = [
        (32_000, SweepConfig::SPEAKERS),
        (
            48_000,
            SweepConfig {
                duration_ms: 999,
                ..SweepConfig::SPEAKERS
            },
        ),
        (
            48_000,
            SweepConfig {
                duration_ms: 15_001,
                ..SweepConfig::SPEAKERS
            },
        ),
        (
            48_000,
            SweepConfig {
                f1_hz: f32::NAN,
                ..SweepConfig::SPEAKERS
            },
        ),
        (
            48_000,
            SweepConfig {
                f1_hz: 30_000.0,
                f2_hz: 40_000.0,
                ..SweepConfig::SPEAKERS
            },
        ),
        (
            48_000,
            SweepConfig {
                f1_hz: 1000.0,
                f2_hz: 500.0,
                ..SweepConfig::SPEAKERS
            },
        ),
    ];
    for (rate, cfg) in bad {
        assert!(
            matches!(LogSweep::new(rate, &cfg), Err(MeasureError::Invalid(_))),
            "{rate} {cfg:?}"
        );
    }
}

#[test]
fn wp_p5time_windows_and_fade() {
    let m = sweep(48_000, &SweepConfig::SPEAKERS);
    let c = m.render_cycle(false);
    let first = m.render_cycle(true);
    assert_eq!(c.len(), 264_000);
    assert_eq!(c[0], 0.0, "attack starts at zero");
    assert!(c.iter().all(|v| v.abs() <= 1.0));
    // After the 5 ms fade the first cycle equals the others.
    assert_eq!(c[240..], first[240..]);
    // Inside the fade the first cycle is quieter (fade times attack).
    assert!(first[100].abs() <= c[100].abs());
    assert_eq!(m.window_gain(-1.0), 0.0);
    assert_eq!(m.window_gain(264_000.0), 0.0);
    assert_eq!(m.window_gain(120.0), 0.5);
    assert_eq!(m.window_gain(264_000.0 - 120.0), 0.5);
    assert_eq!(m.window_gain(1000.0), 1.0);
    assert_eq!(peak_linear(0.0), 1.0);
    assert!((peak_linear(-20.0) - 0.1).abs() < 1e-6);
    assert_eq!(peak_linear(10.0), 1.0);
    assert!(peak_linear(f32::NAN) < 1e-5);
}

#[test]
fn wp_p5time_render_at_another_rate_follows_the_phase() {
    // Rendered at 96 kHz from a 48 kHz sweep, every second sample is the
    // device sample (up to the interpolation and the anti-alias fade, which
    // does not act below 40 kHz).
    let cfg = SweepConfig {
        f1_hz: 20.0,
        f2_hz: 20_000.0,
        duration_ms: 1_000,
        gap_ms: 300,
    };
    let m = sweep(48_000, &cfg);
    let dev = m.render_cycle(false);
    let up = m.render_at_rate(96_000, false).unwrap();
    assert_eq!(up.len(), 2 * dev.len() - 1);
    let mut max_err = 0.0f64;
    for (i, v) in dev.iter().enumerate() {
        max_err = max_err.max((up[2 * i] - v).abs());
    }
    assert!(max_err < 1e-6, "{max_err}");
    // At 44.1 kHz from 48 kHz the part above 0.42 * 44.1 kHz is faded out.
    let down = m.render_at_rate(44_100, false).unwrap();
    let tail = &down[down.len() - 40..];
    assert!(
        tail.iter().all(|v| v.abs() < 1e-12),
        "nothing above 0.45 fs"
    );
    assert!(m.render_at_rate(1_000, false).is_err());
}

#[test]
fn wp_p5time_walk_schedule_order_and_no_gap_after_last() {
    let m = sweep(48_000, &SweepConfig::COUPLER);
    let s = WalkSchedule::new(m, 0b1010_0101, 3).unwrap();
    let outs: Vec<u8> = s.cycles().iter().map(|c| c.output).collect();
    assert_eq!(outs, [0, 2, 5, 7, 0, 2, 5, 7, 0, 2, 5, 7]);
    let period = u64::from(m.cycle_samples + m.gap_samples);
    for (k, c) in s.cycles().iter().enumerate() {
        assert_eq!(c.start, k as u64 * period);
        assert_eq!(c.pass as usize, k / 4);
    }
    assert_eq!(s.total_samples(), 11 * period + u64::from(m.cycle_samples));
    assert_eq!(s.outputs(), vec![0, 2, 5, 7]);
    assert!(WalkSchedule::new(m, 0, 3).is_err());
    assert!(WalkSchedule::new(m, 1, 0).is_err());
    assert!(WalkSchedule::new(m, 1, 9).is_err());
}

#[test]
fn wp_p5time_largest_default_run_fits_the_capture_bound_at_48k_not_96k() {
    // PLAN-P5 section 1.1 step 10: 6 outputs x 3 passes x 6.5 s.
    let m = sweep(48_000, &SweepConfig::SPEAKERS);
    let s = WalkSchedule::new(m, 0b11_1111, 3).unwrap();
    let at48 = s.planned_capture_frames(48_000, 1_000);
    let at96 = s.planned_capture_frames(96_000, 1_000);
    assert!(at48 <= crate::ir::MAX_CAPTURE_SAMPLES as u64, "{at48}");
    assert!(at96 > crate::ir::MAX_CAPTURE_SAMPLES as u64, "{at96}");
}

#[test]
fn wp_p5time_status_model() {
    let m = sweep(48_000, &SweepConfig::COUPLER);
    let s = WalkSchedule::new(m, 0b0110, 2).unwrap();
    let len = f64::from(m.cycle_samples);
    let period = s.period_samples() as f64;
    let st = |ch, done| GeneratorStatus {
        active_channel: ch,
        cycles_done: done,
    };
    assert_eq!(s.status_at(0.0), st(1, 0));
    assert_eq!(
        s.status_at(len + 1.0),
        st(2, 1),
        "the gap shows the next output"
    );
    assert_eq!(s.status_at(period + 10.0), st(2, 1));
    assert_eq!(s.status_at(3.0 * period + 10.0), st(2, 3));
    assert_eq!(
        s.status_at(3.0 * period + len + 1.0),
        st(NO_CHANNEL, 4),
        "no gap after the last"
    );
    assert!(s.status_consistent(len - 10.0, len + 10.0, st(2, 1)));
    assert!(s.status_consistent(len - 10.0, len + 10.0, st(1, 0)));
    assert!(!s.status_consistent(len - 10.0, len + 10.0, st(1, 2)));
    assert!(!s.status_consistent(10.0, 1.0, st(1, 0)));
    assert!(!s.status_consistent(f64::NAN, 1.0, st(1, 0)));
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
fn proptest_wp_p5time_sweep_and_schedule_never_panic() {
    // Rendering is the expensive part: at most 2000 cases under the
    // proptest step.
    let strategy = (
        prop_oneof![Just(44_100u32), Just(48_000), Just(96_000), 0u32..200_000],
        prop_oneof![Just(20.0f32), proptest::num::f32::ANY],
        prop_oneof![Just(20_000.0f32), proptest::num::f32::ANY],
        prop_oneof![Just(1_000u32), 0u32..20_000],
        any::<u16>(),
        any::<u16>(),
        0u16..12,
        proptest::num::f64::ANY,
        prop_oneof![Just(48_000u32), 0u32..400_000],
    );
    run_capped(
        "proptest_wp_p5time_sweep_and_schedule_never_panic",
        64,
        2_000,
        strategy,
        |(rate, f1, f2, dur, gap, mask, repeat, probe, cap_rate)| {
            let cfg = SweepConfig {
                f1_hz: f1,
                f2_hz: f2,
                duration_ms: dur,
                gap_ms: gap,
            };
            if let Ok(m) = LogSweep::new(rate, &cfg) {
                prop_assert!(m.cycle_samples >= 2 * m.fade_samples);
                prop_assert!(m.time_of_freq(f64::from(f2)).is_finite());
                prop_assert!(m.time_of_freq(probe).is_finite());
                prop_assert!(m.window_gain(probe).is_finite());
                if let Ok(s) = WalkSchedule::new(m, mask, repeat) {
                    prop_assert_eq!(
                        s.cycles().len(),
                        mask.count_ones() as usize * repeat as usize
                    );
                    let _ = s.status_at(probe);
                    let st = GeneratorStatus {
                        active_channel: 0,
                        cycles_done: 0,
                    };
                    let _ = s.status_consistent(probe, probe + 1.0, st);
                    prop_assert!(s.planned_capture_frames(cap_rate, 1000) < u64::MAX);
                }
                // Only the short sweeps are rendered.
                if m.cycle_samples <= 50_000 {
                    let c = m.render_cycle(true);
                    prop_assert!(c.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
                    if let Ok(r) = m.render_at_rate(cap_rate, false) {
                        prop_assert!(r.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
                    }
                }
            }
            Ok(())
        },
    );
}
