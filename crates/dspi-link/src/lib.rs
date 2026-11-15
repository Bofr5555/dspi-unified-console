//! DSPi Link client: protocol 1.2 (compatible with 1.1 hubs, spec 13.1).
//!
//! Spec: WeebLabs/DSPi-Link `spec/dspi_link_protocol_spec.md` at 86f10e3
//! (2026-11-02). Reference implementation: DSPi-Console `release/v1.1.6`
//! `Link/` (LinkClient, LinkFrames, LinkMessages, HubBrowser.parseAddress).
//!
//! Layers, each usable on its own:
//! - [`frames`]: the binary data plane (CMD, NOTIFY, POLL, FWDATA, RESYNC);
//! - [`messages`]: the JSON control plane;
//! - [`socket`]: the WebSocket byte pipe ([`socket::WsConnector`]; the
//!   built-in one is `tungstenite` over `std::net`);
//! - [`client`]: one session ([`LinkClient`]);
//! - [`address`]: manual entry (`host`, `host:port`) and `GET /dspi/v1/info`;
//! - [`discovery`]: finding hubs by DNS-SD (the network part behind feature `mdns`).
//!
//! The crate knows nothing about the DSPi core or any FFI; dspi-ffi adapts
//! a [`LinkClient`] to the core's `Transport` trait.

pub mod address;
pub mod client;
pub mod clock;
// -- WP CG2 --
pub mod discovery;
pub mod frames;
pub mod messages;
pub mod socket;
// -- WP SECPIPE -- native only: no sockets, rustls or ring on the web
#[cfg(all(feature = "tls", not(all(target_arch = "wasm32", target_os = "unknown"))))]
pub mod tls;

#[cfg(feature = "fake-hub")]
pub mod fakehub;

use std::time::Duration;

pub use client::{
    close_reason, should_reconnect, AuthState, ConnectOptions, LinkClient, LinkEvents, NotifyItem, SessionConfig,
    SessionState, Timing, TokenLookup,
};
// -- WP LK2 --
pub use client::CmdSpec;
pub use frames::{status, Dir};
pub use messages::{DeviceInfo, HelloHub, PinSource};

/// Default hub port: decimal of the USB VID 0x2E8B (spec 3.1).
pub const DEFAULT_PORT: u16 = 11915;
/// Default WebSocket path (spec 4).
pub const DEFAULT_PATH: &str = "/dspi/v1";

/// `text` from a hub, safe to show in a terminal or a log: control characters
/// (C0, DEL, C1, so ESC and newlines too) become U+FFFD. [`LinkError`]'s Display
/// applies this; its fields keep the hub's bytes.
pub fn sanitize_hub_text(text: &str) -> String {
    text.chars().map(|c| if c.is_control() { char::REPLACEMENT_CHARACTER } else { c }).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    #[error("network error: {}", sanitize_hub_text(.0))]
    Io(String),
    #[error("WebSocket handshake failed: {}", sanitize_hub_text(.0))]
    Handshake(String),
    #[error("protocol error: {}", sanitize_hub_text(.0))]
    Protocol(String),
    #[error("the hub did not answer in time")]
    Timeout,
    #[error("{}", sanitize_hub_text(reason))]
    Closed { code: Option<u16>, reason: String },
    /// An `err` reply: `code` is the spec 7.1 vocabulary (`denied`, `locked`, ...).
    #[error("hub refused: {} {}", sanitize_hub_text(code), sanitize_hub_text(msg))]
    Rejected { code: String, msg: String },
    /// The PIN was refused (`unauthenticated` from both real hubs, `denied` from
    /// others, spec 6.2). `pin_source` says how to word it; a window PIN may also
    /// have expired, or been used up by a success or by 5 failures.
    #[error("{}", match pin_source {
        PinSource::Fixed => "wrong PIN",
        PinSource::Window => "wrong or expired code; open a new pairing window on the hub",
    })]
    WrongPin { pin_source: PinSource },
    /// `rate_limited`: too many failed attempts from this address (spec 6.2).
    /// Says nothing about the token or the PIN; wait [`RATE_LIMIT_BACKOFF`].
    #[error("too many failed attempts; try again in a minute")]
    RateLimited { msg: String },
    /// HTTP 503 at the WebSocket upgrade: all the hub's sessions are in use.
    /// A vanished client holds its slot for up to 30 s, so this clears by itself.
    #[error("the hub is busy: all its sessions are in use")]
    HubBusy,
    /// Any other HTTP status at the WebSocket upgrade (400, 403, ...).
    #[error("the hub refused the connection (HTTP {status}): {}", sanitize_hub_text(msg))]
    HandshakeRefused { status: u16, msg: String },
}

