//! DSPi Link: one device on a hub, as a [`Transport`].
//!
//! Ported from the Android console's `dspi-ffi/src/link.rs`. Status mapping
//! (Link spec 8.1) onto [`TransportError`]: ERROR/BLOCKED -> `Stalled` (the
//! RTA bands-all fallback depends on it), TIMEOUT -> `TimedOut`, NO_DEVICE ->
//! `Disconnected`, DENIED -> `Denied`, LOCKED -> `Locked`, the rest ->
//! `Remote`. Bulk is single-shot (`caps().chunked_bulk == false`); the first
//! bulk read may come from the hub's snapshot. Notifications carry the
//! writer's session id as their origin.
//!
//! Busy window (spec 8.1): BUSY, BULK_LOCKED and CRC_ERROR are retried here,
//! up to [`BUSY_RETRIES`] times [`BUSY_BACKOFF`] apart, then reported as
//! [`TransportError::TimedOut`]. CRC_ERROR guarantees the command did not run,
//! and BUSY/BULK_LOCKED are refused before it does, so even a SET is safe to
//! repeat.
//!
//! Time bounds (§1.7): every call goes through `dspi-link`, which bounds a
//! command by its own timeouts (10 s, 15 s for bulk) and answers NO_DEVICE
//! once the session is closed, so a hub cannot block a caller forever.
//!
//! -- WP PGLINKC -- DSPi Link 1.2 and 1.3 (PLAN-PG section 1.4, section 1.1
//! steps 5, 6 and 8). [`LinkRoute`] says what the device is, from the hub's
//! minor and the device object: through a 1.2 hub page 0 only and no probe
//! (a refused first `REQ_GET_PLATFORM` is [`LinkNotice::Needs13Hub`]); on a
//! 1.3 hub a device with `protocol` gets paged requests only (`wIndex` =
//! `(page << 8) | 2`, a non-zero GET length, the probe answered from the
//! object), one without it nothing ([`LinkNotice::HubServesPagedOnly`]) unless
//! the hub serves legacy devices (our hub's cap `legacy_devices`). Paged
//! answers retry only 0x01, 0x04, 0x17 and 0x84; the firmware's result codes
//! 0x10..=0x1A are refusals ([`firmware_refusal`]); a `0x86` from a device
//! that answered before (or `0x10` to a page-0 request), or a changed
//! `protocol` in the hub's list,
//! makes the transport stale ([`LinkNotice::GenerationChanged`]): nothing more
//! is sent, and the device is opened again.
//!
//! Owned by WP T5 (docs/core-merge/PLAN.md).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use dspi_link::frames::CmdResponse;
use dspi_link::messages::{DeviceInfo, HelloHub, PollSpec, Proto};
use dspi_link::{CmdSpec, Dir, LinkClient, LinkError, NotifyItem, status};
use dspi_proto::generated::opcodes as op;
// -- WP PGLINKC --
use dspi_proto::paged::legacy::{VC_ERR_BUSY, VC_ERR_PIN_IN_USE as VC_ERR_LAST, VC_ERR_UNKNOWN};
use dspi_proto::paged::{
    LegacyBody, PROTOCOL_INFO_LEN, PROTOCOL_MAGIC, PageSet, PagedBody, PagedRequest,
    SYS_PROTOCOL_INFO, cmds,
};

use crate::paged::PLATFORM_ASK;
use crate::{
    Attributed, BUSY_BACKOFF, BUSY_RETRIES, CmdId, DeviceDescriptor, InRead, NO_PAGES,
    NotificationSource, PagedRead, Result, Transport, TransportCaps, TransportError,
};

/// The hub capability that offers `device.snapshot` (spec 7.5).
const CAP_SNAPSHOT: &str = "snapshot";
/// The hub capability that offers `lock.acquire` / `lock.release` (spec 7.7).
const CAP_LOCK: &str = "lock";
/// Longest lock a client may ask for (spec 7.7; `dspi-link` clamps too).
const LOCK_MAX_MS: u32 = 60_000;
/// What [`Transport::max_transfer`] reports when the hub's `max_payload` does
/// not fit a `usize` (spec 7.2's default).
const MAX_PAYLOAD_DEFAULT: usize = 8192;
/// Bounds for the hub's `max_payload`. Spec 7.2 has hubs advertise at least
/// 8192; a 0 or tiny value would make a chunked reader spin, and a vendor
/// request carries at most `u16::MAX` bytes.
const MAX_PAYLOAD_MIN: usize = 64;
const MAX_PAYLOAD_MAX: usize = 65_535;

/// Map a Link CMD status onto a transport error (`None` for OK).
///
/// `opcode` is the request's `bRequest`, kept in the errors that name one.
/// ERROR and BLOCKED are a STALL: capability probes and the analyser's
/// bands-all fallback rely on that (a bridge answers RTA bands-all ERROR and
/// the chunked bulk opcodes BLOCKED). TIMEOUT means the hub gave up on the
/// device; the command may have run, so it is never retried blindly here.
/// BUSY, BULK_LOCKED and CRC_ERROR map to `Remote` here; [`LinkTransport`]
/// retries them first (its busy window) and never returns them as such.
pub fn map_status(status: u8, opcode: u8) -> Option<TransportError> {
    Some(match status {
        status::OK => return None,
        status::ERROR | status::BLOCKED => TransportError::Stalled { opcode },
        status::TIMEOUT => TransportError::TimedOut { opcode, retries: 0 },
        status::NO_DEVICE => TransportError::Disconnected,
        status::DENIED => TransportError::Denied(format!(
            "opcode 0x{opcode:02X} is not allowed for this client"
        )),
        status::LOCKED => TransportError::Locked,
        s => TransportError::Remote(format!(
            "the hub answered {} (0x{s:02X}) to opcode 0x{opcode:02X}",
            status::name(s)
        )),
    })
}

// -- WP P1HL --
/// [`map_status`] with the answer's payload: a `DENIED` whose payload names one
/// of [`crate::HUB_REFUSALS`] (`{"refused", "msg"}`, a hub's limits profile or
/// installer lock) becomes `Denied("<key>: <the hub's words>")`, which
/// [`TransportError::hub_refusal`] reads back; any other answer maps as before.
pub fn map_response(status: u8, opcode: u8, payload: &[u8]) -> Option<TransportError> {
    if status == status::DENIED
        && let Some((key, limit, msg)) = hub_refusal(payload)
    {
        // -- WP P1DKFIX -- F5: a `limit_exceeded` keeps the limit it names,
        // as `limit_exceeded/<limit>` ([`TransportError::hub_refusal_limit`]).
        return Some(TransportError::Denied(match limit {
            Some(limit) => format!("{key}/{limit}: {msg}"),
            None => format!("{key}: {msg}"),
        }));
    }
    map_status(status, opcode)
}

