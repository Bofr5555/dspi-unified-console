//! C functions: the measurement documents (WP P5DOCK, PLAN-P5 T11), over
//! `dspi_core`'s `measure_set_dir`, `measure_docs`, `measure_doc_get`,
//! `measure_doc_put`, `measure_doc_rename`, `measure_doc_note`,
//! `measure_doc_delete`, `measurement_export_rew` and the free
//! `measure_doc_info`, `mic_import`, `target_import`, `target_builtins`.
//!
//! Replies as in the parent module's docs: JSON (camelCase) with `ok`, a
//! list as `value`; a document or REW text is the text itself, or the
//! failure in its place. None touches the device; the store's calls do file
//! I/O, off the UI thread.

use std::ffi::c_char;

use super::{done_reply, guard, json_reply, optional_text};
use crate::FfiCore;
use crate::fns_device::{arg, copy_text, text};
use dspi_core::{DspiError, Result};

/// A document or text, or the failure reply in its place.
fn text_reply(result: Result<String>) -> String {
    result.unwrap_or_else(|e| json_reply::<()>(Err(e)))
}

/// Keep measurement, microphone and target documents under folder `dir`
/// ("": forget it). JSON `{ok}` or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_set_dir(
    core: *mut FfiCore,
    dir: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        done_reply(arg(dir, "dir").and_then(|d| c.measure_set_dir(d.to_owned())))
    })
}

/// The filed documents of `kind` (`measurement`, `mic`, `target`; null or
/// "": every kind), measurements of device `serial` only (null or "": every
/// device's): JSON `{ok, value: [{id, kind, name, note, serial, savedMs,
/// bytes}, ...]}`, or a failure for an unknown kind.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_docs(
    core: *mut FfiCore,
    kind: *const c_char,
    serial: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(
            optional_text(kind, "kind")
                .and_then(|k| Ok((k, optional_text(serial, "serial")?)))
                .and_then(|(k, s)| {
                    c.measure_docs(k.unwrap_or_default(), s.unwrap_or_default())
                }),
        )
    })
}

/// Filed document `id`'s text, or a JSON failure in its place.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_doc_get(
    core: *mut FfiCore,
    id: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        text_reply(arg(id, "id").and_then(|i| c.measure_doc_get(i.to_owned())))
    })
}

/// File a measurement, microphone or target document. JSON: its row `{ok,
/// id, kind, name, note, serial, savedMs, bytes}`, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_doc_put(
    core: *mut FfiCore,
    doc: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(doc, "doc").and_then(|d| c.measure_doc_put(d.to_owned())))
    })
}

/// Rename filed document `id`. JSON: its row, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_doc_rename(
    core: *mut FfiCore,
    id: *const c_char,
    name: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(
            arg(id, "id")
                .and_then(|i| Ok((i, arg(name, "name")?)))
                .and_then(|(i, n)| c.measure_doc_rename(i.to_owned(), n.to_owned())),
        )
    })
}

/// Set filed document `id`'s note. JSON: its row, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_doc_note(
    core: *mut FfiCore,
    id: *const c_char,
    note: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(
            arg(id, "id")
                .and_then(|i| Ok((i, arg(note, "note")?)))
                .and_then(|(i, n)| c.measure_doc_note(i.to_owned(), n.to_owned())),
        )
    })
}

/// Delete filed document `id`. JSON `{ok}` or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_doc_delete(
    core: *mut FfiCore,
    id: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        done_reply(arg(id, "id").and_then(|i| c.measure_doc_delete(i.to_owned())))
    })
}

/// Output `output`'s response of measurement `doc` (a filed id, or a
/// measurement document's text) as REW text, or a JSON failure in its place.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measurement_export_rew(
    core: *mut FfiCore,
    doc: *const c_char,
    output: u8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        text_reply(arg(doc, "doc").and_then(|d| c.measurement_export_rew(d.to_owned(), output)))
    })
}

/// A measurement, microphone or target document's row, validated: JSON
/// `{ok, id, kind, name, note, serial, savedMs, bytes}` or a failure. No
/// handle needed. Returns the bytes needed including the NUL, 0 for a panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_doc_info(doc: *const c_char, buf: *mut c_char, len: u32) -> u32 {
    guard(0, || {
        copy_text(
            &json_reply(arg(doc, "doc").and_then(|d| dspi_core::measure_doc_info(d.to_owned()))),
            buf,
            len,
        )
    })
}

