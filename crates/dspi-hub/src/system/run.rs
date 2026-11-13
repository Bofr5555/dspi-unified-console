//! The system model's I/O (WP SW2): reads every element of the model, applies
//! the alignment, fans a scene out. Each transfer is a command on the device's
//! worker as the hub's own session ([`INTERNAL_SESSION_ID`], admin), so it waits
//! its turn behind clients' commands, honours a client's lock, and goes through
//! BULKFIX's guard. Blocking: callers are connection readers, never hub threads.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use dspi_link::frames::CmdRequest;
use dspi_link::{Dir, status};
use dspi_proto::ext::audio_link::{
    self as alink, ApplyOp, ApplyRequest, ApplyStatus, FrameCounter, LinkInfo, LinkPhase, LinkRole,
    LinkSlots, LinkStatus, SuperframeSnapshot,
};
use dspi_proto::generated::ext as g;
use dspi_proto::generated::opcodes as op;
use serde::Serialize;

use super::alarms::Alarm;
use super::align::{self, Alignment, Observed};
use super::scene::{self, ScenePlan, SceneRefused, SenderSeen};
use super::{SystemShared, alarm_event};
use crate::device::DeviceShared;
use crate::hub::HubHandle;
use crate::session::{Completion, INTERNAL_SESSION_ID, Role, SessionCtx};

/// How long one transfer may take, queue included.
const TRANSFER_WAIT: Duration = Duration::from_secs(3);

/// What `system.status` answers.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatus {
    /// The model's revision.
    pub revision: u64,
    /// What each element reported, by serial.
    pub observed: BTreeMap<String, Observed>,
    /// The alignment as it would be computed now.
    pub alignment: Alignment,
    /// The alarms.
    pub alarms: Vec<Alarm>,
}

/// One write's outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteResult {
    /// The device.
    pub serial: String,
    /// The element.
    pub element: String,
    /// `slots`, `phase`, `out_delay:<n>`, or the op's name.
    pub what: String,
    /// `ok`, `unchanged`, `skipped`, or a Link status name.
    pub result: String,
}

/// What `system.align` answers.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlignReport {
    /// The computation.
    pub alignment: Alignment,
    /// Whether it was written.
    pub applied: bool,
    /// The writes, when applied.
    pub writes: Vec<WriteResult>,
}

/// What `system.scene.apply` answers.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneReport {
    /// The plan: the frame, the path, the ops.
    pub plan: ScenePlan,
    /// Each op's write, with what the unit said of it right after (fixup sw2x
    /// F2): `queued` (waiting for its frame), `applied`, `late` (its frame had
    /// passed: applied at once, not with the others), `cancelled`,
    /// `refused_at_apply`, `queued_unverified` (written, the unit's queue not
    /// readable), or the write's own failure.
    pub writes: Vec<WriteResult>,
    // -- fixup sw2x (F2) --
    /// A link frame read after the last write; `None` if none could be read.
    pub frame_after_writes: Option<u64>,
    /// Whether every op is known to act at the plan's frame (all `queued` or
    /// `applied`, and the frame was still ahead after the writes).
    pub on_time: bool,
}

// -- fixup sw2x --
/// Why a scene or a system mute was not run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SystemRefused {
    /// The scene or mute could not be planned.
    #[error("{0:?}")]
    Scene(SceneRefused),
    /// The caller's role may not do one of its ops directly; names the op.
    #[error("{0}")]
    Denied(String),
    /// No sender of the model is reachable (a system mute).
    #[error("no sender of the model is reachable")]
    NoSender,
}

impl From<SceneRefused> for SystemRefused {
    fn from(e: SceneRefused) -> Self {
        Self::Scene(e)
    }
}

// -- fixup sw2x (F2) --
/// A unit whose round trip could not be measured is given this per write: a
/// Link-bridged unit or a busy worker.
const DEFAULT_WRITE_RTT: Duration = Duration::from_millis(100);
/// The least a write is counted at, however fast its unit's read was: a write
/// can wait behind a client's command, and a read says little about it.
const MIN_WRITE_RTT: Duration = Duration::from_millis(10);
/// Added to the writes' estimate before it becomes a lead.
const LEAD_MARGIN: Duration = Duration::from_millis(50);
/// Time becomes frames at the fastest stream rate, so the lead is never short.
const FASTEST_RATE: u128 = 96_000;

