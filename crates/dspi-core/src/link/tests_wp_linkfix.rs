//! WP LINKFIX (owner: fail closed): a bare (`v1`) token is never sent without
//! an expected hub id. `link_connect` with one fails with
//! `token_without_hub_id` before anything is dialled, so no hub at the address
//! sees it; with the hub's id it connects as before, and a `v2` credential's
//! key pin still guards it without one.

use dspi_link::credential::Credential;
use dspi_link::fakehub::{Auth, FakeHub, FakeTlsKey, HubConfig};

use super::*;

const PIN: &str = "123456";

fn hub() -> FakeHub {
    FakeHub::start(HubConfig {
        auth: Auth::Fixed(PIN.into()),
        bind: "127.0.0.2:0".parse().unwrap(),
        ..HubConfig::default()
    })
    .unwrap()
}

fn connect(core: &DspiCore, hub: &FakeHub, credential: &str, id: &str) -> Result<LinkHello> {
    core.link_connect(
        hub.url(),
        "wp-linkfix".into(),
        "test".into(),
        credential.into(),
        id.into(),
    )
}

/// Pair over plaintext: the hub's id and its bare `v1` token.
fn paired_v1(hub: &FakeHub) -> (String, String) {
    let core = DspiCore::new();
    let hello = connect(&core, hub, "", "").unwrap();
    assert!(hello.needs_pairing);
    let token = core.link_pair(PIN.into()).unwrap();
    core.link_close();
    assert!(Credential::parse(&token).pin.is_none(), "a v1 token");
    (hello.hub_id, token)
}

#[test]
fn wp_linkfix_a_bare_token_without_a_hub_id_is_refused_before_any_dial() {
    let hub = hub();
    let (id, token) = paired_v1(&hub);
    let tokens = hub.tokens_seen().len();
    let dials = (hub.upgrades(), hub.tls_refused());
    let core = DspiCore::new();
    let err = connect(&core, &hub, &token, "").unwrap_err();
    let DspiError::Link { code, msg } = &err else {
        panic!("{err:?}");
    };
    assert_eq!(code, "token_without_hub_id");
    assert!(msg.contains("Pair with the hub again"), "{msg}");
    assert!(!err.notice_detail().is_empty());
    assert_eq!(hub.tokens_seen().len(), tokens, "no token went out");
    assert_eq!(
        (hub.upgrades(), hub.tls_refused()),
        dials,
        "nothing was dialled"
    );
    assert!(!core.link_status().connected);

    // Blank ids are no id either.
    let err = connect(&core, &hub, &token, "  ").unwrap_err();
    assert!(matches!(err, DspiError::Link { ref code, .. } if code == "token_without_hub_id"));

    // With the hub's id: connected as before, no PIN.
    let hello = connect(&core, &hub, &token, &id).unwrap();
    assert!(!hello.needs_pairing);
    assert_eq!(hub.tokens_seen().last(), Some(&token));
    core.link_close();

    // No token and no id: a first contact, as before.
    let hello = connect(&DspiCore::new(), &hub, "", "").unwrap();
    assert!(hello.needs_pairing);
}

#[test]
fn wp_linkfix_a_pinned_credential_without_a_hub_id_still_connects_over_tls() {
    let hub = hub();
    let key = FakeTlsKey::generate();
    hub.set_tls(Some(&key));
    let core = DspiCore::new();
    assert!(connect(&core, &hub, "", "").unwrap().needs_pairing);
    let credential = core.link_pair(PIN.into()).unwrap();
    core.link_close();
    assert_eq!(Credential::parse(&credential).pin, Some(key.pin()));
    // The key pin is checked before the token goes out.
    let hello = connect(&core, &hub, &credential, "").unwrap();
    assert!(!hello.needs_pairing);
    assert!(core.link_security().encrypted);
    core.link_close();
}