// -- WP P1HL --
/// The refusal a hub's `DENIED` payload names, the limit a `limit_exceeded`
/// names (lower-case letters and `_` only), and its words made safe to show.
fn hub_refusal(payload: &[u8]) -> Option<(&'static str, Option<String>, String)> {
    if payload.is_empty() || payload.len() > 4096 {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(payload).ok()?;
    let refused = v.get("refused")?.as_str()?;
    let key = crate::HUB_REFUSALS
        .iter()
        .copied()
        .find(|k| *k == refused)?;
    let msg = v
        .get("msg")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let msg: String = dspi_link::sanitize_hub_text(msg)
        .chars()
        .take(300)
        .collect();
    let limit = v
        .get("limit")
        .and_then(serde_json::Value::as_str)
        .filter(|l| key == "limit_exceeded" && crate::is_limit_code(l))
        .map(str::to_owned);
    Some((key, limit, msg))
}

/// Whether a Link status is a busy window to retry (spec 8.1) on a legacy
/// device: BUSY, BULK_LOCKED and CRC_ERROR, as 1.2 says.
fn is_busy(status: u8) -> bool {
    matches!(
        status,
        status::BUSY | status::BULK_LOCKED | status::CRC_ERROR
    )
}

// -- WP PGLINKC --
/// The codes spec 1.3 section 8.1 lets a client resend unchanged, on a paged
/// device: BUSY, BULK_LOCKED, `VC_ERR_BUSY` and RATE_LIMITED. Nothing else.
pub fn is_retryable_13(status: u8) -> bool {
    matches!(
        status,
        status::BUSY | status::BULK_LOCKED | VC_ERR_BUSY | status::RATE_LIMITED
    )
}

/// Run `attempt` through the busy window: retry BUSY, BULK_LOCKED and
/// CRC_ERROR up to [`BUSY_RETRIES`] times, `backoff` apart; after that the
/// request is `TimedOut { retries: BUSY_RETRIES }`. Any other status is
/// mapped by [`map_response`]; OK yields the reply's payload.
#[cfg(test)]
fn busy_window(
    opcode: u8,
    backoff: Duration,
    attempt: impl FnMut() -> CmdResponse,
) -> Result<Vec<u8>> {
    let r = busy_window_with(opcode, backoff, is_busy, attempt)?;
    match map_response(r.status, opcode, &r.payload) {
        Some(e) => Err(e),
        None => Ok(r.payload),
    }
}

/// Run `attempt` through the busy window: retry what `busy` names up to
/// [`BUSY_RETRIES`] times, `backoff` apart; after that the request is
/// `TimedOut { retries: BUSY_RETRIES }`. Any other answer is returned as it
/// came, for the caller to settle.
fn busy_window_with(
    opcode: u8,
    backoff: Duration,
    busy: fn(u8) -> bool,
    mut attempt: impl FnMut() -> CmdResponse,
) -> std::result::Result<CmdResponse, TransportError> {
    for i in 0..=BUSY_RETRIES {
        let r = attempt();
        if !busy(r.status) {
            return Ok(r);
        }
        if i < BUSY_RETRIES {
            crate::clock::sleep(backoff);
        }
    }
    Err(TransportError::TimedOut {
        opcode,
        retries: BUSY_RETRIES,
    })
}

/// A `lock.acquire` failure as a transport error.
fn map_link_error(e: LinkError) -> TransportError {
    match e {
        LinkError::Rejected { code, .. } if code == "locked" => TransportError::Locked,
        LinkError::Rejected { code, msg } if code == "denied" => {
            TransportError::Denied(dspi_link::sanitize_hub_text(&msg))
        }
        LinkError::Closed { .. } => TransportError::Disconnected,
        // A control-plane request has no opcode
        LinkError::Timeout => TransportError::TimedOut {
            opcode: 0,
            retries: 0,
        },
        other => TransportError::Remote(other.to_string()),
    }
}

/// The hub's `max_payload` as a transfer size, clamped to
/// [`MAX_PAYLOAD_MIN`]..=[`MAX_PAYLOAD_MAX`].
fn clamp_max_payload(max_payload: u32) -> usize {
    usize::try_from(max_payload).map_or(MAX_PAYLOAD_DEFAULT, |n| {
        n.clamp(MAX_PAYLOAD_MIN, MAX_PAYLOAD_MAX)
    })
}

// -- WP PGLINKC --
/// What a DSPi Link device is, for this client (PLAN-PG section 1.1 steps 5
/// and 6), decided once in [`LinkTransport::new`] from the hub's `hello` and
/// its device object. A change is never followed: it makes the transport
/// stale ([`LinkNotice::GenerationChanged`]) and a new one is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkRoute {
    /// Page 0 only: through a 1.2 hub (`l12`: whatever the device is, and
    /// nothing paged, not even the probe: OD-PGe, S1), or a legacy device on
    /// a hub that forwards to one (our hub, cap `legacy_devices`).
    Legacy { l12: bool },
    /// A paged device on a 1.3 hub (`protocol` in its object): paged requests
    /// only, `wIndex` = `(page << 8) | 2`. The probe is answered from the
    /// object, with nothing sent.
    Paged { major: u8, minor: u8 },
    /// A device without `protocol` on a 1.3 hub that does not serve legacy
    /// devices: nothing is sent ([`LinkNotice::HubServesPagedOnly`]).
    Unserved,
}

impl LinkRoute {
    /// The route for `device` (as the hub lists it) on the hub that said `hello`.
    pub fn of(hello: &HelloHub, device: Option<&DeviceInfo>) -> Self {
        if !hello.is_link_13() {
            return LinkRoute::Legacy { l12: true };
        }
        match device.and_then(|d| d.protocol) {
            Some(p) => LinkRoute::Paged {
                major: u8::try_from(p.major).unwrap_or(u8::MAX),
                minor: u8::try_from(p.minor).unwrap_or(u8::MAX),
            },
            None if hello.serves_legacy_devices() => LinkRoute::Legacy { l12: false },
            None => LinkRoute::Unserved,
        }
    }

    fn retryable(self) -> fn(u8) -> bool {
        match self {
            LinkRoute::Paged { .. } => is_retryable_13,
            _ => is_busy,
        }
    }
}

// -- WP PGLINKC --
/// The DSPi Link client's own notices (PLAN-PG section 1.1 steps 5, 6, 8). Each
/// is a [`TransportError::Incompatible`] whose words are [`LinkNotice::text`];
/// [`LinkNotice::of`] gives it back, and [`LinkNotice::code`] is the code of
/// its notice (`transfer.<code>`; the notices and the core's mapping are
/// added outside this WP, PGCORE).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkNotice {
    /// A 1.2 hub, and the device refused the 1.1.6 platform read: it is
    /// probably firmware 1.1.7, which needs a 1.3 hub (never "unrecognised").
    Needs13Hub,
    /// A 1.3 hub lists the device without `protocol`: it forwards nothing to a
    /// 1.1.6-generation device.
    HubServesPagedOnly,
    /// The device behind the handle changed generation (a re-flash): the
    /// transport sends nothing more; open the device again.
    GenerationChanged,
}

impl LinkNotice {
    /// Every notice.
    pub const ALL: [LinkNotice; 3] = [
        LinkNotice::Needs13Hub,
        LinkNotice::HubServesPagedOnly,
        LinkNotice::GenerationChanged,
    ];

    /// The notice's code: the key is `transfer.<code>`.
    pub const fn code(self) -> &'static str {
        match self {
            LinkNotice::Needs13Hub => "link_needs_13_hub",
            LinkNotice::HubServesPagedOnly => "link_hub_paged_only",
            LinkNotice::GenerationChanged => "link_generation_changed",
        }
    }

    /// The error's words.
    pub const fn text(self) -> &'static str {
        match self {
            LinkNotice::Needs13Hub => {
                "this device does not answer the 1.1.6 protocol; firmware 1.1.7 needs a DSPi Link 1.3 hub"
            }
            LinkNotice::HubServesPagedOnly => "this hub serves only firmware 1.1.7 and later",
            LinkNotice::GenerationChanged => {
                "the device's firmware generation changed: open it again"
            }
        }
    }

    /// The error a call fails with.
    pub fn error(self) -> TransportError {
        TransportError::Incompatible(self.text().to_owned())
    }

    /// The notice whose words are `text` exactly.
    pub fn of_text(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|n| n.text() == text)
    }

    /// The notice `e` is, if it is one.
    pub fn of(e: &TransportError) -> Option<Self> {
        match e {
            TransportError::Incompatible(text) => Self::of_text(text),
            _ => None,
        }
    }
}

// -- WP PGLINKC --
/// What a paged device's transport answers to a page-0 (1.1.6-shaped) call:
/// this client never sends one to a paged device (PLAN-PG section 1.1).
pub const PAGED_REFUSES_PAGE0: &str =
    "this device speaks the paged protocol (firmware 1.1.7): no 1.1.6 request is sent to it";

// -- WP PGLINKC --
/// The page count the probe answered from a device object claims: the hub's
/// object carries only the protocol's major and minor (spec 1.3 section 7.4),
/// so this is an upper bound for sizing `SYS.PAGE_LIST` (256 bytes, a
/// bridge's largest non-bulk GET); the device's own list says how many.
pub const LISTED_PROBE_PAGES: u8 = 128;

// -- WP PGLINKC --
/// A firmware result code (`VC_ERR_*`, 0x10..=0x1A, spec 1.3 section 8.1) a hub
/// forwarded for paged request `id`, as a transport error: the hub already
/// explained it, so `PagedTransport` reads no `SYS.LAST_RESULT` after it.
pub fn firmware_refusal(id: CmdId, result: u8) -> TransportError {
    TransportError::Refused { id, result }
}

// -- WP PGLINKC --
/// A poll could not be planned for a paged device (OD-PGo).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PollPlanError {
    /// The 1.1.6 read needs several paged requests: a poll is one GET.
    #[error("opcode 0x{opcode:02X} needs several paged requests; it cannot be polled")]
    MultiRequest { opcode: u8 },
    /// The planner refused the read (absent, wrong length, unknown opcode).
    #[error("opcode 0x{opcode:02X} cannot be polled on this device: {why}")]
    Refused { opcode: u8, why: String },
    /// A poll's GET must ask for at least one byte (spec 1.3 section 8.2).
    #[error("a poll of opcode 0x{opcode:02X} asks for 0 bytes")]
    ZeroLength { opcode: u8 },
}

