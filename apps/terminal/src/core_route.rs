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
    let url = l.link_url();
    let version = env!("CARGO_PKG_VERSION").to_owned();
    let connect = |token: String, hub_id: String| {
        core.link_connect(
            url.clone(),
            CLIENT_NAME.into(),
            version.clone(),
            token,
            hub_id,
        )
    };
    // A known hub gets its token at once; an unknown one says who it is
    // first, and is dialled again with its token if it has one here.
    let mut hello = match &l.hub_id {
        Some(id) => connect(l.stored_token(id).unwrap_or_default(), id.clone())?,
        None => {
            let first = connect(String::new(), String::new())?;
            match l.stored_token(&first.hub_id) {
                Some(token) if first.needs_pairing => connect(token, first.hub_id.clone())?,
                _ => first,
            }
        }
    };
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
    if hello.needs_pairing {
        let Some(code) = l.pin.clone().or_else(|| pin.pin(&hub, &hello.pin_source)) else {
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
    }
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
    let reached = LinkTarget {
        hub_id: (!hello.hub_id.is_empty()).then(|| hello.hub_id.clone()),
        ..l.clone()
    };
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

/// A session on `opened`'s core with its writes routed there: for the
/// one-shot commands and scripts.
pub(crate) fn session_on(opened: &OpenedCore, confirm: Confirm) -> Result<Session, u8> {
    let transport = CoreTransport::new(Arc::clone(&opened.handle));
    let mut session = connect::session_on(Box::new(transport)).map_err(crate::fail_open)?;
    let ctx = crate::context_for(&session);
    session.set_route(Some(Box::new(CoreRoute::new(
        Arc::clone(&opened.handle),
        Some(ctx),
        confirm,
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
    ))));
    c.link = opened.reached.clone();
    Ok(c)
}

/// Open `target` on a core and connect the interface to it: for a device
/// switch and a reconnect (nobody is asked for a PIN then).
pub(crate) fn open_connected(target: &Target) -> Result<Connected, OpenError> {
    let opened = open_core(target, &Stores::default_dirs(), &connect::no_prompt)
        .map_err(OpenFailure::into_open_error)?;
    connected_on(&opened)
}

/// Open `target` on a core and give a one-shot command its routed session.
pub(crate) fn connect_routed(target: &Target, confirm: Confirm) -> Result<Session, u8> {
    let opened = open_core(target, &Stores::default_dirs(), &crate::prompt_pin)
        .map_err(OpenFailure::report)?;
    session_on(&opened, confirm)
}

/// Open `target` on a core and give `uconsole raw` its transport: every
/// transfer the core's.
pub(crate) fn raw_transport(target: &Target) -> Result<Box<dyn Transport>, u8> {
    let opened = open_core(target, &Stores::default_dirs(), &crate::prompt_pin)
        .map_err(OpenFailure::report)?;
    Ok(Box::new(CoreTransport::new(opened.handle)))
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
}

impl CoreRoute {
    pub(crate) fn new(handle: Arc<CoreHandle>, ctx: Option<Context>, confirm: Confirm) -> Self {
        Self {
            handle,
            ctx,
            confirm,
        }
    }

    fn core(&self) -> &DspiCore {
        &self.handle.core
    }

    /// The token for `op`, as this route's [`Confirm`] says: `None` when
    /// nobody can say yes (the core then refuses it).
    fn token(&self, op: ConfirmOp, path: &str) -> Result<Option<String>, WriteError> {
        match self.confirm {
            Confirm::Given => Ok(Some(self.core().confirm_token(op))),
            Confirm::Refuse => Ok(None),
            Confirm::Ask => {
                let what = by_path(path).map_or(path, |d| d.plain);
                let question = format!("`{path}`: {what}. This cannot be undone. Continue? [y/N] ");
                match crate::pro::ask(&question) {
                    Some(a) if matches!(a.to_ascii_lowercase().as_str(), "y" | "yes") => {
                        Ok(Some(self.core().confirm_token(op)))
                    }
                    _ => Err(WriteError::Routed {
                        reason: format!("{path} was not confirmed; nothing was changed"),
                        action: String::new(),
                        kind: RouteFailure::Refused,
                    }),
                }
            }
        }
    }

    /// A dangerous write, with its token.
    fn gated(
        &self,
        op: ConfirmOp,
        path: &'static str,
        indices: &[u8],
    ) -> Result<Outcome, WriteError> {
        let token = self.token(op, path)?;
        let core = self.core();
        match op {
            ConfirmOp::FactoryReset => core.factory_reset_confirmed(token),
            ConfirmOp::EnterBootloader => core.enter_bootloader_confirmed(token),
            _ => core
                .write_param_confirmed(path.into(), indices.to_vec(), ParamValue::Trigger, token)
                .map(|_| ()),
        }
        .map(|()| Outcome::Triggered)
        .map_err(routed)
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
        let back = self.core().write_param(
            path.into(),
            indices.to_vec(),
            ParamValue::from(value.clone()),
        );
        Some(
            back.map(|v| outcome(path, value, Value::from(v)))
                .map_err(routed),
        )
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
        let core = self.core();
        match packet.band {
            b @ 0..=9 => core.set_filter(packet.channel, b, params),
            b @ 20..=23 => core.set_crossover(packet.channel, b.saturating_sub(20), params),
            b => Err(DspiError::InvalidArgument {
                msg: format!("band {b} is neither a PEQ band (1-10) nor a crossover band"),
            }),
        }
        .map(|()| Outcome::Confirmed(Value::Bytes(packet.encode())))
        .map_err(routed)
    }

    fn step(&mut self, redo: bool) -> Result<Option<RoutedStep>, WriteError> {
        let core = self.core();
        let moved = if redo { core.redo() } else { core.undo() }.map_err(routed)?;
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
    match e {
        // `hub`: the hub holds the lock and refused it (same banner)
        DspiError::InstallerLock { code, .. } if code == "locked" || code == "hub" => {
            dspi_core::notice_for_key(dspi_core::LOCK_BANNER_NOTICE.into(), String::new()).reason
        }
        e => e.to_string(),
    }
}

/// A core error as the session reports a routed write's.
pub(crate) fn routed(e: DspiError) -> WriteError {
    let notice = dspi_core::notice_for(e.clone());
    let action = match &e {
        DspiError::ConfirmationRequired { .. } => {
            "Confirm it on a terminal, or add --yes to the command.".to_owned()
        }
        _ if crate::pro::explained(&notice.key) => notice.action,
        _ => String::new(),
    };
    let kind = match crate::pro::error_exit(&e) {
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
        session_on(o, confirm).expect("a routed session")
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
        assert!(o.handle.core.confirmation_required());
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
}
