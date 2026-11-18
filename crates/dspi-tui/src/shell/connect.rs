//! How the terminal reaches a device: USB, the demo device, USB MIDI or a
//! DSPi Link hub, and getting it back after it was lost.
//!
//! The rest of the terminal sees a [`Session`] and does not care which link is
//! under it. What does differ per link lives here:
//!
//! - **Opening** ([`Target`]): choosing a port or a hub's device, and pairing.
//! - **Who changed what** ([`attribution`], [`foreign_words`]): the echo rule
//!   of `docs/core-merge/PLAN.md` §1.2, in words.
//! - **Coming back** ([`Reconnector`]): a replug, or a hub that went away. The
//!   open runs on its own thread, because a Link connect can block for seconds
//!   and the interface has to keep drawing meanwhile.
//!
//! Owned by WP R1.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_cmd::COMMAND;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

// -- WP TM9 --
use dspi_link::discovery::{BrowseOptions, BrowseSource, BrowsedHub, HubBrowser};
use dspi_link::socket::TcpConnector;
use dspi_link::{
    AuthState, ConnectOptions, LinkClient, LinkError, LinkEvents, RetryAdvice, SessionConfig,
};
// -- WP SECTM --
use dspi_link::client::TlsOptions;
use dspi_link::credential::Credential;
use dspi_proto::generated::ext as ext_ids;
// -- WP TM4 --
use dspi_proto::generated::opcodes as op;
use dspi_proto::generated::wire::WIRE_FORMAT_VERSION;
use dspi_proto::packets::PlatformInfo;
use dspi_proto::wire_compat::Normalized;
use dspi_session::compat;
use dspi_session::ext_state::ExtState;
use dspi_session::probe::probe_any_with_image;
use dspi_session::{Capabilities, DeviceState, Session, Source, WriteError};

use super::read_only::{BulkReads, ReadOnlyTransport};
use dspi_transport::{
    DeviceDescriptor, FakeDevice, LinkTransport, Transport, TransportError, TransportKind,
    UsbTransport,
};
use serde::{Deserialize, Serialize};

/// The role a terminal asks a hub for: it edits, it does not administer.
const LINK_ROLE: &str = "control";

/// The wait between reconnect attempts on a local bus or MIDI port, which are
/// cheap to look at (and never faster than once a second, as midir asks). A hub
/// follows [`LinkError::retry_advice`] instead.
const RETRY_LOCAL: Duration = Duration::from_secs(1);

/// How long a pairing that timed out may still come good: the hub may have
/// spent its window on it, and the late `ok` carries the only token there is.
const PAIR_GRACE: Duration = Duration::from_secs(2);

/// `text` from a hub or a device, safe to print: control characters (C0, DEL,
/// C1, and so ESC) become U+FFFD, so a device name cannot rewrite the terminal
/// (PLAN §1.7). The interface needs none of this: its buffer drops them.
pub fn printable(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { '\u{FFFD}' } else { c })
        .collect()
}

/// Where the terminal should find its device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `--device`, or the only DSPi on the bus.
    Usb { serial: Option<String> },
    /// `--demo`: the built-in simulated device.
    Demo,
    /// `--midi [PORT]`: a USB MIDI control channel, by part of its name.
    Midi { port: Option<String> },
    /// `--link HOST[:PORT]`.
    Link(LinkTarget),
}

/// A hub, and which of its devices.
#[derive(Clone, PartialEq, Eq)]
pub struct LinkTarget {
    pub host: String,
    pub port: u16,
    /// A serial (or the end of one) or part of a device's name; `None` is the
    /// hub's only online device.
    pub device: Option<String>,
    /// `--pin`, for scripts and for hubs that issue no token.
    pub pin: Option<String>,
    /// The token file; `None` is the default in the config directory.
    pub tokens: Option<PathBuf>,
    // -- WP TM9 --
    /// The WebSocket path a hub announced; `None` is the default one.
    pub path: Option<String>,
    /// The hub's id, once known. A hub with another id at this address is not
    /// this hub, and a reconnect can look for this one elsewhere (F5).
    pub hub_id: Option<String>,
    /// `--hub`: still to be found on the network; see [`locate_hub`].
    pub find: Option<HubQuery>,
    // -- WP SECTM --
    /// The hub's DNS-SD advert said TLS (`tls=1`, [`BrowsedHub::tls_advertised`]):
    /// a failed TLS handshake is then `tls_refused`, never plaintext.
    pub tls_advertised: bool,
    /// A pasted `dspi://pair` link ([`LinkTarget::from_pair_link`]): the key
    /// the hub must present before the link's PIN goes out.
    pub pair_link: Option<PairHint>,
    /// As reached: whether the session runs over TLS with a verified key.
    /// `None` before a connect (or for a target as asked).
    pub encrypted: Option<bool>,
}

// -- WP SECTM --
/// What a `dspi://pair` link pins: shown on the confirmation, and the
/// credential (key, no token) the first dial uses.
#[derive(Clone, PartialEq, Eq)]
pub struct PairHint {
    /// The `v2` credential pinned to the link's `k=`, with no token.
    pub credential: String,
    /// The key's fingerprint, `SHA256:K7Q2-M9XD-4TNB-0ZRA`.
    pub hub_key: String,
}

// -- WP SECTM --
/// By hand: a PIN (from `--pin` or a pairing link) never reaches a log.
impl std::fmt::Debug for LinkTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkTarget")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("device", &self.device)
            .field("pin", &self.pin.as_ref().map(|_| "<redacted>"))
            .field("tokens", &self.tokens)
            .field("path", &self.path)
            .field("hub_id", &self.hub_id)
            .field("find", &self.find)
            .field("tls_advertised", &self.tls_advertised)
            .field("pair_link", &self.pair_link.as_ref().map(|p| &p.hub_key))
            .field("encrypted", &self.encrypted)
            .finish()
    }
}

// -- WP TM9 --
/// `--hub [NAME]`: which hub on the network is meant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubQuery {
    /// Part of a hub's name, its id or its address; `None` is whichever is there.
    pub wanted: Option<String>,
}

impl LinkTarget {
    /// `HOST`, `HOST:PORT` or a bracketed IPv6 address, as typed. `None` when
    /// it cannot be an address.
    pub fn parse(address: &str) -> Option<Self> {
        let (host, port) = dspi_link::address::parse_address(address)?;
        Some(Self {
            host,
            port,
            device: None,
            pin: None,
            tokens: None,
            path: None,
            hub_id: None,
            find: None,
            tls_advertised: false,
            pair_link: None,
            encrypted: None,
        })
    }

    // -- WP TM9 --
    /// A hub still to be found: `--hub`. [`locate_hub`] gives it an address.
    pub fn find(wanted: Option<String>) -> Self {
        Self {
            host: String::new(),
            port: 0,
            device: None,
            pin: None,
            tokens: None,
            path: None,
            hub_id: None,
            find: Some(HubQuery { wanted }),
            tls_advertised: false,
            pair_link: None,
            encrypted: None,
        }
    }

    /// The same device, at where `hub` announces itself now.
    fn at(&self, hub: &BrowsedHub) -> Self {
        Self {
            host: hub.host.clone(),
            port: hub.port,
            path: Some(hub.path.clone()),
            hub_id: Some(hub.id.clone()),
            find: None,
            tls_advertised: hub.tls_advertised, // -- WP SECTM --
            ..self.clone()
        }
    }

    /// Whether `hub_id`, from a hub's hello, is not the hub this target means.
    fn is_other_hub(&self, hub_id: &str) -> bool {
        self.hub_id
            .as_deref()
            .is_some_and(|mine| !mine.eq_ignore_ascii_case(hub_id))
    }

    fn ws_url(&self) -> String {
        dspi_link::address::ws_url(&self.host, self.port, self.path.as_deref())
    }

    fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    fn token_store(&self) -> TokenStore {
        match &self.tokens {
            Some(p) => TokenStore::at(p.clone()),
            None => TokenStore::default_location(),
        }
    }
}

// -- WP TMCORE --
// What the Terminal needs to open this target as a console core's DSPi Link
// session (`dspi_core::DspiCore::link_connect`) with this target's tokens.
impl LinkTarget {
    /// The hub's WebSocket address.
    pub fn link_url(&self) -> String {
        self.ws_url()
    }

    /// `HOST:PORT`, for messages.
    pub fn link_address(&self) -> String {
        self.address()
    }

    /// Whether `hub_id` is another hub than the one this target knows.
    pub fn names_other_hub(&self, hub_id: &str) -> bool {
        self.is_other_hub(hub_id)
    }

    /// The pairing token stored for `hub_id`, if any.
    pub fn stored_token(&self, hub_id: &str) -> Option<String> {
        self.token_store().get(hub_id)
    }

    /// Keep `token` for `hub_id`.
    pub fn store_token(&self, hub_id: &str, token: &str) -> std::io::Result<()> {
        self.token_store().put(hub_id, token)
    }

    /// Drop the token kept for `hub_id` (one the hub refused).
    /// -- WP PINTLS -- A hub that was pinned keeps a token-less record that
    /// still requires TLS ([`Self::drop_pairing`]); only [`Self::forget_hub`]
    /// removes everything.
    pub fn forget_token(&self, hub_id: &str) -> std::io::Result<()> {
        self.drop_pairing(hub_id)
    }

    // -- WP PINTLS --
    /// Pair Again, or a refused or revoked token: the token goes, but a hub
    /// that was ever pinned stays TLS-only (owner decision 2026-11-17): its
    /// record becomes the core's unpaired one, so a later plaintext connect or
    /// pairing is refused (`tls_required_pinned`) before any PIN goes out.
    pub fn drop_pairing(&self, hub_id: &str) -> std::io::Result<()> {
        let store = self.token_store();
        let kept = store
            .get(hub_id)
            .map(|c| dspi_link::credential::unpaired_record(&c))
            .unwrap_or_default();
        if kept.is_empty() {
            store.forget(hub_id)
        } else {
            store.put(hub_id, &kept)
        }
    }

    /// Forget hub: remove its record entirely (token, key, the TLS-only mark).
    /// The only way back to an unencrypted connection with a hub once pinned.
    pub fn forget_hub(&self, hub_id: &str) -> std::io::Result<()> {
        self.token_store().forget(hub_id)
    }

    /// The record a fresh pairing dials with: what [`Self::drop_pairing`]
    /// keeps of the stored one (no token; TLS-only for a hub once pinned).
    pub fn unpaired_credential(&self, hub_id: &str) -> Option<String> {
        self.token_store()
            .get(hub_id)
            .map(|c| dspi_link::credential::unpaired_record(&c))
            .filter(|c| !c.is_empty())
    }
}

// -- WP SECTM --
// Pairing and keys (docs/core-merge/PLAN-SEC.md 4.3, entry SECTM): the stored
// record per hub is the core's opaque credential (`v1` bare token, or `v2` with
// the hub key's pin), kept in the same 0600 token file; a pasted `dspi://pair`
// link; the hub key's fingerprint at the PIN prompt; pairing again offered only
// on a key or TLS refusal; and the "(unsecured)" badge of a session without TLS.

/// The shared words for a session without TLS: the core's notice rows
/// `dlink.unsecured` (the badge) and `dlink.unsecured.info` (the explanation).
/// dspi-tui does not link the core, so the embedder hands them over
/// ([`set_unsecured_words`]); these defaults are the table's English, and the
/// Terminal's tests hold them to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsecuredWords {
    /// `unsecured`: shown as " (unsecured)" after the connection's name.
    pub badge: String,
    /// The calm explanation (`dlink.unsecured.info` reason).
    pub info: String,
    /// What to do about it (`dlink.unsecured.info` action).
    pub action: String,
}

/// The notice keys: `dlink`, the owner's name for DSPi Link (never bare `link`).
pub const NOTICE_UNSECURED: &str = "dlink.unsecured";
pub const NOTICE_UNSECURED_INFO: &str = "dlink.unsecured.info";

impl Default for UnsecuredWords {
    fn default() -> Self {
        Self {
            badge: "unsecured".into(),
            info: "The connection to this hub is not encrypted. That is fine on a trusted local \
                   network or over a VPN, but on any other network someone could read or change \
                   what is sent, including the pairing PIN and settings."
                .into(),
            action: "If the hub supports encryption, turn TLS on in its settings, or turn its \
                     upstream compatibility mode off; this console then connects encrypted by \
                     itself."
                .into(),
        }
    }
}

static UNSECURED_WORDS: std::sync::OnceLock<UnsecuredWords> = std::sync::OnceLock::new();

/// Use the core's words for the badge and its explanation (once, at start).
pub fn set_unsecured_words(words: UnsecuredWords) {
    let _ = UNSECURED_WORDS.set(words);
}

/// The words in use.
pub fn unsecured_words() -> UnsecuredWords {
    UNSECURED_WORDS.get().cloned().unwrap_or_default()
}

/// " (unsecured)": what follows a connection's name when it has no TLS.
pub fn unsecured_suffix() -> String {
    format!(" ({})", unsecured_words().badge)
}

/// `text` with the badge after it when the connection is `unsecured`: known
/// not to be encrypted, and not to this machine (-- WP SECFIX --: a loopback
/// hub is "local", as on the web; [`LinkTarget::unsecured`],
/// `LinkSecurity::local`).
pub fn with_security(text: &str, unsecured: bool) -> String {
    if unsecured {
        format!("{text}{}", unsecured_suffix())
    } else {
        text.to_owned()
    }
}

/// `text` (a line that may carry the badge) for a terminal: the badge in red
/// when `colour`, else as it is. The words say it either way, so the meaning
/// never rests on the colour alone.
pub fn paint_unsecured(text: &str, colour: bool) -> String {
    let suffix = unsecured_suffix();
    if !colour || !text.contains(&suffix) {
        return text.to_owned();
    }
    text.replacen(&suffix, &format!("\u{1b}[31m{suffix}\u{1b}[0m"), 1)
}

/// Whether a stream may be coloured: a terminal, `NO_COLOR` unset, and not
/// `TERM=dumb`.
pub fn colour_allowed(is_terminal: bool) -> bool {
    is_terminal
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").map_or(true, |t| t != "dumb")
}

/// The core's error codes after which pairing again is offered, and only
/// these (PLAN-SEC 4.3): the hub presents another key, a pairing's key does
/// not check out, or TLS was refused where it is required. Every other
/// failure keeps the stored credential.
pub const REPAIR_CODES: [&str; 4] = [
    "hub_key_changed",
    "hub_key_unverified",
    "tls_refused",
    "tls_required",
];

/// Whether `code` is one of [`REPAIR_CODES`].
pub fn offers_repair(code: &str) -> bool {
    REPAIR_CODES.contains(&code)
}

/// The code of a [`LinkError`], as the core names it, for the four that
/// offer pairing again.
pub fn repair_code(e: &LinkError) -> Option<&'static str> {
    Some(match e {
        LinkError::HubKeyChanged => "hub_key_changed",
        LinkError::HubKeyUnverified => "hub_key_unverified",
        LinkError::TlsRefused { .. } => "tls_refused",
        LinkError::TlsRequired => "tls_required",
        _ => return None,
    })
}

// -- WP PINTLS --
/// The core's code for a hub pinned before that refused TLS: only forgetting
/// the hub helps (owner decision 2026-11-17), never pairing again.
pub const TLS_REQUIRED_PINNED: &str = "tls_required_pinned";

/// After [`TLS_REQUIRED_PINNED`]: ask whether to forget `hub` entirely, which
/// is the only way to reach it without encryption. Nobody to ask: say how, and
/// no. The question says plainly what forgetting gives up.
pub fn offer_forget(prompt: PinPrompt<'_>, hub: &str, address: &str) -> bool {
    let question = format!(
        "{hub}: this hub was paired with encryption before, so this terminal connects to it only \
         encrypted, and it refused encryption; nothing was sent, not even the PIN. Turn TLS on in \
         the hub's settings if you can. Forget the hub entirely? Its pairing and key are deleted, \
         and it can then be paired without encryption: on this network the PIN and settings \
         could be read or changed."
    );
    match prompt.confirm(&question) {
        Some(yes) => yes,
        None => {
            prompt.note(&format!(
                "{hub}: paired with encryption before, and it refused encryption; nothing was sent. \
                 To forget the hub (the only way to connect without encryption), run \
                 `{COMMAND} --link {address}` in a terminal and answer yes"
            ));
            false
        }
    }
}

/// Ask whether to forget this terminal's pairing with `hub` and pair again,
/// after `code` (one of [`REPAIR_CODES`]). Nobody to ask: say how, and no.
/// -- WP PINTLS -- Pairing again keeps a hub that was pinned TLS-only: it
/// never pairs unencrypted (only forgetting the hub allows that).
/// Re-pairing trusts whatever key the hub presents then, so the question
/// says to compare the fingerprint the PIN prompt shows with the hub's.
pub fn offer_repair(prompt: PinPrompt<'_>, hub: &str, address: &str, code: &str) -> bool {
    let why = match code {
        "hub_key_changed" => {
            "this hub presents a different key than the one this terminal paired with"
        }
        "hub_key_unverified" => "this hub's key could not be verified",
        "tls_refused" => "this hub refused the encrypted connection it should have",
        "tls_required" => "this hub only accepts encrypted connections",
        _ => return false,
    };
    let question = format!(
        "{hub}: {why}; nothing was sent to it. If the hub was reinstalled or its key or \
         settings were reset, pair again (compare the fingerprint shown with the PIN prompt \
         with the hub's own). It pairs only over an encrypted connection. Pair again?"
    );
    match prompt.confirm(&question) {
        Some(yes) => yes,
        None => {
            prompt.note(&format!(
                "{hub}: {why}. To pair again, run `{COMMAND} --link {address}` in a terminal and answer yes"
            ));
            false
        }
    }
}

