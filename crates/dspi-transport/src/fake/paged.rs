//! WP PGFAKE1: the paged front of a [`super::FakeDevice`] of a 1.1.7
//! generation ([`super::FakeGeneration::Stock117`], `Bofr117`): what upstream
//! `release/v1.1.7` 103ba37 answers on USB (`firmware/DSPi/vendor/vc_core.c`,
//! `vc_usb.c`; the migration guide, cited as "guide"), driven by the vendored
//! registry (`dspi_proto::paged::registry`).
//!
//! **The checks, in the firmware's order** (guide 5.1, `vc_check`): the page
//! and command exist on this build (`VC_ERR_UNKNOWN`; page 0 included), the
//! direction (`VC_ERR_DIRECTION`), a SET's length (`VC_ERR_LENGTH`: exact as
//! the registry says; a page reported above its vendored revision accepts a
//! longer SET, PLAN-PG R8b), the address as a whole 16-bit value
//! (`VC_ERR_ADDRESS`, `vc_addr_ok`, for the kinds whose bounds the fake
//! knows). Then, as the handlers do: a NaN or infinite float in a SET is
//! `VC_ERR_VALUE` (guide 5.6; the registry does not type payloads, so the
//! fake's own rule: every 4-byte SET of `ROUTE`, `DYN` and `FX`, the gain of
//! `ROUTE.MATRIX_ROUTE` and the floats of `FILTER.BAND`; the limiter and tube
//! setters clamp an infinity). **Stricter than the firmware:** a GET with
//! `wLength` 0 is refused (`VC_ERR_LENGTH`, counted in
//! [`super::FakeRemote::zero_length_gets`]); on real USB it has no data stage
//! and returns nothing, and our hosts never send one (PLAN-PG section 1.2).
//!
//! **Discovery** is answered natively: `SYS.PROTOCOL_INFO` (guide 4.2),
//! `SYS.PAGE_LIST` (with the generation's page revision), `SYS.PAGE_COMMANDS`
//! and `SYS.COMMAND_INFO` (4.7), `SYS.LAST_RESULT` (5.2: every request is
//! recorded, a GET of it never). Commands with a build condition exist when it
//! holds: `PICO_RP2350` on an RP2350 capture, the `DSPI_*` bench options never
//! (a release build, no `DIAG` page).
//!
//! **Everything else runs on the 1.1.6 model** underneath, through the fake's
//! **own inverse** ([`inverse`]): a paged request becomes the 1.1.6 request
//! with the same effect, found from `legacy_map.json`'s `direct`, `act_*` and
//! `split` rows by code of its own, not `dspi_proto::paged::legacy_equivalent`
//! (so the planner and the fake cannot share a bug; a cross-test holds the two
//! equal). A former write-as-read's status byte becomes an ACK or a refusal
//! with the result code its status family maps it to (guide 5.4); a refusal by
//! the 1.1.6 model is `VC_ERR_ADDRESS` for a GET (guide 5.5) and
//! `VC_ERR_VALUE` for a SET. The custom rows' native handlers are WP
//! PGFAKE2's (`State::pg_custom_get`/`pg_custom_set`).
//!
//! **Not modelled:** a listed command the fake has no handler for (the pin
//! availability queries, and until PGFAKE2 the commands only custom rows
//! reach, e.g. `FILTER.BAND`, `ROUTE.MATRIX_ROUTE`, `MEAS.LEVELS`) is listed
//! as a real build lists it and refused with `VC_ERR_UNSUPPORTED`, counted
//! in [`super::FakeRemote::unmodelled`]. [`super::FakeRemote::hide_paged`]
//! takes a command out of the build. The float layout of `FX.UPMIX_CONFIG`.
//! The busy window and `VC_ERR_BUSY`. Stage-accurate STALLs (a refusal is
//! the call's error, whatever the stage).
//!
//! **The tunnel** (`Bofr117` only; the fork's `extension_channel_spec.md`
//! 1.3.2 and 1.3.4, WP FWPG0): page `0xE0` at revision 1, command `0x10`,
//! row `VC_A_INDEX`, GET length varies, SET 1 to 64 bytes, no flags. Its own
//! checks: the sub-command is listed in `EXT_LIST` (or is `EXT_LIST`, or one
//! of the fake-only codes, [`super::FAKE_ONLY_EXTENSIONS`]), else
//! `VC_ERR_ADDRESS`; a SET of a read-only sub-command is `VC_ERR_DIRECTION`;
//! a SET of the wrong exact length `VC_ERR_LENGTH`; a handler refusal
//! `VC_ERR_VALUE`. Never 0x02 or 0x03. The same handlers serve
//! `REQ_EXTENSION` 0x00 on the legacy stack.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::BTreeMap;

