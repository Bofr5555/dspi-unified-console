//! The health poll and device facts (WP P2HP, PLAN-P2 section 1 "Health poll", S6).
//!
//! **On attach** the hub reads `EXT_LIST`, then, where listed, `EXT_IDENTIFY` IN (board,
//! hardware revision, chip) and `EXT_RESET_INFO` (why it last reset, uptime, hang count),
//! and puts one [`BusMessage::Facts`] on the bus. Firmware without the extension channel
//! stalls `EXT_LIST`: its facts carry neither, and nothing else is asked.
//!
//! **Then the poll.** The hub joins the device's existing poll groups (`polls.rs`) with
//! the meter block, `REQ_GET_STATUS` 9, at the length a console asks for (read once on
//! attach), so a console's meter poll and the hub's are one read whoever asks. Each
//! status period it puts a [`BusMessage::Health`] on the bus: CPU (the busier core), the
//! clip latch, temperature, uptime and hang count. Temperature (`REQ_GET_STATUS` 16) and
//! the reset record are single reads on the device's queue, every 30 s and 60 s: a DSPi
//! Link poll runs at 1 Hz at least, which for a 30 s reading would be 30 times the reads.
//! An uptime that went backwards between two reset reads is a reset the hub did not see
//! as a detach: the record is read again into a new `Facts`.
//!
//! **Reads only**, as the hub's internal session (`INTERNAL_SESSION_ID`, Admin: the
//! extension opcode is admin-only in the policy). A session's device lock does not stop
//! them: the poll is read by the device worker beside the queue, and each single read
//! takes a one-time pass past the lock (`Router::pass_hold`). It never writes, and never clears the
//! clip latch: that latch is shared by every client ("clipped since someone cleared"),
//! so the hub only reads it.
//!
//! **Unresponsive.** A device that gives no answer for 10 s while attached (its reads time
//! out) is reported once as [`BusMessage::Unresponsive`]; the next answer ends it, and
//! `health.get` says so. While it is unresponsive the single reads wait, so a hung device
//! costs the other sessions only the poll's own retry.
//!
//! **For the board (WP P3HB, PLAN-P3 F7).** Three more facts per device, in `health.get`
//! and in every `Health`: `counters`, the output underruns (PDM ring, PDM DMA and S/PDIF,
//! `REQ_GET_STATUS` 4, 6 and 8, added up) and the starvation total (17), the counters the
//! diagnostics bundle reads, one single read every 30 s on the device's queue, the four
//! in turn, so each is at most two minutes old; `clip_per_min`, the extension clip events (0xE0) of the last minute, counted from
//! the bus's notifications, on firmware whose `EXT_NOTIFY_FEATURES` announces them (read
//! once on attach where listed; firmware with only the latch has no such field); and
//! `last_change`, the device's newest history record of a change (a write, a recall, an
//! import, a schedule run, new firmware) with who made it, from the history's written
//! records on the bus, so a coalesced sweep counts once. Still reads only.
//!
//! **Message:** `health.get {serial?}` (every role, `docs/integrator/hub-api.md` section 5).
//!
//! Pure state, thin glue: [`Tracker`] is one device's state machine, given the time and
//! the answers; [`HealthService`] runs it on the hub's timer, submits its reads and
//! publishes what it says. Completions only queue the answer and wake the timer.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use dspi_link::frames::CmdRequest;
use dspi_link::messages::PollSpec;
use dspi_link::{Dir, status};
use dspi_proto::ext::{
    IdentifyStatus, LIST_READ_LEN, NOTIFY_EVT_EXT_CLIP, NOTIFY_FEAT_CLIP_EVENT, ResetInfo,
    decode_list,
};
use dspi_proto::generated::ext as gext;
use dspi_proto::generated::opcodes as op;
use dspi_proto::packets::SystemStatus;
use dspi_session::events::{EventKind, EventRecord, ExtEvent, decode_ext_event};
use dspi_session::notify::{self, Event};
use serde_json::{Map, Value, json};

use crate::bus::{Bus, BusMessage, LastChange, StreamCounters, Subscription};
use crate::clock::HubClock;
use crate::hub::HubHandle;
use crate::polls::POLL_MAX_HZ;
use crate::service::{DeviceEvent, HubService, ServiceConfig, ServiceSetup, Waker};
use crate::session::{Completion, INTERNAL_SESSION_ID, Outbound, Pop, Role, Session, SessionCtx};

/// The cap in `hello.caps`.
pub const CAP: &str = "health";
/// `"v"` of every answer.
const VERSION: u8 = 1;
/// `REQ_GET_STATUS` sub-query: the meter block, as the consoles poll it.
const STATUS_METERS: u16 = 9;
/// `REQ_GET_STATUS` sub-query: the chip temperature, signed centi-°C
/// (`dspi_core::consts::STAT_TEMPERATURE`).
const STAT_TEMPERATURE: u16 = 16;
/// The meter block's longest form: 32 channels (the clip latch is 32 bits).
const PROBE_LEN: u16 = 32 * 2 + 7;
/// The hub's poll slot on each device (its own session, so no client's slot).
const POLL_SLOT: u8 = 0;
/// The least wait between two steps of one device.
const MIN_WAIT: Duration = Duration::from_millis(10);
// -- WP P3HB --
/// `REQ_GET_STATUS` sub-query: PDM ring underruns (`dspi_core::consts::STAT_PDM_RING_UNDERRUNS`).
const STAT_PDM_RING_UNDERRUNS: u16 = 4;
/// `REQ_GET_STATUS` sub-query: PDM DMA underruns (`dspi_core::consts::STAT_PDM_DMA_UNDERRUNS`).
const STAT_PDM_DMA_UNDERRUNS: u16 = 6;
/// `REQ_GET_STATUS` sub-query: S/PDIF underruns (`dspi_core::consts::STAT_SPDIF_UNDERRUNS`).
const STAT_UNDERRUNS: u16 = 8;
/// `REQ_GET_STATUS` sub-query: output DMA starvations, every instance
/// (`dspi_core::consts::STAT_STARVATION_TOTAL`).
const STAT_STARVATION: u16 = 17;
/// Clip events kept per device: the firmware sends at most 4 a second (240 a minute),
/// so this bound is never the count's.
const CLIP_EVENTS_MAX: usize = 1024;
/// Devices whose last change is kept across attachments: the inventory's bound.
const LAST_CHANGES_MAX: usize = crate::inventory::MAX_DEVICES;
/// The longest `last_change.text` kept, in characters.
const CHANGE_TEXT_MAX: usize = 256;
/// The longest `last_change.by` kept, in characters.
const CHANGE_BY_MAX: usize = 128;
/// The health service's bus queue: clip events and written history records.
const BUS_CAPACITY: usize = 1024;

