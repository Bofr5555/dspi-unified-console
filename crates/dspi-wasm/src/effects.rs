//! WP WB6h: what the web's effect screens read that `state` and `describe`
//! do not carry.
//!
//! `features {names}`: whether the connect probe found each named feature
//! (`DspiCore::feature_present`: `psychoacoustic_bass`, `upmixer`,
//! `subharmonic_synth`, `tube_preamp`, `loudness_output_mask`, ...), so the
//! page gates an effect by the core's feature names, never by a firmware
//! version. False for every name with no device.
//!
//! `effectChoices`: the words for the effects' choice parameters, by
//! registry path. The registry's own choice names are command keys
//! (`chumoy`, `solid-state`); these are the Console's: the crossfeed voicing
//! and leveller speed from `dspi_proto::enums`, the tube types and
//! rectifiers from `dspi_session::tube` (type 0 is Custom; the core loads a
//! type's row on the write), the upmixer's modes and the subharmonic
//! selectivity as the control-surface model names them
//! (`dspi_session::cs_model::enum_value_label`).

use dspi_core::DspiCore;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::CallError;

/// The most names one `features` call takes (the probe knows about 25).
const MAX_NAMES: usize = 64;

/// `features`' parameters.
#[derive(Debug, Deserialize)]
pub struct FeaturesParams {
    pub names: Vec<String>,
}

/// `features`: `{name: present}` for each name asked.
pub fn features(core: &DspiCore, p: FeaturesParams) -> Result<Value, CallError> {
    if p.names.len() > MAX_NAMES {
        return Err(CallError::new(
            "invalid",
            format!("at most {MAX_NAMES} feature names at once"),
        ));
    }
    let mut out = serde_json::Map::new();
    for name in p.names {
        let present = core.feature_present(&name);
        out.insert(name, Value::Bool(present));
    }
    Ok(Value::Object(out))
}

/// One choice: its wire value, its name, and a line about it ("" for none).
fn choice(value: u8, label: &str, detail: &str) -> Value {
    json!({ "value": value, "label": label, "detail": detail })
}

/// A label "Name, detail" split at its first ", ".
fn split(label: &str) -> (&str, &str) {
    label.split_once(", ").unwrap_or((label, ""))
}

/// `effectChoices`: `{path: [{value, label, detail}]}`.
pub fn effect_choices() -> Value {
    use dspi_proto::enums::{CrossfeedPreset, LevellerSpeed};
    let voicing: Vec<Value> = (0..=3u8)
        .map(|v| {
            let l = CrossfeedPreset::from_raw(v).label();
            let (name, detail) = split(&l);
            choice(v, name, detail)
        })
        .collect();
    let speed: Vec<Value> = (0..=2u8)
        .map(|v| choice(v, &LevellerSpeed::from_raw(v).label(), ""))
        .collect();
    let mut tube = vec![choice(0, &dspi_session::tube::type_name(0), "")];
    for (v, t) in (1u8..).zip(dspi_session::tube::TYPES.iter()) {
        tube.push(choice(v, t.name, t.style));
    }
    let rect: Vec<Value> = (0u8..)
        .zip(dspi_session::tube::RECTIFIERS.iter())
        .map(|(v, r)| choice(v, r.name, ""))
        .collect();
    let list = |names: &[&str]| -> Vec<Value> {
        (0u8..)
            .zip(names.iter())
            .map(|(v, n)| choice(v, n, ""))
            .collect()
    };
    json!({
        "cf.preset": voicing,
        "lev.speed": speed,
        // Caps v5 appended Off as the centre's value 2; the surround puts Off first.
        "up.center_mode": list(&["Sinner", "Logician", "Off"]),
        "up.surround_mode": list(&["Off", "Sinner", "Logician"]),
        "sub.select": list(&["All material", "Percussive", "Sustained"]),
        "tube.type": tube,
        "tube.rectifier": rect,
    })
}

#[cfg(test)]
mod tests_wp_wb6h {
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

    const EFFECTS: [&str; 7] = [
        "psychoacoustic_bass",
        "upmixer",
        "subharmonic_synth",
        "tube_preamp",
        "loudness_output_mask",
        "crossfeed_output_mask",
        "leveller_masks",
    ];

    #[test]
    fn wb6h_features_none_without_a_device_all_on_the_demo() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        let none = ok(h, "features", json!({ "names": EFFECTS }));
        for n in EFFECTS {
            assert_eq!(none[n], false, "{n} with no device");
        }
        ok(h, "open", json!({ "kind": "demo" }));
        let demo = ok(h, "features", json!({ "names": EFFECTS }));
        for n in EFFECTS {
            assert_eq!(demo[n], true, "{n} on the demo");
        }
        let odd = ok(h, "features", json!({ "names": ["no_such_feature"] }));
        assert_eq!(odd["no_such_feature"], false);
        // Too many names at once is refused, not run.
        let many: Vec<String> = (0..65).map(|i| format!("f{i}")).collect();
        assert_eq!(
            run(h, "features", json!({ "names": many }))["code"],
            "invalid"
        );
        assert_eq!(run(h, "features", json!({}))["code"], "invalid");
    }

    #[test]
    fn wb6h_effect_choices_in_the_consoles_words() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        let c = ok(h, "effectChoices", Value::Null);
        let cf = c["cf.preset"].as_array().unwrap();
        assert_eq!(cf.len(), 4);
        assert_eq!(cf[1]["label"], "Chu Moy");
        assert_eq!(cf[1]["detail"], "700 Hz / 6.0 dB");
        assert_eq!(cf[3]["label"], "Custom");
        assert_eq!(c["lev.speed"][2]["label"], "Fast");
        assert_eq!(c["up.center_mode"][2]["label"], "Off");
        assert_eq!(c["up.surround_mode"][0]["label"], "Off");
        assert_eq!(c["sub.select"][1]["label"], "Percussive");
        let tube = c["tube.type"].as_array().unwrap();
        assert_eq!(tube.len(), 17, "Custom and 16 types");
        assert_eq!(tube[0]["value"], 0);
        assert_eq!(tube[0]["label"], "Custom");
        assert_eq!(tube[1]["label"], "12AX7 / ECC83");
        assert!(
            tube.iter()
                .skip(1)
                .all(|r| !r["detail"].as_str().unwrap().is_empty())
        );
        assert_eq!(c["tube.rectifier"].as_array().unwrap().len(), 4);
        assert_eq!(c["tube.rectifier"][0]["label"], "Solid state");
        // Every path names a registry parameter whose choices it covers.
        for (path, rows) in c.as_object().unwrap() {
            let d = ok(h, "describe", json!({ "path": path }));
            let n = rows.as_array().unwrap().len() as u64;
            match d["kind"].as_str().unwrap() {
                "choice" => assert_eq!(d["choices"].as_array().unwrap().len() as u64, n, "{path}"),
                "int" => assert_eq!(d["max"].as_u64().unwrap() + 1, n, "{path}"),
                k => panic!("{path} is a {k}"),
            }
        }
    }
}
