//! WP FWX: the fork's identify (`EXT_IDENTIFY`), reset info
//! (`EXT_RESET_INFO`) and clip event (`NOTIFY_EVT_EXT_CLIP`,
//! `NOTIFY_FEAT_CLIP_EVENT`), answered when [`super::FakeOptions::diag_ext`] is on,
//! as firmware from wp/FWX does (the fork's `extension_channel_spec.md` 2.4.1,
//! 2.5, 2.6).
//!
//! Not modelled: the LED itself; the identify tone (it is remembered as the
//! last tone, nothing plays, and the refusal while the signal generator runs
//! is not checked); the firmware's 250 ms clip rate limit (each
//! [`EXT_FAKE_CLIP`] is one event). The reset reason follows the simulated
//! core-voltage state: a watchdog hang after a [`super::VregSim::Hang`], else a
//! power-on.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::time::Duration;

use dspi_proto::ext::{IDENTIFY_MAX_S, IDENTIFY_TONE_NONE, NOTIFY_EVT_EXT_CLIP};
use dspi_proto::generated::{ext as gext, notify as nfy};

use super::State;
use crate::clock::Instant;

/// `REQ_EXTENSION` OUT sub-command only a [`super::FakeDevice`] answers, as
/// [`super::EXT_FAKE_VREG_SIM`] (WP FWX): a little-endian `u32` channel mask
/// that clipped. The mask is ORed into the `GET_STATUS` 9 clip flags, and with
/// [`super::FakeOptions::diag_ext`] on one clip event goes out. An empty mask stalls.
pub const EXT_FAKE_CLIP: u16 = 0xFFF6;

/// The chip id the fake reports: an RP2350 A2's `SYSINFO.CHIP_ID`.
const CHIP_ID_RP2350: u32 = 0x2000_4927;
/// The hardware revision string, as a board build sets `DSPI_HW_REV`.
pub const FAKE_HW_REV: &str = "Demo rev A";

/// The identify state.
#[derive(Debug, Default)]
pub(super) struct Diag {
    blink_until: Option<Instant>,
    last_tone: Option<u8>,
}

impl State {
    /// The `NOTIFY_FEAT_*` flags this fake reports, when any.
    pub(super) fn notify_features(&self) -> u32 {
        let mut f = 0;
        if self.options.bulk_source {
            f |= 1; // NOTIFY_FEAT_BULK_SOURCE
        }
        if self.options.diag_ext {
            f |= dspi_proto::ext::NOTIFY_FEAT_CLIP_EVENT;
        }
        // -- WP SW1 --
        if self.options.alink {
            f |= dspi_proto::ext::alink::NOTIFY_FEAT_ALINK_STATE
                | dspi_proto::ext::alink::NOTIFY_FEAT_APPLY_RESULT;
        }
        // -- WP AUSB1 --
        f |= self.alink_usb_features();
        // -- WP ATFAKE -- 0xE4, and 0xE2 on an S/PDIF unit's queue
        if self.tb_on() {
            f |= dspi_proto::ext::timebase::NOTIFY_FEAT_TIME_BASE
                | dspi_proto::ext::alink::NOTIFY_FEAT_APPLY_RESULT;
        }
        f
    }

    /// IN replies of the sub-commands this module answers.
    pub(super) fn get_diag(&self, value: u16) -> Option<Vec<u8>> {
        if !self.options.diag_ext {
            return None;
        }
        match value {
            gext::EXT_IDENTIFY => Some(self.identify_status()),
            gext::EXT_RESET_INFO => Some(self.reset_info()),
            _ => None,
        }
    }

    /// OUT: `Some(accepted)` for the sub-commands this module answers.
    pub(super) fn set_diag(&mut self, value: u16, data: &[u8]) -> Option<bool> {
        match value {
            gext::EXT_IDENTIFY if self.options.diag_ext => Some(self.identify(data)),
            EXT_FAKE_CLIP => Some(match *data {
                [a, b, c, d] => self.fake_clip(u32::from_le_bytes([a, b, c, d])),
                _ => false,
            }),
            _ => None,
        }
    }

