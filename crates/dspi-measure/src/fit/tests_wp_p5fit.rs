//! WP P5FIT: the bounded EQ fit, on synthetic rooms and headphones.
#![allow(clippy::arithmetic_side_effects)]

use super::*;
use crate::types::{LogGrid, ProgrammeFeed};
use dspi_proto::FilterType;
use dspi_proto::dsp::{Band, coefficients};
use proptest::prelude::*;

/// The plain test model over `dspi_proto::dsp` and `dspi_proto::xover` at
/// 48 kHz (the core's `SectionModel` is P5DOCK's).
struct PlainModel;

fn proto_band(b: &BandSpec) -> Band {
    Band {
        filter_type: FilterType::from_raw(b.filter_type),
        freq: b.freq,
        q: b.q,
        gain_db: b.gain,
        bypass: b.bypass,
    }
}

impl SectionModel for PlainModel {
    fn sections(&self, band: &BandSpec) -> Vec<Coeffs> {
        let t = FilterType::from_raw(band.filter_type);
        if band.bypass || t == FilterType::Flat || !self.models(band) {
            Vec::new()
        } else if t.is_crossover() {
            dspi_proto::xover::sections(band.filter_type, band.freq)
        } else {
            vec![coefficients(&proto_band(band))]
        }
    }
    fn models(&self, band: &BandSpec) -> bool {
        let t = FilterType::from_raw(band.filter_type);
        if band.bypass || t == FilterType::Flat {
            return true;
        }
        if t.is_crossover() {
            return dspi_proto::xover::meta(band.filter_type).is_some();
        }
        matches!(band.filter_type, 1..=10 | 12 | 13)
    }
    fn sample_rate(&self) -> f64 {
        48_000.0
    }
}

const FLAT: BandSpec = BandSpec {
    filter_type: 0,
    bypass: false,
    freq: 1000.0,
    q: 0.707,
    gain: 0.0,
    qp: 0.0,
};

fn chain() -> ChannelChain {
    ChannelChain {
        peq: vec![FLAT; 10],
        crossover: vec![FLAT; 2],
        gain_db: 0.0,
        delay_ms: 0.0,
        feed: ProgrammeFeed {
            gain_db: 0.0,
            inverted: false,
        },
    }
}

fn freqs() -> Vec<f64> {
    LogGrid::STANDARD.freqs().unwrap()
}

/// A resonance (or, with `peak_db` < 0, an anti-resonance) of a room mode:
/// `|1 + g H_bp(f)|` in dB, `g` set so the value at `f0` is `peak_db`.
fn mode_db(f: f64, f0: f64, q: f64, peak_db: f64) -> f64 {
    let g = 10f64.powf(peak_db / 20.0) - 1.0;
    let x = f / f0;
    let (re, im) = (1.0 - x * x, x / q);
    let den = re * re + im * im;
    // H_bp = j(x/q) / (re + j im)
    let h_re = (im * im) / den;
    let h_im = (im * re) / den;
    let (r, i) = (1.0 + g * h_re, g * h_im);
    10.0 * (r * r + i * i).log10()
}

fn response(mag: Vec<f64>) -> Response {
    Response {
        grid: LogGrid::STANDARD,
        mag_db: mag,
        phase_deg: None,
        valid: None,
        spl: false,
    }
}

fn room(level: f64, modes: &[(f64, f64, f64)]) -> Response {
    response(
        freqs()
            .iter()
            .map(|&f| {
                level
                    + modes
                        .iter()
                        .map(|&(f0, q, g)| mode_db(f, f0, q, g))
                        .sum::<f64>()
            })
            .collect(),
    )
}

fn flat_target() -> Response {
    response(vec![0.0; freqs().len()])
}

/// The fitted bands' summed dB at `f` (through the plain model).
fn bands_db(r: &FitResult, f: f64) -> f64 {
    let bands: Vec<Band> = r.bands.iter().map(|b| proto_band(&b.band)).collect();
    dspi_proto::dsp::response_at(f, &bands)
}

/// Predicted minus measured, without the headroom cut: what the EQ changed.
fn change(r: &FitResult, m: &Response) -> Vec<f64> {
    r.predicted
        .mag_db
        .iter()
        .zip(m.mag_db.iter())
        .map(|(p, m)| p - m - f64::from(r.gain_cut_db))
        .collect()
}

