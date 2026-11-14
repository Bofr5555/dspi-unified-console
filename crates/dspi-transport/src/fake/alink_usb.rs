//! WP AUSB1: an alink USB-direct unit (the fork's `extension_channel_spec.md`
//! 2.9), answered when [`super::FakeDevice::demo_usb_direct`] made one, so the
//! clients and the hub can show and align USB-direct units without a K340 on
//! bofr-main ebdf538.
//!
//! The model: a unit on its USB input, 48 kHz, its host [`UsbDirectSetup::host_ppm`]
//! off its crystal. Every fake on one process sees the same host: the bus
//! frame N runs from one process-wide start ([`bus_epoch`]), 1 per ms, so two
//! fakes report the same bus frame at the same instant, while each counts N
//! from its own origin (a multiple of 2048, as units that enumerated at
//! different times). S = 48 N + k.
//!
//! - **Modes.** `EXT_ALINK_USB` as 2.9.2 refuses it: a mode above 2, an
//!   undefined flag, D 0, a nonzero reserved byte, mode 2 with D outside
//!   170..399. From mode 0 a unit pulls in ([`PULL_IN`]) and, in mode 2,
//!   anchors ([`ANCHOR`]) before it is phase-locked; a new D re-anchors (event
//!   0xE3 `REANCHOR`). Mode 0 parks it (`DISABLED`) and cancels its ops.
//! - **The stream.** A stream plays from the fake's start: its content began
//!   [`UsbDirectSetup::stream_lag_frames`] bus frames after the host's first
//!   one, with `stream_shift` [`UsbDirectSetup::stream_shift`], so the hub has
//!   start offsets to equalise (2.9.3, U6). An anchor places the stream anew
//!   (the first packet after it), as the firmware reports it.
//! - **Time base.** `EXT_FRAME_COUNTER` answers 20 bytes, time base 1 (S and
//!   the bus frame) while the mode is not 0, else as the alink's (or `0xFF`).
//!   `EXT_APPLY_AT_FRAME` takes time base 1 (S) and 2 (bus frame + sample, the
//!   next occurrence; one within the last 64 frames is late) while phase-locked,
//!   queues four, runs them when S passes, sends 0xE2 with the bus form, and its
//!   IN reads the USB queue. CLICK and SYNC_PULSE end `refused` (a demo is no
//!   bench build; the click is modelled on neither chip here).
//! - **[`EXT_FAKE_ALINK_USB_SIM`]** plays the bus's side: a holdover that resumes,
//!   a lost time base (anchors again), a faded gap, the host out of range and
//!   back, a stream restart, settle (finish a pull-in or an anchor now).
//!
//! - **Lock error** (-- WP AUSB2 --, the fork's WP USB3b): a mode-2 unit whose
//!   host is out of range is in lock error: STATUS flags bit 6, 0xE3
//!   `LOCK_ERROR` on entry and `LOCK_OK` on exit, DIAG `lock_errors`. Its action
//!   is `EXT_ALINK_USB` byte 4 (0 mute, 1 play unaligned), refused above 1, and
//!   refused at 1 by a unit made with [`UsbDirectSetup::usb3b`] off (firmware
//!   before WP USB3b, which also answers DIAG layout version 1).
//!
//! Not modelled: the 250 ms rate limit on 0xE3, the servo's dynamics (the
//! phase error is a small slow function of the clock), 44.1 kHz (the fake runs
//! 48 kHz), the input source (the fake is always on USB).

use std::sync::OnceLock;
use std::time::Duration;

use dspi_proto::ext::alink::{
    self, ApplyOp, ApplyRequest, ApplyResult, ApplyResultEvent, ApplyStatus, FrameCounter,
};
use dspi_proto::ext::alink_usb::{
    self as usb, AlinkUsbConfig, AlinkUsbDiag, AlinkUsbEvent, AlinkUsbStatus, FrameCounterTb,
    TimeBase, UsbEventReason, UsbState,
};
use dspi_proto::generated::notify as nfy;

use super::State;
use crate::clock::Instant;