// -- WP PGLINKC --
/// The poll for the 1.1.6 read `opcode`/`value`/`len` on a paged device whose
/// discovery is `pages`: the paged GET of its single-request plan (PLAN-PG
/// section 1.4, OD-PGo). A read whose plan is several requests, or a local
/// answer, is refused here, before `poll.subscribe`. The POLL payload is the
/// paged GET's verbatim reply: a caller whose plan re-shapes the reply feeds
/// it through the same plan.
pub fn paged_poll(
    slot: u8,
    opcode: u8,
    value: u16,
    len: u16,
    hz: u32,
    pages: &PageSet,
) -> std::result::Result<PollSpec, PollPlanError> {
    if len == 0 {
        return Err(PollPlanError::ZeroLength { opcode });
    }
    let plan =
        dspi_proto::paged::plan(opcode, value, LegacyBody::In { len }, pages).map_err(|e| {
            PollPlanError::Refused {
                opcode,
                why: e.to_string(),
            }
        })?;
    match plan.single() {
        Some(PagedRequest {
            id,
            value,
            body: PagedBody::Get { len },
        }) if *len > 0 => Ok(PollSpec {
            slot,
            req: id.cmd(),
            val: *value,
            idx: id.w_index(),
            len: *len,
            hz,
        }),
        _ => Err(PollPlanError::MultiRequest { opcode }),
    }
}

/// One device (a hub handle) on a DSPi Link session, as a [`Transport`].
///
/// Dropping it releases a lock it holds and stops the handle's notification
/// stream on the session (any [`NotificationSource`] it handed out then
/// reports [`TransportError::Disconnected`]). Open at most one per handle and
/// session at a time: the stream is per handle, so a second one would take it
/// over and the first one's drop would end it.
///
/// The notification stream is subscribed in [`LinkTransport::new`], so no
/// event is lost before the reader starts, and it is unbounded until read.
/// The embedder must therefore take [`Transport::notifications`] exactly once
/// per transport and drain it for as long as the transport lives; a transport
/// dropped without that logs a warning.
///
/// -- WP PGLINKC -- What it sends depends on its [`LinkRoute`]: page-0
/// requests to a legacy device, paged requests (the trait's `*_paged` calls,
/// [`LinkTransport::paged_batch`]) to a paged one, nothing to an unserved one.
/// It is a raw wire: `dspi_transport::paged::open` wraps it, and decides the
/// generation from [`Transport::carries_pages`] and the probe this transport
/// answers from the device object, with nothing sent (PLAN-PG S1).
pub struct LinkTransport {
    client: LinkClient,
    handle: u8,
    descriptor: DeviceDescriptor,
    /// The notification stream subscribed at construction, so no event is
    /// lost between opening and starting the reader; the first
    /// [`Transport::notifications`] call takes it, later ones subscribe anew.
    notes: Mutex<Option<Receiver<NotifyItem>>>,
    /// The first bulk read comes from the hub's snapshot when it offers one
    /// (one round trip instead of a device read). Later reads go to the device.
    snapshot_unused: bool,
    // -- WP LK4 --
    /// The snapshot asked for ahead ([`LinkTransport::prefetch_snapshot`]).
    snapshot_pending: Option<dspi_link::client::PendingSnapshot>,
    /// Whether this transport holds the hub's device lock.
    locked: bool,
    // -- WP PGLINKC --
    /// What the device is, decided at construction.
    route: LinkRoute,
    /// The `protocol` the hub listed at construction: a different one later
    /// means the device changed under the handle.
    listed_protocol: Option<Proto>,
    /// The device has answered a request: a `0x86` (or, to a page-0 request,
    /// `0x10`) from now on is a generation change, not a refusal (PLAN-PG
    /// section 1.1 step 8).
    answered: bool,
    /// The device changed generation: nothing more is sent.
    stale: bool,
    /// The last firmware result code a hub forwarded, with its request.
    last_refusal: Option<(CmdId, u8)>,
}

impl LinkTransport {
    /// Device `handle` on the hub `client` is connected to. Link handles are
    /// one byte on the wire (spec 7.4); the embedder API takes a `u32` for
    /// the bindings and refuses anything above 255 before calling this.
    ///
    /// `serial` is the device's serial from the hub's device list. The
    /// descriptor's `bus_id` names the hub (`link:<hub id>`) and its
    /// `address` is the handle.
    pub fn new(client: LinkClient, handle: u8, serial: &str) -> Self {
        let notes = Mutex::new(Some(client.notifications(handle)));
        let hello = client.hello();
        let snapshot_unused = hello.has_cap(CAP_SNAPSHOT);
        let descriptor = DeviceDescriptor {
            serial: serial.to_owned(),
            bus_id: format!("link:{}", hello.hub.id),
            address: handle,
        };
        // -- WP PGLINKC --
        let device = client.device(u32::from(handle));
        let route = LinkRoute::of(hello, device.as_ref());
        let listed_protocol = device.and_then(|d| d.protocol);
        Self {
            client,
            handle,
            descriptor,
            notes,
            snapshot_unused,
            snapshot_pending: None,
            locked: false,
            route,
            listed_protocol,
            answered: false,
            stale: false,
            last_refusal: None,
        }
    }

    // -- WP PGLINKC --
    /// What the device is to this transport.
    pub fn route(&self) -> LinkRoute {
        self.route
    }

    // -- WP PGLINKC --
    /// The device changed generation under this transport: open it again
    /// (`dspi_transport::paged::open` on a new [`LinkTransport`]).
    pub fn needs_reattach(&self) -> bool {
        self.stale
    }

    // -- WP PGLINKC --
    /// The last firmware result code (0x10..=0x1A) a hub forwarded for a paged
    /// request, with the request's id.
    pub fn last_refusal(&self) -> Option<(CmdId, u8)> {
        self.last_refusal
    }

    // -- WP PGLINKC --
    /// The device's 8 platform bytes, read by this client rather than taken
    /// from the hub's 7-byte `platform` (spec 7.4), so the board id in byte 7
    /// is seen (PLAN-PG section 1.9, R7c): `REQ_GET_PLATFORM` on a legacy
    /// device, `SYS.PLATFORM` on a paged one. Older firmware answers fewer.
    pub fn read_platform(&mut self) -> Result<Vec<u8>> {
        match self.route {
            LinkRoute::Paged { .. } => self.control_in_paged(cmds::SYS_PLATFORM, 0, PLATFORM_ASK),
            _ => self.control_in_upto(op::REQ_GET_PLATFORM, 0, PLATFORM_ASK),
        }
    }

    // -- WP LK4 --
    /// Ask the hub for its snapshot now, so the first bulk read finds it on
    /// its way: the reads sent before that read go out behind the request
    /// instead of before it. A no-op without the `snapshot` capability or
    /// once the snapshot was used.
    pub fn prefetch_snapshot(&mut self) {
        if self.snapshot_unused && self.snapshot_pending.is_none() {
            self.snapshot_pending = self.client.snapshot_begin(self.handle).ok();
        }
    }

    /// The bulk image from the hub's snapshot, cut to `len`, if it has one of
    /// at least `min_len` bytes. Any failure falls back to a device read.
    ///
    /// WP LK1: `age_ms` is not a reason to read the device instead. A hub keeps
    /// the blob current (spec 7.5), and the ESP32 bridge reports the time since
    /// it last read it, hours on a bridge up for hours; the macOS Console and
    /// the 0.10 app ignore it too.
    fn snapshot_bulk(&mut self, len: u16, min_len: u16) -> Option<Vec<u8>> {
        let snapshot = match self.snapshot_pending.take() {
            Some(pending) => self.client.snapshot_finish(pending),
            None => self.client.snapshot(self.handle),
        };
        let mut blob = snapshot.ok()?.bulk()?;
        if blob.is_empty() || blob.len() < usize::from(min_len) {
            return None;
        }
        blob.truncate(usize::from(len));
        Some(blob)
    }

    // -- WP PGLINKC --
    /// Before anything is sent: refuse when the device changed generation
    /// (seen now in the hub's list, or earlier), or when the route does not
    /// carry this kind of request (`paged`: a paged one). Nothing goes out
    /// when this fails.
    fn admit(&mut self, paged: bool) -> Result<()> {
        if !self.stale
            && let Some(now) = self.client.device(u32::from(self.handle))
            && now.protocol != self.listed_protocol
        {
            log::info!(
                "DSPi Link handle {}: the device's protocol changed; re-attach",
                self.handle
            );
            self.stale = true;
        }
        if self.stale {
            return Err(LinkNotice::GenerationChanged.error());
        }
        match (self.route, paged) {
            (LinkRoute::Unserved, _) => Err(LinkNotice::HubServesPagedOnly.error()),
            (LinkRoute::Legacy { .. }, true) => Err(TransportError::Incompatible(NO_PAGES.into())),
            (LinkRoute::Paged { .. }, false) => {
                Err(TransportError::Incompatible(PAGED_REFUSES_PAGE0.into()))
            }
            _ => Ok(()),
        }
    }