/// The hub's name for the PIN prompt and the pairing question: its
/// fingerprint when the session has TLS, the badge when it has none, and
/// neither for a hub on this machine (`local`, -- WP SECFIX --).
pub fn pairing_label(hub: &str, encrypted: bool, local: bool, hub_key: &str) -> String {
    if !encrypted && !local {
        format!("{hub}{}", unsecured_suffix())
    } else if !encrypted || hub_key.is_empty() {
        hub.to_owned()
    } else {
        format!("{hub} (hub key {hub_key})")
    }
}

/// A `dspi://pair` link's confirmation (PLAN-SEC 4.3: a hint, never a
/// bypass). Its key has been checked by TLS by now; the person still says yes
/// to this hub and fingerprint before the link's PIN goes out.
pub fn confirm_pair_link(prompt: PinPrompt<'_>, hub: &str, hub_key: &str) -> Result<(), OpenError> {
    let question = format!(
        "Pair with {hub}, hub key {hub_key}, as the pairing link says? Check that the hub shows \
         the same key."
    );
    match prompt.confirm(&question) {
        Some(true) => Ok(()),
        Some(false) => Err(OpenError::NotConfirmed {
            hub: hub.to_owned(),
            reason: "pairing not confirmed; nothing was sent".into(),
        }),
        None => Err(OpenError::NotConfirmed {
            hub: hub.to_owned(),
            reason: format!(
                "a pairing link needs a yes from a person: run `{COMMAND}` with it in a terminal"
            ),
        }),
    }
}

/// A person's answer to a yes/no question.
pub fn parse_yes(line: &str) -> bool {
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Who answers [`Prompt::confirm`] for a plain function or closure prompt:
/// the embedder's (a question on the terminal), set once at start. Unset:
/// nobody.
type Confirmer = fn(&str) -> Option<bool>;
static CONFIRMER: std::sync::OnceLock<Confirmer> = std::sync::OnceLock::new();

/// Let function and closure prompts ask yes/no questions with `confirm`.
pub fn set_confirmer(confirm: Confirmer) {
    let _ = CONFIRMER.set(confirm);
}

impl LinkTarget {
    // -- WP SECTM --
    /// A pasted `dspi://pair?h=&a=&k=&p=` link (the hub's QR code), through the
    /// core's parser: the hub's id and address, its PIN, and the key it must
    /// present. The error never repeats the link (it holds the PIN).
    pub fn from_pair_link(text: &str) -> Result<Self, String> {
        let link = dspi_link::credential::parse_pair_link(text)
            .map_err(|e| format!("not a usable pairing link: {e}"))?;
        let hint = PairHint {
            credential: link.credential().encode(),
            hub_key: dspi_link::credential::pin_display(&link.key),
        };
        Ok(Self {
            host: link.host.clone(),
            port: link.port,
            device: None,
            pin: (!link.pin_code.is_empty()).then(|| link.pin_code.clone()),
            tokens: None,
            path: None,
            hub_id: Some(link.hub_id.to_lowercase()),
            find: None,
            tls_advertised: false,
            pair_link: Some(hint),
            encrypted: None,
        })
    }

    /// Keep `credential` for `hub_id` unless it is already the stored one.
    /// Returns whether it was written.
    pub fn store_credential(&self, hub_id: &str, credential: &str) -> std::io::Result<bool> {
        if credential.is_empty() || self.stored_token(hub_id).as_deref() == Some(credential) {
            return Ok(false);
        }
        self.store_token(hub_id, credential).map(|()| true)
    }

    // -- WP SECFIX --
    /// As reached, not encrypted, and not to this machine: the red
    /// " (unsecured)". A hub on loopback (`localhost`, `127.0.0.1`, `::1`) is
    /// "local", the same rule as the core's `LinkSecurity::local` and the web.
    pub fn unsecured(&self) -> bool {
        self.encrypted == Some(false) && !dspi_link::address::is_loopback_host(&self.host)
    }

    /// This target as reached: the security seen, and a pairing link's key and
    /// PIN spent (the pin is in the stored credential now).
    pub fn reached_with(&self, hub_id: &str, encrypted: bool) -> Self {
        Self {
            hub_id: (!hub_id.is_empty()).then(|| hub_id.to_owned()),
            encrypted: Some(encrypted),
            pin: if self.pair_link.is_some() {
                None
            } else {
                self.pin.clone()
            },
            pair_link: None,
            ..self.clone()
        }
    }
}

// -- WP TMCORE --
/// Pick a hub's device the way `--device` picks one: by the end of its serial
/// or part of its name; with no `wanted`, the only one there is. Each item is
/// `(serial, name)`; the answer is its index.
pub fn pick_link_device(
    items: &[(String, String)],
    wanted: Option<&str>,
) -> Result<usize, OpenError> {
    let indexed: Vec<(usize, &(String, String))> = items.iter().enumerate().collect();
    choose(
        &indexed,
        wanted,
        |(_, (serial, name))| vec![serial.as_str(), name.as_str()],
        |(_, (serial, name))| {
            printable(&if name.is_empty() {
                serial.clone()
            } else {
                format!("{serial} {name}")
            })
        },
        "devices on this hub",
        "--device <serial>",
    )
    .map(|(i, _)| *i)
}

/// Why a device could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("DSPi Link hub {address}: {source}")]
    Link { address: String, source: LinkError },
    /// The hub wants a PIN and nothing supplied one.
    #[error("{hub} needs pairing: {}", pin_hint(.pin_source))]
    NeedsPin { hub: String, pin_source: String },
    /// A PIN was tried and refused.
    #[error("{hub}: {reason}")]
    PinRefused { hub: String, reason: String },
    // -- WP SECTM --
    /// A pairing (a `dspi://pair` link's) that the person did not confirm, or
    /// that nobody was there to confirm. Nothing was sent to the hub.
    #[error("{hub}: {reason}")]
    NotConfirmed { hub: String, reason: String },
    /// More than one candidate, and nothing to tell them apart.
    #[error("{} {what} found ({}); select one with {flag}", .names.len(), .names.join(", "))]
    Ambiguous {
        what: &'static str,
        flag: &'static str,
        names: Vec<String>,
    },
    #[error("this build of {cmd} has no MIDI support", cmd = COMMAND)]
    NoMidi,
    // -- WP TM9 --
    #[error(
        "this build of {cmd} cannot look for hubs on the network; use --link HOST[:PORT]",
        cmd = COMMAND
    )]
    NoMdns,
    /// Looking for hubs did not work; the network, not the hubs.
    #[error("could not look for hubs: {0}")]
    Browse(String),
    /// Nothing on the network answers to this.
    #[error("{}", match .wanted {
        Some(w) => format!("no DSPi Link hub matching `{w}` found on this network"),
        None => "no DSPi Link hub found on this network".to_owned(),
    })]
    NoHub { wanted: Option<String> },
    /// Another hub answered at this address, which its DHCP lease can do. Nothing
    /// was sent to it.
    #[error("{address} is now {found}, not the hub this session was using")]
    WrongHub { address: String, found: String },
    #[error("the device reported an impossible channel map")]
    ChannelMap,
}

impl OpenError {
    /// Whether, and how soon, to try again. A hub's own errors say for
    /// themselves; a PIN is never retried, because a hub counts the failures,
    /// and what the device or the command line got wrong will not change.
    pub fn retry_advice(&self) -> RetryAdvice {
        match self {
            Self::NeedsPin { .. }
            | Self::PinRefused { .. }
            | Self::NotConfirmed { .. } // -- WP SECTM --
            | Self::NoMidi
            | Self::ChannelMap
            | Self::Ambiguous { .. }
            | Self::Transport(TransportError::Incompatible(_)) => RetryAdvice::Stop,
            Self::Link { source, .. } => source.retry_advice(),
            _ => RetryAdvice::Retry {
                min_delay: Duration::ZERO,
            },
        }
    }

    /// Retrying cannot help: a person has to do something first.
    pub fn is_final(&self) -> bool {
        self.retry_advice() == RetryAdvice::Stop
    }

    /// There is simply no such device (exit code 2), not a device in trouble.
    pub fn is_no_device(&self) -> bool {
        matches!(
            self,
            Self::Transport(TransportError::NotFound | TransportError::SerialNotFound(_))
                // -- WP TM9 --
                | Self::NoHub { .. }
        )
    }

    // -- WP TM9 --
    /// Nothing that is this hub answered at the address dialled, so the address
    /// may be the thing that is out of date.
    fn is_dial_failure(&self) -> bool {
        matches!(
            self,
            Self::WrongHub { .. }
                | Self::Link {
                    source: LinkError::Io(_) | LinkError::Timeout | LinkError::Handshake(_),
                    ..
                }
        )
    }
}

fn pin_hint(pin_source: &str) -> &'static str {
    if pin_source == "fixed" {
        concat!(
            "pass the hub's PIN with --pin, or run `",
            dspi_cmd::command!(),
            " --link` in a terminal"
        )
    } else {
        concat!(
            "open pairing on the hub, then pass the PIN it shows with --pin, or run `",
            dspi_cmd::command!(),
            " --link` in a terminal"
        )
    }
}

/// Whoever is opening the device: asked for a hub's PIN, told what went wrong
/// without stopping the connect. A closure or a function is a prompt that says
/// its warnings on standard error, which is right until the interface takes the
/// terminal.
pub trait Prompt {
    /// The PIN for the hub called `hub`, whose `pin_source` is `window` or
    /// `fixed`; `None` when there is nobody to ask.
    fn pin(&self, hub: &str, pin_source: &str) -> Option<String>;

    /// Something the person should know.
    fn warn(&self, message: &str) {
        eprintln!("{COMMAND}: warning: {message}");
    }

    // -- WP TM9 --
    /// Which of these hubs; `None` when there is nobody to ask, or no answer.
    fn pick(&self, _choices: &[String]) -> Option<usize> {
        None
    }

    // -- WP SECTM --
    /// A yes or no from the person (`question` ends in "?"); `None` when there
    /// is nobody to ask. Pairing by a link and pairing again ask this.
    fn confirm(&self, _question: &str) -> Option<bool> {
        None
    }

    // -- WP SECTM --
    /// Something the person should know that is not a problem (a hub key
    /// pinned for the first time).
    fn note(&self, message: &str) {
        eprintln!("{COMMAND}: {message}");
    }
}

impl<F: Fn(&str, &str) -> Option<String>> Prompt for F {
    fn pin(&self, hub: &str, pin_source: &str) -> Option<String> {
        self(hub, pin_source)
    }

    // -- WP SECTM -- the embedder's terminal question, when it set one
    fn confirm(&self, question: &str) -> Option<bool> {
        CONFIRMER.get().and_then(|ask| ask(question))
    }
}

/// How the opening functions take a [`Prompt`].
pub type PinPrompt<'a> = &'a dyn Prompt;

/// A prompt that has nobody to ask: a reconnect, or a check. -- WP SECTM --
/// a value, not a function, so it never answers [`Prompt::confirm`] either
/// (the interface may be drawing on the terminal).
#[allow(non_upper_case_globals)]
pub const no_prompt: NoPrompt = NoPrompt;

/// See [`no_prompt`].
#[derive(Debug, Clone, Copy)]
pub struct NoPrompt;

impl Prompt for NoPrompt {
    fn pin(&self, _hub: &str, _pin_source: &str) -> Option<String> {
        None
    }
}

/// An opened device with everything the interface starts from.
pub struct Connected {
    pub session: Session,
    pub state: DeviceState,
    pub ext: ExtState,
    // -- WP TM9 --
    /// Over Link, the target as it was reached: at the address that answered,
    /// knowing the hub's id. Keep this one, not the one that was asked for.
    pub link: Option<LinkTarget>,
}

// -- WP TM4 --
/// Probe `t` whatever its wire format, and guard it if that is not this
/// build's: the one place a link becomes a device, so no route around it. The
/// image is the probe's own, so a connect reads it once (two seconds over MIDI).
fn probed(
    mut t: Box<dyn Transport>,
) -> Result<(Box<dyn Transport>, Capabilities, Normalized), OpenError> {
    let (caps, image) = probe_any_with_image(t.as_mut()).map_err(|e| match e {
        // The session's words serve every app; this one knows its name.
        TransportError::Incompatible(why) => {
            TransportError::Incompatible(why.replace("this application", "DSPi Terminal"))
        }
        e => e,
    })?;
    if image.is_writable() {
        return Ok((t, caps, image));
    }
    let guarded = ReadOnlyTransport::new(
        t,
        BulkReads::Mapped,
        Some(&caps.firmware_version),
        Some(caps.wire_format),
    );
    Ok((Box::new(guarded), caps, image))
}

/// [`Target::open_session`] on a link already open.
pub fn session_on(t: Box<dyn Transport>) -> Result<Session, OpenError> {
    let (t, caps, _) = probed(t)?;
    Session::new(t, caps).ok_or(OpenError::ChannelMap)
}

/// [`Target::connect`] on a link already open.
pub fn connected_on(t: Box<dyn Transport>) -> Result<Connected, OpenError> {
    let (mut t, caps, image) = probed(t)?;
    // A stall or a refusal is already "absent" in the probe. What is left
    // is the link failing, which only matters if the link is gone: the
    // extension channel is never a reason to turn a device away.
    let ext = match ExtState::probe(t.as_mut()) {
        Ok(e) => e,
        Err(TransportError::Disconnected) => {
            return Err(TransportError::Disconnected.into());
        }
        Err(_) => ExtState::default(),
    };
    let session = Session::new(t, caps.clone()).ok_or(OpenError::ChannelMap)?;
    Ok(Connected {
        session,
        state: DeviceState::new(caps, image.packet),
        ext,
        link: None,
    })
}

/// [`Target::open_raw`] on a link already open. Fails closed: a device whose
/// wire format cannot be read may be anything, so it is not written to either.
/// Only a lost link is an error; `uconsole raw` is how such a device gets looked at.
pub fn guard_raw(mut t: Box<dyn Transport>) -> Result<Box<dyn Transport>, OpenError> {
    let wire = match compat::read_bulk_bytes(t.as_mut()) {
        Ok(image) => image.first().copied(),
        Err(WriteError::Transport(TransportError::Disconnected)) => {
            return Err(TransportError::Disconnected.into());
        }
        Err(_) => None,
    };
    if wire.map(u16::from) == Some(WIRE_FORMAT_VERSION) {
        return Ok(t);
    }
    let firmware = t
        .control_in_upto(op::REQ_GET_PLATFORM, 0, PlatformInfo::REQUEST_LEN as u16)
        .ok()
        .and_then(|reply| PlatformInfo::decode(&reply).ok())
        .map(|info| info.version);
    Ok(Box::new(ReadOnlyTransport::new(
        t,
        BulkReads::AsIs,
        firmware.as_ref(),
        wire,
    )))
}

/// One device a target can see, for `uconsole list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub serial: String,
    pub name: String,
    pub detail: String,
}

impl Target {
    /// Open the link. Nothing has been asked of the device yet.
    pub fn open(&self, pin: PinPrompt<'_>) -> Result<Box<dyn Transport>, OpenError> {
        let raw: Box<dyn Transport> = match self {
            Self::Usb { serial: Some(s) } => Box::new(UsbTransport::open_serial(s)?),
            Self::Usb { serial: None } => Box::new(UsbTransport::open_only()?),
            Self::Demo => Box::new(FakeDevice::demo()),
            Self::Midi { port } => open_midi(port.as_deref())?,
            Self::Link(l) => open_link(l, pin)?,
        };
        // -- WP PGWIRE -- the generation probe first, on the raw wire, for
        // every link (no probe over DSPi Link)
        Ok(dspi_transport::paged::open(raw)?)
    }

    /// Open and probe: enough for a one-shot command.
    pub fn open_session(&self, pin: PinPrompt<'_>) -> Result<Session, OpenError> {
        session_on(self.open(pin)?)
    }

    /// Open, probe and read the whole state: enough for the interface.
    pub fn connect(&self, pin: PinPrompt<'_>) -> Result<Connected, OpenError> {
        // -- WP TM9 --
        if let Self::Link(l) = self {
            let (t, reached) = open_link_reached(l, pin)?;
            let mut c = connected_on(t)?;
            c.link = Some(reached);
            return Ok(c);
        }
        connected_on(self.open(pin)?)
    }

    // -- WP TM4 --
    /// Open a link for `uconsole raw`, which asks for transfers by number and has
    /// no use for a probe: guarded against writes like everything else, but its
    /// bulk reads answer the device's own bytes.
    pub fn open_raw(&self, pin: PinPrompt<'_>) -> Result<Box<dyn Transport>, OpenError> {
        guard_raw(self.open(pin)?)
    }

