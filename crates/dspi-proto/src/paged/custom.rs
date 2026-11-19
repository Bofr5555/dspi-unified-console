//! The 14 `custom` rows of `legacy_map.json`: requests with no one-to-one rule,
//! each written by hand from the migration guide's recipe (upstream
//! `Documentation/Features/vendor_protocol_migration_guide.md` at `103ba37`,
//! sections 6.3, 6.5 and 6.7) and checked against the 1.1.6 handlers
//! (`firmware/DSPi/vendor_commands.c` at `v1.1.6`). **WP PGCUST.**
//!
//! Each recipe is a [`StepMachine`]; one that sends exactly one request is a
//! [`Plan::one_step`] (so a transport may batch it, e.g. the meter poll
//! `GET_STATUS` 9), one that sends nothing is a [`Plan::local`]. Before anything
//! is sent every command the recipe will send is checked against the device's
//! [`PageSet`] (absent: refused locally, nothing on the wire).
//!
//! The rules every recipe keeps:
//! - a read whose paged GET is refused raises the STALL (1.1.6 STALLed a bad
//!   address too); a reply too short for the field the recipe takes is a STALL
//!   with no result (the device broke the registry's length);
//! - a write 1.1.6 ignored silently when it was wrong (`SET_EQ_PARAM`,
//!   `SET_MATRIX_ROUTE`, `SET_PREAMP`) answers [`Outcome::Swallowed`] when the
//!   device refuses it and nothing was applied;
//! - **a refusal after part of a multi-request write was applied is a STALL**
//!   (`SET_PREAMP` after its first channel, `CLEAR_CLIPS` after reading,
//!   `SAVE_PARAMS`): never reported as success;
//! - an action 1.1.6 never refused (`SAVE_PARAMS`, `CLEAR_CLIPS`) raises every
//!   paged refusal rather than invent a status byte.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::legacy::{Custom, LegacyOp, OldDir, VC_ERR_UNKNOWN};
use super::plan::{
    Answer, LegacyBody, Next, Outcome, PagedBody, PagedRequest, Plan, PlanError, StepMachine,
    UsbDir, plan_in, reaches,
};
use super::registry::{self, Command, GetLen, cmds};
use super::reverse::LegacyEquivalent;
use super::{CmdId, PageSet, PagedDir};

#[cfg(test)]
mod tests;

/// `FLASH_OK` (1.1.6 `flash_storage.h`): what `SAVE_PARAMS` always answered.
const FLASH_OK: u8 = 0x00;

/// `GET_USB_ERROR_STATS`' reply: six u32 counters, always zero in 1.1.6 (guide 6.7).
const USB_ERROR_STATS_LEN: usize = 24;

/// 1.1.6 ACKed and ignored a `SET_ALL_PARAMS` payload of this many bytes or
/// fewer and STALLed every other wrong length at SETUP (guide 6.5).
const ALL_PARAMS_IGNORED_UP_TO: usize = 64;

/// Plan a custom row (WP PGCUST). Dispatched by the row's `fn`.
pub(super) fn plan(
    op: &'static LegacyOp,
    row: &'static Custom,
    value: u16,
    body: LegacyBody<'_>,
    pages: Option<&PageSet>,
) -> Result<Plan, PlanError> {
    let env = Env { op, pages };
    match row.func {
        "cs_display_cfg" => cs_display_cfg(&env, env.in_len(body)?),
        "set_eq_param" => set_eq_param(&env, env.out_data(body)?),
        "get_eq_param" => get_eq_param(&env, value, env.in_len(body)?),
        "set_preamp_legacy" => set_preamp(&env, env.out_data(body)?),
        "get_status" => get_status(&env, value, env.in_len(body)?),
        "save_params" => save_params(&env, env.in_len(body)?),
        "set_matrix_route" => set_matrix_route(&env, env.out_data(body)?),
        "get_matrix_route" => get_matrix_route(&env, value, env.in_len(body)?),
        "clear_clips" => clear_clips(&env, env.in_len(body)?),
        "preset_get_startup" => preset_get_startup(&env, env.in_len(body)?),
        "set_all_params" => set_all_params(&env, row, env.out_data(body)?),
        "usb_error_stats" => {
            let len = env.in_len(body)?;
            Ok(Plan::local(Outcome::Reply(cut(
                vec![0; USB_ERROR_STATS_LEN],
                len,
            ))))
        }
        "reset_usb_error_stats" => {
            // 1.1.6 did nothing and answered the acknowledge byte 0x01 (guide 6.7).
            let len = env.in_len(body)?;
            Ok(Plan::local(Outcome::Reply(cut(vec![0x01], len))))
        }
        "get_input_rate" => get_input_rate(&env, env.in_len(body)?),
        _ => Err(PlanError::Unsupported { opcode: op.code }),
    }
}

// ---------------------------------------------------------------------------
// Building requests

struct Env<'a> {
    op: &'static LegacyOp,
    pages: Option<&'a PageSet>,
}