fn three_modes() -> Response {
    room(
        80.0,
        &[(45.0, 6.0, 9.0), (90.0, 5.0, 6.0), (170.0, 7.0, 7.0)],
    )
}

#[test]
fn wp_p5fit_three_room_modes_flattened_within_1_5_db_with_at_most_5_bands() {
    let m = three_modes();
    let r = fit(
        &m,
        &flat_target(),
        &chain(),
        &PlainModel,
        &FitOptions::speakers(),
    )
    .unwrap();
    assert!(!r.bands.is_empty() && r.bands.len() <= 5, "{:#?}", r.bands);
    let mut worst: f64 = 0.0;
    for ((&f, p), _) in freqs()
        .iter()
        .zip(r.predicted.mag_db.iter())
        .zip(m.mag_db.iter())
    {
        if (20.0..=300.0).contains(&f) {
            let e = p - f64::from(r.gain_cut_db) - r.target_offset_db;
            worst = worst.max(e.abs());
        }
    }
    assert!(worst <= 1.5, "worst {worst} dB; {:#?}", r.notes);
    assert!(r.rms_after_db < r.rms_before_db);
    assert!(
        (r.target_offset_db - 80.0).abs() < 1.0,
        "{}",
        r.target_offset_db
    );
    // All three are cuts: no headroom needed.
    assert!(r.bands.iter().all(|b| b.band.gain < 0.0), "{:#?}", r.bands);
    assert_eq!(r.gain_cut_db, 0.0);
    assert!(r.evaluations <= DEFAULT_EVALUATIONS);
}

#[test]
fn wp_p5fit_identical_input_gives_identical_quantised_bands() {
    let m = three_modes();
    let a = fit(
        &m,
        &flat_target(),
        &chain(),
        &PlainModel,
        &FitOptions::speakers(),
    )
    .unwrap();
    let b = fit(
        &m,
        &flat_target(),
        &chain(),
        &PlainModel,
        &FitOptions::speakers(),
    )
    .unwrap();
    assert_eq!(a, b);
    for x in &a.bands {
        let g = f64::from(x.band.gain) * 10.0;
        assert!(
            (g - g.round()).abs() < 1e-3,
            "gain on the 0.1 dB grid: {x:?}"
        );
        let f = f64::from(x.band.freq) * 10.0;
        assert!(
            (f - f.round()).abs() < 1e-2,
            "freq on the 0.1 Hz grid: {x:?}"
        );
        let q = f64::from(x.band.q) * 100.0;
        assert!((q - q.round()).abs() < 1e-2, "Q on the 0.01 grid: {x:?}");
    }
}

/// The quantised proposal for the three-mode room, exactly (RC10: quantised
/// proposals compare exactly on one target).
#[test]
fn wp_p5fit_three_mode_proposal_golden() {
    let m = three_modes();
    let r = fit(
        &m,
        &flat_target(),
        &chain(),
        &PlainModel,
        &FitOptions::speakers(),
    )
    .unwrap();
    let got: Vec<(u8, u8, f32, f32, f32)> = r
        .bands
        .iter()
        .map(|b| {
            (
                b.index,
                b.band.filter_type,
                b.band.freq,
                b.band.q,
                b.band.gain,
            )
        })
        .collect();
    let golden: &[(u8, u8, f32, f32, f32)] = GOLDEN_THREE_MODES;
    assert_eq!(got, golden, "notes: {:#?}", r.notes);
}

/// (index, type, Hz, Q, dB): the three modes cut, nothing else.
///
/// Re-blessed at the wave P5-A review: the fit now smooths with the crate's
/// one kernel (`response::smooth_magnitudes`, power domain: the window's end
/// points count half, where the fit's own helper counted them whole), which
/// moved each Q by 0.02; frequencies and gains are unchanged.
const GOLDEN_THREE_MODES: &[(u8, u8, f32, f32, f32)] = &[
    (0, 1, 45.0, 3.55, -9.0),
    (1, 1, 90.0, 3.51, -6.0),
    (2, 1, 170.1, 4.63, -7.0),
];

