//! The fake hub itself: does it do what the real hubs do?

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::json;

use dspi_link::fakehub::{Auth, DeviceListFault, FakeHub, HubConfig};
use dspi_link::socket::WsRecv;

use crate::support::*;

fn pin_hub() -> FakeHub {
    FakeHub::start(HubConfig::bridge()).unwrap()
}

fn pair_msg(pin: &str) -> serde_json::Value {
    json!({"t": "auth.pair", "id": 1, "pin": pin, "name": "raw", "role": "control"})
}

fn http_status(hub: &FakeHub, request: &str) -> String {
    let mut tcp = TcpStream::connect(hub.addr()).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    tcp.write_all(request.as_bytes()).unwrap();
    let mut out = Vec::new();
    let _ = tcp.read_to_end(&mut out);
    String::from_utf8_lossy(&out).lines().next().unwrap_or("").to_owned()
}

#[test]
fn wp_l1_hub_wrong_pin_is_unauthenticated_with_no_msg() {
    let hub = pin_hub();
    let mut s = raw_socket(&hub);
    raw_hello(&mut s);
    let r = raw_request(&mut s, pair_msg("000000"));
    assert_eq!(r["t"], "err");
    assert_eq!(r["code"], "unauthenticated");
    assert!(r.get("msg").is_none());
}

#[test]
fn wp_l1_hub_five_failures_rate_limit_pins_and_tokens_for_a_minute() {
    let clock = manual_clock();
    let hub = hub_on(&clock, HubConfig::bridge());
    let mut s = raw_socket(&hub);
    raw_hello(&mut s);
    for _ in 0..5 {
        assert_eq!(raw_request(&mut s, pair_msg("000000"))["code"], "unauthenticated");
    }
    assert_eq!(raw_request(&mut s, pair_msg("482913"))["code"], "rate_limited", "even the right PIN");
    let token = json!({"t": "auth.token", "id": 2, "token": "anything"});
    assert_eq!(raw_request(&mut s, token.clone())["code"], "rate_limited");
    clock.advance(Duration::from_secs(61));
    let mut s = raw_socket(&hub); // The old one passed its auth deadline
    raw_hello(&mut s);
    assert_eq!(raw_request(&mut s, token)["code"], "unauthenticated", "the lockout ended");
    assert_eq!(raw_request(&mut s, pair_msg("482913"))["t"], "ok");
}

#[test]
fn wp_l1_hub_window_pin_pairs_one_client_and_five_failures_end_it() {
    let cfg = || HubConfig { auth: Auth::Window("123456".into()), ..HubConfig::default() };
    let hub = FakeHub::start(cfg()).unwrap();
    let mut s = raw_socket(&hub);
    raw_hello(&mut s);
    assert_eq!(raw_request(&mut s, pair_msg("123456"))["t"], "ok");
    let mut s2 = raw_socket(&hub);
    raw_hello(&mut s2);
    assert_eq!(raw_request(&mut s2, pair_msg("123456"))["code"], "unauthenticated", "one success consumes it");
    hub.reopen_window();
    assert_eq!(raw_request(&mut s2, pair_msg("123456"))["t"], "ok");

    let hub = FakeHub::start(HubConfig { rate_limit_failures: 100, ..cfg() }).unwrap();
    let mut s = raw_socket(&hub);
    raw_hello(&mut s);
    for _ in 0..5 {
        assert_eq!(raw_request(&mut s, pair_msg("000000"))["code"], "unauthenticated");
    }
    assert_eq!(raw_request(&mut s, pair_msg("123456"))["code"], "unauthenticated", "the failures ended it");
}

#[test]
fn wp_l1_hub_answers_503_when_full_and_holds_a_vanished_clients_slot() {
    let clock = manual_clock();
    let hub = hub_on(&clock, HubConfig { max_sessions: Some(1), ..HubConfig::default() });
    let mut gone = raw_socket(&hub);
    raw_hello(&mut gone); // Open access: authenticated by the hello. It now says nothing more
    assert!(http_status(
        &hub,
        "GET /dspi/v1 HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: dspi-link-1\r\n\r\n",
    )
    .contains("503"));
    clock.advance(Duration::from_secs(29));
    assert_eq!(hub.slots_in_use(), 1, "still held before the keepalive limit");
    clock.advance(Duration::from_secs(2));
    assert!(wait(|| hub.slots_in_use() == 0), "1001 at the keepalive limit frees the slot");
    let mut got_close = None;
    for _ in 0..3 {
        if let WsRecv::Closed { code, .. } = gone.1.recv() {
            got_close = code;
            break;
        }
    }
    assert_eq!(got_close, Some(1001));
}

