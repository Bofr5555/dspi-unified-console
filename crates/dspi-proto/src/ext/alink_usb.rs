//! WP AUSB1: the alink's USB-direct block (0x8010-0x8012, the fork's private
//! range next to the alink's) and the USB time base of `EXT_FRAME_COUNTER`
//! (0x8005) and `EXT_APPLY_AT_FRAME` (0x8006), with event 0xE3.
//!
//! The layouts are the fork's `extension_channel_spec.md` section 2.9 (WP USB1
//! and USB2, DSPi-bofr bofr-main ebdf538), transcribed as the alink's are: a
//! unit on a USB input locks its clock to the host's Start of Frame (mode 1)
//! and, in mode 2, plays every USB sample a fixed D samples after it arrived,
//! so every unit of one host plays sample-aligned.
//!
//! Pure codecs; the I/O is in `dspi_session::ext`. Every decoder takes device
//! bytes and never panics: a short reply is `None`, extra bytes are ignored.
//! A host reads none of these unless `EXT_LIST` names them (`EXT_ALINK_USB` is
//! the block's discovery, 2.9), and decodes 0xE3 only behind
//! `EXT_NOTIFY_FEATURES` bit 4.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::alink::{ApplyRequest, FrameCounter};

/// `EXT_ALINK_USB` IN/OUT, 8 B: mode, flags, D.
pub const EXT_ALINK_USB: u16 = 0x8010;
/// `EXT_ALINK_USB_STATUS` IN, 64 B: state, host rate, phase, stream start.
pub const EXT_ALINK_USB_STATUS: u16 = 0x8011;
/// `EXT_ALINK_USB_DIAG` IN 64 B / OUT 1 B (clear): why a timeline ended.
pub const EXT_ALINK_USB_DIAG: u16 = 0x8012;

/// The block's sub-commands, ascending.
pub const ALINK_USB_IDS: [u16; 3] = [EXT_ALINK_USB, EXT_ALINK_USB_STATUS, EXT_ALINK_USB_DIAG];

/// What a sub-command of the block is called, for listings; `None` outside
/// it. Always "audio link": "link" alone is DSPi Link.
pub fn alink_usb_name(id: u16) -> Option<&'static str> {
    Some(match id {
        EXT_ALINK_USB => "audio link USB-direct",
        EXT_ALINK_USB_STATUS => "audio link USB-direct status",
        EXT_ALINK_USB_DIAG => "audio link USB-direct diagnostics",
        _ => return None,
    })
}

/// `EXT_NOTIFY_FEATURES` bit 4: the device sends [`NOTIFY_EVT_ALINK_USB`].
pub const NOTIFY_FEAT_ALINK_USB: u32 = 1 << 4;
/// `NOTIFY_EVT_ALINK_USB`: a USB-direct state change, gap or re-anchor.
pub const NOTIFY_EVT_ALINK_USB: u8 = 0xE3;

/// `EXT_ALINK_USB.mode` 0: stock (asynchronous feedback, the crystal clock).
pub const MODE_OFF: u8 = 0;
/// `EXT_ALINK_USB.mode` 1: the clock locked to the host's SOF rate.
pub const MODE_RATE_LOCK: u8 = 1;
/// `EXT_ALINK_USB.mode` 2: rate lock and phase lock (play S at arrival + D).
pub const MODE_PHASE_LOCK: u8 = 2;
/// `EXT_ALINK_USB.flags` bit 0: a phase error beyond 2 samples re-anchors.
pub const FLAG_AUTO_REANCHOR: u8 = 1 << 0;
/// `EXT_ALINK_USB.status_flags` bit 0: D below `d_min` (runs as mode 1).
pub const STATUS_D_BELOW_MIN: u8 = 1 << 0;
/// `EXT_ALINK_USB.status_flags` bit 1: D above `d_max` (runs as mode 1).
pub const STATUS_D_ABOVE_MAX: u8 = 1 << 1;
/// `EXT_ALINK_USB.status_flags` bit 2: mode 2 at 44.1 kHz (runs as mode 1).
pub const STATUS_RATE_UNSUPPORTED: u8 = 1 << 2;
/// The firmware's default D, samples (4.0 ms at 48 kHz; valid at 48 kHz only).
pub const DEFAULT_D: u16 = 192;

/// `EXT_ALINK_USB_STATUS.flags` bit 0: RP2350 hardware SOF timestamps.
pub const USB_FLAG_HW_TIMESTAMP: u8 = 1 << 0;
/// bit 1: the host has the audio interface at alt > 0 (a stream).
pub const USB_FLAG_STREAM: u8 = 1 << 1;
/// bit 2: the host follows the SOF-locked clock (nominal feedback).
pub const USB_FLAG_NOMINAL_FEEDBACK: u8 = 1 << 2;
/// bit 3: a usable time base (SOF within 50 ms, or a holdover).
pub const USB_FLAG_TIME_BASE: u8 = 1 << 3;
/// bit 4: no SOF for over 50 ms: the time base is extrapolated (WP USB2).
pub const USB_FLAG_HOLDOVER: u8 = 1 << 4;

