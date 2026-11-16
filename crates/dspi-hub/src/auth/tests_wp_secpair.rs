//! WP SECPAIR: separate ledgers, the `tls_only` ratchet, idle expiry, `auth.rotate`,
//! and no secret in any `Debug` (PLAN-SEC 4.3, 4.5, 4.9).

#![allow(clippy::unwrap_used, clippy::arithmetic_side_effects)]

use super::*;
use crate::clock::TestClock;
use crate::store::tests_wp_hb1::scratch;
use std::net::Ipv4Addr;
use std::time::UNIX_EPOCH;

const A: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10));
const B: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 11));
const DAY: u64 = 86_400;

/// A wall clock well after `CLOCK_FLOOR`.
fn clock() -> Arc<TestClock> {
    Arc::new(TestClock::at(
        UNIX_EPOCH + Duration::from_secs(1_800_000_000),
    ))
}

fn auth_at(dir: &std::path::Path, clock: &Arc<TestClock>) -> AuthStore {
    let s = AuthStore::new(HubStore::load(dir).unwrap(), AccessMode::Pin).unwrap();
    s.set_clock(Arc::clone(clock) as Arc<dyn HubClock>);
    s
}

fn wrong(pin: &str) -> &'static str {
    if pin == "000000" { "000001" } else { "000000" }
}

fn pair(s: &AuthStore, encrypted: bool) -> Authed {
    let now = Instant::now();
    let w = s.begin_pairing(now).unwrap();
    s.pair_over(&w.pin, "phone", None, A, now, encrypted)
        .unwrap()
}

#[test]
fn wp_secpair_revoked_token_retries_do_not_touch_pairing() {
    let s = auth_at(&scratch("sp-ledger"), &clock());
    let now = Instant::now();
    let stale = pair(&s, true);
    assert!(s.revoke(stale.cid));
    let w = s.begin_pairing(now).unwrap();
    // A stale client retries its revoked token until its address is blocked for tokens.
    for _ in 0..FAILURE_LIMIT {
        assert_eq!(
            s.verify_link_token(stale.token.as_deref().unwrap(), A, now, true),
            Err(AuthError::Unauthenticated)
        );
    }
    assert_eq!(
        s.verify_link_token(stale.token.as_deref().unwrap(), A, now, true),
        Err(AuthError::RateLimited),
        "tokens are still limited per address"
    );
    // The window is open, and the same address may still pair.
    assert!(s.pairing_open(now));
    assert_eq!(
        s.pairing_state(now),
        PairingState::Open { expires: w.expires }
    );
    assert!(s.pair_over(&w.pin, "again", None, A, now, true).is_ok());
}

#[test]
fn wp_secpair_five_wrong_pins_end_the_window_and_say_so() {
    let s = auth_at(&scratch("sp-kill"), &clock());
    let now = Instant::now();
    let w = s.begin_pairing(now).unwrap();
    assert!(!s.take_window_killed());
    for n in 0..FAILURE_LIMIT {
        let peer = IpAddr::V4(Ipv4Addr::new(10, 0, 0, n as u8 + 1));
        assert_eq!(
            s.pair_over(wrong(&w.pin), "x", None, peer, now, true),
            Err(AuthError::Unauthenticated)
        );
    }
    assert_eq!(s.pairing_state(now), PairingState::ClosedAfterWrongPins);
    assert!(s.take_window_killed(), "told once");
    assert!(!s.take_window_killed());
    assert_eq!(s.pairing_state(now), PairingState::Closed);
    // The other way round: wrong PINs still shut their address out of tokens (spec 6.2).
    let w = s.begin_pairing(now).unwrap();
    for _ in 0..FAILURE_LIMIT {
        let _ = s.pair_over(wrong(&w.pin), "x", None, A, now, true);
    }
    assert_eq!(
        s.verify_link_token("anything", A, now, true),
        Err(AuthError::RateLimited)
    );
    let _ = s.take_window_killed();
    // A new window is a clean one.
    let w = s.begin_pairing(now).unwrap();
    assert!(s.pair_over(&w.pin, "x", None, B, now, true).is_ok());
    // An expired window is closed, not "after wrong PINs".
    s.begin_pairing(now).unwrap();
    assert_eq!(s.pairing_state(now + PAIRING_WINDOW), PairingState::Closed);
}

