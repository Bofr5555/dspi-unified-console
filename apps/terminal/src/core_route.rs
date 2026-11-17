//! The Terminal's writes through the console core (WP TMCORE).
//!
//! `uconsole set`, `eq`, `undo` and `redo`, scripts, the extension and raw
//! commands and the interface (dspi-tui) all write through a
//! `dspi_session::Session`. Here that session is put on a console core
//! (`DspiCore`) that holds the device, so the same rules apply as in the GUI
//! clients:
//!
//! - the session's transport is [`CoreTransport`]: every transfer is the
//!   core's (`raw_get`, `raw_set`, its image read), so a write by any route
//!   (a pin, the pair map, a control surface, `uconsole raw`) meets the
//!   installer lock where every core write starts;
//! - its registry writes, band writes, undo, redo and undo groups go to
//!   [`CoreRoute`] (`dspi_session::WriteRoute`): the core's typed setters,
//!   so the limits profile refuses a value outside it with the core's words,
//!   the lock refuses with the `installer_lock.banner` row, and the
//!   dangerous ones (a preset save or delete, a save, a factory reset, the
//!   bootloader) run only with a confirmation token, minted once the person
//!   has said yes (`--yes`, the question on a terminal, or the interface's
//!   own dialog);
//! - notifications come from the core's tap, after the core has applied
//!   them to its own mirror.
//!
//! Over DSPi Link the core opens its own DSPi Link session ([`open_core`]):
//! `heldBy: hub`, the hub's guard (P1HA) and its backstop notice apply. The
//! hub still judges every write against its own limits and lock.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use dspi_cmd::{COMMAND, Context};
use dspi_core::{ConfirmOp, DspiCore, DspiError, FilterParams, HistoryEntry, ParamValue};
use dspi_proto::generated::opcodes as op;
use dspi_proto::registry::{Kind, by_path};
use dspi_proto::value::{EqParamPacket, Value};
use dspi_session::{Outcome, RouteFailure, RoutedStep, Session, WriteError, WriteRoute};
use dspi_transport::{
    Attributed, DeviceDescriptor, NotificationSource, Transport, TransportCaps, TransportError,
    TransportKind,
};
use dspi_tui::shell::connect::{self, Connected, LinkTarget, OpenError, Target, printable};

use crate::pro::Stores;

/// The name this console gives a hub.
const CLIENT_NAME: &str = "DSPi Terminal";

/// How long a notification read waits between looks at the core's tap.
const NOTE_POLL: Duration = Duration::from_millis(20);

/// How long a hub's device list may take to arrive after pairing.
const DEVICES_WAIT: Duration = Duration::from_secs(5);

// -- WP ROMODE --
/// `--read-only`: every core this run makes is in the client read-only mode
/// (`DspiCore::set_client_read_only`), so no write reaches a device, by any
/// route (`set`, scripts, `raw`, `ext`, imports, the interface, a flash).
/// A diagnostic for watching a unit that must not change, not a security
/// boundary.
pub(crate) static CLIENT_READ_ONLY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// The `read_only.client` row for `e`, when it is a refusal of this run's
/// `--read-only`.
pub(crate) fn client_read_only_notice(e: &DspiError) -> Option<dspi_core::Notice> {
    (matches!(e, DspiError::ReadOnly)
        && CLIENT_READ_ONLY.load(std::sync::atomic::Ordering::Relaxed))
    .then(|| dspi_core::notice_for_key(dspi_core::NOTICE_READ_ONLY_CLIENT.into(), String::new()))
}

/// Put `core` in the client read-only mode when this run asked for it.
pub(crate) fn apply_read_only(core: &DspiCore) {
    if CLIENT_READ_ONLY.load(std::sync::atomic::Ordering::Relaxed) {
        // A fresh core: no guarded apply can be waiting, so this cannot be refused.
        if let Err(e) = core.set_client_read_only(true) {
            log::warn!("--read-only: {e}");
        }
    }
}

// ─── The core ───────────────────────────────────────────────────────────

/// A core with a device open, shared by the session's transport and route.
/// The last one to go closes the device (and the DSPi Link session).
pub(crate) struct CoreHandle {
    pub core: Arc<DspiCore>,
    /// The device's serial.
    pub serial: String,
    /// The link it was opened over, for the session's transport.
    caps: TransportCaps,
    descriptor: DeviceDescriptor,
}

impl Drop for CoreHandle {
    fn drop(&mut self) {
        if self.caps.kind == TransportKind::Link {
            self.core.link_close();
        }
        self.core.disconnect();
    }
}

/// The core [`open_core`] opened, and over DSPi Link the target as reached
/// (knowing the hub's id).
pub(crate) struct OpenedCore {
    pub handle: Arc<CoreHandle>,
    pub reached: Option<LinkTarget>,
}

/// Why opening failed: the open's own error, or the core's.
#[derive(Debug)]
pub(crate) enum OpenFailure {
    Open(OpenError),
    Core(DspiError),
}

impl From<OpenError> for OpenFailure {
    fn from(e: OpenError) -> Self {
        Self::Open(e)
    }
}

impl From<DspiError> for OpenFailure {
    fn from(e: DspiError) -> Self {
        Self::Core(e)
    }
}

impl OpenFailure {
    /// Say why, with the exit code a script branches on.
    pub(crate) fn report(self) -> u8 {
        match self {
            Self::Open(e) => crate::fail_open(e),
            Self::Core(e) => crate::pro::fail(&e),
        }
    }

    /// As the interface's reconnect wants it.
    fn into_open_error(self) -> OpenError {
        match self {
            Self::Open(e) => e,
            Self::Core(DspiError::Disconnected) => TransportError::Disconnected.into(),
            Self::Core(e) => TransportError::Remote(e.to_string()).into(),
        }
    }
}

/// Open `target` on a new core with this console's folders named and
/// confirmation tokens required (`pro::prepare`): a USB, MIDI or demo device
/// over its transport; a hub's device as the core's own DSPi Link session.
/// The lock state is read (from the hub when it holds one).
pub(crate) fn open_core(
    target: &Target,
    stores: &Stores,
    pin: connect::PinPrompt<'_>,
) -> Result<OpenedCore, OpenFailure> {
    let core = crate::pro::prepare(stores).map_err(|_| {
        OpenFailure::Core(DspiError::Io {
            msg: "the console's folders could not be named".into(),
        })
    })?;
    let (serial, caps, descriptor, reached) = match target {
        Target::Link(l) => {
            let (serial, reached) = open_dlink(&core, l, pin)?;
            let caps = TransportCaps {
                kind: TransportKind::Link,
                chunked_bulk: false,
                remote: true,
            };
            let descriptor = DeviceDescriptor {
                serial: serial.clone(),
                bus_id: format!("link:{}", reached.hub_id.clone().unwrap_or_default()),
                address: 0,
            };
            (serial, caps, descriptor, Some(reached))
        }
        _ => {
            let transport = target.open(pin)?;
            let descriptor = transport.descriptor().clone();
            let caps = transport.caps();
            let serial = descriptor.serial.clone();
            core.open_transport(transport, serial.clone(), 0)?;
            (serial, caps, descriptor, None)
        }
    };
    let _ = core.installer_lock_refresh();
    // The session's notifications come from the tap.
    core.notify_tap_set(true);
    Ok(OpenedCore {
        handle: Arc::new(CoreHandle {
            core,
            serial,
            caps,
            descriptor,
        }),
        reached,
    })
}

/// The core's DSPi Link session to `l`'s hub, authenticated with this
/// console's stored token, `--pin` or the prompt (a token the hub issues is
/// stored as `uconsole` always has), bound to `l`'s device. Answers the
/// device's serial and the target as reached.
fn open_dlink(
    core: &DspiCore,
    l: &LinkTarget,
    pin: connect::PinPrompt<'_>,
) -> Result<(String, LinkTarget), OpenFailure> {
    let hello = dlink_session(core, l, pin)?;
    pick_dlink_device(core, l, &hello)
}

// -- WP P2TM -- split from `open_dlink`, so `uconsole api` has the hub's
// session with no device bound (an inventory, a schedule: no device needed).
/// The core's DSPi Link session to `l`'s hub, authenticated as
/// [`open_dlink`] says, with no device bound yet: the hub's `hello`.
///
/// -- WP SECTM -- The stored record is the core's credential (PLAN-SEC 4.3),
/// saved whenever the core's changes (a pairing over TLS, a `v1` token's
/// first TLS, a rotation). The PIN prompt names the hub key or "unsecured"; a
/// `dspi://pair` link's PIN goes out only after the person's yes. Pairing again
/// is offered after `hub_key_changed`, `hub_key_unverified`, `tls_refused` and
/// `tls_required`, and never otherwise.
///
/// -- WP PINTLS -- Pairing again keeps a hub that was pinned TLS-only (the
/// core's `link_credential_unpaired` record): it pairs over TLS and is pinned
/// anew, or fails with `tls_required_pinned` before any PIN goes out; then
/// forgetting the hub is offered, the only way to an unencrypted pairing.
pub(crate) fn dlink_session(
    core: &DspiCore,
    l: &LinkTarget,
    pin: connect::PinPrompt<'_>,
) -> Result<dspi_core::LinkHello, OpenFailure> {
    let mut hub_id = l.hub_id.clone();
    let mut fresh = false;
    let mut repaired = false;
    loop {
        let (code, msg) = match dlink_attempt(core, l, pin, &mut hub_id, fresh) {
            Err(OpenFailure::Core(DspiError::Link { code, msg })) => (code, msg),
            other => return other,
        };
        let hub = hub_id.clone().unwrap_or_else(|| l.link_address());
        if code == connect::TLS_REQUIRED_PINNED {
            let Some(id) = hub_id.clone() else {
                return Err(DspiError::Link { code, msg }.into());
            };
            if !connect::offer_forget(pin, &hub, &l.link_address()) {
                return Err(DspiError::Link { code, msg }.into());
            }
            if let Err(e) = l.forget_hub(&id) {
                pin.warn(&format!("could not forget {hub} ({e})"));
                return Err(DspiError::Link { code, msg }.into());
            }
            fresh = true;
            repaired = true;
            continue;
        }
        if !(connect::offers_repair(&code) && l.pair_link.is_none() && !repaired)
            || !connect::offer_repair(pin, &hub, &l.link_address(), &code)
        {
            return Err(DspiError::Link { code, msg }.into());
        }
        if let Some(id) = &hub_id
            && let Err(e) = l.drop_pairing(id)
        {
            pin.warn(&format!("could not drop the pairing with {hub} ({e})"));
        }
        fresh = true;
        repaired = true;
    }
}

