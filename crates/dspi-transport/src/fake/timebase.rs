//! WP ATFAKE: fake units on one time base (PLAN-AT section 1, the fork's
//! `alink_protocol_spec.md` sections 11 and 12, `extension_channel_spec.md`
//! 2.8.7.1, 2.10 and 2.11, `test_signals_spec.md` section 15), so the hub and
//! the client session can schedule generator starts across several units
//! without boards.
//!
//! Written from those specs and the seam's constants
//! (`dspi_proto::ext::timebase`), never from WP ATPROTO's codecs: the replies
//! here are laid out byte by byte from the spec tables.
//!
//! **The domain.** A [`FakeTimeBase`] is one shared sample counter T at
//! 48 kHz, running with [`crate::clock`] from its creation, of one kind: the
//! line (time base 0, the sender's F), USB-direct (time base 1: the host's
//! bus, so S = 48 N + k with each unit's own origin from
//! [`UsbDirectSetup::origin_wraps`]) or S/PDIF (time base 3: the source's F,
//! with its epoch). Every fake that [`FakeTimeBase::join`]s it reads T from
//! the same clock, so units agree on T to the sample whenever they are read.
//! T resolves to one sample of the domain's clock.
//!
//! **A unit** is a [`FakeDevice`] made with [`FakeDevice::demo_on_time_base`]
//! (or with [`super::FakeOptions::time_base`] set by hand): its transport
//! (the line receiver of `fake/alink.rs`, the USB-direct unit of
//! `fake/alink_usb.rs`, or the S/PDIF follower or source modelled here) counts
//! on the domain's clock. Such a fake is firmware with WP FWTB1 and FWSP2:
//! `EXT_LIST` names `EXT_TIME_BASE` and `EXT_SIGGEN_TIMELINE` (and on an
//! S/PDIF unit `EXT_APPLY_AT_FRAME` and `0x8020`-`0x8022`),
//! `EXT_NOTIFY_FEATURES` bit 5 announces `0xE4`, a line unit reports
//! `EXT_ALINK_INFO.features` bit 6 `SIGGEN_SYNC`, and apply ops 9 and 10 work
//! on every time base (`fake/analysis.rs` holds the generator side). A fake
//! without a time base is firmware before FWTB1: it lists none of it and
//! refuses ops 9 and 10 at the write as an unknown op.
//!
//! **The contract.** `EXT_TIME_BASE` (64 bytes) maps each transport's state
//! onto the contract's (`alink_protocol_spec.md` 11.2), with the domain id of
//! C5, `t` and `output_frames` taken at one instant, the disturbance counter
//! and D. Every contract transition sends `0xE4`; every transition out of
//! `LOCKED`, holdover, resume, re-anchor, whole-sample correction and epoch
//! change counts one disturbance. `t - output_frames` is constant while an
//! entry stays `LOCKED` and undisturbed; an anchor restarts the outputs, so
//! it moves then.
//!
//! **Faults** ([`FakeRemote::time_base_fault`]): a holdover and its resume, a
//! re-anchor, a whole-sample correction with or without its event (without:
//! neither `0xE4` nor the counter, a firmware fault only `t - output_frames`
//! shows), lock error on and off, and an S/PDIF source's epoch change
//! ([`TimeBaseFault::Epoch`], also [`FakeTimeBase::new_epoch`]), which every
//! follower notices on its next request and re-anchors on (reason `EPOCH`).
//!
//! **Broadcasts.** An op written to the line sender (`EXT_ALINK_CTRL_MSG`) or
//! the S/PDIF source (`EXT_ALINK_SPDIF_CTRL`) of a domain reaches every other
//! unit of it: each takes it on its next request and applies it at its T (the
//! source too, `EXT_ALINK_SPDIF_CTRL` IN showing its own result). Ops 1-8
//! only report their results, as the per-unit queues do.
//!
//! Where it differs from the firmware: nothing runs between requests (an op
//! is worked out at its exact T when the next request looks, so the T and the
//! generator's start are exact, but an event goes out late); the `0xE4` rate
//! limit (250 ms merge) is not modelled, every transition is one event; the
//! DAC hardware mute is not modelled (`arm_lead_ms` is reported, 150 ms by
//! default as the K340, and enforced as a lead, but no DAC is "held at T");
//! a forced holdover or lock error on a USB-direct unit shows in the contract
//! (`EXT_TIME_BASE`, `0xE4`), not in `EXT_ALINK_USB_STATUS`; one time base per
//! unit (no USB-direct follower that is also an S/PDIF source); the S/PDIF
//! carriage itself (the user bits, the coast rule) is not modelled, only its
//! control plane.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use dspi_proto::ext::alink::{
    ApplyOp, ApplyRequest, ApplyResult, ApplyResultEvent, ApplyStatus, EXT_APPLY_AT_FRAME,
    NOTIFY_EVT_APPLY_RESULT,
};
use dspi_proto::ext::timebase::{self as tbc, reason};
use dspi_proto::generated::notify as nfy;
use dspi_proto::packets::SiggenConfig;
use dspi_proto::paged::legacy as lm;

use super::analysis::PendingStart;
use super::{AlinkSetup, Capture, FakeDevice, FakeGeneration, FakeOptions, FakeRemote, State};
use super::{DEMO_STATUS, UsbDirectSetup};
use crate::clock::Instant;

/// The rate every fake time base runs at.
pub const TIME_BASE_RATE_HZ: u32 = 48_000;
/// `arm_lead_ms` a unit reports unless set otherwise: the K340's
/// `max(hold_ms, release_ms) + 50` (`extension_channel_spec.md` 2.8.7.1).
pub const DEFAULT_ARM_LEAD_MS: u16 = 150;
/// The longest lead an op 9 may have (WP FWTB1: refused at the write above).
pub const OP9_LEAD_MAX_MS: u64 = 120_000;
/// The S/PDIF follower's `d_min` and `d_max` at 48 kHz in the fake (2.10.2's
/// example values).
pub const SPDIF_D_MIN: u16 = 150;
/// See [`SPDIF_D_MIN`].
pub const SPDIF_D_MAX: u16 = 399;
/// `EXT_ALINK_SPDIF.d`'s default (2.10.1).
pub const SPDIF_D_DEFAULT: u16 = 192;
/// The S/PDIF-type output slots of the demo (four stereo pairs): `tx_mask`
/// bits a source may set.
const SPDIF_SLOT_MASK: u8 = 0x0F;
/// How long an S/PDIF follower anchors after a mode or D change.
const SPDIF_ANCHOR: Duration = Duration::from_millis(30);

/// The line's T at the domain's start (a multiple of 32, as the fake line).
const LINE_BASE: u64 = 1_000_000;
/// The S/PDIF source's F at the domain's start (a multiple of 192).
const SPDIF_BASE: u64 = 960_000;
/// Broadcasts a domain keeps for units that have not looked yet.
const MAX_BROADCASTS: usize = 64;
/// Starts a unit records for [`FakeRemote::measure_timed_starts`].
const MAX_STARTS: usize = 256;
/// The apply queue's depth (2.8.7).
const APPLY_QUEUE: usize = 4;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn low32(v: u64) -> u32 {
    (v & u64::from(u32::MAX)) as u32
}

/// `t + adj`, saturating at the ends of `u64`.
fn add_signed(t: u64, adj: i64) -> u64 {
    if adj >= 0 {
        t.saturating_add(adj.unsigned_abs())
    } else {
        t.saturating_sub(adj.unsigned_abs())
    }
}

/// A notification packet: the v2 header, then `payload`.
fn packet(id: u8, payload: &[u8]) -> Vec<u8> {
    let mut p = vec![nfy::NOTIFY_V2_VERSION as u8, id, 0, 0];
    p.extend_from_slice(payload);
    p
}

// ------------------------------------------------------------ the domain

/// What a [`FakeTimeBase`] counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeBaseKind {
    /// The line: the sender's F (time base 0); domain id byte 0 the sender id.
    Line {
        /// The sender every receiver of the domain hears.
        sender_id: u8,
    },
    /// USB-direct units on one host (time base 1); domain id 0x01000000.
    UsbDirect,
    /// S/PDIF between DSPi units (time base 3): the source's F and epoch.
    Spdif {
        /// The source's serial bytes 0-1 (`source_id`).
        source_id: u16,
    },
}

impl TimeBaseKind {
    /// The time base id (`alink_protocol_spec.md` 11.3).
    pub fn time_base(self) -> u8 {
        match self {
            Self::Line { .. } => tbc::TB_ALINK_LINE,
            Self::UsbDirect => tbc::TB_USB_SAMPLE,
            Self::Spdif { .. } => tbc::TB_SPDIF,
        }
    }

    fn base(self) -> u64 {
        match self {
            Self::Line { .. } => LINE_BASE,
            Self::UsbDirect => 0,
            Self::Spdif { .. } => SPDIF_BASE,
        }
    }
}

/// One op travelling in a superframe.
#[derive(Debug, Clone, Copy)]
struct Broadcast {
    seq: u64,
    /// The domain's T when the superframe was sent.
    published: u64,
    req: ApplyRequest,
}

#[derive(Debug)]
struct Domain {
    kind: TimeBaseKind,
    origin: Instant,
    /// S/PDIF: F's jumps at the epochs so far.
    epoch_offset: u64,
    epoch: u8,
    broadcasts: Vec<Broadcast>,
    next_seq: u64,
    next_unit: u32,
}

impl Domain {
    /// Samples of the domain's clock at `at`.
    fn samples_at(&self, at: Instant) -> u64 {
        let ns = at.saturating_duration_since(self.origin).as_nanos();
        let s = ns
            .saturating_mul(u128::from(TIME_BASE_RATE_HZ))
            .checked_div(1_000_000_000)
            .unwrap_or(0);
        u64::try_from(s).unwrap_or(u64::MAX)
    }

