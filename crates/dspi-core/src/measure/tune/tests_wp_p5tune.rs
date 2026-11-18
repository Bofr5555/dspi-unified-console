//! WP P5TUNE tests of the pure part: proposals from measurement documents
//! made offline, the way the session makes them (P5SIM's world rendered from
//! P5TIME's sweep model, analysed by P5TIME, turned into a document by
//! P5SES's `runs`), no device and no clock.

use std::sync::OnceLock;

use dspi_measure::doc::MeasurementDoc;
use dspi_measure::ir::{AnalysisOptions, WalkedCapture, analyse_walked};
use dspi_measure::response::smooth;
use dspi_measure::sim::{Burst, ProtoModel, Stimulus, World, render, three_way_chains};
use dspi_measure::stimulus::{LogSweep, peak_linear};
use dspi_measure::target::{
    DEFAULT_REFERENCE_BAND, Target, TargetDefinition, TargetUse, default_target,
};
use dspi_measure::types::{ChannelChain, LogGrid, Response};
use proptest::prelude::*;

use super::maths::{self, Allow, Bounds, WayIn};
use super::*;
use crate::measure::plan::Plan;
use crate::measure::runs;

pub(super) const RATE: u32 = 48_000;
/// The test world's speakers' common delay already set on the device, ms.
pub(super) const COMMON_MS: f32 = 3.0;
/// The corners the speakers are designed for, Hz.
pub(super) const F1: f64 = 300.0;
pub(super) const F2: f64 = 3_000.0;
pub(super) const SERIAL: &str = "P5TUNE-DEMO";

/// P5SIM's stereo three-way room with the tweeters `extra_ms` further back
/// and the midranges wired inverted.
pub(super) fn world(seed: u64, extra_ms: f64) -> World {
    let mut w = World::three_way_room(seed);
    for side in 0..2 {
        w.outputs[side * 3 + 1].driver.inverted = true;
        w.outputs[side * 3 + 2].driver.offset_ms += extra_ms;
    }
    w.clock.block_frames = 960;
    w
}

/// The device's chains before the tune: the template's LR4 at 300 Hz and
/// 3 kHz moved 1/3 octave up and 1/4 octave down (on the 0.1 Hz grid), a
/// common 3 ms delay.
pub(super) fn start_chains() -> Vec<ChannelChain> {
    let mut c = three_way_chains();
    let q = |f: f64| ((f * 10.0).round() / 10.0) as f32;
    let f1c = q(F1 * (4.0f64 / 12.0).exp2());
    let f2c = q(F2 * (-3.0f64 / 12.0).exp2());
    for side in 0..2 {
        c[side * 3].crossover[0].freq = f1c;
        c[side * 3 + 1].crossover[0].freq = f1c;
        c[side * 3 + 1].crossover[1].freq = f2c;
        c[side * 3 + 2].crossover[0].freq = f2c;
    }
    for x in &mut c {
        x.delay_ms = COMMON_MS;
    }
    c
}

pub(super) fn role(o: u8) -> MeasureRole {
    match o % 3 {
        0 => MeasureRole::Low,
        1 => MeasureRole::Mid,
        _ => MeasureRole::High,
    }
}

/// The stereo three-way layout (outputs 0 to 2 left, 3 to 5 right).
pub(super) fn layout(outputs: &[u8]) -> MeasureLayout {
    MeasureLayout {
        outputs: outputs
            .iter()
            .map(|o| MeasureLayoutOutput {
                output: *o,
                role: role(*o),
                speaker: if *o < 3 { "L".into() } else { "R".into() },
                group: u8::from(*o >= 3),
                feed_input: Some(u8::from(*o >= 3)),
                ..MeasureLayoutOutput::default()
            })
            .collect(),
        template: "3way".into(),
        notes: Vec::new(),
    }
}

