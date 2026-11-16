//! WP P2HA: Home Assistant through the real hub and server, on the demo device, against a
//! fake broker in this test (loopback only).
//!
//! Discovery for a device (its device block, every entity, the preset names); a volume
//! set on the `set` topic moves the device, as the minted key's principal, and comes back
//! as state; hostile and retained payloads change nothing; a set past a limit is refused
//! and the state stays; a device under the installer lock takes volume, mute and preset;
//! a Viewer-role key changes nothing; a forgotten device's entities are removed; and the
//! end-to-end alarm: a real P2AL alarm, published by P2MQ, with the temperature sensor.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use dspi_hub::mqtt::QoS;
use dspi_hub::mqtt::codec::{self, Packet, Publish};
use dspi_hub::mqtt::ha::{self, Entity};
use dspi_hub::session::FIRST_REMOTE_SESSION_ID;
use dspi_hub::{AccessMode, BusMessage, ClientKind, Hub, HubConfig, NetworkConfig, Role};
use dspi_hub::{ServiceConfig, Subscription};
use dspi_transport::{Capture, FakeDevice, FakeOptions};
use serde_json::{Value, json};
use tungstenite::Message;

const WAIT: Duration = Duration::from_secs(20);
const BASE: &str = "p2ha/a";

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work/wp-P2HA, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p2ha").join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

// ── The fake broker ─────────────────────────────────────────────────────────

#[derive(Default)]
struct State {
    /// Every PUBLISH from the hub, in order.
    published: Vec<Publish>,
    /// Each retained topic's current payload (an empty one removes it).
    retained: HashMap<String, Vec<u8>>,
    /// Every filter the hub subscribed to.
    filters: Vec<String>,
    conns: Vec<Option<TcpStream>>,
}

#[derive(Clone)]
struct Broker {
    addr: SocketAddr,
    state: Arc<Mutex<State>>,
}

impl Broker {
    fn start() -> Broker {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let broker = Broker {
            addr: listener.local_addr().unwrap(),
            state: Arc::new(Mutex::new(State::default())),
        };
        let b = broker.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let n = {
                    let mut s = b.state.lock().unwrap();
                    s.conns.push(Some(stream.try_clone().unwrap()));
                    s.conns.len() - 1
                };
                let b = b.clone();
                thread::spawn(move || b.serve(n, stream));
            }
        });
        broker
    }

    fn port(&self) -> u16 {
        self.addr.port()
    }

    fn send(&self, conn: usize, p: &Packet) {
        let bytes = codec::encode(p).unwrap();
        let mut s = self.state.lock().unwrap();
        if let Some(Some(w)) = s.conns.get_mut(conn) {
            let _ = w.write_all(&bytes);
        }
    }

    fn serve(&self, conn: usize, mut stream: TcpStream) {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        'conn: loop {
            while let Ok(Some((p, used))) = codec::decode(&buf, 1 << 20) {
                buf.drain(..used);
                match p {
                    Packet::Connect(_) => self.send(
                        conn,
                        &Packet::Connack {
                            session_present: false,
                            return_code: 0,
                        },
                    ),
                    Packet::Publish(p) => {
                        if p.qos == QoS::AtLeastOnce {
                            self.send(
                                conn,
                                &Packet::Puback {
                                    packet_id: p.packet_id,
                                },
                            );
                        }
                        let mut s = self.state.lock().unwrap();
                        if p.retain {
                            if p.payload.is_empty() {
                                s.retained.remove(&p.topic);
                            } else {
                                s.retained.insert(p.topic.clone(), p.payload.clone());
                            }
                        }
                        s.published.push(p);
                    }
                    Packet::Subscribe { packet_id, filters } => {
                        let codes = vec![1; filters.len()];
                        self.state
                            .lock()
                            .unwrap()
                            .filters
                            .extend(filters.into_iter().map(|(f, _)| f));
                        self.send(
                            conn,
                            &Packet::Suback {
                                packet_id,
                                return_codes: codes,
                            },
                        );
                    }
                    Packet::Pingreq => self.send(conn, &Packet::Pingresp),
                    Packet::Disconnect => break 'conn,
                    _ => {}
                }
            }
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        let mut s = self.state.lock().unwrap();
        if let Some(Some(w)) = s.conns.get_mut(conn) {
            let _ = w.shutdown(Shutdown::Both);
        }
        s.conns[conn] = None;
    }

    /// What a Home Assistant would send: a PUBLISH to every live connection.
    fn deliver(&self, topic: &str, payload: &[u8], retain: bool) {
        let p = Packet::Publish(Publish {
            topic: topic.into(),
            payload: payload.to_vec(),
            qos: QoS::AtMostOnce,
            retain,
            dup: false,
            packet_id: 0,
        });
        let live: Vec<usize> = {
            let s = self.state.lock().unwrap();
            (0..s.conns.len())
                .filter(|i| s.conns[*i].is_some())
                .collect()
        };
        for c in live {
            self.send(c, &p);
        }
    }

    fn retained(&self, topic: &str) -> Option<String> {
        self.state
            .lock()
            .unwrap()
            .retained
            .get(topic)
            .map(|p| String::from_utf8_lossy(p).into_owned())
    }

    fn wait_retained(&self, topic: &str, want: &str) {
        wait_until(&format!("{topic} = {want}"), || {
            self.retained(topic).as_deref() == Some(want)
        });
    }

    fn config(&self, hub: &str, serial: &str, e: Entity) -> Value {
        let topic = ha::config_topic(hub, serial, e);
        wait_until(&topic, || self.retained(&topic).is_some());
        serde_json::from_str(&self.retained(&topic).unwrap()).unwrap()
    }

    /// Publishes of `topic` so far.
    fn count(&self, topic: &str) -> usize {
        self.state
            .lock()
            .unwrap()
            .published
            .iter()
            .filter(|p| p.topic == topic)
            .count()
    }

    fn published_json(&self, topic: &str) -> Vec<Value> {
        self.state
            .lock()
            .unwrap()
            .published
            .iter()
            .filter(|p| p.topic == topic)
            .filter_map(|p| serde_json::from_slice(&p.payload).ok())
            .collect()
    }

    fn subscribed(&self, filter: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .filters
            .iter()
            .any(|f| f == filter)
    }
}

