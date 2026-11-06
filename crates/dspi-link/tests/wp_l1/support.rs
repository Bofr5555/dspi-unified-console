//! Shared helpers.

#![allow(dead_code, unused_macros)]

use std::sync::Arc;
use std::time::Duration;

use dspi_link::clock::{Clock, ManualClock};
use dspi_link::fakehub::{FakeHub, HubConfig};
use dspi_link::socket::{TcpConnector, WsConnector, WsMessage, WsPair, WsRecv};
use dspi_link::ConnectOptions;

pub fn opts(token: Option<String>) -> ConnectOptions {
    ConnectOptions {
        client_name: "Pixel 9".into(),
        app: "DSPi Console for Android".into(),
        app_version: "test".into(),
        token,
    }
}

/// Polls `f` (up to 5 s of real time, normally a few ms).
pub fn wait(mut f: impl FnMut() -> bool) -> bool {
    for _ in 0..1000 {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

pub fn manual_clock() -> Arc<ManualClock> {
    Arc::new(ManualClock::new())
}

pub fn dyn_clock(c: &Arc<ManualClock>) -> Arc<dyn Clock> {
    Arc::clone(c) as Arc<dyn Clock>
}

/// A hub whose timers run on `clock`.
pub fn hub_on(clock: &Arc<ManualClock>, cfg: HubConfig) -> FakeHub {
    FakeHub::start(HubConfig { clock: dyn_clock(clock), ..cfg }).unwrap()
}

/// A bare WebSocket with no client logic: it answers nothing by itself once
/// its reader stops being called, like a client that vanished.
pub fn raw_socket(hub: &FakeHub) -> WsPair {
    TcpConnector::default().connect(&hub.url(), "dspi-link-1").unwrap()
}

const HELLO: &str = r#"{"t":"hello","proto":{"major":1,"minor":2},"client":{"name":"raw","app":"test","version":"0"}}"#;

/// Exchange hellos on a bare socket.
pub fn raw_hello(pair: &mut WsPair) {
    pair.0.send(WsMessage::Text(HELLO.into())).unwrap();
    raw_reply(pair, "hello");
}

/// The next text message with `t` = `t`, as JSON.
pub fn raw_reply(pair: &mut WsPair, t: &str) -> serde_json::Value {
    for _ in 0..50 {
        if let WsRecv::Message(WsMessage::Text(m)) = pair.1.recv() {
            let v: serde_json::Value = serde_json::from_str(&m).unwrap();
            if v["t"] == t {
                return v;
            }
        }
    }
    panic!("no `{t}` from the hub");
}

/// Send `msg` on a bare socket and read the `ok` or `err` it gets.
pub fn raw_request(pair: &mut WsPair, msg: serde_json::Value) -> serde_json::Value {
    pair.0.send(WsMessage::Text(msg.to_string())).unwrap();
    for _ in 0..50 {
        if let WsRecv::Message(WsMessage::Text(m)) = pair.1.recv() {
            let v: serde_json::Value = serde_json::from_str(&m).unwrap();
            if v["t"] == "ok" || v["t"] == "err" {
                return v;
            }
        }
    }
    panic!("no reply from the hub");
}
