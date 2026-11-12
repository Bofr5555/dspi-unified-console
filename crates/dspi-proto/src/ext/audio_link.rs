//! WP SW1 (SYS2 sections 2, 5.3 and 7): the audio link's extension
//! sub-commands (0x8000-0x800A, the fork's private range) and its two
//! notifications (0xE1 `LINK_STATE`, 0xE2 `APPLY_RESULT`).
//!
//! The layouts are the fork's `extension_channel_spec.md` section 2.8 (WP
//! LK7, DSPi-bofr c3909e5), which supersedes SYS2 5.3 where they differ:
//! a 52-byte status, a 24-byte `EXT_LINK_CTRL_MSG` IN, `d_local_min/max` in
//! the info, the event reasons, apply op 0 as `CANCEL_ALL`. The firmware
//! (LK5, LK6) has no header for them yet, so they are transcribed rather than
//! generated: re-pin them to the vendored headers when it does.
//!
//! Pure codecs; the I/O is in `dspi_session::ext`. Every decoder takes device
//! bytes and never panics: a short reply is `None`, extra bytes are ignored.
//! A host reads none of these unless `EXT_LIST` names them, and decodes the
//! events only behind their `EXT_NOTIFY_FEATURES` bits, so stock firmware and
//! the fork without the link see no extra traffic.

/// `EXT_LINK_INFO` IN, 32 B: role, capabilities, pins.
pub const EXT_LINK_INFO: u16 = 0x8000;
/// `EXT_LINK_STATUS` IN: lock state, frame counter, error counters, servo.
pub const EXT_LINK_STATUS: u16 = 0x8001;
/// `EXT_LINK_CONFIG` IN/OUT, 8 B: enable, pins, class, flags.
pub const EXT_LINK_CONFIG: u16 = 0x8002;
/// `EXT_LINK_SLOTS` IN/OUT, 8 B: the link slot for each matrix input.
pub const EXT_LINK_SLOTS: u16 = 0x8003;
/// `EXT_LINK_PHASE` IN/OUT, 8 B: phase-lock mode and the fixed delay.
pub const EXT_LINK_PHASE: u16 = 0x8004;
/// `EXT_FRAME_COUNTER` IN, 16 B: the link frame and output frame at the read.
pub const EXT_FRAME_COUNTER: u16 = 0x8005;
/// `EXT_APPLY_AT_FRAME` OUT 16 B (queue an op) / IN 16 B (the queue's state).
pub const EXT_APPLY_AT_FRAME: u16 = 0x8006;
/// `EXT_LINK_TX` IN/OUT, 16 B: a sender's links.
pub const EXT_LINK_TX: u16 = 0x8007;
/// `EXT_LINK_CTRL_MSG` OUT 16 B: a sender injects a superframe apply.
pub const EXT_LINK_CTRL_MSG: u16 = 0x8008;
/// `EXT_LINK_STATS_RESET` OUT, 1 B: clear the error counters.
pub const EXT_LINK_STATS_RESET: u16 = 0x8009;
/// `EXT_LINK_DIAG` IN, 64 B: bench diagnostics (not decoded here).
pub const EXT_LINK_DIAG: u16 = 0x800A;

/// The audio link's sub-commands, ascending.
pub const LINK_IDS: [u16; 11] = [
    EXT_LINK_INFO,
    EXT_LINK_STATUS,
    EXT_LINK_CONFIG,
    EXT_LINK_SLOTS,
    EXT_LINK_PHASE,
    EXT_FRAME_COUNTER,
    EXT_APPLY_AT_FRAME,
    EXT_LINK_TX,
    EXT_LINK_CTRL_MSG,
    EXT_LINK_STATS_RESET,
    EXT_LINK_DIAG,
];

/// `EXT_LINK_DIAG`'s reply size. Its layout is versioned by byte 0 and not
/// for apps to parse (2.8.11).
pub const LINK_DIAG_SIZE: usize = 64;

/// The frame and superframe version this build speaks (`EXT_LINK_INFO`
/// byte 1). A unit reporting another is treated as having no link (2.8.13).
pub const LINK_PROTOCOL_VERSION: u8 = 1;

/// The receiver's sub-commands (2.8, "Per build"): what a receiver lists.
pub const RECEIVER_IDS: [u16; 10] = [
    EXT_LINK_INFO,
    EXT_LINK_STATUS,
    EXT_LINK_CONFIG,
    EXT_LINK_SLOTS,
    EXT_LINK_PHASE,
    EXT_FRAME_COUNTER,
    EXT_APPLY_AT_FRAME,
    EXT_LINK_CTRL_MSG,
    EXT_LINK_STATS_RESET,
    EXT_LINK_DIAG,
];

/// What a link sub-command is called, for listings; `None` outside the range.
pub fn link_name(id: u16) -> Option<&'static str> {
    Some(match id {
        EXT_LINK_INFO => "link info",
        EXT_LINK_STATUS => "link status",
        EXT_LINK_CONFIG => "link config",
        EXT_LINK_SLOTS => "link slots",
        EXT_LINK_PHASE => "link phase",
        EXT_FRAME_COUNTER => "frame counter",
        EXT_APPLY_AT_FRAME => "apply at frame",
        EXT_LINK_TX => "link transmit",
        EXT_LINK_CTRL_MSG => "link control message",
        EXT_LINK_STATS_RESET => "link stats reset",
        EXT_LINK_DIAG => "link diagnostics",
        _ => return None,
    })
}

/// `INPUT_SOURCE_LINK`: `REQ_SET_INPUT_SOURCE` with 7 selects the link input
/// (SYS2 5.1). Stock firmware refuses 7.
pub const INPUT_SOURCE_LINK: u8 = 7;

