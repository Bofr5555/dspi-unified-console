//! The system model's I/O (WP SW2): reads every element of the model, applies
//! the alignment, fans a scene out. Each transfer is a command on the device's
//! worker as the hub's own session ([`INTERNAL_SESSION_ID`], admin), so it waits
//! its turn behind clients' commands, honours a client's lock, and goes through
//! BULKFIX's guard. Blocking: callers are connection readers, never hub threads.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use dspi_link::frames::CmdRequest;
use dspi_link::{Dir, status};
use dspi_proto::ext::alink::{
    self, AlinkInfo, AlinkPhase, AlinkRole, AlinkSlots, AlinkStatus, ApplyOp, ApplyRequest,
    ApplyStatus, FrameCounter, SuperframeSnapshot,
};
use dspi_proto::generated::ext as g;
use dspi_proto::generated::opcodes as op;
use serde::Serialize;

use dspi_proto::ext::alink_usb::{
    self as usb, AlinkUsbConfig, AlinkUsbDiag, AlinkUsbStatus, TimeBase,
};

use super::alarms::Alarm;
use super::align::{self, Alignment, AlinkFirmware, Observed};
use super::scene::{self, ScenePlan, SceneRefused, SenderSeen, UsbNow, UsbSeenNow};
use super::{SystemShared, alarm_event};
use crate::device::DeviceShared;
use crate::hub::HubHandle;
use crate::session::{Completion, INTERNAL_SESSION_ID, Role, SessionCtx};

// -- WP AUSB1 --
/// Why a USB-direct unit's op of a scene was not written: the audio link's part
/// of the scene was refused (`system.scene.apply`'s `reason`).
pub const USB_HELD_BACK: &str = "the audio link part was refused";
// -- WP AUSB2 -- (review fixm1 F1)
/// Why a USB-direct unit's op was written at once: the reads and writes
/// before it would take longer than a bus frame reaches (1.92 s), even after
/// reading the units again, so a frame could already be past when written
/// and play up to 2 s late.
pub const USB_LEAD_EXCEEDED: &str =
    "applied now: the USB frame would be past before it was written (slow reads)";
/// Why a USB-direct unit's queued op is not at the audio link's frame: the
/// model's lead is longer than a bus frame reaches (1.92 s).
pub const USB_LEAD_CAPPED: &str =
    "at a USB frame 1.92 s ahead, not with the audio link (lead over 1.92 s)";

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
    /// `ok`, `unchanged`, `skipped`, or a Link status name. A scene's op on
    /// a unit without the audio link (WP HWLK1F): `applied_now` (written at
    /// once, not at the frame), `unsupported` (it has no direct command: a
    /// click, a sync pulse); an op on a unit with the alink that no frame could
    /// be read for: `no_frame`.
    pub result: String,
    // -- WP HWLK1F --
    /// Why it was `skipped`: `not connected`, `no audio link (firmware)`,
    /// `limits unknown`, `out of range`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
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
    /// An alink frame read after the last write; `None` if none could be read.
    pub frame_after_writes: Option<u64>,
    /// Whether every op is known to act at the plan's frame (all `queued` or
    /// `applied`, and the frame was still ahead after the writes).
    pub on_time: bool,
    // -- WP FEAT1 --
    /// The zones the scene's `source` ops switched: stored in the model (its
    /// revision moved), their units' slots written at once (the `slots`
    /// writes), not at the frame.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceSwitched>,
}

// -- WP FEAT1 --
/// A zone switched to a source, with its units' slots: (zone, source,
/// [(unit, slots)]).
type Routed = (String, String, Vec<(String, Vec<u8>)>);

// -- WP FEAT1 --
/// One zone a scene's `source` op switched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSwitched {
    /// The zone.
    pub zone: String,
    /// The source it plays now.
    pub source: String,
    /// The zone's units on the audio link that took the source's slots.
    pub units: Vec<String>,
    /// The model's revision with the switch stored.
    pub revision: u64,
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

// -- fixup sw2x (F2), factored by WP FIXM1 --
/// How long writes of these round trips take: each a write and a read of the
/// queue, plus [`LEAD_MARGIN`].
fn writes_estimate(rtts: impl Iterator<Item = Duration>) -> Duration {
    rtts.fold(LEAD_MARGIN, |acc, rtt| {
        acc.saturating_add(rtt.saturating_mul(2))
    })
}

// -- WP FIXM1 -- (review ausb1 F2)
/// The lead (samples) of a scene's USB-direct bus frame, counted from the most
/// recent USB-direct read: the model's lead or, when longer, the audio link's
/// writes (`alink_estimate`) and then the USB-direct ops' own write and
/// read-back (`usb_rtts`), plus the time already gone since that read
/// (`since_read`). -- fixup ausb2 (F4) -- Counted at the unit's own rate
/// (`rate`, samples per second: 1000 × its samples per bus frame), so the
/// 1.92 s cap of [`scene::usb_target`] is 1.92 s, not half of it.
fn usb_lead(
    model_lead: u64,
    alink_estimate: Duration,
    usb_rtts: impl Iterator<Item = Duration>,
    since_read: Duration,
    rate: u64,
) -> u64 {
    let writes = usb_rtts.fold(alink_estimate, |acc, rtt| {
        acc.saturating_add(rtt.saturating_mul(2))
    });
    let samples = |d: Duration| {
        u64::try_from(d.as_micros().saturating_mul(u128::from(rate)) / 1_000_000)
            .unwrap_or(u64::MAX)
    };
    model_lead
        .max(samples(writes))
        .saturating_add(samples(since_read))
}

