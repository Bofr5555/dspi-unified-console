//! The hub family `system.measure.*` (WP ATHUB; PLAN-AT section 1.5,
//! `docs/integrator/hub-api.md` section 23): the units' side of a measurement
//! of several DSPi units with one microphone. The client's core owns the
//! microphone, the plan, the analysis and the documents; the hub reads the
//! units' time bases, stages their generator configs, schedules each unit's
//! op 9 `SIGGEN_START` on the shared counter, watches every unit, and aborts on
//! its own (a dead-man that fails closed).
//!
//! The bodies are the seam's ([`dspi_measure::system::messages`]), never
//! redefined here; device replies travel raw. The measurement never names a
//! transport: a unit is "a unit with a time base" (`EXT_TIME_BASE`).
//!
//! | Message | Who | What |
//! |---|---|---|
//! | `open {units, lease_ms?}` | Control, Admin | takes each unit's device lock for the session, reads `EXT_LIST`, `EXT_TIME_BASE` (twice), `EXT_SIGGEN_TIMELINE`, the outputs; checks G1 (one entry per unit with the group's time base and domain, R7b; the units' `t` agree within the reads' round trips, R7a; one USB bus where known); the lease clamped (R6b) |
//! | `stage {session, serial, config}` | the owner | judges the 36-byte config with `stimulus::check_timed_config` (a finite log sweep, never `RAW`, at most the hard cap, no S/PDIF time-base slot, S-AT6), writes `0xA4` |
//! | `run {session, segments, lead_ms}` | the owner | `t0` = T read now + the lead (at least the largest arm lead + 100 ms + the writes' time, OD-ATn) + whatever the servos still need to settle; each segment's op 9 at `t0 + offset` in each unit's own numbering (USB: S, time base 1, never 2, R5); `bad_schedule` for a unit's segments closer than the lead (R6a), an overlap or a run over 120 s; at most one op 9 pending per unit, the next written once the previous has started and never more than 110 s ahead |
//! | `status {session}` | the owner | renews the lease; per unit `SiggenStatus`, `EXT_SIGGEN_TIMELINE`, the group's `EXT_TIME_BASE` entry, peaks, clip, limiter, events; the time pin `{t, hub_us}` |
//! | `abort {session, reason}` | any Control or Admin principal | `STOP_NOW` and `CANCEL_ALL` on every unit, the broadcast stop where every unit has the ops; locks released; nothing staged can start after it (R4) |
//! | `close {session}` | the owner | `CANCEL_ALL` (and `STOP_NOW` during a run), locks released |
//! | event `aborted {session, unit?, reason}` | to the owner | the hub aborted by itself |
//!
//! **The dead-man** (S-AT1): no `status` within the lease; a unit's contract
//! transition or disturbance (proved by the `disturbances` counter and `t -
//! output_frames`, OD-ATi; `0xE4`/`0xE1`/`0xE3` only make it prompt); a clip
//! event or an output peak at -0.5 dBFS or above; a generator stopped by
//! someone else, a preset load or any change not the session's own; a refused,
//! late or broken start (`EXT_SIGGEN_TIMELINE`); a unit offline or its lock
//! lost (S-AT5); the session's connection closing.
//!
//! **Settling** (FW-B/FW-C review N1, N2): a run's `t0` is never earlier than
//! [`DEFAULT_SETTLE`] after the hub first saw each unit locked with its
//! current disturbance count, so the phase servos have settled.
//!
//! **Served to Link 1.3 clients:** a 1.2-style client (upstream 1.1.6
//! consoles, our older apps) gets no `system_measure` cap and `unsupported`.
//!
//! Locks: the family's table, each session's `io` (held across a session's
//! transfers) above its `st` (a leaf, never held across a transfer or a send).

mod domain;
mod io;
mod schedule;

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use dspi_measure::system::messages::{
    self as wire, AbortReply, AbortRequest, AbortUnit, AbortedEvent, OpenReply, OpenRequest,
    OpenUnit, RunReply, RunRequest, RunSegmentReply, SessionRequest, StageRequest, StatusReply,
    UnitEvent, UnitStatus,
};
use dspi_proto::ext::alink::{self as alink, AlinkEvent, EXT_APPLY_AT_FRAME};
use dspi_proto::ext::alink_usb::{self as usb, UsbEventReason};
use dspi_proto::ext::timebase::{
    self as tbc, SiggenTimeline, TimeBaseEntry, TimeBaseRead, TimeBaseReason,
};
use dspi_proto::generated::notify as nfy;
use dspi_proto::generated::siggen;
use dspi_proto::packets::SiggenConfig;
use serde_json::{Map, Value, json};

use crate::device::DeviceShared;
use crate::hub::{HubHandle, HubInner};
use crate::relay::{PacketTap, Relay};
use crate::session::{Outbound, Role, Session, SessionId};

use domain::{DomainRefusal, Reading};
use io::{Io, Timed};
use schedule::{Due, Seen};

/// How long after the hub first saw a unit locked (with its disturbance count)
/// a run may start: the phase servos' settling (FW-B/FW-C review: about 40 s
/// after first lock).
pub const DEFAULT_SETTLE: Duration = Duration::from_secs(40);
/// The family's thread looks at every session this often.
const TICK: Duration = Duration::from_millis(50);
/// Between runs, every unit is read this often (during a run: every tick).
const IDLE_READ: Duration = Duration::from_millis(250);
/// The device lock is taken for this long and renewed every [`LOCK_RENEW`].
const LOCK_HOLD: Duration = Duration::from_secs(10);
const LOCK_RENEW: Duration = Duration::from_secs(2);
/// Events kept per unit between two `status` calls.
const MAX_EVENTS: usize = 64;
/// An output peak at or above this aborts, dBFS.
const CLIP_DBFS: f32 = -0.5;
/// The broadcast stop's lead on the line, frames (the scenes' default).
const LINE_STOP_LEAD: u64 = 2_400;
/// On S/PDIF: the source's minimum and a block more.
const SPDIF_STOP_LEAD: u64 =
    tbc::SPDIF_CTRL_MIN_LEAD_FRAMES.saturating_add(tbc::SPDIF_BLOCK_FRAMES);
/// `t - output_frames` may move this much between reads (R7c: margin only).
const T_MINUS_OUTPUT_SLACK: u32 = 1;
/// The family's version, `v` in every answer and event (hub-api 1.1).
pub const VERSION: u32 = 1;
/// The most units in one session.
const MAX_UNITS: usize = dspi_measure::system::MAX_UNITS;

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every update under these locks is whole before anything that could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── Answers ─────────────────────────────────────────────────────────────

fn ok(id: Option<u64>, body: Value) -> String {
    let mut m = match body {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    m.insert("t".into(), "ok".into());
    m.insert("v".into(), VERSION.into());
    if let Some(id) = id {
        m.insert("id".into(), id.into());
    }
    Value::Object(m).to_string()
}

fn err(id: Option<u64>, code: &str, msg: &str, serial: Option<&str>) -> String {
    let mut v = json!({"t": "err", "id": id, "v": VERSION, "code": code, "msg": msg});
    if let (Some(s), Some(m)) = (serial, v.as_object_mut()) {
        m.insert("serial".into(), s.into());
    }
    v.to_string()
}

fn ok_of<T: serde::Serialize>(id: Option<u64>, body: &T) -> String {
    match serde_json::to_value(body) {
        Ok(v) => ok(id, v),
        Err(_) => err(id, "internal", "the answer could not be written", None),
    }
}

/// The code for a failed transfer: `device` for the unit's own refusal (and
/// a timeout), as section 1.6 of hub-api has it.
fn transfer_code(s: u8) -> &'static str {
    match io::status_word(s) {
        "offline" => "offline",
        "locked" => "locked",
        "denied" => "denied",
        _ => "device",
    }
}

// ── Entry point ─────────────────────────────────────────────────────────

/// Answer a `system.measure.*` message (`None` for any other), for a
/// connection's reader. Blocks for the units' transfers.
pub fn handle_message(hub: &HubHandle, session: &Arc<Session>, msg: &Value) -> Option<String> {
    let t = msg.get("t")?.as_str()?;
    if !t.starts_with("system.measure.") {
        return None;
    }
    let id = msg.get("id").and_then(Value::as_u64);
    if session.link_minor() < dspi_link::messages::LINK_13_MINOR {
        return Some(err(
            id,
            "unsupported",
            "measuring several units needs a DSPi Link 1.3 client",
            None,
        ));
    }
    if session.role() == Role::Viewer {
        return Some(err(id, "denied", "control role required", None));
    }
    let family = hub.system_measure();
    Some(match t {
        wire::t::OPEN => family.open(hub, session, id, msg),
        wire::t::STAGE => family.stage(session, id, msg),
        wire::t::RUN => family.run(hub, session, id, msg),
        wire::t::STATUS => family.status(hub, session, id, msg),
        wire::t::ABORT => family.abort(hub, session, id, msg),
        wire::t::CLOSE => family.close(hub, session, id, msg),
        _ => err(id, "unknown_type", "", None),
    })
}

