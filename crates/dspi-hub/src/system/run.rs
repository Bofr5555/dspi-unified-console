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
    self as alink, FrameCounter, LinkInfo, LinkPhase, LinkRole, LinkSlots, LinkStatus,
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
    /// Each op's write.
    pub writes: Vec<WriteResult>,
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
    }
    if let Some(c) = get_ext(
        dev,
        alink::EXT_FRAME_COUNTER,
        FrameCounter::SIZE,
        FrameCounter::decode,
    ) {
        o.frame = Some(c.frame).filter(|f| *f != 0).or(o.frame);
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
        let revision = self.system().set_model(model)?;
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
        let mut handles = BTreeMap::new();
        for (serial, dev) in self.system_devices(&model) {
            let o = match &dev {
                Some(d) => observe(d),
                None => Observed::default(),
            };
            if let Some(d) = dev {
                handles.insert(d.handle, (serial.clone(), o.state_events));
            }
            observed.insert(serial, o);
        }
        let alignment = align::align(&model, &observed);
        let changes = self
            .system()
            .observed(observed.clone(), handles, alignment.clone());
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

    /// Apply scene `id` at one link frame (`system.scene.apply`): the frame is
    /// read from the units (and the sender), the ops go through the sender's
    /// superframe or to each unit ([`scene`]). Blocks for the transfers.
    pub fn system_apply_scene(&self, id: &str) -> Result<SceneReport, SceneRefused> {
        let (model, _) = self.system().model();
        let scene = model.scene(id).ok_or(SceneRefused::NoScene)?;
        let mut frames = BTreeMap::new();
        for a in &scene.actions {
            for u in model.resolve_target(&a.target).unwrap_or_default() {
                if frames.contains_key(&u.serial) {
                    continue;
                }
                let frame = self.device_by_serial(&u.serial).and_then(|d| {
                    get_ext(
                        &d,
                        alink::EXT_FRAME_COUNTER,
                        FrameCounter::SIZE,
                        FrameCounter::decode,
                    )
                });
                if let Some(c) = frame.filter(|c| c.frame != 0) {
                    frames.insert(u.serial.clone(), c.frame);
                }
            }
        }
        // The sender, if one is reachable and is a sender build.
        let sender = model.senders.iter().find_map(|s| {
            let serial = s.serial.clone()?;
            let dev = self.device_by_serial(&serial)?;
            let info = get_ext(&dev, alink::EXT_LINK_INFO, LinkInfo::SIZE, LinkInfo::decode)?;
            if info.role != LinkRole::Sender {
                return None;
            }
            let c = get_ext(
                &dev,
                alink::EXT_FRAME_COUNTER,
                FrameCounter::SIZE,
                FrameCounter::decode,
            )?;
            Some(SenderSeen {
                id: s.id.clone(),
                serial,
                frame: c.frame,
            })
        });
        let plan = scene::plan(&model, id, &frames, sender.as_ref())?;
        let superframe = plan.path == "superframe";
        let writes = plan
            .ops
            .iter()
            .map(|o| {
                let payload = o.request(plan.frame).map(|r| {
                    if superframe {
                        r.encode_ctrl_msg(0).to_vec()
                    } else {
                        r.encode().to_vec()
                    }
                });
                let id = if superframe {
                    alink::EXT_LINK_CTRL_MSG
                } else {
                    alink::EXT_APPLY_AT_FRAME
                };
                let result = match (self.device_by_serial(&o.serial), payload) {
                    (Some(d), Some(p)) => {
                        set_ext(&d, id, p).map_or_else(status_name, |()| "ok".into())
                    }
                    (None, _) => "no_device".into(),
                    (_, None) => "skipped".into(),
                };
                WriteResult {
                    serial: o.serial.clone(),
                    element: o.element.clone(),
                    what: o.op.clone(),
                    result,
                }
            })
            .collect();
        Ok(SceneReport { plan, writes })
    }
}
