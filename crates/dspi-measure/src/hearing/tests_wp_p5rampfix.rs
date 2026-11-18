//! WP P5RAMPFIX: the hearing check against real room noise (the first
//! P5SPIKE's capture) and simulated rumble.
#![allow(clippy::arithmetic_side_effects)]

use proptest::prelude::*;

use super::*;
use crate::stimulus::{LogSweep, SweepConfig};
use crate::wav::read_wav;

const RATE: u32 = 48_000;

/// The real capture: 3.2 s of an iMM-6C in a K340 cup, rumble and all.
fn real_noise() -> Vec<f32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/p5/spike/k340_imm6c_cup_noise.wav");
    let bytes = std::fs::read(path).unwrap();
    let w = read_wav(&bytes, 0).unwrap();
    assert_eq!(w.info.rate_hz, RATE);
    w.samples
}

/// A ramp step's stimulus: the 1 s 20 Hz to 20 kHz sweep at unit amplitude.
fn ramp_sweep() -> Vec<f64> {
    let s = LogSweep::new(
        RATE,
        &SweepConfig {
            f1_hz: 20.0,
            f2_hz: 20_000.0,
            duration_ms: 1_000,
            gap_ms: 0,
        },
    )
    .unwrap();
    s.render_at_rate(RATE, true).unwrap()
}

/// `noise` with the sweep added at `peak_dbfs` from `at` samples on.
fn with_sweep(noise: &[f32], sweep: &[f64], peak_dbfs: f64, at: usize) -> Vec<f32> {
    let a = 10f64.powf(peak_dbfs / 20.0);
    let mut v = noise.to_vec();
    for (x, s) in v.iter_mut().skip(at).zip(sweep) {
        *x += (a * s) as f32;
    }
    v
}

fn band() -> (f64, f64) {
    hearing_band(20.0, 20_000.0, RATE)
}

/// The broadband check this replaces (P5SES): 100 Hz to 10 kHz by
/// second-order filters, 125 ms windows, the step's louder half against the
/// pre-roll's median. Kept here to show where it failed.
fn broadband_snr_db(pre: &[f32], step: &[f32]) -> f64 {
    fn powers(x: &[f32]) -> Vec<f64> {
        let fs = f64::from(RATE);
        let pass = |f0: f64, high: bool| {
            let w = std::f64::consts::TAU * f0 / fs;
            let (s, c) = w.sin_cos();
            let al = s / std::f64::consts::SQRT_2;
            let a0 = 1.0 + al;
            let (b0, b1) = if high {
                ((1.0 + c) / 2.0, -(1.0 + c))
            } else {
                ((1.0 - c) / 2.0, 1.0 - c)
            };
            (
                [b0 / a0, b1 / a0, b0 / a0],
                [-2.0 * c / a0, (1.0 - al) / a0],
            )
        };
        let mut st = [[0.0f64; 4]; 2];
        let f = [pass(100.0, true), pass(10_000.0, false)];
        let n = 6000;
        let mut out = Vec::new();
        let (mut sum, mut cnt) = (0.0, 0);
        for s in x {
            let mut y = f64::from(*s);
            for (i, (b, a)) in f.iter().enumerate() {
                let z = b[0] * y + b[1] * st[i][0] + b[2] * st[i][1]
                    - a[0] * st[i][2]
                    - a[1] * st[i][3];
                st[i] = [y, st[i][0], z, st[i][2]];
                y = z;
            }
            sum += y * y;
            cnt += 1;
            if cnt == n {
                out.push(sum / n as f64);
                sum = 0.0;
                cnt = 0;
            }
        }
        out
    }
    let mut p = powers(pre);
    p.remove(0);
    p.sort_by(f64::total_cmp);
    let noise = p[p.len() / 2];
    let mut s = powers(step);
    s.sort_by(|a, b| b.total_cmp(a));
    let half = s.len().div_ceil(2);
    let sig = s.iter().take(half).sum::<f64>() / half as f64;
    10.0 * (sig / noise).log10()
}