/// Whether a session with this Link minor is offered the family
/// (`hello.caps`).
pub fn offered_to(link_minor: u32) -> bool {
    link_minor >= dspi_link::messages::LINK_13_MINOR
}

// ── The family's state ──────────────────────────────────────────────────

/// One unit of a session.
struct MUnit {
    serial: String,
    handle: u8,
    dev: Arc<DeviceShared>,
    /// Every time base its `EXT_TIME_BASE` lists (each queue is cancelled).
    tbs: Vec<u8>,
    /// `EXT_NOTIFY_FEATURES`.
    features: u32,
    /// It has the ops in the superframe: `EXT_ALINK_INFO` bit 6 on the line,
    /// a listed timeline elsewhere.
    siggen_sync: bool,
    /// It is the S/PDIF source of the group's domain.
    spdif_source: bool,
    arm_lead_ms: u16,
    outputs: u8,
    /// Its T minus the domain's.
    origin: i64,
    /// At open: the group entry's counter and `t - output_frames`.
    disturbances: u16,
    t_minus_output: u32,
    /// Outputs that carry the time base to other units (S-AT6).
    forbidden: u16,
    /// The staged config's length, samples.
    staged: Option<u64>,
    events: VecDeque<UnitEvent>,
}

/// One segment of a run.
#[derive(Debug, Clone)]
struct RSeg {
    unit: usize,
    t: u64,
    t_unit: u64,
    len: u64,
    written: bool,
    started: bool,
}

struct SState {
    units: Vec<MUnit>,
    time_base: u8,
    rate: u32,
    lease: Duration,
    last_status: Instant,
    last_lock: Instant,
    last_read: Instant,
    aborted: Option<String>,
    run: Option<Vec<RSeg>>,
    /// An abort the packet tap asks for: the unit and why.
    flagged: Option<(Option<u8>, String)>,
}

struct MSession {
    id: String,
    owner: Arc<Session>,
    /// Held across the session's transfers; above `st`.
    io: Mutex<()>,
    st: Mutex<SState>,
}

/// What the hub first saw of one time base of a unit locked: its domain and
/// disturbance count then.
#[derive(Debug, Clone, Copy)]
struct SeenLocked {
    key: (u32, u16),
    since: Instant,
}

/// The family on one hub.
pub(crate) struct MeasureHub {
    sessions: Mutex<BTreeMap<String, Arc<MSession>>>,
    /// Some session is open: the tap looks at packets.
    active: AtomicBool,
    /// The thread runs.
    running: AtomicBool,
    wake: (Mutex<bool>, Condvar),
    epoch: Instant,
    settle: Mutex<Duration>,
    /// By unit and time base: a unit with two locked entries (R7b) keeps a
    /// clock for each, so noting one never restarts the other's.
    seen: Mutex<BTreeMap<(String, u8), SeenLocked>>,
    next: AtomicU64,
}

/// The families of the hubs in this process, by hub.
static FAMILIES: Mutex<Vec<(Weak<HubInner>, Arc<MeasureHub>)>> = Mutex::new(Vec::new());

impl HubHandle {
    /// This hub's `system.measure.*` family, made at the first use (its
    /// packet tap added then).
    pub(crate) fn system_measure(&self) -> Arc<MeasureHub> {
        let mut all = guard(&FAMILIES);
        all.retain(|(w, _)| w.strong_count() > 0);
        if let Some((_, f)) = all
            .iter()
            .find(|(w, _)| std::ptr::eq(w.as_ptr(), Arc::as_ptr(&self.inner)))
        {
            return Arc::clone(f);
        }
        let family = Arc::new(MeasureHub {
            sessions: Mutex::new(BTreeMap::new()),
            active: AtomicBool::new(false),
            running: AtomicBool::new(false),
            wake: (Mutex::new(false), Condvar::new()),
            epoch: Instant::now(),
            settle: Mutex::new(DEFAULT_SETTLE),
            seen: Mutex::new(BTreeMap::new()),
            next: AtomicU64::new(1),
        });
        self.inner
            .relay
            .add_tap(Arc::new(Tap(Arc::downgrade(&family))));
        all.push((Arc::downgrade(&self.inner), Arc::clone(&family)));
        family
    }

    /// How long after the hub first sees a unit locked a measurement run may
    /// start ([`DEFAULT_SETTLE`]; tests and an embedder that knows its units
    /// have been locked for long may shorten it).
    pub fn set_system_measure_settle(&self, settle: Duration) {
        *guard(&self.system_measure().settle) = settle;
    }

    /// The open `system.measure` sessions (aborted ones until closed).
    pub fn system_measure_sessions(&self) -> usize {
        guard(&self.system_measure().sessions).len()
    }

    /// Whether the online unit `serial`'s device lock is a measuring
    /// session's, exempt from the lock budget.
    pub fn system_measure_lock_exempt(&self, serial: &str) -> bool {
        self.device_by_serial(serial)
            .is_some_and(|d| d.router.lock_holder_exempt(Instant::now()))
    }

    /// Note a read of unit `serial`'s `EXT_TIME_BASE` entry made elsewhere in
    /// the hub (the system model's poll, WP ATHUBM, at the batch merge): a
    /// locked one starts or keeps its settling clock, so a run need not
    /// wait the full [`DEFAULT_SETTLE`] after the hub starts.
    pub(crate) fn system_measure_note_locked(&self, serial: &str, entry: &TimeBaseEntry) {
        if entry.state.is_locked() {
            self.system_measure()
                .note_locked(serial, entry, Instant::now());
        }
    }
}

/// A plain copy of what a transfer on a unit needs, taken under `st`.
#[derive(Clone)]
struct URef {
    index: usize,
    serial: String,
    handle: u8,
    dev: Arc<DeviceShared>,
    tbs: Vec<u8>,
    siggen_sync: bool,
    spdif_source: bool,
    origin: i64,
    arm_lead_ms: u16,
    outputs: u8,
    disturbances: u16,
    t_minus_output: u32,
}

fn urefs(st: &SState) -> Vec<URef> {
    st.units
        .iter()
        .enumerate()
        .map(|(index, u)| URef {
            index,
            serial: u.serial.clone(),
            handle: u.handle,
            dev: Arc::clone(&u.dev),
            tbs: u.tbs.clone(),
            siggen_sync: u.siggen_sync,
            spdif_source: u.spdif_source,
            origin: u.origin,
            arm_lead_ms: u.arm_lead_ms,
            outputs: u.outputs,
            disturbances: u.disturbances,
            t_minus_output: u.t_minus_output,
        })
        .collect()
}

fn unit_byte(i: usize) -> u8 {
    u8::try_from(i).unwrap_or(u8::MAX)
}

/// `t` in the unit's numbering to the domain's.
fn to_domain(t: u64, origin: i64) -> u64 {
    if origin >= 0 {
        t.saturating_sub(origin.unsigned_abs())
    } else {
        t.saturating_add(origin.unsigned_abs())
    }
}

/// `t` in the domain's numbering to the unit's.
fn to_unit(t: u64, origin: i64) -> u64 {
    to_domain(t, origin.saturating_neg())
}

fn samples_in(d: Duration, rate: u32) -> u64 {
    domain::samples_in_us(u64::try_from(d.as_micros()).unwrap_or(u64::MAX), rate)
}

/// The group's entry of a unit's read.
fn group_entry(read: &TimeBaseRead, tb: u8) -> Option<&TimeBaseEntry> {
    read.entries.iter().find(|e| e.time_base == tb)
}

/// What a unit's read says against what the session opened with: `None`
/// when nothing moved (G2: `LOCKED`, the counter unchanged, `t -
/// output_frames` within [`T_MINUS_OUTPUT_SLACK`]).
fn moved(read: &TimeBaseRead, tb: u8, u: &URef) -> Option<String> {
    let Some(e) = group_entry(read, tb) else {
        return Some("its time base entry is gone".into());
    };
    if !e.state.is_locked() {
        return Some(format!("its time base is {}", e.state.as_str()));
    }
    if e.disturbances != u.disturbances {
        return Some(format!(
            "its time base was disturbed ({} -> {}, last {})",
            u.disturbances,
            e.disturbances,
            e.last_reason.as_str()
        ));
    }
    let now = read.t_minus_output(e);
    let d = now.wrapping_sub(u.t_minus_output);
    if d > T_MINUS_OUTPUT_SLACK && d < T_MINUS_OUTPUT_SLACK.wrapping_neg() {
        return Some("its timeline moved against its outputs".into());
    }
    None
}