// ── The hub and its clients ─────────────────────────────────────────────────

struct Ws {
    ws: tungstenite::WebSocket<TcpStream>,
    next_id: u64,
    cid: u32,
}

impl Ws {
    fn read(&mut self) -> Value {
        loop {
            if let Message::Text(text) = self.ws.read().unwrap() {
                return serde_json::from_str(text.as_str()).unwrap();
            }
        }
    }

    fn ask(&mut self, mut msg: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        msg["id"] = id.into();
        self.ws.send(Message::text(msg.to_string())).unwrap();
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no answer to {id}");
            let v = self.read();
            if v["id"] == id {
                return v;
            }
        }
    }

    fn get(&mut self, serial: &str, path: &str) -> Value {
        let v = self.ask(json!({"t": "param.get", "serial": serial, "path": path, "idx": []}));
        assert_eq!(v["t"], "ok", "{v}");
        v["value"].clone()
    }
}

struct Rig {
    hub: Hub,
    broker: Broker,
    hub_id: String,
    // -- WP P2DFIXA --
    /// The hub's store folder's parent, for a restart on the same store.
    dir: PathBuf,
}

fn rig(tag: &str) -> Rig {
    let dir = scratch(tag);
    let hub = start_hub(&dir);
    let hub_id = hub.hub_id().to_owned();
    Rig {
        hub,
        broker: Broker::start(),
        hub_id,
        dir,
    }
}

/// A hub on `dir`'s store, with the inventory, alarms and identify.
fn start_hub(dir: &std::path::Path) -> Hub {
    Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Pin,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir.join("store"))
        }),
        initial_scan: false,
        scan_interval: Duration::from_millis(10),
        offline_grace: Duration::from_millis(300),
        services: ServiceConfig {
            alarms: true,
            identify: true,
            // -- WP P2DFIXA --
            inventory: true,
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("p2ha")
    })
    .unwrap()
}

impl Rig {
    fn plug(&self, serial: &str) -> u8 {
        self.hub
            .attach(Box::new(FakeDevice::demo().with_serial(serial)), 0)
            .unwrap()
    }

    /// The demo device as extension firmware (it has `EXT_IDENTIFY`).
    fn plug_ext(&self, serial: &str) -> u8 {
        let d = FakeDevice::new(
            Capture::demo(),
            FakeOptions {
                extensions: true,
                diag_ext: true,
                ..FakeOptions::default()
            },
        )
        .with_serial(serial);
        self.hub.attach(Box::new(d), 0).unwrap()
    }