/// `EXT_NOTIFY_FEATURES` bit 2: the device sends [`NOTIFY_EVT_LINK_STATE`].
pub const NOTIFY_FEAT_LINK_STATE: u32 = 1 << 2;
/// `EXT_NOTIFY_FEATURES` bit 3: the device sends [`NOTIFY_EVT_APPLY_RESULT`].
pub const NOTIFY_FEAT_APPLY_RESULT: u32 = 1 << 3;
/// `NOTIFY_EVT_LINK_STATE`: lock, loss, re-anchor.
pub const NOTIFY_EVT_LINK_STATE: u8 = 0xE1;
/// `NOTIFY_EVT_APPLY_RESULT`: an apply-at-frame op completed.
pub const NOTIFY_EVT_APPLY_RESULT: u8 = 0xE2;
/// The firmware's rate limit on the link-state event, as for the clip event
/// (the apply result is not limited).
pub const LINK_STATE_MIN_INTERVAL_MS: u16 = 250;

/// Matrix inputs a unit takes from the link (`EXT_LINK_SLOTS`).
pub const LINK_SLOTS: usize = 8;
/// A slot byte that feeds silence to its matrix input.
pub const LINK_SLOT_SILENT: u8 = 0xFF;
/// The most channels a link frame carries (superframe byte 4: 1..64). A
/// slot is below this or silent.
pub const LINK_MAX_CHANNELS: u8 = 64;
/// `hops_offset`'s range in `EXT_LINK_PHASE`.
pub const HOPS_OFFSET_MAX: i16 = 64;

/// `EXT_LINK_INFO.features` bits (2.8.1).
pub mod feature {
    /// `EXT_LINK_PHASE` mode 1.
    pub const PHASE_LOCK: u32 = 1 << 0;
    pub const APPLY_AT_FRAME: u32 = 1 << 1;
    pub const REPEATER: u32 = 1 << 2;
    pub const FAN_OUT: u32 = 1 << 3;
    pub const TEST_PATTERNS: u32 = 1 << 4;
    pub const LOS_PIN: u32 = 1 << 5;
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    b.get(i..i.checked_add(2)?)?
        .try_into()
        .ok()
        .map(u16::from_le_bytes)
}

fn i16_at(b: &[u8], i: usize) -> Option<i16> {
    u16_at(b, i).map(|v| i16::from_le_bytes(v.to_le_bytes()))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    b.get(i..i.checked_add(4)?)?
        .try_into()
        .ok()
        .map(u32::from_le_bytes)
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

// ------------------------------------------------------------ enums

/// What the unit is on the link (`EXT_LINK_INFO` byte 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkRole {
    None,
    Receiver,
    Sender,
    Repeater,
    FanOut,
    /// A value this build does not know.
    Other(u8),
}

impl LinkRole {
    pub fn from_raw(b: u8) -> Self {
        match b {
            0 => Self::None,
            1 => Self::Receiver,
            2 => Self::Sender,
            3 => Self::Repeater,
            4 => Self::FanOut,
            o => Self::Other(o),
        }
    }

    pub fn raw(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Receiver => 1,
            Self::Sender => 2,
            Self::Repeater => 3,
            Self::FanOut => 4,
            Self::Other(o) => o,
        }
    }

    /// A stable lower-case name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Receiver => "receiver",
            Self::Sender => "sender",
            Self::Repeater => "repeater",
            Self::FanOut => "fan_out",
            Self::Other(_) => "unknown",
        }
    }

    /// Whether the unit takes audio from a link (and so offers the link
    /// input source).
    pub fn receives(self) -> bool {
        matches!(self, Self::Receiver | Self::Repeater)
    }
}

/// The receiver's lock state (`EXT_LINK_STATUS` byte 0, event 0xE1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    Off,
    Searching,
    Locked,
    /// The link was lost: outputs muted, VCXO trim held.
    Holdover,
    Other(u8),
}

impl LinkState {
    pub fn from_raw(b: u8) -> Self {
        match b {
            0 => Self::Off,
            1 => Self::Searching,
            2 => Self::Locked,
            3 => Self::Holdover,
            o => Self::Other(o),
        }
    }

    pub fn raw(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Searching => 1,
            Self::Locked => 2,
            Self::Holdover => 3,
            Self::Other(o) => o,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Searching => "searching",
            Self::Locked => "locked",
            Self::Holdover => "holdover",
            Self::Other(_) => "unknown",
        }
    }
}

/// Why a link-state event was sent (event 0xE1 byte 6, 2.8.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkEventReason {
    None,
    /// Anchored and playing.
    Locked,
    /// The LOS pin.
    Los,
    /// Three consecutive bad headers.
    HeaderErrors,
    CommaLost,
    /// Phase error beyond half a frame after a loss, or a new
    /// `EXT_LINK_PHASE`: a short silence.
    Reanchor,
    /// VCXO authority exceeded.
    VcxoLimit,
    /// `enable` cleared or the input source changed.
    Disabled,
    Other(u8),
}

impl LinkEventReason {
    pub fn from_raw(b: u8) -> Self {
        match b {
            0 => Self::None,
            1 => Self::Locked,
            2 => Self::Los,
            3 => Self::HeaderErrors,
            4 => Self::CommaLost,
            5 => Self::Reanchor,
            6 => Self::VcxoLimit,
            7 => Self::Disabled,
            o => Self::Other(o),
        }
    }

    pub fn raw(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Locked => 1,
            Self::Los => 2,
            Self::HeaderErrors => 3,
            Self::CommaLost => 4,
            Self::Reanchor => 5,
            Self::VcxoLimit => 6,
            Self::Disabled => 7,
            Self::Other(o) => o,
        }
    }

    /// A stable lower-case name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Locked => "locked",
            Self::Los => "los",
            Self::HeaderErrors => "header_errors",
            Self::CommaLost => "comma_lost",
            Self::Reanchor => "reanchor",
            Self::VcxoLimit => "vcxo_limit",
            Self::Disabled => "disabled",
            Self::Other(_) => "unknown",
        }
    }

    /// The link went away: LOS, bad headers, a lost comma.
    pub fn is_loss(self) -> bool {
        matches!(self, Self::Los | Self::HeaderErrors | Self::CommaLost)
    }
}

/// An apply-at-frame operation (2.8.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOp {
    /// `EXT_APPLY_AT_FRAME`: cancel every pending op (`frame` ignored).
    /// `EXT_LINK_CTRL_MSG`: no apply, clear a pending injection.
    CancelAll,
    Mute,
    Unmute,
    /// `arg` = preset slot.
    PresetLoad,
    /// `value` = master volume.
    MasterVolume,
    /// `arg` = output, `value` = samples.
    OutDelay,
    /// `arg` = rate code.
    Rate,
    /// A one-sample full-scale impulse on output `arg`, for the scope.
    Click,
    /// Toggle the unit's sync GPIO.
    SyncPulse,
    Other(u8),
}

