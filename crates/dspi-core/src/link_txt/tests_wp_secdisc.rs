//! WP SECDISC: `tls=1` reaches the platforms that browse with their own API
//! (iOS, Android) through the one parser, and an upstream record reads as before.

use super::*;

fn txt(pairs: &[(&str, &str)]) -> Vec<LinkTxtEntry> {
    pairs
        .iter()
        .map(|(k, v)| LinkTxtEntry {
            key: (*k).to_owned(),
            value: (*v).to_owned(),
        })
        .collect()
}

#[test]
fn wp_secdisc_link_parse_txt_reads_tls() {
    let base = [
        ("v", "1"),
        ("hid", "abc"),
        ("kind", "console"),
        ("auth", "pin"),
        ("n", "1"),
    ];
    let mut with = base.to_vec();
    with.push(("tls", "1"));
    let r = link_parse_txt("Studio".into(), txt(&with)).unwrap();
    assert!(r.tls_advertised);
    assert_eq!(r.hub_id, "abc");
    // Upstream (ref-swift `LinkDiscovery.swift`) writes no `tls`: plaintext, listed.
    let r = link_parse_txt("Kitchen".into(), txt(&base)).unwrap();
    assert!(!r.tls_advertised);
    let mut odd = base.to_vec();
    odd.push(("tls", "yes"));
    assert!(
        !link_parse_txt("Hub".into(), txt(&odd))
            .unwrap()
            .tls_advertised
    );
}

#[test]
fn wp_secdisc_the_json_form_carries_the_flag() {
    let r = link_parse_txt("Hub".into(), txt(&[("v", "1"), ("hid", "h"), ("tls", "1")])).unwrap();
    assert_eq!(serde_json::to_value(&r).unwrap()["tlsAdvertised"], true);
}