#[test]
fn wp_p5rampfix_the_real_rooms_noise_alone_is_not_heard() {
    let x = real_noise();
    let sweep = ramp_sweep();
    // A 1 s pre-roll and a 1.6 s step that do not overlap, four ways.
    for (p, st) in [(0, 48_000), (0, 76_800), (105_600, 0), (76_800, 0)] {
        let (pre, step) = (&x[p..p + 48_000], &x[st..st + 76_800]);
        for played in [Played::Sweep(&sweep), Played::Pink] {
            let l = Listener::new(pre, RATE, band(), played).unwrap();
            let h = l.hear(step).unwrap();
            eprintln!(
                "pre {p} step {st} {}: {h:?}",
                matches!(played, Played::Pink)
            );
            assert!(h.snr_db < 0.0, "pre {p} step {st}: {h:?}");
        }
    }
}

#[test]
fn wp_p5rampfix_a_sweep_under_the_real_rumble_is_heard_in_band() {
    let x = real_noise();
    let sweep = ramp_sweep();
    let (pre, noise) = (&x[..48_000], &x[48_000..124_800]);
    let l = Listener::new(pre, RATE, band(), Played::Sweep(&sweep)).unwrap();
    // The guarded 1 kHz tone at -40 dBFS reached this microphone at about
    // -81 dBFS: a ramp step at -42 dBFS is a sweep of about -83 dBFS peak,
    // 32 dB under the broadband rumble.
    let step = with_sweep(noise, &sweep, -83.0, 9_600);
    let h = l.hear(&step).unwrap();
    assert!(h.snr_db >= 12.0, "clearly heard in band: {h:?}");
    // Its peak is where the sweep starts (within a few ms).
    let at = h.peak_s.unwrap();
    assert!((at - 0.2).abs() < 0.01, "{at}");
    // The broadband check saw nothing there (the P5SPIKE failure).
    assert!(broadband_snr_db(pre, &step) < 3.0);
    // A 6 dB step raises it by about 6 dB once heard, never by less than 3,
    // and its level too.
    let mut last: Option<Hearing> = None;
    for db in [-95.0, -89.0, -83.0, -77.0, -71.0] {
        let h = l.hear(&with_sweep(noise, &sweep, db, 9_600)).unwrap();
        eprintln!("sweep at {db} dBFS: {h:?}");
        if let Some(p) = last
            && p.snr_db >= 6.0
        {
            assert!(h.snr_db - p.snr_db >= 4.5, "{db}: {p:?} -> {h:?}");
            assert!(h.level_db - p.level_db >= 4.5, "{db}: {p:?} -> {h:?}");
        }
        last = Some(h);
    }
    assert!(last.unwrap().snr_db > 24.0, "{last:?}");
}

#[test]
fn wp_p5rampfix_a_sweep_far_under_the_in_band_floor_is_not_heard() {
    let x = real_noise();
    let sweep = ramp_sweep();
    let (pre, noise) = (&x[..48_000], &x[48_000..124_800]);
    let l = Listener::new(pre, RATE, band(), Played::Sweep(&sweep)).unwrap();
    let h = l.hear(&with_sweep(noise, &sweep, -110.0, 9_600)).unwrap();
    assert!(h.snr_db < 0.0, "{h:?}");
}

/// White noise and strong rumble below 80 Hz, as a seeded sum.
fn rumbling(n: usize, seed: u64, white_dbfs: f64, rumble_dbfs: f64) -> Vec<f32> {
    let mut s = seed;
    let w = 10f64.powf(white_dbfs / 20.0) * 3f64.sqrt();
    let r = 10f64.powf(rumble_dbfs / 20.0) * (2.0f64 / 8.0).sqrt();
    (0..n)
        .map(|i| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let u = (s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
            let t = i as f64 / f64::from(RATE);
            let rum: f64 = (0..8)
                .map(|k| {
                    (std::f64::consts::TAU * (23.0 + 7.3 * f64::from(k)) * t + f64::from(k)).sin()
                })
                .sum();
            (w * u + r * rum) as f32
        })
        .collect()
}

