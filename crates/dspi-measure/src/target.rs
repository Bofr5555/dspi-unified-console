//! Target curves and coupler compensation (WP P5CAL, PLAN-P5 T3).
//!
//! Built-in targets are **defined by parameters in our own code** (OD-P5p: no
//! third-party curve data is bundled): flat, a tilt in dB/octave, a house curve
//! (a bass shelf plus a tilt) and a default per headphone [`Coupler`], each
//! named as ours. Any other target, and a coupler's compensation, is imported
//! from REW or CSV text with [`crate::cal`]'s bounded parser. A target is
//! normalised at a reference band (default 500 Hz to 2 kHz, mean 0 dB) before
//! it is drawn or fitted.
//!
//! The shapes are analogue prototypes evaluated in closed form (no sample
//! rate): a shelf of gain `G` reads exactly `G/2` dB at its corner and tends to
//! `G` far on its shelf side; a peak of gain `G` reads exactly `G` at its
//! centre; a tilt reads 0 dB at [`TILT_PIVOT_HZ`] and `s` dB one octave above.

use crate::cal::{CurvePoint, ParsedCurve, curve_to_grid, parse_curve};
use crate::types::{Coupler, LogGrid, MeasureError, Response};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

/// The frequency a tilt pivots about (0 dB there), Hz.
pub const TILT_PIVOT_HZ: f64 = 1000.0;
/// The steepest tilt accepted, dB/octave (either sign).
pub const MAX_TILT_DB_PER_OCTAVE: f64 = 3.0;
/// The largest shelf or peak gain accepted, dB (either sign).
pub const MAX_SHAPE_GAIN_DB: f64 = 20.0;
/// The default reference band of a normalisation, Hz.
pub const DEFAULT_REFERENCE_BAND: ReferenceBand = ReferenceBand {
    low_hz: 500.0,
    high_hz: 2000.0,
};

/// What a target is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TargetUse {
    /// Loudspeakers in a room.
    Speakers,
    /// Headphones on a coupler.
    Headphones,
}

/// A second-order shelf (analogue prototype).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shelf {
    /// Gain on the shelf side, dB.
    pub gain_db: f64,
    /// Corner (where the shelf reads half its gain), Hz.
    pub corner_hz: f64,
    /// Quality factor (0.3 to 2; 0.707 is the usual).
    pub q: f64,
}

/// A peaking shape (analogue prototype).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Peak {
    /// Gain at the centre, dB.
    pub gain_db: f64,
    /// Centre, Hz.
    pub center_hz: f64,
    /// Quality factor (0.3 to 10).
    pub q: f64,
}

/// A parametric curve: the sum in dB of a tilt about [`TILT_PIVOT_HZ`], an
/// optional low shelf, peaks and an optional high shelf.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParametricCurve {
    /// Tilt, dB/octave.
    pub tilt_db_per_octave: f64,
    /// A bass shelf.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_shelf: Option<Shelf>,
    /// Peaks (at most 4).
    #[serde(default)]
    pub peaks: Vec<Peak>,
    /// A treble shelf.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high_shelf: Option<Shelf>,
}

/// The most peaks a [`ParametricCurve`] may have.
pub const MAX_PEAKS: usize = 4;

/// A built-in target, defined by parameters.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Builtin {
    /// 0 dB everywhere.
    Flat,
    /// A straight line in dB over log frequency through 0 dB at
    /// [`TILT_PIVOT_HZ`].
    Tilt {
        /// dB per octave (negative falls toward treble).
        db_per_octave: f64,
    },
    /// A house curve: a bass shelf plus a tilt.
    House {
        /// The shelf's gain, dB.
        shelf_gain_db: f64,
        /// The shelf's corner, Hz.
        shelf_corner_hz: f64,
        /// The shelf's Q.
        shelf_q: f64,
        /// The tilt, dB/octave.
        tilt_db_per_octave: f64,
    },
    /// Our default target for a headphone coupler ([`coupler_default`]).
    CouplerDefault {
        /// The coupler type.
        coupler: Coupler,
    },
}

