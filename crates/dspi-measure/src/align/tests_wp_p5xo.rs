//! Tests of WP P5XO's alignment, and the synthetic world the crossover tests
//! share: a plain [`SectionModel`] over `dspi_proto`, drivers made of pure
//! delays and peaking resonances, and "measurements" that are a driver times
//! its chain, exactly as the walked run sees it.

use super::*;
use crate::types::{BandSpec, Coeffs, LogGrid, ProgrammeFeed, SectionModel};
use crate::xover::{cascade_at, sections};
use proptest::prelude::*;

/// A section model over `dspi_proto::dsp` and `dspi_proto::xover` at 48 kHz:
/// what the core's cascade computes, without the core.
pub(crate) struct ProtoModel;

impl SectionModel for ProtoModel {
    fn sections(&self, b: &BandSpec) -> Vec<Coeffs> {
        if b.bypass || b.filter_type == 0 || !self.models(b) {
            return Vec::new();
        }
        if (32..=63).contains(&b.filter_type) {
            return dspi_proto::xover::sections(b.filter_type, b.freq);
        }
        vec![dspi_proto::dsp::coefficients(&dspi_proto::dsp::Band {
            filter_type: dspi_proto::FilterType::from_raw(b.filter_type),
            freq: b.freq,
            q: b.q,
            gain_db: b.gain,
            bypass: false,
        })]
    }
    fn models(&self, b: &BandSpec) -> bool {
        (b.filter_type <= 13 && b.filter_type != 11) || (32..=63).contains(&b.filter_type)
    }
    fn sample_rate(&self) -> f64 {
        48_000.0
    }
}

pub(crate) const LR4_LP: u8 = 34;
pub(crate) const LR4_HP: u8 = 35;

pub(crate) fn band(filter_type: u8, freq: f64) -> BandSpec {
    BandSpec {
        filter_type,
        bypass: false,
        freq: freq as f32,
        q: 0.707,
        gain: 0.0,
        qp: 0.0,
    }
}

pub(crate) fn peak(freq: f64, q: f32, gain: f32) -> BandSpec {
    BandSpec {
        filter_type: 1,
        bypass: false,
        freq: freq as f32,
        q,
        gain,
        qp: 0.0,
    }
}

pub(crate) fn chain(crossover: Vec<BandSpec>) -> ChannelChain {
    ChannelChain {
        peq: Vec::new(),
        crossover,
        gain_db: 0.0,
        delay_ms: 0.0,
        feed: ProgrammeFeed {
            gain_db: 0.0,
            inverted: false,
        },
    }
}

/// A synthetic driver: polarity, an acoustic delay (with the run's common
/// latency) and resonances.
#[derive(Clone)]
pub(crate) struct Drv {
    pub(crate) arrival_ms: f64,
    pub(crate) inverted: bool,
    pub(crate) shape: Vec<BandSpec>,
    pub(crate) level_db: f64,
}

impl Drv {
    pub(crate) fn flat(arrival_ms: f64) -> Drv {
        Drv {
            arrival_ms,
            inverted: false,
            shape: Vec::new(),
            level_db: 0.0,
        }
    }
    pub(crate) fn at(&self, f: f64) -> Cx {
        let secs = sections(&self.shape, &ProtoModel).unwrap();
        let s = cascade_at(&secs, f, 48_000.0).unwrap();
        let sign = if self.inverted { -1.0 } else { 1.0 };
        s.scale(sign * db_to_lin(self.level_db))
            .mul(Cx::delay(f, self.arrival_ms))
    }
}

/// The chain's complex response, delay and gain included.
pub(crate) fn chain_at(c: &ChannelChain, f: f64) -> Cx {
    let p = sections(&c.peq, &ProtoModel).unwrap();
    let x = sections(&c.crossover, &ProtoModel).unwrap();
    cascade_at(&p, f, 48_000.0)
        .unwrap()
        .mul(cascade_at(&x, f, 48_000.0).unwrap())
        .scale(db_to_lin(f64::from(c.gain_db)))
        .mul(Cx::delay(f, f64::from(c.delay_ms)))
}

pub(crate) fn to_response(grid: LogGrid, h: &[Cx]) -> Response {
    Response {
        grid,
        mag_db: h.iter().map(|x| x.db()).collect(),
        phase_deg: Some(h.iter().map(|x| x.arg().to_degrees()).collect()),
        valid: Some(vec![true; h.len()]),
        spl: false,
    }
}