use dspi_proto::ext::PairMap;
use dspi_proto::ext::alink_usb::{EXT_ALINK_USB, EXT_ALINK_USB_DIAG, EXT_ALINK_USB_STATUS};
use dspi_proto::generated::{ext as gext, opcodes as op};
use dspi_proto::paged::legacy::{
    self as lm, Act, Adapter, AddrRule, Encode, Field, LEGACY_OPS, LegacyOp, Status,
};
use dspi_proto::paged::registry::{self, COMMANDS};
use dspi_proto::paged::{
    AddrKind, CmdId, Command, EXT_PAGE, EXT_TUNNEL, Flags, GetLen, LastResult, PROTOCOL_MAGIC,
    PROTOCOL_MAJOR, PageCommands, PagedDir, UsbDir, cmds,
};

use super::{FAKE_ONLY_EXTENSIONS, FakeDevice, FakeGeneration, FakeRemote, SLOTS, State};
use crate::{Result, TransportError};

/// `SYS.PROTOCOL_INFO` bytes 8-9: the largest non-bulk SET payload (guide 4.2).
const MAX_SET: u16 = 64;
/// `SYS.PROTOCOL_INFO` bytes 10-11: the largest non-bulk GET reply (guide 4.2).
const MAX_GET: u16 = 256;
/// Page `0xE0`'s revision (`extension_channel_spec.md` 1.3.2).
const EXT_PAGE_REVISION: u8 = 1;
/// `SYS.LAST_RESULT`'s length (guide 5.2).
const LAST_RESULT_LEN: usize = 8;

/// A paged answer, or the result code (`VC_ERR_*`) it was refused with.
type Vc<T> = std::result::Result<T, u8>;

/// The paged front's state, part of the fake's shared state.
#[derive(Debug, Default)]
pub(super) struct PagedState {
    /// Violations of PLAN-PG section 1.1 ([`FakeRemote::violations`]).
    pub(super) violations: u32,
    pub(super) last_violation: Option<String>,
    /// The `SYS.LAST_RESULT` record: id, direction, result, `wValue`.
    last: Option<(CmdId, bool, u8, u16)>,
    /// The probe was answered as paged: from now on page 0 is a violation on
    /// `Bofr117` (generation.rs).
    probed: bool,
    zero_length_gets: u32,
    unmodelled: u32,
    /// Commands taken out of the build ([`FakeRemote::hide_paged`]).
    hidden: Vec<CmdId>,
}

/// What the dispatcher knows about one command (a registry row, or the tunnel's).
#[derive(Debug, Clone, Copy)]
struct Row {
    id: CmdId,
    get: bool,
    set: bool,
    addr: AddrKind,
    get_len: GetLen,
    set_min: u8,
    set_max: u8,
    flags: Flags,
    command: Option<&'static Command>,
}

impl Row {
    fn of(c: &'static Command) -> Self {
        Self {
            id: c.id,
            get: c.get,
            set: c.set,
            addr: c.addr,
            get_len: c.get_len,
            set_min: c.set_min,
            set_max: c.set_max,
            flags: c.flags,
            command: Some(c),
        }
    }

    /// The tunnel's row (`extension_channel_spec.md` 1.3.2).
    const TUNNEL: Self = Self {
        id: EXT_TUNNEL,
        get: true,
        set: true,
        addr: AddrKind::Index,
        get_len: GetLen::Var,
        set_min: 1,
        // MAX_SET is 64: it fits.
        set_max: MAX_SET as u8,
        flags: Flags(0),
        command: None,
    };

    /// `SYS.COMMAND_INFO`'s 8 bytes (guide 4.7).
    fn info(&self) -> [u8; 8] {
        let [f0, f1] = self.flags.0.to_le_bytes();
        let [g0, g1] = match self.get_len {
            GetLen::None => 0u16,
            GetLen::Fixed(n) => n,
            GetLen::Var => 0xFFFF,
        }
        .to_le_bytes();
        let dirs = u8::from(self.get) | (u8::from(self.set) << 1);
        [
            f0,
            f1,
            g0,
            g1,
            self.set_min,
            self.set_max,
            self.addr.code(),
            dirs,
        ]
    }
}

// ----------------------------------------------------------- the entries

impl FakeDevice {
    /// The page rule of a page-0 request (`control_in`, `control_out`): `Ok`
    /// lets the 1.1.6 model answer it, as every 1.1.6 generation and
    /// `Bofr117`'s legacy stack do. On `Stock117` it is a violation and
    /// refused (`VC_ERR_UNKNOWN`, recorded in `SYS.LAST_RESULT`); on
    /// `Bofr117` after a paged probe answer, a violation.
    pub(super) fn pg_page0(&self, opcode: u8, value: u16, is_in: bool) -> Result<()> {
        self.shared.lock().pg_page0(opcode, value, is_in)
    }

