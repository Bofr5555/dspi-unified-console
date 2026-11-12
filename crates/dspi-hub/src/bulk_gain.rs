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
//! - `0xA1` carries the whole image: [`BulkRelay::take`] writes it repaired.
//! - `0xA3` chunks from a client are held here, answered OK, and written
//!   repaired when the last one arrives. Firmware with the fix gets them passed
//!   through as before. Another request in between drops what is held, as the
//!   firmware's chunk session would.
//!
//! The device worker's transport is wrapped by [`GainGuard::wrap`] (fixup bf
//! F1): after a repair that failed, any relayed unmute first re-sends the gains
//! feeding that output, or is refused. The guard also asks the firmware once per
//! connection whether it has the fix (F4).

use dspi_link::Dir;
use dspi_link::frames::CmdRequest;
use dspi_proto::bulk_gain::plan;
use dspi_proto::generated::{self as g, opcodes as op};
use dspi_proto::wire::WireHeader;
use dspi_session::bulk_gain::{GainGuard, all_outputs, repair};
use dspi_transport::{Transport, TransportError};

/// The largest image a client may stream: the firmware's bulk buffer is 8 KiB.
const MAX_IMAGE: usize = 8192;

/// Write `image` with `write` (the device's own bulk writer), repaired unless the
/// firmware reports the fix. A payload the firmware would refuse anyway (another
/// wire format, a torn image) goes as it is. A repaired write that fails leaves
/// every output suspect in `guard`.
pub(crate) fn write_repaired(
    t: &mut dyn Transport,
    guard: &GainGuard,
    image: &[u8],
    write: impl FnOnce(&mut dyn Transport, &[u8]) -> Result<(), TransportError>,
) -> Result<(), TransportError> {
    let plan = if guard.fixed(t, None) {
        None
    } else {
        plan(image)
    };
    match plan {
        None => write(t, image),
        Some(r) => {
            if let Err(e) = write(t, &r.muted) {
                // Possibly applied (a lost reply): the gains are unknown.
                guard.suspect(all_outputs(r.outputs));
                return Err(e);
            }
            repair(t, &r, Some(guard))
        }
    }
}

/// One device's relay of clients' bulk SETs.
#[derive(Default)]
pub(crate) struct BulkRelay {
    /// The device transport's guard (its fix answer, its suspect outputs).
    pub(crate) guard: GainGuard,
    /// A client's chunks so far, from offset 0; `None` when no session is held
    /// (none open, or one passed through to firmware with the fix).
    held: Option<Vec<u8>>,
}

impl BulkRelay {
    /// A relay over the transport `guard` wraps.
    pub(crate) fn new(guard: GainGuard) -> Self {
        Self { guard, held: None }
    }

