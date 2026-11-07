//! Access: PIN pairing, tokens, roles, rate limits (spec 6). Owned by WP HB1.
//!
//! **PIN pairing is the default and open access is opt-in**, by owner's decision and
//! against the spec's "hubs default to `none`" (6.1); HUB.md section 8 records it.
//! A hub stores only hashes of tokens (spec 6.2). The clock is passed in.
//!
//! **No fixed PIN.** The spec's `fixed` source is for hubs with no screen; every hub
//! here has a UI or a CLI to show a window PIN (HUB.md section 9), so a PIN exists only
//! for a window, and five failures from anywhere end it.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::hint::black_box;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use data_encoding::BASE64URL_NOPAD;
use sha2::{Digest, Sha256};

use crate::error::HubError;
use crate::session::Role;
use crate::store::{HubStore, StoredClient, TokenHash, random_bytes, system_time, unix_seconds};

/// Failures from one address, and failures of one PIN, before the door shuts (spec 6.2).
const FAILURE_LIMIT: usize = 5;
/// The window those failures are counted over, and how long the door stays shut.
const RATE_WINDOW: Duration = Duration::from_secs(60);
/// How long a PIN works (spec 6.2).
const PAIRING_WINDOW: Duration = Duration::from_secs(120);
/// Addresses the failure ledger follows. Past this the stalest goes, so nobody can
/// grow it by arriving from new addresses.
const MAX_TRACKED_PEERS: usize = 1024;
/// Paired clients kept. They are stored on disk and listed in UIs on a client's
/// say-so. A full table refuses a new client and evicts nobody: the user revokes.
pub const MAX_PAIRED_CLIENTS: usize = 64;
/// Longest client name kept, in characters.
pub const MAX_CLIENT_NAME: usize = 64;
/// What a client with no usable name is listed as.
const UNNAMED_CLIENT: &str = "Unnamed client";
/// How stale a stored `last_seen` may get, so a client reconnecting in a loop does not
/// rewrite the file each time.
const LAST_SEEN_RESOLUTION: u64 = 60;

/// How a hub admits a client (spec 6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AccessMode {
    /// A new client pairs with a PIN and then presents a token. The default.
    #[default]
    Pin,
    /// Every connection is an `admin` session. Only when the user asks.
    Open,
}

impl AccessMode {
    /// The word for it in `hello.auth` and `/dspi/v1/info` (spec 6.1).
    pub fn wire_name(self) -> &'static str {
        match self {
            AccessMode::Pin => "pin",
            AccessMode::Open => "none",
        }
    }
}

/// One paired client, as `auth.list` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedClient {
    /// Its id (`cid`).
    pub cid: u32,
    /// The name it gave.
    pub name: String,
    /// Its role.
    pub role: Role,
    /// When it paired.
    pub created: SystemTime,
    /// When it last authenticated.
    pub last_seen: Option<SystemTime>,
}

/// An authentication that succeeded.
#[derive(Clone, PartialEq, Eq)]
pub struct Authed {
    /// The client.
    pub cid: u32,
    /// The role it gets.
    pub role: Role,
    /// The token to hand over, on a pairing only. Never logged.
    pub token: Option<String>,
}

/// By hand, so a token never reaches a log.
impl fmt::Debug for Authed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Authed")
            .field("cid", &self.cid)
            .field("role", &self.role)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Why an authentication failed: the spec 7.1 code a client gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    /// A wrong PIN or an unknown token (`unauthenticated`, as real hubs answer).
    #[error("unauthenticated")]
    Unauthenticated,
    /// Too many failures from this address (`rate_limited`, 60 s).
    #[error("rate_limited")]
    RateLimited,
    /// A right PIN, with [`MAX_PAIRED_CLIENTS`] already paired (`busy`). The window
    /// stays open, so the user can revoke someone and the client try again.
    #[error("busy")]
    TableFull,
    /// A right PIN, and no token could be made or stored (`internal`). The window stays
    /// open. A token that would not survive a restart is not handed out.
    #[error("internal")]
    Internal,
}