impl ApplyOp {
    pub fn from_raw(b: u8) -> Self {
        match b {
            0 => Self::CancelAll,
            1 => Self::Mute,
            2 => Self::Unmute,
            3 => Self::PresetLoad,
            4 => Self::MasterVolume,
            5 => Self::OutDelay,
            6 => Self::Rate,
            7 => Self::Click,
            8 => Self::SyncPulse,
            o => Self::Other(o),
        }
    }

    pub fn raw(self) -> u8 {
        match self {
            Self::CancelAll => 0,
            Self::Mute => 1,
            Self::Unmute => 2,
            Self::PresetLoad => 3,
            Self::MasterVolume => 4,
            Self::OutDelay => 5,
            Self::Rate => 6,
            Self::Click => 7,
            Self::SyncPulse => 8,
            Self::Other(o) => o,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::CancelAll => "cancel_all",
            Self::Mute => "mute",
            Self::Unmute => "unmute",
            Self::PresetLoad => "preset_load",
            Self::MasterVolume => "master_volume",
            Self::OutDelay => "out_delay",
            Self::Rate => "rate",
            Self::Click => "click",
            Self::SyncPulse => "sync_pulse",
            Self::Other(_) => "unknown",
        }
    }

    /// The highest op this build knows.
    pub const MAX: u8 = 8;
}

/// How an apply-at-frame op ended (`EXT_APPLY_AT_FRAME` IN, event 0xE2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyResult {
    Applied,
    /// Its frame had passed: applied at once.
    Late,
    Cancelled,
    /// Refused at apply time (an empty preset slot).
    Refused,
    Other(u8),
}

impl ApplyResult {
    pub fn from_raw(b: u8) -> Self {
        match b {
            0 => Self::Applied,
            1 => Self::Late,
            2 => Self::Cancelled,
            3 => Self::Refused,
            o => Self::Other(o),
        }
    }

    pub fn raw(self) -> u8 {
        match self {
            Self::Applied => 0,
            Self::Late => 1,
            Self::Cancelled => 2,
            Self::Refused => 3,
            Self::Other(o) => o,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Late => "late",
            Self::Cancelled => "cancelled",
            Self::Refused => "refused",
            Self::Other(_) => "unknown",
        }
    }
}

// ------------------------------------------------------------ records

/// `EXT_LINK_INFO` IN, 32 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkInfo {
    pub role: LinkRole,
    pub protocol_version: u8,
    /// b0 L1, b1 L2, b2 L3.
    pub class_mask: u8,
    /// A sender's link outputs; 1 on a receiver.
    pub max_links: u8,
    pub max_channels: u8,
    /// Matrix inputs a unit takes (8).
    pub slots_per_unit: u8,
    pub flags: u16,
    pub rx_pin: u8,
    pub tx_pin: u8,
    /// [`feature`] bits.
    pub feature_bits: u32,
    /// The smallest effective `D_local` the unit accepts under its current
    /// output latency option; re-read after changing the option.
    pub d_local_min: u16,
    /// The largest it accepts.
    pub d_local_max: u16,
}

impl LinkInfo {
    pub const SIZE: usize = 32;

    /// Whether the unit has a link this build can use: a role, and the
    /// protocol version it speaks (2.8.13).
    pub fn usable(&self) -> bool {
        self.role != LinkRole::None && self.protocol_version == LINK_PROTOCOL_VERSION
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        Some(Self {
            role: LinkRole::from_raw(byte(b, 0)?),
            protocol_version: byte(b, 1)?,
            class_mask: byte(b, 2)?,
            max_links: byte(b, 3)?,
            max_channels: byte(b, 4)?,
            slots_per_unit: byte(b, 5)?,
            flags: u16_at(b, 6)?,
            rx_pin: byte(b, 8)?,
            tx_pin: byte(b, 9)?,
            feature_bits: u32_at(b, 12)?,
            d_local_min: u16_at(b, 16)?,
            d_local_max: u16_at(b, 18)?,
        })
    }

    /// The wire form (the fake device's reply).
    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0] = self.role.raw();
        b[1] = self.protocol_version;
        b[2] = self.class_mask;
        b[3] = self.max_links;
        b[4] = self.max_channels;
        b[5] = self.slots_per_unit;
        b[6..8].copy_from_slice(&self.flags.to_le_bytes());
        b[8] = self.rx_pin;
        b[9] = self.tx_pin;
        b[12..16].copy_from_slice(&self.feature_bits.to_le_bytes());
        b[16..18].copy_from_slice(&self.d_local_min.to_le_bytes());
        b[18..20].copy_from_slice(&self.d_local_max.to_le_bytes());
        b
    }
}

/// `EXT_LINK_STATUS` IN, 52 bytes (2.8.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkStatus {
    pub state: LinkState,
    /// 0 unknown, 1-3 = L1-L3.
    pub class: u8,
    /// 0 = 44.1, 1 = 48, 2 = 96 kHz, 0xFF unknown.
    pub rate_code: u8,
    pub channels: u8,
    pub hops: u8,
    /// The LOS pin: 0 clear, 1 asserted, 0xFF none configured.
    pub los_pin: u8,
    pub sender_id: u8,
    /// Superframe flags b2-b5.
    pub flags: u8,
    /// The link frame counter F.
    pub frame: u64,
    pub lock_count: u32,
    pub loss_count: u32,
    pub crc8_errors: u32,
    pub crc16_errors: u32,
    pub bit_slips: u32,
    pub superframe_errors: u32,
    pub vcxo_ppm: f32,
    pub phase_error_frames: f32,
    pub fill_frames: u16,
}

