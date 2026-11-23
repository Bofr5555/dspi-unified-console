//! WP MCP3: the MCP server through C: start and stop, the clients while
//! running and while stopped, the short-buffer rule, pairing through the
//! listener, pause and the active device, and an agent's request end to end.

#![allow(clippy::arithmetic_side_effects)]

use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use super::*;
use crate::fns_core_set::{dspi_core_set_free, dspi_core_set_new, dspi_core_set_open};

fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> Value {
    let mut buf = vec![0u8; 65536];
    let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
    assert!(needed as usize <= buf.len(), "{needed}");
    let end = buf.iter().position(|b| *b == 0).unwrap();
    serde_json::from_slice(&buf[..end]).unwrap()
}

/// A folder beside the test binary (never tmpfs).
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::current_exe()
        .unwrap()
        .with_file_name("test-scratch")
        .join(format!("capi-mcp3-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn demo_set() -> *mut FfiCoreSet {
    let set = dspi_core_set_new();
    let t = CString::new(r#"{"kind":"demo"}"#).unwrap();
    let r = reply(|b, l| dspi_core_set_open(set, t.as_ptr(), b, l));
    assert_eq!(r["ok"], json!(true), "{r}");
    set
}

fn start(mcp: *mut FfiMcp, config: &str) -> Value {
    let c = CString::new(config).unwrap();
    reply(|b, l| dspi_mcp_start(mcp, c.as_ptr(), b, l))
}

fn add(mcp: *mut FfiMcp, name: &str) -> Value {
    let n = CString::new(name).unwrap();
    reply(|b, l| dspi_mcp_add_client(mcp, n.as_ptr(), b, l))
}

/// One POST to the listener: (status, body).
fn post(port: u16, path: &str, auth: Option<&str>, body: &Value) -> (u16, Value) {
    let body = body.to_string();
    let mut head = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(t) = auth {
        head.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    head.push_str("\r\n");
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    s.write_all(head.as_bytes()).unwrap();
    s.write_all(body.as_bytes()).unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    let status = out
        .split(' ')
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let body = out.split_once("\r\n\r\n").map_or("", |p| p.1);
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
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

#[test]
fn wp_mcp3_the_server_through_c() {
    let set = demo_set();
    assert!(dspi_mcp_new(ptr::null()).is_null());
    let mcp = dspi_mcp_new(set);
    assert!(!mcp.is_null());
    let dir = scratch("server");
    let seen = Box::new(Mutex::new(Vec::<Value>::new()));
    dspi_mcp_set_listener(mcp, Some(heard), ptr::from_ref(&*seen).cast_mut().cast());

    // Not started: no clients' folder yet.
    assert_eq!(
        reply(|b, l| dspi_mcp_status(mcp, b, l)),
        json!({"ok": true, "running": false, "paused": false})
    );
    assert_eq!(add(mcp, "x")["code"], "not_running");
    assert!(!dspi_mcp_stop(mcp));

    // The short-buffer rule: an acting call with a small buffer does nothing.
    let c = CString::new("{}").unwrap();
    let mut small = [0 as c_char; 16];
    assert_eq!(
        dspi_mcp_start(mcp, c.as_ptr(), small.as_mut_ptr(), 16),
        MCP_REPLY_MIN
    );
    assert_eq!(
        dspi_mcp_start(mcp, c.as_ptr(), ptr::null_mut(), 0),
        MCP_REPLY_MIN
    );
    assert_eq!(
        reply(|b, l| dspi_mcp_status(mcp, b, l))["running"],
        json!(false)
    );

    assert_eq!(start(mcp, "not json")["code"], "invalid");
    assert_eq!(start(mcp, r#"{"port": 70000}"#)["code"], "invalid");
    let config =
        json!({"port": 0, "storeDir": dir.to_str().unwrap(), "appVersion": "9.9.9"}).to_string();
    // Port 0 is no port for the app: it names a real one.
    assert_eq!(start(mcp, &config)["code"], "invalid");
    // A free port, found the way an app would not need to.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config =
        json!({"port": port, "storeDir": dir.to_str().unwrap(), "appVersion": "9.9.9"}).to_string();
    let r = start(mcp, &config);
    assert_eq!(r["ok"], json!(true), "{r}");
    assert_eq!(r["port"], json!(port));
    assert!(
        r["addresses"].as_array().unwrap().iter().all(|a| a
            .as_str()
            .unwrap()
            .starts_with("127.0.0.1:")
            || a.as_str().unwrap().starts_with("[::1]:"))
    );
    assert_eq!(start(mcp, &config)["code"], "running");

    // A client added by hand, and its token in an agent's request.
    let minted = add(mcp, "Claude Code");
    let token = minted["token"].as_str().unwrap().to_owned();
    assert!(token.starts_with("dspi-mcp-"));
    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "cc"}}});
    let (status, body) = post(port, "/mcp", Some(&token), &init);
    assert_eq!(status, 200);
    assert_eq!(body["result"]["serverInfo"]["version"], "9.9.9");
    assert_eq!(post(port, "/mcp", Some("dspi-mcp-wrong"), &init).0, 401);

    // Pairing through the listener.
    let (status, asked) = post(port, "/pair", None, &json!({"name": "Claude Desktop"}));
    assert_eq!(status, 200);
    let event = seen
        .lock()
        .unwrap()
        .iter()
        .find(|e| e["event"] == "pairingRequested")
        .cloned()
        .unwrap();
    assert_eq!(event["name"], "Claude Desktop");
    assert_eq!(event["code"], asked["code"]);
    assert_eq!(
        reply(|b, l| dspi_mcp_status(mcp, b, l))["pairing"]["id"],
        event["id"]
    );
    let id = event["id"].as_u64().unwrap();
    assert!(dspi_mcp_pairing_answer(mcp, id, true));
    assert!(!dspi_mcp_pairing_answer(mcp, id, true));
    let (status, got) = post(
        port,
        "/pair/wait",
        None,
        &json!({"ticket": asked["ticket"]}),
    );
    assert_eq!((status, got["status"].clone()), (200, json!("allowed")));
    let clients = reply(|b, l| dspi_mcp_clients(mcp, b, l));
    let names: Vec<&str> = clients["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Claude Code", "Claude Desktop"]);
    assert!(
        !clients.to_string().contains(&token),
        "no token in the list"
    );
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|e| e["event"] == "pairingEnded" && e["outcome"] == "paired")
    );

    // Pause, the active device, the activity.
    let session_less = |name: &str| {
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": name, "arguments": {},
            "_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": {}}}})
    };
    let modern = |token: &str, name: &str| {
        let body = session_less(name).to_string();
        let head = format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost:{port}\r\nContent-Type: application/json\r\nAuthorization: Bearer {token}\r\nMCP-Protocol-Version: 2026-07-28\r\nMcp-Method: tools/call\r\nMcp-Name: {name}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(head.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        serde_json::from_str::<Value>(out.split_once("\r\n\r\n").unwrap().1).unwrap()
    };
    dspi_mcp_pause(mcp, true);
    assert_eq!(
        reply(|b, l| dspi_mcp_status(mcp, b, l))["paused"],
        json!(true)
    );
    // -- WP AIVIS -- said as paused, not as an error; what runs is listed
    let paused = modern(&token, "dspi_list_devices");
    assert_eq!(paused["result"]["isError"], json!(false));
    assert_eq!(
        paused["result"]["structuredContent"]["status"],
        json!("paused")
    );
    let work = reply(|b, l| dspi_mcp_work(mcp, b, l));
    assert_eq!(work["running"], json!(true), "{work}");
    assert_eq!(work["calls"], json!([]), "{work}");
    let log = reply(|b, l| dspi_mcp_measure_log(mcp, b, l));
    assert_eq!(log, json!({"ok": true, "value": []}));
    dspi_mcp_pause(mcp, false);
    let bad = CString::new("no/such/id").unwrap();
    assert!(!dspi_mcp_set_active_device(mcp, bad.as_ptr()));
    let demo = CString::new("demo:1").unwrap();
    assert!(dspi_mcp_set_active_device(mcp, demo.as_ptr()));
    let r = modern(&token, "dspi_list_devices");
    assert_eq!(r["result"]["isError"], json!(false), "{r}");
    let activity = reply(|b, l| dspi_mcp_activity(mcp, 10, b, l));
    assert_eq!(activity["value"].as_array().unwrap().len(), 1, "{activity}");
    assert!(dspi_mcp_set_active_device(mcp, ptr::null()));

    // Revoke while running: the token stops at once.
    assert!(dspi_mcp_revoke(mcp, minted["id"].as_u64().unwrap()));
    assert_eq!(post(port, "/mcp", Some(&token), &init).0, 401);
    assert!(!dspi_mcp_revoke(mcp, 999));

    // Stopped: nothing listens, the clients are still there to manage.
    assert!(dspi_mcp_stop(mcp));
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    let clients = reply(|b, l| dspi_mcp_clients(mcp, b, l));
    assert_eq!(clients["value"].as_array().unwrap().len(), 1);
    let again = add(mcp, "Cursor");
    assert_eq!(again["ok"], json!(true));
    assert!(dspi_mcp_revoke(mcp, again["id"].as_u64().unwrap()));
    // A restart knows the paired client's token.
    let token2 = got["token"].as_str().unwrap();
    assert_eq!(start(mcp, &config)["ok"], json!(true));
    assert_eq!(post(port, "/mcp", Some(token2), &init).0, 200);

    // Null handles are refused quietly.
    assert!(!dspi_mcp_stop(ptr::null_mut()));
    assert_eq!(dspi_mcp_status(ptr::null_mut(), ptr::null_mut(), 0), 0);
    dspi_mcp_free(ptr::null_mut());
    dspi_mcp_free(mcp);
    assert!(
        TcpStream::connect(("127.0.0.1", port)).is_err(),
        "freeing stops it"
    );
    dspi_core_set_free(set);
}