impl MeasureHub {
    fn micros_since_epoch(&self, at: Instant) -> u64 {
        u64::try_from(at.saturating_duration_since(self.epoch).as_micros()).unwrap_or(u64::MAX)
    }

    fn mid_us(&self, sent: Instant, got: Instant) -> (u64, u64) {
        let a = self.micros_since_epoch(sent);
        let b = self.micros_since_epoch(got);
        let rtt = b.saturating_sub(a);
        (a.saturating_add(rtt.saturating_div(2)), rtt)
    }

    fn wake(&self) {
        let (m, cv) = &self.wake;
        *guard(m) = true;
        cv.notify_all();
    }

    fn session(&self, id: &str) -> Option<Arc<MSession>> {
        guard(&self.sessions).get(id).cloned()
    }

    /// The session `req` names, if `who` owns it.
    fn owned(
        &self,
        who: &Session,
        session: &str,
        id: Option<u64>,
    ) -> Result<Arc<MSession>, String> {
        let s = self
            .session(session)
            .ok_or_else(|| err(id, "bad_request", "no such measurement session", None))?;
        if s.owner.id() != who.id() {
            return Err(err(
                id,
                "denied",
                "another console's measurement session",
                None,
            ));
        }
        Ok(s)
    }

    /// Note a locked read of unit `serial`; when the hub first saw it so.
    fn note_locked(&self, serial: &str, e: &TimeBaseEntry, at: Instant) -> Instant {
        let key = (e.domain, e.disturbances);
        let at_key = (serial.to_owned(), e.time_base);
        let mut seen = guard(&self.seen);
        match seen.get(&at_key) {
            Some(s) if s.key == key => s.since,
            _ => {
                seen.insert(at_key, SeenLocked { key, since: at });
                at
            }
        }
    }

    /// How long until every unit's time base `tb` has settled.
    fn settle_left(&self, serials: &[String], tb: u8, now: Instant) -> Duration {
        let settle = *guard(&self.settle);
        let seen = guard(&self.seen);
        serials
            .iter()
            .map(|s| {
                seen.get(&(s.clone(), tb)).map_or(settle, |x| {
                    x.since
                        .checked_add(settle)
                        .map_or(Duration::ZERO, |end| end.saturating_duration_since(now))
                })
            })
            .max()
            .unwrap_or(Duration::ZERO)
    }

    // ── open ──────────────────────────────────────────────────────────

    fn open(
        self: &Arc<Self>,
        hub: &HubHandle,
        who: &Arc<Session>,
        id: Option<u64>,
        msg: &Value,
    ) -> String {
        let req: OpenRequest = match serde_json::from_value(msg.clone()) {
            Ok(r) => r,
            Err(e) => return err(id, "bad_request", &e.to_string(), None),
        };
        if req.units.is_empty() || req.units.len() > MAX_UNITS {
            return err(id, "bad_request", "1 to 16 units", None);
        }
        for (i, s) in req.units.iter().enumerate() {
            if s.is_empty() || req.units.iter().take(i).any(|o| o == s) {
                return err(id, "bad_request", "each unit once, by serial", None);
            }
        }
        // Already measured in another session.
        {
            let sessions = guard(&self.sessions);
            for s in sessions.values() {
                let st = guard(&s.st);
                if st.aborted.is_none()
                    && let Some(u) = st.units.iter().find(|u| req.units.contains(&u.serial))
                {
                    return err(
                        id,
                        "locked",
                        "the unit is in another measurement session",
                        Some(&u.serial),
                    );
                }
            }
        }
        let mut devs = Vec::with_capacity(req.units.len());
        for serial in &req.units {
            let Some(dev) = hub.device_by_serial(serial) else {
                return err(id, "offline", "the unit is not online", Some(serial));
            };
            if hub.inner.limits.locked_for(serial, who.id(), who.role()) {
                return err(
                    id,
                    "installer_lock",
                    "the unit is locked by its installer",
                    Some(serial),
                );
            }
            devs.push(dev);
        }
        // The device locks, for the session (S-AT5).
        let mut taken = Vec::new();
        for (serial, dev) in req.units.iter().zip(&devs) {
            if hub.lock_request(who, dev.handle, LOCK_HOLD).is_err() {
                for h in &taken {
                    hub.lock_release(who, *h);
                }
                return err(id, "locked", "another console holds the unit", Some(serial));
            }
            // The session's own lock leaves the router's budget while its lease
            // lives: the dead-man bounds it (coordinator, AT-C). Every other
            // lock keeps the budget.
            dev.router.lock_exempt(who.id(), Instant::now());
            taken.push(dev.handle);
        }
        let release = |answer: String| {
            for h in &taken {
                hub.lock_release(who, *h);
            }
            answer
        };
        match self.open_reads(hub, who, &req, &devs) {
            Ok((units, reply_units, time_base, domain, rate)) => {
                let now = Instant::now();
                let lease = Duration::from_millis(u64::from(wire::clamp_lease_ms(req.lease_ms)));
                let n = self.next.fetch_add(1, Ordering::SeqCst);
                let sid = format!(
                    "m{n}-{:08x}",
                    (self.micros_since_epoch(now) as u32) ^ u32::from(who.id()).wrapping_shl(16)
                );
                let session = Arc::new(MSession {
                    id: sid.clone(),
                    owner: Arc::clone(who),
                    io: Mutex::new(()),
                    st: Mutex::new(SState {
                        units,
                        time_base,
                        rate,
                        lease,
                        last_status: now,
                        last_lock: now,
                        last_read: now,
                        aborted: None,
                        run: None,
                        flagged: None,
                    }),
                });
                let reply = OpenReply {
                    session: sid.clone(),
                    time_base,
                    domain,
                    lease_ms: u32::try_from(lease.as_millis()).unwrap_or(u32::MAX),
                    units: reply_units,
                };
                {
                    let mut sessions = guard(&self.sessions);
                    sessions.insert(sid, session);
                    self.active.store(true, Ordering::SeqCst);
                    if !self.running.swap(true, Ordering::SeqCst) {
                        self.spawn(hub);
                    }
                }
                ok_of(id, &reply)
            }
            Err((code, why, serial)) => release(err(id, code, &why, serial.as_deref())),
        }
    }

