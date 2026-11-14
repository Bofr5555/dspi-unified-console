//! The settings outside the bulk image that a guarded apply's hub revert puts back
//! (WP P1CFIX, F6; review p1c). P1HA's guard captures the device's image (`0xA0`) and
//! writes it back (`0xA1`); a client that reverts by itself also puts back what the
//! image does not carry (P1SN's snapshot recall). So that a client that vanishes is
//! reverted as fully, `apply.begin` also captures, and the revert writes back:
//!
//! - the **device name** and the **input source names** (`REQ_GET/SET_DEVICE_NAME`,
//!   `REQ_GET/SET_INPUT_SOURCE_NAME`, 32-byte NUL-padded fields);
//! - the **output latency** (`REQ_OUTPUT_LATENCY`, the requested option id);
//! - the **control-surface configuration**: groups, macros, IR commands, bindings
//!   (twice, so one that frees a GPIO another wants goes first), binding names, the
//!   display configuration and pages, in the order P1SN's recall writes them, through
//!   `dspi_session::surfaces` (the same records and the same deferred-apply wait).
//!
//! Each part is read back at the revert and written only where it differs, before
//! the image (the image last, as P1SN's recall does). A part the device does not
//! have (an older firmware stalls the read) is not captured and not written. One
//! write each, never repeated: a refusal is noted in the outcome, and a lost device
//! stops the rest. Not covered: an output configuration kept apart from presets
//! (pins and limiters, which the firmware takes from no bulk write); the outcome
//! says what was captured, and the client's status says what was not.
//!
//! The capture runs as the session that begins the guard (in its order, through its
//! lock); the revert as the hub ([`crate::session::INTERNAL_SESSION_ID`]) inside the
//! router's revert window, so no lock refuses it.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use dspi_link::Dir;
use dspi_link::frames::CmdRequest;
use dspi_proto::VENDOR_INTERFACE;
use dspi_proto::generated::opcodes as op;
use dspi_session::surfaces::{
    CsBinding, CsDisplayCfg, CsDisplayPage, CsGroup, CsMacro, CsMacroHeaderWire, CsStatusPacket,
    IrCommand, Limits, SurfaceError, Surfaces, status as cs_status,
};
use dspi_transport::link::map_status;
use dspi_transport::{DeviceDescriptor, Result as TResult, Transport, TransportError};

use crate::device::DeviceShared;
use crate::session::{Completion, SessionCtx};

/// A name field's length on the wire.
const NAME_LEN: usize = 32;
/// The last input source a name can be read for (audio_input.h: `INPUT_SOURCE_MAX`).
const MAX_SOURCE: u8 = 6;

/// The part names in the outcome (`apply.ended`'s `captured` and `restored`).
pub const PART_DEVICE_NAME: &str = "device_name";
/// See [`PART_DEVICE_NAME`].
pub const PART_SOURCE_NAMES: &str = "source_names";
/// See [`PART_DEVICE_NAME`].
pub const PART_OUTPUT_LATENCY: &str = "output_latency";
/// See [`PART_DEVICE_NAME`].
pub const PART_CONTROL_SURFACES: &str = "control_surfaces";

/// The device's commands as `ctx`, each waited for at most `wait`: a [`Transport`]
/// over the hub's worker for `dspi_session`'s readers and writers.
pub(crate) struct GuardTransport {
    dev: Arc<DeviceShared>,
    ctx: SessionCtx,
    wait: Duration,
    descriptor: DeviceDescriptor,
}

impl GuardTransport {
    pub(crate) fn new(dev: Arc<DeviceShared>, ctx: SessionCtx, wait: Duration) -> Self {
        let descriptor = DeviceDescriptor {
            serial: dev.serial(),
            bus_id: "hub".into(),
            address: dev.handle,
        };
        Self {
            dev,
            ctx,
            wait,
            descriptor,
        }
    }

