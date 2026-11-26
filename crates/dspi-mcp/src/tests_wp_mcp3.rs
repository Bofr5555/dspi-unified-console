//! WP MCP3: the transports. The loopback listener over real sockets (both
//! eras, the Origin, Host, token and bound rules), the per-client tokens and
//! pairing, the store, and the stdio relay against a running listener.
#![allow(clippy::arithmetic_side_effects)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dspi_core::{CoreSet, DeviceTarget};
use serde_json::{Value, json};

use crate::auth::{
    Clients, MAX_CLIENTS, PAIRING_TTL_MS, PairEnd, PairPoll, Pairing, STORE_FILE, TOKEN_PREFIX,
};
use crate::http::{Event, HEADER_MISMATCH, HttpConfig, HttpLimits, HttpServer, PAUSED_TEXT};
use crate::protocol::UNSUPPORTED_VERSION;
use crate::relay::{self, MemoryTokenKeeper, NOT_RUNNING_TEXT, RELAY_ERROR, RelayConfig};
use crate::{Server, ServerConfig};

const MODERN: &str = "2026-07-28";

/// A folder beside the test binary (never tmpfs).
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::current_exe()
        .unwrap()
        .with_file_name("test-scratch")
        .join(format!("mcp3-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn demo_server() -> Server {
    let set = CoreSet::new();
    set.open(DeviceTarget::Demo).unwrap();
    Server::new(set, ServerConfig::default())
}

fn config(store: Option<PathBuf>) -> HttpConfig {
    HttpConfig {
        port: 0,
        store_dir: store,
        limits: HttpLimits {
            pair_wait: Duration::from_millis(400),
            ..HttpLimits::default()
        },
    }
}

fn start() -> HttpServer {
    HttpServer::start(demo_server(), config(None), None).unwrap()
}

/// One HTTP exchange: (status, headers, body). `host` None sends none.
struct Http {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Http {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn raw(port: u16, request: &[u8]) -> Http {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    s.write_all(request).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_owned(), v.trim().to_owned()))
        .collect();
    Http {
        status,
        headers,
        body: body.to_owned(),
    }
}

fn request(
    port: u16,
    method: &str,
    path: &str,
    host: Option<&str>,
    headers: &[(&str, &str)],
    body: &str,
) -> Http {
    let mut head = format!("{method} {path} HTTP/1.1\r\n");
    if let Some(h) = host {
        head.push_str(&format!("Host: {h}\r\n"));
    }
    head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    for (n, v) in headers {
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    head.push_str("\r\n");
    head.push_str(body);
    raw(port, head.as_bytes())
}

fn post(port: u16, token: &str, extra: &[(&str, &str)], body: &Value) -> Http {
    let auth = format!("Bearer {token}");
    let host = format!("127.0.0.1:{port}");
    let mut headers = vec![
        ("Content-Type", "application/json"),
        ("Authorization", auth.as_str()),
    ];
    headers.extend_from_slice(extra);
    request(
        port,
        "POST",
        "/mcp",
        Some(&host),
        &headers,
        &body.to_string(),
    )
}

fn meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "test-agent", "version": "1"},
    })
}

/// A modern request with the headers it needs.
fn modern(port: u16, token: &str, method: &str, mut params: Value) -> Http {
    params["_meta"] = meta();
    let name = params
        .get("name")
        .or_else(|| params.get("uri"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut h = vec![("MCP-Protocol-Version", MODERN), ("Mcp-Method", method)];
    if let Some(n) = &name {
        h.push(("Mcp-Name", n.as_str()));
    }
    post(
        port,
        token,
        &h,
        &json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
    )
}

fn initialize(port: u16, token: &str) -> Http {
    post(
        port,
        token,
        &[],
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": {"name": "legacy-agent", "version": "1"}}}),
    )
}

#[test]
fn wp_mcp3_an_http_round_trip_in_both_eras() {
    let http = start();
    let port = http.port();
    let token = http.add_client("Claude Code").unwrap().token;
    assert!(token.starts_with(TOKEN_PREFIX));

    // Legacy: initialize mints a session, which the next requests carry.
    let r = initialize(port, &token);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.header("Content-Type"), Some("application/json"));
    assert_eq!(r.json()["result"]["protocolVersion"], "2025-11-25");
    let session = r.header("Mcp-Session-Id").unwrap().to_owned();
    assert_eq!(session.len(), 32);
    let s = [
        ("Mcp-Session-Id", session.as_str()),
        ("MCP-Protocol-Version", "2025-11-25"),
    ];
    let r = post(
        port,
        &token,
        &s,
        &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    );
    assert_eq!((r.status, r.body.as_str()), (202, ""));
    let r = post(
        port,
        &token,
        &s,
        &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    );
    assert_eq!(r.status, 200);
    assert!(r.json()["result"]["tools"].as_array().unwrap().len() >= 12);

    // Modern: no session, the headers say what the body says.
    let r = modern(port, &token, "server/discover", json!({}));
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["result"]["resultType"], "complete");
    assert!(r.header("Mcp-Session-Id").is_none());
    let r = modern(
        port,
        &token,
        "tools/call",
        json!({"name": "dspi_list_devices", "arguments": {}}),
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["result"]["isError"], json!(false));
    let devices = &r.json()["result"]["structuredContent"]["devices"];
    assert_eq!(devices.as_array().unwrap().len(), 1, "{devices}");

    // Both eras were one client: one actor, one activity trail.
    let activity = http.with_server(|s| s.activity(10));
    assert_eq!(activity.len(), 1);
    assert_eq!(activity[0].tool, "dspi_list_devices");
    http.stop();
}