    /// Take `req` if it is a bulk SET: `None` lets the caller run it as usual.
    /// A `0xA1` is written repaired. A held `0xA3` chunk answers OK at once; the
    /// last one answers with the repaired write's result. A chunk out of
    /// sequence drops the session and stalls, as the firmware does.
    pub(crate) fn take(
        &mut self,
        t: &mut dyn Transport,
        req: &CmdRequest,
        write: impl FnOnce(&mut dyn Transport, &[u8]) -> Result<(), TransportError>,
    ) -> Option<Result<Vec<u8>, TransportError>> {
        if req.dir == Dir::Set && req.b_request == op::REQ_SET_ALL_PARAMS {
            self.held = None;
            return Some(write_repaired(t, &self.guard, &req.payload, write).map(|()| Vec::new()));
        }
        if req.dir != Dir::Set || req.b_request != op::REQ_SET_ALL_PARAMS_CHUNK {
            self.held = None;
            return None;
        }
        let offset = usize::from(req.w_value);
        if offset == 0 {
            self.held = (!self.guard.fixed(t, None)).then(Vec::new);
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
        Some(write_repaired(t, &self.guard, &image, write).map(|()| Vec::new()))
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
    use dspi_transport::{BulkDbDevice, BulkDbOptions, FakeDevice, FakeRemote, GainProbe};

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

    fn write(t: &mut dyn Transport, img: &[u8]) -> Result<(), TransportError> {
        t.control_out(op::REQ_SET_ALL_PARAMS, 0, img)
    }

    struct Dev {
        t: Box<dyn Transport>,
        relay: BulkRelay,
        probe: GainProbe,
        remote: FakeRemote,
        /// The demo's image with a −60 dB crosspoint (input 0 to output 0,
        /// enabled) and a −40 dB output 0, enabled and unmuted.
        image: Vec<u8>,
    }

    /// The demo on (un)fixed firmware, behind the device worker's guard.
    fn device(fixed: bool) -> Dev {
        let d = BulkDbDevice::new(
            FakeDevice::demo(),
            BulkDbOptions {
                fixed,
                apply_delay: 2,
            },
        );
        let (probe, remote) = (d.probe(), d.remote());
        let mut image = remote.image();
        let xp = g::OFF_CROSSPOINTS;
        image[xp] = 1;
        image[xp + 4..xp + 8].copy_from_slice(&(-60.0f32).to_le_bytes());
        let out = g::OFF_OUTPUTS;
        image[out] = 1;
        image[out + 1] = 0;
        image[out + 4..out + 8].copy_from_slice(&(-40.0f32).to_le_bytes());
        let (t, guard) = GainGuard::wrap(Box::new(d), true);
        Dev {
            t,
            relay: BulkRelay::new(guard),
            probe,
            remote,
            image,
        }
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

    fn ext_lists(probe: &GainProbe) -> usize {
        probe
            .gets()
            .iter()
            .filter(|(o, v)| *o == op::REQ_EXTENSION && *v == g::ext::EXT_LIST)
            .count()
    }

    #[test]
    fn wp_bulkfix_a_relayed_single_shot_bulk_set_plays_right() {
        let mut d = device(false);
        let req = set(op::REQ_SET_ALL_PARAMS, 0, d.image.clone());
        d.relay.take(&mut *d.t, &req, write).unwrap().unwrap();
        plays_right(&d.probe);
        assert_eq!(
            d.remote.image()[g::OFF_OUTPUTS + 1],
            0,
            "output 0 unmuted again"
        );
        assert!(d.probe.sets().contains(&op::REQ_SET_MATRIX_ROUTE));
    }

    #[test]
    fn wp_bulkfix_a_clients_chunked_bulk_set_is_held_and_written_repaired() {
        let mut d = device(false);
        let parts: Vec<Vec<u8>> = d.image.chunks(2048).map(<[u8]>::to_vec).collect();
        for (k, part) in parts.iter().enumerate() {
            let req = set(
                op::REQ_SET_ALL_PARAMS_CHUNK,
                (k * 2048) as u16,
                part.clone(),
            );
            let got = d.relay.take(&mut *d.t, &req, write).expect("held");
            assert_eq!(got.unwrap(), Vec::<u8>::new());
            if k + 1 < parts.len() {
                assert!(
                    !d.probe.sets().contains(&op::REQ_SET_ALL_PARAMS),
                    "nothing reaches the device before the last chunk"
                );
            }
        }
        plays_right(&d.probe);
        assert_eq!(
            d.remote.image()[g::OFF_OUTPUTS + 4..g::OFF_OUTPUTS + 8],
            (-40.0f32).to_le_bytes()
        );
    }

    #[test]
    fn wp_bulkfix_a_chunk_session_is_dropped_by_another_request_or_a_gap() {
        let mut d = device(false);
        let image = d.image.clone();
        let first = set(op::REQ_SET_ALL_PARAMS_CHUNK, 0, image[..2048].to_vec());
        assert!(d.relay.take(&mut *d.t, &first, write).unwrap().is_ok());
        let other = set(
            op::REQ_SET_MASTER_VOLUME,
            0,
            (-20.0f32).to_le_bytes().to_vec(),
        );
        assert!(d.relay.take(&mut *d.t, &other, write).is_none());
        let next = set(
            op::REQ_SET_ALL_PARAMS_CHUNK,
            2048,
            image[2048..4096].to_vec(),
        );
        assert!(
            d.relay.take(&mut *d.t, &next, write).is_none(),
            "no session held: the device answers"
        );
        // A gap stalls, as the firmware does.
        assert!(d.relay.take(&mut *d.t, &first, write).unwrap().is_ok());
        let gap = set(op::REQ_SET_ALL_PARAMS_CHUNK, 4096, image[4096..].to_vec());
        assert!(matches!(
            d.relay.take(&mut *d.t, &gap, write),
            Some(Err(TransportError::Stalled { .. }))
        ));
        assert_eq!(d.probe.bulk_applies(), 0);
    }

    #[test]
    fn wp_bulkfix_firmware_with_the_fix_gets_the_chunks_passed_through() {
        let mut d = device(true);
        let image = d.image.clone();
        for (k, part) in image.chunks(2048).enumerate() {
            let req = set(
                op::REQ_SET_ALL_PARAMS_CHUNK,
                (k * 2048) as u16,
                part.to_vec(),
            );
            assert!(d.relay.take(&mut *d.t, &req, write).is_none());
            crate::device::execute(&mut *d.t, &req, false).unwrap();
        }
        let req = set(op::REQ_SET_ALL_PARAMS, 0, image);
        d.relay.take(&mut *d.t, &req, write).unwrap().unwrap();
        assert!(
            d.probe
                .sets()
                .iter()
                .all(|o| matches!(*o, op::REQ_SET_ALL_PARAMS_CHUNK | op::REQ_SET_ALL_PARAMS))
        );
        let _ = d.t.control_in(op::REQ_GET_PLATFORM, 0, 7);
        assert!(d.probe.mismatch_db() <= TOLERANCE_DB);
    }

    // -- fixup bf --

    #[test]
    fn wp_bulkfix_bf_a_relayed_unmute_after_a_failed_repair_resends_the_gains() {
        let mut d = device(false);
        d.probe.fail(op::REQ_SET_OUTPUT_GAIN, true);
        let req = set(op::REQ_SET_ALL_PARAMS, 0, d.image.clone());
        assert!(d.relay.take(&mut *d.t, &req, write).unwrap().is_err());
        assert!(
            d.probe.mismatch_db() > 50.0,
            "the series' −40 dB is still in"
        );
        // Refused while the gains cannot be re-sent...
        let unmute = set(op::REQ_SET_OUTPUT_MUTE, 0, vec![0]);
        assert!(crate::device::execute(&mut *d.t, &unmute, false).is_err());
        assert_eq!(d.remote.image()[g::OFF_OUTPUTS + 1], 1);
        // ...and re-sent first once they can.
        d.probe.fail(op::REQ_SET_OUTPUT_GAIN, false);
        crate::device::execute(&mut *d.t, &unmute, false).unwrap();
        assert_eq!(d.remote.image()[g::OFF_OUTPUTS + 1], 0);
        assert!(
            d.probe.worst_excess_db() <= TOLERANCE_DB,
            "{}",
            d.probe.worst_excess_db()
        );
        assert!(d.probe.mismatch_db() <= TOLERANCE_DB);
    }

    #[test]
    fn wp_bulkfix_bf_a_refused_image_over_an_all_muted_state_unmutes_nothing() {
        let mut d = device(false);
        // The user has every output muted.
        for o in 0..9 {
            d.remote.poke(g::OFF_OUTPUTS + o * 12 + 1, &[1], 0);
        }
        // A client's image for another platform: `bulk_params_apply` refuses it.
        let mut other = d.image.clone();
        other[1] ^= 1;
        let req = set(op::REQ_SET_ALL_PARAMS, 0, other);
        let r = d.relay.take(&mut *d.t, &req, write).unwrap();
        assert!(matches!(r, Err(TransportError::Malformed(_))), "{r:?}");
        assert_eq!(d.probe.bulk_applies(), 0);
        assert!(
            !d.probe.sets().contains(&op::REQ_SET_OUTPUT_MUTE),
            "nothing unmuted"
        );
        let img = d.remote.image();
        assert!((0..9).all(|o| img[g::OFF_OUTPUTS + o * 12 + 1] == 1));
    }

    #[test]
    fn wp_bulkfix_bf_the_hub_asks_for_the_fix_once_per_connection() {
        let mut d = device(false);
        for _ in 0..3 {
            let req = set(op::REQ_SET_ALL_PARAMS, 0, d.image.clone());
            d.relay.take(&mut *d.t, &req, write).unwrap().unwrap();
        }
        assert_eq!(
            ext_lists(&d.probe),
            1,
            "stock firmware stalls EXT_LIST once"
        );
        let mut d = device(true);
        for _ in 0..3 {
            let req = set(op::REQ_SET_ALL_PARAMS, 0, d.image.clone());
            d.relay.take(&mut *d.t, &req, write).unwrap().unwrap();
        }
        assert_eq!(ext_lists(&d.probe), 1);
    }
}
