//! The calls the page's Worker makes, as JSON: `{"method": …, "params": …}`
//! in, `{"ok": true, "value": …}` or `{"ok": false, "code": …, "error": …}`
//! out. The shapes are the core's records as its serde makes them
//! (`dspi_core::DspState`, camelCase, which the C ABI's JSON replies carry
//! too); records without serde are written here with the same field names
//! in camelCase. `apps/web/src/core/protocol.ts` is the same contract in
//! TypeScript. Error codes are the C ABI's style (`read_only`).
//!
//! A module holds any number of sessions, each its own [`DspiCore`] behind a
//! handle: no device is "the" device. The page runs one session per Worker,
//! but nothing here relies on that.
//!
//! Every call goes through [`crate::guard`] first. No borrow of the session
//! table is held while the core runs, so a call that suspends leaves the
//! table usable.

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use dspi_core::{DspiCore, DspiError, RtaRequest};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::guard::{self, Refusal};
// -- WP WB4a --
use crate::usb;
use crate::write;

/// An error reply: a stable code (protocol.ts lists them) and the core's
/// own wording.
#[derive(Debug, Clone, PartialEq)]
pub struct CallError {
    pub code: &'static str,
    pub message: String,
}

impl CallError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<DspiError> for CallError {
    fn from(e: DspiError) -> Self {
        let code = match &e {
            DspiError::Disconnected => "not_open",
            DspiError::Transfer { .. } => "transfer",
            DspiError::Refused { .. } => "refused",
            DspiError::InvalidArgument { .. } => "invalid",
            DspiError::Unsupported => "unsupported",
            DspiError::ReadOnly => "read_only",
            DspiError::Locked => "locked",
            DspiError::Denied => "denied",
            DspiError::Link { .. } => "link",
            _ => "internal",
        };
        Self::new(code, e.to_string())
    }
}

type CallResult = Result<Value, CallError>;

/// `v` as JSON with every `f32` at its own precision: through serde_json's
/// text, which prints an `f32` as the shortest decimal that reads back the
/// same (`-12.3`), where `to_value` widens it to `f64` first
/// (`-12.300000190734863`).
pub fn tidy<T: serde::Serialize + ?Sized>(v: &T) -> Value {
    serde_json::to_string(v)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
}

/// One device session.
struct Session {
    core: Arc<DspiCore>,
    /// The open device is the core's demo: the demo's own simulations apply.
    demo: bool,
}

thread_local! {
    static SESSIONS: RefCell<Vec<Option<Session>>> = const { RefCell::new(Vec::new()) };
}

/// The reply for a call the guard did not let in.
pub fn refusal_reply(r: Refusal) -> Vec<u8> {
    let e = match r {
        Refusal::Busy => CallError::new("busy", "the core is already running a call"),
        Refusal::Dead => CallError::new(
            "dead",
            format!(
                "the core stopped after an internal error: {}",
                guard::panic_message()
            ),
        ),
    };
    reply(Err(e))
}

fn reply(r: CallResult) -> Vec<u8> {
    let v = match r {
        Ok(value) => json!({ "ok": true, "value": value }),
        Err(e) => json!({ "ok": false, "code": e.code, "error": e.message }),
    };
    v.to_string().into_bytes()
}

/// A new session with no device; its handle.
pub fn session_new() -> Result<u32, Refusal> {
    let _in = guard::enter()?;
    Ok(SESSIONS.with_borrow_mut(|s| {
        let free = s.iter().position(Option::is_none);
        let session = Some(Session {
            core: DspiCore::new(),
            demo: false,
        });
        match free {
            Some(i) => {
                if let Some(slot) = s.get_mut(i) {
                    *slot = session;
                }
                u32::try_from(i).unwrap_or(u32::MAX)
            }
            None => {
                s.push(session);
                u32::try_from(s.len().saturating_sub(1)).unwrap_or(u32::MAX)
            }
        }
    }))
}

/// Close the session's device and forget it. False for an unknown handle.
pub fn session_free(h: u32) -> Result<bool, Refusal> {
    let _in = guard::enter()?;
    let taken = SESSIONS.with_borrow_mut(|s| s.get_mut(h as usize).and_then(Option::take));
    Ok(match taken {
        Some(s) => {
            s.core.disconnect();
            true
        }
        None => false,
    })
}

#[derive(Deserialize)]
struct Request {
    method: String,
    #[serde(default)]
    params: Value,
}