    #[allow(clippy::type_complexity)]
    fn open_reads(
        &self,
        hub: &HubHandle,
        who: &Session,
        req: &OpenRequest,
        devs: &[Arc<DeviceShared>],
    ) -> Result<(Vec<MUnit>, Vec<OpenUnit>, u8, u32, u32), (&'static str, String, Option<String>)>
    {
        struct First {
            list: Vec<u16>,
            features: u32,
            timeline: SiggenTimeline,
            tb: Timed<TimeBaseRead>,
            outputs: u8,
        }
        let fail = |code: &'static str, why: String, serial: &str| {
            Err((code, why, Some(serial.to_owned())))
        };
        let mut firsts = Vec::with_capacity(devs.len());
        for (serial, dev) in req.units.iter().zip(devs) {
            let io = Io {
                dev,
                session: who.id(),
            };
            let list = match io.ext_list() {
                Ok(l) => l,
                Err(s) if s == dspi_link::status::ERROR => Vec::new(),
                Err(s) => return fail(transfer_code(s), "EXT_LIST was not read".into(), serial),
            };
            let has = |x: u16| list.contains(&x);
            if !(has(tbc::EXT_TIME_BASE)
                && has(tbc::EXT_SIGGEN_TIMELINE)
                && has(EXT_APPLY_AT_FRAME))
            {
                return fail(
                    wire::code::NO_TIMELINE,
                    "the unit's firmware has no generator start on a time base".into(),
                    serial,
                );
            }
            let features = io.notify_features();
            let timeline = match io.timeline() {
                Ok(t) => t.value,
                Err(s) => return fail(transfer_code(s), "no EXT_SIGGEN_TIMELINE".into(), serial),
            };
            let tb = match io.time_base() {
                Ok(t) => t,
                Err(s) => return fail(transfer_code(s), "no EXT_TIME_BASE".into(), serial),
            };
            let outputs = outputs_of(hub, serial);
            firsts.push(First {
                list,
                features,
                timeline,
                tb,
                outputs,
            });
        }
        // The second reads, back to back (one tick, R7a), at least the rate
        // span after the first.
        if let Some(first) = firsts.first() {
            let span = Duration::from_micros(u64::from(domain::RATE_SPAN_US).saturating_add(2_000));
            let wait = first.tb.got.checked_add(span).map_or(Duration::ZERO, |t| {
                t.saturating_duration_since(Instant::now())
            });
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
        }
        let mut seconds = Vec::with_capacity(devs.len());
        for (serial, dev) in req.units.iter().zip(devs) {
            let io = Io {
                dev,
                session: who.id(),
            };
            match io.time_base() {
                Ok(t) => seconds.push(t),
                Err(s) => return fail(transfer_code(s), "no EXT_TIME_BASE".into(), serial),
            }
        }
        let entries: Vec<&[TimeBaseEntry]> =
            seconds.iter().map(|t| t.value.entries.as_slice()).collect();
        let group = match domain::choose_group(&entries) {
            Ok(g) => g,
            Err(DomainRefusal::NotLocked(i)) => {
                let serial = req.units.get(i).map_or("", String::as_str);
                return fail(
                    wire::code::NOT_LOCKED,
                    "the unit's time base is not locked".into(),
                    serial,
                );
            }
            Err(DomainRefusal::NotOneDomain(why)) => {
                return Err((wire::code::NOT_ONE_DOMAIN, why, None));
            }
        };
        let tb = group.time_base;
        // Each unit's rate from its two reads, and nothing moved between them.
        let mut rate = None;
        for ((serial, f), s) in req.units.iter().zip(&firsts).zip(&seconds) {
            let (Some(a), Some(b)) = (group_entry(&f.tb.value, tb), group_entry(&s.value, tb))
            else {
                return fail(
                    wire::code::NOT_LOCKED,
                    "the unit's time base changed".into(),
                    serial,
                );
            };
            if !a.state.is_locked()
                || a.disturbances != b.disturbances
                || f.tb.value.t_minus_output(a) != s.value.t_minus_output(b)
            {
                return fail(
                    wire::code::NOT_LOCKED,
                    "the unit's time base moved while it was read".into(),
                    serial,
                );
            }
            let r = domain::rate_of((a.t, f.tb.value.time_us), (b.t, s.value.time_us));
            match (r, rate) {
                (None, _) => {
                    return Err((
                        wire::code::NOT_ONE_DOMAIN,
                        format!("unit {serial}'s rate is not one a measurement runs at"),
                        Some(serial.clone()),
                    ));
                }
                (Some(r), Some(prev)) if r != prev => {
                    return Err((
                        wire::code::NOT_ONE_DOMAIN,
                        "the units run at different rates".into(),
                        None,
                    ));
                }
                (Some(r), _) => rate = Some(r),
            }
        }
        let rate = rate.unwrap_or(48_000);
        let readings: Vec<Reading> = seconds
            .iter()
            .map(|s| {
                let (mid_us, rtt_us) = self.mid_us(s.sent, s.got);
                Reading {
                    t: group_entry(&s.value, tb).map_or(0, |e| e.t),
                    mid_us,
                    rtt_us,
                }
            })
            .collect();
        let origins = match domain::origins(tb, rate, &readings) {
            Ok(o) => o,
            Err(DomainRefusal::NotOneDomain(why)) => {
                return Err((wire::code::NOT_ONE_DOMAIN, why, None));
            }
            Err(DomainRefusal::NotLocked(i)) => {
                let serial = req.units.get(i).map_or("", String::as_str);
                return fail(
                    wire::code::NOT_LOCKED,
                    "the unit's time base is not locked".into(),
                    serial,
                );
            }
        };
        if tb == tbc::TB_USB_SAMPLE {
            let buses: Vec<Option<String>> =
                req.units.iter().map(|s| crate::usb::bus_of(s)).collect();
            if !domain::same_bus(&buses) {
                return Err((
                    wire::code::NOT_ONE_DOMAIN,
                    "the USB-direct units are on different host buses: these units do not share \
                     a clock"
                        .into(),
                    None,
                ));
            }
        }
        let mut units = Vec::with_capacity(devs.len());
        let mut reply = Vec::with_capacity(devs.len());
        for (i, (((serial, dev), f), s)) in req
            .units
            .iter()
            .zip(devs)
            .zip(firsts)
            .zip(&seconds)
            .enumerate()
        {
            let Some(e) = group_entry(&s.value, tb) else {
                return fail(
                    wire::code::NOT_LOCKED,
                    "the unit's time base changed".into(),
                    serial,
                );
            };
            self.note_locked(serial, e, s.got);
            let io = Io {
                dev,
                session: who.id(),
            };
            let siggen_sync = if tb == tbc::TB_ALINK_LINE {
                io.alink_info()
                    .is_some_and(|i| i.feature_bits & tbc::FEATURE_SIGGEN_SYNC != 0)
            } else {
                true
            };
            let forbidden = if f.list.contains(&tbc::EXT_ALINK_SPDIF) {
                spdif_forbidden(&io, f.outputs)
            } else {
                0
            };
            let origin = origins.get(i).copied().unwrap_or(0);
            let t_minus_output = s.value.t_minus_output(e);
            reply.push(OpenUnit {
                serial: serial.clone(),
                time_base: tb,
                role: e.role.raw(),
                state: e.state.raw(),
                d: e.d,
                arm_lead_ms: f.timeline.arm_lead_ms,
                outputs: f.outputs,
                t: e.t,
                origin_offset: origin,
                t_minus_output,
                disturbances: e.disturbances,
            });
            units.push(MUnit {
                serial: serial.clone(),
                handle: dev.handle,
                dev: Arc::clone(dev),
                tbs: s.value.entries.iter().map(|e| e.time_base).collect(),
                features: f.features,
                siggen_sync,
                spdif_source: tb == tbc::TB_SPDIF && e.role == tbc::TimeBaseRole::Source,
                arm_lead_ms: f.timeline.arm_lead_ms,
                outputs: f.outputs,
                origin,
                disturbances: e.disturbances,
                t_minus_output,
                forbidden,
                staged: None,
                events: VecDeque::new(),
            });
        }
        Ok((units, reply, tb, group.domain, rate))
    }

    // ── stage ─────────────────────────────────────────────────────────

    fn stage(&self, who: &Session, id: Option<u64>, msg: &Value) -> String {
        let req: StageRequest = match serde_json::from_value(msg.clone()) {
            Ok(r) => r,
            Err(e) => return err(id, "bad_request", &e.to_string(), None),
        };
        let s = match self.owned(who, &req.session, id) {
            Ok(s) => s,
            Err(e) => return e,
        };
        let _io = guard(&s.io);
        let (u, rate) = {
            let st = guard(&s.st);
            if let Some(why) = &st.aborted {
                return err(id, "aborted", why, None);
            }
            if st.run.is_some() {
                return err(id, "bad_request", "a run is in progress", None);
            }
            let Some(u) = urefs(&st).into_iter().find(|u| u.serial == req.serial) else {
                return err(
                    id,
                    "bad_request",
                    "the unit is not in this session",
                    Some(&req.serial),
                );
            };
            (u, st.rate)
        };
        let forbidden = guard(&s.st)
            .units
            .get(u.index)
            .map_or(u16::MAX, |x| x.forbidden);
        let cfg = match (req.config.len() == SiggenConfig::SIZE)
            .then(|| SiggenConfig::decode(&req.config).ok())
            .flatten()
        {
            Some(c) => c,
            None => {
                return err(
                    id,
                    wire::code::BAD_CONFIG,
                    "a config is 36 bytes",
                    Some(&u.serial),
                );
            }
        };
        let outputs_mask = 1u16
            .checked_shl(u32::from(u.outputs))
            .map_or(u16::MAX, |b| b.saturating_sub(1));
        let checked = dspi_measure::stimulus::check_timed_config(&cfg, forbidden)
            .map_err(|e| e.to_string())
            .and_then(|()| {
                if cfg.channel_mask & !outputs_mask != 0 {
                    Err("an output the unit does not have".into())
                } else {
                    Ok(())
                }
            })
            .and_then(|()| schedule::segment_samples(&cfg, rate));
        let len = match checked {
            Ok(l) => l,
            Err(why) => return err(id, wire::code::BAD_CONFIG, &why, Some(&u.serial)),
        };
        let io = Io {
            dev: &u.dev,
            session: who.id(),
        };
        if let Err(st) = io.set_config(&cfg) {
            return err(
                id,
                transfer_code(st),
                "the unit did not take the config",
                Some(&u.serial),
            );
        }
        if let Some(x) = guard(&s.st).units.get_mut(u.index) {
            x.staged = Some(len);
        }
        ok(id, json!({"serial": u.serial, "samples": len}))
    }

    // ── run ───────────────────────────────────────────────────────────