/// A `mic` document from a calibration file's text (`file_name` may say
/// `_90deg`; null: ""; `name` null or "": the vendor and serial), or a JSON
/// failure in its place. No handle needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_mic_import(
    name: *const c_char,
    file_name: *const c_char,
    file_text: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    guard(0, || {
        let r = (|| -> Result<String> {
            let name = optional_text(name, "name")?.unwrap_or_default();
            let file = optional_text(file_name, "file_name")?.unwrap_or_default();
            let text = arg(file_text, "file_text")?;
            dspi_core::mic_import(name, file, text.to_owned())
        })();
        copy_text(&text_reply(r), buf, len)
    })
}

/// A `target` document from a target file's text, for `target_use`
/// (`speakers`, `headphones`) and `coupler` (`iec60318_4`, `flatPlate`,
/// `hats`; null or ""), or a JSON failure in its place. No handle needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_target_import(
    name: *const c_char,
    file_text: *const c_char,
    target_use: *const c_char,
    coupler: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    guard(0, || {
        let r = (|| -> Result<String> {
            let name = optional_text(name, "name")?.unwrap_or_default();
            let text = arg(file_text, "file_text")?;
            let target_use = arg(target_use, "target_use")?;
            let coupler = optional_text(coupler, "coupler")?.unwrap_or_default();
            dspi_core::target_import(name, text.to_owned(), target_use.to_owned(), coupler)
        })();
        copy_text(&text_reply(r), buf, len)
    })
}