    // -- WP PGLINKC --
    /// A device's answer (after the busy window) as this transport reports
    /// it: `opcode` is the `bRequest`, `id` the paged command (`None` for a
    /// page-0 request). Notes a first answer, a generation change (`0x86` or
    /// `0x10` from a device that answered before: PLAN-PG section 1.1 step 8),
    /// a firmware result code, and, through a 1.2 hub, a refused first
    /// platform read (step 6).
    fn settle(&mut self, opcode: u8, id: Option<CmdId>, r: CmdResponse) -> Result<Vec<u8>> {
        if r.status == status::OK {
            self.answered = true;
            return Ok(r.payload);
        }
        // On a paged device 0x10 is also an honest refusal (a command absent
        // from this build, an unlisted page's bitmap), so there only 0x86 is
        // a generation change; a page-0 request answered 0x10 is our hub's
        // "the device is paged now" (PLAN-PG R5a)
        let changed = match self.route {
            LinkRoute::Paged { .. } => r.status == status::UNSUPPORTED,
            _ => matches!(r.status, status::UNSUPPORTED | VC_ERR_UNKNOWN),
        };
        if self.answered && changed {
            log::info!(
                "DSPi Link handle {}: status 0x{:02X} from a device that answered before; re-attach",
                self.handle,
                r.status
            );
            self.stale = true;
            return Err(LinkNotice::GenerationChanged.error());
        }
        let e = match id {
            Some(id) if (VC_ERR_UNKNOWN..=VC_ERR_LAST).contains(&r.status) => {
                self.last_refusal = Some((id, r.status));
                firmware_refusal(id, r.status)
            }
            // -- WP P1HL -- a hub's named refusal keeps its key
            _ => match map_response(r.status, opcode, &r.payload) {
                Some(e) => e,
                None => return Ok(r.payload),
            },
        };
        Err(self.first_platform_refusal(opcode, id, e))
    }

    // -- WP PGLINKC --
    /// Through a 1.2 hub, a refused first `REQ_GET_PLATFORM` (a STALL, 0x02,
    /// DENIED) is [`LinkNotice::Needs13Hub`], never "unrecognised device"
    /// (PLAN-PG section 1.1 step 6, R6e). A timeout stays a timeout: it says
    /// nothing about the protocol (a slow link, a held hub), and the caller's
    /// own retry rules apply to it.
    fn first_platform_refusal(
        &self,
        opcode: u8,
        id: Option<CmdId>,
        e: TransportError,
    ) -> TransportError {
        let refused = matches!(
            e,
            TransportError::Stalled { .. } | TransportError::Denied(_)
        );
        if refused
            && !self.answered
            && id.is_none()
            && opcode == op::REQ_GET_PLATFORM
            && self.route == (LinkRoute::Legacy { l12: true })
        {
            return LinkNotice::Needs13Hub.error();
        }
        e
    }

    /// One request through the busy window, settled.
    #[allow(clippy::too_many_arguments)]
    fn exchange(
        &mut self,
        dir: Dir,
        b_request: u8,
        id: Option<CmdId>,
        value: u16,
        len: u16,
        data: &[u8],
    ) -> Result<Vec<u8>> {
        // -- review/pgb -- A paged request goes only to protocol major 1
        // (spec 1.3 section 7.4: a later major may change what a command
        // means); `paged::open` already refuses such a device from the probe
        // this transport answers, this is the backstop for a direct caller.
        if id.is_some()
            && let LinkRoute::Paged { major, .. } = self.route
            && major != 1
        {
            return Err(crate::paged::Refusal::NewerProtocol.error());
        }
        let w_index = id.map_or(dspi_proto::VENDOR_INTERFACE, CmdId::w_index);
        let (client, handle) = (&self.client, self.handle);
        let r = busy_window_with(b_request, BUSY_BACKOFF, self.route.retryable(), || {
            client.command(handle, dir, b_request, value, w_index, len, data)
        });
        match r {
            Ok(r) => self.settle(b_request, id, r),
            Err(e) => Err(self.first_platform_refusal(b_request, id, e)),
        }
    }

    /// One GET; the reply may be shorter than `len`, never longer. The first
    /// bulk read comes from the snapshot when it has at least `min_len` bytes.
    fn get(&mut self, opcode: u8, value: u16, len: u16, min_len: u16) -> Result<Vec<u8>> {
        self.admit(false)?; // -- WP PGLINKC --
        if opcode == op::REQ_GET_ALL_PARAMS
            && std::mem::take(&mut self.snapshot_unused)
            && let Some(blob) = self.snapshot_bulk(len, min_len)
        {
            return Ok(blob);
        }
        let mut payload = self.exchange(Dir::Get, opcode, None, value, len, &[])?;
        payload.truncate(usize::from(len));
        Ok(payload)
    }

    // -- WP PGLINKC --
    /// Up to the hub's `max_inflight` paged reads on the wire at once: one
    /// round trip for each window instead of one each (PLAN-PG R3; what
    /// [`Transport::control_in_batch_paged`] runs). Each answer is what
    /// [`Transport::control_in_paged`] gives; a retryable one (spec 1.3
    /// section 8.1) goes again on its own, and a `STORE.ALL_PARAMS` read goes
    /// alone (it may come from the snapshot).
    pub fn paged_batch(&mut self, reads: &[PagedRead]) -> Vec<Result<Vec<u8>>> {
        let one = |t: &mut Self, r: &PagedRead| t.control_in_paged(r.id, r.value, r.len);
        let pipelined = reads.len() >= 2
            && self.client.max_inflight() >= 2
            // -- review/pgb -- any other major goes one by one, to its refusal
            && matches!(self.route, LinkRoute::Paged { major: 1, .. })
            && !reads
                .iter()
                .any(|r| r.id == cmds::STORE_ALL_PARAMS || r.id == SYS_PROTOCOL_INFO || r.len == 0);
        if !pipelined || self.admit(true).is_err() {
            return reads.iter().map(|r| one(self, r)).collect();
        }
        let specs: Vec<CmdSpec> = reads
            .iter()
            .map(|r| CmdSpec {
                handle: self.handle,
                dir: Dir::Get,
                b_request: r.id.cmd(),
                w_value: r.value,
                w_index: r.id.w_index(),
                w_length: r.len,
                payload: Vec::new(),
            })
            .collect();
        let answers = self.client.commands(&specs);
        reads
            .iter()
            .zip(answers)
            .map(|(r, a)| {
                if is_retryable_13(a.status) {
                    return one(self, r);
                }
                self.settle(r.id.cmd(), Some(r.id), a).map(|mut p| {
                    p.truncate(usize::from(r.len));
                    p
                })
            })
            .collect()
    }

    // -- WP PGLINKC --
    /// The probe's reply, made from the device object (no request: PLAN-PG
    /// section 1.1 step 5, S1): `SYS.PROTOCOL_INFO`'s 16 bytes with the
    /// listed major and minor, [`LISTED_PROBE_PAGES`], and the hub's
    /// `max_payload` as both payload limits (the hub's own bound; the
    /// device's are smaller or equal and it refuses past them).
    fn listed_probe(&self, major: u8, minor: u8, max_len: u16) -> Vec<u8> {
        let limit = u16::try_from(self.max_transfer()).unwrap_or(u16::MAX);
        let mut reply = Vec::with_capacity(usize::from(PROTOCOL_INFO_LEN));
        reply.extend_from_slice(&PROTOCOL_MAGIC);
        reply.extend_from_slice(&[major, minor, LISTED_PROBE_PAGES, 0]);
        reply.extend_from_slice(&limit.to_le_bytes());
        reply.extend_from_slice(&limit.to_le_bytes());
        reply.resize(usize::from(PROTOCOL_INFO_LEN), 0);
        reply.truncate(usize::from(max_len));
        reply
    }
}

// -- WP LK2 --
/// One answer of a pipelined batch, as [`Transport::control_in`] or
/// [`Transport::control_in_upto`] would give it. A busy status, or
/// RATE_LIMITED (a slot the hub still counts for a request of ours that timed
/// out), means the read did not run: it goes again on its own by `again`.
#[cfg(test)]
fn batch_answer(
    r: &InRead,
    a: CmdResponse,
    again: impl FnOnce() -> Result<Vec<u8>>,
) -> Result<Vec<u8>> {
    let settle = |a: CmdResponse| match map_status(a.status, r.opcode) {
        Some(e) => Err(e),
        None => Ok(a.payload),
    };
    if is_busy(a.status) || a.status == status::RATE_LIMITED {
        return again();
    }
    shape(r, settle(a)?)
}

