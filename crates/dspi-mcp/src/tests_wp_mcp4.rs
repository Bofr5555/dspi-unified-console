//! WP MCP4: the agent policy over the HTTP transport, end to end with a fake
//! agent: calls held for the user and answered while parked off their
//! connections' threads, the hold's end and the poll, an agent that goes away
//! or cancels, the server's own clocks, Stop, and the endpoint's addresses.

use std::io::{Read, Write};
use std::net::{Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_core::{CoreSet, DeviceId, DeviceTarget, DspiCore};
use serde_json::{Value, json};

use crate::activity::words;
use crate::confirm::ConfirmState;
use crate::http::{Event, HttpConfig, HttpLimits, HttpServer, Listener, StartError};
use crate::policy::{AgentPolicy, ConfirmMode};
use crate::relay::{self, MemoryTokenKeeper, RelayConfig};
use crate::{Server, ServerConfig};

const MODERN: &str = "2026-07-28";

/// A server under the agent policy over one demo device, and one client.
struct Rig {
    http: HttpServer,
    core: Arc<DspiCore>,
    id: DeviceId,
    policy: Arc<AgentPolicy>,
    token: String,
    events: Arc<Mutex<Vec<Event>>>,
}

fn rig_with(limits: HttpLimits) -> Rig {
    let set = CoreSet::new();
    let id = set.open(DeviceTarget::Demo).unwrap();
    let core = set.core(&id).unwrap();
    let policy = Arc::new(AgentPolicy::new(ConfirmMode::AskRisky));
    let server = Server::new(
        set,
        ServerConfig {
            policy: policy.clone(),
            ..ServerConfig::default()
        },
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let listener: Listener = {
        let events = Arc::clone(&events);
        Arc::new(move |e: &Event| events.lock().unwrap().push(e.clone()))
    };
    let config = HttpConfig {
        port: 0,
        store_dir: None,
        limits,
    };
    let http = HttpServer::start(server, config, Some(listener)).unwrap();
    let token = http.add_client("Claude Code").unwrap().token;
    core.set_output_gain(1, -12.0).unwrap();
    Rig {
        http,
        core,
        id,
        policy,
        token,
        events,
    }
}

fn rig() -> Rig {
    rig_with(HttpLimits::default())
}

/// A modern request, as bytes.
fn bytes(port: u16, token: &str, rpc_id: &Value, method: &str, mut params: Value) -> Vec<u8> {
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "someone-else", "version": "1"},
    });
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .map(|n| format!("Mcp-Name: {n}\r\n"))
        .unwrap_or_default();
    let body =
        json!({"jsonrpc": "2.0", "id": rpc_id, "method": method, "params": params}).to_string();
    format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
         Authorization: Bearer {token}\r\nMCP-Protocol-Version: {MODERN}\r\nMcp-Method: {method}\r\n\
         {name}Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Send a request and leave the connection open.
fn send(r: &Rig, rpc_id: &Value, method: &str, params: Value) -> TcpStream {
    let port = r.http.port();
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    s.write_all(&bytes(port, &r.token, rpc_id, method, params))
        .unwrap();
    s
}

/// The reply: (status, body).
fn reply(mut s: TcpStream) -> (u16, Value) {
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out).into_owned();
    let status = text
        .split(' ')
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let body = text.split_once("\r\n\r\n").map_or("", |p| p.1);
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

fn call(r: &Rig, tool: &str, args: Value) -> TcpStream {
    send(
        r,
        &json!(7),
        "tools/call",
        json!({"name": tool, "arguments": args}),
    )
}

fn louder(r: &Rig, db: f64) -> TcpStream {
    call(r, "dspi_set_gain", json!({"output": 2, "gain_db": db}))
}

fn gain(r: &Rig) -> f32 {
    r.core.get_state().output_gain_db[1]
}