#[test]
fn wp_mcp3_a_request_with_an_origin_is_refused() {
    let http = start();
    let port = http.port();
    let token = http.add_client("agent").unwrap().token;
    let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
        "name": "dspi_list_devices", "arguments": {}, "_meta": meta()}});
    for origin in ["http://127.0.0.1", "https://evil.example", "null", ""] {
        let r = post(
            port,
            &token,
            &[
                ("Origin", origin),
                ("MCP-Protocol-Version", MODERN),
                ("Mcp-Method", "tools/call"),
                ("Mcp-Name", "dspi_list_devices"),
            ],
            &call,
        );
        assert_eq!(r.status, 403, "Origin {origin:?}");
        // A JSON-RPC error without an id, as the spec allows.
        assert_eq!(r.json()["id"], Value::Null);
    }
    // The pairing door too.
    let host = format!("127.0.0.1:{port}");
    let r = request(
        port,
        "POST",
        "/pair",
        Some(&host),
        &[
            ("Content-Type", "application/json"),
            ("Origin", "https://x.example"),
        ],
        "{\"name\":\"x\"}",
    );
    assert_eq!(r.status, 403);
    assert!(http.pairing().is_none());
    // Nothing reached the server.
    assert!(http.with_server(|s| s.activity(10)).is_empty());
}

#[test]
fn wp_mcp3_a_bad_host_is_refused() {
    let http = start();
    let port = http.port();
    let token = http.add_client("agent").unwrap().token;
    let auth = format!("Bearer {token}");
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}}}).to_string();
    let h = [
        ("Content-Type", "application/json"),
        ("Authorization", auth.as_str()),
    ];
    let other = format!("127.0.0.1:{}", port.wrapping_add(1));
    for host in [
        Some("evil.example"),
        Some("evil.example:47860"),
        Some("192.168.1.10"),
        Some("127.0.0.1:x"),
        Some("::1"),
        None,
    ] {
        let r = request(port, "POST", "/mcp", host, &h, &body);
        assert_eq!(r.status, 403, "Host {host:?}");
    }
    // Two Host headers.
    let both = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nHost: evil.example\r\nContent-Type: application/json\r\nAuthorization: {auth}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    assert_eq!(raw(port, both.as_bytes()).status, 403);
    // The loopback names pass, with any port (an SSH tunnel's) or none.
    for host in [
        other,
        format!("127.0.0.1:{port}"),
        "localhost".into(),
        format!("LOCALHOST:{port}"),
        format!("[::1]:{port}"),
    ] {
        let r = request(port, "POST", "/mcp", Some(&host), &h, &body);
        assert_eq!(r.status, 200, "Host {host}: {}", r.body);
    }
}

#[test]
fn wp_mcp3_a_wrong_token_gets_one_constant_time_refusal() {
    let http = start();
    let port = http.port();
    let right = http.add_client("agent").unwrap().token;
    let call = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});
    let mut refusals = Vec::new();
    let wrong_last = format!(
        "{}{}",
        &right[..right.len() - 1],
        if right.ends_with('A') { 'B' } else { 'A' }
    );
    for token in [
        "",
        "x",
        &right[..right.len() - 1],
        wrong_last.as_str(),
        &format!("{right}x"),
        "dspi-mcp-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    ] {
        let r = post(port, token, &[], &call);
        assert_eq!(r.status, 401, "token {token:?}");
        assert_eq!(
            r.header("WWW-Authenticate"),
            Some("Bearer realm=\"DSPi Console\"")
        );
        refusals.push(r.body);
    }
    // No header at all.
    let host = format!("127.0.0.1:{port}");
    let r = request(
        port,
        "POST",
        "/mcp",
        Some(&host),
        &[("Content-Type", "application/json")],
        &call.to_string(),
    );
    assert_eq!(r.status, 401);
    refusals.push(r.body);
    // One refusal, word for word, whatever was wrong: nothing about how close
    // a guess came.
    assert!(refusals.windows(2).all(|w| w[0] == w[1]), "{refusals:?}");
    assert_eq!(
        post(port, &right, &[], &call).status,
        200,
        "the right token passes"
    );

    // `verify` looks at every stored hash, and finds each client's own.
    let mut clients = Clients::in_memory();
    let minted: Vec<_> = (0..MAX_CLIENTS)
        .map(|i| clients.mint(&format!("c{i}"), 0).unwrap())
        .collect();
    assert!(clients.mint("one too many", 0).is_err());
    for m in &minted {
        assert_eq!(clients.verify(&m.token, 5), Some(m.id));
    }
    assert_eq!(clients.verify("", 5), None);
    assert!(clients.list().iter().all(|c| c.last_used_s == Some(5)));
    // A revoked token stops at once.
    assert!(http.revoke(1));
    assert_eq!(post(port, &right, &[], &call).status, 401);
}