/// A batch read's payload as [`Transport::control_in`] or
/// [`Transport::control_in_upto`] would give it.
fn shape(r: &InRead, mut payload: Vec<u8>) -> Result<Vec<u8>> {
    payload.truncate(usize::from(r.len));
    if r.exact {
        exact_reply(r.opcode, r.len, payload)
    } else {
        Ok(payload)
    }
}

/// `payload` as [`Transport::control_in`] answers it: refused when short.
fn exact_reply(opcode: u8, len: u16, payload: Vec<u8>) -> Result<Vec<u8>> {
    if payload.len() < usize::from(len) {
        return Err(TransportError::ShortRead {
            opcode,
            wanted: usize::from(len),
            got: payload.len(),
        });
    }
    Ok(payload)
}

impl Transport for LinkTransport {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        let payload = self.get(opcode, value, len, len)?;
        exact_reply(opcode, len, payload)
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> Result<Vec<u8>> {
        // WP LK1: `max_len` is a cap (the probe asks for `max_payload`), so any
        // snapshot serves, as a device reply of that size would
        self.get(opcode, value, max_len, 1)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        self.admit(false)?; // -- WP PGLINKC --
        let Ok(len) = u16::try_from(data.len()) else {
            return Err(TransportError::Remote(format!(
                "{} bytes for opcode 0x{opcode:02X} exceed a vendor request",
                data.len()
            )));
        };
        self.exchange(Dir::Set, opcode, None, value, len, data)
            .map(drop)
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    /// The hub's `max_payload` (spec 7.2; 8192 when it does not say),
    /// clamped to 64..=65535 so a bogus advertisement cannot stall a chunked
    /// reader or exceed a vendor request.
    fn max_transfer(&self) -> usize {
        clamp_max_payload(self.client.hello().limits.max_payload)
    }

    /// The device's notification stream. Take it exactly once and drain it
    /// (see [`LinkTransport`]): the first call returns the stream subscribed
    /// in [`LinkTransport::new`]. Any further call subscribes anew, which
    /// replaces the handle's sink on the session: the earlier source then
    /// reports [`TransportError::Disconnected`].
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        let first = self
            .notes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let rx = first.unwrap_or_else(|| self.client.notifications(self.handle));
        Some(Box::new(LinkNotifications { rx }))
    }

    fn caps(&self) -> TransportCaps {
        TransportCaps {
            kind: crate::TransportKind::Link,
            chunked_bulk: false,
            remote: true,
        }
    }

    fn session_id(&self) -> Option<u16> {
        self.client.session_id()
    }

    // -- WP PGWIRE, PGLINKC --
    /// A page goes only to a paged device on a 1.3 hub. `paged::open` probes
    /// only where this is true, and the probe is answered here from the
    /// device object with nothing sent (PLAN-PG section 1.1 step 5, S1): a
    /// 1.2 hub judges by `bRequest` and may forward or drop the page, so
    /// nothing paged goes through one (step 6). True also for a device a 1.3
    /// hub does not serve, whose probe fails with
    /// [`LinkNotice::HubServesPagedOnly`], so `open` sends nothing at all.
    fn carries_pages(&self) -> bool {
        matches!(self.route, LinkRoute::Paged { .. } | LinkRoute::Unserved)
    }

    // -- WP PGLINKC --
    /// A paged GET, `wIndex` = `(page << 8) | 2`, never with `wLength` 0
    /// (spec 1.3 section 8.2). `SYS.PROTOCOL_INFO` is answered from the
    /// device object, with nothing sent; the first `STORE.ALL_PARAMS` read
    /// may come from the hub's snapshot.
    fn control_in_paged(&mut self, id: CmdId, value: u16, max_len: u16) -> Result<Vec<u8>> {
        self.admit(true)?;
        if id == SYS_PROTOCOL_INFO
            && let LinkRoute::Paged { major, minor } = self.route
        {
            return Ok(self.listed_probe(major, minor, max_len));
        }
        if max_len == 0 {
            return Err(TransportError::Remote(format!(
                "a GET of 0x{:04X} with wLength 0 is never sent (spec 1.3 section 8.2)",
                id.0
            )));
        }
        if id == cmds::STORE_ALL_PARAMS
            && std::mem::take(&mut self.snapshot_unused)
            && let Some(blob) = self.snapshot_bulk(max_len, 1)
        {
            return Ok(blob);
        }
        let mut payload = self.exchange(Dir::Get, id.cmd(), Some(id), value, max_len, &[])?;
        payload.truncate(usize::from(max_len));
        Ok(payload)
    }

    // -- WP PGLINKC --
    /// A paged SET, `wIndex` = `(page << 8) | 2`; a zero-length `data` is an
    /// action. Bulk is single-shot `STORE.ALL_PARAMS` (never the chunk form
    /// over Link, spec 1.3 section 8.2).
    fn control_out_paged(&mut self, id: CmdId, value: u16, data: &[u8]) -> Result<()> {
        self.admit(true)?;
        if id == cmds::STORE_ALL_PARAMS_CHUNK {
            return Err(TransportError::Stalled { opcode: id.cmd() });
        }
        let Ok(len) = u16::try_from(data.len()) else {
            return Err(TransportError::Remote(format!(
                "{} bytes for 0x{:04X} exceed a vendor request",
                data.len(),
                id.0
            )));
        };
        self.exchange(Dir::Set, id.cmd(), Some(id), value, len, data)
            .map(drop)
    }

    // -- WP PGLINKC --
    /// [`LinkTransport::paged_batch`]: the discovery and the planned reads of
    /// a paged connect keep the round trips of a legacy one (PLAN-PG R3).
    fn control_in_batch_paged(&mut self, reads: &[PagedRead]) -> Vec<Result<Vec<u8>>> {
        self.paged_batch(reads)
    }

    /// Spec 7.7: a hub without the `lock` capability serialises commands
    /// itself, so this succeeds at once. Another session's lock is
    /// [`TransportError::Locked`], a viewer's request `Denied`.
    fn lock(&mut self, reason: &str, timeout: Duration) -> Result<()> {
        if !self.client.hello().has_cap(CAP_LOCK) {
            return Ok(());
        }
        let ms = u32::try_from(timeout.as_millis())
            .unwrap_or(LOCK_MAX_MS)
            .min(LOCK_MAX_MS);
        self.client
            .lock_acquire(self.handle, reason, ms)
            .map_err(map_link_error)?;
        self.locked = true;
        Ok(())
    }

    /// Fire and forget: a lost release expires on the hub.
    fn unlock(&mut self) {
        if std::mem::take(&mut self.locked) {
            self.client.lock_release(self.handle);
        }
    }

    // -- WP LK2 --
    /// Up to the hub's `max_inflight` of the reads on the wire at once
    /// ([`LinkClient::commands`]): one round trip for each window of them
    /// instead of one each. A busy answer (spec 8.1) is retried on its own
    /// through the busy window afterwards; a bulk read goes one at a time (it
    /// may come from the snapshot).
    ///
    /// -- WP PGLINKC -- Through a 1.2 hub, before the device has answered
    /// anything, a batch that starts with `REQ_GET_PLATFORM` and holds other
    /// reads sends that read alone first: a device that refuses it (firmware
    /// 1.1.7) is sent nothing else (PLAN-PG section 1.1 step 6). It costs the
    /// identity read of a full probe one round trip, once per transport.
    fn control_in_batch(&mut self, reads: &[InRead]) -> Vec<Result<Vec<u8>>> {
        let one = |t: &mut Self, r: &InRead| {
            if r.exact {
                t.control_in(r.opcode, r.value, r.len)
            } else {
                t.control_in_upto(r.opcode, r.value, r.len)
            }
        };
        if let Err(e) = self.admit(false) {
            return reads.iter().map(|_| Err(e.clone())).collect();
        }
        if let [first, rest @ ..] = reads
            && rest.iter().any(|r| r.opcode != op::REQ_GET_PLATFORM)
            && !self.answered
            && first.opcode == op::REQ_GET_PLATFORM
            && self.route == (LinkRoute::Legacy { l12: true })
        {
            let head = one(self, first);
            if let Err(e) = &head
                && LinkNotice::of(e) == Some(LinkNotice::Needs13Hub)
            {
                return reads.iter().map(|_| Err(e.clone())).collect();
            }
            let mut out = vec![head];
            out.extend(self.control_in_batch(rest));
            return out;
        }
        if reads.len() < 2
            || self.client.max_inflight() < 2
            || reads.iter().any(|r| r.opcode == op::REQ_GET_ALL_PARAMS)
        {
            return reads.iter().map(|r| one(self, r)).collect();
        }
        let specs: Vec<CmdSpec> = reads
            .iter()
            .map(|r| CmdSpec {
                handle: self.handle,
                dir: Dir::Get,
                b_request: r.opcode,
                w_value: r.value,
                w_index: dspi_proto::VENDOR_INTERFACE,
                w_length: r.len,
                payload: Vec::new(),
            })
            .collect();
        let answers = self.client.commands(&specs);
        reads
            .iter()
            .zip(answers)
            .map(|(r, a)| {
                // -- WP PGLINKC -- what did not run goes again; the rest is settled
                if is_busy(a.status) || a.status == status::RATE_LIMITED {
                    return one(self, r);
                }
                let payload = self.settle(r.opcode, None, a)?;
                shape(r, payload)
            })
            .collect()
    }
}

