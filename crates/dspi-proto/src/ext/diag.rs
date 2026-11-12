//! WP FWX (PRO1 FW1, FW4, FW5): the identify and reset-info sub-commands and
//! the clip notification, as the fork's `extension_channel_spec.md` sections
//! 2.4.1, 2.5 and 2.6 describe them.
//!
//! Pure codecs; the I/O is in `dspi_session::ext`. Every decoder takes device
//! bytes and never panics: a short reply is `None`, extra bytes are ignored.

use crate::generated::notify as nfy;

/// `EXT_NOTIFY_FEATURES` bit 1, `NOTIFY_FEAT_CLIP_EVENT` (notify.h, a
/// `(1u << 1)` the header generator does not lift; pinned to the header by
/// this module's tests): the device sends [`NOTIFY_EVT_EXT_CLIP`].
pub const NOTIFY_FEAT_CLIP_EVENT: u32 = 1 << 1;

/// The clip event's id, `NOTIFY_EVT_EXT_CLIP`: 0xE0, the first of the fork's
/// event ids (0xE0-0xEF, decoded only when their feature bit is set).
pub const NOTIFY_EVT_EXT_CLIP: u8 = {
    assert!(nfy::NOTIFY_EVT_EXT_CLIP <= u8::MAX as u16);
    nfy::NOTIFY_EVT_EXT_CLIP as u8
};

/// The firmware's rate limit on clip events: at most one per this many
/// milliseconds; clips in between are merged into the next one.
pub const CLIP_MIN_INTERVAL_MS: u16 = nfy::NOTIFY_CLIP_MIN_INTERVAL_MS;

/// One clip event: the channels that clipped since the previous one, in the
/// bit space of the polled clip flags (`REQ_CLEAR_CLIPS`, `GET_STATUS` 9):
/// inputs first, then outputs, the PDM sub last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipEvent {
    /// The lowest channel in `mask`, as the device reported it.
    pub channel: u8,
    /// The channels, bit `c` for channel `c`. Never 0.
    pub mask: u32,
}

impl ClipEvent {
    /// The payload size from byte 4: `[channel, count, 0, 0, mask LE32]`.
    pub const PAYLOAD: usize = 8;

    /// The event from a packet's payload (bytes 4 on, as the session's
    /// decoder keeps it in `Event::Unknown`). `None` when it is short or its
    /// mask is empty (the firmware never sends one). The channel byte is
    /// taken from the mask, which is the authority: a device whose byte 4
    /// disagrees still yields the mask's lowest channel.
    pub fn decode(payload: &[u8]) -> Option<Self> {
        let m = payload.get(4..Self::PAYLOAD)?;
        let mask = u32::from_le_bytes([m[0], m[1], m[2], m[3]]);
        if mask == 0 {
            return None;
        }
        // trailing_zeros of a nonzero u32 is below 32.
        let channel = u8::try_from(mask.trailing_zeros()).unwrap_or(u8::MAX);
        Some(Self { channel, mask })
    }

    /// The channels in the mask, ascending.
    pub fn channels(&self) -> impl Iterator<Item = u8> + '_ {
        (0u8..32).filter(|c| self.mask & (1u32 << c) != 0)
    }
}

/// [`ClipEvent::decode`] behind the feature bit: `notify_features` is the
/// device's `EXT_NOTIFY_FEATURES` (`None` when it lacks or refused it), `id`
/// and `payload` an `Event::Unknown`'s. Only a device that announces
/// [`NOTIFY_FEAT_CLIP_EVENT`] has its 0xE0 read as a clip event, so stock
/// firmware, or a later upstream use of the id, is never misread.
pub fn clip_event(notify_features: Option<u32>, id: u8, payload: &[u8]) -> Option<ClipEvent> {
    let announced = notify_features.is_some_and(|f| f & NOTIFY_FEAT_CLIP_EVENT != 0);
    if !announced || id != NOTIFY_EVT_EXT_CLIP {
        return None;
    }
    ClipEvent::decode(payload)
}

// ------------------------------------------------------------ identify

/// `IDENTIFY_TONE_NONE`: no tone.
pub const IDENTIFY_TONE_NONE: u8 = 0xFF;
/// The longest blink the firmware runs; longer requests are shortened to it.
pub const IDENTIFY_MAX_S: u8 = 120;

/// `EXT_IDENTIFY` OUT: blink the LED for `seconds` (0 stops), and play the
/// quiet identify tone (1 kHz, -40 dBFS, 400 ms, through the output's whole
/// chain) on `tone_output` when given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentifyRequest {
    pub seconds: u8,
    pub tone_output: Option<u8>,
}

