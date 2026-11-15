//! WP P2MQC: the hub's MQTT client against an in-test fake broker on the loopback.
//!
//! The broker speaks the client's own codec (tested on its own against fixed vectors in
//! `src/mqtt/codec.rs`): it records every packet, keeps retained messages, routes publishes
//! to matching subscriptions, and can hold its CONNACK, stop answering PINGREQ, refuse a
//! connection or drop every connection, which is what the client's reconnect needs.
#![allow(clippy::arithmetic_side_effects)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use dspi_hub::mqtt::codec::{self, Packet, Publish};
use dspi_hub::mqtt::{MqttClient, MqttConfig, MqttEvent, QoS, Will};

#[derive(Default)]
struct State {
    /// Every packet from a client, with its connection's number.
    received: Vec<(usize, Packet)>,
    retained: BTreeMap<String, Publish>,
    /// Write halves, by connection number; `None` once closed.
    conns: Vec<Option<TcpStream>>,
    subs: Vec<(usize, String, QoS)>,
    hold_connack: bool,
    ignore_pings: bool,
    ignore_publishes: bool,
    connack_code: u8,
    next_id: u16,
}

#[derive(Clone)]
struct Broker {
    addr: SocketAddr,
    state: Arc<Mutex<State>>,
}

fn matches(filter: &str, topic: &str) -> bool {
    let mut f = filter.split('/');
    let mut t = topic.split('/');
    loop {
        match (f.next(), t.next()) {
            (Some("#"), _) => return true,
            (Some("+"), Some(_)) => {}
            (Some(a), Some(b)) if a == b => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

impl Broker {
    fn start() -> Broker {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let broker = Broker {
            addr,
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

    fn send(&self, conn: usize, p: &Packet) {
        let bytes = codec::encode(p).unwrap();
        let mut s = self.state.lock().unwrap();
        if let Some(Some(w)) = s.conns.get_mut(conn) {
            let _ = w.write_all(&bytes);
        }
    }

    fn serve(&self, conn: usize, mut stream: TcpStream) {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            while let Ok(Some((p, used))) = codec::decode(&buf, 1 << 20) {
                buf.drain(..used);
                self.state.lock().unwrap().received.push((conn, p.clone()));
                if !self.handle(conn, p) {
                    return;
                }
            }
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
    }

    fn handle(&self, conn: usize, p: Packet) -> bool {
        match p {
            Packet::Connect(_) => {
                while self.state.lock().unwrap().hold_connack {
                    thread::sleep(Duration::from_millis(5));
                }
                let return_code = self.state.lock().unwrap().connack_code;
                self.send(
                    conn,
                    &Packet::Connack {
                        session_present: false,
                        return_code,
                    },
                );
            }
            Packet::Subscribe { packet_id, filters } => {
                let codes = filters
                    .iter()
                    .map(|(f, q)| {
                        if f.starts_with("refuse") {
                            0x80
                        } else {
                            u8::from(*q == QoS::AtLeastOnce)
                        }
                    })
                    .collect();
                self.send(
                    conn,
                    &Packet::Suback {
                        packet_id,
                        return_codes: codes,
                    },
                );
                let retained: Vec<Publish> = {
                    let mut s = self.state.lock().unwrap();
                    for (f, q) in &filters {
                        if !f.starts_with("refuse") {
                            s.subs.push((conn, f.clone(), *q));
                        }
                    }
                    s.retained
                        .values()
                        .filter(|r| filters.iter().any(|(f, _)| matches(f, &r.topic)))
                        .cloned()
                        .collect()
                };
                for r in retained {
                    self.send(conn, &Packet::Publish(r));
                }
            }
            Packet::Publish(p) => {
                if self.state.lock().unwrap().ignore_publishes {
                    return true;
                }
                if p.qos == QoS::AtLeastOnce {
                    self.send(
                        conn,
                        &Packet::Puback {
                            packet_id: p.packet_id,
                        },
                    );
                }
                if p.retain {
                    let mut kept = p.clone();
                    kept.qos = QoS::AtMostOnce;
                    kept.packet_id = 0;
                    kept.dup = false;
                    self.state
                        .lock()
                        .unwrap()
                        .retained
                        .insert(p.topic.clone(), kept);
                }
                self.route(&p.topic, &p.payload, p.qos);
            }
            Packet::Pingreq => {
                if !self.state.lock().unwrap().ignore_pings {
                    self.send(conn, &Packet::Pingresp);
                }
            }
            Packet::Disconnect => return false,
            _ => {}
        }
        true
    }

    /// Publishes to every live subscription that matches, as a broker does (not retained).
    fn route(&self, topic: &str, payload: &[u8], qos: QoS) {
        let targets: Vec<(usize, QoS, u16)> = {
            let mut s = self.state.lock().unwrap();
            let subs: Vec<(usize, QoS)> = s
                .subs
                .iter()
                .filter(|(c, f, _)| matches(f, topic) && matches!(s.conns.get(*c), Some(Some(_))))
                .map(|(c, _, q)| (*c, (*q).min(qos)))
                .collect();
            subs.into_iter()
                .map(|(c, q)| {
                    s.next_id = s.next_id.wrapping_add(1).max(1);
                    (c, q, s.next_id)
                })
                .collect()
        };
        for (c, q, id) in targets {
            self.send(
                c,
                &Packet::Publish(Publish {
                    topic: topic.into(),
                    payload: payload.to_vec(),
                    qos: q,
                    retain: false,
                    dup: false,
                    packet_id: if q == QoS::AtLeastOnce { id } else { 0 },
                }),
            );
        }
    }

    fn drop_all(&self) {
        let mut s = self.state.lock().unwrap();
        for c in s.conns.iter_mut() {
            if let Some(w) = c.take() {
                let _ = w.shutdown(Shutdown::Both);
            }
        }
        s.subs.clear();
    }

    fn set(&self, f: impl FnOnce(&mut State)) {
        f(&mut self.state.lock().unwrap());
    }

    fn received(&self) -> Vec<(usize, Packet)> {
        self.state.lock().unwrap().received.clone()
    }

    fn count(&self, f: impl Fn(&Packet) -> bool) -> usize {
        self.received().iter().filter(|(_, p)| f(p)).count()
    }
}

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

fn config(b: &Broker) -> MqttConfig {
    let mut c = MqttConfig::new("127.0.0.1", b.addr.port(), "dspi-hub-test");
    c.backoff_min = Duration::from_millis(50);
    c.backoff_max = Duration::from_millis(200);
    c.connect_timeout = Duration::from_secs(2);
    c.tick = Duration::from_millis(10);
    c
}

type Events = Arc<Mutex<Vec<MqttEvent>>>;

fn start(c: MqttConfig) -> (MqttClient, Events) {
    let events: Events = Arc::default();
    let e = Arc::clone(&events);
    let client = MqttClient::start(c, move |ev| e.lock().unwrap().push(ev)).unwrap();
    (client, events)
}

fn has(events: &Events, f: impl Fn(&MqttEvent) -> bool) -> bool {
    events.lock().unwrap().iter().any(f)
}

fn is_connect(p: &Packet) -> bool {
    matches!(p, Packet::Connect(_))
}

#[test]
fn wp_p2mqc_connect_publish_retained_subscribe() {
    let b = Broker::start();
    let mut c = config(&b);
    c.username = Some("hub".into());
    c.password = Some("s3cret".into());
    c.will = Some(Will {
        topic: "dspi/hub/status".into(),
        payload: b"offline".to_vec(),
        qos: QoS::AtLeastOnce,
        retain: true,
    });
    let (mut client, events) = start(c);
    wait_for("connected", || {
        has(&events, |e| matches!(e, MqttEvent::Connected { .. }))
    });
    assert!(client.status().connected);

    // The CONNECT carried the credentials and the will.
    let connect = b.received().into_iter().find_map(|(_, p)| match p {
        Packet::Connect(c) => Some(c),
        _ => None,
    });
    let connect = connect.unwrap();
    assert_eq!(connect.client_id, "dspi-hub-test");
    assert_eq!(connect.username.as_deref(), Some("hub"));
    assert_eq!(connect.password.as_deref(), Some(&b"s3cret"[..]));
    assert_eq!(connect.keep_alive, 30);
    assert!(connect.clean_session);
    assert_eq!(connect.will.unwrap().payload, b"offline");
    assert!(!format!("{:?}", client.status()).contains("s3cret"));

    // A retained publish reaches the broker and is kept there.
    client
        .publish("dspi/A/state", b"on", QoS::AtMostOnce, true)
        .unwrap();
    wait_for("retained kept", || {
        b.state
            .lock()
            .unwrap()
            .retained
            .contains_key("dspi/A/state")
    });

    // Subscribing delivers the retained message, flagged as such.
    client.subscribe("dspi/+/state", QoS::AtLeastOnce).unwrap();
    wait_for("retained delivered", || {
        has(
            &events,
            |e| matches!(e, MqttEvent::Message { topic, payload, retain: true } if topic == "dspi/A/state" && payload == b"on"),
        )
    });

    // QoS 1 both ways: the broker acknowledges ours, and the client acknowledges the copy
    // the broker routes back to it.
    client
        .publish("dspi/B/state", b"vol", QoS::AtLeastOnce, false)
        .unwrap();
    wait_for("live message", || {
        has(
            &events,
            |e| matches!(e, MqttEvent::Message { topic, retain: false, .. } if topic == "dspi/B/state"),
        )
    });
    wait_for("client PUBACK", || {
        b.count(|p| matches!(p, Packet::Puback { .. })) == 1
    });
    wait_for("inflight cleared", || client.status().inflight == 0);

    // A refused subscription is reported.
    client.subscribe("refuse/this", QoS::AtMostOnce).unwrap();
    wait_for("refusal", || {
        has(
            &events,
            |e| matches!(e, MqttEvent::SubscribeRefused { filter } if filter == "refuse/this"),
        )
    });

    // Bad calls are typed refusals.
    assert!(client.publish("a/+", b"", QoS::AtMostOnce, false).is_err());
    assert!(client.subscribe("a/#/b", QoS::AtMostOnce).is_err());
    assert!(
        client
            .publish("big", &vec![0; 300 * 1024], QoS::AtMostOnce, false)
            .is_err()
    );

    // Stopping says DISCONNECT, so the broker does not publish the will.
    client.stop();
    wait_for("DISCONNECT", || {
        b.count(|p| matches!(p, Packet::Disconnect)) == 1
    });
}

#[test]
fn wp_p2mqc_dropped_broker_reconnects_and_resubscribes() {
    let b = Broker::start();
    let (client, events) = start(config(&b));
    client.subscribe("set/#", QoS::AtLeastOnce).unwrap();
    client.subscribe("other", QoS::AtMostOnce).unwrap();
    wait_for("first subscriptions", || {
        b.state.lock().unwrap().subs.len() == 2
    });

    b.drop_all();
    wait_for("disconnected", || {
        has(&events, |e| matches!(e, MqttEvent::Disconnected { .. }))
    });
    wait_for("second CONNECT", || b.count(is_connect) == 2);
    wait_for("re-subscribed", || b.state.lock().unwrap().subs.len() == 2);
    let resubscribe = b
        .received()
        .into_iter()
        .rev()
        .find_map(|(conn, p)| match p {
            Packet::Subscribe { filters, .. } => Some((conn, filters)),
            _ => None,
        });
    let (conn, filters) = resubscribe.unwrap();
    assert_eq!(conn, 1, "the SUBSCRIBE came on the new connection");
    assert_eq!(
        filters,
        vec![
            ("set/#".to_string(), QoS::AtLeastOnce),
            ("other".to_string(), QoS::AtMostOnce)
        ]
    );
    assert_eq!(client.status().connects, 2);

    // And messages flow on the new connection.
    b.route("set/A/volume", b"-20", QoS::AtLeastOnce);
    wait_for("message after reconnect", || {
        has(
            &events,
            |e| matches!(e, MqttEvent::Message { topic, payload, .. } if topic == "set/A/volume" && payload == b"-20"),
        )
    });
}

#[test]
fn wp_p2mqc_unacknowledged_qos1_is_sent_again_after_reconnect() {
    let b = Broker::start();
    b.set(|s| s.ignore_publishes = true);
    let (client, events) = start(config(&b));
    wait_for("connected", || {
        has(&events, |e| matches!(e, MqttEvent::Connected { .. }))
    });
    client
        .publish("alarm", b"clip", QoS::AtLeastOnce, false)
        .unwrap();
    wait_for("first publish", || {
        b.count(|p| matches!(p, Packet::Publish(_))) == 1
    });
    assert_eq!(client.status().inflight, 1);
    b.set(|s| s.ignore_publishes = false);
    b.drop_all();
    wait_for("publish again", || {
        b.count(|p| matches!(p, Packet::Publish(_))) == 2
    });
    let publishes: Vec<Publish> = b
        .received()
        .into_iter()
        .filter_map(|(_, p)| match p {
            Packet::Publish(p) => Some(p),
            _ => None,
        })
        .collect();
    assert!(!publishes[0].dup);
    assert!(publishes[1].dup);
    assert_eq!(publishes[0].packet_id, publishes[1].packet_id);
    assert_eq!(publishes[1].payload, b"clip");
    wait_for("acknowledged", || client.status().inflight == 0);
}

#[test]
fn wp_p2mqc_queue_drops_the_oldest_when_full() {
    let b = Broker::start();
    b.set(|s| s.hold_connack = true);
    let mut c = config(&b);
    c.queue_cap = 4;
    let (client, events) = start(c);
    wait_for("CONNECT", || b.count(is_connect) == 1);
    for n in 0..10u8 {
        client.publish("q", &[n], QoS::AtMostOnce, false).unwrap();
    }
    let st = client.status();
    assert_eq!((st.queued, st.dropped, st.connected), (4, 6, false));

    b.set(|s| s.hold_connack = false);
    wait_for("connected", || {
        has(&events, |e| matches!(e, MqttEvent::Connected { .. }))
    });
    wait_for("four publishes", || {
        b.count(|p| matches!(p, Packet::Publish(_))) == 4
    });
    thread::sleep(Duration::from_millis(100));
    let payloads: Vec<u8> = b
        .received()
        .into_iter()
        .filter_map(|(_, p)| match p {
            Packet::Publish(p) => Some(p.payload[0]),
            _ => None,
        })
        .collect();
    assert_eq!(payloads, vec![6, 7, 8, 9], "the newest four, in order");
    assert_eq!(client.status().queued, 0);
}

#[test]
fn wp_p2mqc_keep_alive_pings_and_a_silent_broker_is_left() {
    let b = Broker::start();
    let mut c = config(&b);
    c.keep_alive = Duration::from_secs(1);
    let (_client, events) = start(c);
    wait_for("PINGREQ", || b.count(|p| matches!(p, Packet::Pingreq)) >= 1);
    thread::sleep(Duration::from_millis(1800));
    assert!(
        b.count(|p| matches!(p, Packet::Pingreq)) >= 2,
        "pings keep coming"
    );
    assert_eq!(b.count(is_connect), 1, "answered pings keep the connection");

    b.set(|s| s.ignore_pings = true);
    wait_for("reconnect after silence", || b.count(is_connect) >= 2);
    assert!(has(
        &events,
        |e| matches!(e, MqttEvent::Disconnected { reason } if reason.contains("PINGREQ"))
    ));
}

#[test]
fn wp_p2mqc_refused_connection_retries_with_backoff() {
    let b = Broker::start();
    b.set(|s| s.connack_code = 5);
    let mut c = config(&b);
    c.username = Some("hub".into());
    c.password = Some("wrong-pass".into());
    let (client, events) = start(c);
    wait_for("three attempts", || b.count(is_connect) >= 3);
    let st = client.status();
    assert!(!st.connected);
    assert_eq!(st.connects, 0);
    let reason = st.last_error.unwrap();
    assert!(reason.contains("not authorised"), "{reason}");
    assert!(!format!("{:?}", events.lock().unwrap()).contains("wrong-pass"));

    b.set(|s| s.connack_code = 0);
    wait_for("accepted at last", || client.status().connected);
}

/// A hostile broker: an oversized length (read past), garbage, a client packet. The client drops the
/// connection with a reason, never buffers past its limit, and tries again.
#[test]
fn wp_p2mqc_hostile_broker_is_dropped_not_trusted() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let replies: Vec<Vec<u8>> = vec![
        vec![0x20, 2, 0, 0, 0x30, 0xFF, 0xFF, 0xFF, 0x7F], // CONNACK, then a 256 MiB PUBLISH
        vec![0x20, 2, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], // garbage
        vec![0x20, 2, 0, 0, 0x82, 6, 0, 1, 0, 1, b'#', 0], // a SUBSCRIBE from the broker
        vec![0x30, 3, 0, 1, b't'],                         // PUBLISH before CONNACK
    ];
    let n = replies.len();
    thread::spawn(move || {
        for (stream, reply) in listener.incoming().zip(replies) {
            let mut stream = stream.unwrap();
            let mut buf = [0u8; 256];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(&reply);
            thread::sleep(Duration::from_millis(300));
        }
    });
    let mut c = MqttConfig::new("127.0.0.1", port, "hub");
    c.backoff_min = Duration::from_millis(20);
    c.backoff_max = Duration::from_millis(20);
    c.tick = Duration::from_millis(10);
    c.max_packet = 1024;
    let (client, events) = start(c);
    wait_for("every hostile reply seen", || {
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, MqttEvent::Disconnected { .. }))
            .count()
            >= n
    });
    let reasons: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            MqttEvent::Disconnected { reason } => Some(reason.clone()),
            _ => None,
        })
        .collect();
    // The 256 MiB header is read past, not buffered: the connection ends only when the
    // broker closes it.
    assert!(reasons[0].contains("closed"), "{reasons:?}");
    assert!(
        reasons[1].contains("malformed") || reasons[1].contains("reserved"),
        "{reasons:?}"
    );
    assert!(reasons[2].contains("protocol error"), "{reasons:?}");
    assert!(reasons[3].contains("before CONNACK"), "{reasons:?}");
    assert!(!client.status().connected);
}

