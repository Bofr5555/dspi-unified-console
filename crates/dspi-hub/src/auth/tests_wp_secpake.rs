//! WP SECPAKE: `auth.pair2`'s ledger (a failed confirmation is a wrong PIN), and
//! the SEC-C review's idle-expiry nit: a console in a session is not forgotten, and
//! its `last_seen` is refreshed when the session closes.

#![allow(clippy::unwrap_used, clippy::arithmetic_side_effects)]

use super::*;
use crate::clock::TestClock;
use crate::store::tests_wp_hb1::scratch;
use std::net::Ipv4Addr;
use std::time::UNIX_EPOCH;

const A: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10));
const DAY: u64 = 86_400;

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

#[test]
fn wp_secpake_pair2_secret_is_the_windows_pin_hash() {
    let s = auth_at(&scratch("pk-secret"), &clock());
    let now = Instant::now();
    let w = s.begin_pairing(now).unwrap();
    let secret = s.pair2_secret(A, now).unwrap();
    assert_eq!(secret.bytes(), &dspi_link::pake::pin_secret(&w.pin));
    // No window: something else, every time.
    s.cancel_pairing();
    let none = s.pair2_secret(A, now).unwrap();
    assert_ne!(none.bytes(), secret.bytes());
    assert_ne!(s.pair2_secret(A, now).unwrap().bytes(), none.bytes());
    assert!(!format!("{secret:?}").contains(&data_encoding::HEXLOWER.encode(secret.bytes())));
}

#[test]
fn wp_secpake_a_confirmed_pair2_pairs_tls_only() {
    let s = auth_at(&scratch("pk-pair"), &clock());
    let now = Instant::now();
    s.begin_pairing(now).unwrap();
    let secret = s.pair2_secret(A, now).unwrap();
    let authed = s
        .pair2_finish(&secret, true, "phone", Some(Role::Admin), A, now)
        .unwrap();
    assert!(authed.token.is_some());
    assert_eq!(authed.role, PAIR_MAX_ROLE, "a PIN gives at most control");
    assert!(s.is_tls_only(authed.cid));
    assert!(!s.pairing_open(now), "a success ends the window");
}

#[test]
fn wp_secpake_five_failed_confirmations_kill_the_window() {
    let s = auth_at(&scratch("pk-kill"), &clock());
    let now = Instant::now();
    s.begin_pairing(now).unwrap();
    for n in 0..FAILURE_LIMIT {
        let peer = IpAddr::V4(Ipv4Addr::new(10, 0, 0, n as u8 + 1));
        let secret = s.pair2_secret(peer, now).unwrap();
        assert_eq!(
            s.pair2_finish(&secret, false, "x", None, peer, now),
            Err(AuthError::Unauthenticated)
        );
    }
    assert!(!s.pairing_open(now));
    assert_eq!(s.pairing_state(now), PairingState::ClosedAfterWrongPins);
}

#[test]
fn wp_secpake_failures_shut_the_address_out_of_both_pairings() {
    let s = auth_at(&scratch("pk-addr"), &clock());
    let now = Instant::now();
    let w = s.begin_pairing(now).unwrap();
    // Four failed confirmations and one wrong legacy PIN: one ledger.
    for _ in 0..FAILURE_LIMIT - 1 {
        let secret = s.pair2_secret(A, now).unwrap();
        assert!(s.pair2_finish(&secret, false, "x", None, A, now).is_err());
    }
    assert_eq!(
        s.pair(
            if w.pin == "000000" {
                "000001"
            } else {
                "000000"
            },
            "x",
            None,
            A,
            now
        ),
        Err(AuthError::Unauthenticated)
    );
    assert_eq!(s.pair2_secret(A, now).err(), Some(AuthError::RateLimited));
    assert_eq!(
        s.pair(&w.pin, "x", None, A, now),
        Err(AuthError::RateLimited)
    );
}

#[test]
fn wp_secpake_a_secret_from_an_old_window_does_not_pair() {
    let s = auth_at(&scratch("pk-old"), &clock());
    let now = Instant::now();
    s.begin_pairing(now).unwrap();
    let old = s.pair2_secret(A, now).unwrap();
    s.begin_pairing(now).unwrap(); // a new PIN meanwhile
    assert_eq!(
        s.pair2_finish(&old, true, "x", None, A, now),
        Err(AuthError::Unauthenticated)
    );
    // A window that expired between the steps: the same.
    let s = auth_at(&scratch("pk-expired"), &clock());
    let w = s.begin_pairing(now).unwrap();
    let secret = s.pair2_secret(A, now).unwrap();
    assert_eq!(
        s.pair2_finish(&secret, true, "x", None, A, w.expires),
        Err(AuthError::Unauthenticated)
    );
}

#[test]
fn wp_secpake_a_live_session_is_not_idle_and_its_close_refreshes_last_seen() {
    let c = clock();
    let s = auth_at(&scratch("pk-live"), &c);
    let now = Instant::now();
    let w = s.begin_pairing(now).unwrap();
    let long = s
        .pair_over(&w.pin, "long session", None, A, now, true)
        .unwrap();
    s.session_opened(long.cid);
    // A session open for longer than the idle limit, and someone lists the clients.
    c.advance(Duration::from_secs(200 * DAY));
    assert_eq!(s.prune_idle(), 0, "in use: kept");
    assert!(s.clients().iter().any(|x| x.cid == long.cid));
    s.session_closed(long.cid);
    let seen = s
        .clients()
        .into_iter()
        .find(|x| x.cid == long.cid)
        .unwrap()
        .last_seen
        .unwrap();
    assert_eq!(
        seen,
        system_time(unix_seconds(c.wall())),
        "last_seen is the close"
    );
    // Idle from the close on, not from the start.
    c.advance(Duration::from_secs(179 * DAY));
    assert_eq!(s.prune_idle(), 0);
    c.advance(Duration::from_secs(DAY));
    assert_eq!(s.prune_idle(), 1);
}

#[test]
fn wp_secpake_two_sessions_of_one_client_keep_it_live_until_both_close() {
    let c = clock();
    let s = auth_at(&scratch("pk-two"), &c);
    let now = Instant::now();
    let w = s.begin_pairing(now).unwrap();
    let a = s.pair_over(&w.pin, "two", None, A, now, true).unwrap();
    s.session_opened(a.cid);
    s.session_opened(a.cid);
    s.session_closed(a.cid);
    c.advance(Duration::from_secs(181 * DAY));
    assert_eq!(s.prune_idle(), 0, "one session still open");
    s.session_closed(a.cid);
    assert_eq!(s.prune_idle(), 0, "just closed: just seen");
    // A close for a client already gone is harmless.
    assert!(s.revoke(a.cid));
    s.session_closed(a.cid);
}