#[test]
fn wp_secpair_a_token_used_over_tls_is_refused_over_plaintext() {
    let dir = scratch("sp-ratchet");
    let c = clock();
    let s = auth_at(&dir, &c);
    let now = Instant::now();
    // Paired over plaintext (an `off` hub, or compat): usable both ways until TLS.
    let a = pair(&s, false);
    let token = a.token.clone().unwrap();
    assert!(!s.is_tls_only(a.cid));
    s.verify_link_token(&token, A, now, false).unwrap();
    s.verify_link_token(&token, A, now, true).unwrap();
    assert!(s.is_tls_only(a.cid), "marked by its first TLS use");
    assert_eq!(
        s.verify_link_token(&token, A, now, false),
        Err(AuthError::TlsRequired)
    );
    // Not a failure: no block builds up from it.
    for _ in 0..2 * FAILURE_LIMIT {
        let _ = s.verify_link_token(&token, A, now, false);
    }
    s.verify_link_token(&token, A, now, true).unwrap();
    // It is stored.
    drop(s);
    let s = auth_at(&dir, &c);
    assert_eq!(
        s.verify_link_token(&token, A, now, false),
        Err(AuthError::TlsRequired)
    );
    // Issued over TLS: tls_only from the start.
    let b = pair(&s, true);
    assert!(s.is_tls_only(b.cid));
    assert_eq!(
        s.verify_link_token(b.token.as_deref().unwrap(), A, now, false),
        Err(AuthError::TlsRequired)
    );
}

#[test]
fn wp_secpair_api_keys_are_not_ratcheted() {
    let s = auth_at(&scratch("sp-api"), &clock());
    let now = Instant::now();
    let key = s.mint_api_key("ha", Role::Viewer, true).unwrap();
    s.verify_link_token(&key.key, A, now, true).unwrap();
    assert!(!s.is_tls_only(key.cid));
    // The REST check has no ratchet either.
    s.verify_token(&key.key, A, now).unwrap();
}

#[test]
fn wp_secpair_an_idle_token_is_pruned_at_180_days() {
    let dir = scratch("sp-idle");
    let c = clock();
    let s = auth_at(&dir, &c);
    let now = Instant::now();
    let old = pair(&s, true);
    let used = pair(&s, true);
    let key = s.mint_api_key("integration", Role::Viewer, false).unwrap();
    assert_eq!(s.token_idle(), Some(DEFAULT_TOKEN_IDLE));
    assert_eq!(DEFAULT_TOKEN_IDLE, Duration::from_secs(180 * DAY));
    c.advance(Duration::from_secs(100 * DAY));
    s.verify_link_token(used.token.as_deref().unwrap(), A, now, true)
        .unwrap();
    c.advance(Duration::from_secs(80 * DAY) - Duration::from_secs(1));
    assert_eq!(s.clients().len(), 3, "a second before 180 days: kept");
    c.advance(Duration::from_secs(1));
    let left: Vec<u32> = s.clients().iter().map(|c| c.cid).collect();
    assert_eq!(
        left,
        vec![used.cid, key.cid],
        "the idle console went; the API key stays"
    );
    assert_eq!(
        s.verify_link_token(old.token.as_deref().unwrap(), A, now, true),
        Err(AuthError::Unauthenticated)
    );
    // Gone from the file too.
    drop(s);
    let s = auth_at(&dir, &c);
    assert!(s.clients().iter().all(|x| x.cid != old.cid));
    // The used one goes 180 days after its last use.
    c.advance(Duration::from_secs(100 * DAY));
    assert_eq!(s.prune_idle(), 1);
    assert_eq!(s.clients().len(), 1);
}

#[test]
fn wp_secpair_idle_expiry_is_adjustable_and_needs_a_plausible_clock() {
    let c = clock();
    let s = auth_at(&scratch("sp-idle-adj"), &c);
    let a = pair(&s, true);
    s.set_token_idle(None);
    c.advance(Duration::from_secs(1000 * DAY));
    assert_eq!(s.prune_idle(), 0, "never");
    s.set_token_idle(Some(Duration::ZERO));
    assert_eq!(s.token_idle(), None, "0 is never too");
    // A clock gone back to 1970 (no battery) forgets nobody.
    s.set_token_idle(Some(Duration::from_secs(DAY)));
    assert_eq!(s.clients().len(), 0, "applied at once: idle for 1000 days");
    let c2 = Arc::new(TestClock::at(UNIX_EPOCH + Duration::from_secs(1000)));
    let s = auth_at(&scratch("sp-idle-1970"), &c2);
    s.set_token_idle(Some(Duration::from_secs(1)));
    let b = pair(&s, true);
    c2.advance(Duration::from_secs(10 * DAY));
    assert_eq!(s.prune_idle(), 0);
    // When the clock is set right, a last_seen from 1970 does not count against it.
    c2.set_wall(UNIX_EPOCH + Duration::from_secs(1_800_000_000));
    assert_eq!(s.prune_idle(), 0);
    assert!(s.role_of(b.cid).is_some());
    let _ = a;
}

