//! Control surfaces for `dspi --demo cs`.
//!
//! [`FakeDevice`] does not model control surfaces: its GETs stall, on purpose
//! (`dspi_transport::fake`, and its test `wp_t4_unknown_requests_stall`). So
//! `dspi cs` on the demo device talks to it through this wrapper, which answers
//! the `REQ_*_CS_*` requests from `cs_model`'s demo data and hands everything
//! else on. It does what `Surfaces` relies on and little more: records are held
//! and read back, a SET settles at once under its slot tag, and the two refusals
//! a whole-document import can run into are enforced. Only `dspi cs` uses it, so
//! the rest of `--demo` (and its TUI) still shows a device without surfaces.

use std::collections::HashMap;

use dspi_proto::generated::opcodes as op;
use dspi_proto::packets::{
    CsBinding, CsCapsHeader, CsDisplayCfg, CsDisplayCfgReply, CsDisplayPage, CsDisplayStatus,
    CsExtStatusPacket, CsGroup, CsMacro, CsMacroHeaderWire, CsMacroStep, CsNounDesc,
    CsStatusPacket, GPIO_UNUSED, IrCommand,
};
use dspi_session::Session;
use dspi_session::cs_model::{CsData, demo};
use dspi_session::surfaces::{status, tag};
use dspi_transport::{DeviceDescriptor, FakeDevice, Result, Transport, TransportError};

/// `CS_STATUS_INVALID_SLOT`, `_INVALID_TYPE` and the pin-and-gesture clash
/// (control_surfaces.h; `dspi_session::surfaces::explain_status` words them).
const NO_SUCH_SLOT: u8 = 0x10;
const NO_SUCH_TYPE: u8 = 0x11;
const PIN_CLASH: u8 = 0x1A;

/// The demo device with surfaces, as a transport.
pub(crate) fn device() -> DemoSurfaces {
    DemoSurfaces::new(FakeDevice::demo())
}

/// The demo device with surfaces, probed.
pub(crate) fn session() -> std::result::Result<Session, String> {
    let mut t: Box<dyn Transport> = Box::new(device());
    let caps = dspi_session::probe::probe(t.as_mut()).map_err(|e| e.to_string())?;
    Session::new(t, caps).ok_or_else(|| "the demo device has no channel map".to_string())
}

pub(crate) struct DemoSurfaces {
    inner: FakeDevice,
    data: CsData,
    header: CsCapsHeader,
    nouns: HashMap<u16, CsNounDesc>,
    last_status: u8,
    last_slot: u8,
    dirty: bool,
}

/// `len` bytes of `reply`, as a device answers a read: shorter than asked is the
/// transport's `ShortRead`, which [`Transport::control_in`] promises.
fn cut(opcode: u8, mut reply: Vec<u8>, len: u16) -> Result<Vec<u8>> {
    if reply.len() < len as usize {
        return Err(TransportError::ShortRead {
            opcode,
            wanted: len as usize,
            got: reply.len(),
        });
    }
    reply.truncate(len as usize);
    Ok(reply)
}

fn stall(opcode: u8) -> TransportError {
    TransportError::Stalled { opcode }
}

impl DemoSurfaces {
    fn new(inner: FakeDevice) -> Self {
        let caps = demo::caps();
        let header = CsCapsHeader {
            caps_version: caps.caps_version,
            max_bindings: caps.max_bindings,
            type_count: caps.type_count,
            noun_count: caps.noun_count,
            types: caps.types.clone(),
            max_ir_commands: caps.max_ir_commands,
            max_groups: caps.max_groups,
            max_macros: caps.max_macros,
            max_macro_steps: caps.max_macro_steps,
        };
        Self {
            inner,
            data: demo::data(),
            header,
            nouns: (0u16..).zip(demo::nouns()).collect(),
            last_status: status::SUCCESS,
            last_slot: 0,
            dirty: false,
        }
    }

    fn settle(&mut self, slot: u8, code: u8) {
        self.last_slot = slot;
        self.last_status = code;
        self.dirty |= code == status::SUCCESS;
    }

