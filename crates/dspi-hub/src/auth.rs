//! Access: PIN pairing, tokens, roles, rate limits (spec 6). Owned by WP HB1.
//!
//! **PIN pairing is the default and open access is opt-in**, by owner's decision and
//! against the spec's "hubs default to `none`" (6.1); HUB.md section 8 records it.
//! A hub stores only hashes of tokens (spec 6.2). The clock is passed in.
//!
//! **No fixed PIN.** The spec's `fixed` source is for hubs with no screen; every hub
//! here has a UI or a CLI to show a window PIN (HUB.md section 9), so a PIN exists only
//! for a window, and five failures from anywhere end it.
//!
//! -- WP SECPAIR -- **Pairing bound to the key, ratchets and lifetimes (PLAN-SEC 4.3, 4.5,
//! 4.9; OD-SECg, OD-SECm):**
//! - A wrong PIN and an unknown token are counted in **two ledgers**, each per address
//!   ([`FAILURE_LIMIT`] in [`RATE_WINDOW`], then the address waits): a stale client
//!   retrying a revoked token neither shuts its address out of pairing nor touches the
//!   window, which only wrong PINs end. An address shut out for wrong PINs is refused
//!   tokens too, as before (spec 6.2).
//! - **`tls_only`** (ratchet (b)): a console's token that has authenticated over TLS,
//!   or was issued over TLS, is refused over plaintext from then on
//!   ([`AuthError::TlsRequired`]). API keys are not ratcheted: REST has its own rule
//!   (`plaintext_ok`).
//! - **Idle expiry**: a console's token unused for [`DEFAULT_TOKEN_IDLE`] (adjustable,
//!   [`AuthStore::set_token_idle`]) is forgotten, on the hub's wall clock
//!   ([`AuthStore::set_clock`]); never while that clock is not plausible (before
//!   [`crate::tls_identity::CLOCK_FLOOR`]). API keys do not expire.
//! - **`auth.rotate`** ([`AuthStore::rotate_token`]): a fresh token for the same client in
//!   one locked step; the old one is dead the moment the new one is stored.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::hint::black_box;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use data_encoding::BASE64URL_NOPAD;
use sha2::{Digest, Sha256};

use crate::clock::{HubClock, SystemClock};
use crate::error::HubError;
use crate::registry::NameStore;
use crate::session::Role;
use crate::store::{HubStore, StoredClient, TokenHash, random_bytes, system_time, unix_seconds};
// -- WP P2RS --
pub use crate::store::ClientKind;

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
// -- WP PAIRADM --
/// The most a PIN pairing gives, whatever the client asks. Admin is given only
/// on the hub's machine (`uconsole hub role <cid> admin`, [`AuthStore::set_role`]):
/// there is no pairing window that gives more (owner, 2026-11-14).
pub const PAIR_MAX_ROLE: Role = Role::Control;
/// What a client with no usable name is listed as.
const UNNAMED_CLIENT: &str = "Unnamed client";
/// How stale a stored `last_seen` may get, so a client reconnecting in a loop does not
/// rewrite the file each time.
const LAST_SEEN_RESOLUTION: u64 = 60;
// -- WP SECPAIR --
/// How long a console's token may go unused before it is forgotten (OD-SECg).
pub const DEFAULT_TOKEN_IDLE: Duration = Duration::from_secs(180 * 86_400);

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
    // -- WP P2RS --
    /// A console or an API key.
    pub kind: ClientKind,
    /// An API key that may be used over plaintext once the port speaks TLS (WP SECSRV);
    /// no effect yet.
    pub plaintext_ok: bool,
}

// -- WP P2RS --
/// An API key just minted ([`AuthStore::mint_api_key`]). The key is in here and
/// nowhere else: the store keeps its hash.
#[derive(Clone, PartialEq, Eq)]
pub struct MintedKey {
    /// Its client id, for `auth.revoke`.
    pub cid: u32,
    /// Its name, as cleaned.
    pub name: String,
    /// Its role.
    pub role: Role,
    /// As asked.
    pub plaintext_ok: bool,
    /// The key. Show it once; never log it.
    pub key: String,
}

/// By hand, so a key never reaches a log.
impl fmt::Debug for MintedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MintedKey")
            .field("cid", &self.cid)
            .field("name", &self.name)
            .field("role", &self.role)
            .field("plaintext_ok", &self.plaintext_ok)
            .field("key", &"<redacted>")
            .finish()
    }
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
    // -- WP P2CFIXA --
    /// What it authenticated as: a console's token opens DSPi Link sessions only, an
    /// API key REST (and MQTT) only. The caller refuses the other kind.
    pub kind: ClientKind,
}