    fn command(
        &self,
        dir: Dir,
        opcode: u8,
        value: u16,
        len: u16,
        payload: &[u8],
    ) -> TResult<Vec<u8>> {
        let (tx, rx) = mpsc::channel();
        let req = CmdRequest {
            tag: 0,
            handle: self.dev.handle,
            dir,
            b_request: opcode,
            w_value: value,
            w_index: VENDOR_INTERFACE,
            w_length: len,
            payload: payload.to_vec(),
        };
        let deadline = Instant::now().checked_add(self.wait);
        self.dev.submit(
            self.ctx,
            req,
            Completion::new(0, move |r| {
                let _ = tx.send(r);
            }),
            deadline,
        );
        // A transfer already running at the deadline still ends: wait for it.
        let r = rx
            .recv_timeout(self.wait.saturating_add(Duration::from_secs(5)))
            .map_err(|_| TransportError::TimedOut { opcode, retries: 0 })?;
        match map_status(r.status, opcode) {
            Some(e) => Err(e),
            None => Ok(r.payload),
        }
    }
}

impl Transport for GuardTransport {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> TResult<Vec<u8>> {
        let mut d = self.command(Dir::Get, opcode, value, len, &[])?;
        if d.len() < usize::from(len) {
            return Err(TransportError::ShortRead {
                opcode,
                wanted: usize::from(len),
                got: d.len(),
            });
        }
        d.truncate(usize::from(len));
        Ok(d)
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> TResult<Vec<u8>> {
        let mut d = self.command(Dir::Get, opcode, value, max_len, &[])?;
        d.truncate(usize::from(max_len));
        Ok(d)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> TResult<()> {
        let len = u16::try_from(data.len()).map_err(|_| {
            TransportError::Remote(format!(
                "{} bytes for opcode 0x{opcode:02X} exceed a vendor request",
                data.len()
            ))
        })?;
        self.command(Dir::Set, opcode, value, len, data).map(|_| ())
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    fn max_transfer(&self) -> usize {
        usize::from(u16::MAX)
    }
}

/// The control-surface configuration as captured.
#[derive(Debug, Clone)]
struct CsCapture {
    limits: Limits,
    groups: Option<Vec<CsGroup>>,
    macros: Option<Vec<CsMacro>>,
    ir: Option<Vec<IrCommand>>,
    bindings: Vec<(CsBinding, String)>,
    display: Option<(CsDisplayCfg, Vec<CsDisplayPage>)>,
}

/// What a guard captured besides the image.
#[derive(Debug, Clone, Default)]
pub(crate) struct Extras {
    device_name: Option<Vec<u8>>,
    source_names: Vec<Vec<u8>>,
    latency: Option<u8>,
    surfaces: Option<CsCapture>,
}

/// What a revert wrote back besides the image.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Restored {
    /// The parts written back because they differed ([`PART_DEVICE_NAME`], ...).
    pub parts: Vec<String>,
    /// What could not be written back, and why.
    pub notes: Vec<String>,
}

/// A name field as the SET takes it: up to the first NUL, NUL-padded to 32 bytes.
fn name_field(raw: &[u8]) -> Vec<u8> {
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let mut f: Vec<u8> = raw.iter().take(end.min(NAME_LEN)).copied().collect();
    f.resize(NAME_LEN, 0);
    f
}

const NAME_LEN_U16: u16 = NAME_LEN as u16;

impl Extras {
    /// Read every part the device has. Never fails: a part that cannot be read is
    /// not captured (and so not written back).
    pub(crate) fn capture(t: &mut dyn Transport) -> Self {
        let device_name = t
            .control_in(op::REQ_GET_DEVICE_NAME, 0, NAME_LEN_U16)
            .ok()
            .map(|d| name_field(&d));
        let mut source_names = Vec::new();
        for s in 0..=MAX_SOURCE {
            match t.control_in(op::REQ_GET_INPUT_SOURCE_NAME, u16::from(s), NAME_LEN_U16) {
                Ok(d) => source_names.push(name_field(&d)),
                Err(_) => break,
            }
        }
        let latency = t
            .control_in(op::REQ_OUTPUT_LATENCY, 0, 1)
            .ok()
            .and_then(|d| d.first().copied());
        Self {
            device_name,
            source_names,
            latency,
            surfaces: capture_surfaces(t),
        }
    }