    fn status_packet(&self) -> Vec<u8> {
        CsStatusPacket {
            last_status: self.last_status,
            last_slot: self.last_slot,
            max_bindings: self.header.max_bindings,
            dirty: self.dirty,
            slot_status: vec![0; self.header.max_bindings as usize],
            ir_cmd_status: vec![0; self.header.max_ir_commands as usize],
            ..Default::default()
        }
        .encode()
    }

    fn get(&mut self, opcode: u8, value: u16, len: u16) -> Option<Result<Vec<u8>>> {
        let i = value as usize;
        let reply: Vec<u8> = match opcode {
            op::REQ_GET_CS_CAPS if value == 0xFFFF => self.header.encode(),
            op::REQ_GET_CS_CAPS => self.nouns.get(&value)?.encode().to_vec(),
            op::REQ_GET_CS_BINDING => self.data.bindings.get(i)?.encode().to_vec(),
            op::REQ_GET_CS_NAME => {
                let mut b = self.data.names.get(i)?.as_bytes().to_vec();
                b.resize(32, 0);
                b
            }
            op::REQ_GET_CS_IR_CMD => self.data.ir.get(i)?.encode().to_vec(),
            op::REQ_GET_CS_GROUP => self.data.groups.get(i)?.encode().to_vec(),
            op::REQ_GET_CS_MACRO => {
                let m = self.data.macros.get(i)?;
                let mut b = m.header().encode().to_vec();
                for s in m.steps.iter().take(self.header.max_macro_steps as usize) {
                    b.extend_from_slice(&s.encode());
                }
                b.resize(CsMacro::SIZE, 0);
                b
            }
            op::REQ_GET_CS_STATUS => self.status_packet(),
            op::REQ_GET_CS_EXT_STATUS => CsExtStatusPacket {
                max_groups: self.header.max_groups,
                max_macros: self.header.max_macros,
                max_macro_steps: self.header.max_macro_steps,
                macro_running: CsExtStatusPacket::MACRO_NONE,
                ..Default::default()
            }
            .encode()
            .to_vec(),
            op::REQ_GET_CS_DISPLAY_CFG => CsDisplayCfgReply {
                max_pages: self.data.max_pages,
                model_count: self.data.model_count,
                cfg: self.data.display_cfg.clone(),
            }
            .encode()
            .to_vec(),
            op::REQ_GET_CS_DISPLAY_PAGE => self.data.pages.get(i)?.encode().to_vec(),
            op::REQ_GET_CS_DISPLAY_STATUS => CsDisplayStatus::default().encode().to_vec(),
            op::REQ_CS_SAVE => {
                self.dirty = false;
                self.settle(tag::SAVE, status::SUCCESS);
                self.dirty = false;
                vec![status::SUCCESS]
            }
            _ => return None,
        };
        Some(cut(opcode, reply, len))
    }

    /// Why this binding cannot go in `slot`, as the firmware's checks would say.
    fn refuse_binding(&self, slot: usize, b: &CsBinding) -> Option<u8> {
        if slot >= self.data.bindings.len() {
            return Some(NO_SUCH_SLOT);
        }
        if b.is_empty() {
            return None;
        }
        if b.component >= self.header.type_count {
            return Some(NO_SUCH_TYPE);
        }
        let clash = self.data.bindings.iter().enumerate().any(|(i, o)| {
            i != slot
                && !o.is_empty()
                && o.event == b.event
                && b.gpio
                    .iter()
                    .any(|p| *p != GPIO_UNUSED && *p != 0 && o.gpio.contains(p))
        });
        clash.then_some(PIN_CLASH)
    }