/// A walked measurement of `outputs` of `world` through `chains`, made as
/// the session makes it (one walked run per group), as document text.
pub(super) fn offline_doc(
    world: &World,
    chains: &[ChannelChain],
    outputs: &[u8],
    serial: &str,
) -> String {
    let lay = layout(outputs);
    let setup = MeasureSetup {
        outputs: lay
            .outputs
            .iter()
            .map(|o| MeasureOutput {
                output: o.output,
                role: o.role,
                speaker: o.speaker.clone(),
                group: o.group,
                inverted: false,
            })
            .collect(),
        sweep_ms: 1_000,
        gap_ms: 300,
        ..MeasureSetup::default()
    };
    let plan = Plan::new(&setup).unwrap();
    let level = -20.0f32;
    let mut results = Vec::new();
    for run in &plan.runs {
        let schedule = plan.schedule(run, RATE).unwrap();
        let sweep = LogSweep::new(RATE, &plan.sweep).unwrap();
        let amp = peak_linear(level);
        let first: Vec<f32> = sweep
            .render_cycle(true)
            .iter()
            .map(|v| *v as f32 * amp)
            .collect();
        let regular: Vec<f32> = sweep
            .render_cycle(false)
            .iter()
            .map(|v| *v as f32 * amp)
            .collect();
        let pre = u64::from(RATE);
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
        let stim = Stimulus {
            bursts,
            frames: pre + schedule.total_samples() + u64::from(sweep.gap_samples),
        };
        let capture = render(world, &stim, chains, &ProtoModel, RATE).unwrap();
        let start = world.capture_position(pre as f64, RATE).unwrap().round() as u64;
        let a = analyse_walked(
            &schedule,
            &WalkedCapture {
                samples: &capture.samples,
                first_frame: world.clock.first_frame,
                rate_hz: RATE,
                start_frame: start,
                status: &[],
                complete: true,
            },
            &AnalysisOptions::default(),
        )
        .unwrap();
        results.push(runs::walked_result(&a, 0, level, &plan).unwrap());
    }
    let chains_of: crate::measure::Chains = outputs
        .iter()
        .map(|o| (*o, chains[usize::from(*o)].clone()))
        .collect();
    let now = crate::events::now_ms();
    runs::document(
        &plan,
        &results,
        &runs::DocContext {
            serial,
            platform: "RP2350",
            device_rate_hz: RATE,
            chains: &chains_of,
            saved_ms: now,
            saved_utc: runs::utc_text(u64::try_from(now / 1000).unwrap()),
        },
    )
    .unwrap()
}

/// The ways of `doc` for `lay`, the device's chains `now`.
pub(super) fn ways_of(
    doc: &MeasurementDoc,
    lay: &MeasureLayout,
    now: &[ChannelChain],
) -> Vec<WayIn> {
    lay.outputs
        .iter()
        .map(|lo| {
            let om = doc
                .measurement
                .outputs
                .iter()
                .find(|o| o.output == lo.output)
                .unwrap();
            let response = om.response.to_response();
            let n = response.mag_db.len();
            WayIn {
                output: lo.output,
                role: lo.role,
                speaker: lo.speaker.clone(),
                measured_chain: om.chain.chain.clone(),
                chain_now: now[usize::from(lo.output)].clone(),
                snr_db: maths::stored_snr(om, &response.grid, n),
                arrival_ms: om.arrival_ms,
                spread_db: om.response.spread_db.clone(),
                response,
                feeds: vec![(lo.feed_input.unwrap_or(0), 0.0, false)],
                channel: 4 + lo.output,
            }
        })
        .collect()
}

pub(super) fn speakers_target() -> Response {
    Target {
        target_use: TargetUse::Speakers,
        coupler: None,
        definition: TargetDefinition::Builtin {
            builtin: default_target(TargetUse::Speakers, None),
        },
        reference: DEFAULT_REFERENCE_BAND,
    }
    .evaluate(&LogGrid::STANDARD)
    .unwrap()
}

fn bounds() -> Bounds {
    Bounds {
        min_delay_ms: 0.0,
        max_delay_ms: 42.0,
        highpass: Vec::new(),
        max_boost_db: 3.0,
        boost_from_profile: false,
        headphones: false,
    }
}

/// The left speaker's document of the test world (made once).
fn left_doc() -> &'static str {
    static DOC: OnceLock<String> = OnceLock::new();
    DOC.get_or_init(|| offline_doc(&world(7, 0.3), &start_chains(), &[0, 1, 2], SERIAL))
}