/// Run one call on session `h`. The guard refuses a call while another runs;
/// a panic marks the module dead and answers an error.
pub fn call(h: u32, request: &[u8]) -> Vec<u8> {
    let entered = match guard::enter() {
        Ok(e) => e,
        Err(r) => return refusal_reply(r),
    };
    let r = catch_unwind(AssertUnwindSafe(|| dispatch(h, request)));
    drop(entered);
    match r {
        Ok(r) => reply(r),
        Err(payload) => {
            let what = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a panic".to_owned());
            guard::mark_dead(&what);
            refusal_reply(Refusal::Dead)
        }
    }
}

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, CallError> {
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

fn dispatch(h: u32, request: &[u8]) -> CallResult {
    let req: Request = serde_json::from_slice(request)
        .map_err(|e| CallError::new("invalid", format!("not a call: {e}")))?;
    // The core out of the table, so no borrow is held while it runs.
    let (core, demo) = SESSIONS
        .with_borrow(|s| {
            s.get(h as usize)?
                .as_ref()
                .map(|s| (Arc::clone(&s.core), s.demo))
        })
        .ok_or_else(|| CallError::new("not_open", format!("no session {h}")))?;
    match req.method.as_str() {
        "open" => open(h, &core, params(req.params)?),
        "state" => {
            need_open(&core)?;
            // -- WP WB4a -- (a USB device's serial)
            Ok(usb::with_serial(h, snapshot(&core, demo)))
        }
        "write" => {
            let w: write::WriteParams = params(req.params)?;
            let v = write::write(&core, &w)?;
            Ok(tidy(&v))
        }
        "describe" => {
            #[derive(Deserialize)]
            struct P {
                path: String,
            }
            let p: P = params(req.params)?;
            write::describe(&p.path)
        }
        "meters" => meters(&core),
        "poll" => poll(&core),
        "rtaStart" => rta_start(&core, params(req.params)?),
        "rtaStop" => {
            core.rta_set_request(RtaRequest::default());
            Ok(Value::Null)
        }
        "rtaTick" => rta_tick(&core),
        "rta" => Ok(rta(&core)),
        "demo" => demo_action(&core, demo, params(req.params)?),
        "close" => {
            core.disconnect();
            set_demo(h, false);
            // -- WP WB4a --
            usb::closed(h);
            Ok(Value::Null)
        }
        // -- WP WB4a -- WebUSB (src/usb.rs)
        "usbPush" => usb::push(&core, params(req.params)?),
        "usbGone" => {
            set_demo(h, false);
            Ok(usb::gone(h, &core))
        }
        "usbIds" => Ok(usb::ids()),
        #[cfg(any(test, feature = "test-hooks"))]
        "test.sleep" => {
            #[derive(Deserialize)]
            struct P {
                ms: u64,
            }
            let p: P = params(req.params)?;
            dspi_transport::clock::sleep(std::time::Duration::from_millis(p.ms));
            Ok(json!(p.ms))
        }
        #[cfg(any(test, feature = "test-hooks"))]
        "test.panic" => panic!("test.panic was called"),
        other => Err(CallError::new(
            "unknown_method",
            format!("unknown method {other}"),
        )),
    }
}

fn need_open(core: &DspiCore) -> Result<(), CallError> {
    if core.is_connected() {
        Ok(())
    } else {
        Err(CallError::new(
            "not_open",
            "No DSPi is open in this session",
        ))
    }
}

fn set_demo(h: u32, demo: bool) {
    SESSIONS.with_borrow_mut(|s| {
        if let Some(Some(s)) = s.get_mut(h as usize) {
            s.demo = demo;
        }
    });
}

/// `open`'s parameters: protocol.ts `DeviceTarget`.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Target {
    Demo {
        #[serde(default, rename = "readOnly")]
        read_only: bool,
    },
    // -- WP WB4a --
    Usb(usb::UsbTarget),
    Hub {},
}

/// The demo in an older wire format: the core reads it and refuses writes.
const READ_ONLY_DEMO_WIRE: u8 = 31;

fn open(h: u32, core: &DspiCore, target: Target) -> CallResult {
    match target {
        Target::Demo { read_only } => {
            if read_only {
                core.open_demo_wire(READ_ONLY_DEMO_WIRE)?;
            } else {
                core.open_demo()?;
            }
            set_demo(h, true);
            Ok(snapshot(core, true))
        }
        // -- WP WB4a --
        Target::Usb(t) => {
            usb::open(h, core, t)?;
            set_demo(h, false);
            Ok(usb::with_serial(h, snapshot(core, false)))
        }
        // WB4b (Link) brings this.
        Target::Hub {} => Err(CallError::new(
            "refused",
            "This connection type is not available yet",
        )),
    }
}