    /// The same target, held to the device it found: a reconnect must not pick
    /// up whichever other DSPi happens to be there. A MIDI port's name is not
    /// stable across a replug (ALSA numbers its clients), so its filter stays.
    pub fn pinned(&self, serial: &str) -> Self {
        match self {
            Self::Usb { .. } if !serial.is_empty() => Self::Usb {
                serial: Some(serial.to_owned()),
            },
            Self::Link(l) if !serial.is_empty() => Self::Link(LinkTarget {
                device: Some(serial.to_owned()),
                ..l.clone()
            }),
            other => other.clone(),
        }
    }

    /// Whether the bus can be enumerated for more DSPis to switch to.
    pub fn is_usb(&self) -> bool {
        matches!(self, Self::Usb { .. })
    }

    /// Words for the title bar; USB, the default, says nothing.
    pub fn via(&self) -> String {
        match self {
            Self::Usb { .. } => String::new(),
            Self::Demo => "demo".into(),
            Self::Midi { .. } => "MIDI".into(),
            // -- WP SECTM -- a session known to run without TLS says so
            Self::Link(l) => with_security(&format!("Link {}", l.host), l.unsecured()),
        }
    }

    /// Whether every read is slow enough to want the light refresh rate.
    pub fn is_slow(&self) -> bool {
        matches!(self, Self::Midi { .. } | Self::Link(_))
    }

    /// What this target can see, without choosing among it. USB has its own
    /// listing in `uconsole list`.
    pub fn list(&self, pin: PinPrompt<'_>) -> Result<Vec<Listed>, OpenError> {
        match self {
            Self::Usb { .. } => Ok(dspi_transport::list_devices()?
                .iter()
                .map(|d| Listed {
                    serial: d.serial.clone(),
                    name: d.short_name(),
                    detail: format!("bus {} addr {}", d.bus_id, d.address),
                })
                .collect()),
            Self::Demo => {
                let d = FakeDevice::demo();
                Ok(vec![Listed {
                    serial: d.descriptor().serial.clone(),
                    name: d.descriptor().short_name(),
                    detail: "built-in demo device".into(),
                }])
            }
            Self::Midi { .. } => midi_names().map(|names| {
                names
                    .into_iter()
                    .map(|name| Listed {
                        serial: String::new(),
                        name: printable(&name),
                        detail: "USB MIDI".into(),
                    })
                    .collect()
            }),
            Self::Link(l) => {
                let client = link_client(l, pin)?;
                let list = client
                    .devices()
                    .into_iter()
                    .map(|d| Listed {
                        serial: printable(&d.serial),
                        name: printable(&d.name),
                        detail: printable(&d.state),
                    })
                    .collect();
                client.close();
                Ok(list)
            }
        }
    }
}

/// The ones of `items` that `wanted` means. Exact matches (of a key or a whole
/// name) beat partial ones, so a name that contains another still works.
fn matching<'a, T>(
    items: &'a [T],
    wanted: Option<&str>,
    keys: impl Fn(&T) -> Vec<&str>,
) -> Vec<&'a T> {
    let Some(w) = wanted else {
        return items.iter().collect();
    };
    let w = w.to_lowercase();
    let exact: Vec<&T> = items
        .iter()
        .filter(|i| keys(i).iter().any(|k| k.to_lowercase() == w))
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    items
        .iter()
        .filter(|i| keys(i).iter().any(|k| k.to_lowercase().contains(&w)))
        .collect()
}

/// The one of `items` that `wanted` means, by [`matching`].
fn choose<'a, T>(
    items: &'a [T],
    wanted: Option<&str>,
    keys: impl Fn(&T) -> Vec<&str>,
    label: impl Fn(&T) -> String,
    what: &'static str,
    flag: &'static str,
) -> Result<&'a T, OpenError> {
    match matching(items, wanted, keys).as_slice() {
        [] => Err(match wanted {
            Some(w) => TransportError::SerialNotFound(w.to_owned()),
            None => TransportError::NotFound,
        }
        .into()),
        [one] => Ok(one),
        many => Err(OpenError::Ambiguous {
            what,
            flag,
            names: many.iter().map(|i| label(i)).collect(),
        }),
    }
}

#[cfg(feature = "midi")]
fn open_midi(port: Option<&str>) -> Result<Box<dyn Transport>, OpenError> {
    use dspi_transport::midi::midir_port;
    let ports = midir_port::list_ports();
    let chosen = choose(
        &ports,
        port,
        |p| vec![p.name.as_str()],
        |p| printable(&p.name),
        "DSPi MIDI ports",
        "--midi <name>",
    )?;
    Ok(Box::new(midir_port::open(chosen)?))
}

#[cfg(not(feature = "midi"))]
fn open_midi(_port: Option<&str>) -> Result<Box<dyn Transport>, OpenError> {
    Err(OpenError::NoMidi)
}

#[cfg(feature = "midi")]
fn midi_names() -> Result<Vec<String>, OpenError> {
    Ok(dspi_transport::midi::midir_port::list_ports()
        .into_iter()
        .map(|p| p.name)
        .collect())
}

#[cfg(not(feature = "midi"))]
fn midi_names() -> Result<Vec<String>, OpenError> {
    Err(OpenError::NoMidi)
}

/// The DSPi MIDI ports this host can see, for `uconsole doctor`. `None` in a build
/// without MIDI.
pub fn midi_ports() -> Option<Vec<String>> {
    midi_names().ok()
}

fn link_error(l: &LinkTarget, source: LinkError) -> OpenError {
    OpenError::Link {
        address: l.address(),
        source,
    }
}

/// A session with the hub, authenticated as far as a stored token, `--pin` or
/// the prompt takes it. The token is stored against the hub's id and looked up
/// after the hello, so a hub never sees another hub's token. Only a refusal of
/// the token itself forgets it: any other failure is an error, and it stays.
///
/// -- WP SECTM -- What is stored is the core's credential (PLAN-SEC 4.3): TLS
/// first, a pinned hub over TLS only and its key checked before the token goes
/// out; a `v1` token is upgraded to `v2` the first time TLS carries it. After
/// `hub_key_changed`, `hub_key_unverified`, `tls_refused` or `tls_required`,
/// and only then, the person is offered to forget the pairing and pair again.
///
/// -- WP PINTLS -- Pairing again keeps a hub that was pinned TLS-only (its
/// record is the core's unpaired one): it pairs over TLS and is pinned anew,
/// or fails with `tls_required_pinned` before any PIN goes out; the person is
/// then offered to forget the hub, the only way to an unencrypted pairing.
pub fn link_client(l: &LinkTarget, pin: PinPrompt<'_>) -> Result<LinkClient, OpenError> {
    let mut hub_id = l.hub_id.clone();
    let mut fresh = false;
    let mut repaired = false;
    loop {
        let (address, source) = match link_client_once(l, pin, &mut hub_id, fresh) {
            Err(OpenError::Link { address, source }) => (address, source),
            other => return other,
        };
        let hub = hub_id.clone().unwrap_or_else(|| l.address());
        let id = hub_id.clone();
        if matches!(source, LinkError::TlsRequiredPinned) {
            let Some(id) = id.filter(|_| l.pair_link.is_none() || fresh) else {
                return Err(OpenError::Link { address, source });
            };
            if !offer_forget(pin, &hub, &l.address()) {
                return Err(OpenError::Link { address, source });
            }
            if let Err(e) = l.forget_hub(&id) {
                pin.warn(&format!("could not forget {hub} ({e})"));
                return Err(OpenError::Link { address, source });
            }
            // Forgotten: a fresh pairing with no record, as for a new hub
            fresh = true;
            repaired = true;
            continue;
        }
        let code = repair_code(&source).filter(|_| l.pair_link.is_none() && !repaired);
        if !code.is_some_and(|c| offer_repair(pin, &hub, &l.address(), c)) {
            return Err(OpenError::Link { address, source });
        }
        if let Some(id) = &id
            && let Err(e) = l.drop_pairing(id)
        {
            pin.warn(&format!("could not drop the pairing with {hub} ({e})"));
        }
        fresh = true;
        repaired = true;
    }
}

/// [`link_client`]'s one try. `hub_id` is filled in once a hello names the
/// hub. `fresh`: pairing again, so no stored token and no link's key; a hub
/// once pinned is still TLS-only (-- WP PINTLS --).
fn link_client_once(
    l: &LinkTarget,
    pin: PinPrompt<'_>,
    hub_id: &mut Option<String>,
    fresh: bool,
) -> Result<LinkClient, OpenError> {
    let store = l.token_store();
    let pair_link = l.pair_link.as_ref().filter(|_| !fresh);
    // The stored record, by the hub's id; never for a pairing link (whose PIN
    // is the point) or a fresh pairing.
    // -- WP PINTLS -- fresh: what is kept after dropping the pairing
    let stored = |id: &str| -> Option<Credential> {
        if fresh {
            return l.unpaired_credential(id).map(|c| Credential::parse(&c));
        }
        pair_link
            .is_none()
            .then(|| store.get(id))
            .flatten()
            .map(|c| Credential::parse(&c))
    };
    // What TLS must meet: the link's key, or the stored pin of a known hub.
    let known = match (pair_link, hub_id.as_deref()) {
        (Some(hint), _) => Credential::parse(&hint.credential),
        (None, Some(id)) => stored(id).unwrap_or_default(),
        (None, None) => Credential::default(),
    };
    let pinned_known = hub_id.is_some() || pair_link.is_some();
    let (client, auth, credential) = match dial(l, &store, &known, pinned_known) {
        Ok(Dialled::Done(client, auth, sent)) => (
            client,
            auth,
            Credential {
                token: sent,
                ..known
            },
        ),
        // An unknown hub whose id has a pinned record here: again, pinned
        Ok(Dialled::Pinned(id, credential)) => {
            *hub_id = Some(id);
            match dial(l, &store, &credential, true) {
                Ok(Dialled::Done(client, auth, _)) => (client, auth, credential),
                Ok(Dialled::Pinned(..)) => {
                    return Err(link_error(
                        l,
                        LinkError::Protocol("the hub's id changed".into()),
                    ));
                }
                Err(e) => return Err(e),
            }
        }
        Err(OpenError::Link {
            source: LinkError::HubKeyChanged,
            ..
        }) if pair_link.is_some() => {
            // A link whose `k=` is not the key the hub presents: no PIN went out
            return Err(link_error(l, LinkError::HubKeyUnverified));
        }
        Err(e) => return Err(e),
    };
    if hub_id.is_none() && !client.hello().hub.id.is_empty() {
        *hub_id = Some(client.hello().hub.id.clone());
    }
    // -- WP TM9 --
    if l.is_other_hub(&client.hello().hub.id) {
        let found = hub_name(&client, l);
        client.close();
        return Err(OpenError::WrongHub {
            address: l.address(),
            found,
        });
    }
    let id = client.hello().hub.id.clone();
    let tls = client.tls();
    let pin_source = match auth {
        AuthState::Ready => {
            // A token TLS carried: pinned from now on (a v1 upgraded, a rotation)
            learn_key(l, &client, &credential, pin);
            return Ok(client);
        }
        AuthState::NeedsPairing { pin_source } => pin_source,
        AuthState::TokenRefused { pin_source } => {
            // -- WP PINTLS -- a hub once pinned keeps its TLS-only mark
            if let Err(e) = l.drop_pairing(&id) {
                pin.warn(&format!(
                    "could not forget the refused pairing token for {} ({e}); it will be tried again next time",
                    hub_name(&client, l)
                ));
            }
            pin_source
        }
    };
    let saver = Arc::new(TokenSaver {
        store: store.clone(),
        tls,
        failed: Mutex::default(),
    });
    client.set_events(Some(saver.clone()));
    let paired = pair(l, &client, &pin_source, pin, pair_link);
    for e in saver.take_failures() {
        pin.warn(&format!(
            "could not save the pairing token for {} ({e}); it will ask for a PIN next time",
            hub_name(&client, l)
        ));
    }
    if let Err(e) = paired {
        client.close();
        return Err(e);
    }
    Ok(client)
}

/// What a [`dial`] came to.
enum Dialled {
    /// With the token that went out ("" for none).
    Done(LinkClient, AuthState, String),
    /// The hello named a hub with a pinned record here, and the dial was not
    /// pinned: nothing was sent; dial again with it.
    Pinned(String, Credential),
}