    fn run(&self, hub: &HubHandle, who: &Session, id: Option<u64>, msg: &Value) -> String {
        let req: RunRequest = match serde_json::from_value(msg.clone()) {
            Ok(r) => r,
            Err(e) => return err(id, "bad_request", &e.to_string(), None),
        };
        let s = match self.owned(who, &req.session, id) {
            Ok(s) => s,
            Err(e) => return e,
        };
        if req.lead_ms > schedule::MAX_LEAD_MS {
            return err(id, wire::code::BAD_SCHEDULE, "a lead of at most 10 s", None);
        }
        let _io = guard(&s.io);
        let (refs, tb, rate, lens) = {
            let st = guard(&s.st);
            if let Some(why) = &st.aborted {
                return err(id, "aborted", why, None);
            }
            if st.run.is_some() {
                return err(id, "bad_request", "a run is in progress", None);
            }
            let lens: Vec<Option<u64>> = st.units.iter().map(|u| u.staged).collect();
            (urefs(&st), st.time_base, st.rate, lens)
        };
        // Read every unit of the run now: nothing moved, and T.
        let mut used: Vec<usize> = req.segments.iter().map(|x| usize::from(x.unit)).collect();
        used.sort_unstable();
        used.dedup();
        if used.iter().any(|u| *u >= refs.len()) {
            return err(
                id,
                wire::code::BAD_SCHEDULE,
                "a unit not in this session",
                None,
            );
        }
        let mut newest: Option<(u64, Instant)> = None;
        let mut max_rtt = Duration::ZERO;
        for u in refs.iter().filter(|u| used.contains(&u.index)) {
            let io = Io {
                dev: &u.dev,
                session: who.id(),
            };
            let read = match io.time_base() {
                Ok(r) => r,
                Err(st) => {
                    return err(
                        id,
                        transfer_code(st),
                        "the unit was not read",
                        Some(&u.serial),
                    );
                }
            };
            if let Some(why) = moved(&read.value, tb, u) {
                self.abort_session(hub, &s, Some(u.index), "disturbance", true);
                return err(
                    id,
                    "aborted",
                    &format!("unit {}: {why}", u.serial),
                    Some(&u.serial),
                );
            }
            if let Some(e) = group_entry(&read.value, tb) {
                self.note_locked(&u.serial, e, read.got);
                newest = Some((to_domain(e.t, u.origin), read.got));
            }
            max_rtt = max_rtt.max(read.got.saturating_duration_since(read.sent));
        }
        let Some((t_now, at)) = newest else {
            return err(id, wire::code::BAD_SCHEDULE, "no segment", None);
        };
        // The lead: the client's, or the arm leads + 100 ms + the writes.
        let max_arm = refs
            .iter()
            .filter(|u| used.contains(&u.index))
            .map(|u| u.arm_lead_ms)
            .max()
            .unwrap_or(0);
        let writes = max_rtt
            .saturating_mul(2)
            .saturating_mul(u32::try_from(used.len()).unwrap_or(u32::MAX));
        let lead = Duration::from_millis(schedule::lead_ms(
            req.lead_ms,
            max_arm,
            u64::try_from(writes.as_millis()).unwrap_or(u64::MAX),
        ));
        if let Err(why) =
            schedule::check_segments(&req.segments, &lens, samples_in(lead, rate), rate)
        {
            return err(id, wire::code::BAD_SCHEDULE, &why, None);
        }
        let now = Instant::now();
        let serials: Vec<String> = refs
            .iter()
            .filter(|u| used.contains(&u.index))
            .map(|u| u.serial.clone())
            .collect();
        let settle = self.settle_left(&serials, tb, now);
        let t0 = t_now
            .saturating_add(samples_in(now.saturating_duration_since(at), rate))
            .saturating_add(samples_in(lead, rate))
            .saturating_add(samples_in(settle, rate));
        let mut segs: Vec<RSeg> = req
            .segments
            .iter()
            .map(|x| {
                let unit = usize::from(x.unit);
                let t = t0.saturating_add(x.offset);
                RSeg {
                    unit,
                    t,
                    t_unit: to_unit(t, refs.get(unit).map_or(0, |u| u.origin)),
                    len: lens.get(unit).copied().flatten().unwrap_or(0),
                    written: false,
                    started: false,
                }
            })
            .collect();
        // Each unit's first op 9, now (when within 110 s).
        let mut failure = None;
        for u in refs.iter().filter(|u| used.contains(&u.index)) {
            let Some(k) = segs.iter().position(|x| x.unit == u.index) else {
                continue;
            };
            let Some(seg) = segs.get(k) else { continue };
            let elapsed = Instant::now().saturating_duration_since(at);
            let now_unit = to_unit(t_now, u.origin).saturating_add(samples_in(elapsed, rate));
            match schedule::op9_due(seg.t_unit, now_unit, u.arm_lead_ms, rate) {
                Due::Wait => {}
                Due::Late => {
                    failure = Some((u.clone(), "the start would be late".to_string()));
                    break;
                }
                Due::Write => {
                    let io = Io {
                        dev: &u.dev,
                        session: who.id(),
                    };
                    let t_unit = seg.t_unit;
                    let written = io.siggen_start_at(t_unit, tb).and_then(|()| io.timeline());
                    match written.map(|tl| schedule::judge_timeline(&tl.value, t_unit)) {
                        Ok(Seen::Pending | Seen::Started { .. }) => {
                            if let Some(x) = segs.get_mut(k) {
                                x.written = true;
                            }
                        }
                        Ok(Seen::Failed(w)) => {
                            failure =
                                Some((u.clone(), format!("the unit refused the start ({w})")));
                            break;
                        }
                        Err(st) => {
                            failure = Some((
                                u.clone(),
                                format!("the unit refused the start ({})", io::status_word(st)),
                            ));
                            break;
                        }
                    }
                }
            }
        }
        if let Some((u, why)) = failure {
            // Nothing of this run may start; what was staged is spent (R4).
            for r in &refs {
                let io = Io {
                    dev: &r.dev,
                    session: who.id(),
                };
                for t in &r.tbs {
                    let _ = io.cancel_all(*t);
                }
            }
            for x in &mut guard(&s.st).units {
                x.staged = None;
            }
            return err(
                id,
                "device",
                &format!("{why}; nothing of the run starts: stage again"),
                Some(&u.serial),
            );
        }
        let reply = RunReply {
            t0,
            segments: segs
                .iter()
                .map(|x| RunSegmentReply {
                    unit: unit_byte(x.unit),
                    t: x.t,
                    t_unit: x.t_unit,
                })
                .collect(),
        };
        guard(&s.st).run = Some(segs);
        self.wake();
        ok_of(id, &reply)
    }

    // ── status ────────────────────────────────────────────────────────

    fn status(&self, hub: &HubHandle, who: &Session, id: Option<u64>, msg: &Value) -> String {
        let req: SessionRequest = match serde_json::from_value(msg.clone()) {
            Ok(r) => r,
            Err(e) => return err(id, "bad_request", &e.to_string(), None),
        };
        let s = match self.owned(who, &req.session, id) {
            Ok(s) => s,
            Err(e) => return e,
        };
        let (refs, tb) = {
            let mut st = guard(&s.st);
            if let Some(why) = &st.aborted {
                return err(id, "aborted", why, None);
            }
            st.last_status = Instant::now();
            (urefs(&st), st.time_base)
        };
        let _io = guard(&s.io);
        let mut units = Vec::with_capacity(refs.len());
        let mut pin: Option<(u64, u64)> = None;
        for u in &refs {
            let io = Io {
                dev: &u.dev,
                session: who.id(),
            };
            let siggen = io.siggen_status().unwrap_or_default();
            let timeline = io.timeline().map(|t| t.raw).unwrap_or_default();
            let read = match io.time_base() {
                Ok(r) => r,
                Err(st) => {
                    let reason = if st == dspi_link::status::NO_DEVICE {
                        "offline"
                    } else {
                        "disturbance"
                    };
                    self.abort_session(hub, &s, Some(u.index), reason, true);
                    return err(id, "aborted", "the unit was not read", Some(&u.serial));
                }
            };
            if let Some(why) = moved(&read.value, tb, u) {
                self.abort_session(hub, &s, Some(u.index), "disturbance", true);
                return err(
                    id,
                    "aborted",
                    &format!("unit {}: {why}", u.serial),
                    Some(&u.serial),
                );
            }
            let at = read
                .value
                .entries
                .iter()
                .position(|e| e.time_base == tb)
                .unwrap_or(0);
            let off = tbc::TIME_BASE_ENTRY_OFFSET
                .saturating_add(at.saturating_mul(tbc::TIME_BASE_ENTRY_LEN));
            let entry_raw = read
                .raw
                .get(off..off.saturating_add(tbc::TIME_BASE_ENTRY_LEN))
                .map(<[u8]>::to_vec)
                .unwrap_or_default();
            if pin.is_none()
                && let Some(e) = group_entry(&read.value, tb)
            {
                let (mid, _) = self.mid_us(read.sent, read.got);
                pin = Some((to_domain(e.t, u.origin), mid));
            }
            let (peaks, flags) = io
                .meters()
                .ok()
                .and_then(|b| io::decode_meters(&b, u.outputs))
                .unwrap_or_default();
            if peaks.iter().any(|p| *p >= CLIP_DBFS) {
                self.abort_session(hub, &s, Some(u.index), "clip", true);
                return err(
                    id,
                    "aborted",
                    "an output reached -0.5 dBFS",
                    Some(&u.serial),
                );
            }
            let limiter = io.limiter_gr(u.outputs);
            let events = guard(&s.st)
                .units
                .get_mut(u.index)
                .map(|x| x.events.drain(..).collect())
                .unwrap_or_default();
            units.push(UnitStatus {
                unit: unit_byte(u.index),
                siggen,
                timeline,
                time_base_entry: entry_raw,
                output_frames: read.value.output_frames,
                output_peaks_dbfs: peaks,
                clipped: flags != 0,
                limiter_gr_db: limiter,
                events,
            });
        }
        let (t, hub_us) = pin.unwrap_or((0, self.micros_since_epoch(Instant::now())));
        // Under `status`: the body's `t` (the time pin) is not the answer's `t`.
        match serde_json::to_value(StatusReply { t, hub_us, units }) {
            Ok(body) => ok(id, json!({ "status": body })),
            Err(_) => err(id, "internal", "the answer could not be written", None),
        }
    }

