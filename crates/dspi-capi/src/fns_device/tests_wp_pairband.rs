//! WP PAIRBAND tests: a linked input pair's band through C. The behaviour is
//! dspi-core's to test; here the demo device shows the calls reach it and one
//! undo takes both sides back.

use super::*;
use crate::fns_misc::dspi_undo;

const NULL: *mut FfiCore = ptr::null_mut();

fn demo() -> *mut FfiCore {
    let h = dspi_core_new();
    assert!(!h.is_null());
    with_core(h, (), |c| c.core.open_demo().unwrap());
    h
}

fn state(h: *mut FfiCore) -> DspState {
    // SAFETY: the handle is live; the record stays valid until the next call.
    unsafe { *dspi_get_state(h) }
}

fn undo(h: *mut FfiCore) {
    let mut buf = vec![0u8; 16384];
    let needed = dspi_undo(h, buf.as_mut_ptr().cast(), buf.len() as u32);
    assert!(needed > 0 && needed as usize <= buf.len());
}

#[test]
fn wp_pairband_a_linked_pair_band_through_c() {
    let p = FilterParams {
        filter_type: 1,
        bypass: false,
        freq: 1200.0,
        q: 1.4,
        gain: 4.0,
        qp: 0.707,
    };
    assert!(!dspi_set_input_pair_filter(NULL, 0, 0, p));
    assert!(!dspi_set_input_pair_band_bypass(NULL, 0, 0, true));

    let h = demo();
    let before = state(h);
    assert!(
        !dspi_set_input_pair_filter(h, 0, 10, p),
        "band out of range"
    );
    assert!(dspi_set_input_pair_filter(h, 1, 0, p));
    let s = state(h);
    assert_eq!(s.filters[0][0].filter_type, 1);
    assert_eq!(s.filters[1][0].freq, 1200.0);
    assert!(dspi_set_input_pair_band_bypass(h, 0, 0, true));
    let s = state(h);
    assert!(s.filters[0][0].bypass && s.filters[1][0].bypass);
    undo(h);
    let s = state(h);
    assert!(
        !s.filters[0][0].bypass && !s.filters[1][0].bypass,
        "one undo, both bypasses"
    );
    undo(h);
    let s = state(h);
    assert_eq!(
        s.filters[0][0].filter_type,
        before.filters[0][0].filter_type
    );
    assert_eq!(
        s.filters[1][0].filter_type, before.filters[1][0].filter_type,
        "one undo, both bands"
    );
    dspi_core_free(h);
}