// -- WP SECTM --
/// [`dlink_session`]'s one try. `hub_id` is filled in once a hello names the
/// hub. `fresh`: pairing again, so no stored token and no link's key; a hub
/// once pinned is still TLS-only (-- WP PINTLS --).
fn dlink_attempt(
    core: &DspiCore,
    l: &LinkTarget,
    pin: connect::PinPrompt<'_>,
    hub_id: &mut Option<String>,
    fresh: bool,
) -> Result<dspi_core::LinkHello, OpenFailure> {
    let url = l.link_url();
    let version = env!("CARGO_PKG_VERSION").to_owned();
    let connect = |credential: String, hub_id: String| {
        core.link_connect_advertised(
            url.clone(),
            CLIENT_NAME.into(),
            version.clone(),
            credential,
            hub_id,
            l.tls_advertised,
        )
    };
    let pair_link = l.pair_link.as_ref().filter(|_| !fresh);
    // -- WP PINTLS -- fresh: what is kept after dropping the pairing
    let stored = |id: &str| {
        if fresh {
            l.unpaired_credential(id)
        } else {
            l.stored_token(id)
        }
    };
    // A pairing link: its key, no token. A known hub gets its credential at
    // once; an unknown one says who it is first, and is dialled again with
    // its credential if it has one here.
    let mut hello = match (pair_link, hub_id.clone()) {
        (Some(hint), id) => connect(hint.credential.clone(), id.unwrap_or_default())?,
        (None, Some(id)) => connect(stored(&id).unwrap_or_default(), id)?,
        (None, None) => {
            let first = connect(String::new(), String::new())?;
            *hub_id = Some(first.hub_id.clone()).filter(|id| !id.is_empty());
            // -- WP PINTLS -- (review F3) also when the answer asks for no PIN
            // and the record says TLS only (pinned, or pinned before): dialled
            // again with it, so no unencrypted or unchecked session opens in
            // that hub's name
            match stored(&first.hub_id) {
                Some(credential)
                    if first.needs_pairing
                        || dspi_core::link::link_credential_tls_only(credential.clone()) =>
                {
                    connect(credential, first.hub_id.clone())?
                }
                _ => first,
            }
        }
    };
    if hub_id.is_none() && !hello.hub_id.is_empty() {
        *hub_id = Some(hello.hub_id.clone());
    }
    if l.names_other_hub(&hello.hub_id) {
        core.link_close();
        return Err(OpenError::WrongHub {
            address: l.link_address(),
            found: printable(&hello.hub_name),
        }
        .into());
    }
    let hub = if hello.hub_name.is_empty() {
        l.link_address()
    } else {
        printable(&hello.hub_name)
    };
    if hello.token_refused
        && let Err(e) = l.forget_token(&hello.hub_id)
    {
        pin.warn(&format!(
                "could not forget the refused pairing token for {hub} ({e}); it will be tried again next time"
            ));
    }
    let mut paired = false;
    if hello.needs_pairing {
        let security = core.link_security();
        let label =
            connect::pairing_label(&hub, security.encrypted, security.local, &security.hub_key);
        let given = match pair_link {
            Some(hint) => {
                if let Err(e) = connect::confirm_pair_link(pin, &hub, &hint.hub_key) {
                    core.link_close();
                    return Err(e.into());
                }
                l.pin.clone()
            }
            None => l.pin.clone(),
        };
        let Some(code) = given.or_else(|| pin.pin(&label, &hello.pin_source)) else {
            core.link_close();
            return Err(OpenError::NeedsPin {
                hub,
                pin_source: hello.pin_source.clone(),
            }
            .into());
        };
        let token = match core.link_pair(code) {
            Ok(t) => t,
            Err(DspiError::Link { code, msg }) if code == "wrong_pin" => {
                core.link_close();
                return Err(OpenError::PinRefused { hub, reason: msg }.into());
            }
            Err(e) => {
                core.link_close();
                return Err(e.into());
            }
        };
        if !token.is_empty()
            && let Err(e) = l.store_token(&hello.hub_id, &token)
        {
            pin.warn(&format!(
                "could not save the pairing token for {hub} ({e}); it will ask for a PIN next time"
            ));
        }
        hello.needs_pairing = false;
        paired = true;
    }
    // The core's credential, saved when it changed (a v1 token pinned, a
    // proven rotation), and the new key said once.
    let security = core.link_security();
    match l.store_credential(&hello.hub_id, &security.credential) {
        Ok(true) if security.hub_key_new && !paired => pin.note(&format!(
            "encrypted connection set up with {hub}; hub key {}",
            security.hub_key
        )),
        Ok(_) => {}
        Err(e) => pin.warn(&format!(
            "could not save the hub key for {hub} ({e}); it is checked again next time"
        )),
    }
    Ok(hello)
}

/// [`open_dlink`]'s second half: bind `l`'s device (or the only one).
fn pick_dlink_device(
    core: &DspiCore,
    l: &LinkTarget,
    hello: &dspi_core::LinkHello,
) -> Result<(String, LinkTarget), OpenFailure> {
    let devices = wait_for_devices(core);
    let online: Vec<_> = devices.iter().filter(|d| d.state != "offline").collect();
    let names: Vec<(String, String)> = online
        .iter()
        .map(|d| (d.serial.clone(), d.name.clone()))
        .collect();
    let chosen = match connect::pick_link_device(&names, l.device.as_deref()) {
        Ok(i) => online.get(i).copied(),
        Err(e) => {
            core.link_close();
            return Err(e.into());
        }
    };
    let Some(device) = chosen else {
        core.link_close();
        return Err(OpenError::Transport(TransportError::NotFound).into());
    };
    if let Err(e) = core.link_select_device(device.handle, device.serial.clone()) {
        core.link_close();
        return Err(e.into());
    }
    // -- WP SECTM -- with what the session runs over
    let reached = l.reached_with(&hello.hub_id, core.link_security().encrypted);
    Ok((device.serial.clone(), reached))
}

/// The hub's devices, waiting a moment for a list that is not in yet (the
/// core reads it again after a pairing).
fn wait_for_devices(core: &DspiCore) -> Vec<dspi_core::LinkDevice> {
    let deadline = std::time::Instant::now().checked_add(DEVICES_WAIT);
    loop {
        let devices = core.link_devices();
        if !devices.is_empty() || deadline.is_none_or(|d| std::time::Instant::now() >= d) {
            return devices;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

// ─── Sessions on the core ───────────────────────────────────────────────

/// How a dangerous write gets its confirmation token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Confirm {
    /// The person has confirmed it already (`--yes`, the interface's dialog).
    Given,
    /// Ask on the terminal, once per write.
    Ask,
    /// Nobody to ask: the core refuses it, and says to add `--yes`.
    Refuse,
}

impl Confirm {
    /// `--yes`, or a person at the terminal, or neither.
    pub(crate) fn for_command(yes: bool) -> Self {
        if yes {
            Self::Given
        } else if crate::pro::interactive() {
            Self::Ask
        } else {
            Self::Refuse
        }
    }
}

// -- WP TMCOREF -- (review tmcore F1)
/// Where a writing command gets the installer's PIN, P1TM's way: never on the
/// command line, never echoed.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum PinSource {
    /// `--installer-pin -`: the line read from standard input at the start.
    Given(String),
    /// `DSPI_INSTALLER_PIN`.
    Env(String),
    /// A person at the terminal: asked, with no echo, when a write needs it.
    Ask,
    /// Nobody to ask: a locked write is refused.
    Nobody,
    /// The interface: its own unlock prompt (`:unlock`).
    Interface,
}

impl std::fmt::Debug for PinSource {
    /// Never the PIN itself.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Given(_) => "Given(..)",
            Self::Env(_) => "Env(..)",
            Self::Ask => "Ask",
            Self::Nobody => "Nobody",
            Self::Interface => "Interface",
        })
    }
}

/// The flag a writing command takes the PIN with (only `-`).
pub(crate) const PIN_FLAG: &str = "--installer-pin";

/// This process's command words, set once by `main` (empty in tests).
pub(crate) static COMMAND_FLAGS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// The PIN source this process settled on: standard input is read once.
static THIS_COMMAND: std::sync::OnceLock<Result<PinSource, u8>> = std::sync::OnceLock::new();

impl PinSource {
    /// The command line's source: `--installer-pin -` (one line of standard
    /// input, read now, before a script is), then `DSPI_INSTALLER_PIN`, then a
    /// person at the terminal. `--installer-pin` with anything but `-` is a
    /// usage error: a PIN on the command line shows in `ps` and the history.
    pub(crate) fn for_command(flags: &[&str]) -> Result<Self, u8> {
        let mut given = false;
        let mut it = flags.iter();
        while let Some(f) = it.next() {
            let value = match f.split_once('=') {
                Some((PIN_FLAG, v)) => Some(v),
                _ if *f == PIN_FLAG => it.next().copied(),
                _ => continue,
            };
            if value != Some("-") {
                return Err(crate::pro::usage(&format!(
                    "{PIN_FLAG} takes only - (the PIN as a line of standard input): on the \
                     command line it would show in `ps` and the shell's history; or set {}",
                    crate::pro::installer::PIN_ENV
                )));
            }
            given = true;
        }
        let usable = |p: String| {
            let p = p.trim_end_matches(['\r', '\n']).to_owned();
            (!p.trim().is_empty()).then_some(p)
        };
        if given {
            let mut line = String::new();
            let read = std::io::stdin().read_line(&mut line);
            return match read.ok().filter(|n| *n > 0).and_then(|_| usable(line)) {
                Some(p) => Ok(Self::Given(p)),
                None => Err(crate::pro::usage(&format!(
                    "{PIN_FLAG} - found no PIN on standard input"
                ))),
            };
        }
        if let Some(p) = std::env::var(crate::pro::installer::PIN_ENV)
            .ok()
            .and_then(usable)
        {
            return Ok(Self::Env(p));
        }
        Ok(if crate::pro::interactive() {
            Self::Ask
        } else {
            Self::Nobody
        })
    }

    /// [`PinSource::for_command`] of this process's command words, settled
    /// once: a script on standard input calls it before reading the script.
    pub(crate) fn for_this_command() -> Result<Self, u8> {
        THIS_COMMAND
            .get_or_init(|| {
                let words = COMMAND_FLAGS.get().cloned().unwrap_or_default();
                let refs: Vec<&str> = words.iter().map(String::as_str).collect();
                Self::for_command(&refs)
            })
            .clone()
    }

    /// The action line for a lock refusal in this process, without reading
    /// standard input (a `lock` or `limits` command reads its own).
    pub(crate) fn action_for_this_command() -> String {
        if let Some(Ok(p)) = THIS_COMMAND.get() {
            return p.action();
        }
        let words = COMMAND_FLAGS.get().cloned().unwrap_or_default();
        let given = words
            .iter()
            .any(|w| w == PIN_FLAG || w.starts_with("--installer-pin="))
            || std::env::var(crate::pro::installer::PIN_ENV).is_ok_and(|p| !p.trim().is_empty());
        if given {
            Self::Env(String::new()).action()
        } else {
            Self::Nobody.action()
        }
    }

    /// The PIN given up front, if there is one.
    fn given(&self) -> Option<&str> {
        match self {
            Self::Given(p) | Self::Env(p) => Some(p),
            _ => None,
        }
    }

