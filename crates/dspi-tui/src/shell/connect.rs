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
#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub fn forget_token(&self, hub_id: &str) -> std::io::Result<()> {
        self.token_store().forget(hub_id)
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
}

impl<F: Fn(&str, &str) -> Option<String>> Prompt for F {
    fn pin(&self, hub: &str, pin_source: &str) -> Option<String> {
        self(hub, pin_source)
    }
}

/// How the opening functions take a [`Prompt`].
pub type PinPrompt<'a> = &'a dyn Prompt;

/// A prompt that has nobody to ask: a reconnect, or a check.
pub fn no_prompt(_hub: &str, _source: &str) -> Option<String> {
    None
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
        match self {
            Self::Usb { serial: Some(s) } => Ok(Box::new(UsbTransport::open_serial(s)?)),
            Self::Usb { serial: None } => Ok(Box::new(UsbTransport::open_only()?)),
            Self::Demo => Ok(Box::new(FakeDevice::demo())),
            Self::Midi { port } => open_midi(port.as_deref()),
            Self::Link(l) => open_link(l, pin),
        }
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
            Self::Link(l) => format!("Link {}", l.host),
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
pub fn link_client(l: &LinkTarget, pin: PinPrompt<'_>) -> Result<LinkClient, OpenError> {
    let store = l.token_store();
    let lookup = store.clone();
    // -- WP TM9 --
    // An address that went to another hub gets no token, not even its own.
    let asked = l.clone();
    let cfg = SessionConfig {
        token_for: Some(Arc::new(move |hub_id: &str| {
            if asked.is_other_hub(hub_id) {
                None
            } else {
                lookup.get(hub_id)
            }
        })),
        ..SessionConfig::default()
    };
    let opts = ConnectOptions {
        client_name: "DSPi Terminal".into(),
        app: COMMAND.into(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        token: None,
    };
    let saver = Arc::new(TokenSaver {
        store: store.clone(),
        failed: Mutex::default(),
    });
    let events: Arc<dyn LinkEvents> = saver.clone();
    let (client, auth) = LinkClient::connect_with(
        &TcpConnector::default(),
        &l.ws_url(),
        &opts,
        Some(events),
        &cfg,
    )
    .map_err(|e| link_error(l, e))?;
    // -- WP TM9 --
    if l.is_other_hub(&client.hello().hub.id) {
        let found = hub_name(&client, l);
        client.close();
        return Err(OpenError::WrongHub {
            address: l.address(),
            found,
        });
    }
    let pin_source = match auth {
        AuthState::Ready => return Ok(client),
        AuthState::NeedsPairing { pin_source } => pin_source,
        AuthState::TokenRefused { pin_source } => {
            if let Err(e) = store.forget(&client.hello().hub.id) {
                pin.warn(&format!(
                    "could not forget the refused pairing token for {} ({e}); it will be tried again next time",
                    hub_name(&client, l)
                ));
            }
            pin_source
        }
    };
    let paired = pair(l, &client, &pin_source, pin);
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

/// Stores a token the moment the hub issues it, which is also how the `ok` that
/// arrives after [`LinkClient::pair`] has timed out is heard. It runs on the
/// client's reader thread, so failures wait here for whoever can report them.
struct TokenSaver {
    store: TokenStore,
    failed: Mutex<Vec<String>>,
}

impl TokenSaver {
    fn take_failures(&self) -> Vec<String> {
        std::mem::take(&mut *self.failed.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl LinkEvents for TokenSaver {
    fn token_issued(&self, hub_id: &str, token: &str) {
        if let Err(e) = self.store.put(hub_id, token) {
            self.failed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(e.to_string());
        }
    }
}

/// Pair with the PIN, and the token goes to [`TokenSaver`]. After a timeout the
/// session stays open for [`PAIR_GRACE`], in case the answer is only late.
fn pair(
    l: &LinkTarget,
    client: &LinkClient,
    pin_source: &str,
    pin: PinPrompt<'_>,
) -> Result<(), OpenError> {
    let hub = hub_name(client, l);
    let Some(code) = l.pin.clone().or_else(|| pin.pin(&hub, pin_source)) else {
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
    let reached = LinkTarget {
        hub_id: (!id.is_empty()).then_some(id),
        ..l.clone()
    };
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
pub type TargetOpener = dyn Fn(&Target) -> Result<Connected, OpenError> + Send + Sync;

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
        let dial = move |moved: Option<&BrowsedHub>| match (&target, moved) {
            (Target::Link(l), Some(hub)) => open(&Target::Link(l.at(hub))),
            _ => open(&target),
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
