//! F6: every connect error path closes the session; events are bound to it.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use dspi_link::fakehub::{DeviceListFault, FakeHub, HubConfig};
use dspi_link::socket::TcpConnector;
use dspi_link::{LinkClient, LinkError, LinkEvents, SessionConfig, Timing};

use crate::support::*;

fn fast() -> SessionConfig {
    SessionConfig {
        timing: Timing { connect_budget: Duration::from_millis(500), ..Timing::default() },
        ..SessionConfig::default()
    }
}

fn events(r: &Arc<Recorder>) -> Option<Arc<dyn LinkEvents>> {
    Some(Arc::clone(r) as Arc<dyn LinkEvents>)
}

fn connect_ev(hub: &FakeHub, r: &Arc<Recorder>, cfg: &SessionConfig) -> Result<LinkClient, LinkError> {
    LinkClient::connect_with(&TcpConnector::default(), &hub.url(), &opts(None), events(r), cfg).map(|x| x.0)
}

#[test]
fn wp_l1_f6_a_failed_device_list_on_connect_leaves_no_session_and_no_event() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    for fault in [DeviceListFault::Silent, DeviceListFault::Error("internal".into())] {
        hub.fail_device_list(fault, 1);
        let ev = Arc::new(Recorder::default());
        assert!(connect_ev(&hub, &ev, &fast()).is_err());
        assert!(wait(|| hub.slots_in_use() == 0), "the slot came back at once");
        // A failed connect owes its caller only the Err
        assert!(ev.closed.lock().unwrap().is_empty(), "{:?}", ev.closed.lock().unwrap());
        assert!(ev.devices.lock().unwrap().is_empty());
    }
}

#[test]
fn wp_l1_f6_a_silent_token_request_closes_without_a_closed_event() {
    let (mock, conn) = mock_hub("pin", |_| None);
    let ev = Arc::new(Recorder::default());
    let e =
        LinkClient::connect_with(&conn, "ws://mock", &opts(Some("tok".into())), events(&ev), &fast()).err().unwrap();
    assert_eq!(e, LinkError::Timeout, "the real cause, not 'closed by the client'");
    assert!(mock.closed.load(Ordering::SeqCst));
    assert!(ev.closed.lock().unwrap().is_empty());
}

#[test]
fn wp_l1_f6_a_stale_close_is_identifiable_on_a_shared_listener() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let shared = Arc::new(Recorder::default());
    let old = connect_ev(&hub, &shared, &SessionConfig::default()).unwrap();
    let new = connect_ev(&hub, &shared, &SessionConfig::default()).unwrap();
    assert_ne!(old.epoch(), new.epoch());
    old.close();
    let got = shared.closed.lock().unwrap().clone();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, old.epoch(), "the listener can see this is the old session");
    assert!(!new.is_closed());
    assert!(shared.devices.lock().unwrap().contains(&new.epoch()));
}

#[test]
fn wp_l1_f6_the_plain_trait_methods_still_work() {
    #[derive(Default)]
    struct Plain(std::sync::Mutex<Vec<Option<u16>>>);
    impl LinkEvents for Plain {
        fn closed(&self, code: Option<u16>, _reason: &str) {
            self.0.lock().unwrap().push(code);
        }
    }
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let p = Arc::new(Plain::default());
    let ev: Arc<dyn LinkEvents> = Arc::clone(&p) as Arc<dyn LinkEvents>;
    let (_c, _) = LinkClient::connect(&TcpConnector::default(), &hub.url(), &opts(None), Some(ev)).unwrap();
    hub.close_all(4003);
    assert!(wait(|| !p.0.lock().unwrap().is_empty()));
    assert_eq!(*p.0.lock().unwrap(), [Some(4003)]);
}

/// Install the listener only after recording the epoch; a close in between must
/// not be lost, or the embedder would wait forever.
#[test]
fn wp_l1_f6_a_close_before_the_listener_is_installed_is_replayed_once() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let (c, _) = LinkClient::connect(&TcpConnector::default(), &hub.url(), &opts(None), None).unwrap();
    hub.close_all(4003);
    assert!(wait(|| c.is_closed()));
    let ev = Arc::new(Recorder::default());
    c.set_events(events(&ev));
    c.set_events(events(&ev));
    assert_eq!(*ev.closed.lock().unwrap(), [(c.epoch(), Some(4003), "hub shutting down".to_owned())]);
}

#[test]
fn wp_l1_f6_dropping_the_last_handle_ends_the_session() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    let (c, _) = LinkClient::connect(&TcpConnector::default(), &hub.url(), &opts(None), None).unwrap();
    assert_eq!(hub.slots_in_use(), 1);
    drop(c);
    assert!(wait(|| hub.slots_in_use() == 0), "the reader thread must not keep it alive");
}

#[test]
fn wp_l1_f6_a_refused_hello_exchange_closes() {
    let (hub, conn) = mock_hub("none", |_| None);
    hub.fail_sends.store(true, Ordering::SeqCst);
    let e = LinkClient::connect(&conn, "ws://mock", &opts(None), None).err().unwrap();
    assert!(matches!(e, LinkError::Io(_)));
    assert!(hub.closed.load(Ordering::SeqCst), "closed after the hello could not be sent");
}
