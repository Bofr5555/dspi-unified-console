//! WP P0E: the diagnostics bundle over the C ABI.

use std::ffi::{CStr, CString};
use std::ptr::{null, null_mut};

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new};
use crate::fns_misc::dspi_open_demo;

fn reply(f: impl Fn(*mut c_char, u32) -> u32) -> serde_json::Value {
    let mut buf = vec![0 as c_char; 1024];
    let n = f(buf.as_mut_ptr(), 1024);
    assert!(n > 0 && n <= 1024, "{n}");
    let s = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_str().unwrap();
    serde_json::from_str(s).unwrap()
}

/// A fresh directory. `ci/run.sh` points `TMPDIR` at the home volume, not tmpfs.
fn dir() -> std::path::PathBuf {
    let d = std::env::temp_dir()
        .join("wp-p0e-capi")
        .join(std::process::id().to_string());
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn wp_p0e_save_through_the_c_abi() {
    let h = dspi_core_new();
    assert!(dspi_open_demo(h));
    let d = dir();
    let path = CString::new(d.join("b.zip").to_string_lossy().as_bytes()).unwrap();
    let opts =
        CString::new(r#"{"app":"qt test","files":[{"name":"app.log","text":"started"}]}"#).unwrap();
    let r = reply(|b, l| dspi_diagnostics_save(h, path.as_ptr(), opts.as_ptr(), b, l));
    assert_eq!(r["ok"], true, "{r}");
    assert!(r["files"].as_u64().unwrap() >= 8, "{r}");
    let zip = std::fs::read(d.join("b.zip")).unwrap();
    let raw = String::from_utf8_lossy(&zip);
    assert!(raw.contains("client/app.log") && raw.contains("\"client\": \"qt test\""));

    // Null options are the defaults.
    let r = reply(|b, l| dspi_diagnostics_save(h, path.as_ptr(), null(), b, l));
    assert_eq!(r["ok"], true, "{r}");

    // Typed failures.
    let bad = CString::new("{not json").unwrap();
    let r = reply(|b, l| dspi_diagnostics_save(h, path.as_ptr(), bad.as_ptr(), b, l));
    assert_eq!(r["code"], "invalid", "{r}");
    let r = reply(|b, l| dspi_diagnostics_save(h, null(), null(), b, l));
    assert_eq!(r["code"], "invalid", "{r}");
    let nowhere = CString::new(d.join("no/such/b.zip").to_string_lossy().as_bytes()).unwrap();
    let r = reply(|b, l| dspi_diagnostics_save(h, nowhere.as_ptr(), null(), b, l));
    assert_eq!(r["code"], "io", "{r}");

    let r = {
        let mut buf = vec![0 as c_char; 64];
        let n = dspi_diagnostics_file_name(h, buf.as_mut_ptr(), 64);
        assert!(n > 1 && n <= 64);
        unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_str()
            .unwrap()
            .to_owned()
    };
    assert!(
        r.starts_with("dspi-diagnostics-") && r.ends_with(".zip"),
        "{r}"
    );

    assert_eq!(
        dspi_diagnostics_save(null_mut(), path.as_ptr(), null(), null_mut(), 0),
        0
    );
    dspi_core_free(h);
    let _ = std::fs::remove_dir_all(&d);
}