#[test]
fn wp_mcp3_pairing_allow_mints_the_token_and_deny_refuses() {
    let (tx, rx) = mpsc::channel::<Event>();
    let tx = Mutex::new(tx);
    let listener: crate::http::Listener = Arc::new(move |e: &Event| {
        let _ = tx.lock().unwrap().send(e.clone());
    });
    let http = Arc::new(HttpServer::start(demo_server(), config(None), Some(listener)).unwrap());
    let port = http.port();
    let host = format!("127.0.0.1:{port}");
    let json_h = [("Content-Type", "application/json")];
    let pair = |name: &str| {
        request(
            port,
            "POST",
            "/pair",
            Some(&host),
            &json_h,
            &json!({"name": name}).to_string(),
        )
    };
    let wait = |ticket: &str| {
        request(
            port,
            "POST",
            "/pair/wait",
            Some(&host),
            &json_h,
            &json!({"ticket": ticket}).to_string(),
        )
    };

    // Allow.
    let r = pair("Claude Desktop\u{202E}");
    assert_eq!(r.status, 200, "{}", r.body);
    let asked = r.json();
    let code = asked["code"].as_str().unwrap().to_owned();
    assert!(code.len() == 4 && code.bytes().all(|b| b.is_ascii_digit()));
    let ticket = asked["ticket"].as_str().unwrap().to_owned();
    let Event::PairingRequested {
        id,
        name,
        code: shown,
    } = rx.recv_timeout(Duration::from_secs(5)).unwrap()
    else {
        panic!("no pairing event")
    };
    assert_eq!(
        (name.as_str(), shown.as_str()),
        ("Claude Desktop", code.as_str()),
        "the name is cleaned"
    );
    assert_eq!(http.pairing(), Some((id, name.clone(), code.clone())));
    assert!(
        http.clients().is_empty(),
        "no client before the user answers"
    );
    // One request waits at a time.
    assert_eq!(pair("someone else").status, 409);
    // Before the answer: pending (after pair_wait), a wrong ticket learns nothing.
    assert_eq!(wait(&ticket).json()["status"], "pending");
    assert_eq!(wait("not-the-ticket").status, 410);
    assert!(http.pairing_answer(id, true).unwrap());
    assert!(!http.pairing_answer(id, true).unwrap(), "answered once");
    let r = wait(&ticket);
    assert_eq!(r.status, 200);
    let token = r.json()["token"].as_str().unwrap().to_owned();
    assert!(token.starts_with(TOKEN_PREFIX));
    assert_eq!(wait(&ticket).status, 410, "the token is handed out once");
    let clients = http.clients();
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].name, "Claude Desktop");
    assert_eq!(
        initialize(port, &token).status,
        200,
        "the minted token works"
    );
    let events: Vec<Event> = rx.try_iter().collect();
    assert!(events.contains(&Event::ClientsChanged));
    assert!(events.contains(&Event::PairingEnded {
        id,
        outcome: PairEnd::Paired
    }));

    // Deny.
    let r = pair("intruder");
    let ticket = r.json()["ticket"].as_str().unwrap().to_owned();
    let id2 = r.json()["request"].as_u64().unwrap();
    // The user answers while the relay waits.
    let answerer = {
        let http = Arc::clone(&http);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            http.pairing_answer(id2, false).unwrap()
        })
    };
    let r = wait(&ticket);
    assert!(answerer.join().unwrap());
    assert_eq!(
        (r.status, r.json()["status"].clone()),
        (403, json!("denied"))
    );
    assert_eq!(http.clients().len(), 1, "a denial mints nothing");
    assert!(rx.try_iter().any(|e| e
        == Event::PairingEnded {
            id: id2,
            outcome: PairEnd::Denied
        }));

    // Three requests a minute: the fourth is refused.
    let r = pair("third");
    assert_eq!(r.status, 200);
    let id3 = r.json()["request"].as_u64().unwrap();
    assert!(http.pairing_answer(id3, false).unwrap());
    wait(r.json()["ticket"].as_str().unwrap());
    assert_eq!(pair("fourth").status, 429);
}

#[test]
fn wp_mcp3_a_token_not_picked_up_in_time_is_revoked() {
    let mut clients = Clients::in_memory();
    let mut desk = Pairing::default();
    let r = desk.request("slow relay", 1_000, &mut clients).unwrap();
    assert!(desk.answer(r.id, true, 2_000, 0, &mut clients).unwrap());
    assert_eq!(clients.list().len(), 1);
    // Nobody polls; the time runs out.
    let ended = desk.expire(1_000 + PAIRING_TTL_MS, &mut clients);
    assert_eq!(ended, Some((r.id, PairEnd::Expired)));
    assert!(clients.list().is_empty(), "the orphan token is revoked");
    assert_eq!(
        desk.poll(&r.ticket, 1_000 + PAIRING_TTL_MS, &mut clients).0,
        PairPoll::Gone
    );
    // An unanswered request expires the same way, and frees the desk.
    let r = desk.request("again", 200_000, &mut clients).unwrap();
    assert_eq!(desk.waiting(200_000 + PAIRING_TTL_MS, &mut clients), None);
    assert!(
        !desk
            .answer(r.id, true, 200_000 + PAIRING_TTL_MS, 0, &mut clients)
            .unwrap()
    );
    assert!(clients.list().is_empty());
}

