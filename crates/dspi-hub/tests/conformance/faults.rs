//! L1's faults (`crates/dspi-link/tests/wp_l1`), with this hub in place of the fake
//! one: what the fake was told to do, a real network or the hub's own side does here.
//! Not repeated: what only a fake hub can do wrong (duplicate frames, a failed or
//! silent device list, F4), which this hub never sends; L1 covers the client's side.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_hub::server::ServerLimits;
use dspi_link::{
    AuthState, Dir, LinkClient, LinkError, LinkEvents, PinSource, SessionConfig, Timing, status,
};
use dspi_proto::generated::opcodes as op;

use crate::support::*;

fn platform(c: &LinkClient) -> u8 {
    c.command(0, Dir::Get, op::REQ_GET_PLATFORM, 0, 2, 7, &[])
        .status
}

fn timing(f: impl FnOnce(&mut Timing)) -> SessionConfig {
    let mut timing = Timing::default();
    f(&mut timing);
    SessionConfig {
        timing,
        ..SessionConfig::default()
    }
}

fn token(b: &Bench, name: &str) -> String {
    let (c, _) = connect(&b.url(), name, None).unwrap();
    let t = c.pair(&b.pin(), "control").unwrap();
    c.close();
    t
}

#[derive(Default)]
struct Recorder {
    closed: Mutex<Vec<Option<u16>>>,
}

impl LinkEvents for Recorder {
    fn closed(&self, code: Option<u16>, _reason: &str) {
        self.closed.lock().unwrap().push(code);
    }
}

#[test]
fn wp_hb7_l1_a_late_reply_cannot_answer_the_next_command() {
    let b = Bench::pin_hub("late");
    let t = token(&b, "late");
    let proxy = Proxy::start(b.port());
    let cfg = timing(|t| t.request_timeout = Duration::from_millis(300));
    let (c, _) = connect_with(&proxy.url(), "late", Some(t), &cfg).unwrap();
    proxy.hold_down(true);
    assert_eq!(platform(&c), status::TIMEOUT);
    proxy.hold_down(false);
    let serial = c.command(0, Dir::Get, op::REQ_GET_SERIAL, 0, 2, 16, &[]);
    assert_eq!(serial.status, status::OK);
    let own = b.hub.devices()[0].identity.serial.clone();
    assert_eq!(
        String::from_utf8_lossy(&serial.payload).trim_end_matches('\0'),
        own,
        "not the platform bytes that came late"
    );
    assert!(!c.is_closed());
}

#[test]
fn wp_hb7_l1_a_cut_connection_reads_as_connection_lost_and_reconnects() {
    let b = Bench::pin_hub("cut");
    let t = token(&b, "cut");
    let proxy = Proxy::start(b.port());
    let ev = Arc::new(Recorder::default());
    let (c, _) = LinkClient::connect(
        &dspi_link::socket::TcpConnector::default(),
        &proxy.url(),
        &options("cut", Some(t.clone())),
        Some(Arc::clone(&ev) as Arc<dyn LinkEvents>),
    )
    .unwrap();
    proxy.cut();
    wait_until("the close", || c.is_closed());
    assert_eq!(
        c.closed_reason().unwrap(),
        (None, "connection lost".to_owned())
    );
    wait_until("the event", || ev.closed.lock().unwrap().len() == 1);
    b.wait_connections(0);
    let (c2, state) = connect(&proxy.url(), "cut", Some(t)).unwrap();
    assert_eq!(state, AuthState::Ready);
    assert_ne!(c2.session_id(), c.session_id());
}

#[test]
fn wp_hb7_l1_a_hub_restart_says_4003_and_keeps_the_token_good() {
    let b = Bench::pin_hub("restart");
    let (c, _) = connect(&b.url(), "restart", None).unwrap();
    let t = c.pair(&b.pin(), "control").unwrap();
    let b = b.restart();
    wait_until("the close", || c.is_closed());
    let code = c.closed_reason().unwrap().0;
    assert_eq!(code, Some(4003));
    assert!(dspi_link::should_reconnect(code));
    let (c2, state) = connect(&b.url(), "restart", Some(t)).unwrap();
    assert_eq!(state, AuthState::Ready);
    assert_eq!(c2.devices().len(), 1);
}

#[test]
fn wp_hb7_l1_a_stalled_link_stays_bounded_and_the_probe_ends_it() {
    let b = Bench::pin_hub("stall");
    let t = token(&b, "stall");
    let proxy = Proxy::start(b.port());
    let cfg = timing(|t| {
        t.request_timeout = Duration::from_millis(200);
        t.probe_timeout = Duration::from_millis(600);
    });
    let (c, _) = connect_with(&proxy.url(), "stall", Some(t), &cfg).unwrap();
    proxy.stall(true);
    let started = Instant::now();
    assert_eq!(platform(&c), status::TIMEOUT);
    wait_until("the probe's close", || c.is_closed());
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(c.closed_reason().unwrap().0, None);
    // The hub's side: a peer that went silent is dropped by its own limit.
    b.wait_connections(0);
}