impl IdentifyRequest {
    pub const SIZE: usize = 4;

    /// `[seconds, tone_output or 0xFF, 0, 0]`. A tone output of 0xFF is the
    /// same as none.
    pub fn encode(self) -> [u8; Self::SIZE] {
        [
            self.seconds,
            self.tone_output.unwrap_or(IDENTIFY_TONE_NONE),
            0,
            0,
        ]
    }
}

/// Which chip the device is (`IdentifyStatus::chip`, `ResetInfo::chip`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chip {
    Rp2040,
    Rp2350,
    /// A value this build does not know.
    Other(u8),
}

impl Chip {
    pub fn from_raw(b: u8) -> Self {
        match b {
            0 => Self::Rp2040,
            1 => Self::Rp2350,
            other => Self::Other(other),
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::Rp2040 => "RP2040".into(),
            Self::Rp2350 => "RP2350".into(),
            Self::Other(b) => format!("chip {b}"),
        }
    }
}

/// `EXT_IDENTIFY` IN, 56 bytes (identify.h).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifyStatus {
    /// Blink time left, rounded up; 0 when not blinking.
    pub remaining_s: u8,
    /// The output of the last tone asked for since boot.
    pub last_tone: Option<u8>,
    pub chip: Chip,
    /// The SDK's `rp2040_chip_version()` / `rp2350_chip_version()`.
    pub chip_rev: u8,
    /// The raw `SYSINFO.CHIP_ID` register.
    pub chip_id: u32,
    /// The build's `PICO_BOARD` (`pico`, `pico2`).
    pub board: String,
    /// The build's `DSPI_HW_REV`; empty when the build did not set it.
    pub hw_rev: String,
}

impl IdentifyStatus {
    pub const SIZE: usize = 56;

    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        Some(Self {
            remaining_s: b[0],
            last_tone: (b[1] != IDENTIFY_TONE_NONE).then_some(b[1]),
            chip: Chip::from_raw(b[2]),
            chip_rev: b[3],
            chip_id: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            board: c_text(&b[8..24]),
            hw_rev: c_text(&b[24..56]),
        })
    }

    /// Readable `key value` pairs, in wire order.
    pub fn fields(&self) -> Vec<(&'static str, String)> {
        vec![
            ("blink_left_s", self.remaining_s.to_string()),
            (
                "last_tone",
                self.last_tone
                    .map_or_else(|| "none".into(), |o| format!("output {o}")),
            ),
            ("chip", self.chip.label()),
            ("chip_rev", self.chip_rev.to_string()),
            ("chip_id", format!("0x{:08X}", self.chip_id)),
            ("board", self.board.clone()),
            (
                "hw_rev",
                if self.hw_rev.is_empty() {
                    "not set".into()
                } else {
                    self.hw_rev.clone()
                },
            ),
        ]
    }
}

/// A NUL-padded string field: the bytes up to the first NUL, printable ASCII
/// kept, anything else replaced by `?` (the firmware writes ASCII; a device
/// is not trusted to).
fn c_text(b: &[u8]) -> String {
    b.iter()
        .take_while(|c| **c != 0)
        .map(|c| {
            if c.is_ascii_graphic() || *c == b' ' {
                char::from(*c)
            } else {
                '?'
            }
        })
        .collect()
}

// ------------------------------------------------------------ reset info

/// Why the device last reset (`ResetInfo::reason`, reset_info.h).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetReason {
    Unknown,
    /// Power-on. On an RP2040 this also covers a brown-out: the chip does not
    /// tell them apart.
    PowerOn,
    /// The brown-out detector (RP2350 only).
    Brownout,
    /// The RUN pin was pulled low.
    RunPin,
    /// The firmware's 8 s watchdog fired: a hang.
    Watchdog,
    /// A requested reboot: a host's reboot request, the BOOTSEL round trip,
    /// the bootrom starting the app after a firmware copy.
    Software,
    /// The debug port or a rescue reset.
    Debugger,
    /// The RP2350 glitch detector.
    Glitch,
    /// A value this build does not know.
    Other(u8),
}

impl ResetReason {
    pub fn from_raw(b: u8) -> Self {
        match b {
            0 => Self::Unknown,
            1 => Self::PowerOn,
            2 => Self::Brownout,
            3 => Self::RunPin,
            4 => Self::Watchdog,
            5 => Self::Software,
            6 => Self::Debugger,
            7 => Self::Glitch,
            other => Self::Other(other),
        }
    }

