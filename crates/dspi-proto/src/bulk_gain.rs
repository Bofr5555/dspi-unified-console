//! The firmware's bulk-SET dB bug, and the host-side repair (WP BULKFIX).
//!
//! DSPi firmware up to and including release v1.1.6 (and `bofr-main` before
//! FWX's FW2) converts dB to linear in `bulk_params.c` with a 4-term Taylor
//! series of `e^x`. After any bulk SET (`0xA1`, or chunked `0xA3`) the
//! preamps, crosspoints and output gains *play* that value until the next
//! preset load or reboot, while the image keeps (and reads back) the dB that
//! was asked for: −40 dB plays at about +19.5 dB, −60 dB at about +35 dB.
//! The single-parameter SETs (`0xD0`, `0x70`, `0x74`) and preset load use
//! `powf` and are right.
//!
//! The repair, for firmware that does not report the fix
//! ([`EXT_FIRMWARE_FIXES`], [`FIX_BULK_DB_POWF`]):
//! 1. send the image with every output muted ([`GainRepair::muted`]), so the
//!    wrong gains it installs play into muted outputs only;
//! 2. wait until the device has applied it (a bulk read is refused until
//!    then) and check the read-back has the outputs muted;
//! 3. re-send each gain the series gets wrong through its single SET
//!    ([`GainRepair::gains`]), which converts with `powf`;
//! 4. only then unmute the outputs the image wants unmuted
//!    ([`GainRepair::unmutes`]).
//!
//! Pure: this module plans the requests. The I/O is
//! `dspi_session::bulk_gain`, shared by the core and the hub.

use crate::generated::{self, opcodes as op};
use crate::packets::MatrixRoutePacket;
use crate::wire::WireHeader;

/// Proposed extension sub-command (`REQ_EXTENSION` `wValue`): IN, a `uint32`
/// LE of firmware fixes a host may rely on. Proposed to `wp/FWX` for FW2; not
/// yet in the vendored headers, so it is spelled here.
pub const EXT_FIRMWARE_FIXES: u16 = 0x0006;
/// `EXT_FIRMWARE_FIXES` bit 0: a bulk SET converts dB with `powf`, so a host
/// need not repair the gains after one.
pub const FIX_BULK_DB_POWF: u32 = 1 << 0;

/// A gain the series misses by at most this much is left as the bulk SET put
/// it. Everything from about −5.2 to +6.6 dB is within it (and 0 dB is exact).
pub const TOLERANCE_DB: f32 = 0.01;

/// `WireCrosspoint`: enabled, phase invert, 2 reserved, gain f32.
const XP_LEN: usize = 8;
const XP_GAIN: usize = 4;
/// `WireOutputChannel`: enabled, mute, 2 reserved, gain f32, delay f32.
const OUT_LEN: usize = 12;
const OUT_ENABLED: usize = 0;
const OUT_MUTE: usize = 1;
const OUT_GAIN: usize = 4;

/// `db_to_linear` of `bulk_params.c` in v1.1.6, operation for operation in
/// `f32`, so a fake device can play what the firmware plays.
pub fn firmware_bulk_linear(db: f32) -> f32 {
    if db == 0.0 {
        return 1.0;
    }
    // NaN stays NaN, as through the firmware's two comparisons.
    let db = db.clamp(-60.0, 20.0);
    let x = db * 0.115_129_25_f32;
    let linear = 1.0 + x + x * x * 0.5 + x * x * x * 0.166_666_7 + x * x * x * x * 0.041_666_7;
    if linear < 0.0 { 0.0 } else { linear }
}

/// What a bulk SET of `db` plays on buggy firmware, in dB.
pub fn firmware_bulk_db(db: f32) -> f32 {
    20.0 * firmware_bulk_linear(db).log10()
}

/// Whether a bulk SET of `db` plays it to within [`TOLERANCE_DB`]. False for
/// a value that is not finite.
pub fn bulk_plays_right(db: f32) -> bool {
    (firmware_bulk_db(db) - db).abs() <= TOLERANCE_DB
}

/// One OUT request: opcode, `wValue`, payload.
#[derive(Debug, Clone, PartialEq)]
pub struct SetWrite {
    /// `bRequest`.
    pub opcode: u8,
    /// `wValue`.
    pub value: u16,
    /// The data stage.
    pub payload: Vec<u8>,
}

/// The requests that make a bulk SET safe on firmware with the bug.
#[derive(Debug, Clone, PartialEq)]
pub struct GainRepair {
    /// The image to send instead: the requested one with every output muted.
    pub muted: Vec<u8>,
    /// The device's output count (header), the outputs [`GainRepair::muted`]
    /// mutes and a read-back must show muted.
    pub outputs: usize,
    /// Single SETs for each preamp, crosspoint and output gain the series
    /// gets wrong, in that order.
    pub gains: Vec<SetWrite>,
    /// `REQ_SET_OUTPUT_MUTE` 0 for each output the requested image has
    /// unmuted, sent last.
    pub unmutes: Vec<SetWrite>,
}