#[test]
fn wp_p5fit_15_db_narrow_null_gets_at_most_the_max_boost() {
    let m = room(75.0, &[(60.0, 4.0, 6.0), (120.0, 8.0, -15.0)]);
    let opts = FitOptions::speakers();
    let r = fit(&m, &flat_target(), &chain(), &PlainModel, &opts).unwrap();
    let d = change(&r, &m);
    let peak = d.iter().copied().fold(f64::MIN, f64::max);
    assert!(peak <= opts.max_boost_db + 1e-6, "boost {peak}");
    for (&f, _) in freqs().iter().zip(d.iter()) {
        assert!(bands_db(&r, f) <= opts.max_boost_db + 1e-6);
    }
    assert!(
        r.notes.iter().any(|n| n.contains("deeper than 10 dB")),
        "{:#?}",
        r.notes
    );
    // The headroom cut equals the net boost, on the 0.1 dB grid.
    if peak > 0.0 {
        assert!(f64::from(-r.gain_cut_db) >= peak - 1e-6);
        assert!(f64::from(-r.gain_cut_db) < peak + 0.1 + 1e-6);
    }
}

fn dip_room() -> Response {
    room(75.0, &[(50.0, 5.0, 8.0), (140.0, 4.0, -8.0)])
}

fn at(fs: &[f64], v: &[f64], f: f64) -> f64 {
    let i = fs.partition_point(|&x| x < f);
    v[i]
}

#[test]
fn wp_p5fit_dip_that_varies_across_positions_is_left_alone() {
    let m = dip_room();
    let fs = freqs();
    // Without a spread the fit does fill the -8 dB dip (by up to +3 dB).
    let plain = fit(
        &m,
        &flat_target(),
        &chain(),
        &PlainModel,
        &FitOptions::speakers(),
    )
    .unwrap();
    assert!(
        at(&fs, &change(&plain, &m), 140.0) > 1.5,
        "{:#?}",
        plain.bands
    );
    // With 5 dB of spread around it, the dip is left alone.
    let spread: Vec<f64> = fs
        .iter()
        .map(|&f| {
            if (f / 140.0).log2().abs() < 0.4 {
                5.0
            } else {
                0.5
            }
        })
        .collect();
    let opts = FitOptions {
        spread_db: Some(spread),
        ..FitOptions::speakers()
    };
    let r = fit(&m, &flat_target(), &chain(), &PlainModel, &opts).unwrap();
    let d = change(&r, &m);
    for (&f, &x) in fs.iter().zip(d.iter()) {
        if (f / 140.0).log2().abs() < 0.2 {
            assert!(
                (-1.0..=0.5).contains(&x),
                "{f} Hz changed by {x}: {:#?}",
                r.bands
            );
        }
    }
    // The peak at 50 Hz is still cut.
    assert!(at(&fs, &d, 50.0) < -4.0, "{:#?}", r.bands);
    assert!(
        r.notes.iter().any(|n| n.contains("left alone")),
        "{:#?}",
        r.notes
    );
}

#[test]
fn wp_p5fit_untrusted_points_are_not_fitted() {
    let m0 = room(75.0, &[(60.0, 4.0, 8.0)]);
    let fs = freqs();
    let valid: Vec<bool> = fs.iter().map(|&f| !(40.0..=90.0).contains(&f)).collect();
    let m = Response {
        valid: Some(valid),
        ..m0
    };
    let opts = FitOptions {
        smoothing: None,
        ..FitOptions::speakers()
    };
    let r = fit(&m, &flat_target(), &chain(), &PlainModel, &opts).unwrap();
    // Nothing trusted is off the target by more than the flanks of the mode
    // outside the hole; the fit must not chase the hole.
    for b in &r.bands {
        let f = f64::from(b.band.freq);
        assert!(
            !(45.0..=80.0).contains(&f) || b.band.gain.abs() < 3.0,
            "{b:?}"
        );
    }
}