#[test]
fn wp_secpair_rotate_swaps_the_hash_and_kills_the_old_token() {
    let dir = scratch("sp-rotate");
    let c = clock();
    let s = auth_at(&dir, &c);
    let now = Instant::now();
    let a = pair(&s, false);
    let old = a.token.clone().unwrap();
    let new = s.rotate_token(a.cid, true).unwrap();
    assert_eq!(new.cid, a.cid);
    assert_ne!(new.token, old);
    assert_eq!(new.token.len(), 43);
    assert_eq!(
        s.verify_link_token(&old, A, now, true),
        Err(AuthError::Unauthenticated),
        "the old token is dead"
    );
    let ok = s.verify_link_token(&new.token, B, now, true).unwrap();
    assert_eq!((ok.cid, ok.role), (a.cid, a.role));
    assert!(s.is_tls_only(a.cid), "rotated over TLS");
    // Stored, and only as a hash.
    let text = std::fs::read_to_string(dir.join("hub-store.json")).unwrap();
    assert!(!text.contains(&new.token));
    drop(s);
    let s = auth_at(&dir, &c);
    s.verify_link_token(&new.token, A, now, true).unwrap();
    assert_eq!(s.clients().len(), 1, "the same client, not a new one");
    // Nobody to rotate, or an API key: refused.
    assert_eq!(s.rotate_token(999, true), Err(AuthError::Unauthenticated));
    let key = s.mint_api_key("k", Role::Viewer, false).unwrap();
    assert_eq!(
        s.rotate_token(key.cid, true),
        Err(AuthError::Unauthenticated)
    );
}

#[cfg(unix)]
#[test]
fn wp_secpair_a_rotation_that_cannot_be_stored_changes_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("sp-rotate-ro");
    let s = auth_at(&dir, &clock());
    let now = Instant::now();
    let a = pair(&s, true);
    let old = a.token.clone().unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let rotated = s.rotate_token(a.cid, true);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    if rotated.is_ok() {
        // Running as root: the folder is writable anyway; nothing to show.
        return;
    }
    assert_eq!(rotated, Err(AuthError::Internal));
    s.verify_link_token(&old, A, now, true).unwrap();
}

#[test]
fn wp_secpair_no_secret_in_any_debug_string() {
    let s = auth_at(&scratch("sp-debug"), &clock());
    let now = Instant::now();
    let w = s.begin_pairing(now).unwrap();
    let pin_hash = data_encoding::HEXLOWER.encode(&sha256(w.pin.as_bytes()));
    let a = s.pair_over(&w.pin, "x", None, A, now, true).unwrap();
    let token = a.token.clone().unwrap();
    let token_hash = sha256(token.as_bytes());
    let w2 = s.begin_pairing(now).unwrap();
    let rotated = s.rotate_token(a.cid, true).unwrap();
    let rotated_hash = sha256(rotated.token.as_bytes());
    let texts = [
        format!("{s:?}"),
        format!("{:?}", s.lock().store),
        format!("{w2:?}"),
        format!("{a:?}"),
        format!("{rotated:?}"),
        format!("{:?}", s.clients()),
    ];
    for text in &texts {
        for secret in [&w.pin, &w2.pin, &token, &rotated.token, &pin_hash] {
            assert!(!text.contains(secret.as_str()), "{text}");
        }
        for hash in [token_hash, rotated_hash, sha256(w2.pin.as_bytes())] {
            assert!(
                !text.contains(&data_encoding::HEXLOWER.encode(&hash)),
                "{text}"
            );
            assert!(!text.contains(&format!("{:?}", hash)), "{text}");
            let head: Vec<String> = hash[0..8].iter().map(|b| b.to_string()).collect();
            assert!(!text.contains(&head.join(", ")), "{text}");
        }
    }
}
