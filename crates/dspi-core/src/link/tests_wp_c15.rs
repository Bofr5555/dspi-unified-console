//! WP C15: the session, the supervisor, the poll feed and fault injection, on
//! the fake hub. Faults go in at the message level ([`FaultConnector`]), under
//! a real `LinkClient` over a real socket.

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::AtomicUsize;

use dspi_link::fakehub::{Auth, FakeHub, HubConfig, req};
use dspi_link::frames::Frame;
use dspi_link::socket::{WsMessage, WsPair, WsReceiver, WsRecv, WsSender};
use serde_json::{Value, json};

use super::*;

const WAIT: Duration = Duration::from_secs(8);
const FAST: Backoff = Backoff {
    first: Duration::from_millis(20),
    max: Duration::from_millis(100),
};

type Rewrite = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// What a test does to the traffic, shared by every socket of a session.
#[derive(Default)]
struct Faults {
    /// Each frame out takes this long, one at a time: a slow serial link.
    out_delay_ms: AtomicU64,
    /// Text frames containing the first string are held for the given ms.
    hold_text: Mutex<Option<(String, u64)>>,
    /// Deliver every frame in twice.
    duplicate_in: AtomicBool,
    /// Lose the reply to the next command, keeping it for [`Faults::late`].
    swallow_next_cmd: AtomicBool,
    late: Mutex<Option<WsMessage>>,
    /// Frames to deliver next.
    inject: Mutex<VecDeque<WsMessage>>,
    /// End the socket without a close frame.
    cut: AtomicBool,
    /// Change inbound text.
    rewrite: Mutex<Option<Rewrite>>,
    /// Text sent, in order.
    outbound: Mutex<Vec<String>>,
    serial: Mutex<()>,
}

impl Faults {
    fn sent(&self, what: &str) -> usize {
        guard(&self.outbound)
            .iter()
            .filter(|t| t.contains(what))
            .count()
    }

    fn set_rewrite(&self, f: impl Fn(&str) -> Option<String> + Send + Sync + 'static) {
        *guard(&self.rewrite) = Some(Box::new(f));
    }
}

struct FaultConnector {
    faults: Arc<Faults>,
}

impl WsConnector for FaultConnector {
    fn connect(&self, url: &str, protocol: &str) -> std::result::Result<WsPair, LinkError> {
        let (tx, rx) = TcpConnector::default().connect(url, protocol)?;
        Ok((
            Box::new(FaultSender {
                inner: tx,
                faults: Arc::clone(&self.faults),
            }),
            Box::new(FaultReceiver {
                inner: rx,
                faults: Arc::clone(&self.faults),
                again: None,
            }),
        ))
    }
}

struct FaultSender {
    inner: Box<dyn WsSender>,
    faults: Arc<Faults>,
}

impl WsSender for FaultSender {
    fn send(&self, msg: WsMessage) -> io::Result<()> {
        if let WsMessage::Text(t) = &msg {
            guard(&self.faults.outbound).push(t.clone());
            let hold = guard(&self.faults.hold_text).clone();
            if let Some((what, ms)) = hold
                && t.contains(&what)
            {
                std::thread::sleep(Duration::from_millis(ms));
            }
        }
        let delay = self.faults.out_delay_ms.load(Ordering::SeqCst);
        if delay > 0 {
            let _serial = guard(&self.faults.serial);
            std::thread::sleep(Duration::from_millis(delay));
        }
        self.inner.send(msg)
    }

    fn ping(&self) -> io::Result<()> {
        self.inner.ping()
    }

    fn close(&self, code: u16) {
        self.inner.close(code);
    }
}

struct FaultReceiver {
    inner: Box<dyn WsReceiver>,
    faults: Arc<Faults>,
    again: Option<WsMessage>,
}

impl WsReceiver for FaultReceiver {
    fn recv(&mut self) -> WsRecv {
        let f = Arc::clone(&self.faults);
        if f.cut.swap(false, Ordering::SeqCst) {
            return WsRecv::Closed {
                code: None,
                reason: "cut".into(),
            };
        }
        if let Some(m) = self.again.take() {
            return WsRecv::Message(m);
        }
        if let Some(m) = guard(&f.inject).pop_front() {
            return WsRecv::Message(m);
        }
        match self.inner.recv() {
            WsRecv::Message(WsMessage::Text(t)) => {
                let t = guard(&f.rewrite).as_ref().and_then(|r| r(&t)).unwrap_or(t);
                if f.duplicate_in.load(Ordering::SeqCst) {
                    self.again = Some(WsMessage::Text(t.clone()));
                }
                WsRecv::Message(WsMessage::Text(t))
            }
            WsRecv::Message(WsMessage::Binary(b)) => {
                if matches!(Frame::decode(&b), Ok(Frame::CmdResponse(_)))
                    && f.swallow_next_cmd.swap(false, Ordering::SeqCst)
                {
                    *guard(&f.late) = Some(WsMessage::Binary(b));
                    return WsRecv::Idle;
                }
                if f.duplicate_in.load(Ordering::SeqCst) {
                    self.again = Some(WsMessage::Binary(b.clone()));
                }
                WsRecv::Message(WsMessage::Binary(b))
            }
            other => other,
        }
    }
}

