//! The response curves the EQ graph draws (WP WB6b), all the core's
//! (`dspi_core::compute_magnitude_curve`, `compute_response`, `curve_freq`):
//! the page does no filter maths of its own. Nothing here touches a device,
//! so the page may ask for curves of bands it has not written yet (a drag in
//! progress).
//!
//! - `curveGrid`: the frequency of every curve point (201, log-spaced
//!   10 Hz..20 kHz), once per session.
//! - `curves`: for each chain (a channel's PEQ and crossover bands) its
//!   combined magnitude in dB on that grid; for each band of the edited
//!   channel its own curve with its bypass ignored (for its fill and the
//!   graph's hit tests), its level at its own frequency, and for the types
//!   with gain the level per dB of gain there (where its dot sits).
//!   -- WP GAP1 -- With `phase`, the first chain's (the edited channel's)
//!   phase in degrees too (`unwrap`: continuous, else within +-180), and
//!   with `groupDelay` its group delay in ms: `dspi_core::compute_phase_curve`
//!   and `compute_group_delay_curve`, the numbers Qt's graph draws.

use dspi_core::{
    CURVE_POINTS, FilterParams, compute_group_delay_curve, compute_magnitude_curve,
    compute_phase_curve, compute_response, curve_freq,
};
use dspi_proto::FilterType;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::{CallError, tidy};

/// More than any channel has (10 PEQ and 4 crossover bands) or a page shows.
const MAX_BANDS: usize = 32;
const MAX_CHAINS: usize = 32;

/// A band as the page sends it: `FilterParams` in camelCase, as the state
/// carries it.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Band {
    pub filter_type: u8,
    #[serde(default)]
    pub bypass: bool,
    pub freq: f32,
    pub q: f32,
    pub gain: f32,
    #[serde(default)]
    pub qp: f32,
}

impl From<Band> for FilterParams {
    fn from(b: Band) -> Self {
        FilterParams {
            filter_type: b.filter_type,
            bypass: b.bypass,
            freq: b.freq,
            q: b.q,
            gain: b.gain,
            qp: b.qp,
        }
    }
}

/// `curves`' parameters.
#[derive(Debug, Default, Deserialize)]
pub struct CurvesParams {
    #[serde(default)]
    pub chains: Vec<Vec<Band>>,
    #[serde(default)]
    pub bands: Vec<Band>,
    // -- WP GAP1 --
    /// Add the first chain's phase.
    #[serde(default)]
    pub phase: bool,
    /// That phase continuous rather than within +-180.
    #[serde(default)]
    pub unwrap: bool,
    /// Add the first chain's group delay.
    #[serde(default, rename = "groupDelay")]
    pub group_delay: bool,
}

/// `curveGrid`: the frequency (Hz) of each curve point.
pub fn grid() -> Value {
    let freqs: Vec<f64> = (0..CURVE_POINTS)
        .map(|i| curve_freq(u32::try_from(i).unwrap_or(u32::MAX)))
        .collect();
    json!({ "freqs": freqs })
}

// -- WP CG34 --
/// `filterTypeNames`: [{code, name}] for every type the firmware defines
/// (`dspi_core::filter_type_names`, the Console's band-list names).
pub fn type_names() -> Value {
    json!(dspi_core::filter_type_names())
}

/// `curves`: see the module's text.
pub fn curves(p: CurvesParams) -> Result<Value, CallError> {
    if p.chains.len() > MAX_CHAINS
        || p.bands.len() > MAX_BANDS
        || p.chains.iter().any(|c| c.len() > MAX_BANDS)
    {
        return Err(CallError::new(
            "invalid",
            format!("at most {MAX_CHAINS} chains of {MAX_BANDS} bands"),
        ));
    }
    // -- WP GAP1 -- the edited channel's phase and group delay (an empty
    // chain, or none, is flat: zeros)
    let first: Vec<FilterParams> = p
        .chains
        .first()
        .map(|c| c.iter().copied().map(FilterParams::from).collect())
        .unwrap_or_default();
    let chains: Vec<Vec<f64>> = p
        .chains
        .into_iter()
        .map(|c| compute_magnitude_curve(c.into_iter().map(FilterParams::from).collect()))
        .collect();
    let bands: Vec<Value> = p.bands.into_iter().map(band).collect();
    let mut out = serde_json::Map::new();
    out.insert("chains".into(), json!(round(&chains)));
    out.insert("bands".into(), json!(bands));
    if p.phase {
        let phase = compute_phase_curve(first.clone(), p.unwrap);
        out.insert(
            "phase".into(),
            json!(round(&[phase]).pop().unwrap_or_default()),
        );
    }
    if p.group_delay {
        // To a tenth of a microsecond
        let gd: Vec<f64> = compute_group_delay_curve(first)
            .iter()
            .map(|v| (v * 10_000.0).round() / 10_000.0)
            .collect();
        out.insert("groupDelay".into(), json!(gd));
    }
    Ok(Value::Object(out))
}