// -- review P5-A -- a built-in's stable id and named parameters: what a
// `target` document's `definition` holds (`{"type": "builtin", "builtin":
// id, "params": {name: number}}`, docs/integrator/measurement.md 7.5).
/// The `tilt` built-in's parameter: dB per octave.
pub const PARAM_TILT: &str = "dbPerOctave";
/// The `house` built-in's parameters: the shelf's gain, dB.
pub const PARAM_SHELF_GAIN: &str = "shelfGainDb";
/// The shelf's corner, Hz.
pub const PARAM_SHELF_CORNER: &str = "shelfCornerHz";
/// The shelf's Q.
pub const PARAM_SHELF_Q: &str = "shelfQ";
/// The house curve's tilt, dB per octave.
pub const PARAM_HOUSE_TILT: &str = "tiltDbPerOctave";

impl Builtin {
    /// The built-in's stable id, as [`builtins`] lists it: `flat`, `tilt`,
    /// `house`, `coupler-iec60318-4`, `coupler-flat-plate`, `coupler-hats`.
    pub fn id(&self) -> String {
        match self {
            Builtin::Flat => "flat".into(),
            Builtin::Tilt { .. } => "tilt".into(),
            Builtin::House { .. } => "house".into(),
            Builtin::CouplerDefault { coupler } => format!("coupler-{}", coupler_id(*coupler)),
        }
    }

    /// The built-in's parameters by name (none for `flat` and the coupler
    /// defaults, which are fixed curves).
    pub fn params(&self) -> std::collections::BTreeMap<String, f64> {
        let mut p = std::collections::BTreeMap::new();
        match *self {
            Builtin::Flat | Builtin::CouplerDefault { .. } => {}
            Builtin::Tilt { db_per_octave } => {
                p.insert(PARAM_TILT.to_string(), db_per_octave);
            }
            Builtin::House {
                shelf_gain_db,
                shelf_corner_hz,
                shelf_q,
                tilt_db_per_octave,
            } => {
                p.insert(PARAM_SHELF_GAIN.to_string(), shelf_gain_db);
                p.insert(PARAM_SHELF_CORNER.to_string(), shelf_corner_hz);
                p.insert(PARAM_SHELF_Q.to_string(), shelf_q);
                p.insert(PARAM_HOUSE_TILT.to_string(), tilt_db_per_octave);
            }
        }
        p
    }

    /// The built-in an id and named parameters describe. A parameter left
    /// out takes the built-in's default ([`builtins`]); the result is checked
    /// ([`Builtin::curve`]).
    ///
    /// Errors: [`MeasureError::Invalid`] for an id that is not a built-in's,
    /// a parameter name the built-in does not have (a misspelt name must not
    /// silently give the default curve), or a value out of range.
    pub fn from_id(
        id: &str,
        params: &std::collections::BTreeMap<String, f64>,
    ) -> Result<Builtin, MeasureError> {
        let base = builtins()
            .into_iter()
            .find(|b| b.id == id)
            .map(|b| b.builtin)
            .ok_or_else(|| {
                MeasureError::Invalid(format!(
                    "no built-in target \"{}\"",
                    id.chars().take(32).collect::<String>()
                ))
            })?;
        let mut known = base.params();
        for (name, value) in params {
            match known.get_mut(name) {
                Some(slot) => *slot = *value,
                None => {
                    return Err(MeasureError::Invalid(format!(
                        "the built-in target \"{id}\" has no parameter \"{}\"",
                        name.chars().take(32).collect::<String>()
                    )));
                }
            }
        }
        let get = |name: &str| known.get(name).copied().unwrap_or(0.0);
        let built = match base {
            Builtin::Flat | Builtin::CouplerDefault { .. } => base,
            Builtin::Tilt { .. } => Builtin::Tilt {
                db_per_octave: get(PARAM_TILT),
            },
            Builtin::House { .. } => Builtin::House {
                shelf_gain_db: get(PARAM_SHELF_GAIN),
                shelf_corner_hz: get(PARAM_SHELF_CORNER),
                shelf_q: get(PARAM_SHELF_Q),
                tilt_db_per_octave: get(PARAM_HOUSE_TILT),
            },
        };
        built.curve()?;
        Ok(built)
    }
}

