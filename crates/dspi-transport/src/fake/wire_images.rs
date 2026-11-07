//! WP QD6: the demo device as firmware of another wire format, for clients'
//! read-only tests.
//!
//! An image is built from the demo's V32 one by undoing what
//! `dspi_proto::wire_compat` maps, so a value seen after the mapping was found in
//! that firmware's own layout.

use dspi_proto::generated::SECTIONS;
use dspi_proto::wire_compat::bulk_size;

use super::Capture;

/// A section's firmware offset; 0 for a name that is not one, which only a test
/// could pass and whose image would then show it.
fn off(name: &str) -> usize {
    SECTIONS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map_or(0, |(_, offset, _)| *offset)
}

fn set_header(image: &mut [u8], wire: u8, length: usize) {
    if let Some(version) = image.first_mut() {
        *version = wire;
    }
    if let Some(field) = image.get_mut(6..8) {
        field.copy_from_slice(&u16::try_from(length).unwrap_or(u16::MAX).to_le_bytes());
    }
}

/// The V32 image `v32` as firmware of wire format `wire` lays it out. `None` for
/// a format without a fixture: V17, V26, V31 and V33 have one.
fn image_as(v32: &[u8], wire: u8) -> Option<Vec<u8>> {
    if !matches!(wire, 17 | 26 | 31 | 33) {
        return None;
    }
    if wire == 33 {
        // A newer firmware appends: the V32 prefix, then what it added.
        let mut image = v32.to_vec();
        image.extend_from_slice(&[0xEE; 16]);
        let length = image.len();
        set_header(&mut image, wire, length);
        return Some(image);
    }
    let size = bulk_size(wire)?;
    let input_config = off("input_config");
    let mut laid = v32.to_vec();
    if wire < 28 {
        // Before V28 the input config's tail sat one byte earlier.
        laid.copy_within(input_config + 11..input_config + 16, input_config + 10);
    }
    let mut image = if wire >= 18 {
        laid.get(..size)?.to_vec()
    } else {
        let (leveller, preamp, adat) = (off("leveller"), off("preamp"), off("adat_config"));
        let mut image = vec![0u8; size];
        image
            .get_mut(..leveller + 16)?
            .copy_from_slice(v32.get(..leveller + 16)?);
        // Before V18 everything from the preamp on sat four earlier.
        image
            .get_mut(preamp - 4..adat + 4)?
            .copy_from_slice(laid.get(preamp..adat + 8)?);
        image
    };
    set_header(&mut image, wire, size);
    Some(image)
}

/// What each format's firmware says of itself: 4 bytes on 1.1.5, then 7. The
/// demo's platform and output count stay. Released formats as in
/// `dspi-mobile/docs/WIRE-FORMAT-COMPAT.md` section 1.1; V31 was never released,
/// so it reads as the nearest release, and V33 as a release to come.
fn platform_as(demo: &[u8], wire: u8) -> Vec<u8> {
    let mut reply = demo.to_vec();
    match wire {
        17 | 26 => {
            reply.truncate(4);
            if let Some(packed) = reply.get_mut(2) {
                *packed = 0x15;
            }
        }
        _ => {
            // Minor, patch, beta: 1.1.7, or 1.1.6-beta3
            let version: [u8; 3] = if wire == 33 { [1, 7, 0] } else { [1, 6, 3] };
            if let Some(tail) = reply.get_mut(4..7) {
                tail.copy_from_slice(&version);
            }
        }
    }
    reply
}

impl Capture {
    /// The demo capture as firmware of wire format `wire`, or `None` for one
    /// without a fixture: V17, V26, V31 and V33 have one. Every such device is
    /// read-only for this build.
    pub fn demo_as_wire(wire: u8) -> Option<Self> {
        let demo = Self::demo();
        Some(Self {
            platform: platform_as(&demo.platform, wire),
            bulk: image_as(&demo.bulk, wire)?,
        })
    }
}

#[cfg(test)]
mod tests_wp_qd6 {
    use dspi_proto::wire_compat::{WireRead, normalize_bytes};

    use super::*;

    #[test]
    fn wp_qd6_each_fixture_maps_back_to_the_demo() {
        let demo = Capture::demo();
        for wire in [17u8, 26, 31, 33] {
            let capture = Capture::demo_as_wire(wire).unwrap();
            assert_eq!(capture.bulk[0], wire);
            let (mapped, read) = normalize_bytes(&capture.bulk).unwrap();
            assert_eq!(
                read,
                if wire > 32 {
                    WireRead::Newer
                } else {
                    WireRead::Older
                }
            );
            // What every one of these formats holds, found through the mapping.
            for section in ["channel_names", "preamp", "master_volume", "user_volume"] {
                let (_, at, len) = *SECTIONS.iter().find(|(n, _, _)| *n == section).unwrap();
                assert_eq!(
                    mapped[at..at + len],
                    demo.bulk[at..at + len],
                    "{section} of V{wire}"
                );
            }
        }
    }

    #[test]
    fn wp_qd6_no_fixture_for_other_formats() {
        for wire in [0u8, 16, 18, 32, 34] {
            assert!(Capture::demo_as_wire(wire).is_none());
        }
    }

    #[test]
    fn wp_qd6_firmware_reports_its_own_version() {
        assert_eq!(Capture::demo_as_wire(17).unwrap().platform.len(), 4);
        assert_eq!(Capture::demo_as_wire(26).unwrap().platform[2], 0x15);
        assert_eq!(Capture::demo_as_wire(33).unwrap().platform[4..7], [1, 7, 0]);
    }
}