/// `REQ_EXTENSION` OUT sub-command only a [`super::FakeDevice`] answers, as
/// [`super::EXT_FAKE_VREG_SIM`] (WP AUSB1): one byte, what happens on the bus
/// ([`USB_SIM_HOLDOVER`] .. [`USB_SIM_SETTLE`]). Anything else, or a fake that is
/// not a USB-direct unit, stalls.
pub const EXT_FAKE_ALINK_USB_SIM: u16 = 0xFFF8;
/// SOF stops for 300 ms and comes back on the same timeline: `HOLDOVER`, then
/// `RESUMED` (counted in DIAG `holdovers` and `rides`).
pub const USB_SIM_HOLDOVER: u8 = 0;
/// The time base is lost (a bus reset): `SOF_LOST`, then the unit anchors again.
pub const USB_SIM_SOF_LOST: u8 = 1;
/// Two packets go missing: a faded gap (`GAP`, `gap_count`, `missed_samples`).
pub const USB_SIM_GAP: u8 = 2;
/// The host goes beyond 295 ppm (state 6, `HOST_RANGE`); again: back in range.
pub const USB_SIM_HOST_RANGE: u8 = 3;
/// The host restarts the stream, `lag` frames later than before is not modelled:
/// the content starts at the next bus frame plus the setup's lag.
pub const USB_SIM_STREAM_RESTART: u8 = 4;
/// Finish a pull-in or an anchor now.
pub const USB_SIM_SETTLE: u8 = 5;
// -- fixup ausb2 (review ausb2 F2) --
/// The host's stream changes to 96 kHz (again: back to 48 kHz). D's limits
/// become 324..351, so a mode-2 unit at D 192 cannot phase-lock: it runs
/// rate-locked with `D_BELOW_MIN` (lock error, cause CONFIG, `0xE3` `CONFIG`
/// and `LOCK_ERROR`), and refuses a mode-2 write with D outside them. Only the
/// limits, the rate code and the state change: samples are still counted at
/// 48 per frame.
pub const USB_SIM_RATE_96K: u8 = 6;

/// Samples per bus frame (48 kHz).
const SPF: u16 = 48;
/// How long a pull-in takes in the fake (the firmware: 3-12 s).
pub const PULL_IN: Duration = Duration::from_millis(1500);
/// How long an anchor takes (about D + 20 ms).
pub const ANCHOR: Duration = Duration::from_millis(30);
/// How many bus frames before the host's first SOF here its streams began.
const STREAM_BEFORE: u64 = 512;
/// The USB apply queue's depth (2.8.7).
const APPLY_QUEUE: usize = 4;

/// The host every fake of this process is on: its first SOF.
fn bus_epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// Bus frames since [`bus_epoch`], and the sample within the frame.
fn bus_now() -> (u64, u16) {
    let us = Instant::now()
        .saturating_duration_since(bus_epoch())
        .as_micros();
    let frames = u64::try_from(us.checked_div(1000).unwrap_or(0)).unwrap_or(u64::MAX);
    let k = us
        .checked_rem(1000)
        .unwrap_or(0)
        .saturating_mul(u128::from(SPF))
        .checked_div(1000)
        .unwrap_or(0);
    (frames, u16::try_from(k).unwrap_or(0))
}

/// A USB-direct unit's place on its host ([`super::FakeDevice::demo_usb_direct`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UsbDirectSetup {
    /// The mode at start (2 for a phase-locked demo unit).
    pub mode: u8,
    /// D at start, samples.
    pub d: u16,
    /// The host's SOF rate against the unit's crystal, ppm.
    pub host_ppm: f32,
    /// This unit's frame count started this many 2048-frame wraps before the
    /// host's epoch (units enumerated at different times).
    pub origin_wraps: u32,
    /// Bus frames between the host's first stream and this unit's (the host
    /// opens the streams one after the other).
    pub stream_lag_frames: u16,
    /// `stream_shift` the unit reports.
    pub stream_shift: i32,
    /// RP2350 SOF timestamps (STATUS flags bit 0).
    pub hw_timestamp: bool,
    // -- WP AUSB2 --
    /// Firmware since the fork's WP USB3b: `on_lock_error` (byte 4) taken,
    /// lock error reported, DIAG layout version 2. Off: byte 4 must be 0 and
    /// DIAG is version 1.
    pub usb3b: bool,
}

impl Default for UsbDirectSetup {
    fn default() -> Self {
        Self {
            mode: usb::MODE_PHASE_LOCK,
            d: usb::DEFAULT_D,
            host_ppm: -11.5,
            origin_wraps: 3,
            stream_lag_frames: 0,
            stream_shift: 0,
            hw_timestamp: true,
            usb3b: true,
        }
    }
}

