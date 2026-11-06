//! F2: only `unauthenticated` and `denied` mean the token is bad.

use std::time::Duration;

use dspi_link::fakehub::{FakeHub, HubConfig};
use dspi_link::socket::TcpConnector;
use dspi_link::{AuthState, LinkClient, LinkError, RetryAdvice, SessionState};

use crate::support::*;

fn connect(hub: &FakeHub, token: &str) -> Result<(LinkClient, AuthState), LinkError> {
    LinkClient::connect(&TcpConnector::default(), &hub.url(), &opts(Some(token.into())), None)
}

fn paired_token(hub: &FakeHub) -> String {
    let (c, _) = LinkClient::connect(&TcpConnector::default(), &hub.url(), &opts(None), None).unwrap();
    let t = c.pair("482913", "control").unwrap();
    c.close();
    t
}

#[test]
fn wp_l1_f2_refused_tokens_are_a_typed_state() {
    let hub = FakeHub::start(HubConfig::bridge()).unwrap();
    for code in ["unauthenticated", "denied"] {
        hub.fail_tokens_with(Some(code));
        let (c, state) = connect(&hub, "tok").unwrap();
        assert_eq!(state, AuthState::TokenRefused { pin_source: "fixed".into() }, "{code}");
        assert!(state.needs_pin());
        assert_eq!(c.session_state(), SessionState::NeedsAuth);
        c.pair("482913", "control").unwrap();
    }
    hub.fail_tokens_with(None);
    let (_c, state) = connect(&hub, "never issued").unwrap();
    assert!(matches!(state, AuthState::TokenRefused { .. }), "an unknown token is refused too");
    let (_c, state) = LinkClient::connect(&TcpConnector::default(), &hub.url(), &opts(None), None).unwrap();
    assert!(matches!(state, AuthState::NeedsPairing { .. }), "no token: nothing to forget");
}

#[test]
fn wp_l1_f2_rate_limited_keeps_the_token_and_says_retry() {
    let hub = FakeHub::start(HubConfig::bridge()).unwrap();
    hub.fail_tokens_with(Some("rate_limited"));
    let e = connect(&hub, "tok").err().unwrap();
    assert!(matches!(e, LinkError::RateLimited { .. }), "{e:?}");
    let advice = e.retry_advice();
    assert_eq!(advice, RetryAdvice::Retry { min_delay: Duration::from_secs(60) });
    assert_eq!(advice.delay(0), Some(Duration::from_secs(60)), "the backoff never undercuts the lockout");
    assert!(wait(|| hub.slots_in_use() == 0), "the session was closed");
}

#[test]
fn wp_l1_f2_internal_keeps_the_token_and_says_retry() {
    let hub = FakeHub::start(HubConfig::bridge()).unwrap();
    hub.fail_tokens_with(Some("internal"));
    let e = connect(&hub, "tok").err().unwrap();
    assert!(matches!(&e, LinkError::Rejected { code, msg } if code == "internal" && msg == "injected"), "{e:?}");
    assert_eq!(e.retry_advice(), RetryAdvice::Retry { min_delay: dspi_link::HUB_BUSY_BACKOFF });
    assert!(wait(|| hub.slots_in_use() == 0));
}

/// The finding's scenario: another client at this address burns the rate limit,
/// the paired client reconnects, and its token still works a minute later.
#[test]
fn wp_l1_f2_a_browsers_wrong_pins_do_not_cost_a_paired_client_its_token() {
    let clock = manual_clock();
    let hub = hub_on(&clock, HubConfig::bridge());
    let token = paired_token(&hub);
    let (browser, _) = LinkClient::connect(&TcpConnector::default(), &hub.url(), &opts(None), None).unwrap();
    for _ in 0..5 {
        assert!(browser.pair("000000", "control").is_err());
    }
    let e = connect(&hub, &token).err().unwrap();
    assert!(matches!(e, LinkError::RateLimited { .. }), "{e:?}");
    clock.advance(Duration::from_secs(61));
    let (_c, state) = connect(&hub, &token).unwrap();
    assert_eq!(state, AuthState::Ready);
}

#[test]
fn wp_l1_f2_retry_advice_backs_off() {
    let advice = RetryAdvice::Retry { min_delay: Duration::ZERO };
    let delays: Vec<u64> = (0..7).map(|a| advice.delay(a).unwrap().as_secs()).collect();
    assert_eq!(delays, [1, 2, 4, 8, 16, 30, 30]);
    assert_eq!(advice.delay(u32::MAX), Some(Duration::from_secs(30)));
    assert_eq!(RetryAdvice::Stop.delay(0), None);
    assert_eq!(LinkError::Timeout.retry_advice(), advice);
    let closed = |code| LinkError::Closed { code, reason: String::new() };
    assert_eq!(closed(Some(4002)).retry_advice(), RetryAdvice::Stop, "revoked");
    assert_eq!(closed(Some(4000)).retry_advice(), RetryAdvice::Stop, "protocol error");
    assert_eq!(closed(Some(4003)).retry_advice(), advice);
    assert_eq!(closed(None).retry_advice(), advice);
}
