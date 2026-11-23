//! WP CG-SWS-a tests: the stateless facts through C, each the same as the
//! core's call.

use std::ffi::CString;
use std::ptr::{null, null_mut};

use super::super::dspi_open_demo;
use super::*;
use crate::fns_device::{dspi_core_free, dspi_core_new, dspi_curve_sections};
use serde_json::Value; // -- WP INTG -- the module imports it with the link feature only

fn reply(f: impl Fn(*mut c_char, u32) -> u32) -> Value {
    let needed = f(null_mut(), 0);
    assert!(needed > 0);
    let mut buf = vec![0u8; needed as usize];
    assert_eq!(f(buf.as_mut_ptr().cast(), needed), needed);
    let end = buf.iter().position(|b| *b == 0).unwrap();
    serde_json::from_slice(&buf[..end]).unwrap()
}

fn plain(f: impl Fn(*mut c_char, u32) -> u32) -> String {
    let mut buf = vec![0u8; 64];
    let n = f(buf.as_mut_ptr().cast(), 64) as usize;
    String::from_utf8(buf[..n - 1].to_vec()).unwrap()
}

#[test]
fn cgswsa_c_quantizer_and_vreg_match_the_core() {
    for db in [-200.0, -127.8, -3.26, -0.1, 0.0, 4.0] {
        assert_eq!(
            dspi_quantize_master_volume(db),
            dspi_core::quantize_master_volume(db)
        );
    }
    assert!(dspi_quantize_master_volume(f32::NAN).is_nan());
    assert_eq!(dspi_vreg_code_mv(0x0D, 1), 1200);
    assert_eq!(dspi_vreg_code_mv(0x02, 0), 800);
    assert_eq!(dspi_vreg_code_mv(0x1E, 0), 0, "none on an RP2040");
    assert_eq!(plain(|b, l| dspi_vreg_code_label(0x0D, 1, b, l)), "1.20 V");
    assert_eq!(
        plain(|b, l| dspi_vreg_code_label(0x1E, 0, b, l)),
        "code 0x1E"
    );
    assert_eq!(dspi_vreg_code_label(0x0D, 1, null_mut(), 0), 7);
}

#[test]
fn cgswsa_c_tables_match_the_core() {
    let shape = reply(|b, l| dspi_platform_shape(1, b, l));
    assert_eq!(shape["ok"], true);
    assert_eq!(shape["name"], "RP2350");
    assert_eq!(shape["numOutputChannels"], 9);
    assert_eq!(shape["pdmPinIndex"], 4);
    assert_eq!(shape["maxDelayMs"], 42.0);

    let names = reply(|b, l| dspi_known_feature_names(b, l));
    assert_eq!(
        names["value"],
        serde_json::json!(dspi_core::known_feature_names())
    );

    let types = [0u8, 1];
    let dn = reply(|b, l| dspi_default_channel_names(2, 5, 0, types.as_ptr(), 2, b, l));
    assert_eq!(
        dn["value"],
        serde_json::json!([
            "USB 1",
            "USB 2",
            "SPDIF 1 L",
            "SPDIF 1 R",
            "I2S 2 L",
            "I2S 2 R",
            "PDM"
        ])
    );
    let dn = reply(|b, l| dspi_default_channel_names(0, 1, 0, null(), 9, b, l));
    assert_eq!(dn["value"], serde_json::json!(["PDM"]));
    let dn = reply(|b, l| dspi_default_channel_names(2, 0, 1, null(), 0, b, l));
    assert_eq!(dn["value"], serde_json::json!(["SPDIF L", "SPDIF R"]));

    let path = CString::new("sub.select").unwrap();
    let labels = reply(|b, l| dspi_enum_value_labels(path.as_ptr(), b, l));
    assert_eq!(labels["value"][1]["label"], "Percussive");
    assert_eq!(labels["value"][1]["key"], "percussive");
    let bad = CString::new("no.such").unwrap();
    let r = reply(|b, l| dspi_enum_value_labels(bad.as_ptr(), b, l));
    assert_eq!(
        (r["ok"].clone(), r["code"].clone()),
        (false.into(), "invalid".into())
    );
    assert_eq!(dspi_enum_value_labels(null(), null_mut(), 0), 0);
}

#[test]
fn cgswsa_c_curve_sections_match_the_core() {
    let band = crate::types::FilterParams {
        filter_type: 1,
        bypass: false,
        freq: 1000.0,
        q: 1.0,
        gain: 6.0,
        qp: 0.0,
    };
    let r = reply(|b, l| dspi_curve_sections(&band, 1, b, l));
    let core = dspi_core::curve_sections(vec![dspi_core::records::FilterParams::from(&band)]);
    let got = r["value"].as_array().unwrap();
    assert_eq!(got.len(), core.len());
    for (g, c) in got.iter().zip(&core) {
        for (k, want) in [
            ("b0", c.b0),
            ("b1", c.b1),
            ("b2", c.b2),
            ("a1", c.a1),
            ("a2", c.a2),
        ] {
            // JSON text keeps 17 significant digits; the parse may differ in the last bit.
            assert!((g[k].as_f64().unwrap() - want).abs() <= 1e-15, "{k}: {g}");
        }
    }
    let r = reply(|b, l| dspi_curve_sections(null(), 3, b, l));
    assert_eq!(r["value"], serde_json::json!([]));
}

#[test]
fn cgswsa_c_image_extras_follow_the_core() {
    assert_eq!(dspi_image_extras(null_mut(), null_mut(), 0), 0);
    let h = dspi_core_new();
    let r = reply(|b, l| dspi_image_extras(h, b, l));
    assert_eq!(r["decoded"], false);
    assert!(dspi_open_demo(h));
    let r = reply(|b, l| dspi_image_extras(h, b, l));
    assert_eq!(r["ok"], true);
    assert_eq!(r["decoded"], true);
    assert_eq!(r["spdifRxPinExtPresent"].as_array().unwrap().len(), 3);
    dspi_core_free(h);
}

#[cfg(feature = "link")]
#[test]
fn cgswsa_c_link_parse_txt() {
    let name = CString::new("Studio").unwrap();
    let txt = CString::new(r#"{"v":"1","hid":"ABC","kind":"bridge","d":"S1,S2"}"#).unwrap();
    let r = reply(|b, l| dspi_link_parse_txt(name.as_ptr(), txt.as_ptr(), b, l));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["hubId"], "abc");
    assert_eq!(r["deviceCount"], 2);
    assert_eq!(r["path"], "/dspi/v1");
    for bad in [r#"{"hid":"x"}"#, r#"{"v":1,"hid":"x"}"#, "[]", "not json"] {
        let t = CString::new(bad).unwrap();
        let r = reply(|b, l| dspi_link_parse_txt(null(), t.as_ptr(), b, l));
        assert_eq!(r["code"], "invalid", "{bad}: {r}");
    }
    assert_eq!(dspi_link_parse_txt(null(), null(), null_mut(), 0), 0);
}