/// The unit's USB-direct state.
#[derive(Debug, Default)]
pub(super) struct AlinkUsb {
    /// Made a USB-direct unit (`EXT_LIST` names the block).
    pub(super) on: bool,
    setup: Option<UsbDirectSetup>,
    config: Option<AlinkUsbConfig>,
    /// When the current pull-in or anchor ends.
    settling: Option<(UsbState, Instant)>,
    /// The host is out of range.
    rejected: bool,
    // -- fixup ausb2 --
    /// The stream runs at 96 kHz ([`USB_SIM_RATE_96K`]).
    rate_96k: bool,
    /// No time base until then (a lost one comes back after a pull-in).
    lost_until: Option<Instant>,
    /// The bus frame (own count N) the current stream's content began in.
    stream_start: Option<u64>,
    gap_count: u16,
    missed: u32,
    anchors: u32,
    reanchors: u32,
    diag: Option<AlinkUsbDiag>,
    queue: Vec<ApplyRequest>,
    last: Option<ApplyStatus>,
}

fn packet(id: u8, payload: &[u8]) -> Vec<u8> {
    let mut p = vec![nfy::NOTIFY_V2_VERSION as u8, id, 0, 0];
    p.extend_from_slice(payload);
    p
}

/// The bus frame of own count `n`.
fn bus_of(n: u64) -> u16 {
    u16::try_from(n.checked_rem(u64::from(usb::BUS_FRAMES)).unwrap_or(0)).unwrap_or(0)
}

fn low32(v: u64) -> u32 {
    (v & u64::from(u32::MAX)) as u32
}

impl AlinkUsb {
    /// Make the fake a USB-direct unit.
    pub(super) fn enable(&mut self, setup: UsbDirectSetup) {
        self.on = true;
        self.setup = Some(setup);
        self.config = Some(AlinkUsbConfig {
            mode: setup.mode,
            ..AlinkUsbConfig::DEFAULT
        });
        if let Some(c) = self.config.as_mut() {
            c.d = setup.d;
        }
        self.anchors = u32::from(setup.mode == usb::MODE_PHASE_LOCK);
        self.diag = Some(self.diag_clear());
        // The host's streams began well before now (`STREAM_BEFORE` frames
        // before its first SOF here), this one `stream_lag_frames` after the
        // first, so a first read already sees them placed.
        bus_epoch();
        if setup.mode == usb::MODE_PHASE_LOCK {
            self.stream_start = Some(
                self.origin()
                    .saturating_sub(STREAM_BEFORE)
                    .saturating_add(u64::from(setup.stream_lag_frames)),
            );
        }
    }

    fn setup(&self) -> UsbDirectSetup {
        self.setup.unwrap_or_default()
    }

    // -- WP AUSB2 --
    /// DIAG cleared, in the firmware's layout.
    fn diag_clear(&self) -> AlinkUsbDiag {
        AlinkUsbDiag {
            version: if self.setup().usb3b {
                usb::DIAG_VERSION
            } else {
                usb::DIAG_VERSION_1
            },
            ..AlinkUsbDiag::CLEAR
        }
    }

    // -- WP AUSB2 --
    /// In lock error now: mode 2 with the host out of range -- fixup ausb2 --
    /// or a D outside the limits at this rate, on firmware that reports it.
    fn lock_error(&self) -> bool {
        self.setup().usb3b
            && self.config().mode == usb::MODE_PHASE_LOCK
            && (self.rejected || self.config_bad())
    }

    // -- fixup ausb2 --
    /// `d_min`, `d_max` at the stream's rate.
    fn limits(&self) -> (u16, u16) {
        usb::d_limits(if self.rate_96k { 96 } else { SPF })
    }

    // -- fixup ausb2 --
    /// Mode 2 with D outside the limits now: it runs rate-locked.
    fn config_bad(&self) -> bool {
        let c = self.config();
        let (min, max) = self.limits();
        c.mode == usb::MODE_PHASE_LOCK && !(min..=max).contains(&c.d)
    }

    fn config(&self) -> AlinkUsbConfig {
        self.config.unwrap_or(AlinkUsbConfig::DEFAULT)
    }

