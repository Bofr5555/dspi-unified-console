//! WP WB4b: a caller-driven session (the web's: no reader or keepalive thread)
//! against the in-process fake hub. The same client, run the way the browser
//! runs it: the socket is read only while a call waits or `service` turns it.

use std::time::Duration;

use dspi_link::client::set_caller_driven_on_this_thread;
use dspi_link::fakehub::{req, Auth, FakeHub, HubConfig, OFF_MASTER_VOLUME};
use dspi_link::frames::status;
use dspi_link::socket::TcpConnector;
use dspi_link::{AuthState, ConnectOptions, Dir, LinkClient, LinkError, SessionConfig, SessionState, Timing};

fn opts(token: Option<String>) -> ConnectOptions {
    ConnectOptions { client_name: "wb4b".into(), app: "DSPi Console (web)".into(), app_version: "test".into(), token }
}

fn connect(hub: &FakeHub, token: Option<String>, cfg: &SessionConfig) -> (LinkClient, AuthState) {
    set_caller_driven_on_this_thread(true);
    let r = LinkClient::connect_with(&TcpConnector::default(), &hub.url(), &opts(token), None, cfg).unwrap();
    set_caller_driven_on_this_thread(false);
    r
}

/// Turn the session until `f` holds, up to 3 s.
fn serve_until(c: &LinkClient, mut f: impl FnMut() -> bool) -> bool {
    let end = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < end {
        c.service();
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

#[test]
fn wp_wb4b_a_caller_driven_session_connects_commands_and_reads_only_when_turned() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let (c, state) = connect(&hub, None, &SessionConfig::default());
    assert_eq!(state, AuthState::Ready);
    assert_eq!(c.devices().len(), 1, "the device list is read during connect");
    assert_eq!(c.session_state(), SessionState::Online);

    // A write with read-back: the call reads the socket itself while it waits.
    let w = c.command(0, Dir::Set, req::SET_MASTER_VOLUME, 0, 2, 4, &(-9.0f32).to_le_bytes());
    assert_eq!(w.status, status::OK);
    assert_eq!(&hub.image()[OFF_MASTER_VOLUME as usize..][..4], &(-9.0f32).to_le_bytes());
    let bulk = c.command(0, Dir::Get, req::GET_ALL_PARAMS, 0, 2, 6136, &[]);
    assert_eq!(bulk.status, status::OK);
    assert_eq!(&bulk.payload[OFF_MASTER_VOLUME as usize..][..4], &(-9.0f32).to_le_bytes(), "read back");

    // Nobody reads between calls: another client's change waits on the socket
    // until the session is turned.
    let notes = c.notifications(0);
    hub.external_change(OFF_MASTER_VOLUME, &(-3.0f32).to_le_bytes(), 1, 0);
    std::thread::sleep(Duration::from_millis(200));
    assert!(notes.try_recv().is_err(), "nothing is read without a turn");
    let mut got = None;
    assert!(serve_until(&c, || {
        got = notes.try_recv().ok();
        got.is_some()
    }));
    assert_eq!(got.unwrap().origin, 0);
    c.close();
    assert_eq!(c.session_state(), SessionState::Closed);
}

#[test]
fn wp_wb4b_b_pairing_with_a_pin_gives_a_token_that_opens_the_next_session() {
    let hub = FakeHub::start(HubConfig { auth: Auth::Fixed("482913".into()), ..HubConfig::default() }).unwrap();
    let (c, state) = connect(&hub, None, &SessionConfig::default());
    assert!(matches!(state, AuthState::NeedsPairing { .. }));
    assert!(matches!(c.pair("000000", "control"), Err(LinkError::WrongPin { .. })));
    let token = c.pair("482913", "control").unwrap();
    assert!(!token.is_empty());
    assert_eq!(c.session_state(), SessionState::Online);
    c.close();

    let (c2, state) = connect(&hub, Some(token), &SessionConfig::default());
    assert_eq!(state, AuthState::Ready);
    assert_eq!(c2.devices().len(), 1);
}

fn quick() -> SessionConfig {
    SessionConfig {
        timing: Timing {
            request_timeout: Duration::from_millis(300),
            ping_interval: Duration::from_millis(100),
            silence_limit: Duration::from_millis(800),
            ..Timing::default()
        },
        ..SessionConfig::default()
    }
}

#[test]
fn wp_wb4b_c_a_timed_out_command_is_not_repeated_and_the_session_stays() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let (c, _) = connect(&hub, None, &quick());
    let before = hub.log().len();
    hub.drop_next_replies(1);
    let w = c.command(0, Dir::Set, req::SET_MASTER_VOLUME, 0, 2, 4, &(-7.0f32).to_le_bytes());
    assert_eq!(w.status, status::TIMEOUT);
    assert_eq!(hub.log().len(), before + 1, "the write went out once and was never sent again");
    // The hub answers pings: turning the session past the silence limit keeps it.
    let end = std::time::Instant::now() + Duration::from_millis(1500);
    while std::time::Instant::now() < end {
        c.service();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!c.is_closed(), "a live hub keeps the session");
    assert_eq!(c.command(0, Dir::Get, req::GET_PLATFORM, 0, 2, 7, &[]).status, status::OK);
}

#[test]
fn wp_wb4b_d_silence_past_the_limit_ends_the_session_only_on_a_turn() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let (c, _) = connect(&hub, None, &quick());
    hub.stall(true);
    assert!(serve_until(&c, || c.is_closed()), "a silent hub's session ends");
    let (code, _) = c.closed_reason().unwrap();
    assert!(dspi_link::should_reconnect(code), "a lost link is one to reconnect: {code:?}");
}
