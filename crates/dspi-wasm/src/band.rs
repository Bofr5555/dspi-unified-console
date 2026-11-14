//! A whole PEQ band in one write (WP WB8): `DspiCore::set_filter`, the typed
//! setter the desktop's graph uses. The page sends a graph drag's bands this
//! way, so the core journals each tick as one change of the band (the one
//! field it moved, or the whole band) and joins a drag's ticks into one undo
//! step (`setDragging`), where field-by-field writes of `eq.freq` and
//! `eq.gain` would alternate paths and never join.
//!
//! `filter {channel, band, filterType, freq, q, gain, qp?, bypass?}`: `band`
//! is a PEQ band, 0..9 (crossover bands go through `crossoverType` and
//! `write`). Answers the band as the mirror holds it afterwards.

use dspi_core::DspiCore;
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};
use crate::curves::Band;

/// `filter`'s parameters.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterCall {
    /// The wire channel.
    pub channel: u8,
    /// The PEQ band, 0..9.
    pub band: u8,
    #[serde(flatten)]
    pub params: Band,
}

/// `filter`.
pub fn filter(core: &DspiCore, p: FilterCall) -> Result<Value, CallError> {
    core.set_filter(p.channel, p.band, p.params.into())?;
    let now = core
        .get_state()
        .filters
        .get(usize::from(p.channel))
        .and_then(|b| b.get(usize::from(p.band)))
        .copied()
        .ok_or_else(|| {
            CallError::new(
                "invalid",
                format!("no band {} on channel {}", p.band, p.channel),
            )
        })?;
    Ok(tidy(&now))
}

// -- WP PAIRBAND --
/// `inputPairFilter`'s parameters: `{input, band, filterType, freq, q, gain,
/// qp?, bypass?}`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairFilterCall {
    /// Either input of the linked pair (its partner is `input ^ 1`).
    pub input: u8,
    /// The PEQ band, 0..9.
    pub band: u8,
    #[serde(flatten)]
    pub params: Band,
}

// -- WP PAIRBAND --
/// `inputPairFilter`: a linked input pair's band, both sides, one undo step
/// (`DspiCore::set_input_pair_filter`); a drag's ticks too. Answers null.
pub fn input_pair_filter(core: &DspiCore, p: PairFilterCall) -> Result<Value, CallError> {
    core.set_input_pair_filter(p.input, p.band, p.params.into())?;
    Ok(Value::Null)
}

// -- WP PAIRBAND --
/// `inputPairBandBypass`'s parameters.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairBypassCall {
    /// Either input of the linked pair.
    pub input: u8,
    /// The PEQ band, 0..9.
    pub band: u8,
    /// Bypass (`true`) or re-enable.
    pub bypass: bool,
}

// -- WP PAIRBAND --
/// `inputPairBandBypass`: a linked input pair's band bypass, both sides, one
/// undo step (`DspiCore::set_input_pair_band_bypass`). Answers null.
pub fn input_pair_band_bypass(core: &DspiCore, p: PairBypassCall) -> Result<Value, CallError> {
    core.set_input_pair_band_bypass(p.input, p.band, p.bypass)?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests_wp_wb8 {
    use serde_json::{Value, json};

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

    /// A drag of a band through `filter` is one undo step.
    #[test]
    fn wp_wb8_a_band_drag_through_filter_is_one_step() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
        let band = |freq: f32, gain: f32| json!({ "channel": 2, "band": 0, "filterType": 1, "freq": freq, "q": 1.0, "gain": gain });
        let r = run(h, "filter", band(1000.0, 6.0));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["value"]["freq"], 1000.0);
        run(h, "setDragging", json!({ "dragging": true }));
        for i in 1..=8 {
            let f = 1000.0 + 125.0 * i as f32;
            let r = run(h, "filter", band(f, 6.0 - 0.5 * i as f32));
            assert_eq!(r["ok"], true, "{r}");
        }
        run(h, "setDragging", json!({ "dragging": false }));
        let r = run(h, "undo", Value::Null);
        assert_eq!(r["ok"], true, "{r}");
        let st = run(h, "state", Value::Null);
        let b = &st["value"]["state"]["filters"][2][0];
        assert_eq!(b["freq"], 1000.0, "{b}");
        assert_eq!(b["gain"], 6.0, "{b}");
        // A crossover band is not this call's.
        let r = run(
            h,
            "filter",
            json!({ "channel": 2, "band": 20, "filterType": 1, "freq": 100.0, "q": 1.0, "gain": 0.0 }),
        );
        assert_eq!(r["code"], "invalid", "{r}");
    }
}

// -- WP PAIRBAND --
#[cfg(test)]
mod tests_wp_pairband {
    use serde_json::{Value, json};

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

    /// A drag of a linked pair's band through `inputPairFilter` is one undo
    /// step that puts both sides back; the bypass is one step too.
    #[test]
    fn wp_pairband_a_linked_pair_band_through_the_worker_calls() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
        let band = |freq: f32, gain: f32| json!({ "input": 0, "band": 1, "filterType": 1, "freq": freq, "q": 1.0, "gain": gain });
        let r = run(h, "inputPairFilter", band(1000.0, 6.0));
        assert_eq!(r["ok"], true, "{r}");
        run(h, "setDragging", json!({ "dragging": true }));
        for i in 1..=6 {
            let r = run(
                h,
                "inputPairFilter",
                band(1000.0 + 100.0 * i as f32, 6.0 - i as f32),
            );
            assert_eq!(r["ok"], true, "{r}");
        }
        run(h, "setDragging", json!({ "dragging": false }));
        let st = run(h, "state", Value::Null);
        assert_eq!(st["value"]["state"]["filters"][1][1]["freq"], 1600.0);
        let r = run(h, "undo", Value::Null);
        assert_eq!(r["ok"], true, "{r}");
        let st = run(h, "state", Value::Null);
        for ch in 0..2 {
            let b = &st["value"]["state"]["filters"][ch][1];
            assert_eq!(b["freq"], 1000.0, "side {ch}: {b}");
            assert_eq!(b["gain"], 6.0, "side {ch}: {b}");
        }
        let r = run(
            h,
            "inputPairBandBypass",
            json!({ "input": 1, "band": 1, "bypass": true }),
        );
        assert_eq!(r["ok"], true, "{r}");
        run(h, "undo", Value::Null);
        let st = run(h, "state", Value::Null);
        for ch in 0..2 {
            assert_eq!(st["value"]["state"]["filters"][ch][1]["bypass"], false);
        }
        let r = run(
            h,
            "inputPairFilter",
            json!({ "input": 0, "band": 10, "filterType": 1, "freq": 100.0, "q": 1.0, "gain": 0.0 }),
        );
        assert_eq!(r["code"], "invalid", "{r}");
    }
}
