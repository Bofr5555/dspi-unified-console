//! The speaker layout on the web (WP ROLE0, MPLAN section 2.2): the core's
//! `speaker_layout*` calls as JSON. The web has no files: a layout set lives
//! for the session's core only, so the page keeps its own copy and sets it
//! again after it opens the device. Shapes are the core's `SpeakerLayout`
//! (camelCase).
//!
//! - `speakerLayout`: the stored layout, or null.
//! - `speakerLayoutPropose`: what the filters and routing suggest.
//! - `speakerLayoutSet {layout}`: keep it (the user's screens only); the
//!   layout kept.

use dspi_core::{DspiCore, SpeakerLayout};
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

/// `speakerLayoutSet`'s parameters.
#[derive(Debug, Deserialize)]
struct Set {
    layout: SpeakerLayout,
}

fn reply<T: serde::Serialize>(r: dspi_core::Result<T>) -> CallResult {
    r.map(|v| tidy(&v)).map_err(CallError::from)
}

/// The speaker layout's calls; `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "speakerLayout" => reply(core.speaker_layout()),
        "speakerLayoutPropose" => reply(core.speaker_layout_propose()),
        "speakerLayoutSet" => serde_json::from_value::<Set>(p)
            .map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
            .and_then(|q| reply(core.speaker_layout_set(q.layout))),
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_role0 {
    use serde_json::{Value, json};

    use crate::calls::{call, session_free, session_new};
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

    fn ok(h: u32, method: &str, params: Value) -> Value {
        let r = run(h, method, params);
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["value"].clone()
    }

    #[test]
    fn wp_role0_the_speaker_layout_through_the_calls() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        // -- WP UB12 -- the demo comes with its example's roles confirmed
        assert_eq!(ok(h, "speakerLayout", Value::Null)["confirmedByUser"], true);
        let mut p = ok(h, "speakerLayoutPropose", Value::Null);
        assert_eq!(p["confirmedByUser"], false, "{p}");
        p["confirmedByUser"] = Value::Bool(true);
        let k = ok(h, "speakerLayoutSet", json!({ "layout": p }));
        assert_eq!(k["confirmedByUser"], true, "{k}");
        assert_eq!(ok(h, "speakerLayout", Value::Null)["confirmedByUser"], true);
        let r = run(h, "speakerLayoutSet", json!({ "layout": 3 }));
        assert_eq!(r["ok"], false, "{r}");
        ok(h, "close", Value::Null);
        session_free(h).unwrap();
    }
}