fn connect_faulty(core: &DspiCore, hub: &FakeHub, token: &str) -> (Arc<Faults>, LinkHello) {
    let faults = Arc::new(Faults::default());
    let hello = connect_with(core, hub, token, &faults).unwrap();
    (faults, hello)
}

fn connect_with(
    core: &DspiCore,
    hub: &FakeHub,
    token: &str,
    faults: &Arc<Faults>,
) -> Result<LinkHello> {
    core.link_connect_with(
        hub.url(),
        "wp-c15".into(),
        "test".into(),
        token.into(),
        Arc::new(FaultConnector {
            faults: Arc::clone(faults),
        }),
        FAST,
    )
}

fn open_hub() -> FakeHub {
    FakeHub::start(HubConfig::default()).unwrap()
}

fn pin_hub() -> FakeHub {
    FakeHub::start(HubConfig {
        auth: Auth::Fixed("123456".into()),
        ..HubConfig::bridge()
    })
    .unwrap()
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_phase(core: &DspiCore, phase: LinkPhase) {
    wait_until(&format!("{phase:?}"), || core.link_status().phase == phase);
}

fn bind(core: &DspiCore) -> String {
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial.clone()).unwrap();
    core.fetch_all().unwrap();
    d.serial
}

#[derive(Default)]
struct Events {
    devices: AtomicUsize,
    status: AtomicUsize,
    closed: Mutex<Vec<(i32, String)>>,
}

impl LinkListener for Events {
    fn on_link_devices_changed(&self) {
        self.devices.fetch_add(1, Ordering::SeqCst);
    }
    fn on_link_closed(&self, code: i32, reason: String) {
        guard(&self.closed).push((code, reason));
    }
    fn on_link_status_changed(&self) {
        self.status.fetch_add(1, Ordering::SeqCst);
    }
}

// ── The API ────────────────────────────────────────────────────────────