fn until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while !f() {
        assert!(Instant::now() < end, "never: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Wait for `n` confirmations; their ids.
fn waiting(r: &Rig, n: usize) -> Vec<u64> {
    let mut ids = Vec::new();
    until(&format!("{n} confirmations"), || {
        ids = r
            .http
            .confirmations()
            .iter()
            .map(|c| c["confirmation_id"].as_u64().unwrap())
            .collect();
        ids.len() == n
    });
    ids
}

fn count(r: &Rig, e: &Event) -> usize {
    r.events.lock().unwrap().iter().filter(|x| *x == e).count()
}

#[test]
fn wp_mcp4_a_held_call_is_answered_by_allow_and_deny_and_blocks_no_connection() {
    // Two connection threads in all, and three held calls.
    let r = rig_with(HttpLimits {
        max_connections: 2,
        ..HttpLimits::default()
    });
    let calls: Vec<TcpStream> = (1..=3)
        .map(|n| {
            let s = louder(&r, -12.0 + f64::from(n));
            waiting(&r, usize::try_from(n).unwrap());
            s
        })
        .collect();
    let rows = r.http.confirmations();
    assert_eq!(
        rows[0]["client_name"], "AI · Claude Code",
        "the paired name"
    );
    assert_eq!(rows[0]["title"], "Set a gain");
    assert_eq!(rows[0]["device"], json!(r.id.to_string()));
    assert_eq!(rows[0]["risks"][0]["kind"], "louder");
    assert!(rows[0]["remaining_ms"].as_u64().unwrap() > 100_000);
    assert_eq!(gain(&r), -12.0, "nothing sent");
    assert!(count(&r, &Event::ConfirmationsChanged) >= 3);

    // The held ones take no connection: others are served.
    for _ in 0..4 {
        let (status, body) = reply(send(&r, &json!(1), "tools/list", json!({})));
        assert_eq!(status, 200, "{body}");
    }
    // -- WP UB5 -- A fourth question of the same agent's is asked too (8 wait
    // at most, tests_wp_mcp2a); denied, it ends with nothing changed.
    let fourth = louder(&r, -8.5);
    let ids = waiting(&r, 4);
    r.http.answer_confirmation(ids[3], false).unwrap();
    let (_, body) = reply(fourth);
    assert_eq!(body["result"]["isError"], json!(true), "{body}");

    let ids = waiting(&r, 3);
    r.http.answer_confirmation(ids[0], true).unwrap();
    assert_eq!(gain(&r), -11.0, "Allow applies it at once");
    let mut calls = calls.into_iter();
    let (status, body) = reply(calls.next().unwrap());
    assert_eq!(status, 200);
    assert_eq!(body["id"], json!(7));
    assert_eq!(body["result"]["isError"], json!(false), "{body}");
    assert_eq!(body["result"]["resultType"], "complete");
    assert_eq!(body["result"]["structuredContent"]["ok"], json!(true));

    r.http.answer_confirmation(ids[1], false).unwrap();
    let (_, body) = reply(calls.next().unwrap());
    assert_eq!(body["result"]["isError"], json!(true));
    assert!(body.to_string().contains("declined"), "{body}");
    assert_eq!(gain(&r), -11.0);
    assert!(r.http.answer_confirmation(ids[1], true).is_err());

    // The change was the agent's, under the paired name, whatever the agent
    // calls itself.
    let entry = r.core.history(1).remove(0);
    assert_eq!(entry.actor_label.as_deref(), Some("AI · Claude Code"));
    // The log has the change in words, when, and what to undo.
    let log = r.http.activity(50);
    let done = log
        .iter()
        .find(|e| e["outcome"] == "ok")
        .expect("the allowed call");
    assert_eq!(done["detail"], "Out 2 output gain -12 dB to -11 dB");
    // -- WP SHOWFIX -- (finding 4) the app's toast has the core's words, the
    // channel as the person knows it (agents never read them: activity.rs)
    assert_eq!(done["said"], "Out 2: gain \u{2212}11 dB");
    assert_eq!(done["title"], "Set a gain");
    assert_eq!(done["journal_ids"], json!([entry.id]));
    assert!(done["at_unix_ms"].as_u64().unwrap() >= entry.at_ms - 60_000);

    // Removing the client ends its last question.
    let client = r.http.clients()[0].id;
    assert!(r.http.revoke(client));
    let (_, body) = reply(calls.next().unwrap());
    assert_eq!(body["result"]["isError"], json!(true), "{body}");
    assert!(r.http.confirmations().is_empty());
    assert_eq!(gain(&r), -11.0);
}

#[test]
fn wp_mcp4_after_the_hold_the_agent_polls() {
    let r = rig_with(HttpLimits {
        hold: Duration::from_millis(300),
        ..HttpLimits::default()
    });
    let (_, body) = reply(louder(&r, -6.0));
    let d = &body["result"]["structuredContent"];
    assert_eq!(d["status"], "awaiting_confirmation", "{body}");
    assert_eq!(body["result"]["resultType"], "complete");
    let id = d["confirmation_id"].as_u64().unwrap();
    assert_eq!(waiting(&r, 1), [id], "still asked");
    r.http.answer_confirmation(id, true).unwrap();
    assert_eq!(gain(&r), -6.0);
    let (_, body) = reply(call(
        &r,
        "dspi_get_confirmation",
        json!({"confirmation_id": id}),
    ));
    assert_eq!(body["result"]["structuredContent"]["status"], "allowed");
}

#[test]
fn wp_mcp4_an_agent_that_goes_away_or_cancels_leaves_no_question() {
    let r = rig();
    // The connection closes.
    let s = louder(&r, -6.0);
    let id = waiting(&r, 1)[0];
    drop(s);
    until("the question goes", || r.http.confirmations().is_empty());
    assert!(
        r.http.with_server(|s| s.confirmation(id).is_none()),
        "forgotten"
    );
    assert!(r.http.answer_confirmation(id, true).is_err());
    assert_eq!(gain(&r), -12.0);
    // Closed after the answer: the entry does not stay either.
    let s = louder(&r, -6.0);
    let id = waiting(&r, 1)[0];
    drop(s);
    let _ = r.http.answer_confirmation(id, false);
    until("forgotten", || {
        r.http.with_server(|s| s.confirmation(id).is_none())
    });

    // A cancel of another request leaves it; its own ends it.
    let s = louder(&r, -6.0);
    let id = waiting(&r, 1)[0];
    let port = r.http.port();
    let notify = |rpc: i64| {
        let body = json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
            "params": {"requestId": rpc, "_meta": {
                "io.modelcontextprotocol/protocolVersion": MODERN,
                "io.modelcontextprotocol/clientCapabilities": {}}}})
        .to_string();
        let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let head = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
             Authorization: Bearer {}\r\nMCP-Protocol-Version: {MODERN}\r\n\
             Mcp-Method: notifications/cancelled\r\nContent-Length: {}\r\n\r\n{body}",
            r.token,
            body.len()
        );
        c.write_all(head.as_bytes()).unwrap();
        assert_eq!(reply(c).0, 202);
    };
    notify(8);
    assert_eq!(waiting(&r, 1), [id]);
    notify(7);
    assert!(r.http.confirmations().is_empty());
    let (_, body) = reply(s);
    assert!(body.to_string().contains("cancelled"), "{body}");
    assert_eq!(gain(&r), -12.0);
}

