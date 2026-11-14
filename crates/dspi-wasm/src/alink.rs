//! The audio link's calls (WP SW1, the fork's `extension_channel_spec.md`
//! 2.8): the unit's role and limits, its lock status, the slots it takes, the
//! phase-lock mode, the frame counter and apply-at-frame, over `DspiCore`'s
//! calls of the same names. Not DSPi Link ([`crate::link`]). A unit without
//! the alink answers `unsupported` with nothing sent; `alinkInfo` answers
//! null.
//!
//! The demo opens as a unit receiving the alink with `open`'s
//! `{"kind": "demo", "alink": true}`.

use dspi_core::DspiCore;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

/// `setAlinkSlots`'s parameters.
#[derive(Debug, Deserialize)]
pub struct SlotsParams {
    /// 8 values: an audio link channel 0..63, or 255 for silence.
    pub slots: Vec<u8>,
}

/// `setAlinkPhase`'s parameters.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseParams {
    /// 0 fill servo, 1 phase lock.
    pub mode: u8,
    /// The fixed delay, alink frames.
    pub d_local: u16,
    /// A trim of -64..64; normally 0.
    #[serde(default)]
    pub hops_offset: i16,
}

/// `applyAtFrame`'s parameters.
#[derive(Debug, Deserialize)]
pub struct ApplyParams {
    /// The alink frame.
    pub frame: u64,
    /// 0 cancel all .. 8 sync pulse.
    pub op: u8,
    #[serde(default)]
    pub arg: u8,
    #[serde(default)]
    pub value: i16,
}

// -- WP AUSB2 --
/// `setAlinkUsbOnLockError`'s parameters.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockErrorParams {
    /// True: play unaligned in lock error; false: mute.
    pub play_unaligned: bool,
}

// -- WP AUSB1 --
/// `setAlinkUsb`'s parameters.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbParams {
    /// 0 off, 1 rate lock, 2 phase lock.
    pub mode: u8,
    /// Re-anchor on a phase error beyond 2 samples (the default).
    #[serde(default = "yes")]
    pub auto_reanchor: bool,
    /// D, samples.
    pub d: u16,
}

fn yes() -> bool {
    true
}

