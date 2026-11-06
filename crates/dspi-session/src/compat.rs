//! Reading a device whose firmware speaks another wire format, for display.
//!
//! [`dspi_proto::wire_compat`] maps the image; this reads it. The image of an
//! older firmware is shorter than 6136 bytes (a chunked read past its end
//! stalls), a newer one longer (stopping early leaves the device's snapshot
//! session open), so the length comes from the header
//! ([`dspi_proto::wire_compat::image_length`]). Over a link without chunked
//! bulk ([`dspi_transport::TransportCaps::chunked_bulk`]) the single-shot
//! `0xA0` is used instead.
//!
//! Owned by WP S2 (docs/core-merge/PLAN.md).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::wire_compat::Normalized;
use dspi_transport::Transport;

/// Read the bulk image whatever its wire format, and map it onto V32.
///
/// A V32 image comes back as [`dspi_proto::wire_compat::WireRead::Exact`] and
/// may be written back; anything else is display only.
pub fn read_bulk_any(t: &mut dyn Transport) -> Result<Normalized, crate::WriteError> {
    let _ = t;
    todo!("WP S2: compat bulk read (chunked by header length, or single-shot)")
}

/// Read the raw image bytes: chunked (`0xA2`, sized by the header) when the
/// transport allows it, else single-shot `0xA0`.
pub fn read_bulk_bytes(t: &mut dyn Transport) -> Result<Vec<u8>, crate::WriteError> {
    let _ = t;
    todo!("WP S2: raw bulk read")
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    #[test]
    #[ignore = "WP S2"]
    fn an_older_image_is_read_to_its_own_length_and_mapped() {
        todo!("WP S2")
    }

    #[test]
    #[ignore = "WP S2"]
    fn a_link_without_chunked_bulk_reads_single_shot() {
        todo!("WP S2")
    }
}
