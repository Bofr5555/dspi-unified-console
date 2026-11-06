//! F5: the token is looked up after the hello.

use std::sync::{Arc, Mutex};

use dspi_link::fakehub::{FakeHub, HubConfig};
use dspi_link::socket::TcpConnector;
use dspi_link::{AuthState, ConnectOptions, LinkClient, SessionConfig, TokenLookup};

use crate::support::*;

const HUB_A: &str = "hub-a";

fn lookup(store: &[(&str, &str)], asked: &Arc<Mutex<Vec<String>>>) -> TokenLookup {
    let store: Vec<(String, String)> = store.iter().map(|(a, b)| ((*a).into(), (*b).into())).collect();
    let asked = Arc::clone(asked);
    Arc::new(move |hub_id: &str| {
        asked.lock().unwrap().push(hub_id.to_owned());
        store.iter().find(|(id, _)| id == hub_id).map(|(_, t)| t.clone())
    })
}

fn with_lookup(hub: &FakeHub, token_for: TokenLookup, opts: &ConnectOptions) -> (LinkClient, AuthState) {
    let cfg = SessionConfig { token_for: Some(token_for), ..SessionConfig::default() };
    LinkClient::connect_with(&TcpConnector::default(), &hub.url(), opts, None, &cfg).unwrap()
}

#[test]
fn wp_l1_f5_the_lookup_gets_the_id_from_the_hello() {
    let hub = FakeHub::start(HubConfig { hub_id: HUB_A.into(), ..HubConfig::bridge() }).unwrap();
    let (c, _) = LinkClient::connect(&TcpConnector::default(), &hub.url(), &opts(None), None).unwrap();
    let token = c.pair("482913", "control").unwrap();
    c.close();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let (_c, state) = with_lookup(&hub, lookup(&[(HUB_A, &token)], &asked), &opts(None));
    assert_eq!(state, AuthState::Ready);
    assert_eq!(*asked.lock().unwrap(), [HUB_A]);
}

/// The address now belongs to another hub: hub A's secret never goes out, nothing
/// counts against this client there, and B's own stored token is not touched.
#[test]
fn wp_l1_f5_a_token_never_goes_to_the_wrong_hub() {
    let hub_b = FakeHub::start(HubConfig { hub_id: "hub-b".into(), ..HubConfig::bridge() }).unwrap();
    let asked = Arc::new(Mutex::new(Vec::new()));
    // The caller believes this address is hub A, and offers A's token the old way too
    let (c, state) = with_lookup(&hub_b, lookup(&[(HUB_A, "secret-of-a")], &asked), &opts(Some("secret-of-a".into())));
    assert_eq!(state, AuthState::NeedsPairing { pin_source: "fixed".into() }, "B has no token here: not refused");
    assert_eq!(c.hello().hub.id, "hub-b");
    assert_eq!(*asked.lock().unwrap(), ["hub-b"]);
    assert!(hub_b.tokens_seen().is_empty(), "sent: {:?}", hub_b.tokens_seen());
}

#[test]
fn wp_l1_f5_a_hub_with_no_id_gets_no_token() {
    let (hub, conn) = mock_hub_without_id();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let cfg = SessionConfig { token_for: Some(lookup(&[("", "secret")], &asked)), ..SessionConfig::default() };
    let (_c, state) = LinkClient::connect_with(&conn, "ws://mock", &opts(None), None, &cfg).unwrap();
    assert!(matches!(state, AuthState::NeedsPairing { .. }));
    assert!(asked.lock().unwrap().is_empty());
    let sent = hub.sent.lock().unwrap();
    assert!(!sent.iter().any(|m| matches!(m, dspi_link::socket::WsMessage::Text(t) if t.contains("auth.token"))));
}

#[test]
fn wp_l1_f5_factory_reset_at_the_same_address_is_seen_after_hello() {
    let hub = FakeHub::start(HubConfig { hub_id: HUB_A.into(), ..HubConfig::bridge() }).unwrap();
    hub.factory_reset("hub-a-reborn");
    let asked = Arc::new(Mutex::new(Vec::new()));
    let (_c, state) = with_lookup(&hub, lookup(&[(HUB_A, "old")], &asked), &opts(None));
    assert!(matches!(state, AuthState::NeedsPairing { .. }));
    assert!(hub.tokens_seen().is_empty());
}