// -- WP AUSB1 --
/// `applyAtTimeBase`'s parameters.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeBaseApplyParams {
    /// 0 the alink frame, 1 the USB sample index, 2 a bus frame.
    pub time_base: u8,
    /// The frame, sample or bus-frame target.
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
        "alinkInfo" => record(core.alink_info()),
        "fetchAlinkInfo" => record(core.fetch_alink_info()),
        "alinkStatus" => record(core.fetch_alink_status()),
        "alinkSlots" => record(core.fetch_alink_slots()),
        "setAlinkSlots" => {
            parse::<SlotsParams>(params, method).and_then(|s| done(core.set_alink_slots(s.slots)))
        }
        "alinkPhase" => record(core.fetch_alink_phase()),
        "setAlinkPhase" => parse::<PhaseParams>(params, method)
            .and_then(|x| done(core.set_alink_phase(x.mode, x.d_local, x.hops_offset))),
        "alinkFrameCounter" => record(core.fetch_alink_frame_counter()),
        "applyAtFrame" => parse::<ApplyParams>(params, method)
            .and_then(|a| done(core.apply_at_frame(a.frame, a.op, a.arg, a.value))),
        "applyAtFrameStatus" => record(core.fetch_apply_at_frame_status()),
        "resetAlinkStats" => done(core.reset_alink_stats()),
        // -- WP AUSB1 -- the USB-direct block (extension_channel_spec.md 2.9)
        "usbDirectSupported" => record(core.usb_direct_supported()),
        "alinkUsb" => record(core.fetch_alink_usb()),
        "setAlinkUsb" => parse::<UsbParams>(params, method)
            .and_then(|x| done(core.set_alink_usb(x.mode, x.auto_reanchor, x.d))),
        // -- WP AUSB2 --
        "setAlinkUsbOnLockError" => parse::<LockErrorParams>(params, method)
            .and_then(|x| done(core.set_alink_usb_on_lock_error(x.play_unaligned))),
        "alinkUsbStatus" => record(core.fetch_alink_usb_status()),
        "alinkUsbDiag" => record(core.fetch_alink_usb_diag()),
        "clearAlinkUsbDiag" => done(core.clear_alink_usb_diag()),
        "frameCounterTimeBase" => record(core.fetch_frame_counter_time_base()),
        "applyAtTimeBase" => parse::<TimeBaseApplyParams>(params, method).and_then(|a| {
            done(core.apply_at_time_base(a.time_base, a.frame, a.op, a.arg, a.value))
        }),
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
        ok(h, "open", json!({ "kind": "demo", "alink": true }));
        let info = ok(h, "alinkInfo", Value::Null);
        assert_eq!(info["role"], "receiver", "{info}");
        // -- fixup wb8 -- FIXS1: the default 4 ms option's range, 257..399
        assert_eq!(info["dLocalMin"], 257);
        assert_eq!(info["dLocalMax"], 399);
        assert_eq!(ok(h, "alinkStatus", Value::Null)["state"], "locked");
        ok(
            h,
            "setAlinkSlots",
            json!({ "slots": [6, 7, 255, 255, 255, 255, 255, 255] }),
        );
        assert_eq!(ok(h, "alinkSlots", Value::Null)[0], 6);
        ok(h, "setAlinkPhase", json!({ "mode": 1, "dLocal": 320 }));
        assert_eq!(ok(h, "alinkPhase", Value::Null)["dLocal"], 320);
        let f = ok(h, "alinkFrameCounter", Value::Null)["frame"]
            .as_u64()
            .unwrap();
        ok(
            h,
            "applyAtFrame",
            json!({ "frame": f + 48_000_000, "op": 1 }),
        );
        assert_eq!(ok(h, "applyAtFrameStatus", Value::Null)["pending"], 1);
        ok(h, "resetAlinkStats", Value::Null);
        let bad = run(h, "applyAtFrame", json!({ "frame": 1, "op": 99 }));
        assert_eq!(bad["code"], "invalid", "{bad}");
        let bad = run(h, "setAlinkPhase", json!({ "mode": "x" }));
        assert_eq!(bad["code"], "invalid", "{bad}");
    }

    // -- WP AUSB1 --
    #[test]
    fn wp_ausb1_the_usb_direct_calls_on_the_usb_demo() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo", "usbDirect": true }));
        assert_eq!(ok(h, "usbDirectSupported", Value::Null), true);
        assert_eq!(ok(h, "alinkUsb", Value::Null)["modeName"], "phase_lock");
        let s = ok(h, "alinkUsbStatus", Value::Null);
        assert_eq!(
            (s["state"].as_str(), s["dMin"].as_u64()),
            (Some("phase_locked"), Some(170))
        );
        ok(h, "setAlinkUsb", json!({ "mode": 2, "d": 240 }));
        assert_eq!(ok(h, "alinkUsb", Value::Null)["d"], 240);
        assert_eq!(
            ok(h, "alinkUsbDiag", Value::Null)["ends"]
                .as_array()
                .map(Vec::len),
            Some(9)
        );
        ok(h, "clearAlinkUsbDiag", Value::Null);
        let f = ok(h, "frameCounterTimeBase", Value::Null);
        assert_eq!(f["timeBase"], "usb_sample");
        let bad = run(
            h,
            "applyAtTimeBase",
            json!({ "timeBase": 2, "frame": 4096, "op": 1 }),
        );
        assert_eq!(bad["code"], "invalid", "{bad}");
        let bad = run(h, "setAlinkUsb", json!({ "mode": 9, "d": 192 }));
        assert_eq!(bad["code"], "invalid", "{bad}");
        // The plain demo: unsupported, as the alink's calls.
        ok(h, "open", json!({ "kind": "demo" }));
        assert_eq!(ok(h, "usbDirectSupported", Value::Null), false);
        assert_eq!(run(h, "alinkUsbStatus", Value::Null)["code"], "unsupported");
    }

    // -- WP AUSB2 --
    #[test]
    fn wp_ausb2_the_lock_error_action_on_the_usb_demo() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo", "usbDirect": true }));
        assert_eq!(ok(h, "alinkUsb", Value::Null)["onLockError"], "mute");
        ok(
            h,
            "setAlinkUsbOnLockError",
            json!({ "playUnaligned": true }),
        );
        assert_eq!(
            ok(h, "alinkUsb", Value::Null)["onLockError"],
            "play_unaligned"
        );
        assert_eq!(ok(h, "alinkUsbStatus", Value::Null)["lockError"], false);
        assert_eq!(ok(h, "alinkUsbDiag", Value::Null)["version"], 2);
        let bad = run(h, "setAlinkUsbOnLockError", json!({}));
        assert_eq!(bad["code"], "invalid", "{bad}");
    }

    #[test]
    fn wp_sw1_the_plain_demo_has_no_link() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        assert_eq!(ok(h, "alinkInfo", Value::Null), Value::Null);
        let r = run(h, "alinkStatus", Value::Null);
        assert_eq!(r["code"], "unsupported", "{r}");
    }
}
