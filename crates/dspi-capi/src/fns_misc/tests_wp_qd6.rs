//! WP QD6 tests: the read-only fixtures through C.

use dspi_proto::wire_compat::feat;

use super::*;
use crate::fns_device::{
    dspi_clear_clips, dspi_core_free, dspi_core_new, dspi_fetch_all, dspi_get_state,
    dspi_set_bypass,
};
use crate::types::{
    COMPAT_FIRMWARE_TOO_NEW, COMPAT_FIRMWARE_TOO_OLD, WIRE_FEAT_LIMITER, WIRE_FEAT_PSYBASS,
    WIRE_FEAT_SUBHARM, WIRE_FEAT_TUBE, WIRE_FEAT_UPMIX,
};
// -- WP QPI2 --
use crate::types::{WIRE_FEAT_CROSSFEED_MASK, WIRE_FEAT_LEVELLER_MASKS, WIRE_FEAT_LOUDNESS_MASK};
// -- WP CS1 --
use crate::types::{WIRE_FEAT_SUBHARM_EXT, WIRE_FEAT_UPMIX_PRESENCE};

#[test]
fn wp_qd6_feature_bits_match_the_wire_map() {
    assert_eq!(WIRE_FEAT_PSYBASS, feat::PSYBASS);
    assert_eq!(WIRE_FEAT_UPMIX, feat::UPMIX);
    assert_eq!(WIRE_FEAT_SUBHARM, feat::SUBHARM);
    assert_eq!(WIRE_FEAT_TUBE, feat::TUBE);
    assert_eq!(WIRE_FEAT_LIMITER, feat::LIMITER);
    // -- WP QPI2 --
    assert_eq!(WIRE_FEAT_LEVELLER_MASKS, feat::LEVELLER_MASKS);
    assert_eq!(WIRE_FEAT_LOUDNESS_MASK, feat::LOUDNESS_MASK);
    assert_eq!(WIRE_FEAT_CROSSFEED_MASK, feat::CROSSFEED_MASK);
    // -- WP CS1 --
    assert_eq!(WIRE_FEAT_UPMIX_PRESENCE, 1 << 9);
    assert_eq!(WIRE_FEAT_SUBHARM_EXT, 1 << 12);
    assert_eq!(WIRE_FEAT_UPMIX_PRESENCE, feat::UPMIX_PRESENCE);
    assert_eq!(WIRE_FEAT_SUBHARM_EXT, feat::SUBHARM_EXT);
}

#[test]
fn wp_qd6_each_fixture_opens_read_only_and_counts_a_refused_write() {
    let h = dspi_core_new();
    assert_eq!(dspi_wire_features(h), 0, "no device");
    assert!(!dspi_demo_open_wire(h, 32), "no fixture for V32");
    for (wire, compat, has, lacks) in [
        (
            17u8,
            COMPAT_FIRMWARE_TOO_OLD,
            0,
            WIRE_FEAT_PSYBASS | WIRE_FEAT_UPMIX,
        ),
        (
            26,
            COMPAT_FIRMWARE_TOO_OLD,
            WIRE_FEAT_UPMIX,
            WIRE_FEAT_SUBHARM,
        ),
        (
            31,
            COMPAT_FIRMWARE_TOO_OLD,
            WIRE_FEAT_TUBE,
            WIRE_FEAT_LIMITER,
        ),
        (33, COMPAT_FIRMWARE_TOO_NEW, WIRE_FEAT_LIMITER, 0),
    ] {
        assert!(dspi_demo_open_wire(h, wire), "V{wire}");
        assert!(dspi_fetch_all(h), "V{wire}");
        let state = unsafe { &*dspi_get_state(h) };
        assert_eq!(state.compat, compat, "V{wire}");
        assert_eq!(state.format_version, wire, "V{wire}");
        let features = dspi_wire_features(h);
        assert_eq!(features & has, has, "V{wire}");
        assert_eq!(features & lacks, 0, "V{wire}");

        let before = dspi_read_only_refusals(h);
        assert!(!dspi_set_bypass(h, true), "V{wire}");
        assert_eq!(dspi_read_only_refusals(h), before + 1, "V{wire}");
        // What read-only mode keeps working is not refused.
        assert!(dspi_clear_clips(h), "V{wire}");
        // -- WP P1TZ -- with its token.
        let r = with_token_for_test(h, "enter_bootloader", |t, b, l| {
            dspi_enter_bootloader_confirmed(h, t, b, l)
        });
        assert_eq!(r["ok"], true, "V{wire}: {r}");
        assert_eq!(dspi_read_only_refusals(h), before + 1, "V{wire}");
    }
    dspi_core_free(h);
}
