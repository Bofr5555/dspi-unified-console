//! The effect pages' drawings (WP WEB6), for the eye only: the Mac's curves
//! that the Qt console draws in C++ (`EffectsController.cpp` and
//! `ToolsController.cpp`, WPs QP7/QP8), moved here so every client draws the
//! same from one place. Nothing here touches a device.
//!
//! - [`crossfeed_graph`]: the BS2B crossfeed filter's direct and opposite
//!   paths at 48 kHz.
//! - [`loudness_graph`]: the ISO 226:2003 compensation 40 dB below the
//!   reference.
//! - [`psybass_sketch`]: where the psychoacoustic bass's bands sit.
//! - [`subharm_graph`]: the subharmonic synthesizer's bands, the subs they
//!   make an octave below, the LF boost bell and the ceiling.
//! - [`tube_graph`]: the tube's static transfer curve and its harmonics at
//!   full scale; [`transformer_lift`] the output stage's lift.
//! - The starting points the Psy Bass, Subharmonic and Tube pages offer
//!   ("Apply preset"), and the tube's picture family.
//!
//! The effect graphs carry x as a 0..1 position on the 20 Hz..20 kHz log axis
//! ([`freq_x`]) and y as 0..1 from the bottom; the subharmonic and tube
//! graphs, as Qt's, y from the top. Qt keeps its own C++ copy; the tests
//! below and its smoke checks (`SmokeEffects.cpp` qp7, `SmokeTools.cpp` qp8)
//! pin both to the same numbers.

use serde::Serialize;

/// The crossfeed drawing's sample rate.
const CROSSFEED_RATE: f64 = 48_000.0;
/// The loudness drawing's volume: the curve as it is 40 dB below the reference.
const LOUDNESS_SHOWN_VOLUME_DB: f64 = -40.0;
/// Points on the crossfeed curves.
const CROSSFEED_POINTS: usize = 100;

/// The subharmonic graph's axes (the Mac's SubharmBandView).
const SUB_MIN_HZ: f64 = 16.0;
const SUB_MAX_HZ: f64 = 250.0;
const SUB_DB_MIN: f64 = -42.0;
const SUB_DB_MAX: f64 = 18.0;
const BOOST_HZ: f64 = 70.0;
const BOOST_Q: f64 = 0.9;
const DIVIDER_GAIN: f64 = 0.849;
/// A band at or under this level is off; the ceiling at 0 is off.
pub const SUB_FLOOR_DB: f64 = -30.0;
const CEILING_OFF_DB: f64 = 0.0;
/// The transfer curve's output range: a little above full scale.
const TUBE_Y_MAX: f64 = 1.4;
/// Points on the transfer curve, less one.
const TUBE_STEPS: usize = 240;

/// ISO 226:2003 per third-octave: frequency, alpha_f, L_U, T_f (the Mac's IsoData table), in
/// single precision as Qt's copy holds it.
const ISO: [[f32; 4]; 30] = [
    [20.0, 0.532, -31.6, 78.5],
    [25.0, 0.506, -27.2, 68.7],
    [31.5, 0.480, -23.0, 59.5],
    [40.0, 0.455, -19.1, 51.1],
    [50.0, 0.432, -15.9, 44.0],
    [63.0, 0.409, -13.0, 37.5],
    [80.0, 0.387, -10.3, 31.5],
    [100.0, 0.367, -8.1, 26.5],
    [125.0, 0.349, -6.2, 22.1],
    [160.0, 0.330, -4.5, 17.9],
    [200.0, 0.315, -3.1, 14.4],
    [250.0, 0.301, -2.0, 11.4],
    [315.0, 0.288, -1.1, 8.6],
    [400.0, 0.276, -0.4, 6.2],
    [500.0, 0.267, 0.0, 4.4],
    [630.0, 0.259, 0.3, 3.0],
    [800.0, 0.253, 0.5, 2.2],
    [1000.0, 0.250, 0.0, 2.4],
    [1250.0, 0.246, -2.7, 3.5],
    [1600.0, 0.244, -4.1, 1.7],
    [2000.0, 0.243, -1.0, -1.3],
    [2500.0, 0.243, 1.7, -4.2],
    [3150.0, 0.243, 2.5, -6.0],
    [4000.0, 0.242, 1.2, -5.4],
    [5000.0, 0.242, -2.1, -1.5],
    [6300.0, 0.245, -7.1, 6.0],
    [8000.0, 0.254, -11.2, 12.6],
    [10000.0, 0.271, -10.7, 13.9],
    [12500.0, 0.301, -3.1, 12.3],
    [16000.0, 0.310, -2.0, 17.0],
];