/// A generated PIN and its window (spec 6.2: 6 digits, 2 minutes).
#[derive(Clone, PartialEq, Eq)]
pub struct PairingWindow {
    /// The PIN to display.
    pub pin: String,
    /// When it stops working.
    pub expires: Instant,
}

/// By hand: only the screen it is meant for may show a PIN.
impl fmt::Debug for PairingWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairingWindow")
            .field("pin", &"<redacted>")
            .field("expires", &self.expires)
            .finish()
    }
}

#[derive(Debug)]
struct OpenWindow {
    pin_hash: TokenHash,
    expires: Instant,
    failures: usize,
}

#[derive(Debug, Default)]
struct PeerRecord {
    failures: VecDeque<Instant>,
    blocked_until: Option<Instant>,
}

impl PeerRecord {
    fn last_activity(&self) -> Option<Instant> {
        self.failures.back().copied().or(self.blocked_until)
    }
}

#[derive(Debug)]
struct Inner {
    store: HubStore,
    window: Option<OpenWindow>,
    peers: BTreeMap<IpAddr, PeerRecord>,
}

/// The hub's clients and pairing state. Lock level L1b: never held across I/O other
/// than the store's atomic save, and never across a call into the hub.
#[derive(Debug)]
pub struct AuthStore {
    mode: AccessMode,
    inner: Mutex<Inner>,
}

/// The unit a limit is counted per. An IPv4 address, or an IPv6 /64: one machine on a
/// LAN can have as many addresses in its /64 as it likes, but not many /64s.
pub(crate) fn peer_key(peer: IpAddr) -> IpAddr {
    match peer.to_canonical() {
        IpAddr::V6(v6) => {
            let mask = !0u128 << 64;
            IpAddr::V6(Ipv6Addr::from(u128::from(v6) & mask))
        }
        v4 => v4,
    }
}

fn sha256(bytes: &[u8]) -> TokenHash {
    Sha256::digest(bytes).into()
}

/// Every byte is looked at, whatever the first difference. The inputs are hashes, so
/// a caller could learn little from an early exit, but this leaves nothing to argue.
fn hashes_equal(a: &TokenHash, b: &TokenHash) -> bool {
    let difference = a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y));
    black_box(difference) == 0
}

/// Six digits, every PIN equally likely.
fn new_pin() -> Result<String, HubError> {
    // The largest multiple of a million in a u32. Below it `% 1_000_000` is uniform.
    const LIMIT: u32 = u32::MAX - (u32::MAX % 1_000_000);
    loop {
        let value = u32::from_le_bytes(random_bytes()?);
        if value < LIMIT {
            return Ok(format!("{:06}", value.checked_rem(1_000_000).unwrap_or(0)));
        }
    }
}

/// What may be stored and listed of a name a client gave: no control or bidi-override
/// characters, a bounded length, and never empty.
pub(crate) fn clean_name(name: &str) -> String {
    let invisible = |c: &char| {
        c.is_control()
            || matches!(*c,
                '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' |
                '\u{2066}'..='\u{2069}' | '\u{FEFF}')
    };
    let kept: String = name
        .chars()
        .filter(|c| !invisible(c))
        .take(MAX_CLIENT_NAME)
        .collect();
    let kept = kept.trim();
    if kept.is_empty() {
        UNNAMED_CLIENT.to_owned()
    } else {
        kept.to_owned()
    }
}

fn after(now: Instant, wait: Duration) -> Instant {
    now.checked_add(wait).unwrap_or(now)
}

impl Inner {
    fn is_blocked(&mut self, peer: IpAddr, now: Instant) -> bool {
        let key = peer_key(peer);
        let Some(record) = self.peers.get(&key) else {
            return false;
        };
        match record.blocked_until {
            Some(until) if now < until => true,
            // The block has lapsed: a clean slate, with what caused it forgotten.
            Some(_) => {
                self.peers.remove(&key);
                false
            }
            None => false,
        }
    }