// -- WP AUSB2 -- (review fixm1 F1)
/// How long until the USB-direct ops are written, from the latest USB-direct
/// read: the time already gone since it (`since_read`), the audio link's
/// writes (`alink_estimate`, which come first) and the USB-direct ops' own
/// write and read-back. A bus frame further than [`scene::USB_MAX_LEAD`] from
/// that read may be past when it is written.
fn usb_ahead(alink_estimate: Duration, usb_rtts: &[Duration], since_read: Duration) -> Duration {
    usb_rtts
        .iter()
        .fold(alink_estimate, |acc, rtt| {
            acc.saturating_add(rtt.saturating_mul(2))
        })
        .saturating_add(since_read)
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

/// `EXT_ALINK_CTRL_MSG` IN.
fn read_ctrl(dev: &DeviceShared) -> Option<SuperframeSnapshot> {
    get_ext(
        dev,
        alink::EXT_ALINK_CTRL_MSG,
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
        alink::EXT_ALINK_CTRL_MSG,
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
        system.learned_events(
            &serial,
            bits & alink::NOTIFY_FEAT_ALINK_STATE != 0,
            bits & usb::NOTIFY_FEAT_ALINK_USB != 0,
        );
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
    exec_told(dev, dir, opcode, value, payload, len).map_err(|f| f.status)
}

// -- fixup hwlk1f (F1) --
/// Why a transfer failed: its Link status, and whether the device stalled it
/// (a firmware's answer) rather than anything else the status `ERROR` covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Failed {
    status: u8,
    stalled: bool,
}

/// [`exec`], telling a stall from any other failure.
fn exec_told(
    dev: &DeviceShared,
    dir: Dir,
    opcode: u8,
    value: u16,
    payload: Vec<u8>,
    len: u16,
) -> Result<Vec<u8>, Failed> {
    let stalled = Arc::new(std::sync::atomic::AtomicBool::new(false));
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
    })
    .noting_stall(Arc::clone(&stalled));
    dev.submit(
        internal(),
        req,
        done,
        Instant::now().checked_add(TRANSFER_WAIT),
    );
    let failed = |status| Failed {
        status,
        stalled: status == status::ERROR && stalled.load(std::sync::atomic::Ordering::SeqCst),
    };
    match rx.recv_timeout(TRANSFER_WAIT.saturating_add(Duration::from_secs(1))) {
        Ok(r) if r.status == status::OK => Ok(r.payload),
        Ok(r) => Err(failed(r.status)),
        Err(_) => Err(failed(status::TIMEOUT)),
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

// -- WP HWLK1F --
/// `EXT_ALINK_INFO` IN, raw: the decoded block, or why it failed.
fn read_alink_info(dev: &DeviceShared) -> Result<Option<AlinkInfo>, Failed> {
    let len = u16::try_from(AlinkInfo::SIZE).unwrap_or(u16::MAX);
    exec_told(
        dev,
        Dir::Get,
        op::REQ_EXTENSION,
        alink::EXT_ALINK_INFO,
        Vec::new(),
        len,
    )
    .map(|b| AlinkInfo::decode(&b))
}

// -- WP HWLK1F --
/// What a device's firmware says of the audio link, from `EXT_ALINK_INFO`'s
/// answer (`info`): present when it answers. -- fixup hwlk1f (F1) -- Absent
/// only from a firmware's own answer: the device stalled `EXT_ALINK_INFO`, or
/// `EXT_LIST` answered without it (a K340 on 470a127; stock firmware stalls the
/// block). Anything else, a bus fault, a short or malformed reply, a timeout, a
/// busy worker, a Link-bridged unit's refusal, is `None`: not told.
fn alink_of(dev: &DeviceShared, info: &Result<Option<AlinkInfo>, Failed>) -> Option<AlinkFirmware> {
    match info {
        Ok(Some(_)) => return Some(AlinkFirmware::Present),
        Err(Failed { stalled: true, .. }) => return Some(AlinkFirmware::Absent),
        Ok(None) | Err(_) => {}
    }
    match exec(
        dev,
        Dir::Get,
        op::REQ_EXTENSION,
        g::EXT_LIST,
        Vec::new(),
        EXT_LIST_LEN,
    ) {
        Ok(b) if !dspi_proto::ext::decode_list(&b).contains(&alink::EXT_ALINK_INFO) => {
            Some(AlinkFirmware::Absent)
        }
        Ok(_) | Err(_) => None,
    }
}

// -- WP HWLK1F --
/// `EXT_LIST`'s longest reply the hub reads: 64 sub-commands.
const EXT_LIST_LEN: u16 = 128;

// -- WP HWLK1F --
/// Whether `dev`'s firmware has the audio link (two transfers at most).
fn probe_alink(dev: &DeviceShared) -> Option<AlinkFirmware> {
    alink_of(dev, &read_alink_info(dev))
}

/// Read one device's alink. Nothing is written.
fn observe(dev: &DeviceShared) -> Observed {
    let mut o = Observed {
        online: true,
        ..Observed::default()
    };
    // -- WP HWLK1F -- a firmware without the alink is told from one that did not answer
    let raw = read_alink_info(dev);
    o.alink = alink_of(dev, &raw);
    let Some(info) = raw.ok().flatten().filter(AlinkInfo::usable) else {
        return o;
    };
    o.role = Some(info.role.as_str().into());
    o.superframe_path = matches!(info.role, AlinkRole::Sender | AlinkRole::FanOut);
    if info.role.receives() {
        o.d_local_min = Some(info.d_local_min);
        o.d_local_max = Some(info.d_local_max);
        if let Some(p) = get_ext(
            dev,
            alink::EXT_ALINK_PHASE,
            AlinkPhase::SIZE,
            AlinkPhase::decode,
        ) {
            o.phase = Some((p.mode, p.d_local, p.hops_offset, p.flags));
        }
    }
    if let Some(s) = get_ext(
        dev,
        alink::EXT_ALINK_STATUS,
        AlinkStatus::SIZE,
        AlinkStatus::decode,
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
        o.stats = Some(align::AlinkStats {
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
            alink::EXT_ALINK_DIAG,
            alink::ALINK_DIAG_SIZE,
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
    if info.role == AlinkRole::Sender {
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
    .is_some_and(|f| f & alink::NOTIFY_FEAT_ALINK_STATE != 0);
    o
}

// -- WP AUSB1 --
/// Read one USB-direct unit: whether its firmware has the block (`EXT_LIST`,
/// or a stalled `EXT_ALINK_USB`), then `EXT_ALINK_USB`, `_STATUS` and `_DIAG`.
/// Nothing is written. The audio link's sub-commands are not read.
fn observe_usb(dev: &DeviceShared) -> Observed {
    let mut o = Observed {
        online: true,
        ..Observed::default()
    };
    match exec(
        dev,
        Dir::Get,
        op::REQ_EXTENSION,
        g::EXT_LIST,
        Vec::new(),
        EXT_LIST_LEN,
    ) {
        Ok(b) if !dspi_proto::ext::decode_list(&b).contains(&usb::EXT_ALINK_USB) => {
            o.usb_direct = Some(AlinkFirmware::Absent);
            return o;
        }
        Ok(_) => o.usb_direct = Some(AlinkFirmware::Present),
        Err(_) => {}
    }
    let len = |n: usize| u16::try_from(n).unwrap_or(u16::MAX);
    let config = match exec_told(
        dev,
        Dir::Get,
        op::REQ_EXTENSION,
        usb::EXT_ALINK_USB,
        Vec::new(),
        len(AlinkUsbConfig::SIZE),
    ) {
        Ok(b) => AlinkUsbConfig::decode(&b),
        Err(Failed { stalled: true, .. }) => {
            o.usb_direct = Some(AlinkFirmware::Absent);
            return o;
        }
        Err(_) => None,
    };
    let status = get_ext(
        dev,
        usb::EXT_ALINK_USB_STATUS,
        AlinkUsbStatus::SIZE,
        AlinkUsbStatus::decode,
    );
    let diag = get_ext(
        dev,
        usb::EXT_ALINK_USB_DIAG,
        AlinkUsbDiag::SIZE,
        AlinkUsbDiag::decode,
    );
    if let (Some(c), Some(s)) = (config, status) {
        o.usb_direct = Some(AlinkFirmware::Present);
        o.usb = Some(usb_seen(&c, &s, diag.as_ref()));
    }
    o
}

// -- WP AUSB1 --
/// The hub's record of a USB-direct unit's reads.
fn usb_seen(c: &AlinkUsbConfig, s: &AlinkUsbStatus, d: Option<&AlinkUsbDiag>) -> align::UsbSeen {
    // -- WP AUSB2 --
    let lock_error = s.flags & usb::USB_FLAG_LOCK_ERROR != 0;
    let mut flags = Vec::new();
    for (bit, word) in [
        (usb::STATUS_D_BELOW_MIN, "d_below_min"),
        (usb::STATUS_D_ABOVE_MAX, "d_above_max"),
        (usb::STATUS_RATE_UNSUPPORTED, "rate_unsupported"),
    ] {
        if c.status_flags & bit != 0 {
            flags.push(word.to_owned());
        }
    }
    align::UsbSeen {
        mode: c.usb_mode().as_str().into(),
        d: c.d,
        auto_reanchor: c.flags & usb::FLAG_AUTO_REANCHOR != 0,
        config_flags: flags,
        state: s.state.as_str().into(),
        phase_state: s.phase_state,
        rate_code: s.rate_code,
        host_ppm: s.host_ppm,
        vcxo_ppm: s.vcxo_ppm,
        phase_error: s.phase_error,
        bus_frame: s.bus_frame,
        sample: s.sample,
        d_min: s.d_min,
        d_max: s.d_max,
        fill: s.fill,
        stream: s.flags & usb::USB_FLAG_STREAM != 0,
        holdover: s.flags & usb::USB_FLAG_HOLDOVER != 0,
        stream_start_bus_frame: s.stream_start_bus_frame(),
        stream_shift: s.stream_shift,
        gap_count: s.gap_count,
        anchors: s.anchors,
        reanchors: s.reanchors,
        missed_samples: s.missed_samples,
        late_samples: s.late_samples,
        diag: d.map(|d| align::UsbDiagSeen {
            last_cause: d.last_cause_name().map(str::to_owned),
            ends: d.total_ends(),
            holdovers: d.holdovers,
            rides: d.rides,
            suspends: d.suspends,
            bus_resets: d.bus_resets,
            unplugs: d.unplugs,
            longest_ride_us: d.longest_ride_us,
            // -- WP AUSB2 --
            longest_ride_ms: d.longest_ride_us.checked_div(1000).unwrap_or(0),
            lock_errors: d.lock_errors,
            bad_sof_readings: d.bad_sof_readings,
            version: d.version,
        }),
        // -- WP AUSB2 --
        on_lock_error: match c.on_lock_error {
            usb::ON_LOCK_ERROR_PLAY => "play_unaligned",
            usb::ON_LOCK_ERROR_MUTE => "mute",
            _ => "unknown",
        }
        .into(),
        lock_error,
        lock_error_cause: lock_error
            .then(|| align::lock_error_cause(s.state.as_str(), s.phase_state).to_owned()),
        play_unaligned_unsupported: false,
    }
}

// -- WP ATHUBM --
/// Read what the time-base contract adds to unit `o`'s record, from one
/// `EXT_LIST`: `EXT_TIME_BASE` and the 0xE4 feature bit when listed, and, for a
/// unit the model has on S/PDIF (`spdif`: a follower or a source), whether
/// its firmware has the S/PDIF block and `EXT_ALINK_SPDIF` and `_STATUS`.
/// Nothing is written. Returns the `EXT_TIME_BASE` entries it read `LOCKED`
/// (review AT-C: the measurement family's settling clock takes them).
fn observe_contract(
    dev: &DeviceShared,
    o: &mut Observed,
    spdif: bool,
) -> Vec<dspi_proto::ext::timebase::TimeBaseEntry> {
    use dspi_proto::ext::timebase as tb;
    let mut locked = Vec::new();
    let list = match exec(
        dev,
        Dir::Get,
        op::REQ_EXTENSION,
        g::EXT_LIST,
        Vec::new(),
        EXT_LIST_LEN,
    ) {
        Ok(b) => dspi_proto::ext::decode_list(&b),
        Err(_) => return locked,
    };
    if spdif {
        o.spdif_block = Some(if list.contains(&tb::EXT_ALINK_SPDIF) {
            AlinkFirmware::Present
        } else {
            AlinkFirmware::Absent
        });
        if list.contains(&tb::EXT_ALINK_SPDIF) {
            let config = get_ext(
                dev,
                tb::EXT_ALINK_SPDIF,
                tb::ALINK_SPDIF_LEN,
                tb::AlinkSpdif::decode,
            );
            let status = list
                .contains(&tb::EXT_ALINK_SPDIF_STATUS)
                .then(|| {
                    get_ext(
                        dev,
                        tb::EXT_ALINK_SPDIF_STATUS,
                        tb::ALINK_SPDIF_STATUS_LEN,
                        tb::AlinkSpdifStatus::decode,
                    )
                })
                .flatten();
            if let (Some(c), Some(st)) = (config, status) {
                o.spdif = Some(spdif_seen(&c, &st));
            }
        }
    }
    if list.contains(&tb::EXT_TIME_BASE) {
        let read = get_ext(
            dev,
            tb::EXT_TIME_BASE,
            tb::TIME_BASE_LEN,
            tb::TimeBaseRead::decode,
        );
        if let Some(r) = &read {
            locked.extend(r.entries.iter().filter(|e| e.state.is_locked()).copied());
        }
        o.time_base = read.map(|r| align::TimeBaseSeen {
            output_frames: r.output_frames,
            entries: r
                .entries
                .iter()
                .map(|e| align::TimeBaseEntrySeen {
                    time_base: e.time_base,
                    transport: tb::time_base_name(e.time_base).into(),
                    role: e.role.as_str().into(),
                    state: e.state.as_str().into(),
                    last_reason: e.last_reason.as_str().into(),
                    domain: e.domain,
                    t: e.t,
                    t_minus_output: r.t_minus_output(e),
                    disturbances: e.disturbances,
                    d: e.d,
                    phase_error: e.phase_error,
                })
                .collect(),
        });
        o.time_base_events = get_ext(dev, g::EXT_NOTIFY_FEATURES, 4, |b| {
            b.get(..4)?.try_into().ok().map(u32::from_le_bytes)
        })
        .is_some_and(|f| f & tb::NOTIFY_FEAT_TIME_BASE != 0);
    }
    locked
}

// -- WP ATHUBM --
/// The hub's record of an S/PDIF unit's reads.
fn spdif_seen(
    c: &dspi_proto::ext::timebase::AlinkSpdif,
    s: &dspi_proto::ext::timebase::AlinkSpdifStatus,
) -> align::SpdifSeen {
    use dspi_proto::ext::timebase::{self as tb, spdif_flag};
    align::SpdifSeen {
        mode: c.mode.as_str().into(),
        auto_reanchor: c.flags & tb::SPDIF_FLAG_AUTO_REANCHOR != 0,
        d: c.d,
        on_lock_error: match c.on_lock_error {
            tb::SPDIF_ON_LOCK_ERROR_PLAY => "play_unaligned",
            tb::SPDIF_ON_LOCK_ERROR_MUTE => "mute",
            _ => "unknown",
        }
        .into(),
        tx_mask: c.tx_mask,
        state: s.state.as_str().into(),
        phase_state: s.phase_state,
        rate_code: s.rate_code,
        superframe: s.has(spdif_flag::SUPERFRAME),
        holdover: s.has(spdif_flag::HOLDOVER),
        lock_error: s.has(spdif_flag::LOCK_ERROR),
        foreign: s.has(spdif_flag::FOREIGN),
        source_id: s.source_id,
        epoch: s.epoch,
        frame: s.frame,
        phase_error: s.phase_error,
        vcxo_ppm: s.vcxo_ppm,
        d_min: s.d_min,
        d_max: s.d_max,
        lock_count: s.lock_count,
        loss_count: s.loss_count,
        sf_errors: s.sf_errors,
        reanchors: s.reanchors,
    }
}

// -- WP ATHUBM --
/// The `EXT_ALINK_SPDIF` the engine asks of a unit (`p`), built on what it
/// has (`seen`): a follower's D and lock-error action, a source's slots; the
/// flags as read, a source's D and lock-error action as read. A follower
/// never carries a `tx_mask` (review atab: a source turned follower has its
/// mask zeroed, or the write would be refused).
fn spdif_wanted(
    p: &align::SpdifPlan,
    seen: &align::SpdifSeen,
) -> dspi_proto::ext::timebase::AlinkSpdif {
    use dspi_proto::ext::timebase::{self as tb, AlinkSpdif, ContractState, SpdifMode};
    let source = p.mode == "source";
    let code = |s: &str| {
        if s == "play_unaligned" {
            tb::SPDIF_ON_LOCK_ERROR_PLAY
        } else {
            tb::SPDIF_ON_LOCK_ERROR_MUTE
        }
    };
    AlinkSpdif {
        mode: if source {
            SpdifMode::Source
        } else {
            SpdifMode::Follower
        },
        flags: if seen.auto_reanchor {
            tb::SPDIF_FLAG_AUTO_REANCHOR
        } else {
            0
        },
        d: if source { seen.d } else { p.d },
        on_lock_error: code(if source {
            &seen.on_lock_error
        } else {
            &p.on_lock_error
        }),
        tx_mask: if source { p.tx_mask } else { 0 },
        state: ContractState::None,
    }
}

// -- WP ATHUBM --
/// Whether `want` is what the unit has (`seen`): nothing to write, nothing
/// re-anchors.
fn spdif_same(want: &dspi_proto::ext::timebase::AlinkSpdif, seen: &align::SpdifSeen) -> bool {
    use dspi_proto::ext::timebase as tb;
    let lock = if seen.on_lock_error == "play_unaligned" {
        tb::SPDIF_ON_LOCK_ERROR_PLAY
    } else {
        tb::SPDIF_ON_LOCK_ERROR_MUTE
    };
    seen.mode == want.mode.as_str()
        && seen.d == want.d
        && lock == want.on_lock_error
        && seen.tx_mask == want.tx_mask
}

// -- fixup sw2x (F3), factored by WP FIXM1 --
/// Write `plan`'s distance delays (`out_delay:<n>`) to `dev`, each only when
/// the unit holds another value; a delay cut to the unit's longest is said
/// (`clamped (...)`).
fn write_out_delays(dev: &DeviceShared, plan: &align::UnitPlan) -> Vec<WriteResult> {
    plan.out_delays
        .iter()
        .map(|(output, ms)| {
            let now = exec(
                dev,
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
                    dev,
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
            WriteResult {
                serial: plan.serial.clone(),
                element: plan.unit.clone(),
                what: format!("out_delay:{output}"),
                result: r,
                reason: None,
            }
        })
        .collect()
}

// -- WP FIXM1 --
/// A delay write that left the unit with the delay asked (or its longest).
fn delay_written(result: &str) -> bool {
    matches!(
        result,
        "ok" | "unchanged" | "clamped (ok)" | "clamped (unchanged)"
    )
}

// -- WP FIXM1 --
/// A unit's name for people, else its id.
fn unit_name(model: &super::model::SystemModel, id: &str) -> String {
    model
        .unit(id)
        .map(|u| u.name.as_str())
        .filter(|n| !n.is_empty())
        .unwrap_or(id)
        .to_owned()
}

// -- WP AUSB1 --
/// `EXT_ALINK_USB_STATUS` for a scene: the unit's S and samples per frame when
/// it is phase-locked, and how long the read took. -- WP FIXM1 -- `since` is
/// the start of the scene's reads; the read's start is kept from it.
fn timed_usb(dev: &DeviceShared, since: Instant) -> (Option<UsbSeenNow>, Duration) {
    let start = Instant::now();
    let read_at = start.saturating_duration_since(since);
    let s = get_ext(
        dev,
        usb::EXT_ALINK_USB_STATUS,
        AlinkUsbStatus::SIZE,
        AlinkUsbStatus::decode,
    );
    let seen = s.filter(AlinkUsbStatus::phase_locked).and_then(|s| {
        Some(UsbSeenNow {
            sample: s.sample,
            spf: s.spf()?,
            read_at,
        })
    });
    (seen, start.elapsed())
}

// -- WP AUSB1 --
/// What a USB-direct unit's queue said of `req` (at bus frame `bus`, sample 0)
/// right after it was written: as [`apply_result`], its `last_frame` an S.
fn usb_apply_result(seen: Option<ApplyStatus>, req: &ApplyRequest, bus: u16, spf: u16) -> String {
    match seen {
        Some(s)
            if s.last_op == req.op && usb::bus_of_sample(s.last_frame, spf) == Some((bus, 0)) =>
        {
            match s.last_result.as_str() {
                "refused" => "refused_at_apply".into(),
                r => r.into(),
            }
        }
        Some(_) => "queued".into(),
        None => "queued_unverified".into(),
    }
}

// -- fixup ausb2 --
/// One `EXT_ALINK_USB` write's outcome: the mode and D (`ok`, `unchanged`, a
/// refusal) and, when the lock-error action was to change, its own.
struct UsbWrite {
    result: String,
    lock: Option<String>,
}

// -- fixup ausb2 (F5) --
/// A lock-error action in words.
fn lock_words(action: &str) -> &'static str {
    if action == "play_unaligned" {
        "play unaligned"
    } else {
        "mute"
    }
}

// -- WP AUSB2 --
/// How long an alignment waits for USB-direct units it re-anchored to be
/// phase-locked with their streams placed again (the firmware: about D + 20
/// ms, longer after a pull-in from mode 0).
const REANCHOR_WAIT: Duration = Duration::from_secs(3);

// -- WP AUSB2 -- (review fixm1 F3)
/// Wait until each unit of `units` (serial, device, the mode written) is as
/// its mode asks again: phase-locked with its stream placed on the timeline
/// (or no stream), or rate-locked; at most [`REANCHOR_WAIT`] in all.
fn wait_placed(units: &[(String, Arc<DeviceShared>, u8)]) {
    let until = Instant::now().checked_add(REANCHOR_WAIT);
    for (_, dev, mode) in units {
        loop {
            let s = get_ext(
                dev,
                usb::EXT_ALINK_USB_STATUS,
                AlinkUsbStatus::SIZE,
                AlinkUsbStatus::decode,
            );
            let settled = s.is_some_and(|s| match *mode {
                usb::MODE_PHASE_LOCK => {
                    s.state == usb::UsbState::PhaseLocked
                        && (s.flags & usb::USB_FLAG_STREAM == 0
                            || s.stream_start_bus_frame().is_some())
                }
                usb::MODE_RATE_LOCK => s.state == usb::UsbState::RateLocked,
                _ => true,
            });
            if settled || until.is_none_or(|t| Instant::now() >= t) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

// -- WP HWLK1F --
/// The units with the audio link a scene names, one write each for its first
/// op on it: what the superframe path reaches without a write of its own.
fn plan_targets(
    model: &super::model::SystemModel,
    scene: &super::model::Scene,
    no_alink: &BTreeSet<String>,
) -> Vec<WriteResult> {
    scene
        .actions
        .iter()
        .flat_map(|a| {
            model
                .resolve_target(&a.target)
                .unwrap_or_default()
                .into_iter()
                .filter(|u| !no_alink.contains(&u.serial))
                .map(|u| WriteResult {
                    serial: u.serial.clone(),
                    element: u.id.clone(),
                    what: a.op.clone(),
                    result: String::new(),
                    reason: None,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

// -- WP HWLK1F --
/// A scene op written at once to a unit without the audio link, with the
/// command it stands for (the firmware's apply does the same,
/// `alink_input.c` `apply_exec`): mute and unmute the user mute, a preset
/// load, the master volume (dB × 100), an output delay (samples at `rate_hz`,
/// as the firmware's half-sample rounding). A click and a sync pulse have no
/// command: `unsupported`. `applied_now`, or the write's failure.
fn apply_direct(dev: &DeviceShared, o: &scene::PlannedOp, rate_hz: u32) -> String {
    let Some(kind) = super::model::scene_op(&o.op) else {
        return "unsupported".into();
    };
    let done = match kind {
        ApplyOp::Mute | ApplyOp::Unmute => exec(
            dev,
            Dir::Set,
            op::REQ_SET_USER_MUTE,
            0,
            vec![u8::from(kind == ApplyOp::Mute)],
            0,
        )
        .map(|_| ()),
        ApplyOp::PresetLoad => exec(
            dev,
            Dir::Get,
            op::REQ_PRESET_LOAD,
            u16::from(o.arg),
            Vec::new(),
            1,
        )
        .and_then(|b| match b.first() {
            // The preset status byte: 0 is loaded.
            Some(0) | None => Ok(()),
            Some(_) => Err(status::ERROR),
        }),
        ApplyOp::MasterVolume => {
            let db = f32::from(o.value) * 0.01;
            exec(
                dev,
                Dir::Set,
                op::REQ_SET_MASTER_VOLUME,
                0,
                db.to_le_bytes().to_vec(),
                0,
            )
            .map(|_| ())
        }
        ApplyOp::OutDelay => {
            let fs = if rate_hz == 0 {
                48_000.0
            } else {
                f64::from(rate_hz)
            };
            let ms = ((f64::from(o.value) + 0.5) * 1000.0 / fs) as f32;
            exec(
                dev,
                Dir::Set,
                op::REQ_SET_OUTPUT_DELAY,
                u16::from(o.arg),
                ms.to_le_bytes().to_vec(),
                0,
            )
            .map(|_| ())
        }
        _ => return "unsupported".into(),
    };
    done.map_or_else(status_name, |()| "applied_now".into())
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

    // -- fixup hwlk1f (F1) --
    /// Whether `dev`'s firmware has no audio link by two reads that agree: its
    /// answer now, and the last status poll's, or a second read when no poll
    /// has told yet. Only a firmware's own answer counts (see [`alink_of`]).
    fn firmware_without_alink(&self, dev: &DeviceShared, serial: &str) -> bool {
        if probe_alink(dev) != Some(AlinkFirmware::Absent) {
            return false;
        }
        match self.system().last_observed(serial).and_then(|o| o.alink) {
            Some(seen) => seen == AlinkFirmware::Absent,
            None => probe_alink(dev) == Some(AlinkFirmware::Absent),
        }
    }

    // -- WP HWLK1F --
    /// Warnings for the model kept now: each unit (an audio link role by being
    /// a unit) whose device has no audio link in its firmware, read now when
    /// it is attached, else as last seen. `system.set` answers them; the model
    /// is kept all the same. Blocks for the reads.
    pub fn system_alink_warnings(&self) -> Vec<String> {
        let (model, _) = self.system().model();
        // -- WP AUSB1 -- a USB-direct unit is no unit on the audio link: its own warning
        let usb = model
            .units
            .iter()
            .filter(|u| u.usb_direct())
            .filter(|u| match self.device_by_serial(&u.serial) {
                Some(dev) => observe_usb(&dev).without_usb_direct(),
                None => self
                    .system()
                    .last_observed(&u.serial)
                    .is_some_and(|o| o.without_usb_direct()),
            })
            .map(|u| {
                let name = if u.name.is_empty() { &u.id } else { &u.name };
                format!(
                    "{name} ({}): its firmware has no USB-direct block (extension 0x8010), so it \
                     cannot be a USB-direct unit. It is not aligned and takes scenes at once.",
                    u.serial
                )
            })
            .collect::<Vec<_>>();
        model
            .units
            .iter()
            .filter(|u| u.on_alink()) // -- WP ATHUBM -- (was: not USB-direct)
            .filter(|u| match self.device_by_serial(&u.serial) {
                Some(dev) => probe_alink(&dev) == Some(AlinkFirmware::Absent),
                None => self.system().last_seen_without_alink(&u.serial),
            })
            .map(|u| {
                let name = if u.name.is_empty() { &u.id } else { &u.name };
                format!(
                    "{name} ({}): its firmware has no audio link, so it cannot be a {} on it. \
                     It is left out of the alignment and takes scenes at once, not at a frame.",
                    u.serial,
                    if u.repeater { "repeater" } else { "receiver" }
                )
            })
            .chain(usb)
            .chain(self.system_spdif_warnings(&model))
            .collect()
    }

    // -- WP ATHUBM --
    /// [`Self::system_alink_warnings`]' part for units on S/PDIF and S/PDIF
    /// sources whose firmware has no S/PDIF time base (`EXT_LIST` without
    /// 0x8020), read now when attached, else as last seen.
    fn system_spdif_warnings(&self, model: &super::model::SystemModel) -> Vec<String> {
        model
            .units
            .iter()
            .filter(|u| u.spdif() || u.spdif_source)
            .filter(|u| {
                let absent = match self.device_by_serial(&u.serial) {
                    Some(dev) => {
                        let mut o = Observed::default();
                        let _ = observe_contract(&dev, &mut o, true);
                        o.spdif_block
                    }
                    None => self
                        .system()
                        .last_observed(&u.serial)
                        .and_then(|o| o.spdif_block),
                };
                absent == Some(AlinkFirmware::Absent)
            })
            .map(|u| {
                let name = if u.name.is_empty() { &u.id } else { &u.name };
                format!(
                    "{name} ({}): its firmware has no S/PDIF time base (extension 0x8020), so it \
                     cannot be an S/PDIF {}. It is not aligned.",
                    u.serial,
                    if u.spdif_source { "source" } else { "follower" }
                )
            })
            .collect()
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

    /// Read every element of the model (`system.status`): alink state, limits,
    /// hops and hop delays, frame counters. Updates the alarms (changes go out
    /// as `system.alarm`) and computes the alignment, writing nothing. Blocks
    /// for the reads.
    ///
    /// -- WP FIXM1 -- It also watches the USB-direct streams' start offsets:
    /// when one moved since the hub last wrote them, the alignment carries the
    /// issue `usb_start_offsets_changed` ("Align again"). -- WP AUSB2 -- A
    /// client's status never writes: "Keep USB-direct units aligned" and the
    /// lock-error action run from the hub's own poll ([`Self::system_poll_now`]).
    pub fn system_status(&self) -> SystemStatus {
        self.system_status_watching(None)
    }

    // -- WP AUSB2 -- (review fixm1 F2)
    /// The hub's own poll, run every [`super::SYSTEM_POLL`] by the hub while
    /// the model has USB-direct units (and callable at once, for tests and
    /// embedders): [`Self::system_status`], then, as the hub's own session,
    /// each USB-direct unit's lock-error action written where the unit has
    /// another, and with "Keep USB-direct units aligned" the output delays of
    /// the units whose start offset moved written again (recorded as
    /// `usb_realigned`; a unit it could not write as `usb_realign_failed`,
    /// tried again at most every [`super::alarms::REALIGN_RETRY_S`]). Blocks
    /// for the transfers.
    pub fn system_poll_now(&self) -> SystemStatus {
        // -- fixup ausb2 (F6) -- an alignment from now on makes this poll's reads
        // stale: its writes are then left to the next poll
        let seen_gen = self.system().align_generation();
        let status = self.system_status_watching(Some(seen_gen));
        let (model, _) = self.system().model();
        self.sync_lock_errors(&model, &status, seen_gen);
        status
    }

    // -- WP AUSB2 --
    /// How often the hub's own poll runs (default [`super::SYSTEM_POLL`]; at
    /// least 100 ms). Takes effect after the poll's current wait.
    pub fn set_system_poll_interval(&self, every: Duration) {
        self.system().set_poll_interval(every);
    }

    // -- WP AUSB2 --
    /// The hub's own poll's wait, and whether the model has anything for it
    /// (USB-direct units).
    pub(crate) fn system_poll_due(&self) -> (Duration, bool) {
        let (model, _) = self.system().model();
        (
            self.system().poll_interval(),
            // -- WP ATHUBM -- and S/PDIF units, for their alarms
            model
                .units
                .iter()
                .any(|u| u.usb_direct() || u.spdif() || u.spdif_source),
        )
    }

    // -- WP AUSB2 --
    /// Write each USB-direct unit's lock-error action (`EXT_ALINK_USB` byte
    /// 4, with the mode, flags and D as read, so nothing re-anchors) where the
    /// unit has another than the model asks. A unit that refuses "play
    /// unaligned" (firmware before the fork's WP USB3b) is remembered, and the
    /// alignment says so (`usb_lock_error_unsupported`); a failed write is
    /// tried again at most every [`super::alarms::REALIGN_RETRY_S`].
    fn sync_lock_errors(
        &self,
        model: &super::model::SystemModel,
        status: &SystemStatus,
        seen_gen: u64,
    ) -> Vec<WriteResult> {
        let mut writes = Vec::new();
        for u in model.units.iter().filter(|u| u.usb_direct()) {
            let Some(seen) = status
                .observed
                .get(&u.serial)
                .filter(|o| o.online)
                .and_then(|o| o.usb.as_ref())
            else {
                continue;
            };
            if let Some(w) = self.sync_lock_error(u, seen, seen_gen) {
                writes.push(w);
            }
        }
        writes
    }

    // -- WP AUSB2 --
    /// [`Self::sync_lock_errors`] for one unit as read now (`seen`, the poll's
    /// own fresh read): its lock-error action written with the mode, flags and
    /// D it has, only when the action differs; the outcome (`on_lock_error`), or
    /// `None` when nothing was written. -- fixup ausb2 (F5) -- A change made is
    /// recorded as the alarm `usb_setting_restored`.
    fn sync_lock_error(
        &self,
        u: &super::model::Unit,
        seen: &align::UsbSeen,
        seen_gen: u64,
    ) -> Option<WriteResult> {
        let mode = usb::UsbMode::from_name(&seen.mode)?;
        if seen.on_lock_error == self.lock_error_to_write(u, seen).1 {
            return None;
        }
        let key = format!("lock:{}", u.serial);
        let dev = self.device_by_serial(&u.serial)?;
        if !self.system().may_retry(&key) {
            return None;
        }
        // -- fixup ausb2 (F6) -- one writer per unit
        let lock = self.system().unit_lock(&u.serial);
        let _one = super::guard(&lock);
        // An alignment came between the read and now: its settings stand.
        if self.system().align_generation() != seen_gen {
            self.system().retry_done(&key);
            return None;
        }
        let w = self.write_usb_settings(u, &dev, seen, mode.raw(), seen.d);
        let lock_result = w.lock?;
        if lock_result == "ok" {
            self.system().retry_done(&key);
            for a in &self.system().usb_setting_restored(
                &u.id,
                format!(
                    "The hub restored the installation's lock-error action on {} ({}): the unit had {}",
                    if u.name.is_empty() { &u.id } else { &u.name },
                    lock_words(u.on_lock_error.as_str()),
                    lock_words(&seen.on_lock_error)
                ),
            ) {
                self.inner.relay.broadcast_event(&alarm_event(a));
            }
        }
        Some(WriteResult {
            serial: u.serial.clone(),
            element: u.id.clone(),
            what: "on_lock_error".into(),
            result: lock_result,
            reason: None,
        })
    }

    // -- fixup ausb2 (F1, F2) --
    /// The lock-error action to write to unit `u` (code, name): the model's,
    /// or the unit's own when it is known to refuse "play unaligned".
    fn lock_error_to_write(&self, u: &super::model::Unit, seen: &align::UsbSeen) -> (u8, String) {
        let play = u.on_lock_error == super::model::OnLockError::PlayUnaligned;
        if play && self.system().refuses_play_unaligned(&u.serial) {
            let own = if seen.on_lock_error == "play_unaligned" {
                usb::ON_LOCK_ERROR_PLAY
            } else {
                usb::ON_LOCK_ERROR_MUTE
            };
            return (own, seen.on_lock_error.clone());
        }
        (u.on_lock_error.code(), u.on_lock_error.as_str().to_owned())
    }

    // -- fixup ausb2 (F1, F2) --
    /// Write unit `u`'s `EXT_ALINK_USB` in ONE write: `mode` and `d` (as the
    /// alignment asks, or as read), automatic re-anchor as read, and the
    /// model's lock-error action, so a mode or D just written is never put
    /// back. Nothing when all of it is what the unit has (`seen`). A STALL with
    /// "play unaligned" asked is told apart by writing the same again with the
    /// unit's own action: taken, the firmware refuses only the action (before
    /// the fork's WP USB3b: remembered, issue `usb_lock_error_unsupported`);
    /// refused too, the unit refuses its settings (D outside its range at this
    /// rate: issue `usb_settings_refused`, tried again later).
    fn write_usb_settings(
        &self,
        u: &super::model::Unit,
        dev: &DeviceShared,
        seen: &align::UsbSeen,
        mode: u8,
        d: u16,
    ) -> UsbWrite {
        let own = if seen.on_lock_error == "play_unaligned" {
            usb::ON_LOCK_ERROR_PLAY
        } else {
            usb::ON_LOCK_ERROR_MUTE
        };
        let (lock_code, _) = self.lock_error_to_write(u, seen);
        let mode_d_same = usb::UsbMode::from_name(&seen.mode).map(usb::UsbMode::raw) == Some(mode)
            && (seen.d == d || mode != usb::MODE_PHASE_LOCK);
        let lock_same = lock_code == own;
        if mode_d_same && lock_same {
            return UsbWrite {
                result: "unchanged".into(),
                lock: None,
            };
        }
        let config = |on_lock_error| AlinkUsbConfig {
            mode,
            flags: if seen.auto_reanchor {
                usb::FLAG_AUTO_REANCHOR
            } else {
                0
            },
            d,
            on_lock_error,
            status_flags: 0,
            state: 0,
        };
        let write = |c: AlinkUsbConfig| {
            exec_told(
                dev,
                Dir::Set,
                op::REQ_EXTENSION,
                usb::EXT_ALINK_USB,
                c.encode().to_vec(),
                0,
            )
        };
        let changed = |r: &str| {
            if mode_d_same {
                "unchanged".to_owned()
            } else {
                r.to_owned()
            }
        };
        let lock = |r: &str| (!lock_same).then(|| r.to_owned());
        match write(config(lock_code)) {
            Ok(_) => {
                self.system().settings_refused(&u.serial, None);
                if lock_code == usb::ON_LOCK_ERROR_PLAY && !lock_same {
                    self.system().play_unaligned_refused(&u.serial, false);
                }
                UsbWrite {
                    result: changed("ok"),
                    lock: lock("ok"),
                }
            }
            Err(Failed { stalled: true, .. })
                if lock_code == usb::ON_LOCK_ERROR_PLAY && !lock_same =>
            {
                match write(config(own)) {
                    Ok(_) => {
                        // The rest was taken: only the action is refused.
                        self.system().settings_refused(&u.serial, None);
                        self.system().play_unaligned_refused(&u.serial, true);
                        UsbWrite {
                            result: changed("ok"),
                            lock: lock("unsupported"),
                        }
                    }
                    Err(f) => {
                        self.refused_settings(u, seen, f);
                        UsbWrite {
                            result: changed(&status_name(f.status)),
                            lock: lock("refused"),
                        }
                    }
                }
            }
            Err(f) => {
                self.refused_settings(u, seen, f);
                UsbWrite {
                    result: changed(&status_name(f.status)),
                    lock: lock(&status_name(f.status)),
                }
            }
        }
    }

    // -- fixup ausb2 (F2) --
    /// A STALL of unit `u`'s settings: why, from the read's status flags, kept
    /// for the alignment's issue `usb_settings_refused`.
    fn refused_settings(&self, u: &super::model::Unit, seen: &align::UsbSeen, f: Failed) {
        if !f.stalled {
            return;
        }
        let why = if seen.config_flags.is_empty() {
            "its mode or D was refused".to_owned()
        } else {
            format!(
                "its D is outside its range at this rate ({})",
                seen.config_flags.join(", ")
            )
        };
        self.system().settings_refused(&u.serial, Some(why));
    }

    /// [`Self::system_status`]; `keep` lets "Keep USB-direct units aligned"
    /// write (-- WP AUSB2 -- the hub's own poll only).
    fn system_status_watching(&self, keep: Option<u64>) -> SystemStatus {
        let (model, revision) = self.system().model();
        let mut observed = BTreeMap::new();
        // -- WP AUSB1 -- the model's USB-direct units are read as such
        let usb_units: BTreeSet<String> = model
            .units
            .iter()
            .filter(|u| u.usb_direct())
            .map(|u| u.serial.clone())
            .collect();
        // -- WP ATHUBM -- units on S/PDIF are read as such; sources' S/PDIF too
        let spdif_units: BTreeSet<String> = model
            .units
            .iter()
            .filter(|u| u.spdif())
            .map(|u| u.serial.clone())
            .collect();
        let spdif_roles: BTreeSet<String> = model
            .units
            .iter()
            .filter(|u| u.spdif() || u.spdif_source)
            .map(|u| u.serial.clone())
            .collect();
        let unit_serials: BTreeSet<&str> = model.units.iter().map(|u| u.serial.as_str()).collect();
        for (serial, dev) in self.system_devices(&model) {
            let last = self.system().last_observed(&serial);
            let mut o = match &dev {
                // -- WP ATHUBM -- nothing of the audio link or USB-direct is read
                Some(_) if spdif_units.contains(&serial) => Observed {
                    online: true,
                    ..Observed::default()
                },
                // -- WP AUSB1 --
                Some(d) if usb_units.contains(&serial) => {
                    let mut o = observe_usb(d);
                    if o.usb_direct.is_none() {
                        o.usb_direct = last.as_ref().and_then(|l| l.usb_direct);
                    }
                    o
                }
                // -- fixup hwlk1f (F1) -- not told now: as last seen
                Some(d) => {
                    let mut o = observe(d);
                    if o.alink.is_none() {
                        o.alink = last.as_ref().and_then(|l| l.alink);
                    }
                    o
                }
                // -- fixup hwlk1f (F4) -- offline: what it reported last, as last seen
                None => Observed {
                    online: false,
                    ..last.clone().unwrap_or_default()
                },
            };
            // -- WP ATHUBM -- the time-base contract, on every unit's transport
            if let Some(d) = &dev
                && unit_serials.contains(serial.as_str())
            {
                // -- review AT-C -- each locked entry starts or keeps the
                // measurement family's settling clock (ATHUB's hook)
                for e in observe_contract(d, &mut o, spdif_roles.contains(&serial)) {
                    self.system_measure_note_locked(&serial, &e);
                }
                if o.spdif_block.is_none() && spdif_roles.contains(&serial) {
                    o.spdif_block = last.as_ref().and_then(|l| l.spdif_block);
                }
            }
            observed.insert(serial, o);
        }
        // -- WP AUSB2 -- what the hub learned when it wrote "play unaligned"
        for (serial, o) in &mut observed {
            if let Some(u) = o.usb.as_mut() {
                u.play_unaligned_unsupported = self.system().refuses_play_unaligned(serial);
            }
        }
        let mut alignment = align::align(&model, &observed);
        // -- fixup ausb2 (F2) -- a unit that refused its settings, and why
        for u in model.units.iter().filter(|u| u.usb_direct()) {
            if let Some(why) = self.system().refused_settings(&u.serial) {
                let name = if u.name.is_empty() { &u.id } else { &u.name };
                alignment.issues.push(align::Issue {
                    subject: Some(u.id.clone()),
                    code: align::USB_SETTINGS_REFUSED.into(),
                    message: format!(
                        "{name}: the unit refused its settings ({why}), so its lock-error action is not \
                         written; make D fit (the system delay, or the stream's rate) and Align"
                    ),
                });
            }
        }
        // -- WP FIXM1 -- (review ausb1 F3) the start offsets as written, or not
        let offsets = align::usb_offsets(&alignment);
        let mut moved = self.system().usb_offsets_moved(&offsets);
        if let (false, Some(seen_gen), true) = (moved.is_empty(), keep, model.keep_usb_aligned) {
            moved = self.realign_usb(&model, &alignment, &offsets, &moved, seen_gen);
        } else if moved.is_empty() && keep.is_some() {
            // -- WP AUSB2 -- nothing moved (any more): no failure stands
            for a in &self.system().usb_realign_failed(None) {
                self.inner.relay.broadcast_event(&alarm_event(a));
            }
        }
        if !moved.is_empty() {
            let names: Vec<String> = moved.iter().map(|id| unit_name(&model, id)).collect();
            alignment
                .issues
                .push(align::usb_offsets_changed_issue(&names));
        }
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
    /// `EXT_ALINK_PHASE` (phase lock, `d_local`, `hops_offset`) and the
    /// speakers' `out.delay`. A value the unit already has is not written again,
    /// so an unchanged phase does not re-anchor. Blocks for the transfers.
    pub fn system_align(&self, apply: bool) -> AlignReport {
        let status = self.system_status_watching(None);
        let mut alignment = status.alignment;
        let mut writes = Vec::new();
        if apply {
            let (model, _) = self.system().model();
            // -- WP AUSB2 -- USB-direct units whose mode or D was written, and
            // every USB-direct unit's plan, for its delays
            let mut reanchored: Vec<(String, Arc<DeviceShared>, u8)> = Vec::new();
            let mut usb_plans: Vec<(align::UnitPlan, Arc<DeviceShared>)> = Vec::new();
            // -- fixup ausb2 (F6) -- the USB-direct units are this alignment's until it
            // is done: the hub's poll and a scene wait (taken in serial order)
            let usb_serials: BTreeSet<&str> = alignment
                .units
                .iter()
                .filter(|p| p.transport == "usb_direct")
                .map(|p| p.serial.as_str())
                .collect();
            let unit_locks: Vec<Arc<std::sync::Mutex<()>>> = usb_serials
                .iter()
                .map(|serial| self.system().unit_lock(serial))
                .collect();
            let _held: Vec<_> = unit_locks.iter().map(|l| super::guard(l)).collect();
            // A poll that read before this now leaves these units alone.
            self.system().bump_align_generation();
            // -- WP ATHUBM -- units just made S/PDIF followers: their limits
            // are read after the switch and their D written again
            let mut spdif_switched: Vec<String> = Vec::new();
            for plan in &alignment.units {
                let dev = self.device_by_serial(&plan.serial);
                let result = |what: &str, result: String| WriteResult {
                    serial: plan.serial.clone(),
                    element: plan.unit.clone(),
                    what: what.into(),
                    result,
                    reason: None,
                };
                let (Some(dev), true) = (&dev, plan.writable) else {
                    // -- WP HWLK1F -- and why, so a client can say what was not written
                    let seen = status.observed.get(&plan.serial);
                    let usb_unit = plan.transport == "usb_direct";
                    // -- WP ATHUBM --
                    let spdif_unit = plan.transport == "spdif";
                    let reason = if dev.is_none() || !seen.is_some_and(|o| o.online) {
                        "not connected"
                    } else if spdif_unit
                        && seen.is_some_and(|o| o.spdif_block == Some(AlinkFirmware::Absent))
                    {
                        "no S/PDIF time base (firmware)"
                    } else if spdif_unit && seen.is_some_and(|o| o.spdif.is_none()) {
                        "state unknown"
                    } else if spdif_unit {
                        "out of range"
                    } else if usb_unit && seen.is_some_and(Observed::without_usb_direct) {
                        // -- WP AUSB1 --
                        "no USB-direct block (firmware)"
                    } else if usb_unit && seen.is_some_and(|o| o.usb.is_none()) {
                        "state unknown"
                    } else if seen.is_some_and(Observed::without_alink) && !usb_unit {
                        "no audio link (firmware)"
                    } else if !usb_unit && seen.is_some_and(|o| o.d_local_min.is_none()) {
                        "limits unknown"
                    } else {
                        "out of range"
                    };
                    writes.push(WriteResult {
                        reason: Some(reason.into()),
                        ..result(
                            if usb_unit {
                                "usb_direct"
                            } else if spdif_unit {
                                "spdif"
                            } else {
                                "phase"
                            },
                            "skipped".into(),
                        )
                    });
                    continue;
                };
                let dev = Arc::clone(dev);
                // -- WP ATHUBM -- a unit on S/PDIF: its role, then its outputs
                if plan.transport == "spdif" {
                    let r = self.write_spdif(&dev, plan, &status.observed);
                    let was = status
                        .observed
                        .get(&plan.serial)
                        .and_then(|o| o.spdif.as_ref())
                        .map(|s| s.mode.clone());
                    if r == "ok"
                        && plan.spdif.as_ref().is_some_and(|p| p.mode == "follower")
                        && was.as_deref() != Some("follower")
                    {
                        spdif_switched.push(plan.unit.clone());
                    }
                    writes.push(result("spdif", r));
                    writes.extend(write_out_delays(&dev, plan));
                    continue;
                }
                // -- WP AUSB1 -- a USB-direct unit: its mode and D, then the outputs
                if plan.transport == "usb_direct" {
                    let seen = status
                        .observed
                        .get(&plan.serial)
                        .and_then(|o| o.usb.as_ref());
                    let mode = plan
                        .usb_mode
                        .as_deref()
                        .and_then(usb::UsbMode::from_name)
                        .map_or(usb::MODE_OFF, usb::UsbMode::raw);
                    // -- fixup ausb2 (F1) -- mode, D and the lock-error action in one
                    // write, from the read just made, so nothing is put back
                    let d = plan.usb_d.or(seen.map(|s| s.d)).unwrap_or(usb::DEFAULT_D);
                    let (r, lock) = match (model.unit(&plan.unit), seen) {
                        (Some(u), Some(s)) => {
                            let w = self.write_usb_settings(u, &dev, s, mode, d);
                            (w.result, w.lock)
                        }
                        _ => {
                            let want = AlinkUsbConfig {
                                mode,
                                flags: usb::FLAG_AUTO_REANCHOR,
                                d,
                                on_lock_error: usb::ON_LOCK_ERROR_MUTE,
                                status_flags: 0,
                                state: 0,
                            };
                            let r = set_ext(&dev, usb::EXT_ALINK_USB, want.encode().to_vec())
                                .map_or_else(status_name, |()| "ok".into());
                            (r, None)
                        }
                    };
                    // -- WP AUSB2 -- (review fixm1 F3) a new mode or D re-anchors it
                    if r == "ok" {
                        reanchored.push((plan.serial.clone(), Arc::clone(&dev), mode));
                    }
                    writes.push(result("usb_direct", r));
                    if let Some(l) = lock {
                        writes.push(result("on_lock_error", l));
                    }
                    // -- WP ATHUBM -- a USB-direct unit that is an S/PDIF source
                    if plan.spdif.is_some() {
                        writes.push(result(
                            "spdif",
                            self.write_spdif(&dev, plan, &status.observed),
                        ));
                    }
                    // Its delays carry the start offsets: written once every
                    // USB-direct unit is placed again, below.
                    usb_plans.push((plan.clone(), dev));
                    continue;
                } else {
                    if let Some(slots) = model
                        .unit(&plan.unit)
                        .and_then(|u| u.slots.as_ref())
                        .and_then(|s| <[u8; 8]>::try_from(s.as_slice()).ok())
                    {
                        let now = get_ext(
                            &dev,
                            alink::EXT_ALINK_SLOTS,
                            AlinkSlots::SIZE,
                            AlinkSlots::decode,
                        );
                        let r = if now == Some(AlinkSlots(slots)) {
                            "unchanged".into()
                        } else {
                            set_ext(&dev, alink::EXT_ALINK_SLOTS, slots.to_vec())
                                .map_or_else(status_name, |()| "ok".into())
                        };
                        writes.push(result("slots", r));
                    }
                    let want = AlinkPhase {
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
                        set_ext(&dev, alink::EXT_ALINK_PHASE, want.encode().to_vec())
                            .map_or_else(status_name, |()| "ok".into())
                    };
                    writes.push(result("phase", r));
                    // -- WP ATHUBM -- an audio link unit that is an S/PDIF source
                    if plan.spdif.is_some() {
                        writes.push(result(
                            "spdif",
                            self.write_spdif(&dev, plan, &status.observed),
                        ));
                    }
                }
                writes.extend(write_out_delays(&dev, plan));
            }
            // -- WP AUSB2 -- (review fixm1 F3) a unit that re-anchored placed its stream
            // anew: wait for it, read the start offsets again, and write the
            // USB-direct units' delays from those, so "Align again" does not follow.
            if !reanchored.is_empty() {
                wait_placed(&reanchored);
                let fresh = self.system_status_watching(None).alignment;
                usb_plans = usb_plans
                    .into_iter()
                    .map(|(old, dev)| {
                        let plan = fresh
                            .units
                            .iter()
                            .find(|p| p.unit == old.unit)
                            .cloned()
                            .unwrap_or(old);
                        (plan, dev)
                    })
                    .collect();
                alignment = fresh;
            }
            for (plan, dev) in &usb_plans {
                if !plan.writable {
                    continue;
                }
                let delays = write_out_delays(dev, plan);
                // -- WP FIXM1 -- the start offset these delays carry, once all are in
                if delays.iter().all(|w| delay_written(&w.result)) {
                    self.system()
                        .usb_offset_written(&plan.unit, plan.start_offset_samples);
                }
                writes.extend(delays);
            }
            // -- WP ATHUBM -- a unit switched to follower now reports its limits:
            // its domain is worked out again and what differs written
            if !spdif_switched.is_empty() {
                let fresh = self.system_status_watching(None);
                for plan in fresh.alignment.units.iter().filter(|p| p.spdif.is_some()) {
                    let Some(dev) = self.device_by_serial(&plan.serial) else {
                        continue;
                    };
                    let r = self.write_spdif(&dev, plan, &fresh.observed);
                    if r != "unchanged" && r != "skipped" {
                        writes.push(WriteResult {
                            serial: plan.serial.clone(),
                            element: plan.unit.clone(),
                            what: "spdif".into(),
                            result: r,
                            reason: Some("its limits, read after it became a follower".into()),
                        });
                    }
                    // A new domain D moves the source's lag: its speakers too.
                    if plan.writable {
                        writes.extend(
                            write_out_delays(&dev, plan)
                                .into_iter()
                                .filter(|w| w.result != "unchanged"),
                        );
                    }
                }
                alignment = fresh.alignment;
            }
            // -- WP AUSB2 -- (review fixm1 F5) the report says what holds after the
            // writes: "Align again" only for units still not written
            alignment
                .issues
                .retain(|i| i.code != align::USB_OFFSETS_CHANGED);
            let moved = self
                .system()
                .usb_offsets_moved(&align::usb_offsets(&alignment));
            if !moved.is_empty() {
                let names: Vec<String> = moved.iter().map(|id| unit_name(&model, id)).collect();
                alignment
                    .issues
                    .push(align::usb_offsets_changed_issue(&names));
            }
            // -- fixup ausb2 (F6) -- and a poll that read while it wrote, too
            self.system().bump_align_generation();
        }
        AlignReport {
            alignment,
            applied: apply,
            writes,
        }
    }

    // -- WP ATHUBM --
    /// Write `plan`'s S/PDIF role (`EXT_ALINK_SPDIF`) to `dev` from the read
    /// just made: `unchanged` when the unit has it (nothing re-anchors),
    /// `skipped` when it cannot be (not read, out of range), `refused_locally`
    /// for what the unit would refuse anyway, else the write's outcome. A
    /// follower's re-anchor that follows is the hub's own (no alarm).
    fn write_spdif(
        &self,
        dev: &DeviceShared,
        plan: &align::UnitPlan,
        observed: &BTreeMap<String, Observed>,
    ) -> String {
        use dspi_proto::ext::timebase as tb;
        let seen = observed
            .get(&plan.serial)
            .filter(|o| o.online)
            .and_then(|o| o.spdif.as_ref());
        let (Some(p), Some(seen)) = (plan.spdif.as_ref(), seen) else {
            return "skipped".into();
        };
        if !p.writable {
            return "skipped".into();
        }
        let want = spdif_wanted(p, seen);
        if spdif_same(&want, seen) {
            return "unchanged".into();
        }
        if !want.is_valid() {
            return "refused_locally".into();
        }
        if want.mode == tb::SpdifMode::Follower {
            self.system().expect_reanchor(&plan.serial);
        }
        set_ext(dev, tb::EXT_ALINK_SPDIF, want.encode_out().to_vec())
            .map_or_else(status_name, |()| "ok".into())
    }

    // -- WP FIXM1 --
    /// "Keep USB-direct units aligned": write the output delays of the
    /// USB-direct units in `moved` again from `alignment` (their mode, D and
    /// every other unit are left alone, so nothing re-anchors), record it as
    /// the alarm `usb_realigned` and broadcast it. The units still moved (not
    /// writable now, or a write failed).
    fn realign_usb(
        &self,
        model: &super::model::SystemModel,
        alignment: &Alignment,
        offsets: &BTreeMap<String, i64>,
        moved: &[String],
        seen_gen: u64,
    ) -> Vec<String> {
        let mut done = Vec::new();
        let mut failed = Vec::new();
        // -- WP AUSB2 -- (review fixm1 F2) why each failed; none tried again before its time
        let mut why: Vec<String> = Vec::new();
        let mut held = false;
        for plan in alignment
            .units
            .iter()
            .filter(|p| p.transport == "usb_direct" && moved.contains(&p.unit))
        {
            let key = format!("realign:{}", plan.unit);
            let dev = self.device_by_serial(&plan.serial);
            let (Some(dev), true) = (dev, plan.writable) else {
                failed.push(plan.unit.clone());
                if self.system().may_retry(&key) {
                    why.push(format!(
                        "{} ({})",
                        unit_name(model, &plan.unit),
                        if self.device_by_serial(&plan.serial).is_none() {
                            "not connected"
                        } else {
                            "not writable now"
                        }
                    ));
                } else {
                    held = true;
                }
                continue;
            };
            if !self.system().may_retry(&key) {
                failed.push(plan.unit.clone());
                held = true;
                continue;
            }
            // -- fixup ausb2 (F6) --
            let lock = self.system().unit_lock(&plan.serial);
            let one = super::guard(&lock);
            if self.system().align_generation() != seen_gen {
                // An alignment wrote it meanwhile, from fresher reads.
                drop(one);
                self.system().retry_done(&key);
                held = true;
                continue;
            }
            let writes = write_out_delays(&dev, plan);
            drop(one);
            if writes.iter().all(|w| delay_written(&w.result)) {
                self.system()
                    .usb_offset_written(&plan.unit, offsets.get(&plan.unit).copied());
                self.system().retry_done(&key);
                done.push(unit_name(model, &plan.unit));
            } else {
                failed.push(plan.unit.clone());
                let results: Vec<String> = writes
                    .iter()
                    .filter(|w| !delay_written(&w.result))
                    .map(|w| format!("{} {}", w.what, w.result))
                    .collect();
                why.push(format!(
                    "{} ({})",
                    unit_name(model, &plan.unit),
                    results.join(", ")
                ));
            }
        }
        if !why.is_empty() {
            let message = format!(
                "Keep USB-direct units aligned: the output delays could not be written on {}; \
                 tried again in {} s, or Align by hand",
                why.join(", "),
                super::alarms::REALIGN_RETRY_S
            );
            for a in &self.system().usb_realign_failed(Some(message)) {
                self.inner.relay.broadcast_event(&alarm_event(a));
            }
        } else if failed.is_empty() && !held {
            for a in &self.system().usb_realign_failed(None) {
                self.inner.relay.broadcast_event(&alarm_event(a));
            }
        }
        if !done.is_empty() {
            let mut message = format!(
                "USB-direct start offsets changed: output delays written again on {} \
                 (Keep USB-direct units aligned)",
                done.join(", ")
            );
            if !failed.is_empty() {
                let names: Vec<String> = failed.iter().map(|id| unit_name(model, id)).collect();
                message.push_str(&format!("; not on {}", names.join(", ")));
            }
            for a in &self.system().usb_realigned(message) {
                self.inner.relay.broadcast_event(&alarm_event(a));
            }
        }
        failed
    }

    // -- fixup sw2x (roles) --
    /// Refuse what `role` may not do directly: each op of `scene` against the
    /// hub's policy for its direct command ([`super::model::op_authority`]), and
    /// a system mute as muting outputs.
    fn authorize(&self, role: Role, scene: &super::model::Scene) -> Result<(), SystemRefused> {
        let policy = &self.inner.policy;
        for a in &scene.actions {
            // -- WP FEAT1 -- a source switch is judged as choosing a unit's input
            let (code, dir) = if a.op == super::model::SCENE_OP_SOURCE {
                super::model::source_op_authority()
            } else {
                let Some(op) = super::model::scene_op(&a.op) else {
                    return Err(SystemRefused::Denied(format!("op {} is unknown", a.op)));
                };
                super::model::op_authority(op)
            };
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
            let info = get_ext(
                &dev,
                alink::EXT_ALINK_INFO,
                AlinkInfo::SIZE,
                AlinkInfo::decode,
            )?;
            if info.role != AlinkRole::Sender {
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
    /// the model (`EXT_ALINK_CTRL_MSG` op 0, which leaves a pending apply alone).
    /// Every unit on the alink plays silence while it is set. A role that may
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
                reason: None,
            });
        }
        if writes.is_empty() {
            return Err(SystemRefused::NoSender);
        }
        Ok(writes)
    }

    /// Apply scene `id` at one alink frame (`system.scene.apply`) for a caller of
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
        // -- WP P1HL -- (OD-P1l) each unit's limits profile and lock, before anything is sent
        let reached: Vec<Vec<String>> = scene
            .actions
            .iter()
            .map(|a| {
                model
                    .resolve_target(&a.target)
                    .unwrap_or_default()
                    .iter()
                    .map(|u| u.serial.clone())
                    .collect()
            })
            .collect();
        crate::limits::judge_scene(&self.inner.limits, &scene, &reached, role)
            .map_err(SystemRefused::Denied)?;
        // -- WP FEAT1 -- the source switches first: stored, then the slots written;
        // the rest of the scene is planned without them
        let (switched, slot_writes) = self.switch_sources(&scene)?;
        let (model, scene) = if switched.is_empty() {
            (model, scene)
        } else {
            let (mut now, _) = self.system().model();
            for s in now.scenes.iter_mut().filter(|s| s.id == id) {
                s.actions.retain(|a| a.op != super::model::SCENE_OP_SOURCE);
            }
            let rest = now.scene(id).cloned().ok_or(SceneRefused::NoScene)?;
            (now, rest)
        };
        let mut frames = BTreeMap::new();
        let mut rtts: BTreeMap<String, Duration> = BTreeMap::new();
        // -- WP HWLK1F -- units whose firmware has no audio link: their ops go at once
        let mut no_alink = BTreeSet::new();
        let mut offline = Vec::new();
        // -- fixup hwlk1f (F1) -- attached, with the alink (or not told), no frame read
        let mut no_frame: Vec<String> = Vec::new();
        // -- WP AUSB1 -- phase-locked USB-direct units, at a bus frame; the others at once
        let mut usb_now: BTreeMap<String, UsbSeenNow> = BTreeMap::new();
        // -- WP FIXM1 -- the reads' start: each USB-direct read is timed from it
        let reads_began = Instant::now();
        for a in &scene.actions {
            for u in model.resolve_target(&a.target).unwrap_or_default() {
                if rtts.contains_key(&u.serial) || offline.contains(&u.serial) {
                    continue;
                }
                let Some(dev) = self.device_by_serial(&u.serial) else {
                    // Offline: as it was last seen (kept across polls, fixup hwlk1f F4)
                    // -- WP ATHUBM -- a unit on S/PDIF has no audio link frame either
                    if u.usb_direct()
                        || u.spdif()
                        || self.system().last_seen_without_alink(&u.serial)
                    {
                        no_alink.insert(u.serial.clone());
                    }
                    offline.push(u.serial.clone());
                    continue;
                };
                // -- WP ATHUBM -- scenes reach a unit on S/PDIF at once (its time base 3
                // is not a scene path yet)
                if u.spdif() {
                    rtts.insert(u.serial.clone(), Duration::ZERO);
                    no_alink.insert(u.serial.clone());
                    continue;
                }
                // -- WP AUSB1 --
                if u.usb_direct() {
                    let (seen, rtt) = timed_usb(&dev, reads_began);
                    rtts.insert(u.serial.clone(), rtt);
                    match seen {
                        Some(s) => {
                            usb_now.insert(u.serial.clone(), s);
                        }
                        None => {
                            no_alink.insert(u.serial.clone());
                        }
                    }
                    continue;
                }
                // -- WP AUSB2 -- (review fixm1 F4) last seen without the alink (the
                // last status read told it twice): at once, without a frame read
                // that would stall and probe again
                if self.system().last_seen_without_alink(&u.serial) {
                    rtts.insert(u.serial.clone(), Duration::ZERO);
                    no_alink.insert(u.serial.clone());
                    continue;
                }
                let (frame, rtt) = timed_frame(&dev);
                rtts.insert(u.serial.clone(), rtt);
                match frame {
                    Some(f) => {
                        frames.insert(u.serial.clone(), f);
                    }
                    // -- fixup hwlk1f (F1) -- one read never decides a write path: the
                    // firmware's answer now, and again (as last seen, else a second read)
                    None if self.firmware_without_alink(&dev, &u.serial) => {
                        no_alink.insert(u.serial.clone());
                    }
                    None => {
                        no_frame.push(u.serial.clone());
                    }
                }
            }
        }
        let sender = self.reachable_sender(&model);
        if let Some((seen, _, rtt)) = &sender {
            rtts.insert(seen.serial.clone(), *rtt);
        }
        let seen = sender.as_ref().map(|(s, _, _)| s);
        let model_lead = u64::from(model.apply_lead_frames);
        let mut usb = UsbNow {
            units: usb_now.clone(),
            lead: model_lead,
        };
        let first = scene::plan_usb(&model, id, &frames, seen, model_lead, &no_alink, &usb)?;
        // -- fixup sw2x (F2) -- each op is a write and a read of the queue.
        let write_rtt = |serial: &str| {
            rtts.get(serial)
                .copied()
                .unwrap_or(DEFAULT_WRITE_RTT)
                .max(MIN_WRITE_RTT)
        };
        let estimate = writes_estimate(first.ops.iter().map(|o| write_rtt(&o.serial)));
        let needed = frames_in(estimate);
        // -- WP FIXM1 -- (review ausb1 F2) the USB-direct writes come after the
        // audio link's: their lead covers both, each op a write and a read-back,
        // and counts from the most recent USB-direct read, not from now.
        let latest_usb_read = usb_now.values().map(|u| u.read_at).max();
        // -- WP AUSB2 -- (review fixm1 F1) USB-direct units whose bus frame could
        // be past when written: their ops go at once instead
        let mut usb_too_far: BTreeSet<String> = BTreeSet::new();
        let plan = if let Some(read_at) = latest_usb_read.filter(|_| !first.usb_ops.is_empty()) {
            let usb_rtts: Vec<Duration> =
                first.usb_ops.iter().map(|o| write_rtt(&o.serial)).collect();
            let mut since_read = reads_began.elapsed().saturating_sub(read_at);
            if usb_ahead(estimate, &usb_rtts, since_read) > scene::USB_MAX_LEAD {
                // Read the USB-direct units again, once: the lead then counts from now.
                let serials: Vec<String> = usb_now.keys().cloned().collect();
                for serial in serials {
                    let seen = self
                        .device_by_serial(&serial)
                        .and_then(|d| timed_usb(&d, reads_began).0);
                    match seen {
                        Some(u) => {
                            usb_now.insert(serial, u);
                        }
                        None => {
                            usb_now.remove(&serial);
                            no_alink.insert(serial);
                        }
                    }
                }
                since_read = usb_now
                    .values()
                    .map(|u| u.read_at)
                    .max()
                    .map_or(Duration::ZERO, |r| reads_began.elapsed().saturating_sub(r));
                if usb_ahead(estimate, &usb_rtts, since_read) > scene::USB_MAX_LEAD {
                    usb_too_far.extend(std::mem::take(&mut usb_now).into_keys());
                    no_alink.extend(usb_too_far.iter().cloned());
                }
                usb.units = usb_now.clone();
            }
            // -- fixup ausb2 (F4) -- in the newest read unit's own samples
            let rate = usb_now
                .values()
                .max_by_key(|u| u.read_at)
                .map_or(48_000, |u| u64::from(u.spf).saturating_mul(1000));
            usb.lead = usb_lead(model_lead, estimate, usb_rtts.into_iter(), since_read, rate);
            scene::plan_usb(
                &model,
                id,
                &frames,
                seen,
                needed.max(model_lead),
                &no_alink,
                &usb,
            )?
        } else if needed > model_lead {
            scene::plan_usb(&model, id, &frames, seen, needed, &no_alink, &usb)?
        } else {
            first
        };
        // -- WP AUSB1 --
        let usb_units: BTreeSet<String> = model
            .units
            .iter()
            .filter(|u| u.usb_direct())
            .map(|u| u.serial.clone())
            .collect();
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
                reason: None,
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
                    reason: None,
                });
                continue;
            };
            let result = if superframe {
                // Never 0 by default: the bit as the sender has it, or the scene's.
                let flags = plan.system_flags.unwrap_or(1);
                match set_ext(
                    &dev,
                    alink::EXT_ALINK_CTRL_MSG,
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
                reason: None,
            });
        }
        // -- WP AUSB1 -- the USB-direct units: one bus frame, sample 0, on each. When the
        // audio link's part was refused whole (a sender holding a scene), they are held
        // back too, so a room never plays half a scene.
        let alink_refused = !plan.ops.is_empty()
            && !writes.iter().any(|w| {
                plan.ops.iter().any(|o| o.serial == w.serial)
                    && matches!(
                        w.result.as_str(),
                        "ok" | "ok_unverified"
                            | "queued"
                            | "queued_unverified"
                            | "applied"
                            | "late"
                    )
            });
        if let (Some(_), true) = (plan.usb_frame, alink_refused) {
            for o in &plan.usb_ops {
                writes.push(WriteResult {
                    serial: o.serial.clone(),
                    element: o.element.clone(),
                    what: o.op.clone(),
                    result: "skipped".into(),
                    reason: Some(USB_HELD_BACK.into()),
                });
            }
        }
        if let Some(bus) = plan.usb_frame.filter(|_| !alink_refused) {
            for o in &plan.usb_ops {
                let spf = usb_now.get(&o.serial).map_or(48, |u| u.spf);
                let req = usb::bus_target(bus, 0, spf).and_then(|t| o.request(t));
                let (Some(req), Some(dev)) = (req, self.device_by_serial(&o.serial)) else {
                    writes.push(WriteResult {
                        serial: o.serial.clone(),
                        element: o.element.clone(),
                        what: o.op.clone(),
                        result: "no_device".into(),
                        reason: None,
                    });
                    continue;
                };
                // -- fixup ausb2 (F6) -- not while the poll or an alignment writes it
                let lock = self.system().unit_lock(&o.serial);
                let _one = super::guard(&lock);
                let result = match set_ext(
                    &dev,
                    alink::EXT_APPLY_AT_FRAME,
                    usb::encode_apply(&req, TimeBase::BusFrame).to_vec(),
                ) {
                    Err(s) => status_name(s),
                    Ok(()) => usb_apply_result(
                        get_ext(
                            &dev,
                            alink::EXT_APPLY_AT_FRAME,
                            ApplyStatus::SIZE,
                            ApplyStatus::decode,
                        ),
                        &req,
                        bus,
                        spf,
                    ),
                };
                // -- WP AUSB2 -- not with the audio link's frame: said
                let capped = plan.usb_lead_capped && !plan.ops.is_empty();
                writes.push(WriteResult {
                    serial: o.serial.clone(),
                    element: o.element.clone(),
                    what: o.op.clone(),
                    result,
                    reason: capped.then(|| USB_LEAD_CAPPED.into()),
                });
            }
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
        // -- WP HWLK1F -- through the sender, a unit of the scene that is not attached
        // is not known to take it: said, so "on time on every unit" is only said when true
        if superframe {
            let mut named = BTreeSet::new();
            for o in plan_targets(&model, &scene, &no_alink) {
                if offline.contains(&o.serial) && named.insert(o.serial.clone()) {
                    writes.push(WriteResult {
                        result: "no_device".into(),
                        reason: Some("not connected".into()),
                        ..o
                    });
                } else if no_frame.contains(&o.serial) && named.insert(o.serial.clone()) {
                    // -- fixup hwlk1f (F1) -- not known to take it: no frame of its own was read
                    writes.push(WriteResult {
                        result: "no_frame".into(),
                        reason: Some("no audio link frame read".into()),
                        ..o
                    });
                }
            }
        }
        for o in &plan.unscheduled {
            writes.push(WriteResult {
                serial: o.serial.clone(),
                element: o.element.clone(),
                what: o.op.clone(),
                result: "no_frame".into(),
                reason: None,
            });
        }
        // -- WP HWLK1F -- units without the audio link: now, with their own commands
        let rate = model.rate_hz;
        for o in &plan.direct {
            let result = match self.device_by_serial(&o.serial) {
                Some(dev) => apply_direct(&dev, o, rate),
                None => "no_device".into(),
            };
            // -- WP AUSB1 -- a USB-direct unit that is not phase-locked, said as such
            let reason = if usb_too_far.contains(&o.serial) {
                // -- WP AUSB2 --
                USB_LEAD_EXCEEDED
            } else if usb_units.contains(&o.serial) {
                "not phase-locked (USB-direct)"
            } else {
                "no audio link (firmware)"
            };
            writes.push(WriteResult {
                serial: o.serial.clone(),
                element: o.element.clone(),
                what: o.op.clone(),
                result,
                reason: Some(reason.into()),
            });
        }
        // A queued op was in its unit's queue before its frame: on time. One
        // whose queue could not be read is on time only if the frame was still
        // ahead after every write. An op applied at once is not at the frame.
        let still_ahead = frame_after_writes.is_some_and(|f| f < plan.frame);
        // -- WP FEAT1 -- the slots are written at once, not at the frame: apart
        let on_time = writes.iter().all(|w| match w.result.as_str() {
            // -- WP AUSB2 -- queued 1.92 s ahead, not at the audio link's frame
            _ if w.reason.as_deref() == Some(USB_LEAD_CAPPED) => false,
            "queued" | "applied" | "ok" | "ok_unverified" => true,
            "queued_unverified" => still_ahead,
            _ => false,
        });
        let mut all = slot_writes;
        all.extend(writes);
        Ok(SceneReport {
            plan,
            writes: all,
            frame_after_writes,
            on_time,
            sources: switched,
        })
    }

    // -- WP FEAT1 --
    /// The `source` ops of `scene`: each zone's source and its units' slots
    /// stored in the model (one change, at the revision read, again on a
    /// conflict), then each unit's slots written now (`EXT_ALINK_SLOTS`; a
    /// unit offline takes them at the next alignment). Nothing is stored or
    /// written when a zone cannot play its source.
    fn switch_sources(
        &self,
        scene: &super::model::Scene,
    ) -> Result<(Vec<SourceSwitched>, Vec<WriteResult>), SystemRefused> {
        use super::model::SCENE_OP_SOURCE;
        let ops: Vec<(String, String)> = scene
            .actions
            .iter()
            .filter(|a| a.op == SCENE_OP_SOURCE)
            .filter_map(|a| {
                let zone = a.target.strip_prefix("zone:")?;
                Some((zone.to_owned(), a.source.clone()?))
            })
            .collect();
        if ops.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let mut tries = 0u8;
        let (model, routed, revision) = loop {
            let (mut model, at) = self.system().model();
            let mut routed: Vec<Routed> = Vec::new();
            for (zone, source) in &ops {
                let slots = model.zone_slots(zone, source).map_err(|e| {
                    SystemRefused::Denied(format!("op source on zone:{zone}: {}", e.0))
                })?;
                for z in model.zones.iter_mut().filter(|z| z.id == *zone) {
                    z.source = Some(source.clone());
                }
                for (unit, want) in &slots {
                    for u in model.units.iter_mut().filter(|u| u.id == *unit) {
                        u.slots = Some(want.clone());
                    }
                }
                routed.push((zone.clone(), source.clone(), slots));
            }
            if model == self.system().model().0 {
                break (model, routed, at);
            }
            match self.system_set_model_at(model.clone(), Some(at)) {
                Ok(rev) => break (model, routed, rev),
                Err(super::ModelRefused::Stale { .. }) if tries < 3 => {
                    tries = tries.saturating_add(1);
                }
                Err(e) => {
                    return Err(SystemRefused::Denied(format!(
                        "the source switch could not be stored: {e}"
                    )));
                }
            }
        };
        let mut writes = Vec::new();
        let mut switched = Vec::new();
        for (zone, source, slots) in routed {
            for (unit, want) in &slots {
                let serial = model
                    .unit(unit)
                    .map(|u| u.serial.clone())
                    .unwrap_or_default();
                let result = match (
                    self.device_by_serial(&serial),
                    <[u8; 8]>::try_from(want.as_slice()),
                ) {
                    (Some(dev), Ok(table)) => {
                        let now = get_ext(
                            &dev,
                            alink::EXT_ALINK_SLOTS,
                            AlinkSlots::SIZE,
                            AlinkSlots::decode,
                        );
                        if now == Some(AlinkSlots(table)) {
                            "unchanged".into()
                        } else {
                            set_ext(&dev, alink::EXT_ALINK_SLOTS, table.to_vec())
                                .map_or_else(status_name, |()| "ok".into())
                        }
                    }
                    (None, _) => "no_device".into(),
                    (_, Err(_)) => "skipped".into(),
                };
                let reason = (result == "no_device")
                    .then(|| "not connected: it takes the slots at the next alignment".into());
                writes.push(WriteResult {
                    serial,
                    element: unit.clone(),
                    what: "slots".into(),
                    result,
                    reason,
                });
            }
            switched.push(SourceSwitched {
                zone,
                source,
                units: slots.into_iter().map(|(u, _)| u).collect(),
                revision,
            });
        }
        Ok((switched, writes))
    }
}

// -- WP FIXM1 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_fixm1 {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    /// The USB-direct lead covers the audio link's writes, then the USB-direct
    /// ops' write and read-back, and the time gone since the latest read.
    #[test]
    fn wp_fixm1_the_usb_lead_counts_its_own_writes_and_the_time_since_the_read() {
        // A long model lead wins over fast writes, shifted by the time since the read.
        assert_eq!(
            usb_lead(9600, LEAD_MARGIN, [MS].into_iter(), Duration::ZERO, 96_000),
            9600
        );
        assert_eq!(
            usb_lead(9600, LEAD_MARGIN, [MS].into_iter(), 10 * MS, 96_000),
            9600 + 960
        );
        // Fast writes over a short model lead: the margin and one write and read-back.
        assert_eq!(
            usb_lead(2400, LEAD_MARGIN, [MS].into_iter(), Duration::ZERO, 96_000),
            frames_in(52 * MS)
        );
        // Slow USB-direct writes (two units at 100 ms): 50 + 2·2·100 ms = 450 ms.
        let alink = writes_estimate(std::iter::empty());
        assert_eq!(alink, LEAD_MARGIN);
        let slow = [100 * MS, 100 * MS];
        assert_eq!(
            usb_lead(2400, alink, slow.into_iter(), Duration::ZERO, 96_000),
            frames_in(450 * MS)
        );
        // The audio link's writes come first and count too.
        let alink = writes_estimate([200 * MS].into_iter());
        assert_eq!(
            usb_lead(2400, alink, slow.into_iter(), 30 * MS, 96_000),
            frames_in(850 * MS) + frames_in(30 * MS)
        );
    }
}

// -- WP AUSB2 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_ausb2 {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    /// The time until the USB-direct ops are written: what decides that a bus
    /// frame could be past (review fixm1 F1), with no model lead in it.
    #[test]
    fn wp_ausb2_usb_ahead_counts_the_writes_and_the_time_since_the_read() {
        let alink = writes_estimate([200 * MS].into_iter());
        assert_eq!(alink, 450 * MS);
        assert_eq!(usb_ahead(alink, &[100 * MS], 30 * MS), 680 * MS);
        // Reads that took 2 s after the USB-direct read: past the 1.92 s reach.
        assert!(usb_ahead(LEAD_MARGIN, &[MS], 2000 * MS) > scene::USB_MAX_LEAD);
        // Read again: from now, well inside it.
        assert!(usb_ahead(LEAD_MARGIN, &[MS], 2 * MS) < scene::USB_MAX_LEAD);
        // Slow writes alone can pass it too: then the ops go at once.
        assert!(usb_ahead(LEAD_MARGIN, &[1000 * MS], Duration::ZERO) > scene::USB_MAX_LEAD);
    }

    // -- fixup ausb2 (F4) --
    /// The lead is counted at the unit's own rate: 1.2 s of reads at 48 kHz is
    /// 57 600 samples, under the 1.92 s cap (92 160), not over it.
    #[test]
    fn wp_ausb2_the_usb_lead_is_in_the_units_own_samples() {
        let lead = usb_lead(2400, LEAD_MARGIN, [MS].into_iter(), 1200 * MS, 48_000);
        assert_eq!(lead, 2496 + 57_600);
        assert!(lead < scene::USB_MAX_LEAD_FRAMES * 48);
        assert_eq!(
            usb_lead(2400, LEAD_MARGIN, [MS].into_iter(), 1200 * MS, 96_000),
            2 * lead
        );
    }
}

// -- WP ATHUBM --
#[cfg(test)]
mod tests_wp_athubm {
    use super::*;
    use dspi_proto::ext::timebase::{self as tb, SpdifMode};

    fn seen_source() -> align::SpdifSeen {
        align::SpdifSeen {
            mode: "source".into(),
            auto_reanchor: true,
            d: 192,
            on_lock_error: "mute".into(),
            tx_mask: 0x05,
            state: "locked".into(),
            ..align::SpdifSeen::default()
        }
    }

    fn plan(mode: &str, d: u16, tx_mask: u8) -> align::SpdifPlan {
        align::SpdifPlan {
            mode: mode.into(),
            source: "SRC".into(),
            d,
            on_lock_error: "play_unaligned".into(),
            tx_mask,
            lag_samples: None,
            writable: true,
        }
    }

    /// Review atab carry-over: a source made a follower is written with its
    /// `tx_mask` zeroed, which the core's codec (and the firmware) take; the
    /// mask of a source is the model's.
    #[test]
    fn wp_athubm_a_source_turned_follower_has_its_mask_zeroed() {
        let want = spdif_wanted(&plan("follower", 150, 0x05), &seen_source());
        assert_eq!(want.mode, SpdifMode::Follower);
        assert_eq!((want.d, want.tx_mask), (150, 0));
        assert_eq!(want.on_lock_error, tb::SPDIF_ON_LOCK_ERROR_PLAY);
        assert!(want.is_valid());
        assert!(!spdif_same(&want, &seen_source()));
        let bytes = want.encode_out();
        assert_eq!(bytes, [1, 1, 150, 0, 1, 0, 0, 0]);

        let want = spdif_wanted(&plan("source", 0, 0x01), &seen_source());
        assert_eq!(
            (want.mode, want.tx_mask, want.d),
            (SpdifMode::Source, 1, 192)
        );
        // A source keeps its own lock-error action and D.
        assert_eq!(want.on_lock_error, tb::SPDIF_ON_LOCK_ERROR_MUTE);
        assert!(want.is_valid());
        let mut same = seen_source();
        same.tx_mask = 1;
        assert!(spdif_same(&want, &same));
    }
}