#[test]
fn wp_l1_hub_bridge_answers_400_without_the_sub_protocol() {
    let hub = pin_hub();
    let status = http_status(
        &hub,
        "GET /dspi/v1 HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
    );
    assert!(status.contains("400"), "{status}");
}

#[test]
fn wp_l1_hub_closes_4001_at_the_auth_deadline() {
    let clock = manual_clock();
    let hub = hub_on(&clock, HubConfig::bridge());
    let mut s = raw_socket(&hub);
    raw_hello(&mut s);
    clock.advance(Duration::from_secs(31));
    assert!(wait(|| hub.slots_in_use() == 0));
    let mut got = None;
    for _ in 0..3 {
        if let WsRecv::Closed { code, .. } = s.1.recv() {
            got = code;
            break;
        }
    }
    assert_eq!(got, Some(4001));
}

#[test]
fn wp_l1_hub_pings_authenticated_sessions_only() {
    let clock = manual_clock();
    let hub = hub_on(&clock, HubConfig::bridge());
    let mut s = raw_socket(&hub);
    raw_hello(&mut s);
    clock.advance(Duration::from_secs(11));
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(hub.pings_sent(), 0, "not before auth");
    assert_eq!(raw_request(&mut s, pair_msg("482913"))["t"], "ok");
    clock.advance(Duration::from_secs(11));
    assert!(wait(|| hub.pings_sent() == 1));
}

#[test]
fn wp_l1_hub_faults_token_and_device_list() {
    let hub = pin_hub();
    hub.fail_tokens_with(Some("internal"));
    let mut s = raw_socket(&hub);
    raw_hello(&mut s);
    let r = raw_request(&mut s, json!({"t": "auth.token", "id": 3, "token": "t"}));
    assert_eq!(r["code"], "internal");
    assert_eq!(hub.tokens_seen(), ["t"]);
    assert_eq!(raw_request(&mut s, pair_msg("482913"))["t"], "ok");
    hub.fail_device_list(DeviceListFault::Error("internal".into()), 1);
    let list = json!({"t": "device.list", "id": 4});
    assert_eq!(raw_request(&mut s, list.clone())["code"], "internal");
    assert_eq!(raw_request(&mut s, list.clone())["devices"].as_array().unwrap().len(), 1);
    hub.set_board_present(false);
    let offline = raw_request(&mut s, list);
    assert_eq!(offline["devices"][0]["state"], "offline", "the handle is kept for the hold");
}

#[test]
fn wp_l1_hub_restart_keeps_id_and_tokens_and_reset_loses_them() {
    let hub = pin_hub();
    let mut s = raw_socket(&hub);
    raw_hello(&mut s);
    let token = raw_request(&mut s, pair_msg("482913"))["token"].as_str().unwrap().to_owned();
    hub.restart();
    assert!(wait(|| hub.slots_in_use() == 0));
    let mut s = raw_socket(&hub);
    raw_hello(&mut s);
    let again = json!({"t": "auth.token", "id": 5, "token": token});
    assert_eq!(raw_request(&mut s, again.clone())["t"], "ok");
    hub.factory_reset("another-hub");
    assert!(wait(|| hub.slots_in_use() == 0));
    let mut s = raw_socket(&hub);
    s.0.send(dspi_link::socket::WsMessage::Text(
        r#"{"t":"hello","proto":{"major":1,"minor":2},"client":{"name":"raw","app":"test","version":"0"}}"#.into(),
    ))
    .unwrap();
    assert_eq!(raw_reply(&mut s, "hello")["hub"]["id"], "another-hub");
    assert_eq!(raw_request(&mut s, again)["code"], "unauthenticated");
}

#[test]
fn wp_l1_hub_holds_duplicates_and_drops_replies() {
    let hub = pin_hub();
    let mut s = raw_socket(&hub);
    raw_hello(&mut s);
    assert_eq!(raw_request(&mut s, pair_msg("482913"))["t"], "ok");
    let list = json!({"t": "device.list", "id": 6});
    hub.duplicate_frames(true);
    assert_eq!(raw_request(&mut s, list.clone())["t"], "ok");
    assert_eq!(raw_reply(&mut s, "ok")["id"], 6, "again");
    hub.duplicate_frames(false);
    hub.hold_replies();
    s.0.send(dspi_link::socket::WsMessage::Text(list.to_string())).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    hub.release_replies();
    assert_eq!(raw_reply(&mut s, "ok")["id"], 6, "late");
    hub.drop_next_replies(1);
    s.0.send(dspi_link::socket::WsMessage::Text(list.to_string())).unwrap();
    assert_eq!(raw_request(&mut s, json!({"t": "device.list", "id": 7}))["id"], 7, "the one before was swallowed");
}
