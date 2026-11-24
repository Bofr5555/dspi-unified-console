//! WP ADET0 tests: the speaker detection (AUTODES 7.4). Pure: the stage
//! decisions over the sim's driver models (AUTODES appendix A), the damage
//! budget of a misclassified tweeter, the probe's interlock, the layout's
//! version 2. On the demo DSPi with the simulated microphone: whole
//! detections, every generator configuration read back and held to the
//! probe's envelope. The demo runs play in real time on the core's clock;
//! nothing is played anywhere.

use std::sync::Arc;
use std::time::Duration;

use dspi_measure::sim::{Driver, OutputPath, SimFilter, filters_response};
use dspi_measure::types::LogGrid;

use super::super::plan::{Plan, Probe};
use super::super::preflight::{SIG_LOG_SWEEP, SIG_PINK};
use super::super::session::below_floor;
use super::super::{MeasureOutput, MeasureRole, MeasureRunKind, MeasureSetup};
use super::*;
use crate::DspiCore;
use crate::records::{FilterParams, SiggenConfig};
use crate::speaker_layout::{
    DetectClass, DetectConfidence, DetectHeard, Detection, LayoutState, RoleSource, SpeakerLayout,
    SpeakerLayoutOutput, role_changes,
};

const SEED: u64 = 0x00AD_E700;
/// The shaped descent's worst case below a 1 kHz floor, s·FS²: every
/// segment played at -50 dBFS, each two octaves from a sixth of an octave
/// under its step, 0.33 + 0.58 + 0.83 + 1 + 1 s below 1 kHz.
const WORST_SHAPED: f64 = 3.75e-5;

// ── The sim's driver models (AUTODES appendix A) ────────────────────────

fn driver(hp: f64, hq: f64, lp: f64, lq: f64, bk: Option<(f64, f64, f64)>) -> Driver {
    Driver {
        sensitivity_db: -14.0,
        highpass_hz: hp,
        highpass_q: hq,
        rolloff_hz: lp,
        rolloff_q: lq,
        breakup: bk.map(|(freq, q, gain_db)| SimFilter::Peak { freq, q, gain_db }),
        offset_ms: 0.0,
        inverted: false,
    }
}

fn func1_woofer() -> Driver {
    driver(46.0, 0.75, 3600.0, 0.7, Some((4300.0, 5.0, 7.0)))
}
fn func1_tweeter() -> Driver {
    driver(1400.0, 0.9, 0.0, 0.7, Some((23_000.0, 3.0, 4.0)))
}
fn centre_mid() -> Driver {
    driver(200.0, 0.75, 6200.0, 0.7, Some((7600.0, 5.0, 5.0)))
}
fn centre_sub() -> Driver {
    driver(25.0, 0.7, 260.0, 0.7, None)
}

/// Every model of the appendix and its expected class and confidence.
fn models() -> Vec<(&'static str, Driver, DetectClass, Option<DetectConfidence>)> {
    use DetectClass::*;
    use DetectConfidence::*;
    vec![
        ("FUNC1 woofer", func1_woofer(), Woofer, None),
        (
            "Centre woofer",
            driver(55.0, 0.75, 1800.0, 0.7, Some((3200.0, 5.0, 6.0))),
            Woofer,
            None,
        ),
        ("Centre mid", centre_mid(), Midrange, None),
        // Its top falls only 3.7 dB (appendix A): it holds, so it is not
        // probed lower and comes out as a tweeter, like a dome midrange
        // (AUTODES 1.6 calls it a midrange; its own appendix disagrees).
        (
            "three_way mid",
            driver(120.0, 0.71, 12_000.0, 0.7, None),
            Tweeter,
            Some(Medium),
        ),
        ("Centre sub", centre_sub(), Sub, None),
        ("FUNC1 tweeter", func1_tweeter(), Tweeter, Some(High)),
        (
            "Centre tweeter",
            driver(1500.0, 0.95, 0.0, 0.7, Some((22_000.0, 3.0, 4.0))),
            Tweeter,
            Some(High),
        ),
        // Owner 2026-11-23: 2.5 dB of roll-off seen is high confidence (it
        // sits at 2.6 dB).
        (
            "Surround tweeter",
            driver(1300.0, 0.9, 0.0, 0.7, Some((23_000.0, 3.0, 4.0))),
            Tweeter,
            Some(High),
        ),
        (
            "three_way tweeter",
            driver(900.0, 0.9, 0.0, 0.7, Some((24_000.0, 3.0, 3.0))),
            Tweeter,
            Some(Medium),
        ),
        (
            "low tweeter 700",
            driver(700.0, 0.9, 0.0, 0.7, None),
            Tweeter,
            Some(Medium),
        ),
        (
            "low tweeter 500",
            driver(500.0, 0.9, 0.0, 0.7, None),
            Tweeter,
            Some(Medium),
        ),
        // Owner 2026-11-23: full-range and dome midranges come out as
        // tweeters (the user corrects them).
        (
            "dome mid",
            driver(400.0, 0.8, 14_000.0, 0.7, None),
            Tweeter,
            Some(Medium),
        ),
        (
            "full range",
            driver(80.0, 0.7, 15_000.0, 0.7, None),
            Tweeter,
            Some(Medium),
        ),
    ]
}

/// The model's level at `f`, dB re its pass band.
fn model_db(d: &Driver, f: f64) -> f64 {
    let (re, im) = filters_response(&d.filters(), f, 48_000.0);
    10.0 * (re * re + im * im).max(1e-30).log10()
}

/// What a session from `f1` to `f2` measures of the model: the standard
/// grid, trusted where the model is within 50 dB of its pass band (the
/// noise elsewhere), smoothed as the detection smooths.
fn measured(d: &Driver, f1: f64, f2: f64) -> Vec<(f64, f64)> {
    smooth(&measured_raw(d, f1, f2))
}

fn measured_raw(d: &Driver, f1: f64, f2: f64) -> Vec<(f64, f64)> {
    let freqs = LogGrid::STANDARD.freqs().unwrap();
    let mag: Vec<f64> = freqs.iter().map(|f| model_db(d, *f)).collect();
    let valid: Vec<bool> = mag.iter().map(|m| *m > -50.0).collect();
    trusted(&freqs, &mag, Some(&valid), f1, f2)
}

