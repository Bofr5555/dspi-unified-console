//! WP P1: property tests for the bulk image codecs (PLAN §1.9): the V32
//! packet and header, and the read-only mapping of other wire formats. Every
//! byte here comes from the device (§1.7).

use dspi_proto::generated::{BULK_SIZE, SECTIONS};
use dspi_proto::wire::{BulkPacket, WireHeader};
use dspi_proto::wire_compat::{
    MIN_WIRE_FORMAT_VERSION, WireRead, bulk_size, features, image_length, normalize,
    normalize_bytes,
};
use proptest::prelude::*;

const V32: u8 = 32;

fn off(name: &str) -> usize {
    SECTIONS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, o, _)| *o)
        .unwrap_or_else(|| panic!("no section {name}"))
}

/// A version byte: mostly the interesting ones, sometimes anything.
fn version() -> impl Strategy<Value = u8> {
    prop_oneof![3 => 14u8..=34, 1 => any::<u8>()]
}

/// A length near one of the sizes the code compares against.
fn length() -> impl Strategy<Value = usize> {
    let mut near: Vec<usize> = (16..=32).filter_map(bulk_size).collect();
    near.extend([0, 1, 7, 8, 15, 16, 17, off("global"), BULK_SIZE + 64]);
    prop_oneof![
        2 => (prop::sample::select(near), -3i64..=3)
            .prop_map(|(n, d)| (n as i64 + d).max(0) as usize),
        1 => 0usize..7000,
    ]
}

/// Random bytes of `len`, with the version byte set.
fn image() -> impl Strategy<Value = Vec<u8>> {
    (version(), length(), any::<bool>(), any::<u64>()).prop_map(|(v, len, sane, seed)| {
        let mut d = bytes(len, seed);
        if let Some(b) = d.first_mut() {
            *b = v;
        }
        if sane && d.len() >= 8 {
            // A header that passes the consistency checks, so the deeper
            // paths run too.
            d[2..6].copy_from_slice(&[17, 9, 8, 12]);
            d[6..8].copy_from_slice(&(len as u16).to_le_bytes());
        }
        d
    })
}

/// Cheap deterministic filler (a large `vec(any::<u8>())` is slow to shrink).
fn bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

/// The V32 image a V16..V31 image maps onto, written from the version log in
/// `wire_compat`'s module docs: fields both layouts share are copied (moved
/// where V18 and V28 moved them), fields the old format lacks are zero.
fn expected_mapping(data: &[u8], v: u8) -> Vec<u8> {
    let (lev, pre, ic, adat) = (
        off("leveller"),
        off("preamp"),
        off("input_config"),
        off("adat_config"),
    );
    let mut e = vec![0u8; BULK_SIZE];
    // The header and every section up to the leveller's first 16 bytes.
    e[..lev + 16].copy_from_slice(&data[..lev + 16]);
    // V18's leveller masks.
    if v >= 18 {
        e[lev + 16..pre].copy_from_slice(&data[lev + 16..pre]);
    }
    let shift = if v < 18 { 4 } else { 0 };
    let end = if v >= 17 { adat + 8 } else { adat };
    for (o, byte) in e.iter_mut().enumerate().take(end).skip(pre) {
        let src = if v < 28 && (ic + 10..ic + 16).contains(&o) {
            // V28 inserted the fourth S/PDIF pin at ic + 10.
            if o == ic + 10 { None } else { Some(o - 1) }
        } else {
            Some(o)
        };
        *byte = src.map_or(0, |s| data[s - shift]);
    }
    let mut appended = |since: u8, name: &str, len: usize| {
        if v >= since {
            let at = off(name);
            e[at..at + len].copy_from_slice(&data[at..at + len]);
        }
    };
    appended(23, "psybass", off("upmix") - off("psybass"));
    appended(25, "upmix", off("subharm") - off("upmix"));
    appended(
        29,
        "subharm",
        if v == 29 {
            16
        } else {
            off("tube") - off("subharm")
        },
    );
    appended(31, "tube", off("limiter") - off("tube"));
    e
}