/// The 11-bit bus frame number wraps here.
pub const BUS_FRAMES: u16 = 2048;
/// A bus-frame target within this many frames before now is past (2.9.4).
pub const BUS_PAST_FRAMES: u16 = 64;
/// `stream_first_bus_frame` without a stream.
pub const NO_STREAM: u16 = 0xFFFF;
/// `EXT_ALINK_USB_DIAG`'s layout version this build reads.
pub const DIAG_VERSION: u8 = 1;
/// `EXT_ALINK_USB_DIAG` OUT: clear every field.
pub const DIAG_CLEAR: [u8; 1] = [1];
/// `EXT_ALINK_USB_DIAG.last_cause` with no timeline end yet.
pub const NO_CAUSE: u8 = 0xFF;

/// `EXT_FRAME_COUNTER`'s reply on a build with the USB time base (16 before
/// WP USB1; a host reads at least 16).
pub const FRAME_COUNTER_TB_SIZE: usize = 20;

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    b.get(i..i.checked_add(2)?)?
        .try_into()
        .ok()
        .map(u16::from_le_bytes)
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    b.get(i..i.checked_add(4)?)?
        .try_into()
        .ok()
        .map(u32::from_le_bytes)
}

fn i32_at(b: &[u8], i: usize) -> Option<i32> {
    u32_at(b, i).map(|v| i32::from_le_bytes(v.to_le_bytes()))
}

fn f32_at(b: &[u8], i: usize) -> Option<f32> {
    u32_at(b, i).map(f32::from_bits)
}

fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    b.get(i..i.checked_add(8)?)?
        .try_into()
        .ok()
        .map(u64::from_le_bytes)
}

fn byte(b: &[u8], i: usize) -> Option<u8> {
    b.get(i).copied()
}

/// Write `v` at `at` in `b`, if it fits (it always does for the fixed layouts).
fn put(b: &mut [u8], at: usize, v: &[u8]) {
    if let Some(end) = at.checked_add(v.len())
        && let Some(dst) = b.get_mut(at..end)
    {
        dst.copy_from_slice(v);
    }
}

// ------------------------------------------------------------ enums

/// `EXT_ALINK_USB.mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbMode {
    /// 0: stock.
    Off,
    /// 1: SOF rate lock.
    RateLock,
    /// 2: SOF rate lock and phase lock.
    PhaseLock,
    /// A code this build does not know.
    Other(u8),
}

impl UsbMode {
    /// From the wire.
    pub fn from_raw(b: u8) -> Self {
        match b {
            MODE_OFF => Self::Off,
            MODE_RATE_LOCK => Self::RateLock,
            MODE_PHASE_LOCK => Self::PhaseLock,
            o => Self::Other(o),
        }
    }

    /// To the wire.
    pub fn raw(self) -> u8 {
        match self {
            Self::Off => MODE_OFF,
            Self::RateLock => MODE_RATE_LOCK,
            Self::PhaseLock => MODE_PHASE_LOCK,
            Self::Other(o) => o,
        }
    }

    /// `off`, `rate_lock`, `phase_lock` or `unknown`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::RateLock => "rate_lock",
            Self::PhaseLock => "phase_lock",
            Self::Other(_) => "unknown",
        }
    }

    /// From [`Self::as_str`]'s words; `None` for any other.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "off" => Self::Off,
            "rate_lock" => Self::RateLock,
            "phase_lock" => Self::PhaseLock,
            _ => return None,
        })
    }
}

/// `EXT_ALINK_USB_STATUS.state` (and `EXT_ALINK_USB` byte 7, event 0xE3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbState {
    /// 0: mode 0, or the source is not USB.
    Off,
    /// 1: no SOF (not configured, suspended, unplugged).
    NoSof,
    /// 2: measuring the host, VCXO slewing.
    PullIn,
    /// 3: rate-locked (mode 1, or mode 2 that cannot phase-lock).
    RateLocked,
    /// 4: mode 2 anchoring; muted.
    Anchoring,
    /// 5: phase-locked.
    PhaseLocked,
    /// 6: the host is beyond 295 ppm: stock behaviour.
    Rejected,
    /// A code this build does not know.
    Other(u8),
}

impl UsbState {
    /// From the wire.
    pub fn from_raw(b: u8) -> Self {
        match b {
            0 => Self::Off,
            1 => Self::NoSof,
            2 => Self::PullIn,
            3 => Self::RateLocked,
            4 => Self::Anchoring,
            5 => Self::PhaseLocked,
            6 => Self::Rejected,
            o => Self::Other(o),
        }
    }

    /// To the wire.
    pub fn raw(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::NoSof => 1,
            Self::PullIn => 2,
            Self::RateLocked => 3,
            Self::Anchoring => 4,
            Self::PhaseLocked => 5,
            Self::Rejected => 6,
            Self::Other(o) => o,
        }
    }

    /// `off`, `no_sof`, `pull_in`, `rate_locked`, `anchoring`,
    /// `phase_locked`, `rejected` or `unknown`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::NoSof => "no_sof",
            Self::PullIn => "pull_in",
            Self::RateLocked => "rate_locked",
            Self::Anchoring => "anchoring",
            Self::PhaseLocked => "phase_locked",
            Self::Rejected => "rejected",
            Self::Other(_) => "unknown",
        }
    }
}

/// Why event 0xE3 was sent (byte 6, 2.9.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbEventReason {
    /// 0: a state change only.
    None,
    /// 1: rate lock reached (mode 1), or phase-locked and playing (mode 2).
    Locked,
    /// 2: state 6 entered, or the host back in range.
    HostRange,
    /// 3: the time base ended; the unit anchors again when the bus is back.
    SofLost,
    /// 4: re-anchored.
    Reanchor,
    /// 5: packets missing while streaming (a faded gap).
    Gap,
    /// 6: mode 0 written, or the source left USB.
    Disabled,
    /// 7: mode 2 cannot phase-lock (44.1 kHz, D outside the limits).
    Config,
    /// 8: no SOF for 50 ms: extrapolating, timeline kept.
    Holdover,
    /// 9: SOF back within 10 s on the same timeline.
    Resumed,
    /// A code this build does not know.
    Other(u8),
}