fn clamp_to(v: f64, lo: f64, hi: f64) -> f64 {
    if v.is_finite() { v.clamp(lo, hi) } else { lo }
}

fn to_db(mag: f64) -> f64 {
    20.0 * mag.max(1e-10).log10()
}

/// The x position of `hz` on the effect graphs' 20 Hz..20 kHz log axis.
pub fn freq_x(hz: f64) -> f64 {
    let f = clamp_to(if hz.is_finite() { hz } else { 20.0 }, 20.0, 20_000.0);
    (f.log10() - 20f64.log10()) / (20_000f64.log10() - 20f64.log10())
}

// ------------------------------------------------------------------ crossfeed

/// The opposite path's one-pole low pass at `f` (real, imaginary).
fn crossfeed_lowpass(fc: f64, feed_db: f64, f: f64) -> (f64, f64) {
    use std::f64::consts::PI;
    let g = 1.0 / (1.0 + 10f64.powf(feed_db / 20.0));
    let x = (-2.0 * PI * fc / CROSSFEED_RATE).exp();
    let a0 = g * (1.0 - x);
    let w = 2.0 * PI * f / CROSSFEED_RATE;
    let re = 1.0 - x * w.cos();
    let im = x * w.sin();
    let mag2 = re * re + im * im;
    (a0 * re / mag2, -a0 * im / mag2)
}

/// The crossfeed's opposite path at `f`, dB.
pub fn crossfeed_opposite_db(fc: f64, feed_db: f64, f: f64) -> f64 {
    let (re, im) = crossfeed_lowpass(fc, feed_db, f);
    to_db(re.hypot(im))
}

/// The crossfeed's direct path at `f`, dB: one minus the opposite one.
pub fn crossfeed_direct_db(fc: f64, feed_db: f64, f: f64) -> f64 {
    let (re, im) = crossfeed_lowpass(fc, feed_db, f);
    to_db((1.0 - re).hypot(-im))
}

/// One curve of an effect graph: `[x, y]` points, both 0..1, y from the bottom.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EffectCurve {
    /// "direct", "crossfeed", "curve".
    pub name: String,
    /// `[x, y]`, both 0..1, y from the bottom.
    pub points: Vec<[f64; 2]>,
}

/// A graph of the crossfeed or the loudness page: its curves and the dB grid
/// (`grid_db[i]` drawn at `grid_y[i]`, 0..1 from the bottom).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectGraph {
    /// The curves, in drawing order.
    pub curves: Vec<EffectCurve>,
    /// The dB grid's levels.
    pub grid_db: Vec<i32>,
    /// Where each level is drawn, 0..1 from the bottom.
    pub grid_y: Vec<f64>,
}