#[test]
fn wp_mcp4_the_three_settings_over_http() {
    let r = rig();
    let quieter = |r: &Rig| call(r, "dspi_set_gain", json!({"output": 2, "gain_db": -20.0}));
    let (_, body) = reply(quieter(&r));
    assert_eq!(body["result"]["isError"], json!(false), "{body}");
    r.policy.set_mode(ConfirmMode::AskEverything);
    let s = call(&r, "dspi_set_gain", json!({"output": 2, "gain_db": -30.0}));
    let id = waiting(&r, 1)[0];
    r.http.answer_confirmation(id, true).unwrap();
    assert_eq!(reply(s).1["result"]["isError"], json!(false));
    assert_eq!(gain(&r), -30.0);
    r.policy.set_mode(ConfirmMode::NeverAsk);
    let (_, body) = reply(louder(&r, -3.0));
    assert_eq!(body["result"]["isError"], json!(false), "{body}");
    assert_eq!(gain(&r), -3.0);
    assert!(r.http.confirmations().is_empty());
}

#[test]
fn wp_mcp4_the_server_keeps_the_clocks_with_no_request_coming() {
    let r = rig();
    let (_, body) = reply(call(
        &r,
        "dspi_apply_changes",
        json!({"changes": [{"op": "set_gain", "output": 2, "gain_db": -30.0}], "guard_s": 10}),
    ));
    assert_eq!(body["result"]["isError"], json!(false), "{body}");
    assert_eq!(gain(&r), -30.0);
    // Nobody calls: the ticking thread puts the preview back.
    until("the preview goes back", || gain(&r) == -12.0);
    until("the guard is forgotten", || {
        r.http.with_server(|s| s.guarded_devices().is_empty())
    });
}