    fn samples_now(&self) -> u64 {
        self.samples_at(Instant::now())
    }

    /// The first instant at which the clock reads `s` samples.
    fn instant_of_samples(&self, s: u64) -> Instant {
        let ns = u128::from(s)
            .saturating_mul(1_000_000_000)
            .div_ceil(u128::from(TIME_BASE_RATE_HZ));
        let d = Duration::from_nanos(u64::try_from(ns).unwrap_or(u64::MAX));
        self.origin.checked_add(d).unwrap_or(self.origin)
    }

    fn zero(&self) -> u64 {
        self.kind.base().saturating_add(self.epoch_offset)
    }

    fn t_at(&self, at: Instant) -> u64 {
        self.zero().saturating_add(self.samples_at(at))
    }

    fn instant_of_t(&self, t: u64) -> Instant {
        self.instant_of_samples(t.saturating_sub(self.zero()))
    }

    /// The source's outputs restart: a new origin on a block boundary well
    /// after the old F, the epoch byte incremented.
    fn new_epoch(&mut self) {
        let t = self.t_at(Instant::now());
        let block = tbc::SPDIF_BLOCK_FRAMES;
        let target = t
            .saturating_add(192_000)
            .checked_div(block)
            .unwrap_or(0)
            .saturating_add(1)
            .saturating_mul(block);
        self.epoch_offset = self.epoch_offset.saturating_add(target.saturating_sub(t));
        self.epoch = self.epoch.wrapping_add(1);
    }

    fn domain_id(&self) -> u32 {
        let tb = u32::from(self.kind.time_base()) << 24;
        match self.kind {
            TimeBaseKind::Line { sender_id } => tb | u32::from(sender_id),
            TimeBaseKind::UsbDirect => tb,
            TimeBaseKind::Spdif { source_id } => {
                tb | (u32::from(self.epoch) << 16) | u32::from(source_id)
            }
        }
    }
}

/// A shared time base for several fakes: one domain (module docs).
#[derive(Clone)]
pub struct FakeTimeBase {
    domain: Arc<Mutex<Domain>>,
}

impl fmt::Debug for FakeTimeBase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let d = lock(&self.domain);
        f.debug_struct("FakeTimeBase")
            .field("kind", &d.kind)
            .field("epoch", &d.epoch)
            .finish_non_exhaustive()
    }
}

impl FakeTimeBase {
    /// A domain of `kind`, its clock starting now.
    pub fn new(kind: TimeBaseKind) -> Self {
        Self {
            domain: Arc::new(Mutex::new(Domain {
                kind,
                origin: Instant::now(),
                epoch_offset: 0,
                epoch: 1,
                broadcasts: Vec::new(),
                next_seq: 1,
                next_unit: 0,
            })),
        }
    }

    /// The line of sender `sender_id`.
    pub fn line(sender_id: u8) -> Self {
        Self::new(TimeBaseKind::Line { sender_id })
    }

    /// USB-direct units on one host.
    pub fn usb_direct() -> Self {
        Self::new(TimeBaseKind::UsbDirect)
    }

    /// An S/PDIF source `source_id` and its followers.
    pub fn spdif(source_id: u16) -> Self {
        Self::new(TimeBaseKind::Spdif { source_id })
    }

    /// What it counts.
    pub fn kind(&self) -> TimeBaseKind {
        lock(&self.domain).kind
    }

    /// T now in the domain's numbering (USB: S of a unit with origin 0).
    pub fn t_now(&self) -> u64 {
        lock(&self.domain).t_at(Instant::now())
    }

    /// The domain id every locked unit reports (C5).
    pub fn domain_id(&self) -> u32 {
        lock(&self.domain).domain_id()
    }

    /// The S/PDIF source's epoch (1 at the start).
    pub fn epoch(&self) -> u8 {
        lock(&self.domain).epoch
    }

    /// The S/PDIF source restarts its outputs: a new epoch and origin. Every
    /// unit of the domain re-anchors on its next request (reason `EPOCH`).
    pub fn new_epoch(&self) {
        lock(&self.domain).new_epoch();
    }

    /// The instant the domain's clock reaches `t` (domain numbering).
    pub fn instant_of(&self, t: u64) -> Instant {
        lock(&self.domain).instant_of_t(t)
    }

    /// A new unit's membership: give each fake its own.
    pub fn join(&self) -> FakeTimeBaseHandle {
        let mut d = lock(&self.domain);
        let id = d.next_unit;
        d.next_unit = d.next_unit.wrapping_add(1);
        let out_origin = d.samples_now();
        let seen = d.next_seq;
        drop(d);
        FakeTimeBaseHandle {
            domain: Arc::clone(&self.domain),
            unit: Arc::new(Mutex::new(UnitTb::new(id, out_origin, seen))),
        }
    }
}

// ------------------------------------------------------------ a unit's membership

/// One fake's place on a [`FakeTimeBase`]: [`super::FakeOptions::time_base`].
/// Equal only to its own clones.
#[derive(Clone)]
pub struct FakeTimeBaseHandle {
    domain: Arc<Mutex<Domain>>,
    unit: Arc<Mutex<UnitTb>>,
}

impl fmt::Debug for FakeTimeBaseHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let id = lock(&self.unit).id;
        f.debug_struct("FakeTimeBaseHandle")
            .field("unit", &id)
            .finish_non_exhaustive()
    }
}

impl PartialEq for FakeTimeBaseHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.unit, &other.unit)
    }
}

impl Eq for FakeTimeBaseHandle {}

impl FakeTimeBaseHandle {
    /// The domain this unit is on.
    pub fn time_base(&self) -> FakeTimeBase {
        FakeTimeBase {
            domain: Arc::clone(&self.domain),
        }
    }
}

/// Per time base id: the last contract state seen, its reason, the counter.
#[derive(Debug, Default, Clone, Copy)]
struct Book {
    last_state: Option<u8>,
    last_reason: u8,
    disturbances: u16,
}

/// An S/PDIF unit's role (`EXT_ALINK_SPDIF`) and state.
#[derive(Debug, Default)]
struct Spdif {
    mode: u8,
    flags: u8,
    d: u16,
    on_lock_error: u8,
    tx_mask: u8,
    holdover: bool,
    lock_error: bool,
    anchoring_until: Option<Instant>,
    seen_epoch: u8,
    lock_count: u32,
    loss_count: u32,
    sf_errors: u32,
    reanchors: u32,
    queue: Vec<ApplyRequest>,
    last: Option<ApplyStatus>,
    /// The source's pending broadcast.
    ctrl: Option<ApplyRequest>,
    ctrl_last: Option<ApplyStatus>,
}

/// A generator start a unit recorded ([`FakeRemote::measure_timed_starts`]).
#[derive(Debug, Clone, PartialEq)]
pub struct TimedStart {
    /// `EXT_SIGGEN_TIMELINE.run_id` of the run.
    pub run_id: u32,
    /// The time base of `t` (`0xFF`: a plain START without one).
    pub time_base: u8,
    /// T of generator sample 0, in the unit's own numbering (0 without one).
    pub t: u64,
    /// Started by op 9 (else a plain `SIGGEN_CTL_START` or a restart).
    pub timed: bool,
    /// What it plays.
    pub config: SiggenConfig,
}

#[derive(Debug)]
struct UnitTb {
    id: u32,
    /// Domain samples when the unit's outputs last started in sync.
    out_origin: u64,
    /// Whole-sample corrections on the line or S/PDIF T (USB's are kept by
    /// `fake/alink_usb.rs`).
    adj: i64,
    books: [Book; 4],
    /// A holdover or lock error forced onto a line or USB-direct entry.
    force: Option<u8>,
    spdif: Option<Spdif>,
    bcast_seen: u64,
    /// Broadcast ops taken, waiting for their T (with the domain's tb).
    bcast_pending: Vec<ApplyRequest>,
    starts: Vec<TimedStart>,
    arm_lead_ms: u16,
    /// The tunnel's refusal code for the last refused write (`VC_ERR_*`).
    refusal: Option<u8>,
}

impl UnitTb {
    fn new(id: u32, out_origin: u64, bcast_seen: u64) -> Self {
        Self {
            id,
            out_origin,
            adj: 0,
            books: [Book::default(); 4],
            force: None,
            spdif: None,
            bcast_seen,
            bcast_pending: Vec::new(),
            starts: Vec::new(),
            arm_lead_ms: DEFAULT_ARM_LEAD_MS,
            refusal: None,
        }
    }
}

/// An S/PDIF unit's role at its start ([`FakeTimeBaseUnit::Spdif`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpdifSetup {
    /// 1 follower, 2 source (0: off, today's S/PDIF).
    pub mode: u8,
    /// The follower's D, samples.
    pub d: u16,
    /// 0 mute, 1 play unaligned.
    pub on_lock_error: u8,
    /// The source's stamped slots (0: every S/PDIF slot).
    pub tx_mask: u8,
}

impl SpdifSetup {
    /// A follower with D `d`, muting on lock error.
    pub fn follower(d: u16) -> Self {
        Self {
            mode: tbc::SPDIF_MODE_FOLLOWER,
            d,
            on_lock_error: 0,
            tx_mask: 0,
        }
    }

    /// The source, stamping every S/PDIF slot.
    pub fn source() -> Self {
        Self {
            mode: tbc::SPDIF_MODE_SOURCE,
            d: SPDIF_D_DEFAULT,
            on_lock_error: 0,
            tx_mask: 0,
        }
    }
}

/// Where a fake made by [`FakeDevice::demo_on_time_base`] sits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FakeTimeBaseUnit {
    /// A line receiver (or sender) on a [`TimeBaseKind::Line`] domain; its
    /// `sender_id` becomes the domain's.
    Line(AlinkSetup),
    /// A USB-direct unit on a [`TimeBaseKind::UsbDirect`] domain.
    UsbDirect(UsbDirectSetup),
    /// An S/PDIF follower or source on a [`TimeBaseKind::Spdif`] domain.
    Spdif(SpdifSetup),
}

