//! WP WEB6: the effect drawings against Qt's numbers: its smoke checks
//! (`SmokeEffects.cpp` qp7, `SmokeTools.cpp` qp8) and exact vectors that its
//! smoke checks pin too ("web6:" lines).

use super::*;

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

const DEFAULT_TUBE: TubeShape = TubeShape {
    drive: -12.0,
    bias: 10.0,
    asym: 3.0,
    hardness: 40.0,
    mix: 100.0,
    trim: 0.0,
};

fn tube(drive: f64, bias: f64, asym: f64, hardness: f64, mix: f64, trim: f64) -> TubeShape {
    TubeShape {
        drive,
        bias,
        asym,
        hardness,
        mix,
        trim,
    }
}

#[test]
fn wp_web6_crossfeed_paths_as_qt() {
    // qp7: G below the cutoff on the opposite path, 1 - G on the direct one
    let g = 1.0 / (1.0 + 10f64.powf(4.5 / 20.0));
    assert!(near(
        crossfeed_opposite_db(700.0, 4.5, 20.0),
        20.0 * g.log10(),
        0.05
    ));
    assert!(near(
        crossfeed_direct_db(700.0, 4.5, 20.0),
        20.0 * (1.0 - g).log10(),
        0.05
    ));
    assert!(
        crossfeed_opposite_db(700.0, 4.5, 10_000.0)
            < crossfeed_opposite_db(700.0, 4.5, 100.0) - 10.0
    );
    assert!(near(crossfeed_direct_db(700.0, 4.5, 20_000.0), 0.0, 0.5));
    // The shared vectors
    assert!(near(
        crossfeed_opposite_db(700.0, 4.5, 100.0),
        -8.646496,
        1e-5
    ));
    assert!(near(
        crossfeed_opposite_db(700.0, 4.5, 5000.0),
        -25.565034,
        1e-5
    ));
    assert!(near(
        crossfeed_direct_db(700.0, 4.5, 100.0),
        -3.933661,
        1e-5
    ));
    assert!(near(
        crossfeed_direct_db(700.0, 4.5, 5000.0),
        -0.196024,
        1e-5
    ));
    assert!(near(
        crossfeed_direct_db(650.0, 9.5, 1000.0),
        -0.679913,
        1e-5
    ));
    assert!(near(
        crossfeed_opposite_db(650.0, 9.5, 1000.0),
        -17.275456,
        1e-5
    ));
}

#[test]
fn wp_web6_crossfeed_graph_shape() {
    let g = crossfeed_graph(700.0, 4.5);
    assert_eq!(g.curves.len(), 2);
    assert_eq!(g.curves[0].name, "direct");
    assert_eq!(g.curves[1].name, "crossfeed");
    for c in &g.curves {
        assert_eq!(c.points.len(), 100);
        assert!(
            c.points
                .iter()
                .all(|p| (0.0..=1.0).contains(&p[0]) && (0.0..=1.0).contains(&p[1]))
        );
    }
    assert!(near(g.curves[0].points[0][0], 0.0, 1e-12));
    assert!(near(g.curves[0].points[99][0], 1.0, 1e-12));
    // The grid: 5 or 10 dB steps, each drawn inside the plot
    assert!(g.grid_db.len() >= 2);
    let step = (g.grid_db[1] - g.grid_db[0]).abs();
    assert!(step == 5 || step == 10);
    assert_eq!(g.grid_db.len(), g.grid_y.len());
    assert!(g.grid_y.iter().all(|y| (0.0..=1.0).contains(y)));
}

#[test]
fn wp_web6_loudness_as_qt() {
    assert_eq!(iso_rows(), 30);
    assert_eq!(iso_freq(0), 20.0);
    assert_eq!(iso_freq(29), 16_000.0);
    assert_eq!(iso_freq(17), 1000.0);
    assert_eq!(iso_freq(30), 0.0);
    // qp7: 1 kHz alone, 20 Hz lifted, linear in the intensity
    assert!(loudness_compensation_db(83.0, 100.0, 17).abs() < 0.5);
    let bass = loudness_compensation_db(83.0, 100.0, 0);
    assert!(bass > 10.0 && bass < 40.0);
    assert!(near(
        loudness_compensation_db(83.0, 200.0, 0),
        2.0 * bass,
        1e-6
    ));
    assert!(near(loudness_compensation_db(83.0, 0.0, 0), 0.0, 1e-12));
    // The shared vectors (Qt holds the table in single precision, as here)
    assert!(near(bass, 20.918851, 1e-3));
    assert!(near(
        loudness_compensation_db(83.0, 100.0, 7),
        12.033087,
        1e-3
    ));
    assert!(near(
        loudness_compensation_db(70.0, 50.0, 7),
        5.582571,
        1e-3
    ));
    let g = loudness_graph(83.0, 100.0);
    assert_eq!(g.curves.len(), 1);
    assert_eq!(g.curves[0].points.len(), 30);
    assert!(
        g.curves[0]
            .points
            .iter()
            .all(|p| (0.0..=1.0).contains(&p[1]))
    );
    let step = (g.grid_db[1] - g.grid_db[0]).abs();
    assert!(step == 5 || step == 10);
}

#[test]
fn wp_web6_psybass_sketch_and_axis() {
    let s = psybass_sketch(80.0, 12.0, -60.0);
    assert!(near(s.harmonics, 1.0, 1e-12));
    assert!(near(s.original, 0.0, 1e-12));
    assert!(s.fc4 > s.fc);
    assert!(near(freq_x(20.0), 0.0, 1e-12));
    assert!(near(freq_x(20_000.0), 1.0, 1e-12));
    assert!(near(freq_x(632.456), 0.5, 0.001));
    assert!(near(freq_x(f64::NAN), 0.0, 1e-12));
}