#[test]
fn wp_p5fit_only_allowed_band_indices_are_used() {
    let mut c = chain();
    c.peq[0] = BandSpec {
        filter_type: 1,
        freq: 2000.0,
        q: 1.0,
        gain: -2.0,
        ..FLAT
    };
    c.peq[2] = BandSpec {
        filter_type: 2,
        freq: 100.0,
        q: 0.7,
        gain: 1.0,
        ..FLAT
    };
    c.peq[4] = BandSpec {
        filter_type: 1,
        freq: 500.0,
        q: 1.0,
        gain: 3.0,
        bypass: true,
        ..FLAT
    };
    let m = three_modes();
    let r = fit(&m, &flat_target(), &c, &PlainModel, &FitOptions::speakers()).unwrap();
    assert!(!r.bands.is_empty());
    for b in &r.bands {
        assert!(![0u8, 2].contains(&b.index), "used band {}", b.index);
    }
    // The bypassed band 4 is free; the lowest free indices come first.
    assert_eq!(r.bands[0].index, 1);

    let opts = FitOptions {
        bands: Some(vec![7, 5]),
        ..FitOptions::speakers()
    };
    let r = fit(&m, &flat_target(), &c, &PlainModel, &opts).unwrap();
    assert!(r.bands.len() <= 2);
    assert!(r.bands.iter().all(|b| b.index == 5 || b.index == 7));

    let opts = FitOptions {
        max_bands: Some(1),
        ..FitOptions::speakers()
    };
    let r = fit(&m, &flat_target(), &c, &PlainModel, &opts).unwrap();
    assert_eq!(r.bands.len(), 1);
}

#[test]
fn wp_p5fit_a_released_band_in_use_is_replaced() {
    // The user's +6 dB band at 80 Hz is in the measurement; released, the fit
    // takes it out and fits the room again.
    let mut c = chain();
    let user = BandSpec {
        filter_type: 1,
        freq: 80.0,
        q: 2.0,
        gain: 6.0,
        ..FLAT
    };
    c.peq[3] = user;
    let fs = freqs();
    let base = room(70.0, &[(45.0, 6.0, 6.0)]);
    let m = response(
        fs.iter()
            .zip(base.mag_db.iter())
            .map(|(&f, b)| b + dspi_proto::dsp::response_at(f, &[proto_band(&user)]))
            .collect(),
    );
    let opts = FitOptions {
        bands: Some(vec![3, 4]),
        ..FitOptions::speakers()
    };
    let r = fit(&m, &flat_target(), &c, &PlainModel, &opts).unwrap();
    assert!(
        r.notes.iter().any(|n| n.contains("PEQ band 4 is in use")),
        "{:#?}",
        r.notes
    );
    assert!(r.rms_after_db < r.rms_before_db);
    let d = change(&r, &m);
    assert!(
        at(&fs, &d, 80.0) < -3.0,
        "the user's boost is taken out: {:#?}",
        r.bands
    );
}

#[test]
fn wp_p5fit_no_change_when_already_on_target() {
    let m = response(vec![72.0; freqs().len()]);
    let r = fit(
        &m,
        &flat_target(),
        &chain(),
        &PlainModel,
        &FitOptions::speakers(),
    )
    .unwrap();
    assert!(r.bands.is_empty());
    assert_eq!(r.predicted, m);
    assert_eq!(r.gain_cut_db, 0.0);
    assert_eq!(r.rms_after_db, r.rms_before_db);
    assert!(r.notes.iter().any(|n| n.contains("no change")));
}

#[test]
fn wp_p5fit_no_free_band_is_no_change() {
    let mut c = chain();
    for b in &mut c.peq {
        *b = BandSpec {
            filter_type: 1,
            freq: 5000.0,
            q: 1.0,
            gain: -1.0,
            ..FLAT
        };
    }
    let r = fit(
        &three_modes(),
        &flat_target(),
        &c,
        &PlainModel,
        &FitOptions::speakers(),
    )
    .unwrap();
    assert!(r.bands.is_empty());
    assert!(r.notes.iter().any(|n| n.contains("no free PEQ band")));
}

#[test]
fn wp_p5fit_phase_is_predicted_when_measured() {
    let mut m = three_modes();
    m.phase_deg = Some(vec![0.0; freqs().len()]);
    let r = fit(
        &m,
        &flat_target(),
        &chain(),
        &PlainModel,
        &FitOptions::speakers(),
    )
    .unwrap();
    let p = r.predicted.phase_deg.as_ref().unwrap();
    assert!(p.iter().all(|x| x.is_finite()));
    assert!(p.iter().any(|x| x.abs() > 1.0), "a cut moves the phase");
    assert!(p.iter().all(|x| x.abs() < 180.0));
}

