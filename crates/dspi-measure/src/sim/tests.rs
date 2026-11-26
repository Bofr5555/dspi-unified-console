//! Tests of the simulated world (WP P5SIM).
#![allow(clippy::arithmetic_side_effects)]

use super::*;
use crate::types::{ChannelChain, MeasureError};
use proptest::prelude::*;

const FS: u32 = 48_000;

fn impulse_stim(at: &[(usize, u64)], frames: u64) -> Stimulus<'static> {
    static ONE: [f32; 1] = [1.0];
    Stimulus {
        bursts: at
            .iter()
            .map(|&(output, start_frame)| Burst {
                output,
                start_frame,
                samples: &ONE,
                gain: 1.0,
            })
            .collect(),
        frames,
    }
}

fn run(world: &World, stim: &Stimulus<'_>, chains: &[ChannelChain]) -> Capture {
    render(world, stim, chains, &ProtoModel, FS).unwrap()
}

/// The capture as a stream indexed by reported frame (for captures with
/// neither a dropped nor a repeated block).
fn stream(c: &Capture) -> &[f32] {
    &c.samples
}

fn argmax_abs(x: &[f32]) -> usize {
    let mut best = 0;
    for (i, v) in x.iter().enumerate() {
        if v.abs() > x[best].abs() {
            best = i;
        }
    }
    best
}

/// The position of a band-limited impulse with sub-sample precision: the
/// phase slope at a low frequency over 512 samples around the largest one
/// (exact for a pure delay; a parabola is biased by up to 0.2 sample).
fn peak(x: &[f32]) -> f64 {
    let i = argmax_abs(x);
    let start = i.saturating_sub(256);
    let end = (i + 256).min(x.len());
    let w = 2.0 * std::f64::consts::PI / 1200.0;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (n, v) in x[start..end].iter().enumerate() {
        let (sn, cs) = (-w * n as f64).sin_cos();
        re += f64::from(*v) * cs;
        im += f64::from(*v) * sn;
    }
    let mut ph = im.atan2(re);
    if x[i] < 0.0 {
        ph += std::f64::consts::PI;
    }
    start as f64 - ph / w
}

/// The DFT of `x` at `f` Hz for rate `fs`: (magnitude dB, phase rad).
fn dft(x: &[f32], f: f64, fs: f64) -> (f64, f64) {
    let (mut re, mut im) = (0.0f64, 0.0f64);
    let w = -2.0 * std::f64::consts::PI * f / fs;
    for (n, v) in x.iter().enumerate() {
        let (s, c) = (w * n as f64).sin_cos();
        re += f64::from(*v) * c;
        im += f64::from(*v) * s;
    }
    (10.0 * (re * re + im * im).max(1e-30).log10(), im.atan2(re))
}

#[test]
fn wp_p5sim_impulse_through_a_flat_world_is_the_impulse() {
    for rate in [44_100u32, 48_000, 96_000] {
        let mut w = World::flat(2, rate);
        w.clock.first_frame = 777;
        let stim = impulse_stim(&[(0, 1000), (1, 2500)], 4000);
        let c = render(&w, &stim, &flat_chains(2), &ProtoModel, rate).unwrap();
        assert_eq!(c.rate_hz, rate);
        let s = stream(&c);
        assert!(s.len() >= 4000);
        for (i, v) in s.iter().enumerate() {
            let want = if i == 1000 || i == 2500 { 1.0 } else { 0.0 };
            assert_eq!(*v, want, "rate {rate} frame {i}");
        }
        // Blocks of 128 from the reported first frame, contiguous.
        // Blocks of 128 from the reported first frame, contiguous; a sample
        // at full scale (the impulse) marks its block clipped.
        for (i, b) in c.blocks.iter().enumerate() {
            assert_eq!(b.first_frame, 777 + 128 * i as u64);
            assert_eq!(b.len, 128);
            assert_eq!(b.clipped, i == 1000 / 128 || i == 2500 / 128);
        }
        assert_eq!(c.pushes().count(), c.blocks.len());
    }
}

#[test]
fn wp_p5sim_a_50_ppm_world_drifts_by_50_ppm() {
    let mut w = World::flat(1, FS);
    w.clock.ppm = 50.0;
    w.clock.latency_ms = 10.0;
    let n = 20 * u64::from(FS);
    let stim = impulse_stim(&[(0, 1000), (0, 1000 + n)], n + 4000);
    let c = run(&w, &stim, &flat_chains(1));
    let s = stream(&c);
    let p1 = peak(&s[..20_000]);
    let p2 = 900_000.0 + peak(&s[900_000..]);
    let ppm = ((p2 - p1) / n as f64 - 1.0) * 1e6;
    assert!((ppm - 50.0).abs() < 0.5, "measured {ppm} ppm");
    // The ground truth agrees with the render and is exact.
    let g1 = w.capture_position(1000.0, FS).unwrap();
    let g2 = w.capture_position((1000 + n) as f64, FS).unwrap();
    assert!(
        (g1 - p1).abs() < 0.1 && (g2 - p2).abs() < 0.1,
        "{g1} {p1} {g2} {p2}"
    );
    assert!((((g2 - g1) / n as f64 - 1.0) * 1e6 - 50.0).abs() < 1e-6);
    // The latency: 10 ms of device time later, on the fast clock.
    assert!((g1 - (1000.0 + 480.0) * (1.0 + 50e-6)).abs() < 1e-6);
}