/// One connect, TLS first, with `credential`'s pin. `pinned_known`: the hub's id
/// is known, so a record looked up by the hello's id is this credential.
fn dial(
    l: &LinkTarget,
    store: &TokenStore,
    credential: &Credential,
    pinned_known: bool,
) -> Result<Dialled, OpenError> {
    let lookup = store.clone();
    let asked = l.clone();
    let token = credential.token.clone();
    let pinned_elsewhere: Arc<Mutex<Option<(String, Credential)>>> = Arc::default();
    let found = Arc::clone(&pinned_elsewhere);
    let sent: Arc<Mutex<String>> = Arc::default();
    let went_out = Arc::clone(&sent);
    // -- WP TM9 --
    // An address that went to another hub gets no token, not even its own.
    let cfg = SessionConfig {
        token_for: Some(Arc::new(move |hub_id: &str| {
            if asked.is_other_hub(hub_id) {
                return None;
            }
            let give = |t: String| {
                (!t.is_empty()).then(|| {
                    went_out
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .clone_from(&t);
                    t
                })
            };
            if pinned_known {
                return give(token.clone());
            }
            // -- WP SECTM -- an unpinned dial: only a v1 token may go out; a
            // pinned one waits for a dial that checks its key
            let stored = Credential::parse(&lookup.get(hub_id)?);
            if stored.tls_only() {
                // -- WP PINTLS -- a hub pinned before too
                *found.lock().unwrap_or_else(PoisonError::into_inner) =
                    Some((hub_id.to_owned(), stored));
                return None;
            }
            give(stored.token)
        })),
        ..SessionConfig::default()
    };
    let opts = ConnectOptions {
        client_name: "DSPi Terminal".into(),
        app: COMMAND.into(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        token: None,
    };
    let tls = TlsOptions {
        pin: credential.pin,
        pin_next: credential.next,
        advertised: l.tls_advertised,
        plaintext_known: false,
        pinned_before: credential.pinned_before, // -- WP PINTLS --
    };
    let (client, auth) = LinkClient::connect_tls(
        &TcpConnector::default(),
        &l.ws_url(),
        &opts,
        None,
        &cfg,
        &tls,
    )
    .map_err(|e| link_error(l, e))?;
    let mut elsewhere = pinned_elsewhere
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    // -- WP PINTLS -- (review F3) a hub dialled by address that asks for no PIN
    // never reaches the token lookup above. Its record still rules: a hub
    // pinned, or pinned before, is dialled again with it (TLS only, its key
    // checked), so no answer at that address can open an unencrypted or
    // unchecked session in the hub's name.
    if elsewhere.is_none() && !pinned_known {
        let id = client.hello().hub.id.clone();
        if !id.is_empty() && !l.is_other_hub(&id) {
            elsewhere = store
                .get(&id)
                .map(|c| Credential::parse(&c))
                .filter(Credential::tls_only)
                .map(|c| (id, c));
        }
    }
    if let Some((id, credential)) = elsewhere {
        client.close();
        return Ok(Dialled::Pinned(id, credential));
    }
    let sent = std::mem::take(&mut *sent.lock().unwrap_or_else(PoisonError::into_inner));
    Ok(Dialled::Done(client, auth, sent))
}

/// A session that authenticated with `credential`'s token: keep the key TLS
/// met as its pin (a `v1` token's first TLS, a proven rotation), and say so
/// once.
fn learn_key(l: &LinkTarget, client: &LinkClient, credential: &Credential, prompt: PinPrompt<'_>) {
    let Some(seen) = client.tls() else {
        return;
    };
    if credential.token.is_empty() {
        return;
    }
    let id = client.hello().hub.id.clone();
    let upgraded = Credential {
        token: credential.token.clone(),
        pin: Some(seen.key),
        next: seen.next,
        ..Credential::default()
    };
    match l.store_credential(&id, &upgraded.encode()) {
        Ok(true) if credential.pin != upgraded.pin => prompt.note(&format!(
            "encrypted connection set up with {}; hub key {}",
            hub_name(client, l),
            dspi_link::credential::pin_display(&seen.key)
        )),
        Ok(_) => {}
        Err(e) => prompt.warn(&format!(
            "could not save the hub key for {} ({e}); it is checked again next time",
            hub_name(client, l)
        )),
    }
}

/// Stores a token the moment the hub issues it, which is also how the `ok` that
/// arrives after [`LinkClient::pair`] has timed out is heard. It runs on the
/// client's reader thread, so failures wait here for whoever can report them.
struct TokenSaver {
    store: TokenStore,
    // -- WP SECTM -- the TLS the pairing ran over: the token is stored pinned
    tls: Option<dspi_link::TlsSeen>,
    failed: Mutex<Vec<String>>,
}

impl TokenSaver {
    fn take_failures(&self) -> Vec<String> {
        std::mem::take(&mut *self.failed.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl LinkEvents for TokenSaver {
    fn token_issued(&self, hub_id: &str, token: &str) {
        let credential = Credential {
            token: token.to_owned(),
            pin: self.tls.map(|t| t.key),
            next: self.tls.and_then(|t| t.next),
            ..Credential::default()
        };
        if let Err(e) = self.store.put(hub_id, &credential.encode()) {
            self.failed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(e.to_string());
        }
    }
}

/// Pair with the PIN, and the token goes to [`TokenSaver`]. After a timeout the
/// session stays open for [`PAIR_GRACE`], in case the answer is only late.
/// -- WP SECTM -- the prompt names the hub key (or "unsecured"); a pairing
/// link's PIN goes out only after the person's yes.
fn pair(
    l: &LinkTarget,
    client: &LinkClient,
    pin_source: &str,
    pin: PinPrompt<'_>,
    pair_link: Option<&PairHint>,
) -> Result<(), OpenError> {
    let hub = hub_name(client, l);
    let tls = client.tls();
    let key = tls
        .map(|t| dspi_link::credential::pin_display(&t.key))
        .unwrap_or_default();
    let label = pairing_label(
        &hub,
        tls.is_some(),
        dspi_link::address::is_loopback_host(&l.host),
        &key,
    );
    let given = match pair_link {
        Some(hint) => {
            confirm_pair_link(pin, &hub, &hint.hub_key)?;
            l.pin.clone()
        }
        None => l.pin.clone(),
    };
    let Some(code) = given.or_else(|| pin.pin(&label, pin_source)) else {
        return Err(OpenError::NeedsPin {
            hub,
            pin_source: pin_source.to_owned(),
        });
    };
    match client.pair(&code, LINK_ROLE) {
        Ok(_) => Ok(()),
        Err(e @ LinkError::WrongPin { .. }) => Err(OpenError::PinRefused {
            hub,
            reason: e.to_string(),
        }),
        Err(LinkError::Timeout) => {
            let deadline = Instant::now().checked_add(PAIR_GRACE);
            while !client.is_ready() && deadline.is_some_and(|d| Instant::now() < d) {
                std::thread::sleep(Duration::from_millis(50));
            }
            if !client.is_ready() {
                return Err(link_error(l, LinkError::Timeout));
            }
            // A late `ok` leaves the device list to us.
            client.refresh_devices().map_err(|e| link_error(l, e))?;
            Ok(())
        }
        Err(e) => Err(link_error(l, e)),
    }
}

fn hub_name(client: &LinkClient, l: &LinkTarget) -> String {
    let name = &client.hello().hub.name;
    if name.is_empty() {
        l.address()
    } else {
        printable(name)
    }
}

fn open_link(l: &LinkTarget, pin: PinPrompt<'_>) -> Result<Box<dyn Transport>, OpenError> {
    open_link_reached(l, pin).map(|(t, _)| t)
}

/// The link, and `l` knowing the id of the hub it reached.
fn open_link_reached(
    l: &LinkTarget,
    pin: PinPrompt<'_>,
) -> Result<(Box<dyn Transport>, LinkTarget), OpenError> {
    let client = link_client(l, pin)?;
    let id = client.hello().hub.id.clone();
    // -- WP SECTM -- with what the session runs over
    let reached = l.reached_with(&id, client.tls().is_some());
    match bind_link_device(l, &client) {
        Ok(t) => Ok((t, reached)),
        Err(e) => {
            client.close();
            Err(e)
        }
    }
}

fn bind_link_device(l: &LinkTarget, client: &LinkClient) -> Result<Box<dyn Transport>, OpenError> {
    let all = client.devices();
    let online: Vec<_> = all.iter().filter(|d| d.state != "offline").collect();
    let chosen = choose(
        &online,
        l.device.as_deref(),
        |d| vec![d.serial.as_str(), d.name.as_str()],
        |d| {
            printable(&if d.name.is_empty() {
                d.serial.clone()
            } else {
                format!("{} {}", d.serial, d.name)
            })
        },
        "devices on this hub",
        "--device <serial>",
    )?;
    let handle = u8::try_from(chosen.handle).map_err(|_| {
        link_error(
            l,
            LinkError::Protocol(format!("device handle {} is out of range", chosen.handle)),
        )
    })?;
    // Its clone of the client is the last one once `open_link` returns, so the
    // hub session ends with the transport.
    Ok(Box::new(LinkTransport::new(
        client.clone(),
        handle,
        &chosen.serial,
    )))
}

/// Pairing tokens, by hub id, in `~/.config/uconsole/link-tokens.toml`. It is kept
/// apart from `config.toml` so settings can be shared without the secrets, and
/// is readable only by its owner.
#[derive(Debug, Clone)]
struct TokenStore {
    path: Option<PathBuf>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TokenFile {
    #[serde(default)]
    tokens: BTreeMap<String, String>,
}

impl TokenStore {
    fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    fn default_location() -> Self {
        Self {
            path: dirs::config_dir().map(|d| d.join(COMMAND).join("link-tokens.toml")),
        }
    }

    fn load(path: &Path) -> TokenFile {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn get(&self, hub_id: &str) -> Option<String> {
        let mut file = Self::load(self.path.as_deref()?);
        file.tokens.remove(hub_id)
    }

    fn put(&self, hub_id: &str, token: &str) -> std::io::Result<()> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        let mut file = Self::load(path);
        file.tokens.insert(hub_id.to_owned(), token.to_owned());
        Self::store(path, &file)
    }

    /// Drop a token the hub refused. Having none to drop is fine.
    fn forget(&self, hub_id: &str) -> std::io::Result<()> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        let mut file = Self::load(path);
        if file.tokens.remove(hub_id).is_none() {
            return Ok(());
        }
        Self::store(path, &file)
    }

    fn store(path: &Path, file: &TokenFile) -> std::io::Result<()> {
        let text = toml::to_string(file).map_err(std::io::Error::other)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Whole or not at all, and never world-readable, even for a moment.
        let tmp = path.with_extension("toml.new");
        write_private(&tmp, text.as_bytes())?;
        std::fs::rename(&tmp, path)
    }
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    // A leftover from a crash keeps the mode it has, whatever is asked here.
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

/// What stands in for a device that has gone: every request fails at once with
/// no I/O, and the real transport has been let go, so its USB interface is free
/// for the device's return and its hub session is closed.
pub struct Offline {
    descriptor: DeviceDescriptor,
}

impl Offline {
    pub fn new(serial: &str) -> Self {
        Self {
            descriptor: DeviceDescriptor {
                serial: serial.to_owned(),
                bus_id: "offline".into(),
                address: 0,
            },
        }
    }
}

impl Transport for Offline {
    fn control_in(&mut self, _: u8, _: u16, _: u16) -> dspi_transport::Result<Vec<u8>> {
        Err(TransportError::Disconnected)
    }

    fn control_out(&mut self, _: u8, _: u16, _: &[u8]) -> dspi_transport::Result<()> {
        Err(TransportError::Disconnected)
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }
}

/// The words for a change that is not ours (§1.2), given the link it came over.
///
/// [`Source::describe`] says what the source byte says. On a link where that
/// byte is not news (a foreign `HOST_SET` or `BULK_SET` on USB, or anything from
/// another client over Link) these words say what is known instead.
pub fn foreign_words(kind: TransportKind, source: Source, origin: Option<u16>) -> &'static str {
    match (kind, source) {
        (TransportKind::Link, _) if origin.is_some_and(|o| o != 0) => {
            "changed by another Link client"
        }
        (TransportKind::Midi, Source::HostSet) => "changed over USB",
        (_, Source::HostSet | Source::BulkSet) => "changed elsewhere",
        (_, other) => other.describe(),
    }
}

/// The words for a change's source, ours included, by the scalar rule.
pub fn attribution(
    kind: TransportKind,
    session: Option<u16>,
    source: Source,
    origin: Option<u16>,
) -> &'static str {
    if dspi_transport::is_own_write(kind, source.raw(), origin, session, false) {
        "changed by this host"
    } else {
        foreign_words(kind, source, origin)
    }
}

/// Whether a `BULK_INVALIDATED` the echo filter did not claim is worth a line
/// saying someone else changed the settings. It is re-read either way (§1.2);
/// the line has to be true.
///
/// So not when it may be our own doing: a scalar write too big for a
/// `PARAM_CHANGED` falls back to an invalidation on our own path
/// (`notification_protocol_v2_spec.md` 3.4), and `UNKNOWN` is a merge that can
/// include our writes. A preset load and a factory reset have their own
/// messages.
pub fn bulk_is_news(
    kind: TransportKind,
    session: Option<u16>,
    source: Source,
    origin: Option<u16>,
) -> bool {
    !matches!(source, Source::Preset | Source::Factory | Source::Unknown)
        && !dspi_transport::is_own_write(kind, source.raw(), origin, session, false)
}

/// The extension channel in a few words for the title bar: empty on stock
/// firmware.
pub fn ext_label(ext: &ExtState) -> String {
    if ext.supported.is_empty() {
        return String::new();
    }
    let names: Vec<String> = ext
        .supported
        .iter()
        .map(|&id| match id {
            ext_ids::EXT_CROSSFEED_PAIR_MAP => "xfeed".to_owned(),
            ext_ids::EXT_CORE_VREG => "vreg".to_owned(),
            ext_ids::EXT_NOTIFY_FEATURES => "notify".to_owned(),
            other => format!("0x{other:04X}"),
        })
        .collect();
    format!("ext {}", names.join(" "))
}

/// One look at the reconnect in progress.
pub enum Reconnect {
    /// Not back yet.
    Waiting,
    /// It is back: adopt this.
    Connected(Box<Connected>),
    /// Stop trying: a person has to do something first.
    GaveUp(OpenError),
}

#[cfg(test)]
type Opener = dyn Fn() -> Result<Connected, OpenError> + Send + Sync;
// -- WP TMCORE --
/// Opens a target for the interface, in place of [`Target::connect`]: the
/// Terminal's opener puts the device on a console core first.
/// -- WP TMCOREF -- with the prompt to warn on (a reconnect's collects them).
pub type TargetOpener =
    dyn Fn(&Target, PinPrompt<'_>) -> Result<Connected, OpenError> + Send + Sync;

/// One attempt; a hub found at another address is dialled there.
type Dialer = dyn Fn(Option<&BrowsedHub>) -> Result<Connected, OpenError> + Send + Sync;

/// A reconnect's [`Prompt`]: nobody to ask, and its warnings are kept for the
/// interface, because a thread must not write to the terminal it draws on.
struct Collect(Arc<Mutex<Vec<String>>>);

impl Prompt for Collect {
    fn pin(&self, _hub: &str, _pin_source: &str) -> Option<String> {
        None
    }

    fn warn(&self, message: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.to_owned());
    }

    // -- WP SECTM --
    fn note(&self, message: &str) {
        self.warn(message);
    }
}

/// Tries to get a lost device back, on a thread of its own, at a sensible pace.
pub struct Reconnector {
    // -- WP TM9 --
    open: Arc<Dialer>,
    /// Looks for a lost hub at new addresses.
    finder: Option<HubFinder>,
    /// Failures in a row, for the back-off.
    attempt: u32,
    /// A hub's errors set the pace; a local look stays at [`RETRY_LOCAL`].
    follows_advice: bool,
    next_try: Option<Instant>,
    running: Option<Receiver<Result<Connected, OpenError>>>,
    warnings: Arc<Mutex<Vec<String>>>,
}

impl Reconnector {
    /// For `target`: nobody is asked for a PIN, since nobody may be looking.
    pub fn new(target: Target) -> Self {
        Self::browsing(target, browse_source().ok())
    }

    // -- WP TM9 --
    /// [`Reconnector::new`], looking for a lost hub on `source`. Without a
    /// source or a hub id it only ever dials the address it has.
    fn browsing(target: Target, source: Option<Arc<dyn BrowseSource>>) -> Self {
        let follows_advice = matches!(target, Target::Link(_));
        let finder = match (&target, source) {
            (Target::Link(l), Some(source)) => l
                .hub_id
                .clone()
                .map(|id| HubFinder::new(id, source, l.ws_url())),
            _ => None,
        };
        let warnings = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&warnings);
        let open = move |moved: Option<&BrowsedHub>| {
            let prompt = Collect(Arc::clone(&sink));
            match (&target, moved) {
                (Target::Link(l), Some(hub)) => Target::Link(l.at(hub)).connect(&prompt),
                _ => target.connect(&prompt),
            }
        };
        Self {
            warnings,
            finder,
            ..Self::with_dialer(Arc::new(open), follows_advice)
        }
    }

    // -- WP TMCORE --
    /// [`Reconnector::new`], each attempt made by `open` (the Terminal opens
    /// the device on a console core and routes the session through it). A
    /// hub that moved is dialled at its new address, as `new` does.
    pub fn with_open(target: Target, open: Arc<TargetOpener>) -> Self {
        let source = browse_source().ok();
        let follows_advice = matches!(target, Target::Link(_));
        let finder = match (&target, source) {
            (Target::Link(l), Some(source)) => l
                .hub_id
                .clone()
                .map(|id| HubFinder::new(id, source, l.ws_url())),
            _ => None,
        };
        let warnings = Arc::new(Mutex::new(Vec::new()));
        // -- WP TMCOREF -- (review tmcore N3) the attempt's warnings are kept,
        // as `new` keeps them
        let sink = Arc::clone(&warnings);
        let dial = move |moved: Option<&BrowsedHub>| {
            let prompt = Collect(Arc::clone(&sink));
            match (&target, moved) {
                (Target::Link(l), Some(hub)) => open(&Target::Link(l.at(hub)), &prompt),
                _ => open(&target, &prompt),
            }
        };
        Self {
            warnings,
            finder,
            ..Self::with_dialer(Arc::new(dial), follows_advice)
        }
    }

    /// What the attempts so far had to say, each once. An attempt says it
    /// before its result arrives, so after [`Reconnect::Connected`] or
    /// [`Reconnect::GaveUp`] this holds everything; take it then.
    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.warnings.lock().unwrap_or_else(PoisonError::into_inner))
    }

    #[cfg(test)]
    fn with_opener(open: Arc<Opener>, follows_advice: bool) -> Self {
        Self::with_dialer(Arc::new(move |_| open()), follows_advice)
    }

    fn with_dialer(open: Arc<Dialer>, follows_advice: bool) -> Self {
        Self {
            open,
            finder: None,
            attempt: 0,
            follows_advice,
            next_try: None,
            running: None,
            warnings: Arc::default(),
        }
    }

    // -- WP TM8 --
    /// Whether an attempt is under way, for a test to catch one.
    #[cfg(test)]
    pub(crate) fn dialling(&self) -> bool {
        self.running.is_some()
    }

    /// Call as often as you like: it never blocks. The first attempt goes out
    /// at once, later ones as the back-off allows.
    pub fn poll(&mut self, now: Instant) -> Reconnect {
        if let Some(rx) = &self.running {
            match rx.try_recv() {
                Err(TryRecvError::Empty) => return Reconnect::Waiting,
                Ok(Ok(c)) => {
                    self.running = None;
                    self.attempt = 0;
                    self.next_try = None;
                    // -- WP TM9 --
                    self.stop_browsing();
                    return Reconnect::Connected(Box::new(c));
                }
                Ok(Err(e)) => {
                    self.running = None;
                    let Some(delay) = e.retry_advice().delay(self.attempt) else {
                        // -- WP TM9 --
                        self.stop_browsing();
                        return Reconnect::GaveUp(e);
                    };
                    // -- WP TM9 --
                    if let Some(finder) = &mut self.finder {
                        finder.attempt_ended(e.is_dial_failure());
                    }
                    self.back_off(now, delay);
                    return Reconnect::Waiting;
                }
                // The thread died: try again later.
                Err(TryRecvError::Disconnected) => {
                    self.running = None;
                    self.back_off(now, RETRY_LOCAL);
                    return Reconnect::Waiting;
                }
            }
        }
        // -- WP TM9 --
        // A hub that turned up elsewhere is the news the back-off was waiting for.
        let moved = self.finder.as_ref().is_some_and(HubFinder::has_moved);
        if self.next_try.is_some_and(|t| now < t) && !moved {
            return Reconnect::Waiting;
        }
        let found = self.finder.as_mut().and_then(HubFinder::next_dial);
        let (tx, rx) = mpsc::channel();
        let open = Arc::clone(&self.open);
        let started = std::thread::Builder::new()
            .name("dspi-reconnect".into())
            .spawn(move || {
                // The receiver may be gone: a late result then closes itself.
                let _ = tx.send(open(found.as_ref()));
            });
        match started {
            Ok(_) => self.running = Some(rx),
            Err(_) => self.back_off(now, RETRY_LOCAL),
        }
        Reconnect::Waiting
    }

    fn back_off(&mut self, now: Instant, delay: Duration) {
        let delay = if self.follows_advice {
            delay
        } else {
            RETRY_LOCAL
        };
        self.next_try = now.checked_add(delay);
        self.attempt = self.attempt.saturating_add(1);
    }

    // -- WP TM9 --
    fn stop_browsing(&mut self) {
        if let Some(finder) = &mut self.finder {
            finder.stop();
        }
    }
}

// -- WP TM9 --
/// How long a look at the network lasts. Long enough for the hubs on a LAN to
/// answer one query (RFC 6762 lets them wait half a second), so a person does
/// not get a list with one missing.
pub const SCAN_WINDOW: Duration = Duration::from_secs(2);
/// How often [`scan_hubs`] looks at what has been heard.
const SCAN_TICK: Duration = Duration::from_millis(50);

/// The network's hubs, as this build can see them. Tests in this crate never
/// get the real one, so none of them depends on multicast.
#[cfg(all(feature = "mdns", not(test)))]
pub fn browse_source() -> Result<Arc<dyn BrowseSource>, OpenError> {
    // -- WP DISC2 -- Windows' own DNS-SD on Windows
    Ok(Arc::new(dspi_link::discovery::LanSource))
}

