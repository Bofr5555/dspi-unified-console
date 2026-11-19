//! The generation wrapper: every connection is opened through [`open`]
//! (WP PGWIRE, PLAN-PG sections 1.1 and 1.2).
//!
//! **The layering rule:** raw wire (USB, USB MIDI, DSPi Link, WebUSB, a fake,
//! a host's adapter), then the generation wrapper [`open`] made, then every
//! other wrapper. [`open`] is called once on the raw transport, before
//! anything else is sent, at every place a connection is built; the probes
//! of `dspi_session` then run unchanged on what it returns.
//!
//! **The probe** (section 1.1 steps 1 to 4), only on a link that carries a
//! page ([`Transport::carries_pages`]):
//!
//! 1. `SYS.PROTOCOL_INFO` (GET `0x0110`, 16 bytes; SETUP `C1 10 00 00 02 01 10
//!    00`). A timeout is retried [`PROBE_RETRIES`] times [`PROBE_BACKOFF`]
//!    apart, then the open fails with that timeout: never a guess.
//! 2. A paged answer: the device is paged. [`PagedTransport::new`] reads the
//!    discovery (`SYS.PLATFORM`, `SYS.PAGE_LIST`, `SYS.PAGE_COMMANDS`; one
//!    batch, WP PGADAPT) and wraps it: every 1.1.6-shaped call above is
//!    planned into paged requests, and nothing on page 0 is ever sent to it.
//! 3. A later major ([`Refusal::NewerProtocol`]) or an answer that is not a
//!    protocol-info block ([`Refusal::Unrecognised`]): refused, nothing else
//!    is sent.
//! 4. A STALL (USB) or status 0x02 (MIDI): a legacy candidate. `GET_PLATFORM`
//!    must answer; then the device is legacy and [`LegacyGuard`] wraps it,
//!    otherwise it is [`Refusal::Unrecognised`].
//!
//! A link that carries no page (DSPi Link until WP PGLINKC, a host-supplied
//! adapter) is legacy without a probe: nothing paged can reach the device.
//!
//! **A STALLed probe is not explained** (WP PGADAPT, PG-A review note 3): a
//! `SYS.LAST_RESULT` read after it would be a paged request other than the
//! probe to what is most likely a 1.1.6 device, which runs opcode 0x14 for
//! it. The busy case is the firmware's to rule out: the probe is never
//! refused BUSY on USB (upstream answers BUSY off USB only, `vc_core.c`;
//! FWPG0's envelope section).
//!
//! The generation belongs to the connection ([`Transport::generation`]); it is
//! decided again on every [`open`].

use std::time::Duration;

use dspi_proto::generated::opcodes as op;
use dspi_proto::paged::{PROTOCOL_INFO_LEN, ProbeReply, ProtocolInfo, SYS_PROTOCOL_INFO};
// -- WP PGADAPT --
use dspi_proto::paged::legacy as lm;
use dspi_proto::paged::{
    Answer, CommandInfo, EXT_PAGE, EXT_TUNNEL, LastResult, LegacyBody, Next, Outcome,
    PAGE_COMMANDS_LEN, PageSet, PagedBody, PagedDir, PagedRequest, Plan, WireAnswer, cmds,
    decode_page_list, plan, registry,
};

pub use dspi_proto::paged::Generation;

use crate::{
    BUSY_BACKOFF, BUSY_RETRIES, CmdId, DeviceDescriptor, InRead, NotificationSource, PagedRead,
    Result, Transport, TransportCaps, TransportError, TransportKind, with_busy_retry,
};

/// How many times a probe that timed out is sent again (PLAN-PG section 1.1
/// step 1: a flash write may blind the device for a moment).
pub const PROBE_RETRIES: usize = 3;

/// The pause before each probe retry.
pub const PROBE_BACKOFF: Duration = BUSY_BACKOFF;

/// `GET_PLATFORM`'s length asked by the legacy check: 8 bytes, the longest
/// reply any firmware gives (PLAN-PG section 1.9); older ones answer 4, 6 or 7.
pub const PLATFORM_ASK: u16 = 8;

/// What a [`LegacyGuard`] answers to a paged call.
pub const LEGACY_REFUSES_PAGES: &str =
    "this device speaks the 1.1.6 protocol: no firmware 1.1.7 (paged) request is sent to it";

/// Why [`open`] refused a device. Each is a [`TransportError::Incompatible`]
/// whose words are [`Refusal::text`]; [`Refusal::of`] gives it back from the
/// error, and [`Refusal::code`] is its notice's code (`transfer.<code>` in
/// `assets/notices/notices.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The device's protocol major is newer than this build's.
    NewerProtocol,
    /// The device answered neither as paged firmware nor as 1.1.6 firmware
    /// (or answered the probe as paged, then a discovery that does not hold
    /// up, WP PGADAPT).
    Unrecognised,
}

