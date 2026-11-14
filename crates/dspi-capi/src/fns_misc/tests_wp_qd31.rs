//! WP QD31 tests: the demo device's output config save, through C.

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new};

#[test]
fn wp_qd31_output_save_through_c() {
    let h = dspi_core_new();
    assert!(!dspi_demo_output_save_status(h, 4), "no device yet");
    assert!(dspi_open_demo(h));
    // -- WP P1TZ -- the save takes a token; the device's status comes back
    // in its reply.
    let save = || {
        with_token_for_test(h, "save_output_config", |t, b, l| {
            dspi_save_output_config_confirmed(h, t, b, l)
        })
    };
    let r = save();
    assert_eq!(r["ok"], true, "the demo saves: {r}");
    assert!(dspi_demo_output_save_status(h, 4));
    let r = save();
    assert_eq!(r["code"], "refused", "{r}");
    assert!(r["error"].as_str().unwrap().contains("status 4"), "{r}");
    assert!(dspi_demo_output_save_status(h, 0xFF));
    assert_eq!(save()["ok"], false);
    assert!(dspi_demo_output_save_status(h, 0));
    assert_eq!(save()["ok"], true);
    dspi_core_free(h);
    assert!(!dspi_demo_output_save_status(std::ptr::null_mut(), 0));
}
