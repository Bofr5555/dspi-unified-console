//! WP SW1 (SYS2 sections 2, 5.3 and 7): the audio link's extension
//! sub-commands (0x8000-0x800A, the fork's private range) and its two
//! notifications (0xE1 `ALINK_STATE`, 0xE2 `APPLY_RESULT`).
//!
//! The layouts are the fork's `extension_channel_spec.md` section 2.8 (WP
//! LK7, DSPi-bofr c3909e5; `EXT_ALINK_STATUS` offset 50 from LK45F, bofr-main
//! 5a5658d), which supersedes SYS2 5.3 where they differ:
//! a 52-byte status, a 24-byte `EXT_ALINK_CTRL_MSG` IN, `d_local_min/max` in
//! the info, the event reasons, apply op 0 as `CANCEL_ALL`. The firmware
//! (LK5, LK6) has no header for them yet, so they are transcribed rather than
//! generated: re-pin them to the vendored headers when it does.
//!
//! Pure codecs; the I/O is in `dspi_session::ext`. Every decoder takes device
//! bytes and never panics: a short reply is `None`, extra bytes are ignored.
//! A host reads none of these unless `EXT_LIST` names them, and decodes the
//! events only behind their `EXT_NOTIFY_FEATURES` bits, so stock firmware and
//! the fork without the alink see no extra traffic.

/// `EXT_ALINK_INFO` IN, 32 B: role, capabilities, pins.
pub const EXT_ALINK_INFO: u16 = 0x8000;
/// `EXT_ALINK_STATUS` IN: lock state, frame counter, error counters, servo.
pub const EXT_ALINK_STATUS: u16 = 0x8001;
/// `EXT_ALINK_CONFIG` IN/OUT, 8 B: enable, pins, class, flags.
pub const EXT_ALINK_CONFIG: u16 = 0x8002;
/// `EXT_ALINK_SLOTS` IN/OUT, 8 B: the audio link slot for each matrix input.
pub const EXT_ALINK_SLOTS: u16 = 0x8003;
/// `EXT_ALINK_PHASE` IN/OUT, 8 B: phase-lock mode and the fixed delay.
pub const EXT_ALINK_PHASE: u16 = 0x8004;
/// `EXT_FRAME_COUNTER` IN, 16 B: the alink frame and output frame at the read.
pub const EXT_FRAME_COUNTER: u16 = 0x8005;
/// `EXT_APPLY_AT_FRAME` OUT 16 B (queue an op) / IN 16 B (the queue's state).
pub const EXT_APPLY_AT_FRAME: u16 = 0x8006;
/// `EXT_ALINK_TX` IN/OUT, 16 B: a sender's alinks.
pub const EXT_ALINK_TX: u16 = 0x8007;
/// `EXT_ALINK_CTRL_MSG` OUT 16 B: a sender injects a superframe apply.
pub const EXT_ALINK_CTRL_MSG: u16 = 0x8008;
/// `EXT_ALINK_STATS_RESET` OUT, 1 B: clear the error counters.
pub const EXT_ALINK_STATS_RESET: u16 = 0x8009;
/// `EXT_ALINK_DIAG` IN, 64 B: bench diagnostics (not decoded here).
pub const EXT_ALINK_DIAG: u16 = 0x800A;

/// The audio link's sub-commands, ascending.
pub const ALINK_IDS: [u16; 11] = [
    EXT_ALINK_INFO,
    EXT_ALINK_STATUS,
    EXT_ALINK_CONFIG,
    EXT_ALINK_SLOTS,
    EXT_ALINK_PHASE,
    EXT_FRAME_COUNTER,
    EXT_APPLY_AT_FRAME,
    EXT_ALINK_TX,
    EXT_ALINK_CTRL_MSG,
    EXT_ALINK_STATS_RESET,
    EXT_ALINK_DIAG,
];

/// `EXT_ALINK_DIAG`'s reply size. Its layout is versioned by byte 0 and not
/// for apps to parse (2.8.11).
pub const ALINK_DIAG_SIZE: usize = 64;

/// The frame and superframe version this build speaks (`EXT_ALINK_INFO`
/// byte 1). A unit reporting another is treated as having no alink (2.8.13).
pub const ALINK_PROTOCOL_VERSION: u8 = 1;

