//! C functions: offline editing and templates (WP P1OF), over
//! `dspi_core::offline`. Replies as in the parent module's docs: a document
//! (a saved document, a template's text) is the text itself, or the JSON
//! failure; the rest is JSON.

use std::ffi::c_char;

use super::{arg, document_reply, free_text, json_reply, text};

/// Open the `.dspipreset` document `doc` with no device, as the demo device
/// seeded from it; `file_name` is what the client shows ("File: <name>"; null
/// reads as empty). Replaces any open device; nothing touches USB. JSON
/// `{ok, fileName, name, kind, platform, inputs, outputs, notes}`, or the
/// failure (`invalid` for text that is no document).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_open_document(
    core: *mut crate::FfiCore,
    doc: *const c_char,
    file_name: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let file = super::c_str(file_name).unwrap_or("").to_owned();
        document_reply(arg(doc, "text").and_then(|d| c.open_document(d.to_owned(), file)))
    })
}

/// Whether the open connection is a document: JSON `{offline: true,
/// fileName, name, kind, platform}` or `{offline: false}`. No I/O.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_offline_document(
    core: *mut crate::FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| c.offline_document())
}

/// The open document as it now stands, of the kind it was opened as; the
/// edits become the baseline. The document, or the failure (`invalid` when
/// the connection is not a document).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_save_document(
    core: *mut crate::FfiCore,
    app_version: *const c_char,
    links: u8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        document_reply(
            arg(app_version, "app_version").and_then(|v| c.save_document(v.to_owned(), links)),
        )
    })
}

/// The open device or document as a template named `name` (`meta.kind`
/// "template"). The document, or the failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_save_as_template(
    core: *mut crate::FfiCore,
    name: *const c_char,
    app_version: *const c_char,
    links: u8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let made = arg(name, "name").and_then(|name| {
            let version = arg(app_version, "app_version")?;
            c.save_as_template(name.to_owned(), version.to_owned(), links)
        });
        document_reply(made)
    })
}

/// The templates there are, shipped ones first, then those in `user_dir`
/// (null or empty: none): a JSON array of `{id, name, builtin, platform,
/// inputs, outputs, savedUtc, status}`. Never fails; 0 only for a panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_template_list(user_dir: *const c_char, buf: *mut c_char, len: u32) -> u32 {
    free_text(buf, len, || {
        dspi_core::template_list(super::c_str(user_dir).unwrap_or("").to_owned())
    })
}

/// Template `id`'s text (`builtin:<name>`, or a file name in `user_dir`), or
/// the JSON failure. 0 only for a panic.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_template_text(
    id: *const c_char,
    user_dir: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    free_text(buf, len, || match arg(id, "id") {
        Ok(id) => document_reply(dspi_core::template_text(
            id.to_owned(),
            super::c_str(user_dir).unwrap_or("").to_owned(),
        )),
        Err(e) => json_reply::<()>(Err(e)),
    })
}

#[cfg(test)]
mod tests_wp_p1of {
    use std::ffi::{CStr, CString};
    use std::ptr::null;

    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};

    const MAC: &str = include_str!("../../../../tests/fixtures/preset/mac-swift-v32.dspipreset");

    fn read(f: impl FnOnce(*mut c_char, u32) -> u32) -> String {
        let mut buf = vec![0 as c_char; 512 * 1024];
        let needed = f(buf.as_mut_ptr(), buf.len() as u32);
        assert!(
            needed > 0 && needed as usize <= buf.len(),
            "needed {needed}"
        );
        // SAFETY: the call wrote a NUL-terminated reply into `buf`.
        unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    }

    fn json(text: &str) -> serde_json::Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn wp_p1of_open_save_and_templates_through_c() {
        let h = dspi_core_new();
        let doc = CString::new(MAC).unwrap();
        let file = CString::new("hall.dspipreset").unwrap();
        let v = CString::new("t").unwrap();
        let off = json(&read(|b, l| dspi_offline_document(h, b, l)));
        assert_eq!(off["offline"], false);
        let saved = json(&read(|b, l| dspi_save_document(h, v.as_ptr(), 0, b, l)));
        assert_eq!(saved["ok"], false);

        let r = json(&read(|b, l| {
            dspi_open_document(h, doc.as_ptr(), file.as_ptr(), b, l)
        }));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["outputs"], 9);
        let off = json(&read(|b, l| dspi_offline_document(h, b, l)));
        assert_eq!(off["fileName"], "hall.dspipreset");
        let saved = read(|b, l| dspi_save_document(h, v.as_ptr(), 0, b, l));
        assert_eq!(json(&saved)["meta"]["name"], "Mac reference");
        let name = CString::new("Hall").unwrap();
        let t = read(|b, l| dspi_save_as_template(h, name.as_ptr(), v.as_ptr(), 0, b, l));
        assert_eq!(json(&t)["meta"]["kind"], "template");

        let bad = CString::new("nope").unwrap();
        let r = json(&read(|b, l| {
            dspi_open_document(h, bad.as_ptr(), null(), b, l)
        }));
        assert_eq!(r["ok"], false);
        assert_eq!(r["code"], "invalid");
        let r = json(&read(|b, l| dspi_open_document(h, null(), null(), b, l)));
        assert_eq!(r["code"], "invalid");
        dspi_core_free(h);

        let list = json(&read(|b, l| dspi_template_list(null(), b, l)));
        // -- WP P1EFIX -- since TPL1 the generic templates come first; K340 is listed
        assert!(
            list.as_array()
                .unwrap()
                .iter()
                .any(|t| t["id"] == "builtin:k340"),
            "{list}"
        );
        let id = CString::new("builtin:k340").unwrap();
        let k340 = read(|b, l| dspi_template_text(id.as_ptr(), null(), b, l));
        assert_eq!(json(&k340)["meta"]["kind"], "template");
        let gone = CString::new("builtin:gone").unwrap();
        let r = json(&read(|b, l| {
            dspi_template_text(gone.as_ptr(), null(), b, l)
        }));
        assert_eq!(r["code"], "invalid");
    }
}
