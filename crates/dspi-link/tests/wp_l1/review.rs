//! Review 3-B: the fake hub's board, RATE_LIMITED and poll budget.

use std::sync::Arc;
use std::time::Duration;

use dspi_link::fakehub::{req, FakeHub, HubConfig};
use dspi_link::messages::PollSpec;
use dspi_link::socket::TcpConnector;
use dspi_link::{status, Dir, LinkClient, LinkError, LinkEvents, SessionConfig, SessionState, Timing};

use crate::support::*;

fn on_clock(hub: &FakeHub, clock: &Arc<dspi_link::clock::ManualClock>) -> LinkClient {
    LinkClient::connect_with(&TcpConnector::default(), &hub.url(), &opts(None), None, &session_on(clock)).unwrap().0
}

// ── The board (L1-1) ───────────────────────────────────────────────────

#[test]
fn wp_l1_board_goes_offline_then_removed_and_comes_back_added() {
    let clock = manual_clock();
    let hub = hub_on(&clock, HubConfig::default());
    let c = on_clock(&hub, &clock);
    let ev = Arc::new(Recorder::default());
    c.set_events(Some(Arc::clone(&ev) as Arc<dyn LinkEvents>));
    let rx = c.polls(0);
    let meter = PollSpec { slot: 2, req: req::GET_STATUS, val: 9, idx: 2, len: 41, hz: 20 };
    c.poll_subscribe(0, &[meter]).unwrap();
    assert!(rx.recv_timeout(Duration::from_secs(2)).is_ok());

    hub.set_board_present(false);
    assert!(wait(|| c.session_state() == SessionState::NoBoard), "device.changed, with no refresh");
    assert_eq!(c.devices()[0].state, "offline", "the handle is kept");
    assert_eq!(c.command(0, Dir::Get, req::GET_PLATFORM, 0, 2, 7, &[]).status, status::NO_DEVICE);
    assert!(matches!(c.snapshot(0), Err(LinkError::Rejected { code, .. }) if code == "no_device"));
    assert!(matches!(c.poll_subscribe(0, &[meter]), Err(LinkError::Rejected { code, .. }) if code == "no_device"));
    clock.advance(Duration::from_millis(100));
    while rx.recv_timeout(Duration::from_millis(30)).is_ok() {} // Frames already on the way
    clock.advance(Duration::from_millis(100));
    assert!(rx.recv_timeout(Duration::from_millis(30)).is_err(), "polls end when the device goes offline");

    clock.advance(Duration::from_secs(61));
    assert!(wait(|| c.devices().is_empty()), "device.removed after the hold");
    assert_eq!(c.session_state(), SessionState::NoBoard);
    assert!(c.refresh_devices().unwrap().is_empty());

    hub.set_board_present(true);
    assert!(wait(|| c.session_state() == SessionState::Online), "device.added");
    assert_eq!(c.devices()[0].state, "online");
    assert_eq!(c.command(0, Dir::Get, req::GET_PLATFORM, 0, 2, 7, &[]).status, status::OK);
    assert!(ev.devices.lock().unwrap().len() >= 3);
}

#[test]
fn wp_l1_board_back_within_the_hold_is_just_changed() {
    let clock = manual_clock();
    let hub = hub_on(&clock, HubConfig::default());
    let c = on_clock(&hub, &clock);
    hub.set_board_present(false);
    assert!(wait(|| c.session_state() == SessionState::NoBoard));
    clock.advance(Duration::from_secs(30));
    hub.set_board_present(true);
    assert!(wait(|| c.session_state() == SessionState::Online));
    clock.advance(Duration::from_secs(61));
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(c.devices().len(), 1, "the removal was called off");
}

#[test]
fn wp_l1_board_hold_is_configurable_and_a_second_session_hears_too() {
    let clock = manual_clock();
    let hub = hub_on(&clock, HubConfig { board_removal_hold: Duration::from_secs(5), ..HubConfig::default() });
    let a = on_clock(&hub, &clock);
    let b = on_clock(&hub, &clock);
    hub.set_board_present(false);
    assert!(wait(|| b.session_state() == SessionState::NoBoard));
    clock.advance(Duration::from_secs(6));
    assert!(wait(|| a.devices().is_empty() && b.devices().is_empty()));
}

// ── RATE_LIMITED (L1-2) ────────────────────────────────────────────────

#[test]
fn wp_l1_hub_answers_rate_limited_past_max_inflight() {
    let hub = FakeHub::start(HubConfig { max_inflight: 2, ..HubConfig::default() }).unwrap();
    let cfg = SessionConfig {
        timing: Timing { request_timeout: Duration::from_millis(400), ..Timing::default() },
        ..SessionConfig::default()
    };
    let (c, _) = LinkClient::connect_with(&TcpConnector::default(), &hub.url(), &opts(None), None, &cfg).unwrap();
    let get = || c.command(0, Dir::Get, req::GET_PLATFORM, 0, 2, 7, &[]).status;
    hub.hold_replies();
    // F10's slot mismatch: the client frees its slot at its timeout, the hub does not
    assert_eq!(get(), status::TIMEOUT);
    assert_eq!(get(), status::TIMEOUT);
    let ran = hub.log().len();
    assert_eq!(get(), status::RATE_LIMITED, "the hub's own answer, not held");
    assert_eq!(hub.log().len(), ran, "the command did not run");
    hub.release_replies();
    assert!(wait(|| get() != status::RATE_LIMITED && get() == status::OK));
}

