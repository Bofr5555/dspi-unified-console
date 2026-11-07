//! Control surfaces (JSON for the editor).
//!
//! Owned by WP C14 (docs/core-merge/PLAN.md).
//!
//! GPIO buttons, switches, pots, encoders, LEDs, an IR receiver, an I2C
//! display and auxiliary outputs, each bound to a device function (a "noun")
//! through an action; plus channel groups and macros. The configuration lives
//! on the device, not in presets: every SET applies live and marks it dirty,
//! `save` writes it to flash and `revert` reloads the saved one.
//!
//! The editor talks to this module in JSON, in the old desktop core's shapes
//! (its `cs.rs`, deleted by WP Q3, last at `d190535`), which the Qt
//! `CsController` parses:
//! [`DspiCore::cs_snapshot`] describes everything, [`DspiCore::cs_apply`]
//! performs one operation and re-reads what it changed.
//!
//! The records, their codecs and the deferred-SET wait are the session's
//! (`dspi_session::cs_model` and `dspi_session::surfaces`, WP S4); this module
//! keeps the last read ([`CsState`]), turns it into JSON and parses the
//! editor's operations. That JSON is a trust boundary (PLAN §1.7): numbers are
//! range-checked into their wire types and a bad one refuses the whole
//! operation (`InvalidArgument`), indices are checked against the limits the
//! device reported, and nothing in an operation sizes an allocation.

use std::time::Duration;

use serde_json::{Value, json};

use dspi_session::Session;
use dspi_session::cs_model::{self, ApplyError, CsData, aux_supported, ty};
use dspi_session::surfaces::{
    AUX_LEVEL_MAX_Q8, CsAuxStates, CsBinding, CsDisplayCfg, CsDisplayPage, CsExtStatusPacket,
    CsGroup, CsMacroHeaderWire, CsMacroStep, CsStatusPacket, GPIO_UNUSED, IrCommand, IrLearnResult,
    Limits, SurfaceError, Surfaces, learn, status as cs_status,
};
use dspi_transport::TransportError;

use crate::presets::Holder;
use crate::records::PinUse;
use crate::{DspiCore, DspiError, Inner, Result, Wire};

/// `CS_MAX_BINDINGS` (control_surfaces.h:340): the old JSON always carries
/// this many names, slot states and aux entries.
const MAX_BINDINGS: usize = 16;
/// `CS_MAX_DISPLAY_PAGES` (control_surfaces.h:351): the old JSON always
/// carries this many display pages.
const MAX_PAGES: usize = 16;
/// IR sub-slots the status JSON always lists (caps v6 and later).
const MAX_IR: usize = 16;
/// `CS_STATUS_NO_IR` (control_surfaces.h:710): learning needs a live IR
/// receiver; the arm request stalls without one.
const STATUS_NO_IR: u8 = 0x1E;
/// How long a macro write waits for the DSPi Link device lock.
const LOCK_TIMEOUT: Duration = Duration::from_secs(60);
// -- WP CG24 --
/// How long an IR learn holds the device lock: the device's 10 s window plus
/// the UI's polling margin (Swift's `irLearnLockTimeout`).
pub(crate) const LEARN_WINDOW: Duration = Duration::from_secs(15);
const LEARN_REASON: &str = "Learning an IR code";

/// The control-surface editor's state (WP C14), on the session's CS model
/// (WP S4): the last read of the device, kept between operations.
#[derive(Default)]
pub(crate) struct CsState {
    /// The serial of the connection this was read from: a state read from
    /// another device (or none yet) is not this device's.
    owner: Option<String>,
    /// Everything read; `None` when the device has no control surfaces.
    data: Option<CsData>,
    /// The live aux states and levels (caps v18 and later).
    aux: CsAuxStates,
    /// The last IR learn result a notification reported
    /// ([`CsState::note_learn`], WP C7).
    learn: Option<IrLearnResult>,
    /// The configuration as last saved (or read clean), to tell a real
    /// unsaved edit from changes that were undone.
    clean: Option<ConfigKey>,
}

/// The parts of the configuration an edit can change, normalised so an
/// emptied slot compares equal whatever its other bytes hold.
#[derive(Debug, Clone, PartialEq)]
struct ConfigKey {
    bindings: Vec<CsBinding>,
    names: Vec<String>,
    ir: Vec<IrCommand>,
    groups: Vec<CsGroup>,
    macros: Vec<(String, Vec<CsMacroStep>)>,
    display: CsDisplayCfg,
    pages: Vec<CsDisplayPage>,
}

impl ConfigKey {
    fn of(d: &CsData) -> Self {
        Self {
            bindings: d
                .bindings
                .iter()
                .map(|b| {
                    if b.is_empty() {
                        CsBinding::default()
                    } else {
                        b.clone()
                    }
                })
                .collect(),
            names: d.names.clone(),
            ir: d
                .ir
                .iter()
                .map(|c| {
                    if c.is_empty() {
                        IrCommand::default()
                    } else {
                        c.clone()
                    }
                })
                .collect(),
            groups: d.groups.clone(),
            macros: d
                .macros
                .iter()
                .map(|m| (m.name.clone(), m.active_steps().to_vec()))
                .collect(),
            display: d.display_cfg.clone(),
            pages: d.pages.clone(),
        }
    }
}

impl CsState {
    /// The data, if it was read from the connection with this serial.
    pub(crate) fn data_for(&self, serial: &str) -> Option<&CsData> {
        if self.owner.as_deref() == Some(serial) {
            self.data.as_ref()
        } else {
            None
        }
    }

    /// Unsaved edits: the device says dirty and the configuration differs
    /// from the clean one (none known: every edit counts).
    fn unsaved(&self) -> bool {
        let Some(d) = &self.data else { return false };
        d.status.dirty && self.clean.as_ref() != Some(&ConfigKey::of(d))
    }

    fn mark_clean(&mut self) {
        self.clean = self.data.as_ref().map(ConfigKey::of);
    }

    /// An IR learn result from a notification (`EVT_CS_IR_LEARN`), shown as
    /// the snapshot's `learn` (for WP C7).
    pub(crate) fn note_learn(&mut self, state: u8, protocol: u8, code: u32) {
        self.learn = Some(IrLearnResult {
            state,
            protocol,
            code,
        });
    }

    /// A live aux change from a notification (`EVT_CS_AUX`) that was not
    /// our own write (for WP C7). A slot past the table is ignored.
    pub(crate) fn note_aux(&mut self, slot: usize, state: u8, level_q8: u16) {
        if let Some(s) = self.aux.state.get_mut(slot) {
            *s = state;
        }
        if let Some(l) = self.aux.level_q8.get_mut(slot) {
            *l = level_q8;
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// A control-surface write's failure, typed: a settled refusal is the
/// device's status code; a write that never settled is `CS_STATUS_PENDING`,
/// as the old core reported it.
fn apply_error(e: ApplyError) -> DspiError {
    match e {
        ApplyError::Rejected { status } => DspiError::Refused { status },
        ApplyError::Surfaces(s) => surface_error(s),
        ApplyError::Transport(t) => t.into(),
    }
}

fn surface_error(e: SurfaceError) -> DspiError {
    match e {
        SurfaceError::Transport(t) => t.into(),
        SurfaceError::Timeout { .. } => DspiError::Refused {
            status: cs_status::PENDING,
        },
        SurfaceError::Packet(p) => DspiError::Transfer { msg: p.to_string() },
    }
}

/// Whether a failed request was the device's stall (its way of refusing an
/// IN request), as opposed to a link failure.
fn is_stall(e: &SurfaceError) -> bool {
    matches!(e, SurfaceError::Transport(TransportError::Stalled { .. }))
}

// ---------------------------------------------------------------------------
// Device access
// ---------------------------------------------------------------------------

/// The limits the session probed, or the defaults on a device without CS.
fn limits(session: &Session) -> Limits {
    session
        .capabilities()
        .cs
        .as_ref()
        .map(Limits::from)
        .unwrap_or_default()
}

/// Run `f` over the session's control-surface reader/writer.
fn with_surfaces<T>(session: &mut Session, f: impl FnOnce(&mut Surfaces<'_>) -> T) -> Result<T> {
    let limits = limits(session);
    Ok(session.with_transport(|t| {
        let mut s = Surfaces::new(t, limits);
        Ok(f(&mut s))
    })?)
}

/// Re-read one binding and its name. A failed read keeps the old record, as
/// the old core did.
fn read_binding_into(s: &mut Surfaces<'_>, d: &mut CsData, slot: u8) {
    if let Ok(b) = s.read_binding(slot)
        && let Some(x) = d.bindings.get_mut(usize::from(slot))
    {
        *x = b;
    }
    if let Ok(n) = s.read_name(slot)
        && let Some(x) = d.names.get_mut(usize::from(slot))
    {
        *x = n;
    }
}

/// Re-read the status packet, and the extended one where groups or macros
/// exist.
fn read_status_into(s: &mut Surfaces<'_>, d: &mut CsData) {
    if let Ok(st) = s.read_status() {
        d.status = st;
    }
    if (d.caps.max_groups > 0 || d.caps.max_macros > 0)
        && let Ok(e) = s.read_ext_status()
    {
        d.ext = e;
    }
}

/// Re-read the display config, its pages and its live state, on a type
/// table that reaches `CS_TYPE_DISPLAY` (caps v10).
fn read_display_into(s: &mut Surfaces<'_>, d: &mut CsData) {
    if d.caps.type_count <= ty::DISPLAY {
        return;
    }
    if let Ok(r) = s.read_display_cfg() {
        d.max_pages = r.max_pages;
        d.model_count = r.model_count;
        d.display_cfg = r.cfg;
    }
    d.pages
        .resize(usize::from(d.max_pages), CsDisplayPage::default());
    for p in 0..d.max_pages {
        if let Ok(page) = s.read_display_page(p)
            && let Some(x) = d.pages.get_mut(usize::from(p))
        {
            *x = page;
        }
    }
    if let Ok(st) = s.read_display_status() {
        d.display_status = st;
    }
}

/// Re-read the live aux states (caps v18 and later).
fn read_aux_into(s: &mut Surfaces<'_>, d: &CsData, aux: &mut CsAuxStates) {
    if aux_supported(&d.caps)
        && let Ok(a) = s.read_aux_states()
    {
        *aux = a;
    }
}

// -- WP CG16 --
/// Whether an aux SET the status has no complaint about took. During a revert
/// the firmware acknowledges it, changes nothing and leaves the status alone
/// (vendor_commands.c, `REQ_SET_CS_AUX_STATE`), so only the output's own state
/// tells: not as asked is `BUSY`. `aux` becomes what the device holds, either way.
fn confirm_aux(
    session: &mut Session,
    aux: &mut CsAuxStates,
    took: impl FnOnce(&CsAuxStates) -> bool,
) -> Result<()> {
    let live = with_surfaces(session, |s| s.read_aux_states())?.map_err(surface_error)?;
    let ok = took(&live);
    *aux = live;
    if ok {
        Ok(())
    } else {
        Err(DspiError::Refused {
            status: cs_status::BUSY,
        })
    }
}

/// Read the whole configuration into `cs`, for the open connection. Reads
/// only: a read-only connection may be fetched.
fn fetch_into(inner: &mut Inner, cs: &mut CsState) -> Result<()> {
    let conn = inner.conn()?;
    let serial = conn.serial.clone();
    // A state from another device keeps nothing of it.
    if cs.owner.as_deref() != Some(serial.as_str()) {
        *cs = CsState {
            owner: Some(serial),
            ..CsState::default()
        };
    }
    // An unmappable image (§1.5) has no session: shown, but without CS.
    if matches!(conn.link, Wire::Raw(_)) {
        cs.data = None;
        return Ok(());
    }
    let session = conn.session()?;
    if session.capabilities().cs.is_none() {
        cs.data = None;
        return Ok(());
    }
    // `CsData::read` tolerates every failure (an older firmware lacks parts
    // of it), so ask once first whether the device is there at all: a lost
    // or timed-out device is an error, not an empty configuration. A stall
    // or an odd packet is left to `CsData::read`.
    if let Err(SurfaceError::Transport(t)) = with_surfaces(session, |s| s.read_status())?
        && !matches!(t, TransportError::Stalled { .. } | TransportError::Usb(_))
    {
        return Err(t.into());
    }
    let Some(data) = CsData::read(session) else {
        cs.data = None;
        return Ok(());
    };
    let mut aux = CsAuxStates::default();
    with_surfaces(session, |s| read_aux_into(s, &data, &mut aux))?;
    let dirty = data.status.dirty;
    cs.data = Some(data);
    cs.aux = aux;
    // A device that is not dirty is the baseline; a dirty one keeps the
    // known baseline (none at the first read: its edits count as unsaved).
    if !dirty {
        cs.mark_clean();
    }
    Ok(())
}

/// Make sure `cs` holds the open device's configuration, reading it once if
/// it does not.
fn ensure_fetched(inner: &mut Inner, cs: &mut CsState) -> Result<()> {
    let serial = &inner.conn()?.serial;
    if cs.owner.as_deref() != Some(serial.as_str()) {
        fetch_into(inner, cs)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// JSON in: the editor's operations (untrusted)
// ---------------------------------------------------------------------------

pub(crate) fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

/// A number field: absent or null is `None`; an integer, a finite float
/// (rounded) or a bool (0/1) is the value; anything else is refused.
fn int(v: &Value, key: &str) -> Result<Option<i64>> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(i64::from(*b))),
        Some(Value::Number(n)) => {
            if let Some(i) = n.as_i64() {
                return Ok(Some(i));
            }
            match n.as_f64() {
                // Below 2^53 the rounded float is exact and the cast cannot
                // saturate; a larger one is out of every field's range.
                Some(f) if f.is_finite() && f.abs() < 9.0e15 => Ok(Some(f.round() as i64)),
                _ => Err(invalid(format!("{key} is out of range"))),
            }
        }
        Some(_) => Err(invalid(format!("{key} must be a number"))),
    }
}

/// A number field in its wire type, `default` when absent; out of the
/// type's range is refused, never wrapped.
fn num<T: TryFrom<i64>>(v: &Value, key: &str, default: T) -> Result<T> {
    match int(v, key)? {
        None => Ok(default),
        Some(n) => T::try_from(n).map_err(|_| invalid(format!("{key} {n} is out of range"))),
    }
}

/// A string field, empty when absent.
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(""),
        Some(Value::String(s)) => Ok(s),
        Some(_) => Err(invalid(format!("{key} must be a string"))),
    }
}

/// A sub-object, `Null` when absent (every field then takes its default).
fn object<'a>(v: &'a Value, key: &str) -> Result<&'a Value> {
    match v.get(key) {
        None => Ok(&Value::Null),
        Some(o @ (Value::Object(_) | Value::Null)) => Ok(o),
        Some(_) => Err(invalid(format!("{key} must be an object"))),
    }
}