/// Review p2a F2: a retained message over `max_packet` on a subscribed topic is read past;
/// the connection stays up and the messages after it, retained and live, are delivered.
#[test]
fn wp_p2mqc_oversized_retained_is_skipped_not_a_reconnect_storm() {
    let b = Broker::start();
    for (topic, len) in [("set/big", 10_000usize), ("set/ok", 3)] {
        b.set(|s| {
            s.retained.insert(
                topic.into(),
                Publish {
                    topic: topic.into(),
                    payload: vec![b'x'; len],
                    qos: QoS::AtMostOnce,
                    retain: true,
                    dup: false,
                    packet_id: 0,
                },
            );
        });
    }
    let mut c = config(&b);
    c.max_packet = 1024;
    let (client, events) = start(c);
    client.subscribe("set/#", QoS::AtMostOnce).unwrap();
    wait_for("the valid retained message", || {
        has(
            &events,
            |e| matches!(e, MqttEvent::Message { topic, .. } if topic == "set/ok"),
        )
    });
    b.route("set/live", b"1", QoS::AtMostOnce);
    wait_for("a live message", || {
        has(
            &events,
            |e| matches!(e, MqttEvent::Message { topic, .. } if topic == "set/live"),
        )
    });
    thread::sleep(Duration::from_millis(300));
    assert_eq!(b.count(is_connect), 1, "no reconnect");
    assert!(!has(
        &events,
        |e| matches!(e, MqttEvent::Message { topic, .. } if topic == "set/big")
    ));
    let st = client.status();
    assert!(st.connected);
    assert_eq!(st.oversized, 1);
}

