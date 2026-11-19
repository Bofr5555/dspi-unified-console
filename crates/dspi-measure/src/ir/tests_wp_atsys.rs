//! WP ATSYS: a single unit's run through the system analyser is P5's, bit
//! for bit: P5TIME's test worlds (clock, latency, rates, passes, every
//! capture fault, the identity failures, the status polls), P5ARRIVAL's
//! tweeter worlds and its two replays of real captures (P5SPIKE), each
//! analysed by `analyse_walked` and by `analyse_system` with one segment;
//! the results, the errors, the fit's intercepts and the gate are equal.
#![allow(clippy::arithmetic_side_effects)]

use super::tests_wp_p5time::{Fault, Way, World, delta_way, render, schedule};
use super::*;
use crate::stimulus::{GeneratorStatus, LogSweep, NO_CHANNEL, SweepConfig};
use crate::system::{
    DomainId, SystemCapture, SystemSchedule, TimePin, UnitIndex, UnitStatusObservation,
    analyse_system,
};

const DOMAIN: DomainId = DomainId {
    time_base: 0,
    id: 0x0042,
};

/// The domain sample of the run's START (any: the pin maps it).
const T0: u64 = 7_654_321;

/// P5's analysis and the fit's absolute intercepts, by the analyser's own
/// slices (as `analyse_walked` runs them).
fn walked(
    sch: &WalkSchedule,
    cap: &WalkedCapture<'_>,
    opts: &AnalysisOptions,
) -> Result<(WalkedAnalysis, Vec<f64>), TimingError> {
    let cap = WalkedCapture {
        complete: true,
        ..*cap
    };
    let mut a = WalkedAnalyser::new(sch, cap.rate_hz, opts)?;
    loop {
        match a.step(&cap)? {
            Step::Done(r) => {
                let fit = a.fit_parts().unwrap();
                let ic = fit
                    .intercepts
                    .iter()
                    .map(|i| i.unwrap() + fit.first_frame)
                    .collect();
                return Ok((*r, ic));
            }
            Step::NeedCapture => panic!("complete capture"),
            Step::Progress { .. } => {}
        }
    }
}

/// Runs both and asserts the same outcome; returns P5's.
fn same_as_walked(
    sch: &WalkSchedule,
    cap: &WalkedCapture<'_>,
    opts: &AnalysisOptions,
) -> Result<WalkedAnalysis, TimingError> {
    let unit = UnitIndex(3);
    let system = SystemSchedule::single(DOMAIN, unit, T0, sch.clone()).unwrap();
    let status: Vec<UnitStatusObservation> = cap
        .status
        .iter()
        .map(|&observation| UnitStatusObservation { unit, observation })
        .collect();
    // A poll of another unit is not this run's: it must not reach P5.
    let mut status_plus = status.clone();
    status_plus.push(UnitStatusObservation {
        unit: UnitIndex(9),
        observation: StatusObservation {
            capture_frame: cap.start_frame,
            status: GeneratorStatus {
                active_channel: 7,
                cycles_done: 99,
            },
        },
    });
    let pins = [
        TimePin {
            capture_frame: cap.start_frame + 10_000,
            t: T0 + 1,
            round_trip_frames: 4_000,
        },
        TimePin {
            capture_frame: cap.start_frame,
            t: T0,
            round_trip_frames: 96,
        },
    ];
    let sys = analyse_system(
        &SystemCapture {
            samples: cap.samples,
            first_frame: cap.first_frame,
            rate_hz: cap.rate_hz,
            schedule: &system,
            pins: &pins,
            status: &status_plus,
            complete: true,
        },
        opts,
    );
    let p5 = walked(sch, cap, opts);
    let direct = analyse_walked(sch, cap, opts);
    match (&p5, &sys, &direct) {
        (Ok((w, ic)), Ok(s), Ok(d)) => {
            // Bit-equal: Debug prints every f64 in its shortest round-trip
            // form, so equal text is equal bits.
            assert_eq!(format!("{w:?}"), format!("{:?}", s.walked));
            assert_eq!(format!("{d:?}"), format!("{:?}", s.walked));
            assert_eq!(w.quality, s.walked.quality);
            assert_eq!(s.timing_reliable, w.quality.reliable);
            assert_eq!(s.outputs.len(), w.outputs.len());
            for ((o, i), wo) in s.outputs.iter().zip(ic).zip(&w.outputs) {
                assert_eq!(o.intercept_frame.to_bits(), i.to_bits());
                assert_eq!(o.key, wo.output);
                assert_eq!(o.arrival_ms.to_bits(), wo.arrival_ms.to_bits());
                assert_eq!(o.members.len(), 1);
                assert_eq!(o.members[0].unit, unit);
                assert_eq!(o.members[0].output, wo.output);
            }
            assert!(s.delays.is_empty(), "one unit: no cross-unit delay");
            assert_eq!(s.t_ref, T0);
            assert_eq!(s.pin.round_trip_frames, 96);
        }
        (Err(a), Err(b), Err(c)) => {
            assert_eq!(format!("{a:?}"), format!("{b:?}"));
            assert_eq!(format!("{a:?}"), format!("{c:?}"));
        }
        _ => panic!(
            "outcomes differ: walked {:?} system {:?}",
            p5.as_ref().map(|_| "ok"),
            sys.as_ref().map(|_| "ok")
        ),
    }
    direct
}