    // ── abort, close ──────────────────────────────────────────────────

    fn abort(&self, hub: &HubHandle, who: &Session, id: Option<u64>, msg: &Value) -> String {
        let req: AbortRequest = match serde_json::from_value(msg.clone()) {
            Ok(r) => r,
            Err(e) => return err(id, "bad_request", &e.to_string(), None),
        };
        let Some(s) = self.session(&req.session) else {
            return err(id, "bad_request", "no such measurement session", None);
        };
        log::info!(
            "hub: measurement {} aborted by session {}: {}",
            s.id,
            who.id(),
            req.reason.chars().take(200).collect::<String>()
        );
        let _io = guard(&s.io);
        let (units, broadcast) = self.abort_session(hub, &s, None, "abort", false);
        let mut v = serde_json::to_value(AbortReply { units }).unwrap_or(Value::Null);
        if let Some(m) = v.as_object_mut() {
            m.insert("broadcast".into(), broadcast.into());
        }
        ok(id, v)
    }

    fn close(&self, hub: &HubHandle, who: &Session, id: Option<u64>, msg: &Value) -> String {
        let req: SessionRequest = match serde_json::from_value(msg.clone()) {
            Ok(r) => r,
            Err(e) => return err(id, "bad_request", &e.to_string(), None),
        };
        let s = match self.owned(who, &req.session, id) {
            Ok(s) => s,
            Err(e) => return e,
        };
        let _io = guard(&s.io);
        self.close_session(hub, &s);
        ok(id, json!({}))
    }

    /// `CANCEL_ALL` (and `STOP_NOW` during a run) on every unit, the locks
    /// released, the session gone. Called with `s.io` held.
    fn close_session(&self, hub: &HubHandle, s: &Arc<MSession>) {
        let (refs, live, running) = {
            let st = guard(&s.st);
            (urefs(&st), st.aborted.is_none(), st.run.is_some())
        };
        if live {
            for u in &refs {
                let io = Io {
                    dev: &u.dev,
                    session: s.owner.id(),
                };
                if running {
                    let _ = io.stop_now();
                }
                for t in &u.tbs {
                    let _ = io.cancel_all(*t);
                }
                hub.lock_release(&s.owner, u.handle);
            }
        }
        let mut sessions = guard(&self.sessions);
        sessions.remove(&s.id);
        if sessions.is_empty() {
            self.active.store(false, Ordering::SeqCst);
        }
    }

    /// Stop the session's units: `STOP_NOW` and `CANCEL_ALL` on each, the
    /// broadcast stop where it may go; the locks released; the session
    /// aborted (`hub_side`: the hub's own, `aborted` sent to the owner).
    /// Called with `s.io` held. What each unit did, and the broadcast's path.
    fn abort_session(
        &self,
        hub: &HubHandle,
        s: &Arc<MSession>,
        unit: Option<usize>,
        reason: &str,
        hub_side: bool,
    ) -> (Vec<AbortUnit>, &'static str) {
        let (refs, tb, already) = {
            let mut st = guard(&s.st);
            let already = st.aborted.is_some();
            if !already {
                st.aborted = Some(reason.to_owned());
            }
            st.run = None;
            st.flagged = None;
            for u in &mut st.units {
                u.staged = None;
            }
            (urefs(&st), st.time_base, already)
        };
        let me = s.owner.id();
        let mut out = Vec::with_capacity(refs.len());
        for u in &refs {
            let io = Io {
                dev: &u.dev,
                session: me,
            };
            let stop = io.stop_now();
            let mut cancel = Ok(());
            for t in &u.tbs {
                if let Err(e) = io.cancel_all(*t) {
                    cancel = Err(e);
                }
            }
            let error = match (stop, cancel) {
                (Ok(()), Ok(())) => None,
                (Err(e), _) | (_, Err(e)) => Some(io::status_word(e).to_owned()),
            };
            out.push(AbortUnit {
                unit: unit_byte(u.index),
                stopped: error.is_none(),
                error,
            });
        }
        let broadcast = if refs.iter().all(|u| u.siggen_sync) {
            self.broadcast_stop(hub, me, &refs, tb)
        } else {
            "none"
        };
        if !already {
            for u in &refs {
                hub.lock_release(&s.owner, u.handle);
            }
            log::info!("hub: measurement {} aborted ({reason})", s.id);
            if hub_side {
                let event = AbortedEvent {
                    session: s.id.clone(),
                    unit: unit.map(unit_byte),
                    reason: reason.to_owned(),
                };
                if let Ok(Value::Object(mut m)) = serde_json::to_value(&event) {
                    m.insert("t".into(), wire::t::ABORTED.into());
                    m.insert("v".into(), VERSION.into());
                    m.insert("hub".into(), hub.hub_id().into());
                    s.owner
                        .outbox()
                        .push_reliable(Outbound::Event(Value::Object(m).to_string()));
                }
            }
        }
        (out, broadcast)
    }

