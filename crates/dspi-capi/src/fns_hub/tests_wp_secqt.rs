//! WP SECQT: the compatibility mode's effects, as the hub lists them, for the
//! apps' warning.

use serde_json::Value;

use super::*;

#[test]
fn wp_secqt_compat_effects_are_the_hubs_own_lists() {
    let need = dspi_hub_upstream_compat_effects(ptr::null_mut(), 0);
    assert!(need > 1);
    let mut buf = vec![0u8; usize::try_from(need).unwrap()];
    let len = u32::try_from(buf.len()).unwrap();
    assert_eq!(
        dspi_hub_upstream_compat_effects(buf.as_mut_ptr().cast(), len),
        need
    );
    let text = std::str::from_utf8(&buf[..buf.len() - 1]).unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    assert_eq!(v["ok"], true);
    let effects: Vec<&str> = v["effects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e.as_str().unwrap())
        .collect();
    assert_eq!(effects, dspi_hub::server::UPSTREAM_COMPAT_EFFECTS);
    let off: Vec<&str> = v["tlsOffEffects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e.as_str().unwrap())
        .collect();
    assert_eq!(off, dspi_hub::server::TLS_OFF_EFFECTS);
    // A short buffer gets nothing written, and the same length
    let mut short = [7u8; 4];
    assert_eq!(
        dspi_hub_upstream_compat_effects(short.as_mut_ptr().cast(), 4),
        need
    );
    assert_eq!(short, [7u8; 4]);
}