/// The operation's `index` (0 when absent), checked against the device's
/// `limit`.
fn index(op: &Value, limit: usize, what: &str) -> Result<u8> {
    let i: i64 = num(op, "index", 0)?;
    match u8::try_from(i) {
        Ok(i) if usize::from(i) < limit => Ok(i),
        _ => Err(invalid(format!("{what} {i} does not exist on this device"))),
    }
}

pub(crate) fn binding_from_json(v: &Value) -> Result<CsBinding> {
    Ok(CsBinding {
        component: num(v, "type", 0)?,
        noun: num(v, "noun", 0)?,
        action: num(v, "action", 0)?,
        flags: num(v, "flags", 0)?,
        gpio: [num(v, "gpio0", 0)?, num(v, "gpio1", GPIO_UNUSED)?],
        event: num(v, "event", 0)?,
        target: num(v, "target", 0)?,
        index: num(v, "index", 0)?,
        base_bright: num(v, "baseBright", 0)?,
        value: num(v, "value", 0)?,
        step: num(v, "step", 0)?,
        range_min: num(v, "rangeMin", 0)?,
        range_max: num(v, "rangeMax", 0)?,
        on_delay: num(v, "onDelay", 0)?,
        off_delay: num(v, "offDelay", 0)?,
        extras: num(v, "extras", 0)?,
    })
}

pub(crate) fn ir_from_json(v: &Value) -> Result<IrCommand> {
    Ok(IrCommand {
        noun: num(v, "noun", 0)?,
        action: num(v, "action", 0)?,
        flags: num(v, "flags", 0)?,
        target: num(v, "target", 0)?,
        index: num(v, "index", 0)?,
        protocol: num(v, "protocol", 0)?,
        value: num(v, "value", 0)?,
        step: num(v, "step", 0)?,
        code: num(v, "code", 0)?,
    })
}

pub(crate) fn group_from_json(v: &Value) -> Result<CsGroup> {
    Ok(CsGroup {
        target_kind: num(v, "targetKind", 0)?,
        member_mask: num(v, "memberMask", 0)?,
        name: text(v, "name")?.to_owned(),
    })
}

pub(crate) fn step_from_json(v: &Value) -> Result<CsMacroStep> {
    Ok(CsMacroStep {
        noun: num(v, "noun", 0)?,
        action: num(v, "action", 0)?,
        flags: num(v, "flags", 0)?,
        target: num(v, "target", 0)?,
        index: num(v, "index", 0)?,
        value: num(v, "value", 0)?,
        step: num(v, "step", 0)?,
        pre_delay: num(v, "preDelay", 0)?,
    })
}

pub(crate) fn display_cfg_from_json(v: &Value) -> Result<CsDisplayCfg> {
    Ok(CsDisplayCfg {
        mode: num(v, "mode", 0)?,
        home_page: num(v, "homePage", 0)?,
        dwell: num(v, "dwell", 0)?,
        overlay_hold: num(v, "overlayHold", 0)?,
        brightness: num(v, "brightness", 0)?,
        flags: num(v, "flags", 0)?,
        edit_timeout: num(v, "editTimeout", 0)?,
    })
}

pub(crate) fn page_from_json(v: &Value) -> Result<CsDisplayPage> {
    Ok(CsDisplayPage {
        noun: num(v, "noun", 0)?,
        target: num(v, "target", 0)?,
        index: num(v, "index", 0)?,
        flags: num(v, "flags", 0)?,
    })
}

// ---------------------------------------------------------------------------
// JSON out: the old core's shapes
// ---------------------------------------------------------------------------

pub(crate) fn binding_json(b: &CsBinding) -> Value {
    let [gpio0, gpio1] = b.gpio;
    json!({ "type": b.component, "noun": b.noun, "action": b.action, "flags": b.flags,
            "gpio0": gpio0, "gpio1": gpio1, "event": b.event, "target": b.target,
            "index": b.index, "baseBright": b.base_bright, "value": b.value, "step": b.step,
            "rangeMin": b.range_min, "rangeMax": b.range_max, "onDelay": b.on_delay,
            "offDelay": b.off_delay, "extras": b.extras })
}

pub(crate) fn ir_json(c: &IrCommand) -> Value {
    json!({ "noun": c.noun, "action": c.action, "flags": c.flags, "target": c.target,
            "index": c.index, "protocol": c.protocol, "value": c.value, "step": c.step,
            "code": c.code })
}

pub(crate) fn step_json(s: &CsMacroStep) -> Value {
    json!({ "noun": s.noun, "action": s.action, "flags": s.flags, "target": s.target,
            "index": s.index, "value": s.value, "step": s.step, "preDelay": s.pre_delay })
}

/// Exactly `n` entries of `v`, zero-padded (the old JSON's fixed tables).
fn fixed<T: Clone + Default>(v: &[T], n: usize) -> Vec<T> {
    let mut out: Vec<T> = v.iter().take(n).cloned().collect();
    out.resize(n, T::default());
    out
}

/// At least `n` entries of `v`, zero-padded.
fn at_least<T: Clone + Default>(v: &[T], n: usize) -> Vec<T> {
    let mut out = v.to_vec();
    if out.len() < n {
        out.resize(n, T::default());
    }
    out
}

fn status_json(cs: &CsState, s: &CsStatusPacket) -> Value {
    json!({
        "lastStatus": s.last_status, "lastSlot": s.last_slot, "dirty": cs.unsaved(),
        "activeMask": s.active_mask,
        "slotStatus": fixed(&s.slot_status, MAX_BINDINGS),
        "irActiveMask": s.ir_active_mask,
        "learnState": s.ir_learn_state,
        "irStatus": fixed(&s.ir_cmd_status, MAX_IR),
    })
}

/// The configuration as the old core's JSON.
fn snapshot(cs: &CsState, d: &CsData) -> Value {
    let e = &d.ext;
    // An extended status never read (no groups or macros) shows no macro
    // running, as the old core did.
    let running = if *e == CsExtStatusPacket::default() {
        CsExtStatusPacket::MACRO_NONE
    } else {
        e.macro_running
    };
    let ds = &d.display_status;
    let c = &d.display_cfg;
    json!({
        "supported": true,
        "capsVersion": d.caps.caps_version,
        "maxBindings": d.caps.max_bindings,
        "types": d.caps.types.iter()
            .map(|t| json!({ "actions": t.actions, "pins": t.pin_count, "pinClass": t.pin_class }))
            .collect::<Vec<_>>(),
        // -- WP CG20 -- `name` as an indicator shows it, `controlName` as a control
        // acts on it (cs_model::noun_name): the clients' noun tables come from here.
        "nouns": (0..=u8::MAX).zip(d.nouns.iter()).map(|(i, n)| json!({ "kind": n.kind,
            "enumCount": n.enum_count,
            "actions": n.actions, "min": n.min_q, "max": n.max_q, "unit": n.unit,
            "targetKind": n.target_kind, "targetCount": n.target_count,
            "deferred": n.dflags & 1 != 0,
            "name": cs_model::noun_name(i, ty::LED),
            "controlName": cs_model::noun_name(i, ty::BUTTON) })).collect::<Vec<_>>(),
        "maxIr": d.caps.max_ir_commands, "maxGroups": d.caps.max_groups,
        "maxMacros": d.caps.max_macros, "maxSteps": d.caps.max_macro_steps,
        "status": status_json(cs, &d.status),
        "bindings": d.bindings.iter().map(binding_json).collect::<Vec<_>>(),
        "names": at_least(&d.names, MAX_BINDINGS),
        "ir": d.ir.iter().map(ir_json).collect::<Vec<_>>(),
        "groups": d.groups.iter().map(|g| json!({ "targetKind": g.target_kind,
            "memberMask": g.member_mask, "name": g.name })).collect::<Vec<_>>(),
        "macros": d.macros.iter().map(|m| json!({ "name": m.name,
            "steps": m.active_steps().iter().map(step_json).collect::<Vec<_>>() }))
            .collect::<Vec<_>>(),
        "ext": { "macroRunning": running, "macroStep": e.macro_step,
                 "groupStatus": e.group_status, "macroStatus": e.macro_status },
        "display": {
            "maxPages": d.max_pages, "modelCount": d.model_count,
            "config": { "mode": c.mode, "homePage": c.home_page, "dwell": c.dwell,
                        "overlayHold": c.overlay_hold, "brightness": c.brightness,
                        "flags": c.flags, "editTimeout": c.edit_timeout },
            "pages": at_least(&d.pages, MAX_PAGES).iter().map(|p| json!({ "noun": p.noun,
                "target": p.target, "index": p.index, "flags": p.flags })).collect::<Vec<_>>(),
            "status": { "init": ds.init_state, "page": ds.current_page, "flags": ds.flags,
                        "model": ds.model, "naks": ds.nak_count },
        },
        "auxState": fixed(&cs.aux.state, MAX_BINDINGS),
        "auxLevel": fixed(&cs.aux.level_q8, MAX_BINDINGS),
        "learn": cs.learn.map(|l| json!({ "state": l.state, "protocol": l.protocol, "code": l.code })),
    })
}

/// The reply to a write that settled with success.
fn applied() -> Value {
    json!({ "ok": true, "status": cs_status::SUCCESS })
}

/// The reply for a failed operation: a refusal keeps the device's status
/// code (the old core's `{ok: false, status}`), anything else is
/// `{ok: false, error}`.
pub(crate) fn failure(e: &DspiError) -> Value {
    match e {
        DspiError::Refused { status } => json!({ "ok": false, "status": status }),
        other => json!({ "ok": false, "error": other.to_string() }),
    }
}

/// A settled status code as a result: 0 applied, anything else refused.
fn settled(st: u8) -> Result<Value> {
    if st == cs_status::SUCCESS {
        Ok(applied())
    } else {
        Err(DspiError::Refused { status: st })
    }
}

