//! The audio link's calls (WP SW1, the fork's `extension_channel_spec.md`
//! 2.8): the unit's role and limits, its lock status, the slots it takes, the
//! phase-lock mode, the frame counter and apply-at-frame, over `DspiCore`'s
//! calls of the same names. Not DSPi Link ([`crate::link`]). A unit without
//! the link answers `unsupported` with nothing sent; `audioLinkInfo` answers
//! null.
//!
//! The demo opens as a unit receiving the link with `open`'s
//! `{"kind": "demo", "audioLink": true}`.

use dspi_core::DspiCore;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

/// `setAudioLinkSlots`'s parameters.
#[derive(Debug, Deserialize)]
pub struct SlotsParams {
    /// 8 values: a link channel 0..63, or 255 for silence.
    pub slots: Vec<u8>,
}

/// `setAudioLinkPhase`'s parameters.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseParams {
    /// 0 fill servo, 1 phase lock.
    pub mode: u8,
    /// The fixed delay, link frames.
    pub d_local: u16,
    /// A trim of -64..64; normally 0.
    #[serde(default)]
    pub hops_offset: i16,
}

/// `applyAtFrame`'s parameters.
#[derive(Debug, Deserialize)]
pub struct ApplyParams {
    /// The link frame.
    pub frame: u64,
    /// 0 cancel all .. 8 sync pulse.
    pub op: u8,
    #[serde(default)]
    pub arg: u8,
    #[serde(default)]
    pub value: i16,
}

fn parse<T: DeserializeOwned>(params: Value, what: &str) -> Result<T, CallError> {
    serde_json::from_value(params)
        .map_err(|e| CallError::new("invalid", format!("bad parameters for {what}: {e}")))
}

/// A record reply.
fn record<T: serde::Serialize>(r: dspi_core::Result<T>) -> CallResult {
    r.map(|v| tidy(&v)).map_err(Into::into)
}

/// A reply with nothing in it.
fn done(r: dspi_core::Result<()>) -> CallResult {
    r.map(|()| Value::Null).map_err(Into::into)
}

/// Dispatch one of this module's methods; `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, params: Value) -> Option<CallResult> {
    Some(match method {
        "audioLinkInfo" => record(core.audio_link_info()),
        "fetchAudioLinkInfo" => record(core.fetch_audio_link_info()),
        "audioLinkStatus" => record(core.fetch_audio_link_status()),
        "audioLinkSlots" => record(core.fetch_audio_link_slots()),
        "setAudioLinkSlots" => parse::<SlotsParams>(params, method)
            .and_then(|s| done(core.set_audio_link_slots(s.slots))),
        "audioLinkPhase" => record(core.fetch_audio_link_phase()),
        "setAudioLinkPhase" => parse::<PhaseParams>(params, method)
            .and_then(|x| done(core.set_audio_link_phase(x.mode, x.d_local, x.hops_offset))),
        "linkFrameCounter" => record(core.fetch_link_frame_counter()),
        "applyAtFrame" => parse::<ApplyParams>(params, method)
            .and_then(|a| done(core.apply_at_frame(a.frame, a.op, a.arg, a.value))),
        "applyAtFrameStatus" => record(core.fetch_apply_at_frame_status()),
        "resetAudioLinkStats" => done(core.reset_audio_link_stats()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_sw1 {
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
    fn wp_sw1_the_link_calls_on_the_demo_with_the_link() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo", "audioLink": true }));
        let info = ok(h, "audioLinkInfo", Value::Null);
        assert_eq!(info["role"], "receiver", "{info}");
        assert_eq!(info["dLocalMax"], 1024);
        assert_eq!(ok(h, "audioLinkStatus", Value::Null)["state"], "locked");
        ok(
            h,
            "setAudioLinkSlots",
            json!({ "slots": [6, 7, 255, 255, 255, 255, 255, 255] }),
        );
        assert_eq!(ok(h, "audioLinkSlots", Value::Null)[0], 6);
        ok(h, "setAudioLinkPhase", json!({ "mode": 1, "dLocal": 176 }));
        assert_eq!(ok(h, "audioLinkPhase", Value::Null)["dLocal"], 176);
        let f = ok(h, "linkFrameCounter", Value::Null)["frame"]
            .as_u64()
            .unwrap();
        ok(
            h,
            "applyAtFrame",
            json!({ "frame": f + 48_000_000, "op": 1 }),
        );
        assert_eq!(ok(h, "applyAtFrameStatus", Value::Null)["pending"], 1);
        ok(h, "resetAudioLinkStats", Value::Null);
        let bad = run(h, "applyAtFrame", json!({ "frame": 1, "op": 99 }));
        assert_eq!(bad["code"], "invalid", "{bad}");
        let bad = run(h, "setAudioLinkPhase", json!({ "mode": "x" }));
        assert_eq!(bad["code"], "invalid", "{bad}");
    }

    #[test]
    fn wp_sw1_the_plain_demo_has_no_link() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        assert_eq!(ok(h, "audioLinkInfo", Value::Null), Value::Null);
        let r = run(h, "audioLinkStatus", Value::Null);
        assert_eq!(r["code"], "unsupported", "{r}");
    }
}