// -- WP WB4a --
/// Session `h`'s core, for the USB tests.
#[cfg(test)]
pub(crate) fn core_of(h: u32) -> Option<Arc<DspiCore>> {
    SESSIONS.with_borrow(|s| s.get(h as usize)?.as_ref().map(|s| Arc::clone(&s.core)))
}

/// What the page shows of a device: who it is (`device`) and the core's
/// mirror (`state`, `DspState` without its raw image `bulk_raw`).
fn snapshot(core: &DspiCore, demo: bool) -> Value {
    let s = core.get_state();
    let read_only = s.format_version != dspi_proto::generated::wire::WIRE_FORMAT_VERSION as u8;
    let name = core
        .cached_device_name()
        .ok()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "DSPi".to_owned());
    let mut firmware = format!("{}.{}.{}", s.fw_major, s.fw_minor, s.fw_patch);
    if s.fw_beta != 0 {
        firmware.push_str(&format!("-beta{}", s.fw_beta));
    }
    let platform = match s.platform_id {
        0 => "RP2040",
        1 => "RP2350",
        _ => "unknown",
    };
    let serial = if demo { demo_serial() } else { String::new() };
    let mut state = tidy(&s);
    if let Some(m) = state.as_object_mut() {
        m.remove("bulkRaw");
    }
    json!({
        "device": {
            "name": name,
            "serial": serial,
            "firmware": firmware,
            "platform": platform,
            "demo": demo,
            "readOnly": read_only,
            "readOnlyReason": if read_only { DspiError::ReadOnly.to_string() } else { String::new() },
        },
        "state": state,
    })
}

/// The serial the core's demo reports.
fn demo_serial() -> String {
    thread_local! {
        static SERIAL: String = {
            use dspi_transport::Transport;
            dspi_transport::FakeDevice::demo().descriptor().serial.clone()
        };
    }
    SERIAL.with(Clone::clone)
}

/// `meters`: `dspi_core::SystemStatus`, its field names in camelCase.
fn meters(core: &DspiCore) -> CallResult {
    let st = core.fetch_status()?;
    Ok(json!({
        "peaks": tidy(&st.peaks),
        "cpu0": st.cpu0,
        "cpu1": st.cpu1,
        "clipFlags": st.clip_flags,
        "activeInputChannels": st.active_input_channels,
        "numChannels": st.num_channels,
        "stale": st.stale,
        "ageMs": st.age_ms,
    }))
}

/// `poll`: the Worker's notification turn. What changed (`NOTIFY_*` flags),
/// when a failed re-read is due again, and whether the device is still there.
fn poll(core: &DspiCore) -> CallResult {
    if !core.is_connected() {
        return Ok(json!({ "flags": 0, "retryMs": null, "connected": false }));
    }
    let r = core.process_notifications();
    let retry = core
        .notify_retry_due_in()
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    Ok(json!({
        "flags": r.flags,
        "retryMs": retry,
        "connected": core.is_connected(),
    }))
}

/// `rta_start`'s parameters: the fields of `RtaRequest` the page sets.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RtaParams {
    tap: u8,
    mask: u16,
    #[serde(default)]
    wants_bins: bool,
    #[serde(default)]
    fft_order: u8,
    #[serde(default)]
    avg_ms: u16,
    #[serde(default)]
    peak_decay_db_s: u8,
}

fn rta_start(core: &DspiCore, p: RtaParams) -> CallResult {
    need_open(core)?;
    core.rta_start();
    core.rta_set_request(RtaRequest {
        tap: p.tap,
        mask: p.mask,
        wants_bins: p.wants_bins,
        fft_order: p.fft_order,
        avg_ms: p.avg_ms,
        peak_decay_db_s: p.peak_decay_db_s,
        ..RtaRequest::default()
    });
    Ok(Value::Null)
}

/// `rta_tick`: one turn of the analyser, as its thread takes natively. The
/// wait before the next, in ms, or null when there is nothing to tick.
fn rta_tick(core: &DspiCore) -> CallResult {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    let wait = core
        .rta_tick()
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    // Natively the core's own worker thread takes the turns.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    let wait: Option<u64> = {
        let _ = core;
        None
    };
    Ok(json!({ "waitMs": wait }))
}

