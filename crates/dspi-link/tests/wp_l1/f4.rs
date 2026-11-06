//! F4: the token reaches the caller first; a failed device list is non-fatal.

use std::sync::Arc;
use std::time::Duration;

use dspi_link::fakehub::{Auth, DeviceListFault, FakeHub, HubConfig};
use dspi_link::socket::TcpConnector;
use dspi_link::{AuthState, LinkClient, LinkError, SessionConfig, SessionState, Timing};

use crate::support::*;

fn window_hub() -> FakeHub {
    FakeHub::start(HubConfig { auth: Auth::Window("123456".into()), ..HubConfig::default() }).unwrap()
}

fn fast() -> SessionConfig {
    SessionConfig {
        timing: Timing { request_timeout: Duration::from_millis(400), ..Timing::default() },
        ..SessionConfig::default()
    }
}

fn connect(hub: &FakeHub, ev: &Arc<Recorder>, cfg: &SessionConfig) -> LinkClient {
    let events = Arc::clone(ev) as Arc<dyn dspi_link::LinkEvents>;
    LinkClient::connect_with(&TcpConnector::default(), &hub.url(), &opts(None), Some(events), cfg).unwrap().0
}

#[test]
fn wp_l1_f4_a_failed_list_costs_neither_the_token_nor_the_session() {
    for fault in [DeviceListFault::Error("internal".into()), DeviceListFault::Silent] {
        let hub = window_hub();
        let ev = Arc::new(Recorder::default());
        let c = connect(&hub, &ev, &fast());
        hub.fail_device_list(fault.clone(), 1);
        let token = c.pair("123456", "control").unwrap();
        assert!(!token.is_empty(), "{fault:?}");
        assert!(!c.is_closed());
        assert_eq!(c.session_state(), SessionState::DeviceListUnknown, "{fault:?}");
        assert!(c.device_list_error().is_some());
        assert_eq!(*ev.tokens.lock().unwrap(), [(c.hello().hub.id.clone(), token.clone())]);

        // The Mac hub's window is spent: the token is what gets back in
        assert_eq!(c.refresh_devices().unwrap().len(), 1);
        assert_eq!(c.session_state(), SessionState::Online);
        assert_eq!(c.device_list_error(), None);
        c.close();
        let (_c, state) = LinkClient::connect(&TcpConnector::default(), &hub.url(), &opts(Some(token)), None).unwrap();
        assert_eq!(state, AuthState::Ready);
    }
}

#[test]
fn wp_l1_f4_the_token_is_stored_before_the_list_is_even_asked_for() {
    let hub = window_hub();
    let ev = Arc::new(Recorder::default());
    let c = connect(&hub, &ev, &SessionConfig::default());
    hub.fail_device_list(DeviceListFault::Silent, 1);
    let pairing = {
        let c = c.clone();
        std::thread::spawn(move || c.pair("123456", "control"))
    };
    assert!(wait(|| !ev.tokens.lock().unwrap().is_empty()), "while pair still waits for the list");
    assert!(!pairing.is_finished());
    c.close(); // Ends the wait
    assert!(pairing.join().unwrap().is_ok(), "the session ending after the token does not lose it");
}

#[test]
fn wp_l1_f4_a_pair_reply_arriving_after_the_timeout_still_delivers_the_token() {
    let hub = window_hub();
    let ev = Arc::new(Recorder::default());
    let c = connect(&hub, &ev, &fast());
    hub.hold_replies();
    assert_eq!(c.pair("123456", "control").unwrap_err(), LinkError::Timeout);
    assert_eq!(c.session_state(), SessionState::NeedsAuth);
    hub.release_replies();
    assert!(wait(|| !ev.tokens.lock().unwrap().is_empty()), "the window is spent, so this is the only copy");
    let token = ev.tokens.lock().unwrap()[0].1.clone();
    assert_eq!(c.session_state(), SessionState::DeviceListUnknown, "authenticated, list not read");
    c.refresh_devices().unwrap();
    assert_eq!(c.session_state(), SessionState::Online);
    c.close();
    let (_c, state) = LinkClient::connect(&TcpConnector::default(), &hub.url(), &opts(Some(token)), None).unwrap();
    assert_eq!(state, AuthState::Ready);
}

#[test]
fn wp_l1_f4_paired_with_no_board_is_its_own_state() {
    let hub = FakeHub::start(HubConfig::bridge()).unwrap();
    hub.set_board_present(false);
    let (c, _) = LinkClient::connect(&TcpConnector::default(), &hub.url(), &opts(None), None).unwrap();
    assert_eq!(c.session_state(), SessionState::NeedsAuth);
    c.pair("482913", "control").unwrap();
    assert_eq!(c.session_state(), SessionState::NoBoard);
    assert!(!c.is_closed());
    hub.set_board_present(true);
    c.refresh_devices().unwrap();
    assert_eq!(c.session_state(), SessionState::Online);
    c.close();
    assert_eq!(c.session_state(), SessionState::Closed);
}