/// Frames at [`FASTEST_RATE`] in `d`.
fn frames_in(d: Duration) -> u64 {
    u64::try_from(d.as_micros().saturating_mul(FASTEST_RATE) / 1_000_000).unwrap_or(u64::MAX)
}

/// What a unit's queue said of `req` right after it was written.
fn apply_result(seen: Option<ApplyStatus>, req: &ApplyRequest) -> String {
    match seen {
        Some(s) if s.last_frame == req.frame && s.last_op == req.op => {
            match s.last_result.as_str() {
                "refused" => "refused_at_apply".into(),
                r => r.into(),
            }
        }
        Some(_) => "queued".into(),
        None => "queued_unverified".into(),
    }
}

/// `EXT_LINK_CTRL_MSG` IN.
fn read_ctrl(dev: &DeviceShared) -> Option<SuperframeSnapshot> {
    get_ext(
        dev,
        alink::EXT_LINK_CTRL_MSG,
        SuperframeSnapshot::SIZE,
        SuperframeSnapshot::decode,
    )
}

/// `EXT_FRAME_COUNTER` IN, and how long it took.
fn timed_frame(dev: &DeviceShared) -> (Option<u64>, Duration) {
    let start = Instant::now();
    let c = get_ext(
        dev,
        alink::EXT_FRAME_COUNTER,
        FrameCounter::SIZE,
        FrameCounter::decode,
    );
    (c.map(|c| c.frame).filter(|f| *f != 0), start.elapsed())
}

/// Write the system mute on a sender: op 0 ("none", which leaves a pending
/// apply alone) with `system_flags` bit 0, then read it back.
fn write_system_mute(dev: &DeviceShared, muted: bool) -> String {
    let none = ApplyRequest {
        frame: 0,
        op: ApplyOp::CancelAll,
        arg: 0,
        value: 0,
    };
    match set_ext(
        dev,
        alink::EXT_LINK_CTRL_MSG,
        none.encode_ctrl_msg(u8::from(muted)).to_vec(),
    ) {
        Err(s) => status_name(s),
        Ok(()) => match read_ctrl(dev) {
            Some(snap) if (snap.flags & 1 != 0) == muted => "ok".into(),
            Some(_) => "not_applied".into(),
            None => "ok_unverified".into(),
        },
    }
}

/// Ask `dev` whether it sends 0xE1, without waiting: the answer lands in
/// `system` from the device's worker (fixup sw2x F5).
pub(crate) fn learn_events(system: &Arc<SystemShared>, dev: &DeviceShared) {
    let serial = dev.serial();
    let system = Arc::clone(system);
    let req = CmdRequest {
        tag: 0,
        handle: dev.handle,
        dir: Dir::Get,
        b_request: op::REQ_EXTENSION,
        w_value: g::EXT_NOTIFY_FEATURES,
        w_index: dspi_proto::VENDOR_INTERFACE,
        w_length: 4,
        payload: Vec::new(),
    };
    let done = Completion::new(0, move |r| {
        let bits = (r.status == status::OK)
            .then(|| r.payload.get(..4)?.try_into().ok().map(u32::from_le_bytes))
            .flatten()
            .unwrap_or(0);
        system.learned_events(&serial, bits & alink::NOTIFY_FEAT_LINK_STATE != 0);
    });
    dev.submit(internal(), req, done, None);
}

fn internal() -> SessionCtx {
    SessionCtx {
        id: INTERNAL_SESSION_ID,
        role: Role::Admin,
        exempt_inflight: true,
    }
}

/// A Link status as a word.
fn status_name(s: u8) -> String {
    match s {
        status::OK => "ok".into(),
        status::NO_DEVICE => "no_device".into(),
        // A STALL: the device refused it (2.8's refusals, a full queue).
        status::ERROR => "refused".into(),
        status::TIMEOUT => "timeout".into(),
        status::DENIED => "denied".into(),
        status::LOCKED => "locked".into(),
        other => format!("status_{other}"),
    }
}