    /// The host's first SOF on this unit's own count.
    fn origin(&self) -> u64 {
        u64::from(self.setup().origin_wraps).saturating_mul(u64::from(usb::BUS_FRAMES))
    }

    /// N now, on this unit's own count.
    fn frames(&self) -> (u64, u16) {
        let (n, k) = bus_now();
        (n.saturating_add(self.origin()), k)
    }

    /// S now.
    fn sample(&self) -> u64 {
        let (n, k) = self.frames();
        n.saturating_mul(u64::from(SPF))
            .saturating_add(u64::from(k))
    }

    /// The state now, with the pull-in or anchor finished if it is due.
    fn state(&mut self) -> UsbState {
        let now = Instant::now();
        if self.lost_until.is_some_and(|t| now >= t) {
            self.lost_until = None;
        }
        if let Some((_, until)) = self.settling
            && now >= until
        {
            self.settle_step();
        }
        self.state_now()
    }

    fn state_now(&self) -> UsbState {
        let c = self.config();
        if c.mode == usb::MODE_OFF {
            return UsbState::Off;
        }
        if self.rejected {
            return UsbState::Rejected;
        }
        if self.lost_until.is_some() {
            return UsbState::NoSof;
        }
        if let Some((s, _)) = self.settling {
            return s;
        }
        // -- fixup ausb2 -- D outside the limits at this rate: rate lock only
        if c.mode == usb::MODE_RATE_LOCK || self.config_bad() {
            UsbState::RateLocked
        } else {
            UsbState::PhaseLocked
        }
    }

    /// A pull-in ends (an anchor follows in mode 2), or an anchor does.
    fn settle_step(&mut self) {
        match self.settling.take() {
            Some((UsbState::PullIn, _)) if self.config().mode == usb::MODE_PHASE_LOCK => {
                self.settling = Instant::now()
                    .checked_add(ANCHOR)
                    .map(|t| (UsbState::Anchoring, t));
            }
            Some((UsbState::Anchoring, _)) => {
                self.anchors = self.anchors.saturating_add(1);
                self.place_stream();
            }
            _ => {}
        }
    }

    /// The stream as placed on a new timeline: its next packet.
    fn place_stream(&mut self) {
        let (n, _) = self.frames();
        self.stream_start = Some(n.saturating_add(1));
    }

    fn start(&mut self, what: UsbState, d: Duration) {
        self.settling = Instant::now().checked_add(d).map(|t| (what, t));
    }

    /// `EXT_ALINK_USB_STATUS` now.
    fn status(&mut self) -> AlinkUsbStatus {
        let state = self.state();
        let setup = self.setup();
        let (d_min, d_max) = self.limits();
        let (n, _) = self.frames();
        let time_base = !matches!(state, UsbState::Off | UsbState::NoSof);
        let locked = state == UsbState::PhaseLocked;
        let since = Instant::now()
            .saturating_duration_since(bus_epoch())
            .as_secs_f32();
        let holdover = self.diag.is_some_and(|d| d.holdover_us > 0);
        // -- WP FIXM1 -- a stream placed at the next frame is not there yet:
        // reported from its first frame on, so its start never reads one early.
        let stream = locked
            .then_some(self.stream_start)
            .flatten()
            .filter(|s| *s <= n);
        let age = stream.map_or(0, |s| n.saturating_sub(s));
        let age32 = u32::try_from(age).unwrap_or(u32::MAX);
        let mut flags = usb::USB_FLAG_STREAM;
        if setup.hw_timestamp {
            flags |= usb::USB_FLAG_HW_TIMESTAMP;
        }
        if time_base {
            flags |= usb::USB_FLAG_TIME_BASE;
        }
        if matches!(state, UsbState::RateLocked | UsbState::PhaseLocked) {
            flags |= usb::USB_FLAG_NOMINAL_FEEDBACK;
        }
        if holdover {
            flags |= usb::USB_FLAG_HOLDOVER;
        }
        // -- WP AUSB2 --
        if self.lock_error() {
            flags |= usb::USB_FLAG_LOCK_ERROR;
        }
        let measured = !matches!(state, UsbState::Off | UsbState::NoSof);
        AlinkUsbStatus {
            state,
            phase_state: match state {
                UsbState::PhaseLocked => 2,
                UsbState::Anchoring => 1,
                _ => 0,
            },
            rate_code: if self.rate_96k { 2 } else { 1 },
            flags,
            host_ppm: if measured { setup.host_ppm } else { 0.0 },
            vcxo_ppm: match state {
                UsbState::RateLocked | UsbState::PhaseLocked => {
                    setup.host_ppm + 0.05 * (since * std::f32::consts::TAU / 20.0).sin()
                }
                UsbState::PullIn => setup.host_ppm / 2.0,
                _ => 0.0,
            },
            phase_error: if locked {
                0.1 * (since * std::f32::consts::TAU / 30.0).sin()
            } else {
                0.0
            },
            sample: if time_base { self.sample() } else { 0 },
            bus_frame: if time_base { bus_of(n) } else { 0 },
            d_min,
            d_max,
            fill: if locked {
                self.config()
                    .d
                    .saturating_sub(SPF.saturating_add(1).saturating_add(16))
            } else {
                0
            },
            stream_first_bus_frame: stream.map_or(usb::NO_STREAM, bus_of),
            gap_count: self.gap_count,
            stream_age_frames: age32,
            stream_shift: if stream.is_some() {
                setup.stream_shift
            } else {
                0
            },
            stream_samples: age32.saturating_mul(u32::from(SPF)),
            anchors: self.anchors,
            reanchors: self.reanchors,
            missed_samples: self.missed,
            late_samples: 0,
        }
    }