#[test]
fn wp_mcp3_the_store_keeps_only_hashes() {
    let dir = scratch("store");
    let http = HttpServer::start(demo_server(), config(Some(dir.clone())), None).unwrap();
    let minted = http.add_client("Cursor").unwrap();
    let port = http.port();
    assert_eq!(initialize(port, &minted.token).status, 200);
    http.stop();
    let text = std::fs::read_to_string(dir.join(STORE_FILE)).unwrap();
    assert!(
        !text.contains(&minted.token),
        "the token is not in the file"
    );
    assert!(!text.contains(&minted.token[TOKEN_PREFIX.len()..]));
    let doc: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(doc["clients"][0]["name"], "Cursor");
    assert!(doc["clients"][0]["lastUsed"].is_u64(), "{doc}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join(STORE_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    // A restart knows the token.
    let mut again = Clients::load(&dir).unwrap();
    assert_eq!(again.verify(&minted.token, 9), Some(minted.id));
    // Ids are not reused after a revoke.
    assert!(again.revoke(minted.id));
    let next = again.mint("next", 0).unwrap();
    assert!(next.id > minted.id);
    // A damaged file is an error, not an empty list to write over it.
    std::fs::write(dir.join(STORE_FILE), "{not json").unwrap();
    assert!(Clients::load(&dir).is_err());
    assert!(HttpServer::start(demo_server(), config(Some(dir)), None).is_err());
}

#[test]
fn wp_mcp3_modern_headers_must_match_the_body() {
    let http = start();
    let port = http.port();
    let token = http.add_client("agent").unwrap().token;
    let body = |method: &str, params: Value| {
        let mut p = params;
        p["_meta"] = meta();
        json!({"jsonrpc": "2.0", "id": 5, "method": method, "params": p})
    };
    let list = body("tools/list", json!({}));
    for headers in [
        vec![("Mcp-Method", "tools/list")],
        vec![
            ("MCP-Protocol-Version", "2025-11-25"),
            ("Mcp-Method", "tools/list"),
        ],
        vec![("MCP-Protocol-Version", MODERN)],
        vec![
            ("MCP-Protocol-Version", MODERN),
            ("Mcp-Method", "prompts/list"),
        ],
    ] {
        let r = post(port, &token, &headers, &list);
        assert_eq!(r.status, 400, "{headers:?}");
        assert_eq!(r.json()["error"]["code"], json!(HEADER_MISMATCH));
        assert_eq!(r.json()["id"], json!(5));
    }
    let call = body(
        "tools/call",
        json!({"name": "dspi_list_devices", "arguments": {}}),
    );
    let h = [
        ("MCP-Protocol-Version", MODERN),
        ("Mcp-Method", "tools/call"),
        ("Mcp-Name", "dspi_get_overview"),
    ];
    assert_eq!(
        post(port, &token, &h, &call).json()["error"]["code"],
        json!(HEADER_MISMATCH)
    );
    let h = [
        ("MCP-Protocol-Version", MODERN),
        ("Mcp-Method", "tools/call"),
    ];
    assert_eq!(
        post(port, &token, &h, &call).status,
        400,
        "Mcp-Name is required for tools/call"
    );
    // An unsupported version: 400 with -32022.
    let mut future = list.clone();
    future["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!("2099-01-01");
    let r = post(
        port,
        &token,
        &[
            ("MCP-Protocol-Version", "2099-01-01"),
            ("Mcp-Method", "tools/list"),
        ],
        &future,
    );
    assert_eq!(
        (r.status, r.json()["error"]["code"].clone()),
        (400, json!(UNSUPPORTED_VERSION))
    );
    // No capabilities: 400. An unknown method: 404.
    let mut bare = list.clone();
    bare["params"]["_meta"] = json!({"io.modelcontextprotocol/protocolVersion": MODERN});
    assert_eq!(
        post(
            port,
            &token,
            &[
                ("MCP-Protocol-Version", MODERN),
                ("Mcp-Method", "tools/list")
            ],
            &bare
        )
        .status,
        400
    );
    assert_eq!(modern(port, &token, "nothing/here", json!({})).status, 404);
    // A modern notification: 202.
    let mut note =
        json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 1}});
    note["params"]["_meta"] = meta();
    let r = post(
        port,
        &token,
        &[
            ("MCP-Protocol-Version", MODERN),
            ("Mcp-Method", "notifications/cancelled"),
        ],
        &note,
    );
    assert_eq!((r.status, r.body.as_str()), (202, ""));
}