fn compute(doc: &str, options: &MeasureTuneOptions, allow: &Allow) -> maths::Computed {
    let doc = MeasurementDoc::from_text(doc).unwrap();
    let lay = layout(&[0, 1, 2]);
    let ways = ways_of(&doc, &lay, &start_chains());
    maths::compute(
        &doc,
        &ways,
        options,
        &bounds(),
        &speakers_target(),
        allow,
        &CoreSectionModel,
    )
}

/// `r` smoothed to 1/6 octave, minus `target`, levelled by the mean
/// difference over the trusted points of `lo..hi`: (f, error) of each.
pub(super) fn errors(r: &Response, target: &Response, lo: f64, hi: f64) -> Vec<(f64, f64)> {
    let s = smooth(r, 6).unwrap();
    let f = LogGrid::STANDARD.freqs().unwrap();
    let ok = |i: usize| s.valid.as_ref().is_none_or(|v| v[i]);
    let idx: Vec<usize> = (0..f.len())
        .filter(|i| f[*i] >= lo && f[*i] <= hi && ok(*i))
        .collect();
    assert!(idx.len() > 100, "{} trusted points", idx.len());
    let mean = idx
        .iter()
        .map(|i| s.mag_db[*i] - target.mag_db[*i])
        .sum::<f64>()
        / idx.len() as f64;
    idx.iter()
        .map(|i| (f[*i], s.mag_db[*i] - target.mag_db[*i] - mean))
        .collect()
}

/// The room's modes (P5SIM's `three_way_room`): each one's frequency, Hz.
pub(super) const MODES_HZ: [f64; 4] = [42.0, 68.0, 95.0, 130.0];

/// The bass after a tune: no peak more than 2 dB above the target from 30 to
/// 300 Hz, every mode within ±2 dB of it (1/6 octave, levelled by the mean);
/// returns the largest |error| at a mode and the largest peak.
pub(super) fn bass_held(r: &Response, target: &Response) -> (f64, f64) {
    let e = errors(r, target, 30.0, 300.0);
    let peak = e.iter().map(|x| x.1).fold(f64::MIN, f64::max);
    let at_modes = MODES_HZ
        .iter()
        .map(|m| {
            e.iter()
                .min_by(|a, b| (a.0 / m).ln().abs().total_cmp(&(b.0 / m).ln().abs()))
                .unwrap()
                .1
                .abs()
        })
        .fold(0.0, f64::max);
    (at_modes, peak)
}

#[test]
fn wp_p5tune_the_measurement_holds_its_snr() {
    let doc = MeasurementDoc::from_text(left_doc()).unwrap();
    assert!(
        doc.measurement.capture.timing_reliable,
        "{:?}",
        doc.measurement.capture
    );
    for o in &doc.measurement.outputs {
        let r = o.response.to_response();
        let snr = maths::stored_snr(o, &r.grid, r.mag_db.len()).expect("an SNR curve");
        assert!(snr.iter().any(|s| *s > 40.0));
    }
    // A document read and written again keeps it (an unknown key).
    let again = MeasurementDoc::from_text(&doc.to_text().unwrap()).unwrap();
    assert!(again.measurement.outputs[0].extra.contains_key(runs::SNR_KEY));
}