impl Refusal {
    /// Every refusal.
    // -- WP PGADAPT -- `PagedNotYet` is gone: a paged device is controlled.
    pub const ALL: [Refusal; 2] = [Refusal::NewerProtocol, Refusal::Unrecognised];

    /// The notice's code: the key is `transfer.<code>`.
    pub const fn code(self) -> &'static str {
        match self {
            Refusal::NewerProtocol => "newer_protocol",
            Refusal::Unrecognised => "unrecognised_device",
        }
    }

    /// The error's words.
    pub const fn text(self) -> &'static str {
        match self {
            Refusal::NewerProtocol => {
                "this device's firmware uses a newer protocol; update this app"
            }
            Refusal::Unrecognised => "unrecognised device",
        }
    }

    /// The error [`open`] fails with.
    pub fn error(self) -> TransportError {
        TransportError::Incompatible(self.text().to_owned())
    }

    /// The refusal whose words are `text` exactly (an error's message, after
    /// a conversion that kept it).
    pub fn of_text(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.text() == text)
    }

    /// The refusal `e` is, if it is one.
    pub fn of(e: &TransportError) -> Option<Self> {
        match e {
            TransportError::Incompatible(text) => Self::of_text(text),
            _ => None,
        }
    }
}

/// Open a connection on `raw`, the raw wire, before anything else is sent:
/// probe the device's generation (module docs) and wrap it in the generation
/// wrapper. Every place that builds a connection calls this once, and every
/// other wrapper goes on top of what it returns.
///
/// Errors: a refusal ([`Refusal`], as [`TransportError::Incompatible`]); the
/// probe's timeout after [`PROBE_RETRIES`] retries; any other transport error
/// of the probe or of the legacy check (a disconnect, a USB error).
pub fn open(mut raw: Box<dyn Transport>) -> Result<Box<dyn Transport>> {
    if !raw.carries_pages() {
        return Ok(Box::new(LegacyGuard::new(raw)));
    }
    match probe(&mut *raw)? {
        Some(reply) => match ProtocolInfo::decode(&reply) {
            ProbeReply::Paged(info) => Ok(Box::new(PagedTransport::new(raw, info)?)),
            ProbeReply::UnknownMajor(major) => {
                log::warn!("the device's protocol major is {major}; refused");
                Err(Refusal::NewerProtocol.error())
            }
            ProbeReply::Garbled => Err(Refusal::Unrecognised.error()),
        },
        None => {
            let kind = raw.caps().kind;
            let platform = with_busy_retry(
                kind,
                || raw.control_in_upto(op::REQ_GET_PLATFORM, 0, PLATFORM_ASK),
                op::REQ_GET_PLATFORM,
            );
            match platform {
                Ok(reply) if !reply.is_empty() => Ok(Box::new(LegacyGuard::new(raw))),
                Ok(_) | Err(TransportError::Stalled { .. }) => Err(Refusal::Unrecognised.error()),
                Err(e) => Err(e),
            }
        }
    }
}

/// Send the probe: its reply, or `None` when the device refused it (a STALL
/// on USB, status 0x02 on MIDI). A timeout is retried; after
/// [`PROBE_RETRIES`] retries it is the error.
fn probe(raw: &mut dyn Transport) -> Result<Option<Vec<u8>>> {
    let mut retries = 0usize;
    loop {
        match raw.control_in_paged(SYS_PROTOCOL_INFO, 0, PROTOCOL_INFO_LEN) {
            Ok(reply) => return Ok(Some(reply)),
            Err(TransportError::Stalled { .. }) => return Ok(None),
            Err(TransportError::TimedOut { opcode, .. }) => {
                if retries >= PROBE_RETRIES {
                    return Err(TransportError::TimedOut { opcode, retries });
                }
                retries = retries.saturating_add(1);
                crate::clock::sleep(PROBE_BACKOFF);
            }
            Err(e) => return Err(e),
        }
    }
}

/// A 1.1.6-generation device (G1s, G1b, the Zero, the sender): page-0
/// requests pass, every paged call is refused here, with nothing on the wire.
/// Made only by [`open`].
pub struct LegacyGuard {
    inner: Box<dyn Transport>,
}

impl LegacyGuard {
    fn new(inner: Box<dyn Transport>) -> Self {
        Self { inner }
    }
}

fn refuse_paged() -> TransportError {
    TransportError::Incompatible(LEGACY_REFUSES_PAGES.into())
}

impl Transport for LegacyGuard {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        self.inner.control_in(opcode, value, len)
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> Result<Vec<u8>> {
        self.inner.control_in_upto(opcode, value, max_len)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        self.inner.control_out(opcode, value, data)
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        self.inner.descriptor()
    }