    /// A paged GET on a paged generation; `None` on a 1.1.6 generation (the
    /// PGWIRE page rule decides).
    pub(super) fn pg_in(&self, id: CmdId, value: u16, max_len: u16) -> Option<Result<Vec<u8>>> {
        if !self.shared.lock().is_paged() {
            return None;
        }
        Some(self.run(|s| {
            s.paged_requests = s.paged_requests.saturating_add(1);
            s.pg_get(id, value, usize::from(max_len))
        }))
    }

    /// A paged SET on a paged generation; `None` on a 1.1.6 generation.
    pub(super) fn pg_out(&self, id: CmdId, value: u16, data: &[u8]) -> Option<Result<()>> {
        if !self.shared.lock().is_paged() {
            return None;
        }
        Some(self.run(|s| {
            s.paged_requests = s.paged_requests.saturating_add(1);
            s.pg_set(id, value, data)
        }))
    }
}

impl FakeRemote {
    /// GETs with `wLength` 0 a paged fake refused (module docs).
    pub fn zero_length_gets(&self) -> u32 {
        self.shared.lock().pg.zero_length_gets
    }

    /// Requests for a listed command the paged fake has no handler for.
    pub fn unmodelled(&self) -> u32 {
        self.shared.lock().pg.unmodelled
    }

    /// The `SYS.LAST_RESULT` record as the device would answer it, `None`
    /// before the first request.
    pub fn last_result(&self) -> Option<LastResult> {
        let s = self.shared.lock();
        s.pg.last
            .and_then(|_| LastResult::decode(&s.last_result_bytes()))
    }

    /// Take `id` out of the paged build: unlisted in `SYS.PAGE_COMMANDS`,
    /// `VC_ERR_UNKNOWN` on use, as a command a build condition removes.
    pub fn hide_paged(&self, id: CmdId) {
        self.shared.lock().pg.hidden.push(id);
    }
}

// ----------------------------------------------------------- the dispatcher

impl State {
    fn is_paged(&self) -> bool {
        self.generation().is_some_and(FakeGeneration::is_paged)
    }