fn f32_at(b: &[u8], at: usize) -> Option<f32> {
    let end = at.checked_add(4)?;
    let v: [u8; 4] = b.get(at..end)?.try_into().ok()?;
    Some(f32::from_le_bytes(v))
}

/// Plan the repair of `image`, a bulk SET payload. `None` when it is not a
/// whole image in this build's wire format: the firmware refuses those
/// (`bulk_params_apply` checks the version and the length), so nothing is
/// converted and nothing needs repair. The firmware's own channel counts are
/// in the header, and only those channels are planned.
pub fn plan(image: &[u8]) -> Option<GainRepair> {
    if image.len() != generated::BULK_SIZE {
        return None;
    }
    let header = WireHeader::decode(image).ok()?;
    if usize::from(header.payload_length) != image.len() {
        return None;
    }
    let inputs = usize::from(header.num_input_channels)
        .min(usize::from(generated::wire::WIRE_MAX_INPUT_CHANNELS));
    let outputs = usize::from(header.num_output_channels)
        .min(usize::from(generated::wire::WIRE_MAX_OUTPUT_CHANNELS));
    let max_outputs = usize::from(generated::wire::WIRE_MAX_OUTPUT_CHANNELS);

    let mut muted = image.to_vec();
    let mut gains = Vec::new();
    let mut unmutes = Vec::new();

    // Per-channel preamp: `bulk_params_apply` applies it last, over the
    // legacy global field, so it is what plays.
    for ch in 0..inputs {
        let at = generated::OFF_PREAMP.checked_add(ch.checked_mul(4)?)?;
        let db = f32_at(image, at)?;
        if !bulk_plays_right(db) {
            gains.push(SetWrite {
                opcode: op::REQ_SET_PREAMP_CH,
                value: u16::try_from(ch).ok()?,
                payload: db.to_le_bytes().to_vec(),
            });
        }
    }
    // Crosspoints, `crosspoints[input][output]` row-major over the wire's
    // maximum outputs. `REQ_SET_MATRIX_ROUTE` also writes enabled and phase,
    // so they are sent as the image has them.
    for input in 0..inputs {
        for output in 0..outputs {
            let k = input.checked_mul(max_outputs)?.checked_add(output)?;
            let at = generated::OFF_CROSSPOINTS.checked_add(k.checked_mul(XP_LEN)?)?;
            let db = f32_at(image, at.checked_add(XP_GAIN)?)?;
            if bulk_plays_right(db) {
                continue;
            }
            let packet = MatrixRoutePacket {
                input: u8::try_from(input).ok()?,
                output: u8::try_from(output).ok()?,
                enabled: *image.get(at)? != 0,
                phase_invert: *image.get(at.checked_add(1)?)? != 0,
                gain_db: db,
            };
            gains.push(SetWrite {
                opcode: op::REQ_SET_MATRIX_ROUTE,
                value: 0,
                payload: packet.encode().to_vec(),
            });
        }
    }
    // Outputs: the gain of a disabled output too, since enabling one later
    // (`REQ_SET_OUTPUT_ENABLE`) does not convert its gain again.
    for o in 0..outputs {
        let at = generated::OFF_OUTPUTS.checked_add(o.checked_mul(OUT_LEN)?)?;
        let db = f32_at(image, at.checked_add(OUT_GAIN)?)?;
        let value = u16::try_from(o).ok()?;
        if !bulk_plays_right(db) {
            gains.push(SetWrite {
                opcode: op::REQ_SET_OUTPUT_GAIN,
                value,
                payload: db.to_le_bytes().to_vec(),
            });
        }
        let mute_at = at.checked_add(OUT_MUTE)?;
        if *image.get(mute_at)? == 0 {
            unmutes.push(SetWrite {
                opcode: op::REQ_SET_OUTPUT_MUTE,
                value,
                payload: vec![0],
            });
        }
        *muted.get_mut(mute_at)? = 1;
    }
    Some(GainRepair {
        muted,
        outputs,
        gains,
        unmutes,
    })
}

/// Whether `image` (a read-back) has outputs `0..outputs` muted: the image
/// [`GainRepair::muted`] has been applied, not an older one.
pub fn outputs_muted(image: &[u8], outputs: usize) -> bool {
    (0..outputs).all(|o| {
        o.checked_mul(OUT_LEN)
            .and_then(|k| generated::OFF_OUTPUTS.checked_add(k))
            .and_then(|at| at.checked_add(OUT_MUTE))
            .and_then(|at| image.get(at))
            .is_some_and(|m| *m != 0)
    })
}