    pub fn raw(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::PowerOn => 1,
            Self::Brownout => 2,
            Self::RunPin => 3,
            Self::Watchdog => 4,
            Self::Software => 5,
            Self::Debugger => 6,
            Self::Glitch => 7,
            Self::Other(b) => b,
        }
    }

    /// A stable lower-case name, for records and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown | Self::Other(_) => "unknown",
            Self::PowerOn => "power_on",
            Self::Brownout => "brownout",
            Self::RunPin => "run_pin",
            Self::Watchdog => "watchdog",
            Self::Software => "software",
            Self::Debugger => "debugger",
            Self::Glitch => "glitch",
        }
    }

    /// What it means, for a person.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Unknown | Self::Other(_) => "the device did not say",
            Self::PowerOn => "power was applied (on an RP2040 also a supply dip)",
            Self::Brownout => "the supply dipped (brown-out)",
            Self::RunPin => "the reset pin was pulled",
            Self::Watchdog => "the firmware hung and the watchdog restarted it",
            Self::Software => "a restart was asked for (reboot, firmware update)",
            Self::Debugger => "a debugger reset it",
            Self::Glitch => "the glitch detector fired",
        }
    }
}

/// `EXT_RESET_INFO` IN, 20 bytes (reset_info.h).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResetInfo {
    pub reason: ResetReason,
    /// The same as `CoreVregStatus::hang_count`.
    pub hang_count: u8,
    pub chip: Chip,
    pub uptime_s: u32,
    /// How long the run before this reset lasted (1 s steps). `None` after a
    /// power-on or brown-out, which clear the record.
    pub prev_uptime_s: Option<u32>,
    /// Raw `CHIP_RESET` at boot (VREG_AND_CHIP_RESET on RP2040, POWMAN on
    /// RP2350), for a support report.
    pub chip_reset: u32,
    /// Raw `WATCHDOG.REASON` at boot; bit 8 marks the firmware's own timeout.
    pub watchdog_reason: u32,
}

impl ResetInfo {
    pub const SIZE: usize = 20;
    /// `flags` bit 0: `prev_uptime_s` is valid.
    pub const FLAG_PREV_UPTIME: u8 = 0x01;

    pub fn decode(b: &[u8]) -> Option<Self> {
        let b = b.get(..Self::SIZE)?;
        let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        Some(Self {
            reason: ResetReason::from_raw(b[0]),
            prev_uptime_s: (b[1] & Self::FLAG_PREV_UPTIME != 0).then(|| u32_at(8)),
            hang_count: b[2],
            chip: Chip::from_raw(b[3]),
            uptime_s: u32_at(4),
            chip_reset: u32_at(12),
            watchdog_reason: u32_at(16),
        })
    }

    /// Readable `key value` pairs.
    pub fn fields(&self) -> Vec<(&'static str, String)> {
        vec![
            ("reason", self.reason.as_str().into()),
            ("uptime_s", self.uptime_s.to_string()),
            (
                "prev_uptime_s",
                self.prev_uptime_s
                    .map_or_else(|| "n/a".into(), |s| s.to_string()),
            ),
            ("hang_count", self.hang_count.to_string()),
            ("chip", self.chip.label()),
            ("chip_reset", format!("0x{:08X}", self.chip_reset)),
            ("watchdog_reason", format!("0x{:03X}", self.watchdog_reason)),
        ]
    }
}