proptest! {
    #[test]
    fn proptest_wp_p1_wire_header_decode_never_panics(b in prop::collection::vec(any::<u8>(), 0..40)) {
        if let Ok(h) = WireHeader::decode(&b) {
            prop_assert_eq!(h.format_version, V32);
            prop_assert_eq!(
                h.num_input_channels as u16 + h.num_output_channels as u16,
                h.num_channels as u16
            );
            let _ = (h.firmware(), h.platform(), h.channel_map());
        }
    }

    #[test]
    fn proptest_wp_p1_bulk_packet_decode_never_panics(d in image()) {
        if let Ok(p) = BulkPacket::decode(d.clone()) {
            prop_assert_eq!(p.as_bytes(), &d[..]);
        }
    }

    /// A packet that decodes can be read and patched anywhere without a panic.
    #[test]
    fn proptest_wp_p1_bulk_packet_accessors_never_panic(
        seed in any::<u64>(),
        counts in (0u8..=17, 0u8..=17),
        ch in any::<u8>(),
        band in any::<u8>(),
        patch_at in any::<usize>(),
        patch in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        let mut d = bytes(BULK_SIZE, seed);
        let (inputs, outputs) = counts;
        d[0] = V32;
        d[2] = inputs.saturating_add(outputs);
        d[3] = outputs;
        d[4] = inputs;
        d[6..8].copy_from_slice(&(BULK_SIZE as u16).to_le_bytes());
        let mut p = BulkPacket::decode(d).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let _ = (p.channel_map(), p.input_config(), p.band(ch, band), p.xover_band(ch, band));
        for (name, _, len) in SECTIONS {
            prop_assert_eq!(p.section(name).map(<[u8]>::len), Some(len));
        }
        let at = patch_at % (BULK_SIZE + 100);
        let before = p.as_bytes().to_vec();
        match p.patch(at, &patch) {
            Some(_) => prop_assert_eq!(&p.as_bytes()[at..at + patch.len()], &patch[..]),
            None => prop_assert_eq!(p.as_bytes(), &before[..], "a refused patch changes nothing"),
        }
        let _ = p.patch(usize::MAX, &patch);
        let _ = BulkPacket::section_at(patch_at);
    }

    #[test]
    fn proptest_wp_p1_image_length_never_panics(b in prop::collection::vec(any::<u8>(), 0..20)) {
        let n = image_length(&b);
        prop_assert!(n >= off("global"));
    }

    #[test]
    fn proptest_wp_p1_normalize_never_panics(d in image()) {
        let bytes = normalize_bytes(&d);
        let full = normalize(&d);
        if let Ok((img, read)) = &bytes {
            prop_assert_eq!(img.len(), BULK_SIZE);
            let v = d[0];
            prop_assert!(v >= MIN_WIRE_FORMAT_VERSION);
            prop_assert_eq!(*read, match v.cmp(&V32) {
                std::cmp::Ordering::Less => WireRead::Older,
                std::cmp::Ordering::Equal => WireRead::Exact,
                std::cmp::Ordering::Greater => WireRead::Newer,
            });
        }
        if let Ok(n) = full {
            prop_assert!(bytes.is_ok(), "normalize succeeds only if the mapping does");
            prop_assert_eq!(n.device_version, d[0]);
            prop_assert_eq!(n.is_writable(), n.read == WireRead::Exact);
            prop_assert_eq!(n.packet.header().format_version, V32);
        }
    }

    /// For V16..V31, the header and every field both layouts share agree
    /// after mapping, and the fields the old format lacks are zero.
    #[test]
    fn proptest_wp_p1_older_images_map_field_for_field(
        v in 16u8..=31,
        extra in 0usize..64,
        seed in any::<u64>(),
        sane in any::<bool>(),
    ) {
        let size = bulk_size(v).ok_or_else(|| TestCaseError::fail("no size"))?;
        let mut d = bytes(size + extra, seed);
        d[0] = v;
        if sane {
            d[2..6].copy_from_slice(&[17, 9, 8, 12]);
        }
        let (img, read) = normalize_bytes(&d).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(read, WireRead::Older);
        let want = expected_mapping(&d, v);
        prop_assert_eq!(&img[..16], &d[..16], "the header is the device's");
        if let Some(o) = (0..BULK_SIZE).find(|&o| img[o] != want[o]) {
            let section = BulkPacket::section_at(o).unwrap_or("?");
            return Err(TestCaseError::fail(format!(
                "V{v}: byte {o} ({section}) is {:#04x}, want {:#04x}",
                img[o], want[o]
            )));
        }

        match normalize(&d) {
            Ok(n) => {
                prop_assert_eq!(n.read, WireRead::Older);
                prop_assert!(!n.is_writable());
                prop_assert_eq!(n.device_version, v);
                prop_assert_eq!(n.features, features(v));
                let h = n.packet.header();
                prop_assert_eq!(h.format_version, V32);
                prop_assert_eq!(h.payload_length as usize, BULK_SIZE);
                prop_assert_eq!(
                    (h.platform_id, h.num_channels, h.num_output_channels, h.num_input_channels, h.max_bands),
                    (d[1], d[2], d[3], d[4], d[5])
                );
                prop_assert_eq!(&n.packet.as_bytes()[8..], &want[8..]);
            }
            // Only the header's own consistency check can refuse it.
            Err(e) => prop_assert!(!sane, "a consistent header was refused: {e}"),
        }
    }
}
