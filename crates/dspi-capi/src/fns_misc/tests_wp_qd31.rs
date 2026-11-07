//! WP QD31 tests: the demo device's output config save, through C.

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new};

#[test]
fn wp_qd31_output_save_through_c() {
    let h = dspi_core_new();
    assert!(!dspi_demo_output_save_status(h, 4), "no device yet");
    assert!(dspi_open_demo(h));
    assert_eq!(dspi_save_output_config(h), 0, "the demo saves");
    assert!(dspi_demo_output_save_status(h, 4));
    assert_eq!(dspi_save_output_config(h), 4);
    assert!(dspi_demo_output_save_status(h, 0xFF));
    assert_eq!(dspi_save_output_config(h), 0xFF);
    assert!(dspi_demo_output_save_status(h, 0));
    assert_eq!(dspi_save_output_config(h), 0);
    dspi_core_free(h);
    assert!(!dspi_demo_output_save_status(std::ptr::null_mut(), 0));
}