    /// The broadcast `SIGGEN_STOP` now: through the line's sender (the
    /// model's sender whose superframes carry the domain's sender id) or the
    /// group's S/PDIF source. Which path it took.
    fn broadcast_stop(
        &self,
        hub: &HubHandle,
        me: SessionId,
        refs: &[URef],
        tb: u8,
    ) -> &'static str {
        let last_t = |u: &URef| -> Option<u64> {
            let io = Io {
                dev: &u.dev,
                session: me,
            };
            io.time_base()
                .ok()
                .and_then(|r| group_entry(&r.value, tb).map(|e| e.t))
        };
        match tb {
            tbc::TB_SPDIF => {
                let Some(src) = refs.iter().find(|u| u.spdif_source) else {
                    return "none";
                };
                let Some(t) = last_t(src) else { return "none" };
                let io = Io {
                    dev: &src.dev,
                    session: me,
                };
                match io.spdif_stop(t.saturating_add(SPDIF_STOP_LEAD)) {
                    Ok(()) => "spdif_source",
                    Err(_) => "none",
                }
            }
            tbc::TB_ALINK_LINE => {
                let Some(t) = refs.first().and_then(last_t) else {
                    return "none";
                };
                let Some(domain_sender) = refs
                    .first()
                    .and_then(|u| {
                        let io = Io {
                            dev: &u.dev,
                            session: me,
                        };
                        io.time_base().ok()
                    })
                    .and_then(|r| group_entry(&r.value, tb).map(|e| e.domain.to_le_bytes()[0]))
                else {
                    return "none";
                };
                let (model, _) = hub.system_model();
                for serial in model.senders.iter().filter_map(|x| x.serial.as_deref()) {
                    let Some(dev) = hub.device_by_serial(serial) else {
                        continue;
                    };
                    let io = Io {
                        dev: &dev,
                        session: me,
                    };
                    let Some(snap) = io.ctrl_msg() else { continue };
                    if snap.sender_id != domain_sender {
                        continue;
                    }
                    return match io.sender_stop(t.saturating_add(LINE_STOP_LEAD), snap.flags) {
                        Ok(()) => "sender",
                        Err(_) => "none",
                    };
                }
                "none"
            }
            _ => "none",
        }
    }

    // ── the family's thread ───────────────────────────────────────────

    fn spawn(self: &Arc<Self>, hub: &HubHandle) {
        let me = Arc::clone(self);
        let hub = hub.clone();
        let spawned = std::thread::Builder::new()
            .name("hub-measure".into())
            .spawn(move || me.thread(&hub));
        if let Err(e) = spawned {
            log::warn!("hub: no thread for system.measure: {e}");
            self.running.store(false, Ordering::SeqCst);
        }
    }

    fn thread(&self, hub: &HubHandle) {
        loop {
            {
                let (m, cv) = &self.wake;
                let mut woken = guard(m);
                if !*woken {
                    woken = cv
                        .wait_timeout(woken, TICK)
                        .map(|(g, _)| g)
                        .unwrap_or_else(|p| p.into_inner().0);
                }
                *woken = false;
            }
            let sessions: Vec<Arc<MSession>> = {
                let sessions = guard(&self.sessions);
                if sessions.is_empty() {
                    self.running.store(false, Ordering::SeqCst);
                    return;
                }
                sessions.values().cloned().collect()
            };
            for s in &sessions {
                self.tick(hub, s);
            }
        }
    }

    /// One look at one session: its connection, the tap's flags, the lease,
    /// the units online and locked, their time bases and meters, the run.
    fn tick(&self, hub: &HubHandle, s: &Arc<MSession>) {
        let _io = guard(&s.io);
        let now = Instant::now();
        if s.owner.outbox().is_closed() || hub.is_stopped() {
            self.abort_session(hub, s, None, "closed", true);
            self.close_session(hub, s);
            return;
        }
        let (refs, tb, rate, lease_over, flagged, renew, read_now) = {
            let mut st = guard(&s.st);
            if st.aborted.is_some() {
                return;
            }
            let lease_over = now.saturating_duration_since(st.last_status) > st.lease;
            let renew = now.saturating_duration_since(st.last_lock) >= LOCK_RENEW;
            if renew {
                st.last_lock = now;
            }
            let read_now =
                st.run.is_some() || now.saturating_duration_since(st.last_read) >= IDLE_READ;
            if read_now {
                st.last_read = now;
            }
            (
                urefs(&st),
                st.time_base,
                st.rate,
                lease_over,
                st.flagged.take(),
                renew,
                read_now,
            )
        };
        if let Some((unit, why)) = flagged {
            self.abort_session(hub, s, unit.map(usize::from), &why, true);
            return;
        }
        if lease_over {
            self.abort_session(hub, s, None, "lease", true);
            return;
        }
        for u in &refs {
            let same = hub
                .device_by_serial(&u.serial)
                .is_some_and(|d| Arc::ptr_eq(&d, &u.dev));
            if !same {
                self.abort_session(hub, s, Some(u.index), "offline", true);
                return;
            }
            if renew {
                // Renewed only here, after the lease was found live.
                if hub.lock_request(&s.owner, u.handle, LOCK_HOLD).is_err()
                    || !u.dev.router.lock_exempt(s.owner.id(), Instant::now())
                {
                    self.abort_session(hub, s, Some(u.index), "lock", true);
                    return;
                }
            }
        }
        if !read_now {
            return;
        }
        let me = s.owner.id();
        let mut nows: Vec<Option<(u64, Instant)>> = vec![None; refs.len()];
        let mut lines: Vec<Option<SiggenTimeline>> = vec![None; refs.len()];
        let running = guard(&s.st).run.is_some();
        for u in &refs {
            let io = Io {
                dev: &u.dev,
                session: me,
            };
            let read = match io.time_base() {
                Ok(r) => r,
                Err(st) => {
                    let why = if st == dspi_link::status::NO_DEVICE {
                        "offline"
                    } else {
                        "disturbance"
                    };
                    self.abort_session(hub, s, Some(u.index), why, true);
                    return;
                }
            };
            if moved(&read.value, tb, u).is_some() {
                self.abort_session(hub, s, Some(u.index), "disturbance", true);
                return;
            }
            if let Some(e) = group_entry(&read.value, tb) {
                self.note_locked(&u.serial, e, read.got);
                if let Some(n) = nows.get_mut(u.index) {
                    *n = Some((e.t, read.got));
                }
            }
            if let Some((peaks, _)) = io
                .meters()
                .ok()
                .and_then(|b| io::decode_meters(&b, u.outputs))
                && peaks.iter().any(|p| *p >= CLIP_DBFS)
            {
                self.abort_session(hub, s, Some(u.index), "clip", true);
                return;
            }
            if running {
                match io.timeline() {
                    Ok(t) => {
                        if let Some(l) = lines.get_mut(u.index) {
                            *l = Some(t.value);
                        }
                    }
                    Err(_) => {
                        self.abort_session(hub, s, Some(u.index), "timeline", true);
                        return;
                    }
                }
            }
        }
        if running && let Err((unit, why)) = self.schedule(s, &refs, tb, rate, &nows, &lines) {
            log::info!("hub: measurement {}: {why}", s.id);
            self.abort_session(hub, s, Some(unit), "timeline", true);
        }
    }

    /// The run's next op 9s and its proof so far (R6a, G3's facts on the
    /// hub's side). `Err`: the unit whose start went wrong, and how.
    fn schedule(
        &self,
        s: &MSession,
        refs: &[URef],
        tb: u8,
        rate: u32,
        nows: &[Option<(u64, Instant)>],
        lines: &[Option<SiggenTimeline>],
    ) -> Result<(), (usize, String)> {
        let Some(mut segs) = guard(&s.st).run.clone() else {
            return Ok(());
        };
        let me = s.owner.id();
        let now_unit = |i: usize| -> Option<u64> {
            let (t, at) = nows.get(i).copied().flatten()?;
            Some(t.saturating_add(samples_in(
                Instant::now().saturating_duration_since(at),
                rate,
            )))
        };
        for u in refs {
            let Some(now) = now_unit(u.index) else {
                continue;
            };
            let Some(tl) = lines.get(u.index).copied().flatten() else {
                continue;
            };
            // Its segments in order: the first not started decides.
            let mine: Vec<usize> = segs
                .iter()
                .enumerate()
                .filter(|(_, x)| x.unit == u.index)
                .map(|(k, _)| k)
                .collect();
            // The latest started one runs clean.
            if let Some(last) = mine
                .iter()
                .rev()
                .find(|k| segs.get(**k).is_some_and(|x| x.started))
                && let Some(x) = segs.get(*last)
                && tl.start == x.t_unit
                && tl.discontinuities != 0
            {
                return Err((
                    u.index,
                    format!("unit {}: a discontinuity while it played", u.serial),
                ));
            }
            let Some(&k) = mine
                .iter()
                .find(|k| segs.get(**k).is_some_and(|x| !x.started))
            else {
                continue;
            };
            let Some(x) = segs.get(k).cloned() else {
                continue;
            };
            if x.written {
                match schedule::judge_timeline(&tl, x.t_unit) {
                    Seen::Started { clean, .. } => {
                        if !clean {
                            return Err((u.index, format!("unit {}: a discontinuity", u.serial)));
                        }
                        if let Some(y) = segs.get_mut(k) {
                            y.started = true;
                        }
                    }
                    Seen::Pending => {
                        if !schedule::still_in_grace(x.t_unit, now, rate) {
                            return Err((
                                u.index,
                                format!("unit {}: its start never came", u.serial),
                            ));
                        }
                    }
                    Seen::Failed(w) => {
                        return Err((u.index, format!("unit {}: the start was {w}", u.serial)));
                    }
                }
            } else {
                match schedule::op9_due(x.t_unit, now, u.arm_lead_ms, rate) {
                    Due::Wait => {}
                    Due::Late => {
                        return Err((
                            u.index,
                            format!("unit {}: its next start would be late", u.serial),
                        ));
                    }
                    Due::Write => {
                        let io = Io {
                            dev: &u.dev,
                            session: me,
                        };
                        if let Err(e) = io.siggen_start_at(x.t_unit, tb) {
                            return Err((
                                u.index,
                                format!(
                                    "unit {}: the start was refused ({})",
                                    u.serial,
                                    io::status_word(e)
                                ),
                            ));
                        }
                        if let Some(y) = segs.get_mut(k) {
                            y.written = true;
                        }
                    }
                }
            }
        }
        // Done: every segment started and the domain past the last end.
        let done = segs.iter().all(|x| x.started)
            && refs
                .first()
                .and_then(|u| now_unit(u.index).map(|n| to_domain(n, u.origin)))
                .is_some_and(|n| segs.iter().all(|x| n >= x.t.saturating_add(x.len)));
        let mut st = guard(&s.st);
        if st.aborted.is_none() && st.run.is_some() {
            st.run = if done { None } else { Some(segs) };
        }
        Ok(())
    }

    // ── the tap ───────────────────────────────────────────────────────

    /// A packet from `serial`, attributed to `origin`: kept for the next
    /// `status`, and an abort flagged when it says one is due. Never blocks
    /// on a device (the device's reader calls it).
    fn on_packet(&self, serial: &str, origin: SessionId, packet: &[u8]) {
        let (Some(&id), Some(payload)) = (packet.get(1), packet.get(4..)) else {
            return;
        };
        let sessions: Vec<Arc<MSession>> = guard(&self.sessions).values().cloned().collect();
        for s in sessions {
            let mut st = guard(&s.st);
            if st.aborted.is_some() {
                continue;
            }
            let tb = st.time_base;
            let mine = s.owner.id();
            let Some(i) = st.units.iter().position(|u| u.serial == serial) else {
                continue;
            };
            let Some(u) = st.units.get_mut(i) else {
                continue;
            };
            if u.events.len() >= MAX_EVENTS {
                u.events.pop_front();
            }
            u.events.push_back(UnitEvent {
                event: id,
                raw: packet.to_vec(),
            });
            let why = judge_packet(id, payload, u.features, tb, origin, mine);
            if let Some(why) = why
                && st.flagged.is_none()
            {
                st.flagged = Some((Some(unit_byte(i)), why.to_owned()));
                drop(st);
                self.wake();
            }
        }
    }
}