#[test]
fn wp_p5rampfix_rumble_is_never_signal() {
    let sweep = ramp_sweep();
    let pre = rumbling(48_000, 1, -100.0, -50.0);
    // The rumble doubles during the step: nothing below 100 Hz counts.
    let step = rumbling(76_800, 2, -100.0, -44.0);
    for played in [Played::Sweep(&sweep), Played::Pink] {
        let l = Listener::new(&pre, RATE, band(), played).unwrap();
        let h = l.hear(&step).unwrap();
        assert!(h.snr_db < 0.0, "{played:?}: {h:?}");
    }
    // A sweep 40 dB under the rumble's RMS is heard.
    let l = Listener::new(&pre, RATE, band(), Played::Sweep(&sweep)).unwrap();
    let h = l.hear(&with_sweep(
        &rumbling(76_800, 3, -100.0, -50.0),
        &sweep,
        -90.0,
        4_800,
    ));
    assert!(h.unwrap().snr_db > 20.0);
}

#[test]
fn wp_p5rampfix_pink_noise_is_heard_by_its_bands() {
    let pre = rumbling(48_000, 4, -100.0, -50.0);
    let l = Listener::new(&pre, RATE, band(), Played::Pink).unwrap();
    let mut s = 99u64;
    let mut pink = |db: f64| -> Vec<f32> {
        let mut b = [0f64; 3];
        let a = 10f64.powf(db / 20.0);
        rumbling(76_800, 5, -100.0, -50.0)
            .iter()
            .map(|n| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                let w = (s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
                b[0] = 0.997 * b[0] + 0.029 * w;
                b[1] = 0.985 * b[1] + 0.032 * w;
                b[2] = 0.950 * b[2] + 0.048 * w;
                n + (a * (b[0] + b[1] + b[2] + 0.2 * w)) as f32
            })
            .collect()
    };
    let quiet = l.hear(&pink(-100.0)).unwrap().snr_db;
    let loud = l.hear(&pink(-80.0)).unwrap().snr_db;
    assert!(loud > 10.0 && loud - quiet > 10.0, "{quiet} {loud}");
}

#[test]
fn wp_p5rampfix_the_band_follows_the_sweep() {
    assert_eq!(hearing_band(20.0, 20_000.0, 48_000), (100.0, 10_000.0));
    assert_eq!(hearing_band(1_000.0, 20_000.0, 48_000), (1_000.0, 10_000.0));
    assert_eq!(hearing_band(20.0, 20_000.0, 16_000), (100.0, 6_720.0));
    // Wholly below 100 Hz: its own band.
    assert_eq!(hearing_band(20.0, 80.0, 48_000), (20.0, 80.0));
    assert_eq!(hearing_band(f64::NAN, 1.0, 48_000), (100.0, 10_000.0));
}

