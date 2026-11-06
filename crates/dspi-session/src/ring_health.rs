//! Output ring health: the rings' counters and watermarks, and the fill bands
//! scaled to the live latency target (WP CG12).
//!
//! Lives in the session because the Terminal does not link `dspi-core`, which
//! wraps these reads for C and UniFFI. Reference: the Console's
//! `StatsView.swift` (`RingHealth`, `fetchRingHealth`, `BufferFillThresholds`).
//!
//! `REQ_OUTPUT_LATENCY` status is the gate. Firmware without it has no such
//! section ([`read_ring_health`] answers `None`), and its fill percentages are
//! not on the ring's scale. A short status reply reads the same way, as the
//! feature probe treats one. Any other failure is an error, so a dropped
//! connection does not look like older firmware.
//!
//! The counters are read only after the gate passed, and one that cannot be
//! read fails the whole read. The Console shows 0 for it, which would report a
//! healthy ring on a read that told nothing.
//!
//! The reference's `wIndex` 2 on the status request is the vendor interface,
//! which the USB transport always sends; other links have no such field.

#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
#![warn(missing_docs)]

use dspi_proto::generated::opcodes as op;
use dspi_proto::packets::BufferStatsPacket;
use dspi_transport::{Result, Transport, TransportError};

/// `REQ_GET_STATUS`: output ring underrun episodes, cumulative.
pub const STATUS_UNDERRUN_EPISODES: u16 = 26;
/// `REQ_GET_STATUS`: blocks refused by ring admission, cumulative.
pub const STATUS_REFUSED_BLOCKS: u16 = 27;
/// `REQ_GET_STATUS`: lowest pre-write ring fill in frames, a watermark;
/// [`LOWEST_FILL_NONE`] before the first write.
pub const STATUS_LOWEST_FILL: u16 = 28;
/// `REQ_GET_STATUS`: worst input delivery gap in µs, a watermark.
pub const STATUS_WORST_INPUT_GAP: u16 = 29;
/// `REQ_GET_STATUS`: worst gap between audio USB packets in µs, a watermark.
pub const STATUS_WORST_USB_GAP: u16 = 30;
/// `REQ_GET_STATUS`: background steps over their budget, cumulative.
pub const STATUS_OVER_BUDGET_STEPS: u16 = 31;
/// [`STATUS_LOWEST_FILL`]'s value before the first write.
pub const LOWEST_FILL_NONE: u32 = u32::MAX;

/// The scale `REQ_GET_BUFFER_STATS` percentages are measured against, frames.
pub const SLOT_FILL_SCALE_FRAMES: u16 = 768;
/// Output ring size on firmware with the latency setting, frames.
pub const OUTPUT_RING_FRAMES: u16 = 512;
/// Frames ahead of each reader the firmware keeps stamped with silence. A ring
/// filled past `OUTPUT_RING_FRAMES - OUTPUT_RING_ERASE_AHEAD` cannot take a write.
pub const OUTPUT_RING_ERASE_AHEAD: u16 = 64;

/// `OUTPUT_LATENCY_GET_STATUS`, the status `wValue` of `REQ_OUTPUT_LATENCY`.
const LATENCY_GET_STATUS: u16 = 0;
/// `sizeof(OutputLatencyStatus)` (config.h).
const LATENCY_STATUS_LEN: u16 = 12;
/// Where the output entries of `BufferStatsPacket` sit, and their size.
const BUFFER_SPDIF_AT: usize = 4;
const BUFFER_SPDIF_LEN: usize = 8;
/// Output entries in a `BufferStatsPacket`.
const BUFFER_SPDIF_COUNT: u8 = 4;
/// Percent to ring-scale percent.
const RING_SCALE: f64 = SLOT_FILL_SCALE_FRAMES as f64 / OUTPUT_RING_FRAMES as f64;