    /// `EXT_ALINK_USB` IN.
    fn config_in(&mut self) -> AlinkUsbConfig {
        let state = self.state();
        let c = self.config();
        let (d_min, d_max) = self.limits();
        let mut status_flags = 0;
        if c.mode == usb::MODE_PHASE_LOCK {
            if c.d < d_min {
                status_flags |= usb::STATUS_D_BELOW_MIN;
            }
            if c.d > d_max {
                status_flags |= usb::STATUS_D_ABOVE_MAX;
            }
        }
        AlinkUsbConfig {
            status_flags,
            state: state.raw(),
            ..c
        }
    }

    /// Whether S on time base 1 or 2 may be used now.
    fn phase_locked(&mut self) -> bool {
        self.state() == UsbState::PhaseLocked
    }
}

impl State {
    /// Whether the USB time base answers `EXT_FRAME_COUNTER` and
    /// `EXT_APPLY_AT_FRAME` (a USB-direct unit not in mode 0).
    fn usb_time_base(&self) -> bool {
        self.alink_usb.on && self.alink_usb.config().mode != usb::MODE_OFF
    }

    /// The ids a USB-direct unit adds to `EXT_LIST`: the frame counter and the
    /// apply (when the alink does not list them) and the block.
    pub(super) fn alink_usb_ids(&self) -> Vec<u16> {
        if !self.alink_usb.on {
            return Vec::new();
        }
        let mut ids = vec![alink::EXT_FRAME_COUNTER, alink::EXT_APPLY_AT_FRAME];
        ids.extend(usb::ALINK_USB_IDS);
        ids
    }

    /// The notify feature bits a USB-direct unit adds: 0xE2 and 0xE3.
    pub(super) fn alink_usb_features(&self) -> u32 {
        if self.alink_usb.on {
            alink::NOTIFY_FEAT_APPLY_RESULT | usb::NOTIFY_FEAT_ALINK_USB
        } else {
            0
        }
    }

