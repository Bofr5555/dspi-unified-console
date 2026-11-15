//! WP P2AL: device alarms through the real hub and server. A temperature reading over
//! the rule raises an alarm that every session hears as `alarm.changed`, that a webhook
//! (P2WH, an HTTP receiver in this test, loopback only) receives, and that `alarm.list`
//! and `alarm.ack` show; the cap is in `hello.caps`.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use dspi_hub::{AccessMode, BusMessage, Hub, HubConfig, NetworkConfig, ServiceConfig};
use serde_json::{Value, json};
use tungstenite::Message;

const WAIT: Duration = Duration::from_secs(20);

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p2al").join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A receiver answering 200; the bodies it got.
fn receiver() -> (String, Receiver<Vec<u8>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}/hook", l.local_addr().unwrap().port());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { return };
            let tx = tx.clone();
            std::thread::spawn(move || {
                s.set_read_timeout(Some(WAIT)).unwrap();
                let mut got = Vec::new();
                let mut b = [0u8; 4096];
                let end = loop {
                    let Ok(n) = s.read(&mut b) else { return };
                    if n == 0 {
                        return;
                    }
                    got.extend_from_slice(&b[..n]);
                    if let Some(end) = got.windows(4).position(|w| w == b"\r\n\r\n") {
                        break end;
                    }
                };
                let head = String::from_utf8_lossy(&got[..end]).to_string();
                let len: usize = head
                    .split("\r\n")
                    .filter_map(|l| l.split_once(": "))
                    .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
                    .map_or(0, |(_, v)| v.parse().unwrap());
                let mut body = got[end + 4..].to_vec();
                while body.len() < len {
                    let Ok(n) = s.read(&mut b) else { return };
                    if n == 0 {
                        return;
                    }
                    body.extend_from_slice(&b[..n]);
                }
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                let _ = tx.send(body);
            });
        }
    });
    (url, rx)
}

/// A client; what an `ask` passed over is kept for `next`.
struct Ws(tungstenite::WebSocket<TcpStream>, Vec<Value>);

impl Ws {
    fn connect(port: u16) -> (Self, Value) {
        let url = format!("ws://127.0.0.1:{port}/dspi/v1");
        let mut req = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
        req.headers_mut()
            .insert("Sec-WebSocket-Protocol", "dspi-link-1".parse().unwrap());
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        tcp.set_read_timeout(Some(WAIT)).unwrap();
        let mut ws = Self(tungstenite::client(req, tcp).unwrap().0, Vec::new());
        ws.send(json!({"t": "hello", "proto": {"major": 1, "minor": 2},
            "client": {"name": "p2al", "app": "test", "version": "0"}}));
        let hello = ws.next("hello");
        (ws, hello)
    }

    fn send(&mut self, v: Value) {
        self.0.send(Message::text(v.to_string())).unwrap();
    }

    fn next(&mut self, t: &str) -> Value {
        if let Some(i) = self.1.iter().position(|v| v["t"] == t) {
            return self.1.remove(i);
        }
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no {t} in time");
            if let Message::Text(text) = self.0.read().unwrap() {
                let v: Value = serde_json::from_str(text.as_str()).unwrap();
                if v["t"] == t {
                    return v;
                }
            }
        }
    }

    fn ask(&mut self, mut msg: Value, id: u64) -> Value {
        msg["id"] = id.into();
        self.send(msg);
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no answer to {id}");
            if let Message::Text(text) = self.0.read().unwrap() {
                let v: Value = serde_json::from_str(text.as_str()).unwrap();
                if v["id"] == id {
                    return v;
                }
                self.1.push(v);
            }
        }
    }
}

#[test]
fn wp_p2al_a_raised_alarm_reaches_every_session_and_a_webhook() {
    let dir = scratch("e2e");
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            // Every connection is an admin.
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir.clone())
        }),
        initial_scan: false,
        services: ServiceConfig {
            alarms: true,
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("P2AL hub")
    })
    .unwrap();
    let port = hub.network().unwrap().port();
    let (mut ws, hello) = Ws::connect(port);
    assert!(
        hello["caps"].as_array().unwrap().contains(&json!("alarms")),
        "{hello}"
    );
    let (url, got) = receiver();
    let set = ws.ask(
        json!({"t": "outbound.set", "targets": [
            {"id": "ha", "kind": "webhook", "url": url, "kinds": ["alarm.raised"]}]}),
        1,
    );
    assert_eq!(set["t"], "ok", "{set}");
    let rules = ws.ask(
        json!({"t": "alarm.rules.set", "rules": {"temperature": {"above_c": 60}}}),
        2,
    );
    assert_eq!(rules["rules"]["temperature"]["above_c"], 60.0, "{rules}");
    assert!(dir.join("alarms.json").exists());

    // A reading, as P2HP puts it on the bus.
    hub.handle().bus().publish(BusMessage::Health {
        serial: "P2AL-E2E".into(),
        cpu: Some(12),
        clip_mask: 0,
        temp_c: Some(64.5),
        uptime_s: None,
        hang_count: None,
    });
    let changed = ws.next("alarm.changed");
    assert_eq!(changed["v"], 1);
    assert_eq!(changed["hub"], hub.hub_id());
    let alarm = &changed["alarm"];
    assert_eq!(alarm["code"], "temperature");
    assert_eq!(alarm["source"], "device");
    assert_eq!(alarm["serial"], "P2AL-E2E");
    assert_eq!(alarm["state"], "raised");
    assert!(alarm["raised_at"].as_str().unwrap().ends_with('Z'));
    let id = alarm["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("device:"), "{id}");

    let body = got.recv_timeout(WAIT).expect("the webhook got the alarm");
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["kind"], "alarm.raised");
    assert_eq!(body["serial"], "P2AL-E2E");
    assert_eq!(body["alarm"]["id"], id.as_str());
    assert_eq!(body["alarm"]["code"], "temperature");
    assert!(body["text"].as_str().unwrap().contains("64.5 °C"), "{body}");

    let list = ws.ask(json!({"t": "alarm.list", "state": "active"}), 3);
    assert_eq!(list["alarms"].as_array().unwrap().len(), 1, "{list}");
    assert_eq!(list["alarms"][0]["id"], id.as_str());
    let ack = ws.ask(json!({"t": "alarm.ack", "alarm": id}), 4);
    assert_eq!(ack["t"], "ok", "{ack}");
    assert_eq!(ack["alarm"]["state"], "acknowledged");
    assert!(ack["alarm"]["acknowledged_by"].is_string(), "{ack}");
    let changed = ws.next("alarm.changed");
    assert_eq!(changed["alarm"]["state"], "acknowledged");
    // An acknowledgement is no `alarm.raised`: the webhook hears nothing more.
    assert!(got.recv_timeout(Duration::from_millis(500)).is_err());
    hub.stop();
}