/// What a walked run measures for `d` through `c`: no noise in the response,
/// an SNR of `snr_db` where the chain passes and, as a measurement has it,
/// less by the chain's attenuation where it does not (review P5-C2: the SNR
/// per point, which the de-embedding's mask reads as it is).
pub(crate) fn way(output: u8, role: WayRole, c: ChannelChain, d: &Drv, snr_db: f64) -> Way {
    let grid = LogGrid::STANDARD;
    let f = grid.freqs().unwrap();
    let h: Vec<Cx> = f.iter().map(|f| d.at(*f).mul(chain_at(&c, *f))).collect();
    let p = sections(&c.peq, &ProtoModel).unwrap();
    let x = sections(&c.crossover, &ProtoModel).unwrap();
    let snr: Vec<f64> = f
        .iter()
        .map(|f| {
            let shape = cascade_at(&p, *f, 48_000.0)
                .unwrap()
                .mul(cascade_at(&x, *f, 48_000.0).unwrap());
            snr_db - (-shape.db()).max(0.0)
        })
        .collect();
    // -- WP P5HOLD -- a measured arrival is the response's onset: the
    // driver's, the chain's delay and its filters' own onset (review P5-D
    // D1; the hold takes the last off again).
    let onset = crate::xover::chain_onset_ms(&c, &ProtoModel).unwrap_or(0.0);
    Way {
        output,
        role,
        arrival_ms: d.arrival_ms + f64::from(c.delay_ms) + 0.05 + onset,
        chain: c,
        response: to_response(grid, &h),
        snr_db: snr,
        timing_reliable: true,
    }
}

/// A two-way: woofer 0.35 ms later than the tweeter, LR4 at 2 kHz.
fn two_way(delays: (f32, f32)) -> Vec<Way> {
    let mut cw = chain(vec![band(LR4_LP, 2000.0)]);
    let mut ct = chain(vec![band(LR4_HP, 2000.0)]);
    cw.delay_ms = delays.0;
    ct.delay_ms = delays.1;
    vec![
        way(0, WayRole::Low, cw, &Drv::flat(50.85), 60.0),
        way(1, WayRole::High, ct, &Drv::flat(50.5), 60.0),
    ]
}

fn pair_group() -> AlignGroup {
    AlignGroup {
        transitions: vec![Transition {
            low: 0,
            high: 1,
            corner_hz: None,
        }],
    }
}

#[test]
fn wp_p5xo_a_0_35_ms_offset_is_recovered_within_10_us() {
    let r = align(
        &two_way((0.0, 0.0)),
        &pair_group(),
        &AlignOptions::default(),
        &ProtoModel,
    )
    .unwrap();
    let (w, t) = (r.ways[0], r.ways[1]);
    assert_eq!(w.delay_ms, 0.0, "the earliest way keeps its delay: {r:?}");
    assert!(
        (f64::from(t.delay_ms) - 0.35).abs() <= 0.010,
        "tweeter delay {} ms",
        t.delay_ms
    );
    assert!(!w.inverted && !t.inverted);
    assert!(r.pairs[0].confidence > 0.5, "{:?}", r.pairs[0]);
    assert!(r.pairs[0].cost_db < 0.05, "{:?}", r.pairs[0]);
}

#[test]
fn wp_p5xo_an_existing_common_delay_of_3_ms_is_kept_as_the_smallest() {
    let r = align(
        &two_way((3.0, 3.0)),
        &pair_group(),
        &AlignOptions::default(),
        &ProtoModel,
    )
    .unwrap();
    let smallest = r.ways.iter().map(|w| w.delay_ms).fold(f32::MAX, f32::min);
    assert!((smallest - 3.0).abs() < 1e-6, "{r:?}");
    assert!(
        (f64::from(r.ways[1].delay_ms) - 3.35).abs() <= 0.010,
        "{r:?}"
    );
    // The smallest delay held by the other way now: it is the one kept.
    let r = align(
        &two_way((3.0, 3.5)),
        &pair_group(),
        &AlignOptions::default(),
        &ProtoModel,
    )
    .unwrap();
    assert!((r.ways[0].delay_ms - 3.0).abs() < 1e-6, "{r:?}");
    assert!(
        (f64::from(r.ways[1].delay_ms) - 3.35).abs() <= 0.010,
        "{r:?}"
    );
}

