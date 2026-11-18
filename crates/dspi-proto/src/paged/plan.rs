//! The planner: a 1.1.6-shaped request as paged requests, and how to build the
//! 1.1.6 reply from their answers (PLAN-PG section 1.2, WP PGPROTO).
//!
//! Pure: [`plan`] returns a [`Plan`], a small step machine the transport runs
//! (`PagedTransport`, WP PGADAPT): [`Plan::start`] gives the first request,
//! [`Plan::feed`] takes each answer and gives the next request or the outcome.
//! Every row of `legacy_map.json` except the `custom` ones is one request; the
//! custom rows are `paged::custom`'s (WP PGCUST), which may take several.
//!
//! The rules, each from the vendored map's `_schema` and upstream's renderer
//! (`scripts/gen_vendor_protocol.py` `_map_row`), cited where they apply:
//! - a `direct` row keeps the direction and the payload; `wValue` becomes the
//!   address (`addr` overrides it); a target that addresses nothing gets `wValue` 0;
//! - a former write-as-read becomes a SET with a one-byte payload (`act_value`,
//!   `act_packed`) or none (`act_addr`), and its reply is the old status byte;
//! - `split` picks a case by the old `wValue`; a `null` default is what 1.1.6
//!   refused (an IN STALLed, an OUT was swallowed);
//! - our extension channel `REQ_EXTENSION` 0x00 becomes the tunnel `0xE010` with
//!   the same `wValue` and payload when the device lists it, else a local refusal,
//!   so the extension probe sees "no extension channel" as on stock firmware
//!   (PLAN-PG section 1.3, OD-PGc; the firmware spec's "Envelopes" section, WP FWPG0).
//!
//! Never planned: a request on page 0, a GET with `wLength` 0, a SET whose length
//! the vendored revision refuses (checked only when the device's page revision is
//! the vendored one, R8b), a command the device does not list.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::legacy::{self, Act, Adapter, AddrRule, Direct, LegacyOp, OnStall, OutStall, Status};
use super::registry::{self, AddrKind, Command};
use super::{CmdId, EXT_TUNNEL, PageSet, PagedDir};
use crate::generated::opcodes::REQ_EXTENSION;

/// The USB direction of a 1.1.6 request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum UsbDir {
    /// IN (`0xC1`): a read, or a write-as-read.
    In,
    /// OUT (`0x41`): a write with a payload.
    Out,
}

/// A 1.1.6 request's data side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyBody<'a> {
    /// IN: the `wLength` asked for.
    In { len: u16 },
    /// OUT: the payload.
    Out(&'a [u8]),
}

impl LegacyBody<'_> {
    /// The USB direction.
    pub fn dir(&self) -> UsbDir {
        match self {
            LegacyBody::In { .. } => UsbDir::In,
            LegacyBody::Out(_) => UsbDir::Out,
        }
    }
}

/// One paged request.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PagedRequest {
    pub id: CmdId,
    pub value: u16,
    pub body: PagedBody,
}

/// A paged request's data side.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum PagedBody {
    /// A GET of at most `len` bytes (never 0).
    Get { len: u16 },
    /// A SET with this payload (empty for a zero-length action).
    Set(Vec<u8>),
}

impl PagedRequest {
    /// The direction.
    pub fn dir(&self) -> PagedDir {
        match self.body {
            PagedBody::Get { .. } => PagedDir::Get,
            PagedBody::Set(_) => PagedDir::Set,
        }
    }

    /// The 8-byte USB SETUP packet (migration guide 3.1/3.2): `bmRequestType`,
    /// `bRequest`, then `wValue`, `wIndex` and `wLength` little-endian.
    pub fn setup_packet(&self) -> [u8; 8] {
        let (rt, len) = match &self.body {
            PagedBody::Get { len } => (crate::REQ_TYPE_IN, *len),
            PagedBody::Set(d) => (
                crate::REQ_TYPE_OUT,
                u16::try_from(d.len()).unwrap_or(u16::MAX),
            ),
        };
        let [v0, v1] = self.value.to_le_bytes();
        let [i0, i1] = self.id.w_index().to_le_bytes();
        let [l0, l1] = len.to_le_bytes();
        [rt, self.id.cmd(), v0, v1, i0, i1, l0, l1]
    }
}