/// What [`calls_back`] works on.
struct CallsBack {
    mcp: *mut FfiMcp,
    /// The clients listed from inside `clientsChanged`, each time.
    listed: Mutex<Vec<usize>>,
}

// SAFETY: the handle takes calls from any thread; the rest is a mutex.
unsafe impl Send for CallsBack {}
// SAFETY: as for Send.
unsafe impl Sync for CallsBack {}

/// The app's natural listener: on `clientsChanged` it lists the clients, reads
/// the status and (once, for "nested") adds another from inside.
extern "C" fn calls_back(event: *const c_char, user: *mut c_void) {
    // SAFETY: `user` is the test's `CallsBack`, alive for the process.
    let t = unsafe { &*user.cast::<CallsBack>() };
    // SAFETY: the server passes a NUL-terminated string that lives for the call.
    let text = unsafe { CStr::from_ptr(event) }.to_str().unwrap();
    if !text.contains("clientsChanged") {
        return;
    }
    let clients = reply(|b, l| dspi_mcp_clients(t.mcp, b, l));
    let rows = clients["value"].as_array().unwrap();
    t.listed.lock().unwrap().push(rows.len());
    assert_eq!(
        reply(|b, l| dspi_mcp_status(t.mcp, b, l))["running"],
        json!(true)
    );
    dspi_mcp_pause(t.mcp, false);
    if rows.iter().any(|c| c["name"] == "Nested") || rows.len() != 1 {
        return;
    }
    assert_eq!(add(t.mcp, "Nested")["ok"], json!(true));
    // Still inside the first event's callback: refused.
    dspi_mcp_set_listener(t.mcp, None, ptr::null_mut());
}

