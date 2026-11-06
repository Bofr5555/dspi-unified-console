//! Faults the findings do not name, and which must stay harmless.

use std::sync::Arc;
use std::time::Duration;

use dspi_link::fakehub::{req, FakeHub, HubConfig, OFF_MASTER_VOLUME};
use dspi_link::socket::TcpConnector;
use dspi_link::{status, AuthState, Dir, LinkClient, LinkEvents, SessionConfig, Timing};

use crate::support::*;

fn connect(hub: &FakeHub, token: Option<String>, cfg: &SessionConfig) -> (LinkClient, AuthState) {
    LinkClient::connect_with(&TcpConnector::default(), &hub.url(), &opts(token), None, cfg).unwrap()
}

#[test]
fn wp_l1_a_late_reply_cannot_answer_the_next_command() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let cfg = SessionConfig {
        timing: Timing { request_timeout: Duration::from_millis(300), ..Timing::default() },
        ..SessionConfig::default()
    };
    let (c, _) = connect(&hub, None, &cfg);
    hub.hold_replies();
    assert_eq!(c.command(0, Dir::Get, req::GET_PLATFORM, 0, 2, 7, &[]).status, status::TIMEOUT);
    hub.release_replies();
    let serial = c.command(0, Dir::Get, req::GET_SERIAL, 0, 2, 16, &[]);
    assert_eq!(serial.status, status::OK);
    assert_eq!(serial.payload, b"0123456789ABCDEF", "not the platform bytes that came late");
}

#[test]
fn wp_l1_duplicate_frames_answer_once() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let (c, _) = connect(&hub, None, &SessionConfig::default());
    hub.duplicate_frames(true);
    for _ in 0..5 {
        assert_eq!(c.command(0, Dir::Get, req::GET_PLATFORM, 0, 2, 7, &[]).payload.len(), 7);
        assert_eq!(c.refresh_devices().unwrap().len(), 1);
    }
    // A duplicated NOTIFY reads as a gap: the safe answer is to re-read
    let rx = c.notifications(0);
    hub.external_change(OFF_MASTER_VOLUME, &[0; 4], 5, 0);
    let first = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(&first.packet[..2], &[0x02, 0x02]);
    let second = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(&second.packet[..2], &[0x02, 0x03], "BULK_INVALIDATED");
    assert!(!c.is_closed());
}

#[test]
fn wp_l1_a_dropped_reply_times_out_and_the_session_goes_on() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let cfg = SessionConfig {
        timing: Timing { request_timeout: Duration::from_millis(300), ..Timing::default() },
        ..SessionConfig::default()
    };
    let (c, _) = connect(&hub, None, &cfg);
    hub.drop_next_replies(1);
    assert_eq!(c.command(0, Dir::Get, req::GET_PLATFORM, 0, 2, 7, &[]).status, status::TIMEOUT);
    assert_eq!(c.command(0, Dir::Get, req::GET_PLATFORM, 0, 2, 7, &[]).status, status::OK);
}

#[test]
fn wp_l1_a_dropped_connection_reads_as_connection_lost_and_reconnects() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let ev = Arc::new(Recorder::default());
    let events: Arc<dyn LinkEvents> = Arc::clone(&ev) as Arc<dyn LinkEvents>;
    let (c, _) = LinkClient::connect(&TcpConnector::default(), &hub.url(), &opts(None), Some(events)).unwrap();
    hub.drop_connections();
    assert!(wait(|| c.is_closed()));
    assert_eq!(c.closed_reason().unwrap(), (None, "connection lost".to_owned()), "no raw socket text");
    assert!(wait(|| ev.closed.lock().unwrap().len() == 1), "the event follows the state");
    let (c2, _) = LinkClient::connect(&TcpConnector::default(), &hub.url(), &opts(None), None).unwrap();
    assert_ne!(c2.session_id(), c.session_id());
}

#[test]
fn wp_l1_a_hub_restart_keeps_the_token_good() {
    let hub = FakeHub::start(HubConfig::bridge()).unwrap();
    let (c, _) = connect(&hub, None, &SessionConfig::default());
    let token = c.pair("482913", "control").unwrap();
    hub.restart();
    assert!(wait(|| c.is_closed()));
    assert!(dspi_link::should_reconnect(c.closed_reason().unwrap().0));
    assert!(wait(|| hub.slots_in_use() == 0));
    let (c2, state) = connect(&hub, Some(token), &SessionConfig::default());
    assert_eq!(state, AuthState::Ready);
    assert_eq!(c2.devices().len(), 1);
}

#[test]
fn wp_l1_a_hub_shutting_down_says_4003() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let (c, _) = connect(&hub, None, &SessionConfig::default());
    hub.stop();
    assert!(wait(|| c.is_closed()));
    assert_eq!(c.closed_reason().unwrap().0, Some(4003));
}

#[test]
fn wp_l1_commands_on_a_stalled_hub_stay_bounded() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let cfg = SessionConfig {
        timing: Timing { request_timeout: Duration::from_millis(30), ..Timing::default() },
        ..SessionConfig::default()
    };
    let (c, _) = connect(&hub, None, &cfg);
    hub.stall(true);
    let started = std::time::Instant::now();
    for _ in 0..3 {
        assert_eq!(c.command(0, Dir::Get, req::GET_PLATFORM, 0, 2, 7, &[]).status, status::TIMEOUT);
    }
    assert!(started.elapsed() < Duration::from_secs(5));
}