/// Whether output `o` of `image` is enabled (for a fake device's model).
pub fn output_enabled(image: &[u8], o: usize) -> bool {
    o.checked_mul(OUT_LEN)
        .and_then(|k| generated::OFF_OUTPUTS.checked_add(k))
        .and_then(|at| at.checked_add(OUT_ENABLED))
        .and_then(|at| image.get(at))
        .is_some_and(|m| *m != 0)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_bulkfix {
    use super::*;
    use crate::wire::BulkPacket;

    /// A V32 image: 2 inputs, 9 outputs (RP2350), every gain 0 dB, outputs
    /// 0-3 enabled and unmuted.
    fn image() -> Vec<u8> {
        let mut b = vec![0u8; generated::BULK_SIZE];
        b[0] = generated::wire::WIRE_FORMAT_VERSION as u8;
        b[1] = 1;
        b[2] = 11;
        b[3] = 9;
        b[4] = 2;
        b[6..8].copy_from_slice(&(generated::BULK_SIZE as u16).to_le_bytes());
        for o in 0..4 {
            b[generated::OFF_OUTPUTS + o * OUT_LEN] = 1;
        }
        BulkPacket::decode(b.clone()).unwrap();
        b
    }

    fn put(b: &mut [u8], at: usize, v: f32) {
        b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    #[test]
    fn wp_bulkfix_the_series_matches_the_firmware_report() {
        // FWX REPORT section 7: what the bug plays.
        for (asked, plays) in [
            (-60.0, 35.3),
            (-40.0, 19.5),
            (-30.0, 8.2),
            (-20.0, -6.3),
            (-12.0, -10.9),
        ] {
            let got = firmware_bulk_db(asked);
            assert!(
                (got - plays).abs() < 0.1,
                "{asked} dB plays {got}, not {plays}"
            );
        }
        assert_eq!(firmware_bulk_linear(0.0), 1.0);
        assert!(bulk_plays_right(0.0) && bulk_plays_right(0.7) && bulk_plays_right(-3.0));
        assert!(!bulk_plays_right(-6.0) && !bulk_plays_right(-60.0) && !bulk_plays_right(12.0));
        assert!(!bulk_plays_right(f32::NAN));
    }

    #[test]
    fn wp_bulkfix_a_plain_image_only_mutes_and_unmutes() {
        let img = image();
        let r = plan(&img).unwrap();
        assert!(r.gains.is_empty());
        assert_eq!(r.outputs, 9);
        assert!(outputs_muted(&r.muted, 9));
        assert!(!outputs_muted(&img, 9));
        let unmuted: Vec<u16> = r.unmutes.iter().map(|w| w.value).collect();
        assert_eq!(unmuted, (0..9).collect::<Vec<_>>());
        // Nothing else changed.
        let diff = img.iter().zip(&r.muted).filter(|(a, b)| a != b).count();
        assert_eq!(diff, 9);
    }

    #[test]
    fn wp_bulkfix_wrong_gains_are_planned_and_muted_outputs_stay_muted() {
        let mut img = image();
        put(&mut img, generated::OFF_PREAMP + 4, -20.0);
        let xp = generated::OFF_CROSSPOINTS + (9 + 2) * XP_LEN; // in 1 -> out 2
        img[xp] = 1;
        img[xp + 1] = 1;
        put(&mut img, xp + XP_GAIN, -60.0);
        put(
            &mut img,
            generated::OFF_OUTPUTS + 3 * OUT_LEN + OUT_GAIN,
            -6.0,
        );
        img[generated::OFF_OUTPUTS + 5 * OUT_LEN + OUT_MUTE] = 1;
        // Channels past the header's counts are the firmware's to ignore.
        put(&mut img, generated::OFF_PREAMP + 7 * 4, -40.0);

        let r = plan(&img).unwrap();
        assert_eq!(
            r.gains,
            vec![
                SetWrite {
                    opcode: op::REQ_SET_PREAMP_CH,
                    value: 1,
                    payload: (-20.0f32).to_le_bytes().to_vec(),
                },
                SetWrite {
                    opcode: op::REQ_SET_MATRIX_ROUTE,
                    value: 0,
                    payload: MatrixRoutePacket {
                        input: 1,
                        output: 2,
                        enabled: true,
                        phase_invert: true,
                        gain_db: -60.0,
                    }
                    .encode()
                    .to_vec(),
                },
                SetWrite {
                    opcode: op::REQ_SET_OUTPUT_GAIN,
                    value: 3,
                    payload: (-6.0f32).to_le_bytes().to_vec(),
                },
            ]
        );
        assert!(!r.unmutes.iter().any(|w| w.value == 5));
        assert_eq!(r.unmutes.len(), 8);
    }

    #[test]
    fn wp_bulkfix_what_the_firmware_refuses_is_not_planned() {
        let img = image();
        assert!(plan(&img[..100]).is_none());
        let mut old = img.clone();
        old[0] = 31;
        assert!(plan(&old).is_none());
        let mut torn = img;
        torn[6] = 0;
        assert!(plan(&torn).is_none());
        assert!(!outputs_muted(&[], 1));
    }
}