    /// IN: `Some(reply)` (`Some(None)` stalls) for the block, and for the
    /// frame counter and the apply while the USB time base answers (or the
    /// fake has no alink); `None` for anything else.
    pub(super) fn get_alink_usb(&mut self, value: u16) -> Option<Option<Vec<u8>>> {
        if !self.alink_usb.on {
            return None;
        }
        let shared = matches!(value, alink::EXT_FRAME_COUNTER | alink::EXT_APPLY_AT_FRAME);
        if !usb::ALINK_USB_IDS.contains(&value)
            && !(shared && (self.usb_time_base() || !self.options.alink))
        {
            return None;
        }
        self.run_due_usb_ops();
        Some(match value {
            usb::EXT_ALINK_USB => Some(self.alink_usb.config_in().encode_in().to_vec()),
            usb::EXT_ALINK_USB_STATUS => Some(self.alink_usb.status().encode().to_vec()),
            usb::EXT_ALINK_USB_DIAG => {
                let mut d = self
                    .alink_usb
                    .diag
                    .unwrap_or_else(|| self.alink_usb.diag_clear());
                d.max_pass_us = 1800;
                d.max_isr_quiet_us = 1100;
                d.max_isr_late_us = 12;
                Some(d.encode().to_vec())
            }
            alink::EXT_FRAME_COUNTER => {
                let us = Instant::now()
                    .saturating_duration_since(self.started)
                    .as_micros();
                let time_us = (us & u128::from(u32::MAX)) as u32;
                let state = self.alink_usb.state();
                let tb = self.usb_time_base() && !matches!(state, UsbState::NoSof);
                let (n, _) = self.alink_usb.frames();
                let s = self.alink_usb.sample();
                let c = if tb {
                    FrameCounterTb {
                        counter: FrameCounter {
                            frame: s,
                            time_us,
                            output_frames: low32(s.saturating_sub(48_000)),
                        },
                        time_base: TimeBase::UsbSample,
                        bus_frame: bus_of(n),
                    }
                } else {
                    FrameCounterTb {
                        counter: FrameCounter {
                            frame: 0,
                            time_us,
                            output_frames: 0,
                        },
                        time_base: TimeBase::None,
                        bus_frame: 0,
                    }
                };
                Some(c.encode().to_vec())
            }
            alink::EXT_APPLY_AT_FRAME => {
                let pending = u8::try_from(self.alink_usb.queue.len()).unwrap_or(u8::MAX);
                let last = self.alink_usb.last.unwrap_or(ApplyStatus {
                    pending: 0,
                    last_op: ApplyOp::CancelAll,
                    last_result: ApplyResult::Applied,
                    last_frame: 0,
                });
                Some(ApplyStatus { pending, ..last }.encode().to_vec())
            }
            _ => None,
        })
    }

    /// OUT: `Some(accepted)` for the block, [`EXT_FAKE_ALINK_USB_SIM`], and an
    /// apply on time base 1 or 2 (or any while the USB time base answers);
    /// `None` for anything else.
    pub(super) fn set_alink_usb(&mut self, value: u16, data: &[u8]) -> Option<bool> {
        if value == EXT_FAKE_ALINK_USB_SIM {
            return Some(match *data {
                [op] if self.alink_usb.on => self.usb_sim(op),
                _ => false,
            });
        }
        if !self.alink_usb.on {
            return None;
        }
        match value {
            usb::EXT_ALINK_USB => Some(self.write_usb_config(data)),
            usb::EXT_ALINK_USB_DIAG => Some(match *data {
                [1] => {
                    self.alink_usb.diag = Some(self.alink_usb.diag_clear());
                    true
                }
                _ => false,
            }),
            usb::EXT_ALINK_USB_STATUS => Some(false),
            alink::EXT_APPLY_AT_FRAME => {
                let (req, tb) = usb::decode_apply(data)?;
                match tb {
                    TimeBase::UsbSample | TimeBase::BusFrame => Some(
                        data.len() == ApplyRequest::SIZE
                            && data.get(13..) == Some(&[0; 3])
                            && self.queue_usb_op(req, tb),
                    ),
                    // Time base 0 is the alink's frame.
                    TimeBase::Alink if self.options.alink => None,
                    _ => Some(false),
                }
            }
            _ => None,
        }
    }