#[test]
fn wp_p5sim_a_capture_at_another_rate_keeps_time() {
    // Device at 48 kHz, microphone at 44.1 kHz: an impulse at 0.5 s is at
    // 0.5 s of the capture, band-limited.
    let mut w = World::flat(1, 44_100);
    w.clock.rate_hz = 44_100;
    let stim = impulse_stim(&[(0, 24_000)], 48_000);
    let c = run(&w, &stim, &flat_chains(1));
    let p = peak(stream(&c));
    assert!((p - 22_050.0).abs() < 0.1, "{p}");
    // Down-sampling 96k to 48k keeps the level of a 1 kHz tone.
    let tone: Vec<f32> = (0..9600)
        .map(|i| (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / 96_000.0).sin() as f32 * 0.5)
        .collect();
    let mut w = World::flat(1, 48_000);
    w.clock.rate_hz = 48_000;
    let stim = Stimulus {
        bursts: vec![Burst {
            output: 0,
            start_frame: 0,
            samples: &tone,
            gain: 1.0,
        }],
        frames: 9600,
    };
    let c = render(&w, &stim, &flat_chains(1), &ProtoModel, 96_000).unwrap();
    let mid = &c.samples[1000..4000];
    let rms = (mid.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / mid.len() as f64).sqrt();
    assert!((rms - 0.5 / 2f64.sqrt()).abs() < 1e-3, "{rms}");
}

/// A world for the fault tests: flat, an impulse every 0.1 s, noise on so
/// that the noise is seen to move with the microphone's frames.
fn fault_world(faults: Vec<CaptureFault>, noise: bool) -> (World, Stimulus<'static>) {
    let mut w = World::flat(1, FS);
    w.noise_dbfs = noise.then_some(-60.0);
    w.seed = 42;
    w.clock.first_frame = 5000;
    w.faults = faults;
    let at: Vec<(usize, u64)> = (0..20).map(|i| (0, 1000 + 4800 * i)).collect();
    (w, impulse_stim(&at, 100_000))
}

#[test]
fn wp_p5sim_a_dropped_block_is_a_gap_in_the_frame_indices() {
    let (clean_w, stim) = fault_world(vec![], true);
    let clean = run(&clean_w, &stim, &flat_chains(1));
    let (w, _) = fault_world(vec![CaptureFault::DroppedBlock { at_frame: 10_000 }], true);
    let c = run(&w, &stim, &flat_chains(1));
    assert_eq!(c.samples.len(), clean.samples.len() - 128);
    assert_eq!(c.blocks.len(), clean.blocks.len() - 1);
    let lost = 5000 + 9984;
    let gaps: Vec<_> = c
        .blocks
        .windows(2)
        .filter(|p| p[1].first_frame != p[0].first_frame + 128)
        .collect();
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0][0].first_frame + 256, gaps[0][1].first_frame);
    assert_eq!(gaps[0][1].first_frame, lost + 128);
    assert!(c.blocks.iter().all(|b| b.first_frame != lost));
    // Every pushed block is the clean block of the same index.
    for b in &c.blocks {
        let cb = clean
            .blocks
            .iter()
            .find(|x| x.first_frame == b.first_frame)
            .unwrap();
        assert_eq!(c.block_samples(b), clean.block_samples(cb));
    }
    let e = c.events[0];
    assert!(e.visible_in_frame_indices && e.in_capture);
    assert_eq!(e.frame, lost);
}

#[test]
fn wp_p5sim_a_repeated_block_is_pushed_twice_with_the_same_index() {
    let (clean_w, stim) = fault_world(vec![], true);
    let clean = run(&clean_w, &stim, &flat_chains(1));
    let (w, _) = fault_world(vec![CaptureFault::RepeatedBlock { at_frame: 10_000 }], true);
    let c = run(&w, &stim, &flat_chains(1));
    assert_eq!(c.samples.len(), clean.samples.len() + 128);
    let rep = 5000 + 9984;
    let twice: Vec<_> = c.blocks.iter().filter(|b| b.first_frame == rep).collect();
    assert_eq!(twice.len(), 2);
    assert_eq!(c.block_samples(twice[0]), c.block_samples(twice[1]));
    let back: Vec<_> = c
        .blocks
        .windows(2)
        .filter(|p| p[1].first_frame <= p[0].first_frame)
        .collect();
    assert_eq!(back.len(), 1);
    for b in &c.blocks {
        let cb = clean
            .blocks
            .iter()
            .find(|x| x.first_frame == b.first_frame)
            .unwrap();
        assert_eq!(c.block_samples(b), clean.block_samples(cb));
    }
    assert!(c.events[0].visible_in_frame_indices);
    assert_eq!(c.events[0].frame, rep);
}

#[test]
fn wp_p5sim_a_step_slip_shifts_every_later_frame_and_hides_in_the_indices() {
    let (clean_w, stim) = fault_world(vec![], true);
    let clean = run(&clean_w, &stim, &flat_chains(1));
    for n in [128i32, -64, 1] {
        let (w, _) = fault_world(
            vec![CaptureFault::StepSlip {
                at_frame: 10_000,
                frames: n,
            }],
            true,
        );
        let c = run(&w, &stim, &flat_chains(1));
        // The indices are continuous.
        assert!(
            c.blocks
                .windows(2)
                .all(|p| p[1].first_frame == p[0].first_frame + 128)
        );
        let s = stream(&c);
        let cs = stream(&clean);
        for j in 0..10_000 {
            assert_eq!(s[j], cs[j], "slip {n}, frame {j}");
        }
        for (j, v) in s.iter().enumerate().skip(10_000) {
            let k = j as i64 + i64::from(n);
            if (0..cs.len() as i64).contains(&k) {
                assert_eq!(*v, cs[k as usize], "slip {n}, frame {j}");
            }
        }
        let e = c.events[0];
        assert!(!e.visible_in_frame_indices && e.in_capture);
        assert_eq!(e.frame, 15_000);
    }
}