    fn note_failure(&mut self, peer: IpAddr, now: Instant) {
        let key = peer_key(peer);
        if !self.peers.contains_key(&key) && self.peers.len() >= MAX_TRACKED_PEERS {
            self.evict_stalest();
        }
        let record = self.peers.entry(key).or_default();
        while record
            .failures
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= RATE_WINDOW)
        {
            record.failures.pop_front();
        }
        record.failures.push_back(now);
        // The fifth attempt still gets its own error; the sixth is refused unheard.
        if record.failures.len() >= FAILURE_LIMIT {
            record.blocked_until = Some(after(now, RATE_WINDOW));
            record.failures.clear();
        }
    }

    fn evict_stalest(&mut self) {
        let stalest = self
            .peers
            .iter()
            .min_by_key(|(_, r)| r.last_activity())
            .map(|(k, _)| *k);
        if let Some(key) = stalest {
            self.peers.remove(&key);
        }
    }

    fn save(&self) -> Result<(), HubError> {
        self.store.save()
    }

    fn client_mut(&mut self, cid: u32) -> Option<&mut StoredClient> {
        self.store.clients_mut().iter_mut().find(|c| c.cid == cid)
    }
}

impl AuthStore {
    /// Over this store, in this mode. Switching the mode revokes every token (spec 6.2):
    /// a mode different from the store's last one forgets the paired clients.
    pub fn new(mut store: HubStore, mode: AccessMode) -> Result<Self, HubError> {
        let word = mode.wire_name();
        if store.access() != Some(word) {
            if store.access().is_some() && !store.clients().is_empty() {
                log::info!("hub: the access mode changed, every token is revoked");
                store.clients_mut().clear();
            }
            store.set_access(word);
            store.save()?;
        }
        Ok(Self {
            mode,
            inner: Mutex::new(Inner {
                store,
                window: None,
                peers: BTreeMap::new(),
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // Each update is complete before anything that could panic.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The current mode.
    pub fn mode(&self) -> AccessMode {
        self.mode
    }

    /// Open a pairing window, replacing an open one. [`HubError::Io`] if the system
    /// has no random numbers, since a guessable PIN is worse than none.
    pub fn begin_pairing(&self, now: Instant) -> Result<PairingWindow, HubError> {
        let pin = new_pin()?;
        let expires = after(now, PAIRING_WINDOW);
        self.lock().window = Some(OpenWindow {
            pin_hash: sha256(pin.as_bytes()),
            expires,
            failures: 0,
        });
        Ok(PairingWindow { pin, expires })
    }

    /// Close the pairing window.
    pub fn cancel_pairing(&self) {
        self.lock().window = None;
    }

    /// Whether a PIN would be accepted now.
    pub fn pairing_open(&self, now: Instant) -> bool {
        self.lock().window.as_ref().is_some_and(|w| now < w.expires)
    }

    /// `auth.pair`: a right PIN gives a token. Rate limited per address.
    ///
    /// A wrong PIN counts against the address and against the PIN itself, which dies
    /// after five failures from anywhere. A right one pairs one client and ends the
    /// window. With no window open every PIN is wrong. `name` is cleaned and capped.
    /// No `requested` role means `control` (the Swift hub's default).
    pub fn pair(
        &self,
        pin: &str,
        name: &str,
        requested: Option<Role>,
        peer: IpAddr,
        now: Instant,
    ) -> Result<Authed, AuthError> {
        let mut s = self.lock();
        if s.is_blocked(peer, now) {
            return Err(AuthError::RateLimited);
        }
        let presented = sha256(pin.as_bytes());
        let right = match &s.window {
            Some(w) if now < w.expires => hashes_equal(&w.pin_hash, &presented),
            _ => {
                s.window = None;
                false
            }
        };
        if !right {
            s.note_failure(peer, now);
            if let Some(w) = s.window.as_mut() {
                w.failures = w.failures.saturating_add(1);
                if w.failures >= FAILURE_LIMIT {
                    s.window = None;
                }
            }
            return Err(AuthError::Unauthenticated);
        }
        if s.store.clients().len() >= MAX_PAIRED_CLIENTS {
            return Err(AuthError::TableFull);
        }
        let raw: [u8; 32] = random_bytes().map_err(|e| {
            log::error!("hub: no token made: {e}");
            AuthError::Internal
        })?;
        let token = BASE64URL_NOPAD.encode(&raw);
        let role = requested.unwrap_or(Role::Control);
        let seconds = unix_seconds(SystemTime::now());
        let cid = s.store.take_cid();
        s.store.clients_mut().push(StoredClient {
            cid,
            name: clean_name(name),
            role,
            created: seconds,
            last_seen: Some(seconds),
            token_hash: sha256(token.as_bytes()),
        });
        if let Err(e) = s.save() {
            log::error!("hub: pairing not stored: {e}");
            s.store.clients_mut().retain(|c| c.cid != cid);
            return Err(AuthError::Internal);
        }
        s.window = None;
        s.peers.remove(&peer_key(peer));
        Ok(Authed {
            cid,
            role,
            token: Some(token),
        })
    }

    /// `auth.token`. Rate limited per address. Every stored hash is compared, in
    /// constant time, whichever matches.
    pub fn verify_token(
        &self,
        token: &str,
        peer: IpAddr,
        now: Instant,
    ) -> Result<Authed, AuthError> {
        let mut s = self.lock();
        if s.is_blocked(peer, now) {
            return Err(AuthError::RateLimited);
        }
        let presented = sha256(token.as_bytes());
        let found = s.store.clients().iter().fold(None, |found, c| {
            let same = hashes_equal(&c.token_hash, &presented);
            if same { Some(c.cid) } else { found }
        });
        let Some(cid) = found else {
            s.note_failure(peer, now);
            return Err(AuthError::Unauthenticated);
        };
        s.peers.remove(&peer_key(peer));
        let seconds = unix_seconds(SystemTime::now());
        let Some(client) = s.client_mut(cid) else {
            return Err(AuthError::Unauthenticated);
        };
        let role = client.role;
        let stale = client
            .last_seen
            .is_none_or(|seen| seconds.saturating_sub(seen) >= LAST_SEEN_RESOLUTION);
        if stale {
            client.last_seen = Some(seconds);
            if let Err(e) = s.save() {
                log::warn!("hub: last-seen time not stored: {e}");
            }
        }
        Ok(Authed {
            cid,
            role,
            token: None,
        })
    }

    /// `auth.list`.
    pub fn clients(&self) -> Vec<PairedClient> {
        self.lock()
            .store
            .clients()
            .iter()
            .map(|c| PairedClient {
                cid: c.cid,
                name: c.name.clone(),
                role: c.role,
                created: system_time(c.created),
                last_seen: c.last_seen.map(system_time),
            })
            .collect()
    }

    /// A paired client's role now; `None` once it is revoked. A connection asks after
    /// listing its session, which closes the race with [`Self::revoke`].
    pub fn role_of(&self, cid: u32) -> Option<Role> {
        self.lock()
            .store
            .clients()
            .iter()
            .find(|c| c.cid == cid)
            .map(|c| c.role)
    }

    /// `auth.revoke`. The caller closes the client's sessions with 4002. Its token then
    /// reads as unknown, since no trace of the hash is kept. In memory at once, even if
    /// the save fails, which is logged.
    pub fn revoke(&self, cid: u32) -> bool {
        let mut s = self.lock();
        let before = s.store.clients().len();
        s.store.clients_mut().retain(|c| c.cid != cid);
        if s.store.clients().len() == before {
            return false;
        }
        if let Err(e) = s.save() {
            log::error!("hub: revocation of client {cid} not stored: {e}");
        }
        true
    }

    /// `auth.set_role`. The caller applies it to the client's live sessions.
    pub fn set_role(&self, cid: u32, role: Role) -> bool {
        let mut s = self.lock();
        let Some(client) = s.client_mut(cid) else {
            return false;
        };
        client.role = role;
        if let Err(e) = s.save() {
            log::error!("hub: role of client {cid} not stored: {e}");
        }
        true
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_hb1 {
    use super::*;
    use crate::store::tests_wp_hb1::scratch;
    use std::fs;
    use std::net::Ipv4Addr;
    use std::path::Path;

    const A: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10));
    const B: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 11));

    fn store_in(dir: &Path) -> AuthStore {
        AuthStore::new(HubStore::load(dir).unwrap(), AccessMode::Pin).unwrap()
    }

    fn auth() -> AuthStore {
        store_in(&scratch("auth"))
    }

    fn wrong_pin(right: &str) -> String {
        if right == "000000" {
            "000001"
        } else {
            "000000"
        }
        .to_owned()
    }

    fn paired(s: &AuthStore, now: Instant) -> Authed {
        let w = s.begin_pairing(now).unwrap();
        s.pair(&w.pin, "phone", Some(Role::Control), A, now)
            .unwrap()
    }

    #[test]
    fn wp_hb1_a_pin_is_six_digits() {
        let s = auth();
        for _ in 0..50 {
            let pin = s.begin_pairing(Instant::now()).unwrap().pin;
            assert_eq!(pin.len(), 6);
            assert!(pin.bytes().all(|b| b.is_ascii_digit()));
        }
    }

    #[test]
    fn wp_hb1_a_right_pin_pairs_one_client_and_ends_the_window() {
        let (s, now) = (auth(), Instant::now());
        let w = s.begin_pairing(now).unwrap();
        assert_eq!(w.expires, now + Duration::from_secs(120));
        let a = s.pair(&w.pin, "phone", Some(Role::Viewer), A, now).unwrap();
        assert_eq!(a.role, Role::Viewer);
        let token = a.token.unwrap();
        assert_eq!(token.len(), 43, "32 bytes, base64url");
        assert!(!s.pairing_open(now));
        assert_eq!(
            s.pair(&w.pin, "second", None, B, now),
            Err(AuthError::Unauthenticated),
            "one PIN pairs one client"
        );
    }

    #[test]
    fn wp_hb1_no_requested_role_means_control() {
        let (s, now) = (auth(), Instant::now());
        let w = s.begin_pairing(now).unwrap();
        assert_eq!(
            s.pair(&w.pin, "x", None, A, now).unwrap().role,
            Role::Control
        );
    }

    #[test]
    fn wp_hb1_a_pin_with_no_window_is_wrong_and_counts() {
        let (s, now) = (auth(), Instant::now());
        for _ in 0..FAILURE_LIMIT {
            assert_eq!(
                s.pair("123456", "x", None, A, now),
                Err(AuthError::Unauthenticated)
            );
        }
        assert_eq!(
            s.pair("123456", "x", None, A, now),
            Err(AuthError::RateLimited)
        );
    }

    #[test]
    fn wp_hb1_a_window_expires() {
        let (s, now) = (auth(), Instant::now());
        let w = s.begin_pairing(now).unwrap();
        let late = now + PAIRING_WINDOW;
        assert_eq!(
            s.pair(&w.pin, "x", None, A, late),
            Err(AuthError::Unauthenticated)
        );
        assert!(
            !s.pairing_open(now),
            "an expired window is gone, not paused"
        );
    }

    #[test]
    fn wp_hb1_a_cancelled_window_takes_no_pin() {
        let (s, now) = (auth(), Instant::now());
        let w = s.begin_pairing(now).unwrap();
        s.cancel_pairing();
        assert_eq!(
            s.pair(&w.pin, "x", None, A, now),
            Err(AuthError::Unauthenticated)
        );
    }

    #[test]
    fn wp_hb1_a_new_window_replaces_the_old_pin_and_its_count() {
        let (s, now) = (auth(), Instant::now());
        let first = s.begin_pairing(now).unwrap();
        let second = loop {
            let w = s.begin_pairing(now).unwrap();
            if w.pin != first.pin {
                break w;
            }
        };
        assert_eq!(
            s.pair(&first.pin, "x", None, A, now),
            Err(AuthError::Unauthenticated)
        );
        assert!(s.pair(&second.pin, "x", None, B, now).is_ok());
    }

    /// The burst: five wrong from one address, and the right PIN no longer helps it.
    #[test]
    fn wp_hb1_five_wrong_pins_shut_out_the_address_even_with_the_right_pin() {
        let (s, now) = (auth(), Instant::now());
        let w = s.begin_pairing(now).unwrap();
        let bad = wrong_pin(&w.pin);
        for _ in 0..FAILURE_LIMIT {
            assert_eq!(
                s.pair(&bad, "x", None, A, now),
                Err(AuthError::Unauthenticated)
            );
        }
        assert_eq!(
            s.pair(&w.pin, "x", None, A, now),
            Err(AuthError::RateLimited)
        );
        assert_eq!(s.verify_token("tok", A, now), Err(AuthError::RateLimited));
        let later = now + RATE_WINDOW;
        assert_eq!(
            s.verify_token("tok", A, later),
            Err(AuthError::Unauthenticated)
        );
    }

    /// Spreading the guesses over addresses does not get more of them.
    #[test]
    fn wp_hb1_five_wrong_pins_from_anywhere_end_the_window() {
        let (s, now) = (auth(), Instant::now());
        let w = s.begin_pairing(now).unwrap();
        let bad = wrong_pin(&w.pin);
        for n in 0..FAILURE_LIMIT {
            let peer = IpAddr::V4(Ipv4Addr::new(10, 0, 0, n as u8 + 1));
            assert_eq!(
                s.pair(&bad, "x", None, peer, now),
                Err(AuthError::Unauthenticated)
            );
        }
        assert!(!s.pairing_open(now));
        assert_eq!(
            s.pair(&w.pin, "x", None, B, now),
            Err(AuthError::Unauthenticated)
        );
    }

    #[test]
    fn wp_hb1_failures_older_than_a_minute_do_not_count() {
        let (s, now) = (auth(), Instant::now());
        for n in 0..4u64 {
            let t = now + Duration::from_secs(n);
            assert_eq!(s.verify_token("x", A, t), Err(AuthError::Unauthenticated));
        }
        let later = now + Duration::from_secs(70);
        for n in 0..4u64 {
            let t = later + Duration::from_secs(n);
            assert_eq!(s.verify_token("x", A, t), Err(AuthError::Unauthenticated));
        }
    }

    #[test]
    fn wp_hb1_a_good_token_clears_the_failures() {
        let (s, now) = (auth(), Instant::now());
        let token = paired(&s, now).token.unwrap();
        for _ in 0..4 {
            let _ = s.verify_token("wrong", A, now);
        }
        s.verify_token(&token, A, now).unwrap();
        for _ in 0..4 {
            assert_eq!(
                s.verify_token("wrong", A, now),
                Err(AuthError::Unauthenticated)
            );
        }
    }

    #[test]
    fn wp_hb1_a_failed_address_does_not_lock_out_another() {
        let (s, now) = (auth(), Instant::now());
        let token = paired(&s, now).token.unwrap();
        for _ in 0..FAILURE_LIMIT {
            let _ = s.verify_token("wrong", B, now);
        }
        assert_eq!(s.verify_token("wrong", B, now), Err(AuthError::RateLimited));
        s.verify_token(&token, A, now).unwrap();
    }

    /// A machine has all of its /64 to rotate through, so that is what is counted.
    #[test]
    fn wp_hb1_rotating_through_an_ipv6_slash_64_does_not_escape_the_limit() {
        let (s, now) = (auth(), Instant::now());
        let peer = |n: u16| IpAddr::V6(Ipv6Addr::new(0xfd00, 1, 2, 3, 0, 0, 0, n));
        for n in 0..FAILURE_LIMIT {
            let _ = s.verify_token("wrong", peer(n as u16), now);
        }
        assert_eq!(
            s.verify_token("wrong", peer(999), now),
            Err(AuthError::RateLimited)
        );
        let elsewhere = IpAddr::V6(Ipv6Addr::new(0xfd00, 1, 2, 4, 0, 0, 0, 1));
        assert_eq!(
            s.verify_token("wrong", elsewhere, now),
            Err(AuthError::Unauthenticated)
        );
    }

    #[test]
    fn wp_hb1_an_ipv4_mapped_peer_is_the_same_address() {
        let (s, now) = (auth(), Instant::now());
        let mapped = IpAddr::V6(Ipv4Addr::new(192, 168, 1, 10).to_ipv6_mapped());
        for _ in 0..FAILURE_LIMIT {
            let _ = s.verify_token("wrong", mapped, now);
        }
        assert_eq!(s.verify_token("wrong", A, now), Err(AuthError::RateLimited));
    }

    #[test]
    fn wp_hb1_the_failure_ledger_does_not_grow_without_bound() {
        let (s, now) = (auth(), Instant::now());
        for n in 0..(MAX_TRACKED_PEERS as u32 * 2) {
            let peer = IpAddr::V4(Ipv4Addr::from(0x0A00_0000 + n));
            let _ = s.verify_token("wrong", peer, now);
        }
        assert!(s.lock().peers.len() <= MAX_TRACKED_PEERS);
    }

    #[test]
    fn wp_hb1_only_a_hash_reaches_the_disk() {
        let dir = scratch("hash");
        let (s, now) = (store_in(&dir), Instant::now());
        let token = paired(&s, now).token.unwrap();
        let text = fs::read_to_string(dir.join("hub-store.json")).unwrap();
        assert!(!text.contains(&token));
        assert!(text.contains(&data_encoding::HEXLOWER.encode(&sha256(token.as_bytes()))));
        drop(s);
        let s = store_in(&dir);
        assert!(
            s.verify_token(&token, A, now).is_ok(),
            "a token survives a restart"
        );
    }

    #[test]
    fn wp_hb1_token_comparison_finds_any_client_and_no_almost() {
        let (s, now) = (auth(), Instant::now());
        let tokens: Vec<String> = (0..5).map(|_| paired(&s, now).token.unwrap()).collect();
        for (n, t) in tokens.iter().enumerate() {
            assert_eq!(s.verify_token(t, A, now).unwrap().cid as usize, n + 1);
        }
        let mut almost = tokens[0].clone();
        almost.pop();
        assert_eq!(
            s.verify_token(&almost, B, now),
            Err(AuthError::Unauthenticated)
        );
        assert_eq!(s.verify_token("", B, now), Err(AuthError::Unauthenticated));
        assert!(hashes_equal(&[9; 32], &[9; 32]));
        let mut last_differs = [9; 32];
        last_differs[31] = 8;
        assert!(!hashes_equal(&[9; 32], &last_differs));
    }

    #[test]
    fn wp_hb1_a_revoked_token_is_refused_and_counts_as_a_failure() {
        let (s, now) = (auth(), Instant::now());
        let a = paired(&s, now);
        assert!(s.revoke(a.cid));
        assert_eq!(s.role_of(a.cid), None);
        assert!(!s.revoke(a.cid), "already gone");
        assert_eq!(
            s.verify_token(a.token.as_deref().unwrap(), A, now),
            Err(AuthError::Unauthenticated)
        );
    }

    #[test]
    fn wp_hb1_a_revocation_survives_a_restart() {
        let dir = scratch("revoke");
        let (s, now) = (store_in(&dir), Instant::now());
        let a = paired(&s, now);
        s.revoke(a.cid);
        drop(s);
        let s = store_in(&dir);
        assert_eq!(
            s.verify_token(a.token.as_deref().unwrap(), A, now),
            Err(AuthError::Unauthenticated)
        );
    }

    #[test]
    fn wp_hb1_a_role_change_is_kept() {
        let dir = scratch("role");
        let (s, now) = (store_in(&dir), Instant::now());
        let a = paired(&s, now);
        assert!(s.set_role(a.cid, Role::Viewer));
        assert!(!s.set_role(999, Role::Admin));
        drop(s);
        let s = store_in(&dir);
        assert_eq!(s.clients()[0].role, Role::Viewer);
        assert_eq!(
            s.verify_token(a.token.as_deref().unwrap(), A, now)
                .unwrap()
                .role,
            Role::Viewer
        );
    }

    #[test]
    fn wp_hb1_switching_the_mode_revokes_every_token() {
        let dir = scratch("mode");
        let (s, now) = (store_in(&dir), Instant::now());
        let token = paired(&s, now).token.unwrap();
        drop(s);
        drop(store_in(&dir));
        let same = store_in(&dir);
        assert!(same.verify_token(&token, A, now).is_ok(), "same mode: kept");
        drop(same);
        drop(AuthStore::new(HubStore::load(&dir).unwrap(), AccessMode::Open).unwrap());
        let s = store_in(&dir);
        assert!(s.clients().is_empty());
        assert_eq!(
            s.verify_token(&token, A, now),
            Err(AuthError::Unauthenticated)
        );
    }

    #[test]
    fn wp_hb1_a_name_is_capped_and_cleaned() {
        let (s, now) = (auth(), Instant::now());
        for (given, expect) in [
            ("Pixel 9", "Pixel 9".to_owned()),
            ("", UNNAMED_CLIENT.to_owned()),
            ("   ", UNNAMED_CLIENT.to_owned()),
            ("\u{1b}[31mred\n\0", "[31mred".to_owned()),
            ("left\u{202E}right\u{2066}", "leftright".to_owned()),
            (&"é".repeat(5000), "é".repeat(MAX_CLIENT_NAME)),
        ] {
            let w = s.begin_pairing(now).unwrap();
            let a = s.pair(&w.pin, given, None, A, now).unwrap();
            let c = s.clients().into_iter().find(|c| c.cid == a.cid).unwrap();
            assert_eq!(c.name, expect);
        }
    }

    #[test]
    fn wp_hb1_a_full_table_refuses_and_evicts_nobody() {
        let (s, now) = (auth(), Instant::now());
        for _ in 0..MAX_PAIRED_CLIENTS {
            paired(&s, now);
        }
        let w = s.begin_pairing(now).unwrap();
        assert_eq!(s.pair(&w.pin, "x", None, B, now), Err(AuthError::TableFull));
        assert_eq!(s.clients().len(), MAX_PAIRED_CLIENTS);
        assert!(s.pairing_open(now), "the PIN was right: it is not spent");
        assert!(s.revoke(1));
        assert!(s.pair(&w.pin, "x", None, B, now).is_ok());
    }

    #[test]
    fn wp_hb1_secrets_stay_out_of_debug_output() {
        let (s, now) = (auth(), Instant::now());
        let w = s.begin_pairing(now).unwrap();
        assert!(!format!("{w:?}").contains(&w.pin));
        let a = s.pair(&w.pin, "x", None, A, now).unwrap();
        assert!(!format!("{a:?}").contains(a.token.as_deref().unwrap()));
    }

    #[test]
    fn wp_hb1_the_default_mode_is_pin() {
        assert_eq!(AccessMode::default(), AccessMode::Pin);
        assert_eq!(AccessMode::Pin.wire_name(), "pin");
        assert_eq!(AccessMode::Open.wire_name(), "none");
    }
}
