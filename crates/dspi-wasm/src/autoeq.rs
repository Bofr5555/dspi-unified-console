//! AutoEQ preview and apply on the web (WP P1AQ, PRO1 T6). The page holds
//! the database; a call names one entry of it, as the database has it, and
//! the pair of wire channels. No snapshot store on the web: the apply's
//! reply carries the snapshot of the state before as a document
//! (`snapshotDocument`), to keep or download and to recall with
//! `snapshotRecall`.

use dspi_core::{AutoEqProfile, DspiCore};
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

/// `autoeqPreview`'s and `autoeqApply`'s parameters: `profile` one entry of
/// the AutoEQ database (`{id, manufacturer, model, preamp, filters: [{type,
/// freq, q, gain}]}`), `left` and `right` the pair's wire channels.
/// `applyLevel` (optional): whether the profile's preamp is written; absent
/// is the default (yes on inputs, no on outputs, whose calibrated gain is
/// kept and the preamp only suggested as `suggestedTrimDb`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairParams {
    pub profile: Value,
    pub left: u8,
    pub right: u8,
    #[serde(default)]
    pub apply_level: Option<bool>,
}

fn profile(p: &PairParams) -> Result<AutoEqProfile, CallError> {
    Ok(AutoEqProfile::from_entry_json(&p.profile.to_string())?)
}

/// `autoeqPreview`: what the profile would do (`DspiCore::autoeq_preview`):
/// `{profileId, channels: [{channel, isOutput, levelDb, bands, crossover,
/// curve}], levelTarget, suggestedTrimDb, held, unsupported, dropped, notes}`.
/// Sends nothing.
pub fn preview(core: &DspiCore, p: PairParams) -> Result<Value, CallError> {
    let profile = profile(&p)?;
    Ok(tidy(&core.autoeq_preview(
        profile,
        p.left,
        p.right,
        p.apply_level,
    )?))
}

/// `autoeqApply`: apply it (`DspiCore::autoeq_apply`): `{preview,
/// snapshotDocument, snapshotId, notes}`.
pub fn apply(core: &DspiCore, p: PairParams) -> Result<Value, CallError> {
    let profile = profile(&p)?;
    Ok(tidy(&core.autoeq_apply(
        profile,
        p.left,
        p.right,
        p.apply_level,
    )?))
}

#[cfg(test)]
mod tests_wp_p1aq {
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

    #[test]
    fn wp_p1aq_autoeq_on_the_web() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        let opened = run(h, "open", json!({ "kind": "demo" }));
        assert_eq!(opened["ok"], true);
        let first_out = opened["value"]["state"]["numInputChannels"]
            .as_u64()
            .unwrap();
        let profile = json!({
            "id": "o/Sony A", "manufacturer": "Sony", "model": "A", "preamp": -4.0,
            "filters": [{"type": "PK", "freq": 105, "q": 0.7, "gain": 3}]
        });
        let pair = json!({ "profile": profile, "left": 0, "right": 1 });
        let pv = run(h, "autoeqPreview", pair.clone());
        assert_eq!(pv["ok"], true, "{pv}");
        assert_eq!(pv["value"]["levelTarget"], "input_preamp", "{pv}");
        assert_eq!(pv["value"]["channels"][1]["bands"][0]["gain"], 3.0, "{pv}");

        let done = run(h, "autoeqApply", pair);
        assert_eq!(done["ok"], true, "{done}");
        let doc = done["value"]["snapshotDocument"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(done["value"]["snapshotId"], "");
        let back = run(h, "snapshotRecall", json!({ "text": doc }));
        assert_eq!(back["ok"], true, "{back}");

        // An output pair (the demo's outputs 3 and 4) keeps its gain unless
        // asked.
        let out = first_out + 2;
        let kept = run(
            h,
            "autoeqPreview",
            json!({ "profile": profile, "left": out, "right": out + 1 }),
        );
        assert_eq!(kept["value"]["levelTarget"], "none", "{kept}");
        assert_eq!(kept["value"]["suggestedTrimDb"], -4.0, "{kept}");
        let asked = run(
            h,
            "autoeqPreview",
            json!({ "profile": profile, "left": out, "right": out + 1, "applyLevel": true }),
        );
        assert_eq!(asked["value"]["levelTarget"], "output_gain", "{asked}");

        let bad = run(
            h,
            "autoeqPreview",
            json!({ "profile": profile, "left": 0, "right": 0 }),
        );
        assert_eq!(bad["ok"], false, "{bad}");
        assert_eq!(bad["code"], "invalid", "{bad}");
    }
}