#[test]
fn wp_p5sim_a_device_side_slip_moves_everything_after_it() {
    let (clean_w, stim) = fault_world(vec![], false);
    let clean = run(&clean_w, &stim, &flat_chains(1));
    for n in [100i32, -100] {
        let (w, _) = fault_world(
            vec![CaptureFault::DeviceSlip {
                at_device_frame: 30_000,
                frames: n,
            }],
            false,
        );
        let c = run(&w, &stim, &flat_chains(1));
        assert!(
            c.blocks
                .windows(2)
                .all(|p| p[1].first_frame == p[0].first_frame + 128)
        );
        let s = stream(&c);
        let cs = stream(&clean);
        // Same clock rate, no drift: the shift is exactly n frames.
        for j in 0..30_000 {
            assert_eq!(s[j], cs[j]);
        }
        for j in 30_000 + 200..90_000 {
            assert_eq!(
                s[j],
                cs[(j as i64 - i64::from(n)) as usize],
                "slip {n} frame {j}"
            );
        }
        let e = c.events[0];
        assert!(!e.visible_in_frame_indices && e.in_capture);
        assert_eq!(e.frame, 5000 + 30_000);
    }
}

#[test]
fn wp_p5sim_slow_ratio_wander_bends_the_arrivals() {
    let mut w = World::flat(1, FS);
    w.clock.ppm = 10.0;
    let steady = w.clone();
    w.faults = vec![CaptureFault::RatioWander {
        amplitude_ppm: 100.0,
        period_s: 10.0,
    }];
    // Impulses each second over 10 s.
    let at: Vec<(usize, u64)> = (0..10).map(|i| (0, 1000 + 48_000 * i)).collect();
    let stim = impulse_stim(&at, 10 * 48_000 + 4000);
    let c = run(&w, &stim, &flat_chains(1));
    assert!(
        c.blocks
            .windows(2)
            .all(|p| p[1].first_frame == p[0].first_frame + 128)
    );
    let s = stream(&c);
    let mut largest: f64 = 0.0;
    for &(_, d) in &at {
        let truth = w.capture_position(d as f64, FS).unwrap();
        let from = (truth as usize).saturating_sub(100);
        let got = from as f64 + peak(&s[from..from + 200]);
        assert!((got - truth).abs() < 0.1, "{got} {truth}");
        let unbent = steady.capture_position(d as f64, FS).unwrap();
        largest = largest.max((truth - unbent).abs());
    }
    // 2 * 100 ppm * 10 s * 48 kHz / 2 pi = 15.3 frames at the half period.
    assert!((largest - 15.28).abs() < 0.5, "{largest}");
    assert!(!c.events[0].visible_in_frame_indices);
}

#[test]
fn wp_p5sim_events_report_faults_outside_the_capture() {
    let (w, stim) = fault_world(
        vec![
            CaptureFault::DroppedBlock {
                at_frame: 10_000_000,
            },
            CaptureFault::DeviceSlip {
                at_device_frame: 10_000_000,
                frames: 5,
            },
        ],
        false,
    );
    let c = run(&w, &stim, &flat_chains(1));
    assert!(c.events.iter().all(|e| !e.in_capture));
}

#[test]
fn wp_p5sim_render_is_deterministic_by_seed() {
    let w = World::two_way_near_field(7);
    let sweep = test_sweep(FS, 20.0, 20_000.0, 0.2, 5.0, 0.5).unwrap();
    let stim = Stimulus::walked(&sweep, &[0, 1], 2, 2400, 4800, 2400).unwrap();
    let a = run(&w, &stim, &two_way_chains());
    let b = run(&w, &stim, &two_way_chains());
    assert_eq!(a, b);
    let c = run(&World::two_way_near_field(8), &stim, &two_way_chains());
    assert_ne!(a.samples, c.samples);
}

#[test]
fn wp_p5sim_the_chain_and_the_paths_act_as_specified() {
    // The output delay (whole samples) and gain of the chain.
    let w = World::flat(1, FS);
    let mut chain = flat_chain();
    chain.delay_ms = 1.0;
    chain.gain_db = -6.0;
    let c = run(&w, &impulse_stim(&[(0, 1000)], 4000), &[chain]);
    assert_eq!(argmax_abs(&c.samples), 1048);
    assert!((f64::from(c.samples[1048]) - 10f64.powf(-6.0 / 20.0)).abs() < 1e-6);

    // An LR4 low-pass at 300 Hz from the model: -6 dB at the corner.
    let c = run(
        &w,
        &impulse_stim(&[(0, 100)], 24_100),
        &three_way_chains()[..1],
    );
    let ir = &c.samples[100..24_100];
    assert!((dft(ir, 300.0, 48_000.0).0 + 6.02).abs() < 0.05);
    assert!(dft(ir, 3000.0, 48_000.0).0 < -70.0);

    // A fractional acoustic delay: 1.234 ms is 59.232 samples.
    let mut w = World::flat(1, FS);
    w.outputs[0].delay_ms = 1.234;
    let c = run(&w, &impulse_stim(&[(0, 1000)], 4000), &flat_chains(1));
    // The whole band-limited impulse lies in the segment; at 40 Hz the phase
    // of 512.232 samples does not wrap.
    let seg = &c.samples[547..547 + 2048];
    let (mag, ph) = dft(seg, 40.0, 48_000.0);
    let frac = -ph / (2.0 * std::f64::consts::PI * 40.0 / 48_000.0) - 512.0;
    assert!((frac - 0.232).abs() < 1e-3, "{frac}");
    assert!(mag.abs() < 0.01, "{mag}");

    // A summed run with one output inverted cancels two identical paths.
    let w = World::flat(2, FS);
    let sweep = test_sweep(FS, 50.0, 10_000.0, 0.1, 5.0, 0.5).unwrap();
    let stim = Stimulus::summed(&sweep, &[(0, false), (1, true)], 100, 100).unwrap();
    let c = run(&w, &stim, &flat_chains(2));
    assert!(c.samples.iter().all(|v| v.abs() < 1e-6));
    // An inverted driver flips the impulse.
    let mut w = World::flat(1, FS);
    w.outputs[0].driver.inverted = true;
    let c = run(&w, &impulse_stim(&[(0, 10)], 400), &flat_chains(1));
    assert_eq!(c.samples[10], -1.0);
}