impl UsbEventReason {
    /// From the wire.
    pub fn from_raw(b: u8) -> Self {
        match b {
            0 => Self::None,
            1 => Self::Locked,
            2 => Self::HostRange,
            3 => Self::SofLost,
            4 => Self::Reanchor,
            5 => Self::Gap,
            6 => Self::Disabled,
            7 => Self::Config,
            8 => Self::Holdover,
            9 => Self::Resumed,
            o => Self::Other(o),
        }
    }

    /// To the wire.
    pub fn raw(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Locked => 1,
            Self::HostRange => 2,
            Self::SofLost => 3,
            Self::Reanchor => 4,
            Self::Gap => 5,
            Self::Disabled => 6,
            Self::Config => 7,
            Self::Holdover => 8,
            Self::Resumed => 9,
            Self::Other(o) => o,
        }
    }

    /// `none`, `locked`, `host_range`, `sof_lost`, `reanchor`, `gap`,
    /// `disabled`, `config`, `holdover`, `resumed` or `unknown`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Locked => "locked",
            Self::HostRange => "host_range",
            Self::SofLost => "sof_lost",
            Self::Reanchor => "reanchor",
            Self::Gap => "gap",
            Self::Disabled => "disabled",
            Self::Config => "config",
            Self::Holdover => "holdover",
            Self::Resumed => "resumed",
            Self::Other(_) => "unknown",
        }
    }
}

/// The time base of `EXT_FRAME_COUNTER` (byte 16) and `EXT_APPLY_AT_FRAME`
/// OUT (byte 12, 2.9.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeBase {
    /// 0: the alink frame F.
    Alink,
    /// 1: the USB sample index S.
    UsbSample,
    /// 2: a bus frame and the sample in it (apply only).
    BusFrame,
    /// `0xFF` (the counter): no time base.
    None,
    /// A code this build does not know.
    Other(u8),
}

impl TimeBase {
    /// From the wire.
    pub fn from_raw(b: u8) -> Self {
        match b {
            0 => Self::Alink,
            1 => Self::UsbSample,
            2 => Self::BusFrame,
            0xFF => Self::None,
            o => Self::Other(o),
        }
    }

    /// To the wire.
    pub fn raw(self) -> u8 {
        match self {
            Self::Alink => 0,
            Self::UsbSample => 1,
            Self::BusFrame => 2,
            Self::None => 0xFF,
            Self::Other(o) => o,
        }
    }

    /// `alink`, `usb_sample`, `bus_frame`, `none` or `unknown`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Alink => "alink",
            Self::UsbSample => "usb_sample",
            Self::BusFrame => "bus_frame",
            Self::None => "none",
            Self::Other(_) => "unknown",
        }
    }
}

/// `EXT_ALINK_USB_DIAG.ends[i]`'s cause (2.9.6), `None` past the table.
pub fn end_cause_name(i: u8) -> Option<&'static str> {
    Some(match i {
        0 => "sof_lost",
        1 => "long_gap",
        2 => "discontinuity",
        3 => "host_rejected",
        4 => "rate_or_config",
        5 => "reanchor_phase_error",
        6 => "reanchor_output_reset",
        7 => "reanchor_other",
        8 => "stop",
        _ => return None,
    })
}

// ------------------------------------------------------------ limits

/// Samples per 1 ms bus frame at `rate_code` (1 = 48 kHz, 2 = 96 kHz); `None`
/// at 44.1 kHz (no whole number: rate lock only) or an unknown rate.
pub fn samples_per_frame(rate_code: u8) -> Option<u16> {
    match rate_code {
        1 => Some(48),
        2 => Some(96),
        _ => None,
    }
}

/// `d_min`, `d_max` (2.9.2) at `spf` samples per frame, for every output
/// latency option USB runs (block cap `spf + 1`): 170..399 at 48 kHz,
/// 324..351 at 96 kHz.
pub fn d_limits(spf: u16) -> (u16, u16) {
    let cap = spf.saturating_add(1);
    let min = spf
        .saturating_mul(2)
        .saturating_add(spf.checked_div(5).unwrap_or(0))
        .saturating_add(cap)
        .saturating_add(16);
    (min, 448u16.saturating_sub(cap))
}

// ------------------------------------------------------------ records

/// `EXT_ALINK_USB` IN/OUT, 8 bytes (2.9.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlinkUsbConfig {
    /// [`MODE_OFF`], [`MODE_RATE_LOCK`] or [`MODE_PHASE_LOCK`].
    pub mode: u8,
    /// [`FLAG_AUTO_REANCHOR`]; the other bits 0.
    pub flags: u8,
    /// Samples from a USB sample's arrival to its output.
    pub d: u16,
    /// IN only: `STATUS_*` bits.
    pub status_flags: u8,
    /// IN only: [`UsbState`]'s code.
    pub state: u8,
}

impl AlinkUsbConfig {
    /// The wire size.
    pub const SIZE: usize = 8;