// ---------------------------------------------------------------------------
// The API
// ---------------------------------------------------------------------------

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Read the whole control-surface configuration from the device:
    /// capabilities, bindings and names, IR commands, groups, macros, the
    /// display and the aux outputs (about seventy round trips).
    ///
    /// Firmware without control surfaces (or a device shown without a
    /// session, PLAN §1.5) reads as unsupported and is `Ok`. Reads only:
    /// allowed on a read-only connection.
    ///
    /// Errors: [`DspiError::Disconnected`] without a device; a transfer
    /// failure (lost device, timeout) as its typed error.
    pub fn cs_fetch(&self) -> Result<()> {
        self.with_cs(fetch_into)
    }

    /// The control-surface configuration as JSON, in the old core's shape
    /// (`{supported, capsVersion, maxBindings, types, nouns, maxIr,
    /// maxGroups, maxMacros, maxSteps, status, bindings, names, ir, groups,
    /// macros, ext, display, auxState, auxLevel, learn}`), from the last
    /// [`DspiCore::cs_fetch`] and the operations since. `{"supported":
    /// false}` before the first fetch of the open device, without a device,
    /// or on firmware without control surfaces. Never fails.
    pub fn cs_snapshot(&self) -> String {
        let inner = self.lock();
        let v = inner
            .conn
            .as_ref()
            .and_then(|c| inner.cs.data_for(&c.serial))
            .map(|d| snapshot(&inner.cs, d))
            .unwrap_or_else(|| json!({ "supported": false }));
        v.to_string()
    }

    /// One control-surface operation, JSON `{op, index, ...}` (the old
    /// core's ops, which the Qt `CsController` sends): `refresh`,
    /// `refreshStatus`, `binding`, `name`, `ir`, `group`, `macro`,
    /// `displayConfig`, `displayPage`, `displayStatus`, `save`, `revert`,
    /// `learnArm`, `learnCancel`, `learnRead`, `macroFire`, `macroCancel`,
    /// `auxState`, `auxLevel`, `auxRead`. Each write re-reads what it
    /// changed; the first operation on a device not yet fetched fetches it.
    ///
    /// Returns JSON `{ok, status}` (the device's status code; `0x16` = still
    /// pending) plus op-specific fields, or `{ok: false, error}` for a
    /// malformed operation (bad JSON, a number out of its field's range, an
    /// index the device does not have, an unknown op), a write on a
    /// read-only connection (nothing is sent), firmware without control
    /// surfaces, no device, or a transfer failure. `save` writes flash: the
    /// bridge gates it (DangerousOps).
    ///
    /// `learnArm` holds the device lock for the 15 s learn window (WP CG24),
    /// so another DSPi Link session gets `locked` meanwhile (and a refused
    /// lock is `{ok: false}` with nothing sent). A `learnRead` whose `state`
    /// is no longer ARMED, a failed `learnRead`, `learnCancel`, the window's
    /// end or the connection's close releases it;
    /// [`DspiCore::device_lock_release`] does not. Inside a lock taken with
    /// [`DspiCore::device_lock_acquire`] a learn takes no lock of its own and
    /// lasts only as long as that one.
    pub fn cs_apply(&self, op: String) -> String {
        let r = serde_json::from_str::<Value>(&op)
            .map_err(|e| invalid(format!("malformed operation: {e}")))
            .and_then(|v| self.cs_op(&v));
        match r {
            Ok(v) => v,
            Err(e) => failure(&e),
        }
        .to_string()
    }

    /// GPIOs held by live control-surface bindings, as (pin, slot): a
    /// binding reserves its pins only while its active bit is set. From the
    /// last read; empty before the first [`DspiCore::cs_fetch`] of the open
    /// device.
    pub fn cs_pin_uses(&self) -> Vec<PinUse> {
        let inner = self.lock();
        let Some(d) = inner
            .conn
            .as_ref()
            .and_then(|c| inner.cs.data_for(&c.serial))
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (slot, b) in (0..=u8::MAX).zip(d.bindings.iter()) {
            if b.is_empty() || !d.status.is_slot_active(slot) {
                continue;
            }
            let [g0, g1] = b.gpio;
            out.push(PinUse { pin: g0, slot });
            if g1 != GPIO_UNUSED {
                out.push(PinUse { pin: g1, slot });
            }
        }
        out
    }
}

impl DspiCore {
    /// Run `f` with the CS state taken out of `Inner`, so `f` may hold the
    /// connection (`writable_conn` borrows all of `Inner`) and the state at
    /// once.
    fn with_cs<T>(&self, f: impl FnOnce(&mut Inner, &mut CsState) -> Result<T>) -> Result<T> {
        let mut inner = self.lock();
        let mut cs = std::mem::take(&mut inner.cs);
        let r = f(&mut inner, &mut cs);
        inner.cs = cs;
        r
    }

    fn cs_op(&self, op: &Value) -> Result<Value> {
        if !op.is_object() {
            return Err(invalid("an operation is a JSON object"));
        }
        let kind = text(op, "op")?;
        match kind {
            "refresh" => self.cs_fetch().map(|()| json!({ "ok": true })),
            // -- WP CG24 --
            "learnArm" | "learnRead" | "learnCancel" => self.learn_op(kind, op),
            "refreshStatus" | "displayStatus" | "auxRead" => {
                self.with_cs(|inner, cs| read_op(inner, cs, kind))
            }
            "binding" | "name" | "ir" | "group" | "macro" | "displayConfig" | "displayPage"
            | "save" | "revert" | "macroFire" | "macroCancel" | "auxState" | "auxLevel" => {
                self.with_cs(|inner, cs| write_op(inner, cs, kind, op))
            }
            other => Err(invalid(format!("unknown operation {other}"))),
        }
    }

    // -- WP CG24 --
    /// The IR learn ops. The learn window holds the device lock
    /// ([`LEARN_WINDOW`]), so another session cannot rewrite the IR setup
    /// mid-learn (Swift's `csIrLearnArm`). A read that is no longer armed, a
    /// cancel, the window's end or the connection's going releases it;
    /// `device_lock_release` does not. A new arm starts a new window. Unlike
    /// Swift, a failed read (`{ok: false}`) ends the window too: it has no
    /// `state` of ARMED. Inside a caller's lock the learn takes nothing of
    /// its own and ends nothing: the window is the caller's lock's.
    fn learn_op(&self, kind: &str, op: &Value) -> Result<Value> {
        match kind {
            "learnArm" => {
                self.end_held_lock(Some(Holder::Learn));
                let lock = self.take_device_lock(LEARN_REASON, LEARN_WINDOW, true)?;
                match self.with_cs(|inner, cs| write_op(inner, cs, kind, op)) {
                    Ok(v) => {
                        self.hold_device_lock(lock, LEARN_WINDOW, Holder::Learn);
                        Ok(v)
                    }
                    Err(e) => {
                        self.release_device_lock(lock);
                        Err(e)
                    }
                }
            }
            "learnRead" => {
                let r = self.with_cs(|inner, cs| read_op(inner, cs, kind));
                if let Ok(v) = &r
                    && v["state"].as_u64() != Some(u64::from(learn::ARMED))
                {
                    self.end_held_lock(Some(Holder::Learn));
                }
                r
            }
            // "learnCancel"
            _ => {
                let r = self.with_cs(|inner, cs| write_op(inner, cs, kind, op));
                self.end_held_lock(Some(Holder::Learn));
                r
            }
        }
    }
}

/// The operations that only read.
fn read_op(inner: &mut Inner, cs: &mut CsState, kind: &str) -> Result<Value> {
    ensure_fetched(inner, cs)?;
    let session = inner.conn()?.session()?;
    let CsState { data, aux, .. } = cs;
    let d = data.as_mut().ok_or(DspiError::Unsupported)?;
    match kind {
        "refreshStatus" => with_surfaces(session, |s| read_status_into(s, d))?,
        "displayStatus" => {
            if let Ok(st) = with_surfaces(session, |s| s.read_display_status())? {
                d.display_status = st;
            }
        }
        "auxRead" => with_surfaces(session, |s| read_aux_into(s, d, aux))?,
        // "learnRead"
        _ => {
            let l = with_surfaces(session, |s| s.read_learn())?.map_err(surface_error)?;
            return Ok(
                json!({ "ok": true, "state": l.state, "protocol": l.protocol,
                              "code": l.code }),
            );
        }
    }
    Ok(json!({ "ok": true }))
}

/// The operations that change the device: refused on a read-only
/// connection before anything is sent (or read).
fn write_op(inner: &mut Inner, cs: &mut CsState, kind: &str, op: &Value) -> Result<Value> {
    inner.writable_conn()?;
    ensure_fetched(inner, cs)?;
    if cs.data.is_none() {
        return Err(DspiError::Unsupported);
    }
    if kind == "save" || kind == "revert" {
        return save_or_revert(inner, cs, kind == "save");
    }
    let mut w = inner.writable_conn()?;
    let CsState {
        data, aux, learn, ..
    } = cs;
    let d = data.as_mut().ok_or(DspiError::Unsupported)?;
    match kind {
        "binding" => {
            let slot = index(op, d.bindings.len(), "binding slot")?;
            let b = binding_from_json(object(op, "binding")?)?;
            let was = d.binding(usize::from(slot)).component;
            let session = w.session()?;
            let r = cs_model::apply(session, |s| s.write_binding(slot, &b));
            with_surfaces(session, |s| {
                read_binding_into(s, d, slot);
                read_status_into(s, d);
                if b.component > ty::DISPLAY || was > ty::DISPLAY {
                    read_aux_into(s, d, aux);
                }
                if b.component == ty::DISPLAY {
                    read_display_into(s, d);
                }
            })?;
            r.map_err(apply_error)?;
        }
        "name" => {
            let slot = index(op, d.bindings.len(), "binding slot")?;
            let name = text(op, "name")?;
            let session = w.session()?;
            let r = cs_model::apply(session, |s| s.write_name(slot, name));
            with_surfaces(session, |s| {
                read_binding_into(s, d, slot);
                // -- WP CG16 -- A name alone makes the device dirty too
                read_status_into(s, d);
            })?;
            r.map_err(apply_error)?;
        }
        "ir" => {
            let sub = index(op, d.ir.len(), "IR command")?;
            let c = ir_from_json(object(op, "command")?)?;
            let session = w.session()?;
            let r = cs_model::apply(session, |s| s.write_ir_command(sub, &c));
            with_surfaces(session, |s| {
                if let Ok(c) = s.read_ir_command(sub)
                    && let Some(x) = d.ir.get_mut(usize::from(sub))
                {
                    *x = c;
                }
                read_status_into(s, d);
            })?;
            r.map_err(apply_error)?;
        }
        "group" => {
            let g_index = index(op, d.groups.len(), "group")?;
            let g = group_from_json(object(op, "group")?)?;
            let session = w.session()?;
            let r = cs_model::apply(session, |s| s.write_group(g_index, &g));
            with_surfaces(session, |s| {
                if let Ok(g) = s.read_group(g_index)
                    && let Some(x) = d.groups.get_mut(usize::from(g_index))
                {
                    *x = g;
                }
                // Bindings using the group may have changed state.
                for slot in (0..=u8::MAX).take(d.bindings.len()) {
                    read_binding_into(s, d, slot);
                }
                read_status_into(s, d);
            })?;
            r.map_err(apply_error)?;
        }
        "macro" => {
            let m_index = index(op, d.macros.len(), "macro")?;
            let m = object(op, "macro")?;
            let max_steps = usize::from(d.caps.max_macro_steps.max(1));
            let mut steps = Vec::with_capacity(max_steps);
            match m.get("steps") {
                None | Some(Value::Null) => {}
                Some(Value::Array(a)) => {
                    for s in a.iter().take(max_steps) {
                        steps.push(step_from_json(s)?);
                    }
                }
                Some(_) => return Err(invalid("steps must be an array")),
            }
            let header = CsMacroHeaderWire {
                name: text(m, "name")?.to_owned(),
                // At most `max_steps`, itself a u8: never saturates.
                step_count: u8::try_from(steps.len()).unwrap_or(u8::MAX),
            };
            // Every step slot is written (an all-zero step clears it), then
            // the header; the first refusal is the result. Several SETs: one
            // flow under the DSPi Link device lock.
            steps.resize(max_steps, CsMacroStep::default());
            // The session first: nothing can fail between lock and unlock
            // but the write, whose result is kept.
            // -- WP CG24 -- Inside a held device lock it is nested: the
            // hub is not asked, and its unlock would end the holder's lock.
            let held = w.conn().held_lock.is_some();
            let session = w.session()?;
            if !held {
                session
                    .transport_mut()
                    .lock("control-surface macro", LOCK_TIMEOUT)?;
            }
            let r = cs_model::apply(session, |s| s.write_macro(m_index, &header, &steps));
            if !held {
                session.transport_mut().unlock();
            }
            with_surfaces(session, |s| {
                if let Ok(mc) = s.read_macro(m_index)
                    && let Some(x) = d.macros.get_mut(usize::from(m_index))
                {
                    *x = mc;
                }
                read_status_into(s, d);
            })?;
            r.map_err(apply_error)?;
        }
        "displayConfig" => {
            let c = display_cfg_from_json(object(op, "config")?)?;
            let session = w.session()?;
            let r = cs_model::apply(session, |s| s.write_display_cfg(&c));
            // -- WP CG28 -- The status too: a display write makes the device dirty
            with_surfaces(session, |s| {
                read_display_into(s, d);
                read_status_into(s, d);
            })?;
            r.map_err(apply_error)?;
        }
        "displayPage" => {
            let p_index = index(op, usize::from(d.max_pages), "display page")?;
            let p = page_from_json(object(op, "page")?)?;
            let session = w.session()?;
            let r = cs_model::apply(session, |s| s.write_display_page(p_index, &p));
            // -- WP CG28 -- The status too: a display write makes the device dirty
            with_surfaces(session, |s| {
                read_display_into(s, d);
                read_status_into(s, d);
            })?;
            r.map_err(apply_error)?;
        }
        "learnArm" => {
            *learn = None;
            let session = w.session()?;
            // The firmware answers 1 when armed and stalls on a refusal
            // (vendor_commands.c, REQ_CS_IR_LEARN).
            return match with_surfaces(session, |s| s.arm_learn())? {
                Ok(true) => Ok(json!({ "ok": true })),
                Ok(false) => Err(DspiError::Refused {
                    status: cs_status::BUSY,
                }),
                // A refusal stalls and leaves the reason in the status: only
                // the device's own NO_IR means there is no receiver.
                Err(e) if is_stall(&e) => {
                    let st = with_surfaces(session, |s| s.read_status())?.ok();
                    let no_ir = st.as_ref().map(|p| p.last_status) == Some(STATUS_NO_IR);
                    if let Some(st) = st {
                        d.status = st;
                    }
                    Err(DspiError::Refused {
                        status: if no_ir { STATUS_NO_IR } else { cs_status::BUSY },
                    })
                }
                Err(e) => Err(surface_error(e)),
            };
        }
        "learnCancel" => {
            // Best effort, as the old core: a cancel the device refuses
            // leaves nothing armed to cancel.
            let _ = with_surfaces(w.session()?, |s| s.cancel_learn())?;
            return Ok(json!({ "ok": true }));
        }
        "macroFire" | "macroCancel" => {
            let target = if kind == "macroFire" {
                Some(index(op, d.macros.len(), "macro")?)
            } else {
                None
            };
            let session = w.session()?;
            let r = with_surfaces(session, |s| match target {
                Some(i) => s.fire_macro(i),
                None => s.cancel_macro(),
            })?;
            with_surfaces(session, |s| read_status_into(s, d))?;
            match r {
                Ok(_) => {}
                // A refusal stalls and leaves the reason in the status.
                Err(e) if is_stall(&e) => {
                    let st = match d.status.last_status {
                        cs_status::SUCCESS => cs_status::BUSY,
                        st => st,
                    };
                    return Err(DspiError::Refused { status: st });
                }
                Err(e) => return Err(surface_error(e)),
            }
        }
        "auxState" => {
            let slot = index(op, CsAuxStates::SLOTS, "aux slot")?;
            let on = num::<i64>(op, "on", 0)? != 0;
            let session = w.session()?;
            cs_model::apply_code(session, |s| s.set_aux_state(slot, on)).map_err(apply_error)?;
            confirm_aux(session, aux, |a| a.is_on(usize::from(slot)) == on)?;
        }
        "auxLevel" => {
            let slot = index(op, CsAuxStates::SLOTS, "aux slot")?;
            let percent = match op.get("percent") {
                None | Some(Value::Null) => 0.0,
                Some(v) => v
                    .as_f64()
                    .ok_or_else(|| invalid("percent must be a number"))?,
            };
            // Clamped to 0..=100 first, so the 8.8 value fits a u16.
            let q8 = ((percent.clamp(0.0, 100.0) * 256.0).round() as u16).min(AUX_LEVEL_MAX_Q8);
            let session = w.session()?;
            cs_model::apply_code(session, |s| s.set_aux_level(slot, q8)).map_err(apply_error)?;
            confirm_aux(session, aux, |a| {
                a.level_q8.get(usize::from(slot)).copied() == Some(q8)
            })?;
        }
        other => return Err(invalid(format!("unknown operation {other}"))),
    }
    Ok(applied())
}