/// What [`FakeRemote::time_base_fault`] does to a unit's time base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeBaseFault {
    /// The carrier goes: `HOLDOVER` (the line: LOS).
    Holdover,
    /// Back from a holdover on the same timeline: `RESUMED`.
    Resume,
    /// The timeline moves: anchoring, then locked again (`REANCHOR`).
    Reanchor,
    /// Whole samples dropped (negative) or inserted on T; `event`: reported
    /// (`0xE4` `CORRECTION`, a disturbance), else silent.
    Correction {
        /// Samples T moves by.
        samples: i32,
        /// Whether the unit reports it.
        event: bool,
    },
    /// The S/PDIF source restarts (a new epoch; the domain's, any unit).
    Epoch,
    /// Lock error on (`LOCK_ERROR`) or off (`LOCK_OK`).
    LockError(bool),
}

// ------------------------------------------------------------ the contract read

/// One `EXT_TIME_BASE` entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Entry {
    pub(super) tb: u8,
    pub(super) role: u8,
    pub(super) state: u8,
    pub(super) domain: u32,
    pub(super) t: u64,
    pub(super) d: u16,
    pub(super) phase_error: f32,
}

/// Which apply queue an op is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Queue {
    /// The transport's 4-deep queue of time base `tb`.
    Own(u8),
    /// Taken from a broadcast.
    Broadcast,
}

fn is_disturbance(r: u8) -> bool {
    matches!(
        r,
        reason::HOLDOVER | reason::RESUMED | reason::REANCHOR | reason::CORRECTION | reason::EPOCH
    )
}

/// The reason of a transition found by looking (not reported by a fault).
fn derived_reason(prev: u8, new: u8, present: bool) -> u8 {
    match new {
        tbc::TB_STATE_LOCKED => match prev {
            tbc::TB_STATE_HOLDOVER => reason::RESUMED,
            tbc::TB_STATE_LOCK_ERROR => reason::LOCK_OK,
            _ => reason::LOCKED,
        },
        tbc::TB_STATE_HOLDOVER => reason::HOLDOVER,
        tbc::TB_STATE_LOCK_ERROR => reason::LOCK_ERROR,
        tbc::TB_STATE_ANCHORING if prev == tbc::TB_STATE_LOCKED => reason::REANCHOR,
        _ if !present => reason::DISABLED,
        _ if matches!(prev, tbc::TB_STATE_LOCKED | tbc::TB_STATE_HOLDOVER) => reason::LOST,
        _ if prev == tbc::TB_STATE_LOCK_ERROR => reason::LOCK_OK,
        _ => reason::NONE,
    }
}

/// The time base ids an entry can have.
const TBS: [u8; 3] = [tbc::TB_ALINK_LINE, tbc::TB_USB_SAMPLE, tbc::TB_SPDIF];

impl State {
    fn tb_unit<R>(&self, f: impl FnOnce(&mut UnitTb) -> R) -> Option<R> {
        let h = self.options.time_base.as_ref()?;
        let mut u = lock(&h.unit);
        Some(f(&mut u))
    }

    fn tb_domain<R>(&self, f: impl FnOnce(&mut Domain) -> R) -> Option<R> {
        let h = self.options.time_base.as_ref()?;
        let mut d = lock(&h.domain);
        Some(f(&mut d))
    }

    /// The domain's kind, with a time base.
    pub(super) fn tb_kind(&self) -> Option<TimeBaseKind> {
        self.tb_domain(|d| d.kind)
    }

    /// Whether this fake is firmware with the ops and the contract read.
    pub(super) fn tb_on(&self) -> bool {
        self.options.time_base.is_some()
    }

    /// The line's T on the domain's clock (with its corrections), when the
    /// fake's time base is a line.
    pub(super) fn tb_line_t(&self) -> Option<u64> {
        let h = self.options.time_base.as_ref()?;
        let adj = lock(&h.unit).adj;
        let d = lock(&h.domain);
        matches!(d.kind, TimeBaseKind::Line { .. }).then(|| add_signed(d.t_at(Instant::now()), adj))
    }

    /// The domain's clock origin, for a USB-direct unit on one.
    pub(super) fn tb_usb_origin(&self) -> Option<Instant> {
        let h = self.options.time_base.as_ref()?;
        let d = lock(&h.domain);
        (d.kind == TimeBaseKind::UsbDirect).then_some(d.origin)
    }

    /// The instant the domain's clock reads `s` samples.
    pub(super) fn tb_instant_of_samples(&self, s: u64) -> Option<Instant> {
        self.tb_domain(|d| d.instant_of_samples(s))
    }

    /// The domain's clock reading at which T on `tb` is `t`.
    fn tb_samples_of(&self, tb: u8, t: u64) -> Option<u64> {
        if tb == tbc::TB_USB_SAMPLE {
            return self.alink_usb_dom_samples(t);
        }
        let adj = self.tb_unit(|u| u.adj)?;
        self.tb_domain(|d| add_signed(t, adj.saturating_neg()).saturating_sub(d.zero()))
    }

    /// Output frames at the instant T on `tb` read `t` (0: now), so `t` and
    /// `output_frames` are one reading (review R7c).
    pub(super) fn tb_output_frames_at(&self, tb: u8, t: u64) -> Option<u32> {
        let origin = self.tb_unit(|u| u.out_origin)?;
        let at = match t {
            0 => None,
            t => self.tb_samples_of(tb, t),
        };
        let now = match at {
            Some(s) => s,
            None => self.tb_domain(|d| d.samples_now())?,
        };
        Some(low32(now.saturating_sub(origin)))
    }

    /// The `arm_lead_ms` this unit reports.
    pub(super) fn tb_arm_lead_ms(&self) -> u16 {
        self.tb_unit(|u| u.arm_lead_ms)
            .unwrap_or(DEFAULT_ARM_LEAD_MS)
    }

    fn tb_arm_lead_samples(&self) -> u64 {
        u64::from(self.tb_arm_lead_ms()).saturating_mul(u64::from(TIME_BASE_RATE_HZ) / 1000)
    }

    /// The S/PDIF unit's F now (with its corrections).
    fn spdif_t(&self) -> Option<u64> {
        let h = self.options.time_base.as_ref()?;
        let adj = lock(&h.unit).adj;
        let d = lock(&h.domain);
        matches!(d.kind, TimeBaseKind::Spdif { .. })
            .then(|| add_signed(d.t_at(Instant::now()), adj))
    }

    /// T now on time base `tb`, the unit's numbering.
    pub(super) fn tb_t_now(&mut self, tb: u8) -> Option<u64> {
        match tb {
            tbc::TB_ALINK_LINE => self.tb_line_t(),
            tbc::TB_USB_SAMPLE => self.alink_usb.on.then(|| self.alink_usb_sample()),
            tbc::TB_SPDIF => self.spdif_t(),
            _ => None,
        }
    }

    /// The instant the unit's T on `tb` reaches `t`.
    fn tb_instant_of(&self, tb: u8, t: u64) -> Instant {
        let now = Instant::now();
        match tb {
            tbc::TB_USB_SAMPLE => self.alink_usb_instant_of(t).unwrap_or(now),
            _ => {
                let adj = self.tb_unit(|u| u.adj).unwrap_or(0);
                self.tb_domain(|d| d.instant_of_t(add_signed(t, adj.saturating_neg())))
                    .unwrap_or(now)
            }
        }
    }

    // -------------------------------------------------- S/PDIF

    fn spdif_present(&self) -> bool {
        self.tb_unit(|u| u.spdif.is_some()).unwrap_or(false)
    }

    /// The S/PDIF role's contract state, `None` in mode 0 (no entry).
    fn spdif_state(&self) -> Option<(u8, u8)> {
        let now = Instant::now();
        self.tb_unit(|u| {
            let s = u.spdif.as_ref()?;
            let state = match s.mode {
                tbc::SPDIF_MODE_SOURCE => {
                    if s.anchoring_until.is_some_and(|t| now < t) {
                        tbc::TB_STATE_ANCHORING
                    } else {
                        tbc::TB_STATE_LOCKED
                    }
                }
                tbc::SPDIF_MODE_FOLLOWER => {
                    if s.lock_error {
                        tbc::TB_STATE_LOCK_ERROR
                    } else if s.holdover {
                        tbc::TB_STATE_HOLDOVER
                    } else if s.anchoring_until.is_some_and(|t| now < t) {
                        tbc::TB_STATE_ANCHORING
                    } else {
                        tbc::TB_STATE_LOCKED
                    }
                }
                _ => return None,
            };
            let role = if s.mode == tbc::SPDIF_MODE_SOURCE {
                tbc::TB_ROLE_SOURCE
            } else {
                tbc::TB_ROLE_FOLLOWER
            };
            Some((state, role))
        })
        .flatten()
    }

    fn spdif_entry(&self) -> Option<Entry> {
        let (state, role) = self.spdif_state()?;
        let domain = self.tb_domain(|d| d.domain_id()).unwrap_or(0);
        let d = self
            .tb_unit(|u| u.spdif.as_ref().map_or(0, |s| s.d))
            .unwrap_or(0);
        let follower = role == tbc::TB_ROLE_FOLLOWER;
        let locked = state == tbc::TB_STATE_LOCKED;
        let has_t = matches!(
            state,
            tbc::TB_STATE_LOCKED | tbc::TB_STATE_HOLDOVER | tbc::TB_STATE_ANCHORING
        );
        Some(Entry {
            tb: tbc::TB_SPDIF,
            role,
            state,
            domain,
            t: if has_t {
                self.spdif_t().unwrap_or(0)
            } else {
                0
            },
            d: if follower { d } else { 0 },
            phase_error: if follower && locked { 0.03 } else { 0.0 },
        })
    }