    /// The firmware's default: mode 0, automatic re-anchor, D 192.
    pub const DEFAULT: AlinkUsbConfig = AlinkUsbConfig {
        mode: MODE_OFF,
        flags: FLAG_AUTO_REANCHOR,
        d: DEFAULT_D,
        status_flags: 0,
        state: 0,
    };

    /// From an IN reply.
    pub fn decode(b: &[u8]) -> Option<Self> {
        Some(Self {
            mode: byte(b, 0)?,
            flags: byte(b, 1)?,
            d: u16_at(b, 2)?,
            status_flags: byte(b, 6)?,
            state: byte(b, 7)?,
        })
    }

    /// The OUT payload: the IN-only bytes and the reserved ones are 0.
    pub fn encode(&self) -> [u8; Self::SIZE] {
        let [d0, d1] = self.d.to_le_bytes();
        [self.mode, self.flags, d0, d1, 0, 0, 0, 0]
    }

    /// The IN reply (for fakes and fixtures): with the IN-only bytes.
    pub fn encode_in(&self) -> [u8; Self::SIZE] {
        let mut b = self.encode();
        b[6] = self.status_flags;
        b[7] = self.state;
        b
    }

    /// What the firmware refuses regardless of the rate: a mode above 2, an
    /// undefined flag bit, D 0. (Mode 2's D range is checked against the
    /// unit's `d_min..d_max`, [`Self::fits`].)
    pub fn is_valid(&self) -> bool {
        self.mode <= MODE_PHASE_LOCK && self.flags & !FLAG_AUTO_REANCHOR == 0 && self.d != 0
    }

    /// Valid, and in mode 2 with D inside `d_min..=d_max`.
    pub fn fits(&self, d_min: u16, d_max: u16) -> bool {
        self.is_valid() && (self.mode != MODE_PHASE_LOCK || (d_min..=d_max).contains(&self.d))
    }

    /// The mode.
    pub fn usb_mode(&self) -> UsbMode {
        UsbMode::from_raw(self.mode)
    }
}

/// `EXT_ALINK_USB_STATUS` IN, 64 bytes (2.9.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlinkUsbStatus {
    /// [`UsbState`].
    pub state: UsbState,
    /// 0 free, 1 anchoring, 2 locked, 3 re-anchor pending.
    pub phase_state: u8,
    /// 0 = 44.1 kHz, 1 = 48 kHz, 2 = 96 kHz, `0xFF` other.
    pub rate_code: u8,
    /// `USB_FLAG_*` bits.
    pub flags: u8,
    /// The host's SOF rate against the unit's crystal, ppm (+ = host fast).
    pub host_ppm: f32,
    /// The soft VCXO's correction, ppm.
    pub vcxo_ppm: f32,
    /// The phase error, samples (phase-locked only).
    pub phase_error: f32,
    /// S arriving now; 0 without a time base.
    pub sample: u64,
    /// The bus frame of `sample`.
    pub bus_frame: u16,
    /// The smallest D for mode 2 now.
    pub d_min: u16,
    /// The largest.
    pub d_max: u16,
    /// Output ring fill of slot 0, samples.
    pub fill: u16,
    /// Bus frame of the current stream's first placed packet; [`NO_STREAM`].
    pub stream_first_bus_frame: u16,
    /// Faded gaps since boot (wraps).
    pub gap_count: u16,
    /// Frames since that first packet.
    pub stream_age_frames: u32,
    /// Silence inserted minus late samples dropped, since that packet.
    pub stream_shift: i32,
    /// Stream samples placed since that packet.
    pub stream_samples: u32,
    /// Anchors completed since boot.
    pub anchors: u32,
    /// Re-anchors since boot.
    pub reanchors: u32,
    /// Silence inserted for missing packets, since boot.
    pub missed_samples: u32,
    /// Samples dropped because their packet came too late, since boot.
    pub late_samples: u32,
}

impl AlinkUsbStatus {
    /// The wire size.
    pub const SIZE: usize = 64;