#[test]
fn wp_p5tune_the_proposal_aligns_flips_the_mid_and_keeps_the_common_delay() {
    let c = compute(left_doc(), &MeasureTuneOptions::default(), &Allow::default());
    for w in &c.writes {
        println!("{}", w.words);
    }
    for n in &c.notes {
        println!("note: {n}");
    }
    assert!(c.withdrawn.is_empty(), "{:?}", c.withdrawn);
    let (lo, mid, hi) = (&c.chains[0], &c.chains[1], &c.chains[2]);
    // The inverted midrange is corrected: its net polarity differs from the
    // woofer's and the tweeter's, which agree.
    assert_ne!(mid.feed.inverted, lo.feed.inverted);
    assert_eq!(hi.feed.inverted, lo.feed.inverted);
    // The common delay stays: the smallest delay keeps 3 ms.
    let min = [lo, mid, hi]
        .iter()
        .map(|c| c.delay_ms)
        .fold(f32::MAX, f32::min);
    assert!((min - COMMON_MS).abs() < 1e-4, "{min}");
    // Corners move toward the design (set 1/3 octave above and 1/4 octave
    // below it), each to within 1/4 octave.
    let start = start_chains();
    for (now, was, design) in [
        (lo.crossover[0].freq, start[0].crossover[0].freq, F1),
        (hi.crossover[0].freq, start[2].crossover[0].freq, F2),
    ] {
        let off = |f: f32| (f64::from(f) / design).log2().abs();
        assert!(off(now) < off(was) && off(now) <= 0.25 + 1e-6, "{now} from {was}");
    }
    // Nothing raised; timing writes flagged; the EQ within its caps.
    for (w, b) in c.chains.iter().zip(&start_chains()) {
        assert!(w.gain_db <= b.gain_db + 1e-6);
    }
    assert!(
        c.writes
            .iter()
            .filter(|w| w.kind == MeasureTuneKind::Eq)
            .all(|w| !w.timing)
    );
    assert!(
        c.writes
            .iter()
            .any(|w| w.kind == MeasureTuneKind::Polarity && w.timing)
    );
    for w in c.writes.iter().filter(|w| w.kind == MeasureTuneKind::Eq) {
        for p in &w.params {
            if p.path == "eq.gain" {
                let ParamValue::Float(g) = p.value else {
                    panic!()
                };
                assert!((-15.0..=3.0).contains(&g), "{}", w.words);
            }
        }
    }
}

#[test]
fn wp_p5tune_a_0_3_ms_tweeter_offset_is_corrected() {
    // Two worlds that differ only by the tweeter standing 0.3 ms further
    // back: the tweeter's proposed delay relative to the midrange's differs
    // by that, within 10 µs ("align only", so the corners do not move).
    let options = MeasureTuneOptions {
        keep_crossover: true,
        keep_eq: true,
        ..MeasureTuneOptions::default()
    };
    let near = offline_doc(&world(7, 0.0), &start_chains(), &[0, 1, 2], SERIAL);
    let a = compute(&near, &options, &Allow::default());
    let b = compute(left_doc(), &options, &Allow::default());
    let rel = |c: &maths::Computed| f64::from(c.chains[2].delay_ms - c.chains[1].delay_ms);
    let d = rel(&a) - rel(&b);
    println!(
        "tweeter minus mid: {:.4} and {:.4} ms -> {d:.4} ms",
        rel(&a),
        rel(&b)
    );
    assert!((d - 0.3).abs() <= 0.010, "{d}");
    // Align only: no corner or level write.
    assert!(b.writes.iter().all(|w| !matches!(
        w.kind,
        MeasureTuneKind::Crossover | MeasureTuneKind::Level
    )));
}

#[test]
fn wp_p5tune_the_bass_modes_are_reduced_within_2_db() {
    let c = compute(left_doc(), &MeasureTuneOptions::default(), &Allow::default());
    let sp = &c.speakers[0];
    let target = speakers_target();
    let (modes_before, peak_before) = bass_held(sp.before.as_ref().unwrap(), &target);
    let (modes, peak) = bass_held(sp.predicted.as_ref().unwrap(), &target);
    println!(
        "bass 30 to 300 Hz, 1/6 octave: modes within {modes_before:.2} -> {modes:.2} dB, \
         highest peak {peak_before:+.2} -> {peak:+.2} dB"
    );
    assert!(modes <= 2.0 && peak <= 2.0, "{modes} {peak}");
    assert!(modes < modes_before && peak < peak_before);
    let fit = sp.fit.as_ref().unwrap();
    assert!(fit.rms_after_db < fit.rms_before_db);
    // The fit starts above the lowest trusted point, never at 20 Hz, and
    // uses no broad band (speakers: Q 2 or more).
    assert!(fit.bands.iter().all(|b| f64::from(b.band.freq) > 20.0));
    assert!(fit.bands.iter().all(|b| f64::from(b.band.q) >= maths::SPEAKERS_Q_MIN - 1e-3));
    // Every band of the fit is written, on each way that plays there.
    let eq: Vec<_> = c
        .writes
        .iter()
        .filter(|w| w.kind == MeasureTuneKind::Eq)
        .collect();
    assert!(eq.len() >= fit.bands.len());
    assert!(eq.iter().all(|w| w.output != 2), "no bass band on the tweeter");
}