/// `save` (flash) or `revert` (reload flash), then a full re-read: saving
/// folds live aux values into their power-on fields, reverting reloads
/// everything.
fn save_or_revert(inner: &mut Inner, cs: &mut CsState, save: bool) -> Result<Value> {
    let st = {
        let mut w = inner.writable_conn()?;
        let session = w.session()?;
        let r = cs_model::apply(session, |s| if save { s.save() } else { s.revert() });
        match r {
            Ok(()) => cs_status::SUCCESS,
            Err(ApplyError::Rejected { status }) => status,
            Err(ApplyError::Surfaces(SurfaceError::Timeout { .. })) => cs_status::PENDING,
            // A stall means busy (another save or revert is queued): the
            // reason is in the status.
            Err(ApplyError::Surfaces(e)) if is_stall(&e) => {
                with_surfaces(session, |s| s.read_status())?
                    .ok()
                    .map(|p| p.last_status)
                    .filter(|&st| st != cs_status::SUCCESS)
                    .unwrap_or(cs_status::BUSY)
            }
            Err(e) => return Err(apply_error(e)),
        }
    };
    fetch_into(inner, cs)?;
    if st == cs_status::SUCCESS {
        cs.mark_clean();
    }
    settled(st)
}

// -- WP CG16 --
#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// On the demo device only: hold it in a running revert, or let it finish.
    /// While held it gives the refusals a real device has no steady way to show:
    /// a write settles busy, and the save, the revert, a macro fire and a learn
    /// are refused. As [`core_vreg_demo_event`](Self::core_vreg_demo_event), it
    /// answers [`DspiError::Unsupported`] on any other device.
    pub fn cs_demo_busy(&self, on: bool) -> Result<()> {
        let mut inner = self.lock();
        let mut w = inner.writable_conn()?;
        if w.conn().caps.kind != dspi_transport::TransportKind::Fake {
            return Err(DspiError::Unsupported);
        }
        w.conn()
            .transport()
            .control_out(
                dspi_proto::generated::opcodes::REQ_EXTENSION,
                dspi_transport::fake::EXT_FAKE_CS_SIM,
                &[u8::from(on)],
            )
            .map_err(DspiError::from)
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_c14 {
    use std::sync::{Arc, Mutex, MutexGuard};

    use dspi_proto::generated::opcodes as op;
    use dspi_proto::packets::{CsCapsHeader, CsDisplayCfgReply};
    use dspi_proto::wire_compat::WireRead;
    use dspi_session::cs_model::{act, demo, noun};
    use dspi_session::surfaces;
    use dspi_transport::{
        DeviceDescriptor, FakeDevice, NotificationSource, Transport, TransportCaps,
    };
    use proptest::prelude::*;

    use super::*;

    /// `PIN_CONFIG_INVALID_PIN`: the sim refuses a binding on [`Sim::BAD_PIN`]
    /// with it.
    const INVALID_PIN: u8 = 0x01;

    /// The firmware's control-surface side, enough of it for the editor:
    /// every SET applies at once (no PENDING), tagged as the firmware tags
    /// it; `save` and `revert` copy between live and flash.
    struct Sim {
        caps: dspi_session::probe::ControlSurfaceCaps,
        live: CsData,
        flash: CsData,
        aux: CsAuxStates,
        learn: IrLearnResult,
        /// No live IR receiver: arming stalls and reports `NO_IR`.
        no_ir: bool,
        /// Arming stalls without touching the status (some other refusal).
        arm_stalls: bool,
        /// What a successful arm answers.
        arm_reply: u8,
        /// State-changing requests the device received (OUT transfers and
        /// write-as-read opcodes).
        sets: usize,
        /// The OUT transfers, in order: (opcode, wValue).
        outs: Vec<(u8, u16)>,
    }

    impl Sim {
        const BAD_PIN: u8 = 99;

        fn new() -> Self {
            let mut live = demo::data();
            // Every macro slot reads back with all its step slots.
            for m in &mut live.macros {
                m.steps.resize(8, CsMacroStep::default());
            }
            let mut aux = CsAuxStates::default();
            aux.state[10] = 1;
            aux.state[11] = 1;
            aux.level_q8[11] = 50 * 256;
            Self {
                caps: demo::caps(),
                flash: live.clone(),
                live,
                aux,
                learn: IrLearnResult {
                    state: 2,
                    protocol: 1,
                    code: 0x20DF_10EF,
                },
                no_ir: false,
                arm_stalls: false,
                arm_reply: 1,
                sets: 0,
                outs: Vec::new(),
            }
        }

        fn settle(&mut self, tag: u8, status: u8) {
            self.live.status.last_slot = tag;
            self.live.status.last_status = status;
            if status == 0 {
                self.live.status.dirty = true;
            }
        }

        fn stall(opcode: u8) -> dspi_transport::Result<Vec<u8>> {
            Err(TransportError::Stalled { opcode })
        }

        fn get(&mut self, opcode: u8, value: u16) -> Option<dspi_transport::Result<Vec<u8>>> {
            let v = usize::from(value);
            let d = &self.live;
            Some(Ok(match opcode {
                op::REQ_GET_CS_CAPS if value == 0xFFFF => CsCapsHeader {
                    caps_version: self.caps.caps_version,
                    max_bindings: self.caps.max_bindings,
                    type_count: self.caps.type_count,
                    noun_count: self.caps.noun_count,
                    types: self.caps.types.clone(),
                    max_ir_commands: self.caps.max_ir_commands,
                    max_groups: self.caps.max_groups,
                    max_macros: self.caps.max_macros,
                    max_macro_steps: self.caps.max_macro_steps,
                }
                .encode(),
                op::REQ_GET_CS_CAPS => match d.nouns.get(v) {
                    Some(n) => n.encode().to_vec(),
                    None => return Some(Self::stall(opcode)),
                },
                op::REQ_GET_CS_BINDING => d.bindings[v].encode().to_vec(),
                op::REQ_GET_CS_NAME => {
                    let mut b = d.names[v].as_bytes().to_vec();
                    b.resize(32, 0);
                    b
                }
                op::REQ_GET_CS_IR_CMD => d.ir[v].encode().to_vec(),
                op::REQ_GET_CS_GROUP => d.groups[v].encode().to_vec(),
                op::REQ_GET_CS_MACRO => {
                    let m = &d.macros[v];
                    let mut b = m.header().encode().to_vec();
                    for s in &m.steps {
                        b.extend(s.encode());
                    }
                    b
                }
                op::REQ_GET_CS_DISPLAY_CFG => CsDisplayCfgReply {
                    max_pages: d.max_pages,
                    model_count: d.model_count,
                    cfg: d.display_cfg.clone(),
                }
                .encode()
                .to_vec(),
                op::REQ_GET_CS_DISPLAY_PAGE => d.pages[v].encode().to_vec(),
                op::REQ_GET_CS_DISPLAY_STATUS => d.display_status.encode().to_vec(),
                op::REQ_GET_CS_STATUS => d.status.encode(),
                op::REQ_GET_CS_EXT_STATUS => d.ext.encode().to_vec(),
                op::REQ_GET_CS_AUX_STATE => self.aux.encode().to_vec(),
                op::REQ_CS_SAVE | op::REQ_CS_REVERT => {
                    self.sets += 1;
                    if opcode == op::REQ_CS_SAVE {
                        self.flash = self.live.clone();
                    } else {
                        let status = self.live.status.clone();
                        self.live = self.flash.clone();
                        self.live.status = status;
                    }
                    self.live.status.dirty = false;
                    self.settle(surfaces::tag::SAVE, 0);
                    self.live.status.dirty = false;
                    vec![1]
                }
                op::REQ_CS_IR_LEARN => match value {
                    2 => self.learn.encode().to_vec(),
                    _ => {
                        self.sets += 1;
                        if value == 1 && self.no_ir {
                            self.settle(0xFF, STATUS_NO_IR);
                            return Some(Self::stall(opcode));
                        }
                        if value == 1 && self.arm_stalls {
                            return Some(Self::stall(opcode));
                        }
                        vec![if value == 1 { self.arm_reply } else { 1 }]
                    }
                },
                op::REQ_CS_MACRO_FIRE => {
                    self.sets += 1;
                    if value != 0xFFFF && d.macros[v].step_count == 0 {
                        // CS_STATUS_MACRO_INVALID
                        self.settle(0x60 | value as u8, 0x20);
                        return Some(Self::stall(opcode));
                    }
                    vec![1]
                }
                _ => return None,
            }))
        }

        fn set(
            &mut self,
            opcode: u8,
            value: u16,
            data: &[u8],
        ) -> Option<dspi_transport::Result<()>> {
            let v = usize::from(value);
            let slot = value as u8;
            match opcode {
                op::REQ_SET_CS_BINDING => {
                    let b = CsBinding::decode(data).unwrap();
                    if b.gpio[0] == Self::BAD_PIN {
                        self.settle(slot, INVALID_PIN);
                    } else {
                        let bit = 1u16 << slot;
                        if b.is_empty() {
                            self.live.status.active_mask &= !bit;
                        } else {
                            self.live.status.active_mask |= bit;
                        }
                        self.live.bindings[v] = b;
                        self.settle(slot, 0);
                    }
                }
                op::REQ_SET_CS_NAME => {
                    let end = data.iter().position(|&c| c == 0).unwrap_or(data.len());
                    self.live.names[v] = String::from_utf8_lossy(&data[..end]).into_owned();
                    self.settle(slot, 0);
                }
                op::REQ_SET_CS_IR_CMD => {
                    self.live.ir[v] = IrCommand::decode(data).unwrap();
                    self.settle(surfaces::tag::IR | slot, 0);
                }
                op::REQ_SET_CS_GROUP => {
                    self.live.groups[v] = CsGroup::decode(data).unwrap();
                    self.settle(surfaces::tag::GROUP | slot, 0);
                }
                op::REQ_SET_CS_MACRO_STEP => {
                    let (m, s) = (v & 0xFF, v >> 8);
                    self.live.macros[m].steps[s] = CsMacroStep::decode(data).unwrap();
                    self.settle(surfaces::tag::MACRO | m as u8, 0);
                }
                op::REQ_SET_CS_MACRO => {
                    let h = CsMacroHeaderWire::decode(data).unwrap();
                    let m = &mut self.live.macros[v];
                    m.name = h.name;
                    m.step_count = h.step_count;
                    self.settle(surfaces::tag::MACRO | slot, 0);
                }
                op::REQ_SET_CS_DISPLAY_CFG => {
                    self.live.display_cfg = CsDisplayCfg::decode(data).unwrap();
                    self.settle(surfaces::tag::DISPLAY, 0);
                }
                op::REQ_SET_CS_DISPLAY_PAGE => {
                    self.live.pages[v] = CsDisplayPage::decode(data).unwrap();
                    self.settle(surfaces::tag::DISPLAY | slot, 0);
                }
                op::REQ_SET_CS_AUX_STATE | op::REQ_SET_CS_AUX_LEVEL => {
                    let kind = self.live.bindings[v].component;
                    let ok = if opcode == op::REQ_SET_CS_AUX_STATE {
                        kind == ty::AUX_OUT || kind == ty::AUX_PWM
                    } else {
                        kind == ty::AUX_PWM
                    };
                    if !ok {
                        // Recorded in the status only; the transfer is ACKed.
                        self.live.status.last_slot = slot;
                        self.live.status.last_status = surfaces::status::INVALID_AUX;
                    } else if opcode == op::REQ_SET_CS_AUX_STATE {
                        self.aux.state[v] = data[0];
                    } else {
                        self.aux.level_q8[v] = u16::from_le_bytes([data[0], data[1]]);
                    }
                }
                _ => return None,
            }
            self.sets += 1;
            self.outs.push((opcode, value));
            Some(Ok(()))
        }
    }

    /// The demo device with the sim answering every control-surface request.
    struct CsDevice {
        fake: FakeDevice,
        sim: Arc<Mutex<Sim>>,
    }

    impl Transport for CsDevice {
        fn control_in(
            &mut self,
            opcode: u8,
            value: u16,
            len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            let r = self.sim.lock().unwrap().get(opcode, value);
            match r {
                Some(Ok(mut b)) => {
                    let want = usize::from(len);
                    if b.len() < want {
                        return Err(TransportError::ShortRead {
                            opcode,
                            wanted: want,
                            got: b.len(),
                        });
                    }
                    b.truncate(want);
                    Ok(b)
                }
                Some(Err(e)) => Err(e),
                None => self.fake.control_in(opcode, value, len),
            }
        }

        fn control_in_upto(
            &mut self,
            opcode: u8,
            value: u16,
            max_len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            let r = self.sim.lock().unwrap().get(opcode, value);
            match r {
                Some(r) => r.map(|mut b| {
                    b.truncate(usize::from(max_len));
                    b
                }),
                None => self.fake.control_in_upto(opcode, value, max_len),
            }
        }

        fn control_out(
            &mut self,
            opcode: u8,
            value: u16,
            data: &[u8],
        ) -> dspi_transport::Result<()> {
            let r = self.sim.lock().unwrap().set(opcode, value, data);
            r.unwrap_or_else(|| self.fake.control_out(opcode, value, data))
        }

        fn descriptor(&self) -> &DeviceDescriptor {
            self.fake.descriptor()
        }

        fn max_transfer(&self) -> usize {
            self.fake.max_transfer()
        }

        fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
            self.fake.notifications()
        }

        fn caps(&self) -> TransportCaps {
            self.fake.caps()
        }
    }

    /// A probed demo device with control surfaces, not yet fetched.
    fn cs_core_as(serial: &str) -> (Arc<DspiCore>, Arc<Mutex<Sim>>) {
        let sim = Arc::new(Mutex::new(Sim::new()));
        let dev = CsDevice {
            fake: FakeDevice::demo(),
            sim: Arc::clone(&sim),
        };
        let core = DspiCore::new();
        core.attach_probed(Box::new(dev), serial.into(), 0).unwrap();
        (core, sim)
    }

    /// [`cs_core_as`], fetched.
    fn cs_core() -> (Arc<DspiCore>, Arc<Mutex<Sim>>) {
        let (core, sim) = cs_core_as("DEMO");
        core.cs_fetch().unwrap();
        (core, sim)
    }

    fn sim(s: &Arc<Mutex<Sim>>) -> MutexGuard<'_, Sim> {
        s.lock().unwrap()
    }

    fn snap(core: &DspiCore) -> Value {
        serde_json::from_str(&core.cs_snapshot()).unwrap()
    }

    fn apply(core: &DspiCore, op: Value) -> Value {
        serde_json::from_str(&core.cs_apply(op.to_string())).unwrap()
    }

    fn make_read_only(core: &DspiCore) {
        core.lock().conn.as_mut().unwrap().wire = Some(WireRead::Older);
    }

    fn button(pin: u8) -> Value {
        json!({ "type": ty::BUTTON, "noun": noun::USER_MUTE, "action": act::TOGGLE,
                "gpio0": pin, "value": 1 })
    }

    #[test]
    fn wp_c14_nothing_before_a_fetch() {
        let core = DspiCore::new();
        assert_eq!(snap(&core), json!({ "supported": false }));
        assert_eq!(core.cs_fetch(), Err(DspiError::Disconnected));
        assert!(core.cs_pin_uses().is_empty());
        let r = apply(&core, json!({ "op": "refreshStatus" }));
        assert_eq!(r["ok"], false);
        assert!(r["error"].as_str().unwrap().contains("not connected"));

        let (core, _) = cs_core_as("DEMO");
        assert_eq!(snap(&core), json!({ "supported": false }));
        assert!(core.cs_pin_uses().is_empty());
    }

    #[test]
    fn wp_c14_the_snapshot_has_the_old_cores_shape() {
        let (core, _) = cs_core();
        let s = snap(&core);
        let mut keys: Vec<&str> = s.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "auxLevel",
                "auxState",
                "bindings",
                "capsVersion",
                "display",
                "ext",
                "groups",
                "ir",
                "learn",
                "macros",
                "maxBindings",
                "maxGroups",
                "maxIr",
                "maxMacros",
                "maxSteps",
                "names",
                "nouns",
                "status",
                "supported",
                "types",
            ]
        );
        assert_eq!(s["supported"], true);
        assert_eq!(s["capsVersion"], 20);
        assert_eq!(s["maxBindings"], 16);
        assert_eq!(
            (s["maxIr"].clone(), s["maxGroups"].clone()),
            (json!(16), json!(8))
        );
        assert_eq!(
            (s["maxMacros"].clone(), s["maxSteps"].clone()),
            (json!(8), json!(8))
        );
        assert_eq!(s["types"].as_array().unwrap().len(), 11);
        assert_eq!(s["types"][3]["pinClass"], 1);
        assert_eq!(s["nouns"].as_array().unwrap().len(), 79);
        assert_eq!(s["nouns"][0]["min"], -60 * 256);
        assert_eq!(s["nouns"][0]["deferred"], false);
        // -- WP CG20 -- The names, for the clients' labels.
        assert_eq!(s["nouns"][0]["name"], "Volume");
        let clip = usize::from(noun::CLIP);
        assert_eq!(s["nouns"][clip]["name"], "Clipping");
        assert_eq!(s["nouns"][clip]["controlName"], "Clear Clipping");
        let mac = usize::from(noun::MACRO);
        assert_eq!(s["nouns"][mac]["name"], "Running Macro");
        assert_eq!(s["nouns"][mac]["controlName"], "Macro");
        let b0 = &s["bindings"][0];
        assert_eq!(b0["type"], ty::ENCODER);
        assert_eq!(
            (b0["gpio0"].clone(), b0["gpio1"].clone()),
            (json!(10), json!(11))
        );
        assert_eq!(s["bindings"][1]["gpio1"], 255);
        assert_eq!(s["bindings"].as_array().unwrap().len(), 16);
        assert_eq!(s["names"].as_array().unwrap().len(), 16);
        assert_eq!(s["names"][0], "Volume Knob");
        assert_eq!(s["ir"][0]["code"], 0x20DF_40BF_u32);
        assert_eq!(s["groups"][0]["name"], "Front Pair");
        assert_eq!(s["groups"][0]["memberMask"], 3);
        assert_eq!(s["macros"][0]["name"], "Night");
        assert_eq!(s["macros"][0]["steps"].as_array().unwrap().len(), 2);
        assert_eq!(s["macros"][0]["steps"][1]["preDelay"], 50);
        assert_eq!(s["macros"][1]["steps"], json!([]));
        assert_eq!(s["ext"]["macroRunning"], 255);
        assert_eq!(s["ext"]["groupStatus"].as_array().unwrap().len(), 8);
        let st = &s["status"];
        assert_eq!(st["dirty"], false);
        assert_eq!(st["activeMask"], 0b0001_1100_0001_1111);
        assert_eq!(st["slotStatus"].as_array().unwrap().len(), 16);
        assert_eq!(st["irStatus"].as_array().unwrap().len(), 16);
        let d = &s["display"];
        assert_eq!(
            (d["maxPages"].clone(), d["modelCount"].clone()),
            (json!(16), json!(9))
        );
        assert_eq!(d["config"]["dwell"], 30);
        assert_eq!(d["pages"].as_array().unwrap().len(), 16);
        assert_eq!(d["pages"][0]["noun"], noun::USER_VOLUME);
        assert_eq!(d["status"]["model"], 6);
        assert_eq!(s["auxState"].as_array().unwrap().len(), 16);
        assert_eq!(s["auxState"][10], 1);
        assert_eq!(s["auxLevel"][11], 50 * 256);
        assert_eq!(s["learn"], Value::Null);
    }

    #[test]
    fn wp_c14_a_binding_applies_and_is_read_back() {
        let (core, sim_) = cs_core();
        let r = apply(
            &core,
            json!({ "op": "binding", "index": 5, "binding": button(20) }),
        );
        assert_eq!(r, json!({ "ok": true, "status": 0 }));
        assert_eq!(sim(&sim_).live.bindings[5].gpio, [20, GPIO_UNUSED]);
        let s = snap(&core);
        assert_eq!(s["bindings"][5]["gpio0"], 20);
        assert_eq!(s["bindings"][5]["type"], ty::BUTTON);
        assert_eq!(s["status"]["dirty"], true);
        assert_eq!(s["status"]["lastSlot"], 5);
        assert!(core.cs_pin_uses().contains(&PinUse { pin: 20, slot: 5 }));

        let r = apply(&core, json!({ "op": "name", "index": 5, "name": "Mute" }));
        assert_eq!(r["ok"], true);
        assert_eq!(snap(&core)["names"][5], "Mute");
    }

    #[test]
    fn wp_c14_a_refusal_is_the_devices_status_code() {
        let (core, sim_) = cs_core();
        let r = apply(
            &core,
            json!({ "op": "binding", "index": 5, "binding": button(Sim::BAD_PIN) }),
        );
        assert_eq!(r, json!({ "ok": false, "status": INVALID_PIN }));
        // Read back: the slot is still empty, on the device and in the JSON.
        assert!(sim(&sim_).live.bindings[5].is_empty());
        core.cs_fetch().unwrap();
        assert_eq!(snap(&core)["bindings"][5]["type"], 0);
    }

    #[test]
    fn wp_c14_errors_are_typed() {
        assert_eq!(
            apply_error(ApplyError::Rejected { status: 0x13 }),
            DspiError::Refused { status: 0x13 }
        );
        assert_eq!(
            apply_error(ApplyError::Surfaces(SurfaceError::Timeout {
                tag: "binding slot 1".into(),
                polls: 25
            })),
            DspiError::Refused {
                status: cs_status::PENDING
            }
        );
        assert_eq!(
            apply_error(ApplyError::Transport(TransportError::Disconnected)),
            DspiError::Disconnected
        );
        assert_eq!(
            surface_error(SurfaceError::Transport(TransportError::Locked)),
            DspiError::Locked
        );
    }

    #[test]
    fn wp_c14_a_read_only_connection_sends_nothing() {
        let (core, sim_) = cs_core();
        make_read_only(&core);
        let before = sim(&sim_).live.clone();
        for op in [
            json!({ "op": "binding", "index": 5, "binding": button(20) }),
            json!({ "op": "name", "index": 0, "name": "x" }),
            json!({ "op": "ir", "index": 1, "command": { "protocol": 1, "code": 5 } }),
            json!({ "op": "group", "index": 1, "group": { "targetKind": 2, "memberMask": 1 } }),
            json!({ "op": "macro", "index": 1, "macro": { "name": "x", "steps": [] } }),
            json!({ "op": "displayConfig", "config": { "mode": 1 } }),
            json!({ "op": "displayPage", "index": 2, "page": { "flags": 1 } }),
            json!({ "op": "save" }),
            json!({ "op": "revert" }),
            json!({ "op": "learnArm" }),
            json!({ "op": "learnCancel" }),
            json!({ "op": "macroFire", "index": 0 }),
            json!({ "op": "macroCancel" }),
            json!({ "op": "auxState", "index": 10, "on": 0 }),
            json!({ "op": "auxLevel", "index": 11, "percent": 10 }),
        ] {
            let r = apply(&core, op.clone());
            assert_eq!(r["ok"], false, "{op}");
            assert!(
                r["error"].as_str().unwrap().contains("read-only"),
                "{op}: {r}"
            );
        }
        // Proved by read-back, not by the replies: the device saw no change.
        let s = sim(&sim_);
        assert_eq!(s.sets, 0);
        assert_eq!(s.live, before);
        drop(s);
        // Reads still work.
        assert_eq!(apply(&core, json!({ "op": "refreshStatus" }))["ok"], true);
        assert_eq!(
            apply(&core, json!({ "op": "learnRead" }))["code"],
            0x20DF_10EF_u32
        );
        core.cs_fetch().unwrap();
        assert_eq!(snap(&core)["bindings"][5]["type"], 0);
    }

    #[test]
    fn wp_c14_malformed_operations_are_refused_before_any_write() {
        let (core, sim_) = cs_core();
        for (text, why) in [
            ("{", "malformed"),
            ("[1]", "JSON object"),
            ("{\"op\":\"nonsense\"}", "unknown operation nonsense"),
            ("{\"op\":5}", "op must be a string"),
            ("{\"op\":\"binding\",\"index\":16}", "binding slot 16"),
            ("{\"op\":\"binding\",\"index\":-1}", "binding slot -1"),
            (
                "{\"op\":\"binding\",\"index\":\"0\"}",
                "index must be a number",
            ),
            (
                "{\"op\":\"binding\",\"binding\":{\"noun\":300}}",
                "noun 300",
            ),
            (
                "{\"op\":\"binding\",\"binding\":{\"value\":40000}}",
                "value 40000",
            ),
            (
                "{\"op\":\"binding\",\"binding\":{\"gpio0\":1e300}}",
                "gpio0 is out of range",
            ),
            (
                "{\"op\":\"binding\",\"binding\":[]}",
                "binding must be an object",
            ),
            (
                "{\"op\":\"ir\",\"index\":3,\"command\":{\"code\":-1}}",
                "code -1",
            ),
            ("{\"op\":\"group\",\"index\":8}", "group 8"),
            (
                "{\"op\":\"macro\",\"index\":0,\"macro\":{\"steps\":{}}}",
                "steps must be an array",
            ),
            (
                "{\"op\":\"macro\",\"index\":0,\"macro\":{\"steps\":[{\"preDelay\":70000}]}}",
                "preDelay",
            ),
            (
                "{\"op\":\"name\",\"index\":0,\"name\":7}",
                "name must be a string",
            ),
            ("{\"op\":\"displayPage\",\"index\":16}", "display page 16"),
            (
                "{\"op\":\"auxLevel\",\"index\":11,\"percent\":\"x\"}",
                "percent",
            ),
            ("{\"op\":\"auxState\",\"index\":16}", "aux slot 16"),
        ] {
            let r: Value = serde_json::from_str(&core.cs_apply(text.into())).unwrap();
            assert_eq!(r["ok"], false, "{text}");
            let e = r["error"].as_str().unwrap();
            assert!(e.contains(why), "{text}: {e}");
        }
        assert_eq!(sim(&sim_).sets, 0);
        // Floats round and bools count, as the old core read them.
        let r = apply(
            &core,
            json!({ "op": "binding", "index": 6.0,
                    "binding": { "type": 1, "noun": 2, "action": 4, "gpio0": 21.4, "value": true } }),
        );
        assert_eq!(r["ok"], true);
        let b = sim(&sim_).live.bindings[6].clone();
        assert_eq!((b.gpio, b.value), ([21, GPIO_UNUSED], 1));
    }

    #[test]
    fn wp_c14_save_is_clean_and_revert_restores() {
        let (core, sim_) = cs_core();
        assert_eq!(
            apply(
                &core,
                json!({ "op": "binding", "index": 5, "binding": button(20) })
            )["ok"],
            true
        );
        assert_eq!(snap(&core)["status"]["dirty"], true);
        assert_eq!(
            apply(&core, json!({ "op": "save" })),
            json!({ "ok": true, "status": 0 })
        );
        assert_eq!(snap(&core)["status"]["dirty"], false);
        assert_eq!(sim(&sim_).flash.bindings[5].gpio[0], 20);

        assert_eq!(
            apply(
                &core,
                json!({ "op": "binding", "index": 5, "binding": button(21) })
            )["ok"],
            true
        );
        assert_eq!(snap(&core)["bindings"][5]["gpio0"], 21);
        assert_eq!(apply(&core, json!({ "op": "revert" }))["ok"], true);
        let s = snap(&core);
        assert_eq!(s["bindings"][5]["gpio0"], 20);
        assert_eq!(s["status"]["dirty"], false);
    }

    #[test]
    fn wp_c14_an_undone_edit_is_not_unsaved() {
        let (core, sim_) = cs_core();
        apply(
            &core,
            json!({ "op": "binding", "index": 5, "binding": button(20) }),
        );
        assert_eq!(snap(&core)["status"]["dirty"], true);
        // Removed again: the device still says dirty, but nothing differs
        // from the saved configuration (an emptied slot's leftover pins
        // do not count).
        let r = apply(
            &core,
            json!({ "op": "binding", "index": 5, "binding": { "type": 0, "gpio0": 20 } }),
        );
        assert_eq!(r["ok"], true);
        assert!(sim(&sim_).live.status.dirty);
        assert_eq!(snap(&core)["status"]["dirty"], false);
    }

    #[test]
    fn wp_c14_a_macro_writes_every_step_then_the_header() {
        let (core, sim_) = cs_core();
        sim(&sim_).outs.clear();
        let r = apply(
            &core,
            json!({ "op": "macro", "index": 1, "macro": { "name": "Film", "steps": [
                { "noun": noun::INPUT_SOURCE, "action": act::SET, "value": 2 },
                { "noun": noun::PRESET, "action": act::SET, "value": 1, "preDelay": 30 },
            ] } }),
        );
        assert_eq!(r, json!({ "ok": true, "status": 0 }));
        let s = sim(&sim_);
        let expect: Vec<(u8, u16)> = (0..8u16)
            .map(|i| (op::REQ_SET_CS_MACRO_STEP, (i << 8) | 1))
            .chain([(op::REQ_SET_CS_MACRO, 1)])
            .collect();
        assert_eq!(s.outs, expect);
        assert_eq!(
            (s.live.macros[1].name.as_str(), s.live.macros[1].step_count),
            ("Film", 2)
        );
        drop(s);
        let m = &snap(&core)["macros"][1];
        assert_eq!(m["name"], "Film");
        assert_eq!(m["steps"].as_array().unwrap().len(), 2);
        assert_eq!(m["steps"][1]["preDelay"], 30);

        // Firing an empty macro is refused with the status the device left.
        let r = apply(&core, json!({ "op": "macroFire", "index": 2 }));
        assert_eq!(r, json!({ "ok": false, "status": 0x20 }));
        assert_eq!(
            apply(&core, json!({ "op": "macroFire", "index": 1 }))["ok"],
            true
        );
        assert_eq!(apply(&core, json!({ "op": "macroCancel" }))["ok"], true);
    }

    #[test]
    fn wp_c14_ir_group_and_display_writes_read_back() {
        let (core, sim_) = cs_core();
        let r = apply(
            &core,
            json!({ "op": "ir", "index": 2, "command":
                    { "noun": noun::USER_MUTE, "action": act::TOGGLE, "protocol": 1, "code": 0xDEAD_BEEF_u32 } }),
        );
        assert_eq!(r["ok"], true);
        let r = apply(
            &core,
            json!({ "op": "group", "index": 3, "group": { "targetKind": 2, "memberMask": 12, "name": "Rears" } }),
        );
        assert_eq!(r["ok"], true);
        let r = apply(
            &core,
            json!({ "op": "displayConfig", "config": { "mode": 1, "dwell": 40, "brightness": 9 } }),
        );
        assert_eq!(r["ok"], true);
        let r = apply(
            &core,
            json!({ "op": "displayPage", "index": 2, "page": { "noun": noun::PRESET, "flags": 1 } }),
        );
        assert_eq!(r["ok"], true);
        let s = snap(&core);
        assert_eq!(s["ir"][2]["code"], 0xDEAD_BEEF_u32);
        assert_eq!(s["groups"][3]["name"], "Rears");
        assert_eq!(s["display"]["config"]["dwell"], 40);
        assert_eq!(s["display"]["pages"][2]["noun"], noun::PRESET);
        let live = &sim(&sim_).live;
        assert_eq!(live.ir[2].code, 0xDEAD_BEEF);
        assert_eq!(live.groups[3].member_mask, 12);
        assert_eq!(live.display_cfg.brightness, 9);
    }

    // -- WP CG28 --
    #[test]
    fn wp_cg28_display_writes_set_dirty_at_once() {
        for op in [
            json!({ "op": "displayConfig", "config": { "mode": 1, "dwell": 40, "brightness": 9 } }),
            json!({ "op": "displayPage", "index": 2, "page": { "noun": noun::PRESET, "flags": 1 } }),
        ] {
            let (core, sim_) = cs_core();
            assert_eq!(snap(&core)["status"]["dirty"], false);
            assert_eq!(apply(&core, op)["ok"], true);
            assert!(sim(&sim_).live.status.dirty);
            // No refresh or refreshStatus: the write itself re-read the status.
            assert_eq!(snap(&core)["status"]["dirty"], true);
        }
    }

    #[test]
    fn wp_c14_pins_are_held_by_live_bindings_only() {
        let (core, _) = cs_core();
        let pins: Vec<(u8, u8)> = core.cs_pin_uses().iter().map(|p| (p.pin, p.slot)).collect();
        assert_eq!(
            pins,
            [
                (10, 0),
                (11, 0),
                (16, 1),
                (26, 2),
                (15, 3),
                (2, 4),
                (3, 4),
                (27, 10),
                (28, 11),
                (19, 12)
            ]
        );
        // Cleared: the slot's pins are free again.
        apply(
            &core,
            json!({ "op": "binding", "index": 1, "binding": { "type": 0 } }),
        );
        assert!(!core.cs_pin_uses().iter().any(|p| p.slot == 1));
    }

    #[test]
    fn wp_c14_an_arm_refusal_reads_the_reason_from_the_status() {
        let (core, sim_) = cs_core();
        // The firmware's acknowledgement is 1; any other byte is not it.
        sim(&sim_).arm_reply = 1;
        assert_eq!(
            apply(&core, json!({ "op": "learnArm" })),
            json!({ "ok": true })
        );
        for reply in [0, 7] {
            sim(&sim_).arm_reply = reply;
            assert_eq!(
                apply(&core, json!({ "op": "learnArm" })),
                json!({ "ok": false, "status": cs_status::BUSY }),
                "reply {reply}"
            );
        }
        // A stall whose status does not say NO_IR is not "no receiver".
        sim(&sim_).arm_stalls = true;
        assert_eq!(
            apply(&core, json!({ "op": "learnArm" })),
            json!({ "ok": false, "status": cs_status::BUSY })
        );
        // The device's own NO_IR is, and the snapshot holds the fresh status.
        sim(&sim_).no_ir = true;
        assert_eq!(
            apply(&core, json!({ "op": "learnArm" })),
            json!({ "ok": false, "status": STATUS_NO_IR })
        );
        assert_eq!(snap(&core)["status"]["lastStatus"], STATUS_NO_IR);
    }

    /// A transport that counts the Link lock around a real device.
    struct Locked {
        dev: CsDevice,
        locks: Arc<Mutex<Vec<&'static str>>>,
    }

    impl Transport for Locked {
        fn control_in(
            &mut self,
            opcode: u8,
            value: u16,
            len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            self.dev.control_in(opcode, value, len)
        }

        fn control_in_upto(
            &mut self,
            opcode: u8,
            value: u16,
            max_len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            self.dev.control_in_upto(opcode, value, max_len)
        }

        fn control_out(
            &mut self,
            opcode: u8,
            value: u16,
            data: &[u8],
        ) -> dspi_transport::Result<()> {
            if opcode == op::REQ_SET_CS_MACRO {
                self.locks.lock().unwrap().push("header");
            }
            self.dev.control_out(opcode, value, data)
        }

        fn descriptor(&self) -> &DeviceDescriptor {
            self.dev.descriptor()
        }

        fn lock(&mut self, _reason: &str, _timeout: Duration) -> dspi_transport::Result<()> {
            self.locks.lock().unwrap().push("lock");
            Ok(())
        }

        fn unlock(&mut self) {
            self.locks.lock().unwrap().push("unlock");
        }
    }

    #[test]
    fn wp_c14_a_macro_write_runs_under_the_link_lock() {
        let locks = Arc::new(Mutex::new(Vec::new()));
        let dev = Locked {
            dev: CsDevice {
                fake: FakeDevice::demo(),
                sim: Arc::new(Mutex::new(Sim::new())),
            },
            locks: Arc::clone(&locks),
        };
        let core = DspiCore::new();
        core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
        let r = apply(
            &core,
            json!({ "op": "macro", "index": 1, "macro": { "name": "x", "steps": [] } }),
        );
        assert_eq!(r["ok"], true);
        assert_eq!(*locks.lock().unwrap(), ["lock", "header", "unlock"]);
    }

    #[test]
    fn wp_c14_a_reopened_device_is_read_again() {
        let sim_ = Arc::new(Mutex::new(Sim::new()));
        let open = |core: &DspiCore| {
            let dev = CsDevice {
                fake: FakeDevice::demo(),
                sim: Arc::clone(&sim_),
            };
            core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
        };
        let core = DspiCore::new();
        open(&core);
        core.cs_fetch().unwrap();
        core.lock().cs.note_learn(2, 1, 77);
        assert!(!core.cs_pin_uses().is_empty());

        // Closed: nothing is left of it.
        core.disconnect();
        assert_eq!(snap(&core), json!({ "supported": false }));
        assert!(core.cs_pin_uses().is_empty());

        // The device changed while we were away, and is dirty. The same serial
        // shows nothing until it is fetched again.
        {
            let mut s = sim(&sim_);
            s.live.bindings[5] = CsBinding {
                component: ty::BUTTON,
                gpio: [20, GPIO_UNUSED],
                ..Default::default()
            };
            s.live.status.active_mask |= 1 << 5;
            s.live.status.dirty = true;
        }
        open(&core);
        assert_eq!(snap(&core), json!({ "supported": false }));
        assert!(core.cs_pin_uses().is_empty());
        core.cs_fetch().unwrap();
        let s = snap(&core);
        assert_eq!(s["bindings"][5]["gpio0"], 20);
        assert_eq!(s["learn"], Value::Null);
        assert!(core.cs_pin_uses().contains(&PinUse { pin: 20, slot: 5 }));

        // The baseline went too: undoing the change on the device still reads
        // as unsaved, because this connection never saw it clean.
        let original = demo::data().bindings[5].clone();
        {
            let mut s = sim(&sim_);
            s.live.bindings[5] = original;
            s.live.status.active_mask &= !(1 << 5);
        }
        core.cs_fetch().unwrap();
        assert_eq!(snap(&core)["status"]["dirty"], true);

        // A connection displaced without a close forgets it as well.
        core.install(
            crate::Connection::new(Box::new(crate::NoTransport), "DEMO".into(), 0),
            crate::state::default_state(),
            "DEMO".into(),
            0,
        );
        assert_eq!(snap(&core), json!({ "supported": false }));
    }

    #[test]
    fn wp_c14_learning_needs_an_ir_receiver() {
        let (core, sim_) = cs_core();
        assert_eq!(
            apply(&core, json!({ "op": "learnArm" })),
            json!({ "ok": true })
        );
        let r = apply(&core, json!({ "op": "learnRead" }));
        assert_eq!(
            r,
            json!({ "ok": true, "state": 2, "protocol": 1, "code": 0x20DF_10EF_u32 })
        );
        sim(&sim_).no_ir = true;
        assert_eq!(
            apply(&core, json!({ "op": "learnArm" })),
            json!({ "ok": false, "status": STATUS_NO_IR })
        );
        assert_eq!(
            apply(&core, json!({ "op": "learnCancel" })),
            json!({ "ok": true })
        );
        // A learn notification (WP C7) shows in the snapshot.
        core.lock().cs.note_learn(2, 1, 77);
        assert_eq!(
            snap(&core)["learn"],
            json!({ "state": 2, "protocol": 1, "code": 77 })
        );
    }

    #[test]
    fn wp_c14_aux_outputs_switch_and_dim() {
        let (core, sim_) = cs_core();
        assert_eq!(
            apply(&core, json!({ "op": "auxState", "index": 10, "on": false }))["ok"],
            true
        );
        assert_eq!(sim(&sim_).aux.state[10], 0);
        assert_eq!(
            apply(
                &core,
                json!({ "op": "auxLevel", "index": 11, "percent": 25.0 })
            )["ok"],
            true
        );
        assert_eq!(sim(&sim_).aux.level_q8[11], 25 * 256);
        let s = snap(&core);
        assert_eq!(
            (s["auxState"][10].clone(), s["auxLevel"][11].clone()),
            (json!(0), json!(6400))
        );
        // Over 100 % is clamped, as the old core did.
        apply(
            &core,
            json!({ "op": "auxLevel", "index": 11, "percent": 250 }),
        );
        assert_eq!(sim(&sim_).aux.level_q8[11], AUX_LEVEL_MAX_Q8);
        // A level on an on/off output is the device's INVALID_AUX.
        let r = apply(
            &core,
            json!({ "op": "auxLevel", "index": 10, "percent": 10 }),
        );
        assert_eq!(
            r,
            json!({ "ok": false, "status": surfaces::status::INVALID_AUX })
        );
        // Read back from the device.
        sim(&sim_).aux.state[10] = 1;
        assert_eq!(apply(&core, json!({ "op": "auxRead" }))["ok"], true);
        assert_eq!(snap(&core)["auxState"][10], 1);
        core.lock().cs.note_aux(10, 0, 0);
        assert_eq!(snap(&core)["auxState"][10], 0);
        core.lock().cs.note_aux(99, 1, 1);
    }

    #[test]
    fn wp_c14_firmware_without_control_surfaces_is_unsupported() {
        let core = DspiCore::new();
        // -- WP CG16 -- `demo()` has surfaces now
        core.attach_probed(
            Box::new(FakeDevice::demo_without_surfaces()),
            "DEMO".into(),
            0,
        )
        .unwrap();
        core.cs_fetch().unwrap();
        assert_eq!(snap(&core), json!({ "supported": false }));
        let r = apply(&core, json!({ "op": "binding", "index": 0 }));
        assert_eq!(r["ok"], false);
        assert!(r["error"].as_str().unwrap().contains("not supported"));
        assert_eq!(
            apply(&core, json!({ "op": "refresh" })),
            json!({ "ok": true })
        );
    }

    #[test]
    fn wp_c14_another_device_inherits_nothing() {
        let (core, _) = cs_core();
        assert_eq!(snap(&core)["supported"], true);
        let (other, other_sim) = cs_core_as("OTHER");
        // Same core, a different device: its configuration is not read yet.
        let dev = CsDevice {
            fake: FakeDevice::demo(),
            sim: Arc::clone(&other_sim),
        };
        drop(other);
        core.attach_probed(Box::new(dev), "OTHER".into(), 0)
            .unwrap();
        assert_eq!(snap(&core), json!({ "supported": false }));
        assert!(core.cs_pin_uses().is_empty());
        // The first operation reads it.
        assert_eq!(apply(&core, json!({ "op": "refreshStatus" }))["ok"], true);
        assert_eq!(snap(&core)["supported"], true);
    }

    #[test]
    fn wp_c14_a_lost_device_fails_the_fetch() {
        let (core, _) = cs_core();
        let mut inner = core.lock();
        let conn = inner.conn.as_mut().unwrap();
        // The same session, over a transport that has gone.
        let caps = conn.session().unwrap().capabilities().clone();
        let gone = Session::new(Box::new(crate::NoTransport), caps).unwrap();
        conn.link = Wire::Session(Box::new(gone));
        drop(inner);
        assert_eq!(core.cs_fetch(), Err(DspiError::Disconnected));
    }

    proptest! {
        /// Any text into `cs_apply` without a device: a JSON reply, never a
        /// panic.
        #[test]
        fn proptest_cs_apply_never_panics(text in ".{0,200}") {
            let core = DspiCore::new();
            let r: Value = serde_json::from_str(&core.cs_apply(text)).unwrap();
            prop_assert_eq!(&r["ok"], &json!(false));
        }

        /// The operation parsers accept or refuse any JSON value, never
        /// panicking or wrapping a number.
        #[test]
        fn proptest_cs_op_parsers_never_panic(
            keys in proptest::collection::vec(
                prop_oneof![Just("type"), Just("noun"), Just("gpio0"), Just("gpio1"), Just("value"),
                            Just("code"), Just("memberMask"), Just("preDelay"), Just("dwell"),
                            Just("index"), Just("name"), Just("steps")],
                0..8),
            values in proptest::collection::vec(
                prop_oneof![
                    any::<i64>().prop_map(Value::from),
                    any::<f64>().prop_map(Value::from),
                    any::<bool>().prop_map(Value::from),
                    ".{0,8}".prop_map(Value::from),
                    Just(Value::Null),
                    Just(json!([1, 2])),
                ],
                8),
        ) {
            let mut o = serde_json::Map::new();
            for (k, v) in keys.iter().zip(values) {
                o.insert((*k).to_owned(), v);
            }
            let v = Value::Object(o);
            if let Ok(b) = binding_from_json(&v) {
                // Whatever was accepted is the value given, not a wrapped one.
                if let Some(n) = v.get("noun").and_then(Value::as_i64) {
                    prop_assert_eq!(i64::from(b.noun), n);
                }
            }
            let _ = ir_from_json(&v);
            let _ = group_from_json(&v);
            let _ = step_from_json(&v);
            let _ = display_cfg_from_json(&v);
            let _ = page_from_json(&v);
            let _ = index(&v, 16, "slot");
        }
    }

    // -- WP CG24 --

    type LockLog = Arc<Mutex<Vec<&'static str>>>;

    /// A CS device behind [`Locked`], fetched: the sim and the lock log.
    fn locked_core() -> (Arc<DspiCore>, Arc<Mutex<Sim>>, LockLog) {
        let locks = Arc::new(Mutex::new(Vec::new()));
        let sim_ = Arc::new(Mutex::new(Sim::new()));
        let dev = Locked {
            dev: CsDevice {
                fake: FakeDevice::demo(),
                sim: Arc::clone(&sim_),
            },
            locks: Arc::clone(&locks),
        };
        let core = DspiCore::new();
        core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
        core.cs_fetch().unwrap();
        (core, sim_, locks)
    }

    fn lock_log(l: &LockLog) -> Vec<&'static str> {
        l.lock().unwrap().clone()
    }

    #[test]
    fn wp_cg24_the_learn_window_holds_the_lock_until_a_final_read() {
        let (core, sim_, locks) = locked_core();
        sim(&sim_).learn.state = surfaces::learn::ARMED;
        assert_eq!(apply(&core, json!({ "op": "learnArm" }))["ok"], true);
        assert_eq!(core.held_lock_holder(), Some(Holder::Learn));
        // Still armed: still held.
        assert_eq!(apply(&core, json!({ "op": "learnRead" }))["state"], 1);
        assert_eq!(core.held_lock_holder(), Some(Holder::Learn));
        assert_eq!(lock_log(&locks), ["lock"]);
        // A result (or the device's own timeout) ends it.
        sim(&sim_).learn.state = surfaces::learn::DONE;
        assert_eq!(apply(&core, json!({ "op": "learnRead" }))["state"], 2);
        assert_eq!(core.held_lock_holder(), None);
        assert_eq!(lock_log(&locks), ["lock", "unlock"]);
        // A read with nothing held gives nothing back.
        apply(&core, json!({ "op": "learnRead" }));
        assert_eq!(lock_log(&locks), ["lock", "unlock"]);
    }

    #[test]
    fn wp_cg24_a_cancel_or_a_refused_arm_releases_it() {
        let (core, sim_, locks) = locked_core();
        sim(&sim_).learn.state = surfaces::learn::ARMED;
        apply(&core, json!({ "op": "learnArm" }));
        assert_eq!(
            apply(&core, json!({ "op": "learnCancel" })),
            json!({ "ok": true })
        );
        assert_eq!(core.held_lock_holder(), None);
        assert_eq!(lock_log(&locks), ["lock", "unlock"]);
        sim(&sim_).no_ir = true;
        assert_eq!(
            apply(&core, json!({ "op": "learnArm" })),
            json!({ "ok": false, "status": STATUS_NO_IR })
        );
        assert_eq!(core.held_lock_holder(), None);
        assert_eq!(lock_log(&locks), ["lock", "unlock", "lock", "unlock"]);
    }

    #[test]
    fn wp_cg24_the_window_ends_on_its_own_and_a_new_arm_starts_another() {
        let (core, sim_, locks) = locked_core();
        sim(&sim_).learn.state = surfaces::learn::ARMED;
        apply(&core, json!({ "op": "learnArm" }));
        // Arming again restarts the window: the old lock goes first.
        apply(&core, json!({ "op": "learnArm" }));
        assert_eq!(lock_log(&locks), ["lock", "unlock", "lock"]);
        core.expire_held_lock();
        assert_eq!(core.held_lock_holder(), None);
        // The expired lock is given back before a new one is taken.
        apply(&core, json!({ "op": "learnArm" }));
        assert_eq!(
            lock_log(&locks),
            ["lock", "unlock", "lock", "unlock", "lock"]
        );
    }

    #[test]
    fn wp_cg24_inside_a_callers_lock_learning_asks_nothing() {
        let (core, sim_, locks) = locked_core();
        core.device_lock_acquire("Flow".into(), 5000).unwrap();
        sim(&sim_).learn.state = surfaces::learn::ARMED;
        apply(&core, json!({ "op": "learnArm" }));
        sim(&sim_).learn.state = surfaces::learn::DONE;
        apply(&core, json!({ "op": "learnRead" }));
        apply(&core, json!({ "op": "learnCancel" }));
        // A macro write nests too.
        let r = apply(
            &core,
            json!({ "op": "macro", "index": 1, "macro": { "name": "x", "steps": [] } }),
        );
        assert_eq!(r["ok"], true);
        assert_eq!(core.held_lock_holder(), Some(Holder::Caller));
        assert_eq!(lock_log(&locks), ["lock", "header"]);
        core.device_lock_release();
        assert_eq!(lock_log(&locks), ["lock", "header", "unlock"]);
    }

    #[test]
    fn wp_cg24_the_callers_release_leaves_a_learn_window_alone() {
        let (core, sim_, locks) = locked_core();
        sim(&sim_).learn.state = surfaces::learn::ARMED;
        apply(&core, json!({ "op": "learnArm" }));
        core.device_lock_release();
        assert_eq!(core.held_lock_holder(), Some(Holder::Learn));
        assert_eq!(lock_log(&locks), ["lock"]);
        apply(&core, json!({ "op": "learnCancel" }));
        assert_eq!(core.held_lock_holder(), None);
        assert_eq!(lock_log(&locks), ["lock", "unlock"]);
        // An acquire during a window makes it the caller's: the release ends it.
        apply(&core, json!({ "op": "learnArm" }));
        core.device_lock_acquire("Flow".into(), 5000).unwrap();
        assert_eq!(core.held_lock_holder(), Some(Holder::Caller));
        apply(&core, json!({ "op": "learnCancel" }));
        assert_eq!(core.held_lock_holder(), Some(Holder::Caller));
        core.device_lock_release();
        assert_eq!(core.held_lock_holder(), None);
        assert_eq!(lock_log(&locks), ["lock", "unlock", "lock", "unlock"]);
    }

    #[test]
    fn wp_cg24_a_closed_connection_ends_the_window() {
        let (core, sim_, _locks) = locked_core();
        sim(&sim_).learn.state = surfaces::learn::ARMED;
        apply(&core, json!({ "op": "learnArm" }));
        core.disconnect();
        assert_eq!(core.held_lock_holder(), None);
    }

    /// The learn window on the fake hub. The hub has no control surfaces, so
    /// the core learns on the sim and takes the lock through a Link session's
    /// side channel, grafted onto its connection: the hub sees that session
    /// hold the lock, and another session is refused.
    #[cfg(feature = "link")]
    #[test]
    fn wp_cg24_another_session_is_locked_out_during_a_learn_window() {
        use std::time::{Duration, Instant};

        use dspi_link::fakehub::{FakeHub, HubConfig};

        let wait = |what: &str, f: &dyn Fn() -> bool| {
            let start = Instant::now();
            while !f() {
                assert!(start.elapsed() < Duration::from_secs(8), "{what}");
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let bound = |hub: &FakeHub| {
            let core = DspiCore::new();
            core.link_connect(
                hub.url(),
                "wp-cg24".into(),
                "test".into(),
                String::new(),
                String::new(),
            )
            .unwrap();
            wait("the device list", &|| !core.link_devices().is_empty());
            let d = core.link_devices().remove(0);
            core.link_select_device(d.handle, d.serial).unwrap();
            core.fetch_all().unwrap();
            core
        };
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let (a, b) = (bound(&hub), bound(&hub));
        let (core, sim_, _locks) = locked_core();
        let side = a.lock().conn().unwrap().side.clone();
        assert!(side.is_some());
        core.lock().conn().unwrap().side = side;

        sim(&sim_).learn.state = surfaces::learn::ARMED;
        assert_eq!(apply(&core, json!({ "op": "learnArm" }))["ok"], true);
        assert!(hub.lock_holder().is_some());
        assert_eq!(b.set_master_volume(-6.0), Err(DspiError::Locked));
        sim(&sim_).learn.state = surfaces::learn::DONE;
        apply(&core, json!({ "op": "learnRead" }));
        wait("the release", &|| hub.lock_holder().is_none());
        b.set_master_volume(-6.0).unwrap();

        // The cancel, and the connection's close, end it too.
        sim(&sim_).learn.state = surfaces::learn::ARMED;
        apply(&core, json!({ "op": "learnArm" }));
        assert!(hub.lock_holder().is_some());
        apply(&core, json!({ "op": "learnCancel" }));
        wait("the release", &|| hub.lock_holder().is_none());
        apply(&core, json!({ "op": "learnArm" }));
        assert!(hub.lock_holder().is_some());
        core.disconnect();
        wait("the release", &|| hub.lock_holder().is_none());
        b.set_master_volume(-6.0).unwrap();
    }
}

/// WP CG16: the core on the demo device itself, not this file's `Sim`.
#[cfg(test)]
mod tests_wp_cg16 {
    use std::sync::Arc;

    use dspi_transport::{
        DeviceDescriptor, FakeDevice, NotificationSource, Result as TResult, Transport,
    };
    use serde_json::{Value, json};

    use crate::{DspiCore, DspiError};

    fn demo() -> Arc<DspiCore> {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        core.cs_fetch().unwrap();
        core
    }

    fn apply(core: &DspiCore, op: Value) -> Value {
        serde_json::from_str(&core.cs_apply(op.to_string())).unwrap()
    }

    fn snapshot(core: &DspiCore) -> Value {
        serde_json::from_str(&core.cs_snapshot()).unwrap()
    }

    fn button(pin: u8) -> Value {
        json!({ "type": 1, "noun": 2, "action": 4, "gpio0": pin, "gpio1": 255, "value": 1 })
    }

    #[test]
    fn wp_cg16_open_demo_has_a_configuration_to_edit() {
        let core = demo();
        let s = snapshot(&core);
        assert_eq!(s["supported"], true);
        assert_eq!(s["bindings"][0]["type"], 4);
        assert_eq!(s["names"][0], "Volume Knob");
        assert_eq!(s["status"]["dirty"], false);

        let done = apply(
            &core,
            json!({ "op": "binding", "index": 5, "binding": button(5) }),
        );
        assert_eq!(done, json!({ "ok": true, "status": 0 }));
        assert_eq!(snapshot(&core)["status"]["dirty"], true);
        assert_eq!(apply(&core, json!({ "op": "revert" }))["ok"], true);
        let s = snapshot(&core);
        assert_eq!(s["status"]["dirty"], false);
        assert_eq!(s["bindings"][5]["type"], 0);
    }

    #[test]
    fn wp_cg16_a_refusal_is_the_devices_status_code() {
        let core = demo();
        for (pin, status) in [(16, 0x1A), (10, 0x02), (24, 0x01)] {
            let r = apply(
                &core,
                json!({ "op": "binding", "index": 5, "binding": button(pin) }),
            );
            assert_eq!(r, json!({ "ok": false, "status": status }), "pin {pin}");
        }
        assert_eq!(snapshot(&core)["bindings"][5]["type"], 0);
        assert_eq!(
            apply(&core, json!({ "op": "auxState", "index": 0, "on": 1 }))["status"],
            0x26
        );
        assert_eq!(
            apply(&core, json!({ "op": "binding", "index": 3, "binding": {} }))["ok"],
            true
        );
        assert_eq!(apply(&core, json!({ "op": "learnArm" }))["status"], 0x1E);
    }

    #[test]
    fn wp_cg16_a_macro_runs_to_its_end() {
        let core = demo();
        assert_eq!(
            apply(&core, json!({ "op": "macroFire", "index": 0 }))["ok"],
            true
        );
        let running = |c: &DspiCore| {
            apply(c, json!({ "op": "refreshStatus" }));
            snapshot(c)["ext"]["macroRunning"].as_u64().unwrap()
        };
        let mut seen = vec![];
        for _ in 0..4 {
            seen.push(running(&core));
        }
        assert_eq!(*seen.last().unwrap(), 0xFF, "{seen:?}");
    }

    #[test]
    fn wp_cg16_a_held_demo_refuses_and_lets_go() {
        let core = demo();
        core.cs_demo_busy(true).unwrap();
        for op in ["save", "revert"] {
            assert_eq!(
                apply(&core, json!({ "op": op })),
                json!({ "ok": false, "status": 0x1B }),
                "{op}"
            );
        }
        let r = apply(&core, json!({ "op": "macroFire", "index": 0 }));
        assert_eq!(r["ok"], false);
        let r = apply(
            &core,
            json!({ "op": "binding", "index": 5, "binding": button(5) }),
        );
        assert_eq!(r, json!({ "ok": false, "status": 0x1B }));
        core.cs_demo_busy(false).unwrap();
        assert_eq!(apply(&core, json!({ "op": "save" }))["ok"], true);
    }

    #[test]
    fn wp_cg16_a_name_alone_is_unsaved_and_undoing_it_is_not() {
        let core = demo();
        assert_eq!(snapshot(&core)["status"]["dirty"], false);
        let named = apply(&core, json!({ "op": "name", "index": 5, "name": "Mine" }));
        assert_eq!(named, json!({ "ok": true, "status": 0 }));
        let s = snapshot(&core);
        assert_eq!(s["names"][5], "Mine");
        assert_eq!(s["status"]["dirty"], true);
        assert_eq!(s["status"]["lastSlot"], 5);

        apply(&core, json!({ "op": "name", "index": 5, "name": "" }));
        assert_eq!(snapshot(&core)["status"]["dirty"], false);
    }

    /// The firmware ACKs an aux SET during a revert and changes nothing, with no
    /// status to say so.
    #[test]
    fn wp_cg16_an_aux_write_during_a_revert_is_refused_and_not_recorded() {
        let core = demo();
        let live = |c: &DspiCore| {
            let s = snapshot(c);
            (s["auxState"][10].clone(), s["auxLevel"][11].clone())
        };
        let before = live(&core);
        assert_eq!(before, (json!(0), json!(12800)));
        let state = json!({ "op": "auxState", "index": 10, "on": 1 });
        let level = json!({ "op": "auxLevel", "index": 11, "percent": 25 });

        core.cs_demo_busy(true).unwrap();
        for op in [&state, &level] {
            assert_eq!(
                apply(&core, op.clone()),
                json!({ "ok": false, "status": 0x1B }),
                "{op}"
            );
            assert_eq!(live(&core), before, "{op}");
        }

        core.cs_demo_busy(false).unwrap();
        for op in [&state, &level] {
            assert_eq!(apply(&core, op.clone())["ok"], true, "{op}");
        }
        assert_eq!(live(&core), (json!(1), json!(25 * 256)));
    }

    #[test]
    fn wp_cg16_the_hold_is_for_the_demo_device_only() {
        assert!(matches!(
            DspiCore::new().cs_demo_busy(true),
            Err(DspiError::Disconnected)
        ));

        /// The demo device, reporting itself as USB.
        struct Usb(FakeDevice);
        impl Transport for Usb {
            fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> TResult<Vec<u8>> {
                self.0.control_in(opcode, value, len)
            }
            fn control_in_upto(&mut self, opcode: u8, value: u16, len: u16) -> TResult<Vec<u8>> {
                self.0.control_in_upto(opcode, value, len)
            }
            fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> TResult<()> {
                self.0.control_out(opcode, value, data)
            }
            fn descriptor(&self) -> &DeviceDescriptor {
                self.0.descriptor()
            }
            fn max_transfer(&self) -> usize {
                self.0.max_transfer()
            }
            fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
                self.0.notifications()
            }
        }
        let core = DspiCore::new();
        core.attach_probed(Box::new(Usb(FakeDevice::demo())), "REAL".into(), 0)
            .unwrap();
        assert!(matches!(
            core.cs_demo_busy(true),
            Err(DspiError::Unsupported)
        ));
    }
}
