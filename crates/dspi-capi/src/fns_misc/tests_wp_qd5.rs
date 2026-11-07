//! WP QD5 tests: the demo device's output-latency timing, through C.

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new};

#[test]
fn wp_qd5_latency_timing_through_c() {
    let h = dspi_core_new();
    assert!(!dspi_demo_latency_timing(h, 1, 1, 0), "no device yet");
    assert_eq!(dspi_demo_latency_sets(h), u32::MAX);
    assert!(dspi_open_demo(h));
    assert_eq!(dspi_demo_latency_sets(h), 0);
    assert!(!dspi_demo_latency_timing(h, 4, 0, 0));
    assert!(dspi_demo_latency_timing(h, 1, 1, 0));
    assert!(dspi_set_output_latency(h, 3));
    assert_eq!(dspi_demo_latency_sets(h), 1);
    dspi_core_free(h);
    assert_eq!(dspi_demo_latency_sets(std::ptr::null_mut()), u32::MAX);
}