#[test]
fn wp_p5tune_a_timing_unreliable_measurement_gets_no_timing_writes() {
    let mut doc = MeasurementDoc::from_text(left_doc()).unwrap();
    doc.measurement.capture.timing_reliable = false;
    let c = compute(
        &doc.to_text().unwrap(),
        &MeasureTuneOptions::default(),
        &Allow::default(),
    );
    assert!(c.writes.iter().all(|w| !w.timing), "{:?}", c.writes);
    assert!(c.writes.iter().any(|w| w.kind == MeasureTuneKind::Eq));
    assert!(c.withdrawn.iter().any(|i| i.key == maths::KEY_UNRELIABLE));
    // The bass is fitted on the woofer alone, below half its corner.
    let corner = f64::from(start_chains()[0].crossover[0].freq);
    let fit = c.speakers[0].fit.as_ref().unwrap();
    assert!(
        fit.bands
            .iter()
            .all(|b| f64::from(b.band.freq) <= corner / 2.0 + 1.0)
    );
    assert!(
        c.writes
            .iter()
            .all(|w| w.output == 0 || w.kind == MeasureTuneKind::Headroom)
    );
}

#[test]
fn wp_p5tune_a_failed_summed_check_withdraws_the_timing_and_keeps_the_eq() {
    let mut allow = Allow::default();
    allow.no_timing.insert(
        "L".into(),
        (maths::KEY_SUM_FAILED.into(), "the summed run differs".into()),
    );
    let c = compute(left_doc(), &MeasureTuneOptions::default(), &allow);
    assert!(c.writes.iter().all(|w| !w.timing));
    assert!(c.writes.iter().any(|w| w.kind == MeasureTuneKind::Eq));
    assert!(c.withdrawn.iter().any(|i| i.key == maths::KEY_SUM_FAILED));
}

#[test]
fn wp_p5tune_the_proposal_is_deterministic_and_matches_its_golden() {
    let a = compute(left_doc(), &MeasureTuneOptions::default(), &Allow::default());
    let b = compute(left_doc(), &MeasureTuneOptions::default(), &Allow::default());
    assert_eq!(a.writes, b.writes, "identical input, identical writes (RC10)");
    // The golden is of a document filed once (testdata), so a rendering of
    // the world on another machine does not move it.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/testdata/p5/tune/left-3way.dspipreset"
    );
    let golden = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/testdata/p5/tune/left-3way-proposal.json"
    );
    if std::env::var_os("DSPI_P5TUNE_WRITE_GOLDEN").is_some() {
        std::fs::create_dir_all(std::path::Path::new(path).parent().unwrap()).unwrap();
        std::fs::write(path, left_doc()).unwrap();
        let c = compute(left_doc(), &MeasureTuneOptions::default(), &Allow::default());
        std::fs::write(
            golden,
            serde_json::to_string_pretty(&c.writes).unwrap() + "\n",
        )
        .unwrap();
    }
    let doc =
        std::fs::read_to_string(path).expect("the filed measurement (DSPI_P5TUNE_WRITE_GOLDEN=1)");
    let c = compute(&doc, &MeasureTuneOptions::default(), &Allow::default());
    let want = std::fs::read_to_string(golden).unwrap();
    assert_eq!(
        serde_json::to_string_pretty(&c.writes).unwrap() + "\n",
        want,
        "the proposal's writes are its golden's, exactly"
    );
}

#[test]
fn wp_p5tune_every_notice_key_has_a_row() {
    let src = [include_str!("../tune.rs"), include_str!("maths.rs")];
    let mut keys = Vec::new();
    for s in src {
        for part in s.split('"').skip(1).step_by(2) {
            if part.starts_with("measure.tune.") {
                keys.push(part.to_owned());
            }
        }
    }
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), NOTICE_KEYS.len(), "{keys:?}");
    for k in NOTICE_KEYS {
        assert!(keys.contains(&(*k).to_owned()), "{k}");
        assert!(crate::notice_text::has_row(k), "no notice row for {k}");
        assert_eq!(
            crate::notice_for_key((*k).to_owned(), String::new()).key,
            *k
        );
    }
}