#[test]
fn wp_p5fit_target_on_another_grid_and_a_tilt() {
    let tg = LogGrid {
        min_hz: 10.0,
        max_hz: 20_000.0,
        points_per_octave: 12,
    };
    let tf = tg.freqs().unwrap();
    // A -1 dB/octave tilt from 20 Hz.
    let target = Response {
        grid: tg,
        mag_db: tf.iter().map(|f| -(f / 20.0).log2()).collect(),
        phase_deg: None,
        valid: None,
        spl: false,
    };
    let fs = freqs();
    let m = response(
        fs.iter()
            .map(|&f| 80.0 - (f / 20.0).log2() + mode_db(f, 60.0, 5.0, 8.0))
            .collect(),
    );
    let r = fit(&m, &target, &chain(), &PlainModel, &FitOptions::speakers()).unwrap();
    assert!(
        r.rms_after_db < 0.5 * r.rms_before_db,
        "{} -> {}",
        r.rms_before_db,
        r.rms_after_db
    );
}

#[test]
fn wp_p5fit_bad_input_is_a_typed_error() {
    let m = three_modes();
    let t = flat_target();
    let c = chain();
    let o = FitOptions::speakers();
    let short = response(vec![0.0; 10]);
    assert!(matches!(
        fit(&short, &t, &c, &PlainModel, &o),
        Err(MeasureError::Invalid(_))
    ));
    let mut nan = m.clone();
    nan.mag_db[100] = f64::NAN;
    assert!(matches!(
        fit(&nan, &t, &c, &PlainModel, &o),
        Err(MeasureError::Invalid(_))
    ));
    for bad in [
        FitOptions {
            max_boost_db: 13.0,
            ..o.clone()
        },
        FitOptions {
            max_boost_db: f64::NAN,
            ..o.clone()
        },
        FitOptions {
            min_hz: 300.0,
            max_hz: 20.0,
            ..o.clone()
        },
        FitOptions {
            q_min: 5.0,
            q_max: 1.0,
            ..o.clone()
        },
        FitOptions {
            smoothing: Some(0),
            ..o.clone()
        },
        FitOptions {
            bands: Some(vec![10]),
            ..o.clone()
        },
        FitOptions {
            max_evaluations: MAX_EVALUATIONS + 1,
            ..o.clone()
        },
        FitOptions {
            spread_db: Some(vec![1.0; 3]),
            ..o.clone()
        },
    ] {
        assert!(
            matches!(
                fit(&m, &t, &c, &PlainModel, &bad),
                Err(MeasureError::Invalid(_))
            ),
            "{bad:?}"
        );
    }
    // A released band in use whose type the model does not know.
    let mut lt = chain();
    lt.peq[0] = BandSpec {
        filter_type: 11,
        freq: 40.0,
        q: 0.7,
        gain: 20.0,
        qp: 0.5,
        bypass: false,
    };
    let rel = FitOptions {
        bands: Some(vec![0, 1]),
        ..o.clone()
    };
    assert!(matches!(
        fit(&m, &t, &lt, &PlainModel, &rel),
        Err(MeasureError::Invalid(_))
    ));
    // Nothing trusted in the range.
    let none = Response {
        valid: Some(vec![false; freqs().len()]),
        ..m.clone()
    };
    assert!(matches!(
        fit(&none, &t, &c, &PlainModel, &o),
        Err(MeasureError::NoSignal(_))
    ));
}

/// A headphone-like curve with a dozen features across 20 Hz to 10 kHz.
fn headphone() -> Response {
    room(
        90.0,
        &[
            (30.0, 0.7, -4.0),
            (95.0, 1.5, 3.0),
            (250.0, 2.0, -2.5),
            (600.0, 3.0, 3.5),
            (1100.0, 2.5, -3.0),
            (1800.0, 4.0, 4.0),
            (2900.0, 3.0, 6.0),
            (4300.0, 5.0, -5.0),
            (5500.0, 6.0, 5.0),
            (7000.0, 4.0, -4.0),
            (8500.0, 6.0, 7.0),
            (9500.0, 3.0, -3.0),
        ],
    )
}