#[test]
fn wp_mcp3_legacy_sessions_belong_to_their_token() {
    let http = start();
    let port = http.port();
    let a = http.add_client("a").unwrap().token;
    let b = http.add_client("b").unwrap().token;
    let session = initialize(port, &a)
        .header("Mcp-Session-Id")
        .unwrap()
        .to_owned();
    let list = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
    assert_eq!(post(port, &a, &[], &list).status, 400, "no session");
    assert_eq!(
        post(port, &a, &[("Mcp-Session-Id", "0123")], &list).status,
        404,
        "no such session"
    );
    assert_eq!(
        post(port, &b, &[("Mcp-Session-Id", &session)], &list).status,
        404,
        "another token's session"
    );
    assert_eq!(
        post(
            port,
            &a,
            &[
                ("Mcp-Session-Id", &session),
                ("MCP-Protocol-Version", "1999-01-01")
            ],
            &list
        )
        .status,
        400
    );
    assert_eq!(
        post(port, &a, &[("Mcp-Session-Id", &session)], &list).status,
        200
    );
    // DELETE ends it.
    let host = format!("127.0.0.1:{port}");
    let auth = format!("Bearer {a}");
    let r = request(
        port,
        "DELETE",
        "/mcp",
        Some(&host),
        &[("Authorization", &auth), ("Mcp-Session-Id", &session)],
        "",
    );
    assert_eq!(r.status, 204);
    assert_eq!(
        post(port, &a, &[("Mcp-Session-Id", &session)], &list).status,
        404
    );
    // A bounded number of sessions per token: the oldest goes.
    let first = initialize(port, &a)
        .header("Mcp-Session-Id")
        .unwrap()
        .to_owned();
    for _ in 0..HttpLimits::default().sessions_per_client {
        initialize(port, &a);
    }
    assert_eq!(
        post(port, &a, &[("Mcp-Session-Id", &first)], &list).status,
        404
    );
}

#[test]
fn wp_mcp3_the_bounds_hold() {
    let http = HttpServer::start(
        demo_server(),
        HttpConfig {
            port: 0,
            store_dir: None,
            limits: HttpLimits {
                body_bytes: 1024,
                head_bytes: 2048,
                io_timeout: Duration::from_millis(500),
                ..HttpLimits::default()
            },
        },
        None,
    )
    .unwrap();
    let port = http.port();
    let token = http.add_client("agent").unwrap().token;
    let host = format!("127.0.0.1:{port}");
    let auth = format!("Bearer {token}");
    let json_h = [
        ("Content-Type", "application/json"),
        ("Authorization", auth.as_str()),
    ];
    let big = format!("{{\"x\":\"{}\"}}", "a".repeat(2000));
    assert_eq!(
        request(port, "POST", "/mcp", Some(&host), &json_h, &big).status,
        413
    );
    let r = request(
        port,
        "POST",
        "/mcp",
        Some(&host),
        &[("Content-Type", "text/plain"), ("Authorization", &auth)],
        "{}",
    );
    assert_eq!(r.status, 415);
    assert_eq!(
        request(
            port,
            "GET",
            "/mcp",
            Some(&host),
            &[("Authorization", &auth)],
            ""
        )
        .status,
        405
    );
    assert_eq!(
        request(port, "POST", "/elsewhere", Some(&host), &json_h, "{}").status,
        404
    );
    let no_length = format!(
        "POST /mcp HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nAuthorization: {auth}\r\n\r\n"
    );
    assert_eq!(raw(port, no_length.as_bytes()).status, 411);
    let chunked = format!(
        "POST /mcp HTTP/1.1\r\nHost: {host}\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\n\r\n2\r\n{{}}\r\n0\r\n\r\n"
    );
    assert_eq!(raw(port, chunked.as_bytes()).status, 411);
    let long = format!(
        "POST /mcp HTTP/1.1\r\nHost: {host}\r\nX-Pad: {}\r\n\r\n",
        "p".repeat(4000)
    );
    assert_eq!(raw(port, long.as_bytes()).status, 431);
    // A sender that stops halfway gets 408 and the connection back.
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(b"POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\n")
        .unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    assert!(out.starts_with("HTTP/1.1 408"), "{out}");
    assert_eq!(
        request(port, "POST", "/mcp", Some(&host), &json_h, "{nope").status,
        400
    );
}

#[test]
fn wp_mcp3_no_listener_on_any_other_address() {
    let http = start();
    let port = http.port();
    assert!(!http.addrs().is_empty());
    for a in http.addrs() {
        assert!(a.ip().is_loopback(), "{a}");
        assert_eq!(a.port(), port);
    }
    // This machine's own non-loopback address, if it has one, has no listener.
    let own = UdpSocket::bind("0.0.0.0:0")
        .and_then(|u| u.connect("192.0.2.1:9").map(|()| u))
        .and_then(|u| u.local_addr())
        .map(|a| a.ip())
        .ok()
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified());
    if let Some(ip) = own {
        let r = TcpStream::connect_timeout(&SocketAddr::new(ip, port), Duration::from_secs(2));
        assert!(r.is_err(), "something answers on {ip}:{port}");
    } else {
        eprintln!("wp_mcp3: no non-loopback address here; checked the bound addresses only");
    }
    let status = http.status();
    assert!(status["addresses"].as_array().unwrap().iter().all(|a| {
        let s = a.as_str().unwrap();
        s.starts_with("127.0.0.1:") || s.starts_with("[::1]:")
    }));
}