/// Review p2a F5: a broker that accepts and then closes meets a growing backoff, not a
/// reconnect every `backoff_min`.
#[test]
fn wp_p2mqc_accept_then_close_backs_off() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let connects = Arc::new(Mutex::new(0usize));
    let n = Arc::clone(&connects);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            *n.lock().unwrap() += 1;
            let mut buf = [0u8; 256];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(&[0x20, 2, 0, 0]);
            thread::sleep(Duration::from_millis(20));
            let _ = stream.shutdown(Shutdown::Both);
        }
    });
    let mut c = MqttConfig::new("127.0.0.1", port, "hub");
    c.backoff_min = Duration::from_millis(50);
    c.backoff_max = Duration::from_secs(5);
    c.tick = Duration::from_millis(10);
    let (client, events) = start(c);
    thread::sleep(Duration::from_secs(2));
    // Doubling from 50 ms: attempts at about 0, 0.07, 0.19, 0.41, 0.83, 1.65 s.
    let got = *connects.lock().unwrap();
    assert!((3..=8).contains(&got), "{got} connects in 2 s");
    assert!(has(&events, |e| matches!(e, MqttEvent::Connected { .. })));
    assert!(client.status().connects >= 3);
}

/// Review p2a F6: `stop()` returns promptly although the broker never reads and the queue
/// holds far more than the socket takes.
#[test]
fn wp_p2mqc_stop_is_bounded_under_a_broker_that_never_reads() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let held = Arc::new(Mutex::new(Vec::new()));
    let h = Arc::clone(&held);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut buf = [0u8; 256];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(&[0x20, 2, 0, 0]);
            // Keep it open and never read again.
            h.lock().unwrap().push(stream);
        }
    });
    let mut c = MqttConfig::new("127.0.0.1", port, "hub");
    c.connect_timeout = Duration::from_secs(3);
    c.tick = Duration::from_millis(10);
    let (mut client, events) = start(c);
    wait_for("connected", || {
        has(&events, |e| matches!(e, MqttEvent::Connected { .. }))
    });
    for _ in 0..200 {
        client
            .publish("bulk", &vec![0; 200_000], QoS::AtMostOnce, false)
            .unwrap();
    }
    thread::sleep(Duration::from_millis(300));
    let t0 = Instant::now();
    client.stop();
    let took = t0.elapsed();
    assert!(took < Duration::from_secs(2), "stop took {took:?}");
}

/// Review p2a F6: `stop()` while the client is between connection attempts.
#[test]
fn wp_p2mqc_stop_is_bounded_while_connecting() {
    // A refused port: the client fails in open() and then waits a 30 s backoff, which
    // `stop` must not wait out.
    let mut c = MqttConfig::new("127.0.0.1", 1, "hub");
    c.backoff_min = Duration::from_secs(30);
    c.backoff_max = Duration::from_secs(30);
    let (mut client, events) = start(c);
    wait_for("first failure", || {
        has(&events, |e| matches!(e, MqttEvent::Disconnected { .. }))
    });
    let t0 = Instant::now();
    client.stop();
    assert!(
        t0.elapsed() < Duration::from_secs(1),
        "stop took {:?}",
        t0.elapsed()
    );
}