/// The network's hubs, as this build can see them: not at all.
#[cfg(not(all(feature = "mdns", not(test))))]
pub fn browse_source() -> Result<Arc<dyn BrowseSource>, OpenError> {
    Err(OpenError::NoMdns)
}

/// Listen to `source` for `window`, or until `enough` says so, and say which
/// hubs were heard, sorted by name. Blocks: this is for before the interface
/// starts. An error only when browsing failed and nothing was heard.
pub fn scan_hubs(
    source: Arc<dyn BrowseSource>,
    window: Duration,
    enough: impl Fn(&[BrowsedHub]) -> bool,
) -> Result<Vec<BrowsedHub>, OpenError> {
    let browser = HubBrowser::start(source, None, BrowseOptions::default())
        .map_err(|e| OpenError::Browse(printable(&e.to_string())))?;
    let deadline = Instant::now().checked_add(window);
    loop {
        let heard = browser.snapshot();
        let over = deadline.is_none_or(|d| Instant::now() >= d);
        if over || enough(&heard.hubs) {
            return match heard.error {
                Some(why) if heard.hubs.is_empty() => Err(OpenError::Browse(printable(&why))),
                _ => Ok(heard.hubs),
            };
        }
        std::thread::sleep(SCAN_TICK);
    }
}

/// A hub's address as `--link` takes it.
pub fn hub_address(hub: &BrowsedHub) -> String {
    if hub.host.contains(':') {
        format!("[{}]:{}", hub.host, hub.port)
    } else {
        format!("{}:{}", hub.host, hub.port)
    }
}

/// The one of `hubs` that `wanted` means. Several are for `prompt` to choose
/// from; with nobody to ask, the error names them.
fn pick_hub<'a>(
    hubs: &'a [BrowsedHub],
    wanted: Option<&str>,
    prompt: PinPrompt<'_>,
) -> Result<&'a BrowsedHub, OpenError> {
    let found: Vec<&BrowsedHub> = matching(hubs, wanted, |h| {
        vec![h.id.as_str(), h.name.as_str(), h.host.as_str()]
    });
    match found.as_slice() {
        [] => Err(OpenError::NoHub {
            wanted: wanted.map(printable),
        }),
        [one] => Ok(one),
        many => {
            let names: Vec<String> = many
                .iter()
                .map(|h| format!("{} at {}", printable(&h.name), hub_address(h)))
                .collect();
            prompt
                .pick(&names)
                .and_then(|i| many.get(i).copied())
                .ok_or(OpenError::Ambiguous {
                    what: "hubs",
                    flag: "--hub <name>",
                    names,
                })
        }
    }
}

/// Give a `--hub` target an address: look at the network and pick. Any other
/// target comes straight back, with nothing looked at.
pub fn locate_hub(target: Target, prompt: PinPrompt<'_>) -> Result<Target, OpenError> {
    match &target {
        Target::Link(l) if l.find.is_some() => {
            locate_on(l, browse_source()?, SCAN_WINDOW, prompt).map(Target::Link)
        }
        _ => Ok(target),
    }
}

fn locate_on(
    l: &LinkTarget,
    source: Arc<dyn BrowseSource>,
    window: Duration,
    prompt: PinPrompt<'_>,
) -> Result<LinkTarget, OpenError> {
    let wanted = l.find.as_ref().and_then(|q| q.wanted.as_deref());
    // A name asked for by its whole self needs no waiting for the others; with
    // no name, stopping at the first would be choosing for the person.
    let hubs = scan_hubs(source, window, |heard| {
        wanted.is_some_and(|w| {
            heard
                .iter()
                .any(|h| h.id.eq_ignore_ascii_case(w) || h.name.eq_ignore_ascii_case(w))
        })
    })?;
    Ok(l.at(pick_hub(&hubs, wanted, prompt)?))
}

/// A browse for one lost hub, kept running only while it is lost (F5): a hub
/// with a new DHCP lease announces its id from the new address.
struct HubFinder {
    hub_id: String,
    source: Arc<dyn BrowseSource>,
    browser: Option<HubBrowser>,
    /// The address the target came with: dialled while the hub is not heard.
    known_url: String,
    /// What the last attempt dialled.
    dialled_url: String,
    /// The last attempt found nothing that is this hub there.
    dial_failed: bool,
}

impl HubFinder {
    fn new(hub_id: String, source: Arc<dyn BrowseSource>, known_url: String) -> Self {
        Self {
            hub_id,
            source,
            browser: None,
            dialled_url: known_url.clone(),
            known_url,
            dial_failed: false,
        }
    }

    fn current(&self) -> Option<BrowsedHub> {
        self.browser.as_ref()?.resolve(&self.hub_id)
    }

    /// Is announced somewhere other than where dialling just failed.
    fn has_moved(&self) -> bool {
        self.dial_failed
            && self
                .current()
                .is_some_and(|hub| hub.url != self.dialled_url)
    }

    /// Where the next attempt should go: `None` is the address the target has.
    /// Starts the browse on the first call, which only starts a thread. If that
    /// fails the attempt goes ahead without, and the next call tries again.
    fn next_dial(&mut self) -> Option<BrowsedHub> {
        if self.browser.is_none() {
            self.browser =
                HubBrowser::start(Arc::clone(&self.source), None, BrowseOptions::default()).ok();
        }
        let found = self.current();
        found
            .as_ref()
            .map_or(&self.known_url, |hub| &hub.url)
            .clone_into(&mut self.dialled_url);
        found
    }

    /// After a failed attempt. A hub that lost power says no goodbye, and its
    /// old address would be offered for 75 minutes unless the network is asked.
    fn attempt_ended(&mut self, dial_failed: bool) {
        self.dial_failed = dial_failed;
        if let (true, Some(browser)) = (dial_failed, &self.browser) {
            browser.verify(&self.hub_id);
        }
    }

    /// Waits for the browser's thread, which only ever waits on its channel and
    /// on the setup of sockets: not long enough for the interface to notice.
    fn stop(&mut self) {
        self.browser = None;
        self.dial_failed = false;
    }
}