    /// `[seconds, tone_output, 0, 0]`, refused as the firmware does: nonzero
    /// reserved bytes, a tone output past the outputs.
    fn identify(&mut self, data: &[u8]) -> bool {
        let [secs, tone, 0, 0] = *data else {
            return false;
        };
        if tone != IDENTIFY_TONE_NONE && tone >= self.num_outputs() {
            return false;
        }
        let secs = secs.min(IDENTIFY_MAX_S);
        self.diag.blink_until = (secs > 0)
            .then(|| Instant::now().checked_add(Duration::from_secs(u64::from(secs))))
            .flatten();
        if tone != IDENTIFY_TONE_NONE {
            self.diag.last_tone = Some(tone);
        }
        true
    }

    fn identify_status(&self) -> Vec<u8> {
        let left = self.diag.blink_until.map_or(0, |until| {
            let now = Instant::now();
            if until <= now {
                return 0;
            }
            // Rounded up, as identify.c does.
            let ms = until.saturating_duration_since(now).as_millis();
            u8::try_from(ms.div_ceil(1000)).unwrap_or(u8::MAX)
        });
        let mut b = vec![
            left,
            self.diag.last_tone.unwrap_or(IDENTIFY_TONE_NONE),
            1, // RP2350: the demo capture's platform
            2,
        ];
        b.extend(CHIP_ID_RP2350.to_le_bytes());
        b.extend(padded(b"pico2", 16));
        b.extend(padded(FAKE_HW_REV.as_bytes(), 32));
        b
    }

    fn reset_info(&self) -> Vec<u8> {
        // reset_info.h: 4 = WATCHDOG, 1 = POWER_ON; flags bit 0 = previous
        // uptime valid.
        let v = &self.vreg;
        let hung = v.hang_count > 0 || v.failed_trial != 0;
        let (reason, flags, prev) = if hung {
            (4u8, 1u8, v.failed_trial_uptime_s)
        } else {
            (1, 0, 0)
        };
        let mut b = vec![reason, flags, v.hang_count, 1];
        b.extend(self.vreg_uptime_s().to_le_bytes());
        b.extend(prev.to_le_bytes());
        // POWMAN.CHIP_RESET HAD_WATCHDOG_RESET_SWCORE or HAD_POR; REASON.
        let (chip_reset, wdt) = if hung {
            (0x0100_0000u32, 0x101u32)
        } else {
            (0x0001_0000, 0)
        };
        b.extend(chip_reset.to_le_bytes());
        b.extend(wdt.to_le_bytes());
        b
    }

    /// [`EXT_FAKE_CLIP`]: the clip flags latch the mask, and with the feature
    /// on the clip event goes out.
    fn fake_clip(&mut self, mask: u32) -> bool {
        if mask == 0 {
            return false;
        }
        let at = usize::from(self.num_channels())
            .saturating_mul(2)
            .saturating_add(2);
        if let Some(flags) = self.status.get_mut(at..at.saturating_add(4))
            && let Ok(old) = <[u8; 4]>::try_from(&*flags)
        {
            flags.copy_from_slice(&(u32::from_le_bytes(old) | mask).to_le_bytes());
        }
        if self.options.diag_ext {
            let channel = u8::try_from(mask.trailing_zeros()).unwrap_or(u8::MAX);
            let count = u8::try_from(mask.count_ones()).unwrap_or(u8::MAX);
            let mut p = vec![
                nfy::NOTIFY_V2_VERSION as u8,
                NOTIFY_EVT_EXT_CLIP,
                0,
                0,
                channel,
                count,
                0,
                0,
            ];
            p.extend(mask.to_le_bytes());
            self.push(p);
        }
        true
    }
}