    /// From an IN reply.
    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        Some(Self {
            state: UsbState::from_raw(byte(b, 0)?),
            phase_state: byte(b, 1)?,
            rate_code: byte(b, 2)?,
            flags: byte(b, 3)?,
            host_ppm: f32_at(b, 4)?,
            vcxo_ppm: f32_at(b, 8)?,
            phase_error: f32_at(b, 12)?,
            sample: u64_at(b, 16)?,
            bus_frame: u16_at(b, 24)?,
            d_min: u16_at(b, 26)?,
            d_max: u16_at(b, 28)?,
            fill: u16_at(b, 30)?,
            stream_first_bus_frame: u16_at(b, 32)?,
            gap_count: u16_at(b, 34)?,
            stream_age_frames: u32_at(b, 36)?,
            stream_shift: i32_at(b, 40)?,
            stream_samples: u32_at(b, 44)?,
            anchors: u32_at(b, 48)?,
            reanchors: u32_at(b, 52)?,
            missed_samples: u32_at(b, 56)?,
            late_samples: u32_at(b, 60)?,
        })
    }

    /// The reply, for fakes and fixtures.
    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0] = self.state.raw();
        b[1] = self.phase_state;
        b[2] = self.rate_code;
        b[3] = self.flags;
        put(&mut b, 4, &self.host_ppm.to_le_bytes());
        put(&mut b, 8, &self.vcxo_ppm.to_le_bytes());
        put(&mut b, 12, &self.phase_error.to_le_bytes());
        put(&mut b, 16, &self.sample.to_le_bytes());
        put(&mut b, 24, &self.bus_frame.to_le_bytes());
        put(&mut b, 26, &self.d_min.to_le_bytes());
        put(&mut b, 28, &self.d_max.to_le_bytes());
        put(&mut b, 30, &self.fill.to_le_bytes());
        put(&mut b, 32, &self.stream_first_bus_frame.to_le_bytes());
        put(&mut b, 34, &self.gap_count.to_le_bytes());
        put(&mut b, 36, &self.stream_age_frames.to_le_bytes());
        put(&mut b, 40, &self.stream_shift.to_le_bytes());
        put(&mut b, 44, &self.stream_samples.to_le_bytes());
        put(&mut b, 48, &self.anchors.to_le_bytes());
        put(&mut b, 52, &self.reanchors.to_le_bytes());
        put(&mut b, 56, &self.missed_samples.to_le_bytes());
        put(&mut b, 60, &self.late_samples.to_le_bytes());
        b
    }

    /// Phase-locked (`phase_state` 2 or 3): what `EXT_APPLY_AT_FRAME` time
    /// bases 1 and 2 need.
    pub fn phase_locked(&self) -> bool {
        matches!(self.phase_state, 2 | 3)
    }

    /// Samples per bus frame at the unit's rate (`None` at 44.1 kHz).
    pub fn spf(&self) -> Option<u16> {
        samples_per_frame(self.rate_code)
    }

    /// The bus frame the current stream's content began in (2.9.3, U6):
    /// `(bus_frame - stream_age_frames) mod 2048`; `None` without a stream
    /// placed on the timeline.
    pub fn stream_start_bus_frame(&self) -> Option<u16> {
        if self.stream_first_bus_frame == NO_STREAM {
            return None;
        }
        let age = u16::try_from(self.stream_age_frames.checked_rem(u32::from(BUS_FRAMES))?).ok()?;
        let now = self.bus_frame.checked_rem(BUS_FRAMES)?;
        now.wrapping_add(BUS_FRAMES)
            .wrapping_sub(age)
            .checked_rem(BUS_FRAMES)
    }
}

/// Unit `a`'s content plays this many samples earlier than unit `b`'s
/// (2.9.3): `((first_b - first_a) mod 2048, signed) * spf + (shift_b -
/// shift_a)`, with each unit's stream start bus frame and `stream_shift`.
/// The streams started less than a second apart, so the frame difference is
/// taken in `-1024..1024`.
pub fn content_lead(a: (u16, i32), b: (u16, i32), spf: u16) -> i64 {
    let wrap = i64::from(BUS_FRAMES);
    let half = i64::from(BUS_FRAMES / 2);
    let frames = i64::from(b.0)
        .saturating_sub(i64::from(a.0))
        .saturating_add(half)
        .rem_euclid(wrap)
        .saturating_sub(half);
    frames
        .saturating_mul(i64::from(spf))
        .saturating_add(i64::from(b.1).saturating_sub(i64::from(a.1)))
}

/// `EXT_ALINK_USB_DIAG` IN, 64 bytes, layout version 1 (2.9.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlinkUsbDiag {
    /// The latest timeline end, an index into `ends`; `None` for none.
    pub last_cause: Option<u8>,
    /// SOF absences ridden through (resumed).
    pub rides: u16,
    /// SOF absences of 50 ms or more.
    pub holdovers: u16,
    /// Timeline ends by cause ([`end_cause_name`]).
    pub ends: [u16; 9],
    /// USB suspend events.
    pub suspends: u16,
    /// USB resume events.
    pub resumes: u16,
    /// USB bus resets.
    pub bus_resets: u16,
    /// Disconnects.
    pub unplugs: u16,
    /// The latest absence or discontinuity: frames counted on the bus.
    pub last_frames: u32,
    /// Its duration by the unit's clock, µs.
    pub last_us: u32,
    /// Clock minus frames at the host rate, µs.
    pub last_dev_us: i32,
    /// The longest absence ridden through, µs.
    pub longest_ride_us: u32,
    /// The longest main-loop pass while USB is the source, µs.
    pub max_pass_us: u32,
    /// The longest time without a SOF interrupt entry, µs.
    pub max_isr_quiet_us: u32,
    /// The latest a SOF interrupt ran after its SOF, µs.
    pub max_isr_late_us: u32,
    /// The running holdover's age, µs; 0 when none.
    pub holdover_us: u32,
}

impl AlinkUsbDiag {
    /// The wire size.
    pub const SIZE: usize = 64;