    fn max_transfer(&self) -> usize {
        self.inner.max_transfer()
    }

    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.inner.notifications()
    }

    fn caps(&self) -> TransportCaps {
        self.inner.caps()
    }

    fn session_id(&self) -> Option<u16> {
        self.inner.session_id()
    }

    fn lock(&mut self, reason: &str, timeout: Duration) -> Result<()> {
        self.inner.lock(reason, timeout)
    }

    fn unlock(&mut self) {
        self.inner.unlock();
    }

    fn control_in_batch(&mut self, reads: &[InRead]) -> Vec<Result<Vec<u8>>> {
        self.inner.control_in_batch(reads)
    }

    /// No page is ever sent to a legacy device.
    fn carries_pages(&self) -> bool {
        false
    }

    fn control_in_paged(&mut self, _id: CmdId, _value: u16, _max_len: u16) -> Result<Vec<u8>> {
        Err(refuse_paged())
    }

    fn control_out_paged(&mut self, _id: CmdId, _value: u16, _data: &[u8]) -> Result<()> {
        Err(refuse_paged())
    }

    fn generation(&self) -> Option<Generation> {
        Some(Generation::Legacy)
    }
}

// -- WP PGADAPT -- the adapter (was PGWIRE's placeholder)

/// The most paged requests one 1.1.6-shaped call may become: the longest
/// recipe (`SET_PREAMP` on an RP2350: the platform, then 8 channels) with
/// room to spare. A plan that has not finished by then is a STALL.
pub const MAX_PLAN_STEPS: usize = 64;

/// What [`PagedTransport`] answers to a page-0 id on its paged calls: our
/// client never sends a 1.1.6-shaped request to a paged device (PLAN-PG
/// section 1.1, the second hard rule).
pub const PAGED_REFUSES_PAGE_0: &str =
    "this device speaks the paged protocol (firmware 1.1.7): no page-0 request is sent to it";

/// What [`open`] read from a paged device right after the probe, kept for the
/// connection (PLAN-PG section 1.1 step 2) and never across connections: the
/// probe's answer, the pages with their revisions and command bitmaps, and
/// `SYS.PLATFORM`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    /// `SYS.PROTOCOL_INFO`, as the probe decoded it.
    pub info: ProtocolInfo,
    /// `SYS.PAGE_LIST` and `SYS.PAGE_COMMANDS` of every listed page.
    pub pages: PageSet,
    /// `SYS.PLATFORM` as answered, [`PLATFORM_ASK`] bytes asked (section
    /// 1.9): the 1.1.6 `GET_PLATFORM` layout. It does not change while the
    /// device runs, so the adapter answers `GET_PLATFORM` from it.
    pub platform: Vec<u8>,
}

impl Discovery {
    /// Whether the device has our extension channel: page `0xE0` with the
    /// tunnel `0xE010` (G2b). On G2s the planner refuses `REQ_EXTENSION`
    /// locally, so the extension probe above sees "no channel" as on G1s.
    pub fn has_extensions(&self) -> bool {
        self.pages.contains(EXT_TUNNEL, PagedDir::Get)
    }
}

/// A paged device (G2s, G2b), made only by [`open`]: the adapter of PLAN-PG
/// section 1.2.
///
/// - **1.1.6-shaped calls** (`control_in`, `control_in_upto`, `control_out`,
///   `control_in_batch`) are planned by `dspi_proto::paged::plan` (PGPROTO's
///   generated rows, PGCUST's custom recipes, the extension row) into paged
///   requests on the raw wire, and the reply is rebuilt in the 1.1.6 shape. A
///   call the device cannot take (its target is not in the discovery
///   bitmaps, our extension channel on G2s, a 1.1.6 refusal) is refused
///   **locally** as [`TransportError::Stalled`], with nothing on the wire, so
///   the probe-and-cache feature detection above keeps working (OD-PGf). A
///   refusal on the wire is a `Stalled` too: the 1.1.6 shape.
/// - **Paged calls** pass through with the same local checks and answer a
///   refusal as [`TransportError::Refused`] with its result code when it is
///   known, else `Stalled`.
/// - **Never sent:** a page-0 request; a GET with `wLength` 0; a SET whose
///   length the vendored registry refuses, while the device's page revision
///   is the vendored one (a higher revision may grow a payload, so the device
///   judges, R8b); a NaN or an infinity in a field the registry types `f32`;
///   a command the device does not list.
/// - **A STALL on the wire** (USB, USB MIDI, the fake) is explained by one
///   read of `SYS.LAST_RESULT`, trusted only when its id, direction and
///   `wValue` match the refused request and its result is a refusal (PLAN-PG
///   S2: the record is per transport, not per host, guide 5.2). It is read
///   for every refused paged call and for a 1.1.6-shaped call whose rebuilt
///   reply depends on the code (a former write-as-read's status byte, a
///   documented zero-fill, every multi-step recipe). A link that names the
///   code itself (DSPi Link, WP PGLINKC) answers `Refused` and is not asked.
/// - **Retries:** only `VC_ERR_BUSY` (0x17), whose request did not run, up to
///   [`BUSY_RETRIES`] times [`BUSY_BACKOFF`] apart. A timeout is the caller's
///   busy window, as on a legacy device ([`with_busy_retry`]).
/// - **Batches** (R3): a batch of reads whose plans are each one paged GET
///   goes to the wire as one [`Transport::control_in_batch_paged`].
pub struct PagedTransport {
    inner: Box<dyn Transport>,
    disc: Discovery,
    /// `SYS.COMMAND_INFO` answers read so far (this connection only).
    command_info: std::collections::BTreeMap<CmdId, CommandInfo>,
}