/// The receiver's sub-commands (2.8, "Per build"): what a receiver lists.
pub const RECEIVER_IDS: [u16; 10] = [
    EXT_ALINK_INFO,
    EXT_ALINK_STATUS,
    EXT_ALINK_CONFIG,
    EXT_ALINK_SLOTS,
    EXT_ALINK_PHASE,
    EXT_FRAME_COUNTER,
    EXT_APPLY_AT_FRAME,
    EXT_ALINK_CTRL_MSG,
    EXT_ALINK_STATS_RESET,
    EXT_ALINK_DIAG,
];

// -- WP SW2 --
/// The sender's sub-commands (2.8, "Per build", roles 2 and 4): what a sender
/// or fan-out lists.
pub const SENDER_IDS: [u16; 8] = [
    EXT_ALINK_INFO,
    EXT_ALINK_STATUS,
    EXT_ALINK_CONFIG,
    EXT_FRAME_COUNTER,
    EXT_ALINK_TX,
    EXT_ALINK_CTRL_MSG,
    EXT_ALINK_STATS_RESET,
    EXT_ALINK_DIAG,
];

/// A superframe is 32 frames (`link_protocol_spec.md` 6).
pub const SUPERFRAME_FRAMES: u64 = 32;

/// 2.8.9's lead rule for `EXT_ALINK_CTRL_MSG` OUT: the earliest frame a sender
/// whose counter reads `sender_frame` accepts, the next superframe start plus
/// one superframe. `None` past the counter's range.
pub fn ctrl_msg_min_frame(sender_frame: u64) -> Option<u64> {
    sender_frame
        .checked_next_multiple_of(SUPERFRAME_FRAMES)?
        .checked_add(SUPERFRAME_FRAMES)
}

/// What an alink sub-command is called, for listings (the Terminal's extension
/// list); `None` outside the range. Always "audio link": "link" alone is DSPi
/// Link.
pub fn alink_name(id: u16) -> Option<&'static str> {
    Some(match id {
        EXT_ALINK_INFO => "audio link info",
        EXT_ALINK_STATUS => "audio link status",
        EXT_ALINK_CONFIG => "audio link config",
        EXT_ALINK_SLOTS => "audio link slots",
        EXT_ALINK_PHASE => "audio link phase",
        EXT_FRAME_COUNTER => "audio link frame counter",
        EXT_APPLY_AT_FRAME => "audio link apply at frame",
        EXT_ALINK_TX => "audio link transmit",
        EXT_ALINK_CTRL_MSG => "audio link control message",
        EXT_ALINK_STATS_RESET => "audio link stats reset",
        EXT_ALINK_DIAG => "audio link diagnostics",
        _ => return None,
    })
}

/// `INPUT_SOURCE_ALINK`: `REQ_SET_INPUT_SOURCE` with 7 selects the alink input
/// (SYS2 5.1). Stock firmware refuses 7.
pub const INPUT_SOURCE_ALINK: u8 = 7;

/// `EXT_NOTIFY_FEATURES` bit 2: the device sends [`NOTIFY_EVT_ALINK_STATE`].
pub const NOTIFY_FEAT_ALINK_STATE: u32 = 1 << 2;
/// `EXT_NOTIFY_FEATURES` bit 3: the device sends [`NOTIFY_EVT_APPLY_RESULT`].
pub const NOTIFY_FEAT_APPLY_RESULT: u32 = 1 << 3;
/// `NOTIFY_EVT_ALINK_STATE`: lock, loss, re-anchor.
pub const NOTIFY_EVT_ALINK_STATE: u8 = 0xE1;
/// `NOTIFY_EVT_APPLY_RESULT`: an apply-at-frame op completed.
pub const NOTIFY_EVT_APPLY_RESULT: u8 = 0xE2;
/// The firmware's rate limit on the link-state event, as for the clip event
/// (the apply result is not limited).
pub const ALINK_STATE_MIN_INTERVAL_MS: u16 = 250;

/// Matrix inputs a unit takes from the alink (`EXT_ALINK_SLOTS`).
pub const ALINK_SLOTS: usize = 8;
/// A slot byte that feeds silence to its matrix input.
pub const ALINK_SLOT_SILENT: u8 = 0xFF;
/// The most channels an alink frame carries (superframe byte 4: 1..64). A
/// slot is below this or silent.
pub const ALINK_MAX_CHANNELS: u8 = 64;
/// `hops_offset`'s range in `EXT_ALINK_PHASE`.
pub const HOPS_OFFSET_MAX: i16 = 64;

