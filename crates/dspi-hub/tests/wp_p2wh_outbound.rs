//! WP P2WH: notifications out, through the real hub and server, to HTTP receivers in
//! this test (loopback only).
//!
//! A raised alarm on the bus reaches a webhook signed so the receiver can check it;
//! secrets are shown once and never by `outbound.get`; `outbound.test` delivers now;
//! `http://` without `"plaintext": true` is refused with the reason (WP SECOUT: `https://`
//! is the norm, tests/wp_secout_tls.rs). A refusing target backs off on the hub's
//! clock (1 s, then 2 s), a dead and a silent one hold nothing up: not the bus, not
//! another target.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use dspi_hub::outbound::hmac;
use dspi_hub::system::alarms::AlarmState;
use dspi_hub::{
    AccessMode, BusMessage, Hub, HubClock, HubConfig, NetworkConfig, ServiceConfig, TestClock,
};
use serde_json::{Value, json};
use tungstenite::Message;

const WAIT: Duration = Duration::from_secs(20);

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p2wh").join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// What a receiver got.
#[derive(Debug)]
struct Request {
    line: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn read_request(s: &mut TcpStream) -> Option<Request> {
    s.set_read_timeout(Some(WAIT)).ok()?;
    let mut got = Vec::new();
    let mut b = [0u8; 4096];
    let end = loop {
        let n = s.read(&mut b).ok()?;
        if n == 0 {
            return None;
        }
        got.extend_from_slice(&b[..n]);
        if let Some(end) = got.windows(4).position(|w| w == b"\r\n\r\n") {
            break end;
        }
    };
    let head = String::from_utf8(got[..end].to_vec()).ok()?;
    let mut lines = head.split("\r\n");
    let line = lines.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(": "))
        .map(|(n, v)| (n.to_owned(), v.to_owned()))
        .collect();
    let len: usize = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
        .map_or(0, |(_, v)| v.parse().unwrap());
    let mut body = got[end + 4..].to_vec();
    while body.len() < len {
        let n = s.read(&mut b).ok()?;
        if n == 0 {
            return None;
        }
        body.extend_from_slice(&b[..n]);
    }
    Some(Request {
        line,
        headers,
        body,
    })
}

/// An HTTP receiver answering every request with `status`.
fn receiver(status: u16) -> (String, Receiver<Request>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://127.0.0.1:{}/hook?k=1",
        l.local_addr().unwrap().port()
    );
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { return };
            let tx = tx.clone();
            std::thread::spawn(move || {
                if let Some(req) = read_request(&mut s) {
                    let _ = write!(s, "HTTP/1.1 {status} X\r\nContent-Length: 0\r\n\r\n");
                    let _ = tx.send(req);
                }
            });
        }
    });
    (url, rx)
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
            "client": {"name": "p2wh", "app": "test", "version": "0"}}));
        let hello = ws.next("hello");
        assert!(
            hello["caps"]
                .as_array()
                .unwrap()
                .contains(&json!("outbound")),
            "{hello}"
        );
        ws
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

    /// The answer to `msg`, and its raw text.
    fn ask(&mut self, mut msg: Value, id: u64) -> (Value, String) {
        msg["id"] = id.into();
        self.send(msg);
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < WAIT, "no answer to {id}");
            if let Message::Text(text) = self.0.read().unwrap() {
                let v: Value = serde_json::from_str(text.as_str()).unwrap();
                if v["id"] == id {
                    return (v, text.as_str().to_owned());
                }
            }
        }
    }
}

fn networked(dir: &Path, clock: Arc<dyn HubClock>) -> Hub {
    Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            // Every connection is an admin.
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(dir.to_path_buf())
        }),
        initial_scan: false,
        services: ServiceConfig {
            clock,
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("P2WH hub")
    })
    .unwrap()
}

