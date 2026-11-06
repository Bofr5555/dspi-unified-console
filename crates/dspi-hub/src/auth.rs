//! Access: PIN pairing, tokens, roles, rate limits (spec 6). Owned by WP HB1.
//!
//! **PIN pairing is the default and open access is opt-in**, by owner's decision and
//! against the spec's "hubs default to `none`" (6.1); HUB.md section 8 records it.
//! A hub stores only hashes of tokens (spec 6.2). The clock is passed in.

use std::net::IpAddr;
use std::time::{Instant, SystemTime};

use crate::session::Role;

/// How a hub admits a client (spec 6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AccessMode {
    /// A new client pairs with a PIN and then presents a token. The default.
    #[default]
    Pin,
    /// Every connection is an `admin` session. Only when the user asks.
    Open,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authed {
    /// The client.
    pub cid: u32,
    /// The role it gets.
    pub role: Role,
    /// The token to hand over, on a pairing only. Never logged.
    pub token: Option<String>,
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
}

/// A generated PIN and its window (spec 6.2: 6 digits, 2 minutes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingWindow {
    /// The PIN to display.
    pub pin: String,
    /// When it stops working.
    pub expires: Instant,
}

/// The hub's clients and pairing state. Lock level L1b: never held across I/O other
/// than the store's atomic save, and never across a call into the hub.
#[derive(Debug, Default)]
pub struct AuthStore {}

impl AuthStore {
    /// The current mode.
    pub fn mode(&self) -> AccessMode {
        todo!("WP HB1")
    }

    /// Open a pairing window, replacing an open one.
    pub fn begin_pairing(&self, now: Instant) -> PairingWindow {
        let _ = now;
        todo!("WP HB1")
    }

    /// Close the pairing window.
    pub fn cancel_pairing(&self) {
        todo!("WP HB1")
    }

    /// `auth.pair`: a right PIN gives a token. Rate limited per address.
    pub fn pair(
        &self,
        pin: &str,
        name: &str,
        requested: Option<Role>,
        peer: IpAddr,
        now: Instant,
    ) -> Result<Authed, AuthError> {
        let _ = (pin, name, requested, peer, now);
        todo!("WP HB1")
    }

    /// `auth.token`. Rate limited per address.
    pub fn verify_token(
        &self,
        token: &str,
        peer: IpAddr,
        now: Instant,
    ) -> Result<Authed, AuthError> {
        let _ = (token, peer, now);
        todo!("WP HB1")
    }

    /// `auth.list`.
    pub fn clients(&self) -> Vec<PairedClient> {
        todo!("WP HB1")
    }

    /// `auth.revoke`. The caller closes the client's sessions with 4002.
    pub fn revoke(&self, cid: u32) -> bool {
        let _ = cid;
        todo!("WP HB1")
    }

    /// `auth.set_role`. The caller applies it to the client's live sessions.
    pub fn set_role(&self, cid: u32, role: Role) -> bool {
        let _ = (cid, role);
        todo!("WP HB1")
    }
}