    fn set(&mut self, opcode: u8, value: u16, d: &[u8]) -> Option<Result<()>> {
        let i = (value & 0xFF) as usize;
        let slot = (value & 0xFF) as u8;
        let bad = |e| Some(Err(TransportError::Remote(format!("bad record: {e}"))));
        match opcode {
            op::REQ_SET_CS_BINDING => {
                let b = match CsBinding::decode(d) {
                    Ok(b) => b,
                    Err(e) => return bad(e),
                };
                let code = self.refuse_binding(i, &b).unwrap_or(status::SUCCESS);
                if code == status::SUCCESS {
                    *self.data.bindings.get_mut(i)? = b;
                }
                self.settle(slot, code);
            }
            op::REQ_SET_CS_NAME => {
                let end = d.iter().position(|b| *b == 0).unwrap_or(d.len());
                *self.data.names.get_mut(i)? = String::from_utf8_lossy(&d[..end]).into_owned();
                self.settle(slot, status::SUCCESS);
            }
            op::REQ_SET_CS_IR_CMD => {
                let c = match IrCommand::decode(d) {
                    Ok(c) => c,
                    Err(e) => return bad(e),
                };
                *self.data.ir.get_mut(i)? = c;
                self.settle(tag::IR | slot, status::SUCCESS);
            }
            op::REQ_SET_CS_GROUP => {
                let g = match CsGroup::decode(d) {
                    Ok(g) => g,
                    Err(e) => return bad(e),
                };
                *self.data.groups.get_mut(i)? = g;
                self.settle(tag::GROUP | slot, status::SUCCESS);
            }
            op::REQ_SET_CS_MACRO => {
                let h = match CsMacroHeaderWire::decode(d) {
                    Ok(h) => h,
                    Err(e) => return bad(e),
                };
                let m = self.data.macros.get_mut(i)?;
                m.name = h.name;
                m.step_count = h.step_count;
                self.settle(tag::MACRO | slot, status::SUCCESS);
            }
            op::REQ_SET_CS_MACRO_STEP => {
                let s = match CsMacroStep::decode(d) {
                    Ok(s) => s,
                    Err(e) => return bad(e),
                };
                let at = (value >> 8) as usize;
                let m = self.data.macros.get_mut(i)?;
                if m.steps.len() <= at {
                    m.steps.resize(at + 1, CsMacroStep::default());
                }
                m.steps[at] = s;
                self.settle(tag::MACRO | slot, status::SUCCESS);
            }
            op::REQ_SET_CS_DISPLAY_CFG => {
                match CsDisplayCfg::decode(d) {
                    Ok(c) => self.data.display_cfg = c,
                    Err(e) => return bad(e),
                }
                self.settle(tag::DISPLAY, status::SUCCESS);
            }
            op::REQ_SET_CS_DISPLAY_PAGE => {
                let p = match CsDisplayPage::decode(d) {
                    Ok(p) => p,
                    Err(e) => return bad(e),
                };
                *self.data.pages.get_mut(i)? = p;
                self.settle(tag::DISPLAY | slot, status::SUCCESS);
            }
            _ => return None,
        }
        Some(Ok(()))
    }
}

impl Transport for DemoSurfaces {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        match self.get(opcode, value, len) {
            Some(r) => r,
            None => self.inner.control_in(opcode, value, len),
        }
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> Result<Vec<u8>> {
        self.inner.control_in_upto(opcode, value, max_len)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        match self.set(opcode, value, data) {
            Some(r) => r,
            None => {
                let is_cs = matches!(
                    opcode,
                    op::REQ_SET_CS_BINDING
                        | op::REQ_SET_CS_NAME
                        | op::REQ_SET_CS_IR_CMD
                        | op::REQ_SET_CS_GROUP
                        | op::REQ_SET_CS_MACRO
                        | op::REQ_SET_CS_MACRO_STEP
                        | op::REQ_SET_CS_DISPLAY_PAGE
                );
                if is_cs {
                    return Err(stall(opcode));
                }
                self.inner.control_out(opcode, value, data)
            }
        }
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        self.inner.descriptor()
    }

    fn max_transfer(&self) -> usize {
        self.inner.max_transfer()
    }

    fn notifications(&self) -> Option<Box<dyn dspi_transport::NotificationSource>> {
        self.inner.notifications()
    }
}