    /// From an IN reply; `None` short or for a layout other than
    /// [`DIAG_VERSION`].
    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        if byte(b, 0)? != DIAG_VERSION {
            return None;
        }
        let mut ends = [0u16; 9];
        for (i, e) in ends.iter_mut().enumerate() {
            *e = u16_at(b, i.checked_mul(2)?.checked_add(6)?)?;
        }
        let cause = byte(b, 1)?;
        Some(Self {
            last_cause: (cause != NO_CAUSE).then_some(cause),
            rides: u16_at(b, 2)?,
            holdovers: u16_at(b, 4)?,
            ends,
            suspends: u16_at(b, 24)?,
            resumes: u16_at(b, 26)?,
            bus_resets: u16_at(b, 28)?,
            unplugs: u16_at(b, 30)?,
            last_frames: u32_at(b, 32)?,
            last_us: u32_at(b, 36)?,
            last_dev_us: i32_at(b, 40)?,
            longest_ride_us: u32_at(b, 44)?,
            max_pass_us: u32_at(b, 48)?,
            max_isr_quiet_us: u32_at(b, 52)?,
            max_isr_late_us: u32_at(b, 56)?,
            holdover_us: u32_at(b, 60)?,
        })
    }

    /// The reply, for fakes and fixtures.
    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0] = DIAG_VERSION;
        b[1] = self.last_cause.unwrap_or(NO_CAUSE);
        put(&mut b, 2, &self.rides.to_le_bytes());
        put(&mut b, 4, &self.holdovers.to_le_bytes());
        for (i, e) in self.ends.iter().enumerate() {
            if let Some(at) = i.checked_mul(2).and_then(|o| o.checked_add(6)) {
                put(&mut b, at, &e.to_le_bytes());
            }
        }
        put(&mut b, 24, &self.suspends.to_le_bytes());
        put(&mut b, 26, &self.resumes.to_le_bytes());
        put(&mut b, 28, &self.bus_resets.to_le_bytes());
        put(&mut b, 30, &self.unplugs.to_le_bytes());
        put(&mut b, 32, &self.last_frames.to_le_bytes());
        put(&mut b, 36, &self.last_us.to_le_bytes());
        put(&mut b, 40, &self.last_dev_us.to_le_bytes());
        put(&mut b, 44, &self.longest_ride_us.to_le_bytes());
        put(&mut b, 48, &self.max_pass_us.to_le_bytes());
        put(&mut b, 52, &self.max_isr_quiet_us.to_le_bytes());
        put(&mut b, 56, &self.max_isr_late_us.to_le_bytes());
        put(&mut b, 60, &self.holdover_us.to_le_bytes());
        b
    }

    /// Every timeline end counted.
    pub fn total_ends(&self) -> u32 {
        self.ends
            .iter()
            .fold(0u32, |a, e| a.saturating_add(u32::from(*e)))
    }

    /// The latest end's cause by name.
    pub fn last_cause_name(&self) -> Option<&'static str> {
        self.last_cause.and_then(end_cause_name)
    }

    /// All zero, as after a clear (no end yet).
    pub const CLEAR: AlinkUsbDiag = AlinkUsbDiag {
        last_cause: None,
        rides: 0,
        holdovers: 0,
        ends: [0; 9],
        suspends: 0,
        resumes: 0,
        bus_resets: 0,
        unplugs: 0,
        last_frames: 0,
        last_us: 0,
        last_dev_us: 0,
        longest_ride_us: 0,
        max_pass_us: 0,
        max_isr_quiet_us: 0,
        max_isr_late_us: 0,
        holdover_us: 0,
    };
}

/// `EXT_FRAME_COUNTER` IN with its time base (2.9.4): 20 bytes on a build
/// with the USB time base, 16 on one before it (read as the alink's frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameCounterTb {
    /// The first 16 bytes: the frame (F or S), the unit's clock, the output.
    pub counter: FrameCounter,
    /// Which counter `frame` is.
    pub time_base: TimeBase,
    /// Time base 1 only: the bus frame of the latest SOF; 0 otherwise.
    pub bus_frame: u16,
}

impl FrameCounterTb {
    /// The wire size on a build with the USB time base.
    pub const SIZE: usize = FRAME_COUNTER_TB_SIZE;

    /// From an IN reply of at least 16 bytes.
    pub fn decode(b: &[u8]) -> Option<Self> {
        let counter = FrameCounter::decode(b)?;
        let (time_base, bus_frame) = match (byte(b, 16), u16_at(b, 18)) {
            (Some(tb), Some(bus)) => (TimeBase::from_raw(tb), bus),
            // A 16-byte reply: firmware before WP USB1, the alink's counter.
            _ => (TimeBase::Alink, 0),
        };
        Some(Self {
            counter,
            time_base,
            bus_frame,
        })
    }

    /// The 20-byte reply, for fakes and fixtures.
    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        put(&mut b, 0, &self.counter.encode());
        b[16] = self.time_base.raw();
        put(&mut b, 18, &self.bus_frame.to_le_bytes());
        b
    }
}

/// The 8 bytes of an `EXT_APPLY_AT_FRAME` time base 2 target: `bus_frame`
/// (0..2047) in bits 0-10, `sample` (below `spf`) in bits 16-23. `None` out
/// of range.
pub fn bus_target(bus_frame: u16, sample: u8, spf: u16) -> Option<u64> {
    if bus_frame >= BUS_FRAMES || u16::from(sample) >= spf {
        return None;
    }
    Some(u64::from(bus_frame) | u64::from(sample).checked_shl(16)?)
}

/// A time base 2 target's `(bus_frame, sample)`; `None` for a bad form (bits
/// outside 0-10 and 16-23).
pub fn decode_bus_target(frame: u64) -> Option<(u16, u8)> {
    if frame & !0x00FF_07FF != 0 {
        return None;
    }
    let bus = u16::try_from(frame & 0x07FF).ok()?;
    let sample = u8::try_from(frame.checked_shr(16)? & 0xFF).ok()?;
    Some((bus, sample))
}

/// The bus form of sample index `s` at `spf` samples per frame:
/// `((s / spf) mod 2048, s mod spf)`, as event 0xE2 carries a USB op's target.
pub fn bus_of_sample(s: u64, spf: u16) -> Option<(u16, u8)> {
    let spf = u64::from(spf);
    let frame = s.checked_div(spf)?;
    let bus = u16::try_from(frame.checked_rem(u64::from(BUS_FRAMES))?).ok()?;
    let k = u8::try_from(s.checked_rem(spf)?).ok()?;
    Some((bus, k))
}

