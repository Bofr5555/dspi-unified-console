//! WP SECCORE: TLS first and pins in the core (PLAN-SEC 4.3 rules 1 to 3,
//! client side), against the fake hub serving TLS beside plaintext. "No token
//! was received" is the hub's own record (`tokens_seen`, its upgrade counts).

use std::sync::atomic::AtomicUsize;

use dspi_link::credential::{Credential, pin_display, pin_to_b64url};
use dspi_link::fakehub::{Auth, FakeHub, FakeTlsKey, HubConfig};
use serde_json::json;

use super::*;

const WAIT: Duration = Duration::from_secs(10);
const PIN: &str = "123456";

fn pin_hub() -> FakeHub {
    FakeHub::start(HubConfig {
        auth: Auth::Fixed(PIN.into()),
        ..HubConfig::default()
    })
    .unwrap()
}

fn tls_hub(key: &FakeTlsKey) -> FakeHub {
    let hub = pin_hub();
    hub.set_tls(Some(key));
    hub
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[derive(Default)]
struct Events {
    issued: Mutex<Vec<(String, String)>>,
    status: AtomicUsize,
}

impl LinkListener for Events {
    fn on_link_devices_changed(&self) {}
    fn on_link_closed(&self, _code: i32, _reason: String) {}
    fn on_link_status_changed(&self) {
        self.status.fetch_add(1, Ordering::SeqCst);
    }
    fn on_link_token_issued(&self, hub_id: String, token: String) {
        guard(&self.issued).push((hub_id, token));
    }
}

fn connect(core: &DspiCore, hub: &FakeHub, credential: &str) -> Result<LinkHello> {
    core.link_connect(
        hub.url(),
        "wp-seccore".into(),
        "test".into(),
        credential.into(),
        String::new(),
    )
}

fn code(r: Result<LinkHello>) -> String {
    match r {
        Err(DspiError::Link { code, .. }) => code,
        other => panic!("expected a Link error, got {other:?}"),
    }
}

/// Pair over whatever the hub serves now; returns the credential.
fn pair(hub: &FakeHub) -> String {
    let core = DspiCore::new();
    let hello = connect(&core, hub, "").unwrap();
    assert!(hello.needs_pairing);
    let credential = core.link_pair(PIN.into()).unwrap();
    core.link_close();
    credential
}

// ── Upstream hubs: plaintext keeps working ────────────────────────────

#[test]
fn wp_seccore_a_plaintext_hub_still_connects_unencrypted_with_v1() {
    let hub = pin_hub();
    let core = DspiCore::new();
    assert!(connect(&core, &hub, "").unwrap().needs_pairing);
    assert!(hub.tls_refused() >= 1, "TLS was tried first");
    let sec = core.link_security();
    assert!(!sec.encrypted, "unencrypted badge");
    assert_eq!(sec.hub_key, "");
    let token = core.link_pair(PIN.into()).unwrap();
    assert!(
        !token.is_empty() && !token.starts_with("v2."),
        "v1: {token}"
    );
    assert_eq!(core.link_security().credential, token);
    core.link_close();

    let hello = connect(&core, &hub, &token).unwrap();
    assert!(!hello.needs_pairing && !hello.token_refused);
    assert!(!core.link_security().encrypted);
    assert_eq!(core.link_security().credential, token);
    assert_eq!(hub.upgrades().1, 0, "nothing over TLS");
    assert_eq!(link_credential_hub_key(token), "");
}

// -- WP SECFIX --
/// `local`: not encrypted, and the hub is this machine (127.0.0.1, ::1,
/// localhost). The same hub at 127.0.0.2, a loopback-routed address outside
/// the rule, is not local (unsecured), and an encrypted session is never local.
#[test]
fn wp_secfix_a_plaintext_hub_on_loopback_is_local() {
    let hub = pin_hub();
    assert_eq!(hub.addr().ip().to_string(), "127.0.0.1");
    let core = DspiCore::new();
    assert!(!core.link_security().local, "no session: nothing");
    connect(&core, &hub, "").unwrap();
    let sec = core.link_security();
    assert!(!sec.encrypted && sec.local, "{sec:?}");
    core.link_close();
    // By name too
    let by_name = hub.url().replace("127.0.0.1", "localhost");
    core.link_connect(
        by_name,
        "wp-secfix".into(),
        "test".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    assert!(core.link_security().local);
    core.link_close();

    let off = FakeHub::start(HubConfig {
        auth: Auth::Fixed(PIN.into()),
        bind: "127.0.0.2:0".parse().unwrap(),
        ..HubConfig::default()
    })
    .unwrap();
    connect(&core, &off, "").unwrap();
    let sec = core.link_security();
    assert!(!sec.encrypted && !sec.local, "unsecured: {sec:?}");
    core.link_close();

    let key = FakeTlsKey::generate();
    let tls = tls_hub(&key);
    connect(&core, &tls, "").unwrap();
    let sec = core.link_security();
    assert!(sec.encrypted && !sec.local, "{sec:?}");
    core.link_close();
}

#[test]
fn wp_seccore_a_plaintext_hub_that_waits_on_the_hello_connects_after_the_try() {
    let hub = pin_hub();
    hub.set_slow_plaintext(true);
    let core = DspiCore::new();
    let started = Instant::now();
    assert!(connect(&core, &hub, "").unwrap().needs_pairing);
    let took = started.elapsed();
    assert!(took >= dspi_link::client::TLS_TRY_WITHIN, "{took:?}");
    assert!(took < WAIT, "{took:?}");
    assert!(!core.link_security().encrypted);
}

// ── Rule 2: first contact and v1 credentials ──────────────────────────

#[test]
fn wp_seccore_pairing_over_tls_pins_the_key() {
    let key = FakeTlsKey::generate();
    let hub = tls_hub(&key);
    let core = DspiCore::new();
    let events = Arc::new(Events::default());
    core.set_link_listener(Some(events.clone()));
    assert!(connect(&core, &hub, "").unwrap().needs_pairing);
    let sec = core.link_security();
    assert!(sec.encrypted);
    assert_eq!(sec.hub_key, pin_display(&key.pin()));
    assert!(sec.hub_key_new, "first use: show the fingerprint once");
    assert_eq!(sec.credential, "", "no token yet");

    let credential = core.link_pair(PIN.into()).unwrap();
    let parsed = Credential::parse(&credential);
    assert!(credential.starts_with("v2."));
    assert_eq!(parsed.pin, Some(key.pin()));
    assert!(!parsed.token.is_empty());
    assert_eq!(core.link_security().credential, credential);
    assert_eq!(
        link_credential_hub_key(credential.clone()),
        pin_display(&key.pin())
    );
    wait_until("the token event", || !guard(&events.issued).is_empty());
    assert_eq!(
        guard(&events.issued)[0].1,
        credential,
        "the listener stores the v2 credential"
    );
    assert_eq!(hub.upgrades(), (0, 1));
}

#[test]
fn wp_seccore_a_pinned_connect() {
    let key = FakeTlsKey::generate();
    let hub = tls_hub(&key);
    let credential = pair(&hub);
    let token = Credential::parse(&credential).token;
    let core = DspiCore::new();
    let hello = connect(&core, &hub, &credential).unwrap();
    assert!(!hello.needs_pairing);
    let sec = core.link_security();
    assert!(sec.encrypted && !sec.hub_key_new);
    assert_eq!(sec.credential, credential);
    assert_eq!(hub.tokens_seen(), vec![token]);
    assert_eq!(hub.upgrades().0, 0, "never plaintext");
}

#[test]
fn wp_seccore_a_v1_token_is_upgraded_to_v2_on_first_tls() {
    let hub = pin_hub();
    let v1 = pair(&hub);
    assert!(!v1.starts_with("v2."));
    let key = FakeTlsKey::generate();
    hub.set_tls(Some(&key));
    let core = DspiCore::new();
    assert!(!connect(&core, &hub, &v1).unwrap().needs_pairing);
    let sec = core.link_security();
    assert!(sec.encrypted && sec.hub_key_new);
    let v2 = Credential::parse(&sec.credential);
    assert_eq!(v2.token, v1);
    assert_eq!(v2.pin, Some(key.pin()));
    assert_eq!(hub.upgrades().1, 1, "the token went over TLS");
}

// ── Rule 1: a pinned hub ──────────────────────────────────────────────

#[test]
fn wp_seccore_a_changed_key_gets_no_token_and_no_plaintext_try() {
    let key = FakeTlsKey::generate();
    let hub = tls_hub(&key);
    let credential = pair(&hub);
    let plain_before = hub.upgrades().0;
    // Another key, and the advert stripped (link_connect has none)
    hub.set_tls(Some(&FakeTlsKey::generate()));
    let core = DspiCore::new();
    assert_eq!(code(connect(&core, &hub, &credential)), "hub_key_changed");
    assert!(
        hub.tokens_seen().is_empty(),
        "no token to an unverified key"
    );
    assert_eq!(hub.upgrades().0, plain_before, "no plaintext retry");
    assert_eq!(core.link_status().phase, LinkPhase::Idle);

    // No TLS at all now: still never plaintext
    hub.set_tls(None);
    assert_eq!(code(connect(&core, &hub, &credential)), "tls_refused");
    assert!(hub.tokens_seen().is_empty());
    assert_eq!(hub.upgrades().0, plain_before);
}

#[test]
fn wp_seccore_advertised_tls_refused_is_not_retried_in_plaintext() {
    let hub = pin_hub();
    let v1 = pair(&hub);
    let plain_before = hub.upgrades().0;
    let core = DspiCore::new();
    let r =
        core.link_connect_advertised(hub.url(), "t".into(), "1".into(), v1, String::new(), true);
    let Err(DspiError::Link { code, msg }) = r else {
        panic!("{r:?}")
    };
    assert_eq!(code, "tls_refused");
    assert!(msg.contains("advertises encryption"), "{msg}");
    assert!(hub.tokens_seen().is_empty());
    assert_eq!(hub.upgrades().0, plain_before, "no plaintext retry");
}

#[test]
fn wp_seccore_a_valid_key_next_re_pins_in_rotation_mode() {
    let old = FakeTlsKey::generate();
    let hub = tls_hub(&old);
    let credential = pair(&hub);
    let hid = HubConfig::default().hub_id;
    let new = FakeTlsKey::generate();
    hub.set_tls(Some(&new));
    hub.set_key_next(Some(old.key_next_to(&new, &hid)));
    let core = DspiCore::new();
    assert!(!connect(&core, &hub, &credential).unwrap().needs_pairing);
    let sec = core.link_security();
    assert!(sec.encrypted && sec.hub_key_new);
    assert_eq!(sec.hub_key, pin_display(&new.pin()));
    let moved = Credential::parse(&sec.credential);
    assert_eq!(moved.pin, Some(new.pin()));
    assert_eq!(moved.token, Credential::parse(&credential).token);
    assert_eq!(hub.tokens_seen().len(), 1);
}

#[test]
fn wp_seccore_a_key_next_seen_earlier_is_accepted_directly() {
    let old = FakeTlsKey::generate();
    let hub = tls_hub(&old);
    let credential = pair(&hub);
    let hid = HubConfig::default().hub_id;
    let new = FakeTlsKey::generate();
    // Announced while the old key still serves
    hub.set_key_next(Some(old.key_next_to(&new, &hid)));
    let core = DspiCore::new();
    connect(&core, &hub, &credential).unwrap();
    let seen = Credential::parse(&core.link_security().credential);
    assert_eq!(seen.pin, Some(old.pin()));
    assert_eq!(seen.next, Some(new.pin()), "the successor is kept");
    core.link_close();

    // Later: the new key, and no announcement any more
    hub.set_key_next(None);
    hub.set_tls(Some(&new));
    connect(&core, &hub, &seen.encode()).unwrap();
    let moved = Credential::parse(&core.link_security().credential);
    assert_eq!(moved.pin, Some(new.pin()));
    assert_eq!(moved.next, None);
}

#[test]
fn wp_seccore_a_forged_key_next_is_refused_with_no_token_sent() {
    let old = FakeTlsKey::generate();
    let hub = tls_hub(&old);
    let credential = pair(&hub);
    let hid = HubConfig::default().hub_id;
    let new = FakeTlsKey::generate();
    let other = FakeTlsKey::generate();
    hub.set_tls(Some(&new));
    let signed_by_other = other.key_next_to(&new, &hid);
    let msg = dspi_link::tls::rotation_message(&hid, &dspi_link::tls::SpkiHash(new.pin()));
    let forged_sig = json!({
        "spki": new.pin_b64url(),
        "prev_spki": base64url(old.spki_der()),
        "sig": base64url(&other.sign(&msg)),
    });
    let other_hub = old.key_next_to(&new, "another-hub");
    let core = DspiCore::new();
    for next in [signed_by_other, forged_sig, other_hub] {
        hub.set_key_next(Some(next));
        assert_eq!(code(connect(&core, &hub, &credential)), "hub_key_changed");
    }
    assert!(hub.tokens_seen().is_empty());
}

/// base64url without padding (RFC 4648 section 5).
fn base64url(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(char::from(A[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

#[test]
fn wp_seccore_expected_hub_id_is_still_checked() {
    let key = FakeTlsKey::generate();
    let hub = tls_hub(&key);
    let credential = pair(&hub);
    let core = DspiCore::new();
    let r = core.link_connect(
        hub.url(),
        "t".into(),
        "1".into(),
        credential,
        "another-hub".into(),
    );
    assert_eq!(code(r), "wrong_hub");
    assert!(hub.tokens_seen().is_empty());
}

// ── Pairing: the QR link, `ok.hub_key`, the advert's hint ─────────────

#[test]
fn wp_seccore_the_pairing_link_parses_and_a_mismatched_k_is_refused() {
    let key = FakeTlsKey::generate();
    let hub = tls_hub(&key);
    let hid = HubConfig::default().hub_id;
    let text = format!(
        "dspi://pair?h={hid}&a={}&k={}&p={PIN}",
        hub.addr(),
        key.pin_b64url()
    );
    let link = link_parse_pair_link(text).unwrap();
    assert_eq!(link.hub_id, hid);
    assert_eq!(link.url, hub.url());
    assert_eq!(link.hub_key, pin_display(&key.pin()));
    assert_eq!(link.pin, PIN);
    assert!(!format!("{link:?}").contains(PIN), "no PIN in Debug");

    let core = DspiCore::new();
    let hello = core
        .link_connect(
            link.url.clone(),
            "t".into(),
            "1".into(),
            link.credential.clone(),
            link.hub_id.clone(),
        )
        .unwrap();
    assert!(hello.needs_pairing);
    let credential = core.link_pair(link.pin.clone()).unwrap();
    assert_eq!(Credential::parse(&credential).pin, Some(key.pin()));
    core.link_close();

    // A link whose k= is not the hub's key: refused before any PIN goes out
    let wrong = FakeTlsKey::generate();
    let text = format!(
        "dspi://pair?h={hid}&a={}&k={}&p={PIN}",
        hub.addr(),
        pin_to_b64url(&wrong.pin())
    );
    let link = link_parse_pair_link(text).unwrap();
    let r = core.link_connect(
        link.url,
        "t".into(),
        "1".into(),
        link.credential,
        link.hub_id,
    );
    assert_eq!(code(r), "hub_key_unverified");
    assert!(matches!(
        link_parse_pair_link("dspi://pair?h=x".into()),
        Err(DspiError::InvalidArgument { .. })
    ));
}

#[test]
fn wp_seccore_an_ok_hub_key_that_contradicts_tls_keeps_no_token() {
    let key = FakeTlsKey::generate();
    let hub = tls_hub(&key);
    hub.set_pair_hub_key(Some(&FakeTlsKey::generate().pin_b64url()));
    let core = DspiCore::new();
    let events = Arc::new(Events::default());
    core.set_link_listener(Some(events.clone()));
    assert!(connect(&core, &hub, "").unwrap().needs_pairing);
    let Err(DspiError::Link { code, .. }) = core.link_pair(PIN.into()) else {
        panic!("the pairing must fail")
    };
    assert_eq!(code, "hub_key_unverified");
    std::thread::sleep(Duration::from_millis(100));
    assert!(guard(&events.issued).is_empty(), "no token delivered");
    assert_eq!(core.link_security().credential, "");
}

// ── The supervisor: a reconnect that moves the pin ────────────────────

#[test]
fn wp_seccore_a_rotation_on_a_reconnect_is_delivered_to_the_app() {
    let old = FakeTlsKey::generate();
    let hub = tls_hub(&old);
    let credential = pair(&hub);
    let hid = HubConfig::default().hub_id;
    let core = DspiCore::new();
    let events = Arc::new(Events::default());
    core.set_link_listener(Some(events.clone()));
    core.link_connect_hinted(
        hub.url(),
        "t".into(),
        "1".into(),
        credential,
        String::new(),
        Advert::default(),
        Tuning {
            backoff_divisor: 50,
            ..Tuning::default()
        },
    )
    .unwrap();
    let new = FakeTlsKey::generate();
    hub.set_tls(Some(&new));
    hub.set_key_next(Some(old.key_next_to(&new, &hid)));
    hub.close_all(1001);
    wait_until("the re-pinned credential", || {
        guard(&events.issued)
            .iter()
            .any(|(_, c)| Credential::parse(c).pin == Some(new.pin()))
    });
    wait_until("the session back", || core.link_status().ready);
    assert!(core.link_security().hub_key_new);
    assert_eq!(core.link_security().hub_key, pin_display(&new.pin()));
}

#[test]
fn wp_seccore_a_key_change_on_a_reconnect_ends_the_session_as_refused() {
    let key = FakeTlsKey::generate();
    let hub = tls_hub(&key);
    let credential = pair(&hub);
    let core = DspiCore::new();
    core.link_connect_hinted(
        hub.url(),
        "t".into(),
        "1".into(),
        credential,
        String::new(),
        Advert::default(),
        Tuning {
            backoff_divisor: 50,
            ..Tuning::default()
        },
    )
    .unwrap();
    let seen = hub.tokens_seen().len();
    hub.set_tls(Some(&FakeTlsKey::generate()));
    hub.close_all(1001);
    wait_until("the refusal", || {
        core.link_status().phase == LinkPhase::Refused
    });
    assert!(
        core.link_status().close_reason.contains("key changed"),
        "{}",
        core.link_status().close_reason
    );
    assert_eq!(hub.tokens_seen().len(), seen, "no token to the new key");
}

// ── WP SECBFIXA (review secb F1, N4) ─────────────────────────────────

#[test]
fn wp_secbfixa_a_damaged_v2_credential_is_never_sent_as_a_token() {
    let key = FakeTlsKey::generate();
    let hub = tls_hub(&key);
    let credential = pair(&hub);
    let pin = pin_to_b64url(&key.pin());
    let damaged = credential.replacen(&pin, &pin[..pin.len() - 1], 1);
    assert_ne!(damaged, credential);
    // The hub lost TLS (or an attacker resets it)
    hub.set_tls(None);
    let core = DspiCore::new();
    let hello = connect(&core, &hub, &damaged).unwrap();
    assert!(hello.needs_pairing, "no credential: pair again");
    assert!(hub.tokens_seen().is_empty(), "nothing sent as a token");
    assert_eq!(core.link_security().credential, "");
}

#[test]
fn wp_secbfixa_a_tls_required_close_stops_with_a_typed_reason() {
    let hub = pin_hub();
    let token = pair(&hub);
    hub.set_require_tls(true);
    let core = DspiCore::new();
    let started = Instant::now();
    assert_eq!(code(connect(&core, &hub, &token)), "tls_required");
    assert!(started.elapsed() < Duration::from_secs(5), "not retried");

    // A live plaintext session the hub closes with 4003 tls_required: no reconnect
    hub.set_require_tls(false);
    let events = Arc::new(Events::default());
    core.set_link_listener(Some(events.clone()));
    connect(&core, &hub, &token).unwrap();
    let before = hub.upgrades().0;
    hub.close_all_with(4003, "tls_required");
    wait_until("the end", || core.link_status().closed);
    let st = core.link_status();
    assert_eq!(st.close_reason, dspi_link::client::TLS_REQUIRED_REASON);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(hub.upgrades().0, before, "no reconnect");
}