/// `EXT_ALINK_INFO.features` bits (2.8.1).
pub mod feature {
    /// `EXT_ALINK_PHASE` mode 1.
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

/// What the unit is on the alink (`EXT_ALINK_INFO` byte 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlinkRole {
    None,
    Receiver,
    Sender,
    Repeater,
    FanOut,
    /// A value this build does not know.
    Other(u8),
}

impl AlinkRole {
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

    /// Whether the unit takes audio from an alink (and so offers the alink
    /// input source).
    pub fn receives(self) -> bool {
        matches!(self, Self::Receiver | Self::Repeater)
    }
}

/// The receiver's lock state (`EXT_ALINK_STATUS` byte 0, event 0xE1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlinkState {
    Off,
    Searching,
    Locked,
    /// The alink was lost: outputs muted, VCXO trim held.
    Holdover,
    Other(u8),
}

impl AlinkState {
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
pub enum AlinkEventReason {
    None,
    /// Anchored and playing.
    Locked,
    /// The LOS pin.
    Los,
    /// Three consecutive bad headers.
    HeaderErrors,
    CommaLost,
    /// Phase error beyond half a frame after a loss, or a new
    /// `EXT_ALINK_PHASE`: a short silence.
    Reanchor,
    /// VCXO authority exceeded.
    VcxoLimit,
    /// `enable` cleared or the input source changed.
    Disabled,
    Other(u8),
}

impl AlinkEventReason {
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

    /// The alink went away: LOS, bad headers, a lost comma.
    pub fn is_loss(self) -> bool {
        matches!(self, Self::Los | Self::HeaderErrors | Self::CommaLost)
    }
}

/// An apply-at-frame operation (2.8.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOp {
    /// `EXT_APPLY_AT_FRAME`: cancel every pending op (`frame` ignored).
    /// `EXT_ALINK_CTRL_MSG`: no apply, clear a pending injection.
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

/// `EXT_ALINK_INFO` IN, 32 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlinkInfo {
    pub role: AlinkRole,
    pub protocol_version: u8,
    /// b0 L1, b1 L2, b2 L3.
    pub class_mask: u8,
    /// A sender's alink outputs; 1 on a receiver.
    pub max_alinks: u8,
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

impl AlinkInfo {
    pub const SIZE: usize = 32;