/// `EXT_APPLY_AT_FRAME` OUT on time base `tb` (byte 12, reserved on the
/// alink): `req.frame` is F, S or a [`bus_target`].
pub fn encode_apply(req: &ApplyRequest, tb: TimeBase) -> [u8; ApplyRequest::SIZE] {
    let mut b = req.encode();
    b[12] = tb.raw();
    b
}

/// An `EXT_APPLY_AT_FRAME` OUT and its time base (byte 12).
pub fn decode_apply(b: &[u8]) -> Option<(ApplyRequest, TimeBase)> {
    Some((ApplyRequest::decode(b)?, TimeBase::from_raw(byte(b, 12)?)))
}

// ------------------------------------------------------------ the event

/// Event 0xE3: `[state, phase_state, reason, 0, S_low32]` from byte 4 of the
/// packet (2.9.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlinkUsbEvent {
    /// The state after it.
    pub state: UsbState,
    /// The phase state after it.
    pub phase_state: u8,
    /// Why it was sent.
    pub reason: UsbEventReason,
    /// S now, low 32 bits.
    pub sample_low: u32,
}

impl AlinkUsbEvent {
    /// The payload size.
    pub const PAYLOAD: usize = 8;

    /// From the payload (the packet from byte 4).
    pub fn decode(payload: &[u8]) -> Option<Self> {
        Some(Self {
            state: UsbState::from_raw(byte(payload, 0)?),
            phase_state: byte(payload, 1)?,
            reason: UsbEventReason::from_raw(byte(payload, 2)?),
            sample_low: u32_at(payload, 4)?,
        })
    }

    /// The payload.
    pub fn encode(&self) -> [u8; Self::PAYLOAD] {
        let [s0, s1, s2, s3] = self.sample_low.to_le_bytes();
        [
            self.state.raw(),
            self.phase_state,
            self.reason.raw(),
            0,
            s0,
            s1,
            s2,
            s3,
        ]
    }

    /// The timeline ended or moved (a lost time base, a re-anchor, the host
    /// out of range, switched off).
    pub fn is_loss(&self) -> bool {
        matches!(
            self.reason,
            UsbEventReason::SofLost | UsbEventReason::HostRange | UsbEventReason::Reanchor
        )
    }
}