    // -------------------------------------------------- the entries

    /// The unit's `EXT_TIME_BASE` entries, followers first.
    pub(super) fn tb_entries(&mut self) -> Vec<Entry> {
        let mut v = Vec::new();
        let force = self.tb_unit(|u| u.force).flatten();
        let forced = |mut e: Entry| {
            if let Some(f) = force
                && e.state == tbc::TB_STATE_LOCKED
            {
                e.state = f;
                if f != tbc::TB_STATE_HOLDOVER {
                    e.phase_error = 0.0;
                }
            }
            e
        };
        if let Some(e) = self.alink_tb_entry() {
            v.push(forced(e));
        }
        if let Some(e) = self.alink_usb_tb_entry() {
            v.push(forced(e));
        }
        if let Some(e) = self.spdif_entry() {
            v.push(e);
        }
        v.sort_by_key(|e| e.role);
        v.truncate(tbc::TIME_BASE_MAX_ENTRIES);
        v
    }

    /// The entry of time base `tb`, if the unit has one.
    pub(super) fn tb_entry(&mut self, tb: u8) -> Option<Entry> {
        self.tb_entries().into_iter().find(|e| e.tb == tb)
    }

    /// Whether `tb`'s entry is `LOCKED`.
    pub(super) fn tb_locked(&mut self, tb: u8) -> bool {
        self.tb_entry(tb)
            .is_some_and(|e| e.state == tbc::TB_STATE_LOCKED)
    }

    /// The first `LOCKED` entry's time base and T (a plain START's stamp).
    pub(super) fn tb_first_locked(&mut self) -> Option<(u8, u64)> {
        self.tb_entries()
            .into_iter()
            .find(|e| e.state == tbc::TB_STATE_LOCKED)
            .map(|e| (e.tb, e.t))
    }

    /// `EXT_TIME_BASE` (2.11.1), every entry and `output_frames` at one
    /// instant of the domain's clock.
    fn time_base_reply(&mut self) -> Vec<u8> {
        let entries = self.tb_entries();
        let us = Instant::now()
            .saturating_duration_since(self.started)
            .as_micros();
        let out = entries
            .first()
            .and_then(|e| self.tb_output_frames_at(e.tb, e.t))
            .unwrap_or(0);
        let books = self.tb_unit(|u| u.books).unwrap_or_default();
        let mut b = vec![0u8; tbc::TIME_BASE_LEN];
        let mut put = |at: usize, bytes: &[u8]| {
            if let Some(s) = b.get_mut(at..at.saturating_add(bytes.len())) {
                s.copy_from_slice(bytes);
            }
        };
        put(0, &[tbc::TIME_BASE_LAYOUT_VERSION]);
        put(1, &[u8::try_from(entries.len()).unwrap_or(0)]);
        put(4, &((us & u128::from(u32::MAX)) as u32).to_le_bytes());
        put(8, &out.to_le_bytes());
        for (i, e) in entries.iter().enumerate() {
            let at = tbc::TIME_BASE_ENTRY_OFFSET
                .saturating_add(i.saturating_mul(tbc::TIME_BASE_ENTRY_LEN));
            let book = books.get(usize::from(e.tb)).copied().unwrap_or_default();
            put(at, &[e.tb, e.role, e.state, book.last_reason]);
            put(at.saturating_add(4), &e.domain.to_le_bytes());
            put(at.saturating_add(8), &e.t.to_le_bytes());
            put(at.saturating_add(16), &book.disturbances.to_le_bytes());
            put(at.saturating_add(18), &e.d.to_le_bytes());
            put(at.saturating_add(20), &e.phase_error.to_le_bytes());
        }
        b
    }

    // -------------------------------------------------- transitions

    /// Record a contract transition (or a disturbance within `LOCKED`) of
    /// `tb` to `new` with `why`: the counter, `0xE4`, the generator's rules.
    pub(super) fn tb_emit(&mut self, tb: u8, new: u8, why: u8) {
        let t = self.tb_t_now(tb).unwrap_or(0);
        let Some((prev, left, disturbed)) = self
            .tb_unit(|u| {
                let b = u.books.get_mut(usize::from(tb))?;
                let prev = b.last_state.unwrap_or(tbc::TB_STATE_NONE);
                let left = prev == tbc::TB_STATE_LOCKED && new != tbc::TB_STATE_LOCKED;
                let disturbed = left || is_disturbance(why);
                if disturbed {
                    b.disturbances = b.disturbances.wrapping_add(1);
                }
                b.last_state = Some(new);
                b.last_reason = why;
                Some((prev, left, disturbed))
            })
            .flatten()
        else {
            return;
        };
        let mut e = [0u8; 8];
        e[0] = tb;
        e[1] = new;
        e[2] = why;
        e[4..].copy_from_slice(&low32(t).to_le_bytes());
        self.push(packet(tbc::NOTIFY_EVT_TIME_BASE, &e));
        // The outputs restart in sync at every anchor (not after a holdover).
        if new == tbc::TB_STATE_LOCKED && !matches!(prev, tbc::TB_STATE_HOLDOVER) {
            self.tb_restart_outputs();
        }
        if left {
            self.tb_cancel_pending(tbc::timeline_result::CANCELLED_LOSS, Some(tb));
            self.siggen_loss(tb);
        } else if disturbed {
            self.siggen_disturbed();
        }
    }

    /// The outputs start in sync now; an S/PDIF unit's on a block boundary
    /// (F mod 192 = 0, `alink_protocol_spec.md` 12.3).
    fn tb_restart_outputs(&mut self) {
        let spdif = self.spdif_present();
        let Some(s0) = self.tb_domain(|d| {
            let now = d.samples_now();
            if spdif {
                let phase = d.zero().saturating_add(now) % tbc::SPDIF_BLOCK_FRAMES;
                now.saturating_sub(phase)
            } else {
                now
            }
        }) else {
            return;
        };
        self.tb_unit(|u| u.out_origin = s0);
    }

    /// Look at every entry and report what changed since the last look.
    pub(super) fn tb_sync(&mut self) {
        let entries = self.tb_entries();
        for tb in TBS {
            let cur = entries.iter().find(|e| e.tb == tb).map(|e| e.state);
            let last = self
                .tb_unit(|u| u.books.get(usize::from(tb)).and_then(|b| b.last_state))
                .flatten();
            let new = cur.unwrap_or(tbc::TB_STATE_NONE);
            let changed = match last {
                None => cur.is_some(),
                Some(l) => l != new,
            };
            if changed {
                let prev = last.unwrap_or(tbc::TB_STATE_NONE);
                self.tb_emit(tb, new, derived_reason(prev, new, cur.is_some()));
            }
        }
    }

    /// Note the states now without events (a unit's start).
    fn tb_sync_quiet(&mut self) {
        let entries = self.tb_entries();
        self.tb_unit(|u| {
            for e in &entries {
                if let Some(b) = u.books.get_mut(usize::from(e.tb)) {
                    b.last_state = Some(e.state);
                    b.last_reason = if e.state == tbc::TB_STATE_LOCKED {
                        reason::LOCKED
                    } else {
                        reason::NONE
                    };
                }
            }
        });
    }

    /// A fault on `tb` happened with `why`: a state change with that reason,
    /// or a disturbance within the state.
    pub(super) fn tb_note(&mut self, tb: u8, why: u8) {
        if !self.tb_on() {
            return;
        }
        let cur = self.tb_entry(tb).map(|e| e.state);
        let last = self
            .tb_unit(|u| u.books.get(usize::from(tb)).and_then(|b| b.last_state))
            .flatten();
        let new = cur.unwrap_or(tbc::TB_STATE_NONE);
        if last != Some(new) || is_disturbance(why) {
            self.tb_emit(tb, new, why);
        }
        self.tb_sync();
    }

    /// Settle what time did (an anchor ending, an epoch the follower has not
    /// seen), take broadcasts, run the ops that are due and the arming check.
    /// Every request on a fake with a time base starts here.
    pub(super) fn tb_service(&mut self) {
        if !self.tb_on() {
            return;
        }
        self.spdif_settle();
        self.tb_sync();
        self.tb_ingest();
        if self.options.alink {
            self.run_due_ops();
        }
        if self.alink_usb.on {
            self.run_due_usb_ops();
        }
        self.spdif_run_due();
        self.bcast_run_due();
        self.tb_arm_check();
    }

    /// An S/PDIF unit: the end of an anchor, a new epoch of the source.
    fn spdif_settle(&mut self) {
        let epoch = self.tb_domain(|d| d.epoch).unwrap_or(0);
        let now = Instant::now();
        let changed = self
            .tb_unit(|u| {
                let s = u.spdif.as_mut()?;
                if s.anchoring_until.is_some_and(|t| now >= t) {
                    s.anchoring_until = None;
                }
                if s.mode != tbc::SPDIF_MODE_OFF && s.seen_epoch != epoch {
                    s.seen_epoch = epoch;
                    s.reanchors = s.reanchors.wrapping_add(1);
                    s.anchoring_until = None;
                    return Some(());
                }
                None
            })
            .flatten();
        if changed.is_some() {
            // The outputs restart (a source) or the follower re-anchors.
            self.tb_emit(tbc::TB_SPDIF, tbc::TB_STATE_ANCHORING, reason::EPOCH);
            self.tb_emit(tbc::TB_SPDIF, tbc::TB_STATE_LOCKED, reason::LOCKED);
        }
    }

    // -------------------------------------------------- ops 9 and 10

    /// The tunnel's code for the refusal just made, once.
    pub(super) fn tb_take_refusal(&mut self) -> Option<u8> {
        self.tb_unit(|u| u.refusal.take()).flatten()
    }