impl PagedTransport {
    /// The paged connection on `inner`, whose probe answered `info`: reads
    /// the discovery ([`Discovery`]) at once, in one batch where it can.
    ///
    /// Errors: [`Refusal::Unrecognised`] when the device answers the probe
    /// as paged but not `SYS.PLATFORM`, or its discovery does not hold up
    /// (`SYS.PAGE_LIST` names no page `SYS`, or the `SYS` bitmap lacks the
    /// probe it just answered); a link's error (a disconnect, a timeout
    /// after the busy window).
    pub fn new(mut inner: Box<dyn Transport>, info: ProtocolInfo) -> Result<Self> {
        let disc = discover(&mut *inner, info)?;
        Ok(Self {
            inner,
            disc,
            command_info: std::collections::BTreeMap::new(),
        })
    }

    /// What [`open`] read at connect.
    pub fn discovery(&self) -> &Discovery {
        &self.disc
    }

    /// `SYS.COMMAND_INFO` of `id` (guide 4.7), read once per connection and
    /// kept. Errors: [`TransportError::Refused`] for a command the device
    /// does not list; [`TransportError::Malformed`] for a short answer.
    pub fn command_info(&mut self, id: CmdId) -> Result<CommandInfo> {
        if let Some(info) = self.command_info.get(&id) {
            return Ok(*info);
        }
        let listed = self.disc.pages.contains(id, PagedDir::Get)
            || self.disc.pages.contains(id, PagedDir::Set)
            || (id.cmd() == EXT_TUNNEL.cmd() && self.disc.pages.has_page(id.page()));
        if !listed {
            return Err(TransportError::Refused {
                id,
                result: lm::VC_ERR_UNKNOWN,
            });
        }
        let bytes = self.control_in_paged(cmds::SYS_COMMAND_INFO, id.0, COMMAND_INFO_LEN)?;
        let info = CommandInfo::decode(&bytes).ok_or_else(|| {
            TransportError::Malformed(format!(
                "SYS.COMMAND_INFO of 0x{:04X}: {} bytes",
                id.0,
                bytes.len()
            ))
        })?;
        self.command_info.insert(id, info);
        Ok(info)
    }

    /// Why `req` must not be sent, as the result code the firmware would
    /// give (the module's "never sent" list); `None` when it may go.
    fn local_refusal(&self, req: &PagedRequest) -> Option<u8> {
        let id = req.id;
        let dir = req.dir();
        if id.is_legacy() {
            return Some(lm::VC_ERR_UNKNOWN);
        }
        let pages = &self.disc.pages;
        if !pages.contains(id, dir) {
            let other = match dir {
                PagedDir::Get => PagedDir::Set,
                PagedDir::Set => PagedDir::Get,
            };
            return Some(if pages.contains(id, other) {
                lm::VC_ERR_DIRECTION
            } else {
                lm::VC_ERR_UNKNOWN
            });
        }
        match &req.body {
            PagedBody::Get { len: 0 } => Some(lm::VC_ERR_LENGTH),
            PagedBody::Get { .. } => None,
            PagedBody::Set(data) => {
                let judged = pages.revision_is_vendored(id.page());
                let bad_len = registry::command(id).is_some_and(|c| !c.set_len_ok(data.len()));
                if judged && bad_len {
                    Some(lm::VC_ERR_LENGTH)
                } else if registry::has_non_finite(id, data) {
                    Some(lm::VC_ERR_VALUE)
                } else {
                    None
                }
            }
        }
    }

    /// The answer kept from discovery for `req`, if it is one (`SYS.PLATFORM`).
    fn cached(&self, req: &PagedRequest) -> Option<Vec<u8>> {
        match req.body {
            PagedBody::Get { len }
                if req.id == cmds::SYS_PLATFORM
                    && req.value == 0
                    && len <= PLATFORM_ASK
                    && !self.disc.platform.is_empty() =>
            {
                Some(
                    self.disc
                        .platform
                        .iter()
                        .take(usize::from(len))
                        .copied()
                        .collect(),
                )
            }
            _ => None,
        }
    }