/// One detection of `d` by the pure stage decisions, as the driver runs
/// them: the class, the work, and every sweep played `(f1, f2, level)`.
fn probe_model(d: &Driver, floor: f32) -> (Next, Work, Vec<(f32, f32, f32)>) {
    let mut w = Work::new(0, floor);
    let mut played = Vec::new();
    for _ in 0..32 {
        let next = match w.step {
            Step::Top | Step::Band => {
                let (lo, hi) = if w.step == Step::Top {
                    (PROBE_TOP_BAND.0.max(floor), PROBE_TOP_BAND.1)
                } else {
                    (floor, 4_000.0)
                };
                played.push((lo, hi, PROBE_CEILING_DBFS));
                if measured(d, f64::from(lo), f64::from(hi)).len() >= 2 {
                    played.push((floor, 20_000.0, PROBE_CEILING_DBFS));
                    w.level = Some(PROBE_START_DBFS);
                    w.after_shape(measured_raw(d, f64::from(floor), 20_000.0))
                        .unwrap()
                } else {
                    w.not_heard_above()
                }
            }
            Step::Descent(k) => {
                let (f1, f2) = segment_band(w.steps[k]);
                let level = if k >= w.shaped {
                    w.level.unwrap_or(PROBE_CEILING_DBFS)
                } else {
                    PROBE_BELOW_FLOOR_DBFS
                };
                played.push((f1, f2, level));
                w.played(f1);
                w.after_segment(measured(d, f64::from(f1), f64::from(f2)))
            }
        };
        if let Next::Done(..) = next {
            return (next, w, played);
        }
    }
    panic!("the probe never ended: {w:?}");
}

/// The energy below `floor` of the sweeps played, s·FS² (a sweep's time
/// below the floor at its peak level, AUTODES 1.7).
fn energy_below(played: &[(f32, f32, f32)], floor: f32, seg_s: f64) -> f64 {
    played
        .iter()
        .filter(|(f1, _, _)| *f1 < floor)
        .map(|(f1, f2, level)| {
            let (f1, f2, floor) = (f64::from(*f1), f64::from(*f2), f64::from(floor));
            let share = ((floor.min(f2) / f1).ln() / (f2 / f1).ln()).clamp(0.0, 1.0);
            seg_s * share * 10f64.powf(f64::from(*level) / 10.0)
        })
        .sum()
}

#[test]
fn wp_adet0_the_thresholds_classify_the_sims_driver_models() {
    for (name, d, class, confidence) in models() {
        let (next, w, played) = probe_model(&d, 1_000.0);
        let Next::Done(got, conf) = next else {
            unreachable!()
        };
        assert_eq!(got, class, "{name}: {w:?}");
        if let Some(c) = confidence {
            assert_eq!(conf, c, "{name}: {:?}", w.shape);
        }
        // A top that holds never descends: nothing below the floor.
        if got == DetectClass::Tweeter {
            assert!(
                played.iter().all(|(f1, _, _)| *f1 >= 1_000.0),
                "{name}: {played:?}"
            );
        }
        // Whatever plays below the floor plays at the shaped level until
        // the shaped descent has reached 2.5 octaves under the floor.
        let mut shaped_end = false;
        for (f1, f2, level) in &played {
            if *f1 < 1_000.0 && !shaped_end {
                assert!(*level <= PROBE_BELOW_FLOOR_DBFS, "{name}: {played:?}");
                assert!(*f2 <= *f1 * 4.0 + 0.1, "{name}");
            }
            if (*f1 - segment_band(1_000.0 / 2f32.sqrt().powi(5)).0).abs() < 1.0 {
                shaped_end = true;
            }
            assert!(*level <= PROBE_CEILING_DBFS, "{name}");
        }
    }
    // The appendix's readings: the cone midrange's -6 dB point near 150 Hz,
    // the woofers and the sub reach 20 Hz.
    let (_, w, _) = probe_model(&centre_mid(), 1_000.0);
    let f = w.f_low_hz.unwrap();
    assert!((120.0..190.0).contains(&f), "Centre mid f_low {f}");
    let (_, w, _) = probe_model(&func1_woofer(), 1_000.0);
    assert_eq!(w.lowest_hz, Some(20.0));
    assert!(w.f_low_hz.is_none_or(|f| f < 70.0), "{w:?}");
    let (_, w, _) = probe_model(&centre_sub(), 1_000.0);
    assert_eq!(w.lowest_hz, Some(20.0));
    assert_eq!(
        w.heard,
        DetectHeard::AboveFloor,
        "a sub is heard only below 4 kHz"
    );
}

#[test]
fn wp_adet0_a_misclassified_tweeter_gets_at_most_the_damage_budget() {
    // Tweeters forced through "top falls" (a low-pass in front of them: the
    // microphone far off axis, a user's low-pass): each stops in the shaped
    // descent at -50 dBFS, never reaches the free descent, and receives no
    // more below the floor than the shaped descent's worst case
    // ([`WORST_SHAPED`]; AUTODES 1.7 has 2.8e-5 with segments up to 2 kHz,
    // which the two-octave interlock does not allow under 500 Hz).
    let mut worst = 0.0f64;
    for fs in [400.0, 500.0, 700.0, 900.0, 1_100.0, 1_400.0] {
        let d = driver(fs, 0.9, 3_000.0, 0.7, None);
        let (next, w, played) = probe_model(&d, 1_000.0);
        assert!(
            w.shape.is_some_and(|s| s.falls),
            "Fs {fs}: the low-pass makes the top fall"
        );
        assert!(
            matches!(next, Next::Done(DetectClass::Tweeter, _)),
            "Fs {fs}: {next:?} {w:?}"
        );
        assert_eq!(w.steps.len(), w.shaped, "Fs {fs}: entered the free descent");
        assert!(
            played
                .iter()
                .filter(|(f1, _, _)| *f1 < 1_000.0)
                .all(|(_, _, l)| *l <= PROBE_BELOW_FLOOR_DBFS)
        );
        let e = energy_below(&played, 1_000.0, f64::from(PROBE_SEG_MS) / 1000.0);
        assert!(e <= WORST_SHAPED + 1e-9, "Fs {fs}: {e:e} s·FS²");
        worst = worst.max(e);
    }
    // The appendix's 700 Hz tweeter stops at the 354 Hz segment.
    let (_, w, played) = probe_model(&driver(700.0, 0.9, 3_000.0, 0.7, None), 1_000.0);
    assert!((w.stopped_at.unwrap() - 353.6).abs() < 1.0, "{w:?}");
    let e = energy_below(&played, 1_000.0, 1.0);
    assert!(e < 1.8e-5, "{e:e}");
    // Some 1600 times under a second of full-scale music at 500 Hz through a
    // legal 12 dB/octave 1 kHz high-pass (6.3e-2 s·FS²).
    assert!(worst * 1_600.0 < 6.3e-2, "{worst:e}");
}