/// `rta`: the analyser's caps and latest picture, its field names in camelCase.
fn rta(core: &DspiCore) -> Value {
    let c = core.rta_get_caps();
    let s = core.rta_get_snapshot();
    let count = usize::from(s.count);
    let frames: Vec<Value> = s
        .frames
        .iter()
        .take(count)
        .map(|f| {
            json!({
                "channel": f.channel,
                "nBands": f.n_bands,
                "silent": f.silent,
                "avgDb": tidy(&f.avg_db),
                "peakDb": tidy(&f.peak_db),
            })
        })
        .collect();
    json!({
        "caps": {
            "supported": c.supported,
            "inputChannels": c.input_channels,
            "outputChannels": c.output_channels,
            "orderMin": c.order_min,
            "orderMax": c.order_max,
            "orderDefault": c.order_default,
            "bassBands": c.bass_bands,
        },
        "snapshot": {
            "version": s.version,
            "tap": s.tap,
            "count": s.count,
            "frames": frames,
            "populated": s.populated,
            "running": s.running,
            "framesPerS": s.frames_per_s,
            "sampleRate": s.sample_rate,
            "fftOrder": s.fft_order,
            "configRejected": s.config_rejected,
            "refreshIntervalS": tidy(&s.refresh_interval_s),
        },
    })
}

/// `demo`'s parameters: the core's own demo simulations.
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
enum DemoAction {
    /// Another host renames the device: it arrives as a notification.
    RenameElsewhere { name: String },
}

/// `demo_rename_elsewhere`'s target for the device name (input sources are
/// 0..).
const DEVICE_NAME_TARGET: u8 = 0xFF;