#[test]
fn wp_p5sim_a_reseat_with_a_leak_loses_bass() {
    let mut w = World::headphones(Coupler::Iec60318_4, 1);
    w.noise_dbfs = None;
    let stim = impulse_stim(&[(0, 1000)], 49_000);
    let sealed = run(&w, &stim, &flat_chains(2));
    if let Acoustics::Coupler { reseat, .. } = &mut w.acoustics {
        *reseat = 3; // 60 Hz leak
    }
    let leaky = run(&w, &stim, &flat_chains(2));
    let seg = |c: &Capture| c.samples[1000..48_000].to_vec();
    let (s20, _) = dft(&seg(&sealed), 20.0, 48_000.0);
    let (l20, _) = dft(&seg(&leaky), 20.0, 48_000.0);
    let (s3k, _) = dft(&seg(&sealed), 3000.0, 48_000.0);
    let (l3k, _) = dft(&seg(&leaky), 3000.0, 48_000.0);
    assert!(s20 - l20 > 15.0, "{s20} {l20}");
    assert!((s3k - l3k).abs() < 0.1);
}

#[test]
fn wp_p5sim_couplers_differ_and_the_room_has_its_modes() {
    let mut a = World::headphones(Coupler::Iec60318_4, 1);
    let mut b = World::headphones(Coupler::FlatPlate, 1);
    a.noise_dbfs = None;
    b.noise_dbfs = None;
    let stim = impulse_stim(&[(0, 1000)], 49_000);
    let ca = run(&a, &stim, &flat_chains(2));
    let cb = run(&b, &stim, &flat_chains(2));
    let d = dft(&ca.samples[1000..48_000], 2800.0, 48_000.0).0
        - dft(&cb.samples[1000..48_000], 2800.0, 48_000.0).0;
    assert!(d > 3.0, "{d}");

    // The 42 Hz mode of the room shows on the woofer.
    let mut r = World::three_way_room(1);
    r.noise_dbfs = None;
    r.clock.ppm = 0.0;
    r.clock.latency_ms = 0.0;
    let mut free = r.clone();
    free.acoustics = Acoustics::Free;
    let stim = impulse_stim(&[(0, 1000)], 97_000);
    let chains = three_way_chains();
    let cr = run(&r, &stim, &chains);
    let cf = run(&free, &stim, &chains);
    let m = |c: &Capture, f| dft(&c.samples[1000..97_000], f, 48_000.0).0;
    assert!(m(&cr, 42.0) - m(&cf, 42.0) > 6.0);
}

#[test]
fn wp_p5sim_the_calibration_text_matches_the_microphone() {
    let mic = Microphone::typical();
    let umik = mic.calibration_text(CalFormat::Umik, 48_000.0).unwrap();
    let mut lines = umik.lines();
    assert_eq!(
        lines.next().unwrap(),
        "\"Sens Factor =-1.2500dB, AGain=0dB, SERNO: SIM-7000417\""
    );
    let pts: Vec<(f64, f64)> = lines
        .map(|l| {
            let mut it = l.split('\t');
            (
                it.next().unwrap().parse().unwrap(),
                it.next().unwrap().parse().unwrap(),
            )
        })
        .collect();
    assert_eq!(pts.len(), 132);
    assert!((pts[0].0 - 10.0).abs() < 1e-9 && pts.last().unwrap().0 <= 20_000.0);
    for (f, db) in &pts {
        assert!((db - mic.response_db(*f, 48_000.0)).abs() <= 0.005 + 1e-9);
    }
    // The rendered capture carries the same response: a flat world with
    // this microphone measures it.
    let mut w = World::flat(1, FS);
    w.mic = mic.clone();
    // Half scale, so the microphone's rise does not clip.
    let mut stim = impulse_stim(&[(0, 1000)], 49_000);
    stim.bursts[0].gain = 0.5;
    let c = run(&w, &stim, &flat_chains(1));
    for f in [100.0, 1000.0, 11_000.0, 17_000.0] {
        let got = dft(&c.samples[1000..49_000], f, 48_000.0).0 + 20.0 * 2f64.log10();
        assert!(
            (got - mic.response_db(f, 48_000.0)).abs() < 0.01,
            "{f}: {got}"
        );
    }
    let rew = mic.calibration_text(CalFormat::Rew, 48_000.0).unwrap();
    assert!(rew.starts_with("* "));
    assert_eq!(rew.lines().filter(|l| !l.starts_with('*')).count(), 132);
    let flat = Microphone::flat()
        .calibration_text(CalFormat::Rew, 48_000.0)
        .unwrap();
    assert!(
        flat.lines()
            .filter(|l| !l.starts_with('*'))
            .all(|l| l.ends_with(" 0.00") || l.ends_with(" -0.00"))
    );
}