/// One transfer on `dev`'s worker, waited for.
fn exec(
    dev: &DeviceShared,
    dir: Dir,
    opcode: u8,
    value: u16,
    payload: Vec<u8>,
    len: u16,
) -> Result<Vec<u8>, u8> {
    let (tx, rx) = mpsc::sync_channel(1);
    let req = CmdRequest {
        tag: 0,
        handle: dev.handle,
        dir,
        b_request: opcode,
        w_value: value,
        w_index: dspi_proto::VENDOR_INTERFACE,
        w_length: match dir {
            Dir::Get => len,
            Dir::Set => u16::try_from(payload.len()).unwrap_or(u16::MAX),
        },
        payload,
    };
    let done = Completion::new(0, move |r| {
        let _ = tx.send(r);
    });
    dev.submit(
        internal(),
        req,
        done,
        Instant::now().checked_add(TRANSFER_WAIT),
    );
    match rx.recv_timeout(TRANSFER_WAIT.saturating_add(Duration::from_secs(1))) {
        Ok(r) if r.status == status::OK => Ok(r.payload),
        Ok(r) => Err(r.status),
        Err(_) => Err(status::TIMEOUT),
    }
}

fn get_ext<T>(
    dev: &DeviceShared,
    id: u16,
    size: usize,
    decode: fn(&[u8]) -> Option<T>,
) -> Option<T> {
    let len = u16::try_from(size).ok()?;
    exec(dev, Dir::Get, op::REQ_EXTENSION, id, Vec::new(), len)
        .ok()
        .and_then(|b| decode(&b))
}

fn set_ext(dev: &DeviceShared, id: u16, payload: Vec<u8>) -> Result<(), u8> {
    exec(dev, Dir::Set, op::REQ_EXTENSION, id, payload, 0).map(|_| ())
}

/// Read one device's link. Nothing is written.
fn observe(dev: &DeviceShared) -> Observed {
    let mut o = Observed {
        online: true,
        ..Observed::default()
    };
    let Some(info) = get_ext(dev, alink::EXT_LINK_INFO, LinkInfo::SIZE, LinkInfo::decode)
        .filter(LinkInfo::usable)
    else {
        return o;
    };
    o.role = Some(info.role.as_str().into());
    o.superframe_path = matches!(info.role, LinkRole::Sender | LinkRole::FanOut);
    if info.role.receives() {
        o.d_local_min = Some(info.d_local_min);
        o.d_local_max = Some(info.d_local_max);
        if let Some(p) = get_ext(
            dev,
            alink::EXT_LINK_PHASE,
            LinkPhase::SIZE,
            LinkPhase::decode,
        ) {
            o.phase = Some((p.mode, p.d_local, p.hops_offset, p.flags));
        }
    }
    if let Some(s) = get_ext(
        dev,
        alink::EXT_LINK_STATUS,
        LinkStatus::SIZE,
        LinkStatus::decode,
    ) {
        o.state = Some(s.state.as_str().into());
        o.hops = Some(s.hops);
        o.hop_delay_frames = Some(s.hop_delay_frames);
        o.sender_id = Some(s.sender_id);
        o.rate_code = Some(s.rate_code);
        o.crc16_errors = Some(s.crc16_errors);
        o.loss_count = Some(s.loss_count);
        o.phase_error_frames = Some(s.phase_error_frames);
        o.frame = Some(s.frame).filter(|f| *f != 0);
        // -- WP SW3 --
        o.stats = Some(align::LinkStats {
            line_class: s.class,
            channels: s.channels,
            lock_count: s.lock_count,
            crc8_errors: s.crc8_errors,
            bit_slips: s.bit_slips,
            superframe_errors: s.superframe_errors,
            vcxo_ppm: s.vcxo_ppm,
            fill_frames: s.fill_frames,
        });
    }
    // -- WP SW3 -- a receiver's trim scan; a layout other than version 1 is not read.
    if info.role.receives() {
        o.trim_scan = get_ext(
            dev,
            alink::EXT_LINK_DIAG,
            alink::LINK_DIAG_SIZE,
            alink::TrimScan::decode_diag,
        )
        .map(|t| align::TrimScan {
            state: t.state_name().into(),
            trim_lo: t.trim_lo,
            trim_hi: t.trim_hi,
            trim_used: t.trim_used,
            clean_mask: t.clean_mask,
        });
    }
    if let Some(c) = get_ext(
        dev,
        alink::EXT_FRAME_COUNTER,
        FrameCounter::SIZE,
        FrameCounter::decode,
    ) {
        o.frame = Some(c.frame).filter(|f| *f != 0).or(o.frame);
    }
    // -- fixup sw2x (F1) --
    if info.role == LinkRole::Sender {
        // -- fixup sw3x -- and the apply it holds (flags bit 2), from the same read
        let snap = read_ctrl(dev);
        o.system_mute = snap.as_ref().map(|snap| snap.flags & 1 != 0);
        o.pending_apply_frame = snap
            .filter(|snap| snap.flags & 0b100 != 0)
            .map(|snap| snap.apply.frame);
    }
    o.state_events = get_ext(dev, g::EXT_NOTIFY_FEATURES, 4, |b| {
        b.get(..4)?.try_into().ok().map(u32::from_le_bytes)
    })
    .is_some_and(|f| f & alink::NOTIFY_FEAT_LINK_STATE != 0);
    o
}