/// A directory for this crate's test files, under the build's `target` (never
/// tmpfs).
#[cfg(test)]
pub(crate) fn test_dir() -> PathBuf {
    let base = std::env::var_os("CARGO_TARGET_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"),
        PathBuf::from,
    );
    let dir = base.join("dspi-tui-test-files");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_r1 {
    use super::*;
    use dspi_link::fakehub::{Auth, FakeHub, HubConfig};
    use dspi_session::Notifications;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread::sleep;

    fn scratch(name: &str) -> PathBuf {
        let dir = test_dir().join(format!("r1-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("tokens.toml")
    }

    fn link(hub: &FakeHub, tokens: PathBuf) -> LinkTarget {
        let mut l = LinkTarget::parse(&hub.addr().to_string()).unwrap();
        l.tokens = Some(tokens);
        l
    }

    fn pin_hub() -> FakeHub {
        FakeHub::start(HubConfig {
            auth: Auth::Window("1234".into()),
            ..HubConfig::default()
        })
        .unwrap()
    }

    #[test]
    fn wp_r1_the_demo_device_connects_with_its_extensions() {
        let c = Target::Demo.connect(&no_prompt).unwrap();
        assert!(c.state.connected);
        assert!(!c.ext.supported.is_empty());
        assert!(ext_label(&c.ext).starts_with("ext "), "{:?}", c.ext);
    }

    #[test]
    fn wp_r1_a_hub_device_opens_probes_and_reads_single_shot() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let l = link(&hub, scratch("open"));
        let c = Target::Link(l).connect(&no_prompt).unwrap();
        assert!(!c.state.caps.serial.is_empty());
        let chunked = hub
            .log()
            .iter()
            .filter(|e| e.2 == dspi_link::fakehub::req::GET_ALL_PARAMS_CHUNK)
            .count();
        assert_eq!(chunked, 0, "a hub is never asked for chunks (spec 8.2)");
    }

    #[test]
    fn wp_r1_a_bridge_hub_that_blocks_chunks_still_opens() {
        let hub = FakeHub::start(HubConfig::bridge()).unwrap();
        let mut l = link(&hub, scratch("bridge"));
        l.pin = bridge_pin();
        Target::Link(l).connect(&no_prompt).unwrap();
    }

    fn bridge_pin() -> Option<String> {
        match HubConfig::bridge().auth {
            Auth::Fixed(pin) | Auth::Window(pin) => Some(pin),
            Auth::None => None,
        }
    }

    #[test]
    fn wp_r1_closing_the_session_ends_the_hub_session() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let c = Target::Link(link(&hub, scratch("close")))
            .connect(&no_prompt)
            .unwrap();
        assert_eq!(hub.sessions().len(), 1);
        drop(c);
        for _ in 0..100 {
            if hub.sessions().is_empty() {
                return;
            }
            sleep(Duration::from_millis(20));
        }
        panic!("the hub still lists {:?}", hub.sessions());
    }

    #[test]
    fn wp_r1_a_pin_hub_with_no_way_to_ask_says_so_and_is_final() {
        let hub = pin_hub();
        let Err(e) = Target::Link(link(&hub, scratch("nopin"))).open(&no_prompt) else {
            panic!("opened without a PIN");
        };
        assert!(matches!(e, OpenError::NeedsPin { .. }), "{e}");
        assert!(e.is_final());
    }

    #[test]
    fn wp_r1_a_wrong_pin_names_the_cause() {
        let hub = pin_hub();
        let prompt = |_: &str, _: &str| Some("0000".to_string());
        let Err(e) = Target::Link(link(&hub, scratch("wrong"))).open(&prompt) else {
            panic!("a wrong PIN was accepted");
        };
        assert!(matches!(e, OpenError::PinRefused { .. }), "{e}");
        assert!(e.is_final());
    }

    #[test]
    fn wp_r1_pairing_stores_a_token_that_replaces_the_pin_next_time() {
        let hub = pin_hub();
        let tokens = scratch("token");
        let asked = AtomicUsize::new(0);
        let prompt = |_: &str, _: &str| {
            asked.fetch_add(1, Ordering::SeqCst);
            Some("1234".to_string())
        };
        let target = Target::Link(link(&hub, tokens.clone()));
        drop(target.open(&prompt).unwrap());
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        assert!(tokens.exists(), "the token is on disk");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&tokens).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "only its owner reads the token file");
        }
        drop(target.open(&prompt).unwrap());
        assert_eq!(asked.load(Ordering::SeqCst), 1, "the token was enough");
    }

    #[test]
    fn wp_r1_choose_prefers_exact_then_partial_and_refuses_to_guess() {
        let items = ["DSPi Control", "DSPi Control 2", "Other"];
        let pick = |w: Option<&str>| {
            choose(
                &items,
                w,
                |i| vec![*i],
                |i| (*i).to_string(),
                "ports",
                "--midi",
            )
            .copied()
        };
        assert_eq!(pick(Some("dspi control")).unwrap(), "DSPi Control");
        assert_eq!(pick(Some("control 2")).unwrap(), "DSPi Control 2");
        assert!(matches!(
            pick(Some("control")),
            Err(OpenError::Ambiguous { .. })
        ));
        assert!(matches!(
            pick(Some("nothing")),
            Err(OpenError::Transport(TransportError::SerialNotFound(_)))
        ));
        assert!(matches!(pick(None), Err(OpenError::Ambiguous { .. })));
        assert!(matches!(
            choose(
                &[] as &[&str],
                None,
                |i| vec![*i],
                |i| i.to_string(),
                "p",
                "f"
            ),
            Err(OpenError::Transport(TransportError::NotFound))
        ));
    }

    #[test]
    fn wp_r1_a_target_is_pinned_to_the_device_it_found() {
        let usb = Target::Usb { serial: None }.pinned("ABC");
        assert_eq!(
            usb,
            Target::Usb {
                serial: Some("ABC".into())
            }
        );
        // An empty serial pins nothing.
        assert_eq!(
            Target::Usb { serial: None }.pinned(""),
            Target::Usb { serial: None }
        );
        let midi = Target::Midi { port: None };
        assert_eq!(midi.pinned("ABC"), midi, "a port name is not stable");
        let Target::Link(l) = Target::Link(LinkTarget::parse("hub").unwrap()).pinned("ABC") else {
            panic!("still a hub");
        };
        assert_eq!(l.device.as_deref(), Some("ABC"));
        assert_eq!(l.port, dspi_link::DEFAULT_PORT);
    }

    /// The table of §1.2, as words. `HOST_SET` and `BULK_SET` only mean this
    /// host on USB.
    #[test]
    fn wp_r1_words_follow_the_link_not_the_source_alone() {
        use TransportKind as K;
        // USB: HOST_SET is ours.
        assert_eq!(
            attribution(K::Usb, None, Source::HostSet, None),
            "changed by this host"
        );
        assert_eq!(
            attribution(K::Usb, None, Source::BulkSet, None),
            "changed elsewhere",
            "a BULK_SET with no write of ours pending is not ours"
        );
        assert_eq!(
            attribution(K::Usb, None, Source::Midi, None),
            "changed over MIDI"
        );
        // MIDI: our writes notify as MIDI; HOST_SET is another host's.
        assert_eq!(
            attribution(K::Midi, None, Source::Midi, None),
            "changed by this host"
        );
        assert_eq!(
            attribution(K::Midi, None, Source::HostSet, None),
            "changed over USB"
        );
        // Link: only the origin tells clients apart.
        assert_eq!(
            attribution(K::Link, Some(7), Source::HostSet, Some(7)),
            "changed by this host"
        );
        assert_eq!(
            attribution(K::Link, Some(7), Source::HostSet, Some(8)),
            "changed by another Link client"
        );
        assert_eq!(
            attribution(K::Link, Some(7), Source::HostSet, Some(0)),
            "changed elsewhere",
            "origin 0 is the device's own change, never this host"
        );
        assert_eq!(
            attribution(K::Link, Some(7), Source::Gpio, Some(0)),
            Source::Gpio.describe()
        );
    }

    #[test]
    fn wp_r1_a_bulk_line_is_only_said_when_it_cannot_be_our_own_doing() {
        use TransportKind as K;
        let news = |k, s, src, o| bulk_is_news(k, s, src, o);
        assert!(news(K::Usb, None, Source::BulkSet, None));
        assert!(news(K::Usb, None, Source::Midi, None));
        assert!(news(K::Midi, None, Source::HostSet, None));
        assert!(news(K::Link, Some(7), Source::HostSet, Some(8)));
        // Our own path: a big scalar write falls back to an invalidation.
        assert!(!news(K::Usb, None, Source::HostSet, None));
        assert!(!news(K::Midi, None, Source::Midi, None));
        assert!(!news(K::Link, Some(7), Source::HostSet, Some(7)));
        // A merge may hold ours; these two report themselves.
        for quiet in [Source::Unknown, Source::Preset, Source::Factory] {
            assert!(!news(K::Usb, None, quiet, None), "{quiet:?}");
        }
    }

    #[test]
    fn wp_r1_the_ext_label_names_what_the_firmware_lists() {
        assert_eq!(ext_label(&ExtState::default()), "");
        let ext = ExtState {
            supported: vec![
                ext_ids::EXT_CROSSFEED_PAIR_MAP,
                ext_ids::EXT_CORE_VREG,
                ext_ids::EXT_NOTIFY_FEATURES,
                0x8001,
            ],
            ..ExtState::default()
        };
        assert_eq!(ext_label(&ext), "ext xfeed vreg notify 0x8001");
    }

    fn poll_until_connected(r: &mut Reconnector) -> Box<Connected> {
        for _ in 0..500 {
            match r.poll(Instant::now()) {
                Reconnect::Connected(c) => return c,
                Reconnect::GaveUp(e) => panic!("gave up: {e}"),
                Reconnect::Waiting => sleep(Duration::from_millis(10)),
            }
        }
        panic!("never connected");
    }

    #[test]
    fn wp_r1_reconnect_retries_at_a_pace_until_the_device_is_back() {
        let tries = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&tries);
        let opener: Arc<Opener> = Arc::new(move || {
            if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(TransportError::NotFound.into())
            } else {
                Target::Demo.connect(&no_prompt)
            }
        });
        let mut r = Reconnector::with_opener(opener, false);
        let t0 = Instant::now();
        // The first attempt goes out at once and fails.
        assert!(matches!(r.poll(t0), Reconnect::Waiting));
        while tries.load(Ordering::SeqCst) == 0 {
            sleep(Duration::from_millis(5));
        }
        sleep(Duration::from_millis(30));
        assert!(matches!(r.poll(t0), Reconnect::Waiting));
        // Nothing more until the back-off has passed.
        for _ in 0..5 {
            assert!(matches!(r.poll(t0), Reconnect::Waiting));
        }
        sleep(Duration::from_millis(30));
        assert_eq!(tries.load(Ordering::SeqCst), 1);
        // Then it comes back.
        let later = t0 + RETRY_LOCAL;
        assert!(matches!(r.poll(later), Reconnect::Waiting));
        for _ in 0..500 {
            if let Reconnect::Connected(_) = r.poll(later) {
                assert_eq!(tries.load(Ordering::SeqCst), 2);
                return;
            }
            sleep(Duration::from_millis(10));
        }
        panic!("never connected");
    }

    #[test]
    fn wp_r1_reconnect_gives_up_when_a_person_is_needed() {
        let opener: Arc<Opener> = Arc::new(|| {
            Err(OpenError::NeedsPin {
                hub: "Hub".into(),
                pin_source: "window".into(),
            })
        });
        let mut r = Reconnector::with_opener(opener, false);
        for _ in 0..500 {
            match r.poll(Instant::now()) {
                Reconnect::GaveUp(e) => {
                    assert!(e.to_string().contains("needs pairing"), "{e}");
                    return;
                }
                Reconnect::Connected(_) => panic!("connected"),
                Reconnect::Waiting => sleep(Duration::from_millis(10)),
            }
        }
        panic!("never gave up");
    }

    #[test]
    fn wp_r1_a_hub_that_restarts_is_reconnected_with_its_token() {
        let hub = pin_hub();
        let tokens = scratch("restart");
        let target = Target::Link(link(&hub, tokens));
        let prompt = |_: &str, _: &str| Some("1234".to_string());
        let c = target.connect(&prompt).unwrap();
        let serial = c.state.caps.serial.clone();
        let mut session = c.session;
        let source = session
            .with_transport(|t| Ok(t.notifications()))
            .unwrap()
            .unwrap();
        let notes = Notifications::start(source).unwrap();

        hub.close_all(1001);
        for _ in 0..200 {
            if notes.is_disconnected() {
                break;
            }
            sleep(Duration::from_millis(10));
        }
        assert!(notes.is_disconnected(), "the reader saw the session end");

        let mut r = Reconnector::new(target.pinned(&serial));
        let back = poll_until_connected(&mut r);
        assert_eq!(back.state.caps.serial, serial);
    }

    #[test]
    fn wp_r1_offline_answers_disconnected_without_io() {
        let mut t = Offline::new("ABC");
        assert_eq!(t.descriptor().serial, "ABC");
        assert!(matches!(
            t.control_in(1, 0, 4),
            Err(TransportError::Disconnected)
        ));
        assert!(matches!(
            t.control_out(1, 0, &[0]),
            Err(TransportError::Disconnected)
        ));
        assert!(t.notifications().is_none());
    }

    /// A file where the token file's directory should be.
    fn unwritable_tokens(name: &str) -> PathBuf {
        let blocked = scratch(name);
        std::fs::write(&blocked, "x").unwrap();
        blocked.join("tokens.toml")
    }

    struct Recording(Mutex<Vec<String>>);

    impl Prompt for Recording {
        fn pin(&self, _: &str, _: &str) -> Option<String> {
            Some("1234".into())
        }

        fn warn(&self, message: &str) {
            self.0.lock().unwrap().push(message.to_owned());
        }
    }

    #[test]
    fn wp_r1_a_token_that_cannot_be_saved_warns_and_still_connects() {
        let hub = pin_hub();
        let prompt = Recording(Mutex::new(Vec::new()));
        let target = Target::Link(link(&hub, unwritable_tokens("warn")));
        target.open(&prompt).unwrap();
        let said = prompt.0.lock().unwrap();
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].contains("pairing token"), "{said:?}");
    }

    #[test]
    fn wp_r1_a_saved_token_warns_about_nothing() {
        let hub = pin_hub();
        let prompt = Recording(Mutex::new(Vec::new()));
        let target = Target::Link(link(&hub, scratch("nowarn")));
        target.open(&prompt).unwrap();
        assert!(prompt.0.lock().unwrap().is_empty());
    }

    #[test]
    fn wp_r1_a_reconnect_keeps_its_warnings_for_the_interface() {
        let hub = pin_hub();
        let mut l = link(&hub, unwritable_tokens("collect"));
        l.pin = Some("1234".into());
        let mut r = Reconnector::new(Target::Link(l));
        poll_until_connected(&mut r);
        let said = r.take_warnings();
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].contains("pairing token"), "{said:?}");
        assert!(r.take_warnings().is_empty(), "each once");
    }

    fn hub_id() -> String {
        HubConfig::default().hub_id
    }

    /// `AuthState::TokenRefused`: the stored token is forgotten, and the PIN
    /// gets a new one.
    #[test]
    fn wp_r1_a_stale_token_is_forgotten_and_paired_over() {
        let hub = pin_hub();
        let tokens = scratch("stale");
        let store = TokenStore::at(tokens.clone());
        store.put(&hub_id(), "stale").unwrap();
        let asked = AtomicUsize::new(0);
        let prompt = |_: &str, _: &str| {
            asked.fetch_add(1, Ordering::SeqCst);
            Some("1234".to_string())
        };
        let target = Target::Link(link(&hub, tokens));
        let t = target.open(&prompt).unwrap();
        assert!(t.session_id().is_some(), "the client is really paired");
        drop(t);
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        assert_eq!(hub.tokens_seen(), vec!["stale".to_string()]);
        let fresh = store.get(&hub_id()).unwrap();
        assert_ne!(fresh, "stale");
        // And the new token works, with no PIN.
        drop(target.open(&no_prompt).unwrap());
    }

    /// With nobody to ask, a refused token must not leave an unpaired client
    /// behind; it is an error, and the token is gone all the same.
    #[test]
    fn wp_r1_a_stale_token_with_nobody_to_ask_needs_a_pin() {
        let hub = pin_hub();
        let tokens = scratch("stale-nopin");
        let store = TokenStore::at(tokens.clone());
        store.put(&hub_id(), "stale").unwrap();
        let Err(e) = Target::Link(link(&hub, tokens)).open(&no_prompt) else {
            panic!("opened with a refused token and no PIN");
        };
        assert!(matches!(e, OpenError::NeedsPin { .. }), "{e}");
        assert_eq!(store.get(&hub_id()), None);
    }

    /// A hub saying "not now" says nothing about the token.
    #[test]
    fn wp_r1_a_rate_limited_token_is_kept_and_the_wait_is_respected() {
        let hub = pin_hub();
        let tokens = scratch("limited");
        let store = TokenStore::at(tokens.clone());
        store.put(&hub_id(), "good").unwrap();
        hub.fail_tokens_with(Some("rate_limited"));
        let Err(e) = Target::Link(link(&hub, tokens)).open(&no_prompt) else {
            panic!("opened while rate limited");
        };
        assert!(
            matches!(
                e,
                OpenError::Link {
                    source: LinkError::RateLimited { .. },
                    ..
                }
            ),
            "{e}"
        );
        assert!(!e.is_final());
        assert_eq!(store.get(&hub_id()).as_deref(), Some("good"));
        let wait = e.retry_advice().delay(0).unwrap();
        assert!(wait >= dspi_link::RATE_LIMIT_BACKOFF, "{wait:?}");
    }

    /// One attempt, and none again until the hub's wait has passed, however
    /// early in the back-off this is.
    #[test]
    fn wp_r1_reconnect_waits_as_long_as_the_hub_asks() {
        let tries = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&tries);
        let opener: Arc<Opener> = Arc::new(move || {
            seen.fetch_add(1, Ordering::SeqCst);
            Err(OpenError::Link {
                address: "hub".into(),
                source: LinkError::RateLimited { msg: String::new() },
            })
        });
        let mut r = Reconnector::with_opener(opener, true);
        let t0 = Instant::now();
        assert!(matches!(r.poll(t0), Reconnect::Waiting));
        while r.next_try.is_none() {
            sleep(Duration::from_millis(5));
            assert!(matches!(r.poll(t0), Reconnect::Waiting));
        }
        let early = t0 + dspi_link::RATE_LIMIT_BACKOFF - Duration::from_secs(1);
        assert!(matches!(r.poll(early), Reconnect::Waiting));
        assert!(r.running.is_none(), "too early for another attempt");
        assert_eq!(tries.load(Ordering::SeqCst), 1);
        let after = t0 + dspi_link::RATE_LIMIT_BACKOFF;
        assert!(matches!(r.poll(after), Reconnect::Waiting));
        assert!(r.running.is_some(), "and then it tries");
    }

    #[test]
    fn wp_r1_a_refused_handshake_stops_and_a_busy_hub_does_not() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let target = Target::Link(link(&hub, scratch("handshake")));
        hub.refuse_handshake(Some(403));
        let Err(e) = target.open(&no_prompt) else {
            panic!("opened through a 403");
        };
        assert!(e.is_final(), "{e}");

        hub.refuse_handshake(Some(503));
        let Err(e) = target.open(&no_prompt) else {
            panic!("opened through a 503");
        };
        assert!(!e.is_final(), "{e}");
        let wait = e.retry_advice().delay(0).unwrap();
        assert!(wait >= dspi_link::HUB_BUSY_BACKOFF, "{wait:?}");
    }

    /// A window PIN pairs one client: a second one needs a new window, and is
    /// told so rather than retried.
    #[test]
    fn wp_r1_a_used_window_pin_is_refused_until_the_window_reopens() {
        let hub = pin_hub();
        let prompt = |_: &str, _: &str| Some("1234".to_string());
        let first = Target::Link(link(&hub, scratch("window-a")));
        drop(first.open(&prompt).unwrap());
        let second = Target::Link(link(&hub, scratch("window-b")));
        let Err(e) = second.open(&prompt) else {
            panic!("the window paired twice");
        };
        assert!(matches!(e, OpenError::PinRefused { .. }), "{e}");
        assert!(e.to_string().contains("new pairing window"), "{e}");
        assert!(e.is_final());
        hub.reopen_window();
        drop(second.open(&prompt).unwrap());
    }

    /// F4: the hub took the PIN but answered after `pair` gave up. The grace
    /// keeps the session open, so the token is heard and the client is usable.
    #[test]
    fn wp_r1_a_pair_answered_late_still_stores_its_token_and_connects() {
        let hub = Arc::new(pin_hub());
        let tokens = scratch("late");
        let target = Target::Link(link(&hub, tokens.clone()));
        let held = Arc::clone(&hub);
        let prompt = move |_: &str, _: &str| {
            held.hold_replies();
            let held = Arc::clone(&held);
            std::thread::spawn(move || {
                sleep(dspi_link::Timing::default().request_timeout + Duration::from_millis(500));
                held.release_replies();
            });
            Some("1234".to_string())
        };
        let t = target.open(&prompt).unwrap();
        assert!(t.session_id().is_some());
        drop(t);
        assert!(TokenStore::at(tokens).get(&hub_id()).is_some());
        drop(target.open(&no_prompt).unwrap());
    }

    #[test]
    fn wp_r1_printable_replaces_every_control_character() {
        assert_eq!(printable("Bench 1 é"), "Bench 1 é");
        assert_eq!(
            printable("a\u{1b}[2Jb\u{9b}c\u{7f}\n"),
            "a\u{FFFD}[2Jb\u{FFFD}c\u{FFFD}\u{FFFD}"
        );
    }

    #[test]
    fn wp_r1_a_hub_name_with_an_escape_never_reaches_an_error_or_a_listing() {
        let hub = FakeHub::start(HubConfig {
            name: "Evil\u{1b}]0;x".into(),
            device_name: "Dev\u{1b}[2J".into(),
            auth: Auth::Window("1234".into()),
            ..HubConfig::default()
        })
        .unwrap();
        let target = Target::Link(link(&hub, scratch("evil")));
        let Err(e) = target.open(&no_prompt) else {
            panic!("opened without a PIN");
        };
        assert!(!e.to_string().contains('\u{1b}'), "{e}");
        let prompt = |h: &str, _: &str| {
            assert!(!h.contains('\u{1b}'));
            Some("1234".to_string())
        };
        let listed = target.list(&prompt).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].name.contains('\u{1b}'), "{listed:?}");
    }

    #[test]
    fn wp_r1_what_will_not_change_is_not_retried() {
        let stop = [
            OpenError::ChannelMap,
            OpenError::Ambiguous {
                what: "ports",
                flag: "--midi",
                names: Vec::new(),
            },
            OpenError::Transport(TransportError::Incompatible("old".into())),
        ];
        for e in &stop {
            assert!(e.is_final(), "{e}");
        }
        let retry = [
            OpenError::Transport(TransportError::NotFound),
            OpenError::Transport(TransportError::Disconnected),
        ];
        for e in &retry {
            assert!(!e.is_final(), "{e}");
        }
    }

    #[test]
    fn wp_r1_a_leftover_token_temp_file_cannot_widen_the_new_tokens_mode() {
        let path = scratch("leftover");
        let tmp = path.with_extension("toml.new");
        std::fs::write(&tmp, "old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        TokenStore::at(path.clone()).put("a", "one").unwrap();
        assert!(!tmp.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0);
        }
    }

    #[test]
    fn wp_r1_tokens_survive_a_second_hub_and_a_bad_file() {
        let path = scratch("store");
        let s = TokenStore::at(path.clone());
        assert_eq!(s.get("a"), None);
        s.put("a", "one").unwrap();
        s.put("b", "two").unwrap();
        assert_eq!(s.get("a").as_deref(), Some("one"));
        assert_eq!(s.get("b").as_deref(), Some("two"));
        s.forget("a").unwrap();
        s.forget("never there").unwrap();
        assert_eq!(s.get("a"), None);
        assert_eq!(s.get("b").as_deref(), Some("two"));
        std::fs::write(&path, "not [ toml").unwrap();
        assert_eq!(s.get("a"), None, "a bad file reads as no tokens");
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_tm9 {
    use super::*;
    use dspi_link::discovery::{FakeAdvertiser, HubAdvert};
    use dspi_link::fakehub::{Auth, FakeHub, HubConfig};
    use std::net::{IpAddr, Ipv4Addr, TcpListener};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread::sleep;

    const WINDOW: Duration = Duration::from_millis(150);
    const LOCAL: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    fn tokens(name: &str) -> PathBuf {
        let dir = test_dir().join(format!("tm9-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("tokens.toml")
    }

    fn advert(name: &str, id: &str, port: u16) -> HubAdvert {
        HubAdvert {
            name: name.into(),
            port,
            hub_id: id.into(),
            kind: "console".into(),
            auth: "none".into(),
            device_count: 1,
            ..HubAdvert::default()
        }
    }

    /// A port nothing listens on: a hub's old address.
    fn dead_port() -> u16 {
        let listener = TcpListener::bind((LOCAL, 0)).unwrap();
        listener.local_addr().unwrap().port()
    }

    /// `id`'s hub, last known at `port` of this machine.
    fn known_at(port: u16, id: Option<&str>, name: &str) -> LinkTarget {
        LinkTarget {
            hub_id: id.map(str::to_owned),
            tokens: Some(tokens(name)),
            ..LinkTarget::parse(&format!("127.0.0.1:{port}")).unwrap()
        }
    }

    fn eventually(what: &str, mut done: impl FnMut() -> bool) {
        for _ in 0..500 {
            if done() {
                return;
            }
            sleep(Duration::from_millis(10));
        }
        panic!("{what}");
    }

    /// Polls at one frozen moment, so nothing here is the back-off running out.
    fn poll_to_end(r: &mut Reconnector, at: Instant) -> Reconnect {
        for _ in 0..500 {
            match r.poll(at) {
                Reconnect::Waiting => sleep(Duration::from_millis(10)),
                ended => return ended,
            }
        }
        panic!("the attempt never ended");
    }

    /// F5: dial, fail, ask the network, hear the hub elsewhere, dial there.
    #[test]
    fn wp_tm9_a_hub_with_a_new_address_is_found_by_its_id_without_waiting() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let id = HubConfig::default().hub_id;
        let stale = dead_port();
        let net = FakeAdvertiser::new();
        // Lost power: still listed, at the address that has stopped answering.
        net.announce(&advert("Studio", &id, stale), &[LOCAL]);
        let target = Target::Link(known_at(stale, Some(&id), "moved"));
        let mut r = Reconnector::browsing(target, Some(Arc::new(net.clone())));
        assert_eq!(net.opens(), 0, "no browse before anything is lost");

        let t0 = Instant::now();
        assert!(matches!(r.poll(t0), Reconnect::Waiting));
        eventually("the failed dial never asked the network", || {
            assert!(matches!(r.poll(t0), Reconnect::Waiting));
            !net.verified().is_empty()
        });
        assert_eq!(net.verified(), [FakeAdvertiser::instance("Studio")]);
        assert_eq!(net.open_now(), 1, "the browse runs while reconnecting");

        // The back-off (a second) has a long way to go at this frozen moment.
        let moved = hub.addr().port();
        net.announce(&advert("Studio", &id, moved), &[LOCAL]);
        let Reconnect::Connected(c) = poll_to_end(&mut r, t0) else {
            panic!("did not connect");
        };
        let reached = c.link.clone().unwrap();
        assert_eq!(reached.port, moved, "the target learns the new address");
        assert_eq!(reached.hub_id.as_deref(), Some(id.as_str()));
        assert_eq!(hub.sessions().len(), 1);
        eventually("the browse outlived the reconnect", || net.open_now() == 0);
    }

    #[test]
    fn wp_tm9_a_hub_gone_from_the_list_is_dialled_at_the_address_it_had() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let net = FakeAdvertiser::new();
        let id = HubConfig::default().hub_id;
        let target = Target::Link(known_at(hub.addr().port(), Some(&id), "same"));
        let mut r = Reconnector::browsing(target, Some(Arc::new(net.clone())));
        let Reconnect::Connected(c) = poll_to_end(&mut r, Instant::now()) else {
            panic!("did not connect");
        };
        assert_eq!(c.link.unwrap().port, hub.addr().port());
    }

    #[test]
    fn wp_tm9_without_a_hub_id_or_off_a_hub_nothing_browses() {
        let net = FakeAdvertiser::new();
        let source = || -> Option<Arc<dyn BrowseSource>> { Some(Arc::new(net.clone())) };
        let typed = Target::Link(known_at(dead_port(), None, "noid"));
        let mut r = Reconnector::browsing(typed, source());
        assert!(matches!(r.poll(Instant::now()), Reconnect::Waiting));
        let mut r2 = Reconnector::browsing(Target::Demo, source());
        assert!(matches!(
            poll_to_end(&mut r2, Instant::now()),
            Reconnect::Connected(_)
        ));
        assert_eq!(net.opens(), 0);
        drop(r);
    }

    #[test]
    fn wp_tm9_a_person_being_needed_ends_the_browse_too() {
        let hub = FakeHub::start(HubConfig {
            auth: Auth::Window("1234".into()),
            ..HubConfig::default()
        })
        .unwrap();
        let id = HubConfig::default().hub_id;
        let net = FakeAdvertiser::new();
        let target = Target::Link(known_at(hub.addr().port(), Some(&id), "pin"));
        let mut r = Reconnector::browsing(target, Some(Arc::new(net.clone())));
        let Reconnect::GaveUp(e) = poll_to_end(&mut r, Instant::now()) else {
            panic!("did not give up");
        };
        assert!(matches!(e, OpenError::NeedsPin { .. }), "{e}");
        assert_eq!(net.opens(), 1);
        eventually("the browse outlived giving up", || net.open_now() == 0);
    }

    /// A DHCP lease can hand a hub's old address to another hub.
    #[test]
    fn wp_tm9_another_hub_at_the_address_is_refused_unasked_and_tried_again() {
        let other = FakeHub::start(HubConfig {
            auth: Auth::Window("1234".into()),
            ..HubConfig::default()
        })
        .unwrap();
        let asked = AtomicUsize::new(0);
        let prompt = |_: &str, _: &str| {
            asked.fetch_add(1, Ordering::SeqCst);
            Some("1234".to_string())
        };
        let mine = known_at(other.addr().port(), Some("another-hub"), "other");
        let tokens = mine.tokens.clone().unwrap();
        let Err(e) = Target::Link(mine).connect(&prompt) else {
            panic!("adopted another hub");
        };
        assert!(matches!(e, OpenError::WrongHub { .. }), "{e}");
        assert!(!e.is_final(), "the right one may yet appear");
        assert!(e.is_dial_failure());
        assert_eq!(asked.load(Ordering::SeqCst), 0, "no PIN went to it");
        assert!(!tokens.exists(), "and no token came back");
        eventually("the session stayed open", || other.sessions().is_empty());
    }

    #[test]
    fn wp_tm9_a_hub_id_is_compared_without_case() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let id = HubConfig::default().hub_id.to_uppercase();
        let l = known_at(hub.addr().port(), Some(&id), "case");
        assert!(Target::Link(l).connect(&no_prompt).is_ok());
    }

    #[test]
    fn wp_tm9_a_typed_address_learns_its_hubs_id_on_connecting() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let l = known_at(hub.addr().port(), None, "learn");
        let c = Target::Link(l.clone()).connect(&no_prompt).unwrap();
        let reached = c.link.unwrap();
        assert_eq!(reached.hub_id, Some(HubConfig::default().hub_id));
        assert_eq!((reached.host, reached.port), (l.host, l.port));
        assert!(Target::Demo.connect(&no_prompt).unwrap().link.is_none());
    }

    /// Answers a choice as told.
    struct Picks(Option<usize>);

    impl Prompt for Picks {
        fn pin(&self, _hub: &str, _pin_source: &str) -> Option<String> {
            None
        }

        fn pick(&self, _choices: &[String]) -> Option<usize> {
            self.0
        }
    }

    fn two_hubs() -> FakeAdvertiser {
        let net = FakeAdvertiser::new();
        net.announce(
            &advert("Studio Mac", "aaaa", 4848),
            &["192.168.1.20".parse().unwrap()],
        );
        net.announce(
            &advert("Attic", "bbbb", 4849),
            &["192.168.1.30".parse().unwrap()],
        );
        net
    }

    fn locate(
        net: &FakeAdvertiser,
        wanted: Option<&str>,
        prompt: PinPrompt<'_>,
    ) -> Result<LinkTarget, OpenError> {
        let l = LinkTarget::find(wanted.map(str::to_owned));
        locate_on(&l, Arc::new(net.clone()), WINDOW, prompt)
    }

    #[test]
    fn wp_tm9_the_only_hub_needs_no_name_and_brings_what_it_announced() {
        let net = FakeAdvertiser::new();
        net.announce(
            &HubAdvert {
                path: Some("/other/path".into()),
                ..advert("Studio", "AAAA", 4848)
            },
            &["192.168.1.20".parse().unwrap()],
        );
        let device = LinkTarget {
            device: Some("SERIAL".into()),
            pin: Some("1234".into()),
            ..LinkTarget::find(None)
        };
        let l = locate_on(&device, Arc::new(net), WINDOW, &no_prompt).unwrap();
        assert_eq!((l.host.as_str(), l.port), ("192.168.1.20", 4848));
        assert_eq!(l.hub_id.as_deref(), Some("aaaa"));
        assert_eq!(l.ws_url(), "ws://192.168.1.20:4848/other/path");
        assert_eq!(l.find, None, "it has been found");
        assert_eq!(l.device.as_deref(), Some("SERIAL"), "the flags stay");
        assert_eq!(l.pin.as_deref(), Some("1234"));
    }

    #[test]
    fn wp_tm9_several_hubs_are_for_a_person_to_pick_or_for_the_error_to_name() {
        let net = two_hubs();
        // Sorted by name, and the answer is an index into that.
        let l = locate(&net, None, &Picks(Some(1))).unwrap();
        assert_eq!(l.hub_id.as_deref(), Some("aaaa"));
        let l = locate(&net, None, &Picks(Some(0))).unwrap();
        assert_eq!(l.hub_id.as_deref(), Some("bbbb"));
        for nobody in [Picks(None), Picks(Some(2))] {
            let Err(e) = locate(&net, None, &nobody) else {
                panic!("chose for the person");
            };
            let OpenError::Ambiguous { names, flag, .. } = &e else {
                panic!("{e}");
            };
            assert_eq!(*flag, "--hub <name>");
            assert_eq!(
                names,
                &[
                    "Attic at 192.168.1.30:4849",
                    "Studio Mac at 192.168.1.20:4848"
                ]
            );
        }
    }

    #[test]
    fn wp_tm9_a_hub_is_named_by_part_of_its_name_its_id_or_its_address() {
        let net = two_hubs();
        for (wanted, id) in [
            ("mac", "aaaa"),
            ("STUDIO MAC", "aaaa"),
            ("bbbb", "bbbb"),
            ("192.168.1.30", "bbbb"),
        ] {
            let l = locate(&net, Some(wanted), &no_prompt).unwrap();
            assert_eq!(l.hub_id.as_deref(), Some(id), "{wanted}");
        }
        // Both have this in their address.
        assert!(matches!(
            locate(&net, Some("192.168"), &no_prompt),
            Err(OpenError::Ambiguous { .. })
        ));
    }

    #[test]
    fn wp_tm9_no_such_hub_counts_as_no_device() {
        for (net, wanted) in [(two_hubs(), Some("cellar")), (FakeAdvertiser::new(), None)] {
            let Err(e) = locate(&net, wanted, &no_prompt) else {
                panic!("found a hub");
            };
            assert!(matches!(e, OpenError::NoHub { .. }), "{e}");
            assert!(e.is_no_device());
        }
    }

    #[test]
    fn wp_tm9_a_whole_name_ends_the_wait_but_no_name_never_does() {
        let net = two_hubs();
        let source: Arc<dyn BrowseSource> = Arc::new(net);
        let long = Duration::from_secs(30);
        let l = LinkTarget::find(Some("Attic".into()));
        let t0 = Instant::now();
        locate_on(&l, Arc::clone(&source), long, &no_prompt).unwrap();
        assert!(t0.elapsed() < long / 3, "{:?}", t0.elapsed());
        // With no name, the whole window: the other hub may be a moment away.
        let t0 = Instant::now();
        let hubs = scan_hubs(source, WINDOW, |_| false).unwrap();
        assert!(t0.elapsed() >= WINDOW);
        assert_eq!(hubs.len(), 2);
    }

    #[test]
    fn wp_tm9_browsing_that_fails_says_so_rather_than_finding_nothing() {
        let net = FakeAdvertiser::new();
        net.fail_next_opens(&["no interface"]);
        let Err(e) = locate(&net, None, &no_prompt) else {
            panic!("found a hub");
        };
        assert!(matches!(e, OpenError::Browse(_)), "{e}");
        assert!(e.to_string().contains("no interface"), "{e}");
        assert!(!e.is_no_device());
    }

    #[test]
    fn wp_tm9_a_found_hub_connects_and_a_target_with_an_address_is_left_alone() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let id = HubConfig::default().hub_id;
        let net = FakeAdvertiser::new();
        net.announce(&advert("Studio", &id, hub.addr().port()), &[LOCAL]);
        let asked = LinkTarget {
            tokens: Some(tokens("found")),
            ..LinkTarget::find(None)
        };
        let l = locate_on(&asked, Arc::new(net), WINDOW, &no_prompt).unwrap();
        assert!(Target::Link(l).connect(&no_prompt).is_ok());

        let typed = Target::Link(LinkTarget::parse("hub").unwrap());
        assert_eq!(locate_hub(typed.clone(), &no_prompt).unwrap(), typed);
        assert_eq!(locate_hub(Target::Demo, &no_prompt).unwrap(), Target::Demo);
        // In this crate's tests, as in a build without mDNS.
        let Err(e) = locate_hub(Target::Link(LinkTarget::find(None)), &no_prompt) else {
            panic!("reached a real network from a test");
        };
        assert!(matches!(e, OpenError::NoMdns), "{e}");
    }
}

