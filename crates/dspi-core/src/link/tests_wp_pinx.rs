//! WP PINX (review-fu1 N1): [`link_credential_has_token`] says whether a stored
//! record is a pairing, so a damaged `v2` record or the unpaired record of a hub
//! pinned before is never taken for one.

use dspi_link::credential::{Credential, unpaired_record};

use super::*;

const PIN: [u8; 32] = [7; 32];

#[test]
fn wp_pinx_has_token_only_for_a_record_with_a_token() {
    let pinned = Credential {
        token: "tok-1".into(),
        pin: Some(PIN),
        ..Credential::default()
    }
    .encode();
    assert!(link_credential_has_token(pinned.clone()));
    assert!(link_credential_has_token("tok-12-0".into()), "a v1 token");
    let unpaired = unpaired_record(&pinned);
    assert!(!unpaired.is_empty());
    assert!(
        !link_credential_has_token(unpaired.clone()),
        "pinned before: no token"
    );
    assert!(link_credential_tls_only(unpaired));
    for none in ["", "   ", "v2.damaged", "v2.", "v2.dA.short.t"] {
        assert!(!link_credential_has_token(none.into()), "{none:?}");
    }
    // A pin with no token (a pairing link's credential) is no pairing either
    let link_only = Credential {
        pin: Some(PIN),
        ..Credential::default()
    }
    .encode();
    assert!(!link_credential_has_token(link_only.clone()));
    assert!(link_credential_tls_only(link_only));
}