/// By hand, so a token never reaches a log.
impl fmt::Debug for Authed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Authed")
            .field("cid", &self.cid)
            .field("role", &self.role)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("kind", &self.kind)
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
    // -- WP SECPAIR --
    /// A right token that has been used over TLS, presented over plaintext
    /// (`tls_required`, ratchet (b)): the client must come back encrypted.
    #[error("tls_required")]
    TlsRequired,
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

struct OpenWindow {
    pin_hash: TokenHash,
    expires: Instant,
    failures: usize,
}

// -- WP SECPAIR -- by hand: not even the PIN's hash goes to a log
impl fmt::Debug for OpenWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenWindow")
            .field("pin_hash", &"<redacted>")
            .field("expires", &self.expires)
            .field("failures", &self.failures)
            .finish()
    }
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

// -- WP SECPAIR --
/// Failures per address, of one kind (PINs, or tokens), and the addresses waiting.
#[derive(Debug, Default)]
struct Ledger {
    peers: BTreeMap<IpAddr, PeerRecord>,
}

#[derive(Debug)]
struct Inner {
    store: HubStore,
    window: Option<OpenWindow>,
    /// Wrong PINs, per address. -- WP SECPAIR -- tokens have [`Self::tokens`].
    peers: Ledger,
    // -- WP SECPAIR --
    /// Unknown tokens and API keys, per address: apart from the PINs'.
    tokens: Ledger,
    /// The hub's time: `created`, `last_seen` and idle expiry.
    clock: Arc<dyn HubClock>,
    /// Idle expiry of console tokens; `None`: never.
    token_idle: Option<Duration>,
    /// The last window ended by [`FAILURE_LIMIT`] wrong PINs, not yet told.
    window_killed: bool,
}

/// The hub's clients and pairing state. Lock level L1b: never held across I/O other
/// than the store's atomic save, and never across a call into the hub.
#[derive(Debug)]
pub struct AuthStore {
    mode: AccessMode,
    // -- WP HB6 --
    tokens_revoked: usize,
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

// -- WP SECPAIR -- the ledger as it was, now one of two
impl Ledger {
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

    fn clear(&mut self, peer: IpAddr) {
        self.peers.remove(&peer_key(peer));
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.peers.len()
    }
}

impl Inner {
    fn save(&self) -> Result<(), HubError> {
        self.store.save()
    }

    fn client_mut(&mut self, cid: u32) -> Option<&mut StoredClient> {
        self.store.clients_mut().iter_mut().find(|c| c.cid == cid)
    }

    // -- WP SECPAIR --
    /// The wall clock now, in seconds.
    fn wall_seconds(&self) -> u64 {
        unix_seconds(self.clock.wall())
    }

    // -- WP SECPAIR --
    /// Forget every console whose token has not been used for [`Self::token_idle`]
    /// (OD-SECg), and store that. API keys stay. Nothing while the wall clock is not
    /// plausible, and a `last_seen` from such a clock does not count against a client.
    fn prune_idle(&mut self) -> usize {
        let Some(idle) = self.token_idle else {
            return 0;
        };
        let now = self.wall_seconds();
        let floor = crate::tls_identity::CLOCK_FLOOR;
        if now < floor {
            return 0;
        }
        let idle = idle.as_secs();
        let stale = |c: &StoredClient| {
            let seen = c.last_seen.unwrap_or(c.created).max(c.created);
            c.kind == ClientKind::Console && seen >= floor && now.saturating_sub(seen) >= idle
        };
        let gone: Vec<u32> = self
            .store
            .clients()
            .iter()
            .filter(|c| stale(c))
            .map(|c| c.cid)
            .collect();
        if gone.is_empty() {
            return 0;
        }
        self.store.clients_mut().retain(|c| !gone.contains(&c.cid));
        log::info!(
            "hub: {} client(s) unused for {} days forgotten: {gone:?}",
            gone.len(),
            idle / 86_400
        );
        if let Err(e) = self.save() {
            log::error!("hub: forgetting idle clients not stored: {e}");
        }
        gone.len()
    }