#[test]
fn wp_p5xo_timing_unreliable_gets_no_alignment() {
    let mut ways = two_way((0.0, 0.0));
    ways[1].timing_reliable = false;
    let e = align(&ways, &pair_group(), &AlignOptions::default(), &ProtoModel).unwrap_err();
    assert!(
        matches!(&e, MeasureError::Invalid(m) if m.starts_with("timing unreliable")),
        "{e:?}"
    );
}

#[test]
fn wp_p5xo_an_inverted_tweeter_is_flipped_through_the_feed() {
    let mut cm = chain(vec![band(LR4_HP, 300.0), band(LR4_LP, 3000.0)]);
    cm.delay_ms = 0.0;
    let mut tw = Drv::flat(50.2);
    tw.inverted = true;
    let ways = vec![
        way(
            0,
            WayRole::Low,
            chain(vec![band(LR4_LP, 300.0)]),
            &Drv::flat(50.6),
            60.0,
        ),
        way(1, WayRole::Mid, cm, &Drv::flat(50.3), 60.0),
        way(
            2,
            WayRole::High,
            chain(vec![band(LR4_HP, 3000.0)]),
            &tw,
            60.0,
        ),
    ];
    let group = AlignGroup {
        transitions: vec![
            Transition {
                low: 0,
                high: 1,
                corner_hz: None,
            },
            Transition {
                low: 1,
                high: 2,
                corner_hz: None,
            },
        ],
    };
    let r = align(&ways, &group, &AlignOptions::default(), &ProtoModel).unwrap();
    assert_eq!(
        r.ways.iter().map(|w| w.inverted).collect::<Vec<_>>(),
        vec![false, false, true],
        "{r:?}"
    );
    // Delays: the latest way (the woofer) keeps 0; the others are delayed
    // by their arrival difference less the group delay of the mid's own
    // 3 kHz low-pass, which the flattest sum takes into account (so the
    // sums through both transitions come out flat).
    assert_eq!(r.ways[0].delay_ms, 0.0);
    assert!(
        r.ways[1].delay_ms > 0.1 && r.ways[1].delay_ms < 0.3,
        "{r:?}"
    );
    assert!(r.ways[2].delay_ms > r.ways[1].delay_ms, "{r:?}");
    assert!(r.pairs.iter().all(|p| p.cost_db < 0.05), "{r:?}");
}

#[test]
fn wp_p5xo_the_programme_feed_polarity_is_part_of_the_sum() {
    // The woofer's crosspoint is inverted today, the drivers are not: the
    // programme sum cancels, so the tweeter's feed must be inverted too (the
    // root keeps its polarity).
    let mut ways = two_way((0.0, 0.0));
    ways[0].chain.feed.inverted = true;
    let r = align(&ways, &pair_group(), &AlignOptions::default(), &ProtoModel).unwrap();
    assert!(r.ways[0].inverted && r.ways[1].inverted, "{r:?}");
    assert!(r.pairs[0].flipped);
}

#[test]
fn wp_p5xo_keep_delays_judges_polarity_only() {
    let mut ways = two_way((0.0, 0.35));
    ways[1].chain.feed.inverted = true;
    let opts = AlignOptions {
        keep_delays: true,
        ..AlignOptions::default()
    };
    let r = align(&ways, &pair_group(), &opts, &ProtoModel).unwrap();
    assert_eq!((r.ways[0].delay_ms, r.ways[1].delay_ms), (0.0, 0.35));
    assert!(!r.ways[1].inverted, "{r:?}");
}

#[test]
fn wp_p5xo_a_delay_outside_the_range_is_refused() {
    let opts = AlignOptions {
        max_delay_ms: 0.2,
        ..AlignOptions::default()
    };
    let e = align(&two_way((0.0, 0.0)), &pair_group(), &opts, &ProtoModel).unwrap_err();
    assert!(matches!(e, MeasureError::Invalid(m) if m.contains("outside")));
}

#[test]
fn wp_p5xo_too_few_trusted_points_is_no_signal() {
    let mut ways = two_way((0.0, 0.0));
    ways[0].response.valid = Some(vec![false; ways[0].response.mag_db.len()]);
    assert!(matches!(
        align(&ways, &pair_group(), &AlignOptions::default(), &ProtoModel),
        Err(MeasureError::NoSignal(_))
    ));
}