    /// 2.9.2's refusals; a change of mode or D re-anchors.
    fn write_usb_config(&mut self, data: &[u8]) -> bool {
        let Some(c) = AlinkUsbConfig::decode(data) else {
            return false;
        };
        let (d_min, d_max) = self.alink_usb.limits();
        // -- WP AUSB2 -- byte 4 is `on_lock_error` since WP USB3b, reserved before
        let byte4_ok = c.on_lock_error == 0 || self.alink_usb.setup().usb3b;
        if data.len() != AlinkUsbConfig::SIZE
            || data.get(5) != Some(&0)
            || !byte4_ok
            || !c.fits(d_min, d_max)
        {
            return false;
        }
        let c = AlinkUsbConfig {
            status_flags: 0,
            state: 0,
            ..c
        };
        let old = self.alink_usb.config();
        if (old.mode, old.d, old.flags) == (c.mode, c.d, c.flags) {
            // -- WP AUSB2 -- the lock-error action alone changes nothing else
            self.alink_usb.config = Some(c);
            return true;
        }
        let was = self.alink_usb.state();
        let was_error = self.alink_usb.lock_error();
        self.alink_usb.config = Some(c);
        match (old.mode, c.mode) {
            (_, usb::MODE_OFF) => {
                self.alink_usb.settling = None;
                for q in std::mem::take(&mut self.alink_usb.queue) {
                    self.usb_op_done(q, ApplyResult::Cancelled);
                }
                self.usb_event(UsbEventReason::Disabled);
            }
            (usb::MODE_OFF, _) => {
                self.alink_usb.start(UsbState::PullIn, PULL_IN);
                self.usb_event(UsbEventReason::None);
            }
            (_, usb::MODE_PHASE_LOCK) => {
                if was == UsbState::PhaseLocked {
                    self.alink_usb.reanchors = self.alink_usb.reanchors.saturating_add(1);
                    self.bump_end(7);
                }
                self.alink_usb.start(UsbState::Anchoring, ANCHOR);
                self.usb_event(UsbEventReason::Reanchor);
            }
            _ => {
                // Phase lock to rate lock: the timeline ends, the rate stays.
                self.alink_usb.settling = None;
                self.usb_event(UsbEventReason::None);
            }
        }
        self.lock_error_since(was_error);
        true
    }

    // -- WP AUSB2 --
    /// Mode 2 entered (`LOCK_ERROR`, DIAG `lock_errors`) or left (`LOCK_OK`)
    /// lock error since it was `was` (2.9.2).
    fn lock_error_since(&mut self, was: bool) {
        match (was, self.alink_usb.lock_error()) {
            (false, true) => {
                if let Some(d) = self.alink_usb.diag.as_mut() {
                    d.lock_errors = d.lock_errors.saturating_add(1);
                }
                self.usb_event(UsbEventReason::LockError);
            }
            (true, false) => self.usb_event(UsbEventReason::LockOk),
            _ => {}
        }
    }

    /// Count a timeline end of cause `i` in DIAG.
    fn bump_end(&mut self, i: u8) {
        if let Some(d) = self.alink_usb.diag.as_mut() {
            if let Some(e) = d.ends.get_mut(usize::from(i)) {
                *e = e.saturating_add(1);
            }
            d.last_cause = Some(i);
        }
    }

    /// An op on time base 1 or 2: refused unless phase-locked, a full queue,
    /// a bad bus form or bad arguments; late (run now) when its sample has
    /// passed. `CANCEL_ALL` cancels the queue in any state.
    fn queue_usb_op(&mut self, r: ApplyRequest, tb: TimeBase) -> bool {
        if r.op == ApplyOp::CancelAll {
            for q in std::mem::take(&mut self.alink_usb.queue) {
                self.usb_op_done(q, ApplyResult::Cancelled);
            }
            return true;
        }
        if !r.args_valid() || !self.alink_usb.phase_locked() {
            return false;
        }
        let now = self.alink_usb.sample();
        let target = match tb {
            TimeBase::UsbSample => r.frame,
            _ => {
                let Some((bus, k)) = usb::decode_bus_target(r.frame) else {
                    return false;
                };
                if u16::from(k) >= SPF {
                    return false;
                }
                let (n, _) = self.alink_usb.frames();
                let wrap = u64::from(usb::BUS_FRAMES);
                let delta = u64::from(bus)
                    .saturating_add(wrap)
                    .saturating_sub(u64::from(bus_of(n)))
                    .checked_rem(wrap)
                    .unwrap_or(0);
                let past = wrap.saturating_sub(u64::from(usb::BUS_PAST_FRAMES));
                let frame = if delta >= past {
                    n.saturating_sub(wrap.saturating_sub(delta))
                } else {
                    n.saturating_add(delta)
                };
                frame
                    .saturating_mul(u64::from(SPF))
                    .saturating_add(u64::from(k))
            }
        };
        let req = ApplyRequest { frame: target, ..r };
        // A sample already written to the rings (about a frame behind the arrival).
        if target <= now.saturating_sub(u64::from(SPF)) {
            self.usb_op_done(req, ApplyResult::Late);
            return true;
        }
        if self.alink_usb.queue.len() >= APPLY_QUEUE {
            return false;
        }
        self.alink_usb.queue.push(req);
        true
    }