    /// A paired client with `role`, logged in over the socket.
    fn client(&self, name: &str, role: Role) -> Ws {
        let net = self.hub.network().unwrap();
        let window = net.auth().begin_pairing(Instant::now()).unwrap();
        let authed = net
            .auth()
            .pair(
                &window.pin,
                name,
                Some(role.min(Role::Control)),
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                Instant::now(),
            )
            .unwrap();
        if role == Role::Admin {
            assert!(net.auth().set_role(authed.cid, Role::Admin));
        }
        let url = format!("ws://127.0.0.1:{}/dspi/v1", net.port());
        let mut req = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
        req.headers_mut()
            .insert("Sec-WebSocket-Protocol", "dspi-link-1".parse().unwrap());
        let tcp = TcpStream::connect(("127.0.0.1", net.port())).unwrap();
        tcp.set_read_timeout(Some(WAIT)).unwrap();
        let mut ws = Ws {
            ws: tungstenite::client(req, tcp).unwrap().0,
            next_id: 100,
            cid: authed.cid,
        };
        ws.ws
            .send(Message::text(
                json!({"t": "hello", "proto": {"major": 1, "minor": 2},
                "client": {"name": name, "app": "wp_p2ha", "version": "0"}})
                .to_string(),
            ))
            .unwrap();
        while ws.read()["t"] != "hello" {}
        let ok = ws.ask(json!({"t": "auth.token", "token": authed.token.unwrap()}));
        assert_eq!(ok["t"], "ok", "{ok}");
        ws
    }

    /// MQTT on, with Home Assistant (`key` as given; none mints one).
    fn enable(&self, admin: &mut Ws, key: Option<u32>) -> Value {
        let mut ha = json!({"enabled": true});
        if let Some(k) = key {
            ha["key"] = k.into();
        }
        let set = admin.ask(json!({"t": "mqtt.set", "settings": {"enabled": true,
            "broker": "127.0.0.1", "port": self.broker.port(), "base_topic": BASE,
            "client_id": "p2ha", "ha": ha}}));
        assert_eq!(set["t"], "ok", "{set}");
        set
    }

    fn state(&self, serial: &str, e: Entity) -> String {
        ha::state_topic(BASE, serial, e)
    }

    fn set(&self, serial: &str, e: Entity, payload: &str) {
        self.broker
            .deliver(&ha::set_topic(BASE, serial, e), payload.as_bytes(), false);
    }

    /// Wait for the subscription and the device's first states, so a set has a device.
    fn ready(&self, serial: &str) {
        let filter = ha::set_filter(BASE);
        wait_until("the set subscription", || self.broker.subscribed(&filter));
        wait_until("the first volume state", || {
            self.broker
                .retained(&self.state(serial, Entity::MasterVolume))
                .is_some()
        });
    }

    /// The preset that is not loaded now, of `names` (two), once the state is known.
    fn other_preset<'a>(&self, serial: &str, names: [&'a str; 2]) -> &'a str {
        let topic = self.state(serial, Entity::Preset);
        wait_until("the preset state", || {
            self.broker.retained(&topic).is_some()
        });
        if self.broker.retained(&topic).as_deref() == Some(names[0]) {
            names[1]
        } else {
            names[0]
        }
    }

    /// Set `e` (to a value other than its state now) and wait until its state is
    /// published again: what it says then.
    fn set_and_wait(&self, serial: &str, e: Entity, payload: &str) -> String {
        let topic = self.state(serial, e);
        let before = self.broker.count(&topic);
        self.set(serial, e, payload);
        wait_until(&format!("{topic} after {payload}"), || {
            self.broker.count(&topic) > before
        });
        // Settle: a notification may follow the answer.
        thread::sleep(Duration::from_millis(200));
        self.broker.retained(&topic).unwrap()
    }
}

fn next_write(sub: &Subscription, serial: &str) -> (u16, Option<String>) {
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < WAIT, "no notification from {serial}");
        if let Some(m) = sub.recv_timeout(Duration::from_millis(100))
            && let BusMessage::Notification {
                serial: s,
                session,
                by,
                ..
            } = &*m
            && s == serial
            && *session != 0
        {
            return (*session, by.clone());
        }
    }
}

// ── The tests ───────────────────────────────────────────────────────────────