#[test]
fn wp_p5tune_the_layout_of_the_template_is_recognised() {
    let mut s = crate::state::default_state();
    s.num_input_channels = 2;
    s.num_output_channels = 6;
    s.output_enabled = vec![true; 6];
    s.matrix_routing = vec![
        vec![true, true, true, false, false, false],
        vec![false, false, false, true, true, true],
    ];
    s.matrix_gain = vec![vec![0.0; 6]; 2];
    s.matrix_invert = vec![
        vec![false; 6],
        vec![false, false, false, true, false, false],
    ];
    let band = |b: &dspi_measure::types::BandSpec| crate::records::FilterParams {
        filter_type: b.filter_type,
        bypass: b.bypass,
        freq: b.freq,
        q: b.q,
        gain: b.gain,
        qp: b.qp,
    };
    s.filters = vec![vec![]; 2];
    s.xover = vec![vec![]; 2];
    for c in &start_chains() {
        s.filters.push(c.peq.iter().map(band).collect());
        s.xover.push(c.crossover.iter().map(band).collect());
    }
    let l = layout_of(&s);
    assert_eq!(l.template, "3way");
    let roles: Vec<MeasureRole> = l.outputs.iter().map(|o| o.role).collect();
    assert_eq!(roles, (0..6).map(role).collect::<Vec<_>>());
    let speakers: Vec<&str> = l.outputs.iter().map(|o| o.speaker.as_str()).collect();
    assert_eq!(speakers, ["L", "L", "L", "R", "R", "R"]);
    assert_eq!(
        l.outputs.iter().map(|o| o.group).collect::<Vec<_>>(),
        [0, 0, 0, 1, 1, 1]
    );
    // The programme feed: input 2 feeds output 4 inverted.
    assert!(l.outputs[3].feed_inverted && !l.outputs[0].feed_inverted);
    assert_eq!(l.outputs[3].feed_input, Some(1));
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, .. ProptestConfig::default() })]
    /// Whatever the responses and the options, a proposal never panics,
    /// raises no gain, keeps every new PEQ gain within the caps and every
    /// delay in range, and a timing-unreliable one has no timing write.
    #[test]
    fn proptest_wp_p5tune_proposals_stay_bounded(
        noise in proptest::collection::vec(-30.0f64..30.0, 8),
        window in 0.0f64..2.0,
        keep in 0u8..16,
        lo in 0.0f64..200.0,
        hi in 0.0f64..20_000.0,
        unreliable in any::<bool>(),
    ) {
        let mut doc = MeasurementDoc::from_text(left_doc()).unwrap();
        doc.measurement.capture.timing_reliable = !unreliable;
        for (k, o) in doc.measurement.outputs.iter_mut().enumerate() {
            let mut r = o.response.to_response();
            for (i, m) in r.mag_db.iter_mut().enumerate() {
                *m += noise[(i / 97 + k) % noise.len()];
            }
            o.response = dspi_measure::doc::Curve::from_response(&r);
        }
        let options = MeasureTuneOptions {
            window_octaves: window,
            keep_crossover: keep & 1 != 0,
            keep_delays: keep & 2 != 0,
            keep_levels: keep & 4 != 0,
            keep_eq: keep & 8 != 0,
            fit_min_hz: lo,
            fit_max_hz: hi,
            ..MeasureTuneOptions::default()
        };
        let c = compute(&doc.to_text().unwrap(), &options, &Allow::default());
        for (w, b) in c.chains.iter().zip(&start_chains()) {
            prop_assert!(w.gain_db <= b.gain_db + 1e-6);
            prop_assert!((0.0..=42.0).contains(&w.delay_ms));
            for (p, q) in w.peq.iter().zip(&b.peq) {
                if p != q {
                    prop_assert!(p.gain <= 3.0 + 1e-6 && p.gain >= -15.0 - 1e-6);
                }
            }
        }
        if unreliable {
            prop_assert!(c.writes.iter().all(|w| !w.timing));
        }
    }
}