    /// Send one paged request: its answer, or its refusal with the result
    /// code when known (`explain`: read `SYS.LAST_RESULT` after a STALL).
    /// `Err` only for a link failure (a disconnect, a timeout, ...).
    fn send(&mut self, req: &PagedRequest, explain: bool) -> Result<WireAnswer> {
        if let Some(code) = self.local_refusal(req) {
            log::debug!(
                "paged 0x{:04X} {:?} refused locally (0x{code:02X})",
                req.id.0,
                req.dir()
            );
            return Ok(WireAnswer::Refused { result: Some(code) });
        }
        if let Some(bytes) = self.cached(req) {
            return Ok(WireAnswer::Data(bytes));
        }
        let mut busy = 0usize;
        loop {
            let sent = match &req.body {
                PagedBody::Get { len } => self
                    .inner
                    .control_in_paged(req.id, req.value, *len)
                    .map(WireAnswer::Data),
                PagedBody::Set(data) => self
                    .inner
                    .control_out_paged(req.id, req.value, data)
                    .map(|()| WireAnswer::Ack),
            };
            let result = match sent {
                Ok(answer) => return Ok(answer),
                Err(TransportError::Refused { result, .. }) => Some(result),
                Err(TransportError::Stalled { .. }) if explain => self.explain(req)?,
                Err(TransportError::Stalled { .. }) => None,
                Err(e) => return Err(e),
            };
            if result == Some(lm::VC_ERR_BUSY) && busy < BUSY_RETRIES {
                busy = busy.saturating_add(1);
                crate::clock::sleep(BUSY_BACKOFF);
                continue;
            }
            return Ok(WireAnswer::Refused { result });
        }
    }