impl LinkStatus {
    pub const SIZE: usize = 52;

    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        let f32_at = |i| u32_at(b, i).map(f32::from_bits);
        Some(Self {
            state: LinkState::from_raw(byte(b, 0)?),
            class: byte(b, 1)?,
            rate_code: byte(b, 2)?,
            channels: byte(b, 3)?,
            hops: byte(b, 4)?,
            los_pin: byte(b, 5)?,
            sender_id: byte(b, 6)?,
            flags: byte(b, 7)?,
            frame: u64_at(b, 8)?,
            lock_count: u32_at(b, 16)?,
            loss_count: u32_at(b, 20)?,
            crc8_errors: u32_at(b, 24)?,
            crc16_errors: u32_at(b, 28)?,
            bit_slips: u32_at(b, 32)?,
            superframe_errors: u32_at(b, 36)?,
            vcxo_ppm: f32_at(40)?,
            phase_error_frames: f32_at(44)?,
            fill_frames: u16_at(b, 48)?,
        })
    }

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[..8].copy_from_slice(&[
            self.state.raw(),
            self.class,
            self.rate_code,
            self.channels,
            self.hops,
            self.los_pin,
            self.sender_id,
            self.flags,
        ]);
        b[8..16].copy_from_slice(&self.frame.to_le_bytes());
        for (i, v) in [
            self.lock_count,
            self.loss_count,
            self.crc8_errors,
            self.crc16_errors,
            self.bit_slips,
            self.superframe_errors,
            self.vcxo_ppm.to_bits(),
            self.phase_error_frames.to_bits(),
        ]
        .into_iter()
        .enumerate()
        {
            b[16 + 4 * i..20 + 4 * i].copy_from_slice(&v.to_le_bytes());
        }
        b[48..50].copy_from_slice(&self.fill_frames.to_le_bytes());
        b
    }
}

/// `EXT_LINK_CONFIG` IN/OUT, 8 bytes. 0xFF in a pin field is "default" (rx)
/// or "none" (the others).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkConfig {
    pub enable: bool,
    pub rx_pin: u8,
    /// 0 auto, 1-3.
    pub class: u8,
    pub tx_pin: u8,
    pub los_pin: u8,
    pub txdis_pin: u8,
    /// b0 mute on loss, b1 auto re-anchor.
    pub flags: u8,
}

impl LinkConfig {
    pub const SIZE: usize = 8;

    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        Some(Self {
            enable: byte(b, 0)? != 0,
            rx_pin: byte(b, 1)?,
            class: byte(b, 2)?,
            tx_pin: byte(b, 3)?,
            los_pin: byte(b, 4)?,
            txdis_pin: byte(b, 5)?,
            flags: byte(b, 6)?,
        })
    }

    pub fn encode(&self) -> [u8; Self::SIZE] {
        [
            u8::from(self.enable),
            self.rx_pin,
            self.class,
            self.tx_pin,
            self.los_pin,
            self.txdis_pin,
            self.flags,
            0,
        ]
    }
}

/// `EXT_LINK_SLOTS` IN/OUT: the link slot for each matrix input, or
/// [`LINK_SLOT_SILENT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkSlots(pub [u8; LINK_SLOTS]);

impl LinkSlots {
    pub const SIZE: usize = LINK_SLOTS;
    /// Every input silent.
    pub const SILENT: LinkSlots = LinkSlots([LINK_SLOT_SILENT; LINK_SLOTS]);

    pub fn decode(b: &[u8]) -> Option<Self> {
        b.get(..Self::SIZE)?.try_into().ok().map(LinkSlots)
    }

    /// The default: slot `i` into input `i`.
    pub const IDENTITY: LinkSlots = LinkSlots([0, 1, 2, 3, 4, 5, 6, 7]);

    /// Each slot below [`LINK_MAX_CHANNELS`] or silent; 64-0xFE are refused.
    /// A slot at or above the stream's channels is accepted and plays
    /// silence until the stream grows (2.8.4).
    pub fn is_valid(&self) -> bool {
        self.0
            .iter()
            .all(|s| *s == LINK_SLOT_SILENT || *s < LINK_MAX_CHANNELS)
    }
}

/// `EXT_LINK_PHASE` mode: today's fill servo.
pub const PHASE_MODE_FILL: u8 = 0;
/// `EXT_LINK_PHASE` mode: phase lock to the frame counter.
pub const PHASE_MODE_LOCK: u8 = 1;

/// `EXT_LINK_PHASE` IN/OUT, 8 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkPhase {
    /// [`PHASE_MODE_FILL`] or [`PHASE_MODE_LOCK`].
    pub mode: u8,
    /// The unit's fixed delay in frames (D - hops).
    pub d_local: u16,
    /// Normally 0: `d_local` already includes the hops.
    pub hops_offset: i16,
    /// IN: bit 0 `D_BELOW_MIN`. OUT: must be 0.
    pub flags: u8,
    /// IN only: 0 free, 1 anchoring, 2 locked, 3 re-anchor pending.
    pub state: u8,
}

/// `EXT_LINK_PHASE.flags` bit 0: the effective delay is below `d_local_min`;
/// the unit runs as the fill servo until a valid value is written.
pub const PHASE_FLAG_D_BELOW_MIN: u8 = 1 << 0;

impl LinkPhase {
    pub const SIZE: usize = 8;

    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        Some(Self {
            mode: byte(b, 0)?,
            d_local: u16_at(b, 2)?,
            hops_offset: i16_at(b, 4)?,
            flags: byte(b, 6)?,
            state: byte(b, 7)?,
        })
    }

    /// The effective delay, `d_local + hops_offset`.
    pub fn effective(&self) -> i32 {
        i32::from(self.d_local).saturating_add(i32::from(self.hops_offset))
    }

    /// Whether a write would be accepted by a unit reporting `info` (2.8.5):
    /// mode 0 or 1, `hops_offset` within +-64, the effective delay within
    /// [`LinkInfo::d_local_min`, `d_local_max`], no OUT flags.
    pub fn is_valid_for(&self, info: &LinkInfo) -> bool {
        self.mode <= PHASE_MODE_LOCK
            && (-HOPS_OFFSET_MAX..=HOPS_OFFSET_MAX).contains(&self.hops_offset)
            && self.flags == 0
            && (i32::from(info.d_local_min)..=i32::from(info.d_local_max))
                .contains(&self.effective())
    }

    /// The wire form; on OUT the state byte is ignored by the firmware.
    pub fn encode(&self) -> [u8; Self::SIZE] {
        let [d0, d1] = self.d_local.to_le_bytes();
        let [h0, h1] = self.hops_offset.to_le_bytes();
        [self.mode, 0, d0, d1, h0, h1, self.flags, self.state]
    }
}