#[test]
fn wp_p2ha_discovery_and_sets_through_the_executor_as_the_minted_key() {
    let r = rig("discovery");
    let serial = "P2HA-DISCO";
    r.plug_ext(serial);
    let mut admin = r.client("Installer", Role::Admin);
    for (slot, name) in [(0, "House"), (1, "Night")] {
        let saved = admin.ask(json!({"t": "preset.save", "serial": serial, "slot": slot,
            "name": name, "confirm": true}));
        assert_eq!(saved["t"], "ok", "{saved}");
    }
    let set = r.enable(&mut admin, None);

    // A control key named Home Assistant was minted; its cid is the setting.
    let cid = set["settings"]["ha"]["key"].as_u64().unwrap() as u32;
    let net = r.hub.network().unwrap();
    let key = net
        .auth()
        .clients()
        .into_iter()
        .find(|c| c.cid == cid)
        .unwrap();
    assert_eq!(
        (key.name.as_str(), key.kind, key.role),
        ("Home Assistant", ClientKind::Api, Role::Control)
    );
    let get = admin.ask(json!({"t": "mqtt.get"}));
    assert_eq!(get["settings"]["ha"], json!({"enabled": true, "key": cid}));
    // -- WP P2DFIXA -- N1: `key: null` while on keeps the key; no second one is minted.
    let keys = net.auth().clients().len();
    let again = admin.ask(json!({"t": "mqtt.set", "settings": {"ha":
        {"enabled": true, "key": null}}}));
    assert_eq!(again["settings"]["ha"]["key"], cid, "{again}");
    assert_eq!(net.auth().clients().len(), keys);

    // Discovery: every entity, with the device block.
    for e in Entity::ALL {
        let c = r.broker.config(&r.hub_id, serial, e);
        assert_eq!(c["device"]["serial_number"], serial, "{e:?}: {c}");
        assert_eq!(c["device"]["identifiers"][0], format!("dspi_{serial}"));
        assert!(c["device"]["sw_version"].is_string(), "{c}");
        assert!(c["device"]["model"].is_string(), "{c}");
        assert_eq!(c["availability"][0]["topic"], format!("{BASE}/status"));
    }
    let select = r.broker.config(&r.hub_id, serial, Entity::Preset);
    wait_until("the preset names", || {
        r.broker
            .config(&r.hub_id, serial, Entity::Preset)
            .get("options")
            == Some(&json!(["House", "Night"]))
    });
    assert_eq!(
        select["command_topic"],
        ha::set_topic(BASE, serial, Entity::Preset)
    );
    r.broker
        .wait_retained(&ha::availability_topic(BASE, serial), "online");
    r.broker.wait_retained(&format!("{BASE}/status"), "online");
    r.ready(serial);
    let first: f64 = r
        .broker
        .retained(&r.state(serial, Entity::MasterVolume))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(first, admin.get(serial, "vol.master").as_f64().unwrap());

    // A volume set moves the device, as the key's principal, and comes back as state.
    let sub = r.hub.handle().bus().subscribe("p2ha-test", 1024);
    assert_eq!(r.set_and_wait(serial, Entity::MasterVolume, "-25"), "-25");
    assert_eq!(admin.get(serial, "vol.master"), json!(-25.0));
    let (session, by) = next_write(&sub, serial);
    assert!(session >= FIRST_REMOTE_SESSION_ID, "{session}");
    assert_eq!(
        by.as_deref(),
        Some("Home Assistant"),
        "attributed to the key"
    );

    // Hostile and retained payloads change nothing.
    let topic = ha::set_topic(BASE, serial, Entity::MasterVolume);
    for bad in [
        &b"NaN"[..],
        b"5",
        b"-1e400",
        b"loud",
        &[b'1'; 200],
        &[0xFF, 0xFE],
    ] {
        r.broker.deliver(&topic, bad, false);
    }
    r.broker.deliver(&topic, b"-40", true);
    r.broker.deliver(
        &format!("{BASE}/NOT-A-DEVICE/master_volume/set"),
        b"-40",
        false,
    );
    r.broker
        .deliver(&format!("{BASE}/{serial}/temperature/set"), b"1", false);
    thread::sleep(Duration::from_millis(500));
    assert_eq!(admin.get(serial, "vol.master"), json!(-25.0));
    assert_eq!(
        r.broker.retained(&r.state(serial, Entity::MasterVolume)),
        Some("-25".into())
    );

    // Mute, then a preset by its name.
    assert_eq!(r.set_and_wait(serial, Entity::Mute, "ON"), "ON");
    assert_eq!(admin.get(serial, "vol.mute"), json!(true));
    assert_eq!(r.set_and_wait(serial, Entity::Mute, "OFF"), "OFF");
    let other = r.other_preset(serial, ["House", "Night"]);
    assert_eq!(r.set_and_wait(serial, Entity::Preset, other), other);
    let list = admin.ask(json!({"t": "preset.list", "serial": serial}));
    assert_eq!(
        list["active"],
        if other == "House" { 0 } else { 1 },
        "{list}"
    );

    // Identify: the blink, attributed to the key.
    r.set(serial, Entity::Identify, "PRESS");
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < WAIT, "no identify record");
        if let Some(m) = sub.recv_timeout(Duration::from_millis(100))
            && let BusMessage::Event { serial: s, record } = &*m
            && s == serial
            && record.source.as_deref() == Some("identify")
        {
            assert_eq!(record.by.as_deref(), Some("Home Assistant"));
            break;
        }
    }

    // Off: the entities go, and a set does nothing.
    let off = admin.ask(json!({"t": "mqtt.set", "settings": {"ha": {"enabled": false}}}));
    assert_eq!(off["t"], "ok", "{off}");
    wait_until("the entities removed", || {
        Entity::ALL.iter().all(|e| {
            r.broker
                .retained(&ha::config_topic(&r.hub_id, serial, *e))
                .is_none()
        })
    });
    r.set(serial, Entity::MasterVolume, "-30");
    thread::sleep(Duration::from_millis(500));
    assert_eq!(admin.get(serial, "vol.master"), json!(-25.0));
    r.hub.stop();
}

