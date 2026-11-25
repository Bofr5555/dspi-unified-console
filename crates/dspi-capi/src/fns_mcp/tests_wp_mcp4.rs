//! WP MCP4: the agent policy through C: the setting, a held call's sheet and
//! its answer, the activity kept over a stop, and the AutoEQ database.

#![allow(clippy::arithmetic_side_effects)]

use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use super::*;
use crate::fns_core_set::{dspi_core_set_free, dspi_core_set_new, dspi_core_set_open};

fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> Value {
    let mut buf = vec![0u8; 65536];
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

/// An agent's tool call, on a connection left open.
fn call(port: u16, token: &str, tool: &str, args: Value) -> TcpStream {
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
    s
}

fn answer(mut s: TcpStream) -> Value {
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    serde_json::from_str::<Value>(out.split_once("\r\n\r\n").map_or("", |p| p.1)).unwrap()["result"]
        .clone()
}

/// Two confirmations' worth of waiting: the sheet, once it is there.
fn sheet(mcp: *mut FfiMcp) -> Value {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let r = reply(|b, l| dspi_mcp_confirmations(mcp, b, l));
        if let Some(first) = r["value"].as_array().and_then(|v| v.first()) {
            return first.clone();
        }
        assert!(Instant::now() < end, "no sheet");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn wp_mcp4_the_policy_and_the_confirmations_through_c() {
    let set = dspi_core_set_new();
    let t = CString::new(r#"{"kind":"demo"}"#).unwrap();
    assert_eq!(
        reply(|b, l| dspi_core_set_open(set, t.as_ptr(), b, l))["ok"],
        json!(true)
    );
    let mcp = dspi_mcp_new(set);
    let seen = Box::new(Mutex::new(Vec::<Value>::new()));
    dspi_mcp_set_listener(mcp, Some(heard), ptr::from_ref(&*seen).cast_mut().cast());
    let mode = |m: &str| {
        let m = CString::new(m).unwrap();
        dspi_mcp_set_confirm(mcp, m.as_ptr())
    };
    assert!(!mode("sometimes"));
    assert!(!dspi_mcp_set_confirm(mcp, ptr::null()));
    assert!(!dspi_mcp_set_autoeq(mcp, b"junk".as_ptr(), 4));
    assert!(dspi_mcp_set_autoeq(mcp, ptr::null(), 0));

    // Stopped: nothing waits, nothing can be answered.
    assert_eq!(
        reply(|b, l| dspi_mcp_confirmations(mcp, b, l))["value"],
        json!([])
    );
    assert!(!dspi_mcp_confirm_answer(mcp, 1, true));

    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config = CString::new(json!({"port": port}).to_string()).unwrap();
    let start = || reply(|b, l| dspi_mcp_start(mcp, config.as_ptr(), b, l));
    let r = start();
    assert_eq!(r["ok"], json!(true), "{r}");
    let name = CString::new("Claude Code").unwrap();
    let minted = reply(|b, l| dspi_mcp_add_client(mcp, name.as_ptr(), b, l));
    let token = minted["token"].as_str().unwrap().to_owned();

    // Writes are on, and risky ones are held by default.
    let quiet = answer(call(
        port,
        &token,
        "dspi_set_gain",
        json!({"output": 1, "gain_db": -20.0}),
    ));
    assert_eq!(quiet["isError"], json!(false), "{quiet}");
    let held = call(
        port,
        &token,
        "dspi_set_gain",
        json!({"output": 1, "gain_db": -10.0}),
    );
    let s = sheet(mcp);
    assert_eq!(s["client_name"], "AI · Claude Code");
    assert_eq!(s["risks"][0]["kind"], "louder", "{s}");
    let id = s["confirmation_id"].as_u64().unwrap();
    assert!(dspi_mcp_confirm_answer(mcp, id, true));
    assert!(!dspi_mcp_confirm_answer(mcp, id, true), "once");
    let r = answer(held);
    assert_eq!(r["isError"], json!(false), "{r}");
    let names: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|e| e["event"].as_str().unwrap().to_owned())
        .collect();
    assert!(names.contains(&"confirmationsChanged".to_owned()));
    assert!(names.contains(&"activity".to_owned()));

    // Never ask: the setting is live.
    assert!(mode("never_ask"));
    let r = answer(call(
        port,
        &token,
        "dspi_set_gain",
        json!({"output": 1, "gain_db": -5.0}),
    ));
    assert_eq!(r["isError"], json!(false), "{r}");

    // Stop keeps the log, and the next run adds to it.
    let before = reply(|b, l| dspi_mcp_activity(mcp, 100, b, l))["value"].clone();
    assert!(dspi_mcp_stop(mcp));
    let kept = reply(|b, l| dspi_mcp_activity(mcp, 100, b, l))["value"].clone();
    assert_eq!(kept, before);
    assert!(kept.as_array().unwrap().iter().all(|e| e["run"] == 1));
    assert_eq!(start()["ok"], json!(true));
    let name = CString::new("Cursor").unwrap();
    let minted = reply(|b, l| dspi_mcp_add_client(mcp, name.as_ptr(), b, l));
    let token = minted["token"].as_str().unwrap().to_owned();
    // (never ask carries over the restart)
    let r = answer(call(
        port,
        &token,
        "dspi_set_gain",
        json!({"output": 1, "gain_db": -3.0}),
    ));
    assert_eq!(r["isError"], json!(false), "{r}");
    let all = reply(|b, l| dspi_mcp_activity(mcp, 100, b, l))["value"].clone();
    let last = all.as_array().unwrap().last().unwrap().clone();
    assert_eq!((&last["run"], &last["id"]), (&json!(2), &json!(1)));
    let one = reply(|b, l| dspi_mcp_activity(mcp, 1, b, l))["value"].clone();
    assert_eq!(one, json!([last]), "the newest");

    dspi_mcp_free(mcp);
    dspi_core_set_free(set);
}