/// The house curve's defaults: +6 dB shelf at 120 Hz, Q 0.707, -0.3 dB/octave.
pub const HOUSE_DEFAULT: Builtin = Builtin::House {
    shelf_gain_db: 6.0,
    shelf_corner_hz: 120.0,
    shelf_q: 0.707,
    tilt_db_per_octave: -0.3,
};

/// A built-in's listing for a client (P5DOCK's `target_builtins`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinInfo {
    /// A stable id (`flat`, `tilt`, `house`, `coupler-iec60318-4`, …).
    pub id: String,
    /// The display name (says it is ours).
    pub name: String,
    /// What it is for.
    pub target_use: TargetUse,
    /// The coupler it is the default for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coupler: Option<Coupler>,
    /// The parameters with their defaults.
    pub builtin: Builtin,
}

/// Every built-in with its default parameters, in a stable order.
pub fn builtins() -> Vec<BuiltinInfo> {
    let mut v = vec![
        BuiltinInfo {
            id: "flat".into(),
            name: "Flat".into(),
            target_use: TargetUse::Speakers,
            coupler: None,
            builtin: Builtin::Flat,
        },
        BuiltinInfo {
            id: "tilt".into(),
            name: "Tilt (DSPi)".into(),
            target_use: TargetUse::Speakers,
            coupler: None,
            builtin: Builtin::Tilt {
                db_per_octave: -0.5,
            },
        },
        BuiltinInfo {
            id: "house".into(),
            name: "House curve (DSPi)".into(),
            target_use: TargetUse::Speakers,
            coupler: None,
            builtin: HOUSE_DEFAULT,
        },
    ];
    for c in [Coupler::Iec60318_4, Coupler::FlatPlate, Coupler::Hats] {
        v.push(BuiltinInfo {
            id: format!("coupler-{}", coupler_id(c)),
            name: format!("{} default (DSPi)", coupler_name(c)),
            target_use: TargetUse::Headphones,
            coupler: Some(c),
            builtin: Builtin::CouplerDefault { coupler: c },
        });
    }
    v
}

/// The default target for a use: the house curve for speakers, the coupler's
/// default for headphones (the IEC 60318-4 one when no coupler is given).
pub fn default_target(target_use: TargetUse, coupler: Option<Coupler>) -> Builtin {
    match target_use {
        TargetUse::Speakers => HOUSE_DEFAULT,
        TargetUse::Headphones => Builtin::CouplerDefault {
            coupler: coupler.unwrap_or(Coupler::Iec60318_4),
        },
    }
}

fn coupler_id(c: Coupler) -> &'static str {
    match c {
        Coupler::Iec60318_4 => "iec60318-4",
        Coupler::FlatPlate => "flat-plate",
        Coupler::Hats => "hats",
    }
}

fn coupler_name(c: Coupler) -> &'static str {
    match c {
        Coupler::Iec60318_4 => "IEC 60318-4 ear simulator",
        Coupler::FlatPlate => "Flat-plate coupler",
        Coupler::Hats => "HATS or ear coupler with pinna",
    }
}

/// Our default target on a coupler, as parameters (PLAN-P5 section 1.3). These
/// are **our own** smooth curves, not any published target:
/// - IEC 60318-4 ear simulator: a diffuse-field-like shape, +5 dB bass shelf at
///   120 Hz, the ear-canal gain as +9 dB at 2.8 kHz (Q 1), a -4 dB treble shelf
///   at 10 kHz.
/// - HATS or another ear coupler with pinna: as the ear simulator (its own
///   compensation file, if any, does the rest).
/// - Flat-plate coupler: no ear-canal gain to expect, so +4 dB bass shelf at
///   120 Hz and a mild +3 dB at 3 kHz (Q 0.9); a vendor's compensation file
///   is better.
pub fn coupler_default(coupler: Coupler) -> ParametricCurve {
    match coupler {
        Coupler::Iec60318_4 | Coupler::Hats => ParametricCurve {
            tilt_db_per_octave: 0.0,
            low_shelf: Some(Shelf {
                gain_db: 5.0,
                corner_hz: 120.0,
                q: 0.707,
            }),
            peaks: vec![Peak {
                gain_db: 9.0,
                center_hz: 2800.0,
                q: 1.0,
            }],
            high_shelf: Some(Shelf {
                gain_db: -4.0,
                corner_hz: 10_000.0,
                q: 0.707,
            }),
        },
        Coupler::FlatPlate => ParametricCurve {
            tilt_db_per_octave: 0.0,
            low_shelf: Some(Shelf {
                gain_db: 4.0,
                corner_hz: 120.0,
                q: 0.707,
            }),
            peaks: vec![Peak {
                gain_db: 3.0,
                center_hz: 3000.0,
                q: 0.9,
            }],
            high_shelf: None,
        },
    }
}