/// What the wire said to one paged request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer<'a> {
    /// A GET's reply bytes.
    Data(&'a [u8]),
    /// A SET was ACKed.
    Ack,
    /// A STALL (USB) or an error status (MIDI, UART, a hub). `result` is the
    /// `VC_*` code when known: from `SYS.LAST_RESULT` when it matched the request
    /// (S2), or the status byte of a framed transport.
    Refused { result: Option<u8> },
}

/// The 1.1.6-shaped result of a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum Outcome {
    /// The IN reply (a read's bytes, or a write-as-read's status byte).
    Reply(Vec<u8>),
    /// The OUT write was accepted.
    Ack,
    /// The OUT write was refused but 1.1.6 ACKed such a write silently
    /// (`out_stall` default), so the 1.1.6 shape is an ACK; `result` says why.
    Swallowed { result: Option<u8> },
    /// Raise a STALL to the 1.1.6 caller; `result` is the paged refusal, if known.
    Stall { result: Option<u8> },
}

/// What the transport does next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Send this request and [`Plan::feed`] its answer.
    Send(PagedRequest),
    /// Finished.
    Done(Outcome),
}

/// A multi-step recipe (the custom rows, WP PGCUST). Never panics on any answer.
pub trait StepMachine: Send + core::fmt::Debug {
    /// The first step.
    fn start(&mut self) -> Next;
    /// The answer to the last request sent; the next step.
    fn feed(&mut self, answer: Answer<'_>) -> Next;
}

/// Why a 1.1.6 request was not planned. Nothing was sent; a transport answers
/// the caller as it would a STALL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("opcode 0x{0:02X} is not a 1.1.6 opcode")]
    UnknownOpcode(u8),
    #[error("1.1.6 had no {dir:?} form of opcode 0x{opcode:02X}")]
    NoDirection { opcode: u8, dir: UsbDir },
    #[error("1.1.6 refused opcode 0x{opcode:02X} with wValue 0x{value:04X}")]
    RefusedBy116 { opcode: u8, value: u16 },
    #[error("the device does not implement {id:?} {dir:?}")]
    Absent { id: CmdId, dir: PagedDir },
    #[error("wValue 0x{value:04X} of opcode 0x{opcode:02X} gives no valid address")]
    Address { opcode: u8, value: u16 },
    #[error("{id:?} takes {min}..={max} bytes, not {len}")]
    Length {
        id: CmdId,
        len: usize,
        min: u8,
        max: u8,
    },
    #[error("a GET with wLength 0 ({id:?})")]
    ZeroLengthGet { id: CmdId },
    #[error("the device has no extension channel")]
    NoExtChannel,
    #[error("opcode 0x{opcode:02X} needs a recipe this build does not have")]
    Unsupported { opcode: u8 },
    #[error("the vendored table is inconsistent at opcode 0x{opcode:02X}")]
    Table { opcode: u8 },
}

/// How the one request of a simple plan becomes the 1.1.6 reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rule {
    /// A read: the reply bytes, cut to the asked length; `on_stall` may fill.
    Data { len: u16, on_stall: Option<OnStall> },
    /// A write: an ACK; a refusal raised or swallowed.
    Ack(OutStall),
    /// A former write-as-read: its status byte, cut to the asked length.
    Status { len: u16, status: Status },
}

impl Rule {
    fn outcome(self, answer: Answer<'_>) -> Outcome {
        match (self, answer) {
            (Rule::Data { len, .. }, Answer::Data(b)) => {
                Outcome::Reply(b.iter().take(usize::from(len)).copied().collect())
            }
            (Rule::Data { .. }, Answer::Ack) => Outcome::Reply(Vec::new()),
            (Rule::Data { len, on_stall }, Answer::Refused { result }) => {
                match (on_stall, result) {
                    (Some(f), Some(r)) if f.codes.contains(&r) => {
                        Outcome::Reply(vec![f.fill; usize::from(f.len.min(len))])
                    }
                    _ => Outcome::Stall { result },
                }
            }
            (Rule::Ack(_), Answer::Data(_) | Answer::Ack) => Outcome::Ack,
            (Rule::Ack(OutStall::Swallow), Answer::Refused { result }) => {
                Outcome::Swallowed { result }
            }
            (Rule::Ack(OutStall::Raise), Answer::Refused { result }) => Outcome::Stall { result },
            (Rule::Status { len, status }, Answer::Data(_) | Answer::Ack) => {
                Outcome::Reply(vec![status.success; usize::from(len.min(1))])
            }
            (Rule::Status { len, status }, Answer::Refused { result }) => {
                match result.and_then(|r| status.byte_for(r)) {
                    Some(byte) => Outcome::Reply(vec![byte; usize::from(len.min(1))]),
                    None => Outcome::Stall { result },
                }
            }
        }
    }

