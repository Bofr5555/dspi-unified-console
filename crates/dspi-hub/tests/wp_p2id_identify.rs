//! WP P2ID: `device.identify` through the real server, on the demo device as firmware
//! with `EXT_IDENTIFY`: the cap in `hello`, a blink answered with the device's status,
//! the rate limit, `unsupported` on a device without it, and the history record on
//! the bus. The rest of the done criteria are `src/identify/tests_wp_p2id.rs`.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dspi_hub::{
    AccessMode, BusMessage, Hub, HubClock, HubConfig, NetworkConfig, ServiceConfig, TestClock,
};
use dspi_transport::{Capture, FakeDevice, FakeOptions};
use serde_json::{Value, json};
use tungstenite::Message;

const WAIT: Duration = Duration::from_secs(20);

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p2id").join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

struct Ws(tungstenite::WebSocket<TcpStream>);

impl Ws {
    fn connect(port: u16) -> Self {
        let url = format!("ws://127.0.0.1:{port}/dspi/v1");
        let mut req = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
        req.headers_mut()
            .insert("Sec-WebSocket-Protocol", "dspi-link-1".parse().unwrap());
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        tcp.set_read_timeout(Some(WAIT)).unwrap();
        Self(tungstenite::client(req, tcp).unwrap().0)
    }

    fn send(&mut self, v: Value) {
        self.0.send(Message::text(v.to_string())).unwrap();
    }

    fn next(&mut self, t: &str) -> Value {
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
            }
        }
    }
}

#[test]
fn wp_p2id_identify_through_the_server() {
    let dir = scratch("server");
    let clock = Arc::new(TestClock::new());
    let hub = Hub::start(HubConfig {
        scan_interval: Duration::from_millis(10),
        initial_scan: false,
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir.clone())
        }),
        services: ServiceConfig {
            clock: Arc::clone(&clock) as Arc<dyn HubClock>,
            identify: true,
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("P2ID hub")
    })
    .unwrap();
    let with = FakeDevice::new(
        Capture::demo(),
        FakeOptions {
            extensions: true,
            diag_ext: true,
            ..FakeOptions::default()
        },
    )
    .with_serial("P2ID-WS");
    hub.attach(Box::new(with), 0).unwrap();
    hub.attach(Box::new(FakeDevice::demo().with_serial("P2ID-WS-OLD")), 0)
        .unwrap();
    let sub = hub.bus().subscribe("p2id-ws", 256);

    let mut ws = Ws::connect(hub.network().unwrap().port());
    ws.send(json!({"t": "hello", "proto": {"major": 1, "minor": 2},
        "client": {"name": "p2id", "app": "test", "version": "0"}}));
    let hello = ws.next("hello");
    assert!(
        hello["caps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "identify"),
        "{hello}"
    );

    let ok = ws.ask(
        json!({"t": "device.identify", "serial": "P2ID-WS", "seconds": 10, "tone_output": 0}),
        1,
    );
    assert_eq!(ok["t"], "ok", "{ok}");
    assert_eq!(ok["v"], 1);
    assert_eq!(ok["serial"], "P2ID-WS");
    assert!(ok["status"]["remaining_s"].as_u64().unwrap() >= 9, "{ok}");
    assert_eq!(ok["status"]["last_tone"], 0);

    let again = ws.ask(
        json!({"t": "device.identify", "serial": "P2ID-WS", "seconds": 10}),
        2,
    );
    assert_eq!(again["code"], "rate_limited", "{again}");
    assert!(again["retry_in_ms"].as_u64().unwrap() > 0);
    clock.advance(Duration::from_secs(2));
    let stop = ws.ask(
        json!({"t": "device.identify", "serial": "P2ID-WS", "seconds": 0}),
        3,
    );
    assert_eq!(stop["status"]["remaining_s"], 0, "{stop}");

    let old = ws.ask(
        json!({"t": "device.identify", "serial": "P2ID-WS-OLD", "seconds": 10}),
        4,
    );
    assert_eq!(old["code"], "unsupported", "{old}");

    let records: Vec<_> = sub
        .drain()
        .iter()
        .filter_map(|m| match &**m {
            BusMessage::Event { serial, record } => Some((serial.clone(), record.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(records.len(), 2, "the blink and the stop: {records:?}");
    for (serial, r) in &records {
        assert_eq!(serial, "P2ID-WS");
        assert_eq!(r.source.as_deref(), Some("identify"));
        assert!(r.by.is_some(), "{r:?}");
        assert!(r.session.is_some());
    }
    drop(ws);
    hub.stop();
    let _ = std::fs::remove_dir_all(&dir);
}