#[test]
fn wp_adet0_a_raised_floor_moves_every_band() {
    let (next, w, played) = probe_model(&func1_tweeter(), 2_500.0);
    assert!(matches!(next, Next::Done(DetectClass::Tweeter, _)), "{w:?}");
    assert!(played.iter().all(|(f1, _, _)| *f1 >= 2_500.0), "{played:?}");
    let (next, w, played) = probe_model(&func1_woofer(), 2_500.0);
    assert_eq!(
        next,
        Next::Done(DetectClass::Woofer, DetectConfidence::High),
        "{w:?}"
    );
    let shaped: Vec<f32> = shaped_steps(2_500.0);
    assert!((shaped[0] - 1_767.8).abs() < 1.0 && (shaped[4] - 441.9).abs() < 1.0);
    for (f1, _, level) in &played {
        if *f1 < 2_500.0 && *f1 > 390.0 {
            assert!(*level <= PROBE_BELOW_FLOOR_DBFS, "{played:?}");
        }
    }
}

// ── Pure pieces ─────────────────────────────────────────────────────────

#[test]
fn wp_adet0_the_judgement_pieces() {
    // f_low between two bands, an unheard band crossing at its top, none.
    let b = |lo: f64, hi: f64, mean: Option<f64>| Band {
        lo,
        hi,
        mean,
        reference: 0.0,
    };
    let bands = [
        b(1000.0, 1414.0, Some(0.0)),
        b(707.0, 1000.0, Some(-2.0)),
        b(500.0, 707.0, Some(-10.0)),
    ];
    let f = f_low(&bands).unwrap();
    assert!((707.0..841.0).contains(&f), "{f}");
    let bands = [b(1000.0, 1414.0, Some(0.0)), b(707.0, 1000.0, None)];
    assert_eq!(f_low(&bands), Some(1000.0));
    assert_eq!(f_low(&[b(707.0, 1000.0, Some(-1.0))]), None);
    // Leading unheard bands (a sub's top) are passed over.
    let bands = [b(1000.0, 1414.0, None), b(707.0, 1000.0, Some(-1.0))];
    assert_eq!(f_low(&bands), None);
    // A dip higher up that recovered is not the roll-off.
    let bands = [
        b(1000.0, 1414.0, Some(0.0)),
        b(250.0, 354.0, Some(-7.0)),
        b(177.0, 250.0, Some(-1.0)),
        b(63.0, 125.0, Some(-12.0)),
    ];
    let f = f_low(&bands).unwrap();
    assert!((95.0..210.0).contains(&f), "{f}");
    // Each band against the reference it was judged by: a room mode found
    // lower down (the reference rising 8 dB) does not move f_low up.
    let mut bands = vec![
        b(1000.0, 1414.0, Some(0.0)),
        b(500.0, 707.0, Some(0.0)),
        b(32.0, 63.0, Some(8.0)),
        b(20.0, 32.0, Some(-3.0)),
    ];
    bands[2].reference = 8.0;
    bands[3].reference = 8.0;
    let f = f_low(&bands).unwrap();
    assert!((25.0..45.0).contains(&f), "{f}");
    // The classes of a descent.
    let c = |f: Option<f64>, l: Option<f64>, s: Option<f64>| classify_descent(f, l, 0.0, s, 1000.0);
    use DetectClass::*;
    use DetectConfidence::*;
    assert_eq!(c(Some(400.0), None, Some(500.0)), (Tweeter, High));
    assert_eq!(c(Some(220.0), None, Some(250.0)), (Tweeter, Medium));
    assert_eq!(c(Some(150.0), Some(0.0), None), (Midrange, High));
    assert_eq!(c(Some(40.0), Some(-1.0), None), (Woofer, High));
    assert_eq!(c(None, Some(-40.0), None), (Sub, High));
    let (f1, f2) = segment_band(707.1);
    assert!((f1 - 630.0).abs() < 0.5 && (f2 - 4.0 * f1).abs() < 0.01);
    // Steps and alignment.
    let s = shaped_steps(1000.0);
    let want = [707.1, 500.0, 353.6, 250.0, 176.8];
    assert!(
        s.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.2),
        "{s:?}"
    );
    assert_eq!(free_steps(176.8), vec![125.0, 63.0, 32.0, 20.0]);
    assert_eq!(free_steps(441.9), vec![125.0, 63.0, 32.0, 20.0]);
    let old = vec![(1000.0, 0.0), (1200.0, 0.0), (1500.0, 0.0)];
    let new = vec![(800.0, -3.0), (1000.0, 3.0), (1200.0, 3.0), (1500.0, 3.0)];
    let a = align(&old, new);
    assert!(a[1].1.abs() < 1e-9 && (a[0].1 + 6.0).abs() < 1e-9, "{a:?}");
    // At most 6 dB.
    let a = align(&old, vec![(1000.0, 20.0), (1200.0, 20.0)]);
    assert!((a[0].1 - 14.0).abs() < 1e-9);
    // The roles.
    assert_eq!(role_of(Unknown), Some(MeasureRole::High));
    assert_eq!(role_of(Nothing), None);
    assert_eq!(role_of(Off), None);
}

#[test]
fn wp_adet0_the_summary_in_words() {
    let o = |output: u8, class: DetectClass, confidence: DetectConfidence| DetectedOutput {
        output,
        class,
        confidence,
        ..DetectedOutput::default()
    };
    use DetectClass::*;
    use DetectConfidence::*;
    let d = Detection {
        outputs: vec![
            o(0, Tweeter, High),
            o(1, Tweeter, High),
            o(2, Woofer, High),
            o(3, Nothing, High),
            o(4, Woofer, High),
            o(5, Midrange, Medium),
            o(6, Sub, High),
            o(7, Off, Low),
            o(8, Off, Low),
        ],
        ..Detection::default()
    };
    assert_eq!(
        summary_of(&d, 7),
        "2 tweeters, 2 woofers, 1 midrange?, 1 subwoofer; nothing heard on Out 4; \
         Out 8, 9 off, not probed."
    );
    let d = Detection {
        outputs: vec![
            o(0, Tweeter, Medium),
            o(1, Unknown, Low),
            o(2, Unknown, Low),
            o(3, Unknown, Low),
        ],
        stopped: true,
        ..Detection::default()
    };
    assert_eq!(
        summary_of(&d, 1),
        "1 tweeter?; Out 2–4 unknown, treated as tweeters; stopped after 1 of 4 outputs."
    );
}