#[test]
fn wp_mcp3_paused_agents_change_nothing() {
    let http = start();
    let port = http.port();
    let token = http.add_client("agent").unwrap().token;
    http.set_paused(true);
    let r = modern(
        port,
        &token,
        "tools/call",
        json!({"name": "dspi_list_devices", "arguments": {}}),
    );
    assert_eq!(r.status, 200);
    // -- WP AIVIS -- said as paused, not as an error
    assert_eq!(r.json()["result"]["isError"], json!(false));
    assert_eq!(
        r.json()["result"]["structuredContent"]["status"],
        json!("paused")
    );
    assert_eq!(r.json()["result"]["content"][0]["text"], PAUSED_TEXT);
    assert!(
        http.with_server(|s| s.activity(10)).is_empty(),
        "the call never reached a tool"
    );
    // Reading the lists still works.
    assert_eq!(modern(port, &token, "tools/list", json!({})).status, 200);
    http.set_paused(false);
    let r = modern(
        port,
        &token,
        "tools/call",
        json!({"name": "dspi_list_devices", "arguments": {}}),
    );
    assert_eq!(r.json()["result"]["isError"], json!(false));
}

/// Run the relay over `lines` against `port`; (stdout lines, stderr).
fn run_relay(port: u16, keeper: &mut MemoryTokenKeeper, lines: &[Value]) -> (Vec<Value>, String) {
    let input: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let mut input = std::io::Cursor::new(input.into_bytes());
    let mut out = Vec::new();
    let mut err = Vec::new();
    let cfg = RelayConfig {
        port,
        timeout: Duration::from_secs(20),
        ..RelayConfig::default()
    };
    relay::run(cfg, keeper, &mut input, &mut out, &mut err).unwrap();
    let out = String::from_utf8(out).unwrap();
    // -- WP UAG -- the replies; the relay's own log notices (the pairing code) aside
    let replies = out
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|v| v["method"] != json!("notifications/message"))
        .collect();
    (replies, String::from_utf8(err).unwrap())
}

#[test]
fn wp_mcp3_the_relay_forwards_stdio_to_http() {
    let dir = scratch("relay");
    // The user allows whatever asks.
    let (tx, rx) = mpsc::channel::<u64>();
    let tx = Mutex::new(tx);
    let listener: crate::http::Listener = Arc::new(move |e: &Event| {
        if let Event::PairingRequested { id, .. } = e {
            let _ = tx.lock().unwrap().send(*id);
        }
    });
    let http = Arc::new(
        HttpServer::start(demo_server(), config(Some(dir.clone())), Some(listener)).unwrap(),
    );
    let port = http.port();
    // Weak: the test takes the listener back to restart it.
    let answerer = {
        let http = Arc::downgrade(&http);
        std::thread::spawn(move || {
            while let Ok(id) = rx.recv_timeout(Duration::from_secs(30)) {
                if let Some(http) = http.upgrade() {
                    let _ = http.pairing_answer(id, true);
                }
            }
        })
    };
    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-11-25", "capabilities": {},
        "clientInfo": {"name": "claude-ai", "title": "Claude Desktop", "version": "1"}}});
    let mut discover =
        json!({"jsonrpc": "2.0", "id": 4, "method": "server/discover", "params": {}});
    discover["params"]["_meta"] = meta();
    let mut call = json!({"jsonrpc": "2.0", "id": "c", "method": "tools/call", "params": {"name": "dspi_get_overview", "arguments": {}}});
    call["params"]["_meta"] = meta();
    let mut keeper = MemoryTokenKeeper::default();
    let (replies, err) = run_relay(
        port,
        &mut keeper,
        &[
            init.clone(),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            discover,
            call,
        ],
    );
    // One reply per request, in order, none for the notification.
    let ids: Vec<Value> = replies.iter().map(|r| r["id"].clone()).collect();
    assert_eq!(
        ids,
        vec![json!(1), json!(2), json!(4), json!("c")],
        "{replies:?}\n{err}"
    );
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-11-25");
    // -- WP UAG -- the relay declares its log notice (the pairing code)
    assert_eq!(replies[0]["result"]["capabilities"]["logging"], json!({}));
    assert!(replies[1]["result"]["tools"].is_array());
    assert_eq!(replies[2]["result"]["resultType"], "complete");
    assert_eq!(
        replies[3]["result"]["isError"],
        json!(false),
        "{}",
        replies[3]
    );
    // It paired under the agent's own name and printed the code.
    let token = keeper.token.clone().expect("the token is kept");
    assert_eq!(http.clients()[0].name, "Claude Desktop");
    assert!(err.contains("code "), "{err}");
    assert!(!err.contains(&token), "the token is never printed");

    // Next run: the kept token, no pairing.
    let (replies, err) = run_relay(
        port,
        &mut keeper,
        &[
            init.clone(),
            json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}),
        ],
    );
    assert_eq!(replies.len(), 2);
    assert!(!err.contains("code "), "{err}");
    assert_eq!(http.clients().len(), 1);

    // A revoked token: the relay pairs again and goes on.
    http.revoke(http.clients()[0].id);
    let (replies, _) = run_relay(port, &mut keeper, std::slice::from_ref(&init));
    assert_eq!(replies[0]["id"], json!(1));
    assert!(replies[0].get("result").is_some(), "{}", replies[0]);
    assert_ne!(keeper.token.as_deref(), Some(token.as_str()));
    assert_eq!(http.clients().len(), 1);

    // The app restarts between two requests: the relay starts a new session.
    let Ok(http) = Arc::try_unwrap(http) else {
        panic!("still shared")
    };
    drop(answerer);
    let input_lines = [
        init,
        json!({"jsonrpc": "2.0", "id": 9, "method": "tools/list"}),
    ];
    let input: String = input_lines.iter().map(|l| format!("{l}\n")).collect();
    // The relay reads line by line, so a pipe lets the test restart the app
    // between the two.
    let (mut writer, reader) = pipe();
    let relay_thread = {
        let mut keeper = keeper.clone();
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(reader);
            let mut out = Vec::new();
            let mut err = Vec::new();
            let cfg = RelayConfig {
                port,
                timeout: Duration::from_secs(20),
                ..RelayConfig::default()
            };
            relay::run(cfg, &mut keeper, &mut reader, &mut out, &mut err).unwrap();
            String::from_utf8(out).unwrap()
        })
    };
    let mut lines = input.lines();
    writeln!(writer, "{}", lines.next().unwrap()).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    http.stop();
    let mut restarted = HttpConfig {
        port,
        ..config(Some(dir))
    };
    restarted.limits.pair_wait = Duration::from_millis(400);
    let http = HttpServer::start(demo_server(), restarted, None).unwrap();
    writeln!(writer, "{}", lines.next().unwrap()).unwrap();
    drop(writer);
    let out = relay_thread.join().unwrap();
    let replies: Vec<Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(replies.len(), 2, "{out}");
    assert!(replies[1]["result"]["tools"].is_array(), "{}", replies[1]);

    // The app gone: the agent is told what to do, and the relay keeps going.
    http.stop();
    let mut keeper2 = keeper.clone();
    let (replies, _) = run_relay(
        port,
        &mut keeper2,
        &[
            json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}),
        ],
    );
    assert_eq!(replies.len(), 2);
    for r in &replies {
        assert_eq!(r["error"]["code"], json!(RELAY_ERROR));
        assert_eq!(r["error"]["message"], NOT_RUNNING_TEXT);
    }
    // A line that is not JSON gets a parse error, not silence.
    let mut input = std::io::Cursor::new(b"not json\n".to_vec());
    let mut out = Vec::new();
    relay::run(
        RelayConfig {
            port,
            ..RelayConfig::default()
        },
        &mut keeper2,
        &mut input,
        &mut out,
        &mut Vec::new(),
    )
    .unwrap();
    let r: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(r["error"]["code"], json!(-32700));
}

