//! F8: a typed handshake status.

use std::time::Duration;

use dspi_link::fakehub::{FakeHub, HubConfig};
use dspi_link::socket::TcpConnector;
use dspi_link::{LinkClient, LinkError, RetryAdvice};

use crate::support::*;

fn connect(hub: &FakeHub) -> Result<(LinkClient, dspi_link::AuthState), LinkError> {
    LinkClient::connect(&TcpConnector::default(), &hub.url(), &opts(None), None)
}

#[test]
fn wp_l1_f8_a_full_hub_is_busy_and_clears_when_the_slot_does() {
    let clock = manual_clock();
    let hub = hub_on(&clock, HubConfig { max_sessions: Some(1), ..HubConfig::default() });
    let mut vanished = raw_socket(&hub);
    raw_hello(&mut vanished);
    let e = connect(&hub).err().unwrap();
    assert_eq!(e, LinkError::HubBusy);
    assert_eq!(e.to_string(), "the hub is busy: all its sessions are in use");
    assert_eq!(e.retry_advice(), RetryAdvice::Retry { min_delay: dspi_link::HUB_BUSY_BACKOFF });
    clock.advance(Duration::from_secs(31));
    assert!(wait(|| hub.slots_in_use() == 0));
    assert!(connect(&hub).is_ok(), "the same retry works once the hub let the slot go");
}

#[test]
fn wp_l1_f8_400_and_403_stop_retrying() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    for status in [400u16, 403] {
        hub.refuse_handshake(Some(status));
        let e = connect(&hub).err().unwrap();
        assert!(matches!(&e, LinkError::HandshakeRefused { status: s, .. } if *s == status), "{e:?}");
        assert_eq!(e.retry_advice(), RetryAdvice::Stop);
    }
}

#[test]
fn wp_l1_f8_other_server_errors_retry() {
    let hub = FakeHub::start(HubConfig::default()).unwrap();
    hub.refuse_handshake(Some(500));
    let e = connect(&hub).err().unwrap();
    assert!(matches!(e, LinkError::HandshakeRefused { status: 500, .. }), "{e:?}");
    assert!(matches!(e.retry_advice(), RetryAdvice::Retry { .. }));
    hub.refuse_handshake(None);
    assert!(connect(&hub).is_ok());
}

#[test]
fn wp_l1_f8_foreign_connectors_map_status_the_same() {
    assert_eq!(LinkError::from_handshake_status(503, "x"), LinkError::HubBusy);
    assert!(matches!(LinkError::from_handshake_status(403, "x"), LinkError::HandshakeRefused { status: 403, .. }));
}