impl Env<'_> {
    fn in_len(&self, body: LegacyBody<'_>) -> Result<u16, PlanError> {
        match body {
            LegacyBody::In { len } => Ok(len),
            LegacyBody::Out(_) => Err(PlanError::Table {
                opcode: self.op.code,
            }),
        }
    }

    fn out_data<'b>(&self, body: LegacyBody<'b>) -> Result<&'b [u8], PlanError> {
        match body {
            LegacyBody::Out(d) => Ok(d),
            LegacyBody::In { .. } => Err(PlanError::Table {
                opcode: self.op.code,
            }),
        }
    }

    /// The registry row of `id`, refused when the device does not list it.
    fn need(&self, id: CmdId, dir: PagedDir) -> Result<&'static Command, PlanError> {
        let c = registry::command(id).ok_or(PlanError::Table {
            opcode: self.op.code,
        })?;
        if self.pages.is_some_and(|p| !p.contains(id, dir)) {
            return Err(PlanError::Absent { id, dir });
        }
        Ok(c)
    }

    /// Whether the device lists `id` for `dir` (always, without a page set).
    fn has(&self, id: CmdId, dir: PagedDir) -> bool {
        self.pages.is_none_or(|p| p.contains(id, dir))
    }

    /// A GET of `len` bytes (never 0).
    fn get(&self, id: CmdId, value: u16, len: u16) -> Result<PagedRequest, PlanError> {
        self.need(id, PagedDir::Get)?;
        if len == 0 {
            return Err(PlanError::ZeroLengthGet { id });
        }
        Ok(PagedRequest {
            id,
            value,
            body: PagedBody::Get { len },
        })
    }

    /// A GET of the command's whole reply (its registry length, else `fallback`).
    fn get_whole(&self, id: CmdId, value: u16, fallback: u16) -> Result<PagedRequest, PlanError> {
        let len = match registry::command(id).map(|c| c.get_len) {
            Some(GetLen::Fixed(n)) if n > 0 => n,
            _ => fallback,
        };
        self.get(id, value, len)
    }

    /// A SET; its length is judged locally only at the vendored page revision (R8b).
    fn set(&self, id: CmdId, value: u16, data: Vec<u8>) -> Result<PagedRequest, PlanError> {
        let c = self.need(id, PagedDir::Set)?;
        let judged = self
            .pages
            .is_some_and(|p| p.revision_is_vendored(id.page()));
        if judged && !c.set_len_ok(data.len()) {
            return Err(PlanError::Length {
                id,
                len: data.len(),
                min: c.set_min,
                max: c.set_max,
            });
        }
        Ok(PagedRequest {
            id,
            value,
            body: PagedBody::Set(data),
        })
    }
}

/// `v` cut to the `wLength` the 1.1.6 caller asked for.
fn cut(mut v: Vec<u8>, len: u16) -> Vec<u8> {
    v.truncate(usize::from(len));
    v
}

fn byte(b: &[u8], at: usize) -> Option<u8> {
    b.get(at).copied()
}

fn bytes<const N: usize>(b: &[u8], at: usize) -> Option<[u8; N]> {
    b.get(at..at.checked_add(N)?)?.try_into().ok()
}

// ---------------------------------------------------------------------------
// The generic machine: requests known up front, one reply built from the answers

/// What a refusal of any step answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refuse {
    /// Raise the STALL.
    Stall,
    /// 1.1.6 ignored such a write silently and nothing was applied.
    Swallow,
    /// `VC_ERR_UNKNOWN` (a command absent from this build) reads as zero bytes,
    /// as 1.1.6 answered (`zero_if_absent`); anything else raises.
    ZeroIfUnknown { len: u16 },
}

/// How the answers become the 1.1.6 reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finish {
    /// A write: ACK.
    Ack,
    /// The first reply as it is, cut to `len`.
    Raw { len: u16 },
    /// A `GET_STATUS` scalar: one field of the first reply, as the old u32.
    Field { ty: Ty, off: u8, len: u16 },
    /// `GET_STATUS` 0 and 1: two u16 peaks of `MEAS.LEVELS`.
    Peaks { a: u8, b: u8, len: u16 },
    /// `GET_STATUS` 2: a peak of `MEAS.LEVELS`, then two bytes of `SYS.SYSTEM_STATUS`.
    PeakCpu { ch: u8, lo: u8, hi: u8, len: u16 },
    /// `GET_EQ_PARAM`: one parameter of the `FILTER.BAND` reply, as the old u32.
    EqParam { param: u8, len: u16 },
    /// `GET_MATRIX_ROUTE`: the old packet rebuilt from the new reply.
    Matrix { value: u16, len: u16 },
    /// `GET_CS_DISPLAY_CFG`: 4 caps bytes, then the 12-byte config (if asked).
    CsDisplay { len: u16 },
    /// `PRESET_GET_STARTUP`: `STORE.STARTUP`'s 2 bytes, then `STORE.DIRECTORY` byte 4 (if asked).
    Startup { len: u16 },
    /// `GET_INPUT_RATE`: `SYS.SYSTEM_STATUS` bytes 4-7, then `INPUT.I2S_RX_RATE` (if asked).
    InputRate { len: u16 },
    /// `CLEAR_CLIPS`: the 4 flag bytes read first (the clear's ACK follows).
    ClipFlags { len: u16 },
}