fn rendered_case(world: &World, sch: &WalkSchedule, opts: &AnalysisOptions) -> bool {
    let r = render(world, sch);
    same_as_walked(
        sch,
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
    .is_ok()
}

#[test]
fn wp_atsys_p5time_worlds_give_the_same_result_through_the_system_analyser() {
    let opts = AnalysisOptions::default();
    // The 100 ppm world.
    let sch = schedule(48_000, 2_700, 300, 0b11, 3);
    let mut w = World::new(vec![delta_way(0, 0.001), delta_way(1, 0.0015)]);
    w.ppm = 100.0;
    assert!(rendered_case(&w, &sch, &opts));
    // Two outputs 1.234 ms apart, latency and 80 ppm.
    let sch = schedule(48_000, 2_000, 300, 0b101, 3);
    let mut w = World::new(vec![
        delta_way(0, 0.000_3),
        delta_way(2, 0.000_3 + 0.001_234),
    ]);
    w.ppm = 80.0;
    assert!(rendered_case(&w, &sch, &opts));
    // Two passes, by default and when allowed.
    let sch = schedule(48_000, 1_000, 300, 0b11, 2);
    let w = World::new(vec![delta_way(0, 0.001), delta_way(1, 0.002)]);
    assert!(rendered_case(&w, &sch, &opts));
    let two = AnalysisOptions {
        min_passes_reliable: 2,
        ..AnalysisOptions::default()
    };
    assert!(rendered_case(&w, &sch, &two));
    // One pass (no clock fit).
    let sch1 = schedule(48_000, 1_000, 300, 0b11, 1);
    assert!(rendered_case(&w, &sch1, &opts));
    // A capture at 44.1 kHz of a 48 kHz device, -30 ppm.
    let sch = schedule(48_000, 1_000, 300, 0b11, 3);
    let mut w = World::new(vec![delta_way(0, 0.0005), delta_way(1, 0.0015)]);
    w.capture_rate = 44_100;
    w.ppm = -30.0;
    assert!(rendered_case(&w, &sch, &opts));
    // A 96 kHz device.
    let sch = schedule(96_000, 1_000, 300, 0b11, 3);
    let mut w = World::new(vec![delta_way(0, 0.0005), delta_way(1, 0.0009)]);
    w.capture_rate = 96_000;
    assert!(rendered_case(&w, &sch, &opts));
}

#[test]
fn wp_atsys_every_p5time_capture_fault_gives_the_same_gate() {
    let sch = schedule(48_000, 1_000, 300, 0b111, 3);
    let faults = [
        Fault::StepSlip {
            at_frame: 48_000.0 * 5.5,
            frames: 3.0,
        },
        Fault::StepSlip {
            at_frame: 48_000.0 * 2.0,
            frames: -1.0,
        },
        Fault::DeviceSlip {
            at_sample: 48_000.0 * 4.1,
            samples: 2.0,
        },
        Fault::Wander {
            amp_us: 30.0,
            period_s: 3.0,
        },
        Fault::Wander {
            amp_us: 8.0,
            period_s: 0.7,
        },
    ];
    for f in faults {
        let mut w = World::new(vec![
            delta_way(0, 0.0004),
            delta_way(1, 0.0011),
            delta_way(2, 0.0023),
        ]);
        w.ppm = 40.0;
        w.faults = vec![f];
        assert!(
            rendered_case(&w, &sch, &AnalysisOptions::default()),
            "{f:?}"
        );
    }
}

#[test]
fn wp_atsys_identity_failures_and_status_polls_are_p5s() {
    let sch = schedule(48_000, 1_000, 300, 0b11, 3);
    let world = World::new(vec![delta_way(0, 0.001), delta_way(1, 0.002)]);
    let r = render(&world, &sch);
    let period = sch.period_samples() as f64;
    let zero = r.first_frame as f64 + r.zero_at;
    let at = |dev: f64| (zero + dev) as u64;
    let obs = |dev: f64, ch: u8, done: u16| StatusObservation {
        capture_frame: at(dev),
        status: GeneratorStatus {
            active_channel: ch,
            cycles_done: done,
        },
    };
    let good = [
        obs(0.5 * period, 0, 0),
        obs(2.4 * period, 0, 2),
        obs(7.0 * period, NO_CHANNEL, 6),
    ];
    let bad = [obs(2.4 * period, 1, 3)];
    let opts = AnalysisOptions::default();
    let case = |shift: f64, status: &[StatusObservation]| {
        same_as_walked(
            &sch,
            &WalkedCapture {
                samples: &r.samples,
                first_frame: r.first_frame,
                rate_hz: 48_000,
                start_frame: (r.start_frame as f64 + shift * period) as u64,
                status,
                complete: true,
            },
            &opts,
        )
    };
    assert!(case(0.0, &good).is_ok());
    assert!(matches!(
        case(0.0, &bad),
        Err(TimingError::CycleIdentity(_))
    ));
    for shift in [1.0, -1.0] {
        assert!(matches!(
            case(shift, &[]),
            Err(TimingError::CycleIdentity(_))
        ));
    }
    // A silent output.
    let mut silent = World::new(vec![delta_way(0, 0.001), delta_way(1, 0.002)]);
    silent.ways[1].gain = 0.0;
    assert!(!rendered_case(&silent, &sch, &opts));
}

// ---------------------------------------------------------------------------
// P5ARRIVAL's worlds and replays
// ---------------------------------------------------------------------------

/// P5ARRIVAL's tweeter: two band-limited lobes through a 3.5 kHz high-pass
/// (`tests_wp_p5arrival::tweeter_fir`, rebuilt here: that module keeps it
/// private).
fn tweeter_fir(gap_us: f64, second: f64) -> Vec<f64> {
    let rate = 48_000.0;
    let taps = 160usize;
    let t1 = 12.0;
    let t2 = t1 + gap_us * 1e-6 * rate;
    let fc = 16_000.0 / rate;
    let bl = |x: f64| {
        let w = if x.abs() < 12.0 {
            0.5 + 0.5 * (std::f64::consts::PI * x / 12.0).cos()
        } else {
            0.0
        };
        2.0 * fc * sinc(2.0 * fc * x) * w
    };
    let raw: Vec<f64> = (0..taps)
        .map(|i| bl(i as f64 - t1) - second * bl(i as f64 - t2))
        .collect();
    let w0 = std::f64::consts::TAU * 3_500.0 / rate;
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

#[test]
fn wp_atsys_p5arrival_tweeter_worlds_give_the_same_result() {
    for wander in [false, true] {
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
        rendered_case(&world, &sch, &AnalysisOptions::default());
    }
}

struct RealRun {
    samples: Vec<f32>,
    schedule: WalkSchedule,
    start_frame: u64,
}

/// `tests_wp_p5arrival::real_run`: a P5SPIKE excerpt rebuilt from its
/// manifest (that module keeps the loader private).
fn real_run(name: &str) -> RealRun {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/p5/spike");
    let man: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join(format!("{name}.json"))).unwrap()).unwrap();
    let bytes = std::fs::read(dir.join(format!("{name}.wav"))).unwrap();
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
    let taper = (man["taperMs"].as_f64().unwrap() * 48_000.0 / 1000.0) as usize;
    let mut samples = vec![0.0f32; total];
    let mut read = 0usize;
    for seg in man["segments"].as_array().unwrap() {
        let frames = seg["frames"].as_u64().unwrap() as usize;
        let at = (seg["sourceFrame"].as_i64().unwrap() - origin) as usize;
        let cycle = seg["kind"].as_str() == Some("cycle");
        for i in 0..frames {
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
    let cfg = SweepConfig {
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
    let sweep = LogSweep::new(48_000, &cfg).unwrap();
    RealRun {
        samples,
        schedule: WalkSchedule::new(sweep, mask, passes).unwrap(),
        start_frame: man["startFrame"].as_u64().unwrap(),
    }
}

#[test]
fn wp_atsys_p5arrival_replays_of_real_captures_give_the_same_result() {
    for name in ["only3_p5_excerpt", "ramp30_excerpt"] {
        let run = real_run(name);
        let a = same_as_walked(
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
        );
        assert!(a.is_ok(), "{name}: {a:?}");
    }
}
