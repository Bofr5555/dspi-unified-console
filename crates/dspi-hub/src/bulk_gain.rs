//! A client's bulk SET, made safe on firmware with the bulk dB bug (WP BULKFIX).
//!
//! Firmware up to v1.1.6 plays a bulk SET's preamp, crosspoint and output gains
//! through a Taylor series, so a deep cut plays loud (`dspi_proto::bulk_gain`).
//! The hub relays bulk SETs from every console, the released ones and other
//! people's included, so it repairs them here unless the firmware reports the
//! fix: the image goes to the device with its outputs muted, and once applied the
//! gains are re-sent one by one and the outputs unmuted
//! (`dspi_session::bulk_gain`).
//!
//! - `0xA1` carries the whole image: [`write_repaired`] does it in one call.
//! - `0xA3` chunks from a client ([`ChunkedSet`]) are held here, answered OK, and
//!   written repaired when the last one arrives. Firmware with the fix gets them
//!   passed through as before. Another request in between drops what is held, as
//!   the firmware's chunk session would.

use dspi_link::Dir;
use dspi_link::frames::CmdRequest;
use dspi_proto::bulk_gain::plan;
use dspi_proto::generated::{self as g, opcodes as op};
use dspi_proto::wire::WireHeader;
use dspi_session::bulk_gain::{fix_reported, repair};
use dspi_transport::{Transport, TransportError};

/// The largest image a client may stream: the firmware's bulk buffer is 8 KiB.
const MAX_IMAGE: usize = 8192;

/// Write `image` with `write` (the device's own bulk writer), repaired unless the
/// firmware reports the fix. A payload the firmware would refuse anyway (another
/// wire format, a torn image) goes as it is.
pub(crate) fn write_repaired(
    t: &mut dyn Transport,
    image: &[u8],
    write: impl FnOnce(&mut dyn Transport, &[u8]) -> Result<(), TransportError>,
) -> Result<(), TransportError> {
    let plan = if fix_reported(t) { None } else { plan(image) };
    match plan {
        None => write(t, image),
        Some(r) => {
            write(t, &r.muted)?;
            repair(t, &r)
        }
    }
}

/// A client's chunked bulk SET in progress.
#[derive(Debug, Default)]
pub(crate) struct ChunkedSet {
    /// The chunks so far, from offset 0; `None` when no session is held (none
    /// open, or one passed through to firmware with the fix).
    held: Option<Vec<u8>>,
}

impl ChunkedSet {
    /// Take `req` if it is a `0xA3` SET this hub holds: `None` lets the caller
    /// run it as usual. A held chunk answers OK at once; the last one answers with
    /// the repaired write's result. A chunk out of sequence drops the session and
    /// stalls, as the firmware does.
    pub(crate) fn take(
        &mut self,
        t: &mut dyn Transport,
        req: &CmdRequest,
        write: impl FnOnce(&mut dyn Transport, &[u8]) -> Result<(), TransportError>,
    ) -> Option<Result<Vec<u8>, TransportError>> {
        if req.dir != Dir::Set || req.b_request != op::REQ_SET_ALL_PARAMS_CHUNK {
            self.held = None;
            return None;
        }
        let offset = usize::from(req.w_value);
        if offset == 0 {
            self.held = (!fix_reported(t)).then(Vec::new);
        }
        let held = self.held.as_mut()?;
        let stall = Err(TransportError::Stalled {
            opcode: op::REQ_SET_ALL_PARAMS_CHUNK,
        });
        let end = offset.saturating_add(req.payload.len());
        if offset != held.len() || end > MAX_IMAGE {
            self.held = None;
            return Some(stall);
        }
        held.extend_from_slice(&req.payload);
        if held.len() < declared_len(held) {
            return Some(Ok(Vec::new()));
        }
        let image = self.held.take().unwrap_or_default();
        Some(write_repaired(t, &image, write).map(|()| Vec::new()))
    }
}