impl Finish {
    /// `None`: a reply too short for what the recipe takes.
    fn build(self, got: &[Vec<u8>]) -> Option<Outcome> {
        let first = got.first().map_or(&[][..], Vec::as_slice);
        let second = got.get(1).map(Vec::as_slice);
        let reply = match self {
            Finish::Ack => return Some(Outcome::Ack),
            Finish::Raw { len } => cut(first.to_vec(), len),
            Finish::Field { ty, off, len } => cut(
                ty.read(first, usize::from(off))?.to_le_bytes().to_vec(),
                len,
            ),
            Finish::Peaks { a, b, len } => {
                let pa = bytes::<2>(first, usize::from(a).checked_mul(2)?)?;
                let pb = bytes::<2>(first, usize::from(b).checked_mul(2)?)?;
                cut([pa, pb].concat(), len)
            }
            Finish::PeakCpu { ch, lo, hi, len } => {
                let [p0, p1] = bytes::<2>(first, usize::from(ch).checked_mul(2)?)?;
                let s = second?;
                cut(
                    vec![p0, p1, byte(s, usize::from(lo))?, byte(s, usize::from(hi))?],
                    len,
                )
            }
            Finish::EqParam { param, len } => {
                // Guide 6.5, the old 4-byte reads: type byte 0, freq 4-7, Q 8-11,
                // gain 12-15, bypass byte 1, Qp bytes 2-3; params 6 and 7 read 0.
                let v: [u8; 4] = match param {
                    0 => [byte(first, 0)?, 0, 0, 0],
                    1 => bytes::<4>(first, 4)?,
                    2 => bytes::<4>(first, 8)?,
                    3 => bytes::<4>(first, 12)?,
                    4 => [byte(first, 1)?, 0, 0, 0],
                    5 => {
                        let [a, b] = bytes::<2>(first, 2)?;
                        [a, b, 0, 0]
                    }
                    _ => [0; 4],
                };
                cut(v.to_vec(), len)
            }
            Finish::Matrix { value, len } => {
                // Guide 6.5: input, output, r[0], r[1], r[4..7].
                let [input, output] = value.to_be_bytes();
                let head = bytes::<2>(first, 0)?;
                let gain = bytes::<4>(first, 4)?;
                cut([&[input, output][..], &head, &gain].concat(), len)
            }
            Finish::CsDisplay { len } => match second {
                None => cut(first.to_vec(), len),
                Some(cfg) => cut([&bytes::<4>(first, 0)?[..], cfg].concat(), len),
            },
            Finish::Startup { len } => {
                let mut v = bytes::<2>(first, 0)?.to_vec();
                if let Some(dir) = second {
                    v.push(byte(dir, 4)?);
                }
                cut(v, len)
            }
            Finish::InputRate { len } => {
                let mut v = bytes::<4>(first, 4)?.to_vec();
                if let Some(rate) = second {
                    v.extend_from_slice(&bytes::<4>(rate, 0)?);
                }
                cut(v, len)
            }
            Finish::ClipFlags { len } => cut(bytes::<4>(first, 0)?.to_vec(), len),
        };
        Some(Outcome::Reply(reply))
    }
}

/// Requests known up front, sent in order; any refusal ends the recipe.
#[derive(Debug)]
struct Seq {
    reqs: Vec<PagedRequest>,
    got: Vec<Vec<u8>>,
    finish: Finish,
    refuse: Refuse,
}

impl Seq {
    fn plan(reqs: Vec<PagedRequest>, finish: Finish, refuse: Refuse) -> Plan {
        let one = reqs.len() == 1;
        let m = Box::new(Seq {
            reqs,
            got: Vec::new(),
            finish,
            refuse,
        });
        if one {
            Plan::one_step(m)
        } else {
            Plan::machine(m)
        }
    }

    fn next(&self) -> Next {
        match self.reqs.get(self.got.len()) {
            Some(r) => Next::Send(r.clone()),
            None => Next::Done(
                self.finish
                    .build(&self.got)
                    .unwrap_or(Outcome::Stall { result: None }),
            ),
        }
    }
}

impl StepMachine for Seq {
    fn start(&mut self) -> Next {
        self.got.clear();
        self.next()
    }

    fn feed(&mut self, answer: Answer<'_>) -> Next {
        if self.got.len() >= self.reqs.len() {
            return Next::Done(Outcome::Stall { result: None });
        }
        match answer {
            Answer::Data(b) => self.got.push(b.to_vec()),
            Answer::Ack => self.got.push(Vec::new()),
            Answer::Refused { result } => {
                return Next::Done(match self.refuse {
                    Refuse::Stall => Outcome::Stall { result },
                    Refuse::Swallow => Outcome::Swallowed { result },
                    Refuse::ZeroIfUnknown { len } if result == Some(VC_ERR_UNKNOWN) => {
                        Outcome::Reply(cut(vec![0; 4], len))
                    }
                    Refuse::ZeroIfUnknown { .. } => Outcome::Stall { result },
                });
            }
        }
        self.next()
    }
}

// ---------------------------------------------------------------------------
// The rows

/// `GET_CS_DISPLAY_CFG` 0x28 (guide 6.5): `CS.DISPLAY_CAPS` (4) then
/// `CS.DISPLAY_CONFIG` (12), the old 16 bytes. A read of 4 bytes or fewer needs
/// only the caps.
fn cs_display_cfg(env: &Env<'_>, len: u16) -> Result<Plan, PlanError> {
    if len == 0 {
        return Err(PlanError::ZeroLengthGet {
            id: cmds::CS_DISPLAY_CAPS,
        });
    }
    let mut reqs = vec![env.get_whole(cmds::CS_DISPLAY_CAPS, 0, 4)?];
    if len > 4 {
        reqs.push(env.get_whole(cmds::CS_DISPLAY_CONFIG, 0, 12)?);
    }
    Ok(Seq::plan(reqs, Finish::CsDisplay { len }, Refuse::Stall))
}