// -- WP SECTM --
#[cfg(test)]
mod tests_wp_sectm {
    use super::*;
    use dspi_link::credential::{pin_display, pin_to_b64url};
    use dspi_link::fakehub::{Auth, FakeHub, FakeTlsKey, HubConfig};

    const PIN: &str = "123456";

    fn scratch(name: &str) -> PathBuf {
        let dir = test_dir().join(format!("sectm-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("tokens.toml")
    }

    fn pin_hub() -> FakeHub {
        FakeHub::start(HubConfig {
            auth: Auth::Fixed(PIN.into()),
            // -- WP SECFIX -- not "local" (only 127.0.0.1, ::1 and localhost
            // are), so a plaintext session here is unsecured, as on a LAN
            bind: OFF_LOOPBACK.parse().unwrap(),
            ..HubConfig::default()
        })
        .unwrap()
    }

    /// -- WP SECFIX -- A hub address the "local" rule does not cover, still on
    /// this machine (Linux routes all of 127.0.0.0/8 to the loopback device).
    const OFF_LOOPBACK: &str = "127.0.0.2:0";

    fn link(hub: &FakeHub, tokens: PathBuf) -> LinkTarget {
        LinkTarget {
            tokens: Some(tokens),
            ..LinkTarget::parse(&hub.addr().to_string()).unwrap()
        }
    }

    fn hid() -> String {
        HubConfig::default().hub_id
    }

    /// A person who types `pin` and answers `yes`, and what they were shown.
    #[derive(Default)]
    struct Person {
        pin: Option<String>,
        yes: Option<bool>,
        asked: Mutex<Vec<String>>,
        questions: Mutex<Vec<String>>,
        notes: Mutex<Vec<String>>,
        // -- WP PINTLS -- answers to the first questions, in order; then `yes`
        answers: Mutex<Vec<bool>>,
    }

    impl Person {
        fn new(pin: Option<&str>, yes: Option<bool>) -> Self {
            Self {
                pin: pin.map(str::to_owned),
                yes,
                ..Self::default()
            }
        }
        fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap().clone()
        }
        fn questions(&self) -> Vec<String> {
            self.questions.lock().unwrap().clone()
        }
        fn notes(&self) -> Vec<String> {
            self.notes.lock().unwrap().clone()
        }
    }

    impl Prompt for Person {
        fn pin(&self, hub: &str, _pin_source: &str) -> Option<String> {
            self.asked.lock().unwrap().push(hub.to_owned());
            self.pin.clone()
        }
        fn warn(&self, message: &str) {
            self.notes.lock().unwrap().push(message.to_owned());
        }
        fn confirm(&self, question: &str) -> Option<bool> {
            self.questions.lock().unwrap().push(question.to_owned());
            let mut answers = self.answers.lock().unwrap();
            if answers.is_empty() {
                self.yes
            } else {
                Some(answers.remove(0))
            }
        }
        fn note(&self, message: &str) {
            self.notes.lock().unwrap().push(message.to_owned());
        }
    }

    fn link_err(r: Result<LinkClient, OpenError>) -> LinkError {
        match r {
            Err(OpenError::Link { source, .. }) => source,
            Err(e) => panic!("expected a Link error, got {e}"),
            Ok(_) => panic!("expected a Link error, got a session"),
        }
    }

    /// Paired in plaintext (a v1 token), then the hub turns TLS on: the next
    /// connect pins its key and saves the v2 credential in the 0600 file.
    fn paired_then_pinned(hub: &FakeHub, tokens: &Path) -> (LinkTarget, FakeTlsKey) {
        let l = link(hub, tokens.to_owned());
        let person = Person::new(Some(PIN), None);
        link_client(&l, &person).unwrap().close();
        let v1 = l.stored_token(&hid()).unwrap();
        assert!(!v1.starts_with("v2."), "plaintext pairing: a v1 token");
        assert!(
            person.asked()[0].ends_with(" (unsecured)"),
            "the PIN prompt says the pairing is unencrypted: {:?}",
            person.asked()
        );
        let key = FakeTlsKey::generate();
        hub.set_tls(Some(&key));
        let client = link_client(&l, &person).unwrap();
        assert!(client.tls().is_some(), "TLS first");
        client.close();
        let v2 = Credential::parse(&l.stored_token(&hid()).unwrap());
        assert_eq!(v2.pin, Some(key.pin()), "pinned to the key TLS met");
        assert_eq!(v2.token, v1, "the same token, upgraded");
        assert!(
            person
                .notes()
                .iter()
                .any(|n| n.contains(&pin_display(&key.pin()))),
            "the new hub key is said once: {:?}",
            person.notes()
        );
        (l, key)
    }

    #[test]
    fn wp_sectm_a_v1_token_is_upgraded_to_a_saved_v2_credential() {
        let hub = pin_hub();
        let tokens = scratch("upgrade");
        let (l, key) = paired_then_pinned(&hub, &tokens);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&tokens).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "the credential file is 0600");
        }
        // Again: pinned, nothing new to save or say
        let person = Person::new(None, None);
        let client = link_client(&l, &person).unwrap();
        assert_eq!(client.tls().map(|t| t.key), Some(key.pin()));
        client.close();
        assert!(person.notes().is_empty(), "{:?}", person.notes());
        assert!(person.asked().is_empty(), "no PIN asked");
    }

    #[test]
    fn wp_sectm_another_key_is_refused_and_pairing_again_only_on_a_yes() {
        let hub = pin_hub();
        let tokens = scratch("changed");
        let (l, key) = paired_then_pinned(&hub, &tokens);
        let stored = l.stored_token(&hid()).unwrap();
        let other = FakeTlsKey::generate();
        hub.set_tls(Some(&other));
        let seen = hub.tokens_seen().len();

        // Nobody to ask: refused, the record kept, and how to pair again said
        let nobody = Person::new(None, None);
        assert!(matches!(
            link_err(link_client(&l, &nobody)),
            LinkError::HubKeyChanged
        ));
        assert!(
            nobody.notes().iter().any(|n| n.contains("pair again")),
            "{:?}",
            nobody.notes()
        );
        // A no: the same
        let no = Person::new(Some(PIN), Some(false));
        assert!(matches!(
            link_err(link_client(&l, &no)),
            LinkError::HubKeyChanged
        ));
        assert_eq!(no.questions().len(), 1);
        assert!(no.asked().is_empty(), "no PIN asked after a no");
        assert_eq!(
            hub.tokens_seen().len(),
            seen,
            "no token went to the other key"
        );
        assert_eq!(l.stored_token(&hid()).as_deref(), Some(stored.as_str()));

        // A yes: forgotten, paired again, pinned to the new key (shown at the PIN)
        let yes = Person::new(Some(PIN), Some(true));
        link_client(&l, &yes).unwrap().close();
        let asked = yes.asked();
        assert_eq!(asked.len(), 1);
        assert!(asked[0].contains(&pin_display(&other.pin())), "{asked:?}");
        let now = Credential::parse(&l.stored_token(&hid()).unwrap());
        assert_eq!(now.pin, Some(other.pin()));
        assert_ne!(now.pin, Some(key.pin()));
    }

    #[test]
    fn wp_sectm_a_pinned_hub_without_tls_gets_no_token_in_plaintext() {
        let hub = pin_hub();
        let tokens = scratch("refused");
        let (l, _) = paired_then_pinned(&hub, &tokens);
        let stored = l.stored_token(&hid()).unwrap();
        hub.set_tls(None);
        let seen = hub.tokens_seen().len();
        let nobody = Person::new(None, None);
        assert!(matches!(
            link_err(link_client(&l, &nobody)),
            LinkError::TlsRefused { .. }
        ));
        assert_eq!(hub.tokens_seen().len(), seen, "never plaintext");
        assert_eq!(
            l.stored_token(&hid()).as_deref(),
            Some(stored.as_str()),
            "kept"
        );
    }

    #[test]
    fn wp_sectm_other_failures_keep_the_record_and_offer_nothing() {
        let hub = pin_hub();
        let tokens = scratch("other");
        let (l, _) = paired_then_pinned(&hub, &tokens);
        let stored = l.stored_token(&hid()).unwrap();
        drop(hub);
        let person = Person::new(Some(PIN), Some(true));
        assert!(link_client(&l, &person).is_err());
        assert!(person.questions().is_empty(), "no pairing again offered");
        assert_eq!(l.stored_token(&hid()).as_deref(), Some(stored.as_str()));
        assert!(!offers_repair("io") && !offers_repair("wrong_pin"));
        for code in REPAIR_CODES {
            assert!(offers_repair(code));
        }
        assert_eq!(repair_code(&LinkError::TlsRequired), Some("tls_required"));
        assert_eq!(repair_code(&LinkError::Timeout), None);
    }

    // -- WP PINTLS -- (owner decision 2026-11-17) a hub once pinned stays TLS-only

    /// Pinned, then the hub serves no TLS (an attacker resetting it, or TLS
    /// turned off). Pair Again (yes) keeps it TLS-only: refused with
    /// `tls_required_pinned` before any PIN; Forget is then offered (no here).
    #[test]
    fn wp_pintls_pair_again_never_pairs_in_plaintext() {
        let hub = pin_hub();
        let tokens = scratch("pintls-again");
        let (l, key) = paired_then_pinned(&hub, &tokens);
        hub.set_tls(None);
        let seen = hub.tokens_seen().len();
        let person = Person::new(Some(PIN), None);
        *person.answers.lock().unwrap() = vec![true, false];
        assert!(matches!(
            link_err(link_client(&l, &person)),
            LinkError::TlsRequiredPinned
        ));
        let questions = person.questions();
        assert_eq!(questions.len(), 2, "{questions:?}");
        assert!(
            questions[0].contains("only over an encrypted connection"),
            "{questions:?}"
        );
        assert!(
            questions[1].contains("Forget the hub entirely")
                && questions[1].contains("without encryption"),
            "{questions:?}"
        );
        assert!(person.asked().is_empty(), "no PIN asked, so none sent");
        assert_eq!(hub.tokens_seen().len(), seen, "no token in plaintext");
        let kept = Credential::parse(&l.stored_token(&hid()).unwrap());
        assert!(kept.pinned_before && kept.tls_only() && kept.token.is_empty());
        assert_eq!(kept.previous, Some(key.pin()));

        // Next time, nobody to ask: refused at once, and how to forget said
        let nobody = Person::new(None, None);
        assert!(matches!(
            link_err(link_client(&l, &nobody)),
            LinkError::TlsRequiredPinned
        ));
        assert!(nobody.asked().is_empty());
        assert!(
            nobody.notes().iter().any(|n| n.contains("forget the hub")),
            "{:?}",
            nobody.notes()
        );
        assert!(l.stored_token(&hid()).is_some(), "still TLS-only");
    }

    /// (review F3) Dialled by address, an answer that names the hub and asks for
    /// no PIN (an impostor, or the hub reset to open access with TLS off) gets
    /// no unencrypted session while the record says TLS only: pinned, pinned
    /// before, or damaged. Forgetting the hub is the way back.
    #[test]
    fn wp_pintls_an_open_answer_by_address_gets_no_plaintext_session() {
        let hub = pin_hub();
        let tokens = scratch("pintls-open");
        let (l, _) = paired_then_pinned(&hub, &tokens);
        let impostor = FakeHub::start(HubConfig {
            auth: Auth::None,
            bind: OFF_LOOPBACK.parse().unwrap(),
            ..HubConfig::default()
        })
        .unwrap();
        let there = link(&impostor, tokens.clone());
        assert!(there.hub_id.is_none(), "dialled by address");
        let nobody = Person::new(None, None);
        // Pinned
        assert!(matches!(
            link_err(link_client(&there, &nobody)),
            LinkError::TlsRefused { .. }
        ));
        // Pinned before (Pair Again)
        l.drop_pairing(&hid()).unwrap();
        assert!(matches!(
            link_err(link_client(&there, &nobody)),
            LinkError::TlsRequiredPinned
        ));
        // A damaged record
        there.store_credential(&hid(), "v2.dA.short.t").unwrap();
        assert!(matches!(
            link_err(link_client(&there, &nobody)),
            LinkError::TlsRequiredPinned
        ));
        assert!(impostor.tokens_seen().is_empty());
        // Forgotten: an open hub like any other
        there.forget_hub(&hid()).unwrap();
        let client = link_client(&there, &nobody).unwrap();
        assert!(client.tls().is_none());
        client.close();
    }

    /// After Pair Again, a TLS pairing goes through and pins the new key.
    #[test]
    fn wp_pintls_pair_again_over_tls_re_pins() {
        let hub = pin_hub();
        let tokens = scratch("pintls-tls");
        let (l, _) = paired_then_pinned(&hub, &tokens);
        l.drop_pairing(&hid()).unwrap();
        assert!(Credential::parse(&l.stored_token(&hid()).unwrap()).pinned_before);
        let new_key = FakeTlsKey::generate();
        hub.set_tls(Some(&new_key));
        let person = Person::new(Some(PIN), None);
        let client = link_client(&l, &person).unwrap();
        assert_eq!(client.tls().map(|t| t.key), Some(new_key.pin()));
        client.close();
        assert_eq!(person.asked().len(), 1, "the PIN prompt");
        assert!(person.asked()[0].contains(&pin_display(&new_key.pin())));
        let now = Credential::parse(&l.stored_token(&hid()).unwrap());
        assert_eq!(now.pin, Some(new_key.pin()));
        assert!(!now.token.is_empty());
    }

    /// Forget hub is the only way back to plaintext: after a yes, the record is
    /// gone and the pairing runs unencrypted, the prompt saying "(unsecured)".
    #[test]
    fn wp_pintls_forget_hub_allows_plaintext_again() {
        let hub = pin_hub();
        let tokens = scratch("pintls-forget");
        let (l, _) = paired_then_pinned(&hub, &tokens);
        l.drop_pairing(&hid()).unwrap();
        hub.set_tls(None);
        let person = Person::new(Some(PIN), Some(true));
        let client = link_client(&l, &person).unwrap();
        assert!(client.tls().is_none());
        client.close();
        assert_eq!(person.questions().len(), 1, "the Forget question only");
        assert!(
            person.asked()[0].ends_with(" (unsecured)"),
            "{:?}",
            person.asked()
        );
        let v1 = l.stored_token(&hid()).unwrap();
        assert!(!v1.starts_with("v2."), "a plaintext v1 token: {v1}");
        // A refused v1 token keeps nothing; a v1 hub was never pinned
        l.drop_pairing(&hid()).unwrap();
        assert!(l.stored_token(&hid()).is_none());
    }

    fn pair_link(hub: &FakeHub, key: &FakeTlsKey, tokens: PathBuf) -> LinkTarget {
        let text = format!(
            "dspi://pair?h={}&a={}&k={}&p={PIN}",
            hid(),
            hub.addr(),
            pin_to_b64url(&key.pin())
        );
        LinkTarget {
            tokens: Some(tokens),
            ..LinkTarget::from_pair_link(&text).unwrap()
        }
    }

    #[test]
    fn wp_sectm_a_pair_link_with_a_wrong_key_is_refused_before_any_pin() {
        let key = FakeTlsKey::generate();
        let hub = pin_hub();
        hub.set_tls(Some(&key));
        let tokens = scratch("wrong-k");
        let l = pair_link(&hub, &FakeTlsKey::generate(), tokens);
        let person = Person::new(Some(PIN), Some(true));
        assert!(matches!(
            link_err(link_client(&l, &person)),
            LinkError::HubKeyUnverified
        ));
        assert!(
            person.questions().is_empty(),
            "never asked: the key failed first"
        );
        assert!(person.asked().is_empty());
        assert_eq!(l.stored_token(&hid()), None, "nothing stored");
    }

    #[test]
    fn wp_sectm_a_pair_link_pairs_only_after_a_yes() {
        let key = FakeTlsKey::generate();
        let hub = pin_hub();
        hub.set_tls(Some(&key));
        let tokens = scratch("pair-link");
        let l = pair_link(&hub, &key, tokens);
        assert_eq!(l.pin.as_deref(), Some(PIN));
        assert!(!format!("{l:?}").contains(PIN), "no PIN in Debug: {l:?}");

        assert!(matches!(
            link_client(&l, &no_prompt),
            Err(OpenError::NotConfirmed { .. })
        ));
        let no = Person::new(None, Some(false));
        assert!(matches!(
            link_client(&l, &no),
            Err(OpenError::NotConfirmed { .. })
        ));
        assert_eq!(l.stored_token(&hid()), None);

        let yes = Person::new(None, Some(true));
        let client = link_client(&l, &yes).unwrap();
        client.close();
        let q = yes.questions();
        assert_eq!(q.len(), 1);
        assert!(
            q[0].contains(&pin_display(&key.pin())),
            "the fingerprint: {q:?}"
        );
        assert!(yes.asked().is_empty(), "the link's PIN, not a typed one");
        let stored = Credential::parse(&l.stored_token(&hid()).unwrap());
        assert_eq!(stored.pin, Some(key.pin()));
        assert!(!stored.token.is_empty());
        // As reached, the link is spent
        let reached = l.reached_with(&hid(), true);
        assert!(reached.pair_link.is_none() && reached.pin.is_none());
    }

    #[test]
    fn wp_sectm_a_bad_pair_link_is_refused_without_repeating_it() {
        for bad in [
            "dspi://pair?h=x",
            "dspi://pair?h=abc&a=host:1&k=AAAA&p=987654",
            "dspi://other?h=abc",
        ] {
            let e = LinkTarget::from_pair_link(bad).unwrap_err();
            assert!(!e.contains("987654") && !e.contains("dspi://"), "{e}");
        }
    }

    #[test]
    fn wp_sectm_reached_says_whether_the_session_is_encrypted() {
        let hub = pin_hub();
        let tokens = scratch("reached");
        let person = Person::new(Some(PIN), None);
        let plain = Target::Link(link(&hub, tokens.clone()))
            .connect(&person)
            .unwrap();
        let reached = plain.link.clone().unwrap();
        assert_eq!(reached.encrypted, Some(false));
        assert!(Target::Link(reached).via().ends_with(" (unsecured)"));
        drop(plain);

        let key = FakeTlsKey::generate();
        hub.set_tls(Some(&key));
        let tls = Target::Link(link(&hub, tokens)).connect(&person).unwrap();
        let reached = tls.link.clone().unwrap();
        assert_eq!(reached.encrypted, Some(true));
        assert!(!Target::Link(reached).via().contains("unsecured"));
    }

    // -- WP SECFIX --
    /// A plaintext hub on this machine is "local": no badge on the PIN prompt
    /// or the connection, as the web shows it; the same hub off the "local"
    /// rule's addresses is unsecured (the test above).
    #[test]
    fn wp_secfix_a_plaintext_hub_on_loopback_is_local_not_unsecured() {
        let hub = FakeHub::start(HubConfig {
            auth: Auth::Fixed(PIN.into()),
            ..HubConfig::default()
        })
        .unwrap();
        assert!(hub.addr().ip().is_loopback());
        let tokens = scratch("secfix_local");
        let person = Person::new(Some(PIN), None);
        let plain = Target::Link(link(&hub, tokens)).connect(&person).unwrap();
        let reached = plain.link.clone().unwrap();
        assert_eq!(reached.encrypted, Some(false), "not encrypted");
        assert!(!reached.unsecured(), "but local");
        assert_eq!(
            Target::Link(reached).via(),
            format!("Link {}", hub.addr().ip())
        );
        assert!(
            person.asked().iter().all(|a| !a.contains("unsecured")),
            "{:?}",
            person.asked()
        );
        for host in ["localhost", "::1", "hub.localhost"] {
            let l = LinkTarget {
                encrypted: Some(false),
                ..LinkTarget::parse(host).unwrap()
            };
            assert!(!l.unsecured(), "{host}");
        }
        let lan = LinkTarget {
            encrypted: Some(false),
            ..LinkTarget::parse("192.168.1.5").unwrap()
        };
        assert!(lan.unsecured());
    }

    #[test]
    fn wp_sectm_the_badge_is_words_first_colour_second() {
        assert_eq!(with_security("Link h", true), "Link h (unsecured)");
        assert_eq!(with_security("Link h", false), "Link h");
        assert_eq!(
            paint_unsecured("Link h (unsecured)", false),
            "Link h (unsecured)"
        );
        assert_eq!(
            paint_unsecured("Link h (unsecured)", true),
            "Link h\u{1b}[31m (unsecured)\u{1b}[0m"
        );
        assert_eq!(paint_unsecured("Link h", true), "Link h");
        assert!(!colour_allowed(false));
        assert!(parse_yes("y\n") && parse_yes(" YES ") && !parse_yes("") && !parse_yes("n"));
        assert_eq!(
            pairing_label("Studio", false, false, ""),
            "Studio (unsecured)"
        );
        assert_eq!(
            pairing_label("Studio", true, false, "SHA256:K7Q2"),
            "Studio (hub key SHA256:K7Q2)"
        );
        // -- WP SECFIX -- a hub on this machine: neither
        assert_eq!(pairing_label("Studio", false, true, ""), "Studio");
    }

    // -- WP PINX -- (review-fu1 F1, for the Terminal) Records are kept by the
    // hub's id only, never by the typed address, and a dial by address sends no
    // token before the hello names the hub and its record is read.

    /// A hub pinned since its plaintext pairing, back without TLS and asking for
    /// its PIN, dialled by address: no token goes out in plaintext, no PIN is asked.
    #[test]
    fn wp_pinx_a_pinned_hub_at_a_typed_address_gets_no_token_in_plaintext() {
        let hub = pin_hub();
        let tokens = scratch("pinx-stale");
        let (l, _) = paired_then_pinned(&hub, &tokens);
        let seen = hub.tokens_seen().len();
        hub.set_tls(None);
        let there = link(&hub, tokens);
        assert!(there.hub_id.is_none(), "dialled by address");
        let nobody = Person::new(None, None);
        assert!(matches!(
            link_err(link_client(&there, &nobody)),
            LinkError::TlsRefused { .. }
        ));
        assert_eq!(hub.tokens_seen().len(), seen, "no token in plaintext");
        assert!(nobody.asked().is_empty(), "no PIN asked");
        let kept = Credential::parse(&l.stored_token(&hid()).unwrap());
        assert!(
            kept.pin.is_some() && !kept.token.is_empty(),
            "the record kept"
        );
    }

    /// A hub paired in plaintext (a v1 token, never pinned), dialled by address
    /// again: its token goes once the hello names it, and no PIN is asked.
    #[test]
    fn wp_pinx_a_token_only_typed_address_connects_without_a_pin() {
        let hub = pin_hub();
        let tokens = scratch("pinx-v1");
        let l = link(&hub, tokens.clone());
        link_client(&l, &Person::new(Some(PIN), None))
            .unwrap()
            .close();
        let v1 = l.stored_token(&hid()).unwrap();
        assert!(!v1.starts_with("v2."));
        let there = link(&hub, tokens);
        let nobody = Person::new(None, None);
        let client = link_client(&there, &nobody).unwrap();
        assert!(client.tls().is_none());
        client.close();
        assert!(nobody.asked().is_empty(), "no PIN asked");
        assert_eq!(hub.tokens_seen().last(), Some(&v1));
    }
}