#[test]
fn wp_p5rampfix_bad_input_is_refused() {
    let sweep = ramp_sweep();
    let pre = vec![0.0f32; 48_000];
    assert!(Listener::new(&pre, 1_000, band(), Played::Pink).is_err());
    assert!(Listener::new(&pre, RATE, (10.0, 5.0), Played::Pink).is_err());
    assert!(matches!(
        Listener::new(&pre[..1_000], RATE, band(), Played::Sweep(&sweep)),
        Err(MeasureError::NoSignal(_))
    ));
    let l = Listener::new(&pre, RATE, band(), Played::Sweep(&sweep)).unwrap();
    assert!(matches!(
        l.hear(&pre[..100]),
        Err(MeasureError::NoSignal(_))
    ));
    // Digital silence: nothing heard; anything at all: heard at the top.
    let silent = vec![0.0f32; 76_800];
    assert_eq!(l.hear(&silent).unwrap().snr_db, NO_SNR_DB);
    // A sweep in digital silence: as high as its own sidelobes allow.
    let step = with_sweep(&silent, &sweep, -120.0, 4_800);
    let h = l.hear(&step).unwrap();
    assert!(h.snr_db > 41.0, "{h:?}");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn proptest_wp_p5rampfix_any_capture_gives_a_finite_answer(
        pre in proptest::collection::vec(prop_oneof![-1.5f32..1.5, Just(f32::NAN), Just(f32::INFINITY)], 4_096..12_000),
        step in proptest::collection::vec(-1.5f32..1.5, 0..16_000),
        rate in prop_oneof![Just(8_000u32), Just(44_100u32), Just(48_000u32), 0u32..400_000],
        f1 in -10.0f64..30_000.0,
        f2 in -10.0f64..30_000.0,
        pink in any::<bool>(),
    ) {
        let band = hearing_band(f1, f2, rate);
        prop_assert!(band.0.is_finite() && band.1.is_finite());
        let tmpl: Vec<f64> = (0..4_000).map(|i| (i as f64 * 0.01).sin()).collect();
        let played = if pink { Played::Pink } else { Played::Sweep(&tmpl) };
        if let Ok(l) = Listener::new(&pre, rate, band, played)
            && let Ok(h) = l.hear(&step)
        {
            prop_assert!(h.snr_db.is_finite());
            prop_assert!((NO_SNR_DB..=MAX_SNR_DB).contains(&h.snr_db));
            prop_assert!(h.peak_s.is_none_or(f64::is_finite));
        }
    }
}

/// P5SIM's world with heavy rumble: the microphone hears the output 45 dB
/// down, white noise at -85 dBFS (about the real microphone's floor above
/// 1 kHz) and rumble at -45 dBFS RMS below 80 Hz.
#[cfg(feature = "sim")]
fn rumble_world() -> crate::sim::World {
    let mut w = crate::sim::World::flat(1, RATE);
    w.noise_dbfs = Some(-85.0);
    w.rumble = Some(crate::sim::Rumble {
        dbfs: -45.0,
        top_hz: 80.0,
    });
    w.outputs[0].driver.sensitivity_db = -45.0;
    w.clock.latency_ms = 30.0;
    w.seed = 11;
    w
}

/// A pre-roll (1 s, nothing played) and a ramp step at `level` (the 1 s
/// sweep and 0.6 s of tail) rendered from capture frame `at`.
#[cfg(feature = "sim")]
fn sim_capture(world: &crate::sim::World, level: Option<f32>, at: u64) -> Vec<f32> {
    use crate::sim::{ProtoModel, Stimulus, flat_chains, render};
    let mut w = world.clone();
    w.clock.first_frame = at;
    let chains = flat_chains(1);
    let sweep: Vec<f32> = ramp_sweep()
        .iter()
        .map(|v| (*v * f64::from(level.map_or(0.0, crate::stimulus::peak_linear))) as f32)
        .collect();
    let stim = match level {
        Some(_) => Stimulus::summed(&sweep, &[(0, false)], 0, 28_800).unwrap(),
        None => Stimulus {
            bursts: Vec::new(),
            frames: 48_000,
        },
    };
    render(&w, &stim, &chains, &ProtoModel, RATE)
        .unwrap()
        .samples
}