/// `EXT_FRAME_COUNTER` IN, 16 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameCounter {
    /// The link frame F at the read.
    pub frame: u64,
    /// The unit's time at the read, µs (wraps).
    pub time_us: u32,
    /// Output frames played, low 32 bits.
    pub output_frames: u32,
}

impl FrameCounter {
    pub const SIZE: usize = 16;

    pub fn decode(b: &[u8]) -> Option<Self> {
        Some(Self {
            frame: u64_at(b, 0)?,
            time_us: u32_at(b, 8)?,
            output_frames: u32_at(b, 12)?,
        })
    }

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[..8].copy_from_slice(&self.frame.to_le_bytes());
        b[8..12].copy_from_slice(&self.time_us.to_le_bytes());
        b[12..].copy_from_slice(&self.output_frames.to_le_bytes());
        b
    }
}

/// `EXT_APPLY_AT_FRAME` OUT (and the sender's `EXT_LINK_CTRL_MSG` apply):
/// run `op` when the link frame reaches `frame`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyRequest {
    pub frame: u64,
    pub op: ApplyOp,
    pub arg: u8,
    pub value: i16,
}

impl ApplyRequest {
    pub const SIZE: usize = 16;

    /// `[frame u64, op, arg, value i16, 0 x 4]`.
    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[..8].copy_from_slice(&self.frame.to_le_bytes());
        b[8] = self.op.raw();
        b[9] = self.arg;
        b[10..12].copy_from_slice(&self.value.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        Some(Self {
            frame: u64_at(b, 0)?,
            op: ApplyOp::from_raw(byte(b, 8)?),
            arg: byte(b, 9)?,
            value: i16_at(b, 10)?,
        })
    }

    /// Whether `arg` and `value` are in the op's range (2.8.7; unused ones
    /// 0). `OUT_DELAY`'s sample limit is the device's (2047 on an RP2350,
    /// 1023 on an RP2040) and is checked only for sign here.
    pub fn args_valid(&self) -> bool {
        let (a, v) = (self.arg, self.value);
        match self.op {
            ApplyOp::CancelAll | ApplyOp::Mute | ApplyOp::Unmute | ApplyOp::SyncPulse => {
                a == 0 && v == 0
            }
            ApplyOp::PresetLoad => a <= 9 && v == 0,
            ApplyOp::MasterVolume => a == 0 && (-12800..=0).contains(&v),
            ApplyOp::OutDelay => v >= 0,
            ApplyOp::Rate => a <= 2 && v == 0,
            ApplyOp::Click => v == 0,
            ApplyOp::Other(_) => false,
        }
    }

    /// `EXT_LINK_CTRL_MSG` OUT on a sender: the apply plus the superframe's
    /// system flags.
    pub fn encode_ctrl_msg(&self, system_flags: u8) -> [u8; Self::SIZE] {
        let mut b = self.encode();
        b[12] = system_flags;
        b
    }
}

/// `EXT_APPLY_AT_FRAME` IN, 16 bytes: the unit's apply queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyStatus {
    /// Ops queued (at most 4).
    pub pending: u8,
    pub last_op: ApplyOp,
    pub last_result: ApplyResult,
    pub last_frame: u64,
}

impl ApplyStatus {
    pub const SIZE: usize = 16;

    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        Some(Self {
            pending: byte(b, 0)?,
            last_op: ApplyOp::from_raw(byte(b, 1)?),
            last_result: ApplyResult::from_raw(byte(b, 2)?),
            last_frame: u64_at(b, 4)?,
        })
    }

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0] = self.pending;
        b[1] = self.last_op.raw();
        b[2] = self.last_result.raw();
        b[4..12].copy_from_slice(&self.last_frame.to_le_bytes());
        b
    }
}

/// `EXT_LINK_CTRL_MSG` IN, 24 bytes: the last valid superframe received
/// (receiver) or sent (sender), 2.8.9. All zero before the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuperframeSnapshot {
    pub flags: u8,
    pub rate_code: u8,
    pub channels: u8,
    pub class: u8,
    pub hops: u8,
    pub sender_id: u8,
    /// The pending apply it carried (frame 0: none).
    pub apply: ApplyRequest,
    /// The superframe's first frame, low 32 bits.
    pub frame_low: u32,
}

impl SuperframeSnapshot {
    pub const SIZE: usize = 24;

    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        Some(Self {
            flags: byte(b, 0)?,
            rate_code: byte(b, 1)?,
            channels: byte(b, 2)?,
            class: byte(b, 3)?,
            hops: byte(b, 4)?,
            sender_id: byte(b, 5)?,
            apply: ApplyRequest::decode(b.get(8..20)?)?,
            frame_low: u32_at(b, 20)?,
        })
    }

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[..6].copy_from_slice(&[
            self.flags,
            self.rate_code,
            self.channels,
            self.class,
            self.hops,
            self.sender_id,
        ]);
        b[8..20].copy_from_slice(&self.apply.encode()[..12]);
        b[20..].copy_from_slice(&self.frame_low.to_le_bytes());
        b
    }
}

/// `EXT_LINK_TX` IN/OUT, 16 bytes: a sender's link outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkTx {
    pub link_mask: u16,
    pub class: u8,
    pub channels: u8,
    pub rate_code: u8,
    /// 0 audio, 1 PRBS-7, 2 idle, 3 tone.
    pub test_pattern: u8,
    pub slot_map_id: u8,
    pub sync_pin: u8,
    pub txdis_mask: u16,
    /// IN only.
    pub los_mask: u16,
    /// IN only.
    pub usb_alt: u8,
    pub flags: u8,
}