fn demo_action(core: &DspiCore, demo: bool, a: DemoAction) -> CallResult {
    if !demo {
        return Err(CallError::new("refused", "only the demo DSPi does that"));
    }
    match a {
        DemoAction::RenameElsewhere { name } => {
            core.demo_rename_elsewhere(DEVICE_NAME_TARGET, name)?;
        }
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

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

    fn code(r: &Value) -> &str {
        r["code"].as_str().unwrap_or("")
    }

    #[test]
    fn wb3_two_demo_sessions_connect_write_with_read_back_and_stay_apart() {
        guard::reset_for_test();
        let a = session_new().unwrap();
        let b = session_new().unwrap();
        assert_ne!(a, b);
        for h in [a, b] {
            let snap = ok(h, "open", json!({ "kind": "demo" }));
            assert_eq!(snap["device"]["demo"], true);
            assert_eq!(snap["device"]["readOnly"], false);
            assert!(!snap["device"]["serial"].as_str().unwrap().is_empty());
            assert!(snap["state"]["userVolumeDb"].is_number());
            assert!(snap["state"].get("bulkRaw").is_none());
        }
        let v = ok(
            a,
            "write",
            json!({ "path": "vol.user", "value": { "float": -17.5 } }),
        );
        assert_eq!(v, json!({ "float": -17.5 }));
        // f32 at its own precision, not widened to f64's digits.
        let v = ok(
            a,
            "write",
            json!({ "path": "vol.user", "value": { "float": -12.3 } }),
        );
        assert_eq!(v.to_string(), r#"{"float":-12.3}"#);
        let v = ok(
            a,
            "write",
            json!({ "path": "vol.user", "value": { "float": -17.5 } }),
        );
        assert_eq!(v, json!({ "float": -17.5 }));
        assert_eq!(ok(a, "state", Value::Null)["state"]["userVolumeDb"], -17.5);
        assert_ne!(ok(b, "state", Value::Null)["state"]["userVolumeDb"], -17.5);

        let g = ok(
            a,
            "write",
            json!({ "path": "out.gain", "indices": [1], "value": { "float": -3.0 } }),
        );
        assert_eq!(g, json!({ "float": -3.0 }));

        // -- c6e W3-1 -- `pre` trims the one input asked for, `pre.all` every input.
        let p = ok(
            a,
            "write",
            json!({ "path": "pre", "indices": [1], "value": { "float": -4.0 } }),
        );
        assert_eq!(p, json!({ "float": -4.0 }));
        let st = ok(a, "state", Value::Null);
        assert_eq!(st["state"]["inputPreampDb"][1], -4.0);
        assert_eq!(st["state"]["inputPreampDb"][0], 0.0, "only input 1 changed");
        let r = run(
            a,
            "write",
            json!({ "path": "pre", "indices": [200], "value": { "float": -4.0 } }),
        );
        assert!(
            !code(&r).is_empty(),
            "an input the device does not have is refused: {r}"
        );
        let all = ok(
            a,
            "write",
            json!({ "path": "pre.all", "value": { "float": -2.0 } }),
        );
        assert_eq!(all, json!({ "float": -2.0 }));

        let r = run(
            a,
            "write",
            json!({ "path": "vol.user", "value": { "float": 3.0 } }),
        );
        assert_eq!(code(&r), "out_of_range");
        assert_eq!(r["error"], "Volume must be between -60 and 0 dB");
        let r = run(
            a,
            "write",
            json!({ "path": "vol.user", "value": { "bool": true } }),
        );
        assert_eq!(code(&r), "invalid");
        let r = run(
            a,
            "write",
            json!({ "path": "out.gain", "value": { "float": -3.0 } }),
        );
        assert_eq!(code(&r), "invalid", "out.gain needs its output");
        let r = run(
            a,
            "write",
            json!({ "path": "no.such", "value": { "float": 0.0 } }),
        );
        assert_eq!(code(&r), "unknown_param");

        ok(a, "close", Value::Null);
        assert_eq!(code(&run(a, "state", Value::Null)), "not_open");
        for h in [a, b] {
            assert!(session_free(h).unwrap());
        }
        assert_eq!(code(&run(a, "state", Value::Null)), "not_open");
    }

    #[test]
    fn wb3_an_older_firmware_demo_is_read_only_in_the_cores_words() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        let snap = ok(h, "open", json!({ "kind": "demo", "readOnly": true }));
        assert_eq!(snap["device"]["readOnly"], true);
        assert_eq!(
            snap["device"]["readOnlyReason"],
            "the connected firmware is read-only in this version"
        );
        let r = run(
            h,
            "write",
            json!({ "path": "vol.user", "value": { "float": -10.0 } }),
        );
        assert_eq!(code(&r), "read_only");
        session_free(h).unwrap();
    }

    #[test]
    fn wb3_usb_and_hubs_are_not_offered_yet_and_unknown_methods_say_so() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        // -- WP WB4a -- USB is offered now; without a chosen device it says so.
        assert_eq!(code(&run(h, "open", json!({ "kind": "usb" }))), "invalid");
        assert_eq!(
            code(&run(
                h,
                "open",
                json!({ "kind": "hub", "url": "ws://h:1", "deviceId": "" })
            )),
            "refused"
        );
        assert_eq!(
            code(&run(h, "simulate", json!({ "what": "link-lost" }))),
            "unknown_method"
        );
        assert_eq!(code(&run(9999, "state", Value::Null)), "not_open");
        let out = call(h, b"not json");
        assert_eq!(code(&serde_json::from_slice(&out).unwrap()), "invalid");
        session_free(h).unwrap();
    }

    #[test]
    fn wb3_a_foreign_rename_arrives_through_poll() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        ok(
            h,
            "demo",
            json!({ "action": "renameElsewhere", "name": "Elsewhere" }),
        );
        let until = Instant::now() + Duration::from_secs(5);
        let mut flags = 0;
        while flags & dspi_core::consts::NOTIFY_NAMES == 0 && Instant::now() < until {
            flags |= ok(h, "poll", Value::Null)["flags"].as_u64().unwrap() as u32;
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_ne!(flags & dspi_core::consts::NOTIFY_NAMES, 0);
        assert_eq!(ok(h, "state", Value::Null)["device"]["name"], "Elsewhere");
        session_free(h).unwrap();
    }

    #[test]
    fn wb3_meters_and_the_analyser_answer_in_the_cores_shapes() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        let m = ok(h, "meters", Value::Null);
        assert!(!m["peaks"].as_array().unwrap().is_empty());
        ok(h, "rtaStart", json!({ "tap": 1, "mask": 3 }));
        let until = Instant::now() + Duration::from_secs(10);
        let mut count = 0;
        while count == 0 && Instant::now() < until {
            count = ok(h, "rta", Value::Null)["snapshot"]["count"]
                .as_u64()
                .unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(count >= 1, "the native worker thread delivered frames");
        assert_eq!(ok(h, "rta", Value::Null)["caps"]["supported"], true);
        ok(h, "rtaStop", Value::Null);
        session_free(h).unwrap();
    }

    #[test]
    fn wb3_a_panic_marks_the_module_dead_and_every_later_call_is_refused() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        ok(h, "open", json!({ "kind": "demo" }));
        let r = run(h, "test.panic", Value::Null);
        assert_eq!(code(&r), "dead");
        assert!(
            r["error"]
                .as_str()
                .unwrap()
                .contains("test.panic was called")
        );
        let later = run(h, "state", Value::Null);
        assert_eq!(code(&later), "dead");
        assert!(
            later["error"]
                .as_str()
                .unwrap()
                .contains("test.panic was called")
        );
        assert_eq!(session_new().err(), Some(Refusal::Dead));
        guard::reset_for_test();
    }
}