/// When the hub reads what. [`HealthConfig::default`] is PLAN-P2's; tests shorten it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthConfig {
    /// A `Health` message per device this often, from the poll (2 s).
    pub status_every: Duration,
    /// The temperature read (30 s).
    pub temperature_every: Duration,
    /// The reset record read: uptime, hang count (60 s).
    pub reset_info_every: Duration,
    /// No answer for this long while attached is `Unresponsive` (10 s).
    pub unresponsive_after: Duration,
    /// A failed attach read, or a refused poll, is tried again after this (5 s).
    pub retry_after: Duration,
    // -- WP P3HB --
    /// One stream-counter read this often, underruns and starvation in turn (30 s).
    /// The first comes half a period after attach, between two temperature reads.
    pub counters_every: Duration,
    /// What `clip_per_min` counts over (a minute).
    pub clip_window: Duration,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            status_every: Duration::from_secs(2),
            temperature_every: Duration::from_secs(30),
            reset_info_every: Duration::from_secs(60),
            unresponsive_after: Duration::from_secs(10),
            retry_after: Duration::from_secs(5),
            // -- WP P3HB --
            counters_every: Duration::from_secs(30),
            clip_window: Duration::from_secs(60),
        }
    }
}

impl HealthConfig {
    /// The poll's rate: at least one reading per status period, inside 1 Hz (the
    /// least a DSPi Link poll runs at) to the most granted.
    fn poll_hz(&self) -> u32 {
        let ms = self.status_every.as_millis().max(1);
        let hz = 1000u128.div_ceil(ms);
        u32::try_from(hz)
            .unwrap_or(POLL_MAX_HZ)
            .clamp(1, POLL_MAX_HZ)
    }
}

/// The service, when [`ServiceConfig::health`] asks for it.
pub(crate) fn service(
    config: &ServiceConfig,
    setup: &ServiceSetup<'_>,
) -> Option<Arc<dyn HubService>> {
    let health = config.health?;
    Some(Arc::new(HealthService {
        config: health,
        clock: Arc::clone(setup.clock),
        bus: Arc::clone(setup.bus),
        // -- WP P3HB --
        sub: setup.bus.subscribe_filtered("health", BUS_CAPACITY, wanted),
        wake: setup.wake.clone(),
        inner: Arc::new(Mutex::new(Inner::default())),
    }))
}

// ── One device's state ──────────────────────────────────────────────────────

/// A read the hub makes on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Read {
    /// `EXT_LIST`.
    ExtList,
    /// `EXT_IDENTIFY` IN.
    Identify,
    /// `EXT_RESET_INFO`.
    ResetInfo,
    /// The meter block at its longest, once: its length.
    Probe,
    /// `REQ_GET_STATUS` 16.
    Temperature,
    /// The meter block, when the poll was refused.
    Status,
    // -- WP P3HB --
    /// `EXT_NOTIFY_FEATURES`, once on attach where listed: whether 0xE0 is sent.
    NotifyFeatures,
    /// `REQ_GET_STATUS` 8, S/PDIF underruns.
    Underruns,
    /// `REQ_GET_STATUS` 17.
    Starvation,
    /// `REQ_GET_STATUS` 4, PDM ring underruns.
    PdmRingUnderruns,
    /// `REQ_GET_STATUS` 6, PDM DMA underruns.
    PdmDmaUnderruns,
}

// -- WP P3HB --
/// The counter reads, in the order the one read a period takes them. `underruns` is
/// the sum of the three underrun counters (hub-api.md section 5), `starvation` the last.
const COUNTER_READS: [Read; 4] = [
    Read::Underruns,
    Read::Starvation,
    Read::PdmRingUnderruns,
    Read::PdmDmaUnderruns,
];

impl Read {
    fn is_counter(self) -> bool {
        COUNTER_READS.contains(&self)
    }
}

impl Read {
    const ALL: [Read; 11] = [
        Read::ExtList,
        Read::Identify,
        Read::ResetInfo,
        Read::Probe,
        Read::Temperature,
        Read::Status,
        Read::NotifyFeatures,
        Read::Underruns,
        Read::Starvation,
        Read::PdmRingUnderruns,
        Read::PdmDmaUnderruns,
    ];

    fn bit(self) -> u16 {
        1u16.checked_shl(u32::from(self as u8)).unwrap_or(0)
    }

    /// `(bRequest, wValue, wLength)`.
    fn transfer(self, meter_len: Option<u16>) -> (u8, u16, u16) {
        match self {
            Read::ExtList => (
                op::REQ_EXTENSION,
                gext::EXT_LIST,
                u16::try_from(LIST_READ_LEN).unwrap_or(u16::MAX),
            ),
            Read::Identify => (
                op::REQ_EXTENSION,
                gext::EXT_IDENTIFY,
                u16::try_from(IdentifyStatus::SIZE).unwrap_or(u16::MAX),
            ),
            Read::ResetInfo => (
                op::REQ_EXTENSION,
                gext::EXT_RESET_INFO,
                u16::try_from(ResetInfo::SIZE).unwrap_or(u16::MAX),
            ),
            Read::Probe => (op::REQ_GET_STATUS, STATUS_METERS, PROBE_LEN),
            Read::Temperature => (op::REQ_GET_STATUS, STAT_TEMPERATURE, 4),
            Read::Status => (
                op::REQ_GET_STATUS,
                STATUS_METERS,
                meter_len.unwrap_or(PROBE_LEN),
            ),
            // -- WP P3HB --
            Read::NotifyFeatures => (op::REQ_EXTENSION, gext::EXT_NOTIFY_FEATURES, 4),
            Read::Underruns => (op::REQ_GET_STATUS, STAT_UNDERRUNS, 4),
            Read::Starvation => (op::REQ_GET_STATUS, STAT_STARVATION, 4),
            Read::PdmRingUnderruns => (op::REQ_GET_STATUS, STAT_PDM_RING_UNDERRUNS, 4),
            Read::PdmDmaUnderruns => (op::REQ_GET_STATUS, STAT_PDM_DMA_UNDERRUNS, 4),
        }
    }
}

/// What a [`Tracker`] wants done.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Effect {
    /// Queue this read on the device.
    Submit(Read),
    /// Join the poll groups with the meter block of this length, at this rate.
    Subscribe {
        /// `wLength`.
        len: u16,
        /// The rate asked for.
        hz: u32,
    },
    /// Put this on the bus.
    Publish(Box<BusMessage>),
}