/// The old `EqParamPacket`'s fields in the new `FILTER.BAND` order with the Qp
/// `qp` (guide 6.5): `wValue = (p[0] << 8) | p[1]`, payload `p[2], p[3], qp,
/// p[4..16]`. `None` for a packet shorter than 16 bytes.
fn eq_band(p: &[u8], qp: [u8; 2]) -> Option<(u16, Vec<u8>)> {
    let [ch, band, ty, bypass] = bytes::<4>(p, 0)?;
    let floats = bytes::<12>(p, 4)?;
    let value = u16::from_be_bytes([ch, band]);
    Some((value, [&[ty, bypass][..], &qp, &floats].concat()))
}

/// `SET_EQ_PARAM` 0x42 (guide 6.5). 1.1.6 took 16 bytes (kept the stored Linkwitz
/// Qp) or 18 (bytes 16-17 the Qp); a shorter payload was ignored. 1.1.7 always
/// stores the Qp it is sent (rule 1), so a 16-byte write reads the band first and
/// copies its Qp. A refused write was ignored by 1.1.6: swallowed.
fn set_eq_param(env: &Env<'_>, p: &[u8]) -> Result<Plan, PlanError> {
    let Some((value, _)) = eq_band(p, [0, 0]) else {
        return Ok(Plan::local(Outcome::Swallowed { result: None }));
    };
    if let Some(qp) = bytes::<2>(p, 16) {
        let (value, data) = eq_band(p, qp).ok_or(PlanError::Table {
            opcode: env.op.code,
        })?;
        let req = env.set(cmds::FILTER_BAND, value, data)?;
        return Ok(Seq::plan(vec![req], Finish::Ack, Refuse::Swallow));
    }
    let read = env.get_whole(cmds::FILTER_BAND, value, 16)?;
    // Checked now so the write cannot be refused locally half-way.
    env.set(cmds::FILTER_BAND, value, vec![0; 16])?;
    let packet: [u8; 16] = bytes::<16>(p, 0).ok_or(PlanError::Table {
        opcode: env.op.code,
    })?;
    Ok(Plan::machine(Box::new(EqKeepQp {
        read,
        packet,
        stage: 0,
    })))
}

/// `SET_EQ_PARAM` with 16 bytes: read the band's Qp, then write the band with it.
#[derive(Debug)]
struct EqKeepQp {
    read: PagedRequest,
    packet: [u8; 16],
    stage: u8,
}

impl StepMachine for EqKeepQp {
    fn start(&mut self) -> Next {
        self.stage = 1;
        Next::Send(self.read.clone())
    }

    fn feed(&mut self, answer: Answer<'_>) -> Next {
        match (self.stage, answer) {
            // Nothing was written: 1.1.6 ignored a write to a band it refused.
            (1 | 2, Answer::Refused { result }) => Next::Done(Outcome::Swallowed { result }),
            (1, Answer::Data(b)) => match bytes::<2>(b, 2).and_then(|qp| eq_band(&self.packet, qp))
            {
                Some((value, data)) => {
                    self.stage = 2;
                    Next::Send(PagedRequest {
                        id: self.read.id,
                        value,
                        body: PagedBody::Set(data),
                    })
                }
                None => Next::Done(Outcome::Stall { result: None }),
            },
            (2, Answer::Ack | Answer::Data(_)) => Next::Done(Outcome::Ack),
            _ => Next::Done(Outcome::Stall { result: None }),
        }
    }
}

/// `GET_EQ_PARAM` 0x43 (guide 6.5): old `wValue` `(channel << 8) | (band << 3) |
/// param`; reads `FILTER.BAND` at `(channel << 8) | band` and answers the one
/// parameter as the old 4 bytes.
fn get_eq_param(env: &Env<'_>, value: u16, len: u16) -> Result<Plan, PlanError> {
    if len == 0 {
        return Err(PlanError::ZeroLengthGet {
            id: cmds::FILTER_BAND,
        });
    }
    let [_, low] = value.to_be_bytes();
    let band = u16::from(low.checked_shr(3).unwrap_or(0) & 0x1F);
    let param = low & 0x07;
    let req = env.get_whole(cmds::FILTER_BAND, (value & 0xFF00) | band, 16)?;
    Ok(Seq::plan(
        vec![req],
        Finish::EqParam { param, len },
        Refuse::Stall,
    ))
}

/// `SET_PREAMP` 0x44, retired (guide 6.7): 1.1.6 wrote the same f32 dB to every
/// input; now one `ROUTE.PREAMP` SET per input channel, the count from
/// `SYS.PLATFORM` byte 0 (0 RP2040: 2 inputs, 1 RP2350: 8; guide 4.6). Fewer than
/// 4 bytes was ignored.
fn set_preamp(env: &Env<'_>, p: &[u8]) -> Result<Plan, PlanError> {
    let Some(db) = bytes::<4>(p, 0) else {
        return Ok(Plan::local(Outcome::Swallowed { result: None }));
    };
    let platform = env.get_whole(cmds::SYS_PLATFORM, 0, 7)?;
    env.set(cmds::ROUTE_PREAMP, 0, db.to_vec())?;
    Ok(Plan::machine(Box::new(Preamp {
        platform,
        db,
        inputs: 0,
        next: 0,
    })))
}

/// The input channels of a platform (`SYS.PLATFORM` byte 0, guide 4.6 and 6.7).
fn inputs_of(platform: u8) -> Option<u16> {
    match platform {
        0 => Some(2),
        1 => Some(8),
        _ => None,
    }
}

