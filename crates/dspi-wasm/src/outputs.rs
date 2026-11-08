//! The outputs slice's calls (WP WB6c): what `write` by path does not cover
//! for the crossover, the matrix and the delay.
//!
//! - `crossoverType`: a crossover band's type (and, with it, its frequency).
//!   `eq.type`'s registry choices are the PEQ types 0..13 only, so `write`
//!   refuses a crossover code (32..63) on a crossover band (20..23). This is
//!   the core's typed setter for those bands (`DspiCore::set_crossover`, the
//!   one the Qt app calls), on the band as the mirror holds it. Frequency and
//!   bypass of a crossover band go through `write` (`eq.freq`, `eq.bypass`).
//! - `crosspoint`: one matrix crosspoint by its fields. It is `write` of the
//!   registry's `mix` (the core's `write_param`, read back), with the packet
//!   built here from the core's own `MatrixRoutePacket`, so the page never
//!   lays out wire bytes.
//! - `outputLimits`: the longest delay this device takes
//!   (`DspiCore::max_delay_ms`: 21 ms on RP2040, 42 ms on RP2350); the
//!   registry's `out.delay` range (0..85) is the wire's, not the device's.

use dspi_core::{DspiCore, FilterParams};
use dspi_proto::FilterType;
use dspi_proto::packets::MatrixRoutePacket;
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};
use crate::write::{self, ParamValue, WriteParams};

/// Crossover bands per output (wire bands 20..23).
const XOVER_BANDS: u8 = 4;

/// `crossoverType`'s parameters.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossoverParams {
    /// The output's wire channel.
    pub channel: u8,
    /// The crossover band, 0..3.
    pub band: u8,
    /// 0 (off) or a crossover type, 32..63.
    pub filter_type: u8,
    /// The corner, Hz; the band's own when absent.
    #[serde(default)]
    pub freq: Option<f32>,
}

/// `crossoverType`: the band with its new type (and frequency), as the
/// Console sends a crossover band: Q 0.707 when it has none, gain 0. The
/// answer is the band as the mirror holds it afterwards.
pub fn crossover_type(core: &DspiCore, p: CrossoverParams) -> Result<Value, CallError> {
    if p.band >= XOVER_BANDS {
        return Err(CallError::new(
            "invalid",
            format!("there is no crossover band {} (0..{XOVER_BANDS})", p.band),
        ));
    }
    let t = FilterType::from_raw(p.filter_type);
    if t != FilterType::Flat && !t.is_crossover() {
        return Err(CallError::new(
            "out_of_range",
            format!("{} is not a crossover type", t.display_name()),
        ));
    }
    let band = |core: &DspiCore| {
        core.get_state()
            .xover
            .get(usize::from(p.channel))
            .and_then(|b| b.get(usize::from(p.band)))
            .copied()
    };
    let now = band(core).ok_or_else(|| {
        CallError::new(
            "invalid",
            format!("channel {} has no crossover bands", p.channel),
        )
    })?;
    let next = FilterParams {
        filter_type: p.filter_type,
        freq: p.freq.unwrap_or(now.freq),
        q: if now.q > 0.0 { now.q } else { 0.707 },
        gain: 0.0,
        ..now
    };
    core.set_crossover(p.channel, p.band, next)?;
    let kept = band(core).unwrap_or(next);
    Ok(tidy(&kept))
}

/// `crosspoint`'s parameters (and its answer's fields).
#[derive(Debug, Deserialize)]
pub struct CrosspointParams {
    pub input: u8,
    pub output: u8,
    pub enabled: bool,
    /// dB; the core quantizes it to 0.1 dB.
    pub gain: f32,
    pub invert: bool,
}

/// `crosspoint`: `write` of `mix` [input, output], answered as the fields
/// the device kept.
pub fn crosspoint(core: &DspiCore, p: CrosspointParams) -> Result<Value, CallError> {
    let packet = MatrixRoutePacket {
        input: p.input,
        output: p.output,
        enabled: p.enabled,
        phase_invert: p.invert,
        gain_db: p.gain,
    };
    let w = WriteParams {
        path: "mix".into(),
        indices: vec![p.input, p.output],
        value: ParamValue::Bytes(packet.encode().to_vec()),
    };
    let back = match write::write(core, &w)? {
        ParamValue::Bytes(b) => MatrixRoutePacket::decode(&b)
            .map_err(|e| CallError::new("internal", format!("mix read back: {e}")))?,
        other => {
            return Err(CallError::new(
                "internal",
                format!("mix read back as {other:?}"),
            ));
        }
    };
    #[derive(serde::Serialize)]
    struct Kept {
        input: u8,
        output: u8,
        enabled: bool,
        gain: f32,
        invert: bool,
    }
    Ok(tidy(&Kept {
        input: back.input,
        output: back.output,
        enabled: back.enabled,
        gain: back.gain_db,
        invert: back.phase_invert,
    }))
}