#[cfg(feature = "sim")]
#[test]
fn wp_p5rampfix_sim_rumble_world_the_old_check_misses_what_the_new_hears() {
    let world = rumble_world();
    let pre = sim_capture(&world, None, 0);
    let sweep = ramp_sweep();
    let l = Listener::new(&pre, RATE, band(), Played::Sweep(&sweep)).unwrap();
    // The ramp from -50 dBFS: by start + 18 dB the old check still heard
    // nothing, the new one hears every step.
    for (i, level) in [-50.0f32, -44.0, -38.0, -32.0].into_iter().enumerate() {
        let step = sim_capture(&world, Some(level), 48_000 + 80_000 * i as u64);
        let old = broadband_snr_db(&pre, &step);
        let new = l.hear(&step).unwrap();
        eprintln!("{level} dBFS: broadband {old:.1} dB, in band {new:?}");
        assert!(old < 6.0, "{level}: {old}");
        assert!(new.snr_db >= 6.0, "{level}: {new:?}");
    }
    // Inaudible: the output 120 dB down.
    let mut quiet = world.clone();
    quiet.outputs[0].driver.sensitivity_db = -120.0;
    let pre = sim_capture(&quiet, None, 0);
    let l = Listener::new(&pre, RATE, band(), Played::Sweep(&sweep)).unwrap();
    let step = sim_capture(&quiet, Some(-32.0), 48_000);
    assert!(l.hear(&step).unwrap().snr_db < 6.0);
}

#[cfg(feature = "sim")]
#[test]
fn wp_p5rampfix_sim_rumble_joins_without_a_seam() {
    let mut w = crate::sim::World::flat(1, RATE);
    w.rumble = Some(crate::sim::Rumble {
        dbfs: -40.0,
        top_hz: 80.0,
    });
    let a = sim_capture(&w, None, 0);
    let b = sim_capture(&w, None, 48_000);
    // One function of the frame index: the second render goes on where the
    // first ended, and its RMS is the level.
    let last = f64::from(a[47_999]);
    assert!((crate::sim::rumble_at(&w, 47_999) - last).abs() < 1e-6);
    assert!((crate::sim::rumble_at(&w, 48_000) - f64::from(b[0])).abs() < 1e-6);
    let rms = (a
        .iter()
        .chain(&b)
        .map(|v| f64::from(*v).powi(2))
        .sum::<f64>()
        / 96_000.0)
        .sqrt();
    assert!(
        (20.0 * rms.log10() + 40.0).abs() < 2.0,
        "{}",
        20.0 * rms.log10()
    );
    // Out of range: refused by render, silent in rumble_at.
    w.rumble = Some(crate::sim::Rumble {
        dbfs: 3.0,
        top_hz: 80.0,
    });
    assert_eq!(crate::sim::rumble_at(&w, 10), 0.0);
    let stim = crate::sim::Stimulus {
        bursts: Vec::new(),
        frames: 100,
    };
    assert!(
        crate::sim::render(
            &w,
            &stim,
            &crate::sim::flat_chains(1),
            &crate::sim::ProtoModel,
            RATE
        )
        .is_err()
    );
}

/// Where the ramp settles for the K340 cup (the headphones' ceiling, -30
/// dBFS: the sweep at about -71 dBFS at this microphone), the walked
/// analyser's identity check (a deconvolved peak 30 dB over the median,
/// `ir::FOUND_RATIO`) holds in the real room's noise even for a 1 s sweep;
/// a run's 5.5 s sweeps gain 7 dB more.
#[test]
fn wp_p5rampfix_the_run_finds_in_real_noise_what_the_ramp_settles_on() {
    let x = real_noise();
    let sweep = ramp_sweep();
    let (pre, noise) = (&x[..48_000], &x[48_000..124_800]);
    let l = Listener::new(pre, RATE, band(), Played::Sweep(&sweep)).unwrap();
    let step = with_sweep(noise, &sweep, -71.0, 9_600);
    assert!(l.hear(&step).unwrap().snr_db >= 20.0);
    let rec: Vec<f64> = step.iter().map(|v| f64::from(*v)).collect();
    let h = crate::ir::deconvolve(&rec, &sweep, 48_000.0, (20.0, 20_000.0)).unwrap();
    let mut p: Vec<f64> = h.iter().map(|v| v * v).collect();
    let peak = p.iter().copied().fold(0.0, f64::max);
    p.sort_by(f64::total_cmp);
    let ratio_db = 10.0 * (peak / p[p.len() / 2]).log10();
    assert!(ratio_db >= 36.0, "{ratio_db}");
}