#[derive(Debug)]
struct Preamp {
    platform: PagedRequest,
    db: [u8; 4],
    /// 0 until the platform is known.
    inputs: u16,
    /// The next channel to write.
    next: u16,
}

impl Preamp {
    fn write(&mut self) -> Next {
        let ch = self.next;
        self.next = self.next.saturating_add(1);
        Next::Send(PagedRequest {
            id: cmds::ROUTE_PREAMP,
            value: ch,
            body: PagedBody::Set(self.db.to_vec()),
        })
    }
}

impl StepMachine for Preamp {
    fn start(&mut self) -> Next {
        self.inputs = 0;
        self.next = 0;
        Next::Send(self.platform.clone())
    }

    fn feed(&mut self, answer: Answer<'_>) -> Next {
        if self.inputs == 0 {
            return match answer {
                Answer::Data(b) => match byte(b, 0).and_then(inputs_of) {
                    Some(n) => {
                        self.inputs = n;
                        self.write()
                    }
                    None => Next::Done(Outcome::Stall { result: None }),
                },
                Answer::Ack => Next::Done(Outcome::Stall { result: None }),
                Answer::Refused { result } => Next::Done(Outcome::Stall { result }),
            };
        }
        match answer {
            // The first channel refused: nothing applied, as 1.1.6 ignoring it.
            Answer::Refused { result } if self.next <= 1 => {
                Next::Done(Outcome::Swallowed { result })
            }
            // Some channels have the new gain and some not: never success.
            Answer::Refused { result } => Next::Done(Outcome::Stall { result }),
            Answer::Ack | Answer::Data(_) if self.next < self.inputs => self.write(),
            Answer::Ack | Answer::Data(_) => Next::Done(Outcome::Ack),
        }
    }
}

/// The old 4-byte `GET_STATUS` value's type in its struct (guide 6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ty {
    U8,
    U16,
    I16,
    U32,
    I32,
}

impl Ty {
    /// The field at `off`, widened as the old 4-byte value (zero- or sign-extended).
    fn read(self, b: &[u8], off: usize) -> Option<u32> {
        Some(match self {
            Ty::U8 => u32::from(byte(b, off)?),
            Ty::U16 => u32::from(u16::from_le_bytes(bytes(b, off)?)),
            Ty::I16 => {
                u32::from_le_bytes(i32::from(i16::from_le_bytes(bytes(b, off)?)).to_le_bytes())
            }
            Ty::U32 | Ty::I32 => u32::from_le_bytes(bytes(b, off)?),
        })
    }

    fn size(self) -> u8 {
        match self {
            Ty::U8 => 1,
            Ty::U16 | Ty::I16 => 2,
            Ty::U32 | Ty::I32 => 4,
        }
    }
}

/// Where a `GET_STATUS` selector lives now (guide 6.3, `legacy_map.json`
/// `GET_STATUS` `fields`, `peaks`, `peak_cpu`, `levels_raw`, `zero`; a test holds
/// this table equal to the vendored row's data).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sel {
    /// One field of one struct.
    Field { id: CmdId, ty: Ty, off: u8 },
    /// `MEAS.LEVELS` whole: the old combined packet of selector 9.
    Levels,
    /// Two u16 peaks of `MEAS.LEVELS` (channel indices).
    Peaks(u8, u8),
    /// A `MEAS.LEVELS` peak, then two bytes of `id`.
    PeakCpu { ch: u8, id: CmdId, lo: u8, hi: u8 },
    /// No home: 4 zero bytes (selector 7, and every selector 1.1.6 did not use in
    /// a release build, the loop-profiler ones 64 and up included).
    Zero,
}

/// The `GET_STATUS` selector table.
pub(crate) fn status_selector(sel: u16) -> Sel {
    use Ty::*;
    let f = |id, ty, off| Sel::Field { id, ty, off };
    match sel {
        0 => Sel::Peaks(0, 1),
        1 => Sel::Peaks(2, 3),
        2 => Sel::PeakCpu {
            ch: 4,
            id: cmds::SYS_SYSTEM_STATUS,
            lo: 12,
            hi: 13,
        },
        3 => f(cmds::SYS_PDM_STATS, U32, 0),
        4 => f(cmds::SYS_PDM_STATS, U32, 4),
        5 => f(cmds::SYS_PDM_STATS, U32, 8),
        6 => f(cmds::SYS_PDM_STATS, U32, 12),
        8 => f(cmds::SYS_SPDIF_TX_STATS, U32, 0),
        9 => Sel::Levels,
        10 => f(cmds::SYS_USB_AUDIO_STATUS, U32, 0),
        11 => f(cmds::SYS_USB_AUDIO_STATUS, U8, 12),
        12 => f(cmds::SYS_USB_AUDIO_STATUS, U8, 13),
        13 => f(cmds::SYS_SYSTEM_STATUS, U32, 0),
        14 => f(cmds::SYS_SYSTEM_STATUS, U16, 8),
        15 => f(cmds::SYS_SYSTEM_STATUS, U32, 4),
        16 => f(cmds::SYS_SYSTEM_STATUS, I16, 10),
        17 => f(cmds::SYS_SPDIF_TX_STATS, U32, 4),
        18 => f(cmds::SYS_SPDIF_TX_STATS, U32, 8),
        19 => f(cmds::SYS_SPDIF_TX_STATS, U32, 12),
        20 => f(cmds::SYS_SPDIF_TX_STATS, U32, 16),
        21 => f(cmds::SYS_SPDIF_TX_STATS, U32, 20),
        22 => f(cmds::SYS_USB_AUDIO_STATUS, U32, 4),
        23 => f(cmds::INPUT_STATUS, U8, 1),
        24 => f(cmds::DIAG_LOOPBACK_STATS, U32, 0),
        25 => f(cmds::DIAG_LOOPBACK_STATS, U32, 4),
        26 => f(cmds::SYS_PIPELINE_STATS, U32, 0),
        27 => f(cmds::SYS_PIPELINE_STATS, U32, 4),
        28 => f(cmds::SYS_PIPELINE_STATS, U32, 8),
        29 => f(cmds::SYS_PIPELINE_STATS, U32, 12),
        30 => f(cmds::SYS_USB_AUDIO_STATUS, U32, 8),
        31 => f(cmds::SYS_PIPELINE_STATS, U32, 16),
        32 => f(cmds::INPUT_STATUS, U32, 4),
        33 => f(cmds::INPUT_STATUS, I32, 8),
        _ => Sel::Zero,
    }
}

