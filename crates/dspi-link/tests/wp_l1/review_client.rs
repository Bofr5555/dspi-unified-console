//! Review 3-B: the client's settled pair ids, redaction and sanitising.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use dspi_link::messages::{AuthOk, Inbound};
use dspi_link::socket::WsMessage;
use dspi_link::{
    sanitize_hub_text, ConnectOptions, LinkClient, LinkError, LinkEvents, SessionConfig, SessionState, Timing,
};

use crate::support::*;

#[test]
fn wp_l1_a_late_pair_refusal_settles_and_a_late_ok_for_it_does_nothing() {
    use dspi_link::socket::WsRecv;
    let (mock, conn) = mock_hub("pin", |_| None);
    let cfg = SessionConfig {
        timing: Timing { request_timeout: Duration::from_millis(20), ..Timing::default() },
        ..SessionConfig::default()
    };
    let ev = Arc::new(Recorder::default());
    let events = Arc::clone(&ev) as Arc<dyn LinkEvents>;
    let (c, _) = LinkClient::connect_with(&conn, "ws://mock", &opts(None), Some(events), &cfg).unwrap();
    assert_eq!(c.pair("1", "control").unwrap_err(), LinkError::Timeout);
    let id = mock
        .sent
        .lock()
        .unwrap()
        .iter()
        .find_map(|m| match m {
            WsMessage::Text(t) if t.contains("auth.pair") => {
                serde_json::from_str::<serde_json::Value>(t).unwrap()["id"].as_u64()
            }
            _ => None,
        })
        .unwrap();
    mock.push(WsRecv::Message(WsMessage::Text(json!({"t": "err", "id": id, "code": "unauthenticated"}).to_string())));
    mock.settle();
    // The id is gone: a forged or duplicate ok for it cannot authenticate the session
    let ok = json!({"t": "ok", "id": id, "session": 3, "role": "admin", "token": "x"});
    mock.push(WsRecv::Message(WsMessage::Text(ok.to_string())));
    mock.settle();
    assert_eq!(c.session_state(), SessionState::NeedsAuth);
    assert!(ev.tokens.lock().unwrap().is_empty());
}

#[test]
fn wp_l1_tokens_and_pins_stay_out_of_debug() {
    let shown = format!("{:?}", ConnectOptions { token: Some("s3cret".into()), ..opts(None) });
    assert!(!shown.contains("s3cret") && shown.contains("Pixel 9"), "{shown}");

    let ok = AuthOk { token: Some("s3cret".into()), role: "admin".into(), ..AuthOk::default() };
    assert!(!format!("{ok:?}").contains("s3cret"));

    let inbound = Inbound::parse(r#"{"t":"ok","id":1,"token":"s3cret"}"#).unwrap();
    assert!(!format!("{inbound:?}").contains("s3cret"));

    let pair = WsMessage::Text(r#"{"t":"auth.pair","pin":"123456"}"#.into());
    assert!(!format!("{pair:?}").contains("123456"));
}

#[test]
fn wp_l1_hub_text_is_sanitised_for_display() {
    let evil = "bad\u{1b}[2Jname\u{9b}31m\nline\u{7f}\u{0}é";
    let clean = "bad\u{fffd}[2Jname\u{fffd}31m\u{fffd}line\u{fffd}\u{fffd}é";
    assert_eq!(sanitize_hub_text(evil), clean);
    let rejected = LinkError::Rejected { code: evil.into(), msg: evil.into() };
    assert_eq!(rejected.to_string(), format!("hub refused: {clean} {clean}"));
    let refused = LinkError::HandshakeRefused { status: 403, msg: evil.into() };
    assert_eq!(refused.to_string(), format!("the hub refused the connection (HTTP 403): {clean}"));
    assert_eq!(LinkError::Protocol(evil.into()).to_string(), format!("protocol error: {clean}"));
    assert_eq!(LinkError::Closed { code: None, reason: evil.into() }.to_string(), clean);
    // The field keeps what the hub sent
    assert!(matches!(rejected, LinkError::Rejected { msg, .. } if msg == evil));
}