impl Builtin {
    /// The built-in as a parametric curve.
    ///
    /// Errors: [`MeasureError::Invalid`] for a parameter out of range.
    pub fn curve(&self) -> Result<ParametricCurve, MeasureError> {
        let c = match *self {
            Builtin::Flat => ParametricCurve {
                tilt_db_per_octave: 0.0,
                low_shelf: None,
                peaks: Vec::new(),
                high_shelf: None,
            },
            Builtin::Tilt { db_per_octave } => ParametricCurve {
                tilt_db_per_octave: db_per_octave,
                low_shelf: None,
                peaks: Vec::new(),
                high_shelf: None,
            },
            Builtin::House {
                shelf_gain_db,
                shelf_corner_hz,
                shelf_q,
                tilt_db_per_octave,
            } => ParametricCurve {
                tilt_db_per_octave,
                low_shelf: Some(Shelf {
                    gain_db: shelf_gain_db,
                    corner_hz: shelf_corner_hz,
                    q: shelf_q,
                }),
                peaks: Vec::new(),
                high_shelf: None,
            },
            Builtin::CouplerDefault { coupler } => coupler_default(coupler),
        };
        c.validate()?;
        Ok(c)
    }

    /// The built-in on `grid`, not normalised (exact at its defining points).
    ///
    /// Errors: [`MeasureError::Invalid`] for a bad grid or parameter.
    pub fn evaluate(&self, grid: &LogGrid) -> Result<Response, MeasureError> {
        self.curve()?.evaluate(grid)
    }
}

impl ParametricCurve {
    /// Checks every parameter's range.
    ///
    /// Errors: [`MeasureError::Invalid`] naming the first parameter out of range.
    pub fn validate(&self) -> Result<(), MeasureError> {
        let bad = |what: &str| Err(MeasureError::Invalid(format!("target {what} out of range")));
        if !self.tilt_db_per_octave.is_finite()
            || self.tilt_db_per_octave.abs() > MAX_TILT_DB_PER_OCTAVE
        {
            return bad("tilt");
        }
        if self.peaks.len() > MAX_PEAKS {
            return bad("peak count");
        }
        let shape_ok = |g: f64, f: f64, q: f64, qmin: f64, qmax: f64| {
            g.is_finite()
                && g.abs() <= MAX_SHAPE_GAIN_DB
                && f.is_finite()
                && (10.0..=22_000.0).contains(&f)
                && q.is_finite()
                && (qmin..=qmax).contains(&q)
        };
        for s in self.low_shelf.iter().chain(self.high_shelf.iter()) {
            if !shape_ok(s.gain_db, s.corner_hz, s.q, 0.3, 2.0) {
                return bad("shelf");
            }
        }
        for p in &self.peaks {
            if !shape_ok(p.gain_db, p.center_hz, p.q, 0.3, 10.0) {
                return bad("peak");
            }
        }
        Ok(())
    }

    /// The curve's value at `f` Hz, dB (assumes [`ParametricCurve::validate`]
    /// passed and `f > 0`).
    pub fn value_at(&self, f: f64) -> f64 {
        let mut db = self.tilt_db_per_octave * (f / TILT_PIVOT_HZ).log2();
        if let Some(s) = self.low_shelf {
            db += low_shelf_db(s, f);
        }
        for p in &self.peaks {
            db += peak_db(*p, f);
        }
        if let Some(s) = self.high_shelf {
            db += high_shelf_db(s, f);
        }
        db
    }