#[test]
fn wp_p5sim_walked_runs_follow_the_generator_schedule() {
    let sig = vec![0.5f32; 100];
    let s = Stimulus::walked(&sig, &[2, 0, 1], 2, 50, 1000, 300).unwrap();
    let starts: Vec<(usize, u64)> = s.bursts.iter().map(|b| (b.output, b.start_frame)).collect();
    assert_eq!(
        starts,
        vec![
            (2, 1000),
            (0, 1150),
            (1, 1300),
            (2, 1450),
            (0, 1600),
            (1, 1750)
        ]
    );
    // No gap after the last cycle, then the post-roll.
    assert_eq!(s.frames, 1750 + 100 + 300);
    assert!(Stimulus::walked(&sig, &[], 2, 0, 0, 0).is_err());
    assert!(Stimulus::walked(&sig, &[0], 0, 0, 0, 0).is_err());
    assert!(Stimulus::walked(&sig, &[0], 1, 0, MAX_DEVICE_FRAMES, 0).is_err());
    let s = Stimulus::summed(&sig, &[(0, false), (3, true)], 10, 5).unwrap();
    assert_eq!(s.frames, 115);
    assert_eq!(s.bursts[1].gain, -1.0);
}

#[test]
fn wp_p5sim_bad_input_is_a_typed_error() {
    let stim = impulse_stim(&[(0, 10)], 100);
    let bad = |w: &World| {
        matches!(
            render(w, &stim, &flat_chains(w.outputs.len()), &ProtoModel, FS),
            Err(MeasureError::Invalid(_))
        )
    };
    let base = World::flat(1, FS);
    let mut w = base.clone();
    w.clock.ppm = f64::NAN;
    assert!(bad(&w));
    let mut w = base.clone();
    w.outputs[0].driver.sensitivity_db = f64::INFINITY;
    assert!(bad(&w));
    let mut w = base.clone();
    w.clock.block_frames = 0;
    assert!(bad(&w));
    let mut w = base.clone();
    w.outputs.clear();
    assert!(bad(&w));
    let mut w = base.clone();
    w.faults = vec![CaptureFault::StepSlip {
        at_frame: 0,
        frames: i32::MAX,
    }];
    assert!(bad(&w));
    let mut w = base.clone();
    w.acoustics = Acoustics::Coupler {
        coupler: Coupler::Hats,
        leak_hz: vec![0.0],
        reseat: 1,
    };
    assert!(bad(&w));
    // Chains: count, an unmodelled type (the Linkwitz Transform), a model
    // at another rate.
    let w = World::flat(2, FS);
    assert!(render(&w, &stim, &flat_chains(1), &ProtoModel, FS).is_err());
    let mut lt = flat_chains(2);
    lt[0].peq.push(BandSpec {
        filter_type: 11,
        bypass: false,
        freq: 40.0,
        q: 0.7,
        gain: 25.0,
        qp: 0.5,
    });
    assert!(render(&w, &stim, &lt, &ProtoModel, FS).is_err());
    let two_way = two_way_chains();
    assert!(render(&w, &stim, &two_way, &ProtoModel, 96_000).is_err());
    assert!(render(&w, &stim, &flat_chains(2), &ProtoModel, 96_000).is_ok());
    // A burst on an output the world lacks; a NaN sample.
    let nan = [f32::NAN];
    let s = Stimulus {
        bursts: vec![Burst {
            output: 0,
            start_frame: 0,
            samples: &nan,
            gain: 1.0,
        }],
        frames: 10,
    };
    assert!(render(&w, &s, &flat_chains(2), &ProtoModel, FS).is_err());
    assert!(
        render(
            &w,
            &impulse_stim(&[(5, 0)], 10),
            &flat_chains(2),
            &ProtoModel,
            FS
        )
        .is_err()
    );
    // A capture past its bound is refused before anything is rendered.
    let long = impulse_stim(&[(0, 0)], MAX_DEVICE_FRAMES);
    assert!(render(&base, &long, &flat_chains(1), &ProtoModel, FS).is_err());
}

#[test]
fn wp_p5sim_a_capture_exports_as_a_wav_file() {
    let w = World::two_way_near_field(3);
    let sweep = test_sweep(FS, 20.0, 20_000.0, 0.3, 5.0, 0.5).unwrap();
    let stim = Stimulus::walked(&sweep, &[0, 1], 1, 4800, 4800, 4800).unwrap();
    let c = run(&w, &stim, &two_way_chains());
    let wav = crate::wav::read_wav(&c.to_wav(crate::wav::WavFormat::Float32).unwrap(), 0).unwrap();
    assert_eq!(wav.info.rate_hz, 48_000);
    assert_eq!(wav.samples, c.samples);
}

#[test]
fn wp_p5sim_clipping_is_flagged_per_block() {
    let w = World::flat(1, FS);
    let loud = [1.5f32];
    let stim = Stimulus {
        bursts: vec![Burst {
            output: 0,
            start_frame: 300,
            samples: &loud,
            gain: 1.0,
        }],
        frames: 1000,
    };
    let c = run(&w, &stim, &flat_chains(1));
    assert_eq!(c.samples[300], 1.0);
    let flagged: Vec<u64> = c
        .blocks
        .iter()
        .filter(|b| b.clipped)
        .map(|b| b.first_frame)
        .collect();
    assert_eq!(flagged, vec![256]);
}