impl HubHandle {
    fn system(&self) -> &Arc<SystemShared> {
        &self.inner.system
    }

    /// The model and its revision.
    pub fn system_model(&self) -> (super::model::SystemModel, u64) {
        self.system().model()
    }

    /// Replace the model (`system.set`); `system.changed` goes to every network
    /// session.
    pub fn system_set_model(
        &self,
        model: super::model::SystemModel,
    ) -> Result<u64, super::ModelRefused> {
        self.system_set_model_at(model, None)
    }

    // -- fixup sw3x --
    /// [`Self::system_set_model`] only if the hub's revision is still
    /// `expected` (when given): an editor that read revision N cannot
    /// overwrite a newer model by accident.
    pub fn system_set_model_at(
        &self,
        model: super::model::SystemModel,
        expected: Option<u64>,
    ) -> Result<u64, super::ModelRefused> {
        let revision = self.system().set_model_at(model, expected)?;
        // -- fixup sw2x (F5) -- the model's devices online now.
        let (model, _) = self.system().model();
        for (_, dev) in self.system_devices(&model) {
            if let Some(d) = dev {
                learn_events(self.system(), &d);
            }
        }
        self.inner.relay.broadcast_event(
            &serde_json::json!({"t": "system.changed", "revision": revision}).to_string(),
        );
        Ok(revision)
    }

    /// The alarms.
    pub fn system_alarms(&self) -> Vec<Alarm> {
        self.system().alarms()
    }

    /// Acknowledge an alarm; `system.alarm` follows.
    pub fn system_acknowledge(&self, id: u64) -> Option<Alarm> {
        let a = self.system().acknowledge(id)?;
        self.inner.relay.broadcast_event(&alarm_event(&a));
        Some(a)
    }

    /// Every serial the model names, with its device if it is online.
    fn system_devices(
        &self,
        model: &super::model::SystemModel,
    ) -> Vec<(String, Option<Arc<DeviceShared>>)> {
        let serials = model
            .senders
            .iter()
            .filter_map(|s| s.serial.clone())
            .chain(model.fan_outs.iter().filter_map(|f| f.serial.clone()))
            .chain(model.units.iter().map(|u| u.serial.clone()));
        serials
            .map(|s| {
                let d = self.device_by_serial(&s);
                (s, d)
            })
            .collect()
    }

    /// Read every element of the model (`system.status`): link state, limits,
    /// hops and hop delays, frame counters. Updates the alarms (changes go out
    /// as `system.alarm`) and computes the alignment, writing nothing. Blocks
    /// for the reads.
    pub fn system_status(&self) -> SystemStatus {
        let (model, revision) = self.system().model();
        let mut observed = BTreeMap::new();
        for (serial, dev) in self.system_devices(&model) {
            let o = match &dev {
                Some(d) => observe(d),
                None => Observed::default(),
            };
            observed.insert(serial, o);
        }
        let alignment = align::align(&model, &observed);
        let changes = self.system().observed(observed.clone(), alignment.clone());
        for a in &changes {
            self.inner.relay.broadcast_event(&alarm_event(a));
        }
        SystemStatus {
            revision,
            observed,
            alignment,
            alarms: self.system().alarms(),
        }
    }