    /// The parts captured, by name.
    pub(crate) fn captured(&self) -> Vec<String> {
        let mut parts = Vec::new();
        if self.device_name.is_some() {
            parts.push(PART_DEVICE_NAME.to_owned());
        }
        if !self.source_names.is_empty() {
            parts.push(PART_SOURCE_NAMES.to_owned());
        }
        if self.latency.is_some() {
            parts.push(PART_OUTPUT_LATENCY.to_owned());
        }
        if self.surfaces.is_some() {
            parts.push(PART_CONTROL_SURFACES.to_owned());
        }
        parts
    }

    /// Write back every captured part that differs from the device now. Stops at a
    /// lost device; any other refusal is noted and the rest goes on.
    pub(crate) fn restore(&self, t: &mut dyn Transport) -> Restored {
        let mut out = Restored::default();
        let r = (|| -> TResult<()> {
            if let Some(want) = &self.device_name {
                let now = t.control_in(op::REQ_GET_DEVICE_NAME, 0, NAME_LEN_U16);
                if now.ok().map(|d| name_field(&d)).as_ref() != Some(want) {
                    write(
                        t,
                        op::REQ_SET_DEVICE_NAME,
                        0,
                        want,
                        PART_DEVICE_NAME,
                        &mut out,
                    )?;
                }
            }
            for (s, want) in (0u16..).zip(&self.source_names) {
                let now = t.control_in(op::REQ_GET_INPUT_SOURCE_NAME, s, NAME_LEN_U16);
                if now.ok().map(|d| name_field(&d)).as_ref() != Some(want) {
                    write(
                        t,
                        op::REQ_SET_INPUT_SOURCE_NAME,
                        s,
                        want,
                        PART_SOURCE_NAMES,
                        &mut out,
                    )?;
                }
            }
            if let Some(want) = self.latency {
                let now = t.control_in(op::REQ_OUTPUT_LATENCY, 0, 1);
                if now.ok().and_then(|d| d.first().copied()) != Some(want) {
                    write(
                        t,
                        op::REQ_OUTPUT_LATENCY,
                        0,
                        &[want],
                        PART_OUTPUT_LATENCY,
                        &mut out,
                    )?;
                }
            }
            if let Some(cs) = &self.surfaces {
                restore_surfaces(t, cs, &mut out)?;
            }
            Ok(())
        })();
        if let Err(e) = r {
            out.notes.push(format!("stopped: {e}"));
        }
        out
    }
}

/// One SET of a part; a lost device ends the restore, any other refusal is noted.
fn write(
    t: &mut dyn Transport,
    opcode: u8,
    value: u16,
    data: &[u8],
    part: &str,
    out: &mut Restored,
) -> TResult<()> {
    match t.control_out(opcode, value, data) {
        Ok(()) => {
            if !out.parts.iter().any(|p| p == part) {
                out.parts.push(part.to_owned());
            }
            Ok(())
        }
        Err(TransportError::Disconnected) => Err(TransportError::Disconnected),
        Err(e) => {
            out.notes.push(format!("{part}: not written ({e})"));
            Ok(())
        }
    }
}

fn capture_surfaces(t: &mut dyn Transport) -> Option<CsCapture> {
    let caps = dspi_session::probe::read_cs_caps(t)?;
    let mut limits = Limits::from(&caps);
    let mut s = Surfaces::new(t, limits);
    // Groups and macros came at caps v9, the display at v10: an older device has
    // none (zero maxima), and a stalled read of one leaves that part out.
    let groups = (caps.max_groups > 0)
        .then(|| s.read_all_groups().ok())
        .flatten();
    let macros = (caps.max_macros > 0)
        .then(|| s.read_all_macros().ok())
        .flatten();
    let ir = s.read_all_ir_commands().ok();
    let bindings = s.read_all_bindings().ok()?;
    let display = (caps.caps_version >= 10)
        .then(|| {
            let cfg = s.read_display_cfg().ok()?.cfg;
            let pages = s.read_all_pages().ok()?;
            Some((cfg, pages))
        })
        .flatten();
    limits = s.limits();
    Some(CsCapture {
        limits,
        groups,
        macros,
        ir,
        bindings,
        display,
    })
}

/// One control-surface write's outcome: a lost device stops the restore; a
/// refusal (on the wire or in the status packet) is noted.
fn cs_written(
    r: Result<CsStatusPacket, SurfaceError>,
    what: String,
    out: &mut Restored,
) -> TResult<()> {
    match r {
        Ok(st) if st.last_status == cs_status::SUCCESS => {
            if !out.parts.iter().any(|p| p == PART_CONTROL_SURFACES) {
                out.parts.push(PART_CONTROL_SURFACES.to_owned());
            }
            Ok(())
        }
        Ok(st) => {
            out.notes.push(format!(
                "control_surfaces: {what} refused ({})",
                dspi_session::surfaces::explain_status(st.last_status)
            ));
            Ok(())
        }
        Err(SurfaceError::Transport(TransportError::Disconnected)) => {
            Err(TransportError::Disconnected)
        }
        Err(e) => {
            out.notes
                .push(format!("control_surfaces: {what} not written ({e})"));
            Ok(())
        }
    }
}

fn restore_surfaces(t: &mut dyn Transport, c: &CsCapture, out: &mut Restored) -> TResult<()> {
    let mut s = Surfaces::new(t, c.limits);
    // Groups and macros before the bindings that may name them (P1SN's order).
    for (i, g) in (0u8..).zip(c.groups.iter().flatten()) {
        if s.read_group(i).ok().as_ref() != Some(g) {
            cs_written(
                s.write_group(i, g),
                format!("group {}", u16::from(i) + 1),
                out,
            )?;
        }
    }
    for (i, m) in (0u8..).zip(c.macros.iter().flatten()) {
        if s.read_macro(i).ok().as_ref() != Some(m) {
            let header = CsMacroHeaderWire {
                name: m.name.clone(),
                step_count: m.step_count,
            };
            cs_written(
                s.write_macro(i, &header, &m.steps),
                format!("macro {}", u16::from(i) + 1),
                out,
            )?;
        }
    }
    for (i, cmd) in (0u8..).zip(c.ir.iter().flatten()) {
        if s.read_ir_command(i).ok().as_ref() != Some(cmd) {
            cs_written(
                s.write_ir_command(i, cmd),
                format!("IR command {}", u16::from(i) + 1),
                out,
            )?;
        }
    }
    // Twice: a binding that frees a GPIO another one wants goes first.
    for pass in 0..2 {
        for (slot, (b, _)) in (0u8..).zip(&c.bindings) {
            if s.read_binding(slot).ok().as_ref() != Some(b) {
                let r = s.write_binding(slot, b);
                // The first pass may be refused for a GPIO the second frees.
                if pass == 0 && matches!(&r, Ok(st) if st.last_status != cs_status::SUCCESS) {
                    continue;
                }
                cs_written(r, format!("binding {}", u16::from(slot) + 1), out)?;
            }
        }
    }
    for (slot, (_, name)) in (0u8..).zip(&c.bindings) {
        if s.read_name(slot).ok().as_ref() != Some(name) {
            cs_written(
                s.write_name(slot, name),
                format!("binding {} name", u16::from(slot) + 1),
                out,
            )?;
        }
    }
    if let Some((cfg, pages)) = &c.display {
        if s.read_display_cfg().ok().map(|r| r.cfg).as_ref() != Some(cfg) {
            cs_written(
                s.write_display_cfg(cfg),
                "display configuration".into(),
                out,
            )?;
        }
        for (i, p) in (0u8..).zip(pages) {
            if s.read_display_page(i).ok().as_ref() != Some(p) {
                cs_written(
                    s.write_display_page(i, p),
                    format!("display page {}", u16::from(i) + 1),
                    out,
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests_wp_p1cfix {
    use super::*;

    #[test]
    fn wp_p1cfix_a_name_field_is_cut_at_its_nul_and_padded() {
        let mut raw = b"Stage left".to_vec();
        raw.push(0);
        raw.extend_from_slice(b"garbage after the NUL");
        let f = name_field(&raw);
        assert_eq!(f.len(), NAME_LEN);
        assert_eq!(&f[..10], b"Stage left");
        assert!(f[10..].iter().all(|&b| b == 0));
        assert_eq!(name_field(&[]), vec![0; NAME_LEN], "unnamed clears");
        assert_eq!(name_field(&[b'x'; 40]).len(), NAME_LEN);
    }
}