    /// Whether the unit has an alink this build can use: a role, and the
    /// protocol version it speaks (2.8.13).
    pub fn usable(&self) -> bool {
        self.role != AlinkRole::None && self.protocol_version == ALINK_PROTOCOL_VERSION
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        Some(Self {
            role: AlinkRole::from_raw(byte(b, 0)?),
            protocol_version: byte(b, 1)?,
            class_mask: byte(b, 2)?,
            max_alinks: byte(b, 3)?,
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
        b[3] = self.max_alinks;
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

/// `EXT_ALINK_STATUS` IN, 52 bytes (2.8.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlinkStatus {
    pub state: AlinkState,
    /// 0 unknown, 1-3 = L1-L3.
    pub class: u8,
    /// 0 = 44.1, 1 = 48, 2 = 96 kHz, 0xFF unknown.
    pub rate_code: u8,
    pub channels: u8,
    pub hops: u8,
    /// The LOS pin: 0 clear, 1 asserted, 0xFF none configured.
    pub los_pin: u8,
    pub sender_id: u8,
    /// The superframe's flags byte as last received (2.8.2): bit 0 system
    /// mute, 1 rate change pending, 2 apply pending, 3 the sender has a live
    /// USB stream, 4 test pattern, 5 the sender is slaved.
    pub flags: u8,
    /// The alink frame counter F.
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
    // -- WP SW2 --
    /// Offset 50 (LK45F): on a re-emitter (fan-out, role 4, or repeater, role
    /// 3) the frames between receiving alink frame F and sending it again, 0
    /// while it is not re-emitting; 0 on every other unit. The hub
    /// compensates it through `EXT_ALINK_PHASE.hops_offset` of the units
    /// behind it (`link_protocol_spec.md` 7.2).
    pub hop_delay_frames: u8,
}

impl AlinkStatus {
    pub const SIZE: usize = 52;

    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        let f32_at = |i| u32_at(b, i).map(f32::from_bits);
        Some(Self {
            state: AlinkState::from_raw(byte(b, 0)?),
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
            // -- WP SW2 --
            hop_delay_frames: byte(b, 50)?,
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
        // -- WP SW2 -- offset 51 is reserved, 0.
        b[50] = self.hop_delay_frames;
        b
    }
}

/// `EXT_ALINK_CONFIG` IN/OUT, 8 bytes. 0xFF in a pin field is "default" (rx)
/// or "none" (the others).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlinkConfig {
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

impl AlinkConfig {
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

/// `EXT_ALINK_SLOTS` IN/OUT: the audio link slot for each matrix input, or
/// [`ALINK_SLOT_SILENT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlinkSlots(pub [u8; ALINK_SLOTS]);

impl AlinkSlots {
    pub const SIZE: usize = ALINK_SLOTS;
    /// Every input silent.
    pub const SILENT: AlinkSlots = AlinkSlots([ALINK_SLOT_SILENT; ALINK_SLOTS]);

    pub fn decode(b: &[u8]) -> Option<Self> {
        b.get(..Self::SIZE)?.try_into().ok().map(AlinkSlots)
    }

    /// The default: slot `i` into input `i`.
    pub const IDENTITY: AlinkSlots = AlinkSlots([0, 1, 2, 3, 4, 5, 6, 7]);

    /// Each slot below [`ALINK_MAX_CHANNELS`] or silent; 64-0xFE are refused.
    /// A slot at or above the stream's channels is accepted and plays
    /// silence until the stream grows (2.8.4).
    pub fn is_valid(&self) -> bool {
        self.0
            .iter()
            .all(|s| *s == ALINK_SLOT_SILENT || *s < ALINK_MAX_CHANNELS)
    }
}

/// `EXT_ALINK_PHASE` mode: today's fill servo.
pub const PHASE_MODE_FILL: u8 = 0;
/// `EXT_ALINK_PHASE` mode: phase lock to the frame counter.
pub const PHASE_MODE_LOCK: u8 = 1;

/// `EXT_ALINK_PHASE` IN/OUT, 8 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlinkPhase {
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

/// `EXT_ALINK_PHASE.flags` bit 0: the effective delay is below `d_local_min`;
/// the unit runs as the fill servo until a valid value is written.
pub const PHASE_FLAG_D_BELOW_MIN: u8 = 1 << 0;

impl AlinkPhase {
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
    /// [`AlinkInfo::d_local_min`, `d_local_max`], no OUT flags.
    pub fn is_valid_for(&self, info: &AlinkInfo) -> bool {
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
    /// The alink frame F at the read.
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

// -- fixup sw2x (F4) --
/// Outputs an apply op may name (`OUT_DELAY`, `CLICK`): the 8 output slots and
/// the PDM sub of an RP2350 (`sender_link.h` `SND_OP_OUTPUTS`).
pub const APPLY_OUTPUTS: u8 = 9;
/// The longest `OUT_DELAY`, samples: `MAX_DELAY_SAMPLES - 1` on an RP2350
/// (2.8.7, `SND_OP_DELAY_MAX`).
pub const APPLY_DELAY_MAX: i16 = 2047;

/// `EXT_APPLY_AT_FRAME` OUT (and the sender's `EXT_ALINK_CTRL_MSG` apply):
/// run `op` when the alink frame reaches `frame`.
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
    /// 0). -- fixup sw2x (F4) -- as the sender checks them (DSPi-bofr
    /// `sender_link.c` `snd_op_args_ok`): an output below [`APPLY_OUTPUTS`] for
    /// `OUT_DELAY` and `CLICK`, a delay of 0..=[`APPLY_DELAY_MAX`] samples (an
    /// RP2040 unit refuses above 1023 itself).
    pub fn args_valid(&self) -> bool {
        let (a, v) = (self.arg, self.value);
        match self.op {
            ApplyOp::CancelAll | ApplyOp::Mute | ApplyOp::Unmute | ApplyOp::SyncPulse => {
                a == 0 && v == 0
            }
            ApplyOp::PresetLoad => a <= 9 && v == 0,
            ApplyOp::MasterVolume => a == 0 && (-12800..=0).contains(&v),
            ApplyOp::OutDelay => a < APPLY_OUTPUTS && (0..=APPLY_DELAY_MAX).contains(&v),
            ApplyOp::Rate => a <= 2 && v == 0,
            ApplyOp::Click => a < APPLY_OUTPUTS && v == 0,
            ApplyOp::Other(_) => false,
        }
    }

    /// `EXT_ALINK_CTRL_MSG` OUT on a sender: the apply plus the superframe's
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

/// `EXT_ALINK_CTRL_MSG` IN, 24 bytes: the last valid superframe received
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

/// `EXT_ALINK_TX` IN/OUT, 16 bytes: a sender's alink outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlinkTx {
    pub alink_mask: u16,
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

impl AlinkTx {
    pub const SIZE: usize = 16;

    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        Some(Self {
            alink_mask: u16_at(b, 0)?,
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
        b[..2].copy_from_slice(&self.alink_mask.to_le_bytes());
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

/// `EXT_ALINK_STATS_RESET`'s payload: clear the counters.
pub const ALINK_STATS_RESET: [u8; 1] = [1];

// ------------------------------------------------------------ events

/// Event 0xE1: `[state, class, reason, 0, F_low32]` from byte 4 of the
/// packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlinkStateEvent {
    pub state: AlinkState,
    pub class: u8,
    pub reason: AlinkEventReason,
    /// The alink frame's low 32 bits when it was sent.
    pub frame_low: u32,
}

impl AlinkStateEvent {
    /// The payload size from byte 4.
    pub const PAYLOAD: usize = 8;

    pub fn decode(payload: &[u8]) -> Option<Self> {
        Some(Self {
            state: AlinkState::from_raw(byte(payload, 0)?),
            class: byte(payload, 1)?,
            reason: AlinkEventReason::from_raw(byte(payload, 2)?),
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
        self.reason.is_loss() || self.state != AlinkState::Locked
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

/// One of the alink's notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlinkEvent {
    State(AlinkStateEvent),
    Apply(ApplyResultEvent),
}

/// The alink's events behind their feature bits, as [`super::clip_event`]:
/// `notify_features` is the device's `EXT_NOTIFY_FEATURES` (`None` when it
/// lacks or refused it), `id` and `payload` an `Event::Unknown`'s (bytes 4
/// on). Only a device announcing bit 2 has its 0xE1 read, only bit 3 its
/// 0xE2.
pub fn alink_event(notify_features: Option<u32>, id: u8, payload: &[u8]) -> Option<AlinkEvent> {
    let f = notify_features?;
    match id {
        NOTIFY_EVT_ALINK_STATE if f & NOTIFY_FEAT_ALINK_STATE != 0 => {
            AlinkStateEvent::decode(payload).map(AlinkEvent::State)
        }
        NOTIFY_EVT_APPLY_RESULT if f & NOTIFY_FEAT_APPLY_RESULT != 0 => {
            ApplyResultEvent::decode(payload).map(AlinkEvent::Apply)
        }
        _ => None,
    }
}

// -- WP SW3 --
/// The trim scan at link-up from `EXT_ALINK_DIAG` (2.8.11, offsets 32-39): the
/// sampling trims that decoded clean and the one in use. A healthy alink has a
/// clean range centred on 0 within one step. The DIAG layout is versioned by
/// byte 0, so only version 1 (a receiver's) is read; a sender build (`0x81`)
/// or a later layout gives `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrimScan {
    /// Lowest clean trim, -9..+9 (one step moves the window 3.26 ns).
    pub trim_lo: i8,
    /// Highest clean trim.
    pub trim_hi: i8,
    /// The trim in use.
    pub trim_used: i8,
    /// 0 no scan, 1 running, 2 done, 3 done but no trim was clean.
    pub scan_state: u8,
    /// Bit i set = trim i - 9 was clean.
    pub clean_mask: u32,
}

impl TrimScan {
    /// The DIAG layout this decodes.
    pub const DIAG_VERSION: u8 = 1;

    /// From a whole `EXT_ALINK_DIAG` reply; `None` when short or not version 1.
    pub fn decode_diag(b: &[u8]) -> Option<Self> {
        let b = b.get(..ALINK_DIAG_SIZE)?;
        if byte(b, 0)? != Self::DIAG_VERSION {
            return None;
        }
        Some(Self {
            trim_lo: i8::from_le_bytes([byte(b, 32)?]),
            trim_hi: i8::from_le_bytes([byte(b, 33)?]),
            trim_used: i8::from_le_bytes([byte(b, 34)?]),
            scan_state: byte(b, 35)?,
            clean_mask: u32_at(b, 36)?,
        })
    }

    /// The scan state's name: `none`, `running`, `done`, `no_clean_trim`.
    pub fn state_name(&self) -> &'static str {
        match self.scan_state {
            0 => "none",
            1 => "running",
            2 => "done",
            3 => "no_clean_trim",
            _ => "unknown",
        }
    }
}

#[cfg(test)]
mod tests_wp_sw3 {
    use super::*;

    #[test]
    fn wp_sw3_trim_scan_from_diag_version_1_only() {
        let mut d = vec![0u8; ALINK_DIAG_SIZE];
        d[0] = 1;
        d[32] = (-3i8) as u8;
        d[33] = 4;
        d[34] = 0;
        d[35] = 2;
        d[36..40].copy_from_slice(&0x0000_FF00u32.to_le_bytes());
        let t = TrimScan::decode_diag(&d).unwrap();
        assert_eq!((t.trim_lo, t.trim_hi, t.trim_used), (-3, 4, 0));
        assert_eq!(t.state_name(), "done");
        assert_eq!(t.clean_mask, 0xFF00);
        d[0] = 0x81;
        assert_eq!(TrimScan::decode_diag(&d), None);
        d[0] = 1;
        assert_eq!(TrimScan::decode_diag(&d[..40]), None);
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
        assert_eq!(ALINK_IDS.first(), Some(&0x8000));
        assert_eq!(ALINK_IDS.last(), Some(&0x800A));
        assert!(ALINK_IDS.windows(2).all(|w| w[1] == w[0] + 1));
        for id in ALINK_IDS {
            assert!(alink_name(id).is_some());
            assert_eq!(super::super::name(id), alink_name(id).unwrap());
            // -- fixup sw1 (F2) -- never a bare "link" (that is DSPi Link)
            assert!(alink_name(id).unwrap().starts_with("audio link "));
        }
        // A receiver lists all but the sender's EXT_ALINK_TX.
        assert!(!RECEIVER_IDS.contains(&EXT_ALINK_TX));
        assert_eq!(RECEIVER_IDS.len(), ALINK_IDS.len() - 1);
        assert_eq!(alink_name(0x800B), None);
        assert_eq!(super::super::name(0x800B), "experimental");
        assert_eq!(INPUT_SOURCE_ALINK, 7);
        assert_eq!((NOTIFY_FEAT_ALINK_STATE, NOTIFY_FEAT_APPLY_RESULT), (4, 8));
        assert_eq!(
            (NOTIFY_EVT_ALINK_STATE, NOTIFY_EVT_APPLY_RESULT),
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
        let i = AlinkInfo::decode(&INFO_EXAMPLE).unwrap();
        assert_eq!(i.role, AlinkRole::Receiver);
        assert_eq!((i.protocol_version, i.class_mask, i.max_alinks), (1, 7, 1));
        assert_eq!((i.max_channels, i.slots_per_unit), (64, 8));
        assert_eq!((i.rx_pin, i.tx_pin), (0, 0xFF));
        assert_eq!(
            i.feature_bits,
            feature::PHASE_LOCK | feature::APPLY_AT_FRAME | feature::LOS_PIN
        );
        assert_eq!((i.d_local_min, i.d_local_max), (112, 1024));
        assert!(i.usable());
        assert_eq!(i.encode(), INFO_EXAMPLE);
        assert_eq!(AlinkInfo::decode(&INFO_EXAMPLE[..31]), None);
        // Role 0 or another protocol version: no alink.
        assert!(
            !AlinkInfo {
                role: AlinkRole::None,
                ..i
            }
            .usable()
        );
        assert!(
            !AlinkInfo {
                protocol_version: 2,
                ..i
            }
            .usable()
        );
        assert!(AlinkRole::Receiver.receives() && AlinkRole::Repeater.receives());
        assert!(!AlinkRole::Sender.receives());
        for raw in 0..=255u8 {
            assert_eq!(AlinkRole::from_raw(raw).raw(), raw);
            assert_eq!(AlinkState::from_raw(raw).raw(), raw);
            assert_eq!(AlinkEventReason::from_raw(raw).raw(), raw);
            assert_eq!(ApplyOp::from_raw(raw).raw(), raw);
            assert_eq!(ApplyResult::from_raw(raw).raw(), raw);
        }
    }

    #[test]
    fn wp_sw1_status_layout() {
        let s = AlinkStatus {
            state: AlinkState::Locked,
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
            hop_delay_frames: 0,
        };
        let b = s.encode();
        assert_eq!(b.len(), 52);
        assert_eq!(b[0], 2);
        assert_eq!(&b[8..16], &[8, 7, 6, 5, 4, 3, 2, 1]);
        assert_eq!(&b[36..40], &[6, 0, 0, 0]);
        assert_eq!(f32::from_le_bytes(b[40..44].try_into().unwrap()), -12.5);
        assert_eq!(&b[48..52], &[96, 0, 0, 0]);
        assert_eq!(AlinkStatus::decode(&b), Some(s));
        assert_eq!(AlinkStatus::decode(&b[..51]), None);
    }

    // -- fixup sw2x --
    /// Review sw2x probe C: `OUT_DELAY` and `CLICK` are bounded as the sender
    /// bounds them.
    #[test]
    fn wp_sw2x_out_delay_and_click_are_bounded() {
        let r = |op, arg, value| {
            ApplyRequest {
                frame: 0,
                op,
                arg,
                value,
            }
            .args_valid()
        };
        assert!(r(ApplyOp::OutDelay, 8, 2047));
        assert!(!r(ApplyOp::OutDelay, 200, 30000));
        assert!(!r(ApplyOp::OutDelay, 0, 2048));
        assert!(!r(ApplyOp::OutDelay, 9, 0));
        assert!(r(ApplyOp::Click, 8, 0));
        assert!(!r(ApplyOp::Click, 250, 0));
        assert!(!r(ApplyOp::Click, 9, 0));
    }

    // -- WP SW2 --
    /// 2.8.9: the next superframe start plus 32.
    #[test]
    fn wp_sw2_ctrl_msg_lead() {
        assert_eq!(ctrl_msg_min_frame(0), Some(32));
        assert_eq!(ctrl_msg_min_frame(1), Some(64));
        assert_eq!(ctrl_msg_min_frame(32), Some(64));
        assert_eq!(ctrl_msg_min_frame(u64::MAX), None);
        assert!(SENDER_IDS.iter().all(|id| ALINK_IDS.contains(id)));
        assert!(!SENDER_IDS.contains(&EXT_ALINK_SLOTS) && !RECEIVER_IDS.contains(&EXT_ALINK_TX));
    }

    // -- WP SW2 --
    /// `extension_channel_spec.md` 2.8.2 on bofr-main 5a5658d (LK45F): offset
    /// 50 is `hop_delay_frames`, u8; 51 is reserved. A fan-out that re-emits
    /// 3 frames late, behind one re-emitter, reports hops 1 and 3.
    #[test]
    fn wp_sw2_status_offset_50_is_the_hop_delay() {
        let mut b = [0u8; AlinkStatus::SIZE];
        b[0] = 2;
        b[4] = 1;
        b[50] = 3;
        b[51] = 0xAA;
        let s = AlinkStatus::decode(&b).unwrap();
        assert_eq!((s.hops, s.hop_delay_frames), (1, 3));
        let e = s.encode();
        assert_eq!(e[50], 3);
        assert_eq!(e[51], 0, "reserved reads 0 in what the fake sends");
        // Still 52 bytes: a 51-byte reply has no hop delay and is short.
        assert_eq!(AlinkStatus::decode(&b[..51]), None);
    }

    #[test]
    fn wp_sw1_slots_and_phase() {
        assert!(AlinkSlots([4, 5, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]).is_valid());
        // At or above C is fine; 64..0xFE is refused.
        assert!(AlinkSlots([63; 8]).is_valid());
        assert!(!AlinkSlots([64, 0, 0, 0, 0, 0, 0, 0]).is_valid());
        assert!(!AlinkSlots([0xFE; 8]).is_valid());
        assert!(AlinkSlots::SILENT.is_valid() && AlinkSlots::IDENTITY.is_valid());
        assert_eq!(AlinkSlots::decode(&AlinkSlots::IDENTITY.0[..7]), None);

        // 2.8.5's example.
        let ex = [0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x02];
        let p = AlinkPhase::decode(&ex).unwrap();
        assert_eq!(
            (p.mode, p.d_local, p.hops_offset, p.flags, p.state),
            (1, 128, 0, 0, 2)
        );
        assert_eq!(p.encode(), ex);
        let info = AlinkInfo::decode(&INFO_EXAMPLE).unwrap();
        assert!(p.is_valid_for(&info));
        let bad = |q: AlinkPhase| !q.is_valid_for(&info);
        assert!(bad(AlinkPhase { mode: 2, ..p }));
        assert!(bad(AlinkPhase { d_local: 111, ..p }));
        assert!(bad(AlinkPhase { d_local: 1025, ..p }));
        assert!(bad(AlinkPhase {
            hops_offset: 65,
            ..p
        }));
        assert!(bad(AlinkPhase {
            flags: PHASE_FLAG_D_BELOW_MIN,
            ..p
        }));
        // The offset counts: 120 - 8 = 112 is in, 112 - 1 is not.
        assert!(
            AlinkPhase {
                d_local: 120,
                hops_offset: -8,
                ..p
            }
            .is_valid_for(&info)
        );
        assert!(bad(AlinkPhase {
            d_local: 112,
            hops_offset: -1,
            ..p
        }));
        assert_eq!(
            AlinkPhase {
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
        let c = AlinkConfig::decode(&ex).unwrap();
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

        let tx = AlinkTx {
            alink_mask: 0x0003,
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
        assert_eq!(AlinkTx::decode(&tx.encode()), Some(tx));
    }

    #[test]
    fn wp_sw1_events_are_the_examples_and_need_their_bits() {
        // 2.8.12: locked on L1, reason LOCKED, F_low32 = 1 000 000.
        let p = [
            0x02, 0xe1, 0x00, 0x00, 0x02, 0x01, 0x01, 0x00, 0x40, 0x42, 0x0f, 0x00,
        ];
        let want = AlinkStateEvent {
            state: AlinkState::Locked,
            class: 1,
            reason: AlinkEventReason::Locked,
            frame_low: 1_000_000,
        };
        assert_eq!(
            alink_event(Some(4), 0xE1, &p[4..]),
            Some(AlinkEvent::State(want))
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
            alink_event(Some(8), 0xE2, &p[4..]),
            Some(AlinkEvent::Apply(want))
        );
        assert_eq!(want.encode(), p[4..]);

        let lost = AlinkStateEvent {
            state: AlinkState::Holdover,
            class: 1,
            reason: AlinkEventReason::Los,
            frame_low: 0,
        };
        let q = lost.encode();
        assert!(lost.is_loss());
        // No bit, the other bit, no features: nothing.
        assert_eq!(alink_event(Some(8), 0xE1, &q), None);
        assert_eq!(alink_event(Some(2), 0xE1, &q), None);
        assert_eq!(alink_event(None, 0xE1, &q), None);
        assert_eq!(alink_event(Some(4), 0xE2, &p[4..]), None);
        // The clip id is not an alink event.
        assert_eq!(alink_event(Some(u32::MAX), 0xE0, &q), None);
        // Short: nothing, never a panic.
        assert_eq!(alink_event(Some(u32::MAX), 0xE1, &q[..7]), None);
    }

    proptest! {
        #[test]
        fn proptest_wp_sw1_decoders_never_panic(b in proptest::collection::vec(any::<u8>(), 0..80)) {
            let _ = AlinkInfo::decode(&b);
            let _ = AlinkStatus::decode(&b);
            let _ = AlinkConfig::decode(&b);
            let _ = AlinkSlots::decode(&b);
            let _ = AlinkPhase::decode(&b);
            let _ = FrameCounter::decode(&b);
            let _ = ApplyRequest::decode(&b).map(|a| a.args_valid());
            let _ = ApplyStatus::decode(&b);
            let _ = SuperframeSnapshot::decode(&b);
            let _ = AlinkTx::decode(&b);
            for id in 0xE0..=0xE3u8 {
                let _ = alink_event(Some(u32::MAX), id, &b);
            }
            prop_assert_eq!(AlinkStatus::decode(&b).is_some(), b.len() >= AlinkStatus::SIZE);
        }
    }
}