#[test]
fn wp_adet0_every_notice_key_has_its_row() {
    let keys: Vec<String> = crate::notice_text::notice_texts()
        .into_iter()
        .map(|n| n.key)
        .collect();
    for k in DETECT_NOTICE_KEYS {
        assert!(
            keys.iter().any(|x| x == k),
            "{k} has no row in notices.json"
        );
    }
}

// ── The interlock (pure) ────────────────────────────────────────────────

fn probe_plan(fixed: bool, f1: f32, f2: f32) -> Plan {
    let mut p = Plan::new(&MeasureSetup {
        kind: MeasureRunKind::LiveSweep,
        outputs: vec![MeasureOutput {
            output: 3,
            role: MeasureRole::Full,
            ..MeasureOutput::default()
        }],
        sweep_ms: 1_000,
        ceiling_db: Some(PROBE_CEILING_DBFS),
        ..MeasureSetup::default()
    })
    .unwrap();
    p.probe = Some(Probe {
        floor_hz: 1_000.0,
        below_dbfs: PROBE_BELOW_FLOOR_DBFS,
        ceiling_dbfs: PROBE_CEILING_DBFS,
    });
    if fixed {
        p.fixed_level = true;
        p.start_db = PROBE_BELOW_FLOOR_DBFS;
        p.ceiling_db = PROBE_BELOW_FLOOR_DBFS;
    }
    p.set_sweep_band(f1, f2).unwrap();
    p
}

fn cfg(signal: u8, f1: f32, f2: f32, level: f32) -> SiggenConfig {
    SiggenConfig {
        signal_type: signal,
        channel_mask: 1 << 3,
        invert_mask: 0,
        flags: 0,
        level_db: level,
        duration_ms: 1_000,
        repeat: 1,
        gap_ms: 300,
        p: vec![f1, f2, 0.0, 0.0],
    }
}

#[test]
fn wp_adet0_the_interlock_holds_an_unknown_output_to_the_probe() {
    let p = probe_plan(false, 1_000.0, 20_000.0);
    // Above the floor: a sweep up to the probe's ceiling.
    assert!(below_floor(&p, &cfg(SIG_LOG_SWEEP, 1_000.0, 20_000.0, -24.0)).is_none());
    assert!(below_floor(&p, &cfg(SIG_LOG_SWEEP, 1_000.0, 20_000.0, -23.0)).is_some());
    // Below it: only a sweep of two octaves at most, at -50 dBFS at most.
    assert!(below_floor(&p, &cfg(SIG_LOG_SWEEP, 500.0, 2_000.0, -50.0)).is_none());
    assert!(below_floor(&p, &cfg(SIG_LOG_SWEEP, 500.0, 2_000.0, -49.0)).is_some());
    assert!(below_floor(&p, &cfg(SIG_LOG_SWEEP, 500.0, 2_100.0, -50.0)).is_some());
    assert!(below_floor(&p, &cfg(SIG_LOG_SWEEP, 2_000.0, 500.0, -50.0)).is_none());
    assert!(below_floor(&p, &cfg(SIG_LOG_SWEEP, f32::NAN, 2_000.0, -60.0)).is_some());
    // Nothing but a sweep.
    assert!(below_floor(&p, &cfg(SIG_PINK, 1_000.0, 20_000.0, -60.0)).is_some());
    // Another output of the mask is the plan's ordinary business.
    let mut other = cfg(SIG_PINK, 0.0, 0.0, -60.0);
    other.channel_mask = 1;
    assert!(below_floor(&p, &other).is_none());
    // The plan: a ramped run never starts below the floor; a fixed-level
    // segment of two octaves may, and its (unused) ramp still never does.
    assert_eq!(p.sweep_of(&p.runs[0]).f1_hz, 1_000.0);
    let p = probe_plan(false, 500.0, 2_000.0);
    assert_eq!(p.sweep_of(&p.runs[0]).f1_hz, 1_000.0);
    let p = probe_plan(true, 500.0, 2_000.0);
    assert_eq!(p.sweep_of(&p.runs[0]).f1_hz, 500.0);
    let ramp = super::super::runs::ramp_config(&p, &p.runs[0], 3, -50.0);
    assert!(ramp.p[0] >= 1_000.0, "a ramp never plays below the floor");
    let p = probe_plan(true, 400.0, 2_000.0);
    assert_eq!(
        p.sweep_of(&p.runs[0]).f1_hz,
        1_000.0,
        "wider than two octaves"
    );
    // A probe's plan keeps its outputs' roles off the layout's.
    let mut p = probe_plan(true, 500.0, 2_000.0);
    let layout = SpeakerLayout {
        outputs: vec![SpeakerLayoutOutput {
            output: 3,
            role: MeasureRole::High,
            source: RoleSource::User,
            ..SpeakerLayoutOutput::default()
        }],
        confirmed_by_user: true,
        ..SpeakerLayout::default()
    };
    p.resolve_roles(&Vec::new(), Some(&layout)).unwrap();
    assert_eq!(p.role_of(3), MeasureRole::Full);
    assert_eq!(p.sweep_of(&p.runs[0]).f1_hz, 500.0);
}

// ── The layout's version 2 ──────────────────────────────────────────────

fn row(output: u8, role: MeasureRole, source: RoleSource) -> SpeakerLayoutOutput {
    SpeakerLayoutOutput {
        output,
        role,
        speaker: "L".into(),
        group: 0,
        source,
    }
}

#[test]
fn wp_adet0_the_layouts_state_and_trusted_roles() {
    let mut l = SpeakerLayout {
        outputs: vec![
            row(0, MeasureRole::Low, RoleSource::Detected),
            row(2, MeasureRole::High, RoleSource::Detected),
        ],
        ..SpeakerLayout::default()
    };
    assert_eq!(l.state(), LayoutState::Unconfirmed, "no detection recorded");
    assert_eq!(trusted_roles(&l), None);
    l.detected = Some(Detection::default());
    assert_eq!(l.state(), LayoutState::AutoDetected);
    assert_eq!(
        trusted_roles(&l),
        Some(vec![Some(MeasureRole::Low), None, Some(MeasureRole::High)])
    );
    l.outputs[0].source = RoleSource::User;
    assert_eq!(
        l.state(),
        LayoutState::AutoDetected,
        "the user's row stands"
    );
    l.outputs
        .push(row(3, MeasureRole::Full, RoleSource::Suggested));
    assert_eq!(
        l.state(),
        LayoutState::Unconfirmed,
        "a suggestion is nobody's"
    );
    l.confirmed_by_user = true;
    assert_eq!(l.state(), LayoutState::Confirmed);
}

