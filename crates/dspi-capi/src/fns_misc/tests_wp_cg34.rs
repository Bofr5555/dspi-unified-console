//! WP CG34 tests: the filter types' names and a Linkwitz Transform by path
//! through C.

use std::ffi::CString;

use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new};

fn read(f: impl Fn(*mut c_char, u32) -> u32) -> String {
    let needed = f(ptr::null_mut(), 0);
    let mut buf = vec![0u8; needed as usize];
    assert_eq!(f(buf.as_mut_ptr().cast(), needed), needed);
    let end = buf.iter().position(|b| *b == 0).unwrap();
    String::from_utf8(buf[..end].to_vec()).unwrap()
}

#[test]
fn wp_cg34_the_names_through_c() {
    let r: serde_json::Value =
        serde_json::from_str(&read(|b, l| dspi_filter_type_names(b, l))).unwrap();
    assert_eq!(r["ok"], true, "{r}");
    let table = r["value"].as_array().unwrap();
    assert_eq!(table.len(), 46);
    assert_eq!(
        table[4],
        serde_json::json!({ "code": 4, "name": "High Cut (12dB)" })
    );
    assert_eq!(
        read(|b, l| dspi_filter_type_name(13, b, l)),
        "Low Cut (6dB)"
    );
    assert_eq!(read(|b, l| dspi_filter_type_name(34, b, l)), "LR4 Low pass"); // -- INT016 -- (A10)
    assert_eq!(read(|b, l| dspi_filter_type_name(20, b, l)), "Type 20");
}

#[test]
fn wp_cg34_a_linkwitz_band_by_path_through_c() {
    let h = dspi_core_new();
    assert!(dspi_open_demo(h));
    let write = |request: &str| -> serde_json::Value {
        let req = CString::new(request).unwrap();
        let mut buf = vec![0u8; 1024];
        dspi_write_param(h, req.as_ptr(), buf.as_mut_ptr().cast(), buf.len() as u32);
        let end = buf.iter().position(|b| *b == 0).unwrap();
        serde_json::from_slice(&buf[..end]).unwrap()
    };
    let r = write(r#"{"path":"eq.type","indices":[0,1],"value":{"choice":11}}"#);
    assert_eq!(r["ok"], true, "{r}");
    let r = write(r#"{"path":"eq.fp","indices":[0,1],"value":{"float":25}}"#);
    assert_eq!(r["value"]["float"], 25.0, "{r}");
    let r = write(r#"{"path":"eq.qp","indices":[0,1],"value":{"float":0.5}}"#);
    assert_eq!(r["value"]["float"], 0.5, "{r}");
    let r = write(r#"{"path":"eq.gain","indices":[0,1],"value":{"float":3}}"#);
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["code"], "invalid", "{r}");
    dspi_core_free(h);
}