/// The built-in targets: JSON `{ok, value: [{id, name, targetUse, coupler,
/// params: [{name, value}]}, ...]}`. No handle needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_target_builtins(buf: *mut c_char, len: u32) -> u32 {
    guard(0, || {
        copy_text(
            &json_reply(Ok::<_, DspiError>(dspi_core::target_builtins())),
            buf,
            len,
        )
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5dock {
    //! The same text through the C ABI as from the core (the done criterion),
    //! and the store's calls on the demo device. The store lives under
    //! `std::env::temp_dir()` (`ci/run.sh`: never tmpfs).

    use std::ffi::{CStr, CString};
    use std::ptr::null;

    use serde_json::{Value, json};

    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};
    use crate::fns_misc::dspi_open_demo;

    const UMIK: &str = include_str!("../../../dspi-measure/testdata/p5/cal/umik1_7000001.txt");
    const TARGET: &str = include_str!("../../../dspi-measure/testdata/p5/cal/target_rew.txt");

    fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> String {
        let mut buf = vec![0 as c_char; 1024 * 1024];
        let needed = f(buf.as_mut_ptr(), buf.len() as u32);
        assert!(needed > 0 && needed as usize <= buf.len(), "needed {needed}");
        // SAFETY: the call wrote a NUL-terminated reply into `buf`.
        unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    }

    fn json_of(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    /// `savedUtc` holds the second the document was made: equal apart from it.
    fn without_time(text: &str) -> Value {
        let mut v = json_of(text);
        v["meta"]["savedUtc"] = Value::Null;
        v
    }

    #[test]
    fn wp_p5dock_the_same_text_through_the_c_abi() {
        let c = |s: &str| CString::new(s).unwrap();
        let (name, file, umik) = (c("Mic"), c("umik1_7000001.txt"), c(UMIK));
        let mic = reply(|b, l| dspi_mic_import(name.as_ptr(), file.as_ptr(), umik.as_ptr(), b, l));
        let core = dspi_core::mic_import("Mic".into(), "umik1_7000001.txt".into(), UMIK.into())
            .unwrap();
        assert_eq!(without_time(&mic), without_time(&core));

        let (tn, tt, use_, cpl) = (c("T"), c(TARGET), c("headphones"), c("hats"));
        let t = reply(|b, l| {
            dspi_target_import(tn.as_ptr(), tt.as_ptr(), use_.as_ptr(), cpl.as_ptr(), b, l)
        });
        let core_t = dspi_core::target_import(
            "T".into(),
            TARGET.into(),
            "headphones".into(),
            "hats".into(),
        )
        .unwrap();
        assert_eq!(without_time(&t), without_time(&core_t));

        let b = json_of(&reply(|b, l| dspi_target_builtins(b, l)));
        assert_eq!(b["ok"], true);
        assert_eq!(
            b["value"],
            serde_json::to_value(dspi_core::target_builtins()).unwrap()
        );

        let info_in = c(&mic);
        let info = json_of(&reply(|b, l| dspi_measure_doc_info(info_in.as_ptr(), b, l)));
        assert_eq!((info["ok"].clone(), info["kind"].clone()), (json!(true), json!("mic")));

        // Failures in the document's place, typed.
        let bad = c("no numbers");
        let r = json_of(&reply(|b, l| dspi_mic_import(null(), null(), bad.as_ptr(), b, l)));
        assert_eq!((r["ok"].clone(), r["code"].clone()), (json!(false), json!("invalid")));
        let r = json_of(&reply(|b, l| dspi_measure_doc_info(null(), b, l)));
        assert_eq!(r["code"], "invalid");
        let r = json_of(&reply(|b, l| {
            dspi_target_import(null(), tt.as_ptr(), c("room").as_ptr(), null(), b, l)
        }));
        assert_eq!(r["code"], "invalid");
    }

    #[test]
    fn wp_p5dock_the_store_through_the_c_abi() {
        let dir = std::env::temp_dir()
            .join("wp-p5dock-capi")
            .join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        let dir_c = CString::new(dir.to_string_lossy().as_bytes()).unwrap();
        let h = dspi_core_new();
        assert!(dspi_open_demo(h));
        let mic = CString::new(
            dspi_core::mic_import(String::new(), "u.txt".into(), UMIK.into()).unwrap(),
        )
        .unwrap();
        // No folder: a typed failure.
        let r = json_of(&reply(|b, l| dspi_measure_doc_put(h, mic.as_ptr(), b, l)));
        assert_eq!(r["code"], "io", "{r}");
        let r = json_of(&reply(|b, l| dspi_measure_set_dir(h, dir_c.as_ptr(), b, l)));
        assert_eq!(r["ok"], true, "{r}");
        let put = json_of(&reply(|b, l| dspi_measure_doc_put(h, mic.as_ptr(), b, l)));
        assert_eq!((put["ok"].clone(), put["kind"].clone()), (json!(true), json!("mic")));
        let id = CString::new(put["id"].as_str().unwrap()).unwrap();
        let list = json_of(&reply(|b, l| dspi_measure_docs(h, null(), null(), b, l)));
        assert_eq!(list["value"].as_array().unwrap().len(), 1, "{list}");
        let k = CString::new("target").unwrap();
        let list = json_of(&reply(|b, l| dspi_measure_docs(h, k.as_ptr(), null(), b, l)));
        assert_eq!(list["value"], json!([]));
        let text = reply(|b, l| dspi_measure_doc_get(h, id.as_ptr(), b, l));
        assert_eq!(json_of(&text)["mic"]["id"], put["id"]);
        let nm = CString::new("Röd").unwrap();
        let r = json_of(&reply(|b, l| dspi_measure_doc_rename(h, id.as_ptr(), nm.as_ptr(), b, l)));
        assert_eq!(r["name"], "Röd");
        let r = json_of(&reply(|b, l| dspi_measure_doc_note(h, id.as_ptr(), nm.as_ptr(), b, l)));
        assert_eq!(r["note"], "Röd");
        // Not a measurement: no REW export.
        let r = json_of(&reply(|b, l| dspi_measurement_export_rew(h, id.as_ptr(), 0, b, l)));
        assert_eq!(r["code"], "invalid", "{r}");
        let r = json_of(&reply(|b, l| dspi_measure_doc_delete(h, id.as_ptr(), b, l)));
        assert_eq!(r["ok"], true);
        let r = json_of(&reply(|b, l| dspi_measure_doc_get(h, id.as_ptr(), b, l)));
        assert_eq!(r["code"], "invalid");
        dspi_core_free(h);
    }
}