// -- WP WB6F --
impl State {
    /// `REQ_CLEAR_CLIPS`: the clip flags latched so far (by [`EXT_FAKE_CLIP`]),
    /// as the firmware answers, and cleared, so a host's clip hold can end.
    pub(super) fn clear_clips(&mut self) -> Vec<u8> {
        let at = usize::from(self.num_channels())
            .saturating_mul(2)
            .saturating_add(2);
        match self.status.get_mut(at..at.saturating_add(4)) {
            Some(flags) => {
                let old = flags.to_vec();
                flags.fill(0);
                old
            }
            None => vec![0; 4],
        }
    }
}

fn padded(s: &[u8], n: usize) -> Vec<u8> {
    s.iter()
        .copied()
        .chain(std::iter::repeat(0))
        .take(n)
        .collect()
}

/// WP FWX: the fake answers as wp/FWX firmware does, and as 04122d3 without
/// the option.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_fwx {
    use super::super::{Capture, FakeDevice, FakeOptions};
    use super::*;
    use crate::{Transport, TransportError};
    use dspi_proto::generated::opcodes as op;

    fn device(diag_ext: bool) -> FakeDevice {
        FakeDevice::new(
            Capture::demo(),
            FakeOptions {
                extensions: true,
                bulk_source: true,
                diag_ext,
                ..FakeOptions::default()
            },
        )
    }

    #[test]
    fn wp_fwx_lists_and_reports_the_new_items() {
        let mut d = device(true);
        let list = d
            .control_in_upto(op::REQ_EXTENSION, gext::EXT_LIST, 64)
            .unwrap();
        assert_eq!(list, [1, 0, 2, 0, 3, 0, 4, 0, 5, 0]);
        let f = d
            .control_in(op::REQ_EXTENSION, gext::EXT_NOTIFY_FEATURES, 4)
            .unwrap();
        assert_eq!(f, [3, 0, 0, 0]);

        let s = d
            .control_in(op::REQ_EXTENSION, gext::EXT_IDENTIFY, 56)
            .unwrap();
        assert_eq!(s.len(), 56);
        assert_eq!((s[0], s[1], s[2]), (0, IDENTIFY_TONE_NONE, 1));
        assert_eq!(&s[24..24 + FAKE_HW_REV.len()], FAKE_HW_REV.as_bytes());
        d.control_out(op::REQ_EXTENSION, gext::EXT_IDENTIFY, &[200, 2, 0, 0])
            .unwrap();
        let s = d
            .control_in(op::REQ_EXTENSION, gext::EXT_IDENTIFY, 56)
            .unwrap();
        assert!(
            (119..=IDENTIFY_MAX_S).contains(&s[0]),
            "shortened to 120 s: {}",
            s[0]
        );
        assert_eq!(s[1], 2);
        d.control_out(op::REQ_EXTENSION, gext::EXT_IDENTIFY, &[0, 0xFF, 0, 0])
            .unwrap();
        let s = d
            .control_in(op::REQ_EXTENSION, gext::EXT_IDENTIFY, 56)
            .unwrap();
        assert_eq!((s[0], s[1]), (0, 2), "stopped; the last tone is kept");
        // Refusals: a tone past the 9 outputs, reserved bytes, a wrong length.
        for bad in [&[5u8, 9, 0, 0][..], &[5, 0xFF, 1, 0], &[5, 0xFF, 0]] {
            assert!(matches!(
                d.control_out(op::REQ_EXTENSION, gext::EXT_IDENTIFY, bad),
                Err(TransportError::Stalled { .. })
            ));
        }

        let r = d
            .control_in(op::REQ_EXTENSION, gext::EXT_RESET_INFO, 20)
            .unwrap();
        assert_eq!(
            (r.len(), r[0], r[1]),
            (20, 1, 0),
            "a power-on, no previous run"
        );
        assert!(
            d.control_out(op::REQ_EXTENSION, gext::EXT_RESET_INFO, &[0])
                .is_err()
        );
    }

    #[test]
    fn wp_fwx_the_clip_event_and_the_clip_flags() {
        let mut d = device(true);
        let mut src = d.notifications().unwrap();
        d.control_out(op::REQ_EXTENSION, EXT_FAKE_CLIP, &0x1200u32.to_le_bytes())
            .unwrap();
        let p = src.read(Duration::from_millis(100)).unwrap();
        assert_eq!(&p[..2], [2, NOTIFY_EVT_EXT_CLIP]);
        assert_eq!(&p[4..], [9, 2, 0, 0, 0x00, 0x12, 0, 0]);
        // The polled flags latch it too (17 channels: flags at 36).
        let st = d.control_in(op::REQ_GET_STATUS, 9, 41).unwrap();
        assert_eq!(&st[36..40], 0x1200u32.to_le_bytes());
        assert!(
            d.control_out(op::REQ_EXTENSION, EXT_FAKE_CLIP, &[0; 4])
                .is_err()
        );
    }

    #[test]
    fn wp_fwx_without_the_option_it_is_bofr_main_04122d3() {
        let mut d = device(false);
        let list = d
            .control_in_upto(op::REQ_EXTENSION, gext::EXT_LIST, 64)
            .unwrap();
        assert_eq!(list, [1, 0, 2, 0, 3, 0]);
        let f = d
            .control_in(op::REQ_EXTENSION, gext::EXT_NOTIFY_FEATURES, 4)
            .unwrap();
        assert_eq!(f, [1, 0, 0, 0]);
        for id in [gext::EXT_IDENTIFY, gext::EXT_RESET_INFO] {
            assert!(d.control_in(op::REQ_EXTENSION, id, 56).is_err());
        }
        assert!(
            d.control_out(op::REQ_EXTENSION, gext::EXT_IDENTIFY, &[5, 0xFF, 0, 0])
                .is_err()
        );
        // The clip sim still sets the flags, but no event goes out.
        let mut src = d.notifications().unwrap();
        d.control_out(op::REQ_EXTENSION, EXT_FAKE_CLIP, &1u32.to_le_bytes())
            .unwrap();
        let got = src.read(Duration::from_millis(50));
        assert!(
            !got.is_ok_and(|p| p.get(1) == Some(&NOTIFY_EVT_EXT_CLIP)),
            "no clip event"
        );
    }
}