#[test]
fn wp_p2ha_a_set_past_a_limit_is_refused_and_the_state_stays() {
    let r = rig("limit");
    let serial = "P2HA-LIMIT";
    let handle = r.plug(serial);
    let mut admin = r.client("Installer", Role::Admin);
    let p = json!({"outputs": [{"output": 0, "maxGainDb": 3.0}], "masterVolumeMaxDb": -10.0});
    let set = admin.ask(json!({"t": "limits.set", "handle": handle, "profile": p}));
    assert_eq!(set["t"], "ok", "{set}");
    r.enable(&mut admin, None);
    r.ready(serial);
    assert_eq!(r.set_and_wait(serial, Entity::MasterVolume, "-21"), "-21");

    // Past the limit: refused, nothing reaches the device, the state is put back.
    assert_eq!(r.set_and_wait(serial, Entity::MasterVolume, "-3"), "-21");
    assert_eq!(admin.get(serial, "vol.master"), json!(-21.0));
    // Within it: written.
    assert_eq!(r.set_and_wait(serial, Entity::MasterVolume, "-12"), "-12");
    assert_eq!(admin.get(serial, "vol.master"), json!(-12.0));
    r.hub.stop();
}

#[test]
fn wp_p2ha_a_locked_device_takes_volume_mute_and_preset_only() {
    let r = rig("locked");
    let serial = "P2HA-LOCK";
    let handle = r.plug(serial);
    let mut admin = r.client("Installer", Role::Admin);
    for (slot, name) in [(0, "House"), (1, "Stage")] {
        let saved = admin.ask(json!({"t": "preset.save", "serial": serial, "slot": slot,
            "name": name, "confirm": true}));
        assert_eq!(saved["t"], "ok", "{saved}");
    }
    let p = json!({"outputs": [{"output": 0, "maxGainDb": 3.0}], "masterVolumeMaxDb": -10.0,
        "lockedPages": ["eq", "routing"]});
    assert_eq!(
        admin.ask(json!({"t": "limits.set", "handle": handle, "profile": p}))["t"],
        "ok"
    );
    let pin = admin.ask(json!({"t": "limits.pin", "handle": handle, "pin": "2468"}));
    assert_eq!(pin["t"], "ok", "{pin}");
    // The lock is real for a Control client: the rest is refused.
    let mut c = r.client("Venue tablet", Role::Control);
    let gain = c.ask(
        json!({"t": "param.set", "serial": serial, "path": "out.gain",
        "idx": [0], "value": 1.0}),
    );
    assert_eq!(gain["code"], "installer_lock", "{gain}");

    r.enable(&mut admin, None);
    r.ready(serial);
    wait_until("the preset names", || {
        r.broker
            .config(&r.hub_id, serial, Entity::Preset)
            .get("options")
            == Some(&json!(["House", "Stage"]))
    });
    assert_eq!(r.set_and_wait(serial, Entity::MasterVolume, "-22"), "-22");
    assert_eq!(admin.get(serial, "vol.master"), json!(-22.0));
    assert_eq!(r.set_and_wait(serial, Entity::Mute, "ON"), "ON");
    assert_eq!(admin.get(serial, "vol.mute"), json!(true));
    let other = r.other_preset(serial, ["House", "Stage"]);
    assert_eq!(r.set_and_wait(serial, Entity::Preset, other), other);
    r.hub.stop();
}