    /// The curve on `grid`, not normalised; every point valid, no phase.
    ///
    /// Errors: [`MeasureError::Invalid`] for a bad grid or parameter.
    pub fn evaluate(&self, grid: &LogGrid) -> Result<Response, MeasureError> {
        self.validate()?;
        let freqs = grid.freqs()?;
        let mag: Vec<f64> = freqs.iter().map(|&f| self.value_at(f)).collect();
        if mag.iter().any(|v| !v.is_finite()) {
            return Err(MeasureError::Invalid("target not finite".to_string()));
        }
        let n = mag.len();
        Ok(Response {
            grid: *grid,
            mag_db: mag,
            phase_deg: None,
            valid: Some(vec![true; n]),
            spl: false,
        })
    }
}

/// `10 log10(|num|² / |den|²)` for complex `num` and `den` given as (re, im).
fn ratio_db(num: (f64, f64), den: (f64, f64)) -> f64 {
    let n = num.0 * num.0 + num.1 * num.1;
    let d = den.0 * den.0 + den.1 * den.1;
    if n > 0.0 && d > 0.0 {
        10.0 * (n / d).log10()
    } else {
        0.0
    }
}

/// Analogue low shelf `A (s² + √A/Q s + A) / (A s² + √A/Q s + 1)` at `w = f/fc`.
fn low_shelf_db(s: Shelf, f: f64) -> f64 {
    let a = 10f64.powf(s.gain_db / 40.0);
    let w = f / s.corner_hz;
    let k = a.sqrt() / s.q * w;
    // 20 log10(A) + |(A - w²) + jk| / |(1 - A w²) + jk|
    20.0 * a.log10() + ratio_db((a - w * w, k), (1.0 - a * w * w, k))
}

/// Analogue high shelf `A (A s² + √A/Q s + 1) / (s² + √A/Q s + A)` at `w = f/fc`.
fn high_shelf_db(s: Shelf, f: f64) -> f64 {
    let a = 10f64.powf(s.gain_db / 40.0);
    let w = f / s.corner_hz;
    let k = a.sqrt() / s.q * w;
    20.0 * a.log10() + ratio_db((1.0 - a * w * w, k), (a - w * w, k))
}

/// Analogue peak `(s² + A/Q s + 1) / (s² + s/(A Q) + 1)` at `w = f/f0`.
fn peak_db(p: Peak, f: f64) -> f64 {
    let a = 10f64.powf(p.gain_db / 40.0);
    let w = f / p.center_hz;
    let re = 1.0 - w * w;
    ratio_db((re, a / p.q * w), (re, w / (a * p.q)))
}

/// A frequency band, Hz, for normalisation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceBand {
    /// Lower edge, Hz.
    pub low_hz: f64,
    /// Upper edge, Hz.
    pub high_hz: f64,
}

/// Shifts `resp` so that its mean over `band` (valid points only) is 0 dB;
/// returns the shift that was added, dB.
///
/// Errors: [`MeasureError::Invalid`] for a bad band or grid, or when no valid
/// point lies in the band.
pub fn normalise(resp: &mut Response, band: ReferenceBand) -> Result<f64, MeasureError> {
    if !(band.low_hz.is_finite()
        && band.high_hz.is_finite()
        && band.low_hz > 0.0
        && band.high_hz >= band.low_hz)
    {
        return Err(MeasureError::Invalid("bad reference band".to_string()));
    }
    let freqs = resp.grid.freqs()?;
    if freqs.len() != resp.mag_db.len() {
        return Err(MeasureError::Invalid(
            "response length does not match its grid".to_string(),
        ));
    }
    let mut sum = 0.0;
    let mut count = 0.0;
    for (i, (f, m)) in freqs.iter().zip(&resp.mag_db).enumerate() {
        let valid = resp
            .valid
            .as_ref()
            .is_none_or(|v| v.get(i).copied().unwrap_or(false));
        if valid && m.is_finite() && (band.low_hz..=band.high_hz).contains(f) {
            sum += m;
            count += 1.0;
        }
    }
    if count < 1.0 {
        return Err(MeasureError::Invalid(
            "no valid point in the reference band".to_string(),
        ));
    }
    let shift = -sum / count;
    if !shift.is_finite() {
        return Err(MeasureError::Invalid("values out of range".to_string()));
    }
    if resp.mag_db.iter().any(|m| !(m + shift).is_finite()) {
        return Err(MeasureError::Invalid("values out of range".to_string()));
    }
    for m in &mut resp.mag_db {
        *m += shift;
    }
    Ok(shift)
}

