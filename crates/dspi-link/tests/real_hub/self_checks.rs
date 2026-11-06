//! The promises of `main.rs`, held to by tests that need no hub.

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};

use dspi_link::socket::{WsConnector, WsMessage, WsPair, WsReceiver, WsRecv, WsSender};
use dspi_link::LinkError;
use serde_json::{json, Value};

use crate::guard;
use crate::real::Config;
use crate::target::{forget_pairing, Forgotten, CLIENT_NAME, TEMP_CLIENT_NAME};

/// No environment: nothing is configured, so `real_*` skip.
#[test]
fn wp_xc3_no_hub_set_means_no_config() {
    assert!(Config::parse(|_| None).unwrap().is_none());
    assert!(Config::parse(|n| (n == "DSPI_REAL_HUB").then(|| "  ".to_owned())).unwrap().is_none());
    assert!(Config::parse(|n| (n == "DSPI_REAL_HUB_ALLOW_LOCKOUT").then(|| "1".to_owned())).unwrap().is_none());
    assert!(Config::parse(|n| (n == "DSPI_REAL_HUB").then(|| "not an/address".to_owned())).is_err());
}

#[test]
fn wp_xc3_gates_are_closed_unless_exactly_one() {
    let c = Config::parse(|n| (n == "DSPI_REAL_HUB").then(|| "10.0.0.7".to_owned())).unwrap().unwrap();
    assert_eq!((c.host.as_str(), c.port), ("10.0.0.7", dspi_link::DEFAULT_PORT));
    assert!(!c.allow_pair && !c.allow_lockout && !c.revoke && !c.no_board && !c.operator);
    for stray in ["0", "true", "yes", ""] {
        let c = Config::parse(|n| match n {
            "DSPI_REAL_HUB" => Some("hub:1234".to_owned()),
            "DSPI_REAL_HUB_ALLOW_LOCKOUT" | "DSPI_REAL_HUB_ALLOW_PAIR" => Some(stray.to_owned()),
            _ => None,
        })
        .unwrap()
        .unwrap();
        assert!(!c.allow_pair && !c.allow_lockout, "{stray:?} opened a gate");
    }
    let c = Config::parse(|n| match n {
        "DSPI_REAL_HUB" => Some("hub:1234".to_owned()),
        "DSPI_REAL_HUB_ALLOW_LOCKOUT" => Some("1".to_owned()),
        _ => None,
    })
    .unwrap()
    .unwrap();
    assert!(c.allow_lockout && !c.allow_pair, "one variable opens one gate");
}

/// The fixed attach reads fit a bridge, as the sweep assumes.
#[test]
fn wp_xc3_the_table_fits_a_bridge() {
    for row in guard::ATTACH {
        assert!(row.len <= guard::BRIDGE_GET_LIMIT, "{}", row.name);
    }
    assert!(guard::cs_caps_full(12).len <= guard::BRIDGE_GET_LIMIT);
}

/// The harness's whole source: it has one door to the device, which only reads,
/// and it holds nothing that would write. The patterns are split so this file
/// does not match itself.
#[test]
fn wp_xc3_the_source_has_no_way_to_write() {
    let files = [
        ("main", include_str!("main.rs")),
        ("guard", include_str!("guard.rs")),
        ("proxy", include_str!("proxy.rs")),
        ("real", include_str!("real.rs")),
        ("scenarios", include_str!("scenarios.rs")),
        ("target", include_str!("target.rs")),
        ("fake", include_str!("fake.rs")),
        ("self_checks", include_str!("self_checks.rs")),
    ];
    let command = concat!(".com", "mand(");
    let forbidden = [
        concat!("c.com", "mand("),
        concat!("Dir::", "Set,"),
        concat!("lock_", "acquire"),
        concat!("poll_", "subscribe"),
        concat!("auth.", "set_role"),
        concat!("device.", "rename"),
        concat!("usb", "_audio"),
        concat!("control_", "out"),
    ];
    let mut doors = 0;
    for (name, source) in files {
        doors += source.matches(command).count();
        for bad in forbidden {
            let allowed = name == "guard" && bad == forbidden[0];
            assert!(allowed || !source.contains(bad), "{name}.rs has `{bad}`");
        }
    }
    assert_eq!(doors, 1, "the device has one door, guard::get");
    assert!(include_str!("guard.rs").contains(concat!("c.com", "mand(handle, Dir::", "Get,")), "and it only reads");
}

