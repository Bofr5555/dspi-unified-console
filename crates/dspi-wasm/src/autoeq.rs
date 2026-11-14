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
#[derive(Debug, Deserialize)]
pub struct PairParams {
    pub profile: Value,
    pub left: u8,
    pub right: u8,
}

fn profile(p: &PairParams) -> Result<AutoEqProfile, CallError> {
    Ok(AutoEqProfile::from_entry_json(&p.profile.to_string())?)
}

/// `autoeqPreview`: what the profile would do (`DspiCore::autoeq_preview`):
/// `{profileId, channels: [{channel, isOutput, levelDb, bands, crossover,
/// curve}], levelTarget, held, unsupported, dropped, notes}`. Sends nothing.
pub fn preview(core: &DspiCore, p: PairParams) -> Result<Value, CallError> {
    let profile = profile(&p)?;
    Ok(tidy(&core.autoeq_preview(profile, p.left, p.right)?))
}

/// `autoeqApply`: apply it (`DspiCore::autoeq_apply`): `{preview,
/// snapshotDocument, snapshotId, notes}`.
pub fn apply(core: &DspiCore, p: PairParams) -> Result<Value, CallError> {
    let profile = profile(&p)?;
    Ok(tidy(&core.autoeq_apply(profile, p.left, p.right)?))
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
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
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

        let bad = run(
            h,
            "autoeqPreview",
            json!({ "profile": profile, "left": 0, "right": 0 }),
        );
        assert_eq!(bad["ok"], false, "{bad}");
        assert_eq!(bad["code"], "invalid", "{bad}");
    }
}