    fn refuse(&self, code: u8) -> bool {
        self.tb_unit(|u| u.refusal = Some(code));
        false
    }

    fn queue_of(&mut self, tb: u8) -> Option<&mut Vec<ApplyRequest>> {
        match tb {
            tbc::TB_ALINK_LINE => Some(self.alink_queue()),
            tbc::TB_USB_SAMPLE => Some(self.alink_usb_queue()),
            _ => None,
        }
    }

    fn queue_len(&mut self, tb: u8) -> usize {
        if tb == tbc::TB_SPDIF {
            return self
                .tb_unit(|u| u.spdif.as_ref().map_or(0, |s| s.queue.len()))
                .unwrap_or(0);
        }
        self.queue_of(tb).map_or(0, |q| q.len())
    }

    fn queue_push(&mut self, tb: u8, r: ApplyRequest) {
        if tb == tbc::TB_SPDIF {
            self.tb_unit(|u| {
                if let Some(s) = u.spdif.as_mut() {
                    s.queue.push(r);
                }
            });
        } else if let Some(q) = self.queue_of(tb) {
            q.push(r);
        }
    }

    /// Take op 9 at `t` out of queue `q`.
    fn queue_take_start(&mut self, q: Queue, tb: u8, t: u64) -> Option<ApplyRequest> {
        let pick = |v: &mut Vec<ApplyRequest>| {
            let i = v
                .iter()
                .position(|r| r.op == ApplyOp::SiggenStart && r.frame == t)?;
            Some(v.remove(i))
        };
        match (q, tb) {
            (Queue::Broadcast, _) => self.tb_unit(|u| pick(&mut u.bcast_pending)).flatten(),
            (Queue::Own(tbc::TB_SPDIF), _) => self
                .tb_unit(|u| u.spdif.as_mut().and_then(|s| pick(&mut s.queue)))
                .flatten(),
            (Queue::Own(x), _) => self.queue_of(x).and_then(pick),
        }
    }

    /// An op of time base `tb` completed: `0xE2` and the queue's IN record.
    pub(super) fn tb_op_done(&mut self, tb: u8, r: ApplyRequest, result: ApplyResult) {
        match tb {
            tbc::TB_ALINK_LINE => self.alink_op_done(r, result),
            tbc::TB_USB_SAMPLE => self.alink_usb_op_done(r, result),
            _ => {
                self.tb_unit(|u| {
                    if let Some(s) = u.spdif.as_mut() {
                        s.last = Some(ApplyStatus {
                            pending: 0,
                            last_op: r.op,
                            last_result: result,
                            last_frame: r.frame,
                        });
                    }
                });
                let e = ApplyResultEvent {
                    op: r.op,
                    result,
                    frame_low: low32(r.frame),
                };
                self.push(packet(NOTIFY_EVT_APPLY_RESULT, &e.encode()));
            }
        }
    }

    /// Cancel the pending op 9 (on `only` if given): its entry leaves its
    /// queue with `0xE2` cancelled, the timeline says `result`.
    pub(super) fn tb_cancel_pending(&mut self, result: u8, only: Option<u8>) {
        let Some(p) = self.siggen_pending() else {
            return;
        };
        if only.is_some_and(|tb| tb != p.tb) {
            return;
        }
        self.siggen_take_pending(result);
        let q = if p.broadcast {
            Queue::Broadcast
        } else {
            Queue::Own(p.tb)
        };
        if let Some(r) = self.queue_take_start(q, p.tb, p.t) {
            self.tb_op_done(p.tb, r, ApplyResult::Cancelled);
        }
    }

    /// `CANCEL_ALL` of time base `tb` (before the transport cancels its own
    /// queue): the staged config is no longer fresh, and a pending op 9 there
    /// is cancelled by the host.
    pub(super) fn tb_on_cancel_all(&mut self, tb: u8) {
        if !self.tb_on() {
            return;
        }
        self.siggen_unfresh();
        if let Some(p) = self.siggen_pending()
            && p.tb == tb
        {
            self.siggen_take_pending(tbc::timeline_result::CANCELLED_HOST);
            if p.broadcast
                && let Some(r) = self.queue_take_start(Queue::Broadcast, tb, p.t)
            {
                self.tb_op_done(tb, r, ApplyResult::Cancelled);
            }
        }
    }

    /// Op 9 or 10 written to time base `tb`'s own queue (the transport has
    /// checked the envelope, the lock and the arguments; `r.frame` is T in
    /// the unit's numbering). True: accepted.
    pub(super) fn tb_write_siggen_op(&mut self, tb: u8, r: ApplyRequest) -> bool {
        if !self.tb_on() || !r.args_valid() {
            return self.refuse(lm::VC_ERR_VALUE);
        }
        if !self.tb_locked(tb) {
            return self.refuse(lm::VC_ERR_STATE);
        }
        let now = self.tb_t_now(tb).unwrap_or(0);
        match r.op {
            ApplyOp::SiggenStart => {
                if tb == tbc::TB_USB_BUS_FRAME {
                    return self.refuse(lm::VC_ERR_VALUE);
                }
                let lead = i128::from(r.frame).saturating_sub(i128::from(now));
                let max =
                    i128::from(OP9_LEAD_MAX_MS.saturating_mul(u64::from(TIME_BASE_RATE_HZ) / 1000));
                let lead_ok = lead >= i128::from(self.tb_arm_lead_samples()) && lead <= max;
                if !lead_ok || self.siggen_pending().is_some() || self.queue_len(tb) >= APPLY_QUEUE
                {
                    return self.refuse(lm::VC_ERR_STATE);
                }
                self.queue_push(tb, r);
                self.siggen_set_pending(PendingStart {
                    tb,
                    t: r.frame,
                    broadcast: false,
                });
                true
            }
            ApplyOp::SiggenStop => {
                if r.frame <= now {
                    // A late stop is applied at once.
                    self.siggen_timed_stop(r.arg, Instant::now());
                    self.tb_op_done(tb, r, ApplyResult::Late);
                    return true;
                }
                if self.queue_len(tb) >= APPLY_QUEUE {
                    return self.refuse(lm::VC_ERR_STATE);
                }
                self.queue_push(tb, r);
                true
            }
            _ => self.refuse(lm::VC_ERR_VALUE),
        }
    }

    /// Op 9 or 10 of time base `tb` reached its T: the result to report.
    pub(super) fn tb_siggen_due(&mut self, tb: u8, r: ApplyRequest) -> ApplyResult {
        let at = self.tb_instant_of(tb, r.frame);
        match r.op {
            ApplyOp::SiggenStart => {
                let ours = self
                    .siggen_pending()
                    .is_some_and(|p| p.tb == tb && p.t == r.frame);
                if !ours {
                    return ApplyResult::Refused;
                }
                let locked = self.tb_locked(tb);
                if self.siggen_timed_start(tb, r.frame, at, locked) {
                    ApplyResult::Applied
                } else {
                    ApplyResult::Refused
                }
            }
            ApplyOp::SiggenStop => {
                self.siggen_timed_stop(r.arg, at);
                ApplyResult::Applied
            }
            _ => ApplyResult::Applied,
        }
    }

    /// At `T - arm_lead` the preconditions are checked: a pending op 9 whose
    /// config is not finite and fresh completes refused then.
    fn tb_arm_check(&mut self) {
        let Some(p) = self.siggen_pending() else {
            return;
        };
        let now = self.tb_t_now(p.tb).unwrap_or(0);
        let armed = now >= p.t.saturating_sub(self.tb_arm_lead_samples());
        if armed && !self.siggen_ready_to_arm() {
            self.siggen_take_pending(tbc::timeline_result::REFUSED_NOT_READY);
            let q = if p.broadcast {
                Queue::Broadcast
            } else {
                Queue::Own(p.tb)
            };
            if let Some(r) = self.queue_take_start(q, p.tb, p.t) {
                self.tb_op_done(p.tb, r, ApplyResult::Refused);
            }
        }
    }

    /// Whether the pending op 9 is armed now.
    pub(super) fn tb_pending_armed(&mut self) -> bool {
        let Some(p) = self.siggen_pending() else {
            return false;
        };
        let now = self.tb_t_now(p.tb).unwrap_or(0);
        now >= p.t.saturating_sub(self.tb_arm_lead_samples())
    }

    /// The starts this unit recorded, oldest first.
    pub(super) fn tb_starts(&self) -> Vec<TimedStart> {
        self.tb_unit(|u| u.starts.clone()).unwrap_or_default()
    }

    /// Record a generator start for the hooks.
    pub(super) fn tb_log_start(&mut self, s: TimedStart) {
        self.tb_unit(|u| {
            if u.starts.len() >= MAX_STARTS {
                u.starts.remove(0);
            }
            u.starts.push(s);
        });
    }

    // -------------------------------------------------- the S/PDIF queue

    fn spdif_run_due(&mut self) {
        let Some(now) = self.spdif_t() else {
            return;
        };
        let mut due = self
            .tb_unit(|u| {
                let s = u.spdif.as_mut()?;
                let mut due = Vec::new();
                s.queue.retain(|r| {
                    let d = r.frame <= now;
                    if d {
                        due.push(*r);
                    }
                    !d
                });
                Some(due)
            })
            .flatten()
            .unwrap_or_default();
        due.sort_by_key(|r| r.frame);
        for r in due {
            let result = match r.op {
                ApplyOp::SiggenStart | ApplyOp::SiggenStop => self.tb_siggen_due(tbc::TB_SPDIF, r),
                _ => ApplyResult::Applied,
            };
            self.tb_op_done(tbc::TB_SPDIF, r, result);
        }
    }