/// The y range around the curves' gains: at least 10 dB, with a fifth of it
/// above and below (the Mac's domain), its grid every 5 dB (10 dB above 40).
fn graph_of(curves: Vec<(&str, Vec<(f64, f64)>)>) -> EffectGraph {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for (_, c) in &curves {
        for &(_, db) in c {
            lo = lo.min(db);
            hi = hi.max(db);
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        lo = 0.0;
        hi = 0.0;
    }
    let visual = (hi - lo).max(10.0);
    let pad = visual * 0.2;
    let (d_min, d_max) = (lo - pad, hi + pad);
    let span = d_max - d_min;
    let y_of = |db: f64| (db - d_min) / span;
    let curves = curves
        .into_iter()
        .map(|(name, c)| EffectCurve {
            name: name.to_string(),
            points: c.iter().map(|&(f, db)| [freq_x(f), y_of(db)]).collect(),
        })
        .collect();
    let step = if span > 40.0 { 10.0 } else { 5.0 };
    let mut grid_db = Vec::new();
    let mut grid_y = Vec::new();
    let mut v = (d_min / step).ceil() * step;
    while v <= d_max {
        grid_db.push(v as i32);
        grid_y.push(y_of(v));
        v += step;
    }
    EffectGraph {
        curves,
        grid_db,
        grid_y,
    }
}

/// The crossfeed's frequency response at a cutoff and feed: "direct" and
/// "crossfeed", 100 points each from 20 Hz to 20 kHz.
pub fn crossfeed_graph(freq_hz: f64, feed_db: f64) -> EffectGraph {
    let mut direct = Vec::with_capacity(CROSSFEED_POINTS);
    let mut opposite = Vec::with_capacity(CROSSFEED_POINTS);
    for i in 0..CROSSFEED_POINTS {
        let f = 20.0 * 1000f64.powf(i as f64 / (CROSSFEED_POINTS - 1) as f64);
        direct.push((f, crossfeed_direct_db(freq_hz, feed_db, f)));
        opposite.push((f, crossfeed_opposite_db(freq_hz, feed_db, f)));
    }
    graph_of(vec![("direct", direct), ("crossfeed", opposite)])
}

// ------------------------------------------------------------------- loudness

/// The SPL of `phon` at one ISO row.
fn iso_spl(r: &[f32; 4], phon: f64) -> f64 {
    let [_, af, lu, tf] = r.map(f64::from);
    let b = 0.4 * 10f64.powf((tf + lu) / 10.0 - 9.0);
    let a = (4.47e-3 * (10f64.powf(0.025 * phon) - 1.15) + b.powf(af)).max(1e-10);
    (10.0 / af) * a.log10() - lu + 94.0
}

/// The ISO table's rows: 30, 20 Hz to 16 kHz.
pub fn iso_rows() -> usize {
    ISO.len()
}

/// The frequency of ISO row `row`, 0 for none.
pub fn iso_freq(row: usize) -> f64 {
    ISO.get(row).map_or(0.0, |r| f64::from(r[0]))
}

/// The loudness compensation at ISO row `row`, 40 dB below `ref_spl`, dB.
pub fn loudness_compensation_db(ref_spl: f64, intensity_pct: f64, row: usize) -> f64 {
    let Some(r) = ISO.get(row) else { return 0.0 };
    let phon = (ref_spl + LOUDNESS_SHOWN_VOLUME_DB).clamp(20.0, ref_spl.max(20.0));
    if phon >= ref_spl {
        return 0.0;
    }
    let shaped = iso_spl(r, phon) - iso_spl(r, ref_spl);
    let flat = phon - ref_spl;
    (shaped - flat) * (intensity_pct / 100.0)
}

/// The loudness compensation curve: "curve", a point per ISO row.
pub fn loudness_graph(ref_spl: f64, intensity_pct: f64) -> EffectGraph {
    let curve = (0..iso_rows())
        .map(|i| {
            (
                iso_freq(i),
                loudness_compensation_db(ref_spl, intensity_pct, i),
            )
        })
        .collect();
    graph_of(vec![("curve", curve)])
}

// -------------------------------------------------------------------- psy bass

/// Where the psychoacoustic bass's bands sit: the cutoff and four times it on
/// the log axis, the original's and the harmonics' heights 0..1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct PsybassSketch {
    /// The cutoff on the log axis.
    pub fc: f64,
    /// Four times the cutoff, where the harmonics end.
    pub fc4: f64,
    /// The original bass's height, 0..1 (-60..0 dB).
    pub original: f64,
    /// The harmonics' height, 0..1 (-24..+12 dB).
    pub harmonics: f64,
}

