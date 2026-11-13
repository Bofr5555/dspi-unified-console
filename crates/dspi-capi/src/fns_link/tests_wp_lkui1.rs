//! WP LKUI1 tests: the System calls' replies without a hub session, their
//! argument checks, and the act rule on short buffers. The calls against a
//! real hub are dspi-hub's `tests/wp_lkui1_system_view.rs`.

#[cfg(feature = "link")]
mod link {
    use std::ffi::{CString, c_char};
    use std::ptr::{null, null_mut};

    use serde_json::Value;

    use crate::fns_device::{dspi_core_free, dspi_core_new};
    use crate::fns_link::link::*;

    fn json(f: impl FnOnce(*mut c_char, u32) -> u32) -> Value {
        let mut buf = vec![0u8; 64 * 1024];
        let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
        assert!(
            needed > 0 && needed as usize <= buf.len(),
            "needed {needed}"
        );
        serde_json::from_slice(&buf[..needed as usize - 1]).unwrap()
    }

    #[test]
    fn wp_lkui1_system_calls_reply_disconnected_and_check_arguments() {
        let h = dspi_core_new();
        let edits = CString::new(r#"{"setSystemDelay":true,"systemDelayFrames":null}"#).unwrap();
        let scene = CString::new("evening").unwrap();
        for v in [
            json(|b, l| dspi_alink_system_view(h, b, l)),
            json(|b, l| dspi_alink_system_save(h, 3, edits.as_ptr(), b, l)),
            json(|b, l| dspi_alink_system_align(h, b, l)),
            json(|b, l| dspi_alink_system_recall(h, scene.as_ptr(), b, l)),
        ] {
            assert_eq!(v["ok"], false, "{v}");
            assert_eq!(v["code"], "disconnected", "{v}");
        }
        // Bad arguments are refused before anything is sent.
        let bad = CString::new("{not json").unwrap();
        let v = json(|b, l| dspi_alink_system_save(h, 3, bad.as_ptr(), b, l));
        assert_eq!(v["code"], "invalid", "{v}");
        let v = json(|b, l| dspi_alink_system_save(h, 3, null(), b, l));
        assert_eq!(v["code"], "invalid", "{v}");
        let v = json(|b, l| dspi_alink_system_recall(h, null(), b, l));
        assert_eq!(v["code"], "invalid", "{v}");
        // A call that acts does nothing with a short buffer: it asks for the minimum.
        assert_eq!(
            dspi_alink_system_recall(h, scene.as_ptr(), null_mut(), 0),
            LINK_REPLY_MIN
        );
        assert_eq!(dspi_alink_system_view(null_mut(), null_mut(), 0), 0);
        dspi_core_free(h);
    }
}