/// Shortest wait after `rate_limited` (the hub locks an address for 60 s).
pub const RATE_LIMIT_BACKOFF: Duration = Duration::from_secs(60);
/// Shortest wait after a hub says it is busy or had an internal error.
pub const HUB_BUSY_BACKOFF: Duration = Duration::from_secs(5);
/// The reconnect backoff's first step and cap (spec 5: 1, 2, 4, ... 30 s).
const BACKOFF_FIRST: Duration = Duration::from_secs(1);
const BACKOFF_CAP: Duration = Duration::from_secs(30);

/// What to do after a failed connect or a lost session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryAdvice {
    /// Try again with backoff, waiting at least `min_delay`.
    Retry { min_delay: Duration },
    /// Trying again cannot help until something changes (the user, the hub's
    /// settings, the app version).
    Stop,
}

impl RetryAdvice {
    /// The wait before try `attempt` (0-based) of a run of failures: 1, 2, 4 ...
    /// 30 s, and never less than `min_delay`. `None` for [`RetryAdvice::Stop`].
    pub fn delay(self, attempt: u32) -> Option<Duration> {
        match self {
            RetryAdvice::Stop => None,
            RetryAdvice::Retry { min_delay } => {
                let step = BACKOFF_FIRST.saturating_mul(1u32.checked_shl(attempt).unwrap_or(u32::MAX));
                Some(step.min(BACKOFF_CAP).max(min_delay))
            }
        }
    }
}

impl LinkError {
    /// What an HTTP status other than 101 at the upgrade means. For foreign
    /// [`socket::WsConnector`]s too, so their failures read the same.
    pub fn from_handshake_status(status: u16, msg: impl Into<String>) -> LinkError {
        match status {
            503 => LinkError::HubBusy,
            _ => LinkError::HandshakeRefused { status, msg: msg.into() },
        }
    }

    /// Whether, and how soon, to try again. Whatever this says, the stored
    /// token stays: only [`AuthState::TokenRefused`] means it is bad.
    pub fn retry_advice(&self) -> RetryAdvice {
        const NOW: RetryAdvice = RetryAdvice::Retry { min_delay: Duration::ZERO };
        const BUSY: RetryAdvice = RetryAdvice::Retry { min_delay: HUB_BUSY_BACKOFF };
        match self {
            LinkError::Io(_) | LinkError::Timeout | LinkError::Handshake(_) => NOW,
            LinkError::Closed { code, .. } => {
                if should_reconnect(*code) {
                    NOW
                } else {
                    RetryAdvice::Stop
                }
            }
            LinkError::RateLimited { .. } => RetryAdvice::Retry { min_delay: RATE_LIMIT_BACKOFF },
            LinkError::HubBusy => BUSY,
            LinkError::HandshakeRefused { status, .. } if *status >= 500 => BUSY,
            LinkError::Rejected { code, .. } => match code.as_str() {
                "rate_limited" => RetryAdvice::Retry { min_delay: RATE_LIMIT_BACKOFF },
                "internal" | "busy" => BUSY,
                _ => RetryAdvice::Stop,
            },
            LinkError::HandshakeRefused { .. } | LinkError::Protocol(_) | LinkError::WrongPin { .. } => {
                RetryAdvice::Stop
            }
        }
    }
}