#[test]
fn wp_mcp3_the_relay_reports_a_denial() {
    let (tx, rx) = mpsc::channel::<u64>();
    let tx = Mutex::new(tx);
    let listener: crate::http::Listener = Arc::new(move |e: &Event| {
        if let Event::PairingRequested { id, .. } = e {
            let _ = tx.lock().unwrap().send(*id);
        }
    });
    let http = Arc::new(HttpServer::start(demo_server(), config(None), Some(listener)).unwrap());
    let port = http.port();
    let answerer = {
        let http = Arc::clone(&http);
        std::thread::spawn(move || {
            if let Ok(id) = rx.recv_timeout(Duration::from_secs(30)) {
                let _ = http.pairing_answer(id, false);
            }
        })
    };
    let mut keeper = MemoryTokenKeeper::default();
    let (replies, err) = run_relay(
        port,
        &mut keeper,
        &[
            json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}),
        ],
    );
    answerer.join().unwrap();
    assert_eq!(replies.len(), 2);
    assert!(
        replies[0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("denied"),
        "{}",
        replies[0]
    );
    // It does not ask again at once.
    assert!(
        replies[1]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("a moment ago"),
        "{}",
        replies[1]
    );
    assert!(err.contains("denied"));
    assert!(keeper.token.is_none());
    assert!(http.clients().is_empty());
}

/// An in-process pipe: a writer and a blocking reader.
fn pipe() -> (PipeWriter, PipeReader) {
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    (
        PipeWriter(tx),
        PipeReader {
            rx,
            buf: Vec::new(),
        },
    )
}

struct PipeWriter(mpsc::Sender<Vec<u8>>);

impl Write for PipeWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0
            .send(b.to_vec())
            .map_err(|_| std::io::ErrorKind::BrokenPipe)?;
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct PipeReader {
    rx: mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
}

impl Read for PipeReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.buf.is_empty() {
            match self.rx.recv() {
                Ok(b) => self.buf = b,
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len());
        out[..n].copy_from_slice(&self.buf[..n]);
        self.buf.drain(..n);
        Ok(n)
    }
}