#[test]
fn wp_adet0_version_1_files_still_load() {
    let v1 = serde_json::json!({
        "kind": "speakerLayout", "version": 1, "serial": "X",
        "outputs": [{"output": 0, "role": "low", "speaker": "L", "group": 0},
                    {"output": 1, "role": "high", "speaker": "L", "group": 0}],
        "confirmedByUser": true, "updatedMs": 5, "notes": []
    });
    let l = SpeakerLayout::from_value(&v1).unwrap();
    assert!(l.outputs.iter().all(|o| o.source == RoleSource::User));
    assert_eq!(l.state(), LayoutState::Confirmed);
    assert!(l.detected.is_none());
    let mut v1u = v1.clone();
    v1u["confirmedByUser"] = serde_json::json!(false);
    let l = SpeakerLayout::from_value(&v1u).unwrap();
    assert!(l.outputs.iter().all(|o| o.source == RoleSource::Suggested));
    assert_eq!(l.state(), LayoutState::Unconfirmed);
    // Version 2 round trip, the detection and the sources kept.
    let mut l2 = l.clone();
    l2.outputs[1].source = RoleSource::Detected;
    l2.outputs[0].source = RoleSource::User;
    l2.detected = Some(Detection {
        device_outputs: 8,
        summary: "1 tweeter.".into(),
        ..Detection::default()
    });
    let doc = l2.to_document();
    assert_eq!(doc["version"], 2);
    assert_eq!(doc["outputs"][1]["source"], "detected");
    let back = SpeakerLayout::from_value(&doc).unwrap();
    assert_eq!(back, l2);
    assert_eq!(back.state(), LayoutState::AutoDetected);
    // Another output count: the detection no longer stands.
    let moved = back.clone().checked_against(4);
    assert_eq!(moved.state(), LayoutState::Unconfirmed);
    assert_eq!(moved.outputs[0].source, RoleSource::User);
    assert_eq!(back.checked_against(8).state(), LayoutState::AutoDetected);
}

#[test]
fn wp_adet0_the_role_changes_in_words() {
    let mut before = SpeakerLayout {
        outputs: vec![
            row(2, MeasureRole::High, RoleSource::Detected),
            row(3, MeasureRole::Low, RoleSource::Detected),
        ],
        detected: Some(Detection {
            outputs: vec![DetectedOutput {
                output: 2,
                class: DetectClass::Tweeter,
                confidence: DetectConfidence::Medium,
                ..DetectedOutput::default()
            }],
            ..Detection::default()
        }),
        ..SpeakerLayout::default()
    };
    let mut after = before.clone();
    after.outputs[0].role = MeasureRole::Mid;
    after.outputs.remove(1);
    after
        .outputs
        .push(row(5, MeasureRole::Sub, RoleSource::User));
    assert_eq!(
        role_changes(Some(&before), &after),
        "Out 3 midrange (was tweeter?), Out 6 subwoofer (added), Out 4 left out"
    );
    before.detected = None;
    assert_eq!(role_changes(Some(&before), &before), "");
}

// ── The demo DSPi and the simulated microphone ──────────────────────────