impl Effect {
    fn publish(m: BusMessage) -> Self {
        Effect::Publish(Box::new(m))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PollState {
    /// Not asked yet (no meter length yet).
    None,
    /// Asked; the answer has not come back.
    Asking,
    /// Running: readings come through the session's outbox.
    Granted,
    /// Refused (the budget): single reads instead, asked again at this time.
    Refused(Instant),
    /// The device has no meter block worth reading.
    Unavailable,
}

/// One online device's health, fed with the time and the answers. No threads, no I/O.
#[derive(Debug)]
pub(crate) struct Tracker {
    serial: String,
    handle: u8,
    fw: String,
    /// `EXT_LIST`: `None` until read; empty on firmware without the extension channel.
    ext: Option<Vec<u16>>,
    identify: Option<IdentifyStatus>,
    reset: Option<ResetInfo>,
    /// Reads of the facts still to come back (bits of [`Read::bit`]).
    facts_pending: u16,
    facts_sent: bool,
    /// Reads queued and not answered.
    in_flight: u16,
    /// When each attach read may be tried (again).
    retry: BTreeMap<Read, Instant>,
    meter_len: Option<u16>,
    poll: PollState,
    cpu: Option<u8>,
    clip_mask: Option<u32>,
    temp_c: Option<f32>,
    uptime_s: Option<u32>,
    hang_count: Option<u8>,
    /// The newest read, Unix seconds.
    read_at: Option<u64>,
    /// A status reading not yet in a `Health`.
    fresh: bool,
    /// The last answer of any kind.
    last_answer: Instant,
    unresponsive_since: Option<u64>,
    next_status: Instant,
    next_temperature: Instant,
    next_reset: Instant,
    due: Instant,
    // -- WP P3HB --
    /// `EXT_NOTIFY_FEATURES` is listed and not read yet.
    features_pending: bool,
    /// Its flags, once read (0 when it stalled).
    notify_features: Option<u32>,
    /// Each counter of [`COUNTER_READS`] as last read.
    counter_values: [Option<u32>; 4],
    /// Counters the firmware refused (stalled): not asked again on this attachment,
    /// and 0 in the underrun sum.
    counters_refused: u16,
    /// The index in [`COUNTER_READS`] the next counter read starts from.
    next_counter: usize,
    /// When the next counter read is due; set on the first step.
    next_counters: Option<Instant>,
    /// When each clip event of the last minute came.
    clip_events: VecDeque<Instant>,
    /// The newest change, and who made it.
    last_change: Option<LastChange>,
}

/// What a reading's payload said, for the tests and the glue alike.
fn parse_temperature(payload: &[u8]) -> Option<f32> {
    let raw: [u8; 4] = payload.get(..4)?.try_into().ok()?;
    // Centi-°C, signed in the firmware's word.
    #[allow(clippy::cast_precision_loss)]
    let c = i32::from_le_bytes(raw) as f32 / 100.0;
    Some(c)
}

// -- WP P3HB --
/// A little-endian `u32` reply (a status counter, `EXT_NOTIFY_FEATURES`).
fn parse_u32(payload: &[u8]) -> Option<u32> {
    let raw: [u8; 4] = payload.get(..4)?.try_into().ok()?;
    Some(u32::from_le_bytes(raw))
}

/// Whether `len` bytes can be a meter block: `n * 2 + 7`.
fn meter_block_len(len: usize) -> Option<u16> {
    let peaks = len.checked_sub(SystemStatus::TAIL)?;
    (peaks & 1 == 0).then(|| u16::try_from(len).ok()).flatten()
}

impl Tracker {
    pub(crate) fn new(serial: String, handle: u8, fw: String, now: Instant) -> Self {
        Self {
            serial,
            handle,
            fw,
            ext: None,
            identify: None,
            reset: None,
            facts_pending: 0,
            facts_sent: false,
            in_flight: 0,
            retry: BTreeMap::new(),
            meter_len: None,
            poll: PollState::None,
            cpu: None,
            clip_mask: None,
            temp_c: None,
            uptime_s: None,
            hang_count: None,
            read_at: None,
            fresh: false,
            last_answer: now,
            unresponsive_since: None,
            next_status: now,
            next_temperature: now,
            next_reset: now,
            due: now,
            // -- WP P3HB --
            features_pending: false,
            notify_features: None,
            counter_values: [None; 4],
            counters_refused: 0,
            next_counter: 0,
            next_counters: None,
            clip_events: VecDeque::new(),
            last_change: None,
        }
    }

    /// When [`Tracker::step`] has something to do, unless an answer comes first.
    pub(crate) fn due(&self) -> Instant {
        self.due
    }

    fn lists(&self, id: u16) -> bool {
        self.ext.as_ref().is_some_and(|l| l.contains(&id))
    }

    fn busy(&self, read: Read) -> bool {
        self.in_flight & read.bit() != 0
    }

    fn may_try(&self, read: Read, now: Instant) -> bool {
        !self.busy(read) && self.retry.get(&read).is_none_or(|at| *at <= now)
    }

    fn submit(&mut self, read: Read, out: &mut Vec<Effect>) {
        self.in_flight |= read.bit();
        out.push(Effect::Submit(read));
    }

    fn facts(&self) -> BusMessage {
        BusMessage::Facts {
            serial: self.serial.clone(),
            identify: self.identify.clone(),
            reset_info: self.reset,
            fw: self.fw.clone(),
        }
    }

    fn health(&self) -> BusMessage {
        BusMessage::Health {
            serial: self.serial.clone(),
            cpu: self.cpu,
            clip_mask: self.clip_mask.unwrap_or(0),
            temp_c: self.temp_c,
            uptime_s: self.uptime_s,
            hang_count: self.hang_count,
            // -- WP P3HB --
            counters: self.counters(),
            clip_per_min: self.clip_per_min(),
            last_change: self.last_change.clone(),
        }
    }

    // -- WP P3HB --

    /// The counters, once either is known: `underruns` once each of its three parts
    /// was read or refused (a refused one counts 0) and one was read.
    fn counters(&self) -> Option<StreamCounters> {
        let part = |r: Read| {
            let i = COUNTER_READS.iter().position(|x| *x == r)?;
            let v = self.counter_values.get(i).copied().flatten();
            match v {
                Some(v) => Some(Some(v)),
                None if self.counters_refused & r.bit() != 0 => Some(None),
                None => None,
            }
        };
        let parts = [
            part(Read::Underruns),
            part(Read::PdmRingUnderruns),
            part(Read::PdmDmaUnderruns),
        ];
        let underruns = if parts.iter().all(Option::is_some)
            && parts.iter().any(|p| matches!(p, Some(Some(_))))
        {
            Some(
                parts
                    .iter()
                    .map(|p| p.flatten().unwrap_or(0))
                    .fold(0u32, u32::saturating_add),
            )
        } else {
            None
        };
        let starvation = part(Read::Starvation).flatten();
        let c = StreamCounters {
            underruns,
            starvation,
        };
        (c.underruns.is_some() || c.starvation.is_some()).then_some(c)
    }

    /// Whether the firmware sends extension clip events (0xE0).
    fn clip_events_announced(&self) -> bool {
        self.notify_features
            .is_some_and(|f| f & NOTIFY_FEAT_CLIP_EVENT != 0)
    }

    /// Clip events in the last minute (as of the last [`Tracker::forget_clips`]), on
    /// firmware that sends them; `None` on firmware with only the latch.
    fn clip_per_min(&self) -> Option<u32> {
        self.clip_events_announced()
            .then(|| u32::try_from(self.clip_events.len()).unwrap_or(u32::MAX))
    }

    /// Drop the clip events older than the window (a minute).
    pub(crate) fn forget_clips(&mut self, now: Instant, cfg: &HealthConfig) {
        while let Some(at) = self.clip_events.front() {
            if now.saturating_duration_since(*at) < cfg.clip_window {
                break;
            }
            self.clip_events.pop_front();
        }
    }

    /// An extension clip event (0xE0) of this device came at `now`.
    pub(crate) fn clip_event(&mut self, now: Instant, cfg: &HealthConfig) {
        self.forget_clips(now, cfg);
        if self.clip_events.len() >= CLIP_EVENTS_MAX {
            self.clip_events.pop_front();
        }
        self.clip_events.push_back(now);
    }

    /// The device's newest change.
    pub(crate) fn set_last_change(&mut self, change: LastChange) {
        if self
            .last_change
            .as_ref()
            .is_none_or(|old| change.at_ms >= old.at_ms)
        {
            self.last_change = Some(change);
        }
    }

    /// The counter the next counter read asks for, in turn; `None` when the firmware
    /// refused both.
    fn counter_turn(&mut self) -> Option<Read> {
        let n = COUNTER_READS.len();
        let (i, read) = (0..n)
            .map(|k| {
                self.next_counter
                    .saturating_add(k)
                    .checked_rem(n)
                    .unwrap_or(0)
            })
            .filter_map(|i| COUNTER_READS.get(i).map(|r| (i, *r)))
            .find(|(_, r)| self.counters_refused & r.bit() == 0)?;
        self.next_counter = i.saturating_add(1).checked_rem(n).unwrap_or(0);
        Some(read)
    }

    fn counters_wanted(&self) -> bool {
        COUNTER_READS
            .iter()
            .any(|r| self.counters_refused & r.bit() == 0)
    }

    fn counter_busy(&self) -> bool {
        COUNTER_READS.iter().any(|r| self.busy(*r))
    }

    /// The device answered: it is not unresponsive.
    fn answered(&mut self, now: Instant) {
        self.last_answer = now;
        if self.unresponsive_since.take().is_some() {
            log::info!("hub health: device {} answers again", self.serial);
        }
    }

    /// One meter block, from the poll or a single read.
    pub(crate) fn status_reading(&mut self, payload: &[u8], now: Instant, wall_s: u64) {
        self.answered(now);
        if let Ok(s) = SystemStatus::decode(payload) {
            self.cpu = Some(s.cpu0.max(s.cpu1));
            self.clip_mask = Some(s.clip_flags);
            self.read_at = Some(wall_s);
            self.fresh = true;
        }
    }

    /// The poll was granted (`true`) or refused.
    pub(crate) fn poll_answer(&mut self, granted: bool, now: Instant, cfg: &HealthConfig) {
        self.poll = if granted {
            PollState::Granted
        } else {
            PollState::Refused(now.checked_add(cfg.retry_after).unwrap_or(now))
        };
    }

    /// A read came back with this DSPi Link status.
    #[allow(clippy::too_many_arguments)] // the answer, its time on both clocks, the config
    pub(crate) fn answer(
        &mut self,
        read: Read,
        st: u8,
        payload: &[u8],
        now: Instant,
        wall_s: u64,
        cfg: &HealthConfig,
        out: &mut Vec<Effect>,
    ) {
        self.in_flight &= !read.bit();
        // A stall is the firmware's answer ("not this"); a timeout or a lost device is none.
        let answered = st == status::OK || st == status::ERROR;
        let attach_read = match read {
            Read::ExtList | Read::Identify | Read::Probe | Read::NotifyFeatures => true,
            Read::ResetInfo => !self.facts_sent,
            Read::Temperature
            | Read::Status
            | Read::Underruns
            | Read::Starvation
            | Read::PdmRingUnderruns
            | Read::PdmDmaUnderruns => false,
        };
        if answered {
            self.answered(now);
            self.retry.remove(&read);
        } else if attach_read {
            // Tried again after a while; the periodic reads simply wait for their turn.
            self.retry
                .insert(read, now.checked_add(cfg.retry_after).unwrap_or(now));
        }
        let ok = st == status::OK;
        match read {
            Read::ExtList if answered => {
                let list = if ok { decode_list(payload) } else { Vec::new() };
                for (read, id) in [
                    (Read::Identify, gext::EXT_IDENTIFY),
                    (Read::ResetInfo, gext::EXT_RESET_INFO),
                ] {
                    if list.contains(&id) {
                        self.facts_pending |= read.bit();
                    }
                }
                // -- WP P3HB --
                self.features_pending = list.contains(&gext::EXT_NOTIFY_FEATURES);
                self.ext = Some(list);
            }
            Read::Identify if answered => {
                self.identify = ok.then(|| IdentifyStatus::decode(payload)).flatten();
                self.facts_pending &= !read.bit();
            }
            Read::ResetInfo if answered => {
                let info = ok.then(|| ResetInfo::decode(payload)).flatten();
                if let Some(info) = info {
                    self.uptime_s = Some(info.uptime_s);
                    self.hang_count = Some(info.hang_count);
                    self.read_at = Some(wall_s);
                }
                if !self.facts_sent {
                    self.reset = info;
                    self.facts_pending &= !read.bit();
                } else if let Some(info) = info {
                    let before = self.reset.map(|r| r.uptime_s);
                    self.reset = Some(info);
                    // Uptime went backwards: a reset between two reads, unseen as a
                    // detach. The facts again, with the new record.
                    if before.is_some_and(|b| info.uptime_s < b) {
                        log::info!(
                            "hub health: device {} restarted between reads ({})",
                            self.serial,
                            info.reason.as_str()
                        );
                        out.push(Effect::publish(self.facts()));
                    }
                }
            }
            Read::Probe if ok => match meter_block_len(payload.len()) {
                Some(len) => {
                    self.meter_len = Some(len);
                    self.status_reading(payload, now, wall_s);
                    self.poll = PollState::Asking;
                    out.push(Effect::Subscribe {
                        len,
                        hz: cfg.poll_hz(),
                    });
                }
                None => self.poll = PollState::Unavailable,
            },
            Read::Probe if answered => self.poll = PollState::Unavailable,
            Read::Temperature if ok => {
                if let Some(c) = parse_temperature(payload) {
                    self.temp_c = Some(c);
                    self.read_at = Some(wall_s);
                }
            }
            Read::Status if ok => self.status_reading(payload, now, wall_s),
            // -- WP P3HB --
            Read::NotifyFeatures if answered => {
                self.features_pending = false;
                self.notify_features = Some(if ok {
                    parse_u32(payload).unwrap_or(0)
                } else {
                    0
                });
            }
            r if r.is_counter() && ok => {
                if let Some(v) = parse_u32(payload)
                    && let Some(i) = COUNTER_READS.iter().position(|x| *x == r)
                    && let Some(slot) = self.counter_values.get_mut(i)
                {
                    *slot = Some(v);
                    self.read_at = Some(wall_s);
                }
            }
            r if r.is_counter() && answered => {
                // Stalled: this firmware has no such counter.
                self.counters_refused |= read.bit();
            }
            _ => {}
        }
    }

    /// Do what is due at `now`.
    pub(crate) fn step(
        &mut self,
        now: Instant,
        wall_s: u64,
        cfg: &HealthConfig,
        out: &mut Vec<Effect>,
    ) {
        let unresponsive = self.unresponsive_since.is_some();
        // The facts: the list, then what it lists.
        if self.ext.is_none() {
            if self.may_try(Read::ExtList, now) {
                self.submit(Read::ExtList, out);
            }
        } else if !self.facts_sent {
            for read in [Read::Identify, Read::ResetInfo] {
                if self.facts_pending & read.bit() != 0 && self.may_try(read, now) {
                    self.submit(read, out);
                }
            }
            if self.facts_pending == 0 {
                self.facts_sent = true;
                self.next_reset = now.checked_add(cfg.reset_info_every).unwrap_or(now);
                out.push(Effect::publish(self.facts()));
            }
        }
        // -- WP P3HB -- whether the firmware sends clip events, once.
        if self.features_pending && self.may_try(Read::NotifyFeatures, now) {
            self.submit(Read::NotifyFeatures, out);
        }
        // The meter block's length, then the poll.
        match self.poll {
            PollState::None if self.may_try(Read::Probe, now) => self.submit(Read::Probe, out),
            PollState::Refused(at) if at <= now => {
                if let Some(len) = self.meter_len {
                    self.poll = PollState::Asking;
                    out.push(Effect::Subscribe {
                        len,
                        hz: cfg.poll_hz(),
                    });
                }
            }
            _ => {}
        }
        // Each status period: a single read where the poll was refused, and the `Health`.
        if now >= self.next_status {
            self.next_status = later(self.next_status, now, cfg.status_every);
            if matches!(self.poll, PollState::Refused(_))
                && !unresponsive
                && !self.busy(Read::Status)
            {
                self.submit(Read::Status, out);
            }
            if std::mem::take(&mut self.fresh) {
                out.push(Effect::publish(self.health()));
            }
        }
        if now >= self.next_temperature && !unresponsive && !self.busy(Read::Temperature) {
            self.next_temperature = later(self.next_temperature, now, cfg.temperature_every);
            self.submit(Read::Temperature, out);
        }
        if self.facts_sent
            && self.lists(gext::EXT_RESET_INFO)
            && now >= self.next_reset
            && !unresponsive
            && !self.busy(Read::ResetInfo)
        {
            self.next_reset = later(self.next_reset, now, cfg.reset_info_every);
            self.submit(Read::ResetInfo, out);
        }
        // -- WP P3HB -- one counter read a period, the two counters in turn.
        let counters_at = *self.next_counters.get_or_insert_with(|| {
            let half = cfg.counters_every.checked_div(2).unwrap_or_default();
            now.checked_add(half).unwrap_or(now)
        });
        if now >= counters_at && !unresponsive && !self.counter_busy() && self.counters_wanted() {
            self.next_counters = Some(later(counters_at, now, cfg.counters_every));
            if let Some(read) = self.counter_turn() {
                self.submit(read, out);
            }
        }
        self.forget_clips(now, cfg);
        // No answer for long enough: reported once, until it answers again.
        let deadline = self.last_answer.checked_add(cfg.unresponsive_after);
        if !unresponsive && deadline.is_some_and(|d| now >= d) {
            let silent = now.saturating_duration_since(self.last_answer).as_secs();
            let since = wall_s.saturating_sub(silent);
            self.unresponsive_since = Some(since);
            log::warn!(
                "hub health: device {} has not answered for {silent} s",
                self.serial
            );
            out.push(Effect::publish(BusMessage::Unresponsive {
                serial: self.serial.clone(),
                since,
            }));
        }
        self.due = self.next_due(now, deadline);
    }

    fn next_due(&self, now: Instant, deadline: Option<Instant>) -> Instant {
        let mut due = self.next_status;
        let mut consider = |at: Instant| due = due.min(at);
        if self.unresponsive_since.is_none() {
            if let Some(d) = deadline {
                consider(d);
            }
            if !self.busy(Read::Temperature) {
                consider(self.next_temperature);
            }
            if self.facts_sent && self.lists(gext::EXT_RESET_INFO) && !self.busy(Read::ResetInfo) {
                consider(self.next_reset);
            }
            // -- WP P3HB --
            if let Some(at) = self.next_counters
                && !self.counter_busy()
                && self.counters_wanted()
            {
                consider(at);
            }
        }
        for read in Read::ALL {
            if let Some(at) = self.retry.get(&read)
                && !self.busy(read)
            {
                consider(*at);
            }
        }
        if let PollState::Refused(at) = self.poll {
            consider(at);
        }
        // Never now: a step that did all it could waits for the next thing.
        due.max(now.checked_add(MIN_WAIT).unwrap_or(now))
    }

    /// `health.get`'s record.
    pub(crate) fn record(&self) -> Value {
        let mut r = Map::new();
        r.insert("serial".into(), self.serial.clone().into());
        r.insert("handle".into(), self.handle.into());
        if let Some(at) = self.read_at {
            r.insert("at".into(), iso8601(at).into());
        }
        if let Some(cpu) = self.cpu {
            r.insert("cpu_pct".into(), cpu.into());
        }
        if let Some(mask) = self.clip_mask {
            r.insert("clip_mask".into(), mask.into());
        }
        if let Some(t) = self.temp_c {
            // To the firmware's 0.01 °C, without f32's tail.
            r.insert(
                "temp_c".into(),
                json!((f64::from(t) * 100.0).round() / 100.0),
            );
        }
        if let Some(u) = self.uptime_s {
            r.insert("uptime_s".into(), u.into());
        }
        if let Some(h) = self.hang_count {
            r.insert("hang_count".into(), h.into());
        }
        r.insert(
            "unresponsive".into(),
            self.unresponsive_since.is_some().into(),
        );
        if let Some(since) = self.unresponsive_since {
            r.insert("unresponsive_since".into(), iso8601(since).into());
        }
        if let Some(reset) = self.reset {
            let mut o = json!({"reason": reset.reason.as_str()});
            if let (Some(p), Some(map)) = (reset.prev_uptime_s, o.as_object_mut()) {
                map.insert("prev_uptime_s".into(), p.into());
            }
            r.insert("reset".into(), o);
        }
        // -- WP P3HB --
        if let Some(c) = self.counters() {
            let mut o = Map::new();
            if let Some(u) = c.underruns {
                o.insert("underruns".into(), u.into());
            }
            if let Some(s) = c.starvation {
                o.insert("starvation".into(), s.into());
            }
            r.insert("counters".into(), Value::Object(o));
        }
        if let Some(n) = self.clip_per_min() {
            r.insert("clip_per_min".into(), n.into());
        }
        if let Some(c) = &self.last_change {
            let secs = u64::try_from(c.at_ms.max(0) / 1000).unwrap_or(0);
            let mut o = Map::new();
            o.insert("at".into(), iso8601(secs).into());
            if let Some(by) = &c.by {
                o.insert("by".into(), by.clone().into());
            }
            o.insert("text".into(), c.text.clone().into());
            r.insert("last_change".into(), Value::Object(o));
        }
        Value::Object(r)
    }
}

/// The next time of a period that was due at `due`, past `now` (no burst after a stall).
fn later(due: Instant, now: Instant, every: Duration) -> Instant {
    due.checked_add(every)
        .filter(|next| *next > now)
        .or_else(|| now.checked_add(every))
        .unwrap_or(now)
}

/// ISO-8601 UTC with a `Z` (spec 7.1), as `server.rs` writes it.
fn iso8601(secs: u64) -> String {
    // Howard Hinnant's civil_from_days. Every step is within i64 for any u64 of seconds
    // divided down to days, and the divisors are constants.
    #![allow(clippy::arithmetic_side_effects, clippy::cast_possible_wrap)]
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

// ── The glue ────────────────────────────────────────────────────────────────

/// One device as the service holds it.
struct Slot {
    tracker: Tracker,
    /// Which attachment this is, so a late answer of an earlier one is dropped.
    token: u64,
    /// The hub's internal session on this device: its poll's frames come here.
    session: Arc<Session>,
}

/// A read's answer, queued by its completion.
struct Reply {
    serial: String,
    token: u64,
    read: Read,
    status: u8,
    payload: Vec<u8>,
}

#[derive(Default)]
struct Inner {
    devices: BTreeMap<String, Slot>,
    /// Device events, for the timer to act on (`on_device` may not block).
    events: Vec<DeviceEvent>,
    replies: Vec<Reply>,
    /// Something arrived at this time: run as soon as the timer can.
    wake_at: Option<Instant>,
    next_token: u64,
    // -- WP P3HB --
    /// Each device's newest change, kept across attachments (bounded).
    last_changes: BTreeMap<String, LastChange>,
}

/// The health service.
pub(crate) struct HealthService {
    config: HealthConfig,
    clock: Arc<dyn HubClock>,
    bus: Arc<Bus>,
    // -- WP P3HB -- clip events and written history records.
    sub: Subscription,
    wake: Waker,
    inner: Arc<Mutex<Inner>>,
}

fn lock(m: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    // Every section under it leaves the state whole before anything that could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The hub's own reads: its internal session, which needs no in-flight slot.
fn internal() -> SessionCtx {
    SessionCtx {
        id: INTERNAL_SESSION_ID,
        role: Role::Admin,
        exempt_inflight: true,
    }
}

/// What run_due has to do once the state is unlocked.
struct Work {
    serial: String,
    handle: u8,
    token: u64,
    session: Arc<Session>,
    meter_len: Option<u16>,
    effects: Vec<Effect>,
}

impl HealthService {
    /// A new device: its tracker.
    fn device_online(inner: &mut Inner, serial: String, handle: u8, fw: String, now: Instant) {
        let token = inner.next_token;
        inner.next_token = token.wrapping_add(1);
        let session = Session::new(INTERNAL_SESSION_ID, Role::Admin, None, true);
        let mut tracker = Tracker::new(serial.clone(), handle, fw, now);
        // -- WP P3HB -- the last change outlives the attachment.
        if let Some(c) = inner.last_changes.get(&serial) {
            tracker.set_last_change(c.clone());
        }
        if let Some(old) = inner.devices.insert(
            serial,
            Slot {
                tracker,
                token,
                session,
            },
        ) {
            old.session.outbox().close();
        }
    }

    /// Queue `read` on the device; its answer comes back through `replies`.
    fn submit(&self, hub: &HubHandle, w: &Work, read: Read) {
        let reply_to = Arc::clone(&self.inner);
        let clock = Arc::clone(&self.clock);
        let wake = self.wake.clone();
        let (serial, token) = (w.serial.clone(), w.token);
        let done = Completion::new(0, move |r| {
            let at = clock.now();
            {
                let mut inner = lock(&reply_to);
                inner.replies.push(Reply {
                    serial,
                    token,
                    read,
                    status: r.status,
                    payload: r.payload,
                });
                inner.wake_at = Some(inner.wake_at.map_or(at, |w| w.min(at)));
            }
            wake.wake();
        });
        let device = hub
            .device_by_serial(&w.serial)
            .filter(|d| d.handle == w.handle);
        let Some(device) = device else {
            // Gone: the offline event follows, and the dropped completion answers
            // NO_DEVICE, which counts as no answer.
            return;
        };
        let (b_request, w_value, w_length) = read.transfer(w.meter_len);
        let req = CmdRequest {
            tag: 0,
            handle: w.handle,
            dir: Dir::Get,
            b_request,
            w_value,
            w_index: dspi_proto::VENDOR_INTERFACE,
            w_length,
            payload: Vec::new(),
        };
        // A read changes nothing a lock protects, so a console's device lock must not
        // stall it: the router locks out even the hub's own reads otherwise. One pass,
        // for this very request, taken by its admission (P1HL's `pass_hold`, as P2ID
        // uses). Passes are matched by (opcode, wValue, payload) and taken one each, so
        // P2ID's `EXT_IDENTIFY` IN pass and this one are interchangeable: two requests,
        // two passes, whichever admission takes which.
        device.router.pass_hold(&req);
        device.submit(internal(), req, done, None);
    }

    // -- WP P3HB --
    /// What the bus said since the last run: clip events, changes, forgotten devices.
    fn take_bus(&self, inner: &mut Inner, now: Instant) {
        let dropped = self.sub.take_dropped();
        if dropped > 0 {
            log::info!("hub health: {dropped} bus message(s) lost: clip counts may be low");
        }
        for m in self.sub.drain() {
            match &*m {
                BusMessage::Notification { serial, packet, .. } => {
                    if is_clip_event(packet)
                        && let Some(slot) = inner.devices.get_mut(serial)
                    {
                        slot.tracker.clip_event(now, &self.config);
                    }
                }
                BusMessage::Event { serial, record } => {
                    let Some(change) = change_of(record) else {
                        continue;
                    };
                    if let Some(slot) = inner.devices.get_mut(serial) {
                        slot.tracker.set_last_change(change.clone());
                    }
                    remember_change(&mut inner.last_changes, serial, change);
                }
                BusMessage::DeviceForgotten { serial } => {
                    inner.last_changes.remove(serial);
                }
                _ => {}
            }
        }
    }

    fn run(&self, hub: &HubHandle) {
        let now = self.clock.now();
        let wall_s = self.clock.unix_seconds();
        let events = {
            let mut inner = lock(&self.inner);
            inner.wake_at = None;
            std::mem::take(&mut inner.events)
        };
        // The firmware of the new ones, from the registry, with this lock released.
        let events: Vec<(DeviceEvent, String)> = events
            .into_iter()
            .map(|e| {
                let fw = match &e {
                    DeviceEvent::Online { serial, .. } => hub
                        .device_by_serial(serial)
                        .map(|d| {
                            d.identity
                                .read()
                                .unwrap_or_else(PoisonError::into_inner)
                                .firmware
                                .clone()
                        })
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                (e, fw)
            })
            .collect();
        let mut work = Vec::new();
        {
            let mut inner = lock(&self.inner);
            for (event, fw) in events {
                match event {
                    DeviceEvent::Online { serial, handle } => {
                        Self::device_online(&mut inner, serial, handle, fw, now);
                    }
                    DeviceEvent::Offline { serial, .. } | DeviceEvent::Removed { serial, .. } => {
                        // The poll goes with the attachment; the outbox is closed so
                        // nothing more is kept.
                        if let Some(slot) = inner.devices.remove(&serial) {
                            slot.session.outbox().close();
                        }
                    }
                }
            }
            // -- WP P3HB -- after the new devices, so an event of one counts.
            self.take_bus(&mut inner, now);
            let replies = std::mem::take(&mut inner.replies);
            let mut effects: BTreeMap<String, Vec<Effect>> = BTreeMap::new();
            for r in replies {
                if let Some(slot) = inner.devices.get_mut(&r.serial)
                    && slot.token == r.token
                {
                    let out = effects.entry(r.serial.clone()).or_default();
                    slot.tracker.answer(
                        r.read,
                        r.status,
                        &r.payload,
                        now,
                        wall_s,
                        &self.config,
                        out,
                    );
                }
            }
            for (serial, slot) in &mut inner.devices {
                // The poll's newest frame (a newer one replaces the queued one).
                let mut newest = None;
                while let Pop::Item(item) = slot.session.outbox().pop(Duration::ZERO) {
                    if let Outbound::Poll { payload, .. } = item {
                        newest = Some(payload);
                    }
                }
                if let Some(payload) = newest {
                    slot.tracker.status_reading(&payload, now, wall_s);
                }
                let out = effects.entry(serial.clone()).or_default();
                slot.tracker.step(now, wall_s, &self.config, out);
                if !out.is_empty() {
                    work.push(Work {
                        serial: serial.clone(),
                        handle: slot.tracker.handle,
                        token: slot.token,
                        session: Arc::clone(&slot.session),
                        meter_len: slot.tracker.meter_len,
                        effects: std::mem::take(out),
                    });
                }
            }
        }
        // Unlocked: a submit refused at once answers on this thread, into `replies`.
        for w in &work {
            for effect in &w.effects {
                match effect {
                    Effect::Submit(read) => self.submit(hub, w, *read),
                    Effect::Publish(m) => self.bus.publish((**m).clone()),
                    Effect::Subscribe { len, hz } => {
                        let spec = PollSpec {
                            slot: POLL_SLOT,
                            req: op::REQ_GET_STATUS,
                            val: STATUS_METERS,
                            idx: dspi_proto::VENDOR_INTERFACE,
                            len: *len,
                            hz: *hz,
                        };
                        let granted = match hub.poll_subscribe(&w.session, w.handle, &[spec]) {
                            Ok(g) => g.iter().any(|g| g.slot == POLL_SLOT && g.hz > 0),
                            Err(e) => {
                                log::info!("hub health: {} not polled: {e}", w.serial);
                                false
                            }
                        };
                        let mut inner = lock(&self.inner);
                        if let Some(slot) = inner.devices.get_mut(&w.serial)
                            && slot.token == w.token
                        {
                            slot.tracker
                                .poll_answer(granted, self.clock.now(), &self.config);
                        }
                    }
                }
            }
        }
    }

    /// `health.get {serial?, handle?}`.
    fn get(&self, hub: &HubHandle, id: &Value, msg: &Value) -> String {
        let serial = msg.get("serial");
        let handle = msg.get("handle");
        let serial = match serial {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => return err(id, "bad_request", "serial must be a string"),
        };
        let handle = match handle {
            None | Some(Value::Null) => None,
            Some(h) => match h.as_u64().and_then(|h| u8::try_from(h).ok()) {
                Some(h) => Some(h),
                None => return err(id, "bad_request", "handle must be a number 0 to 255"),
            },
        };
        let known = if serial.is_some() || handle.is_some() {
            hub.devices()
        } else {
            Vec::new()
        };
        // Which serial is asked for, if any.
        let wanted = match (serial, handle) {
            (None, None) => None,
            (Some(s), None) => {
                if !known.iter().any(|d| d.identity.serial == s) {
                    return err(id, "not_found", "the hub does not know this serial");
                }
                Some(s)
            }
            (s, Some(h)) => {
                let Some(d) = known.iter().find(|d| d.handle == h) else {
                    return err(id, "no_device", "no device at this handle");
                };
                if s.as_ref().is_some_and(|s| *s != d.identity.serial) {
                    return err(
                        id,
                        "bad_request",
                        "serial and handle name different devices",
                    );
                }
                Some(d.identity.serial.clone())
            }
        };
        let now = self.clock.now();
        let mut inner = lock(&self.inner);
        // -- WP P3HB -- a minute's clip events as of now, not of the last step.
        for slot in inner.devices.values_mut() {
            slot.tracker.forget_clips(now, &self.config);
        }
        let devices: Vec<Value> = match &wanted {
            None => inner.devices.values().map(|s| s.tracker.record()).collect(),
            Some(s) => match inner.devices.get(s) {
                Some(slot) => vec![slot.tracker.record()],
                None => return err(id, "offline", "the device is not attached"),
            },
        };
        drop(inner);
        let mut out = json!({"t": "ok", "id": id, "v": VERSION});
        if let Some(map) = out.as_object_mut() {
            map.insert("devices".into(), Value::Array(devices));
        }
        out.to_string()
    }
}

// -- WP P3HB --

/// What the health service takes from the bus: the extension clip events (0xE0), the
/// history's written records (an id: written, so a coalesced one once) and forgotten
/// devices. Runs on the publisher's thread: quick, no lock.
fn wanted(m: &BusMessage) -> bool {
    match m {
        BusMessage::Notification { packet, .. } => packet.get(1) == Some(&NOTIFY_EVT_EXT_CLIP),
        BusMessage::Event { record, .. } => record.id != 0 && is_change(record.kind),
        BusMessage::DeviceForgotten { .. } => true,
        _ => false,
    }
}

/// Whether `packet` is an extension clip event. The bus carries the notifications of
/// every device; whether this one announces 0xE0 is its tracker's
/// ([`Tracker::clip_per_min`]).
fn is_clip_event(packet: &[u8]) -> bool {
    match notify::decode(packet) {
        Ok((_, Event::Unknown { id, bytes })) => matches!(
            decode_ext_event(Some(NOTIFY_FEAT_CLIP_EVENT), id, &bytes),
            Some(ExtEvent::Clip { .. })
        ),
        _ => false,
    }
}

/// The history kinds that are a change to the device: a setting written (a
/// notification, with its writer), a preset recalled, a document imported, a schedule
/// run, new firmware. Not what the device or the hub observed (clips, temperature,
/// counters, resets, connections, alarms, the audio link, access).
fn is_change(kind: EventKind) -> bool {
    matches!(
        kind,
        EventKind::Notification
            | EventKind::Recall
            | EventKind::Import
            | EventKind::Schedule
            | EventKind::Firmware
    )
}

/// A written record as a last change, bounded; `None` for a record that is no change.
fn change_of(record: &EventRecord) -> Option<LastChange> {
    if record.id == 0 || !is_change(record.kind) {
        return None;
    }
    let cut = |s: &str, n: usize| s.chars().take(n).collect::<String>();
    Some(LastChange {
        at_ms: record.t,
        by: record
            .by
            .as_deref()
            .filter(|b| !b.is_empty())
            .map(|b| cut(b, CHANGE_BY_MAX)),
        text: cut(&record.text, CHANGE_TEXT_MAX),
    })
}

/// Keep `change` as `serial`'s newest, within [`LAST_CHANGES_MAX`] devices: past it the
/// oldest change goes.
fn remember_change(book: &mut BTreeMap<String, LastChange>, serial: &str, change: LastChange) {
    if let Some(old) = book.get_mut(serial) {
        if change.at_ms >= old.at_ms {
            *old = change;
        }
        return;
    }
    if book.len() >= LAST_CHANGES_MAX {
        let oldest = book
            .iter()
            .min_by_key(|(_, c)| c.at_ms)
            .map(|(s, _)| s.clone());
        if let Some(s) = oldest {
            book.remove(&s);
        }
    }
    book.insert(serial.to_owned(), change);
}

fn err(id: &Value, code: &str, msg: &str) -> String {
    json!({"t": "err", "id": id, "v": VERSION, "code": code, "msg": msg}).to_string()
}

impl HubService for HealthService {
    fn name(&self) -> &'static str {
        "health"
    }

    fn caps(&self) -> Vec<&'static str> {
        vec![CAP]
    }

    fn handle_message(
        &self,
        hub: &HubHandle,
        _session: &Arc<Session>,
        msg: &Value,
    ) -> Option<String> {
        if msg.get("t")?.as_str()? != "health.get" {
            return None;
        }
        // Every role may read it.
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        if !(id.is_u64() || id.is_i64()) {
            return Some(err(&Value::Null, "bad_request", "id is required"));
        }
        Some(self.get(hub, &id, msg))
    }

    fn next_due(&self) -> Option<Instant> {
        let inner = lock(&self.inner);
        let trackers = inner.devices.values().map(|s| s.tracker.due()).min();
        match (inner.wake_at, trackers) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn run_due(&self, hub: &HubHandle, _now: Instant) {
        self.run(hub);
    }

    fn on_device(&self, event: &DeviceEvent) {
        let at = self.clock.now();
        {
            let mut inner = lock(&self.inner);
            inner.events.push(event.clone());
            inner.wake_at = Some(inner.wake_at.map_or(at, |w| w.min(at)));
        }
        self.wake.wake();
    }

    fn stop(&self) {
        // A closed outbox leaves its poll group at the next read.
        let mut inner = lock(&self.inner);
        for slot in inner.devices.values() {
            slot.session.outbox().close();
        }
        inner.devices.clear();
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
mod tests_wp_p2hp;

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
mod tests_wp_p3hb;
