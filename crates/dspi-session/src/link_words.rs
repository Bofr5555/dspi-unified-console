//! The words for each state of a DSPi Link connection (WP CG10, moved here by
//! WP TM8): one table for every client.
//!
//! It lives this low because the Terminal has no `dspi-core`. `dspi-core` maps
//! its `LinkPhase` onto these entries and exports them to C and UniFFI, so each
//! key and English sentence is written only here. See `dspi_core::link_text`
//! for how clients use them.

/// A button or key a state must offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkAction {
    /// Stable key of `text`, for localising.
    pub key: &'static str,
    /// The English.
    pub text: &'static str,
}

/// What to tell the user about one state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkWords {
    /// Stable key of `text`, for localising. Keys are never reused.
    pub key: &'static str,
    /// The English.
    pub text: &'static str,
    /// The way out, for a state that must offer one.
    pub action: Option<LinkAction>,
}

const fn words(key: &'static str, text: &'static str) -> LinkWords {
    LinkWords {
        key,
        text,
        action: None,
    }
}

/// No session.
pub const IDLE: LinkWords = words("idle", "Not connected to a hub");
/// The hub wants its PIN, and that PIN is fixed.
pub const NEEDS_PAIRING_FIXED: LinkWords = words("needs_pairing.fixed", "Enter the hub's PIN.");
/// The hub wants a PIN from a pairing window, or does not say.
pub const NEEDS_PAIRING_WINDOW: LinkWords = words(
    "needs_pairing.window",
    "Enter the PIN shown on the hub. On a DSPi Console, open Settings > Networking \
     and choose Allow a New Client to get one.",
);
/// Connected, and the hub has no DSPi to offer.
pub const NO_DEVICE: LinkWords = words("no_device", "Connected to hub, no DSPi found");
/// Connected, and a DSPi is available but not chosen.
pub const CHOOSE_DEVICE: LinkWords = words("choose_device", "Connected to hub, choose a DSPi");
/// Connected to a device.
pub const BOUND: LinkWords = words("bound", "Connected");
/// The session was lost and is being retried. The owner's rule is that a lost
/// connection always offers the way out.
pub const RECONNECTING: LinkWords = LinkWords {
    key: "reconnecting",
    text: "Connection lost, reconnecting",
    action: Some(DISCONNECT),
};
/// [`RECONNECTING`]'s way out.
pub const DISCONNECT: LinkAction = LinkAction {
    key: "reconnecting.disconnect",
    text: "Disconnect",
};
/// The session ended, and nothing is retrying it.
pub const CLOSED: LinkWords = words("closed", "The connection to the hub has ended");
/// The hub would not have this client.
pub const REFUSED: LinkWords = words("refused", "The hub refused this connection");