/// The psychoacoustic bass's sketch at a cutoff (Hz), harmonics and original levels (dB).
pub fn psybass_sketch(cutoff_hz: f64, harmonics_db: f64, original_db: f64) -> PsybassSketch {
    PsybassSketch {
        fc: freq_x(cutoff_hz),
        fc4: freq_x(cutoff_hz * 4.0),
        original: clamp_to((original_db + 60.0) / 60.0, 0.0, 1.0),
        harmonics: clamp_to((harmonics_db + 24.0) / 36.0, 0.0, 1.0),
    }
}

// ----------------------------------------------------------------- subharmonic

/// The x position of `hz` on the band graph's 16..250 Hz log axis.
pub fn subharm_x(hz: f64) -> f64 {
    let f = clamp_to(hz, SUB_MIN_HZ, SUB_MAX_HZ);
    (f.log10() - SUB_MIN_HZ.log10()) / (SUB_MAX_HZ.log10() - SUB_MIN_HZ.log10())
}

/// The 70 Hz LF boost bell: the analog peaking prototype, peak gain A² with A = 10^(boost/40).
pub fn subharm_bell_db(boost_db: f64, hz: f64) -> f64 {
    if boost_db.is_nan() || boost_db <= 0.0 {
        return 0.0;
    }
    let a = 10f64.powf(boost_db / 40.0);
    let w = hz / BOOST_HZ;
    let base = (1.0 - w * w) * (1.0 - w * w);
    let num = base + (a * w / BOOST_Q).powi(2);
    let den = base + (w / (a * BOOST_Q)).powi(2);
    10.0 * (num / den).log10()
}

/// A synthesized sub's steady level: the band, the divider's own gain, the bell there.
pub fn subharm_sub_db(band_db: f64, boost_db: f64, hz: f64) -> f64 {
    band_db + 20.0 * DIVIDER_GAIN.log10() + subharm_bell_db(boost_db, hz)
}

/// One sub band of the band graph. x positions on the 16..250 Hz axis, y
/// 0..1 from the top of the plot.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubBand {
    /// The band's level is at the floor: no sub.
    pub off: bool,
    /// The sub's range: its lower end.
    pub x0: f64,
    /// Its upper end.
    pub x1: f64,
    /// The program band it comes from, an octave above: its lower end.
    pub src_x0: f64,
    /// Its upper end.
    pub src_x1: f64,
    /// The sub's level at its middle.
    pub mid_y: f64,
    /// The sub's range in Hz, "24-36".
    pub label: String,
    /// Its program band's, "48-72".
    pub source: String,
    /// The octave arrow: from the program band.
    pub from_x: f64,
    /// To its sub.
    pub to_x: f64,
    /// The arrow's row, px from the top of the drawing.
    pub arrow_row: f64,
    /// The sub's top, following the bell: `[x, y]`, empty when off.
    pub top: Vec<[f64; 2]>,
}

/// The subharmonic synthesizer's band graph.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubharmGraph {
    /// The LF boost bell, `[x, y]`, empty while the boost is off.
    pub bell: Vec<[f64; 2]>,
    /// Where the bell's label goes: at 70 Hz,
    pub bell_x: f64,
    /// at its peak.
    pub bell_label_y: f64,
    /// The three sub bands, lowest first.
    pub blocks: Vec<SubBand>,
    /// Every band is off.
    pub all_off: bool,
    /// The ceiling holds the sub down: drawn.
    pub ceiling_on: bool,
    /// The ceiling's level,
    pub ceiling_y: f64,
    /// drawn up to 80 Hz.
    pub ceiling_x: f64,
    /// The 0 dB line.
    pub zero_y: f64,
    /// Frequency lines,
    pub grid_x: Vec<f64>,
    /// at these frequencies (Hz).
    pub freqs: Vec<i32>,
    /// Level lines,
    pub grid_y: Vec<f64>,
    /// at these levels (dB).
    pub grid_db: Vec<i32>,
}