    /// What a lock refusal tells the user to do.
    fn action(&self) -> String {
        match self {
            Self::Interface => {
                "Type :unlock and the installer's PIN to unlock this DSPi here.".into()
            }
            Self::Given(_) | Self::Env(_) => {
                "The installer's PIN given did not unlock it; check it, or ask the installer."
                    .into()
            }
            _ => format!(
                "Give the installer's PIN: on a terminal it asks; in a script set {} or pass \
                 {PIN_FLAG} - with the PIN as a line of standard input.",
                crate::pro::installer::PIN_ENV
            ),
        }
    }
}

/// Unlock `core` up front for a writing command, when its device is locked
/// and the PIN was given (or, with `prompt`, when a person can be asked): the
/// P1TM way (`ensure_unlocked`: the refusal window, a wrong PIN, the recovery
/// code). Nothing is asked with no lock.
pub(crate) fn unlock_up_front(core: &DspiCore, pins: &PinSource, prompt: bool) -> Result<(), u8> {
    let st = core.installer_lock_state();
    if !st.locked || st.unlocked {
        return Ok(());
    }
    let pin = match (pins.given(), pins) {
        (Some(p), _) => Some(p.to_owned()),
        (None, PinSource::Ask) if prompt => None,
        _ => return Ok(()),
    };
    let serial = printable(&st.serial);
    let mut current = move || {
        pin.clone().or_else(|| {
            crate::pro::ask_secret(&format!("Installer PIN (or recovery code) for {serial}: "))
        })
    };
    let mut pins = crate::pro::installer::Pins {
        current: Box::new(&mut current),
        new: Box::new(|| Ok(None)),
    };
    crate::pro::installer::ensure_unlocked(core, &mut pins).map(|_| ())
}

/// A session on `opened`'s core with its writes routed there: for the
/// one-shot commands and scripts.
pub(crate) fn session_on(
    opened: &OpenedCore,
    confirm: Confirm,
    pins: PinSource,
) -> Result<Session, u8> {
    let transport = CoreTransport::new(Arc::clone(&opened.handle));
    let mut session = connect::session_on(Box::new(transport)).map_err(crate::fail_open)?;
    let ctx = crate::context_for(&session);
    session.set_route(Some(Box::new(CoreRoute::new(
        Arc::clone(&opened.handle),
        Some(ctx),
        confirm,
        pins,
    ))));
    Ok(session)
}

/// [`session_on`] for the interface: the whole state read, the writes
/// routed, every dangerous one already confirmed by its dialog.
pub(crate) fn connected_on(opened: &OpenedCore) -> Result<Connected, OpenError> {
    let transport = CoreTransport::new(Arc::clone(&opened.handle));
    let mut c = connect::connected_on(Box::new(transport))?;
    let ctx = crate::context_for(&c.session);
    c.session.set_route(Some(Box::new(CoreRoute::new(
        Arc::clone(&opened.handle),
        Some(ctx),
        Confirm::Given,
        PinSource::Interface,
    ))));
    c.link = opened.reached.clone();
    Ok(c)
}

/// Open `target` on a core and connect the interface to it: for a device
/// switch and a reconnect (nobody is asked for a PIN then; `prompt` keeps
/// the warnings).
pub(crate) fn open_connected(
    target: &Target,
    prompt: connect::PinPrompt<'_>,
) -> Result<Connected, OpenError> {
    let opened =
        open_core(target, &Stores::default_dirs(), prompt).map_err(OpenFailure::into_open_error)?;
    connected_on(&opened)
}

/// Open `target` on a core and give a one-shot command its routed session.
/// A given PIN unlocks a locked device first; `prompt`: a person at the
/// terminal is asked up front too (for a command whose writes all need it),
/// otherwise when a write is refused.
pub(crate) fn connect_routed(
    target: &Target,
    confirm: Confirm,
    pins: PinSource,
    prompt: bool,
) -> Result<Session, u8> {
    let opened = open_core(target, &Stores::default_dirs(), &crate::prompt_pin)
        .map_err(OpenFailure::report)?;
    // -- WP TMCOREF2 -- (re-review N4) without `prompt`, every PIN (given or
    // asked) waits for the first write the lock refuses: a stale
    // DSPI_INSTALLER_PIN never refuses a volume write or spends a try
    if prompt {
        unlock_up_front(&opened.handle.core, &pins, true)?;
    }
    session_on(&opened, confirm, pins)
}

/// Open `target` on a core and give `uconsole raw` its transport: every
/// transfer the core's; a locked device is unlocked first as for
/// [`connect_routed`] with `prompt`.
pub(crate) fn raw_transport(target: &Target) -> Result<RawLink, u8> {
    let pins = PinSource::for_this_command()?;
    let opened = open_core(target, &Stores::default_dirs(), &crate::prompt_pin)
        .map_err(OpenFailure::report)?;
    Ok(RawLink {
        transport: Box::new(CoreTransport::new(Arc::clone(&opened.handle))),
        handle: opened.handle,
        pins,
    })
}

// -- WP TMCOREF2 -- (re-review N4)
/// `uconsole raw`'s link on the core, with the PIN kept for later: a raw
/// transfer may be one the lock allows (a volume, the analyser), so the PIN
/// is only used when the lock refuses it ([`RawLink::again`]).
pub(crate) struct RawLink {
    pub transport: Box<dyn Transport>,
    handle: Arc<CoreHandle>,
    pins: PinSource,
}

impl RawLink {
    /// After `e`: when the lock refused it and a PIN can be had (given, or
    /// asked at the terminal), unlock; `Ok(true)` means send it again.
    pub(crate) fn again(&self, e: &TransportError) -> Result<bool, u8> {
        if raw_lock_action(e, &self.pins).is_none() {
            return Ok(false);
        }
        let can = matches!(
            self.pins,
            PinSource::Ask | PinSource::Given(_) | PinSource::Env(_)
        );
        if !can {
            return Ok(false);
        }
        unlock_up_front(&self.handle.core, &self.pins, true)?;
        Ok(!self.handle.core.installer_lock_state().locked)
    }
}

// -- WP TMCOREF -- (review tmcore F4)
/// `uconsole ext vreg set|trial|trial-end|auto|clear` through the core's typed
/// `core_vreg_op`: the installer lock and the history apply, and the CG14
/// rule is the same function's (a raise above 1.20 V needs `word`, exactly
/// `OVERVOLT`; without `--word` a person at the terminal is asked once).
pub(crate) fn vreg_op(
    target: &Target,
    op: dspi_proto::ext::VregOp,
    word: Option<&str>,
    yes: bool,
    ask: &mut dyn FnMut() -> Option<String>,
) -> Result<(), u8> {
    let pins = &PinSource::for_this_command()?;
    use dspi_proto::ext::VregOp as V;
    // -- WP P1TZF -- a SET stores the voltage in flash: a gated call (its
    // token, `vreg_save`), so asked first, or `--yes`
    if matches!(op, V::Set(_)) {
        crate::pro::confirmed(yes, "Store this core voltage in the device's flash?")?;
    }
    let (code, value) = match op {
        V::Set(v) => (0, v),
        V::Trial(v) => (1, v),
        V::TrialEnd => (2, 0),
        V::Auto(on) => (3, u8::from(on)),
        V::Clear => (4, 0),
    };
    let opened = open_core(target, &Stores::default_dirs(), &crate::prompt_pin)
        .map_err(OpenFailure::report)?;
    vreg_on(&opened.handle.core, (code, value), word, ask, pins)
}

/// The token a core-voltage request takes: one for a SET (op 0), minted for
/// this try, once the person said yes (WP P1TZF); none for the others.
fn vreg_token(core: &DspiCore, code: u8) -> Option<String> {
    (code == 0).then(|| core.confirm_token(ConfirmOp::VregSave))
}

/// [`vreg_op`] on an open core: `request` is the core's `(op, value)`.
pub(crate) fn vreg_on(
    core: &DspiCore,
    (code, value): (u8, u8),
    word: Option<&str>,
    ask: &mut dyn FnMut() -> Option<String>,
    pins: &PinSource,
) -> Result<(), u8> {
    unlock_up_front(core, pins, true)?;
    // -- WP P1TZF -- the confirmed form; a token per try (a word refusal spends it)
    let send =
        |w: Option<String>| core.core_vreg_op_confirmed(code, value, w, vreg_token(core, code));
    let sent = match send(word.map(str::to_owned)) {
        Err(DspiError::OvervoltWordRequired { msg }) if word.is_none() => match ask() {
            Some(typed) => send(Some(typed)),
            None => Err(DspiError::OvervoltWordRequired { msg }),
        },
        other => other,
    };
    match sent {
        Ok(()) => Ok(()),
        Err(e @ DspiError::OvervoltWordRequired { .. }) => {
            eprintln!("{COMMAND}: {e}");
            eprintln!(
                "      On a terminal it asks; in a script, pass --word {}. --force does not count.",
                dspi_session::overvolt::WORD
            );
            Err(crate::exit::REJECTED)
        }
        Err(e) => {
            let w = routed(e, pins);
            say(&w);
            Err(crate::write_exit(&w))
        }
    }
}

// ─── The transport ──────────────────────────────────────────────────────

/// A `Transport` whose every transfer is the core's: reads through
/// `raw_get` (the image through `read_image_bytes`), writes through
/// `raw_set`, under the core's lock, read-only state and installer lock.
pub(crate) struct CoreTransport {
    handle: Arc<CoreHandle>,
    /// The image the chunk reads are cut from.
    image: Vec<u8>,
}

impl CoreTransport {
    pub(crate) fn new(handle: Arc<CoreHandle>) -> Self {
        Self {
            handle,
            image: Vec::new(),
        }
    }

    fn core(&self) -> &DspiCore {
        &self.handle.core
    }

    /// One IN transfer, at most `len` bytes back.
    fn get(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        let want = usize::from(len);
        match opcode {
            op::REQ_GET_ALL_PARAMS => {
                let mut image = self.core().read_image_bytes().map_err(transport_error)?;
                self.image = image.clone();
                image.truncate(want);
                Ok(image)
            }
            op::REQ_GET_ALL_PARAMS_CHUNK => {
                if value == 0 || self.image.is_empty() {
                    self.image = self.core().read_image_bytes().map_err(transport_error)?;
                }
                let from = usize::from(value).min(self.image.len());
                let to = from.saturating_add(want).min(self.image.len());
                Ok(self.image.get(from..to).unwrap_or_default().to_vec())
            }
            _ => self
                .core()
                .raw_get(opcode, value, len)
                .map_err(transport_error),
        }
    }
}