/// Commands a release build does not have, read as zero like 1.1.6
/// (`zero_if_absent`; guide 6.3: "read 0 if `VC_ERR_UNKNOWN`").
pub(crate) const ZERO_IF_ABSENT: [CmdId; 1] = [cmds::DIAG_LOOPBACK_STATS];

/// `GET_STATUS` 0x50 (guide 6.3): each selector to its struct and offset.
/// Selectors 24 and 25 (`DIAG.LOOPBACK_STATS`, `DSPI_LOOPBACK` builds only): on
/// a device that does not list the command (a release build) nothing is sent and
/// the answer is 4 zero bytes as 1.1.6's; a `VC_ERR_UNKNOWN` refusal reads 0 too.
fn get_status(env: &Env<'_>, value: u16, len: u16) -> Result<Plan, PlanError> {
    let sel = status_selector(value);
    let zeros = || Plan::local(Outcome::Reply(cut(vec![0; 4], len)));
    let levels = |n: u8| -> Result<PagedRequest, PlanError> {
        let need = u16::from(n).saturating_add(1).saturating_mul(2);
        env.get(cmds::MEAS_LEVELS, 0, need)
    };
    Ok(match sel {
        Sel::Zero => zeros(),
        Sel::Levels => Seq::plan(
            vec![env.get(cmds::MEAS_LEVELS, 0, len)?],
            Finish::Raw { len },
            Refuse::Stall,
        ),
        Sel::Peaks(_, _) | Sel::PeakCpu { .. } | Sel::Field { .. } if len == 0 => {
            let id = match sel {
                Sel::Field { id, .. } => id,
                _ => cmds::MEAS_LEVELS,
            };
            return Err(PlanError::ZeroLengthGet { id });
        }
        Sel::Peaks(a, b) => Seq::plan(
            vec![levels(a.max(b))?],
            Finish::Peaks { a, b, len },
            Refuse::Stall,
        ),
        Sel::PeakCpu { ch, id, lo, hi } => Seq::plan(
            vec![
                levels(ch)?,
                env.get_whole(id, 0, u16::from(hi.saturating_add(1)))?,
            ],
            Finish::PeakCpu { ch, lo, hi, len },
            Refuse::Stall,
        ),
        Sel::Field { id, ty, off } => {
            let absent_reads_zero = ZERO_IF_ABSENT.contains(&id);
            if absent_reads_zero && !env.has(id, PagedDir::Get) {
                return Ok(zeros());
            }
            let end = u16::from(off.saturating_add(ty.size()));
            let req = env.get_whole(id, 0, end)?;
            let refuse = if absent_reads_zero {
                Refuse::ZeroIfUnknown { len }
            } else {
                Refuse::Stall
            };
            Seq::plan(vec![req], Finish::Field { ty, off, len }, refuse)
        }
    })
}

/// `SAVE_PARAMS` 0x51, retired (guide 6.7): read `STORE.PRESET_ACTIVE` (1 byte),
/// then `STORE.PRESET_SAVE` with that slot. 1.1.6 always answered `FLASH_OK`; a
/// refusal of either step is raised, never answered as saved.
fn save_params(env: &Env<'_>, len: u16) -> Result<Plan, PlanError> {
    let active = env.get_whole(cmds::STORE_PRESET_ACTIVE, 0, 1)?;
    env.set(cmds::STORE_PRESET_SAVE, 0, Vec::new())?;
    Ok(Plan::machine(Box::new(SaveParams {
        active,
        len,
        stage: 0,
    })))
}

#[derive(Debug)]
struct SaveParams {
    active: PagedRequest,
    len: u16,
    stage: u8,
}

impl StepMachine for SaveParams {
    fn start(&mut self) -> Next {
        self.stage = 1;
        Next::Send(self.active.clone())
    }

    fn feed(&mut self, answer: Answer<'_>) -> Next {
        match (self.stage, answer) {
            (_, Answer::Refused { result }) => Next::Done(Outcome::Stall { result }),
            (1, Answer::Data(b)) => match byte(b, 0) {
                Some(slot) => {
                    self.stage = 2;
                    Next::Send(PagedRequest {
                        id: cmds::STORE_PRESET_SAVE,
                        value: u16::from(slot),
                        body: PagedBody::Set(Vec::new()),
                    })
                }
                None => Next::Done(Outcome::Stall { result: None }),
            },
            (2, Answer::Ack | Answer::Data(_)) => {
                Next::Done(Outcome::Reply(cut(vec![FLASH_OK], self.len)))
            }
            _ => Next::Done(Outcome::Stall { result: None }),
        }
    }
}