    /// Run the queued ops whose sample has come.
    fn run_due_usb_ops(&mut self) {
        let now = self.alink_usb.sample();
        let mut due = Vec::new();
        self.alink_usb.queue.retain(|r| {
            let d = r.frame <= now;
            if d {
                due.push(*r);
            }
            !d
        });
        due.sort_by_key(|r| r.frame);
        for r in due {
            // No click injection or GPIO in the demo: refused at apply time.
            let result = match r.op {
                ApplyOp::Click | ApplyOp::SyncPulse => ApplyResult::Refused,
                _ => ApplyResult::Applied,
            };
            self.usb_op_done(r, result);
        }
    }

    fn usb_op_done(&mut self, r: ApplyRequest, result: ApplyResult) {
        self.alink_usb.last = Some(ApplyStatus {
            pending: 0,
            last_op: r.op,
            last_result: result,
            last_frame: r.frame,
        });
        let (bus, k) = usb::bus_of_sample(r.frame, SPF).unwrap_or((0, 0));
        let e = ApplyResultEvent {
            op: r.op,
            result,
            frame_low: u32::from(bus) | u32::from(k).checked_shl(16).unwrap_or(0),
        };
        self.push(packet(alink::NOTIFY_EVT_APPLY_RESULT, &e.encode()));
    }

    fn usb_event(&mut self, reason: UsbEventReason) {
        let status = self.alink_usb.status();
        let e = AlinkUsbEvent {
            state: status.state,
            phase_state: status.phase_state,
            reason,
            sample_low: low32(status.sample),
        };
        self.push(packet(usb::NOTIFY_EVT_ALINK_USB, &e.encode()));
    }

    fn usb_sim(&mut self, op: u8) -> bool {
        match op {
            USB_SIM_HOLDOVER => {
                if let Some(d) = self.alink_usb.diag.as_mut() {
                    d.holdovers = d.holdovers.saturating_add(1);
                    d.rides = d.rides.saturating_add(1);
                    d.last_frames = 300;
                    d.last_us = 300_000;
                    d.last_dev_us = 0;
                    d.longest_ride_us = d.longest_ride_us.max(300_000);
                }
                self.usb_event(UsbEventReason::Holdover);
                self.usb_event(UsbEventReason::Resumed);
            }
            USB_SIM_SOF_LOST => {
                self.alink_usb.lost_until = Instant::now().checked_add(Duration::from_millis(200));
                self.alink_usb.settling = None;
                self.bump_end(0);
                if let Some(d) = self.alink_usb.diag.as_mut() {
                    d.bus_resets = d.bus_resets.saturating_add(1);
                }
                self.usb_event(UsbEventReason::SofLost);
                if self.alink_usb.config().mode == usb::MODE_PHASE_LOCK {
                    // Back after the bus: anchor again.
                    let after = Duration::from_millis(200).saturating_add(ANCHOR);
                    self.alink_usb.settling = Instant::now()
                        .checked_add(after)
                        .map(|t| (UsbState::Anchoring, t));
                }
            }
            USB_SIM_GAP => {
                let u = &mut self.alink_usb;
                u.gap_count = u.gap_count.wrapping_add(1);
                u.missed = u.missed.saturating_add(u32::from(SPF).saturating_mul(2));
                self.usb_event(UsbEventReason::Gap);
            }
            USB_SIM_HOST_RANGE => {
                let was = self.alink_usb.lock_error();
                self.alink_usb.rejected = !self.alink_usb.rejected;
                if self.alink_usb.rejected {
                    self.bump_end(3);
                }
                self.usb_event(UsbEventReason::HostRange);
                self.lock_error_since(was);
            }
            USB_SIM_STREAM_RESTART => {
                self.alink_usb.place_stream();
            }
            // -- fixup ausb2 --
            USB_SIM_RATE_96K => {
                let was = self.alink_usb.lock_error();
                self.alink_usb.rate_96k = !self.alink_usb.rate_96k;
                if self.alink_usb.config_bad() {
                    self.bump_end(4);
                    self.usb_event(UsbEventReason::Config);
                }
                self.lock_error_since(was);
            }
            USB_SIM_SETTLE => {
                while self.alink_usb.settling.is_some() {
                    self.alink_usb.settle_step();
                }
                self.alink_usb.lost_until = None;
                self.usb_event(UsbEventReason::Locked);
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests_wp_ausb1;

// -- WP AUSB2 --
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests_wp_ausb2;