#[test]
fn wp_p5fit_ten_bands_on_the_standard_grid_within_the_cap_and_time() {
    let m = headphone();
    let opts = FitOptions {
        target_rms_db: 0.0,
        ..FitOptions::headphones()
    };
    let t0 = std::time::Instant::now();
    let r = fit(&m, &flat_target(), &chain(), &PlainModel, &opts).unwrap();
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "wp_p5fit: 10 bands, {} evaluations, {ms:.1} ms, {:.2} -> {:.2} dB RMS",
        r.evaluations, r.rms_before_db, r.rms_after_db
    );
    assert!(r.evaluations <= DEFAULT_EVALUATIONS);
    assert!(r.bands.len() <= 10);
    assert!(r.bands.len() >= 6, "{:#?}", r.bands);
    assert!(r.rms_after_db < 0.5 * r.rms_before_db);
    if !cfg!(debug_assertions) {
        assert!(ms < 200.0, "{ms} ms in a release build");
    }
}

fn arb_room() -> impl Strategy<Value = Vec<(f64, f64, f64)>> {
    prop::collection::vec((15.0f64..400.0, 0.5f64..12.0, -18.0f64..12.0), 0..5)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Bounds hold whatever the room, the bands in use and the options.
    #[test]
    fn proptest_wp_p5fit_bounds_hold(
        modes in arb_room(),
        used in prop::collection::vec(any::<bool>(), 10),
        boost in 0.0f64..6.0,
        cut in 1.0f64..15.0,
        qmin in 0.3f64..2.0,
        qspan in 0.0f64..8.0,
        lo in 15.0f64..100.0,
        span in 0.5f64..4.0,
        shelves in any::<bool>(),
        smoothing in prop::option::of(1u16..48),
        spread_at in prop::option::of(20.0f64..300.0),
        evals in 0u32..600,
    ) {
        let m = room(70.0, &modes);
        let mut c = chain();
        for (b, u) in c.peq.iter_mut().zip(used.iter()) {
            if *u {
                *b = BandSpec { filter_type: 1, freq: 3000.0, q: 2.0, gain: -1.0, ..FLAT };
            }
        }
        let hi = lo * span.exp2();
        let spread = spread_at.map(|s| {
            freqs().iter().map(|&f| if (f / s).log2().abs() < 0.3 { 6.0 } else { 0.0 }).collect()
        });
        let opts = FitOptions {
            min_hz: lo,
            max_hz: hi,
            smoothing,
            max_boost_db: boost,
            max_cut_db: cut,
            q_min: qmin,
            q_max: qmin + qspan,
            shelves,
            spread_db: spread,
            max_evaluations: evals,
            ..FitOptions::speakers()
        };
        let r = fit(&m, &flat_target(), &c, &PlainModel, &opts).unwrap();
        prop_assert!(r.evaluations <= evals);
        prop_assert!(r.rms_after_db <= r.rms_before_db);
        prop_assert!(r.rms_after_db.is_finite() && r.max_error_after_db.is_finite());
        prop_assert!(r.gain_cut_db <= 0.0 && r.gain_cut_db.is_finite());
        prop_assert!(r.predicted.mag_db.iter().all(|x| x.is_finite()));
        let free = used.iter().filter(|u| !**u).count();
        prop_assert!(r.bands.len() <= free);
        let mut seen = Vec::new();
        for b in &r.bands {
            prop_assert!(!used[usize::from(b.index)], "band {} is in use", b.index);
            prop_assert!(!seen.contains(&b.index));
            seen.push(b.index);
            let s = b.band;
            prop_assert!(matches!(s.filter_type, 1..=3) && !s.bypass);
            prop_assert!(shelves || s.filter_type == 1);
            let f = f64::from(s.freq);
            prop_assert!(f >= lo - 1e-3 && f <= hi + 1e-3, "{f} outside {lo}..{hi}");
            let q = f64::from(s.q);
            prop_assert!(q >= qmin - 1e-3 && q <= qmin + qspan + 1e-3, "Q {q}");
            let g = f64::from(s.gain);
            prop_assert!(g >= -cut - 1e-4 && g <= boost + 1e-4, "gain {g}");
            prop_assert!(g.abs() >= 0.5 - 1e-4);
            prop_assert!(((g * 10.0) - (g * 10.0).round()).abs() < 1e-3);
        }
        for f in freqs() {
            prop_assert!(bands_db(&r, f) <= boost + 1e-5, "sum boost at {f}");
        }
        if r.bands.is_empty() {
            prop_assert_eq!(&r.predicted, &m);
        }
    }

    /// Garbage in: an error or a finite result, never a panic.
    #[test]
    fn proptest_wp_p5fit_never_panics(
        mag in prop::collection::vec(prop::num::f64::ANY, 0..1100),
        tmag in prop::collection::vec(-40.0f64..40.0, 0..300),
        ppo in 0u16..120,
        lo in prop::num::f64::ANY,
        hi in prop::num::f64::ANY,
        boost in prop::num::f64::ANY,
        qmin in prop::num::f64::ANY,
        bands in prop::option::of(prop::collection::vec(any::<u8>(), 0..12)),
        evals in 0u32..300,
    ) {
        let grid = LogGrid { min_hz: 10.0, max_hz: 22_000.0, points_per_octave: ppo };
        let n = grid.freqs().map(|f| f.len()).unwrap_or(0);
        let m = Response { grid, mag_db: mag.iter().take(n).copied().collect(), phase_deg: None, valid: None, spl: false };
        let tg = LogGrid { min_hz: 10.0, max_hz: 22_000.0, points_per_octave: 24 };
        let tn = tg.freqs().unwrap().len();
        let mut tm = tmag.clone();
        tm.resize(tn, 0.0);
        let t = Response { grid: tg, mag_db: tm, phase_deg: None, valid: None, spl: false };
        let opts = FitOptions {
            min_hz: lo,
            max_hz: hi,
            max_boost_db: boost,
            q_min: qmin,
            bands,
            max_evaluations: evals,
            ..FitOptions::speakers()
        };
        if let Ok(r) = fit(&m, &t, &chain(), &PlainModel, &opts) {
            prop_assert!(r.rms_after_db <= r.rms_before_db);
            prop_assert!(r.predicted.mag_db.iter().all(|x| x.is_finite()));
            prop_assert!(r.bands.iter().all(|b| b.band.freq.is_finite() && b.band.q.is_finite() && b.band.gain.is_finite()));
        }
    }

    /// Identical input, identical result (RC10), over random rooms.
    #[test]
    fn proptest_wp_p5fit_deterministic(modes in arb_room(), evals in 0u32..400) {
        let m = room(70.0, &modes);
        let o = FitOptions { max_evaluations: evals, ..FitOptions::speakers() };
        let a = fit(&m, &flat_target(), &chain(), &PlainModel, &o).unwrap();
        let b = fit(&m, &flat_target(), &chain(), &PlainModel, &o).unwrap();
        prop_assert_eq!(a, b);
    }
}