impl LinkTx {
    pub const SIZE: usize = 16;

    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        Some(Self {
            link_mask: u16_at(b, 0)?,
            class: byte(b, 2)?,
            channels: byte(b, 3)?,
            rate_code: byte(b, 4)?,
            test_pattern: byte(b, 5)?,
            slot_map_id: byte(b, 6)?,
            sync_pin: byte(b, 7)?,
            txdis_mask: u16_at(b, 8)?,
            los_mask: u16_at(b, 10)?,
            usb_alt: byte(b, 12)?,
            flags: byte(b, 13)?,
        })
    }

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[..2].copy_from_slice(&self.link_mask.to_le_bytes());
        b[2..8].copy_from_slice(&[
            self.class,
            self.channels,
            self.rate_code,
            self.test_pattern,
            self.slot_map_id,
            self.sync_pin,
        ]);
        b[8..10].copy_from_slice(&self.txdis_mask.to_le_bytes());
        b[10..12].copy_from_slice(&self.los_mask.to_le_bytes());
        b[12] = self.usb_alt;
        b[13] = self.flags;
        b
    }
}

/// `EXT_LINK_STATS_RESET`'s payload: clear the counters.
pub const LINK_STATS_RESET: [u8; 1] = [1];

// ------------------------------------------------------------ events

/// Event 0xE1: `[state, class, reason, 0, F_low32]` from byte 4 of the
/// packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkStateEvent {
    pub state: LinkState,
    pub class: u8,
    pub reason: LinkEventReason,
    /// The link frame's low 32 bits when it was sent.
    pub frame_low: u32,
}

impl LinkStateEvent {
    /// The payload size from byte 4.
    pub const PAYLOAD: usize = 8;

    pub fn decode(payload: &[u8]) -> Option<Self> {
        Some(Self {
            state: LinkState::from_raw(byte(payload, 0)?),
            class: byte(payload, 1)?,
            reason: LinkEventReason::from_raw(byte(payload, 2)?),
            frame_low: u32_at(payload, 4)?,
        })
    }

    pub fn encode(&self) -> [u8; Self::PAYLOAD] {
        let mut b = [0u8; Self::PAYLOAD];
        b[0] = self.state.raw();
        b[1] = self.class;
        b[2] = self.reason.raw();
        b[4..].copy_from_slice(&self.frame_low.to_le_bytes());
        b
    }

    /// Whether the event says the audio stopped: a loss reason, or any
    /// state but locked.
    pub fn is_loss(&self) -> bool {
        self.reason.is_loss() || self.state != LinkState::Locked
    }
}

/// Event 0xE2: `[op, result, 0, 0, F_low32]` from byte 4 of the packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyResultEvent {
    pub op: ApplyOp,
    pub result: ApplyResult,
    /// The op's target frame, low 32 bits, to match it to its request.
    pub frame_low: u32,
}

impl ApplyResultEvent {
    pub const PAYLOAD: usize = 8;

    pub fn decode(payload: &[u8]) -> Option<Self> {
        Some(Self {
            op: ApplyOp::from_raw(byte(payload, 0)?),
            result: ApplyResult::from_raw(byte(payload, 1)?),
            frame_low: u32_at(payload, 4)?,
        })
    }

    pub fn encode(&self) -> [u8; Self::PAYLOAD] {
        let mut b = [0u8; Self::PAYLOAD];
        b[0] = self.op.raw();
        b[1] = self.result.raw();
        b[4..].copy_from_slice(&self.frame_low.to_le_bytes());
        b
    }
}

/// One of the link's notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkEvent {
    State(LinkStateEvent),
    Apply(ApplyResultEvent),
}

/// The link's events behind their feature bits, as [`super::clip_event`]:
/// `notify_features` is the device's `EXT_NOTIFY_FEATURES` (`None` when it
/// lacks or refused it), `id` and `payload` an `Event::Unknown`'s (bytes 4
/// on). Only a device announcing bit 2 has its 0xE1 read, only bit 3 its
/// 0xE2.
pub fn link_event(notify_features: Option<u32>, id: u8, payload: &[u8]) -> Option<LinkEvent> {
    let f = notify_features?;
    match id {
        NOTIFY_EVT_LINK_STATE if f & NOTIFY_FEAT_LINK_STATE != 0 => {
            LinkStateEvent::decode(payload).map(LinkEvent::State)
        }
        NOTIFY_EVT_APPLY_RESULT if f & NOTIFY_FEAT_APPLY_RESULT != 0 => {
            ApplyResultEvent::decode(payload).map(LinkEvent::Apply)
        }
        _ => None,
    }
}