// -- Goldens of three worlds ------------------------------------------------

/// Third-octave centres from 20 Hz to 20 kHz.
fn third_octaves() -> Vec<f64> {
    (0..31)
        .map(|i| 1000.0 * 2f64.powf((f64::from(i) - 17.0) / 3.0))
        .collect()
}

/// A world's summary: per output, the arrival of its impulse and its
/// magnitude at third octaves; the capture's size and level.
fn summary(world: &World, chains: &[ChannelChain]) -> serde_json::Value {
    let n = world.outputs.len();
    let at: Vec<(usize, u64)> = (0..n).map(|o| (o, 12_000 + 48_000 * o as u64)).collect();
    let stim = impulse_stim(&at, 12_000 + 48_000 * n as u64);
    let c = run(world, &stim, chains);
    let origin = world.clock.first_frame as f64;
    let freqs = third_octaves();
    let mut outs = Vec::new();
    for &(o, d) in &at {
        let start = (world.capture_position(d as f64, FS).unwrap() - origin).floor() as usize;
        let seg = &c.samples[start..start + 24_000];
        let mags: Vec<f64> = freqs
            .iter()
            .map(|f| (dft(seg, *f, 48_000.0).0 * 100.0).round() / 100.0)
            .collect();
        outs.push(serde_json::json!({
            "output": o,
            "name": world.outputs[o].name,
            "arrivalFrames": argmax_abs(seg),
            "magDb": mags,
        }));
    }
    let rms = (c.samples.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>()
        / c.samples.len() as f64)
        .sqrt();
    serde_json::json!({
        "world": world.name,
        "deviceRateHz": FS,
        "captureRateHz": c.rate_hz,
        "frames": c.samples.len(),
        "blocks": c.blocks.len(),
        "rmsDbfs": (20.0 * rms.log10() * 100.0).round() / 100.0,
        "freqsHz": freqs.iter().map(|f| (f * 100.0).round() / 100.0).collect::<Vec<_>>(),
        "outputs": outs,
    })
}

fn golden(file: &str, world: &World, chains: &[ChannelChain]) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/p5/sim")
        .join(file);
    let got = summary(world, chains);
    if std::env::var("DSPI_P5SIM_BLESS").is_ok_and(|v| v == "1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(&got).unwrap() + "\n").unwrap();
        return;
    }
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}; render it with DSPI_P5SIM_BLESS=1", path.display()));
    let want: serde_json::Value = serde_json::from_str(&text).unwrap();
    for key in ["world", "deviceRateHz", "captureRateHz", "frames", "blocks"] {
        assert_eq!(got[key], want[key], "{file}: {key}");
    }
    let close = |a: &serde_json::Value, b: &serde_json::Value, tol: f64| {
        (a.as_f64().unwrap() - b.as_f64().unwrap()).abs() <= tol
    };
    assert!(
        close(&got["rmsDbfs"], &want["rmsDbfs"], 0.05),
        "{file}: rms"
    );
    let (go, wo) = (
        got["outputs"].as_array().unwrap(),
        want["outputs"].as_array().unwrap(),
    );
    assert_eq!(go.len(), wo.len());
    for (g, w) in go.iter().zip(wo) {
        let (ga, wa) = (
            g["arrivalFrames"].as_i64().unwrap(),
            w["arrivalFrames"].as_i64().unwrap(),
        );
        assert!(
            (ga - wa).abs() <= 1,
            "{file}: output {} arrival {ga} vs {wa}",
            g["output"]
        );
        let (gm, wm) = (
            g["magDb"].as_array().unwrap(),
            w["magDb"].as_array().unwrap(),
        );
        assert_eq!(gm.len(), wm.len());
        for (i, (a, b)) in gm.iter().zip(wm).enumerate() {
            assert!(
                close(a, b, 0.05),
                "{file}: output {} band {i}: {a} vs {b}",
                g["output"]
            );
        }
    }
}

#[test]
fn wp_p5sim_golden_three_way_speaker_in_a_room() {
    golden(
        "three_way_room.json",
        &World::three_way_room(1),
        &three_way_chains(),
    );
}

#[test]
fn wp_p5sim_golden_two_way_near_field() {
    golden(
        "two_way_near_field.json",
        &World::two_way_near_field(1),
        &two_way_chains(),
    );
}

#[test]
fn wp_p5sim_golden_headphones_on_the_711_coupler() {
    golden(
        "headphones_711.json",
        &World::headphones(Coupler::Iec60318_4, 1),
        &flat_chains(2),
    );
}

#[test]
fn wp_p5sim_ready_made_worlds_are_valid() {
    for w in [
        World::three_way_room(0),
        World::two_way_near_field(0),
        World::headphones(Coupler::Iec60318_4, 0),
        World::headphones(Coupler::FlatPlate, 0),
        World::headphones(Coupler::Hats, 0),
        World::flat(16, 96_000),
    ] {
        assert!(w.capture_position(0.0, FS).is_ok(), "{}", w.name);
        let s = serde_json::to_string(&w).unwrap();
        assert_eq!(serde_json::from_str::<World>(&s).unwrap(), w);
    }
    let w = World::three_way_room(0);
    assert_eq!(w.outputs.len(), three_way_chains().len());
    assert!((w.acoustic_delay_ms(0).unwrap() - 9.0).abs() < 1e-12);
    assert!((w.acoustic_delay_ms(5).unwrap() - 8.9).abs() < 1e-12);
    assert!(w.acoustic_delay_ms(6).is_none());
}