    /// `EXT_APPLY_AT_FRAME` OUT with time base 3 (2.10.4).
    fn spdif_apply(&mut self, data: &[u8]) -> bool {
        let Some(r) = ApplyRequest::decode(data) else {
            return self.refuse(lm::VC_ERR_LENGTH);
        };
        if data.len() != ApplyRequest::SIZE || data.get(13..) != Some(&[0; 3]) {
            return self.refuse(lm::VC_ERR_VALUE);
        }
        if !self.spdif_present() {
            return self.refuse(lm::VC_ERR_STATE);
        }
        if r.op == ApplyOp::CancelAll {
            self.tb_on_cancel_all(tbc::TB_SPDIF);
            let all = self
                .tb_unit(|u| u.spdif.as_mut().map(|s| std::mem::take(&mut s.queue)))
                .flatten()
                .unwrap_or_default();
            for q in all {
                self.tb_op_done(tbc::TB_SPDIF, q, ApplyResult::Cancelled);
            }
            return true;
        }
        if !r.args_valid() || r.op == ApplyOp::Rate {
            return self.refuse(lm::VC_ERR_VALUE);
        }
        if !self.tb_locked(tbc::TB_SPDIF) {
            return self.refuse(lm::VC_ERR_STATE);
        }
        if matches!(r.op, ApplyOp::SiggenStart | ApplyOp::SiggenStop) {
            return self.tb_write_siggen_op(tbc::TB_SPDIF, r);
        }
        let now = self.spdif_t().unwrap_or(0);
        if r.frame <= now {
            self.tb_op_done(tbc::TB_SPDIF, r, ApplyResult::Late);
            return true;
        }
        if self.queue_len(tbc::TB_SPDIF) >= APPLY_QUEUE {
            return self.refuse(lm::VC_ERR_STATE);
        }
        self.queue_push(tbc::TB_SPDIF, r);
        true
    }

    fn spdif_queue_in(&mut self) -> Vec<u8> {
        let (pending, last) = self
            .tb_unit(|u| {
                u.spdif
                    .as_ref()
                    .map(|s| (s.queue.len(), s.last))
                    .unwrap_or((0, None))
            })
            .unwrap_or((0, None));
        apply_status_in(pending, last)
    }

    // -------------------------------------------------- broadcasts

    /// Publish a broadcast op on the domain (the line sender's superframe or
    /// the S/PDIF source's).
    pub(super) fn tb_broadcast(&mut self, req: ApplyRequest) {
        self.tb_domain(|d| {
            let published = d.t_at(Instant::now());
            let seq = d.next_seq;
            d.next_seq = d.next_seq.wrapping_add(1);
            if d.broadcasts.len() >= MAX_BROADCASTS {
                d.broadcasts.remove(0);
            }
            d.broadcasts.push(Broadcast {
                seq,
                published,
                req,
            });
        });
    }

    /// Whether this unit hears the domain's superframes: a line receiver on a
    /// line domain, an S/PDIF follower or source on an S/PDIF domain.
    fn hears_broadcasts(&self) -> Option<u8> {
        match self.tb_kind()? {
            TimeBaseKind::Line { .. } if self.alink_receives() => Some(tbc::TB_ALINK_LINE),
            TimeBaseKind::Spdif { .. } if self.spdif_state().is_some() => Some(tbc::TB_SPDIF),
            _ => None,
        }
    }

    /// Take every broadcast this unit has not seen.
    fn tb_ingest(&mut self) {
        let Some(tb) = self.hears_broadcasts() else {
            return;
        };
        let seen = self.tb_unit(|u| u.bcast_seen).unwrap_or(u64::MAX);
        let new: Vec<Broadcast> = self
            .tb_domain(|d| {
                let v: Vec<Broadcast> = d
                    .broadcasts
                    .iter()
                    .filter(|b| b.seq >= seen)
                    .copied()
                    .collect();
                v
            })
            .unwrap_or_default();
        if let Some(last) = new.last() {
            let next = last.seq.saturating_add(1);
            self.tb_unit(|u| u.bcast_seen = next);
        }
        let arm = self.tb_arm_lead_samples();
        let max = OP9_LEAD_MAX_MS.saturating_mul(u64::from(TIME_BASE_RATE_HZ) / 1000);
        for b in new {
            let r = b.req;
            if r.op == ApplyOp::SiggenStart {
                let lead = r.frame.saturating_sub(b.published);
                let ok = r.frame > b.published
                    && lead >= arm
                    && lead <= max
                    && self.siggen_pending().is_none()
                    && self.siggen_present();
                if !ok {
                    self.siggen_note_result(tbc::timeline_result::REFUSED_NOT_READY);
                    self.bcast_done(tb, r, ApplyResult::Refused);
                    continue;
                }
                self.siggen_set_pending(PendingStart {
                    tb,
                    t: r.frame,
                    broadcast: true,
                });
            }
            self.tb_unit(|u| u.bcast_pending.push(r));
        }
    }

    fn bcast_run_due(&mut self) {
        let Some(tb) = self.hears_broadcasts() else {
            return;
        };
        let now = self.tb_t_now(tb).unwrap_or(0);
        let mut due = self
            .tb_unit(|u| {
                let mut due = Vec::new();
                u.bcast_pending.retain(|r| {
                    let d = r.frame <= now;
                    if d {
                        due.push(*r);
                    }
                    !d
                });
                due
            })
            .unwrap_or_default();
        due.sort_by_key(|r| r.frame);
        for r in due {
            let result = match r.op {
                ApplyOp::SiggenStart | ApplyOp::SiggenStop => self.tb_siggen_due(tb, r),
                _ => ApplyResult::Applied,
            };
            self.bcast_done(tb, r, result);
        }
    }

    /// A broadcast op completed on this unit: `0xE2`, and on the source its
    /// broadcast queue's IN.
    fn bcast_done(&mut self, tb: u8, r: ApplyRequest, result: ApplyResult) {
        self.tb_unit(|u| {
            if let Some(s) = u.spdif.as_mut()
                && s.mode == tbc::SPDIF_MODE_SOURCE
            {
                if s.ctrl.is_some_and(|c| c.frame == r.frame && c.op == r.op) {
                    s.ctrl = None;
                }
                s.ctrl_last = Some(ApplyStatus {
                    pending: 0,
                    last_op: r.op,
                    last_result: result,
                    last_frame: r.frame,
                });
            }
        });
        self.tb_op_done(tb, r, result);
    }

    /// `EXT_ALINK_SPDIF_CTRL` OUT on a source (2.10.3).
    fn spdif_ctrl(&mut self, data: &[u8]) -> bool {
        let Some(r) = ApplyRequest::decode(data) else {
            return self.refuse(lm::VC_ERR_LENGTH);
        };
        if data.len() != tbc::ALINK_SPDIF_CTRL_LEN
            || data.get(12) != Some(&tbc::TB_SPDIF)
            || data.get(13..) != Some(&[0; 3])
        {
            return self.refuse(lm::VC_ERR_VALUE);
        }
        let carried = matches!(
            r.op,
            ApplyOp::Mute
                | ApplyOp::Unmute
                | ApplyOp::MasterVolume
                | ApplyOp::Click
                | ApplyOp::SiggenStart
                | ApplyOp::SiggenStop
        );
        if !carried || !r.args_valid() {
            return self.refuse(lm::VC_ERR_VALUE);
        }
        let source = self.spdif_state() == Some((tbc::TB_STATE_LOCKED, tbc::TB_ROLE_SOURCE));
        if !source {
            return self.refuse(lm::VC_ERR_STATE);
        }
        let now = self.spdif_t().unwrap_or(0);
        let pending = self
            .tb_unit(|u| {
                let s = u.spdif.as_mut()?;
                if s.ctrl.is_some_and(|c| c.frame <= now) {
                    s.ctrl = None;
                }
                Some(s.ctrl.is_some())
            })
            .flatten()
            .unwrap_or(true);
        let lead_ok = r.frame >= now.saturating_add(tbc::SPDIF_CTRL_MIN_LEAD_FRAMES);
        if pending || !lead_ok {
            return self.refuse(lm::VC_ERR_STATE);
        }
        self.tb_unit(|u| {
            if let Some(s) = u.spdif.as_mut() {
                s.ctrl = Some(r);
            }
        });
        self.tb_broadcast(r);
        true
    }

    fn spdif_ctrl_in(&mut self) -> Vec<u8> {
        let now = self.spdif_t().unwrap_or(0);
        let (pending, last, source) = self
            .tb_unit(|u| match u.spdif.as_mut() {
                Some(s) if s.mode == tbc::SPDIF_MODE_SOURCE => {
                    if s.ctrl.is_some_and(|c| c.frame <= now) {
                        s.ctrl = None;
                    }
                    (usize::from(s.ctrl.is_some()), s.ctrl_last, true)
                }
                _ => (0, None, false),
            })
            .unwrap_or((0, None, false));
        if !source {
            return vec![0; tbc::ALINK_SPDIF_CTRL_LEN];
        }
        apply_status_in(pending, last)
    }

    // -------------------------------------------------- the S/PDIF block

    fn spdif_config_in(&mut self) -> Vec<u8> {
        let state = self.spdif_state().map_or(tbc::TB_STATE_NONE, |s| s.0);
        self.tb_unit(|u| {
            let s = u.spdif.as_ref()?;
            let [d0, d1] = s.d.to_le_bytes();
            Some(vec![
                s.mode,
                s.flags,
                d0,
                d1,
                s.on_lock_error,
                0,
                s.tx_mask,
                state,
            ])
        })
        .flatten()
        .unwrap_or_else(|| vec![0; tbc::ALINK_SPDIF_LEN])
    }

