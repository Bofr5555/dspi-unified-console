//! WP MCP5: an agent's measurement request through C: the start's
//! `measuring`, the `measureRequests` event, the request listed and taken
//! once, started on the device's core as the agent, reported, and gone once
//! the server stops.

#![allow(clippy::arithmetic_side_effects)]

use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use dspi_core::measure::MeasureRole;
use dspi_core::{SpeakerLayout, SpeakerLayoutOutput};

use super::*;
use crate::fns_core_set::{
    dspi_core_set_core, dspi_core_set_free, dspi_core_set_new, dspi_core_set_open,
};

fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> Value {
    let mut buf = vec![0u8; 1 << 20];
    let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
    assert!(needed as usize <= buf.len(), "{needed}");
    let end = buf.iter().position(|b| *b == 0).unwrap();
    serde_json::from_slice(&buf[..end]).unwrap()
}

extern "C" fn heard(event: *const c_char, user: *mut c_void) {
    // SAFETY: `user` is the test's `Mutex<Vec<Value>>`, alive for the test.
    let seen = unsafe { &*user.cast::<Mutex<Vec<Value>>>() };
    // SAFETY: the server passes a NUL-terminated string that lives for the call.
    let text = unsafe { CStr::from_ptr(event) }.to_str().unwrap();
    seen.lock()
        .unwrap()
        .push(serde_json::from_str(text).unwrap());
}

fn call(port: u16, token: &str, tool: &str, args: Value) -> Value {
    let body = json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {
        "name": tool, "arguments": args, "_meta": {
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {}}}})
    .to_string();
    let head = format!(
        "POST /mcp HTTP/1.1\r\nHost: localhost:{port}\r\nContent-Type: application/json\r\nAuthorization: Bearer {token}\r\nMCP-Protocol-Version: 2026-07-28\r\nMcp-Method: tools/call\r\nMcp-Name: {tool}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    s.write_all(head.as_bytes()).unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    serde_json::from_str::<Value>(out.split_once("\r\n\r\n").map_or("", |p| p.1)).unwrap()["result"]
        .clone()
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn wp_mcp5_a_measurement_request_through_c() {
    let set = dspi_core_set_new();
    let t = CString::new(r#"{"kind":"demo"}"#).unwrap();
    let opened = reply(|b, l| dspi_core_set_open(set, t.as_ptr(), b, l));
    let id = CString::new(opened["id"].as_str().unwrap()).unwrap();
    let ffi_core = dspi_core_set_core(set, id.as_ptr());
    assert!(!ffi_core.is_null());
    let core = set_arc(set)
        .unwrap()
        .core(&DeviceId::parse(id.to_str().unwrap()).unwrap())
        .unwrap();
    core.fetch_all().unwrap();
    core.set_user_volume(-20.0).unwrap();
    core.speaker_layout_set(SpeakerLayout {
        outputs: vec![SpeakerLayoutOutput {
            output: 0,
            role: MeasureRole::Low,
            speaker: "L".into(),
            group: 0,
        }],
        confirmed_by_user: true,
        ..SpeakerLayout::default()
    })
    .unwrap();
    assert!(core.measure_sim_set(true, 5).unwrap().on);

    let mcp = dspi_mcp_new(set);
    let seen = Box::new(Mutex::new(Vec::<Value>::new()));
    dspi_mcp_set_listener(mcp, Some(heard), ptr::from_ref(&*seen).cast_mut().cast());
    let never = CString::new("never_ask").unwrap();
    assert!(dspi_mcp_set_confirm(mcp, never.as_ptr()));

    // Stopped: nothing waits, nothing to take.
    assert_eq!(
        reply(|b, l| dspi_mcp_measure_requests(mcp, b, l))["value"],
        json!([])
    );
    assert_eq!(
        reply(|b, l| dspi_mcp_measure_take(mcp, 1, b, l))["code"],
        json!("gone")
    );

    // Without `measuring` the app has no Measure view for agents.
    let port = free_port();
    let config = CString::new(json!({"port": port}).to_string()).unwrap();
    assert_eq!(
        reply(|b, l| dspi_mcp_start(mcp, config.as_ptr(), b, l))["ok"],
        json!(true)
    );
    let name = CString::new("Claude Code").unwrap();
    let token = reply(|b, l| dspi_mcp_add_client(mcp, name.as_ptr(), b, l))["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let r = call(port, &token, "dspi_measure_start", json!({"outputs": [0]}));
    assert_eq!(r["isError"], json!(true), "{r}");
    assert!(
        r["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Measure view")
    );
    assert!(dspi_mcp_stop(mcp));

    // With it: the request, the event, take once, begin as the agent.
    let port = free_port();
    let config = CString::new(json!({"port": port, "measuring": true}).to_string()).unwrap();
    assert_eq!(
        reply(|b, l| dspi_mcp_start(mcp, config.as_ptr(), b, l))["ok"],
        json!(true)
    );
    let token = reply(|b, l| dspi_mcp_add_client(mcp, name.as_ptr(), b, l))["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let r = call(
        port,
        &token,
        "dspi_measure_start",
        json!({"outputs": [0], "sweep_ms": 1000}),
    );
    assert_eq!(r["isError"], json!(false), "{r}");
    assert_eq!(r["structuredContent"]["status"], json!("waiting_for_app"));
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|e| e["event"] == json!("measureRequests"))
    );
    let waiting = reply(|b, l| dspi_mcp_measure_requests(mcp, b, l))["value"].clone();
    let req = &waiting[0];
    assert_eq!(req["client_name"], json!("AI · Claude Code"), "{waiting}");
    assert_eq!(req["loop"], json!("session"));
    let rid = req["request_id"].as_u64().unwrap();
    let taken = reply(|b, l| dspi_mcp_measure_take(mcp, rid, b, l));
    assert_eq!(taken["ok"], json!(true), "{taken}");
    assert_eq!(
        reply(|b, l| dspi_mcp_measure_take(mcp, rid, b, l))["code"],
        json!("gone"),
        "taken once"
    );
    let ticket = CString::new(taken.to_string()).unwrap();
    let state = reply(|b, l| dspi_mcp_measure_begin(ffi_core, ticket.as_ptr(), 48_000, b, l));
    assert_eq!(state["ok"], json!(true), "{state}");
    assert_ne!(state["actor"], json!(0), "the agent's: {state}");
    assert!(dspi_mcp_measure_report(mcp, rid, true, ptr::null()));
    assert!(
        !dspi_mcp_measure_report(mcp, rid, true, ptr::null()),
        "once"
    );
    assert!(core.measure_state().active);
    let st = call(port, &token, "dspi_measure_status", json!({}));
    assert_eq!(
        st["structuredContent"]["request"]["state"],
        json!("running"),
        "{st}"
    );

    // Its Stop through the agent; a bad ticket starts nothing.
    let r = call(port, &token, "dspi_measure_stop", json!({}));
    assert_eq!(
        r["structuredContent"]["stopped"],
        json!(["measurement"]),
        "{r}"
    );
    assert!(!core.measure_state().active);
    let junk = CString::new("{\"kind\":\"measure\"}").unwrap();
    let r = reply(|b, l| dspi_mcp_measure_begin(ffi_core, junk.as_ptr(), 48_000, b, l));
    assert_eq!(r["code"], json!("refused"), "{r}");
    assert!(!core.measure_state().active);

    assert!(dspi_mcp_stop(mcp));
    dspi_mcp_free(mcp);
    dspi_core_set_free(set);
}