impl Transport for CoreTransport {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        let mut reply = self.get(opcode, value, len)?;
        let want = usize::from(len);
        if reply.len() < want {
            return Err(TransportError::ShortRead {
                opcode,
                wanted: want,
                got: reply.len(),
            });
        }
        reply.truncate(want);
        Ok(reply)
    }

    fn control_in_upto(
        &mut self,
        opcode: u8,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        self.get(opcode, value, max_len)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        self.core()
            .raw_set(opcode, value, data.to_vec())
            .map_err(transport_error)
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        &self.handle.descriptor
    }

    fn max_transfer(&self) -> usize {
        usize::from(u16::MAX)
    }

    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        Some(Box::new(CoreNotes {
            handle: Arc::clone(&self.handle),
            queue: VecDeque::new(),
        }))
    }

    /// The link's own, except that the image is the core's to read whole.
    fn caps(&self) -> TransportCaps {
        TransportCaps {
            chunked_bulk: false,
            ..self.handle.caps
        }
    }

    fn session_id(&self) -> Option<u16> {
        if self.handle.caps.kind != TransportKind::Link {
            return None;
        }
        u16::try_from(self.core().link_status().session)
            .ok()
            .filter(|s| *s != 0)
    }
}

/// The core's notifications, once its mirror has them.
struct CoreNotes {
    handle: Arc<CoreHandle>,
    queue: VecDeque<dspi_core::RawNotification>,
}

impl CoreNotes {
    fn fill(&mut self) -> dspi_transport::Result<()> {
        let core = &self.handle.core;
        let _ = core.process_notifications();
        self.queue.extend(core.notify_tap_drain());
        if self.queue.is_empty() && !core.is_connected() {
            return Err(TransportError::Disconnected);
        }
        Ok(())
    }
}

impl NotificationSource for CoreNotes {
    fn read(&mut self, timeout: Duration) -> dspi_transport::Result<Vec<u8>> {
        self.read_attributed(timeout).map(|a| a.packet)
    }

    fn read_attributed(&mut self, timeout: Duration) -> dspi_transport::Result<Attributed> {
        if self.queue.is_empty() {
            self.fill()?;
        }
        if self.queue.is_empty() {
            std::thread::sleep(timeout.min(NOTE_POLL));
            self.fill()?;
        }
        Ok(match self.queue.pop_front() {
            Some(n) => Attributed {
                packet: n.packet,
                origin: n.origin,
            },
            None => Attributed {
                packet: Vec::new(),
                origin: None,
            },
        })
    }
}

/// A core error as a transfer's.
fn transport_error(e: DspiError) -> TransportError {
    match e {
        DspiError::Disconnected => TransportError::Disconnected,
        DspiError::Locked => TransportError::Locked,
        DspiError::Refused { status: 0 } => TransportError::Remote("the device stalled".into()),
        e @ (DspiError::InstallerLock { .. }
        | DspiError::LimitExceeded { .. }
        | DspiError::ConfirmationRequired { .. }
        | DspiError::ImageRefused { .. }
        | DspiError::LimitsUnknown { .. }
        | DspiError::ReadOnly
        | DspiError::Denied) => TransportError::Denied(refusal_words(&e)),
        e => TransportError::Remote(e.to_string()),
    }
}

// ─── The route ──────────────────────────────────────────────────────────

/// Where a session on the core sends its writes (`dspi_session::WriteRoute`).
pub(crate) struct CoreRoute {
    handle: Arc<CoreHandle>,
    /// The device's channel names, for the words of an undo.
    ctx: Option<Context>,
    confirm: Confirm,
    // -- WP TMCOREF --
    /// Where the installer's PIN comes from when a write meets the lock.
    pins: PinSource,
    /// Whether the person was asked for the PIN already (once a command).
    asked: bool,
}

impl CoreRoute {
    pub(crate) fn new(
        handle: Arc<CoreHandle>,
        ctx: Option<Context>,
        confirm: Confirm,
        pins: PinSource,
    ) -> Self {
        Self {
            handle,
            ctx,
            confirm,
            pins,
            asked: false,
        }
    }

    // -- WP TMCOREF --
    /// Run `f` on the core; refused by the installer lock with a person at
    /// the terminal, ask for the PIN once (no echo), unlock and run it again.
    fn attempt<T>(
        &mut self,
        f: impl Fn(&DspiCore) -> Result<T, DspiError>,
    ) -> Result<T, WriteError> {
        let first = f(self.core());
        match first {
            // -- WP TMCOREF2 -- (N4) a given PIN is deferred here too
            Err(e)
                if is_lock(&e)
                    && matches!(
                        self.pins,
                        PinSource::Ask | PinSource::Given(_) | PinSource::Env(_)
                    )
                    && !self.asked =>
            {
                self.asked = true;
                if unlock_up_front(self.core(), &self.pins, true).is_ok()
                    && !self.core().installer_lock_state().locked
                {
                    return f(self.core()).map_err(|e| routed(e, &self.pins));
                }
                Err(routed(e, &self.pins))
            }
            other => other.map_err(|e| routed(e, &self.pins)),
        }
    }

    fn core(&self) -> &DspiCore {
        &self.handle.core
    }

    /// Whether `path`'s write is confirmed, as this route's [`Confirm`]
    /// says: `false` when nobody can say yes (the core then refuses it).
    fn decide(&self, path: &str) -> Result<bool, WriteError> {
        match self.confirm {
            Confirm::Given => Ok(true),
            Confirm::Refuse => Ok(false),
            Confirm::Ask => {
                let what = by_path(path).map_or(path, |d| d.plain);
                let question = format!("`{path}`: {what}. This cannot be undone. Continue? [y/N] ");
                match crate::pro::ask(&question) {
                    Some(a) if matches!(a.to_ascii_lowercase().as_str(), "y" | "yes") => Ok(true),
                    _ => Err(WriteError::Routed {
                        reason: format!("{path} was not confirmed; nothing was changed"),
                        action: String::new(),
                        kind: RouteFailure::Refused,
                    }),
                }
            }
        }
    }

    /// A dangerous write, with a token minted for each try.
    fn gated(
        &mut self,
        op: ConfirmOp,
        path: &'static str,
        indices: &[u8],
    ) -> Result<Outcome, WriteError> {
        let confirmed = self.decide(path)?;
        self.attempt(|core| {
            let token = confirmed.then(|| core.confirm_token(op));
            match op {
                ConfirmOp::FactoryReset => core.factory_reset_confirmed(token),
                ConfirmOp::EnterBootloader => core.enter_bootloader_confirmed(token),
                _ => core
                    .write_param_confirmed(
                        path.into(),
                        indices.to_vec(),
                        ParamValue::Trigger,
                        token,
                    )
                    .map(|_| ()),
            }
        })
        .map(|()| Outcome::Triggered)
    }

    /// The words for a step of the history.
    fn describe(&self, e: &HistoryEntry, redo: bool) -> String {
        if let Some(label) = &e.label {
            return label.clone();
        }
        let value = if redo {
            Some(&e.after)
        } else {
            e.before.as_ref()
        };
        match (by_path(&e.path), value, &self.ctx) {
            (Some(d), Some(v), Some(ctx)) => dspi_cmd::format(
                &dspi_cmd::Command::Set {
                    path: d.path,
                    indices: e.indices.clone(),
                    value: Value::from(v.clone()),
                },
                ctx,
            ),
            _ => e.path.clone(),
        }
    }
}

/// The confirmation a path's write takes, when it is a dangerous one.
fn gate_of(path: &str) -> Option<ConfirmOp> {
    match path {
        "dev.reset" => Some(ConfirmOp::FactoryReset),
        "dev.bootloader" => Some(ConfirmOp::EnterBootloader),
        p => dspi_core::confirm::gated_path(p),
    }
}

impl WriteRoute for CoreRoute {
    fn write(
        &mut self,
        path: &'static str,
        indices: &[u8],
        value: &Value,
    ) -> Option<Result<Outcome, WriteError>> {
        if let Some(op) = gate_of(path) {
            return Some(self.gated(op, path, indices));
        }
        // A path a flow of the core's own writes (the analyser, the signal
        // generator, the control surfaces, ...): the session sends it, over
        // the core's transport, under the lock.
        if dspi_core::write_refusal(path).is_some() {
            return None;
        }
        let back = self.attempt(|core| {
            core.write_param(
                path.into(),
                indices.to_vec(),
                ParamValue::from(value.clone()),
            )
        });
        Some(back.map(|v| outcome(path, value, Value::from(v))))
    }

    fn write_band(&mut self, packet: &EqParamPacket) -> Result<Outcome, WriteError> {
        let params = FilterParams {
            filter_type: packet.filter_type.to_raw(),
            bypass: packet.bypass,
            freq: packet.freq,
            q: packet.q,
            gain: packet.gain_db,
            qp: packet.qp.unwrap_or(0.707),
        };
        let (channel, band) = (packet.channel, packet.band);
        self.attempt(|core| match band {
            b @ 0..=9 => core.set_filter(channel, b, params),
            b @ 20..=23 => core.set_crossover(channel, b.saturating_sub(20), params),
            b => Err(DspiError::InvalidArgument {
                msg: format!("band {b} is neither a PEQ band (1-10) nor a crossover band"),
            }),
        })
        .map(|()| Outcome::Confirmed(Value::Bytes(packet.encode())))
    }

    fn step(&mut self, redo: bool) -> Result<Option<RoutedStep>, WriteError> {
        let moved = self.attempt(|core| if redo { core.redo() } else { core.undo() })?;
        let mut skipped: Vec<String> = moved
            .skipped
            .iter()
            .map(|e| {
                if e.undoable {
                    format!("`{}` has changed since, or has no value to restore", e.path)
                } else {
                    format!("`{}` cannot be undone", e.path)
                }
            })
            .collect();
        let command = moved.entry.as_ref().map(|e| self.describe(e, redo));
        if let Some(note) = moved.note {
            skipped.push(note);
        }
        Ok((command.is_some() || !skipped.is_empty()).then_some(RoutedStep { command, skipped }))
    }

    fn begin_group(&mut self, label: &str) {
        self.core().begin_group(label.to_owned());
    }

    fn end_group(&mut self) {
        self.core().end_group();
    }

    // -- WP TMCOREF2 -- the core's profile for the device, for the
    // interface's whole-document import (held, as `uconsole import` holds it)
    fn limits_profile(&mut self) -> Option<dspi_session::limits::LimitsProfile> {
        let doc = self.core().limits_profile_document();
        if doc.is_empty() {
            return None;
        }
        dspi_session::limits::from_document(&doc).ok()
    }

    // -- WP TMCOREF -- (review tmcore F1) the interface's unlock prompt
    fn unlock(&mut self, pin: &str) -> Option<Result<String, String>> {
        let core = self.core();
        let st = core.installer_lock_state();
        if !st.pin_set {
            return Some(Ok(
                "No installer PIN is set for this DSPi: nothing is locked.".into(),
            ));
        }
        Some(match core.installer_unlock(pin.to_owned()) {
            Ok(st) if st.unlocked && st.recovered => Ok(
                "Unlocked with the recovery code until this connection ends; set a new PIN with \
                 `uconsole lock set-pin`."
                    .into(),
            ),
            Ok(st) if st.unlocked => Ok("Unlocked until this connection ends.".into()),
            Ok(_) => Err(dspi_core::notice_for_key(
                "installer_lock.wrong_pin".into(),
                String::new(),
            )
            .reason),
            Err(e) => Err(e.to_string()),
        })
    }
}