/// The band graph for the three band levels, the LF boost and the ceiling, dB.
pub fn subharm_graph(low: f64, high: f64, top: f64, boost: f64, ceiling: f64) -> SubharmGraph {
    let y_of =
        |db: f64| (SUB_DB_MAX - clamp_to(db, SUB_DB_MIN, SUB_DB_MAX)) / (SUB_DB_MAX - SUB_DB_MIN);
    let mut bell = Vec::new();
    if boost > 0.0 {
        for i in 0..=96 {
            let t = i as f64 / 96.0;
            let f = 10f64.powf(SUB_MIN_HZ.log10() + t * (SUB_MAX_HZ.log10() - SUB_MIN_HZ.log10()));
            bell.push([t, y_of(subharm_bell_db(boost, f))]);
        }
    }
    // lo, hi, srcLo, srcHi, level, arrow from, arrow to, arrow row
    let bands: [[f64; 8]; 3] = [
        [24.0, 36.0, 48.0, 72.0, low, 60.0, 30.0, 15.0],
        [36.0, 56.0, 72.0, 112.0, high, 92.0, 46.0, 23.0],
        [56.0, 80.0, 112.0, 160.0, top, 136.0, 68.0, 31.0],
    ];
    let mut all_off = true;
    let blocks = bands
        .iter()
        .map(|&[lo, hi, src_lo, src_hi, level, from, to, row]| {
            let off = level <= SUB_FLOOR_DB;
            all_off = all_off && off;
            let top = if off {
                Vec::new()
            } else {
                (0..=16)
                    .map(|i| {
                        let f = lo * (hi / lo).powf(i as f64 / 16.0);
                        [subharm_x(f), y_of(subharm_sub_db(level, boost, f))]
                    })
                    .collect()
            };
            SubBand {
                off,
                x0: subharm_x(lo),
                x1: subharm_x(hi),
                src_x0: subharm_x(src_lo),
                src_x1: subharm_x(src_hi),
                mid_y: y_of(subharm_sub_db(level, boost, (lo * hi).sqrt())),
                label: format!("{}-{}", lo as i32, hi as i32),
                source: format!("{}-{}", src_lo as i32, src_hi as i32),
                from_x: subharm_x(from),
                to_x: subharm_x(to),
                arrow_row: row,
                top,
            }
        })
        .collect();
    let freqs = vec![20, 50, 100, 200];
    let grid_db = vec![12, 0, -12, -24, -36];
    SubharmGraph {
        bell,
        bell_x: subharm_x(BOOST_HZ),
        bell_label_y: y_of(subharm_bell_db(boost, BOOST_HZ)),
        blocks,
        all_off,
        ceiling_on: ceiling < CEILING_OFF_DB && !all_off,
        ceiling_y: y_of(ceiling),
        ceiling_x: subharm_x(80.0),
        zero_y: y_of(0.0),
        grid_x: freqs.iter().map(|&f| subharm_x(f64::from(f))).collect(),
        freqs,
        grid_y: grid_db.iter().map(|&d| y_of(f64::from(d))).collect(),
        grid_db,
    }
}

// ------------------------------------------------------------------------ tube

/// The tube's static shaper (the Mac's): drive moves the knee, not the level.
struct Shaper {
    m: f64,
    b: f64,
    ratio_n: f64,
    c1: f64,
    c3: f64,
    c5: f64,
    s_p: f64,
    s_n: f64,
    dry_w: f64,
    wet_w: f64,
    v0: f64,
}