    /// `EXT_ALINK_SPDIF` OUT (2.10.1).
    fn spdif_config_out(&mut self, data: &[u8]) -> bool {
        let [
            mode,
            flags,
            d0,
            d1,
            on_lock_error,
            reserved,
            tx_mask,
            _state,
        ] = *data
        else {
            return self.refuse(lm::VC_ERR_LENGTH);
        };
        let d = u16::from_le_bytes([d0, d1]);
        let bad = mode > tbc::SPDIF_MODE_SOURCE
            || flags & !1 != 0
            || reserved != 0
            || on_lock_error > 1
            || (mode == tbc::SPDIF_MODE_FOLLOWER && !(SPDIF_D_MIN..=SPDIF_D_MAX).contains(&d))
            || (mode == tbc::SPDIF_MODE_SOURCE && tx_mask & !SPDIF_SLOT_MASK != 0);
        if bad {
            return self.refuse(lm::VC_ERR_VALUE);
        }
        let epoch = self.tb_domain(|d| d.epoch).unwrap_or(0);
        let now = Instant::now();
        let Some((old_mode, old_d)) = self
            .tb_unit(|u| u.spdif.as_ref().map(|s| (s.mode, s.d)))
            .flatten()
        else {
            return self.refuse(lm::VC_ERR_STATE);
        };
        let reanchor = (old_mode != mode || old_d != d) && mode != tbc::SPDIF_MODE_OFF;
        self.tb_unit(|u| {
            if let Some(s) = u.spdif.as_mut() {
                s.mode = mode;
                s.flags = flags;
                s.d = d;
                s.on_lock_error = on_lock_error;
                s.tx_mask = if mode == tbc::SPDIF_MODE_SOURCE {
                    tx_mask
                } else {
                    0
                };
                s.seen_epoch = epoch;
                if reanchor {
                    s.holdover = false;
                    s.lock_error = false;
                    if old_mode != tbc::SPDIF_MODE_OFF {
                        s.reanchors = s.reanchors.wrapping_add(1);
                    }
                    s.anchoring_until = now.checked_add(SPDIF_ANCHOR);
                }
            }
        });
        if mode == tbc::SPDIF_MODE_OFF && old_mode != tbc::SPDIF_MODE_OFF {
            let all = self
                .tb_unit(|u| u.spdif.as_mut().map(|s| std::mem::take(&mut s.queue)))
                .flatten()
                .unwrap_or_default();
            for q in all {
                if q.op == ApplyOp::SiggenStart {
                    self.siggen_take_pending(tbc::timeline_result::CANCELLED_LOSS);
                }
                self.tb_op_done(tbc::TB_SPDIF, q, ApplyResult::Cancelled);
            }
            self.tb_emit(tbc::TB_SPDIF, tbc::TB_STATE_NONE, reason::DISABLED);
        } else if reanchor {
            let why = if old_mode == tbc::SPDIF_MODE_OFF {
                reason::NONE
            } else {
                reason::REANCHOR
            };
            self.tb_emit(tbc::TB_SPDIF, tbc::TB_STATE_ANCHORING, why);
        }
        true
    }

    /// `EXT_ALINK_SPDIF_STATUS` (2.10.2).
    fn spdif_status(&mut self) -> Vec<u8> {
        let st = self.spdif_state();
        let state = st.map_or(tbc::TB_STATE_NONE, |s| s.0);
        let source = st.is_some_and(|s| s.1 == tbc::TB_ROLE_SOURCE);
        let has_t = matches!(state, tbc::TB_STATE_LOCKED | tbc::TB_STATE_HOLDOVER);
        let f = if has_t {
            self.spdif_t().unwrap_or(0)
        } else {
            0
        };
        let (epoch, source_id) = self
            .tb_domain(|d| match d.kind {
                TimeBaseKind::Spdif { source_id } => (d.epoch, source_id),
                _ => (0, 0),
            })
            .unwrap_or((0, 0));
        let mut b = vec![0u8; tbc::ALINK_SPDIF_STATUS_LEN];
        self.tb_unit(|u| {
            let Some(s) = u.spdif.as_ref() else {
                return;
            };
            let follower = s.mode == tbc::SPDIF_MODE_FOLLOWER;
            let phase_state = match (follower, state) {
                (true, tbc::TB_STATE_LOCKED) => 2,
                (true, tbc::TB_STATE_ANCHORING) => 1,
                _ => 0,
            };
            let mut flags = 0u8;
            if matches!(state, tbc::TB_STATE_LOCKED | tbc::TB_STATE_HOLDOVER) {
                flags |= 0b11; // SUPERFRAME, TIME_BASE
            }
            if state == tbc::TB_STATE_HOLDOVER {
                flags |= 1 << 2;
                flags &= !1;
            }
            if s.lock_error {
                flags |= 1 << 3 | 1 << 5;
            }
            if source {
                flags |= 1 << 4;
            }
            let (d_min, d_max) = if follower {
                (SPDIF_D_MIN, SPDIF_D_MAX)
            } else {
                (0, 0)
            };
            let mut put = |at: usize, bytes: &[u8]| {
                if let Some(x) = b.get_mut(at..at.saturating_add(bytes.len())) {
                    x.copy_from_slice(bytes);
                }
            };
            put(0, &[state, phase_state, 1, flags]);
            if s.mode != tbc::SPDIF_MODE_OFF {
                put(4, &source_id.to_le_bytes());
                put(6, &[epoch]);
            }
            put(8, &f.to_le_bytes());
            let pe: f32 = if follower && state == tbc::TB_STATE_LOCKED {
                0.03
            } else {
                0.0
            };
            put(16, &pe.to_le_bytes());
            put(20, &(if follower { 1.5f32 } else { 0.0 }).to_le_bytes());
            put(24, &d_min.to_le_bytes());
            put(26, &d_max.to_le_bytes());
            let fill = if follower && state == tbc::TB_STATE_LOCKED {
                s.d / 2
            } else {
                0
            };
            put(28, &fill.to_le_bytes());
            put(32, &s.lock_count.to_le_bytes());
            put(36, &s.loss_count.to_le_bytes());
            put(40, &(if follower { s.sf_errors } else { 0 }).to_le_bytes());
            put(44, &(if follower { s.reanchors } else { 0 }).to_le_bytes());
        });
        b
    }

    // -------------------------------------------------- dispatch (both envelopes)

    /// The ids a fake with a time base adds to `EXT_LIST`.
    pub(super) fn tb_ids(&self) -> Vec<u16> {
        if !self.tb_on() {
            return Vec::new();
        }
        let mut ids = tbc::TIME_BASE_IDS.to_vec();
        if self.spdif_present() {
            ids.push(EXT_APPLY_AT_FRAME);
            ids.extend(tbc::ALINK_SPDIF_IDS);
        }
        ids
    }

    /// IN: `Some(reply)` (`Some(None)` stalls) for the time-base ids, and for
    /// `EXT_APPLY_AT_FRAME` on an S/PDIF unit; `None` for anything else.
    pub(super) fn get_timebase(&mut self, value: u16) -> Option<Option<Vec<u8>>> {
        if !self.tb_on() {
            return None;
        }
        let spdif = self.spdif_present();
        let ours = tbc::TIME_BASE_IDS.contains(&value)
            || (spdif && (tbc::ALINK_SPDIF_IDS.contains(&value) || value == EXT_APPLY_AT_FRAME));
        if !ours {
            return None;
        }
        self.tb_service();
        Some(Some(match value {
            tbc::EXT_TIME_BASE => self.time_base_reply(),
            tbc::EXT_SIGGEN_TIMELINE => self.siggen_timeline_reply(),
            tbc::EXT_ALINK_SPDIF => self.spdif_config_in(),
            tbc::EXT_ALINK_SPDIF_STATUS => self.spdif_status(),
            tbc::EXT_ALINK_SPDIF_CTRL => self.spdif_ctrl_in(),
            // EXT_APPLY_AT_FRAME: an S/PDIF unit's first entry is its own.
            _ => self.spdif_queue_in(),
        }))
    }

    /// OUT: `Some(accepted)` for the time-base ids, an S/PDIF unit's ids and
    /// `EXT_APPLY_AT_FRAME` with time base 3; `None` for anything else.
    pub(super) fn set_timebase(&mut self, value: u16, data: &[u8]) -> Option<bool> {
        if !self.tb_on() {
            return None;
        }
        let spdif = self.spdif_present();
        let apply3 = value == EXT_APPLY_AT_FRAME && data.get(12) == Some(&tbc::TB_SPDIF);
        let ours = tbc::TIME_BASE_IDS.contains(&value)
            || (spdif && (tbc::ALINK_SPDIF_IDS.contains(&value) || value == EXT_APPLY_AT_FRAME))
            || apply3;
        if !ours {
            return None;
        }
        self.tb_unit(|u| u.refusal = None);
        self.tb_service();
        Some(match value {
            tbc::EXT_TIME_BASE | tbc::EXT_SIGGEN_TIMELINE | tbc::EXT_ALINK_SPDIF_STATUS => {
                self.refuse(lm::VC_ERR_DIRECTION)
            }
            tbc::EXT_ALINK_SPDIF => self.spdif_config_out(data),
            tbc::EXT_ALINK_SPDIF_CTRL => self.spdif_ctrl(data),
            // EXT_APPLY_AT_FRAME
            _ if apply3 => self.spdif_apply(data),
            // Another time base on an S/PDIF-only unit.
            _ => self.refuse(lm::VC_ERR_STATE),
        })
    }

    /// A preset load (or factory reset): an S/PDIF source's outputs restart.
    pub(super) fn tb_on_preset(&mut self) {
        let source = self
            .spdif_state()
            .is_some_and(|s| s.1 == tbc::TB_ROLE_SOURCE);
        if source {
            self.tb_domain(Domain::new_epoch);
            self.tb_service();
        }
    }

    // -------------------------------------------------- faults