// -- WP WB6F --
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_wb6f {
    use super::super::{Capture, FakeDevice, FakeOptions};
    use super::*;
    use crate::Transport;
    use dspi_proto::generated::opcodes as op;

    #[test]
    fn wp_wb6f_clear_clips_clears_what_a_fake_clip_latched() {
        let mut d = FakeDevice::new(
            Capture::demo(),
            FakeOptions {
                extensions: true,
                ..FakeOptions::default()
            },
        );
        let flags = |d: &mut FakeDevice| {
            let s = d.control_in_upto(op::REQ_GET_STATUS, 9, 256).unwrap();
            let at = (s.len() - 7) + 2;
            u32::from_le_bytes([s[at], s[at + 1], s[at + 2], s[at + 3]])
        };
        assert_eq!(flags(&mut d), 0);
        d.control_out(op::REQ_EXTENSION, EXT_FAKE_CLIP, &5u32.to_le_bytes())
            .unwrap();
        assert_eq!(flags(&mut d), 5, "latched");
        // The answer is the flags it cleared, as the firmware's.
        let r = d.control_in_upto(op::REQ_CLEAR_CLIPS, 0, 4).unwrap();
        assert_eq!(r, 5u32.to_le_bytes());
        assert_eq!(flags(&mut d), 0, "cleared");
        let r = d.control_in_upto(op::REQ_CLEAR_CLIPS, 0, 4).unwrap();
        assert_eq!(r, [0; 4]);
    }
}
