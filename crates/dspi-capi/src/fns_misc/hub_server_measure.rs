//! C functions: the hub's measurement documents (WP P5HUB), over
//! `dspi_core::hub_server::measure` (`docs/integrator/hub-api.md` section
//! 22): `measure.doc.list`, `get`, `put`, `rename`, `note`, `delete`, and the
//! merged list of this console's store and the hub's. Replies as in the
//! parent module's docs: JSON, the records' fields camelCase beside
//! `"ok": true`, a list as `value`.
//!
//! Without the cap `hub_measure` (or off a hub) a call fails with `code`
//! `unsupported` and sends nothing. The hub's refusals come as the core's
//! kinds (`denied`, `invalid`, `read_only`) or as `link` with `noticeKey`
//! `dlink.<code>` (`not_found`, `conflict`, `busy`). Every call is a round
//! trip (a large document several): off the UI thread.

use std::ffi::c_char;

use super::{arg, done_reply, json_reply, optional_text, text};

/// The hub's documents of `kind` (`measurement`, `mic`, `target`; null or ""
/// every kind), measurements of device `serial` (null or "" the open
/// device's): JSON `{ok, value: [{id, kind, name, note, serial, savedMs,
/// bytes}, ...]}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_measure_docs(
    core: *mut crate::FfiCore,
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
                    c.hub_measure_docs(k.unwrap_or_default(), s.unwrap_or_default())
                }),
        )
    })
}

/// Document `id` on the hub: JSON `{ok, doc: {id, kind, ...}, document}`
/// (`document` its compact text; a large one is read in parts and checked).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_measure_doc_get(
    core: *mut crate::FfiCore,
    id: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(id, "id").and_then(|i| c.hub_measure_doc_get(i.to_owned())))
    })
}

/// File a measurement, microphone or target document on the hub: JSON `{ok,
/// doc: {...}, created}`; a text that is not such a document fails
/// `invalid` with nothing sent.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_measure_doc_put(
    core: *mut crate::FfiCore,
    document: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(document, "document").and_then(|d| c.hub_measure_doc_put(d.to_owned())))
    })
}

/// Rename document `id` on the hub: JSON, its entry.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_measure_doc_rename(
    core: *mut crate::FfiCore,
    id: *const c_char,
    name: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(
            arg(id, "id")
                .and_then(|i| Ok((i, arg(name, "name")?)))
                .and_then(|(i, n)| c.hub_measure_doc_rename(i.to_owned(), n.to_owned())),
        )
    })
}

/// Set document `id`'s note on the hub: JSON, its entry.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_measure_doc_note(
    core: *mut crate::FfiCore,
    id: *const c_char,
    note: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(
            arg(id, "id")
                .and_then(|i| Ok((i, arg(note, "note")?)))
                .and_then(|(i, n)| c.hub_measure_doc_note(i.to_owned(), n.to_owned())),
        )
    })
}

/// Delete document `id` from the hub (Admin): JSON `{ok}` or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_measure_doc_delete(
    core: *mut crate::FfiCore,
    id: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        done_reply(arg(id, "id").and_then(|i| c.hub_measure_doc_delete(i.to_owned())))
    })
}

/// The documents of `kind` (null or "" every kind) on both sides, measurements
/// of `serial` (null or "": every device's here, the open device's on the
/// hub): JSON `{ok, entries: [{doc, where, localCopy}], hubListed,
/// hubNoticeKey}` (`where`: `local`, `hub` or `both`), or a failure for an
/// unknown kind.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_measure_docs_merged(
    core: *mut crate::FfiCore,
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
                    c.measure_docs_merged(k.unwrap_or_default(), s.unwrap_or_default())
                }),
        )
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_p5hub {
    use std::ffi::{CString, c_char};

    use serde_json::Value;

    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};

    fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> Value {
        let mut buf = vec![0u8; 65536];
        let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
        assert!(needed as usize <= buf.len(), "{needed}");
        let end = buf.iter().position(|b| *b == 0).unwrap();
        serde_json::from_slice(&buf[..end]).unwrap()
    }

    /// Off a hub: the core's own answers, through C.
    #[test]
    fn wp_p5hub_the_hub_measure_calls_through_c() {
        let h = dspi_core_new();
        let caps = reply(|b, l| super::super::hub_server::dspi_hub_caps(h, b, l));
        assert_eq!(caps["hubMeasure"], false, "{caps}");
        let id = CString::new("1-0").unwrap();
        let name = CString::new("n").unwrap();
        for r in [
            reply(|b, l| dspi_hub_measure_docs(h, std::ptr::null(), std::ptr::null(), b, l)),
            reply(|b, l| dspi_hub_measure_doc_get(h, id.as_ptr(), b, l)),
            reply(|b, l| dspi_hub_measure_doc_rename(h, id.as_ptr(), name.as_ptr(), b, l)),
            reply(|b, l| dspi_hub_measure_doc_note(h, id.as_ptr(), name.as_ptr(), b, l)),
            reply(|b, l| dspi_hub_measure_doc_delete(h, id.as_ptr(), b, l)),
        ] {
            assert_eq!(r["ok"], false, "{r}");
            assert_eq!(r["code"], "unsupported", "{r}");
        }
        let not_a_doc = CString::new("{}").unwrap();
        let r = reply(|b, l| dspi_hub_measure_doc_put(h, not_a_doc.as_ptr(), b, l));
        assert_eq!(r["code"], "invalid", "{r}");
        let r = reply(|b, l| dspi_measure_docs_merged(h, std::ptr::null(), std::ptr::null(), b, l));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["entries"], Value::Array(Vec::new()), "{r}");
        assert_eq!(r["hubListed"], false, "{r}");
        let bad = CString::new("nope").unwrap();
        let r = reply(|b, l| dspi_measure_docs_merged(h, bad.as_ptr(), std::ptr::null(), b, l));
        assert_eq!(r["code"], "invalid", "{r}");
        dspi_core_free(h);
    }
}