/// A target's definition: a built-in with parameters, or imported points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "source")]
pub enum TargetDefinition {
    /// A built-in.
    Builtin {
        /// Its parameters.
        builtin: Builtin,
    },
    /// Points imported from a REW or CSV file.
    Imported {
        /// Sorted, de-duplicated, in range.
        points: Vec<CurvePoint>,
    },
}

/// A target ready for a graph or a fit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    /// What it is for.
    pub target_use: TargetUse,
    /// The coupler it is for (headphones).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coupler: Option<Coupler>,
    /// The curve.
    pub definition: TargetDefinition,
    /// The band it is normalised at.
    pub reference: ReferenceBand,
}

impl Target {
    /// The target on `grid`, normalised at its reference band. Magnitude only;
    /// imported points outside the file's range are held and not valid.
    ///
    /// Errors: [`MeasureError::Invalid`] for a bad grid, parameter or band.
    pub fn evaluate(&self, grid: &LogGrid) -> Result<Response, MeasureError> {
        let mut r = match &self.definition {
            TargetDefinition::Builtin { builtin } => builtin.evaluate(grid)?,
            TargetDefinition::Imported { points } => {
                let mut r = curve_to_grid(points, grid)?;
                r.phase_deg = None;
                r
            }
        };
        normalise(&mut r, self.reference)?;
        Ok(r)
    }
}

/// Imports a target from REW or CSV text (the [`crate::cal`] parser's bounds
/// and errors). A phase column is ignored; a sensitivity line is ignored.
///
/// Errors: [`MeasureError::Parse`] as [`crate::cal::parse_curve`].
pub fn import_target(
    text: &str,
    target_use: TargetUse,
    coupler: Option<Coupler>,
) -> Result<Target, MeasureError> {
    let parsed = parse_curve(text, "")?;
    Ok(Target {
        target_use,
        coupler: if target_use == TargetUse::Headphones {
            coupler
        } else {
            None
        },
        definition: TargetDefinition::Imported {
            points: magnitude_only(&parsed),
        },
        reference: DEFAULT_REFERENCE_BAND,
    })
}

/// A coupler's compensation: the coupler's own deviation, imported, removed
/// from a headphone measurement with [`crate::cal::apply_correction`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Compensation {
    /// The coupler it is for.
    pub coupler: Coupler,
    /// Sorted, de-duplicated, in range; magnitude only.
    pub points: Vec<CurvePoint>,
}

impl Compensation {
    /// The compensation on `grid` (not normalised: it is a correction).
    ///
    /// Errors: [`MeasureError::Invalid`] for a bad grid.
    pub fn to_response(&self, grid: &LogGrid) -> Result<Response, MeasureError> {
        curve_to_grid(&self.points, grid)
    }
}

/// Imports a coupler compensation from REW or CSV text.
///
/// Errors: [`MeasureError::Parse`] as [`crate::cal::parse_curve`].
pub fn import_compensation(text: &str, coupler: Coupler) -> Result<Compensation, MeasureError> {
    let parsed = parse_curve(text, "")?;
    Ok(Compensation {
        coupler,
        points: magnitude_only(&parsed),
    })
}

fn magnitude_only(parsed: &ParsedCurve) -> Vec<CurvePoint> {
    parsed
        .points
        .iter()
        .map(|p| CurvePoint {
            phase_deg: None,
            ..*p
        })
        .collect()
}