/// What a routed write did, from the value the core read back: a toggle or a
/// choice that did not take is the session's `Rejected`, as before.
fn outcome(path: &str, sent: &Value, back: Value) -> Outcome {
    match by_path(path).map(|d| d.kind) {
        Some(Kind::Trigger) => Outcome::Triggered,
        Some(Kind::Packet) => Outcome::Accepted,
        Some(Kind::Bool | Kind::Choice(_)) if back != *sent => Outcome::Rejected {
            sent: sent.clone(),
            actual: back,
        },
        _ => Outcome::Confirmed(back),
    }
}

/// A core refusal's sentence: the lock's (this console's or the hub's) is its
/// banner row, the same line in every client; any other is the core's own
/// words.
fn refusal_words(e: &DspiError) -> String {
    // -- WP ROMODE --
    if let Some(n) = client_read_only_notice(e) {
        return n.reason;
    }
    match e {
        // `hub`: the hub holds the lock and refused it (same banner)
        DspiError::InstallerLock { code, .. } if code == "locked" || code == "hub" => {
            dspi_core::notice_for_key(dspi_core::LOCK_BANNER_NOTICE.into(), String::new()).reason
        }
        e => e.to_string(),
    }
}

// -- WP TMCOREF --
/// Whether `e` is the installer lock's refusal (this console's, or the hub's).
fn is_lock(e: &DspiError) -> bool {
    matches!(e, DspiError::InstallerLock { code, .. } if code == "locked" || code == "hub")
}

/// A core error as the session reports a routed write's. A lock refusal's
/// action line says how this Terminal takes the PIN (`pins`).
pub(crate) fn routed(e: DspiError, pins: &PinSource) -> WriteError {
    let notice = dspi_core::notice_for(e.clone());
    let action = match &e {
        DspiError::ConfirmationRequired { .. } => {
            "Confirm it on a terminal, or add --yes to the command.".to_owned()
        }
        // -- WP TMCOREF -- the way that exists here, always (review N2)
        e if is_lock(e) => pins.action(),
        _ if crate::pro::explained(&notice.key) => notice.action,
        _ => String::new(),
    };
    let kind = match crate::pro::error_exit(&e) {
        _ if is_lock(&e) => RouteFailure::Locked,
        crate::exit::USAGE => RouteFailure::Input,
        crate::exit::NO_DEVICE => RouteFailure::NoDevice,
        crate::exit::REJECTED => RouteFailure::Refused,
        _ => RouteFailure::Transport,
    };
    WriteError::Routed {
        reason: refusal_words(&e),
        action,
        kind,
    }
}

/// The action line a routed refusal carries, for the one-shot commands.
pub(crate) fn action_of(e: &WriteError) -> Option<&str> {
    match e {
        WriteError::Routed { action, .. } if !action.is_empty() => Some(action),
        _ => None,
    }
}

// -- WP TMCOREF --
/// The action line for a transfer the core's lock refused on the raw path
/// (`uconsole raw out`, the pair map, ...): the lock's banner is its text.
pub(crate) fn raw_lock_action(e: &TransportError, pins: &PinSource) -> Option<String> {
    let banner =
        dspi_core::notice_for_key(dspi_core::LOCK_BANNER_NOTICE.into(), String::new()).reason;
    matches!(e, TransportError::Denied(w) if *w == banner).then(|| pins.action())
}