/// Decode event 0xE3 from a device whose `EXT_NOTIFY_FEATURES` is
/// `notify_features`: only with bit 4; `None` otherwise.
pub fn alink_usb_event(
    notify_features: Option<u32>,
    id: u8,
    payload: &[u8],
) -> Option<AlinkUsbEvent> {
    let f = notify_features?;
    (id == NOTIFY_EVT_ALINK_USB && f & NOTIFY_FEAT_ALINK_USB != 0)
        .then(|| AlinkUsbEvent::decode(payload))
        .flatten()
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_ausb1 {
    use super::super::alink::ApplyOp;
    use super::*;

    #[test]
    fn wp_ausb1_the_config_example_decodes_and_encodes() {
        // 2.9.2: `02 01 c0 00 00 00 00 05`: phase lock, auto re-anchor, D 192, phase-locked.
        let raw = [0x02, 0x01, 0xc0, 0, 0, 0, 0, 0x05];
        let c = AlinkUsbConfig::decode(&raw).unwrap();
        assert_eq!(
            (c.usb_mode(), c.flags, c.d, c.state),
            (UsbMode::PhaseLock, FLAG_AUTO_REANCHOR, 192, 5)
        );
        assert_eq!(
            c.encode(),
            [0x02, 0x01, 0xc0, 0, 0, 0, 0, 0],
            "IN-only bytes 0 on OUT"
        );
        assert_eq!(c.encode_in(), raw);
        assert!(AlinkUsbConfig::decode(&raw[..7]).is_none());
        assert!(c.fits(170, 399));
        let bad = |c: AlinkUsbConfig| !c.is_valid();
        assert!(bad(AlinkUsbConfig { mode: 3, ..c }));
        assert!(bad(AlinkUsbConfig { flags: 2, ..c }));
        assert!(bad(AlinkUsbConfig { d: 0, ..c }));
        assert!(!AlinkUsbConfig { d: 400, ..c }.fits(170, 399));
        assert!(
            AlinkUsbConfig {
                d: 400,
                mode: 1,
                ..c
            }
            .fits(170, 399),
            "mode 1 ignores D"
        );
    }

    #[test]
    fn wp_ausb1_the_limits_are_the_specs() {
        assert_eq!(d_limits(48), (170, 399));
        assert_eq!(d_limits(96), (324, 351));
        assert_eq!(samples_per_frame(0), None);
        assert_eq!(samples_per_frame(2), Some(96));
    }

    #[test]
    fn wp_ausb1_the_status_round_trips_and_gives_the_stream_start() {
        let s = AlinkUsbStatus {
            state: UsbState::PhaseLocked,
            phase_state: 2,
            rate_code: 1,
            flags: USB_FLAG_STREAM | USB_FLAG_TIME_BASE,
            host_ppm: -11.5,
            vcxo_ppm: -11.4,
            phase_error: 0.11,
            sample: 48 * 5000 + 7,
            bus_frame: 5000 % 2048,
            d_min: 170,
            d_max: 399,
            fill: 150,
            stream_first_bus_frame: 10,
            gap_count: 1,
            stream_age_frames: 2000,
            stream_shift: -3,
            stream_samples: 96_000,
            anchors: 1,
            reanchors: 0,
            missed_samples: 48,
            late_samples: 0,
        };
        let b = s.encode();
        assert_eq!(AlinkUsbStatus::decode(&b), Some(s));
        assert_eq!(AlinkUsbStatus::decode(&b[..63]), None);
        // 904 - 2000 mod 2048 = 952.
        assert_eq!(s.stream_start_bus_frame(), Some(952));
        let none = AlinkUsbStatus {
            stream_first_bus_frame: NO_STREAM,
            ..s
        };
        assert_eq!(none.stream_start_bus_frame(), None);
        assert!(s.phase_locked() && s.spf() == Some(48));
    }

    #[test]
    fn wp_ausb1_content_lead_is_signed_across_the_wrap() {
        // B's stream began 2 frames later: A's content plays 96 samples earlier.
        assert_eq!(content_lead((2046, 0), (0, 0), 48), 96);
        assert_eq!(content_lead((0, 0), (2046, 0), 48), -96);
        assert_eq!(content_lead((5, 10), (5, 4), 48), -6);
    }

    #[test]
    fn wp_ausb1_the_diag_reads_only_version_1() {
        let mut d = AlinkUsbDiag::CLEAR;
        d.ends[0] = 3;
        d.ends[8] = 1;
        d.last_cause = Some(0);
        d.holdovers = 4;
        d.rides = 3;
        d.last_dev_us = -12;
        let b = d.encode();
        assert_eq!(AlinkUsbDiag::decode(&b), Some(d));
        assert_eq!(d.total_ends(), 4);
        assert_eq!(d.last_cause_name(), Some("sof_lost"));
        let mut v2 = b;
        v2[0] = 2;
        assert_eq!(AlinkUsbDiag::decode(&v2), None);
        assert_eq!(
            AlinkUsbDiag::decode(&AlinkUsbDiag::CLEAR.encode())
                .unwrap()
                .last_cause,
            None
        );
    }

    #[test]
    fn wp_ausb1_the_frame_counter_reads_16_and_20_bytes() {
        let c = FrameCounterTb {
            counter: FrameCounter {
                frame: 48 * 1100,
                time_us: 5,
                output_frames: 9,
            },
            time_base: TimeBase::UsbSample,
            bus_frame: 1100,
        };
        assert_eq!(FrameCounterTb::decode(&c.encode()), Some(c));
        let old = FrameCounterTb::decode(&c.encode()[..16]).unwrap();
        assert_eq!((old.time_base, old.bus_frame), (TimeBase::Alink, 0));
        assert_eq!(FrameCounterTb::decode(&c.encode()[..15]), None);
    }

    #[test]
    fn wp_ausb1_the_apply_example_on_the_bus() {
        // 2.9.4: MASTER_VOLUME -20 dB at bus frame 1100, sample 0.
        let req = ApplyRequest {
            frame: bus_target(1100, 0, 48).unwrap(),
            op: ApplyOp::MasterVolume,
            arg: 0,
            value: -2000,
        };
        let b = encode_apply(&req, TimeBase::BusFrame);
        assert_eq!(
            b,
            [
                0x4c, 0x04, 0, 0, 0, 0, 0, 0, 0x04, 0, 0x30, 0xf8, 0x02, 0, 0, 0
            ]
        );
        assert_eq!(decode_apply(&b), Some((req, TimeBase::BusFrame)));
        assert_eq!(decode_bus_target(req.frame), Some((1100, 0)));
        assert_eq!(
            decode_bus_target(1 << 12),
            None,
            "bit 12 is not in the form"
        );
        assert_eq!(bus_target(2048, 0, 48), None);
        assert_eq!(bus_target(1, 48, 48), None);
        assert_eq!(bus_of_sample(48 * (2048 + 1100) + 7, 48), Some((1100, 7)));
    }

    #[test]
    fn wp_ausb1_the_event_example_and_its_bit() {
        // 2.9.5: `02 e3 00 07 05 02 01 00 00 b4 c4 04`.
        let packet = [0x02, 0xe3, 0, 7, 5, 2, 1, 0, 0x00, 0xb4, 0xc4, 0x04];
        let payload = &packet[4..];
        let e = alink_usb_event(Some(NOTIFY_FEAT_ALINK_USB), 0xE3, payload).unwrap();
        assert_eq!(
            (e.state, e.phase_state, e.reason, e.sample_low),
            (
                UsbState::PhaseLocked,
                2,
                UsbEventReason::Locked,
                0x04c4_b400
            )
        );
        assert_eq!(e.encode(), payload);
        assert!(
            alink_usb_event(Some(0), 0xE3, payload).is_none(),
            "bit 4 off"
        );
        assert!(alink_usb_event(None, 0xE3, payload).is_none());
        assert!(alink_usb_event(Some(NOTIFY_FEAT_ALINK_USB), 0xE1, payload).is_none());
    }

    #[test]
    fn wp_ausb1_names() {
        assert_eq!(alink_usb_name(0x8011), Some("audio link USB-direct status"));
        assert_eq!(alink_usb_name(0x8013), None);
        assert_eq!(UsbMode::from_name("rate_lock"), Some(UsbMode::RateLock));
        assert_eq!(UsbState::from_raw(6).as_str(), "rejected");
        assert_eq!(UsbEventReason::from_raw(9).as_str(), "resumed");
        assert_eq!(end_cause_name(9), None);
    }
}