impl Drop for LinkTransport {
    fn drop(&mut self) {
        let untaken = self
            .notes
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some();
        if untaken {
            log::warn!(
                "DSPi Link handle {}: transport dropped without its notifications ever being taken",
                self.handle
            );
        }
        // A closed session holds no lock, and the send could wait on a half-open
        // socket on the caller's thread.
        if !self.client.is_closed() {
            self.unlock();
        }
        self.client.stop_notifications(self.handle);
    }
}

/// The notification stream of one Link device.
struct LinkNotifications {
    rx: Receiver<NotifyItem>,
}

impl NotificationSource for LinkNotifications {
    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        self.read_attributed(timeout).map(|a| a.packet)
    }

    /// The packet with the hub's origin (spec 8.3): the writer's session id,
    /// 0 for a change made at the device. Silence is an empty packet; the
    /// session ending (or the transport dropped) is `Disconnected`.
    fn read_attributed(&mut self, timeout: Duration) -> Result<Attributed> {
        // -- WP WB4b -- the web's caller-driven reader looks without waiting,
        // and `recv_timeout` reads `std`'s clock, which panics there
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        let received = match self.rx.try_recv() {
            Ok(item) => Ok(item),
            Err(std::sync::mpsc::TryRecvError::Empty) => Err(RecvTimeoutError::Timeout),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(RecvTimeoutError::Disconnected),
        };
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        let received = self.rx.recv_timeout(timeout);
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        let _ = timeout;
        match received {
            Ok(item) => Ok(Attributed {
                packet: item.packet,
                origin: Some(item.origin),
            }),
            Err(RecvTimeoutError::Timeout) => Ok(Attributed {
                packet: Vec::new(),
                origin: None,
            }),
            Err(RecvTimeoutError::Disconnected) => Err(TransportError::Disconnected),
        }
    }
}