/// -- WP RBFIX -- review RB R4: `clientsChanged` comes inside the call that
/// caused it, with nothing of the handle locked.
#[test]
fn wp_rbfix_a_listener_may_call_back_from_clients_changed() {
    let set = demo_set();
    let t: &'static CallsBack = Box::leak(Box::new(CallsBack {
        mcp: dspi_mcp_new(set),
        listed: Mutex::default(),
    }));
    let user = ptr::from_ref(t).cast_mut().cast();
    dspi_mcp_set_listener(t.mcp, Some(calls_back), user);
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let dir = scratch("callback");
    let config = json!({"port": port, "storeDir": dir.to_str().unwrap()}).to_string();
    assert_eq!(start(t.mcp, &config)["ok"], json!(true));

    let set = set as usize;
    let (done, ended) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // By hand, with one more added from inside the event.
        let first = add(t.mcp, "First");
        assert_eq!(first["ok"], json!(true));
        assert_eq!(*t.listed.lock().unwrap(), [1, 2]);
        // A pairing allowed.
        let (status, _) = post(port, "/pair", None, &json!({"name": "Paired"}));
        assert_eq!(status, 200);
        let id = reply(|b, l| dspi_mcp_status(t.mcp, b, l))["pairing"]["id"]
            .as_u64()
            .unwrap();
        assert!(dspi_mcp_pairing_answer(t.mcp, id, true));
        assert_eq!(*t.listed.lock().unwrap(), [1, 2, 3]);
        // A client removed.
        assert!(dspi_mcp_revoke(t.mcp, first["id"].as_u64().unwrap()));
        assert_eq!(*t.listed.lock().unwrap(), [1, 2, 3, 2]);
        assert!(dspi_mcp_stop(t.mcp));
        done.send(()).unwrap();
    });
    ended
        .recv_timeout(Duration::from_secs(60))
        .expect("a call back from clientsChanged deadlocked or failed");
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    dspi_mcp_free(t.mcp);
    dspi_core_set_free(set as *mut FfiCoreSet);
}

#[test]
fn wp_mcp3_the_relay_refuses_a_bad_configuration() {
    let bad = CString::new("{").unwrap();
    assert_eq!(dspi_mcp_relay_stdio(bad.as_ptr(), None, ptr::null_mut()), 2);
    let port = CString::new(r#"{"port": 0}"#).unwrap();
    assert_eq!(
        dspi_mcp_relay_stdio(port.as_ptr(), None, ptr::null_mut()),
        2
    );
}