    /// Compute the alignment from fresh reads (`system.align`) and, with
    /// `apply`, write it: each writable unit's slots (when the model has them),
    /// `EXT_LINK_PHASE` (phase lock, `d_local`, `hops_offset`) and the
    /// speakers' `out.delay`. A value the unit already has is not written again,
    /// so an unchanged phase does not re-anchor. Blocks for the transfers.
    pub fn system_align(&self, apply: bool) -> AlignReport {
        let status = self.system_status();
        let alignment = status.alignment;
        let mut writes = Vec::new();
        if apply {
            let (model, _) = self.system().model();
            for plan in &alignment.units {
                let dev = self.device_by_serial(&plan.serial);
                let result = |what: &str, result: String| WriteResult {
                    serial: plan.serial.clone(),
                    element: plan.unit.clone(),
                    what: what.into(),
                    result,
                };
                let (Some(dev), true) = (dev, plan.writable) else {
                    writes.push(result("phase", "skipped".into()));
                    continue;
                };
                if let Some(slots) = model
                    .unit(&plan.unit)
                    .and_then(|u| u.slots.as_ref())
                    .and_then(|s| <[u8; 8]>::try_from(s.as_slice()).ok())
                {
                    let now = get_ext(
                        &dev,
                        alink::EXT_LINK_SLOTS,
                        LinkSlots::SIZE,
                        LinkSlots::decode,
                    );
                    let r = if now == Some(LinkSlots(slots)) {
                        "unchanged".into()
                    } else {
                        set_ext(&dev, alink::EXT_LINK_SLOTS, slots.to_vec())
                            .map_or_else(status_name, |()| "ok".into())
                    };
                    writes.push(result("slots", r));
                }
                let want = LinkPhase {
                    mode: alink::PHASE_MODE_LOCK,
                    d_local: plan.d_local,
                    hops_offset: plan.hops_offset,
                    flags: 0,
                    state: 0,
                };
                let seen = status.observed.get(&plan.serial).and_then(|o| o.phase);
                let r = if seen.is_some_and(|(m, d, h, _)| {
                    (m, d, h) == (want.mode, want.d_local, want.hops_offset)
                }) {
                    "unchanged".into()
                } else {
                    self.system().expect_reanchor(&plan.serial);
                    set_ext(&dev, alink::EXT_LINK_PHASE, want.encode().to_vec())
                        .map_or_else(status_name, |()| "ok".into())
                };
                writes.push(result("phase", r));
                for (output, ms) in &plan.out_delays {
                    let what = format!("out_delay:{output}");
                    let now = exec(
                        &dev,
                        Dir::Get,
                        op::REQ_GET_OUTPUT_DELAY,
                        u16::from(*output),
                        Vec::new(),
                        4,
                    )
                    .ok()
                    .and_then(|b| b.get(..4)?.try_into().ok().map(f32::from_le_bytes));
                    let r = if now.is_some_and(|n| (n - ms).abs() < 0.0005) {
                        "unchanged".into()
                    } else {
                        exec(
                            &dev,
                            Dir::Set,
                            op::REQ_SET_OUTPUT_DELAY,
                            u16::from(*output),
                            ms.to_le_bytes().to_vec(),
                            0,
                        )
                        .map_or_else(status_name, |_| "ok".into())
                    };
                    // -- fixup sw2x (F3) -- a cut delay is never just "ok".
                    let r = if plan.clamped_outputs.contains(output) && !r.starts_with("refused") {
                        format!("clamped ({r})")
                    } else {
                        r
                    };
                    writes.push(result(&what, r));
                }
            }
        }
        AlignReport {
            alignment,
            applied: apply,
            writes,
        }
    }

    // -- fixup sw2x (roles) --
    /// Refuse what `role` may not do directly: each op of `scene` against the
    /// hub's policy for its direct command ([`super::model::op_authority`]), and
    /// a system mute as muting outputs.
    fn authorize(&self, role: Role, scene: &super::model::Scene) -> Result<(), SystemRefused> {
        let policy = &self.inner.policy;
        for a in &scene.actions {
            let Some(op) = super::model::scene_op(&a.op) else {
                return Err(SystemRefused::Denied(format!("op {} is unknown", a.op)));
            };
            let (code, dir) = super::model::op_authority(op);
            if !policy.allows(role, code, dir) {
                return Err(SystemRefused::Denied(format!(
                    "op {} on {} needs a role that may send it directly; this one may not",
                    a.op, a.target
                )));
            }
        }
        if scene.system_mute.is_some() {
            let (code, dir) = super::model::system_mute_authority();
            if !policy.allows(role, code, dir) {
                return Err(SystemRefused::Denied(
                    "the system mute needs a role that may mute outputs".into(),
                ));
            }
        }
        Ok(())
    }