/// `outputLimits`: `{maxDelayMs}`.
pub fn limits(core: &DspiCore) -> Value {
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Limits {
        max_delay_ms: f32,
    }
    tidy(&Limits {
        max_delay_ms: core.max_delay_ms(),
    })
}

#[cfg(test)]
mod tests_wb6c {
    use crate::calls::{call, session_new};
    use crate::guard;
    use serde_json::{Value, json};

    fn run(h: u32, method: &str, params: Value) -> Value {
        let out = call(
            h,
            json!({ "method": method, "params": params })
                .to_string()
                .as_bytes(),
        );
        serde_json::from_slice(&out).unwrap()
    }

    fn ok(h: u32, method: &str, params: Value) -> Value {
        let r = run(h, method, params);
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["value"].clone()
    }

    #[test]
    fn wb6c_crossover_types_crosspoints_and_the_delay_limit_on_the_demo() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        let st = ok(h, "open", json!({ "kind": "demo" }))["state"].clone();
        let out1 = st["numInputChannels"].as_u64().unwrap();

        // `write` refuses a crossover type: the registry's choices are the PEQ's.
        let refused = run(
            h,
            "write",
            json!({ "path": "eq.type", "indices": [out1, 20], "value": { "choice": 35 } }),
        );
        assert_eq!(refused["ok"], false, "{refused}");

        // crossoverType sets it, read back from the mirror; the frequency by `write`.
        let b = ok(
            h,
            "crossoverType",
            json!({ "channel": out1, "band": 1, "filterType": 35, "freq": 80.0 }),
        );
        assert_eq!(b["filterType"], 35, "{b}");
        assert_eq!(b["freq"], 80.0);
        assert_eq!(b["gain"], 0.0);
        ok(
            h,
            "write",
            json!({ "path": "eq.freq", "indices": [out1, 21], "value": { "float": 95.5 } }),
        );
        ok(
            h,
            "write",
            json!({ "path": "eq.bypass", "indices": [out1, 21], "value": { "bool": true } }),
        );
        let x = ok(h, "state", Value::Null)["state"]["xover"][out1 as usize][1].clone();
        assert_eq!(
            (
                x["filterType"].clone(),
                x["freq"].clone(),
                x["bypass"].clone()
            ),
            (json!(35), json!(95.5), json!(true)),
            "{x}"
        );
        // Off, a PEQ type, an input and a band past 3 are refused or cleared.
        assert_eq!(
            ok(
                h,
                "crossoverType",
                json!({ "channel": out1, "band": 1, "filterType": 0 })
            )["filterType"],
            0
        );
        for bad in [
            json!({ "channel": out1, "band": 0, "filterType": 1 }),
            json!({ "channel": out1, "band": 4, "filterType": 34 }),
            json!({ "channel": 0, "band": 0, "filterType": 34 }),
            json!({ "channel": 200, "band": 0, "filterType": 34 }),
        ] {
            let r = run(h, "crossoverType", bad.clone());
            assert_eq!(r["ok"], false, "{bad}: {r}");
        }

        // A crosspoint by its fields; the gain quantized to 0.1 dB.
        let c = ok(
            h,
            "crosspoint",
            json!({ "input": 1, "output": 2, "enabled": true, "gain": -3.04, "invert": true }),
        );
        assert_eq!(c["enabled"], true);
        assert_eq!(c["invert"], true);
        assert!((c["gain"].as_f64().unwrap() + 3.0).abs() < 1e-4, "{c}");
        let st = ok(h, "state", Value::Null)["state"].clone();
        assert_eq!(st["matrixRouting"][1][2], true);
        assert_eq!(st["matrixInvert"][1][2], true);
        let r = run(
            h,
            "crosspoint",
            json!({ "input": 1, "output": 99, "enabled": true, "gain": 0.0, "invert": false }),
        );
        assert_eq!(r["ok"], false, "{r}");

        // The device's delay limit, and a fractional delay kept exactly.
        let max = ok(h, "outputLimits", Value::Null)["maxDelayMs"]
            .as_f64()
            .unwrap();
        assert!(max == 21.0 || max == 42.0, "{max}");
        let d = ok(
            h,
            "write",
            json!({ "path": "out.delay", "indices": [0], "value": { "float": 12.3456 } }),
        );
        assert!((d["float"].as_f64().unwrap() - 12.3456).abs() < 1e-4, "{d}");
    }
}