#[test]
fn wp_l1_rate_limited_counts_per_session() {
    let hub = FakeHub::start(HubConfig { max_inflight: 1, ..HubConfig::default() }).unwrap();
    let cfg = SessionConfig {
        timing: Timing { request_timeout: Duration::from_millis(400), ..Timing::default() },
        ..SessionConfig::default()
    };
    let connect = || LinkClient::connect_with(&TcpConnector::default(), &hub.url(), &opts(None), None, &cfg).unwrap().0;
    let (a, b) = (connect(), connect());
    hub.hold_replies();
    assert_eq!(a.command(0, Dir::Get, req::GET_PLATFORM, 0, 2, 7, &[]).status, status::TIMEOUT);
    assert_eq!(a.command(0, Dir::Get, req::GET_PLATFORM, 0, 2, 7, &[]).status, status::RATE_LIMITED);
    assert_eq!(b.command(0, Dir::Get, req::GET_PLATFORM, 0, 2, 7, &[]).status, status::TIMEOUT, "b has its own count");
}

// ── The poll budget ────────────────────────────────────────────────────

/// A poll of `len` bytes costs `len + 18` bytes per tick.
fn poll(slot: u8, val: u16, len: u16, hz: u32) -> PollSpec {
    PollSpec { slot, req: req::GET_STATUS, val, idx: 2, len, hz }
}

fn budget_hub(clock: &Arc<dspi_link::clock::ManualClock>, bps: u32) -> FakeHub {
    hub_on(clock, HubConfig { poll_budget_bps: bps, ..HubConfig::default() })
}

#[test]
fn wp_l1_polls_are_cut_to_the_budget_and_the_hello_says_it() {
    let clock = manual_clock();
    let hub = budget_hub(&clock, 500);
    let c = on_clock(&hub, &clock);
    assert_eq!(c.hello().limits.poll_budget_bps, 500);
    // 41 + 18 = 59 bytes a tick: 500 / 59 = 8 Hz
    let g = c.poll_subscribe(0, &[poll(0, 9, 41, 20)]).unwrap();
    assert_eq!((g[0].slot, g[0].hz), (0, 8));
}

#[test]
fn wp_l1_polls_over_budget_answer_busy_and_change_nothing() {
    let clock = manual_clock();
    let hub = budget_hub(&clock, 100);
    let c = on_clock(&hub, &clock);
    let g = c.poll_subscribe(0, &[poll(0, 9, 41, 1)]).unwrap();
    assert_eq!(g[0].hz, 1);
    let e = c.poll_subscribe(0, &[poll(0, 9, 41, 1), poll(1, 3, 41, 1)]).unwrap_err();
    assert!(matches!(&e, LinkError::Rejected { code, .. } if code == "busy"), "{e:?}");
    // Slot 0 still runs, and slot 1 never started
    let rx = c.polls(0);
    while rx.recv_timeout(Duration::from_millis(20)).is_ok() {}
    clock.advance(Duration::from_millis(1001));
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap().slot, 0);
    assert!(rx.recv_timeout(Duration::from_millis(30)).is_err());
}

#[test]
fn wp_l1_the_budget_is_shared_and_identical_polls_count_once() {
    let clock = manual_clock();
    let hub = budget_hub(&clock, 118); // 59 x 2
    let (a, b) = (on_clock(&hub, &clock), on_clock(&hub, &clock));
    assert_eq!(a.poll_subscribe(0, &[poll(0, 9, 41, 2)]).unwrap()[0].hz, 2);
    assert_eq!(b.poll_subscribe(0, &[poll(5, 9, 41, 2)]).unwrap()[0].hz, 2, "the same poll: no extra cost");
    let e = b.poll_subscribe(0, &[poll(6, 3, 41, 1)]).unwrap_err();
    assert!(matches!(&e, LinkError::Rejected { code, .. } if code == "busy"), "another session's polls count");
    // Replacing a slot frees what it used
    assert_eq!(a.poll_subscribe(0, &[poll(0, 9, 41, 1)]).unwrap()[0].hz, 1);
    b.poll_unsubscribe(0, &[5]);
    assert!(wait(|| b.poll_subscribe(0, &[poll(6, 3, 41, 1)]).is_ok()), "room again");
}

#[test]
fn wp_l1_poll_frames_come_at_the_granted_rate() {
    let clock = manual_clock();
    let hub = budget_hub(&clock, 118);
    let c = on_clock(&hub, &clock);
    let rx = c.polls(0);
    let g = c.poll_subscribe(0, &[poll(2, 9, 41, 20)]).unwrap();
    assert_eq!(g[0].hz, 2, "asked for 20 Hz");
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap().slot, 2, "the first at once");
    clock.advance(Duration::from_millis(400));
    assert!(rx.recv_timeout(Duration::from_millis(40)).is_err(), "not at the 20 Hz it asked for");
    clock.advance(Duration::from_millis(100));
    assert!(rx.recv_timeout(Duration::from_secs(2)).is_ok(), "500 ms after the first");
}