#[test]
fn wp_p2ha_a_viewer_key_changes_nothing_and_only_api_keys_are_taken() {
    let r = rig("viewer");
    let serial = "P2HA-VIEW";
    r.plug(serial);
    let mut admin = r.client("Installer", Role::Admin);
    // A console's cid, or one that is no client, is no key for Home Assistant.
    let console = admin.ask(json!({"t": "mqtt.set", "settings": {"ha":
        {"enabled": true, "key": admin.cid}}}));
    assert_eq!(console["code"], "bad_request", "{console}");
    let none = admin.ask(json!({"t": "mqtt.set", "settings": {"ha":
        {"enabled": true, "key": 9999}}}));
    assert_eq!(none["code"], "bad_request", "{none}");

    let minted = admin.ask(json!({"t": "auth.mint", "name": "HA viewer", "role": "viewer"}));
    assert_eq!(minted["t"], "ok", "{minted}");
    let cid = minted["cid"].as_u64().unwrap() as u32;
    let set = r.enable(&mut admin, Some(cid));
    assert_eq!(set["settings"]["ha"]["key"], cid, "no other key minted");
    r.ready(serial);
    let before = admin.get(serial, "vol.master");
    let shown = r
        .broker
        .retained(&r.state(serial, Entity::MasterVolume))
        .unwrap();
    assert_eq!(
        r.set_and_wait(serial, Entity::MasterVolume, "-30"),
        shown,
        "refused: the state stays"
    );
    assert_eq!(admin.get(serial, "vol.master"), before);
    let mute = admin.get(serial, "vol.mute");
    r.set_and_wait(
        serial,
        Entity::Mute,
        if mute == json!(true) { "OFF" } else { "ON" },
    );
    assert_eq!(admin.get(serial, "vol.mute"), mute);

    // Revoked: nothing written either.
    let revoked = admin.ask(json!({"t": "auth.revoke", "cid": cid}));
    assert_eq!(revoked["t"], "ok", "{revoked}");
    r.set(serial, Entity::MasterVolume, "-31");
    thread::sleep(Duration::from_millis(500));
    assert_eq!(admin.get(serial, "vol.master"), before);
    r.hub.stop();
}

#[test]
fn wp_p2ha_a_forgotten_devices_entities_are_removed() {
    let r = rig("forgotten");
    let serial = "P2HA-GONE";
    let handle = r.plug(serial);
    let mut admin = r.client("Installer", Role::Admin);
    r.enable(&mut admin, None);
    r.ready(serial);
    for e in Entity::ALL {
        if e != Entity::Preset {
            r.broker.config(&r.hub_id, serial, e);
        }
    }
    r.hub.detach(handle).unwrap();
    // -- WP P2DFIXA -- offline: unavailable, and still there past the registry's grace.
    r.broker
        .wait_retained(&ha::availability_topic(BASE, serial), "offline");
    let gone = r.hub.handle();
    wait_until("the grace to end", || {
        gone.devices().iter().all(|d| d.identity.serial != serial)
    });
    thread::sleep(Duration::from_millis(300));
    for e in [Entity::MasterVolume, Entity::Mute, Entity::Clip] {
        assert!(
            r.broker
                .retained(&ha::config_topic(&r.hub_id, serial, e))
                .is_some(),
            "{e:?} kept until forgotten"
        );
    }
    assert_eq!(
        r.broker.retained(&ha::availability_topic(BASE, serial)),
        Some("offline".into())
    );
    // Forgotten by an Admin: every config, state and availability cleared.
    let forgot = admin.ask(json!({"t": "inventory.forget", "serial": serial}));
    assert_eq!(forgot["t"], "ok", "{forgot}");
    wait_until("the entities removed", || {
        Entity::ALL.iter().all(|e| {
            r.broker
                .retained(&ha::config_topic(&r.hub_id, serial, *e))
                .is_none()
        })
    });
    assert!(
        r.broker
            .retained(&r.state(serial, Entity::MasterVolume))
            .is_none()
    );
    assert!(
        r.broker
            .retained(&ha::availability_topic(BASE, serial))
            .is_none()
    );
    // Plugged again: discovered again.
    r.plug(serial);
    r.broker.config(&r.hub_id, serial, Entity::MasterVolume);
    r.hub.stop();
}