    /// The first reachable sender of the model that is a sender build, with its
    /// counter and its system mute, and how long its frame read took.
    fn reachable_sender(
        &self,
        model: &super::model::SystemModel,
    ) -> Option<(SenderSeen, Arc<DeviceShared>, Duration)> {
        model.senders.iter().find_map(|s| {
            let serial = s.serial.clone()?;
            let dev = self.device_by_serial(&serial)?;
            let info = get_ext(&dev, alink::EXT_LINK_INFO, LinkInfo::SIZE, LinkInfo::decode)?;
            if info.role != LinkRole::Sender {
                return None;
            }
            // Without the bit there is no superframe write that keeps it.
            let snap = read_ctrl(&dev)?;
            let (frame, rtt) = timed_frame(&dev);
            Some((
                SenderSeen {
                    id: s.id.clone(),
                    serial,
                    frame: frame?,
                    system_mute: snap.flags & 1 != 0,
                },
                dev,
                rtt,
            ))
        })
    }

    /// Note a system mute the hub wrote or saw, and tell every session.
    fn system_mute_changed(&self, sender: &str, muted: bool) {
        for a in self.system().system_mute_seen(sender, muted) {
            self.inner.relay.broadcast_event(&alarm_event(&a));
        }
        self.inner.relay.broadcast_event(
            &serde_json::json!({"t": "system.mute", "sender": sender, "muted": muted}).to_string(),
        );
    }

    // -- fixup sw2x (F1) --
    /// `system.mute`: set or clear the system mute on every reachable sender of
    /// the model (`EXT_LINK_CTRL_MSG` op 0, which leaves a pending apply alone).
    /// Every unit on the link plays silence while it is set. A role that may
    /// mute outputs may do it. `system.mute` and `system.alarm` follow.
    pub fn system_set_mute(
        &self,
        muted: bool,
        role: Role,
    ) -> Result<Vec<WriteResult>, SystemRefused> {
        let (code, dir) = super::model::system_mute_authority();
        if !self.inner.policy.allows(role, code, dir) {
            return Err(SystemRefused::Denied(
                "the system mute needs a role that may mute outputs".into(),
            ));
        }
        let (model, _) = self.system().model();
        let mut writes = Vec::new();
        for s in &model.senders {
            let Some(serial) = s.serial.clone() else {
                continue;
            };
            let Some(dev) = self.device_by_serial(&serial) else {
                continue;
            };
            let result = write_system_mute(&dev, muted);
            if result.starts_with("ok") {
                self.system_mute_changed(&s.id, muted);
            }
            writes.push(WriteResult {
                serial,
                element: s.id.clone(),
                what: if muted {
                    "system_mute"
                } else {
                    "system_unmute"
                }
                .into(),
                result,
            });
        }
        if writes.is_empty() {
            return Err(SystemRefused::NoSender);
        }
        Ok(writes)
    }

