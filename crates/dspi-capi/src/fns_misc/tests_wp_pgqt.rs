//! WP PGQT tests: the demo as each fake generation through C.

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new};

#[test]
fn wp_pgqt_the_demo_opens_as_each_generation_through_c() {
    let h = dspi_core_new();
    assert_eq!(dspi_demo_generation_violations(h), u32::MAX, "none opened");
    // -- WP QTSENDER -- 8 the dual-stack sender
    for code in 0..=8u8 {
        assert!(dspi_demo_open_generation(h, code), "code {code}");
        assert_eq!(dspi_demo_generation_violations(h), 0, "code {code}");
    }
    assert!(!dspi_demo_open_generation(h, 9));
    assert!(!dspi_demo_open_generation(std::ptr::null_mut(), 0));
    assert_eq!(
        dspi_demo_generation_violations(std::ptr::null_mut()),
        u32::MAX
    );
    dspi_core_free(h);
}