impl Shaper {
    fn new(drive: f64, bias: f64, asym: f64, hardness: f64, mix: f64, trim: f64) -> Self {
        let m = 10f64.powf(drive / 20.0);
        let h = hardness / 100.0;
        let c1 = 1.5 + 0.375 * h;
        let mut s = Shaper {
            m,
            b: bias / 200.0,
            ratio_n: 10f64.powf(-asym / 20.0),
            c1,
            c3: -0.5 - 0.75 * h,
            c5: 0.375 * h,
            s_p: 1.0 / (c1 * m),
            s_n: 10f64.powf(asym / 20.0) / (c1 * m),
            dry_w: 1.0 - mix / 100.0,
            wet_w: mix / 100.0 * 10f64.powf(trim / 20.0),
            v0: 0.0,
        };
        s.v0 = s.shape(0.0);
        s
    }

    fn shape(&self, x: f64) -> f64 {
        let mut t = self.m * x + self.b;
        if t < 0.0 {
            t *= self.ratio_n;
        }
        let t = t.clamp(-1.0, 1.0);
        let t2 = t * t;
        t * (self.c1 + t2 * (self.c3 + t2 * self.c5)) * if t >= 0.0 { self.s_p } else { self.s_n }
    }

    fn output(&self, x: f64) -> f64 {
        self.dry_w * x + self.wet_w * (self.shape(x) - self.v0)
    }
}

/// The tube's controls the drawings take: drive dB, bias %, asymmetry dB,
/// hardness %, mix %, trim dB.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
pub struct TubeShape {
    /// dB.
    pub drive: f64,
    /// %.
    pub bias: f64,
    /// dB.
    pub asym: f64,
    /// %.
    pub hardness: f64,
    /// %.
    pub mix: f64,
    /// dB.
    pub trim: f64,
}

impl TubeShape {
    fn shaper(&self) -> Shaper {
        Shaper::new(
            self.drive,
            self.bias,
            self.asym,
            self.hardness,
            self.mix,
            self.trim,
        )
    }
}

/// The stage's output for input `x` (full scale 1).
pub fn tube_output(t: &TubeShape, x: f64) -> f64 {
    t.shaper().output(x)
}

/// The second and third harmonic of a full-scale sine against the
/// fundamental, dB (-120 for none): a 256-point DFT at three bins, exact for
/// a curve without memory.
pub fn tube_harmonics(t: &TubeShape) -> (f64, f64) {
    use std::f64::consts::PI;
    let s = t.shaper();
    const N: u32 = 256;
    // The fundamental, the second and the third: (harmonic, re, im)
    let mut bins = [(1.0f64, 0.0f64, 0.0f64), (2.0, 0.0, 0.0), (3.0, 0.0, 0.0)];
    for i in 0..N {
        let phase = 2.0 * PI * f64::from(i) / f64::from(N);
        let y = s.output(phase.sin());
        for (h, re, im) in &mut bins {
            *re += y * (*h * phase).cos();
            *im += y * (*h * phase).sin();
        }
    }
    let [first, second, third] = bins.map(|(_, re, im)| re.hypot(im));
    let rel = |a: f64| {
        if first > 1e-12 && a > first * 1e-6 {
            20.0 * (a / first).log10()
        } else {
            -120.0
        }
    };
    (rel(second), rel(third))
}