// -- Property tests ----------------------------------------------------------

fn any_f64() -> impl Strategy<Value = f64> {
    prop_oneof![
        4 => -100.0f64..100.0,
        1 => proptest::num::f64::ANY,
        1 => 0.0f64..30_000.0,
    ]
}

fn any_filter() -> impl Strategy<Value = SimFilter> {
    (0u8..5, any_f64(), any_f64(), any_f64()).prop_map(|(k, freq, q, gain_db)| match k {
        0 => SimFilter::Peak { freq, q, gain_db },
        1 => SimFilter::LowShelf { freq, q, gain_db },
        2 => SimFilter::HighShelf { freq, q, gain_db },
        3 => SimFilter::HighPass { freq, q },
        _ => SimFilter::LowPass { freq, q },
    })
}

fn any_fault() -> impl Strategy<Value = CaptureFault> {
    (0u8..5, 0u64..6000, -300i32..300, any_f64(), any_f64()).prop_map(|(k, at, n, a, p)| match k {
        0 => CaptureFault::DroppedBlock { at_frame: at },
        1 => CaptureFault::RepeatedBlock { at_frame: at },
        2 => CaptureFault::StepSlip {
            at_frame: at,
            frames: n,
        },
        3 => CaptureFault::RatioWander {
            amplitude_ppm: a.abs(),
            period_s: p.abs(),
        },
        _ => CaptureFault::DeviceSlip {
            at_device_frame: at,
            frames: n,
        },
    })
}

fn any_world() -> impl Strategy<Value = World> {
    (
        1usize..4,
        proptest::collection::vec((any_f64(), any_f64(), any_f64(), any::<bool>()), 3),
        proptest::collection::vec(any_filter(), 0..3),
        proptest::collection::vec(any_fault(), 0..3),
        (
            any_f64(),
            any_f64(),
            prop_oneof![
                Just(8000u32),
                Just(44_100),
                Just(48_000),
                Just(96_000),
                any::<u32>()
            ],
            1u32..300,
        ),
        (0u8..3, any_f64(), any_f64(), any::<u64>()),
    )
        .prop_map(
            |(n, drv, mic, faults, (ppm, lat, rate, block), (ac, a1, a2, seed))| {
                let mut w = World::flat(n, rate);
                for (i, o) in w.outputs.iter_mut().enumerate() {
                    let (s, f, d, inv) = drv[i % 3];
                    o.driver.sensitivity_db = s;
                    o.driver.highpass_hz = f.abs();
                    o.driver.rolloff_hz = (f * 7.0).abs();
                    o.driver.breakup = mic.first().copied();
                    o.driver.offset_ms = d.abs() % 120.0;
                    o.driver.inverted = inv;
                    o.delay_ms = (d * 0.1).abs();
                }
                w.mic.response = mic;
                w.faults = faults;
                w.clock.ppm = ppm;
                w.clock.latency_ms = lat.abs() % 50.0;
                w.clock.block_frames = block;
                w.noise_dbfs = Some(-a1.abs());
                w.seed = seed;
                w.acoustics = match ac {
                    0 => Acoustics::Free,
                    1 => Acoustics::Room {
                        modes: vec![RoomMode {
                            freq: a1.abs(),
                            gain_db: a2,
                            t60_s: a2.abs() / 50.0,
                        }],
                        reflections: vec![Reflection {
                            delay_ms: a2.abs() / 10.0,
                            gain: (a1 / 100.0).clamp(-1.0, 1.0),
                        }],
                    },
                    _ => Acoustics::Coupler {
                        coupler: Coupler::FlatPlate,
                        leak_hz: vec![a1.abs()],
                        reseat: 0,
                    },
                };
                w
            },
        )
}