/// WP FWX: the codecs against the firmware's layouts.
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_fwx {
    use super::*;
    use crate::generated::ext as g;

    const NOTIFY_H: &str = include_str!("../../firmware/notify.h");
    const CONFIG_H: &str = include_str!("../../firmware/config.h");

    #[test]
    fn wp_fwx_constants_match_the_vendored_headers() {
        assert!(NOTIFY_H.contains("#define NOTIFY_FEAT_CLIP_EVENT       (1u << 1)"));
        assert_eq!(NOTIFY_FEAT_CLIP_EVENT, 2);
        assert_eq!(NOTIFY_EVT_EXT_CLIP, 0xE0);
        assert_eq!(CLIP_MIN_INTERVAL_MS, 250);
        assert_eq!(g::EXT_IDENTIFY, 0x0004);
        assert_eq!(g::EXT_RESET_INFO, 0x0005);
        assert!(CONFIG_H.contains("IdentifyStatus (56 B)"));
        assert!(CONFIG_H.contains("ResetInfo (20 B)"));
    }

    #[test]
    fn wp_fwx_the_clip_event_decodes_only_behind_the_feature_bit() {
        // [channel, count, 0, 0, mask]: outputs 1 and 4 of an RP2350 (8 in).
        let payload = [9, 2, 0, 0, 0x00, 0x12, 0x00, 0x00];
        let want = ClipEvent {
            channel: 9,
            mask: (1 << 9) | (1 << 12),
        };
        assert_eq!(clip_event(Some(0b11), 0xE0, &payload), Some(want));
        assert_eq!(want.channels().collect::<Vec<_>>(), vec![9, 12]);
        // No bit, no features, another id: not a clip event.
        assert_eq!(clip_event(Some(0b01), 0xE0, &payload), None);
        assert_eq!(clip_event(None, 0xE0, &payload), None);
        assert_eq!(clip_event(Some(u32::MAX), 0xE1, &payload), None);
        // Short or empty: nothing, never a panic.
        assert_eq!(clip_event(Some(2), 0xE0, &payload[..7]), None);
        assert_eq!(clip_event(Some(2), 0xE0, &[]), None);
        assert_eq!(clip_event(Some(2), 0xE0, &[0; 8]), None);
        // The mask decides the channel.
        let odd = ClipEvent::decode(&[0, 0, 0, 0, 0, 0, 0, 0x80]).unwrap();
        assert_eq!((odd.channel, odd.mask), (31, 1 << 31));
    }

    #[test]
    fn wp_fwx_identify_request_and_status() {
        let r = IdentifyRequest {
            seconds: 10,
            tone_output: None,
        };
        assert_eq!(r.encode(), [10, 0xFF, 0, 0]);
        let r = IdentifyRequest {
            seconds: 0,
            tone_output: Some(3),
        };
        assert_eq!(r.encode(), [0, 3, 0, 0]);

        let mut b = vec![7, 0xFF, 1, 2, 0x27, 0x49, 0x00, 0x20];
        b.extend(
            b"pico2"
                .iter()
                .copied()
                .chain(std::iter::repeat(0))
                .take(16),
        );
        b.extend(
            b"Board rev A"
                .iter()
                .copied()
                .chain(std::iter::repeat(0))
                .take(32),
        );
        assert_eq!(b.len(), IdentifyStatus::SIZE);
        let s = IdentifyStatus::decode(&b).unwrap();
        assert_eq!(s.remaining_s, 7);
        assert_eq!(s.last_tone, None);
        assert_eq!(s.chip, Chip::Rp2350);
        assert_eq!(s.chip_rev, 2);
        assert_eq!(s.chip_id, 0x2000_4927);
        assert_eq!(s.board, "pico2");
        assert_eq!(s.hw_rev, "Board rev A");
        assert!(IdentifyStatus::decode(&b[..55]).is_none());
        // Unprintable bytes and a missing NUL never panic.
        let mut hostile = b.clone();
        hostile[8..24].fill(0x07);
        hostile[24..56].fill(b'x');
        let s = IdentifyStatus::decode(&hostile).unwrap();
        assert_eq!(s.board, "?".repeat(16));
        assert_eq!(s.hw_rev.len(), 32);
        hostile[1] = 4;
        assert_eq!(IdentifyStatus::decode(&hostile).unwrap().last_tone, Some(4));
    }

    #[test]
    fn wp_fwx_reset_info() {
        let mut b = vec![4, 1, 1, 1];
        b.extend(12u32.to_le_bytes());
        b.extend(3600u32.to_le_bytes());
        b.extend(0x0100_0000u32.to_le_bytes());
        b.extend(0x101u32.to_le_bytes());
        let r = ResetInfo::decode(&b).unwrap();
        assert_eq!(r.reason, ResetReason::Watchdog);
        assert_eq!(r.prev_uptime_s, Some(3600));
        assert_eq!(r.hang_count, 1);
        assert_eq!(r.chip, Chip::Rp2350);
        assert_eq!(r.uptime_s, 12);
        assert_eq!(r.chip_reset, 0x0100_0000);
        assert_eq!(r.watchdog_reason, 0x101);
        // A power-on clears the record: no previous uptime.
        b[0] = 1;
        b[1] = 0;
        let r = ResetInfo::decode(&b).unwrap();
        assert_eq!(r.reason, ResetReason::PowerOn);
        assert_eq!(r.prev_uptime_s, None);
        assert!(ResetInfo::decode(&b[..19]).is_none());
        for raw in 0..=255u8 {
            assert_eq!(ResetReason::from_raw(raw).raw(), raw);
        }
        assert_eq!(ResetReason::from_raw(9), ResetReason::Other(9));
        assert_eq!(ResetReason::Other(9).as_str(), "unknown");
    }
}