/// WP SW1: the codecs against `extension_channel_spec.md` 2.8 (LK7), its
/// example bytes included.
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_sw1 {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn wp_sw1_ids_and_bits_are_the_specs() {
        assert_eq!(LINK_IDS.first(), Some(&0x8000));
        assert_eq!(LINK_IDS.last(), Some(&0x800A));
        assert!(LINK_IDS.windows(2).all(|w| w[1] == w[0] + 1));
        for id in LINK_IDS {
            assert!(link_name(id).is_some());
            assert_eq!(super::super::name(id), link_name(id).unwrap());
        }
        // A receiver lists all but the sender's EXT_LINK_TX.
        assert!(!RECEIVER_IDS.contains(&EXT_LINK_TX));
        assert_eq!(RECEIVER_IDS.len(), LINK_IDS.len() - 1);
        assert_eq!(link_name(0x800B), None);
        assert_eq!(super::super::name(0x800B), "experimental");
        assert_eq!(INPUT_SOURCE_LINK, 7);
        assert_eq!((NOTIFY_FEAT_LINK_STATE, NOTIFY_FEAT_APPLY_RESULT), (4, 8));
        assert_eq!(
            (NOTIFY_EVT_LINK_STATE, NOTIFY_EVT_APPLY_RESULT),
            (0xE1, 0xE2)
        );
        // The fork's clip event keeps its own bit and id.
        assert_eq!(super::super::NOTIFY_FEAT_CLIP_EVENT & (4 | 8), 0);
    }

    /// 2.8.1's example: the fake device, a receiver on GP0, 1 ms latency.
    const INFO_EXAMPLE: [u8; 32] = [
        0x01, 0x01, 0x07, 0x01, 0x40, 0x08, 0x00, 0x00, 0x00, 0xff, 0x00, 0x00, 0x23, 0x00, 0x00,
        0x00, 0x70, 0x00, 0x00, 0x04, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];

    #[test]
    fn wp_sw1_info_example_round_trips() {
        let i = LinkInfo::decode(&INFO_EXAMPLE).unwrap();
        assert_eq!(i.role, LinkRole::Receiver);
        assert_eq!((i.protocol_version, i.class_mask, i.max_links), (1, 7, 1));
        assert_eq!((i.max_channels, i.slots_per_unit), (64, 8));
        assert_eq!((i.rx_pin, i.tx_pin), (0, 0xFF));
        assert_eq!(
            i.feature_bits,
            feature::PHASE_LOCK | feature::APPLY_AT_FRAME | feature::LOS_PIN
        );
        assert_eq!((i.d_local_min, i.d_local_max), (112, 1024));
        assert!(i.usable());
        assert_eq!(i.encode(), INFO_EXAMPLE);
        assert_eq!(LinkInfo::decode(&INFO_EXAMPLE[..31]), None);
        // Role 0 or another protocol version: no link.
        assert!(
            !LinkInfo {
                role: LinkRole::None,
                ..i
            }
            .usable()
        );
        assert!(
            !LinkInfo {
                protocol_version: 2,
                ..i
            }
            .usable()
        );
        assert!(LinkRole::Receiver.receives() && LinkRole::Repeater.receives());
        assert!(!LinkRole::Sender.receives());
        for raw in 0..=255u8 {
            assert_eq!(LinkRole::from_raw(raw).raw(), raw);
            assert_eq!(LinkState::from_raw(raw).raw(), raw);
            assert_eq!(LinkEventReason::from_raw(raw).raw(), raw);
            assert_eq!(ApplyOp::from_raw(raw).raw(), raw);
            assert_eq!(ApplyResult::from_raw(raw).raw(), raw);
        }
    }

    #[test]
    fn wp_sw1_status_layout() {
        let s = LinkStatus {
            state: LinkState::Locked,
            class: 1,
            rate_code: 1,
            channels: 8,
            hops: 0,
            los_pin: 0xFF,
            sender_id: 0x42,
            flags: 0b1000,
            frame: 0x0102_0304_0506_0708,
            lock_count: 1,
            loss_count: 2,
            crc8_errors: 3,
            crc16_errors: 4,
            bit_slips: 5,
            superframe_errors: 6,
            vcxo_ppm: -12.5,
            phase_error_frames: 0.25,
            fill_frames: 96,
        };
        let b = s.encode();
        assert_eq!(b.len(), 52);
        assert_eq!(b[0], 2);
        assert_eq!(&b[8..16], &[8, 7, 6, 5, 4, 3, 2, 1]);
        assert_eq!(&b[36..40], &[6, 0, 0, 0]);
        assert_eq!(f32::from_le_bytes(b[40..44].try_into().unwrap()), -12.5);
        assert_eq!(&b[48..52], &[96, 0, 0, 0]);
        assert_eq!(LinkStatus::decode(&b), Some(s));
        assert_eq!(LinkStatus::decode(&b[..51]), None);
    }

    #[test]
    fn wp_sw1_slots_and_phase() {
        assert!(LinkSlots([4, 5, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]).is_valid());
        // At or above C is fine; 64..0xFE is refused.
        assert!(LinkSlots([63; 8]).is_valid());
        assert!(!LinkSlots([64, 0, 0, 0, 0, 0, 0, 0]).is_valid());
        assert!(!LinkSlots([0xFE; 8]).is_valid());
        assert!(LinkSlots::SILENT.is_valid() && LinkSlots::IDENTITY.is_valid());
        assert_eq!(LinkSlots::decode(&LinkSlots::IDENTITY.0[..7]), None);

        // 2.8.5's example.
        let ex = [0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x02];
        let p = LinkPhase::decode(&ex).unwrap();
        assert_eq!(
            (p.mode, p.d_local, p.hops_offset, p.flags, p.state),
            (1, 128, 0, 0, 2)
        );
        assert_eq!(p.encode(), ex);
        let info = LinkInfo::decode(&INFO_EXAMPLE).unwrap();
        assert!(p.is_valid_for(&info));
        let bad = |q: LinkPhase| !q.is_valid_for(&info);
        assert!(bad(LinkPhase { mode: 2, ..p }));
        assert!(bad(LinkPhase { d_local: 111, ..p }));
        assert!(bad(LinkPhase { d_local: 1025, ..p }));
        assert!(bad(LinkPhase {
            hops_offset: 65,
            ..p
        }));
        assert!(bad(LinkPhase {
            flags: PHASE_FLAG_D_BELOW_MIN,
            ..p
        }));
        // The offset counts: 120 - 8 = 112 is in, 112 - 1 is not.
        assert!(
            LinkPhase {
                d_local: 120,
                hops_offset: -8,
                ..p
            }
            .is_valid_for(&info)
        );
        assert!(bad(LinkPhase {
            d_local: 112,
            hops_offset: -1,
            ..p
        }));
        assert_eq!(
            LinkPhase {
                hops_offset: -1,
                ..p
            }
            .encode()[4..6],
            [0xFF, 0xFF]
        );
    }

    #[test]
    fn wp_sw1_apply_example_and_ranges() {
        // 2.8.7: MASTER_VOLUME -20 dB at frame 1 002 400.
        let ex = [
            0xa0, 0x4b, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x30, 0xf8, 0x00, 0x00,
            0x00, 0x00,
        ];
        let a = ApplyRequest::decode(&ex).unwrap();
        assert_eq!(
            a,
            ApplyRequest {
                frame: 1_002_400,
                op: ApplyOp::MasterVolume,
                arg: 0,
                value: -2000
            }
        );
        assert_eq!(a.encode(), ex);
        assert!(a.args_valid());
        let st = [
            0x00, 0x04, 0x00, 0x00, 0xa0, 0x4b, 0x0f, 0x00, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let s = ApplyStatus::decode(&st).unwrap();
        assert_eq!(
            (s.pending, s.last_op, s.last_result, s.last_frame),
            (0, ApplyOp::MasterVolume, ApplyResult::Applied, 1_002_400)
        );
        assert_eq!(s.encode(), st);
        assert_eq!(a.encode_ctrl_msg(0x01)[12], 0x01);

        let r = |op, arg, value| {
            ApplyRequest {
                frame: 0,
                op,
                arg,
                value,
            }
            .args_valid()
        };
        assert!(r(ApplyOp::CancelAll, 0, 0) && !r(ApplyOp::Mute, 1, 0));
        assert!(r(ApplyOp::PresetLoad, 9, 0) && !r(ApplyOp::PresetLoad, 10, 0));
        assert!(!r(ApplyOp::MasterVolume, 0, 1) && !r(ApplyOp::MasterVolume, 0, -12801));
        assert!(r(ApplyOp::OutDelay, 3, 2047) && !r(ApplyOp::OutDelay, 3, -1));
        assert!(r(ApplyOp::Rate, 2, 0) && !r(ApplyOp::Rate, 3, 0));
        assert!(r(ApplyOp::Click, 5, 0) && !r(ApplyOp::Other(9), 0, 0));
    }

    #[test]
    fn wp_sw1_superframe_config_tx_counter() {
        let snap = SuperframeSnapshot {
            flags: 0x08,
            rate_code: 1,
            channels: 8,
            class: 1,
            hops: 0,
            sender_id: 0x42,
            apply: ApplyRequest {
                frame: 5,
                op: ApplyOp::Mute,
                arg: 0,
                value: 0,
            },
            frame_low: 0x1234,
        };
        let b = snap.encode();
        assert_eq!(&b[6..8], &[0, 0]);
        assert_eq!(b[16], 1);
        assert_eq!(SuperframeSnapshot::decode(&b), Some(snap));
        assert_eq!(SuperframeSnapshot::decode(&b[..23]), None);

        // 2.8.3's example.
        let ex = [0x01, 0x00, 0x01, 0x16, 0x15, 0xff, 0x01, 0x00];
        let c = LinkConfig::decode(&ex).unwrap();
        assert!(c.enable);
        assert_eq!(
            (c.rx_pin, c.class, c.tx_pin, c.los_pin, c.txdis_pin, c.flags),
            (0, 1, 22, 21, 0xFF, 1)
        );
        assert_eq!(c.encode(), ex);

        let c = FrameCounter {
            frame: 48_000 * 3600,
            time_us: 7,
            output_frames: 9,
        };
        assert_eq!(FrameCounter::decode(&c.encode()), Some(c));
        assert_eq!(FrameCounter::decode(&c.encode()[..15]), None);

        let tx = LinkTx {
            link_mask: 0x0003,
            class: 1,
            channels: 8,
            rate_code: 1,
            test_pattern: 1,
            slot_map_id: 0,
            sync_pin: 21,
            txdis_mask: 0x00FF,
            los_mask: 0x0002,
            usb_alt: 3,
            flags: 0,
        };
        assert_eq!(LinkTx::decode(&tx.encode()), Some(tx));
    }

    #[test]
    fn wp_sw1_events_are_the_examples_and_need_their_bits() {
        // 2.8.12: locked on L1, reason LOCKED, F_low32 = 1 000 000.
        let p = [
            0x02, 0xe1, 0x00, 0x00, 0x02, 0x01, 0x01, 0x00, 0x40, 0x42, 0x0f, 0x00,
        ];
        let want = LinkStateEvent {
            state: LinkState::Locked,
            class: 1,
            reason: LinkEventReason::Locked,
            frame_low: 1_000_000,
        };
        assert_eq!(
            link_event(Some(4), 0xE1, &p[4..]),
            Some(LinkEvent::State(want))
        );
        assert_eq!(want.encode(), p[4..]);
        assert!(!want.is_loss());
        // MASTER_VOLUME applied at frame 1 002 400.
        let p = [
            0x02, 0xe2, 0x00, 0x01, 0x04, 0x00, 0x00, 0x00, 0xa0, 0x4b, 0x0f, 0x00,
        ];
        let want = ApplyResultEvent {
            op: ApplyOp::MasterVolume,
            result: ApplyResult::Applied,
            frame_low: 1_002_400,
        };
        assert_eq!(
            link_event(Some(8), 0xE2, &p[4..]),
            Some(LinkEvent::Apply(want))
        );
        assert_eq!(want.encode(), p[4..]);

        let lost = LinkStateEvent {
            state: LinkState::Holdover,
            class: 1,
            reason: LinkEventReason::Los,
            frame_low: 0,
        };
        let q = lost.encode();
        assert!(lost.is_loss());
        // No bit, the other bit, no features: nothing.
        assert_eq!(link_event(Some(8), 0xE1, &q), None);
        assert_eq!(link_event(Some(2), 0xE1, &q), None);
        assert_eq!(link_event(None, 0xE1, &q), None);
        assert_eq!(link_event(Some(4), 0xE2, &p[4..]), None);
        // The clip id is not a link event.
        assert_eq!(link_event(Some(u32::MAX), 0xE0, &q), None);
        // Short: nothing, never a panic.
        assert_eq!(link_event(Some(u32::MAX), 0xE1, &q[..7]), None);
    }

    proptest! {
        #[test]
        fn proptest_wp_sw1_decoders_never_panic(b in proptest::collection::vec(any::<u8>(), 0..80)) {
            let _ = LinkInfo::decode(&b);
            let _ = LinkStatus::decode(&b);
            let _ = LinkConfig::decode(&b);
            let _ = LinkSlots::decode(&b);
            let _ = LinkPhase::decode(&b);
            let _ = FrameCounter::decode(&b);
            let _ = ApplyRequest::decode(&b).map(|a| a.args_valid());
            let _ = ApplyStatus::decode(&b);
            let _ = SuperframeSnapshot::decode(&b);
            let _ = LinkTx::decode(&b);
            for id in 0xE0..=0xE3u8 {
                let _ = link_event(Some(u32::MAX), id, &b);
            }
            prop_assert_eq!(LinkStatus::decode(&b).is_some(), b.len() >= LinkStatus::SIZE);
        }
    }
}