/// The fill level the rings are kept at, from `REQ_OUTPUT_LATENCY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RingTarget {
    /// How full the rings are kept, frames.
    pub target_frames: u16,
    /// The same as a time, µs.
    pub target_us: u16,
    /// Frames a stream input delivers per call; the fill swings by about this.
    pub batch_frames: u16,
}

/// The output rings' counters and watermarks. The underrun, refused and
/// over-budget counts are cumulative; the rest clear on a buffer stats reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RingHealth {
    /// The fill target.
    pub target: RingTarget,
    /// Dry episodes while a producer owed audio. A paused host does not count.
    pub underrun_episodes: u32,
    /// Blocks refused because the rings had no room.
    pub refused_blocks: u32,
    /// Lowest fill seen just before a write, frames; `None` before the first.
    pub lowest_fill: Option<u32>,
    /// Longest wait between input deliveries, µs.
    pub worst_input_gap_us: u32,
    /// Longest gap between audio-carrying USB packets, µs.
    pub worst_usb_gap_us: u32,
    /// Background steps that overran their main-loop budget.
    pub over_budget_steps: u32,
}

/// How a fill level reads. The Console shows these green, yellow and red.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillBand {
    /// Within the expected swing.
    #[default]
    Good,
    /// Outside the expected swing.
    Warn,
    /// Empty, or too full to take a write.
    Bad,
}

/// One output's fill, as a Stats row shows it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct OutputFill {
    /// Fill to frame resolution, %: of the ring where the firmware has the
    /// latency setting (at most 100), else of the firmware's fixed scale (which
    /// can pass 100).
    pub fill_pct: f64,
    /// Lowest fill since the watermarks were reset, % on the same scale.
    pub min_pct: u8,
    /// Highest fill since the watermarks were reset, % on the same scale.
    pub max_pct: u8,
    /// The band of `fill_pct`.
    pub band: FillBand,
}

/// Read the rings' health, or `None` on firmware without the status.
///
/// # Errors
/// The transport's, including a stall or short reply on a counter.
pub fn read_ring_health(t: &mut dyn Transport) -> Result<Option<RingHealth>> {
    let Some(target) = read_ring_target(t)? else {
        return Ok(None);
    };
    Ok(Some(RingHealth {
        target,
        underrun_episodes: read_counter(t, STATUS_UNDERRUN_EPISODES)?,
        refused_blocks: read_counter(t, STATUS_REFUSED_BLOCKS)?,
        lowest_fill: Some(read_counter(t, STATUS_LOWEST_FILL)?).filter(|v| *v != LOWEST_FILL_NONE),
        worst_input_gap_us: read_counter(t, STATUS_WORST_INPUT_GAP)?,
        worst_usb_gap_us: read_counter(t, STATUS_WORST_USB_GAP)?,
        over_budget_steps: read_counter(t, STATUS_OVER_BUDGET_STEPS)?,
    }))
}