#[test]
fn wp_p5xo_malformed_input_is_refused() {
    let ways = two_way((0.0, 0.0));
    let bad_t = AlignGroup {
        transitions: vec![Transition {
            low: 0,
            high: 5,
            corner_hz: None,
        }],
    };
    assert!(align(&ways, &bad_t, &AlignOptions::default(), &ProtoModel).is_err());
    assert!(align(&[], &pair_group(), &AlignOptions::default(), &ProtoModel).is_err());
    let mut no_phase = ways.clone();
    no_phase[0].response.phase_deg = None;
    assert!(
        align(
            &no_phase,
            &pair_group(),
            &AlignOptions::default(),
            &ProtoModel
        )
        .is_err()
    );
    let mut nan = ways.clone();
    nan[0].chain.delay_ms = f32::NAN;
    assert!(align(&nan, &pair_group(), &AlignOptions::default(), &ProtoModel).is_err());
}

#[test]
fn wp_p5xo_alignment_is_deterministic() {
    let a = align(
        &two_way((0.5, 0.0)),
        &pair_group(),
        &AlignOptions::default(),
        &ProtoModel,
    )
    .unwrap();
    let b = align(
        &two_way((0.5, 0.0)),
        &pair_group(),
        &AlignOptions::default(),
        &ProtoModel,
    )
    .unwrap();
    assert_eq!(a, b);
}