#[test]
fn wp_hb7_l1_f1_the_hubs_pings_keep_an_idle_session_up() {
    let b = Bench::pin_hub("f1");
    let t = token(&b, "f1");
    // The hub pings every 100 ms and drops a silent peer after 1.5 s; the client sends
    // no pings of its own in this time and gives up after 1 s of silence.
    let cfg = timing(|t| {
        t.silence_limit = Duration::from_secs(1);
        t.ping_interval = Duration::from_secs(60);
    });
    let (c, _) = connect_with(&b.url(), "f1", Some(t), &cfg).unwrap();
    std::thread::sleep(Duration::from_millis(3500));
    assert!(!c.is_closed(), "{:?}", c.closed_reason());
    assert_eq!(platform(&c), status::OK);
}

#[test]
fn wp_hb7_l1_f2_a_token_from_another_hub_is_refused_as_a_typed_state() {
    let first = Bench::pin_hub("f2a");
    let t = token(&first, "f2");
    let other = Bench::pin_hub("f2b");
    let (c, state) = connect(&other.url(), "f2", Some(t)).unwrap();
    assert_eq!(
        state,
        AuthState::TokenRefused {
            pin_source: "window".into()
        }
    );
    assert!(!c.is_closed(), "pairing goes on on the same session");
    c.pair(&other.pin(), "viewer").unwrap();
    assert!(c.is_ready());
}

#[test]
fn wp_hb7_l1_f3_a_used_window_and_the_lockout_are_typed() {
    let b = Bench::pin_hub("f3");
    let pin = b.pin();
    connect(&b.url(), "f3", None)
        .unwrap()
        .0
        .pair(&pin, "viewer")
        .unwrap();
    let (c, _) = connect(&b.url(), "f3", None).unwrap();
    assert_eq!(
        c.pair(&pin, "viewer").unwrap_err(),
        LinkError::WrongPin {
            pin_source: PinSource::Window
        },
        "the window was used up"
    );
    let mut last = None;
    for _ in 0..6 {
        let (c, _) = connect(&b.url(), "f3", None).unwrap();
        last = Some(c.pair("999999", "viewer").unwrap_err());
        if matches!(last, Some(LinkError::RateLimited { .. })) {
            break;
        }
    }
    assert!(
        matches!(last, Some(LinkError::RateLimited { .. })),
        "{last:?}"
    );
}

#[test]
fn wp_hb7_l1_f5_the_token_is_looked_up_by_the_id_in_the_hello() {
    let b = Bench::pin_hub("f5");
    let t = token(&b, "f5");
    let id = b.hub.hub_id().to_owned();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&asked);
    let cfg = SessionConfig {
        token_for: Some(Arc::new(move |hub: &str| {
            seen.lock().unwrap().push(hub.to_owned());
            (hub == id).then(|| t.clone())
        })),
        ..SessionConfig::default()
    };
    let (_, state) = connect_with(&b.url(), "f5", None, &cfg).unwrap();
    assert_eq!(state, AuthState::Ready);
    assert_eq!(*asked.lock().unwrap(), vec![b.hub.hub_id().to_owned()]);
}

#[test]
fn wp_hb7_l1_f6_a_hub_gone_before_the_connect_is_an_error_and_no_event() {
    let b = Bench::pin_hub("f6");
    let url = b.url();
    b.hub.stop();
    drop(b);
    let ev = Arc::new(Recorder::default());
    let r = LinkClient::connect(
        &dspi_link::socket::TcpConnector::default(),
        &url,
        &options("f6", None),
        Some(Arc::clone(&ev) as Arc<dyn LinkEvents>),
    );
    assert!(r.is_err());
    std::thread::sleep(Duration::from_millis(200));
    assert!(ev.closed.lock().unwrap().is_empty());
}

#[test]
fn wp_hb7_l1_f8_a_full_hub_is_busy_and_clears_when_the_slot_does() {
    let b = Bench::start(Setup {
        limits: ServerLimits {
            max_connections: 1,
            ..quick()
        },
        ..Setup::pin("f8")
    });
    let (first, _) = connect(&b.url(), "f8", None).unwrap();
    let e = connect(&b.url(), "f8", None).err().unwrap();
    assert_eq!(e, LinkError::HubBusy);
    first.close();
    b.wait_connections(0);
    assert!(connect(&b.url(), "f8", None).is_ok());
}

#[test]
fn wp_hb7_l1_an_unplug_leaves_the_session_up_and_answers_no_device() {
    let b = Bench::pin_hub("unplug");
    let t = token(&b, "unplug");
    let (c, _) = connect(&b.url(), "unplug", Some(t)).unwrap();
    b.remote.disconnect();
    wait_until("the device offline", || {
        c.devices().first().is_none_or(|d| d.state != "online")
    });
    assert_eq!(platform(&c), status::NO_DEVICE);
    assert!(!c.is_closed());
}
