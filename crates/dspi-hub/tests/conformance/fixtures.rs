//! Every upstream message fixture (`tests/fixtures/link/messages`, WeebLabs/DSPi-Link
//! 86f10e3) has an outcome here: a request is sent as written and answered as the
//! table says, a message from a hub is matched by what this hub sends. HB1 has the
//! frame fixtures and the shapes of the common replies; this is the whole list, so a
//! fixture added upstream fails here until someone decides what it means for us.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use dspi_link::socket::{TcpConnector, WsConnector, WsMessage, WsPair, WsRecv};
use dspi_transport::FakeDevice;
use serde_json::{Value, json};

use crate::support::*;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/link/messages")
}

fn fixture(name: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(dir().join(name)).unwrap()).unwrap()
}

/// What this hub does with a fixture.
enum Outcome {
    /// A request answered `ok`, shaped like the named reply fixture where there is one,
    /// with these keys allowed to be missing.
    Ok(Option<&'static str>, &'static [&'static str]),
    /// A request answered `err` with this code.
    Err(&'static str),
    /// A message from a hub, which this hub sends: checked by `wp_hb7_the_hubs_messages`.
    Sent,
    /// A message only the bridge sends (its config, Wi-Fi and update features), or a
    /// reply fixture checked with its request.
    NotOurs,
}

/// Every file in the directory. Order matters only where one request needs another's
/// result: the snapshot and polls before the lock, the revoke last.
const TABLE: &[(&str, Outcome)] = &[
    ("hello_client.json", Outcome::Sent),
    ("hello_hub.json", Outcome::Sent),
    ("hello_hub_bridge.json", Outcome::NotOurs),
    ("auth_pair.json", Outcome::Sent),
    ("auth_token.json", Outcome::Sent),
    ("auth_ok.json", Outcome::Sent),
    ("err_unauthenticated.json", Outcome::Sent),
    ("err_denied.json", Outcome::Sent),
    ("device_added.json", Outcome::Sent),
    ("device_removed.json", Outcome::Sent),
    // Spec 7.4: `inputs` and `wire_version` are left out until read (HB3).
    (
        "device_list.json",
        Outcome::Ok(Some("device_list_ok.json"), &["inputs", "wire_version"]),
    ),
    ("device_list_ok.json", Outcome::NotOurs),
    ("device_snapshot.json", Outcome::Ok(None, &[])),
    // The fixture asks 15 Hz of slot 1; HB3 grants what its budget allows.
    (
        "poll_subscribe.json",
        Outcome::Ok(Some("poll_subscribe_ok.json"), &[]),
    ),
    ("poll_subscribe_ok.json", Outcome::NotOurs),
    ("poll_unsubscribe.json", Outcome::Ok(None, &[])),
    ("lock_acquire.json", Outcome::Ok(None, &[])),
    ("lock_release.json", Outcome::Ok(None, &[])),
    (
        "auth_list.json",
        Outcome::Ok(Some("auth_list_ok.json"), &[]),
    ),
    ("auth_list_ok.json", Outcome::NotOurs),
    // The bridge's own requests, which this hub does not know, as the Swift hub
    // (`LinkSessionHandler.swift:164`). Access is the embedder's setting (HB6).
    ("hub_auth_set_none.json", Outcome::Err("unknown_type")),
    ("hub_auth_set_pin.json", Outcome::Err("unknown_type")),
    ("hub_auth_set_ok.json", Outcome::NotOurs),
    ("hub_config_get.json", Outcome::Err("unknown_type")),
    ("hub_config_get_ok.json", Outcome::NotOurs),
    ("hub_config_set.json", Outcome::Err("unknown_type")),
    ("hub_config_changed.json", Outcome::NotOurs),
    ("hub_wifi_scan.json", Outcome::Err("unknown_type")),
    ("hub_wifi_scan_ok.json", Outcome::NotOurs),
    ("hub_wifi_join.json", Outcome::Err("unknown_type")),
    ("hub_update.json", Outcome::Err("unknown_type")),
    ("hub_update_done.json", Outcome::NotOurs),
    ("hub_restart.json", Outcome::Err("unknown_type")),
    ("hub_reset.json", Outcome::Err("unknown_type")),
    ("err_bad_field.json", Outcome::NotOurs),
    // cid 2 is the fixture's; here it is nobody. The Swift hub answers `ok`.
    ("auth_revoke.json", Outcome::Err("bad_request")),
];

fn raw(b: &Bench) -> WsPair {
    TcpConnector::default()
        .connect(&b.url(), "dspi-link-1")
        .unwrap()
}

fn next(ws: &mut WsPair, kinds: &[&str]) -> Value {
    let start = Instant::now();
    while start.elapsed() < WAIT {
        if let WsRecv::Message(WsMessage::Text(m)) = ws.1.recv() {
            let v: Value = serde_json::from_str(&m).unwrap();
            if kinds.iter().any(|k| v["t"] == *k) {
                return v;
            }
        }
    }
    panic!("no {kinds:?} from the hub");
}

fn send(ws: &mut WsPair, v: &Value) {
    ws.0.send(WsMessage::Text(v.to_string())).unwrap();
}

fn request(ws: &mut WsPair, v: &Value) -> Value {
    send(ws, v);
    next(ws, &["ok", "err"])
}

/// Every key of the fixture is in ours, into objects and first array elements.
fn assert_shape(fixture: &Value, ours: &Value, skip: &[&str], at: &str) {
    match (fixture, ours) {
        (Value::Object(f), Value::Object(o)) => {
            for (key, value) in f {
                if skip.contains(&key.as_str()) {
                    continue;
                }
                let inner = o
                    .get(key)
                    .unwrap_or_else(|| panic!("{at}.{key} missing from {ours}"));
                assert_shape(value, inner, skip, &format!("{at}.{key}"));
            }
        }
        (Value::Array(f), Value::Array(o)) => {
            if let (Some(f), Some(o)) = (f.first(), o.first()) {
                assert_shape(f, o, skip, &format!("{at}[0]"));
            }
        }
        // `null` in a fixture is a value that may be absent or anything.
        (Value::Null, _) => {}
        (Value::Number(_), Value::Number(_))
        | (Value::String(_), Value::String(_))
        | (Value::Bool(_), Value::Bool(_)) => {}
        (f, o) => panic!("{at}: {f} and {o} are different kinds"),
    }
}

/// An admin session on a raw socket, opened with the fixtures' own hello and pair.
fn admin(b: &Bench) -> WsPair {
    let mut ws = raw(b);
    send(&mut ws, &fixture("hello_client.json"));
    let hello = next(&mut ws, &["hello"]);
    assert_shape(&fixture("hello_hub.json"), &hello, &[], "hello");
    let mut pair = fixture("auth_pair.json");
    pair["pin"] = b.admin_pin().into();
    pair["role"] = "admin".into();
    let ok = request(&mut ws, &pair);
    assert_eq!(ok["t"], "ok", "{ok}");
    ws
}

#[test]
fn wp_hb7_every_fixture_has_an_outcome() {
    let on_disk: BTreeSet<String> = fs::read_dir(dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    let listed: BTreeSet<String> = TABLE.iter().map(|(n, _)| (*n).to_owned()).collect();
    assert_eq!(on_disk, listed, "a fixture added or removed upstream");
}

#[test]
fn wp_hb7_every_request_fixture_is_answered_as_the_table_says() {
    let b = Bench::pin_hub("fixtures");
    let mut ws = admin(&b);
    for (name, outcome) in TABLE {
        let reply = match outcome {
            Outcome::Ok(..) | Outcome::Err(_) => request(&mut ws, &fixture(name)),
            Outcome::Sent | Outcome::NotOurs => continue,
        };
        assert_eq!(reply["id"], fixture(name)["id"], "{name}: {reply}");
        match outcome {
            Outcome::Ok(shape, skip) => {
                assert_eq!(reply["t"], "ok", "{name}: {reply}");
                if let Some(shape) = shape {
                    assert_shape(&fixture(shape), &reply, skip, name);
                }
            }
            Outcome::Err(code) => {
                assert_eq!(reply["t"], "err", "{name}: {reply}");
                assert_eq!(reply["code"], *code, "{name}: {reply}");
            }
            Outcome::Sent | Outcome::NotOurs => {}
        }
    }
    // A refused request leaves the session as it was.
    let list = request(&mut ws, &fixture("device_list.json"));
    assert_eq!(list["t"], "ok", "{list}");
}

#[test]
fn wp_hb7_the_hubs_messages() {
    let b = Bench::start(Setup {
        offline_grace: Some(Duration::from_millis(200)),
        ..Setup::pin("messages")
    });
    // Before authentication.
    let mut ws = raw(&b);
    send(&mut ws, &fixture("hello_client.json"));
    next(&mut ws, &["hello"]);
    let unauth = request(&mut ws, &fixture("device_list.json"));
    assert_shape(
        &fixture("err_unauthenticated.json"),
        &unauth,
        &["id"],
        "unauthenticated",
    );
    assert_eq!(unauth["code"], "unauthenticated");

    // A viewer asking for an admin request.
    let mut viewer = raw(&b);
    send(&mut viewer, &fixture("hello_client.json"));
    next(&mut viewer, &["hello"]);
    let mut pair = fixture("auth_pair.json");
    pair["pin"] = b.pin().into();
    pair["role"] = "viewer".into();
    let ok = request(&mut viewer, &pair);
    assert_shape(&fixture("auth_ok.json"), &ok, &[], "auth ok");
    let mut by_token = fixture("auth_token.json");
    by_token["token"] = ok["token"].clone();
    let mut again = raw(&b);
    send(&mut again, &fixture("hello_client.json"));
    next(&mut again, &["hello"]);
    let ok2 = request(&mut again, &by_token);
    assert_eq!(ok2["t"], "ok", "{ok2}");
    assert_eq!(ok2["role"], "viewer");
    let denied = request(&mut viewer, &fixture("auth_list.json"));
    assert_shape(&fixture("err_denied.json"), &denied, &["msg"], "denied");
    assert_eq!(denied["code"], "denied");

    // The device gone, and back.
    b.hub.detach(0).unwrap();
    let removed = loop {
        let v = next(&mut viewer, &["device.removed", "device.changed"]);
        if v["t"] == "device.removed" {
            break v;
        }
    };
    assert_shape(
        &fixture("device_removed.json"),
        &removed,
        &[],
        "device.removed",
    );
    let handle = b.hub.attach(Box::new(FakeDevice::demo()), 0).unwrap();
    let added = next(&mut viewer, &["device.added"]);
    assert_shape(&fixture("device_added.json"), &added, &[], "device.added");
    assert_eq!(added["device"]["handle"], json!(handle));
}

/// What the Swift hub answers otherwise (`LinkSessionHandler.swift:345-366`), recorded in
/// `docs/core-merge/HUB.md` section 8: it renames itself and answers `hub.stats` with
/// an empty body, and revokes an unknown client with `ok`.
#[test]
fn wp_hb7_where_this_hub_answers_otherwise_than_the_swift_one() {
    let b = Bench::pin_hub("swift-diff");
    let mut ws = admin(&b);
    let stats = request(&mut ws, &json!({"t": "hub.stats", "id": 40}));
    assert_eq!(
        (&stats["t"], &stats["code"]),
        (&json!("err"), &json!("unsupported")),
        "{stats}"
    );
    let rename = request(
        &mut ws,
        &json!({"t": "hub.rename", "id": 41, "name": "Other"}),
    );
    assert_eq!(
        (&rename["t"], &rename["code"]),
        (&json!("err"), &json!("unsupported")),
        "{rename}"
    );
    assert_eq!(b.hub.name(), HUB_NAME);
    let revoke = request(&mut ws, &json!({"t": "auth.revoke", "id": 42, "cid": 999}));
    assert_eq!(
        (&revoke["t"], &revoke["code"]),
        (&json!("err"), &json!("bad_request")),
        "{revoke}"
    );
}