// -- WP PGLINKC --
#[cfg(test)]
#[path = "link/tests_wp_pglinkc.rs"]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests_wp_pglinkc;

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_t5 {
    use super::*;
    use crate::{TransportKind, is_own_write};
    use dspi_link::fakehub::{FakeHub, HubConfig, OFF_MASTER_VOLUME, req};
    use dspi_link::socket::TcpConnector;
    use dspi_link::{AuthState, ConnectOptions};

    const WAIT: Duration = Duration::from_secs(3);

    fn connect(hub: &FakeHub) -> LinkClient {
        let opts = ConnectOptions {
            client_name: "wp-t5".into(),
            app: "dspi-transport tests".into(),
            app_version: "test".into(),
            token: None,
        };
        let (c, state) = LinkClient::connect(&TcpConnector::default(), &hub.url(), &opts, None)
            .expect("connect to the fake hub");
        assert_eq!(state, AuthState::Ready);
        c
    }

    fn open(hub: &FakeHub) -> LinkTransport {
        let c = connect(hub);
        let serial = c.devices()[0].serial.clone();
        LinkTransport::new(c, 0, &serial)
    }

    fn bulk_reads(hub: &FakeHub) -> usize {
        hub.log()
            .iter()
            .filter(|e| e.2 == req::GET_ALL_PARAMS)
            .count()
    }

    fn wait(mut f: impl FnMut() -> bool) -> bool {
        for _ in 0..300 {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// The next non-empty notification, with its origin.
    fn next_note(src: &mut dyn NotificationSource) -> Attributed {
        for _ in 0..30 {
            let a = src.read_attributed(Duration::from_millis(100)).unwrap();
            if !a.packet.is_empty() {
                return a;
            }
        }
        panic!("no notification within 3 s");
    }

    #[test]
    fn link_statuses_map_onto_transport_errors() {
        use TransportError as E;
        let m = |s| map_status(s, 0x0F);
        assert!(m(status::OK).is_none());
        assert!(
            matches!(m(status::ERROR), Some(E::Stalled { opcode: 0x0F })),
            "RTA bands-all fallback"
        );
        assert!(matches!(
            m(status::BLOCKED),
            Some(E::Stalled { opcode: 0x0F })
        ));
        assert!(matches!(
            m(status::TIMEOUT),
            Some(E::TimedOut { opcode: 0x0F, .. })
        ));
        assert!(matches!(m(status::NO_DEVICE), Some(E::Disconnected)));
        assert!(matches!(m(status::DENIED), Some(E::Denied(_))));
        assert!(matches!(m(status::LOCKED), Some(E::Locked)));
        for s in [
            status::BUSY,
            status::BULK_LOCKED,
            status::CRC_ERROR,
            status::OVERSIZE,
            status::FRAME_ERROR,
            status::TOO_LARGE,
            status::RATE_LIMITED,
            status::UNSUPPORTED,
            status::BAD_FRAME,
            0x42,
        ] {
            match m(s) {
                Some(E::Remote(msg)) => assert!(msg.contains(status::name(s)), "{msg}"),
                other => panic!("status 0x{s:02X} -> {other:?}"),
            }
        }
    }

    #[test]
    fn a_device_on_the_fake_hub_reads_its_bulk_image() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let mut t = open(&hub);
        let len = u16::try_from(hub.image().len()).unwrap();
        assert_eq!(
            t.control_in(req::GET_ALL_PARAMS, 0, len).unwrap(),
            hub.image()
        );
        assert_eq!(bulk_reads(&hub), 0, "the first bulk read is the snapshot");
        assert_eq!(
            t.control_in(req::GET_ALL_PARAMS, 0, len).unwrap(),
            hub.image()
        );
        assert_eq!(bulk_reads(&hub), 1, "later reads go to the device");
    }

    // -- WP LK1 --
    /// The probe's read asks for `max_payload`, more than the image: the
    /// snapshot still serves it, as on the 0.10 app.
    #[test]
    fn wp_lk1_the_probes_capped_bulk_read_comes_from_the_snapshot() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let mut t = open(&hub);
        let max = u16::try_from(t.max_transfer()).unwrap();
        assert!(usize::from(max) > hub.image().len());
        assert_eq!(
            t.control_in_upto(req::GET_ALL_PARAMS, 0, max).unwrap(),
            hub.image()
        );
        assert_eq!(bulk_reads(&hub), 0, "no second bulk transfer");
    }

    /// An ESP32 bridge's snapshot is as old as its last read of the blob, which
    /// its notifications keep current: it is used whatever its age.
    #[test]
    fn wp_lk1_an_old_snapshot_is_used() {
        let hub = FakeHub::start(HubConfig {
            snapshot_age_ms: 3_600_000,
            auth: dspi_link::fakehub::Auth::None,
            ..HubConfig::bridge()
        })
        .unwrap();
        let mut t = open(&hub);
        let len = u16::try_from(hub.image().len()).unwrap();
        assert_eq!(
            t.control_in(req::GET_ALL_PARAMS, 0, len).unwrap(),
            hub.image()
        );
        assert_eq!(bulk_reads(&hub), 0);
    }

    /// An exact read the snapshot is too short for goes to the device.
    #[test]
    fn wp_lk1_an_exact_read_longer_than_the_snapshot_reads_the_device() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let mut t = open(&hub);
        let len = u16::try_from(hub.image().len() + 1).unwrap();
        assert!(matches!(
            t.control_in(req::GET_ALL_PARAMS, 0, len),
            Err(TransportError::ShortRead { .. })
        ));
        assert_eq!(
            bulk_reads(&hub),
            1,
            "read from the device after the snapshot"
        );
    }

    #[test]
    fn wp_t5_bulk_reads_the_device_on_a_hub_without_snapshots() {
        let mut cfg = HubConfig::default();
        cfg.caps.retain(|c| c != CAP_SNAPSHOT);
        let hub = FakeHub::start(cfg).unwrap();
        let mut t = open(&hub);
        let len = u16::try_from(hub.image().len()).unwrap();
        assert_eq!(
            t.control_in(req::GET_ALL_PARAMS, 0, len).unwrap(),
            hub.image()
        );
        assert_eq!(bulk_reads(&hub), 1);
    }

    #[test]
    fn wp_t5_identity_caps_and_limits() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let t = open(&hub);
        assert_eq!(t.descriptor().serial, "0123456789ABCDEF");
        assert_eq!(t.descriptor().address, 0);
        assert!(t.descriptor().bus_id.starts_with("link:"));
        assert_eq!(t.max_transfer(), 8192);
        let caps = t.caps();
        assert_eq!(caps.kind, TransportKind::Link);
        assert!(!caps.chunked_bulk && caps.remote);
        assert_eq!(t.session_id(), Some(12));
    }

    #[test]
    fn wp_t5_short_replies_and_stalls() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let mut t = open(&hub);
        // GET_PLATFORM answers 7 bytes, whatever is asked
        assert_eq!(
            t.control_in_upto(req::GET_PLATFORM, 0, 16).unwrap().len(),
            7
        );
        assert!(matches!(
            t.control_in(req::GET_PLATFORM, 0, 16),
            Err(TransportError::ShortRead {
                wanted: 16,
                got: 7,
                ..
            })
        ));
        assert_eq!(t.control_in(req::GET_PLATFORM, 0, 4).unwrap().len(), 4);
        assert!(matches!(
            t.control_in(0x99, 0, 4),
            Err(TransportError::Stalled { opcode: 0x99 })
        ));
        let mut gone = LinkTransport::new(t.client.clone(), 3, "none");
        assert!(matches!(
            gone.control_in(req::GET_PLATFORM, 0, 7),
            Err(TransportError::Disconnected)
        ));
    }

    #[test]
    fn wp_t5_a_bridge_stalls_chunked_bulk_and_a_hub_timeout_is_timed_out() {
        let hub = FakeHub::start(HubConfig {
            bridge_limits: true,
            timeout_requests: [req::GET_STATUS].into(),
            ..HubConfig::default()
        })
        .unwrap();
        let mut t = open(&hub);
        assert!(matches!(
            t.control_in(req::GET_ALL_PARAMS_CHUNK, 0, 64),
            Err(TransportError::Stalled { .. })
        ));
        assert!(matches!(
            t.control_in(req::RTA_GET_BANDS_ALL, 0, 64),
            Err(TransportError::Stalled { .. })
        ));
        assert!(matches!(
            t.control_in(req::GET_STATUS, 9, 4),
            Err(TransportError::TimedOut { .. })
        ));
        assert!(matches!(
            t.control_out(0x42, 0, &[0; 65]),
            Err(TransportError::Remote(_))
        ));
    }

    /// The hub's text must not reach a terminal or a log with its escapes intact.
    #[test]
    fn wp_l1_a_denied_message_from_the_hub_is_sanitised() {
        let e = map_link_error(LinkError::Rejected {
            code: "denied".into(),
            msg: "no\u{1b}[2J\u{9b}31m\nway".into(),
        });
        match e {
            TransportError::Denied(m) => assert_eq!(m, "no\u{fffd}[2J\u{fffd}31m\u{fffd}way"),
            other => panic!("{other:?}"),
        }
    }

    /// A session past `max_inflight` gets RATE_LIMITED. The transport does not
    /// retry it (the command did not run, and only time frees a slot): it is a
    /// typed error naming the status, at once, and the next command works.
    #[test]
    fn wp_l1_rate_limited_surfaces_as_a_remote_error() {
        let hub = FakeHub::start(HubConfig {
            max_inflight: 1,
            ..HubConfig::default()
        })
        .unwrap();
        let cfg = dspi_link::SessionConfig {
            timing: dspi_link::Timing {
                request_timeout: Duration::from_millis(500),
                ..dspi_link::Timing::default()
            },
            ..dspi_link::SessionConfig::default()
        };
        let opts = ConnectOptions {
            client_name: "wp-l1".into(),
            app: "dspi-transport tests".into(),
            app_version: "test".into(),
            token: None,
        };
        let (c, _) =
            LinkClient::connect_with(&TcpConnector::default(), &hub.url(), &opts, None, &cfg)
                .unwrap();
        let serial = c.devices()[0].serial.clone();
        let mut t = LinkTransport::new(c, 0, &serial);
        hub.hold_replies();
        assert!(matches!(
            t.control_in(req::GET_PLATFORM, 0, 7),
            Err(TransportError::TimedOut { .. })
        ));
        let ran = hub.log().len();
        match t.control_in(req::GET_PLATFORM, 0, 7) {
            Err(TransportError::Remote(m)) => assert!(m.contains("RATE_LIMITED"), "{m}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(hub.log().len(), ran, "not retried, and it did not run");
        hub.release_replies();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(t.control_in(req::GET_PLATFORM, 0, 7).unwrap().len(), 7);
    }

    #[test]
    fn wp_t5_a_set_lands_and_notifies_with_our_origin() {
        use dspi_proto::generated::notify as n;
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let mut t = open(&hub);
        let mut notes = t.notifications().unwrap();
        let session = t.session_id();
        let v = (-12.0f32).to_le_bytes();
        t.control_out(req::SET_MASTER_VOLUME, 0, &v).unwrap();
        let off = usize::from(OFF_MASTER_VOLUME);
        assert_eq!(&hub.image()[off..off + 4], &v);

        // Our own write: the echo carries our session id
        let own = next_note(notes.as_mut());
        assert_eq!(&own.packet[..2], &[0x02, 0x02], "PARAM_CHANGED");
        assert_eq!(own.origin, session);
        assert!(is_own_write(
            TransportKind::Link,
            own.packet[8],
            own.origin,
            session,
            false
        ));

        // Another client's write, and a change at the device (origin 0)
        let host = n::PARAM_SRC_HOST_SET as u8;
        hub.external_change(OFF_MASTER_VOLUME, &(-6.0f32).to_le_bytes(), host, 40);
        let other = next_note(notes.as_mut());
        assert_eq!(other.origin, Some(40));
        assert!(!is_own_write(
            TransportKind::Link,
            other.packet[8],
            other.origin,
            session,
            false
        ));
        hub.external_change(OFF_MASTER_VOLUME, &(-3.0f32).to_le_bytes(), host, 0);
        let device = next_note(notes.as_mut());
        assert_eq!(device.origin, Some(0));
        assert!(!is_own_write(
            TransportKind::Link,
            device.packet[8],
            device.origin,
            session,
            false
        ));

        // Silence is an empty packet, the session ending is Disconnected
        assert!(notes.read(Duration::from_millis(50)).unwrap().is_empty());
        t.client.close();
        assert!(matches!(
            notes.read(WAIT),
            Err(TransportError::Disconnected)
        ));
        assert!(matches!(
            t.control_out(req::SET_MASTER_VOLUME, 0, &v),
            Err(TransportError::Disconnected)
        ));
    }

    #[test]
    fn wp_t5_dropping_the_transport_ends_its_notifications() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let t = open(&hub);
        let mut notes = t.notifications().unwrap();
        drop(t);
        assert!(matches!(
            notes.read(WAIT),
            Err(TransportError::Disconnected)
        ));
    }

    #[test]
    fn wp_t5_the_device_lock() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let mut t = open(&hub);
        let sid = t.session_id().unwrap();
        let v = (-20.0f32).to_le_bytes();

        // Another console holds it: our writes and our lock are refused
        hub.hold_lock(99, 10_000);
        assert!(matches!(
            t.control_out(req::SET_MASTER_VOLUME, 0, &v),
            Err(TransportError::Locked)
        ));
        assert!(matches!(
            t.lock("Saving preset", Duration::from_secs(5)),
            Err(TransportError::Locked)
        ));
        hub.release_lock();

        t.lock("Saving preset", Duration::from_secs(5)).unwrap();
        assert_eq!(hub.lock_holder(), Some(sid));
        t.control_out(req::SET_MASTER_VOLUME, 0, &v).unwrap();
        t.unlock();
        assert!(wait(|| hub.lock_holder().is_none()), "unlock releases");

        // A dropped transport gives its lock back
        t.lock("Restoring", Duration::from_secs(120)).unwrap();
        assert_eq!(hub.lock_holder(), Some(sid));
        drop(t);
        assert!(wait(|| hub.lock_holder().is_none()), "drop releases");
    }

    #[test]
    fn wp_t5_a_hub_without_locks_grants_at_once() {
        let mut cfg = HubConfig::default();
        cfg.caps.retain(|c| c != CAP_LOCK);
        let hub = FakeHub::start(cfg).unwrap();
        let mut t = open(&hub);
        t.lock("Saving preset", Duration::from_secs(5)).unwrap();
        assert_eq!(hub.lock_holder(), None);
        t.unlock();
    }

    /// A scripted status sequence through the busy window; `calls` counts
    /// the attempts.
    fn scripted(statuses: &[u8]) -> (Result<Vec<u8>>, usize) {
        let mut calls = 0;
        let r = busy_window(0xA0, Duration::ZERO, || {
            let status = statuses.get(calls).copied().unwrap_or(status::OK);
            calls += 1;
            CmdResponse {
                tag: 1,
                status,
                payload: vec![7; 3],
            }
        });
        (r, calls)
    }

    // -- WP LK2 --
    #[test]
    fn wp_lk2_a_batch_answers_as_one_read_at_a_time_does() {
        let hub = FakeHub::start(HubConfig {
            max_inflight: 4,
            ..HubConfig::default()
        })
        .unwrap();
        let reads = [
            InRead::exact(req::GET_PLATFORM, 0, 7),
            InRead::upto(req::GET_PLATFORM, 0, 64),
            InRead::exact(req::GET_PLATFORM, 0, 64),
            InRead::exact(0xEE, 0, 4),
            InRead::upto(req::GET_STATUS, 9, 41),
            InRead::exact(req::GET_PLATFORM, 0, 4),
        ];
        let mut t = open(&hub);
        let one: Vec<String> = reads
            .iter()
            .map(|r| {
                let a = if r.exact {
                    t.control_in(r.opcode, r.value, r.len)
                } else {
                    t.control_in_upto(r.opcode, r.value, r.len)
                };
                format!("{a:?}")
            })
            .collect();
        let batch: Vec<String> = t
            .control_in_batch(&reads)
            .iter()
            .map(|a| format!("{a:?}"))
            .collect();
        assert_eq!(batch, one);
        assert!(batch[2].contains("ShortRead"), "{}", batch[2]);
        assert!(batch[3].contains("Stalled"), "{}", batch[3]);
    }

    #[test]
    fn wp_lk2_a_batch_keeps_at_most_max_inflight_on_the_wire() {
        let hub = FakeHub::start(HubConfig {
            max_inflight: 4,
            ..HubConfig::default()
        })
        .unwrap();
        let mut t = open(&hub);
        let base = hub.log().len();
        hub.hold_replies();
        let reads = vec![InRead::exact(req::GET_PLATFORM, 0, 7); 10];
        let batch = std::thread::spawn(move || t.control_in_batch(&reads));
        assert!(wait(|| hub.log().len() == base + 4), "four out at once");
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(hub.log().len(), base + 4, "and no more (no RATE_LIMITED)");
        hub.release_replies();
        let answers = batch.join().unwrap();
        assert_eq!(answers.len(), 10);
        assert!(
            answers
                .iter()
                .all(|a| a.as_ref().is_ok_and(|d| d.len() == 7))
        );
        assert_eq!(hub.log().len(), base + 10);
    }

    #[test]
    fn wp_lk2_a_batch_retries_what_did_not_run_and_nothing_else() {
        let r = InRead::exact(0x7F, 0, 3);
        let answer = |status, payload: Vec<u8>| CmdResponse {
            tag: 1,
            status,
            payload,
        };
        for s in [
            status::BUSY,
            status::BULK_LOCKED,
            status::CRC_ERROR,
            status::RATE_LIMITED,
        ] {
            let again = batch_answer(&r, answer(s, Vec::new()), || Ok(vec![1, 2, 3]));
            assert_eq!(again.unwrap(), vec![1, 2, 3], "0x{s:02X} goes again");
        }
        for s in [status::TIMEOUT, status::ERROR, status::NO_DEVICE] {
            let mut retried = false;
            let e = batch_answer(&r, answer(s, Vec::new()), || {
                retried = true;
                Ok(Vec::new())
            });
            assert!(
                e.is_err() && !retried,
                "0x{s:02X} is not repeated: it may have run"
            );
        }
        assert!(matches!(
            batch_answer(&r, answer(status::OK, vec![9; 2]), || unreachable!()),
            Err(TransportError::ShortRead { got: 2, .. })
        ));
        assert_eq!(
            batch_answer(
                &InRead::upto(0x7F, 0, 3),
                answer(status::OK, vec![9; 5]),
                || unreachable!()
            )
            .unwrap(),
            vec![9; 3]
        );
    }

    #[test]
    fn wp_t5_busy_statuses_are_retried_then_time_out() {
        for busy in [status::BUSY, status::BULK_LOCKED, status::CRC_ERROR] {
            let (r, calls) = scripted(&[busy, busy]);
            assert_eq!(r.unwrap(), vec![7; 3], "status 0x{busy:02X} recovers");
            assert_eq!(calls, 3);
        }
        let (r, calls) = scripted(&[status::BUSY; BUSY_RETRIES + 1]);
        assert!(matches!(
            r,
            Err(TransportError::TimedOut {
                opcode: 0xA0,
                retries: BUSY_RETRIES
            })
        ));
        assert_eq!(calls, BUSY_RETRIES + 1, "gives up rather than looping");
        let (r, calls) = scripted(&[status::BUSY, status::ERROR]);
        assert!(matches!(r, Err(TransportError::Stalled { opcode: 0xA0 })));
        assert_eq!(calls, 2);
        let (r, calls) = scripted(&[status::TIMEOUT]);
        assert!(
            matches!(r, Err(TransportError::TimedOut { retries: 0, .. })),
            "a hub TIMEOUT may have run: not retried"
        );
        assert_eq!(calls, 1);
    }

    #[test]
    fn wp_t5_max_payload_is_clamped() {
        assert_eq!(clamp_max_payload(0), 64);
        assert_eq!(clamp_max_payload(1), 64);
        assert_eq!(clamp_max_payload(8192), 8192);
        assert_eq!(clamp_max_payload(u32::MAX), 65_535);
    }

    #[test]
    fn wp_t5_a_second_notifications_call_ends_the_first() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let mut t = open(&hub);
        let mut first = t.notifications().unwrap();
        let mut second = t.notifications().unwrap();
        assert!(matches!(
            first.read(WAIT),
            Err(TransportError::Disconnected)
        ));
        t.control_out(req::SET_MASTER_VOLUME, 0, &(-9.0f32).to_le_bytes())
            .unwrap();
        assert_eq!(next_note(second.as_mut()).origin, t.session_id());
    }
}

