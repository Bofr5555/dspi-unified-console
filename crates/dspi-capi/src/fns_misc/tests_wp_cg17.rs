//! WP CG17 tests: the pair-map rule and the K340 values, through C.

use std::ptr::{null, null_mut};

use super::*;

fn valid(map: &[u8], pairs: u8) -> bool {
    dspi_crossfeed_pair_map_valid(map.as_ptr(), map.len() as u32, pairs)
}

#[test]
fn wp_cg17_the_rule_through_c_matches_the_core() {
    assert!(valid(&[0x02, 0x13, 0, 0], 4));
    assert!(valid(&[0x02, 0x13, 0, 0], 2));
    assert!(valid(&[0; 4], 4));
    assert!(!valid(&[0x02, 0, 0, 0], 4), "an output in two pairs");
    assert!(!valid(&[0x11, 0, 0, 0], 4), "a pair on one output");
    assert!(!valid(&[0x80, 0, 0, 0], 4), "the PDM sub");
    assert!(!valid(&[0, 0, 0x54, 0], 2), "a byte past the pairs");
    assert!(!valid(&[0; 4], 0));
    assert!(!valid(&[0; 4], 5));
}

#[test]
fn wp_cg17_null_and_short_buffers_are_false() {
    assert!(!dspi_crossfeed_pair_map_valid(null(), 4, 4));
    assert!(!dspi_crossfeed_pair_map_valid(null(), 0, 4));
    assert!(!valid(&[0; 3], 4));
    assert!(!valid(&[0; 5], 4));

    let mut out = [0xAAu8; 4];
    assert!(!dspi_crossfeed_k340_pair_map(null_mut(), 4));
    assert!(!dspi_crossfeed_k340_pair_map(out.as_mut_ptr(), 3));
    assert_eq!(out, [0xAA; 4], "nothing written");
}

#[test]
fn wp_cg17_the_k340_values() {
    let mut out = [0xAAu8; 5];
    assert!(dspi_crossfeed_k340_pair_map(out.as_mut_ptr(), 5));
    assert_eq!(
        out,
        [0x02, 0x13, 0, 0, 0xAA],
        "four bytes, in a larger buffer"
    );
    assert_eq!(dspi_crossfeed_k340_pair_mask(), 0x03);
}