#[test]
fn wp_web6_subharmonic_as_qt() {
    // qp8
    assert!(near(subharm_bell_db(3.0, 70.0), 3.0, 1e-6));
    assert!(subharm_bell_db(3.0, 280.0) < 1.0);
    assert_eq!(subharm_bell_db(0.0, 70.0), 0.0);
    assert!(near(subharm_sub_db(0.0, 0.0, 30.0), -1.42, 0.01));
    assert_eq!(subharm_x(16.0), 0.0);
    assert!(near(subharm_x(250.0), 1.0, 1e-9));
    assert!(subharm_x(70.0) > 0.5);
    // The shared vectors
    assert!(near(subharm_bell_db(3.0, 40.0), 1.412553, 1e-5));
    assert!(near(subharm_bell_db(6.0, 100.0), 4.157328, 1e-5));
    assert!(near(subharm_sub_db(0.0, 0.0, 30.0), -1.421846, 1e-5));

    let g = subharm_graph(0.0, -6.0, -30.0, 3.0, -12.0);
    assert_eq!(g.blocks.len(), 3);
    assert!(!g.blocks[0].off && !g.blocks[1].off && g.blocks[2].off);
    assert_eq!(g.blocks[0].top.len(), 17);
    assert!(g.blocks[2].top.is_empty());
    assert_eq!(g.bell.len(), 97);
    assert!(g.ceiling_on);
    assert_eq!(g.blocks[0].label, "24-36");
    assert_eq!(g.blocks[2].source, "112-160");
    assert_eq!(g.freqs, vec![20, 50, 100, 200]);
    assert_eq!(g.grid_db, vec![12, 0, -12, -24, -36]);
    assert!(near(g.zero_y, 0.3, 1e-12));
    let off = subharm_graph(-30.0, -30.0, -30.0, 0.0, -12.0);
    assert!(off.all_off && !off.ceiling_on && off.bell.is_empty());
}

#[test]
fn wp_web6_tube_as_qt() {
    // qp8: unity small-signal gain at the default 12AX7
    let g = tube_output(&DEFAULT_TUBE, 0.001) / 0.001;
    assert!(near(g, 1.0, 0.05));
    let (second, third) = tube_harmonics(&tube(6.0, 0.0, 0.0, 50.0, 100.0, 0.0));
    assert!(second <= -100.0 && third > -100.0);
    let (second, _) = tube_harmonics(&tube(6.0, 30.0, 6.0, 40.0, 100.0, 0.0));
    assert!(second > -100.0);
    let (second, third) = tube_harmonics(&tube(-12.0, 10.0, 3.0, 40.0, 0.0, 0.0));
    assert!(second <= -100.0 && third <= -100.0);
    let lift = transformer_lift(2.0);
    assert!(near(lift.bell, 2.50, 0.01) && near(lift.top, 1.58, 0.01));
    assert!(
        tube_family(0) == 0 && tube_family(6) == 3 && tube_family(10) == 4 && tube_family(16) == 9
    );
    // The shared vectors
    assert!(near(tube_output(&DEFAULT_TUBE, 0.5), 0.489851, 1e-5));
    let hot = tube(24.0, 0.0, 6.0, 40.0, 100.0, 0.0);
    assert!(near(tube_output(&hot, 0.5), 0.038240, 1e-5));
    assert!(near(tube_output(&hot, -0.25), -0.076299, 1e-5));
    let (s2, s3) = tube_harmonics(&tube(6.0, 30.0, 6.0, 40.0, 100.0, 0.0));
    assert!(near(s2, -13.654910, 1e-4) && near(s3, -15.928919, 1e-4));
    let (d2, d3) = tube_harmonics(&DEFAULT_TUBE);
    assert!(near(d2, -39.037918, 1e-4) && near(d3, -43.730812, 1e-4));
    assert!(near(lift.bell, 2.498775, 1e-5) && near(lift.top, 1.583625, 1e-5));

    let graph = tube_graph(&DEFAULT_TUBE);
    assert_eq!(graph.curve.len(), 241);
    assert!(
        graph
            .curve
            .iter()
            .all(|p| (0.0..=1.0).contains(&p[0]) && (0.0..=1.0).contains(&p[1]))
    );
    assert!(graph.clip_pos.is_none() && graph.clip_neg.is_none());
    let hot = tube_graph(&hot);
    assert!(hot.clip_pos.is_some() && hot.clip_neg.is_some());
    assert!(near(hot.zero_y, 0.5, 1e-12));
}

#[test]
fn wp_web6_starting_points_are_the_macs() {
    assert_eq!(psybass_starting_points().len(), 4);
    assert_eq!(
        psybass_starting_points()[2].writes[0],
        ("bass.cutoff", 180.0)
    );
    let sub = subharm_starting_points(true);
    assert_eq!(sub.len(), 4);
    assert_eq!(
        sub[3].writes,
        vec![
            ("sub.low", 3.0),
            ("sub.high", -12.0),
            ("sub.boost", 0.0),
            ("sub.top", -30.0)
        ]
    );
    assert_eq!(subharm_starting_points(false)[0].writes.len(), 3);
    let t = tube_starting_points();
    assert_eq!(t.len(), 5);
    // The type first: the core loads its character, the rest keeps its own values
    assert_eq!(t[4].writes[0], ("tube.type", 12.0));
    assert!(
        t.iter()
            .all(|p| p.writes.iter().any(|w| w.0 == "tube.rectifier"))
    );
}
