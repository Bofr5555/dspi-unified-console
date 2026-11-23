//! WP FEAT2: an agent's request to open a device through C: the start's
//! `opening`, the remembered devices, the `deviceRequests` event, the request
//! listed, taken once and reported.

#![allow(clippy::arithmetic_side_effects)]

use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use super::*;
use crate::fns_core_set::{dspi_core_set_free, dspi_core_set_new};

fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> Value {
    let mut buf = vec![0u8; 1 << 16];
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

fn start(mcp: *mut FfiMcp, opening: bool) -> (u16, String) {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config = CString::new(json!({"port": port, "opening": opening}).to_string()).unwrap();
    let r = reply(|b, l| dspi_mcp_start(mcp, config.as_ptr(), b, l));
    assert_eq!(r["ok"], json!(true), "{r}");
    let name = CString::new("Claude Code").unwrap();
    let r = reply(|b, l| dspi_mcp_add_client(mcp, name.as_ptr(), b, l));
    (port, r["token"].as_str().unwrap().to_owned())
}

#[test]
fn wp_feat2_an_open_request_through_c() {
    let set = dspi_core_set_new();
    let mcp = dspi_mcp_new(set);
    let seen = Box::new(Mutex::new(Vec::<Value>::new()));
    dspi_mcp_set_listener(mcp, Some(heard), ptr::from_ref(&*seen).cast_mut().cast());
    // Stopped: nothing waits; the remembered devices are kept for the start.
    assert_eq!(
        reply(|b, l| dspi_mcp_device_requests(mcp, b, l))["value"],
        json!([])
    );
    assert_eq!(
        reply(|b, l| dspi_mcp_device_take(mcp, 1, b, l))["code"],
        json!("gone")
    );
    let known = CString::new(r#"[{"device":"usb:FEAT2-C","name":"Hall"}]"#).unwrap();
    assert!(dspi_mcp_set_known_devices(mcp, known.as_ptr()));
    let junk = CString::new("{}").unwrap();
    assert!(!dspi_mcp_set_known_devices(mcp, junk.as_ptr()));

    let open = json!({"device": "usb:FEAT2-C"});
    // Without `opening` the app takes none.
    let (port, token) = start(mcp, false);
    let r = call(port, &token, "dspi_open_device", open.clone());
    assert_eq!(r["isError"], json!(true), "{r}");
    assert!(dspi_mcp_stop(mcp));

    let (port, token) = start(mcp, true);
    let r = call(port, &token, "dspi_open_device", open);
    assert_eq!(
        r["structuredContent"]["status"],
        json!("waiting_for_app"),
        "{r}"
    );
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|e| e["event"] == json!("deviceRequests"))
    );
    let waiting = reply(|b, l| dspi_mcp_device_requests(mcp, b, l))["value"].clone();
    assert_eq!(
        waiting[0]["client_name"],
        json!("AI · Claude Code"),
        "{waiting}"
    );
    let id = waiting[0]["request_id"].as_u64().unwrap();
    // A short buffer takes nothing.
    assert_eq!(
        dspi_mcp_device_take(mcp, id, ptr::null_mut(), 0),
        MCP_REPLY_MIN
    );
    let t = reply(|b, l| dspi_mcp_device_take(mcp, id, b, l));
    assert_eq!(t["ticket"]["serial"], json!("FEAT2-C"), "{t}");
    assert_eq!(t["ticket"]["transport"], json!("usb"));
    assert_eq!(
        reply(|b, l| dspi_mcp_device_take(mcp, id, b, l))["code"],
        json!("gone"),
        "taken once"
    );
    let why = CString::new("not plugged in").unwrap();
    assert!(dspi_mcp_device_report(mcp, id, false, why.as_ptr()));
    assert!(!dspi_mcp_device_report(mcp, id, true, ptr::null()), "once");
    let k = call(port, &token, "dspi_list_known_devices", json!({}));
    let r = &k["structuredContent"]["requests"][0];
    assert_eq!(
        (r["state"].clone(), r["note"].clone()),
        (json!("declined"), json!("not plugged in"))
    );
    // Null for none.
    assert!(dspi_mcp_set_known_devices(mcp, ptr::null()));
    let k = call(port, &token, "dspi_list_known_devices", json!({}));
    assert_eq!(k["structuredContent"]["known"], json!([]), "{k}");

    assert!(dspi_mcp_stop(mcp));
    dspi_mcp_free(mcp);
    dspi_core_set_free(set);
}