    fn tb_fault(&mut self, f: TimeBaseFault) -> bool {
        let Some(kind) = self.tb_kind() else {
            return false;
        };
        self.tb_service();
        let tb = kind.time_base();
        match (f, kind) {
            (TimeBaseFault::Epoch, TimeBaseKind::Spdif { .. }) => {
                self.tb_domain(Domain::new_epoch);
                self.tb_service();
                return true;
            }
            (TimeBaseFault::Epoch, _) => return false,
            (TimeBaseFault::Correction { samples, event }, _) => {
                let n = i64::from(samples);
                if tb == tbc::TB_USB_SAMPLE {
                    self.alink_usb_correct(n);
                } else {
                    self.tb_unit(|u| u.adj = u.adj.saturating_add(n));
                }
                // An op 9 whose T was skipped is late.
                if let Some(p) = self.siggen_pending()
                    && p.tb == tb
                    && self.tb_t_now(tb).is_some_and(|now| now >= p.t)
                {
                    self.siggen_take_pending(tbc::timeline_result::REFUSED_LATE);
                    let q = if p.broadcast {
                        Queue::Broadcast
                    } else {
                        Queue::Own(tb)
                    };
                    if let Some(r) = self.queue_take_start(q, tb, p.t) {
                        self.tb_op_done(tb, r, ApplyResult::Refused);
                    }
                }
                if event {
                    self.tb_note(tb, reason::CORRECTION);
                }
                return true;
            }
            _ => {}
        }
        match kind {
            TimeBaseKind::Line { .. } => self.line_fault(f),
            TimeBaseKind::UsbDirect => self.usb_fault(f),
            TimeBaseKind::Spdif { .. } => self.spdif_fault(f),
        }
    }

    fn set_force(&mut self, v: Option<u8>) {
        self.tb_unit(|u| u.force = v);
    }

    fn line_fault(&mut self, f: TimeBaseFault) -> bool {
        match f {
            TimeBaseFault::Holdover => self.alink_sim_op(super::ALINK_SIM_LOSE),
            TimeBaseFault::Resume => self.alink_sim_op(super::ALINK_SIM_LOCK),
            TimeBaseFault::Reanchor => self.alink_sim_op(super::ALINK_SIM_REANCHOR),
            TimeBaseFault::LockError(on) => {
                self.set_force(on.then_some(tbc::TB_STATE_LOCK_ERROR));
                let why = if on {
                    reason::LOCK_ERROR
                } else {
                    reason::LOCK_OK
                };
                self.tb_note(tbc::TB_ALINK_LINE, why);
                true
            }
            _ => false,
        }
    }

    fn usb_fault(&mut self, f: TimeBaseFault) -> bool {
        let tb = tbc::TB_USB_SAMPLE;
        match f {
            TimeBaseFault::Holdover => {
                self.set_force(Some(tbc::TB_STATE_HOLDOVER));
                self.tb_note(tb, reason::HOLDOVER);
            }
            TimeBaseFault::Resume => {
                self.set_force(None);
                self.tb_note(tb, reason::RESUMED);
            }
            TimeBaseFault::Reanchor => {
                self.alink_usb_count_reanchor();
                self.tb_emit(tb, tbc::TB_STATE_ANCHORING, reason::REANCHOR);
                self.tb_emit(tb, tbc::TB_STATE_LOCKED, reason::LOCKED);
            }
            TimeBaseFault::LockError(on) => {
                self.set_force(on.then_some(tbc::TB_STATE_LOCK_ERROR));
                let why = if on {
                    reason::LOCK_ERROR
                } else {
                    reason::LOCK_OK
                };
                self.tb_note(tb, why);
            }
            _ => return false,
        }
        true
    }

    fn spdif_fault(&mut self, f: TimeBaseFault) -> bool {
        let tb = tbc::TB_SPDIF;
        let follower = self
            .spdif_state()
            .is_some_and(|s| s.1 == tbc::TB_ROLE_FOLLOWER);
        if !follower && !matches!(f, TimeBaseFault::Reanchor) {
            return false;
        }
        let set = |s: &mut Spdif| match f {
            TimeBaseFault::Holdover => {
                if !s.holdover {
                    s.loss_count = s.loss_count.wrapping_add(1);
                    s.sf_errors = s.sf_errors.wrapping_add(tbc::SPDIF_COAST_BLOCKS);
                }
                s.holdover = true;
                reason::HOLDOVER
            }
            TimeBaseFault::Resume => {
                s.holdover = false;
                reason::RESUMED
            }
            TimeBaseFault::LockError(on) => {
                if on && !s.lock_error {
                    s.loss_count = s.loss_count.wrapping_add(1);
                }
                s.lock_error = on;
                if on {
                    reason::LOCK_ERROR
                } else {
                    reason::LOCK_OK
                }
            }
            _ => {
                s.reanchors = s.reanchors.wrapping_add(1);
                reason::REANCHOR
            }
        };
        let Some(why) = self.tb_unit(|u| u.spdif.as_mut().map(set)).flatten() else {
            return false;
        };
        if why == reason::REANCHOR {
            self.tb_emit(tb, tbc::TB_STATE_ANCHORING, reason::REANCHOR);
            self.tb_emit(tb, tbc::TB_STATE_LOCKED, reason::LOCKED);
        } else {
            self.tb_note(tb, why);
        }
        true
    }
}

/// `EXT_APPLY_AT_FRAME` IN's layout (2.8.7): pending, last op, result,
/// reserved, last frame, reserved; all zero before the first completion.
fn apply_status_in(pending: usize, last: Option<ApplyStatus>) -> Vec<u8> {
    let pending = u8::try_from(pending).unwrap_or(u8::MAX);
    let mut b = vec![0u8; 16];
    if let Some(l) = last {
        let mut put = |at: usize, bytes: &[u8]| {
            if let Some(x) = b.get_mut(at..at.saturating_add(bytes.len())) {
                x.copy_from_slice(bytes);
            }
        };
        put(1, &[l.last_op.raw(), l.last_result.raw()]);
        put(4, &l.last_frame.to_le_bytes());
    }
    if let Some(p) = b.first_mut() {
        *p = pending;
    }
    b
}

// ------------------------------------------------------------ construction

impl FakeDevice {
    /// The demo device ([`FakeDevice::demo`]) as a unit of `tb`: firmware with
    /// the time-base contract and ops 9 and 10 (module docs), on the classic
    /// 1.1.6 fake with the extension channel.
    pub fn demo_on_time_base(tb: &FakeTimeBase, unit: FakeTimeBaseUnit) -> Self {
        Self::make_on_time_base(tb, unit, None)
    }

    /// [`FakeDevice::demo_on_time_base`] as firmware of `generation` (G1b
    /// `Bofr116`, G2b `Bofr117` with the tunnel).
    pub fn demo_on_time_base_as(
        tb: &FakeTimeBase,
        unit: FakeTimeBaseUnit,
        generation: FakeGeneration,
    ) -> Self {
        Self::make_on_time_base(tb, unit, Some(generation))
    }

    fn make_on_time_base(
        tb: &FakeTimeBase,
        unit: FakeTimeBaseUnit,
        generation: Option<FakeGeneration>,
    ) -> Self {
        let handle = tb.join();
        let options = FakeOptions {
            extensions: true,
            surfaces: true,
            analysis: true,
            alink: matches!(unit, FakeTimeBaseUnit::Line(_)),
            generation,
            time_base: Some(handle),
            ..FakeOptions::default()
        };
        let dev = Self::new(Capture::demo(), options);
        {
            let mut s = dev.shared.lock();
            if DEMO_STATUS.len() == s.status_len() {
                s.status = DEMO_STATUS.to_vec();
            }
            let epoch = s.tb_domain(|d| d.epoch).unwrap_or(0);
            match unit {
                FakeTimeBaseUnit::Line(mut setup) => {
                    if let TimeBaseKind::Line { sender_id } = tb.kind() {
                        setup.sender_id = sender_id;
                    }
                    s.alink.setup = setup;
                }
                FakeTimeBaseUnit::UsbDirect(setup) => {
                    s.alink_usb.enable(setup);
                    let origin = s.tb_usb_origin();
                    s.alink_usb_set_clock(origin);
                }
                FakeTimeBaseUnit::Spdif(setup) => {
                    s.tb_unit(|u| {
                        u.spdif = Some(Spdif {
                            mode: setup.mode.min(tbc::SPDIF_MODE_SOURCE),
                            flags: 1,
                            d: setup.d,
                            on_lock_error: setup.on_lock_error.min(1),
                            tx_mask: setup.tx_mask & SPDIF_SLOT_MASK,
                            seen_epoch: epoch,
                            lock_count: u32::from(setup.mode != tbc::SPDIF_MODE_OFF),
                            ..Spdif::default()
                        });
                    });
                }
            }
            s.tb_restart_outputs();
            s.tb_sync_quiet();
        }
        dev
    }
}

// ------------------------------------------------------------ the test side

impl FakeRemote {
    /// Play `fault` on this unit's time base (module docs). False on a fake
    /// without one, or a fault its transport does not have (an epoch change
    /// off S/PDIF; a holdover, resume or lock error on an S/PDIF source).
    pub fn time_base_fault(&self, fault: TimeBaseFault) -> bool {
        let ok = self.shared.lock().tb_fault(fault);
        self.shared.wake.notify_all();
        ok
    }

    /// Set the `arm_lead_ms` this unit reports and enforces (150 by default,
    /// the K340's; a board without a DAC mute has 50).
    pub fn time_base_set_arm_lead_ms(&self, ms: u16) {
        self.shared.lock().tb_unit(|u| u.arm_lead_ms = ms);
    }

    /// The unit's time base now: `(time_base, T)` of its first entry, as
    /// `EXT_TIME_BASE` would report it.
    pub fn time_base_t(&self) -> Option<(u8, u64)> {
        let mut s = self.shared.lock();
        s.tb_service();
        let e = s.tb_entries().into_iter().next()?;
        drop(s);
        self.shared.wake.notify_all();
        Some((e.tb, e.t))
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests_wp_atfake;