/// The tube's transfer curve: output against input over one full-scale
/// swing, x and y 0..1 across the plot (y from the top), the straight line a
/// clean stage draws (from `line_from` at x 0 to `line_to` at x 1), where the
/// input clips (None: it does not), and the harmonics at full scale.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TubeGraph {
    /// 241 `[x, y]` points.
    pub curve: Vec<[f64; 2]>,
    /// The input where the positive half clips, x.
    pub clip_pos: Option<f64>,
    /// Where the negative half clips, x.
    pub clip_neg: Option<f64>,
    /// Full scale's lines: +1,
    pub full_scale_top: f64,
    /// and -1.
    pub full_scale_bottom: f64,
    /// Half scale's lines, +0.5 and -0.5.
    pub half: [f64; 2],
    /// The 0 line.
    pub zero_y: f64,
    /// The clean stage's line: y at x 0,
    pub line_from: f64,
    /// and at x 1.
    pub line_to: f64,
    /// The second harmonic at full scale, dB against the fundamental.
    pub second: f64,
    /// The third.
    pub third: f64,
}

/// The tube's transfer curve at its controls.
pub fn tube_graph(t: &TubeShape) -> TubeGraph {
    let x_of = |x: f64| (x + 1.0) / 2.0;
    let y_of = |y: f64| (TUBE_Y_MAX - y.clamp(-TUBE_Y_MAX, TUBE_Y_MAX)) / (2.0 * TUBE_Y_MAX);
    let s = t.shaper();
    let curve = (0..=TUBE_STEPS)
        .map(|i| {
            let x = -1.0 + 2.0 * i as f64 / TUBE_STEPS as f64;
            [x_of(x), y_of(s.output(x))]
        })
        .collect();
    // The inputs the stage clips: m x + b = 1, and (m x + b) ratio = -1
    let x_pos = (1.0 - s.b) / s.m;
    let x_neg = (-1.0 / s.ratio_n - s.b) / s.m;
    let (second, third) = tube_harmonics(t);
    TubeGraph {
        curve,
        clip_pos: (x_pos < 1.0).then(|| x_of(x_pos.max(-1.0))),
        clip_neg: (x_neg > -1.0).then(|| x_of(x_neg.min(1.0))),
        full_scale_top: y_of(1.0),
        full_scale_bottom: y_of(-1.0),
        half: [y_of(0.5), y_of(-0.5)],
        zero_y: y_of(0.0),
        line_from: y_of(-1.0),
        line_to: y_of(1.0),
        second,
        third,
    }
}

/// The output stage's lift at a damping factor, dB: at resonance and at the
/// top (the source impedance against a speaker rising to 4x and 2x).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct TransformerLift {
    /// At the speaker's resonance, dB.
    pub bell: f64,
    /// At the top, dB.
    pub top: f64,
}

/// The output stage's lift at a damping factor (at least 1).
pub fn transformer_lift(damping: f64) -> TransformerLift {
    let df = damping.max(1.0);
    let bump = 4.0 * (df + 1.0) / (4.0 * df + 1.0);
    let top = 2.0 * (df + 1.0) / (2.0 * df + 1.0);
    TransformerLift {
        bell: 20.0 * bump.log10(),
        top: 20.0 * top.log10(),
    }
}

/// The picture of tube type `tube_type` (the Mac's families): 0 noval
/// triode, 1 noval pentode, 2 noval power, 3 octal glass, 4 octal metal, 5
/// large octal power, 6 small octal power, 7 shouldered power, 8 beam bottle,
/// 9 directly heated.
pub fn tube_family(tube_type: u8) -> u8 {
    match tube_type {
        6 | 7 => 3,
        9 => 1,
        10 => 4,
        11 => 2,
        12 => 5,
        13 => 7,
        14 => 6,
        15 => 8,
        16 => 9,
        _ => 0,
    }
}

// ------------------------------------------------------------- starting points

/// A starting point an effect page offers ("Apply preset"): its name, what it
/// is, and the registry paths it writes with their values, in order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StartingPoint {
    /// "Cinema LFE".
    pub name: &'static str,
    /// "Lowest octave only".
    pub detail: &'static str,
    /// Registry path and value, in the order written.
    pub writes: Vec<(&'static str, f64)>,
}