// INTB: CIPAR's proptest case (proptest-regressions/sim/tests.txt): a
// subnormal f1 made f2 / f1 overflow, ln() infinite and every sample NaN.
#[test]
fn intb_test_sweep_refuses_a_span_wider_than_f64() {
    assert!(test_sweep(48_000, 1e-308, 32.9, 0.02, 0.0, 0.5).is_err());
    assert!(test_sweep(48_000, 1e-308, 32.9, 0.02, 0.0, -0.0).is_err());
    // The narrowest span that still fits is a finite sweep
    let s = test_sweep(48_000, 1e-300, 32.9, 0.02, 0.0, 0.5).unwrap();
    assert!(s.iter().all(|v| v.is_finite() && v.abs() <= 0.5 + 1e-6));
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    #[test]
    fn proptest_wp_p5sim_render_never_panics_and_stays_finite(
        world in any_world(),
        sig in proptest::collection::vec(-1.0f32..1.0, 1..200),
        starts in proptest::collection::vec((0usize..4, 0u64..3000, -2.0f32..2.0), 0..4),
        frames in 1u64..4000,
        rate in prop_oneof![Just(48_000u32), Just(44_100), Just(96_000), any::<u32>()],
        chains_kind in 0u8..3,
    ) {
        let stim = Stimulus {
            bursts: starts
                .iter()
                .map(|&(output, start_frame, gain)| Burst { output, start_frame, samples: &sig, gain })
                .collect(),
            frames,
        };
        let n = world.outputs.len();
        let chains = match chains_kind {
            0 => flat_chains(n),
            1 => three_way_chains().into_iter().cycle().take(n).collect(),
            _ => {
                let mut c = flat_chains(n);
                c[0].delay_ms = 0.5;
                c[0].gain_db = -3.0;
                c
            }
        };
        if let Ok(c) = render(&world, &stim, &chains, &ProtoModel, rate) {
            prop_assert!(c.samples.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
            let total: usize = c.blocks.iter().map(|b| b.len).sum();
            prop_assert_eq!(total, c.samples.len());
            prop_assert!(c.blocks.iter().all(|b| c.block_samples(b).len() == b.len));
            prop_assert_eq!(c.events.len(), world.faults.len());
        }
        let _ = world.capture_position(f64::from(rate), rate);
    }

    #[test]
    fn proptest_wp_p5sim_test_sweep_is_bounded(rate in any::<u32>(), f1 in any_f64(), f2 in any_f64(), dur in prop_oneof![0.0f64..0.05, proptest::num::f64::ANY], fade in any_f64(), amp in any_f64()) {
        if let Ok(s) = test_sweep(rate % 200_000, f1, f2, dur, fade, amp) {
            prop_assert!(s.iter().all(|v| v.is_finite() && f64::from(v.abs()) <= amp + 1e-6));
        }
    }

    #[test]
    fn proptest_wp_p5sim_calibration_text_is_finite(mic in proptest::collection::vec(any_filter(), 0..4), sens in any_f64(), fs in any_f64(), rew in any::<bool>()) {
        let m = Microphone { response: mic, sens_factor_db: sens, serial: "x y\n\"z".into() };
        let fmt = if rew { CalFormat::Rew } else { CalFormat::Umik };
        if let Ok(t) = m.calibration_text(fmt, fs) {
            prop_assert!(!t.contains("NaN") && !t.contains("inf"));
            prop_assert!(t.lines().count() >= 132);
        }
    }

    #[test]
    fn proptest_wp_p5sim_capture_position_is_monotonic(ppm in -1000.0f64..1000.0, lat in 0.0f64..1000.0, amp in 0.0f64..1000.0, per in 0.1f64..100.0, a in 0.0f64..1e7, b in 0.0f64..1e7) {
        let mut w = World::flat(1, FS);
        w.clock.ppm = ppm;
        w.clock.latency_ms = lat;
        w.faults = vec![CaptureFault::RatioWander { amplitude_ppm: amp.min(500.0), period_s: per }];
        let (pa, pb) = (w.capture_position(a, FS).unwrap(), w.capture_position(b, FS).unwrap());
        prop_assert!(pa.is_finite() && pb.is_finite());
        if a < b { prop_assert!(pa <= pb); }
    }

    #[test]
    fn proptest_wp_p5sim_helpers_are_finite(
        filters in proptest::collection::vec(any_filter(), 0..4),
        f in any_f64(),
        fs in any_f64(),
        mode in (any_f64(), any_f64(), any_f64()),
        band in (any::<u8>(), any::<bool>(), proptest::num::f32::ANY, proptest::num::f32::ANY, proptest::num::f32::ANY),
        block in (any::<u64>(), any::<usize>(), 0usize..100),
        out in any::<usize>(),
    ) {
        let mic = Microphone { response: filters.clone(), sens_factor_db: 0.0, serial: String::new() };
        prop_assert!(mic.response_db(f, fs).is_finite());
        let room = RoomMode { freq: mode.0, gain_db: mode.1, t60_s: mode.2 };
        if let SimFilter::Peak { q, .. } = room.filter() {
            prop_assert!(!mode.0.is_finite() || q.is_finite());
        }
        for c in [Coupler::Iec60318_4, Coupler::FlatPlate, Coupler::Hats] {
            prop_assert!(coupler_filters(c).iter().all(|x| x.magnitude_db(f, fs).is_finite()));
        }
        let d = Driver { breakup: filters.first().copied(), highpass_hz: f, rolloff_hz: fs, ..Driver::flat() };
        prop_assert!(d.filters().len() <= 3);
        let b = BandSpec { filter_type: band.0, bypass: band.1, freq: band.2, q: band.3, gain: band.4, qp: 0.0 };
        let secs = ProtoModel.sections(&b);
        prop_assert!(ProtoModel.models(&b) || secs.is_empty());
        let w = World::three_way_room(0);
        prop_assert!(w.acoustic_delay_ms(out).is_none_or(f64::is_finite));
        let cap = Capture { rate_hz: 48_000, samples: vec![0.25; 50], blocks: vec![], events: vec![] };
        let cb = CaptureBlock { first_frame: block.0, offset: block.1, len: block.2, clipped: false };
        prop_assert!(cap.block_samples(&cb).len() <= 50);
        prop_assert!(cap.to_wav(crate::wav::WavFormat::Pcm16).is_ok());
    }

    #[test]
    fn proptest_wp_p5sim_stimulus_builders_never_panic(len in 0usize..100, outs in proptest::collection::vec(0usize..20, 0..20), passes in 0u32..10_000, gap in any::<u64>(), start in any::<u64>(), post in any::<u64>()) {
        let sig = vec![0.1f32; len];
        if let Ok(s) = Stimulus::walked(&sig, &outs, passes, gap, start, post) {
            prop_assert!(s.frames <= MAX_DEVICE_FRAMES && s.bursts.len() <= MAX_BURSTS);
        }
        let pairs: Vec<(usize, bool)> = outs.iter().map(|o| (*o, o % 2 == 0)).collect();
        if let Ok(s) = Stimulus::summed(&sig, &pairs, start, post) {
            prop_assert!(s.frames <= MAX_DEVICE_FRAMES);
        }
    }
}