/// `SET_MATRIX_ROUTE` 0x70 (guide 6.5): old packet `p` (8 bytes, `wValue`
/// ignored) to `wValue = (p[0] << 8) | p[1]`, payload `p[2], p[3], 00, 00,
/// p[4..8]`. Fewer than 8 bytes, or a refused crosspoint, was ignored by 1.1.6.
fn set_matrix_route(env: &Env<'_>, p: &[u8]) -> Result<Plan, PlanError> {
    let (Some([input, output, enabled, phase]), Some(gain)) = (bytes::<4>(p, 0), bytes::<4>(p, 4))
    else {
        return Ok(Plan::local(Outcome::Swallowed { result: None }));
    };
    let data = [&[enabled, phase, 0, 0][..], &gain].concat();
    let req = env.set(
        cmds::ROUTE_MATRIX_ROUTE,
        u16::from_be_bytes([input, output]),
        data,
    )?;
    Ok(Seq::plan(vec![req], Finish::Ack, Refuse::Swallow))
}

/// `GET_MATRIX_ROUTE` 0x71 (guide 6.5): the same `wValue`; the old reply rebuilt
/// as `input, output, r[0], r[1], r[4..8]`.
fn get_matrix_route(env: &Env<'_>, value: u16, len: u16) -> Result<Plan, PlanError> {
    if len == 0 {
        return Err(PlanError::ZeroLengthGet {
            id: cmds::ROUTE_MATRIX_ROUTE,
        });
    }
    let req = env.get_whole(cmds::ROUTE_MATRIX_ROUTE, value, 8)?;
    Ok(Seq::plan(
        vec![req],
        Finish::Matrix { value, len },
        Refuse::Stall,
    ))
}

/// `CLEAR_CLIPS` 0x83 (guide 6.5): `MEAS.CLIP_FLAGS` (4 bytes, reading does not
/// clear), then the zero-length `MEAS.CLIP_CLEAR`; answers the flags read.
fn clear_clips(env: &Env<'_>, len: u16) -> Result<Plan, PlanError> {
    let reqs = vec![
        env.get_whole(cmds::MEAS_CLIP_FLAGS, 0, 4)?,
        env.set(cmds::MEAS_CLIP_CLEAR, 0, Vec::new())?,
    ];
    Ok(Seq::plan(reqs, Finish::ClipFlags { len }, Refuse::Stall))
}

/// `PRESET_GET_STARTUP` 0x97 (guide 6.5): `{mode, default slot}` from
/// `STORE.STARTUP`, the last active slot from `STORE.DIRECTORY` byte 4. A read of
/// 2 bytes or fewer needs only the first.
fn preset_get_startup(env: &Env<'_>, len: u16) -> Result<Plan, PlanError> {
    if len == 0 {
        return Err(PlanError::ZeroLengthGet {
            id: cmds::STORE_STARTUP,
        });
    }
    let mut reqs = vec![env.get_whole(cmds::STORE_STARTUP, 0, 2)?];
    if len > 2 {
        reqs.push(env.get_whole(cmds::STORE_DIRECTORY, 0, 7)?);
    }
    Ok(Seq::plan(reqs, Finish::Startup { len }, Refuse::Stall))
}

/// `SET_ALL_PARAMS` 0xA1 (guide 6.5, section 9): exactly the row's `len` (6136)
/// bytes go to `STORE.ALL_PARAMS` with `wValue` 0; 1.1.6 ACKed and ignored 64
/// bytes or fewer (swallowed, nothing sent) and STALLed every other length at
/// SETUP (a local STALL). A refusal of the write (busy, a header the apply
/// refuses) is raised: a bulk write is never reported as applied when it was not.
fn set_all_params(env: &Env<'_>, row: &Custom, p: &[u8]) -> Result<Plan, PlanError> {
    let exact = row.len.ok_or(PlanError::Table {
        opcode: env.op.code,
    })?;
    if p.len() == usize::from(exact) {
        let req = env.set(cmds::STORE_ALL_PARAMS, 0, p.to_vec())?;
        return Ok(Seq::plan(vec![req], Finish::Ack, Refuse::Stall));
    }
    Ok(Plan::local(if p.len() <= ALL_PARAMS_IGNORED_UP_TO {
        Outcome::Swallowed { result: None }
    } else {
        Outcome::Stall { result: None }
    }))
}

/// `GET_INPUT_RATE` 0xEE (guide 6.5): bytes 0-3 the device rate from
/// `SYS.SYSTEM_STATUS` bytes 4-7, bytes 4-7 `INPUT.I2S_RX_RATE`. A read of 4
/// bytes or fewer needs only the first.
fn get_input_rate(env: &Env<'_>, len: u16) -> Result<Plan, PlanError> {
    if len == 0 {
        return Err(PlanError::ZeroLengthGet {
            id: cmds::SYS_SYSTEM_STATUS,
        });
    }
    let mut reqs = vec![env.get_whole(cmds::SYS_SYSTEM_STATUS, 0, 8)?];
    if len > 4 {
        reqs.push(env.get_whole(cmds::INPUT_I2S_RX_RATE, 0, 4)?);
    }
    Ok(Seq::plan(reqs, Finish::InputRate { len }, Refuse::Stall))
}