/// One band of the edited channel, its bypass ignored: an off band has no
/// curve.
fn band(b: Band) -> Value {
    let ty = FilterType::from_raw(b.filter_type);
    let mut p = FilterParams::from(b);
    p.bypass = false;
    if b.filter_type == 0 {
        return json!({ "curve": [], "at": 0.0, "unit": 0.0, "usesGain": false, "usesQ": false });
    }
    let curve = compute_magnitude_curve(vec![p]);
    let at = compute_response(vec![p], p.freq);
    // Where a dot with gain sits: the band's level at its own frequency per
    // dB of gain (one for a bell, about a half for a shelf), measured.
    let unit = if ty.uses_gain() {
        let mut probe = p;
        probe.gain = 12.0;
        compute_response(vec![probe], p.freq) / 12.0
    } else {
        0.0
    };
    json!({
        "curve": round(&[curve]).pop().unwrap_or_default(),
        "at": tidy(&at),
        "unit": tidy(&unit),
        "usesGain": ty.uses_gain(),
        "usesQ": ty.uses_q(),
    })
}

/// Curves to a thousandth of a dB: what a graph can show, and a third of
/// the JSON.
fn round(curves: &[Vec<f64>]) -> Vec<Vec<f64>> {
    curves
        .iter()
        .map(|c| c.iter().map(|v| (v * 1000.0).round() / 1000.0).collect())
        .collect()
}

#[cfg(test)]
mod tests_wp_wb6b {
    use super::*;
    use crate::calls::{call, session_new};
    use crate::guard;

    fn run(h: u32, method: &str, params: Value) -> Value {
        let out = call(
            h,
            json!({ "method": method, "params": params })
                .to_string()
                .as_bytes(),
        );
        serde_json::from_slice(&out).unwrap()
    }

    fn bell(freq: f32, gain: f32, q: f32) -> Value {
        json!({ "filterType": 1, "bypass": false, "freq": freq, "q": q, "gain": gain, "qp": 0.0 })
    }

    #[test]
    fn wb6b_curves_are_the_cores_on_its_grid() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        // No device needed: the curves are the core's maths only.
        let g = run(h, "curveGrid", Value::Null);
        assert_eq!(g["ok"], true, "{g}");
        let freqs = g["value"]["freqs"].as_array().unwrap();
        assert_eq!(freqs.len(), 201);
        assert!((freqs[0].as_f64().unwrap() - 10.0).abs() < 1e-9);
        assert!((freqs[200].as_f64().unwrap() - 20_000.0).abs() < 1e-6);

        let r = run(
            h,
            "curves",
            json!({
                "chains": [[bell(1000.0, 6.0, 1.0)], []],
                "bands": [bell(1000.0, 6.0, 1.0), { "filterType": 0, "freq": 1000.0, "q": 0.7, "gain": 0.0 },
                          { "filterType": 2, "bypass": true, "freq": 100.0, "q": 0.707, "gain": 6.0 },
                          { "filterType": 4, "freq": 2000.0, "q": 2.0, "gain": 0.0 }],
            }),
        );
        assert_eq!(r["ok"], true, "{r}");
        let v = &r["value"];
        let chain = v["chains"][0].as_array().unwrap();
        assert_eq!(chain.len(), 201);
        // The same numbers the core gives every other client.
        let core = compute_magnitude_curve(vec![FilterParams {
            filter_type: 1,
            bypass: false,
            freq: 1000.0,
            q: 1.0,
            gain: 6.0,
            qp: 0.0,
        }]);
        for (a, b) in chain.iter().zip(core.iter()) {
            assert!((a.as_f64().unwrap() - b).abs() <= 0.0005);
        }
        // An empty chain is flat.
        assert!(
            v["chains"][1]
                .as_array()
                .unwrap()
                .iter()
                .all(|x| x.as_f64() == Some(0.0))
        );
        // The bell sits at its gain; an off band has no curve.
        assert!((v["bands"][0]["at"].as_f64().unwrap() - 6.0).abs() < 0.01);
        assert!((v["bands"][0]["unit"].as_f64().unwrap() - 1.0).abs() < 0.01);
        assert_eq!(v["bands"][0]["usesGain"], true);
        assert_eq!(v["bands"][1]["curve"].as_array().unwrap().len(), 0);
        // A bypassed shelf still has its own curve (the graph shows its
        // shape), with its dot at about half its gain.
        let shelf = &v["bands"][2];
        assert_eq!(shelf["curve"].as_array().unwrap().len(), 201);
        let unit = shelf["unit"].as_f64().unwrap();
        assert!(unit > 0.4 && unit < 0.6, "{unit}");
        // A second-order cut: Q, no gain; its level at the corner is 20 log10 Q.
        let cut = &v["bands"][3];
        assert_eq!(cut["usesGain"], false);
        assert_eq!(cut["usesQ"], true);
        assert!(
            (cut["at"].as_f64().unwrap() - 20.0 * 2f64.log10()).abs() < 0.2,
            "{cut}"
        );

        let many: Vec<Value> = (0..40).map(|_| bell(100.0, 1.0, 1.0)).collect();
        let r = run(h, "curves", json!({ "bands": many }));
        assert_eq!(r["code"], "invalid", "{r}");
    }

    // -- WP CG34 --
    #[test]
    fn wp_cg34_the_page_reads_the_cores_type_names() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        let r = run(h, "filterTypeNames", Value::Null);
        assert_eq!(r["ok"], true, "{r}");
        let t = r["value"].as_array().unwrap();
        assert_eq!(t.len(), 46);
        assert_eq!(t[0], json!({ "code": 0, "name": "Off" }));
        assert_eq!(t[12], json!({ "code": 12, "name": "High Cut (6dB)" }));
        assert_eq!(t[14], json!({ "code": 32, "name": "LR2 Low pass" })); // -- INT016 -- (A10)
    }
}

// -- WP GAP1 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#[path = "curves/tests_wp_gap1.rs"]
mod tests_wp_gap1;