// -- WP PGWIRE --
#[cfg(test)]
mod tests_wp_pgwire {
    use super::*;
    use dspi_link::fakehub::{FakeHub, HubConfig};

    /// Every link the TUI opens goes through the generation wrapper: the demo
    /// (a 1.1.6 device) is legacy after the probe, a DSPi Link device is
    /// legacy without one, and neither carries a page afterwards.
    #[test]
    fn wp_pgwire_every_target_opens_through_the_generation_wrapper() {
        let mut demo = Target::Demo.open(&no_prompt).unwrap();
        assert_eq!(demo.generation(), Some(dspi_transport::Generation::Legacy));
        assert!(!demo.carries_pages());
        assert!(matches!(
            demo.control_in_paged(dspi_transport::CmdId(0x0553), 0, 4),
            Err(TransportError::Incompatible(_))
        ));
        drop(demo);

        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let dir = test_dir().join(format!("pgwire-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut l = LinkTarget::parse(&hub.addr().to_string()).unwrap();
        l.tokens = Some(dir.join("tokens.toml"));
        let t = Target::Link(l).open(&no_prompt).unwrap();
        assert_eq!(t.generation(), Some(dspi_transport::Generation::Legacy));
        assert_eq!(t.caps().kind, dspi_transport::TransportKind::Link);
    }
}
