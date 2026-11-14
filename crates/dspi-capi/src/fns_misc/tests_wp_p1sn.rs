//! WP P1SN tests: snapshots through the C ABI on the demo device. The store
//! lives under `std::env::temp_dir()` (`ci/run.sh`: never tmpfs).

use std::ffi::{CStr, CString};
use std::ptr::null;

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new};

fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> String {
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

fn read_json(f: impl FnOnce(*mut c_char, u32) -> u32) -> serde_json::Value {
    serde_json::from_str(&reply(f)).unwrap()
}

#[test]
fn wp_p1sn_snapshots_through_the_c_abi() {
    let dir = std::env::temp_dir()
        .join("wp-p1sn-capi")
        .join(std::process::id().to_string());
    let _ = std::fs::remove_dir_all(&dir);
    let dir_c = CString::new(dir.to_string_lossy().as_bytes()).unwrap();
    let h = dspi_core_new();
    assert!(dspi_open_demo(h));

    // No folder yet: a typed failure.
    let name = CString::new("Abend ☾").unwrap();
    let r = read_json(|b, l| dspi_snapshot_capture(h, name.as_ptr(), null(), b, l));
    assert_eq!(
        (r["ok"].clone(), r["code"].clone()),
        (json!(false), json!("io")),
        "{r}"
    );

    let r = read_json(|b, l| dspi_snapshot_set_dir(h, dir_c.as_ptr(), b, l));
    assert_eq!(r["ok"], true, "{r}");
    let s = read_json(|b, l| dspi_snapshot_capture(h, name.as_ptr(), null(), b, l));
    assert_eq!(s["ok"], true, "{s}");
    assert_eq!(s["name"], "Abend ☾");
    assert_eq!(s["wireFormat"], 32);
    let id = CString::new(s["id"].as_str().unwrap()).unwrap();

    let list = read_json(|b, l| dspi_snapshot_list(h, null(), b, l));
    assert_eq!(list["value"].as_array().unwrap().len(), 1, "{list}");
    let renamed = CString::new("Nacht").unwrap();
    let r = read_json(|b, l| dspi_snapshot_rename(h, id.as_ptr(), renamed.as_ptr(), b, l));
    assert_eq!(r["name"], "Nacht");
    let note = CString::new("für später").unwrap();
    let r = read_json(|b, l| dspi_snapshot_set_note(h, id.as_ptr(), note.as_ptr(), b, l));
    assert_eq!(r["note"], "für später");

    // Change, recall by id: one bulk write back.
    with_core(h, (), |c| c.core.set_input_preamp(0, -6.0).unwrap());
    let r = read_json(|b, l| dspi_snapshot_recall(h, id.as_ptr(), b, l));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["image"], true);
    assert!(!r["changes"].as_array().unwrap().is_empty(), "{r}");

    // Export, then recall the document's text; import it back.
    let doc = reply(|b, l| dspi_snapshot_export(h, id.as_ptr(), b, l));
    let doc_c = CString::new(doc).unwrap();
    let r = read_json(|b, l| dspi_snapshot_recall_document(h, doc_c.as_ptr(), b, l));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["changes"], json!([]));
    let r = read_json(|b, l| dspi_snapshot_import(h, doc_c.as_ptr(), null(), null(), b, l));
    assert_eq!(r["name"], "Nacht", "{r}");
    let text = reply(|b, l| dspi_snapshot_capture_document(h, name.as_ptr(), null(), b, l));
    assert!(text.contains("\"kind\": \"snapshot\""));

    // Refusals keep their kind.
    let bad = CString::new("../x").unwrap();
    let r = read_json(|b, l| dspi_snapshot_recall(h, bad.as_ptr(), b, l));
    assert_eq!(r["code"], "invalid", "{r}");
    let r = read_json(|b, l| dspi_snapshot_delete(h, id.as_ptr(), b, l));
    assert_eq!(r["ok"], true);
    let r = read_json(|b, l| dspi_snapshot_list(h, null(), b, l));
    assert_eq!(r["value"].as_array().unwrap().len(), 1);
    assert_eq!(
        read_json(|b, l| dspi_snapshot_recall(h, null(), b, l))["code"],
        "invalid"
    );
    dspi_core_free(h);
    let _ = std::fs::remove_dir_all(&dir);
}