// -- WP FFIX -- FUNC1 bug 7: one token per agent program
#[test]
fn wp_ffix_each_agent_program_on_the_computer_pairs_as_itself() {
    let (tx, rx) = mpsc::channel::<u64>();
    let tx = Mutex::new(tx);
    let listener: crate::http::Listener = Arc::new(move |e: &Event| {
        if let Event::PairingRequested { id, .. } = e {
            let _ = tx.lock().unwrap().send(*id);
        }
    });
    let http = Arc::new(HttpServer::start(demo_server(), config(None), Some(listener)).unwrap());
    let port = http.port();
    let answerer = {
        let http = Arc::downgrade(&http);
        std::thread::spawn(move || {
            while let Ok(id) = rx.recv_timeout(Duration::from_secs(10)) {
                if let Some(http) = http.upgrade() {
                    let _ = http.pairing_answer(id, true);
                }
            }
        })
    };
    let init = |name: &str, title: &str| {
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": {"name": name, "title": title, "version": "1"}}})
    };
    // One keeper, as the app's one secret store: two programs, two clients.
    let mut keeper = MemoryTokenKeeper::default();
    let (r, err) = run_relay(port, &mut keeper, &[init("claude-ai", "Claude Desktop")]);
    assert!(r[0].get("result").is_some(), "{r:?}");
    assert!(err.contains("code "), "{err}");
    let (r, err) = run_relay(port, &mut keeper, &[init("cursor-vscode", "Cursor")]);
    assert!(r[0].get("result").is_some(), "{r:?}");
    assert!(
        err.contains("code "),
        "a second program is asked about: {err}"
    );
    let mut names: Vec<String> = http.clients().into_iter().map(|c| c.name).collect();
    names.sort();
    assert_eq!(names, ["Claude Desktop", "Cursor"]);
    assert_eq!(keeper.tokens.len(), 2);
    assert_ne!(keeper.tokens["claude-ai"], keeper.tokens["cursor-vscode"]);
    // Each comes back with its own token, unasked.
    let (_, err) = run_relay(port, &mut keeper, &[init("claude-ai", "Claude Desktop")]);
    assert!(!err.contains("code "), "{err}");
    assert_eq!(http.clients().len(), 2);
    // The kept text: a token per program, and an old single token is no one's.
    let text = crate::relay::tokens_text(&keeper.tokens);
    assert_eq!(crate::relay::tokens_of(&text), keeper.tokens);
    assert!(crate::relay::tokens_of("abcdef0123").is_empty());
    http.stop();
    drop(answerer);
}

// -- WP UAG -- (UXP A9) the pairing code where the person can check it: the
// agent host's log (an MCP log notice) and, when a tool call starts the
// pairing, that call's result, which the agent's chat shows at once
#[test]
fn wp_uag_the_relay_shows_the_pairing_code_in_the_log_and_the_chat() {
    let (tx, rx) = mpsc::channel::<u64>();
    let tx = Mutex::new(tx);
    let listener: crate::http::Listener = Arc::new(move |e: &Event| {
        if let Event::PairingRequested { id, .. } = e {
            let _ = tx.lock().unwrap().send(*id);
        }
    });
    let http = Arc::new(HttpServer::start(demo_server(), config(None), Some(listener)).unwrap());
    let port = http.port();
    let answerer = {
        let http = Arc::clone(&http);
        std::thread::spawn(move || {
            if let Ok(id) = rx.recv_timeout(Duration::from_secs(30)) {
                std::thread::sleep(Duration::from_millis(300));
                let _ = http.pairing_answer(id, true);
            }
        })
    };
    let mut call = json!({"jsonrpc": "2.0", "id": "c", "method": "tools/call",
                          "params": {"name": "dspi_get_overview", "arguments": {}}});
    call["params"]["_meta"] = meta();
    let mut again = call.clone();
    again["id"] = json!("d");
    let lines = [
        call,
        json!({"jsonrpc": "2.0", "id": 5, "method": "logging/setLevel", "params": {"level": "info"}}),
        again,
    ];
    let input: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let mut input = std::io::Cursor::new(input.into_bytes());
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let cfg = RelayConfig {
        port,
        timeout: Duration::from_secs(20),
        ..RelayConfig::default()
    };
    let mut keeper = MemoryTokenKeeper::default();
    relay::run(cfg, &mut keeper, &mut input, &mut out, &mut err).unwrap();
    answerer.join().unwrap();
    let all: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    // The log notice first, with the code the app shows
    assert_eq!(all[0]["method"], json!("notifications/message"), "{all:?}");
    assert_eq!(all[0]["params"]["level"], json!("notice"));
    let data = all[0]["params"]["data"].as_str().unwrap();
    let code: String = data
        .split_once("the code ")
        .map(|(_, r)| r.chars().take_while(char::is_ascii_digit).collect())
        .unwrap_or_default();
    assert!(!code.is_empty(), "{data}");
    assert!(String::from_utf8(err).unwrap().contains(&code));
    // The call that started it: answered at once, naming the code, for the chat
    assert_eq!(all[1]["id"], json!("c"));
    assert_eq!(all[1]["result"]["isError"], json!(true));
    let text = all[1]["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains(&format!("the code {code}")) && text.contains("click Allow"),
        "{text}"
    );
    // The log level is the relay's own
    assert_eq!(all[2], json!({"jsonrpc": "2.0", "id": 5, "result": {}}));
    // The next call waits for the person's Allow, then goes through
    assert_eq!(all[3]["id"], json!("d"));
    assert_eq!(all[3]["result"]["isError"], json!(false), "{}", all[3]);
    assert!(keeper.token.is_some());
    assert_eq!(all.len(), 4, "{all:?}");
}