/// Read the fill target, or `None` on firmware without the status.
///
/// # Errors
/// The transport's, except a stall or a short reply.
pub fn read_ring_target(t: &mut dyn Transport) -> Result<Option<RingTarget>> {
    match t.control_in(
        op::REQ_OUTPUT_LATENCY,
        LATENCY_GET_STATUS,
        LATENCY_STATUS_LEN,
    ) {
        Ok(raw) => Ok(decode_target(&raw)),
        Err(TransportError::Stalled { .. } | TransportError::ShortRead { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Read the live outputs' fill levels, scaled and banded against the fill
/// target. Two transfers: the target, then `REQ_GET_BUFFER_STATS`.
///
/// # Errors
/// The transport's, including a stall or short reply on the buffer stats.
pub fn read_output_fills(t: &mut dyn Transport) -> Result<Vec<OutputFill>> {
    let target = read_ring_target(t)?;
    let want = u16::try_from(BufferStatsPacket::SIZE).unwrap_or(u16::MAX);
    let raw = t.control_in(op::REQ_GET_BUFFER_STATS, 0, want)?;
    if raw.len() < BufferStatsPacket::SIZE {
        return Err(TransportError::ShortRead {
            opcode: op::REQ_GET_BUFFER_STATS,
            wanted: BufferStatsPacket::SIZE,
            got: raw.len(),
        });
    }
    Ok(decode_output_fills(&raw, target.as_ref()))
}

/// The band of an output's fill, in percent as [`OutputFill::fill_pct`]: around
/// the live target where there is one, else the fixed bands of firmware whose
/// single target sat at 50% of the reported scale. A healthy ring swings from
/// about one delivery below the target to one above it, so good spans a quarter
/// of the target up to two deliveries over. A NaN is `Bad`.
pub fn fill_band(target: Option<&RingTarget>, fill_pct: f64) -> FillBand {
    if fill_pct.is_nan() {
        return FillBand::Bad;
    }
    let pct = fill_pct.clamp(0.0, 100.0);
    let Some(target) = target else {
        return match pct.floor() {
            v if v == 0.0 || v == 100.0 => FillBand::Bad,
            v if !(25.0..=75.0).contains(&v) => FillBand::Warn,
            _ => FillBand::Good,
        };
    };
    let frames = pct / 100.0 * f64::from(OUTPUT_RING_FRAMES);
    let writable = f64::from(OUTPUT_RING_FRAMES.saturating_sub(OUTPUT_RING_ERASE_AHEAD));
    let target_frames = f64::from(target.target_frames);
    if frames < 1.0 || frames >= writable {
        FillBand::Bad
    } else if frames < target_frames / 4.0
        || frames > target_frames + 2.0 * f64::from(target.batch_frames)
    {
        FillBand::Warn
    } else {
        FillBand::Good
    }
}

/// `OutputLatencyStatus` (config.h): the three fields a fill is judged by.
fn decode_target(raw: &[u8]) -> Option<RingTarget> {
    let status: &[u8; LATENCY_STATUS_LEN as usize] = raw
        .get(..usize::from(LATENCY_STATUS_LEN))?
        .try_into()
        .ok()?;
    let &[_, _, us0, us1, target0, target1, batch0, batch1, ..] = status;
    Some(RingTarget {
        target_frames: u16::from_le_bytes([target0, target1]),
        target_us: u16::from_le_bytes([us0, us1]),
        batch_frames: u16::from_le_bytes([batch0, batch1]),
    })
}

/// The live entries of a `BufferStatsPacket`. Read here rather than through the
/// packet's codec for bytes 6-7, the fill in 0.01%: the percentages move in
/// 6.25% steps (48 frames), which reads as an empty ring at a 32-frame target.
fn decode_output_fills(raw: &[u8], target: Option<&RingTarget>) -> Vec<OutputFill> {
    let live = raw.first().map_or(0, |n| (*n).min(BUFFER_SPDIF_COUNT));
    raw.get(BUFFER_SPDIF_AT..)
        .unwrap_or_default()
        .as_chunks::<BUFFER_SPDIF_LEN>()
        .0
        .iter()
        .take(usize::from(live))
        .map(|&[_, _, _, pct, min, max, centi0, centi1]| {
            let centi = u16::from_le_bytes([centi0, centi1]);
            let fill = |pct: u8, centi: u16| {
                // Firmware that predates the field leaves it zero.
                if centi == 0 && pct != 0 {
                    f64::from(pct)
                } else {
                    f64::from(centi) / 100.0
                }
            };
            let fill_pct = match target {
                Some(_) => fill(ring_pct(pct), ring_centi(centi)),
                None => fill(pct, centi),
            };
            let (min_pct, max_pct) = match target {
                Some(_) => (ring_pct(min), ring_pct(max)),
                None => (min, max),
            };
            OutputFill {
                fill_pct,
                min_pct,
                max_pct,
                band: fill_band(target, fill_pct),
            }
        })
        .collect()
}

/// A percentage of the firmware's fixed scale as one of the ring, at most 100.
fn ring_pct(pct: u8) -> u8 {
    // In 0..=100 by the `min`, so the cast keeps the value.
    (f64::from(pct) * RING_SCALE).round().min(100.0) as u8
}

/// [`ring_pct`] for 0.01% units.
fn ring_centi(centi: u16) -> u16 {
    // In 0..=10000 by the `min`, so the cast keeps the value.
    (f64::from(centi) * RING_SCALE).round().min(10_000.0) as u16
}

/// One `REQ_GET_STATUS` `u32`.
fn read_counter(t: &mut dyn Transport, which: u16) -> Result<u32> {
    let raw = t.control_in(op::REQ_GET_STATUS, which, 4)?;
    let bytes = raw.first_chunk::<4>().ok_or(TransportError::ShortRead {
        opcode: op::REQ_GET_STATUS,
        wanted: 4,
        got: raw.len(),
    })?;
    Ok(u32::from_le_bytes(*bytes))
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg12 {
    use dspi_transport::MockTransport;
    use dspi_transport::mock::{Direction, Reply};
    use proptest::prelude::*;

    use super::*;

    const TARGET: RingTarget = RingTarget {
        target_frames: 96,
        target_us: 2000,
        batch_frames: 48,
    };

    fn latency() -> Vec<u8> {
        vec![3, 3, 0xD0, 0x07, 96, 0, 48, 0, 0, 7, 0, 0]
    }

    fn counters(values: [u32; 6]) -> Reply {
        Reply::Sequence(
            values
                .iter()
                .map(|v| Reply::Data(v.to_le_bytes().to_vec()))
                .collect(),
        )
    }

    fn buffer_stats(entries: &[(u8, u8, u8, u16)]) -> Vec<u8> {
        let mut raw = vec![0u8; BufferStatsPacket::SIZE];
        raw[0] = entries.len() as u8;
        for (i, (pct, min, max, centi)) in entries.iter().enumerate() {
            let e = &mut raw[4 + i * 8..4 + (i + 1) * 8];
            e[3] = *pct;
            e[4] = *min;
            e[5] = *max;
            e[6..8].copy_from_slice(&centi.to_le_bytes());
        }
        raw
    }

    #[test]
    fn wp_cg12_reads_every_field_from_its_status() {
        let mut t = MockTransport::new()
            .data(op::REQ_OUTPUT_LATENCY, latency())
            .reply(op::REQ_GET_STATUS, counters([1, 2, 3, 4, 5, 6]));
        let log = t.log_handle();
        assert_eq!(
            read_ring_health(&mut t).unwrap(),
            Some(RingHealth {
                target: TARGET,
                underrun_episodes: 1,
                refused_blocks: 2,
                lowest_fill: Some(3),
                worst_input_gap_us: 4,
                worst_usb_gap_us: 5,
                over_budget_steps: 6,
            })
        );
        let sent: Vec<_> = log
            .lock()
            .unwrap()
            .iter()
            .map(|e| (e.direction, e.opcode, e.value))
            .collect();
        let status = |v| (Direction::In, op::REQ_GET_STATUS, v);
        assert_eq!(
            sent,
            [
                (Direction::In, op::REQ_OUTPUT_LATENCY, 0),
                status(26),
                status(27),
                status(28),
                status(29),
                status(30),
                status(31),
            ]
        );
    }

    #[test]
    fn wp_cg12_a_device_without_the_status_has_no_section() {
        let mut t = MockTransport::new();
        let log = t.log_handle();
        assert_eq!(read_ring_health(&mut t).unwrap(), None);
        // The counters are not asked for; on such firmware they answer 0.
        assert_eq!(log.lock().unwrap().len(), 1);
    }

    #[test]
    fn wp_cg12_lowest_fill_max_means_no_value_yet() {
        let mut t = MockTransport::new()
            .data(op::REQ_OUTPUT_LATENCY, latency())
            .reply(op::REQ_GET_STATUS, counters([0, 0, u32::MAX, 0, 0, 0]));
        let health = read_ring_health(&mut t).unwrap().unwrap();
        assert_eq!(health.lowest_fill, None);
    }

    #[test]
    fn wp_cg12_lowest_fill_zero_is_a_value() {
        let mut t = MockTransport::new()
            .data(op::REQ_OUTPUT_LATENCY, latency())
            .reply(op::REQ_GET_STATUS, counters([0; 6]));
        let health = read_ring_health(&mut t).unwrap().unwrap();
        assert_eq!(health.lowest_fill, Some(0));
    }

    #[test]
    fn wp_cg12_a_short_status_reply_reads_as_no_section() {
        let mut t = MockTransport::new()
            .data(op::REQ_OUTPUT_LATENCY, vec![0u8; 11])
            .data(op::REQ_GET_STATUS, vec![9u8; 4]);
        assert_eq!(read_ring_health(&mut t).unwrap(), None);
    }

    #[test]
    fn wp_cg12_a_short_counter_reply_is_an_error_not_a_zero() {
        let mut t = MockTransport::new()
            .data(op::REQ_OUTPUT_LATENCY, latency())
            .data(op::REQ_GET_STATUS, vec![1u8, 2, 3]);
        assert!(matches!(
            read_ring_health(&mut t),
            Err(TransportError::ShortRead { opcode, .. }) if opcode == op::REQ_GET_STATUS
        ));
    }

    #[test]
    fn wp_cg12_a_lost_device_is_not_older_firmware() {
        let mut t = MockTransport::new().reply(op::REQ_OUTPUT_LATENCY, Reply::Disconnect);
        assert!(matches!(
            read_ring_health(&mut t),
            Err(TransportError::Disconnected)
        ));
    }

    #[test]
    fn wp_cg12_a_counter_stall_fails_the_read() {
        let mut t = MockTransport::new()
            .data(op::REQ_OUTPUT_LATENCY, latency())
            .reply(op::REQ_GET_STATUS, Reply::Stall);
        assert!(matches!(
            read_ring_health(&mut t),
            Err(TransportError::Stalled { .. })
        ));
    }

    #[test]
    fn wp_cg12_bands_around_the_target() {
        // Reference: 96-frame target, 48-frame batch, on a 512-frame ring.
        let band = |frames: f64| fill_band(Some(&TARGET), frames / 512.0 * 100.0);
        assert_eq!(band(0.0), FillBand::Bad);
        assert_eq!(band(0.9), FillBand::Bad);
        assert_eq!(band(1.0), FillBand::Warn);
        assert_eq!(band(23.9), FillBand::Warn);
        assert_eq!(band(24.0), FillBand::Good);
        assert_eq!(band(192.0), FillBand::Good);
        assert_eq!(band(192.1), FillBand::Warn);
        assert_eq!(band(447.9), FillBand::Warn);
        assert_eq!(band(448.0), FillBand::Bad);
        assert_eq!(fill_band(Some(&TARGET), 150.0), FillBand::Bad);
        assert_eq!(fill_band(Some(&TARGET), -1.0), FillBand::Bad);
        assert_eq!(fill_band(Some(&TARGET), f64::NAN), FillBand::Bad);
    }

    #[test]
    fn wp_cg12_fixed_bands_without_a_target() {
        for (pct, want) in [
            (0.9, FillBand::Bad),
            (1.0, FillBand::Warn),
            (24.9, FillBand::Warn),
            (25.0, FillBand::Good),
            (75.9, FillBand::Good),
            (76.0, FillBand::Warn),
            (99.9, FillBand::Warn),
            (100.0, FillBand::Bad),
            // The firmware's scale can pass 100; the Console clamps first.
            (140.0, FillBand::Bad),
        ] {
            assert_eq!(fill_band(None, pct), want, "{pct}");
        }
        assert_eq!(fill_band(None, f64::NAN), FillBand::Bad);
    }

    #[test]
    fn wp_cg12_fills_are_rescaled_to_the_ring() {
        // 240 frames: 31.25% of 768, 46.875% of 512. Only the live two count.
        let raw = buffer_stats(&[(31, 25, 50, 3125), (90, 80, 100, 9000)]);
        let mut t = MockTransport::new()
            .data(op::REQ_OUTPUT_LATENCY, latency())
            .data(op::REQ_GET_BUFFER_STATS, raw);
        let fills = read_output_fills(&mut t).unwrap();
        assert_eq!(fills.len(), 2);
        assert_eq!(fills[0].fill_pct, 46.88);
        assert_eq!((fills[0].min_pct, fills[0].max_pct), (38, 75));
        assert_eq!(fills[0].band, FillBand::Warn);
        // 9000 × 1.5 clamps to a full ring; the watermarks clamp as well.
        assert_eq!(fills[1].fill_pct, 100.0);
        assert_eq!((fills[1].min_pct, fills[1].max_pct), (100, 100));
        assert_eq!(fills[1].band, FillBand::Bad);
    }

    #[test]
    fn wp_cg12_a_low_target_is_judged_to_the_frame_not_the_bucket() {
        // A 0.67 ms target: 32 frames, under one 48-frame bucket, so the coarse
        // percentage says 0 while the ring is exactly where it should be.
        let target = RingTarget {
            target_frames: 32,
            target_us: 667,
            batch_frames: 16,
        };
        let centi = (30u32 * 10_000 / 768) as u16;
        let raw = buffer_stats(&[(0, 0, 0, centi)]);
        let fills = decode_output_fills(&raw, Some(&target));
        assert_eq!(fills[0].band, FillBand::Good);
    }

    #[test]
    fn wp_cg12_no_target_keeps_the_firmware_scale() {
        let raw = buffer_stats(&[(50, 25, 75, 5000), (50, 25, 75, 14000)]);
        let mut t = MockTransport::new().data(op::REQ_GET_BUFFER_STATS, raw);
        let fills = read_output_fills(&mut t).unwrap();
        assert_eq!(fills[0].fill_pct, 50.0);
        assert_eq!((fills[0].min_pct, fills[0].max_pct), (25, 75));
        assert_eq!(fills[0].band, FillBand::Good);
        assert_eq!(fills[1].fill_pct, 140.0);
        assert_eq!(fills[1].band, FillBand::Bad);
    }

    #[test]
    fn wp_cg12_firmware_before_the_fine_fill_uses_the_percentage() {
        let raw = buffer_stats(&[(50, 0, 0, 0), (0, 0, 0, 0)]);
        let fills = decode_output_fills(&raw, None);
        assert_eq!(fills[0].fill_pct, 50.0);
        let fills = decode_output_fills(&raw, Some(&TARGET));
        assert_eq!(fills[0].fill_pct, 75.0);
        assert_eq!(fills[1].fill_pct, 0.0);
        assert_eq!(fills[1].band, FillBand::Bad);
    }

    #[test]
    fn wp_cg12_a_short_buffer_stats_reply_is_an_error() {
        let mut t = MockTransport::new()
            .data(op::REQ_OUTPUT_LATENCY, latency())
            .data(op::REQ_GET_BUFFER_STATS, vec![2u8; 43]);
        assert!(matches!(
            read_output_fills(&mut t),
            Err(TransportError::ShortRead { .. })
        ));
    }

    #[test]
    fn wp_cg12_a_large_output_count_is_clamped_to_the_packet() {
        let mut raw = buffer_stats(&[]);
        raw[0] = 200;
        assert_eq!(decode_output_fills(&raw, None).len(), 4);
    }

    proptest! {
        #[test]
        fn proptest_wp_cg12_decoding_survives_any_reply(
            raw in proptest::collection::vec(any::<u8>(), 0..64),
            has_target in any::<bool>(),
        ) {
            let target = has_target.then_some(TARGET);
            for fill in decode_output_fills(&raw, target.as_ref()) {
                prop_assert!(fill.fill_pct >= 0.0);
                if has_target {
                    prop_assert!(fill.fill_pct <= 100.0);
                }
            }
            let _ = decode_target(&raw);
        }
    }
}