    fn needs_result(self) -> bool {
        match self {
            Rule::Data { on_stall, .. } => on_stall.is_some(),
            Rule::Ack(_) => false,
            Rule::Status { status, .. } => !status.from_vc.is_empty(),
        }
    }
}

#[derive(Debug)]
enum Inner {
    Single { req: PagedRequest, rule: Rule },
    Local(Outcome),
    Machine(Box<dyn StepMachine>),
}

/// A planned 1.1.6 request: a step machine over paged requests.
#[derive(Debug)]
pub struct Plan(Inner);

impl Plan {
    /// A plan that sends nothing and answers `outcome`.
    pub fn local(outcome: Outcome) -> Self {
        Plan(Inner::Local(outcome))
    }

    /// A multi-step plan (the custom rows).
    pub fn machine(m: Box<dyn StepMachine>) -> Self {
        Plan(Inner::Machine(m))
    }

    /// The one request of a single-step plan, `None` for a local or multi-step
    /// one. A transport may batch single-step plans and subscribe polls to them
    /// (PLAN-PG R3, OD-PGo).
    pub fn single(&self) -> Option<&PagedRequest> {
        match &self.0 {
            Inner::Single { req, .. } => Some(req),
            _ => None,
        }
    }

    /// Whether the outcome of a refusal depends on its result code, so a USB
    /// transport should read `SYS.LAST_RESULT` after a STALL (a multi-step plan
    /// always says yes).
    pub fn needs_result(&self) -> bool {
        match &self.0 {
            Inner::Single { rule, .. } => rule.needs_result(),
            Inner::Local(_) => false,
            Inner::Machine(_) => true,
        }
    }

    /// The first step.
    pub fn start(&mut self) -> Next {
        match &mut self.0 {
            Inner::Single { req, .. } => Next::Send(req.clone()),
            Inner::Local(o) => Next::Done(o.clone()),
            Inner::Machine(m) => m.start(),
        }
    }

    /// The answer to the last request sent; the next step.
    pub fn feed(&mut self, answer: Answer<'_>) -> Next {
        match &mut self.0 {
            Inner::Single { rule, .. } => Next::Done(rule.outcome(answer)),
            Inner::Local(o) => Next::Done(o.clone()),
            Inner::Machine(m) => m.feed(answer),
        }
    }

    /// Run the plan to its outcome with `wire` answering each request; at most
    /// `max_steps` requests (a guard against a recipe that never finishes:
    /// exceeding it is a STALL).
    pub fn run(
        mut self,
        max_steps: usize,
        mut wire: impl FnMut(&PagedRequest) -> WireAnswer,
    ) -> Outcome {
        let mut next = self.start();
        for _ in 0..max_steps {
            match next {
                Next::Done(o) => return o,
                Next::Send(req) => {
                    let a = wire(&req);
                    next = self.feed(a.as_answer());
                }
            }
        }
        match next {
            Next::Done(o) => o,
            Next::Send(_) => Outcome::Stall { result: None },
        }
    }
}

/// An owned [`Answer`], for [`Plan::run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireAnswer {
    Data(Vec<u8>),
    Ack,
    Refused { result: Option<u8> },
}

impl WireAnswer {
    /// Borrow as an [`Answer`].
    pub fn as_answer(&self) -> Answer<'_> {
        match self {
            WireAnswer::Data(d) => Answer::Data(d),
            WireAnswer::Ack => Answer::Ack,
            WireAnswer::Refused { result } => Answer::Refused { result: *result },
        }
    }
}

/// Plan a 1.1.6 request for a paged device that implements `pages`.
pub fn plan(
    opcode: u8,
    value: u16,
    body: LegacyBody<'_>,
    pages: &PageSet,
) -> Result<Plan, PlanError> {
    plan_in(opcode, value, body, Some(pages))
}