// -- WP P5TRUST -- the level option (review P5-C2, follow-up 3).

/// A room 4 dB too loud below about 150 Hz (a broad excess, no mode): the
/// free level does not see it, the level anchored in the reference band
/// does, and cuts it.
#[test]
fn wp_p5trust_an_anchored_level_sees_a_broad_bass_excess() {
    let m = response(
        freqs()
            .iter()
            .map(|&f| 75.0 + 4.0 / (1.0 + (f / 150.0).powi(4)))
            .collect(),
    );
    let t = flat_target();
    let fs = freqs();
    let free = fit(&m, &t, &chain(), &PlainModel, &FitOptions::speakers()).unwrap();
    let off = reference_offset_db(&m, &t, 500.0, 2000.0).unwrap();
    assert!((off - 75.0).abs() < 0.1, "{off}");
    let opts = FitOptions {
        level: FitLevel::Anchored { offset_db: off },
        leave_out_dips: true,
        ..FitOptions::speakers()
    };
    let anchored = fit(&m, &t, &chain(), &PlainModel, &opts).unwrap();
    let (c_free, c_anch) = (change(&free, &m), change(&anchored, &m));
    println!("free {:#?}\nanchored {:#?}", free.bands, anchored.bands);
    assert!(at(&fs, &c_free, 50.0) > -1.5, "{}", at(&fs, &c_free, 50.0));
    assert!(at(&fs, &c_anch, 50.0) < -3.0, "{}", at(&fs, &c_anch, 50.0));
    assert!(anchored.rms_after_db < 1.0, "{}", anchored.rms_after_db);
    // Anchored, the error is the error: before, it is the excess itself.
    assert!(anchored.rms_before_db > free.rms_before_db);
    assert!((anchored.target_offset_db - off).abs() < 1e-9);
    // The boost cap still holds everywhere.
    for f in &fs {
        assert!(bands_db(&anchored, *f) <= opts.max_boost_db + 1e-6);
    }
}