    /// Apply scene `id` at one link frame (`system.scene.apply`) for a caller of
    /// `role`. Each op must be one the role may send directly, or nothing runs.
    /// The frame is read from the units (and the sender); the lead is the
    /// model's, or more when the units' measured round trips say the writes
    /// take longer. The ops go through the sender's superframe, carrying its
    /// system mute as it is, or to each unit, whose queue is read back after
    /// each write. Blocks for the transfers.
    pub fn system_apply_scene(&self, id: &str, role: Role) -> Result<SceneReport, SystemRefused> {
        let (model, _) = self.system().model();
        let scene = model.scene(id).ok_or(SceneRefused::NoScene)?.clone();
        self.authorize(role, &scene)?;
        let mut frames = BTreeMap::new();
        let mut rtts: BTreeMap<String, Duration> = BTreeMap::new();
        for a in &scene.actions {
            for u in model.resolve_target(&a.target).unwrap_or_default() {
                if rtts.contains_key(&u.serial) {
                    continue;
                }
                let Some(dev) = self.device_by_serial(&u.serial) else {
                    continue;
                };
                let (frame, rtt) = timed_frame(&dev);
                rtts.insert(u.serial.clone(), rtt);
                if let Some(f) = frame {
                    frames.insert(u.serial.clone(), f);
                }
            }
        }
        let sender = self.reachable_sender(&model);
        if let Some((seen, _, rtt)) = &sender {
            rtts.insert(seen.serial.clone(), *rtt);
        }
        let seen = sender.as_ref().map(|(s, _, _)| s);
        let model_lead = u64::from(model.apply_lead_frames);
        let first = scene::plan(&model, id, &frames, seen, model_lead)?;
        // -- fixup sw2x (F2) -- each op is a write and a read of the queue.
        let estimate = first
            .ops
            .iter()
            .map(|o| {
                rtts.get(&o.serial)
                    .copied()
                    .unwrap_or(DEFAULT_WRITE_RTT)
                    .max(MIN_WRITE_RTT)
            })
            .fold(LEAD_MARGIN, |acc, rtt| {
                acc.saturating_add(rtt.saturating_mul(2))
            });
        let needed = frames_in(estimate);
        let plan = if needed > model_lead {
            scene::plan(&model, id, &frames, seen, needed)?
        } else {
            first
        };
        let mut writes = Vec::new();
        let sender_dev = sender.as_ref().map(|(_, d, _)| d);
        let mute_write = |muted: bool| {
            let (serial, element, result) = match (&sender, sender_dev) {
                (Some((s, _, _)), Some(d)) => {
                    (s.serial.clone(), s.id.clone(), write_system_mute(d, muted))
                }
                _ => (String::new(), String::new(), "no_device".into()),
            };
            WriteResult {
                serial,
                element,
                what: if muted {
                    "system_mute"
                } else {
                    "system_unmute"
                }
                .into(),
                result,
            }
        };
        let superframe = plan.path == "superframe";
        // Mute first, unmute last, so nothing plays half-changed.
        if !superframe && plan.system_mute == Some(true) {
            writes.push(mute_write(true));
        }
        for o in &plan.ops {
            let Some(req) = o.request(plan.frame) else {
                continue;
            };
            let Some(dev) = self.device_by_serial(&o.serial) else {
                writes.push(WriteResult {
                    serial: o.serial.clone(),
                    element: o.element.clone(),
                    what: o.op.clone(),
                    result: "no_device".into(),
                });
                continue;
            };
            let result = if superframe {
                // Never 0 by default: the bit as the sender has it, or the scene's.
                let flags = plan.system_flags.unwrap_or(1);
                match set_ext(
                    &dev,
                    alink::EXT_LINK_CTRL_MSG,
                    req.encode_ctrl_msg(flags).to_vec(),
                ) {
                    Err(s) => status_name(s),
                    Ok(()) => match read_ctrl(&dev) {
                        Some(snap) if snap.apply.frame == plan.frame => "queued".into(),
                        Some(_) => "late".into(),
                        None => "queued_unverified".into(),
                    },
                }
            } else {
                match set_ext(&dev, alink::EXT_APPLY_AT_FRAME, req.encode().to_vec()) {
                    Err(s) => status_name(s),
                    Ok(()) => apply_result(
                        get_ext(
                            &dev,
                            alink::EXT_APPLY_AT_FRAME,
                            ApplyStatus::SIZE,
                            ApplyStatus::decode,
                        ),
                        &req,
                    ),
                }
            };
            writes.push(WriteResult {
                serial: o.serial.clone(),
                element: o.element.clone(),
                what: o.op.clone(),
                result,
            });
        }
        if !superframe && plan.system_mute == Some(false) {
            writes.push(mute_write(false));
        }
        if let (Some(muted), Some((s, _, _))) = (plan.system_mute, &sender) {
            // Carried by the superframe write, or by its own write.
            let written = writes.iter().any(|w| {
                w.serial == s.serial
                    && (w.result.starts_with("ok") || w.result.starts_with("queued"))
            });
            if written {
                self.system_mute_changed(&s.id, muted);
            }
        }
        // The frame again, now the writes are in.
        let frame_after_writes = plan
            .ops
            .first()
            .and_then(|o| self.device_by_serial(&o.serial))
            .and_then(|d| timed_frame(&d).0);
        // A queued op was in its unit's queue before its frame: on time. One
        // whose queue could not be read is on time only if the frame was still
        // ahead after every write.
        let still_ahead = frame_after_writes.is_some_and(|f| f < plan.frame);
        let on_time = writes.iter().all(|w| match w.result.as_str() {
            "queued" | "applied" | "ok" | "ok_unverified" => true,
            "queued_unverified" => still_ahead,
            _ => false,
        });
        Ok(SceneReport {
            plan,
            writes,
            frame_after_writes,
            on_time,
        })
    }
}
