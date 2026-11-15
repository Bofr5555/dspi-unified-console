//! The channel clipboard on the web (WP CLIP1, PARITY M45), over
//! `DspiCore::copy_channel` and `DspiCore::paste_channel`.
//!
//! - `copyChannel {ch}`: wire channel `ch` (inputs, then outputs) from the
//!   mirror, nothing sent. Answers `{clip}`, the clip document
//!   (`dspi_core::channel_clip_to_json`: `{kind: "dspi.channelClip",
//!   version, sourceChannel, sourceName, filters, crossover, output}`), for
//!   the page to keep as it likes (in memory, `localStorage`).
//! - `pasteChannel {clip, ch, mirror?}`: put `clip` (that document, as an
//!   object or as its text) onto `ch`, and its PEQ onto the linked partner
//!   `mirror` when given, by the Console's rules; one undo step. Answers
//!   `null`. A refusal (code `invalid` for another clip version or a bad
//!   channel or value, `read_only`, `installer_lock`, `limit_exceeded`, ...)
//!   writes nothing.

use dspi_core::{ChannelPasteOptions, DspiCore, channel_clip_from_json, channel_clip_to_json};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::CallError;

type CallResult = Result<Value, CallError>;

#[derive(Deserialize)]
struct Copy {
    ch: u8,
}

#[derive(Deserialize)]
struct Paste {
    clip: Value,
    ch: u8,
    #[serde(default)]
    mirror: Option<u8>,
}

fn params<T: for<'de> Deserialize<'de>>(p: Value) -> Result<T, CallError> {
    serde_json::from_value(p).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

fn copy(core: &DspiCore, p: Copy) -> CallResult {
    let text = channel_clip_to_json(core.copy_channel(p.ch)?)?;
    let clip: Value = serde_json::from_str(&text)
        .map_err(|e| CallError::new("invalid", format!("the clip did not encode: {e}")))?;
    Ok(json!({ "clip": clip }))
}

fn paste(core: &DspiCore, p: Paste) -> CallResult {
    let text = match p.clip {
        Value::String(s) => s,
        other => other.to_string(),
    };
    let clip = channel_clip_from_json(text)?;
    core.paste_channel(clip, p.ch, ChannelPasteOptions { mirror: p.mirror })?;
    Ok(Value::Null)
}

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "copyChannel" => params(p).and_then(|c| copy(core, c)),
        "pasteChannel" => params(p).and_then(|c| paste(core, c)),
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_clip1 {
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

    fn gain(h: u32, o: usize) -> Value {
        run(h, "state", Value::Null)["value"]["state"]["outputGainDb"][o].clone()
    }

    #[test]
    fn wp_clip1_the_page_copies_pastes_and_undoes_in_one_step() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
        let st = run(h, "state", Value::Null);
        let ni = st["value"]["state"]["numInputChannels"].as_u64().unwrap();
        let r = run(
            h,
            "write",
            json!({ "path": "out.gain", "indices": [0], "value": { "float": -7.0 } }),
        );
        assert_eq!(r["ok"], true, "{r}");
        let before = gain(h, 1);

        let r = run(h, "copyChannel", json!({ "ch": ni }));
        let clip = r["value"]["clip"].clone();
        assert_eq!(clip["kind"], "dspi.channelClip", "{r}");
        assert_eq!(clip["version"], 1, "{r}");
        let r = run(h, "pasteChannel", json!({ "clip": clip, "ch": ni + 1 }));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(gain(h, 1), json!(-7.0));
        // As text, with a partner, too.
        let r = run(
            h,
            "pasteChannel",
            json!({ "clip": clip.to_string(), "ch": ni + 2, "mirror": ni + 3 }),
        );
        assert_eq!(r["ok"], true, "{r}");

        assert_eq!(run(h, "undo", Value::Null)["ok"], true);
        assert_eq!(run(h, "undo", Value::Null)["ok"], true);
        assert_eq!(gain(h, 1), before);

        let mut old = clip;
        old["version"] = 0.into();
        let r = run(h, "pasteChannel", json!({ "clip": old, "ch": ni + 1 }));
        assert_eq!(r["ok"], false, "{r}");
        assert_eq!(r["code"], "invalid", "{r}");
        let r = run(h, "copyChannel", json!({ "ch": 250 }));
        assert_eq!(r["code"], "invalid", "{r}");
    }
}
