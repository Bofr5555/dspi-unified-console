//! WP PINTLS (owner decision 2026-11-17): a hub that was once pinned stays
//! TLS-only after Pair Again. The record [`link_credential_unpaired`] keeps
//! refuses plaintext with `tls_required_pinned` before any PIN or token goes out
//! (the fake hub counts no plaintext upgrade at all), lets a TLS pairing through
//! and re-pins; only forgetting the hub (no record) allows plaintext again, with
//! the red "(unsecured)" state.

use dspi_link::credential::{Credential, pin_display};
use dspi_link::fakehub::{Auth, FakeHub, FakeTlsKey, HubConfig};

use super::*;

const PIN: &str = "123456";

/// A PIN hub off the loopback rule (127.0.0.2), so plaintext reads as
/// unsecured, not local.
fn lan_hub() -> FakeHub {
    FakeHub::start(HubConfig {
        auth: Auth::Fixed(PIN.into()),
        bind: "127.0.0.2:0".parse().unwrap(),
        ..HubConfig::default()
    })
    .unwrap()
}

fn connect(core: &DspiCore, hub: &FakeHub, credential: &str) -> Result<LinkHello> {
    core.link_connect(
        hub.url(),
        "wp-pintls".into(),
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

/// Pinned, then the hub's key changed: the client gets `hub_key_changed` and
/// the person chooses Pair Again, which stores the unpaired record.
fn pinned_then_key_changed(hub: &FakeHub) -> String {
    let key = FakeTlsKey::generate();
    hub.set_tls(Some(&key));
    let core = DspiCore::new();
    assert!(connect(&core, hub, "").unwrap().needs_pairing);
    let credential = core.link_pair(PIN.into()).unwrap();
    core.link_close();
    assert_eq!(Credential::parse(&credential).pin, Some(key.pin()));
    hub.set_tls(Some(&FakeTlsKey::generate()));
    assert_eq!(code(connect(&core, hub, &credential)), "hub_key_changed");
    let kept = link_credential_unpaired(credential.clone());
    assert!(link_credential_tls_only(kept.clone()), "{kept}");
    assert_eq!(link_credential_hub_key(kept.clone()), "", "no key enforced");
    assert!(
        !kept.contains(&Credential::parse(&credential).token),
        "the old token is dropped"
    );
    kept
}

#[test]
fn wp_pintls_pair_again_refuses_plaintext_before_any_pin() {
    let hub = lan_hub();
    let kept = pinned_then_key_changed(&hub);
    let plain_before = hub.upgrades().0;
    // The hub now serves no TLS (or an attacker resets it), no advert
    hub.set_tls(None);
    let core = DspiCore::new();
    let err = connect(&core, &hub, &kept).unwrap_err();
    let DspiError::Link { code, msg } = &err else {
        panic!("{err:?}");
    };
    assert_eq!(code, "tls_required_pinned");
    assert!(msg.contains("forget the hub"), "{msg}");
    assert_eq!(err.notice_key(), "dlink.tls_required_pinned");
    assert_eq!(
        hub.upgrades().0,
        plain_before,
        "no plaintext session: no PIN could go out"
    );
    assert!(hub.tokens_seen().is_empty());
    assert_eq!(core.link_status().phase, LinkPhase::Idle);
    // Also with the hub's id expected and no session to pair on
    assert!(matches!(
        core.link_pair(PIN.into()),
        Err(DspiError::Disconnected)
    ));
}

#[test]
fn wp_pintls_pair_again_over_tls_pairs_and_re_pins() {
    let hub = lan_hub();
    let kept = pinned_then_key_changed(&hub);
    let new_key = FakeTlsKey::generate();
    hub.set_tls(Some(&new_key));
    let plain_before = hub.upgrades().0;
    let core = DspiCore::new();
    let hello = connect(&core, &hub, &kept).unwrap();
    assert!(hello.needs_pairing, "no token: the PIN prompt");
    let sec = core.link_security();
    assert!(sec.encrypted && sec.hub_key_new);
    assert_eq!(sec.hub_key, pin_display(&new_key.pin()));
    let credential = core.link_pair(PIN.into()).unwrap();
    let parsed = Credential::parse(&credential);
    assert_eq!(parsed.pin, Some(new_key.pin()), "pinned to the new key");
    assert!(!parsed.pinned_before && !parsed.token.is_empty());
    assert_eq!(hub.upgrades().0, plain_before, "never plaintext");
    core.link_close();
    // The new pin holds from here on
    assert!(!connect(&core, &hub, &credential).unwrap().needs_pairing);
    assert!(core.link_security().encrypted);
}

#[test]
fn wp_pintls_forget_allows_plaintext_again_unsecured() {
    let hub = lan_hub();
    let _kept = pinned_then_key_changed(&hub);
    hub.set_tls(None);
    // Forget hub: the record is removed, so the core gets no credential
    let core = DspiCore::new();
    let hello = connect(&core, &hub, "").unwrap();
    assert!(hello.needs_pairing);
    let sec = core.link_security();
    assert!(
        !sec.encrypted && !sec.local,
        "the red \"(unsecured)\" state: {sec:?}"
    );
    let token = core.link_pair(PIN.into()).unwrap();
    assert!(
        !token.is_empty() && !token.starts_with("v2."),
        "v1: {token}"
    );
    assert!(!link_credential_tls_only(token.clone()));
    assert_eq!(
        link_credential_unpaired(token),
        "",
        "never pinned: nothing to keep"
    );
}

#[test]
fn wp_pintls_unpaired_also_covers_a_pin_without_a_token_and_v1() {
    let key = FakeTlsKey::generate();
    let link_only = Credential {
        pin: Some(key.pin()),
        ..Credential::default()
    }
    .encode();
    assert!(link_credential_tls_only(link_credential_unpaired(
        link_only
    )));
    assert_eq!(link_credential_unpaired(String::new()), "");
    assert!(!link_credential_tls_only(String::new()));
}
