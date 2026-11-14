//! WP P1PR tests: "protect all outputs", the warning mask and a linked
//! pair's preamp through C. The behaviour is dspi-core's to test; here the
//! demo device shows the calls reach it and undo takes each back in one step.

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
fn wp_p1pr_protect_all_outputs_through_c() {
    assert!(!dspi_protect_all_outputs(NULL, -1.0));
    assert_eq!(dspi_outputs_without_limiter(NULL, -10.0), 0);
    assert!(!dspi_set_input_pair_preamp(NULL, 0, 0.0));

    let h = demo();
    let n = usize::from(state(h).num_output_channels);
    assert!(dspi_set_limiter_param(h, 0xFF, 0, 0.0), "all off");
    for o in 0..n {
        assert!(dspi_set_output_gain(h, o as u8, -20.0));
    }
    assert!(dspi_set_output_gain(h, 0, 0.0));
    assert!(dspi_set_output_gain(h, 1, 0.0));
    let on = |s: &DspState| s.output_enabled;
    let loud = (0..2)
        .filter(|&k| on(&state(h))[k])
        .fold(0u32, |m, k| m | 1 << k);
    assert_eq!(dspi_outputs_without_limiter(h, -10.0), loud);
    let before = state(h);

    assert!(!dspi_protect_all_outputs(h, f32::NAN), "a NaN is refused");
    assert!(dspi_protect_all_outputs(h, -1.0));
    let s = state(h);
    for k in 0..n {
        assert!(s.limiter_enabled[k], "output {k}");
        assert_eq!(s.limiter_threshold_db[k], -1.0);
    }
    assert_eq!(s.limiter_link_group[0], 1);
    assert_eq!(s.limiter_link_group[1], 1);
    assert_eq!(dspi_outputs_without_limiter(h, -10.0), 0);

    // one undo: groups and thresholds back; the limiters stay on (switching
    // one is audible, never undone)
    undo(h);
    let s = state(h);
    assert!(s.limiter_enabled[..n].iter().all(|&e| e));
    assert_eq!(s.limiter_threshold_db, before.limiter_threshold_db);
    assert_eq!(s.limiter_link_group, before.limiter_link_group);

    assert!(dspi_set_input_pair_preamp(h, 0, -3.0));
    let s = state(h);
    assert_eq!((s.input_preamp_db[0], s.input_preamp_db[1]), (-3.0, -3.0));
    undo(h);
    let s = state(h);
    assert_eq!(
        (s.input_preamp_db[0], s.input_preamp_db[1]),
        (before.input_preamp_db[0], before.input_preamp_db[1]),
        "one undo, both inputs"
    );
    dspi_core_free(h);
}
