//! The mirror codec: the device's bulk image <-> [`DspState`].
//!
//! A port of the old desktop core's `protocol::decode_bulk` /
//! `encode_bulk`, on `dspi-proto`'s generated offsets instead of hand-copied
//! ones. The contract the rest of the core relies on:
//!
//! - `decode_bulk(image)` fills every field the image carries and keeps the
//!   image itself in `bulk_raw` (`bulk_valid` true for an exact V32 image);
//! - `encode_bulk(state)` starts from `state.bulk_raw` and writes every
//!   decoded field back, so `decode(encode(s)) == s` for every field (the old
//!   core's `every_decoded_field_is_encoded` test) and bytes this build does
//!   not model are preserved;
//! - fields the image does not carry (preset directory and names, Core 1
//!   mode, DAC mute support, control interfaces, subharm solo) are left
//!   alone by `decode_bulk`.
//!
//! The fixture `tests/fixtures/device/dspstate_v32.json` (the Android app's,
//! made by the old core from a real capture) is the expected decode of
//! `tests/fixtures/link/device/{platform,bulk_v32}.bin`.
//!
//! Owned by WP C0 (docs/core-merge/PLAN.md).
#![allow(unused_variables)]

use dspi_proto::wire::WireError;

use crate::records::DspState;

/// The state before any device: what `get_state` returns while
/// disconnected (the old core's `DspState::default()`, including the
/// upmixer, subharm and tube default tables).
pub fn default_state() -> DspState {
    todo!("WP C0")
}

/// Apply a `REQ_GET_PLATFORM` reply (4, 6 or 7 bytes) to `state`.
pub fn apply_platform(reply: &[u8], state: &mut DspState) -> Result<(), WireError> {
    todo!("WP C0")
}

/// Decode a V32-layout image into `state` (see the module docs). An image of
/// another wire format is normalised first by the caller
/// (`dspi_proto::wire_compat`), which also decides `bulk_valid`.
pub fn decode_bulk(image: &[u8], state: &mut DspState) -> Result<(), WireError> {
    todo!("WP C0")
}

/// The image for `state`: `state.bulk_raw` with every decoded field written
/// back.
pub fn encode_bulk(state: &DspState) -> Vec<u8> {
    todo!("WP C0")
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    #[test]
    #[ignore = "WP C0"]
    fn the_capture_decodes_to_the_fixture() {
        todo!("WP C0")
    }

    #[test]
    #[ignore = "WP C0"]
    fn every_decoded_field_is_encoded() {
        todo!("WP C0")
    }
}