fn settle(core: &DspiCore) {
    for _ in 0..5 {
        core.process_notifications();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn path(driver: Driver) -> OutputPath {
    OutputPath {
        name: "out".into(),
        speaker: 0,
        driver,
        delay_ms: 8.75,
        gain_db: 0.0,
    }
}

fn unwired() -> Driver {
    Driver {
        sensitivity_db: -200.0,
        ..Driver::flat()
    }
}

/// The demo DSPi ready to detect: the volume at -20 dB, the processing the
/// maths does not model off, outputs `0..drivers.len()` on with flat
/// crossovers and the given drivers in the room, the others off.
fn demo(drivers: Vec<Driver>) -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    core.fetch_all().unwrap();
    core.set_user_volume(-20.0).unwrap();
    let s = core.get_state();
    if s.loudness_enabled {
        core.set_loudness(false).unwrap();
    }
    if s.crossfeed_enabled {
        core.set_crossfeed(false).unwrap();
    }
    if s.psybass_enabled {
        core.set_psybass_enabled(false).unwrap();
    }
    let n_in = s.num_input_channels;
    let on = drivers.len() as u8;
    for o in 0..s.num_output_channels {
        let want = o < on;
        if s.output_enabled[usize::from(o)] != want {
            core.set_output_enable(o, want).unwrap();
        }
        if want && s.output_muted[usize::from(o)] {
            core.set_output_mute(o, false).unwrap();
        }
        if !want {
            continue;
        }
        let bands = core.get_state().xover[usize::from(n_in + o)].len() as u8;
        for b in 0..bands {
            core.set_crossover(
                n_in + o,
                b,
                FilterParams {
                    filter_type: 0,
                    bypass: true,
                    freq: 1_000.0,
                    q: 0.707,
                    gain: 0.0,
                    qp: 0.0,
                },
            )
            .unwrap();
        }
    }
    settle(&core);
    let mut paths: Vec<OutputPath> = drivers.into_iter().map(path).collect();
    while paths.len() < usize::from(s.num_output_channels) {
        paths.push(path(unwired()));
    }
    core.measure_sim_test_paths(paths);
    let info = core.measure_sim_set(true, SEED).unwrap();
    assert!(info.on, "{info:?}");
    core
}

/// Every generator configuration the demo DSPi was given, read back after
/// each tick.
#[derive(Default)]
struct Starts(Vec<SiggenConfig>);

impl Starts {
    fn see(&mut self, core: &DspiCore) {
        if let Ok(c) = core.siggen_get_config()
            && c.channel_mask != 0
            && self.0.last() != Some(&c)
        {
            self.0.push(c);
        }
    }

    fn of(&self, output: u8) -> Vec<&SiggenConfig> {
        self.0
            .iter()
            .filter(|c| c.channel_mask & (1 << output) != 0)
            .collect()
    }

    /// Every configuration that played `output` keeps the probe's envelope
    /// (AUTODES 1.8): a sweep; at most the ceiling; below `floor` at most
    /// -50 dBFS and two octaves, until the shaped descent has reached its
    /// last step (only then the free descent, still under the ceiling).
    fn envelope_holds(&self, output: u8, floor: f32) {
        let last_shaped = segment_band(floor / 2f32.sqrt().powi(5)).0;
        let mut shaped_done = false;
        for c in self.of(output) {
            assert_eq!(c.signal_type, SIG_LOG_SWEEP, "{c:?}");
            assert!(c.level_db <= PROBE_CEILING_DBFS, "{c:?}");
            let (lo, hi) = (c.p[0].min(c.p[1]), c.p[0].max(c.p[1]));
            if lo < floor && !shaped_done {
                assert!(c.level_db <= PROBE_BELOW_FLOOR_DBFS, "out {output}: {c:?}");
                assert!(hi <= lo * 4.0 * 1.001, "out {output}: {c:?}");
            }
            if (lo - last_shaped).abs() < 1.0 {
                shaped_done = true;
            }
        }
    }

    /// The energy below `floor` the output received, s·FS² (peak level
    /// squared times the sweep's time below the floor).
    fn energy_below(&self, output: u8, floor: f32) -> f64 {
        let played: Vec<(f32, f32, f32)> = self
            .of(output)
            .iter()
            .map(|c| (c.p[0].min(c.p[1]), c.p[0].max(c.p[1]), c.level_db))
            .collect();
        energy_below(&played, floor, 1.0)
    }
}

/// The detection's client loop: tick, nothing pushed (the simulated
/// microphone); `stop_after` stops it once that many outputs are done and
/// the next is in its descent.
fn detect(core: &DspiCore, starts: &mut Starts, stop_after: Option<u16>) -> DetectState {
    let end = std::time::Instant::now() + Duration::from_secs(900);
    loop {
        let st = core.detect_tick();
        starts.see(core);
        if let Some(n) = stop_after
            && st.outputs_done >= n
            && st.stage == DetectStage::Descent
        {
            return core.detect_stop();
        }
        if !st.running || std::time::Instant::now() > end {
            return st;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn class_of(st: &DetectState, output: u8) -> (DetectClass, DetectConfidence) {
    let d = st.detection.as_ref().unwrap();
    let o = d.outputs.iter().find(|o| o.output == output).unwrap();
    (o.class, o.confidence)
}

#[test]
fn wp_adet0_sim_a_woofer_and_a_tweeter_are_told_apart() {
    let core = demo(vec![func1_woofer(), func1_tweeter()]);
    let mut starts = Starts::default();
    let st = core.detect_start(DetectSetup::default()).unwrap();
    assert!(st.running, "{st:#?}");
    assert_eq!(st.outputs_total, 2);
    // From the first sound on, the layout is auto-detected: both outputs
    // unknown, tweeters, until their turn.
    let l = core.speaker_layout().unwrap().unwrap();
    assert_eq!(l.state(), LayoutState::AutoDetected);
    assert!(l.outputs.iter().all(|o| o.role == MeasureRole::High));
    let st = detect(&core, &mut starts, None);
    assert_eq!(st.stage, DetectStage::Done, "{st:#?}");
    assert_eq!(class_of(&st, 0).0, DetectClass::Woofer, "{st:#?}");
    assert_eq!(
        class_of(&st, 1),
        (DetectClass::Tweeter, DetectConfidence::High),
        "{st:#?}"
    );
    starts.envelope_holds(0, 1_000.0);
    starts.envelope_holds(1, 1_000.0);
    // The tweeter never got anything below 1 kHz.
    assert!(starts.of(1).iter().all(|c| c.p[0].min(c.p[1]) >= 1_000.0));
    // The woofer went down to 20 Hz.
    assert!(starts.of(0).iter().any(|c| c.p[0] <= 20.0));
    // Every other output was off: not probed, never played.
    let d = st.detection.clone().unwrap();
    assert!(
        d.outputs
            .iter()
            .skip(2)
            .all(|o| o.class == DetectClass::Off)
    );
    assert!(starts.0.iter().all(|c| c.channel_mask & !0b11 == 0));
    // The layout holds the result; the roles are trusted.
    let l = core.speaker_layout().unwrap().unwrap();
    assert_eq!(l.state(), LayoutState::AutoDetected);
    assert_eq!(
        trusted_roles(&l),
        Some(vec![Some(MeasureRole::Low), Some(MeasureRole::High)])
    );
    assert!(
        d.summary.starts_with("1 tweeter, 1 woofer"),
        "{}",
        d.summary
    );
    // A measurement from the layout is no longer refused.
    let setup = core
        .setup_roles(MeasureSetup {
            roles_from_layout: true,
            ..MeasureSetup::default()
        })
        .unwrap();
    assert_eq!(setup.outputs.len(), 2);
    // A second detection is refused once the user has confirmed the layout.
    let mut confirmed = l.clone();
    confirmed.confirmed_by_user = true;
    core.speaker_layout_set(confirmed).unwrap();
    let e = core
        .detect_start(DetectSetup::default())
        .unwrap_err()
        .to_string();
    assert!(e.contains("detect.confirmed_layout"), "{e}");
}

#[test]
fn wp_adet0_sim_a_midrange_and_a_sub() {
    let core = demo(vec![centre_mid(), centre_sub()]);
    let mut starts = Starts::default();
    core.detect_start(DetectSetup::default()).unwrap();
    let st = detect(&core, &mut starts, None);
    assert_eq!(st.stage, DetectStage::Done, "{st:#?}");
    assert_eq!(class_of(&st, 0).0, DetectClass::Midrange, "{st:#?}");
    assert_eq!(class_of(&st, 1).0, DetectClass::Sub, "{st:#?}");
    starts.envelope_holds(0, 1_000.0);
    starts.envelope_holds(1, 1_000.0);
    let l = core.speaker_layout().unwrap().unwrap();
    assert_eq!(
        trusted_roles(&l),
        Some(vec![Some(MeasureRole::Mid), Some(MeasureRole::Sub)])
    );
}

#[test]
fn wp_adet0_sim_an_unwired_output_is_nothing_and_stop_keeps_the_rows() {
    let core = demo(vec![unwired(), func1_woofer(), func1_tweeter()]);
    let mut starts = Starts::default();
    core.detect_start(DetectSetup::default()).unwrap();
    let st = detect(&core, &mut starts, Some(1));
    assert_eq!(st.stage, DetectStage::Stopped, "{st:#?}");
    assert!(!st.running);
    // Nothing on Out 1: never above the ceiling, nothing below the floor
    // above -50 dBFS, and left out of the layout.
    assert_eq!(class_of(&st, 0).0, DetectClass::Nothing, "{st:#?}");
    assert!(
        starts
            .of(0)
            .iter()
            .all(|c| c.level_db <= PROBE_CEILING_DBFS)
    );
    starts.envelope_holds(0, 1_000.0);
    // The woofer stopped in its descent: unknown, a tweeter; the tweeter
    // not reached: unknown too.
    assert_eq!(class_of(&st, 1).0, DetectClass::Unknown);
    assert_eq!(class_of(&st, 2).0, DetectClass::Unknown);
    let d = st.detection.clone().unwrap();
    assert!(d.stopped);
    assert!(d.summary.contains("stopped after 1 of 3"), "{}", d.summary);
    let l = core.speaker_layout().unwrap().unwrap();
    assert_eq!(l.state(), LayoutState::AutoDetected);
    assert!(l.output(0).is_none(), "nothing heard: left out");
    assert_eq!(l.output(1).unwrap().role, MeasureRole::High);
    assert_eq!(l.output(2).unwrap().role, MeasureRole::High);
    // The generator is stopped: nothing plays.
    assert!(!core.measure_state().active);
    core.detect_clear().unwrap();
    assert_eq!(core.detect_state().stage, DetectStage::Idle);
}

#[test]
fn wp_adet0_sim_a_tweeter_forced_through_top_falls_stops_at_354_hz() {
    // A 700 Hz tweeter behind a low-pass (the microphone far off axis): the
    // top falls, the shaped descent stops where its roll-off reaches -8 dB.
    let core = demo(vec![driver(700.0, 0.9, 3_000.0, 0.7, None)]);
    let mut starts = Starts::default();
    core.detect_start(DetectSetup::default()).unwrap();
    let st = detect(&core, &mut starts, None);
    assert_eq!(st.stage, DetectStage::Done, "{st:#?}");
    let d = st.detection.clone().unwrap();
    let o = &d.outputs[0];
    assert_eq!(o.class, DetectClass::Tweeter, "{o:#?}");
    let stop = o.stopped_at_hz.unwrap();
    assert!((stop - 353.6).abs() < 1.0, "{o:#?}");
    starts.envelope_holds(0, 1_000.0);
    assert!(
        starts.of(0).iter().all(|c| c.p[0].min(c.p[1]) >= 310.0),
        "no lower than its stop"
    );
    let e = starts.energy_below(0, 1_000.0);
    assert!(e <= WORST_SHAPED + 1e-9, "{e:e} s·FS²");
}

#[test]
fn wp_adet0_sim_a_raised_floor_moves_every_band() {
    let core = demo(vec![func1_tweeter(), func1_woofer()]);
    let mut starts = Starts::default();
    core.detect_start(DetectSetup {
        floor_hz: 2_500.0,
        ..DetectSetup::default()
    })
    .unwrap();
    let st = detect(&core, &mut starts, None);
    assert_eq!(st.stage, DetectStage::Done, "{st:#?}");
    assert_eq!(class_of(&st, 0).0, DetectClass::Tweeter, "{st:#?}");
    assert_eq!(class_of(&st, 1).0, DetectClass::Woofer, "{st:#?}");
    assert!(starts.of(0).iter().all(|c| c.p[0].min(c.p[1]) >= 2_500.0));
    starts.envelope_holds(0, 2_500.0);
    starts.envelope_holds(1, 2_500.0);
    // A floor under 1 kHz is refused.
    core.detect_clear().unwrap();
    let e = core
        .detect_start(DetectSetup {
            floor_hz: 500.0,
            ..DetectSetup::default()
        })
        .unwrap_err()
        .to_string();
    assert!(e.contains("floor"), "{e}");
}

// -- WP ADETS -- the K340 world (scenario.py k340): a dynamic woofer with its
// low-pass near 5 kHz and a breakup at 6 kHz, probed from the owner's 2.5 kHz
// floor, is a woofer; the transformer-fed tweeter a tweeter.
#[test]
fn wp_adets_the_k340_drivers_classify_from_a_2500_hz_floor() {
    let woofer = driver(20.0, 0.6, 5_000.0, 0.7, Some((6_000.0, 4.0, 5.0)));
    let (next, w, played) = probe_model(&woofer, 2_500.0);
    assert!(
        matches!(next, Next::Done(DetectClass::Woofer, _)),
        "{next:?} {w:?} {played:?}"
    );
    let tweeter = driver(2_000.0, 0.8, 0.0, 0.7, Some((14_000.0, 3.0, 3.0)));
    let (next, w, _) = probe_model(&tweeter, 2_500.0);
    assert!(
        matches!(next, Next::Done(DetectClass::Tweeter, _)),
        "{next:?} {w:?}"
    );
}

// -- WP ADETS -- the same in the sim, on the headphones' levels (-60 / -36 dBFS):
// the woofer's presence run plays louder than its shaped segments
#[test]
fn wp_adets_sim_the_k340_cup_on_the_headphones_levels() {
    let mut woofer = driver(20.0, 0.6, 5_000.0, 0.7, Some((6_000.0, 4.0, 5.0)));
    woofer.sensitivity_db = -4.5;
    let mut tweeter = driver(2_000.0, 0.8, 0.0, 0.7, Some((14_000.0, 3.0, 3.0)));
    tweeter.sensitivity_db = -16.5;
    let core = demo(vec![woofer, tweeter]);
    let mut starts = Starts::default();
    core.detect_start(DetectSetup {
        floor_hz: 2_500.0,
        placement: MeasurePlacement::Headphones,
        ..DetectSetup::default()
    })
    .unwrap();
    let st = detect(&core, &mut starts, None);
    assert_eq!(st.stage, DetectStage::Done, "{st:#?}");
    assert_eq!(class_of(&st, 0).0, DetectClass::Woofer, "{st:#?}");
    assert_eq!(class_of(&st, 1).0, DetectClass::Tweeter, "{st:#?}");
}

// -- WP ADETS -- found by the function test (S19, the Centre's sub): the A2
// ramp heard the sub's tail near 1 kHz, but stage B's sweep had no trusted
// point above the floor; the output ended `unknown`. Silent above the floor
// it goes on (AUTODES 1.3 step 3): after the top band the A2 ramp, after A2
// the shaped descent (where the sub is heard and classified).
#[test]
fn wp_adets_a_stage_b_with_nothing_trusted_goes_on_as_silent_above_the_floor() {
    let mut w = Work::new(4, 1_000.0);
    assert!(matches!(w.after_top(Vec::new()), Next::Session));
    assert_eq!(w.step, Step::Band);
    assert!(matches!(w.after_top(Vec::new()), Next::Session));
    assert_eq!(w.step, Step::Descent(0));
    assert_eq!(w.heard, DetectHeard::Never);
    // The sub, heard from the shaped descent on: a sub
    let sub = centre_sub();
    let mut next = Next::Session;
    for _ in 0..16 {
        let Step::Descent(k) = w.step else {
            unreachable!()
        };
        let (f1, f2) = segment_band(w.steps[k]);
        next = w.after_segment(measured(&sub, f64::from(f1), f64::from(f2)));
        if let Next::Done(..) = next {
            break;
        }
    }
    assert!(
        matches!(next, Next::Done(DetectClass::Sub, _)),
        "{next:?} {w:?}"
    );
    assert_eq!(w.heard, DetectHeard::BelowFloor);
}

// -- RGFIX -- (review RG F1) a second detection on some outputs keeps what
// the first found of the others: their rows (the hard tweeter rule's roles)
// and the `nothing` records; its summary counts its own outputs only.
#[test]
fn rgfix_a_second_detection_on_some_outputs_keeps_the_rest() {
    let core = demo(vec![func1_woofer(), func1_tweeter(), unwired()]);
    let mut starts = Starts::default();
    core.detect_start(DetectSetup::default()).unwrap();
    let st = detect(&core, &mut starts, None);
    assert_eq!(st.stage, DetectStage::Done, "{st:#?}");
    let l = core.speaker_layout().unwrap().unwrap();
    assert_eq!(l.output(0).unwrap().role, MeasureRole::Low);
    assert_eq!(l.output(1).unwrap().role, MeasureRole::High);
    assert!(l.output(2).is_none());
    assert_eq!(l.detected_output(2).unwrap().class, DetectClass::Nothing);
    // The microphone moved near Out 2: detect that one again
    core.detect_clear().unwrap();
    core.detect_start(DetectSetup {
        outputs: vec![1],
        ..DetectSetup::default()
    })
    .unwrap();
    let kept = |l: &SpeakerLayout, when: &str| {
        assert_eq!(l.state(), LayoutState::AutoDetected, "{when}: {l:#?}");
        assert_eq!(
            l.output(0).map(|o| (o.role, o.source)),
            Some((MeasureRole::Low, RoleSource::Detected)),
            "{when}: the woofer's row survives"
        );
        let nothing = l.detected_output(2).unwrap();
        assert_eq!(
            (nothing.class, nothing.heard),
            (DetectClass::Nothing, DetectHeard::Never),
            "{when}: nothing heard on Out 3 is still known"
        );
        assert!(l.output(2).is_none(), "{when}");
        let roles = trusted_roles(l).unwrap();
        assert_eq!(
            roles.first().copied().flatten(),
            Some(MeasureRole::Low),
            "{when}"
        );
        assert_eq!(
            roles.get(1).copied().flatten(),
            Some(MeasureRole::High),
            "{when}"
        );
    };
    kept(
        &core.speaker_layout().unwrap().unwrap(),
        "at the second start",
    );
    let st = detect(&core, &mut starts, None);
    assert_eq!(st.stage, DetectStage::Done, "{st:#?}");
    let l = core.speaker_layout().unwrap().unwrap();
    kept(&l, "at its end");
    assert_eq!(class_of(&st, 1).0, DetectClass::Tweeter, "{st:#?}");
    let d = l.detected.as_ref().unwrap();
    assert_eq!(
        d.outputs.iter().map(|o| o.output).collect::<Vec<_>>(),
        (0..d.device_outputs).collect::<Vec<_>>(),
        "every output's record, in order"
    );
    assert!(d.summary.starts_with("1 tweeter"), "{}", d.summary);
    assert!(!d.summary.contains("woofer"), "{}", d.summary);
    assert!(!d.summary.contains("nothing heard"), "{}", d.summary);
}

/// A limits-profile directory beside the test binary (never tmpfs).
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::current_exe()
        .unwrap()
        .with_file_name("test-scratch")
        .join(format!("rgfix-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// -- RGFIX -- (review RG F2) the limits profile's high-pass minimum raises
// the probe's floor of its output in a detection given no floor (the K340's
// 2.5 kHz on the tweeter outputs, AUTODES owner Q3); the others keep 1 kHz.
#[test]
fn rgfix_sim_the_profiles_highpass_minimum_is_the_detections_floor() {
    use dspi_session::limits::{LimitsProfile, OutputLimits};
    let core = demo(vec![func1_woofer(), func1_tweeter()]);
    core.limits_set_dir(scratch("profile").to_string_lossy().into())
        .unwrap();
    let profile = LimitsProfile {
        outputs: vec![OutputLimits {
            output: 1,
            highpass_min_hz: Some(2_500.0),
            highpass_min_slope_db: Some(12),
            ..OutputLimits::default()
        }],
        ..LimitsProfile::default()
    };
    core.limits_apply_profile(profile.to_document()).unwrap();
    let mut starts = Starts::default();
    core.detect_start(DetectSetup::default()).unwrap();
    let st = detect(&core, &mut starts, None);
    assert_eq!(st.stage, DetectStage::Done, "{st:#?}");
    assert_eq!(class_of(&st, 0).0, DetectClass::Woofer, "{st:#?}");
    assert_eq!(class_of(&st, 1).0, DetectClass::Tweeter, "{st:#?}");
    let d = st.detection.as_ref().unwrap();
    let floor = |o: u8| d.outputs.iter().find(|r| r.output == o).unwrap().floor_hz;
    assert_eq!(floor(0), 1_000.0);
    assert_eq!(floor(1), 2_500.0);
    assert!(!starts.of(1).is_empty());
    assert!(
        starts.of(1).iter().all(|c| c.p[0].min(c.p[1]) >= 2_500.0),
        "{:?}",
        starts.of(1)
    );
    starts.envelope_holds(1, 2_500.0);
    // Out 1 (no profile floor): its band from 1 kHz
    assert!(
        starts.of(0).iter().any(|c| c.p[0].min(c.p[1]) == 1_000.0),
        "{:?}",
        starts.of(0)
    );
    starts.envelope_holds(0, 1_000.0);
}