/// The paged commands a 1.1.6 request reaches, with their directions (for the
/// hub's judges, PLAN-PG section 1.6 and OD-PGh: a translated request is judged by
/// the stricter of its classes). A custom row reaches every command its recipe
/// `uses`; the extension row reaches the tunnel. Empty for a request 1.1.6
/// refused, an unknown opcode or a direction 1.1.6 never had.
pub fn reaches(opcode: u8, value: u16, dir: UsbDir) -> Vec<(CmdId, PagedDir)> {
    if opcode == REQ_EXTENSION {
        return vec![(EXT_TUNNEL, paged_dir(dir))];
    }
    let Some(op) = legacy::op(opcode) else {
        return Vec::new();
    };
    let Some(mut a) = side(op, dir) else {
        return Vec::new();
    };
    while let Adapter::Split(s) = a {
        match s.case(value) {
            Some(c) => a = c,
            None => return Vec::new(),
        }
    }
    match a {
        Adapter::Direct(d) => vec![(d.target, paged_dir(dir))],
        Adapter::ActValue(x) | Adapter::ActAddr(x) | Adapter::ActPacked(x) => {
            vec![(x.target, PagedDir::Set)]
        }
        Adapter::Custom(c) => c.uses.to_vec(),
        Adapter::Split(_) => Vec::new(),
    }
}

fn paged_dir(dir: UsbDir) -> PagedDir {
    match dir {
        UsbDir::In => PagedDir::Get,
        UsbDir::Out => PagedDir::Set,
    }
}

fn side(op: &'static LegacyOp, dir: UsbDir) -> Option<&'static Adapter> {
    match dir {
        UsbDir::In => op.r#in,
        UsbDir::Out => op.out,
    }
}

/// The planner. `pages` `None` plans without the device checks (presence and
/// SET lengths): the reverse view verifies its candidates this way.
pub(super) fn plan_in(
    opcode: u8,
    value: u16,
    body: LegacyBody<'_>,
    pages: Option<&PageSet>,
) -> Result<Plan, PlanError> {
    if opcode == REQ_EXTENSION {
        return plan_ext(value, body, pages);
    }
    let op = legacy::op(opcode).ok_or(PlanError::UnknownOpcode(opcode))?;
    let dir = body.dir();
    let adapter = side(op, dir).ok_or(PlanError::NoDirection { opcode, dir })?;
    let ctx = Ctx {
        op,
        value,
        body,
        pages,
    };
    ctx.resolve(adapter, OutStall::Swallow)
}

/// Our extension row (PLAN-PG section 1.3).
fn plan_ext(value: u16, body: LegacyBody<'_>, pages: Option<&PageSet>) -> Result<Plan, PlanError> {
    let dir = paged_dir(body.dir());
    if pages.is_some_and(|p| !p.contains(EXT_TUNNEL, dir)) {
        return Err(PlanError::NoExtChannel);
    }
    let (body, rule) = match body {
        LegacyBody::In { len: 0 } => return Err(PlanError::ZeroLengthGet { id: EXT_TUNNEL }),
        LegacyBody::In { len } => (
            PagedBody::Get { len },
            Rule::Data {
                len,
                on_stall: None,
            },
        ),
        // Our extension channel STALLs every refusal (extension_channel_spec.md,
        // "a refusal is a STALL"): nothing to swallow.
        LegacyBody::Out(d) => (PagedBody::Set(d.to_vec()), Rule::Ack(OutStall::Raise)),
    };
    Ok(Plan(Inner::Single {
        req: PagedRequest {
            id: EXT_TUNNEL,
            value,
            body,
        },
        rule,
    }))
}

struct Ctx<'a> {
    op: &'static LegacyOp,
    value: u16,
    body: LegacyBody<'a>,
    pages: Option<&'a PageSet>,
}

