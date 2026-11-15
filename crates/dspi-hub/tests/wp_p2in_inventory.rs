//! WP P2IN: the inventory through the real server: the cap in `hello`, a list, an
//! admin's set, the `inventory.changed` event on another session, and a device going
//! offline staying in the inventory while `device.list` is unchanged.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dspi_hub::{AccessMode, Hub, HubClock, HubConfig, NetworkConfig, ServiceConfig, TestClock};
use dspi_transport::FakeDevice;
use serde_json::{Value, json};
use tungstenite::Message;

const WAIT: Duration = Duration::from_secs(20);

fn scratch(name: &str) -> PathBuf {
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work, not tmpfs.
    let dir = std::env::temp_dir()
        .join("wp-p2in")
        .join(format!("{name}-{}", std::process::id()));
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
        let mut ws = Self(tungstenite::client(req, tcp).unwrap().0);
        ws.send(json!({"t": "hello", "proto": {"major": 1, "minor": 2},
            "client": {"name": "p2in", "app": "test", "version": "0"}}));
        ws
    }

    fn send(&mut self, v: Value) {
        self.0.send(Message::text(v.to_string())).unwrap();
    }

    /// The next message for which `pick` holds.
    fn next_where(&mut self, what: &str, pick: impl Fn(&Value) -> bool) -> Value {
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no {what} in time");
            if let Message::Text(text) = self.0.read().unwrap() {
                let v: Value = serde_json::from_str(text.as_str()).unwrap();
                if pick(&v) {
                    return v;
                }
            }
        }
    }

    fn ask(&mut self, mut msg: Value, id: u64) -> Value {
        msg["id"] = id.into();
        self.send(msg);
        self.next_where("an answer", |v| v["id"] == id)
    }
}

#[test]
fn wp_p2in_inventory_over_the_socket() {
    let dir = scratch("socket");
    let clock: Arc<dyn HubClock> = Arc::new(TestClock::new());
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir.clone())
        }),
        initial_scan: false,
        offline_grace: Duration::from_millis(50),
        services: ServiceConfig {
            clock,
            inventory: true,
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("P2IN socket")
    })
    .unwrap();
    let handle = hub
        .attach(Box::new(FakeDevice::demo().with_serial("P2IN-W")), 0)
        .unwrap();
    let port = hub.network().unwrap().port();

    let mut admin = Ws::connect(port);
    let hello = admin.next_where("hello", |v| v["t"] == "hello");
    assert!(
        hello["caps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "inventory"),
        "{hello}"
    );
    let mut other = Ws::connect(port);
    other.next_where("hello", |v| v["t"] == "hello");
    // Open access: the session opened with `hello`; a round trip makes sure.
    other.ask(json!({"t": "device.list"}), 1);

    let list = admin.ask(json!({"t": "inventory.list"}), 2);
    assert_eq!(list["t"], "ok", "{list}");
    assert_eq!(list["v"], 1);
    assert_eq!(list["devices"][0]["serial"], "P2IN-W");
    assert_eq!(list["devices"][0]["handle"], handle);
    assert_eq!(list["devices"][0]["online"], true);

    let set = admin.ask(
        json!({"t": "inventory.set", "serial": "P2IN-W", "location": "Stage left", "tags": ["foh"]}),
        3,
    );
    assert_eq!(set["t"], "ok", "{set}");
    let changed = other.next_where("inventory.changed", |v| {
        v["t"] == "inventory.changed" && v["device"]["location"] == "Stage left"
    });
    assert_eq!(changed["v"], 1);
    assert_eq!(changed["serial"], "P2IN-W");
    assert_eq!(changed["hub"], hub.hub_id());
    assert_eq!(changed["device"]["tags"], json!(["foh"]));

    // Detached and past the grace: gone from `device.list`, kept in the inventory.
    hub.detach(handle).unwrap();
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < WAIT, "device.list kept it");
        let l = admin.ask(json!({"t": "device.list"}), 10);
        if l["devices"].as_array().unwrap().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let list = admin.ask(json!({"t": "inventory.list", "serial": "P2IN-W"}), 4);
    assert_eq!(list["devices"][0]["online"], false, "{list}");
    assert_eq!(list["devices"][0]["handle"], Value::Null);
    assert_eq!(list["devices"][0]["location"], "Stage left");
    let forget = admin.ask(json!({"t": "inventory.forget", "serial": "P2IN-W"}), 5);
    assert_eq!(forget["forgot"][0], "record", "{forget}");
    let gone = other.next_where("the forget event", |v| {
        v["t"] == "inventory.changed" && v["device"].is_null()
    });
    assert_eq!(gone["serial"], "P2IN-W");
    drop(admin);
    drop(other);
    hub.stop();
}