#[test]
fn wp_mcp4_stop_reverts_the_previews_and_ends_the_questions() {
    let r = rig();
    let (_, body) = reply(call(
        &r,
        "dspi_apply_changes",
        json!({"changes": [{"op": "set_gain", "output": 2, "gain_db": -30.0}], "guard_s": 300}),
    ));
    assert_eq!(body["result"]["isError"], json!(false), "{body}");
    let s = louder(&r, 0.0);
    waiting(&r, 1);
    r.http.stop();
    assert_eq!(gain(&r), -12.0, "the preview was put back");
    let (_, body) = reply(s);
    assert!(body.to_string().contains("turned AI agents off"), "{body}");
    assert!(r.http.confirmations().is_empty());
    let log = r.http.activity(10);
    let last = log.last().unwrap();
    assert_eq!(last["tool"], "dspi_revert_changes", "{log:?}");
    assert_eq!(last["outcome"], "ok", "{last}");
    assert!(TcpStream::connect(("127.0.0.1", r.http.port())).is_err());
    r.http.stop();
}

#[test]
fn wp_mcp4_another_program_on_the_ipv6_loopback_is_a_start_failure() {
    let Ok(squatter) = TcpListener::bind(SocketAddr::from((Ipv6Addr::LOCALHOST, 0))) else {
        eprintln!("no IPv6 loopback here: not run");
        return;
    };
    let port = squatter.local_addr().unwrap().port();
    if TcpListener::bind(("127.0.0.1", port)).is_err() {
        eprintln!("127.0.0.1:{port} is taken: not run");
        return;
    }
    let start = || {
        HttpServer::start(
            Server::new(CoreSet::new(), ServerConfig::default()),
            HttpConfig {
                port,
                ..HttpConfig::default()
            },
            None,
        )
    };
    match start() {
        Err(StartError::Bind(e)) => assert!(e.to_string().contains("::1"), "{e}"),
        other => panic!("{other:?}"),
    }
    drop(squatter);
    let http = start().unwrap();
    assert_eq!(http.addrs().len(), 2, "{:?}", http.addrs());
}

#[test]
fn wp_mcp4_the_relay_carries_a_held_call() {
    let r = rig();
    let mut keeper = MemoryTokenKeeper::default();
    // -- WP FFIX -- a call with no clientInfo is the fallback name's program
    relay::TokenKeeper::save(&mut keeper, &RelayConfig::default().fallback_name, &r.token);
    let mut params = json!({"name": "dspi_set_gain", "arguments": {"output": 2, "gain_db": -2.0}});
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    let line = format!(
        "{}\n",
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": params})
    );
    let port = r.http.port();
    let agent = std::thread::spawn(move || {
        let cfg = RelayConfig {
            port,
            ..RelayConfig::default()
        };
        let (mut out, mut log) = (Vec::new(), Vec::new());
        relay::run(
            cfg,
            &mut keeper,
            &mut std::io::Cursor::new(line.into_bytes()),
            &mut out,
            &mut log,
        )
        .unwrap();
        String::from_utf8(out).unwrap()
    });
    let id = waiting(&r, 1)[0];
    r.http.answer_confirmation(id, true).unwrap();
    let out: Value = serde_json::from_str(agent.join().unwrap().trim()).unwrap();
    assert_eq!(out["id"], json!(3));
    assert_eq!(
        out["result"]["structuredContent"]["ok"],
        json!(true),
        "{out}"
    );
    assert_eq!(gain(&r), -2.0);
}

#[test]
fn wp_mcp4_a_listener_may_answer_and_stop_from_an_event() {
    // The app's listener only queues; a listener that calls back at once must
    // not deadlock either, on whichever thread the event came.
    let set = CoreSet::new();
    let id = set.open(DeviceTarget::Demo).unwrap();
    let core = set.core(&id).unwrap();
    core.set_output_gain(1, -12.0).unwrap();
    let policy = Arc::new(AgentPolicy::new(ConfirmMode::AskRisky));
    let server = Server::new(
        set,
        ServerConfig {
            policy,
            ..ServerConfig::default()
        },
    );
    let slot: Arc<Mutex<Option<Arc<HttpServer>>>> = Arc::default();
    let seen = Arc::new(AtomicUsize::new(0));
    let listener: Listener = {
        let (slot, seen) = (Arc::clone(&slot), Arc::clone(&seen));
        Arc::new(move |e: &Event| {
            let http = slot.lock().unwrap().clone();
            if let (Event::ConfirmationsChanged, Some(http)) = (e, http) {
                for c in http.confirmations() {
                    let id = c["confirmation_id"].as_u64().unwrap();
                    let _ = http.answer_confirmation(id, true);
                }
                if seen.fetch_add(1, Ordering::SeqCst) >= 1 {
                    http.stop();
                }
            }
        })
    };
    let http = Arc::new(
        HttpServer::start(
            server,
            HttpConfig {
                port: 0,
                ..HttpConfig::default()
            },
            Some(listener),
        )
        .unwrap(),
    );
    let token = http.add_client("x").unwrap().token;
    *slot.lock().unwrap() = Some(Arc::clone(&http));
    let mut s = TcpStream::connect(("127.0.0.1", http.port())).unwrap();
    s.write_all(&bytes(
        http.port(),
        &token,
        &json!(1),
        "tools/call",
        json!({"name": "dspi_set_gain", "arguments": {"output": 2, "gain_db": -1.0}}),
    ))
    .unwrap();
    let (_, body) = reply(s);
    assert_eq!(body["result"]["isError"], json!(false), "{body}");
    assert_eq!(core.get_state().output_gain_db[1], -1.0);
    until("stopped", || {
        TcpStream::connect(("127.0.0.1", http.port())).is_err()
    });
    *slot.lock().unwrap() = None;
}