/// A 12 dB dip no band may fill: left out of the cost and the level, so it
/// is not boosted and does not pull the level down; the mode beside it is
/// still cut.
#[test]
fn wp_p5trust_a_dip_past_the_boost_cap_is_left_out_of_the_fit_and_its_level() {
    let m = room(75.0, &[(45.0, 5.0, 6.0), (110.0, 1.2, -12.0)]);
    let t = flat_target();
    let fs = freqs();
    let with = FitOptions {
        leave_out_dips: true,
        ..FitOptions::speakers()
    };
    let plain = fit(&m, &t, &chain(), &PlainModel, &FitOptions::speakers()).unwrap();
    let left = fit(&m, &t, &chain(), &PlainModel, &with).unwrap();
    println!("plain {:#?}\nleft {:#?}", plain.bands, left.bands);
    assert!(
        left.notes.iter().any(|n| n.contains("left out of the fit")),
        "{:#?}",
        left.notes
    );
    let c = change(&left, &m);
    assert!(at(&fs, &c, 110.0) <= 0.5, "{}", at(&fs, &c, 110.0));
    assert!(at(&fs, &c, 45.0) < -3.0, "{}", at(&fs, &c, 45.0));
    // The level the fit settles on is the room's, not pulled into the dip.
    assert!(
        (left.target_offset_db - 75.0).abs() < (plain.target_offset_db - 75.0).abs(),
        "{} {}",
        left.target_offset_db,
        plain.target_offset_db
    );
    // Anchored at the room's level, the same.
    let anch = FitOptions {
        level: FitLevel::Anchored { offset_db: 75.0 },
        ..with
    };
    let a = fit(&m, &t, &chain(), &PlainModel, &anch).unwrap();
    let c = change(&a, &m);
    assert!(at(&fs, &c, 110.0) <= 0.5, "{}", at(&fs, &c, 110.0));
    assert!(at(&fs, &c, 45.0) < -3.0, "{}", at(&fs, &c, 45.0));
}

#[test]
fn wp_p5trust_a_bad_anchor_is_refused_and_no_reference_gives_none() {
    let m = three_modes();
    for off in [f64::NAN, f64::INFINITY, 201.0] {
        let o = FitOptions {
            level: FitLevel::Anchored { offset_db: off },
            ..FitOptions::speakers()
        };
        assert!(fit(&m, &flat_target(), &chain(), &PlainModel, &o).is_err());
    }
    let mut untrusted = m.clone();
    untrusted.valid = Some(vec![false; m.mag_db.len()]);
    assert!(reference_offset_db(&untrusted, &flat_target(), 500.0, 2000.0).is_none());
    assert!(reference_offset_db(&m, &flat_target(), 3.0, 4.0).is_none());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    /// Anchored or free, dips left out or not: never a panic, never worse.
    #[test]
    fn proptest_wp_p5trust_the_level_options_never_panic_or_worsen(
        modes in arb_room(),
        off in -30.0f64..30.0,
        anchored in any::<bool>(),
        leave in any::<bool>(),
    ) {
        let m = room(70.0, &modes);
        let o = FitOptions {
            level: if anchored { FitLevel::Anchored { offset_db: 70.0 + off } } else { FitLevel::Free },
            leave_out_dips: leave,
            max_evaluations: 300,
            ..FitOptions::speakers()
        };
        if let Ok(r) = fit(&m, &flat_target(), &chain(), &PlainModel, &o) {
            prop_assert!(r.rms_after_db <= r.rms_before_db);
            prop_assert!(r.predicted.mag_db.iter().all(|x| x.is_finite()));
            let again = fit(&m, &flat_target(), &chain(), &PlainModel, &o).unwrap();
            prop_assert_eq!(r, again);
        }
    }
}