/// Answers from a script; records what was sent.
struct Scripted {
    replies: Arc<Mutex<VecDeque<WsRecv>>>,
    sent: Arc<Mutex<Vec<Value>>>,
    answer: fn(&Value) -> Vec<WsRecv>,
}

struct ScriptSender(Arc<Mutex<VecDeque<WsRecv>>>, Arc<Mutex<Vec<Value>>>, fn(&Value) -> Vec<WsRecv>);
struct ScriptReceiver(Arc<Mutex<VecDeque<WsRecv>>>);

impl WsSender for ScriptSender {
    fn send(&self, msg: WsMessage) -> io::Result<()> {
        let WsMessage::Text(t) = msg else { return Ok(()) };
        let v: Value = serde_json::from_str(&t).unwrap();
        self.0.lock().unwrap().extend((self.2)(&v));
        self.1.lock().unwrap().push(v);
        Ok(())
    }
    fn ping(&self) -> io::Result<()> {
        Ok(())
    }
    fn close(&self, _code: u16) {}
}

impl WsReceiver for ScriptReceiver {
    fn recv(&mut self) -> WsRecv {
        self.0.lock().unwrap().pop_front().unwrap_or(WsRecv::Idle)
    }
}

impl WsConnector for Scripted {
    fn connect(&self, _url: &str, _protocol: &str) -> Result<WsPair, LinkError> {
        Ok((
            Box::new(ScriptSender(Arc::clone(&self.replies), Arc::clone(&self.sent), self.answer)),
            Box::new(ScriptReceiver(Arc::clone(&self.replies))),
        ))
    }
}

fn text(v: Value) -> WsRecv {
    WsRecv::Message(WsMessage::Text(v.to_string()))
}

fn scripted(answer: fn(&Value) -> Vec<WsRecv>) -> Scripted {
    Scripted { replies: Default::default(), sent: Default::default(), answer }
}

/// A hub with client management and three clients, one of them ours under a name
/// that only looks like it.
fn admin_hub(v: &Value) -> Vec<WsRecv> {
    match v["t"].as_str().unwrap() {
        "hello" => vec![text(json!({"t": "hello"}))],
        "auth.token" => vec![text(json!({"t": "ok", "id": v["id"], "role": "admin"}))],
        "auth.list" => vec![text(json!({"t": "ok", "id": v["id"], "clients": [
            {"cid": 3, "name": "Living Room iPad"},
            {"cid": 7, "name": CLIENT_NAME},
            {"cid": 9, "name": format!("{CLIENT_NAME} 2")},
        ]}))],
        // The hub closes a session whose own record goes
        "auth.revoke" => vec![WsRecv::Closed { code: Some(4002), reason: String::new() }],
        other => panic!("unexpected {other}"),
    }
}

#[test]
fn wp_xc3_forget_pairing_revokes_exactly_its_own_record() {
    let hub = scripted(admin_hub);
    assert_eq!(forget_pairing(&hub, "ws://hub", "tok", &[TEMP_CLIENT_NAME, CLIENT_NAME]), Forgotten::Revoked(1));
    let sent = hub.sent.lock().unwrap();
    let revoked: Vec<_> = sent.iter().filter(|v| v["t"] == "auth.revoke").map(|v| v["cid"].clone()).collect();
    assert_eq!(revoked, [json!(7)], "the harness's record, and nobody else's");
}

#[test]
fn wp_xc3_forget_pairing_revokes_nothing_without_admin() {
    fn viewer_hub(v: &Value) -> Vec<WsRecv> {
        match v["t"].as_str().unwrap() {
            "hello" => vec![text(json!({"t": "hello"}))],
            "auth.token" => vec![text(json!({"t": "ok", "id": v["id"], "role": "viewer"}))],
            _ => vec![text(json!({"t": "err", "id": v["id"], "code": "denied"}))],
        }
    }
    let hub = scripted(viewer_hub);
    assert!(
        matches!(forget_pairing(&hub, "ws://hub", "tok", &[CLIENT_NAME]), Forgotten::Refused(why) if why.contains("denied"))
    );
    assert!(hub.sent.lock().unwrap().iter().all(|v| v["t"] != "auth.revoke"));
}