#[test]
fn wp_mcp4_the_changes_in_words() {
    // -- WP SHOWFIX -- a result's numbers count from 1, as the words do
    let w = |changes: Value| words(&json!({"ok": true, "changes": changes}));
    assert_eq!(
        w(json!([{"what": "output_muted", "output": 1, "before": true, "after": false}])),
        "Out 1 output muted on to off"
    );
    assert_eq!(
        w(json!([{"what": "peq_band", "channel": 9, "slot": 2,
            "before": {"free": true, "type_label": "Flat"},
            "after": {"free": false, "type_label": "Peaking", "freq_hz": 2500.0, "gain_db": -3.5, "q": 2.0}}])),
        "channel 9 band 2 peq band flat to Peaking 2500 Hz -3.5 dB Q 2"
    );
    assert_eq!(
        w(json!([{"what": "crossover", "output": 4, "before": [],
            "after": [{"type_label": "LR4 HP", "freq_hz": 80.0}]}])),
        "Out 4 crossover none to LR4 HP 80 Hz"
    );
    // A name is never put into the log.
    assert_eq!(
        w(json!([{"what": "channel_name", "channel": 1, "before": "a", "after": "b"}])),
        "channel 1 channel name"
    );
    let many: Vec<Value> = (1..7)
        .map(|o| json!({"what": "output_gain_db", "output": o, "before": 0.0, "after": -1.0}))
        .collect();
    assert!(w(json!(many)).ends_with("; 2 more"));
    assert_eq!(words(&json!({"ok": true})), "");
}

#[test]
fn wp_mcp4_the_holds_lapse_and_the_released_one_is_pruned() {
    // Sans-IO: release, then the lapse, and nothing left to resume.
    let set = CoreSet::new();
    set.open(DeviceTarget::Demo).unwrap();
    let mut s = Server::new(
        set,
        ServerConfig {
            policy: Arc::new(AgentPolicy::new(ConfirmMode::AskRisky)),
            ..ServerConfig::default()
        },
    );
    let c = s.connect_as("x");
    let msg = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
        "name": "dspi_set_user_mute", "arguments": {"muted": false},
        "_meta": {"io.modelcontextprotocol/protocolVersion": MODERN,
                  "io.modelcontextprotocol/clientCapabilities": {}}}});
    let on = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
        "name": "dspi_set_user_mute", "arguments": {"muted": true},
        "_meta": {"io.modelcontextprotocol/protocolVersion": MODERN,
                  "io.modelcontextprotocol/clientCapabilities": {}}}});
    assert_eq!(s.handle_held_value(c, 0, on).1, None, "a mute is not asked");
    let (_, id) = s.handle_held_value(c, 0, msg);
    let id = id.expect("an unmute is held");
    assert!(s.resume_value(id).is_none());
    let r = s.release_held(id).unwrap();
    assert_eq!(
        r["result"]["structuredContent"]["status"],
        "awaiting_confirmation"
    );
    assert!(s.release_held(id).is_none(), "released once");
    s.abandon_held(id);
    assert_eq!(
        s.confirmation(id).unwrap().state,
        ConfirmState::Waiting,
        "nothing is held: the poll goes on"
    );
    s.tick(crate::confirm::CONFIRM_HOLD_MS);
    assert_eq!(s.confirmation(id).unwrap().state, ConfirmState::Expired);
}
