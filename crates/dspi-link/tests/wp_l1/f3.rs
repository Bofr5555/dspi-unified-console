//! F3: one wrong-PIN error for both codes, carrying `pin_source`.

use serde_json::json;

use dspi_link::fakehub::{Auth, FakeHub, HubConfig};
use dspi_link::socket::TcpConnector;
use dspi_link::{LinkClient, LinkError, PinSource, RetryAdvice};

use crate::support::*;

fn connect(hub: &FakeHub) -> LinkClient {
    LinkClient::connect(&TcpConnector::default(), &hub.url(), &opts(None), None).unwrap().0
}

#[test]
fn wp_l1_f3_a_bridge_wrong_pin_is_typed_and_says_fixed() {
    let hub = FakeHub::start(HubConfig::bridge()).unwrap();
    let e = connect(&hub).pair("000000", "control").unwrap_err();
    assert_eq!(e, LinkError::WrongPin { pin_source: PinSource::Fixed });
    assert_eq!(e.to_string(), "wrong PIN");
    assert_eq!(e.retry_advice(), RetryAdvice::Stop, "never retried by itself: that walks into the lockout");
}

#[test]
fn wp_l1_f3_a_closed_window_is_typed_and_says_window() {
    let hub = FakeHub::start(HubConfig { auth: Auth::Window("123456".into()), ..HubConfig::default() }).unwrap();
    connect(&hub).pair("123456", "control").unwrap();
    let e = connect(&hub).pair("123456", "control").unwrap_err();
    assert_eq!(e, LinkError::WrongPin { pin_source: PinSource::Window }, "the window was used up");
    assert!(e.to_string().contains("pairing window"));
}

#[test]
fn wp_l1_f3_denied_means_the_same() {
    let (_hub, conn) = mock_hub("pin", |v| {
        (v["t"] == "auth.pair").then(|| json!({"t": "err", "id": v["id"], "code": "denied", "msg": "wrong PIN"}))
    });
    let (c, _) = LinkClient::connect(&conn, "ws://mock", &opts(None), None).unwrap();
    assert_eq!(c.pair("1", "control").unwrap_err(), LinkError::WrongPin { pin_source: PinSource::Window });
}

#[test]
fn wp_l1_f3_a_rate_limited_pair_is_not_a_wrong_pin() {
    let hub = FakeHub::start(HubConfig::bridge()).unwrap();
    let c = connect(&hub);
    for _ in 0..5 {
        assert!(matches!(c.pair("000000", "control"), Err(LinkError::WrongPin { .. })));
    }
    assert!(matches!(c.pair("482913", "control"), Err(LinkError::RateLimited { .. })));
}