fn point(
    name: &'static str,
    detail: &'static str,
    writes: &[(&'static str, f64)],
) -> StartingPoint {
    StartingPoint {
        name,
        detail,
        writes: writes.to_vec(),
    }
}

/// The Psy Bass starting points (the Mac's table): cutoff, harmonics, drive, character, original.
pub fn psybass_starting_points() -> Vec<StartingPoint> {
    let p = |name, detail, cutoff, harmonics, drive, character, original| {
        point(
            name,
            detail,
            &[
                ("bass.cutoff", cutoff),
                ("bass.harmonics", harmonics),
                ("bass.drive", drive),
                ("bass.character", character),
                ("bass.original", original),
            ],
        )
    };
    vec![
        p(
            "Bookshelf speakers",
            "Gentle low-end help",
            60.0,
            0.0,
            6.0,
            50.0,
            0.0,
        ),
        p(
            "Small Bluetooth",
            "Portable speaker",
            100.0,
            3.0,
            9.0,
            40.0,
            -12.0,
        ),
        p(
            "Laptop / tablet",
            "Tiny drivers, protect them",
            180.0,
            6.0,
            12.0,
            50.0,
            -24.0,
        ),
        p(
            "Headphone bass feel",
            "Extra sub sensation",
            45.0,
            -3.0,
            6.0,
            30.0,
            0.0,
        ),
    ]
}

/// The Subharmonic starting points (the Mac's table): low, high, boost; each
/// sets the top band to its floor, since none uses it (`extended`: the
/// device has the top band).
pub fn subharm_starting_points(extended: bool) -> Vec<StartingPoint> {
    let p = |name, detail, low, high, boost| {
        let mut writes = vec![("sub.low", low), ("sub.high", high), ("sub.boost", boost)];
        if extended {
            writes.push(("sub.top", SUB_FLOOR_DB));
        }
        StartingPoint {
            name,
            detail,
            writes,
        }
    };
    vec![
        p("Subwoofer feed", "Subtle added weight", -6.0, -6.0, 0.0),
        p("Club / large PA", "Lower two bands, full", 0.0, 0.0, 3.0),
        p("Thin recordings", "Add a missing bottom", 0.0, -6.0, 3.0),
        p("Cinema LFE", "Lowest octave only", 3.0, -12.0, 0.0),
    ]
}

/// The Tube starting points (the Mac's table): every control but the
/// character, which the type loads, so the same sound whatever came before.
pub fn tube_starting_points() -> Vec<StartingPoint> {
    let p = |name, detail, ty: f64, drive, rect: f64, xfmr: bool, damping, resonance| {
        point(
            name,
            detail,
            &[
                ("tube.type", ty),
                ("tube.drive", drive),
                ("tube.rectifier", rect),
                ("tube.damping", damping),
                ("tube.resonance", resonance),
                ("tube.xfmr", if xfmr { 1.0 } else { 0.0 }),
                ("tube.mix", 100.0),
                ("tube.trim", 0.0),
            ],
        )
    };
    vec![
        p(
            "Clean default",
            "12AX7, level-neutral, output stage on",
            1.0,
            -12.0,
            1.0,
            true,
            2.0,
            95.0,
        ),
        p(
            "Warm hi-fi",
            "12AU7 line stage, tightly damped",
            5.0,
            -3.0,
            1.0,
            true,
            10.0,
            95.0,
        ),
        p(
            "Single-ended sweetness",
            "300B, loose damping",
            16.0,
            3.0,
            1.0,
            true,
            2.0,
            95.0,
        ),
        p(
            "Guitar-amp style",
            "12AX7 pushed, 5U4, loose damping",
            1.0,
            15.0,
            2.0,
            true,
            2.0,
            100.0,
        ),
        p(
            "Push-pull power",
            "EL34 with the output stage on",
            12.0,
            0.0,
            1.0,
            true,
            6.0,
            95.0,
        ),
    ]
}

#[cfg(test)]
#[path = "effect_curves/tests_wp_web6.rs"]
mod tests_wp_web6;
