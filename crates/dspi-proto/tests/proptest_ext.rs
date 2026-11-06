//! WP P1: property tests for the extension-channel codecs (PLAN §1.9). The
//! replies come from the device and the pair-map and voltage strings from a
//! user, so none of these may panic on any input (§1.7).

use dspi_proto::ext::{
    CoreVregStatus, PairMap, decode_list, name, parse_vreg, vreg_label, vreg_mv,
};
use proptest::prelude::*;

/// Arguments as a user might type them, and some they should not.
fn pair_arg() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("default".to_string()),
        Just("-".to_string()),
        (any::<u8>(), any::<u8>()).prop_map(|(l, r)| format!("{l}:{r}")),
        (0u8..16, 0u8..16).prop_map(|(l, r)| format!("{l}:{r}")),
        any::<u8>().prop_map(|b| format!("0x{b:02X}")),
        "0[xX][0-9a-fA-F]{0,4}",
        "[0-9:]{0,6}",
        ".{0,8}",
    ]
}

proptest! {
    #[test]
    fn proptest_wp_p1_pair_map_pair_never_panics(map in any::<[u8; 4]>(), p in any::<usize>()) {
        let m = PairMap(map);
        let (l, r) = m.pair(p);
        if p < 4 && map[p] != 0 {
            prop_assert_eq!((l, r), (map[p] & 0x0F, map[p] >> 4));
        } else if p < 128 {
            // The default pairing, also for a pair past the map.
            prop_assert_eq!((l as usize, r as usize), (2 * p, 2 * p + 1));
        }
        let _ = m.normalized();
        let _ = m.to_string();
    }

    #[test]
    fn proptest_wp_p1_pair_map_parse_never_panics(args in prop::collection::vec(pair_arg(), 0..7)) {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        match PairMap::parse(&refs) {
            Ok(m) => {
                prop_assert!(args.len() <= 4);
                // Every pair the map stores is readable, and so is what it says.
                for p in 0..4 {
                    let _ = m.pair(p);
                }
                let _ = m.normalized().to_string();
            }
            Err(e) => prop_assert!(!e.0.is_empty()),
        }
    }

    /// `L:R` with valid outputs reads back as typed.
    #[test]
    fn proptest_wp_p1_pair_map_parse_reads_back(pairs in prop::collection::vec((0u8..16, 0u8..16), 0..=4)) {
        let args: Vec<String> = pairs.iter().map(|(l, r)| format!("{l}:{r}")).collect();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let m = PairMap::parse(&refs).map_err(|e| TestCaseError::fail(e.0))?;
        for (p, (l, r)) in pairs.iter().enumerate() {
            let raw = (r << 4) | l;
            if raw != 0 {
                prop_assert_eq!(m.pair(p), (*l, *r));
            }
        }
        for p in pairs.len()..4 {
            prop_assert_eq!(m.0[p], 0, "missing pairs are the default");
        }
    }

    #[test]
    fn proptest_wp_p1_core_vreg_status_decode_never_panics(b in prop::collection::vec(any::<u8>(), 0..40)) {
        match CoreVregStatus::decode(&b) {
            Some(s) => {
                prop_assert!(b.len() >= CoreVregStatus::SIZE);
                prop_assert_eq!(s.live_vreg, b[0]);
                prop_assert_eq!(s.uptime_s, u32::from_le_bytes([b[12], b[13], b[14], b[15]]));
                prop_assert_eq!(s.fields().len(), 9);
            }
            None => prop_assert!(b.len() < CoreVregStatus::SIZE),
        }
    }

    #[test]
    fn proptest_wp_p1_ext_list_and_names_never_panic(b in prop::collection::vec(any::<u8>(), 0..80)) {
        let ids = decode_list(&b);
        prop_assert_eq!(ids.len(), b.len() / 2);
        for id in ids {
            let _ = name(id);
        }
    }

    #[test]
    fn proptest_wp_p1_vreg_codes_never_panic(code in any::<u8>()) {
        let label = vreg_label(code);
        if let Some(mv) = vreg_mv(code) {
            prop_assert!((850..=1300).contains(&mv));
            prop_assert_eq!(parse_vreg(&format!("{mv}")), Ok(code));
        }
        prop_assert!(!label.is_empty());
    }

    #[test]
    fn proptest_wp_p1_parse_vreg_never_panics(s in prop_oneof![
        ".{0,12}".boxed(),
        "[0-9.eE+-]{0,12}[vV]?".boxed(),
        any::<f64>().prop_map(|v| v.to_string()).boxed(),
    ]) {
        if let Ok(code) = parse_vreg(&s) {
            prop_assert!(s.starts_with("0x") || s.starts_with("0X") || vreg_mv(code).is_some());
        }
    }
}