impl Ctx<'_> {
    fn resolve(&self, a: &'static Adapter, out_stall: OutStall) -> Result<Plan, PlanError> {
        let opcode = self.op.code;
        match a {
            Adapter::Split(s) => {
                // A split's own `out_stall: raise` covers its cases (UPMIX_SET_PARAM).
                let inherited = match (out_stall, s.out_stall) {
                    (OutStall::Raise, _) | (_, OutStall::Raise) => OutStall::Raise,
                    _ => OutStall::Swallow,
                };
                match s.case(self.value) {
                    Some(case) => self.resolve(case, inherited),
                    None => match (self.body, inherited) {
                        // A null case: 1.1.6 swallowed such an OUT (schema `split`).
                        (LegacyBody::Out(_), OutStall::Swallow) => {
                            Ok(Plan::local(Outcome::Swallowed { result: None }))
                        }
                        _ => Err(PlanError::RefusedBy116 {
                            opcode,
                            value: self.value,
                        }),
                    },
                }
            }
            Adapter::Direct(d) => self.direct(d, out_stall),
            Adapter::ActValue(x) | Adapter::ActAddr(x) | Adapter::ActPacked(x) => self.act(x),
            Adapter::Custom(c) => {
                super::custom::plan(self.op, c, self.value, self.body, self.pages)
            }
        }
    }

    fn target(&self, id: CmdId, dir: PagedDir) -> Result<&'static Command, PlanError> {
        let c = registry::command(id).ok_or(PlanError::Table {
            opcode: self.op.code,
        })?;
        if self.pages.is_some_and(|p| !p.contains(id, dir)) {
            return Err(PlanError::Absent { id, dir });
        }
        Ok(c)
    }

    /// The paged `wValue`. A target that addresses nothing takes 0: 1.1.6 ignored
    /// `wValue` there, 1.1.7 requires 0 (upstream `_map_row`; guide 5.5).
    fn address(&self, rule: AddrRule, c: &Command) -> Result<u16, PlanError> {
        if c.addr == AddrKind::None {
            return Ok(0);
        }
        match rule {
            AddrRule::Old => Ok(self.value),
            AddrRule::Const(k) => Ok(k),
            AddrRule::Field(f) => f.apply(self.value).ok_or(PlanError::Address {
                opcode: self.op.code,
                value: self.value,
            }),
        }
    }

    fn check_set_len(&self, c: &Command, len: usize, bulk: bool) -> Result<(), PlanError> {
        let judged = self
            .pages
            .is_some_and(|p| p.revision_is_vendored(c.id.page()));
        if judged && !bulk && !c.set_len_ok(len) {
            return Err(PlanError::Length {
                id: c.id,
                len,
                min: c.set_min,
                max: c.set_max,
            });
        }
        Ok(())
    }

    fn direct(&self, d: &Direct, out_stall: OutStall) -> Result<Plan, PlanError> {
        let dir = paged_dir(self.body.dir());
        let c = self.target(d.target, dir)?;
        let value = self.address(d.addr, c)?;
        let (body, rule) = match self.body {
            LegacyBody::In { len: 0 } => return Err(PlanError::ZeroLengthGet { id: c.id }),
            LegacyBody::In { len } => (
                PagedBody::Get { len },
                Rule::Data {
                    len,
                    on_stall: d.on_stall,
                },
            ),
            LegacyBody::Out(data) => {
                let cut = d
                    .trunc
                    .map_or(data.len(), |t| data.len().min(usize::from(t)));
                let data = data.get(..cut).unwrap_or(data);
                self.check_set_len(c, data.len(), d.bulk)?;
                let stall = match (out_stall, d.out_stall) {
                    (OutStall::Raise, _) | (_, OutStall::Raise) => OutStall::Raise,
                    _ => OutStall::Swallow,
                };
                (PagedBody::Set(data.to_vec()), Rule::Ack(stall))
            }
        };
        Ok(Plan(Inner::Single {
            req: PagedRequest {
                id: c.id,
                value,
                body,
            },
            rule,
        }))
    }

    fn act(&self, x: &Act) -> Result<Plan, PlanError> {
        let LegacyBody::In { len } = self.body else {
            // The generator puts act_* rows on IN forms only.
            return Err(PlanError::Table {
                opcode: self.op.code,
            });
        };
        let c = self.target(x.target, PagedDir::Set)?;
        let value = self.address(x.addr, c)?;
        let data = match x.value {
            Some((field, encode)) => vec![encode.byte(field.extract(self.value))],
            None => Vec::new(),
        };
        self.check_set_len(c, data.len(), false)?;
        Ok(Plan(Inner::Single {
            req: PagedRequest {
                id: c.id,
                value,
                body: PagedBody::Set(data),
            },
            rule: Rule::Status {
                len,
                status: x.status,
            },
        }))
    }
}