#[test]
fn wp_p2ha_end_to_end_a_real_alarm_reaches_mqtt_and_the_sensor() {
    let r = rig("alarm");
    let serial = "P2HA-ALARM";
    r.plug(serial);
    let mut admin = r.client("Installer", Role::Admin);
    let rules = admin.ask(json!({"t": "alarm.rules.set", "rules":
        {"temperature": {"above_c": 60}}}));
    assert_eq!(rules["t"], "ok", "{rules}");
    r.enable(&mut admin, None);
    r.ready(serial);

    // A reading over the rule, as P2HP puts it on the bus: P2AL raises a real alarm,
    // P2MQ publishes it, and the temperature sensor follows.
    r.hub.handle().bus().publish(BusMessage::Health {
        serial: serial.into(),
        cpu: Some(12),
        clip_mask: 1,
        temp_c: Some(64.5),
        uptime_s: None,
        hang_count: None,
        // -- WP P3HB --
        counters: None,
        clip_per_min: None,
        last_change: None,
    });
    let topic = format!("{BASE}/{serial}/event");
    wait_until("the alarm on MQTT", || {
        r.broker
            .published_json(&topic)
            .iter()
            .any(|v| v["kind"] == "alarm")
    });
    let alarm = r
        .broker
        .published_json(&topic)
        .into_iter()
        .find(|v| v["kind"] == "alarm")
        .unwrap();
    assert_eq!(alarm["code"], "temperature", "{alarm}");
    assert_eq!(alarm["source"], "device");
    assert_eq!(alarm["serial"], serial);
    assert_eq!(alarm["state"], "raised");
    assert_eq!(alarm["hub"], r.hub_id.as_str());
    assert!(alarm["id"].as_str().unwrap().starts_with("device:"));
    let list = admin.ask(json!({"t": "alarm.list", "state": "active"}));
    assert_eq!(list["alarms"][0]["id"], alarm["id"], "{list}");
    r.broker
        .wait_retained(&r.state(serial, Entity::Temperature), "64.5");
    r.broker.wait_retained(&r.state(serial, Entity::Cpu), "12");
    r.broker.wait_retained(&r.state(serial, Entity::Clip), "ON");
    r.hub.stop();
}

// -- WP P2DFIXA -- the review's probe (review-p2d `probe_p2ha_restart.rs`), on the same
// store: a device unplugged while the hub was down is shown unavailable, not with the
// `online` retained from the first run.
#[test]
fn wp_p2dfixa_a_device_gone_while_the_hub_was_down_is_unavailable() {
    let r = rig("restart");
    let serial = "P2HA-RESTART";
    r.plug(serial);
    let mut admin = r.client("Installer", Role::Admin);
    r.enable(&mut admin, None);
    r.ready(serial);
    r.broker
        .wait_retained(&ha::availability_topic(BASE, serial), "online");
    drop(admin);
    // A clean stop: the hub's status goes offline; the device's topics stay retained.
    r.hub.stop();
    r.broker.wait_retained(&format!("{BASE}/status"), "offline");
    assert_eq!(
        r.broker.retained(&ha::availability_topic(BASE, serial)),
        Some("online".into()),
        "retained from the first run"
    );

    // The hub again, on the same store and broker, the device not plugged in.
    let hub2 = start_hub(&r.dir);
    assert_eq!(hub2.hub_id(), r.hub_id, "the same hub");
    let r2 = Rig {
        hub: hub2,
        broker: r.broker.clone(),
        hub_id: r.hub_id.clone(),
        dir: r.dir.clone(),
    };
    // MQTT and Home Assistant come back on from `mqtt.json`, with the same key.
    r2.broker.wait_retained(&format!("{BASE}/status"), "online");
    r2.broker
        .wait_retained(&ha::availability_topic(BASE, serial), "offline");
    // Its entities stay (it is not forgotten), unavailable.
    assert!(
        r2.broker
            .retained(&ha::config_topic(&r2.hub_id, serial, Entity::MasterVolume))
            .is_some()
    );
    r2.hub.stop();
}