/// The image's length as its header states it (`payload_length`), this build's
/// size until the header is in or when it states something no image is.
fn declared_len(image: &[u8]) -> usize {
    match (image.get(6), image.get(7)) {
        (Some(lo), Some(hi)) => {
            let n = usize::from(u16::from_le_bytes([*lo, *hi]));
            if (WireHeader::LEN..=MAX_IMAGE).contains(&n) {
                n
            } else {
                g::BULK_SIZE
            }
        }
        _ => g::BULK_SIZE,
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_bulkfix {
    use super::*;
    use dspi_proto::VENDOR_INTERFACE;
    use dspi_proto::bulk_gain::TOLERANCE_DB;
    use dspi_transport::{BulkDbDevice, BulkDbOptions, FakeDevice, GainProbe};

    fn set(opcode: u8, value: u16, payload: Vec<u8>) -> CmdRequest {
        CmdRequest {
            tag: 1,
            handle: 0,
            dir: Dir::Set,
            b_request: opcode,
            w_value: value,
            w_index: VENDOR_INTERFACE,
            w_length: payload.len() as u16,
            payload,
        }
    }

    /// The demo on buggy firmware, and an image of it with a −60 dB crosspoint
    /// (input 0 to output 0, enabled) and a −40 dB output 0.
    fn device(fixed: bool) -> (BulkDbDevice, GainProbe, Vec<u8>) {
        let d = BulkDbDevice::new(
            FakeDevice::demo(),
            BulkDbOptions {
                fixed,
                apply_delay: 2,
            },
        );
        let probe = d.probe();
        let mut image = d.remote().image();
        let xp = g::OFF_CROSSPOINTS;
        image[xp] = 1;
        image[xp + 4..xp + 8].copy_from_slice(&(-60.0f32).to_le_bytes());
        let out = g::OFF_OUTPUTS;
        image[out] = 1;
        image[out + 1] = 0;
        image[out + 4..out + 8].copy_from_slice(&(-40.0f32).to_le_bytes());
        (d, probe, image)
    }

    fn plays_right(probe: &GainProbe) {
        assert_eq!(probe.bulk_applies(), 1);
        assert!(
            probe.worst_excess_db() <= TOLERANCE_DB,
            "{}",
            probe.worst_excess_db()
        );
        assert!(
            probe.mismatch_db() <= TOLERANCE_DB,
            "{}",
            probe.mismatch_db()
        );
    }

    #[test]
    fn wp_bulkfix_a_relayed_single_shot_bulk_set_plays_right() {
        let (mut d, probe, image) = device(false);
        crate::device::execute(
            &mut d,
            &set(op::REQ_SET_ALL_PARAMS, 0, image.clone()),
            false,
        )
        .unwrap();
        plays_right(&probe);
        assert_eq!(
            d.remote().image()[g::OFF_OUTPUTS + 1],
            0,
            "output 0 unmuted again"
        );
        assert!(probe.sets().contains(&op::REQ_SET_MATRIX_ROUTE));
    }

    #[test]
    fn wp_bulkfix_a_clients_chunked_bulk_set_is_held_and_written_repaired() {
        let (mut d, probe, image) = device(false);
        let mut chunks = ChunkedSet::default();
        let write =
            |t: &mut dyn Transport, img: &[u8]| t.control_out(op::REQ_SET_ALL_PARAMS, 0, img);
        let parts: Vec<&[u8]> = image.chunks(2048).collect();
        for (k, part) in parts.iter().enumerate() {
            let req = set(
                op::REQ_SET_ALL_PARAMS_CHUNK,
                (k * 2048) as u16,
                part.to_vec(),
            );
            let got = chunks.take(&mut d, &req, write).expect("held");
            assert_eq!(got.unwrap(), Vec::<u8>::new());
            if k + 1 < parts.len() {
                assert!(
                    !probe.sets().contains(&op::REQ_SET_ALL_PARAMS_CHUNK),
                    "nothing reaches the device before the last chunk"
                );
            }
        }
        plays_right(&probe);
        assert_eq!(
            d.remote().image()[g::OFF_OUTPUTS + 4..g::OFF_OUTPUTS + 8],
            (-40.0f32).to_le_bytes()
        );
    }

    #[test]
    fn wp_bulkfix_a_chunk_session_is_dropped_by_another_request_or_a_gap() {
        let (mut d, probe, image) = device(false);
        let mut chunks = ChunkedSet::default();
        let write =
            |t: &mut dyn Transport, img: &[u8]| t.control_out(op::REQ_SET_ALL_PARAMS, 0, img);
        let first = set(op::REQ_SET_ALL_PARAMS_CHUNK, 0, image[..2048].to_vec());
        assert!(chunks.take(&mut d, &first, write).unwrap().is_ok());
        let other = set(
            op::REQ_SET_MASTER_VOLUME,
            0,
            (-20.0f32).to_le_bytes().to_vec(),
        );
        assert!(chunks.take(&mut d, &other, write).is_none());
        let next = set(
            op::REQ_SET_ALL_PARAMS_CHUNK,
            2048,
            image[2048..4096].to_vec(),
        );
        assert!(
            chunks.take(&mut d, &next, write).is_none(),
            "no session held: the device answers"
        );
        // A gap stalls, as the firmware does.
        assert!(chunks.take(&mut d, &first, write).unwrap().is_ok());
        let gap = set(op::REQ_SET_ALL_PARAMS_CHUNK, 4096, image[4096..].to_vec());
        assert!(matches!(
            chunks.take(&mut d, &gap, write),
            Some(Err(TransportError::Stalled { .. }))
        ));
        assert_eq!(probe.bulk_applies(), 0);
    }

    #[test]
    fn wp_bulkfix_firmware_with_the_fix_gets_the_chunks_passed_through() {
        let (mut d, probe, image) = device(true);
        let mut chunks = ChunkedSet::default();
        let write =
            |t: &mut dyn Transport, img: &[u8]| t.control_out(op::REQ_SET_ALL_PARAMS, 0, img);
        for (k, part) in image.chunks(2048).enumerate() {
            let req = set(
                op::REQ_SET_ALL_PARAMS_CHUNK,
                (k * 2048) as u16,
                part.to_vec(),
            );
            assert!(chunks.take(&mut d, &req, write).is_none());
            crate::device::execute(&mut d, &req, false).unwrap();
        }
        crate::device::execute(&mut d, &set(op::REQ_SET_ALL_PARAMS, 0, image), false).unwrap();
        assert!(
            probe
                .sets()
                .iter()
                .all(|o| *o == op::REQ_SET_ALL_PARAMS_CHUNK)
        );
        let _ = d.control_in(op::REQ_GET_PLATFORM, 0, 7);
        assert!(probe.mismatch_db() <= TOLERANCE_DB);
    }
}