fn raised(n: u64, at: u64) -> BusMessage {
    BusMessage::Alarm {
        id: format!("device:{n}"),
        source: "device".into(),
        code: "temperature".into(),
        subject: "P2WH-1".into(),
        serial: Some("P2WH-1".into()),
        severity: "critical".into(),
        state: AlarmState::Raised,
        message: "72 °C".into(),
        at,
        raised_at: at,
        cleared_at: None,
        acknowledged_by: None,
        count: 1,
    }
}

#[test]
fn wp_p2wh_a_raised_alarm_reaches_a_webhook_signed() {
    let dir = scratch("signed");
    let clock: Arc<dyn HubClock> = Arc::new(dspi_hub::SystemClock);
    let hub = networked(&dir, Arc::clone(&clock));
    let h = hub.handle();
    let mut ws = Ws::connect(hub.network().unwrap().port());
    let (url, got) = receiver(200);
    let (push_url, pushed) = receiver(204);

    let (get, _) = ws.ask(json!({"t": "outbound.get"}), 1);
    assert_eq!(get["targets"], json!([]), "none by default: {get}");

    // WP SECOUT: http:// without "plaintext": true is refused, with the reason, and
    // nothing is kept.
    let (https, _) = ws.ask(
        json!({"t": "outbound.set", "targets": [
            {"id": "ha", "kind": "webhook", "url": "http://10.0.0.2/hook"}]}),
        2,
    );
    assert_eq!(https["code"], "bad_request", "{https}");
    assert_eq!(https["target"], 0);
    assert!(
        https["msg"].as_str().unwrap().contains("plaintext"),
        "{https}"
    );
    assert!(!dir.join("outbound.json").exists());

    let (set, set_text) = ws.ask(
        json!({"t": "outbound.set", "targets": [
            {"id": "ha", "name": "Home Assistant", "kind": "webhook", "url": url, "plaintext": true,
             "kinds": ["alarm.raised"], "secret": "new"},
            {"id": "phone", "kind": "push", "url": push_url, "plaintext": true, "kinds": ["alarm.raised"]}]}),
        3,
    );
    assert_eq!(set["t"], "ok", "{set}");
    assert_eq!(set["v"], 1);
    let secret = set["secrets"]["ha"].as_str().unwrap().to_owned();
    assert_eq!(secret.len(), 64);
    assert_eq!(set["secrets"].as_object().unwrap().len(), 1);
    assert_eq!(set["targets"][0]["secret"], "set");
    assert_eq!(set["targets"][1]["secret"], "not set");
    assert_eq!(
        set_text.matches(&secret).count(),
        1,
        "shown once, in `secrets`"
    );
    // Kept in the private file only.
    let file = std::fs::read_to_string(dir.join("outbound.json")).unwrap();
    assert!(file.contains(&secret));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join("outbound.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // The alarm, as P2AL puts it on the bus.
    let at = clock.unix_seconds();
    h.bus().publish(raised(7, at));
    let req = got.recv_timeout(WAIT).expect("the webhook got the alarm");
    assert_eq!(req.line, "POST /hook?k=1 HTTP/1.1");
    assert_eq!(req.header("Content-Type"), Some("application/json"));
    let ts = req.header("X-DSPi-Timestamp").unwrap();
    let ts_n: u64 = ts.parse().unwrap();
    assert!(ts_n.abs_diff(clock.unix_seconds()) < 60, "recent: {ts}");
    // The receiver's check: HMAC-SHA256(secret, timestamp + "." + body).
    let sig = req.header("X-DSPi-Signature").unwrap();
    assert_eq!(sig, hmac::signature(&secret, ts_n, &req.body));
    assert_ne!(sig, hmac::signature("another secret", ts_n, &req.body));
    assert_ne!(
        sig,
        hmac::signature(&secret, ts_n + 1, &req.body),
        "the time is signed"
    );
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    assert_eq!(body["v"], 1);
    assert_eq!(body["hub"], hub.hub_id());
    assert!(!hub.hub_id().is_empty());
    assert_eq!(body["kind"], "alarm.raised");
    assert_eq!(body["serial"], "P2WH-1");
    assert_eq!(body["text"], "Alarm on P2WH-1: 72 °C");
    assert_eq!(body["alarm"]["code"], "temperature");
    assert_eq!(body["alarm"]["severity"], "critical");
    assert!(body["at"].as_str().unwrap().ends_with('Z'));

    // The push target, unsigned: the text alone.
    let push = pushed.recv_timeout(WAIT).expect("the push target got it");
    assert_eq!(
        push.header("Content-Type"),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(
        String::from_utf8(push.body.clone()).unwrap(),
        "Alarm on P2WH-1: 72 °C"
    );
    assert!(push.header("X-DSPi-Signature").is_none());
    assert!(push.header("X-DSPi-Timestamp").is_some());

    // Cleared is not a kind these two want.
    let mut cleared = raised(7, at);
    if let BusMessage::Alarm {
        state, cleared_at, ..
    } = &mut cleared
    {
        *state = AlarmState::Cleared;
        *cleared_at = Some(at);
    }
    h.bus().publish(cleared);

    // outbound.get: the result, never the secret.
    let start = Instant::now();
    let (get, get_text) = loop {
        let (get, text) = ws.ask(json!({"t": "outbound.get"}), 4);
        if get["targets"][0]["last"]["ok"] == true || start.elapsed() > WAIT {
            break (get, text);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(get["targets"][0]["last"]["ok"], true, "{get}");
    assert_eq!(get["targets"][0]["last"]["status"], 200);
    assert_eq!(get["targets"][0]["queued"], 0);
    assert_eq!(get["targets"][0]["dropped"], 0);
    assert_eq!(get["targets"][0]["secret"], "set");
    assert!(!get_text.contains(&secret), "{get_text}");

    // outbound.test: one delivery, now, signed alike.
    let (test, _) = ws.ask(json!({"t": "outbound.test", "target": "ha"}), 5);
    assert_eq!(test["delivered"], true, "{test}");
    assert_eq!(test["status"], 200);
    let req = got.recv_timeout(WAIT).unwrap();
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    assert_eq!(body["kind"], "test");
    let ts: u64 = req.header("X-DSPi-Timestamp").unwrap().parse().unwrap();
    assert_eq!(
        req.header("X-DSPi-Signature").unwrap(),
        hmac::signature(&secret, ts, &req.body)
    );
    assert!(
        got.recv_timeout(Duration::from_millis(300)).is_err(),
        "nothing for the cleared alarm"
    );

    // The list sent back as `outbound.get` showed it keeps the secret; no new one shown.
    let mut targets = get["targets"].clone();
    for t in targets.as_array_mut().unwrap() {
        let t = t.as_object_mut().unwrap();
        for k in ["queued", "dropped", "last"] {
            t.remove(k);
        }
    }
    let (again, _) = ws.ask(json!({"t": "outbound.set", "targets": targets}), 6);
    assert_eq!(again["secrets"], json!({}), "{again}");
    assert_eq!(again["targets"][0]["secret"], "set");
    assert!(
        std::fs::read_to_string(dir.join("outbound.json"))
            .unwrap()
            .contains(&secret)
    );
    drop(ws);
    hub.stop();

    // A restart keeps the targets and delivers without anyone asking.
    let hub = networked(&dir, Arc::clone(&clock));
    let start = Instant::now();
    loop {
        hub.handle().bus().publish(raised(8, clock.unix_seconds()));
        if let Ok(req) = got.recv_timeout(Duration::from_millis(200)) {
            let ts: u64 = req.header("X-DSPi-Timestamp").unwrap().parse().unwrap();
            assert_eq!(
                req.header("X-DSPi-Signature").unwrap(),
                hmac::signature(&secret, ts, &req.body)
            );
            break;
        }
        assert!(start.elapsed() < WAIT, "nothing after a restart");
    }
    hub.stop();
}

#[test]
fn wp_p2wh_dead_targets_back_off_and_never_block_the_bus() {
    let dir = scratch("backoff");
    let test_clock = Arc::new(TestClock::new());
    let clock: Arc<dyn HubClock> = Arc::clone(&test_clock) as Arc<dyn HubClock>;
    let hub = networked(&dir, Arc::clone(&clock));
    let h = hub.handle();
    let mut ws = Ws::connect(hub.network().unwrap().port());

    let (refusing, refused) = receiver(500);
    let (good, delivered) = receiver(204);
    let dead = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://127.0.0.1:{}/", l.local_addr().unwrap().port())
    };
    // Takes the connection (the backlog does) and never answers.
    let silent_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let silent = format!(
        "http://127.0.0.1:{}/",
        silent_listener.local_addr().unwrap().port()
    );
    let (set, _) = ws.ask(
        json!({"t": "outbound.set", "targets": [
            {"id": "silent", "kind": "webhook", "url": silent, "plaintext": true},
            {"id": "dead", "kind": "webhook", "url": dead, "plaintext": true},
            {"id": "refusing", "kind": "webhook", "url": refusing, "plaintext": true},
            {"id": "good", "kind": "webhook", "url": good, "plaintext": true}]}),
        1,
    );
    assert_eq!(set["t"], "ok", "{set}");

    h.bus().publish(raised(1, clock.unix_seconds()));
    // The good target is not held up by the silent one's 10 s.
    let started = Instant::now();
    delivered
        .recv_timeout(WAIT)
        .expect("the good target got it");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "held up: {:?}",
        started.elapsed()
    );
    refused.recv_timeout(WAIT).expect("first try");
    std::thread::sleep(Duration::from_millis(300));

    // Publishing never waits for a target.
    let started = Instant::now();
    for n in 0..2000 {
        h.bus().publish(BusMessage::Unresponsive {
            serial: format!("S{n}"),
            since: 0,
        });
    }
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );

    // The refusing target: tried again 1 s later on the hub's clock, then 2 s later.
    std::thread::sleep(Duration::from_millis(1500));
    h.wake_services();
    assert!(
        refused.recv_timeout(Duration::from_millis(500)).is_err(),
        "tried again before its backoff, with the hub's clock standing still"
    );
    test_clock.advance(Duration::from_secs(1));
    h.wake_services();
    refused.recv_timeout(WAIT).expect("the second try, 1 s on");
    // Let the worker record the failure before the clock moves again.
    std::thread::sleep(Duration::from_millis(300));
    test_clock.advance(Duration::from_secs(1));
    h.wake_services();
    assert!(
        refused.recv_timeout(Duration::from_millis(700)).is_err(),
        "the backoff doubled to 2 s"
    );
    test_clock.advance(Duration::from_secs(1));
    h.wake_services();
    refused
        .recv_timeout(WAIT)
        .expect("the third try, 2 s after the second");

    // The dead one says why, and holds its delivery for later.
    let (get, _) = ws.ask(json!({"t": "outbound.get"}), 2);
    let dead = get["targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "dead")
        .unwrap()
        .clone();
    assert_eq!(dead["last"]["ok"], false, "{get}");
    assert!(
        dead["last"]["msg"].as_str().unwrap().contains("connect"),
        "{dead}"
    );
    assert_eq!(dead["queued"], 1);
    let refusing = get["targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "refusing")
        .unwrap()
        .clone();
    assert_eq!(refusing["last"]["status"], 500, "{refusing}");

    // And the good target still delivers at once.
    h.bus().publish(raised(2, clock.unix_seconds()));
    delivered.recv_timeout(WAIT).expect("still delivering");
    drop(ws);
    let stopping = Instant::now();
    hub.stop();
    assert!(
        stopping.elapsed() < Duration::from_secs(5),
        "a silent target does not hold the stop up"
    );
    drop(silent_listener);
}