#[test]
fn wp_c15_free_functions() {
    let a = link_parse_address(" hub.local:8080 ".into()).unwrap();
    assert_eq!((a.host.as_str(), a.port), ("hub.local", 8080));
    assert!(matches!(
        link_parse_address("a b".into()),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert_eq!(
        link_ws_url("10.0.0.1".into(), 11915, String::new()),
        "ws://10.0.0.1:11915/dspi/v1"
    );
    assert_eq!(
        link_ws_url("fe80::1".into(), 1, "/x".into()),
        "ws://[fe80::1]:1/x"
    );

    let hub = open_hub();
    let addr = hub.addr();
    let info = link_fetch_info(addr.ip().to_string(), addr.port()).unwrap();
    assert_eq!(info.hub_name, "Fake Hub");
    assert_eq!(info.ws_path, "/dspi/v1");
    assert_eq!(info.serials.len(), 1);
}

#[test]
fn wp_c15_no_session() {
    let core = DspiCore::new();
    let st = core.link_status();
    assert_eq!(st.phase, LinkPhase::Idle);
    assert_eq!(st.close_code, -1);
    assert!(core.link_devices().is_empty());
    assert_eq!(core.link_pair("1".into()), Err(DspiError::Disconnected));
    assert_eq!(
        core.link_select_device(0, "x".into()),
        Err(DspiError::Disconnected)
    );
    core.link_close();
    core.link_retry_now();
}

#[test]
fn wp_c15_handles_over_255_are_refused() {
    let core = DspiCore::new();
    assert!(matches!(
        core.link_select_device(256, "x".into()),
        Err(DspiError::InvalidArgument { .. })
    ));
}

#[test]
fn wp_c15_connect_select_fetch_close() {
    let hub = open_hub();
    let core = DspiCore::new();
    let hello = core
        .link_connect(hub.url(), "t".into(), "1".into(), String::new())
        .unwrap();
    assert!(!hello.needs_pairing);
    assert_eq!(hello.hub_name, "Fake Hub");
    wait_phase(&core, LinkPhase::ChooseDevice);
    let st = core.link_status();
    assert!(st.connected && st.ready && !st.closed);
    assert_ne!(st.session, 0);

    let serial = bind(&core);
    assert_eq!(core.link_status().phase, LinkPhase::Bound);
    assert!(core.get_state().bulk_valid);
    assert_eq!(core.lock().conn().unwrap().serial, serial);

    core.link_close();
    assert!(!core.is_connected());
    assert_eq!(core.link_status().phase, LinkPhase::Idle);
    wait_until("the hub to see the close", || hub.sessions().is_empty());
}

#[test]
fn wp_c15_a_connect_failure_names_the_cause() {
    let hub = open_hub();
    let url = hub.url();
    drop(hub);
    std::thread::sleep(Duration::from_millis(100));
    let core = DspiCore::new();
    let e = core
        .link_connect(url, "t".into(), "1".into(), String::new())
        .unwrap_err();
    assert!(matches!(e, DspiError::Link { .. }), "{e:?}");
    assert_eq!(core.link_status().phase, LinkPhase::Idle);
}

#[test]
fn wp_c15_error_words() {
    let rejected = |code: &str| LinkError::Rejected {
        code: code.into(),
        msg: String::new(),
    };
    let code = |e: DspiError| match e {
        DspiError::Link { code, .. } => code,
        other => panic!("{other:?}"),
    };
    // What both real hubs answer, and what the fake hub answers.
    for wrong in ["unauthenticated", "denied"] {
        assert_eq!(code(map_pair_error(rejected(wrong), "fixed")), "wrong_pin");
        let DspiError::Link { msg, .. } = map_pair_error(rejected(wrong), "window") else {
            panic!()
        };
        assert!(msg.contains("expired"), "{msg}");
    }
    assert_eq!(
        code(map_pair_error(rejected("rate_limited"), "fixed")),
        "rate_limited"
    );
    assert_eq!(
        code(map_pair_error(rejected("internal"), "fixed")),
        "internal"
    );
    assert_eq!(code(map_pair_error(LinkError::Timeout, "fixed")), "timeout");
    assert_eq!(
        code(map_error(LinkError::Handshake("HTTP error: 503".into()))),
        "hub_busy"
    );
    assert_eq!(
        code(map_error(LinkError::Handshake("HTTP error: 400".into()))),
        "handshake"
    );
    assert_eq!(map_lock_error(rejected("locked")), DspiError::Locked);
    assert_eq!(map_lock_error(rejected("denied")), DspiError::Denied);
    assert_eq!(
        map_lock_error(LinkError::Closed {
            code: None,
            reason: String::new()
        }),
        DspiError::Disconnected
    );
}

#[test]
fn wp_c15_backoff_doubles_up_to_the_limit() {
    let b = Backoff::default();
    let secs: Vec<u64> = (0..8).map(|i| b.delay(i).as_secs()).collect();
    assert_eq!(secs, [1, 2, 4, 8, 16, 30, 30, 30]);
    assert_eq!(b.delay(u32::MAX).as_secs(), 30);
}

// ── Pairing ────────────────────────────────────────────────────────────

#[test]
fn wp_c15_pairing_wrong_pin_then_right() {
    let hub = pin_hub();
    let core = DspiCore::new();
    let (_, hello) = connect_faulty(&core, &hub, "");
    assert!(hello.needs_pairing);
    assert_eq!(hello.pin_source, "fixed");
    wait_phase(&core, LinkPhase::NeedsPairing);
    assert_eq!(core.link_status().pin_source, "fixed");
    assert!(core.link_devices().is_empty());

    match core.link_pair("000000".into()) {
        Err(DspiError::Link { code, .. }) => assert_eq!(code, "wrong_pin"),
        other => panic!("{other:?}"),
    }
    assert_eq!(core.link_status().phase, LinkPhase::NeedsPairing);

    let token = core.link_pair(" 123456 ".into()).unwrap();
    assert!(!token.is_empty());
    wait_phase(&core, LinkPhase::ChooseDevice);
    assert_eq!(core.link_devices().len(), 1);
}

#[test]
fn wp_c15_a_prompt_outlives_the_socket_and_pairing_redials() {
    let hub = pin_hub();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    // The hub's auth deadline.
    hub.close_all(4001);
    wait_until("the socket to close", || !core.link_status().connected);
    std::thread::sleep(Duration::from_millis(150));
    let st = core.link_status();
    assert_eq!(st.phase, LinkPhase::NeedsPairing, "{st:?}");
    assert_eq!(st.reconnect_attempt, 0);
    assert_eq!(faults.sent("\"hello\""), 1, "no redial during the prompt");

    core.link_pair("123456".into()).unwrap();
    assert_eq!(faults.sent("\"hello\""), 2);
    wait_phase(&core, LinkPhase::ChooseDevice);
}

// ── Reconnecting (fault injection) ─────────────────────────────────────

#[test]
fn wp_c15_a_lost_session_reconnects_with_the_token_and_rebinds() {
    let hub = pin_hub();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    let events = Arc::new(Events::default());
    core.set_link_listener(Some(events.clone()));
    core.link_pair("123456".into()).unwrap();
    bind(&core);
    wait_phase(&core, LinkPhase::Bound);
    let generation = core.lock().conn().unwrap().generation;

    hub.close_all(1001);
    wait_until("the rebind", || {
        core.link_status().phase == LinkPhase::Bound
            && core.lock().conn().is_ok_and(|c| c.generation != generation)
    });
    assert_eq!(faults.sent("auth.pair"), 1, "no PIN again");
    assert_eq!(
        faults.sent("auth.token"),
        1,
        "{:?}",
        guard(&faults.outbound)
    );
    assert!(core.get_state().bulk_valid, "the mirror is read again");
    assert!(events.status.load(Ordering::SeqCst) >= 2);
    assert!(guard(&events.closed).is_empty(), "a loss is not an end");
    core.set_link_listener(None);
}

#[test]
fn wp_c15_a_cut_socket_reconnects_and_a_failed_dial_backs_off() {
    let hub = open_hub();
    let addr = hub.addr();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    faults.cut.store(true, Ordering::SeqCst);
    // The cut shows at the next receive tick.
    hub.resync();
    wait_until("a reconnect", || faults.sent("\"hello\"") >= 2);
    wait_phase(&core, LinkPhase::ChooseDevice);

    // The hub goes away for a while, then comes back on the same address.
    drop(hub);
    wait_until("the retries to count up", || {
        core.link_status().reconnect_attempt >= 2
    });
    let st = core.link_status();
    assert_eq!(st.phase, LinkPhase::Reconnecting);
    assert!(!st.connected);
    assert!(!st.close_reason.is_empty());

    let _hub = restart(addr, HubConfig::default());
    wait_phase(&core, LinkPhase::ChooseDevice);
    assert_eq!(core.link_status().reconnect_attempt, 0);
}

/// A hub on `addr`, once the old listener has let go of it.
fn restart(addr: std::net::SocketAddr, cfg: HubConfig) -> FakeHub {
    let start = Instant::now();
    loop {
        match FakeHub::start(HubConfig {
            bind: addr,
            ..cfg.clone()
        }) {
            Ok(hub) => return hub,
            Err(e) => {
                assert!(start.elapsed() < WAIT, "{e}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

#[test]
fn wp_c15_a_restarted_hub_refuses_the_token_and_the_prompt_comes_back() {
    let hub = pin_hub();
    let addr = hub.addr();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    core.link_pair("123456".into()).unwrap();
    wait_phase(&core, LinkPhase::ChooseDevice);
    drop(hub);
    // Same id, fresh token table.
    let _hub = restart(
        addr,
        HubConfig {
            auth: Auth::Fixed("123456".into()),
            ..HubConfig::bridge()
        },
    );
    wait_phase(&core, LinkPhase::NeedsPairing);
    assert_eq!(faults.sent("auth.token"), 1);
    let hellos = faults.sent("\"hello\"");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(faults.sent("\"hello\""), hellos, "no retry into a lockout");
    core.link_pair("123456".into()).unwrap();
    wait_phase(&core, LinkPhase::ChooseDevice);
}

#[test]
fn wp_c15_the_token_does_not_go_to_another_hub_at_the_address() {
    let hub = pin_hub();
    let addr = hub.addr();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    core.link_pair("123456".into()).unwrap();
    wait_phase(&core, LinkPhase::ChooseDevice);
    drop(hub);
    let _other = restart(
        addr,
        HubConfig {
            hub_id: "11111111-2222-4333-8444-555555555555".into(),
            auth: Auth::Fixed("654321".into()),
            ..HubConfig::bridge()
        },
    );
    wait_until("a refused try", || {
        core.link_status().close_reason.contains("another hub")
    });
    assert_eq!(faults.sent("auth.token"), 0, "the secret stays home");
    assert_eq!(core.link_status().phase, LinkPhase::Reconnecting);
}

#[test]
fn wp_c15_revocation_ends_the_session() {
    let hub = pin_hub();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    let events = Arc::new(Events::default());
    core.set_link_listener(Some(events.clone()));
    core.link_pair("123456".into()).unwrap();
    hub.revoke_all();
    wait_phase(&core, LinkPhase::Closed);
    let st = core.link_status();
    assert!(st.closed);
    assert_eq!(st.close_code, 4002);
    wait_until("the listener", || guard(&events.closed).len() == 1);
    assert_eq!(guard(&events.closed)[0].0, 4002);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(faults.sent("\"hello\""), 1, "not retried");
    core.set_link_listener(None);
}

#[test]
fn wp_c15_link_close_during_backoff_stops_the_retries() {
    let hub = open_hub();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    drop(hub);
    wait_until("a retry", || core.link_status().reconnect_attempt >= 1);
    core.link_close();
    assert_eq!(core.link_status().phase, LinkPhase::Idle);
    std::thread::sleep(Duration::from_millis(250));
    let n = faults.sent("\"hello\"");
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(faults.sent("\"hello\""), n);
}

#[test]
fn wp_c15_disconnect_with_the_session_up_is_not_undone() {
    let hub = open_hub();
    let core = DspiCore::new();
    connect_faulty(&core, &hub, "");
    bind(&core);
    wait_phase(&core, LinkPhase::Bound);
    core.disconnect();
    wait_phase(&core, LinkPhase::ChooseDevice);
    hub.close_all(1001);
    wait_until("the session to come back", || {
        let st = core.link_status();
        st.phase == LinkPhase::ChooseDevice && st.ready
    });
    std::thread::sleep(Duration::from_millis(150));
    assert!(!core.is_connected());
}

#[test]
fn wp_c15_duplicate_frames_do_no_harm() {
    let hub = open_hub();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    faults.duplicate_in.store(true, Ordering::SeqCst);
    bind(&core);
    core.set_master_volume(-20.0).unwrap();
    core.set_master_volume(-21.0).unwrap();
    assert_eq!(core.get_state().master_volume_db, -21.0);
}

#[test]
fn wp_c15_a_stalled_reply_times_out_and_comes_late_without_harm() {
    let hub = open_hub();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    bind(&core);
    faults.swallow_next_cmd.store(true, Ordering::SeqCst);
    let start = Instant::now();
    assert!(core.set_master_volume(-30.0).is_err());
    assert!(start.elapsed() < Duration::from_secs(12), "bounded");
    let late = guard(&faults.late).take().expect("the reply was held");
    guard(&faults.inject).push_back(late);
    // An unknown tag, then the session goes on.
    core.set_master_volume(-31.0).unwrap();
    assert_eq!(core.get_state().master_volume_db, -31.0);
}

// ── Threads ────────────────────────────────────────────────────────────

/// Calls the core from inside every callback.
struct Reentrant {
    core: Arc<DspiCore>,
    calls: AtomicUsize,
}

impl LinkListener for Reentrant {
    fn on_link_devices_changed(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _ = self.core.link_devices();
    }
    fn on_link_closed(&self, _code: i32, _reason: String) {
        let _ = self.core.link_status();
    }
    fn on_link_status_changed(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _ = self.core.link_status();
        let _ = self.core.get_state();
    }
}

#[test]
fn wp_c15_a_listener_calling_back_does_not_deadlock() {
    let hub = open_hub();
    let core = DspiCore::new();
    let l = Arc::new(Reentrant {
        core: Arc::clone(&core),
        calls: AtomicUsize::new(0),
    });
    core.set_link_listener(Some(l.clone()));
    connect_faulty(&core, &hub, "");
    bind(&core);
    hub.close_all(1001);
    wait_until("a callback", || l.calls.load(Ordering::SeqCst) > 1);
    wait_phase(&core, LinkPhase::Bound);
    core.set_link_listener(None);
}

#[test]
fn wp_c15_a_session_end_during_a_setter_does_not_deadlock() {
    // A failed send reports the end on the setter's thread, under the lock.
    let hub = open_hub();
    let core = DspiCore::new();
    let l = Arc::new(Reentrant {
        core: Arc::clone(&core),
        calls: AtomicUsize::new(0),
    });
    core.set_link_listener(Some(l));
    connect_faulty(&core, &hub, "");
    bind(&core);
    let client = core.lock().link.as_ref().unwrap().client.clone().unwrap();
    client.close();
    let _ = core.set_master_volume(-3.0);
    wait_until("the core to answer", || {
        core.link_status().phase != LinkPhase::Bound
    });
    core.set_link_listener(None);
}

#[test]
fn wp_c15_the_device_lock_is_not_waited_for_under_the_core_lock() {
    let hub = open_hub();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    bind(&core);
    *guard(&faults.hold_text) = Some(("lock.acquire".into(), 700));
    let worker = {
        let core = Arc::clone(&core);
        std::thread::spawn(move || core.apply_all_params())
    };
    wait_until("the request", || faults.sent("lock.acquire") == 1);
    let start = Instant::now();
    let _ = core.get_state();
    assert!(
        start.elapsed() < Duration::from_millis(300),
        "the core stayed free: {:?}",
        start.elapsed()
    );
    worker.join().unwrap().unwrap();
    assert_eq!(faults.sent("lock.release"), 1);
}

#[test]
fn wp_c15_another_consoles_lock_is_reported() {
    let hub = open_hub();
    let core = DspiCore::new();
    connect_faulty(&core, &hub, "");
    bind(&core);
    hub.hold_lock(99, 5000);
    assert_eq!(core.save_params(), Err(DspiError::Locked));
    assert_eq!(core.apply_all_params(), Err(DspiError::Locked));
    hub.release_lock();
    core.apply_all_params().unwrap();
    wait_until("the release", || hub.lock_holder().is_none());
}

#[test]
fn wp_c15_a_reopen_during_the_lock_wait_gives_the_lock_back() {
    let hub = open_hub();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    bind(&core);
    *guard(&faults.hold_text) = Some(("lock.acquire".into(), 400));
    let worker = {
        let core = Arc::clone(&core);
        std::thread::spawn(move || core.save_params())
    };
    wait_until("the request", || faults.sent("lock.acquire") == 1);
    core.disconnect();
    assert_eq!(worker.join().unwrap(), Err(DspiError::Disconnected));
    assert_eq!(faults.sent("lock.release"), 1);
}

// ── Hub slots ──────────────────────────────────────────────────────────

#[test]
fn wp_c15_selecting_again_keeps_one_session() {
    let hub = open_hub();
    let core = DspiCore::new();
    connect_faulty(&core, &hub, "");
    bind(&core);
    bind(&core);
    wait_phase(&core, LinkPhase::Bound);
    assert_eq!(hub.sessions().len(), 1);
}

#[test]
fn wp_c15_a_new_connect_closes_the_old_session() {
    let hub = open_hub();
    let core = DspiCore::new();
    connect_faulty(&core, &hub, "");
    bind(&core);
    connect_faulty(&core, &hub, "");
    wait_until("one session", || hub.sessions().len() == 1);
    assert!(!core.is_connected(), "its device went with it");
}

#[test]
fn wp_c15_a_displaced_connection_closes_the_session() {
    let hub = open_hub();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    bind(&core);
    wait_phase(&core, LinkPhase::Bound);
    core.open_demo().unwrap();
    wait_until("the slot back", || hub.sessions().is_empty());
    assert_eq!(core.link_status().phase, LinkPhase::Idle);
    assert!(core.is_connected(), "the new device stays");
    let hellos = faults.sent("\"hello\"");
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(faults.sent("\"hello\""), hellos, "no reconnect");
}

#[test]
fn wp_c15_a_reconnect_leaves_one_session() {
    let hub = open_hub();
    let core = DspiCore::new();
    connect_faulty(&core, &hub, "");
    bind(&core);
    let generation = core.lock().conn().unwrap().generation;
    hub.close_all(1001);
    wait_until("the rebind", || {
        core.link_status().phase == LinkPhase::Bound
            && core.lock().conn().is_ok_and(|c| c.generation != generation)
    });
    wait_until("one session", || hub.sessions().len() == 1);
}

// ── No board (finding F9) ──────────────────────────────────────────────

/// Every device reads `state` in device messages.
fn device_state(faults: &Faults, state: &'static str) {
    faults.set_rewrite(move |t| {
        if !t.contains("\"state\"") {
            return None;
        }
        let mut v: Value = serde_json::from_str(t).ok()?;
        if let Some(list) = v.get_mut("devices").and_then(Value::as_array_mut) {
            for d in list {
                d["state"] = json!(state);
            }
        }
        if let Some(d) = v.get_mut("device") {
            d["state"] = json!(state);
        }
        Some(v.to_string())
    });
}

#[test]
fn wp_c15_paired_with_no_board_is_its_own_phase() {
    let hub = pin_hub();
    let core = DspiCore::new();
    let faults = Arc::new(Faults::default());
    device_state(&faults, "offline");
    connect_with(&core, &hub, "", &faults).unwrap();
    assert_eq!(core.link_status().phase, LinkPhase::NeedsPairing);
    core.link_pair("123456".into()).unwrap();
    wait_phase(&core, LinkPhase::NoDevice);
    let st = core.link_status();
    assert!(st.ready && st.connected && !st.closed);
}

#[test]
fn wp_c15_a_board_that_comes_back_is_bound_again() {
    let hub = open_hub();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    bind(&core);
    let generation = core.lock().conn().unwrap().generation;

    device_state(&faults, "offline");
    // `lock.acquire` makes the fake hub announce `device.changed`.
    hub.hold_lock(0, 0);
    let client = core.lock().link.as_ref().unwrap().client.clone().unwrap();
    client.lock_acquire(0, "t", 1).unwrap();
    wait_phase(&core, LinkPhase::NoDevice);
    assert_eq!(core.lock().conn().unwrap().generation, generation);

    device_state(&faults, "online");
    client.lock_acquire(0, "t", 1).unwrap();
    wait_until("the rebind", || {
        core.link_status().phase == LinkPhase::Bound
            && core.lock().conn().is_ok_and(|c| c.generation != generation)
    });
}

// ── Meters ─────────────────────────────────────────────────────────────

/// Ask until a frame of the feed is there. (The fixture's block is mostly
/// zero, so the frame shows in the feed rather than in the record.)
fn meters_live(core: &DspiCore) {
    let side = bound_side(core);
    guard(&side.feed).latest.remove(&METER_SLOT);
    wait_until("a meter frame", || {
        core.fetch_status().unwrap();
        guard(&side.feed).latest.contains_key(&METER_SLOT)
    });
}

fn tunnelled_status(hub: &FakeHub) -> usize {
    hub.log().iter().filter(|e| e.2 == req::GET_STATUS).count()
}

/// The grant the hub would make, changed by `f`.
fn grant(faults: &Faults, f: impl Fn(u64) -> Option<u64> + Send + Sync + 'static) {
    faults.set_rewrite(move |t| {
        if !t.contains("\"granted\"") {
            return None;
        }
        let mut v: Value = serde_json::from_str(t).ok()?;
        let id = v["id"].clone();
        for g in v["granted"].as_array_mut()? {
            match f(g["hz"].as_u64()?) {
                Some(hz) => g["hz"] = json!(hz),
                None => return Some(json!({"t": "err", "id": id, "code": "busy"}).to_string()),
            }
        }
        Some(v.to_string())
    });
}

#[test]
fn wp_c15_meters_come_from_the_poll_feed_and_stop_when_nobody_asks() {
    let hub = open_hub();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    bind(&core);
    // The first call has no frame yet: silence, not an error.
    let first = core.fetch_status().unwrap();
    assert!(first.peaks.iter().all(|p| *p == 0.0));
    meters_live(&core);
    assert_eq!(tunnelled_status(&hub), 0, "no round trips");
    assert_eq!(faults.sent("poll.subscribe"), 1);
    let sub = guard(&faults.outbound)
        .iter()
        .find(|t| t.contains("poll.subscribe"))
        .cloned()
        .unwrap();
    let v: Value = serde_json::from_str(&sub).unwrap();
    assert_eq!(v["polls"][0]["req"], json!(op::REQ_GET_STATUS));
    assert_eq!(v["polls"][0]["val"], json!(9));
    let n = u64::from(core.get_state().num_channels);
    assert_eq!(v["polls"][0]["len"], json!(n * 2 + 7));

    // The UI stops asking.
    wait_until("the unsubscribe", || faults.sent("poll.unsubscribe") == 1);
    assert_eq!(faults.sent("poll.subscribe"), 1);
    // It asks again.
    meters_live(&core);
    assert_eq!(faults.sent("poll.subscribe"), 2);
}

#[test]
fn wp_c15_meters_subscribe_again_after_a_reconnect() {
    let hub = open_hub();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    bind(&core);
    meters_live(&core);
    let generation = core.lock().conn().unwrap().generation;
    hub.close_all(1001);
    wait_until("the rebind", || {
        core.link_status().phase == LinkPhase::Bound
            && core.lock().conn().is_ok_and(|c| c.generation != generation)
    });
    meters_live(&core);
    assert_eq!(faults.sent("poll.subscribe"), 2);
    assert_eq!(tunnelled_status(&hub), 0);
}

#[test]
fn wp_c15_a_reduced_grant_is_taken() {
    let hub = open_hub();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    grant(&faults, |_| Some(2));
    bind(&core);
    meters_live(&core);
    let side = core
        .lock()
        .link
        .as_ref()
        .and_then(|s| s.bound.as_ref())
        .and_then(|b| b.side.clone())
        .unwrap();
    assert_eq!(guard(&side.feed).meter_hz, Some(2));
    assert_eq!(tunnelled_status(&hub), 0);
}

#[test]
fn wp_c15_a_refused_poll_falls_back_to_slow_reads_and_asks_again() {
    let hub = open_hub();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    grant(&faults, |_| None);
    bind(&core);
    let _ = core.fetch_status().unwrap();
    wait_until("the refusal", || {
        core.lock()
            .link
            .as_ref()
            .and_then(|s| s.bound.as_ref())
            .and_then(|b| b.side.as_ref())
            .is_some_and(|s| guard(&s.feed).meter_refused_until.is_some())
    });
    let start = Instant::now();
    let mut calls = 0;
    while start.elapsed() < Duration::from_millis(500) {
        core.fetch_status().unwrap();
        calls += 1;
    }
    let reads = tunnelled_status(&hub);
    assert!(
        calls > 20 && (1..=7).contains(&reads),
        "{calls} calls, {reads} reads"
    );

    // The refusal runs out; the hub has room again.
    *guard(&faults.rewrite) = None;
    {
        let inner = core.lock();
        let side = inner.link.as_ref().unwrap().bound.as_ref().unwrap();
        guard(&side.side.as_ref().unwrap().feed).meter_refused_until = Some(Instant::now());
    }
    wait_until("the grant", || {
        let _ = core.fetch_status();
        core.lock()
            .link
            .as_ref()
            .and_then(|s| s.bound.as_ref())
            .and_then(|b| b.side.as_ref())
            .is_some_and(|s| guard(&s.feed).meter_hz.is_some())
    });
}

#[test]
fn wp_c15_a_hub_without_polls_is_read_slowly() {
    let hub = FakeHub::start(HubConfig {
        caps: ["cmd", "notify", "lock"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        ..HubConfig::default()
    })
    .unwrap();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    bind(&core);
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(300) {
        core.fetch_status().unwrap();
    }
    let reads = tunnelled_status(&hub);
    assert!((1..=5).contains(&reads), "{reads}");
    assert_eq!(faults.sent("poll.subscribe"), 0);
}

#[test]
fn wp_c15_a_meter_poll_never_delays_a_user_command() {
    // A slow link: every frame out takes 40 ms, one at a time, with a budget
    // the hub cuts the grant to. A tunnelled meter loop would queue with the
    // command; the feed adds nothing to the link at all.
    let hub = open_hub();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    grant(&faults, |_| Some(5));
    bind(&core);
    meters_live(&core);
    faults.out_delay_ms.store(40, Ordering::SeqCst);

    let stop = Arc::new(AtomicBool::new(false));
    let spinner = {
        let (core, stop) = (Arc::clone(&core), Arc::clone(&stop));
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                core.fetch_status().unwrap();
                std::thread::sleep(Duration::from_millis(1));
            }
        })
    };
    let mut slowest = Duration::ZERO;
    for i in 0..15 {
        let t = Instant::now();
        core.set_master_volume(-20.0 - i as f32).unwrap();
        slowest = slowest.max(t.elapsed());
    }
    stop.store(true, Ordering::SeqCst);
    spinner.join().unwrap();
    // One frame's 40 ms plus scheduling: not a queue of meter reads.
    assert!(slowest < Duration::from_millis(250), "{slowest:?}");
    assert_eq!(tunnelled_status(&hub), 0, "no meter read on the link");
    assert_eq!(faults.sent("poll.subscribe"), 1);
}

// ── The analyser over Link ─────────────────────────────────────────────

/// A band frame the fake firmware does not have, as the hub would push it.
fn inject_band(faults: &Faults, ch: u8, seq: u8) {
    let frame = RtaBandFrame {
        channel: ch,
        seq,
        n_bands: 34,
        age_ms: 5,
        avg: [200; RtaBandFrame::MAX_BANDS],
        peak: [210; RtaBandFrame::MAX_BANDS],
    };
    let poll = Frame::Poll(PollFrame {
        seq: u16::from(seq),
        handle: 0,
        slot: band_slot(ch).unwrap(),
        payload: frame.encode().to_vec(),
    });
    guard(&faults.inject).push_back(WsMessage::Binary(poll.encode()));
}

fn bound_side(core: &DspiCore) -> Arc<LinkSide> {
    core.lock()
        .link
        .as_ref()
        .and_then(|s| s.bound.as_ref())
        .and_then(|b| b.side.clone())
        .unwrap()
}

#[test]
fn wp_c15_band_rate_is_kept_inside_the_budget() {
    // A 1 Mbaud bridge: half of 40 kB/s.
    assert_eq!(band_hz(1, 20, 40_000), 15);
    assert_eq!(band_hz(9, 20, 40_000), 15);
    assert_eq!(band_hz(16, 20, 40_000), 13);
    // A slower UART.
    assert_eq!(band_hz(9, 20, 4_000), 2);
    assert_eq!(band_hz(9, 20, 100), 1, "the hub's floor; it may say busy");
    assert_eq!(band_hz(9, 5, 0), 5, "no stated budget");
    assert_eq!(band_hz(2, 0, 0), 15);
}

#[test]
fn wp_c15_band_frames_come_from_polls_on_a_bridge() {
    let hub = FakeHub::start(HubConfig {
        auth: Auth::None,
        ..HubConfig::bridge()
    })
    .unwrap();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    bind(&core);
    let side = bound_side(&core);

    let mut t = Arc::clone(&side).rta_transport(0b11);
    assert_eq!(faults.sent("poll.subscribe"), 1);
    let v: Value = {
        let out = guard(&faults.outbound);
        serde_json::from_str(out.iter().find(|t| t.contains("poll.subscribe")).unwrap()).unwrap()
    };
    let polls = v["polls"].as_array().unwrap();
    assert_eq!(polls.len(), 2);
    assert_eq!(polls[1]["req"], json!(op::REQ_RTA_GET_BANDS));
    assert_eq!(polls[1]["val"], json!(1));
    assert_eq!(polls[1]["slot"], json!(2));
    assert_eq!(polls[1]["len"], json!(RtaBandFrame::SIZE));
    // Same channels: no more traffic.
    drop(t);
    t = Arc::clone(&side).rta_transport(0b11);
    assert_eq!(faults.sent("poll.subscribe"), 1);

    // The feed answers; the link is not asked for 0x0B or 0x0F.
    inject_band(&faults, 0, 1);
    inject_band(&faults, 1, 1);
    hub.resync(); // wakes the reader
    let size = RtaBandFrame::SIZE as u16;
    wait_until("the frames", || {
        hub.resync();
        drop(Arc::clone(&side).rta_transport(0b11));
        t.control_in(op::REQ_RTA_GET_BANDS, 1, size).is_ok()
            && t.control_in_upto(op::REQ_RTA_GET_BANDS_ALL, 0, size * 2)
                .is_ok_and(|all| all.len() == usize::from(size) * 2)
    });
    let tunnelled = hub
        .log()
        .iter()
        .filter(|e| e.2 == op::REQ_RTA_GET_BANDS || e.2 == op::REQ_RTA_GET_BANDS_ALL)
        .count();
    assert_eq!(tunnelled, 0);

    // A pushed configuration: frames of the old one are forgotten.
    let _ = t.control_out(op::REQ_RTA_SET_CONFIG, 0, &[0; 4]);
    assert!(t.control_in(op::REQ_RTA_GET_BANDS, 1, size).is_err());

    // Nobody watches: the hub may stop polling.
    drop(Arc::clone(&side).rta_transport(0));
    assert_eq!(faults.sent("poll.unsubscribe"), 1);
    assert_eq!(guard(&side.feed).band_granted, 0);
}

#[test]
fn wp_c15_a_refused_band_poll_throttles_tunnelled_reads_on_a_bridge() {
    let hub = FakeHub::start(HubConfig {
        auth: Auth::None,
        ..HubConfig::bridge()
    })
    .unwrap();
    let core = DspiCore::new();
    let (faults, _) = connect_faulty(&core, &hub, "");
    grant(&faults, |_| None);
    bind(&core);
    let side = bound_side(&core);
    let size = RtaBandFrame::SIZE as u16;

    let mut t = Arc::clone(&side).rta_transport(0b1);
    assert!(guard(&side.feed).band_refused_until.is_some());
    // The first poll may read (the fake firmware has no analyser: a stall).
    assert!(t.control_in(op::REQ_RTA_GET_BANDS, 0, size).is_err());
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(500) {
        let mut t = Arc::clone(&side).rta_transport(0b1);
        let _ = t.control_in(op::REQ_RTA_GET_BANDS, 0, size);
        std::thread::sleep(Duration::from_millis(10));
    }
    let reads = hub
        .log()
        .iter()
        .filter(|e| e.2 == op::REQ_RTA_GET_BANDS)
        .count();
    assert!((2..=4).contains(&reads), "{reads}");
    assert_eq!(faults.sent("poll.subscribe"), 1, "asked again only later");
}

#[test]
fn wp_c15_bins_on_a_bridge_are_read_at_a_low_rate() {
    let hub = FakeHub::start(HubConfig {
        auth: Auth::None,
        ..HubConfig::bridge()
    })
    .unwrap();
    let core = DspiCore::new();
    connect_faulty(&core, &hub, "");
    bind(&core);
    let side = bound_side(&core);
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(500) {
        let mut t = Arc::clone(&side).rta_transport(0);
        let _ = t.control_in_upto(op::REQ_RTA_GET_BINS, 0, 64);
        std::thread::sleep(Duration::from_millis(10));
    }
    let reads = hub
        .log()
        .iter()
        .filter(|e| e.2 == op::REQ_RTA_GET_BINS)
        .count();
    assert!((2..=4).contains(&reads), "{reads}");
}