/// Whether a unit's packet is a reason to abort, and which.
fn judge_packet(
    id: u8,
    payload: &[u8],
    features: u32,
    tb: u8,
    origin: SessionId,
    mine: SessionId,
) -> Option<&'static str> {
    let foreign = origin != mine;
    let is = |n: u16| u16::from(id) == n;
    if is(nfy::NOTIFY_EVT_EXT_CLIP) {
        return Some("clip");
    }
    if id == tbc::NOTIFY_EVT_TIME_BASE {
        let e = tbc::time_base_event(Some(features), id, payload)?;
        let disturbing = !matches!(
            e.reason,
            TimeBaseReason::None | TimeBaseReason::Locked | TimeBaseReason::LockOk
        );
        return (e.time_base == tb && (!e.state.is_locked() || disturbing))
            .then_some("disturbance");
    }
    if id == alink::NOTIFY_EVT_ALINK_STATE {
        if tb != tbc::TB_ALINK_LINE {
            return None;
        }
        return match alink::alink_event(Some(features), id, payload)? {
            AlinkEvent::State(e) if e.is_loss() => Some("disturbance"),
            _ => None,
        };
    }
    if id == usb::NOTIFY_EVT_ALINK_USB {
        if tb != tbc::TB_USB_SAMPLE {
            return None;
        }
        let e = usb::alink_usb_event(Some(features), id, payload)?;
        return matches!(
            e.reason,
            UsbEventReason::SofLost
                | UsbEventReason::Reanchor
                | UsbEventReason::Gap
                | UsbEventReason::Disabled
                | UsbEventReason::Config
                | UsbEventReason::Holdover
                | UsbEventReason::Resumed
                | UsbEventReason::LockError
        )
        .then_some("disturbance");
    }
    if is(nfy::NOTIFY_EVT_SIGGEN_STATE) {
        // `[state, reason, type, channel]`: a stop by a host, a preset or a
        // new config that is not the session's own.
        let reason = u16::from(*payload.get(1)?);
        let stopped_by_other = matches!(
            reason,
            siggen::SIGGEN_STOP_HOST | siggen::SIGGEN_STOP_PRESET | siggen::SIGGEN_STOP_RECONFIG
        );
        return (foreign && stopped_by_other).then_some("foreign");
    }
    if is(nfy::NOTIFY_EVT_PRESET_LOADED) || is(nfy::NOTIFY_EVT_BULK_INVALIDATED) {
        return Some("foreign");
    }
    if (is(nfy::NOTIFY_EVT_PARAM_CHANGED) || is(nfy::NOTIFY_EVT_MASTER_VOLUME)) && foreign {
        return Some("foreign");
    }
    None
}

/// The packets the tap looks at.
fn watched(id: u8) -> bool {
    let n = u16::from(id);
    matches!(
        n,
        nfy::NOTIFY_EVT_EXT_CLIP
            | nfy::NOTIFY_EVT_SIGGEN_STATE
            | nfy::NOTIFY_EVT_PRESET_LOADED
            | nfy::NOTIFY_EVT_BULK_INVALIDATED
            | nfy::NOTIFY_EVT_PARAM_CHANGED
            | nfy::NOTIFY_EVT_MASTER_VOLUME
    ) || matches!(
        id,
        tbc::NOTIFY_EVT_TIME_BASE
            | alink::NOTIFY_EVT_ALINK_STATE
            | alink::NOTIFY_EVT_APPLY_RESULT
            | usb::NOTIFY_EVT_ALINK_USB
    )
}

/// The family's packet tap (a weak link: the family may be gone).
struct Tap(Weak<MeasureHub>);

impl PacketTap for Tap {
    fn wants(&self, packet: &[u8]) -> bool {
        packet.get(1).is_some_and(|id| watched(*id))
            && self
                .0
                .upgrade()
                .is_some_and(|f| f.active.load(Ordering::SeqCst))
    }

    fn on_packet(&self, _relay: &Relay, serial: &str, origin: SessionId, packet: &[u8]) {
        if let Some(f) = self.0.upgrade() {
            f.on_packet(serial, origin, packet);
        }
    }
}

/// The outputs the unit `serial` registered with.
fn outputs_of(hub: &HubHandle, serial: &str) -> u8 {
    hub.devices()
        .into_iter()
        .find(|d| d.identity.serial == serial)
        .map_or(0, |d| d.identity.outputs)
}

/// An S/PDIF source's outputs that carry the time base (S-AT6): the slots of
/// `tx_mask` (slot n = outputs 2n and 2n + 1), or with `tx_mask` 0 every
/// S/PDIF-type slot (`REQ_GET_OUTPUT_TYPE`; a slot it cannot read counts).
fn spdif_forbidden(io: &Io<'_>, outputs: u8) -> u16 {
    let Some(cfg) = io.alink_spdif() else {
        return 0;
    };
    if cfg.mode != tbc::SpdifMode::Source {
        return 0;
    }
    let slots = outputs.div_ceil(2).min(8);
    let carries = |slot: u8| -> bool {
        if cfg.tx_mask != 0 {
            return cfg.tx_mask & 1u8.wrapping_shl(u32::from(slot)) != 0;
        }
        io.output_type(slot).is_none_or(|t| t == 0)
    };
    (0..slots).filter(|s| carries(*s)).fold(0u16, |m, s| {
        m | 0b11u16.wrapping_shl(u32::from(s.saturating_mul(2)))
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests_wp_athub {
    use super::*;

    fn tb_event(tb: u8, state: u8, reason: u8) -> Vec<u8> {
        vec![tb, state, reason, 0, 1, 2, 3, 4]
    }

    #[test]
    fn wp_athub_packets_judged() {
        let f = tbc::NOTIFY_FEAT_TIME_BASE;
        let line = tbc::TB_ALINK_LINE;
        // A holdover on the group's time base; a lock on another; a clip.
        assert_eq!(
            judge_packet(0xE4, &tb_event(line, 4, 3), f, line, 0, 5),
            Some("disturbance")
        );
        assert_eq!(judge_packet(0xE4, &tb_event(3, 4, 3), f, line, 0, 5), None);
        assert_eq!(
            judge_packet(0xE4, &tb_event(line, 3, 1), f, line, 0, 5),
            None
        );
        assert_eq!(
            judge_packet(0xE4, &tb_event(line, 3, 6), f, line, 0, 5),
            Some("disturbance"),
            "a correction while locked"
        );
        assert_eq!(
            judge_packet(0xE4, &tb_event(line, 4, 3), 0, line, 0, 5),
            None,
            "not announced"
        );
        assert_eq!(judge_packet(0xE0, &[0; 8], 0, line, 0, 5), Some("clip"));
        // A generator stopped by someone else; finished by itself; our own stop.
        let stop = |reason: u8| vec![0, reason, 4, 0xFF];
        assert_eq!(judge_packet(7, &stop(1), 0, line, 0, 5), Some("foreign"));
        assert_eq!(judge_packet(7, &stop(2), 0, line, 0, 5), None);
        assert_eq!(judge_packet(7, &stop(1), 0, line, 5, 5), None);
        assert_eq!(
            judge_packet(4, &[1], 0, line, 5, 5),
            Some("foreign"),
            "a preset load"
        );
        assert_eq!(judge_packet(2, &[0; 8], 0, line, 0, 5), Some("foreign"));
        assert_eq!(judge_packet(2, &[0; 8], 0, line, 5, 5), None);
    }

    #[test]
    fn wp_athub_origins_map_both_ways() {
        assert_eq!(to_unit(1_000, -300), 700);
        assert_eq!(to_domain(700, -300), 1_000);
        assert_eq!(to_unit(1_000, 300), 1_300);
        assert_eq!(to_domain(5, 300), 0, "saturates");
        assert!(watched(0xE4) && watched(7) && !watched(0x10));
        assert!(offered_to(3) && !offered_to(2));
    }
}