    pub(super) fn pg_page0(&mut self, opcode: u8, value: u16, is_in: bool) -> Result<()> {
        let dir = if is_in { "GET" } else { "SET" };
        match self.generation() {
            Some(FakeGeneration::Stock117 { .. }) => {
                self.violation(format!(
                    "a page-0 request 0x{opcode:02X} ({dir}) reached stock 1.1.7 firmware, \
                     which refuses all of page 0: a paged device gets only paged requests \
                     (PLAN-PG section 1.1)"
                ));
                self.record(CmdId::new(0, opcode), is_in, value, lm::VC_ERR_UNKNOWN);
                Err(TransportError::Stalled { opcode })
            }
            Some(FakeGeneration::Bofr117) if self.pg.probed => {
                self.violation(format!(
                    "a page-0 request 0x{opcode:02X} ({dir}) reached the dual-stack port after \
                     it answered the probe as paged: this client speaks only paged to it \
                     (PLAN-PG section 1.1, OD-PGd)"
                ));
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn pg_get(&mut self, id: CmdId, value: u16, max_len: usize) -> Result<Vec<u8>> {
        if id.is_legacy() {
            // A page-0 id through the paged call is a page-0 request.
            self.pg_page0(id.cmd(), value, true)?;
            let mut r = self.get(id.cmd(), value, max_len)?;
            r.truncate(max_len);
            return Ok(r);
        }
        let r = self.pg_get_checked(id, value, max_len);
        if id != cmds::SYS_LAST_RESULT {
            self.record(
                id,
                true,
                value,
                r.as_ref().err().copied().unwrap_or(lm::VC_OK),
            );
        }
        if id == cmds::SYS_PROTOCOL_INFO && r.is_ok() {
            self.pg.probed = true;
        }
        r.map_err(|_| TransportError::Stalled { opcode: id.cmd() })
    }

    fn pg_set(&mut self, id: CmdId, value: u16, data: &[u8]) -> Result<()> {
        if id.is_legacy() {
            self.pg_page0(id.cmd(), value, false)?;
            return self.set(id.cmd(), value, data);
        }
        let r = self.pg_set_checked(id, value, data);
        self.record(id, false, value, r.err().unwrap_or(lm::VC_OK));
        r.map_err(|_| TransportError::Stalled { opcode: id.cmd() })
    }

    fn record(&mut self, id: CmdId, is_get: bool, value: u16, result: u8) {
        self.pg.last = Some((id, is_get, result, value));
    }

    fn last_result_bytes(&self) -> [u8; LAST_RESULT_LEN] {
        let Some((id, is_get, result, value)) = self.pg.last else {
            return [0; LAST_RESULT_LEN];
        };
        let [i0, i1] = id.0.to_le_bytes();
        let [v0, v1] = value.to_le_bytes();
        [i0, i1, u8::from(is_get), result, v0, v1, 0, 0]
    }

    // -------------------------------------------------- what exists

    /// Whether this build has `c` (guide 4.8): a release build for the
    /// capture's platform.
    fn built(&self, c: &Command) -> bool {
        match c.build {
            None => true,
            Some("PICO_RP2350") => self.platform.first() == Some(&1),
            Some(_) => false,
        }
    }

    /// The row of `id` when this build has it.
    fn pg_row(&self, id: CmdId) -> Option<Row> {
        if id.is_legacy() || self.pg.hidden.contains(&id) {
            return None;
        }
        if id == EXT_TUNNEL {
            return (self.generation() == Some(FakeGeneration::Bofr117)).then_some(Row::TUNNEL);
        }
        registry::command(id).filter(|c| self.built(c)).map(Row::of)
    }

    /// The implemented pages, ascending, with their reported revisions.
    fn pg_pages(&self) -> BTreeMap<u8, u8> {
        let mut pages = BTreeMap::new();
        for c in COMMANDS.iter() {
            if self.pg_row(c.id).is_some() {
                pages.insert(c.id.page(), self.pg_revision(c.id.page()));
            }
        }
        if self.pg_row(EXT_TUNNEL).is_some() {
            pages.insert(EXT_PAGE, EXT_PAGE_REVISION);
        }
        pages
    }

    /// The revision `page` reports.
    fn pg_revision(&self, page: u8) -> u8 {
        if page == EXT_PAGE {
            return EXT_PAGE_REVISION;
        }
        let vendored = registry::page(page).map_or(0, |p| p.revision);
        match self.generation() {
            Some(FakeGeneration::Stock117 { revision }) => vendored.max(revision),
            _ => vendored,
        }
    }

    fn pg_bitmap(&self, page: u8) -> PageCommands {
        let mut m = PageCommands::EMPTY;
        for cmd in 0..=u8::MAX {
            if let Some(row) = self.pg_row(CmdId::new(page, cmd)) {
                if row.get {
                    m.insert(cmd, PagedDir::Get);
                }
                if row.set {
                    m.insert(cmd, PagedDir::Set);
                }
            }
        }
        m
    }

    /// `vc_addr_ok` (vc_core.c) for the kinds whose bounds the fake knows;
    /// the others pass to the handler.
    fn addr_ok(&self, kind: AddrKind, a: u16, is_get: bool) -> bool {
        let [lo, hi] = a.to_le_bytes();
        let chans = u16::from(self.num_channels());
        let ins = u16::from(self.num_inputs());
        let outs = u16::from(self.num_outputs());
        match kind {
            AddrKind::None => a == 0,
            AddrKind::Channel => a < chans,
            AddrKind::Input => a < ins,
            AddrKind::Output => a < outs,
            AddrKind::OutputAll => a < outs || (!is_get && a == 0xFF),
            // NUM_SPDIF_INSTANCES: the stereo outputs, at most 4.
            AddrKind::Slot => a < (outs / 2).min(4),
            AddrKind::Preset => usize::from(a) < SLOTS,
            AddrKind::ChBand => u16::from(hi) < chans,
            AddrKind::InOut => u16::from(hi) < ins && u16::from(lo) < outs,
            AddrKind::Role => a <= 1,
            AddrKind::Page => a <= 0xFF,
            _ => true,
        }
    }

    // -------------------------------------------------- GET

    fn pg_get_checked(&mut self, id: CmdId, value: u16, max_len: usize) -> Vc<Vec<u8>> {
        let row = self.pg_row(id).ok_or(lm::VC_ERR_UNKNOWN)?;
        if !row.get {
            return Err(lm::VC_ERR_DIRECTION);
        }
        if !self.addr_ok(row.addr, value, true) {
            return Err(lm::VC_ERR_ADDRESS);
        }
        if max_len == 0 {
            self.pg.zero_length_gets = self.pg.zero_length_gets.saturating_add(1);
            return Err(lm::VC_ERR_LENGTH);
        }
        let mut reply = self.pg_answer_get(&row, value, max_len)?;
        if let GetLen::Fixed(n) = row.get_len {
            reply.resize(usize::from(n), 0);
        }
        reply.truncate(max_len);
        Ok(reply)
    }

    fn pg_answer_get(&mut self, row: &Row, value: u16, max_len: usize) -> Vc<Vec<u8>> {
        let id = row.id;
        if id == cmds::SYS_PROTOCOL_INFO {
            return Ok(self.protocol_info());
        }
        if id == cmds::SYS_PAGE_LIST {
            return Ok(self
                .pg_pages()
                .into_iter()
                .flat_map(|(p, r)| [p, r])
                .collect());
        }
        if id == cmds::SYS_PAGE_COMMANDS {
            // Page 0 and an absent page are VC_ERR_ADDRESS (guide 4.7).
            let page = u8::try_from(value).map_err(|_| lm::VC_ERR_ADDRESS)?;
            if !self.pg_pages().contains_key(&page) {
                return Err(lm::VC_ERR_ADDRESS);
            }
            return Ok(self.pg_bitmap(page).encode().to_vec());
        }
        if id == cmds::SYS_COMMAND_INFO {
            let r = self.pg_row(CmdId(value)).ok_or(lm::VC_ERR_ADDRESS)?;
            return Ok(r.info().to_vec());
        }
        if id == cmds::SYS_LAST_RESULT {
            return Ok(self.last_result_bytes().to_vec());
        }
        if id == EXT_TUNNEL {
            return self.tunnel_get(value, max_len);
        }
        if let Some(r) = self.pg_custom_get(id, value, max_len) {
            return r;
        }
        match inverse(id, PagedDir::Get, value, &[]) {
            Some(inv) => self
                .get(inv.opcode, inv.value, max_len)
                .map_err(|_| lm::VC_ERR_ADDRESS),
            None => Err(self.unmodelled()),
        }
    }

    /// `SYS.PROTOCOL_INFO` (guide 4.2).
    fn protocol_info(&self) -> Vec<u8> {
        let pages = u8::try_from(self.pg_pages().len()).unwrap_or(u8::MAX);
        let mut b = PROTOCOL_MAGIC.to_vec();
        b.extend([PROTOCOL_MAJOR, registry::VENDORED_MINOR, pages, 0]);
        b.extend(MAX_SET.to_le_bytes());
        b.extend(MAX_GET.to_le_bytes());
        b.extend([0; 4]);
        b
    }

    fn unmodelled(&mut self) -> u8 {
        self.pg.unmodelled = self.pg.unmodelled.saturating_add(1);
        lm::VC_ERR_UNSUPPORTED
    }

    // -------------------------------------------------- SET

    fn pg_set_checked(&mut self, id: CmdId, value: u16, data: &[u8]) -> Vc<()> {
        let row = self.pg_row(id).ok_or(lm::VC_ERR_UNKNOWN)?;
        if !row.set {
            return Err(lm::VC_ERR_DIRECTION);
        }
        let len = data.len();
        let data = if row.flags.has(Flags::BULK) {
            if len == 0 || len > dspi_proto::generated::BULK_SIZE {
                return Err(lm::VC_ERR_LENGTH);
            }
            data
        } else {
            let vendored = registry::page(id.page()).map_or(0, |p| p.revision);
            let newer = id.page() != EXT_PAGE && self.pg_revision(id.page()) > vendored;
            let max = if newer {
                usize::from(MAX_SET)
            } else {
                usize::from(row.set_max)
            };
            if len < usize::from(row.set_min) || len > max {
                return Err(lm::VC_ERR_LENGTH);
            }
            // A later revision's extra bytes are at the end: this model reads
            // the vendored ones.
            data.get(..usize::from(row.set_max)).unwrap_or(data)
        };
        if !self.addr_ok(row.addr, value, false) {
            return Err(lm::VC_ERR_ADDRESS);
        }
        if let Some(c) = row.command
            && !floats_ok(c, data)
        {
            return Err(lm::VC_ERR_VALUE);
        }
        if id == EXT_TUNNEL {
            return self.tunnel_set(value, data);
        }
        if let Some(r) = self.pg_custom_set(id, value, data) {
            return r;
        }
        match inverse(id, PagedDir::Set, value, data) {
            Some(inv) => self.run_inverse_set(&inv),
            None => Err(self.unmodelled()),
        }
    }

    /// Run a SET's 1.1.6 equivalent on the model.
    fn run_inverse_set(&mut self, inv: &Inverse) -> Vc<()> {
        if inv.action {
            // A former write-as-read: an IN request answering a status byte.
            let status = inv.status.ok_or(lm::VC_ERR_VALUE)?;
            let reply = self
                .get(inv.opcode, inv.value, 1)
                .map_err(|_| lm::VC_ERR_VALUE)?;
            return match reply.first() {
                Some(b) if *b == status.success => Ok(()),
                Some(b) => Err(result_for(&status, *b)),
                None => Err(lm::VC_ERR_VALUE),
            };
        }
        // 1.1.7 refuses what 1.1.6 swallowed: run the model strictly.
        let lax = std::mem::replace(&mut self.options.ack_unknown_sets, false);
        let r = self.set(inv.opcode, inv.value, &inv.data);
        self.options.ack_unknown_sets = lax;
        r.map_err(|_| lm::VC_ERR_VALUE)
    }

    // -------------------------------------------------- the tunnel

    fn tunnel_ext_ok(&mut self, ext: u16) -> Vc<()> {
        if ext == gext::EXT_LIST || FAKE_ONLY_EXTENSIONS.contains(&ext) {
            return Ok(());
        }
        let list = self
            .get(op::REQ_EXTENSION, gext::EXT_LIST, 64)
            .unwrap_or_default();
        if list
            .as_chunks::<2>()
            .0
            .iter()
            .any(|b| u16::from_le_bytes(*b) == ext)
        {
            Ok(())
        } else {
            Err(lm::VC_ERR_ADDRESS)
        }
    }

    fn tunnel_get(&mut self, ext: u16, max_len: usize) -> Vc<Vec<u8>> {
        self.tunnel_ext_ok(ext)?;
        self.get(op::REQ_EXTENSION, ext, max_len)
            .map_err(|_| lm::VC_ERR_VALUE)
    }

    fn tunnel_set(&mut self, ext: u16, data: &[u8]) -> Vc<()> {
        self.tunnel_ext_ok(ext)?;
        if TUNNEL_READ_ONLY.contains(&ext) {
            return Err(lm::VC_ERR_DIRECTION);
        }
        if let Some((_, n)) = TUNNEL_SET_LEN.iter().find(|(e, _)| *e == ext)
            && data.len() != *n
        {
            return Err(lm::VC_ERR_LENGTH);
        }
        // -- WP ATFAKE -- a time-base handler names its own code (STATE,
        // DIRECTION, ...; extension_channel_spec.md 2.8.7.1, 2.10, 2.11)
        let _ = self.tb_take_refusal();
        self.set(op::REQ_EXTENSION, ext, data)
            .map_err(|_| self.tb_take_refusal().unwrap_or(lm::VC_ERR_VALUE))
    }

    // -------------------------------------------------- WP PGFAKE2's hook

    // -- WP PGFAKE2 -- the bodies call `paged_custom` (the `mod` lines are at
    // the end of this file).

    /// A custom row's native GET handler (WP PGFAKE2), `Some` when one answers `id`.
    fn pg_custom_get(&mut self, id: CmdId, value: u16, max_len: usize) -> Option<Vc<Vec<u8>>> {
        self.custom_paged_get(id, value, max_len)
    }

    /// A custom row's native SET handler (WP PGFAKE2), `Some` when one answers `id`.
    fn pg_custom_set(&mut self, id: CmdId, value: u16, data: &[u8]) -> Option<Vc<()>> {
        self.custom_paged_set(id, value, data)
    }
}

/// Sub-commands with no OUT form (`extension_channel_spec.md` 1.3.4).
const TUNNEL_READ_ONLY: [u16; 5] = [
    gext::EXT_LIST,
    gext::EXT_NOTIFY_FEATURES,
    gext::EXT_RESET_INFO,
    dspi_proto::bulk_gain::EXT_FIRMWARE_FIXES,
    EXT_ALINK_USB_STATUS,
];

/// Exact SET lengths the tunnel checks itself (`extension_channel_spec.md`
/// section 2); the other sub-commands' handlers judge their payloads.
const TUNNEL_SET_LEN: [(u16, usize); 5] = [
    (gext::EXT_CROSSFEED_PAIR_MAP, PairMap::SIZE),
    (gext::EXT_CORE_VREG, 2),
    (gext::EXT_IDENTIFY, dspi_proto::ext::IdentifyRequest::SIZE),
    (EXT_ALINK_USB, 8),
    (EXT_ALINK_USB_DIAG, 1),
];

/// The result code of a former write-as-read's old status byte `b` (guide
/// 5.4): the lowest code its family maps to `b`.
fn result_for(status: &Status, b: u8) -> u8 {
    status
        .from_vc
        .iter()
        .find(|(_, old)| *old == b)
        .map_or(lm::VC_ERR_VALUE, |(code, _)| *code)
}

/// Byte offsets of the floats in `c`'s SET payload (guide 5.6; module docs).
fn float_offsets(c: &Command) -> &'static [usize] {
    if c.id == cmds::ROUTE_MATRIX_ROUTE {
        &[4]
    } else if c.id == cmds::FILTER_BAND {
        &[4, 8, 12]
    } else if matches!(c.page, "ROUTE" | "DYN" | "FX") && c.set_min == 4 && c.set_max == 4 {
        &[0]
    } else {
        &[]
    }
}

/// Whether `data`'s floats are acceptable to `c`'s handler: never NaN;
/// infinite only for the setters that clamp it (guide 5.6).
fn floats_ok(c: &Command, data: &[u8]) -> bool {
    let clamps_infinity = (c.page == "DYN" && c.name.starts_with("LIMITER_"))
        || (c.page == "FX" && c.name.starts_with("TUBE_"));
    float_offsets(c).iter().all(|o| {
        let Some(b) = o
            .checked_add(4)
            .and_then(|end| data.get(*o..end))
            .and_then(|b| <[u8; 4]>::try_from(b).ok())
        else {
            return true;
        };
        let f = f32::from_le_bytes(b);
        !f.is_nan() && (f.is_finite() || clamps_infinity)
    })
}

// ----------------------------------------------------------- the inverse

/// The 1.1.6 request the fake runs for a paged one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Inverse {
    pub(super) opcode: u8,
    pub(super) dir: UsbDir,
    /// A former write-as-read (an IN request that changes state).
    pub(super) action: bool,
    pub(super) value: u16,
    /// The OUT payload; empty for an IN request.
    pub(super) data: Vec<u8>,
    /// A write-as-read's status family.
    status: Option<Status>,
}

/// The 1.1.6 request with the same effect as the paged request `(id, dir,
/// value, payload)`, from the `direct`, `act_*` and `split` rows; `None` when
/// none has it (the custom rows, the commands 1.1.7 added). The fake's own
/// code, independent of `dspi_proto::paged::legacy_equivalent`: each row is
/// solved for the old `wValue`, then run forward by [`forward`] and kept only
/// when it gives the request back exactly. Ties: rows not marked retired
/// first, then by opcode, the OUT form before the IN form, cases in order.
pub(super) fn inverse(id: CmdId, dir: PagedDir, value: u16, payload: &[u8]) -> Option<Inverse> {
    if id.is_legacy() || id == EXT_TUNNEL {
        return None;
    }
    let kind = registry::command(id)?.addr;
    let payload = match dir {
        PagedDir::Get => &[][..],
        PagedDir::Set => payload,
    };
    let want = (id, dir, value, payload.to_vec());
    let mut retired = None;
    for op in LEGACY_OPS.iter() {
        for (usb, side) in [(UsbDir::Out, op.out), (UsbDir::In, op.r#in)] {
            let Some(a) = side else { continue };
            for (leaf, key) in flatten(a) {
                let Some(inv) = solve(op, usb, leaf, key, id, dir, kind, value, payload) else {
                    continue;
                };
                if forward(op, usb, inv.value, &inv.data).as_ref() != Some(&want) {
                    continue;
                }
                if matches!(leaf, Adapter::Direct(d) if d.retired) {
                    retired.get_or_insert(inv);
                } else {
                    return Some(inv);
                }
            }
        }
    }
    retired
}

/// A selector and the case key that leads to a leaf (`None`: the default).
type Key = Option<(Field, Option<u16>)>;

/// The adapters a side can end in, one split level deep, with their keys.
fn flatten(a: &'static Adapter) -> Vec<(&'static Adapter, Key)> {
    match a {
        Adapter::Split(s) => s
            .cases
            .iter()
            .map(|(k, c)| (*c, Some((s.selector, Some(*k)))))
            .chain(s.default.map(|d| (d, Some((s.selector, None)))))
            .collect(),
        other => vec![(other, None)],
    }
}

/// `(old >> shift) & mask`.
fn bits(f: Field, old: u16) -> u16 {
    old.checked_shr(u32::from(f.shift)).unwrap_or(0) & f.mask
}

/// `x` written into `old` at `f`, undoing `add` when `with_add`; `None` when it
/// does not fit.
fn put(old: u16, f: Field, x: u16, with_add: bool) -> Option<u16> {
    let raw = if with_add {
        i32::from(x).checked_sub(f.add)?
    } else {
        i32::from(x)
    };
    let raw = u16::try_from(raw).ok()?;
    if raw & !f.mask != 0 {
        return None;
    }
    let placed = u16::try_from(u32::from(raw).checked_shl(u32::from(f.shift))?).ok()?;
    Some(old | placed)
}

/// The old `wValue` bits that give paged address `value` under `rule`.
fn addr_bits(rule: AddrRule, kind: AddrKind, value: u16, old: u16) -> Option<u16> {
    if kind == AddrKind::None {
        // Such a target is sent wValue 0, whatever the old one was.
        return (value == 0).then_some(old);
    }
    match rule {
        AddrRule::Old => Some(old | value),
        AddrRule::Const(k) => (k == value).then_some(old),
        AddrRule::Field(f) => put(old, f, value, true),
    }
}

#[allow(clippy::too_many_arguments)]
fn solve(
    op: &'static LegacyOp,
    usb: UsbDir,
    leaf: &'static Adapter,
    key: Key,
    id: CmdId,
    dir: PagedDir,
    kind: AddrKind,
    value: u16,
    payload: &[u8],
) -> Option<Inverse> {
    let mut old = 0u16;
    if let Some((sel, Some(k))) = key {
        old = put(old, sel, k, false)?;
    }
    let (action, status) = match leaf {
        Adapter::Direct(d) => {
            let same = matches!(
                (usb, dir),
                (UsbDir::In, PagedDir::Get) | (UsbDir::Out, PagedDir::Set)
            );
            if d.target != id || !same {
                return None;
            }
            old = addr_bits(d.addr, kind, value, old)?;
            (false, None)
        }
        Adapter::ActValue(x) | Adapter::ActAddr(x) | Adapter::ActPacked(x) => {
            if x.target != id || dir != PagedDir::Set || usb != UsbDir::In {
                return None;
            }
            old = addr_bits(x.addr, kind, value, old)?;
            old = value_bits(x, payload, old)?;
            (true, Some(x.status))
        }
        Adapter::Split(_) | Adapter::Custom(_) => return None,
    };
    Some(Inverse {
        opcode: op.code,
        dir: usb,
        action,
        value: old,
        data: match usb {
            UsbDir::Out => payload.to_vec(),
            UsbDir::In => Vec::new(),
        },
        status,
    })
}

/// The old `wValue` bits a former write-as-read's payload byte came from.
fn value_bits(x: &Act, payload: &[u8], old: u16) -> Option<u16> {
    match (x.value, payload) {
        (None, []) => Some(old),
        (Some((_, Encode::Bool)), [b]) if *b > 1 => None,
        // The byte is the field without `add` (schema `act_value`).
        (Some((f, _)), [b]) => put(old, f, u16::from(*b), false),
        _ => None,
    }
}

/// The fake's own forward reading of a row (the `_schema` of
/// `legacy_map.json`): the paged request a 1.1.6 request is, for the rows
/// that are one request. Used to check [`solve`]'s candidates.
fn forward(
    op: &'static LegacyOp,
    usb: UsbDir,
    old: u16,
    data: &[u8],
) -> Option<(CmdId, PagedDir, u16, Vec<u8>)> {
    let mut a = match usb {
        UsbDir::Out => op.out,
        UsbDir::In => op.r#in,
    }?;
    if let Adapter::Split(s) = a {
        let sel = bits(s.selector, old);
        a = s
            .cases
            .iter()
            .find(|(k, _)| *k == sel)
            .map(|(_, c)| *c)
            .or(s.default)?;
    }
    let address = |rule: AddrRule, target: CmdId| -> Option<u16> {
        if registry::command(target)?.addr == AddrKind::None {
            return Some(0);
        }
        match rule {
            AddrRule::Old => Some(old),
            AddrRule::Const(k) => Some(k),
            AddrRule::Field(f) => u16::try_from(i32::from(bits(f, old)).checked_add(f.add)?).ok(),
        }
    };
    match a {
        Adapter::Direct(d) => {
            let v = address(d.addr, d.target)?;
            Some(match usb {
                UsbDir::In => (d.target, PagedDir::Get, v, Vec::new()),
                UsbDir::Out => {
                    let cut = d
                        .trunc
                        .map_or(data.len(), |t| data.len().min(usize::from(t)));
                    (d.target, PagedDir::Set, v, data.get(..cut)?.to_vec())
                }
            })
        }
        Adapter::ActValue(x) | Adapter::ActAddr(x) | Adapter::ActPacked(x) => {
            if usb != UsbDir::In {
                return None;
            }
            let v = address(x.addr, x.target)?;
            let payload = match x.value {
                None => Vec::new(),
                Some((f, enc)) => {
                    let raw = bits(f, old);
                    vec![match enc {
                        Encode::U8 => raw.to_le_bytes().first().copied().unwrap_or(0),
                        Encode::U8Sat => u8::try_from(raw).unwrap_or(u8::MAX),
                        Encode::Bool => u8::from(raw != 0),
                    }]
                }
            };
            Some((x.target, PagedDir::Set, v, payload))
        }
        Adapter::Split(_) | Adapter::Custom(_) => None,
    }
}

#[cfg(test)]
#[path = "paged/tests_wp_pgfake1.rs"]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests_wp_pgfake1;

// -- WP PGFAKE2 -- the custom rows' paged commands; the Zero and the sender.
// The files are `fake/{paged_custom,boards}.rs` (PLAN-PG, PGFAKE2's entry).
#[path = "boards.rs"]
pub(super) mod boards;
#[path = "paged_custom.rs"]
mod paged_custom;
