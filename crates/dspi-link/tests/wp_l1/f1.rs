//! F1: control frames count as liveness.

use std::time::Duration;

use dspi_link::fakehub::HubConfig;
use dspi_link::socket::{TcpConnector, WsRecv};
use dspi_link::{LinkClient, SessionState};

use crate::support::*;

#[test]
fn wp_l1_f1_control_frames_are_liveness() {
    let clock = manual_clock();
    let (hub, c) = mock_session(&clock, session_on(&clock));
    for _ in 0..5 {
        clock.advance(Duration::from_secs(30));
        hub.push(WsRecv::Alive);
        hub.settle();
        assert!(!c.is_closed(), "30 s after a pong is within the limit");
    }
    clock.advance(Duration::from_secs(36));
    assert!(wait(|| c.is_closed()), "silence, control frames included, still ends it");
    assert_eq!(c.closed_reason().unwrap().0, None);
}

/// The whole path through real sockets: the hub's pings keep an idle session up
/// for more than three silence limits.
#[test]
fn wp_l1_f1_healthy_idle_session_stays_up_past_the_silence_limit() {
    let clock = manual_clock();
    let hub = hub_on(&clock, HubConfig::default());
    let (c, _) =
        LinkClient::connect_with(&TcpConnector::default(), &hub.url(), &opts(None), None, &session_on(&clock)).unwrap();
    for _ in 0..12 {
        let pongs = hub.pongs_received();
        clock.advance(Duration::from_secs(10));
        assert!(wait(|| hub.pongs_received() > pongs), "the hub's ping was answered");
    }
    // The real socket reports silence to the client's reader once a second
    std::thread::sleep(Duration::from_millis(1200));
    assert!(!c.is_closed(), "dropped: {:?}", c.closed_reason());
    assert_eq!(c.session_state(), SessionState::Online);
    assert_eq!(hub.slots_in_use(), 1);
}

#[test]
fn wp_l1_f1_a_dark_network_still_ends_the_session() {
    let clock = manual_clock();
    let hub = hub_on(&clock, HubConfig::default());
    let (c, _) =
        LinkClient::connect_with(&TcpConnector::default(), &hub.url(), &opts(None), None, &session_on(&clock)).unwrap();
    hub.stall(true);
    clock.advance(Duration::from_secs(36));
    assert!(wait(|| c.is_closed()));
    assert_eq!(c.closed_reason().unwrap(), (None, "connection lost".to_owned()));
}
