//! C functions: the notice table (WP P1NT), over `dspi_core::notice_text`.
//!
//! A failure reply of the JSON functions carries `noticeKey`, the error's stable
//! notice key; [`dspi_notice_for`] turns it into the words to show. None of these
//! takes a core handle or blocks. Replies as in the parent module's docs.

use std::ffi::c_char;

use dspi_core::{Notice, Result};

use super::{c_str, copy_text, guard, json_reply};

/// The notice for a key, in JSON: {ok, key, kind, title, reason, action, detail}.
/// `key` is a failure reply's `noticeKey`; `detail` (null reads as empty) is put in
/// `detail`, normally that reply's `error`. `kind` is `info`, `warning` or `error`.
/// A key the table does not have gives the `unknown` row. Returns the bytes needed
/// including the NUL, 0 for a null `key` or a panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_notice_for(
    key: *const c_char,
    detail: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    guard(0, || {
        let Some(key) = c_str(key) else {
            return 0;
        };
        let detail = c_str(detail).unwrap_or("").to_owned();
        let reply: Result<Notice> = Ok(dspi_core::notice_for_key(key.to_owned(), detail));
        copy_text(&json_reply(reply), buf, len)
    })
}

/// The whole notice table in JSON, every key once, `detail` empty: {ok,
/// value:[{key, kind, title, reason, action, detail}]}. Returns the bytes needed
/// including the NUL, 0 for a panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_notice_texts(buf: *mut c_char, len: u32) -> u32 {
    guard(0, || {
        copy_text(&json_reply(Ok(dspi_core::notice_texts())), buf, len)
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p1nt {
    use std::ffi::{CString, c_char};
    use std::ptr;

    use dspi_core::DspiError;
    use serde_json::Value;

    use super::*;

    fn read(f: impl Fn(*mut c_char, u32) -> u32) -> Value {
        let needed = f(ptr::null_mut(), 0);
        assert!(needed > 0);
        let mut buf = vec![0u8; needed as usize];
        assert_eq!(f(buf.as_mut_ptr().cast(), needed), needed);
        assert_eq!(buf.pop(), Some(0));
        serde_json::from_slice(&buf).unwrap()
    }

    /// A failure reply, as the C ABI writes it, turned into its notice.
    fn notice_of_reply(reply: &str) -> Value {
        let v: Value = serde_json::from_str(reply).unwrap();
        assert_eq!(v["ok"], false);
        let key = CString::new(v["noticeKey"].as_str().unwrap()).unwrap();
        let detail = CString::new(v["error"].as_str().unwrap()).unwrap();
        read(|b, l| dspi_notice_for(key.as_ptr(), detail.as_ptr(), b, l))
    }

    fn errors() -> Vec<DspiError> {
        vec![
            DspiError::Disconnected,
            DspiError::Locked,
            DspiError::Refused { status: 4 },
            DspiError::Link {
                code: "wrong_pin".into(),
                msg: "the hub said no".into(),
            },
            DspiError::Link {
                code: "brand_new".into(),
                msg: "?".into(),
            },
            DspiError::OvervoltWordRequired {
                msg: "type OVERVOLT".into(),
            },
        ]
    }

    /// The same words as the core's `notice_for`, which UniFFI exports as it is,
    /// through both reply styles (fns_misc's and fns_link's).
    #[test]
    fn wp_p1nt_c_gives_the_cores_notice_for_a_failure_reply() {
        for e in errors() {
            let core = dspi_core::notice_for(e.clone());
            let mut replies = vec![json_reply::<()>(Err(e.clone()))];
            #[cfg(feature = "link")]
            replies.push(crate::fns_link::link::reply_for_tests(Err(e.clone())));
            for reply in replies {
                let n = notice_of_reply(&reply);
                assert_eq!(n["ok"], true);
                for (field, want) in [
                    ("key", &core.key),
                    ("title", &core.title),
                    ("reason", &core.reason),
                    ("action", &core.action),
                ] {
                    assert_eq!(n[field], want.as_str(), "{e:?}: {field}");
                }
                let kind = serde_json::to_value(core.kind).unwrap();
                assert_eq!(n["kind"], kind);
            }
        }
    }

    #[test]
    fn wp_p1nt_c_unknown_key_null_detail_and_table() {
        let key = CString::new("nothing like it").unwrap();
        let n = read(|b, l| dspi_notice_for(key.as_ptr(), ptr::null(), b, l));
        assert_eq!(n["key"], "unknown");
        assert_eq!(n["detail"], "");
        assert_eq!(
            dspi_notice_for(ptr::null(), ptr::null(), ptr::null_mut(), 0),
            0
        );
        let table = read(|b, l| dspi_notice_texts(b, l));
        let rows = table["value"].as_array().unwrap();
        assert_eq!(rows.len(), dspi_core::notice_texts().len());
        assert!(rows.iter().any(|r| r["key"] == "dlink.wrong_pin"));
    }
}