// -- WP P1HL --
#[cfg(test)]
mod tests_wp_p1hl {
    use super::*;

    fn denied(payload: &str) -> Option<TransportError> {
        map_response(status::DENIED, 0x74, payload.as_bytes())
    }

    /// A hub's named refusal keeps its notice key and its words; anything else
    /// maps as `map_status` does.
    #[test]
    fn wp_p1hl_a_hubs_named_refusal_keeps_its_notice_key() {
        for key in crate::HUB_REFUSALS {
            let e = denied(&format!(
                r#"{{"refused":"{key}","limit":"out_gain","msg":"output 1 gain 6 dB"}}"#
            ))
            .unwrap();
            assert_eq!(e.hub_refusal(), Some((key, "output 1 gain 6 dB")), "{e:?}");
            // -- WP P1DKFIX -- only a limit refusal carries the limit.
            let limit = (key == "limit_exceeded").then_some("out_gain");
            assert_eq!(e.hub_refusal_limit(), limit, "{e:?}");
            assert!(
                matches!(e, TransportError::Denied(_)),
                "still a denial: {e:?}"
            );
        }
        // No payload, an unknown key, not JSON, another status: as before.
        for e in [
            map_response(status::DENIED, 0x74, &[]),
            denied(r#"{"refused":"something_else","msg":"x"}"#),
            denied("not json"),
            denied(r#"{"msg":"x"}"#),
        ] {
            let e = e.unwrap();
            assert!(matches!(e, TransportError::Denied(_)), "{e:?}");
            assert_eq!(e.hub_refusal(), None, "{e:?}");
        }
        assert!(matches!(
            map_response(status::LOCKED, 0x74, br#"{"refused":"limit_exceeded"}"#),
            Some(TransportError::Locked)
        ));
        assert!(map_response(status::OK, 0x74, b"payload").is_none());
        // The hub's words are made safe to show.
        let e = denied("{\"refused\":\"installer_lock\",\"msg\":\"a\\u001b[2Jb\"}").unwrap();
        assert_eq!(e.hub_refusal(), Some(("installer_lock", "a\u{fffd}[2Jb")));
        // -- WP P1DKFIX -- a limit that is not a code is dropped, the refusal kept.
        let e = denied(r#"{"refused":"limit_exceeded","limit":"a: b","msg":"m"}"#).unwrap();
        assert_eq!(e.hub_refusal(), Some(("limit_exceeded", "m")));
        assert_eq!(e.hub_refusal_limit(), None);
        // A plain Denied whose words merely look like it is not a refusal unless the
        // key is one of the hub's.
        assert_eq!(
            TransportError::Denied("viewer: no".into()).hub_refusal(),
            None
        );
    }
}