    /// After a STALL of `req`: `SYS.LAST_RESULT`'s code when its record is
    /// about `req` (id, direction, `wValue`) and is a refusal (PLAN-PG S2);
    /// `None` otherwise, or when the read itself is refused. A link that
    /// carries the code itself is not asked.
    fn explain(&mut self, req: &PagedRequest) -> Result<Option<u8>> {
        let kind = self.inner.caps().kind;
        if req.id == cmds::SYS_LAST_RESULT || kind == TransportKind::Link {
            return Ok(None);
        }
        match self
            .inner
            .control_in_paged(cmds::SYS_LAST_RESULT, 0, LAST_RESULT_LEN)
        {
            Ok(bytes) => Ok(LastResult::decode(&bytes)
                .filter(|r| r.matches(req.id, req.dir(), req.value) && r.result != lm::VC_OK)
                .map(|r| r.result)),
            Err(TransportError::Stalled { .. } | TransportError::Refused { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Run `plan` to its outcome.
    fn run(&mut self, mut plan: Plan) -> Result<Outcome> {
        let explain = plan.needs_result();
        let mut next = plan.start();
        for _ in 0..MAX_PLAN_STEPS {
            match next {
                Next::Done(outcome) => return Ok(raise_lost_write(outcome)),
                Next::Send(req) => {
                    // -- review/pgb -- A refused SET is always explained: the
                    // code decides whether it is retried (`VC_ERR_BUSY`) and
                    // whether 1.1.6 would have ignored it too.
                    let explain = explain || matches!(req.body, PagedBody::Set(_));
                    // A NaN or an infinity is never sent and never reported
                    // written: 1.1.6 stored some and ignored others, 1.1.7
                    // stores none, so the 1.1.6-shaped write fails here.
                    if let PagedBody::Set(data) = &req.body
                        && registry::has_non_finite(req.id, data)
                    {
                        return Ok(Outcome::Stall {
                            result: Some(lm::VC_ERR_VALUE),
                        });
                    }
                    let answer = self.send(&req, explain)?;
                    next = plan.feed(answer.as_answer());
                }
            }
        }
        match next {
            Next::Done(outcome) => Ok(raise_lost_write(outcome)),
            Next::Send(_) => Ok(Outcome::Stall { result: None }),
        }
    }

    /// A 1.1.6-shaped request's outcome; a request the planner refuses is a
    /// local STALL.
    fn legacy(&mut self, opcode: u8, value: u16, body: LegacyBody<'_>) -> Result<Outcome> {
        match plan(opcode, value, body, &self.disc.pages) {
            Ok(p) => self.run(p),
            Err(e) => {
                log::debug!("opcode 0x{opcode:02X} wValue 0x{value:04X}: not sent: {e}");
                Ok(Outcome::Stall { result: None })
            }
        }
    }

    /// A 1.1.6-shaped IN read, `exact` as [`Transport::control_in`].
    fn legacy_in(&mut self, opcode: u8, value: u16, len: u16, exact: bool) -> Result<Vec<u8>> {
        let outcome = self.legacy(opcode, value, LegacyBody::In { len })?;
        in_result(opcode, len, exact, outcome)
    }

    /// A paged answer as the paged calls give it.
    fn paged_result(id: CmdId, answer: WireAnswer) -> Result<Vec<u8>> {
        match answer {
            WireAnswer::Data(d) => Ok(d),
            WireAnswer::Ack => Ok(Vec::new()),
            WireAnswer::Refused { result: Some(r) } => {
                Err(TransportError::Refused { id, result: r })
            }
            WireAnswer::Refused { result: None } => {
                Err(TransportError::Stalled { opcode: id.cmd() })
            }
        }
    }
}

/// -- review/pgb -- A write the planner would swallow (1.1.6 ACKed and
/// ignored a rejected OUT) is raised as a STALL when the device refused it
/// for a reason 1.1.6 never had: `VC_ERR_BUSY` still standing after the
/// retries (guide 5.8: the request did not run; 1.1.6 kept the newest band
/// write, it never dropped it) and `VC_ERR_LENGTH` (only a device at a
/// higher page revision judges a length itself, R8b: every such write would
/// be lost). Reporting either as done would lose a write silently.
fn raise_lost_write(outcome: Outcome) -> Outcome {
    match outcome {
        Outcome::Swallowed {
            result: Some(r @ (lm::VC_ERR_BUSY | lm::VC_ERR_LENGTH)),
        } => Outcome::Stall { result: Some(r) },
        other => other,
    }
}

/// `SYS.COMMAND_INFO`'s length (guide 4.7).
const COMMAND_INFO_LEN: u16 = 8;
/// `SYS.LAST_RESULT`'s length (guide 5.2).
const LAST_RESULT_LEN: u16 = 8;

/// The 1.1.6 shape of an IN outcome: a short reply is an error when `exact`.
fn in_result(opcode: u8, len: u16, exact: bool, outcome: Outcome) -> Result<Vec<u8>> {
    let bytes = match outcome {
        Outcome::Reply(b) => b,
        Outcome::Ack | Outcome::Swallowed { .. } => Vec::new(),
        Outcome::Stall { .. } => return Err(TransportError::Stalled { opcode }),
    };
    if exact && bytes.len() < usize::from(len) {
        return Err(TransportError::ShortRead {
            opcode,
            wanted: usize::from(len),
            got: bytes.len(),
        });
    }
    Ok(bytes)
}

/// The discovery of [`PagedTransport::new`]: `SYS.PLATFORM`, `SYS.PAGE_LIST`
/// and `SYS.PAGE_COMMANDS` of every page this build knows plus `0xE0`, in one
/// batch (the bitmaps of an unlisted page are asked and ignored: one round
/// trip instead of two, R3); a listed page this build does not know gets its
/// bitmap in a second batch. Reads that time out are retried through the
/// busy window; nothing else is.
fn discover(raw: &mut dyn Transport, info: ProtocolInfo) -> Result<Discovery> {
    let kind = raw.caps().kind;
    let list_len = u16::from(info.pages).saturating_mul(2).max(2);
    let mut guess: Vec<u8> = registry::PAGES.iter().map(|p| p.page).collect();
    if !guess.contains(&EXT_PAGE) {
        guess.push(EXT_PAGE);
    }
    let mut reads = vec![
        PagedRead::new(cmds::SYS_PLATFORM, 0, PLATFORM_ASK),
        PagedRead::new(cmds::SYS_PAGE_LIST, 0, list_len),
    ];
    reads.extend(guess.iter().map(|p| bitmap_read(*p)));
    let mut answers = raw.control_in_batch_paged(&reads).into_iter();
    let mut next = |raw: &mut dyn Transport, read: &PagedRead| -> Result<Option<Vec<u8>>> {
        let first = answers.next().unwrap_or(Err(TransportError::Stalled {
            opcode: read.id.cmd(),
        }));
        settle(raw, kind, read, first)
    };
    let (platform_read, list_read) = (
        PagedRead::new(cmds::SYS_PLATFORM, 0, PLATFORM_ASK),
        PagedRead::new(cmds::SYS_PAGE_LIST, 0, list_len),
    );
    let platform = next(raw, &platform_read)?.filter(|p| !p.is_empty());
    let list = next(raw, &list_read)?.map(|b| decode_page_list(&b));
    let mut bitmaps = std::collections::BTreeMap::new();
    for (page, read) in guess.iter().zip(reads.iter().skip(2)) {
        if let Some(b) = next(raw, read)? {
            bitmaps.insert(*page, b);
        }
    }
    let (Some(platform), Some(list)) = (platform, list) else {
        return Err(Refusal::Unrecognised.error());
    };
    if !list.iter().any(|p| p.page == cmds::SYS_PAGE_LIST.page()) {
        return Err(Refusal::Unrecognised.error());
    }
    let missing: Vec<PagedRead> = list
        .iter()
        .filter(|p| !guess.contains(&p.page))
        .map(|p| bitmap_read(p.page))
        .collect();
    if !missing.is_empty() {
        let answers = raw.control_in_batch_paged(&missing);
        for (read, first) in missing.iter().zip(answers) {
            if let Some(b) = settle(raw, kind, read, first)? {
                bitmaps.insert(read.value.to_le_bytes()[0], b);
            }
        }
    }
    let pages = PageSet::from_discovery(&list, |p| bitmaps.get(&p).map(Vec::as_slice));
    // The device just answered the probe: its own bitmaps must say so.
    if !pages.contains(SYS_PROTOCOL_INFO, PagedDir::Get) {
        return Err(Refusal::Unrecognised.error());
    }
    Ok(Discovery {
        info,
        pages,
        platform,
    })
}

fn bitmap_read(page: u8) -> PagedRead {
    PagedRead::new(
        cmds::SYS_PAGE_COMMANDS,
        u16::from(page),
        PAGE_COMMANDS_LEN as u16,
    )
}

/// One discovery answer: the bytes, `None` for a refusal, the error of a
/// link failure. A single timeout is retried through the busy window.
fn settle(
    raw: &mut dyn Transport,
    kind: TransportKind,
    read: &PagedRead,
    first: Result<Vec<u8>>,
) -> Result<Option<Vec<u8>>> {
    match first {
        Err(TransportError::TimedOut { retries: 0, .. }) => with_busy_retry(
            kind,
            || match raw.control_in_paged(read.id, read.value, read.len) {
                // A refusal is the answer here, not a busy signal.
                Err(TransportError::Stalled { .. } | TransportError::Refused { .. }) => Ok(None),
                other => other.map(Some),
            },
            read.id.cmd(),
        ),
        Ok(b) => Ok(Some(b)),
        Err(TransportError::Stalled { .. } | TransportError::Refused { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

impl Transport for PagedTransport {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        self.legacy_in(opcode, value, len, true)
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> Result<Vec<u8>> {
        self.legacy_in(opcode, value, max_len, false)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        match self.legacy(opcode, value, LegacyBody::Out(data))? {
            Outcome::Reply(_) | Outcome::Ack => Ok(()),
            Outcome::Swallowed { result } => {
                // 1.1.6 ACKed such a write and ignored it (legacy_map `out_stall`).
                log::debug!("opcode 0x{opcode:02X}: refused ({result:?}), ACKed as 1.1.6 did");
                Ok(())
            }
            Outcome::Stall { .. } => Err(TransportError::Stalled { opcode }),
        }
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        self.inner.descriptor()
    }

    fn max_transfer(&self) -> usize {
        self.inner.max_transfer()
    }

    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.inner.notifications()
    }

    /// The link's, with the chunked bulk form only where the device lists
    /// `STORE.ALL_PARAMS_CHUNK` (what `0xA2`/`0xA3` become).
    fn caps(&self) -> TransportCaps {
        let mut caps = self.inner.caps();
        caps.chunked_bulk = caps.chunked_bulk
            && self
                .disc
                .pages
                .contains(cmds::STORE_ALL_PARAMS_CHUNK, PagedDir::Get);
        caps
    }

    fn session_id(&self) -> Option<u16> {
        self.inner.session_id()
    }

    fn lock(&mut self, reason: &str, timeout: Duration) -> Result<()> {
        self.inner.lock(reason, timeout)
    }

    fn unlock(&mut self) {
        self.inner.unlock();
    }

    /// The reads whose plans are each one paged GET go out as one paged
    /// batch (R3); the others (multi-step recipes, local answers, former
    /// write-as-reads) one at a time. A batched read that came back refused
    /// and whose reply depends on the code is asked again alone, so
    /// `SYS.LAST_RESULT` can explain it (the record holds only the last
    /// request).
    fn control_in_batch(&mut self, reads: &[InRead]) -> Vec<Result<Vec<u8>>> {
        let mut out: Vec<Option<Result<Vec<u8>>>> = vec![None; reads.len()];
        let mut batched: Vec<(usize, Plan)> = Vec::new();
        let mut wire: Vec<PagedRead> = Vec::new();
        for (i, r) in reads.iter().enumerate() {
            let p = match plan(
                r.opcode,
                r.value,
                LegacyBody::In { len: r.len },
                &self.disc.pages,
            ) {
                Ok(p) => p,
                Err(_) => {
                    if let Some(slot) = out.get_mut(i) {
                        *slot = Some(Err(TransportError::Stalled { opcode: r.opcode }));
                    }
                    continue;
                }
            };
            let single = p.single().and_then(|req| match req.body {
                PagedBody::Get { len }
                    if self.local_refusal(req).is_none() && self.cached(req).is_none() =>
                {
                    Some(PagedRead::new(req.id, req.value, len))
                }
                _ => None,
            });
            if let Some(read) = single {
                wire.push(read);
                batched.push((i, p));
            }
        }
        let answers = if wire.is_empty() {
            Vec::new()
        } else {
            self.inner.control_in_batch_paged(&wire)
        };
        let mut answers = answers.into_iter();
        for (i, mut p) in batched {
            let Some(r) = reads.get(i) else { continue };
            let answer = answers
                .next()
                .unwrap_or(Err(TransportError::Stalled { opcode: r.opcode }));
            let result = match answer {
                Ok(bytes) => {
                    let _ = p.start();
                    match p.feed(Answer::Data(&bytes)) {
                        Next::Done(o) => in_result(r.opcode, r.len, r.exact, o),
                        // A one-step plan always finishes on its answer.
                        Next::Send(_) => Err(TransportError::Stalled { opcode: r.opcode }),
                    }
                }
                Err(TransportError::Stalled { .. } | TransportError::Refused { .. })
                    if p.needs_result() =>
                {
                    self.legacy_in(r.opcode, r.value, r.len, r.exact)
                }
                Err(TransportError::Refused { result, .. }) => {
                    let _ = p.start();
                    match p.feed(Answer::Refused {
                        result: Some(result),
                    }) {
                        Next::Done(o) => in_result(r.opcode, r.len, r.exact, o),
                        Next::Send(_) => Err(TransportError::Stalled { opcode: r.opcode }),
                    }
                }
                Err(TransportError::Stalled { .. }) => {
                    Err(TransportError::Stalled { opcode: r.opcode })
                }
                Err(e) => Err(e),
            };
            if let Some(slot) = out.get_mut(i) {
                *slot = Some(result);
            }
        }
        for (i, r) in reads.iter().enumerate() {
            if out.get(i).is_some_and(Option::is_none) {
                let result = self.legacy_in(r.opcode, r.value, r.len, r.exact);
                if let Some(slot) = out.get_mut(i) {
                    *slot = Some(result);
                }
            }
        }
        out.into_iter()
            .map(|r| r.unwrap_or(Err(TransportError::Disconnected)))
            .collect()
    }

    /// Paged requests reach a paged device.
    fn carries_pages(&self) -> bool {
        true
    }

    fn control_in_paged(&mut self, id: CmdId, value: u16, max_len: u16) -> Result<Vec<u8>> {
        if id.is_legacy() {
            return Err(TransportError::Incompatible(PAGED_REFUSES_PAGE_0.into()));
        }
        let req = PagedRequest {
            id,
            value,
            body: PagedBody::Get { len: max_len },
        };
        let answer = self.send(&req, true)?;
        Self::paged_result(id, answer)
    }

    fn control_out_paged(&mut self, id: CmdId, value: u16, data: &[u8]) -> Result<()> {
        if id.is_legacy() {
            return Err(TransportError::Incompatible(PAGED_REFUSES_PAGE_0.into()));
        }
        let req = PagedRequest {
            id,
            value,
            body: PagedBody::Set(data.to_vec()),
        };
        let answer = self.send(&req, true)?;
        Self::paged_result(id, answer).map(|_| ())
    }

    /// The same local checks per read, then one batch on the wire. A
    /// refusal on the wire is a `Stalled` here (the device's record names
    /// only the batch's last request); ask alone for its code.
    fn control_in_batch_paged(&mut self, reads: &[PagedRead]) -> Vec<Result<Vec<u8>>> {
        let mut out: Vec<Option<Result<Vec<u8>>>> = vec![None; reads.len()];
        let mut wire = Vec::new();
        let mut at = Vec::new();
        for (i, r) in reads.iter().enumerate() {
            let req = PagedRequest {
                id: r.id,
                value: r.value,
                body: PagedBody::Get { len: r.len },
            };
            let local = if r.id.is_legacy() {
                Some(Err(TransportError::Incompatible(
                    PAGED_REFUSES_PAGE_0.into(),
                )))
            } else if let Some(code) = self.local_refusal(&req) {
                Some(Err(TransportError::Refused {
                    id: r.id,
                    result: code,
                }))
            } else {
                self.cached(&req).map(Ok)
            };
            match local {
                Some(answer) => {
                    if let Some(slot) = out.get_mut(i) {
                        *slot = Some(answer);
                    }
                }
                None => {
                    wire.push(*r);
                    at.push(i);
                }
            }
        }
        if !wire.is_empty() {
            for (i, answer) in at.into_iter().zip(self.inner.control_in_batch_paged(&wire)) {
                if let Some(slot) = out.get_mut(i) {
                    *slot = Some(answer);
                }
            }
        }
        out.into_iter()
            .map(|r| r.unwrap_or(Err(TransportError::Disconnected)))
            .collect()
    }

    fn generation(&self) -> Option<Generation> {
        Some(Generation::Paged(self.disc.info))
    }
}

#[cfg(test)]
#[path = "paged/tests_wp_pgwire.rs"]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_pgwire;

// -- WP PGADAPT --
#[cfg(test)]
#[path = "paged/tests_wp_pgadapt.rs"]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_pgadapt;