/// Small random ways on a coarse grid for the property tests.
pub(crate) fn arb_ways() -> impl Strategy<Value = (Vec<Way>, Vec<Transition>)> {
    let grid = LogGrid {
        min_hz: 20.0,
        max_hz: 20_000.0,
        points_per_octave: 6,
    };
    let n = grid.freqs().unwrap().len();
    let way = (
        prop::collection::vec(prop_oneof![Just(f64::NAN), -300.0..300.0f64], n),
        prop::collection::vec(prop_oneof![Just(f64::INFINITY), -1e6..1e6f64], n),
        prop::collection::vec(-10.0..80.0f64, n),
        (0u8..70, 0.0f32..30_000.0, -60.0f32..60.0, -5.0f32..50.0),
        any::<bool>(),
        -100.0..100.0f64,
    )
        .prop_map(move |(mag, ph, snr, (t, f, g, d), inv, arr)| {
            let mut c = chain(vec![BandSpec {
                filter_type: t,
                bypass: false,
                freq: f,
                q: 0.7,
                gain: g,
                qp: 0.0,
            }]);
            c.delay_ms = d;
            c.gain_db = g;
            c.feed.inverted = inv;
            Way {
                output: 0,
                role: WayRole::Mid,
                chain: c,
                response: Response {
                    grid,
                    mag_db: mag,
                    phase_deg: Some(ph),
                    valid: None,
                    spl: false,
                },
                snr_db: snr,
                arrival_ms: arr,
                timing_reliable: true,
            }
        });
    (
        prop::collection::vec(way, 1..4),
        prop::collection::vec(
            (0usize..4, 0usize..4, prop::option::of(1.0..30_000.0f64)),
            0..4,
        ),
    )
        .prop_map(|(mut ways, ts)| {
            for (i, w) in ways.iter_mut().enumerate() {
                w.output = i as u8;
            }
            let ts = ts
                .into_iter()
                .map(|(low, high, corner_hz)| Transition {
                    low,
                    high,
                    corner_hz,
                })
                .collect();
            (ways, ts)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn proptest_wp_p5xo_align_never_panics_and_is_finite(
        (ways, ts) in arb_ways(),
        max in 0.0..50.0f64,
        keep in any::<bool>(),
    ) {
        let opts = AlignOptions { min_delay_ms: 0.0, max_delay_ms: max, keep_delays: keep };
        if let Ok(r) = align(&ways, &AlignGroup { transitions: ts }, &opts, &ProtoModel) {
            prop_assert_eq!(r.ways.len(), ways.len());
            for w in &r.ways {
                prop_assert!(w.delay_ms.is_finite());
                prop_assert!(f64::from(w.delay_ms) <= max + 1e-6 && w.delay_ms >= 0.0);
            }
            for p in &r.pairs {
                prop_assert!(p.relative_delay_ms.is_finite() && p.cost_db.is_finite());
                prop_assert!((0.0..=1.0).contains(&p.confidence));
            }
        }
    }
}

// -- WP P5TRUST2 -- alignment alone holds its timing near the arrivals, as
// the crossover search's candidates do (review P5TRUST N2).
#[test]
fn wp_p5trust2_alignment_alone_never_takes_a_timing_far_from_the_arrivals() {
    let corner = 238.0;
    let ways = || {
        vec![
            way(
                0,
                WayRole::Low,
                chain(vec![band(LR4_LP, corner)]),
                &Drv::flat(50.85),
                60.0,
            ),
            way(
                1,
                WayRole::High,
                chain(vec![band(LR4_HP, corner)]),
                &Drv::flat(50.5),
                60.0,
            ),
        ]
    };
    // As measured: the timing is the arrivals' and nothing is held.
    let r = align(
        &ways(),
        &pair_group(),
        &AlignOptions::default(),
        &ProtoModel,
    )
    .unwrap();
    assert!(
        !r.notes.iter().any(|n| n.contains("measured arrivals")),
        "{r:?}"
    );
    assert!((r.pairs[0].relative_delay_ms - 0.35).abs() < 0.01, "{r:?}");
    // The arrivals put the tweeter 1.8 ms later than its response does
    // (0.43 of the corner's period): the responses' best is outside the
    // hold. Review P5-D: the best inside the hold is then the half-cycle-off
    // one with the polarity flipped (the tweeter is wired right), which is
    // no timing to propose: the delay and the polarity are kept, and said.
    let mut w = ways();
    w[1].arrival_ms -= 1.8;
    let r = align(&w, &pair_group(), &AlignOptions::default(), &ProtoModel).unwrap();
    assert!(!r.pairs[0].flipped, "{r:?}");
    assert_eq!(r.pairs[0].relative_delay_ms, 0.0, "{r:?}");
    assert!(
        r.ways.iter().all(|w| !w.inverted && w.delay_ms == 0.0),
        "{r:?}"
    );
    assert!(
        r.notes
            .iter()
            .any(|n| n.contains("measured arrivals") && n.contains("are kept")),
        "{r:?}"
    );
}

// Review P5-D: what the hold leaves is proposed only when it keeps the
// polarity and is not the hold's edge.
#[test]
fn review_p5d_a_held_timing_never_flips_and_is_never_the_holds_edge() {
    let corner = 238.0;
    let period = 1000.0 / corner;
    let hold = crate::xover::timing_hold_ms(corner);
    let half = half_window_ms(corner);
    // Two ways in phase at +0.3 ms: the free search finds it; arrivals 0.4 of
    // a period off put it outside the hold, where the flipped half-cycle-off
    // alignment is the best inside.
    let freqs: Vec<f64> = (0..97)
        .map(|i| corner * (f64::from(i) / 48.0 - 1.0).exp2())
        .collect();
    let ctx = |delay_ms: f64| PairCtx {
        points: freqs
            .iter()
            .map(|&f| {
                let w_ms = std::f64::consts::TAU * f / 1000.0;
                Point {
                    w_ms,
                    x: f.log2(),
                    a: Cx::polar(1.0, 0.0),
                    b: Cx::polar(1.0, w_ms * delay_ms),
                    r: 0.0,
                }
            })
            .collect(),
        kind: CostKind::Phase,
    };
    let c = ctx(0.3);
    let free = c.search(0.3 + 0.4 * period, half, 48, 24);
    assert!(!free.flip && (free.delta_ms - 0.3).abs() < 0.02, "{free:?}");
    let held = c.search_held(0.3 + 0.4 * period, half, 48, 24, hold);
    // What is left inside is the flipped, half-cycle-off alignment or the
    // hold's edge: unclear either way.
    assert!(held.held && held.held_unclear, "{held:?}");
    assert!(
        held.flip
            || ((held.delta_ms - (0.3 + 0.4 * period)).abs() - hold).abs() < half / 48.0 + 1e-9,
        "{held:?}"
    );
    // The pair as it is, judged.
    let kept = c.kept(0.0, held.evals);
    assert!(kept.held && kept.held_unclear && !kept.flip, "{kept:?}");
    assert_eq!(kept.delta_ms, 0.0);
    assert_eq!(kept.evals, held.evals + 2);
    // A held choice that keeps the polarity inside the hold is proposed.
    let near = c.search_held(0.3 + 0.2 * period, half, 48, 24, hold);
    assert!(!near.held && !near.held_unclear && !near.flip, "{near:?}");
    assert!((near.delta_ms - 0.3).abs() < 0.02, "{near:?}");
}