    // -- WP SECPAIR --
    /// The client whose token hashes to `presented`: every stored hash is compared in
    /// constant time, whichever matches.
    fn find_token(&self, presented: &TokenHash) -> Option<u32> {
        self.store.clients().iter().fold(None, |found, c| {
            let same = hashes_equal(&c.token_hash, presented);
            if same { Some(c.cid) } else { found }
        })
    }
}

// -- WP SECPAIR --
/// Where pairing stands, for a hub's UI (PLAN-SEC 4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingState {
    /// A PIN is accepted until then.
    Open {
        /// When the window ends.
        expires: Instant,
    },
    /// No window.
    Closed,
    /// The last window was ended by [`FAILURE_LIMIT`] wrong PINs: the UI says
    /// "pairing was closed after 5 wrong PINs", so the person reopens it knowingly.
    ClosedAfterWrongPins,
}

// -- WP SECPAIR --
/// A token just made by [`AuthStore::rotate_token`]. Shown to its client once; never
/// logged.
#[derive(Clone, PartialEq, Eq)]
pub struct RotatedToken {
    /// The client.
    pub cid: u32,
    /// The new token. The old one is dead.
    pub token: String,
}

impl fmt::Debug for RotatedToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RotatedToken")
            .field("cid", &self.cid)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl AuthStore {
    /// Over this store, in this mode. Switching the mode revokes every token (spec 6.2):
    /// a mode different from the store's last one forgets the paired clients.
    pub fn new(mut store: HubStore, mode: AccessMode) -> Result<Self, HubError> {
        let word = mode.wire_name();
        // -- WP HB6 --
        let tokens_revoked = tokens_lost_by_switch(store.access(), store.clients().len(), mode);
        if store.access() != Some(word) {
            if tokens_revoked > 0 {
                log::info!("hub: the access mode changed, every token is revoked");
                store.clients_mut().clear();
            }
            store.set_access(word);
            store.save()?;
        }
        Ok(Self {
            mode,
            tokens_revoked,
            inner: Mutex::new(Inner {
                store,
                window: None,
                peers: Ledger::default(),
                // -- WP SECPAIR --
                tokens: Ledger::default(),
                clock: Arc::new(SystemClock),
                token_idle: Some(DEFAULT_TOKEN_IDLE),
                window_killed: false,
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // Each update is complete before anything that could panic.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `hid`, from the store.
    pub fn hub_id(&self) -> String {
        self.lock().store.hub_id().to_owned()
    }

    /// The current mode.
    pub fn mode(&self) -> AccessMode {
        self.mode
    }

    /// Open a pairing window, replacing an open one. [`HubError::Io`] if the system
    /// has no random numbers, since a guessable PIN is worse than none.
    ///
    /// -- WP HWLK1F, PAIRADM -- Its pairing gives at most [`PAIR_MAX_ROLE`]
    /// (`control`), whatever the client asks. Admin is given only on the hub's
    /// machine, with `uconsole hub role <cid> admin` ([`Self::set_role`]).
    pub fn begin_pairing(&self, now: Instant) -> Result<PairingWindow, HubError> {
        let pin = new_pin()?;
        let expires = after(now, PAIRING_WINDOW);
        let mut s = self.lock();
        s.window = Some(OpenWindow {
            pin_hash: sha256(pin.as_bytes()),
            expires,
            failures: 0,
        });
        // -- WP SECPAIR -- a new window: the old one's end is old news
        s.window_killed = false;
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
    /// No `requested` role means `control` (the Swift hub's default). -- WP HWLK1F,
    /// PAIRADM -- A role above [`PAIR_MAX_ROLE`] is cut to it: asking for admin
    /// with a PIN gets control.
    pub fn pair(
        &self,
        pin: &str,
        name: &str,
        requested: Option<Role>,
        peer: IpAddr,
        now: Instant,
    ) -> Result<Authed, AuthError> {
        self.pair_over(pin, name, requested, peer, now, false)
    }

    // -- WP SECPAIR --
    /// [`Self::pair`] on a connection that is `encrypted` (TLS) or not: a token issued
    /// over TLS is `tls_only` from the start (ratchet (b)). Whether plaintext pairing
    /// is allowed at all is the server's to decide, before this (OD-SECk).
    pub fn pair_over(
        &self,
        pin: &str,
        name: &str,
        requested: Option<Role>,
        peer: IpAddr,
        now: Instant,
        encrypted: bool,
    ) -> Result<Authed, AuthError> {
        let mut s = self.lock();
        if s.peers.is_blocked(peer, now) {
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
            s.peers.note_failure(peer, now);
            if let Some(w) = s.window.as_mut() {
                w.failures = w.failures.saturating_add(1);
                if w.failures >= FAILURE_LIMIT {
                    s.window = None;
                    // -- WP SECPAIR -- for the hub's UI (PLAN-SEC 4.5)
                    s.window_killed = true;
                    log::warn!("hub: pairing closed after {FAILURE_LIMIT} wrong PINs");
                }
            }
            return Err(AuthError::Unauthenticated);
        }
        // -- WP SECPAIR -- idle clients make room first
        s.prune_idle();
        if s.store.clients().len() >= MAX_PAIRED_CLIENTS {
            return Err(AuthError::TableFull);
        }
        let raw: [u8; 32] = random_bytes().map_err(|e| {
            log::error!("hub: no token made: {e}");
            AuthError::Internal
        })?;
        let token = BASE64URL_NOPAD.encode(&raw);
        let role = requested.unwrap_or(Role::Control).min(PAIR_MAX_ROLE);
        let seconds = s.wall_seconds(); // -- WP SECPAIR -- the hub's clock
        let cid = s.store.take_cid();
        s.store.clients_mut().push(StoredClient {
            cid,
            name: clean_name(name),
            role,
            created: seconds,
            last_seen: Some(seconds),
            token_hash: sha256(token.as_bytes()),
            // -- WP P2RS --
            kind: ClientKind::Console,
            plaintext_ok: false,
            // -- WP SECPAIR --
            tls_only: encrypted,
        });
        if let Err(e) = s.save() {
            log::error!("hub: pairing not stored: {e}");
            s.store.clients_mut().retain(|c| c.cid != cid);
            return Err(AuthError::Internal);
        }
        s.window = None;
        s.peers.clear(peer);
        Ok(Authed {
            cid,
            role,
            token: Some(token),
            // -- WP P2CFIXA --
            kind: ClientKind::Console,
        })
    }

    /// `auth.token`. Rate limited per address. Every stored hash is compared, in
    /// constant time, whichever matches.
    ///
    /// -- WP SECPAIR -- Counted in the tokens' ledger, not the PINs'. No `tls_only`
    /// ratchet here: this is the REST mirror's check (API keys, `plaintext_ok`); a
    /// DSPi Link session uses [`Self::verify_link_token`].
    pub fn verify_token(
        &self,
        token: &str,
        peer: IpAddr,
        now: Instant,
    ) -> Result<Authed, AuthError> {
        self.verify(token, peer, now, None)
    }

    // -- WP SECPAIR --
    /// `auth.token` on a DSPi Link connection, `encrypted` (TLS) or not. Ratchet (b):
    /// a console's token used over TLS is marked `tls_only`, and a `tls_only` token
    /// over plaintext is [`AuthError::TlsRequired`] (not a failure: it is a right
    /// token, in the wrong place). An idle console's token is forgotten first.
    pub fn verify_link_token(
        &self,
        token: &str,
        peer: IpAddr,
        now: Instant,
        encrypted: bool,
    ) -> Result<Authed, AuthError> {
        self.verify(token, peer, now, Some(encrypted))
    }

    /// [`Self::verify_token`] and [`Self::verify_link_token`]: `link` is the
    /// connection's TLS on a DSPi Link session, `None` for REST.
    fn verify(
        &self,
        token: &str,
        peer: IpAddr,
        now: Instant,
        link: Option<bool>,
    ) -> Result<Authed, AuthError> {
        let mut s = self.lock();
        // -- WP SECPAIR -- an address shut out for wrong PINs is shut out for tokens too
        // (spec 6.2), but token failures never shut it out of pairing
        if s.tokens.is_blocked(peer, now) || s.peers.is_blocked(peer, now) {
            return Err(AuthError::RateLimited);
        }
        // -- WP SECPAIR -- an idle token reads as unknown, as a revoked one does
        s.prune_idle();
        let presented = sha256(token.as_bytes());
        let Some(cid) = s.find_token(&presented) else {
            s.tokens.note_failure(peer, now);
            return Err(AuthError::Unauthenticated);
        };
        let seconds = s.wall_seconds();
        let Some(client) = s.client_mut(cid) else {
            return Err(AuthError::Unauthenticated);
        };
        let role = client.role;
        // -- WP P2CFIXA --
        let kind = client.kind;
        // -- WP SECPAIR -- ratchet (b), for a console's token on a DSPi Link session
        let mut marked = false;
        if let (Some(encrypted), ClientKind::Console) = (link, kind) {
            if !encrypted && client.tls_only {
                log::info!(
                    "hub: client {cid}'s token, used over TLS before, refused over plaintext"
                );
                return Err(AuthError::TlsRequired);
            }
            if encrypted && !client.tls_only {
                client.tls_only = true;
                marked = true;
            }
        }
        let stale = client
            .last_seen
            .is_none_or(|seen| seconds.saturating_sub(seen) >= LAST_SEEN_RESOLUTION);
        if stale {
            client.last_seen = Some(seconds);
        }
        if (stale || marked)
            && let Err(e) = s.save()
        {
            log::warn!("hub: last-seen time or TLS mark not stored: {e}");
        }
        s.tokens.clear(peer);
        Ok(Authed {
            cid,
            role,
            token: None,
            // -- WP P2CFIXA --
            kind,
        })
    }

    /// `auth.list`. -- WP SECPAIR -- Idle clients are forgotten first.
    pub fn clients(&self) -> Vec<PairedClient> {
        let mut s = self.lock();
        s.prune_idle();
        s.store
            .clients()
            .iter()
            .map(|c| PairedClient {
                cid: c.cid,
                name: c.name.clone(),
                role: c.role,
                created: system_time(c.created),
                last_seen: c.last_seen.map(system_time),
                // -- WP P2RS --
                kind: c.kind,
                plaintext_ok: c.plaintext_ok,
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

    // -- WP P2RS --
    /// `auth.mint`: an API key, a paired client of kind [`ClientKind::Api`] made without
    /// a PIN (OD-P2q). It shares [`MAX_PAIRED_CLIENTS`] with the consoles
    /// ([`AuthError::TableFull`] past it) and is revoked like one. The key is 32 bytes
    /// from the OS's CSPRNG; only its SHA-256 is stored, and a key that would not
    /// survive a restart is not handed out ([`AuthError::Internal`]). The caller checks
    /// that `role` is not above its own.
    pub fn mint_api_key(
        &self,
        name: &str,
        role: Role,
        plaintext_ok: bool,
    ) -> Result<MintedKey, AuthError> {
        let mut s = self.lock();
        if s.store.clients().len() >= MAX_PAIRED_CLIENTS {
            return Err(AuthError::TableFull);
        }
        let raw: [u8; 32] = random_bytes().map_err(|e| {
            log::error!("hub: no API key made: {e}");
            AuthError::Internal
        })?;
        let key = BASE64URL_NOPAD.encode(&raw);
        let cid = s.store.take_cid();
        let name = clean_name(name);
        let created = s.wall_seconds(); // -- WP SECPAIR -- the hub's clock
        s.store.clients_mut().push(StoredClient {
            cid,
            name: name.clone(),
            role,
            created,
            last_seen: None,
            token_hash: sha256(key.as_bytes()),
            kind: ClientKind::Api,
            plaintext_ok,
            // -- WP SECPAIR -- API keys are not ratcheted
            tls_only: false,
        });
        if let Err(e) = s.save() {
            log::error!("hub: API key not stored: {e}");
            s.store.clients_mut().retain(|c| c.cid != cid);
            return Err(AuthError::Internal);
        }
        Ok(MintedKey {
            cid,
            name,
            role,
            plaintext_ok,
            key,
        })
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

// -- WP SECPAIR --
impl AuthStore {
    /// The hub's clock for `created`, `last_seen` and idle expiry ([`SystemClock`]
    /// until set; the hub sets its services' clock, so a test moves time).
    pub fn set_clock(&self, clock: Arc<dyn HubClock>) {
        self.lock().clock = clock;
    }

    /// How long a console's token may go unused before it is forgotten; `None`: never.
    /// [`DEFAULT_TOKEN_IDLE`] until set. Applied at once.
    pub fn set_token_idle(&self, idle: Option<Duration>) {
        let mut s = self.lock();
        s.token_idle = idle.filter(|d| !d.is_zero());
        s.prune_idle();
    }

    /// The idle limit in force.
    pub fn token_idle(&self) -> Option<Duration> {
        self.lock().token_idle
    }

    /// Forget the idle clients now (the hub also does at every authentication, pairing
    /// and listing). How many went.
    pub fn prune_idle(&self) -> usize {
        self.lock().prune_idle()
    }

    /// Where pairing stands now.
    pub fn pairing_state(&self, now: Instant) -> PairingState {
        let s = self.lock();
        match &s.window {
            Some(w) if now < w.expires => PairingState::Open { expires: w.expires },
            _ if s.window_killed => PairingState::ClosedAfterWrongPins,
            _ => PairingState::Closed,
        }
    }

    /// Whether a window was ended by wrong PINs since this was last asked: once per
    /// such end, for a UI that says so.
    pub fn take_window_killed(&self) -> bool {
        std::mem::take(&mut self.lock().window_killed)
    }

    /// `auth.rotate`: a fresh token for console `cid`, in place (OD-SECg). The new
    /// hash replaces the old under one lock and is stored before the token is given
    /// out; if it cannot be stored, nothing changed ([`AuthError::Internal`]). The old
    /// token is dead from then on. `encrypted`: asked over TLS, so the new token is
    /// `tls_only` (a client already ratcheted stays so). API keys are rotated by
    /// minting a new one and revoking the old ([`AuthError::Unauthenticated`] here).
    pub fn rotate_token(&self, cid: u32, encrypted: bool) -> Result<RotatedToken, AuthError> {
        let raw: [u8; 32] = random_bytes().map_err(|e| {
            log::error!("hub: no token made: {e}");
            AuthError::Internal
        })?;
        let token = BASE64URL_NOPAD.encode(&raw);
        let mut s = self.lock();
        let seconds = s.wall_seconds();
        let Some(client) = s.client_mut(cid).filter(|c| c.kind == ClientKind::Console) else {
            return Err(AuthError::Unauthenticated);
        };
        let old = (client.token_hash, client.tls_only, client.last_seen);
        client.token_hash = sha256(token.as_bytes());
        client.tls_only = client.tls_only || encrypted;
        client.last_seen = Some(seconds);
        if let Err(e) = s.save() {
            log::error!("hub: token rotation of client {cid} not stored: {e}");
            if let Some(client) = s.client_mut(cid) {
                (client.token_hash, client.tls_only, client.last_seen) = old;
            }
            return Err(AuthError::Internal);
        }
        log::info!("hub: client {cid} rotated its token");
        Ok(RotatedToken { cid, token })
    }

    /// Whether console `cid`'s token is `tls_only` (ratchet (b)).
    pub fn is_tls_only(&self, cid: u32) -> bool {
        self.lock()
            .store
            .clients()
            .iter()
            .any(|c| c.cid == cid && c.tls_only)
    }
}

// -- WP HB6 --
/// How many tokens opening a store in `mode` forgets: all of them when the store last
/// ran in another mode (spec 6.2). The one statement of the rule, for
/// [`AuthStore::new`], which applies it, and [`SavedHub`], which warns of it.
fn tokens_lost_by_switch(stored: Option<&str>, clients: usize, mode: AccessMode) -> usize {
    match stored {
        Some(word) if word != mode.wire_name() => clients,
        _ => 0,
    }
}

// -- WP HB6 --
impl AuthStore {
    /// Paired clients forgotten when this store opened, because the mode had changed.
    /// A UI says so, as those clients must pair again.
    pub fn tokens_revoked_at_start(&self) -> usize {
        self.tokens_revoked
    }

    /// Keep the hub's name for the next start. A failed save is logged: the name in use
    /// is the hub's own, fixed at its start.
    pub fn set_name(&self, name: &str) {
        let mut s = self.lock();
        if s.store.name() == Some(name) {
            return;
        }
        s.store.set_name(name);
        if let Err(e) = s.save() {
            log::error!("hub: name not stored: {e}");
        }
    }
}

// -- WP HB6 --
/// What a hub's store holds, read with no hub running, for a settings page. Reading it
/// changes nothing, and nothing in it is a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedHub {
    /// `hid`.
    pub hub_id: String,
    /// The hub's stored name, if it was ever given one.
    pub name: Option<String>,
    /// The access mode it last ran in, as [`AccessMode::wire_name`].
    pub access: Option<String>,
    /// The paired clients.
    pub clients: Vec<PairedClient>,
}

impl SavedHub {
    /// The store in `dir`; `None` if there is nothing usable (see [`HubStore::peek`]).
    pub fn peek(dir: &std::path::Path) -> Result<Option<Self>, HubError> {
        Ok(HubStore::peek(dir)?.map(|store| Self {
            hub_id: store.hub_id().to_owned(),
            name: store.name().map(str::to_owned),
            access: store.access().map(str::to_owned),
            clients: store
                .clients()
                .iter()
                .map(|c| PairedClient {
                    cid: c.cid,
                    name: c.name.clone(),
                    role: c.role,
                    created: system_time(c.created),
                    last_seen: c.last_seen.map(system_time),
                    // -- WP P2RS --
                    kind: c.kind,
                    plaintext_ok: c.plaintext_ok,
                })
                .collect(),
        }))
    }

    /// How many tokens starting in `mode` would revoke: what a UI warns of.
    pub fn tokens_lost_starting_in(&self, mode: AccessMode) -> usize {
        tokens_lost_by_switch(self.access.as_deref(), self.clients.len(), mode)
    }
}

/// Hub-stored device names (WP HB3's [`NameStore`]) in the hub's one store, through
/// the [`AuthStore`] that owns it: clients and names share a file, so two writers would
/// each put back what the other had just changed.
pub(crate) struct StoredNames(pub(crate) Arc<AuthStore>);

impl NameStore for StoredNames {
    fn load(&self) -> BTreeMap<String, String> {
        self.0.lock().store.device_names().clone()
    }

    fn save(&self, names: &BTreeMap<String, String>) -> Result<(), HubError> {
        let mut s = self.0.lock();
        s.store.set_device_names(names);
        s.save()
    }
}

// -- WP SW2 --
/// The system model, kept in the same file as the clients and names.
pub(crate) struct StoredSystem(pub(crate) Arc<AuthStore>);

impl crate::system::SystemStore for StoredSystem {
    fn load(&self) -> Option<crate::system::model::SystemModel> {
        self.0.lock().store.system()
    }

    fn save(&self, model: &crate::system::model::SystemModel) -> Result<(), HubError> {
        let mut s = self.0.lock();
        s.store.set_system(model)?;
        s.save()
    }

    // -- WP AUSB2 --
    fn load_usb_offsets(&self) -> Option<BTreeMap<String, i64>> {
        self.0.lock().store.system_usb_offsets().cloned()
    }

    fn save_usb_offsets(&self, offsets: &BTreeMap<String, i64>) -> Result<(), HubError> {
        let mut s = self.0.lock();
        s.store.set_system_usb_offsets(offsets);
        s.save()
    }
}

// -- WP P1HL --
/// Limits profiles, PIN hashes and pinned modes, kept in the same file as the
/// clients and names (`limits.rs`).
pub(crate) struct StoredLimitsFile(pub(crate) Arc<AuthStore>);

impl crate::limits::LimitsPersist for StoredLimitsFile {
    fn load(&self) -> crate::limits::StoredLimits {
        self.0.lock().store.limits()
    }

    fn save(&self, data: &crate::limits::StoredLimits) -> Result<(), HubError> {
        let mut s = self.0.lock();
        s.store.set_limits(data);
        s.save()
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

    // -- WP HWLK1F --
    #[test]
    fn wp_pairadm_a_pin_gives_at_most_control_admin_only_by_role() {
        let (s, now) = (auth(), Instant::now());
        let w = s.begin_pairing(now).unwrap();
        let a = s.pair(&w.pin, "asks", Some(Role::Admin), A, now).unwrap();
        assert_eq!(a.role, Role::Control, "asked for admin, got control");
        assert_eq!(s.role_of(a.cid), Some(Role::Control), "and kept so");
        let w = s.begin_pairing(now).unwrap();
        let v = s.pair(&w.pin, "less", Some(Role::Viewer), A, now).unwrap();
        assert_eq!(v.role, Role::Viewer, "less is given");
        // Admin: only `uconsole hub role <cid> admin` on the hub's machine.
        assert!(s.set_role(a.cid, Role::Admin));
        assert_eq!(s.role_of(a.cid), Some(Role::Admin));
        let w = s.begin_pairing(now).unwrap();
        let z = s.pair(&w.pin, "next", Some(Role::Admin), A, now).unwrap();
        assert_eq!(z.role, Role::Control, "every window caps at control");
        assert_eq!(PAIR_MAX_ROLE, Role::Control);
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
        assert!(s.lock().tokens.len() <= MAX_TRACKED_PEERS); // -- WP SECPAIR --
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

// -- WP SECPAIR --
#[cfg(test)]
#[path = "auth/tests_wp_secpair.rs"]
mod tests_wp_secpair;