// ---------------------------------------------------------------------------
// The reverse view

/// The 1.1.6 request a paged request is the custom translation of, if any (WP
/// PGCUST). Consulted by [`super::reverse::legacy_equivalent`] only when no
/// one-to-one row matches. A candidate is kept only when the planner agrees: a
/// one-request recipe must plan back to exactly this request; a multi-request
/// one must reach this command (with `wValue` 0 and, for a SET, no payload, as
/// its recipe sends it).
pub(super) fn legacy_equivalent(
    id: CmdId,
    dir: PagedDir,
    value: u16,
    payload: &[u8],
) -> Option<LegacyEquivalent> {
    let cand = candidate(id, dir, value, payload)?;
    verify(&cand, id, dir, value, payload).then_some(cand)
}

/// The `GET_STATUS` selector that reads a status struct (its first field; 9 the
/// whole of `MEAS.LEVELS`).
fn status_struct_selector(id: CmdId) -> Option<u16> {
    match id {
        cmds::MEAS_LEVELS => Some(9),
        cmds::SYS_PDM_STATS => Some(3),
        cmds::SYS_SPDIF_TX_STATS => Some(8),
        cmds::SYS_USB_AUDIO_STATUS => Some(10),
        cmds::SYS_SYSTEM_STATUS => Some(13),
        cmds::INPUT_STATUS => Some(23),
        cmds::DIAG_LOOPBACK_STATS => Some(24),
        cmds::SYS_PIPELINE_STATS => Some(26),
        _ => None,
    }
}

fn equivalent(opcode: u8, dir: UsbDir, value: u16, data: Vec<u8>) -> Option<LegacyEquivalent> {
    let op = super::legacy::op(opcode)?;
    Some(LegacyEquivalent {
        opcode,
        dir,
        action: dir == UsbDir::In && op.old_dir == OldDir::WriteAsRead,
        value,
        data,
    })
}

fn candidate(id: CmdId, dir: PagedDir, value: u16, payload: &[u8]) -> Option<LegacyEquivalent> {
    use PagedDir::{Get, Set};
    match (id, dir) {
        (cmds::FILTER_BAND, Get) => {
            let [ch, band] = value.to_be_bytes();
            if band > 0x1F {
                return None;
            }
            let old = u16::from_be_bytes([ch, band.checked_shl(3)?]);
            equivalent(0x43, UsbDir::In, old, Vec::new())
        }
        (cmds::FILTER_BAND, Set) => {
            if payload.len() != 16 {
                return None;
            }
            let [ch, band] = value.to_be_bytes();
            let head = bytes::<2>(payload, 0)?;
            let qp = bytes::<2>(payload, 2)?;
            let floats = bytes::<12>(payload, 4)?;
            let data = [&[ch, band][..], &head, &floats, &qp].concat();
            equivalent(0x42, UsbDir::Out, 0, data)
        }
        (cmds::ROUTE_MATRIX_ROUTE, Get) => equivalent(0x71, UsbDir::In, value, Vec::new()),
        (cmds::ROUTE_MATRIX_ROUTE, Set) => {
            if payload.len() != 8 {
                return None;
            }
            let [input, output] = value.to_be_bytes();
            let [enabled, phase, r0, r1] = bytes::<4>(payload, 0)?;
            let gain = bytes::<4>(payload, 4)?;
            if r0 != 0 || r1 != 0 {
                return None;
            }
            let data = [&[input, output, enabled, phase][..], &gain].concat();
            equivalent(0x70, UsbDir::Out, 0, data)
        }
        (cmds::STORE_ALL_PARAMS, Set) => equivalent(0xA1, UsbDir::Out, 0, payload.to_vec()),
        (cmds::CS_DISPLAY_CAPS | cmds::CS_DISPLAY_CONFIG, Get) => {
            equivalent(0x28, UsbDir::In, 0, Vec::new())
        }
        (cmds::MEAS_CLIP_CLEAR, Set) => equivalent(0x83, UsbDir::In, 0, Vec::new()),
        (cmds::STORE_STARTUP, Get) => equivalent(0x97, UsbDir::In, 0, Vec::new()),
        (cmds::INPUT_I2S_RX_RATE, Get) => equivalent(0xEE, UsbDir::In, 0, Vec::new()),
        (_, Get) => equivalent(0x50, UsbDir::In, status_struct_selector(id)?, Vec::new()),
        _ => None,
    }
}

/// Plan the candidate forward (without device checks) and compare.
fn verify(cand: &LegacyEquivalent, id: CmdId, dir: PagedDir, value: u16, payload: &[u8]) -> bool {
    let body = match cand.dir {
        UsbDir::In => LegacyBody::In { len: 64 },
        UsbDir::Out => LegacyBody::Out(&cand.data),
    };
    let Ok(plan) = plan_in(cand.opcode, cand.value, body, None) else {
        return false;
    };
    match plan.single() {
        Some(req) => {
            req.id == id
                && req.value == value
                && match (&req.body, dir) {
                    (PagedBody::Get { .. }, PagedDir::Get) => true,
                    (PagedBody::Set(d), PagedDir::Set) => d.as_slice() == payload,
                    _ => false,
                }
        }
        None => {
            value == 0
                && (dir == PagedDir::Get || payload.is_empty())
                && reaches(cand.opcode, cand.value, cand.dir).contains(&(id, dir))
        }
    }
}