/// Say a command's own failure about the session's device.
pub(crate) fn say(e: &WriteError) {
    eprintln!("{COMMAND}: {e}");
    if let Some(action) = action_of(e) {
        eprintln!("      {action}");
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests_wp_tmcore {
    use super::*;
    use crate::pro::tests::scratch;
    use dspi_session::limits::{LimitsProfile, OutputLimits};

    /// The demo device on a core with this console's stores in `dir`.
    fn opened(dir: &std::path::Path) -> OpenedCore {
        open_core(&Target::Demo, &Stores::under(dir), &connect::no_prompt)
            .expect("the demo opens on a core")
    }

    fn routed(o: &OpenedCore, confirm: Confirm) -> Session {
        session_on(o, confirm, PinSource::Nobody).expect("a routed session")
    }

    fn profile(max_gain_db: f32, locked: bool) -> String {
        LimitsProfile {
            outputs: vec![OutputLimits {
                output: 0,
                max_gain_db: Some(max_gain_db),
                ..Default::default()
            }],
            locked_pages: if locked {
                vec!["front".into()]
            } else {
                Vec::new()
            },
            ..Default::default()
        }
        .to_document()
    }

    fn gain(o: &OpenedCore, output: usize) -> f32 {
        o.handle.core.get_state().output_gain_db[output]
    }

    fn line(s: &Session, path: &'static str, indices: &[u8], value: Value) -> String {
        dspi_cmd::format(
            &dspi_cmd::Command::Set {
                path,
                indices: indices.to_vec(),
                value,
            },
            &crate::context_for(s),
        )
    }

    #[test]
    fn wp_tmcore_a_write_beyond_the_profile_is_refused_in_the_cores_words() {
        let dir = scratch("tmcore_limit");
        let o = opened(&dir);
        o.handle
            .core
            .limits_apply_profile(profile(0.0, false))
            .unwrap();
        let mut s = routed(&o, Confirm::Refuse);
        // The core's own refusal, word for word, and nothing sent.
        let theirs = o.handle.core.set_output_gain(0, 3.0).unwrap_err();
        let e = s.write("out.gain", &[0], Value::Float(3.0)).unwrap_err();
        assert_eq!(e.to_string(), theirs.to_string());
        assert!(matches!(
            e,
            WriteError::Routed {
                kind: RouteFailure::Refused,
                ..
            }
        ));
        assert_eq!(crate::write_exit(&e), crate::exit::REJECTED);
        assert!(gain(&o, 0) <= 0.05);
        // Within it, and on an output it does not hold, the write goes.
        assert!(s.write("out.gain", &[0], Value::Float(-2.0)).is_ok());
        assert!((gain(&o, 0) + 2.0).abs() < 0.05);
        assert!(s.write("out.gain", &[1], Value::Float(4.0)).is_ok());
        assert!((gain(&o, 1) - 4.0).abs() < 0.05);
        // A band past the PEQ boost limit is the core's to refuse too.
        o.handle
            .core
            .limits_apply_profile(
                LimitsProfile {
                    max_peq_boost_db: Some(3.0),
                    ..Default::default()
                }
                .to_document(),
            )
            .unwrap();
        let band = EqParamPacket {
            channel: 0,
            band: 0,
            filter_type: dspi_proto::FilterType::from_raw(1),
            bypass: false,
            freq: 1000.0,
            q: 1.0,
            gain_db: 9.0,
            qp: None,
        };
        let e = s.write_band(&band).unwrap_err();
        assert_eq!(crate::write_exit(&e), crate::exit::REJECTED, "{e}");
        // A dry run stays the session's: nothing reaches the core.
        s.dry_run = true;
        assert!(s.write("out.gain", &[0], Value::Float(3.0)).is_ok());
        assert!(gain(&o, 0) < 0.0);
    }

    #[test]
    fn wp_tmcore_a_write_while_locked_says_the_banner_row() {
        let dir = scratch("tmcore_lock");
        {
            let o = opened(&dir);
            o.handle
                .core
                .limits_apply_profile(profile(6.0, true))
                .unwrap();
            o.handle.core.installer_pin_set("4711-door".into()).unwrap();
        }
        // The next command: locked.
        let o = opened(&dir);
        assert!(o.handle.core.installer_lock_state().locked);
        let mut s = routed(&o, Confirm::Refuse);
        let banner =
            dspi_core::notice_for_key(dspi_core::LOCK_BANNER_NOTICE.into(), String::new()).reason;
        assert!(!banner.is_empty());
        let before = gain(&o, 0);
        let e = s.write("out.gain", &[0], Value::Float(-1.0)).unwrap_err();
        assert_eq!(e.to_string(), banner);
        assert_eq!(crate::write_exit(&e), crate::exit::REJECTED);
        assert_eq!(gain(&o, 0), before);
        // Every other route to the device meets the lock as well: a raw write
        // on the session's transport.
        let raw = s.with_transport(|t| t.control_out(op::REQ_SET_EQ_PARAM, 0, &[0; 16]));
        assert!(
            matches!(raw, Err(TransportError::Denied(ref w)) if *w == banner),
            "{raw:?}"
        );
        // The analyser's own requests pass, as the core's analyser calls do.
        let rta = s.with_transport(|t| t.control_in(op::REQ_RTA_CONTROL, 0, 1));
        assert!(!matches!(rta, Err(TransportError::Denied(_))), "{rta:?}");
        // Volume, mute and a preset recall stay open.
        assert!(s.write("vol.master", &[], Value::Float(-12.0)).is_ok());
        assert!(s.write("out.mute", &[0], Value::Bool(true)).is_ok());
        assert!(s.write("preset.load", &[0], Value::Trigger).is_ok());
        // Unlocked, the write goes.
        o.handle.core.installer_unlock("4711-door".into()).unwrap();
        assert!(s.write("out.gain", &[0], Value::Float(-1.0)).is_ok());
    }

    #[test]
    fn wp_tmcore_a_dangerous_write_needs_its_token() {
        let dir = scratch("tmcore_token");
        let o = opened(&dir);
        // Tokens are required by every core (P1TZ).
        // Nobody to say yes: the core refuses, and the action says how.
        let mut s = routed(&o, Confirm::Refuse);
        let e = s.write("preset.save", &[1], Value::Trigger).unwrap_err();
        assert_eq!(crate::write_exit(&e), crate::exit::REJECTED, "{e}");
        assert!(action_of(&e).is_some_and(|a| a.contains("--yes")), "{e:?}");
        let e = s.write("dev.reset", &[], Value::Trigger).unwrap_err();
        assert_eq!(crate::write_exit(&e), crate::exit::REJECTED, "{e}");
        // `--yes`: the token is minted and spent.
        let mut s = routed(&o, Confirm::Given);
        assert_eq!(
            s.write("preset.save", &[1], Value::Trigger).unwrap(),
            Outcome::Triggered
        );
        assert_eq!(
            s.write("preset.delete", &[1], Value::Trigger).unwrap(),
            Outcome::Triggered
        );
    }

    // -- WP P1TZF -- (review p1tz F1)
    /// The Terminal's own dangerous writes go through the typed confirmed
    /// calls (refused without a yes, done with one); the raw path under its
    /// session (`CoreTransport`) refuses the gated opcodes and still carries
    /// the rest.
    #[test]
    fn wp_p1tzf_dangerous_writes_are_typed_and_the_raw_path_refuses_them() {
        let dir = scratch("p1tzf_route");
        let o = opened(&dir);
        let gated: [(&str, &[u8]); 6] = [
            ("preset.save", &[1]),
            ("preset.delete", &[1]),
            ("dev.save", &[]),
            ("dev.save.io", &[]),
            ("dev.reset", &[]),
            ("dev.bootloader", &[]),
        ];
        let mut s = routed(&o, Confirm::Refuse);
        for (path, indices) in gated {
            let e = s.write(path, indices, Value::Trigger).unwrap_err();
            assert_eq!(crate::write_exit(&e), crate::exit::REJECTED, "{path}: {e}");
        }
        let mut s = routed(&o, Confirm::Given);
        for (path, indices) in &gated[..5] {
            assert_eq!(
                s.write(path, indices, Value::Trigger).unwrap(),
                Outcome::Triggered,
                "{path}"
            );
        }
        // The transport under the session: the gated opcodes never go out raw.
        let mut t = CoreTransport::new(Arc::clone(&o.handle));
        for code in [
            op::REQ_PRESET_SAVE,
            op::REQ_PRESET_DELETE,
            op::REQ_SAVE_PARAMS,
            op::REQ_SAVE_OUTPUT_CONFIG,
            op::REQ_FACTORY_RESET,
            op::REQ_ENTER_BOOTLOADER,
        ] {
            assert!(t.control_in_upto(code, 1, 1).is_err(), "0x{code:02X} in");
            assert!(t.control_out(code, 1, &[0]).is_err(), "0x{code:02X} out");
        }
        assert!(o.handle.core.is_connected());
        // Everything else still goes through it.
        assert!(t.control_in_upto(op::REQ_PRESET_GET_ACTIVE, 0, 1).is_ok());
        assert!(t.control_in_upto(op::REQ_PRESET_LOAD, 0, 1).is_ok());
        let mut s = routed(&o, Confirm::Refuse);
        assert!(s.write("vol.master", &[], Value::Float(-10.0)).is_ok());
    }

    /// `ext vreg set` stores the voltage: a gated call, with a token per try.
    #[test]
    fn wp_p1tzf_a_vreg_set_takes_its_token() {
        let dir = scratch("p1tzf_vreg");
        let o = opened(&dir);
        let core = &o.handle.core;
        let mut never = || -> Option<String> { panic!("asked for the word") };
        core.fetch_core_vreg()
            .expect("the demo has the core-voltage extension");
        // Without a token the core refuses the SET, nothing written.
        assert!(matches!(
            core.core_vreg_op_confirmed(0, 0x0E, Some("OVERVOLT".into()), None),
            Err(DspiError::ConfirmationRequired { .. })
        ));
        // The Terminal's path mints it (the person said yes in `vreg_op`).
        vreg_on(
            core,
            (0, 0x0E),
            Some("OVERVOLT"),
            &mut never,
            &PinSource::Nobody,
        )
        .unwrap();
        assert_eq!(core.fetch_core_vreg().unwrap().stored_sel, 0x0E);
        // AUTO takes no token.
        vreg_on(core, (3, 1), None, &mut never, &PinSource::Nobody).unwrap();
    }

    #[test]
    fn wp_tmcore_cli_undo_and_redo_are_the_cores() {
        let dir = scratch("tmcore_undo");
        let o = opened(&dir);
        let mut s = routed(&o, Confirm::Refuse);
        // A fresh core has nothing to undo, as a fresh session had not.
        assert!(s.undo().unwrap().is_none());
        let before = gain(&o, 2);
        s.write("out.gain", &[2], Value::Float(-7.0)).unwrap();
        let u = s.undo().unwrap().unwrap();
        assert_eq!(
            u.command.as_deref(),
            Some(line(&s, "out.gain", &[2], Value::Float(before)).as_str())
        );
        assert!((gain(&o, 2) - before).abs() < 0.05);
        s.redo().unwrap().unwrap();
        assert!((gain(&o, 2) + 7.0).abs() < 0.05);
    }

    #[test]
    fn wp_tmcore_a_tui_compound_edit_is_one_undo_step() {
        let dir = scratch("tmcore_tui");
        let o = opened(&dir);
        let mut c = connected_on(&o).unwrap();
        let theme = dspi_tui::Theme::console(
            dspi_tui::theme::ColorDepth::TrueColor,
            dspi_tui::theme::Glyphs::Braille,
        );
        let mut live = dspi_tui::live::Live::new(
            c.state,
            theme,
            dspi_tui::perf::Performance::default(),
            Box::new(dspi_tui::live::ConsoleScreens::with_config_path(
                dir.join("config.toml"),
            )),
        );
        let s = &mut c.session;
        let (g1, g2) = (gain(&o, 0), gain(&o, 1));
        let block = format!(
            "{}\n{}\n# Both outputs",
            line(s, "out.gain", &[0], Value::Float(-5.0)),
            line(s, "out.gain", &[1], Value::Float(-6.0)),
        );
        live.run_commands(s, &block);
        assert!((gain(&o, 0) + 5.0).abs() < 0.05);
        assert!((gain(&o, 1) + 6.0).abs() < 0.05);
        // One step in the core's history, named after the gesture.
        let h = o.handle.core.history(2);
        assert!(
            h.iter().all(|e| e.label.as_deref() == Some("Both outputs")),
            "{h:?}"
        );
        // One undo puts both back.
        live.run_command(s, "undo");
        assert!((gain(&o, 0) - g1).abs() < 0.05, "{}", gain(&o, 0));
        assert!((gain(&o, 1) - g2).abs() < 0.05, "{}", gain(&o, 1));
        // A single write stays a step of its own.
        live.run_commands(s, &line(s, "out.gain", &[0], Value::Float(-4.0)));
        live.run_command(s, "undo");
        assert!((gain(&o, 0) - g1).abs() < 0.05);
        assert!((gain(&o, 1) - g2).abs() < 0.05);
    }

    #[test]
    fn wp_tmcore_the_session_reads_through_the_core() {
        let dir = scratch("tmcore_read");
        let o = opened(&dir);
        let mut s = routed(&o, Confirm::Refuse);
        // The image (chunked on USB, whole here) and a plain read.
        let bulk = s.snapshot().unwrap();
        assert!(!bulk.as_bytes().is_empty());
        s.write("out.gain", &[3], Value::Float(-9.0)).unwrap();
        let v = s.read("out.gain", &[3]).unwrap();
        assert!((v.as_f32().unwrap() + 9.0).abs() < 0.05, "{v:?}");
        // The device's notifications come through the core's tap.
        let mut notes = s
            .with_transport(|t| Ok(t.notifications()))
            .unwrap()
            .unwrap();
        let mut seen = false;
        for _ in 0..50 {
            if !notes.read(Duration::from_millis(20)).unwrap().is_empty() {
                seen = true;
                break;
            }
        }
        assert!(seen, "the write's notification came through");
    }

    // ─── WP TMCOREF ─────────────────────────────────────────────────────

    const PIN: &str = "4711-door";

    /// The demo device with this console's PIN set and the lock covering the
    /// front page, opened again: locked.
    fn locked(dir: &std::path::Path) -> OpenedCore {
        {
            let o = opened(dir);
            o.handle
                .core
                .limits_apply_profile(profile(6.0, true))
                .unwrap();
            o.handle.core.installer_pin_set(PIN.into()).unwrap();
        }
        let o = opened(dir);
        assert!(o.handle.core.installer_lock_state().locked);
        o
    }

    /// Review tmcore F1: a writing command takes the installer's PIN P1TM's
    /// way, and the refusal says how.
    #[test]
    fn wp_tmcoref_a_writing_command_takes_the_installers_pin() {
        let dir = scratch("tmcoref_pin");
        let o = locked(&dir);
        // Nobody gave it: refused, and the action line names the ways that exist.
        unlock_up_front(&o.handle.core, &PinSource::Nobody, true).unwrap();
        let mut s = session_on(&o, Confirm::Refuse, PinSource::Nobody).unwrap();
        let e = s.write("out.gain", &[0], Value::Float(-3.0)).unwrap_err();
        assert!(matches!(
            e,
            WriteError::Routed {
                kind: RouteFailure::Locked,
                ..
            }
        ));
        assert_eq!(crate::write_exit(&e), crate::exit::REJECTED);
        let action = action_of(&e).unwrap();
        assert!(
            action.contains("DSPI_INSTALLER_PIN") && action.contains("--installer-pin -"),
            "{action}"
        );
        // A wrong one: refused before anything is written.
        assert_eq!(
            unlock_up_front(&o.handle.core, &PinSource::Env("1234-no".into()), true),
            Err(crate::exit::REJECTED)
        );
        // The right one (DSPI_INSTALLER_PIN, or a line of standard input).
        unlock_up_front(&o.handle.core, &PinSource::Given(PIN.into()), true).unwrap();
        assert!(s.write("out.gain", &[0], Value::Float(-3.0)).is_ok());
        assert!((gain(&o, 0) + 3.0).abs() < 0.05);
        // Never on the command line, never shown.
        for bad in [
            vec!["set", "--installer-pin", PIN],
            vec!["--installer-pin=1234"],
            vec!["--installer-pin"],
        ] {
            assert_eq!(
                PinSource::for_command(&bad),
                Err(crate::exit::USAGE),
                "{bad:?}"
            );
        }
        assert!(!format!("{:?}", PinSource::Given(PIN.into())).contains(PIN));
    }

    /// Review tmcore F1: the interface refuses a locked write with its own
    /// way out, offers the PIN prompt (nothing typed is drawn), and unlocks.
    #[test]
    fn wp_tmcoref_the_interface_unlocks_with_its_prompt() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let dir = scratch("tmcoref_tui");
        let o = locked(&dir);
        let mut c = connected_on(&o).unwrap();
        let theme = dspi_tui::Theme::console(
            dspi_tui::theme::ColorDepth::TrueColor,
            dspi_tui::theme::Glyphs::Braille,
        );
        let mut live = dspi_tui::live::Live::new(
            c.state,
            theme,
            dspi_tui::perf::Performance::default(),
            Box::new(dspi_tui::live::ConsoleScreens::with_config_path(
                dir.join("config.toml"),
            )),
        );
        let s = &mut c.session;
        let before = gain(&o, 0);
        let write = line(s, "out.gain", &[0], Value::Float(-4.0));
        live.run_commands(s, &write);
        assert_eq!(gain(&o, 0), before, "refused");
        let text = |live: &mut dspi_tui::live::Live| {
            dspi_tui::render_frame(140, 45, |area, buf| live.draw(area, buf))
        };
        assert!(text(&mut live).contains("Unlock"), "the prompt is offered");
        for ch in PIN.chars() {
            live.handle_key(s, KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert!(!text(&mut live).contains(PIN), "the PIN is never drawn");
        live.handle_key(s, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!o.handle.core.installer_lock_state().locked, "unlocked");
        live.run_commands(s, &write);
        assert!((gain(&o, 0) + 4.0).abs() < 0.05);
        // A wrong PIN says so and opens nothing.
        let o2 = locked(&scratch("tmcoref_tui2"));
        let mut s2 = session_on(&o2, Confirm::Given, PinSource::Interface).unwrap();
        assert!(s2.unlock("1234-no").unwrap().is_err());
        let e = s2.write("out.gain", &[0], Value::Float(-4.0)).unwrap_err();
        assert!(action_of(&e).unwrap().contains(":unlock"), "{e:?}");
        assert!(s2.unlock(PIN).unwrap().is_ok());
    }

    /// Review tmcore F3: an import's refused writes keep their reasons.
    #[test]
    fn wp_tmcoref_a_locked_filter_import_names_each_refusal() {
        let dir = scratch("tmcoref_import");
        let o = locked(&dir);
        let mut s = session_on(&o, Confirm::Refuse, PinSource::Nobody).unwrap();
        let channel = dspi_session::preset_file::FilterChannel {
            header: "in 1".into(),
            index: Some(0),
            is_output: false,
            preamp_db: Some(-3.0),
            peq: Vec::new(),
            crossover: Vec::new(),
        };
        let mut report = dspi_session::preset_file::FilterReport::default();
        dspi_session::preset_file::apply_filter_channel(&mut s, 0, &channel, &mut report);
        let banner =
            dspi_core::notice_for_key(dspi_core::LOCK_BANNER_NOTICE.into(), String::new()).reason;
        assert_eq!(report.refused, 1);
        assert!(
            report.refusals[0].contains(&banner),
            "{:?}",
            report.refusals
        );
    }

    /// Review tmcore F4: a core-voltage request is the core's typed call:
    /// held by the lock, in the history, the word rule unchanged.
    #[test]
    fn wp_tmcoref_ext_vreg_is_held_by_the_lock() {
        let dir = scratch("tmcoref_vreg");
        let o = locked(&dir);
        let core = &o.handle.core;
        let mut never = || -> Option<String> { panic!("asked for the word") };
        // auto off: refused while locked, nothing journalled.
        assert_eq!(
            vreg_on(core, (3, 0), None, &mut never, &PinSource::Nobody),
            Err(crate::exit::REJECTED)
        );
        assert!(core.history(5).iter().all(|e| e.path != "ext.vreg"));
        // With the PIN it goes, and the history has it.
        vreg_on(core, (3, 0), None, &mut never, &PinSource::Env(PIN.into())).unwrap();
        assert!(core.history(5).iter().any(|e| e.path == "ext.vreg"));
        // A raise to 1.30 V without the word: asked once, refused unanswered.
        let mut asked = 0;
        let mut nobody = || {
            asked += 1;
            None
        };
        assert_eq!(
            vreg_on(core, (0, 0x0F), None, &mut nobody, &PinSource::Nobody),
            Err(crate::exit::REJECTED)
        );
        assert_eq!(asked, 1);
    }

    // ─── WP TMCOREF2 ────────────────────────────────────────────────────

    /// Re-review N4: a PIN given up front waits for the first write the lock
    /// refuses: a stale one never refuses volume or mute, nor spends a try.
    #[test]
    fn wp_tmcoref2_a_given_pin_waits_for_the_lock() {
        let dir = scratch("tmcoref2_defer");
        let o = locked(&dir);
        let tries = |o: &OpenedCore| o.handle.core.installer_lock_state().wrong_tries;
        let mut s = session_on(&o, Confirm::Refuse, PinSource::Env("1234-no".into())).unwrap();
        assert!(s.write("vol.master", &[], Value::Float(-11.0)).is_ok());
        assert!(s.write("out.mute", &[0], Value::Bool(true)).is_ok());
        assert_eq!(tries(&o), 0, "no try spent on writes the lock allows");
        // A locked write tries it, once, and is refused.
        let e = s.write("out.gain", &[0], Value::Float(-3.0)).unwrap_err();
        assert_eq!(crate::write_exit(&e), crate::exit::REJECTED);
        assert_eq!(tries(&o), 1);
        assert!(s.write("out.gain", &[0], Value::Float(-3.0)).is_err());
        assert_eq!(tries(&o), 1, "tried once a command");
        // The right one, deferred the same way: unlocked at the locked write.
        let mut s = session_on(&o, Confirm::Refuse, PinSource::Given(PIN.into())).unwrap();
        assert!(s.write("vol.master", &[], Value::Float(-12.0)).is_ok());
        assert!(o.handle.core.installer_lock_state().locked, "not used yet");
        assert!(s.write("out.gain", &[0], Value::Float(-3.0)).is_ok());
        assert!((gain(&o, 0) + 3.0).abs() < 0.05);
    }

    /// The interface's Import Device Configuration is held to the core's
    /// profile, as `uconsole import` is, and says so.
    #[test]
    fn wp_tmcoref2_the_interface_import_is_held_to_the_profile() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let dir = scratch("tmcoref2_import");
        let o = opened(&dir);
        let core = &o.handle.core;
        core.set_output_gain(0, 6.0).unwrap();
        let doc = core.config_export("held".into(), "t".into(), 0).unwrap();
        let file = dir.join("held.dspipreset");
        std::fs::write(&file, doc).unwrap();
        core.limits_apply_profile(profile(0.0, false)).unwrap();
        assert!(gain(&o, 0) <= 0.05);
        let mut c = connected_on(&o).unwrap();
        let theme = dspi_tui::Theme::console(
            dspi_tui::theme::ColorDepth::TrueColor,
            dspi_tui::theme::Glyphs::Braille,
        );
        let mut live = dspi_tui::live::Live::new(
            c.state,
            theme,
            dspi_tui::perf::Performance::default(),
            Box::new(dspi_tui::live::ConsoleScreens::with_config_path(
                dir.join("config.toml"),
            )),
        );
        let s = &mut c.session;
        live.run_command(s, &format!("import-config {}", file.display()));
        live.handle_key(s, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let frame = dspi_tui::render_frame(160, 50, |area, buf| live.draw(area, buf));
        assert!(frame.contains("Held to the limits profile"), "{frame}");
        assert!(gain(&o, 0) <= 0.05, "held at the limit: {}", gain(&o, 0));
    }

    // -- WP PAIRBAND --
    /// The filter list's write on a linked pair (both sides' lines, one block,
    /// as `FilterList::write` builds it) is one undo step: one undo puts both
    /// sides back, so the pair never shows two different filters.
    #[test]
    fn wp_pairband_a_tui_linked_pair_band_is_one_undo_step() {
        let dir = scratch("pairband_tui");
        let o = opened(&dir);
        let mut c = connected_on(&o).unwrap();
        let theme = dspi_tui::Theme::console(
            dspi_tui::theme::ColorDepth::TrueColor,
            dspi_tui::theme::Glyphs::Braille,
        );
        let mut live = dspi_tui::live::Live::new(
            c.state,
            theme,
            dspi_tui::perf::Performance::default(),
            Box::new(dspi_tui::live::ConsoleScreens::with_config_path(
                dir.join("config.toml"),
            )),
        );
        let s = &mut c.session;
        let band = |ch: usize| o.handle.core.get_state().filters[ch][2];
        let before = (band(0), band(1));
        let block = format!(
            "{}\n{}\n{}\n{}",
            line(s, "eq.freq", &[0, 2], Value::Float(2500.0)),
            line(s, "eq.gain", &[0, 2], Value::Float(-3.0)),
            line(s, "eq.freq", &[1, 2], Value::Float(2500.0)),
            line(s, "eq.gain", &[1, 2], Value::Float(-3.0)),
        );
        live.run_commands(s, &block);
        assert_eq!(band(0).freq, 2500.0);
        assert_eq!(band(1).freq, 2500.0);
        live.run_command(s, "undo");
        assert_eq!((band(0), band(1)), before, "one undo, both sides back");
    }
}

// -- WP SECTM --
#[cfg(test)]
mod tests_wp_sectm {
    use super::*;
    use crate::pro::tests::scratch;
    use dspi_link::credential::{Credential, pin_display, pin_to_b64url};
    use dspi_link::fakehub::{Auth, FakeHub, FakeTlsKey, HubConfig};
    use std::sync::Mutex;

    const PIN: &str = "123456";

    fn pin_hub() -> FakeHub {
        FakeHub::start(HubConfig {
            auth: Auth::Fixed(PIN.into()),
            // -- WP SECFIX -- off the "local" rule's addresses (127.0.0.1, ::1,
            // localhost), still on this machine: a plaintext session here is
            // unsecured, as on a LAN
            bind: "127.0.0.2:0".parse().unwrap(),
            ..HubConfig::default()
        })
        .unwrap()
    }

    fn hid() -> String {
        HubConfig::default().hub_id
    }

    fn link(hub: &FakeHub, name: &str) -> LinkTarget {
        LinkTarget {
            tokens: Some(scratch(name).join("tokens.toml")),
            ..LinkTarget::parse(&hub.addr().to_string()).unwrap()
        }
    }

    #[derive(Default)]
    struct Person {
        pin: Option<String>,
        yes: Option<bool>,
        asked: Mutex<Vec<String>>,
        questions: Mutex<Vec<String>>,
        notes: Mutex<Vec<String>>,
    }

    impl Person {
        fn new(pin: Option<&str>, yes: Option<bool>) -> Self {
            Self {
                pin: pin.map(str::to_owned),
                yes,
                ..Self::default()
            }
        }
    }

    impl connect::Prompt for Person {
        fn pin(&self, hub: &str, _pin_source: &str) -> Option<String> {
            self.asked.lock().unwrap().push(hub.to_owned());
            self.pin.clone()
        }
        fn warn(&self, message: &str) {
            self.notes.lock().unwrap().push(message.to_owned());
        }
        fn confirm(&self, question: &str) -> Option<bool> {
            self.questions.lock().unwrap().push(question.to_owned());
            self.yes
        }
        fn note(&self, message: &str) {
            self.notes.lock().unwrap().push(message.to_owned());
        }
    }

    fn code(r: Result<dspi_core::LinkHello, OpenFailure>) -> String {
        match r {
            Err(OpenFailure::Core(DspiError::Link { code, .. })) => code,
            Err(e) => panic!("expected a Link error, got {e:?}"),
            Ok(_) => panic!("expected a Link error, got a session"),
        }
    }

    /// Paired over plaintext through the core (v1), then TLS on: pinned and
    /// the v2 credential saved.
    fn pinned(hub: &FakeHub, l: &LinkTarget) -> FakeTlsKey {
        let core = DspiCore::new();
        let person = Person::new(Some(PIN), None);
        dlink_session(&core, l, &person).unwrap();
        assert!(!core.link_security().encrypted);
        assert!(!core.link_security().local, "127.0.0.2: not local"); // -- WP SECFIX --
        let mut v = serde_json::json!({ "via": "Link h" });
        crate::pro::link_security_into(&mut v, &core.link_security());
        assert_eq!(v["via"], "Link h (unsecured)");
        assert_eq!(v["link"]["encrypted"], false);
        core.link_close();
        let asked = person.asked.lock().unwrap().clone();
        assert!(asked[0].ends_with(" (unsecured)"), "{asked:?}");
        let v1 = l.stored_token(&hid()).unwrap();
        assert!(!v1.starts_with("v2."));

        let key = FakeTlsKey::generate();
        hub.set_tls(Some(&key));
        let core = DspiCore::new();
        let person = Person::new(None, None);
        dlink_session(&core, l, &person).unwrap();
        let security = core.link_security();
        assert!(security.encrypted);
        let mut v = serde_json::json!({ "via": "Link h" });
        crate::pro::link_security_into(&mut v, &security);
        assert_eq!(v["via"], "Link h");
        assert_eq!(v["link"]["hubKey"], pin_display(&key.pin()));
        core.link_close();
        let v2 = Credential::parse(&l.stored_token(&hid()).unwrap());
        assert_eq!(v2.pin, Some(key.pin()), "saved, pinned");
        assert_eq!(v2.token, v1, "the same token");
        let notes = person.notes.lock().unwrap().clone();
        assert!(
            notes.iter().any(|n| n.contains(&pin_display(&key.pin()))),
            "{notes:?}"
        );
        key
    }

    // -- WP SECFIX --
    /// A plaintext hub on loopback through the core: `local`, so neither the
    /// PIN prompt nor the status says "(unsecured)", and `link.local` says
    /// why; the same hub off loopback is unsecured (`pinned` above).
    #[test]
    fn wp_secfix_core_a_plaintext_hub_on_loopback_is_local() {
        let hub = FakeHub::start(HubConfig {
            auth: Auth::Fixed(PIN.into()),
            ..HubConfig::default()
        })
        .unwrap();
        assert_eq!(hub.addr().ip().to_string(), "127.0.0.1");
        let l = link(&hub, "secfix_local");
        let core = DspiCore::new();
        let person = Person::new(Some(PIN), None);
        dlink_session(&core, &l, &person).unwrap();
        let security = core.link_security();
        assert!(!security.encrypted && security.local, "{security:?}");
        let mut v = serde_json::json!({ "via": "Link h" });
        crate::pro::link_security_into(&mut v, &security);
        assert_eq!(v["via"], "Link h");
        assert_eq!(v["link"]["local"], true);
        assert_eq!(v["link"]["encrypted"], false);
        let asked = person.asked.lock().unwrap().clone();
        assert!(asked.iter().all(|a| !a.contains("unsecured")), "{asked:?}");
        core.link_close();
    }

    #[test]
    fn wp_sectm_core_a_v1_token_is_upgraded_and_saved() {
        let hub = pin_hub();
        let l = link(&hub, "sectm_upgrade");
        pinned(&hub, &l);
    }

    #[test]
    fn wp_sectm_core_another_key_offers_pairing_again_and_only_a_yes_does_it() {
        let hub = pin_hub();
        let l = link(&hub, "sectm_changed");
        pinned(&hub, &l);
        let stored = l.stored_token(&hid()).unwrap();
        let other = FakeTlsKey::generate();
        hub.set_tls(Some(&other));

        let core = DspiCore::new();
        let no = Person::new(Some(PIN), Some(false));
        assert_eq!(code(dlink_session(&core, &l, &no)), "hub_key_changed");
        assert_eq!(no.questions.lock().unwrap().len(), 1);
        assert_eq!(
            l.stored_token(&hid()).as_deref(),
            Some(stored.as_str()),
            "kept"
        );

        let yes = Person::new(Some(PIN), Some(true));
        dlink_session(&core, &l, &yes).unwrap();
        core.link_close();
        let asked = yes.asked.lock().unwrap().clone();
        assert!(asked[0].contains(&pin_display(&other.pin())), "{asked:?}");
        let now = Credential::parse(&l.stored_token(&hid()).unwrap());
        assert_eq!(now.pin, Some(other.pin()), "pinned to the new key");
    }

    /// -- WP PINTLS -- (review F3) by address, an answer that names the hub and
    /// asks for no PIN gets no unencrypted session while the record says TLS only.
    #[test]
    fn wp_pintls_core_an_open_answer_by_address_gets_no_plaintext_session() {
        let hub = pin_hub();
        let l = link(&hub, "pintls_core_open");
        pinned(&hub, &l);
        let impostor = FakeHub::start(HubConfig {
            auth: Auth::None,
            bind: "127.0.0.2:0".parse().unwrap(),
            ..HubConfig::default()
        })
        .unwrap();
        let there = LinkTarget {
            tokens: l.tokens.clone(),
            ..LinkTarget::parse(&impostor.addr().to_string()).unwrap()
        };
        assert!(there.hub_id.is_none(), "dialled by address");
        let core = DspiCore::new();
        let nobody = Person::new(None, None);
        assert_eq!(code(dlink_session(&core, &there, &nobody)), "tls_refused");
        l.drop_pairing(&hid()).unwrap();
        assert_eq!(
            code(dlink_session(&core, &there, &nobody)),
            "tls_required_pinned"
        );
        there.store_credential(&hid(), "v2.dA.short.t").unwrap();
        assert_eq!(
            code(dlink_session(&core, &there, &nobody)),
            "tls_required_pinned"
        );
        assert!(impostor.tokens_seen().is_empty());
        there.forget_hub(&hid()).unwrap();
        dlink_session(&core, &there, &nobody).unwrap();
        assert!(!core.link_security().encrypted);
        core.link_close();
    }

    // -- WP PINTLS -- (owner decision 2026-11-17) a hub once pinned stays TLS-only
    #[test]
    fn wp_pintls_core_pair_again_is_tls_only_and_forget_is_the_way_back() {
        let hub = pin_hub();
        let l = link(&hub, "pintls_core");
        pinned(&hub, &l);
        hub.set_tls(None);
        let seen = hub.tokens_seen().len();

        // Pair Again kept the hub TLS-only: nobody to ask, refused, no PIN asked
        l.drop_pairing(&hid()).unwrap();
        let core = DspiCore::new();
        let nobody = Person::new(None, None);
        assert_eq!(
            code(dlink_session(&core, &l, &nobody)),
            "tls_required_pinned"
        );
        assert!(nobody.asked.lock().unwrap().is_empty(), "no PIN asked");
        let notes = nobody.notes.lock().unwrap().clone();
        assert!(
            notes.iter().any(|n| n.contains("forget the hub")),
            "{notes:?}"
        );
        assert_eq!(hub.tokens_seen().len(), seen);
        assert!(Credential::parse(&l.stored_token(&hid()).unwrap()).pinned_before);

        // A no to Forget: still refused, the record kept
        let no = Person::new(Some(PIN), Some(false));
        assert_eq!(code(dlink_session(&core, &l, &no)), "tls_required_pinned");
        assert!(no.asked.lock().unwrap().is_empty());
        assert!(l.stored_token(&hid()).is_some());

        // A yes: forgotten, then paired unencrypted, the prompt saying so
        let yes = Person::new(Some(PIN), Some(true));
        dlink_session(&core, &l, &yes).unwrap();
        assert!(!core.link_security().encrypted);
        core.link_close();
        let questions = yes.questions.lock().unwrap().clone();
        assert_eq!(questions.len(), 1, "{questions:?}");
        assert!(
            questions[0].contains("Forget the hub entirely"),
            "{questions:?}"
        );
        let asked = yes.asked.lock().unwrap().clone();
        assert!(asked[0].ends_with(" (unsecured)"), "{asked:?}");
        assert!(!l.stored_token(&hid()).unwrap().starts_with("v2."));
    }

    #[test]
    fn wp_pintls_core_pair_again_over_tls_re_pins() {
        let hub = pin_hub();
        let l = link(&hub, "pintls_core_tls");
        pinned(&hub, &l);
        let other = FakeTlsKey::generate();
        hub.set_tls(Some(&other));
        // Pair Again after hub_key_changed: the yes drops the pairing, keeps TLS-only
        let core = DspiCore::new();
        let yes = Person::new(Some(PIN), Some(true));
        dlink_session(&core, &l, &yes).unwrap();
        assert!(core.link_security().encrypted);
        core.link_close();
        let questions = yes.questions.lock().unwrap().clone();
        assert!(
            questions[0].contains("only over an encrypted connection"),
            "{questions:?}"
        );
        let now = Credential::parse(&l.stored_token(&hid()).unwrap());
        assert_eq!(now.pin, Some(other.pin()));
    }

    #[test]
    fn wp_sectm_core_a_pair_link_is_checked_then_confirmed() {
        let key = FakeTlsKey::generate();
        let hub = pin_hub();
        hub.set_tls(Some(&key));
        let text = |k: &FakeTlsKey| {
            format!(
                "dspi://pair?h={}&a={}&k={}&p={PIN}",
                hid(),
                hub.addr(),
                pin_to_b64url(&k.pin())
            )
        };
        let tokens = scratch("sectm_pair_link").join("tokens.toml");
        let with = |t: String| LinkTarget {
            tokens: Some(tokens.clone()),
            ..LinkTarget::from_pair_link(&t).unwrap()
        };
        let core = DspiCore::new();

        // A wrong k=: refused before any question or PIN
        let wrong = with(text(&FakeTlsKey::generate()));
        let person = Person::new(None, Some(true));
        assert_eq!(
            code(dlink_session(&core, &wrong, &person)),
            "hub_key_unverified"
        );
        assert!(person.questions.lock().unwrap().is_empty());
        assert_eq!(wrong.stored_token(&hid()), None);

        // The right one: nothing without a yes, paired and pinned with one
        let right = with(text(&key));
        let no = Person::new(None, Some(false));
        assert!(matches!(
            dlink_session(&core, &right, &no),
            Err(OpenFailure::Open(OpenError::NotConfirmed { .. }))
        ));
        assert_eq!(right.stored_token(&hid()), None);
        let yes = Person::new(None, Some(true));
        dlink_session(&core, &right, &yes).unwrap();
        core.link_close();
        let q = yes.questions.lock().unwrap().clone();
        assert!(q[0].contains(&pin_display(&key.pin())), "{q:?}");
        assert!(yes.asked.lock().unwrap().is_empty(), "the link's PIN");
        let stored = Credential::parse(&right.stored_token(&hid()).unwrap());
        assert_eq!(stored.pin, Some(key.pin()));
    }

    #[test]
    fn wp_sectm_the_badge_words_are_the_core_table() {
        let defaults = connect::UnsecuredWords::default();
        let badge = dspi_core::notice_for_key(connect::NOTICE_UNSECURED.into(), String::new());
        let info = dspi_core::notice_for_key(connect::NOTICE_UNSECURED_INFO.into(), String::new());
        assert_eq!(badge.key, connect::NOTICE_UNSECURED, "the row exists");
        assert_eq!(info.key, connect::NOTICE_UNSECURED_INFO, "the row exists");
        assert_eq!(defaults.badge, badge.title);
        assert_eq!(defaults.info, info.reason);
        assert_eq!(defaults.action, info.action);
    }
}
