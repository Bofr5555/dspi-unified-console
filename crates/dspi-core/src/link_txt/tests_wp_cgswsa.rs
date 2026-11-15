//! WP CG-SWS-a: the TXT reader, by the spec 3.1 keys and against the Mac's
//! `hub(fromTXT:)` (`apps/apple/DSPi Console/Link/HubBrowser.swift:583-597`).

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
fn cgswsa_link_parse_txt_reads_the_spec_keys() {
    let r = link_parse_txt(
        "Studio Mac".into(),
        txt(&[
            ("v", "1"),
            ("hid", "3F2A-ABC"),
            ("kind", "bridge"),
            ("auth", "none"),
            ("n", "2"),
            ("d", "SN1, SN2"),
            ("path", "/dspi/v2"),
            ("future", "ignored"),
        ]),
    )
    .unwrap();
    assert_eq!(
        r,
        LinkTxtRecord {
            hub_id: "3f2a-abc".into(),
            name: "Studio Mac".into(),
            kind: "bridge".into(),
            auth: "none".into(),
            device_count: 2,
            serials: vec!["SN1".into(), "SN2".into()],
            path: "/dspi/v2".into(),
            tls_advertised: false, // -- WP SECDISC --
        }
    );
}

/// Where the Mac and the core agree: the same keys give the same hub.
#[test]
fn cgswsa_link_parse_txt_agrees_with_swift_on_a_full_record() {
    let r = link_parse_txt(
        "Kitchen._dspi._tcp.local.".into(),
        txt(&[
            ("v", "1"),
            ("hid", "abc"),
            ("kind", "console"),
            ("auth", "pin"),
            ("n", "1"),
            ("d", "X1"),
        ]),
    )
    .unwrap();
    // HubBrowser.swift:586-594: id hid, kind, authMode, deviceCount, serials,
    // path "/dspi/v1" by default.
    assert_eq!(r.hub_id, "abc");
    assert_eq!(r.name, "Kitchen");
    assert_eq!(r.kind, "console");
    assert_eq!(r.auth, "pin");
    assert_eq!(r.device_count, 1);
    assert_eq!(r.serials, ["X1"]);
    assert_eq!(r.path, "/dspi/v1");
}

#[test]
fn cgswsa_link_parse_txt_falls_back_and_refuses_like_the_spec() {
    // Defaults: kind unknown, auth pin, n from d, the default path; keys in any case.
    let r = link_parse_txt(
        "".into(),
        txt(&[
            ("V", "2"),
            ("HID", "x"),
            ("auth", "weird"),
            ("d", "a,b,c"),
            ("path", "no slash"),
        ]),
    )
    .unwrap();
    assert_eq!((r.kind.as_str(), r.auth.as_str()), ("unknown", "pin"));
    assert_eq!(r.device_count, 3);
    assert_eq!(r.path, "/dspi/v1");
    assert_eq!(r.name, "x", "no instance name: the hid");
    for bad in [
        txt(&[("hid", "x")]),
        txt(&[("v", "0"), ("hid", "x")]),
        txt(&[("v", "1")]),
        txt(&[("v", "1"), ("hid", "")]),
        // -- WP SECDISC -- `tls=1` is a hub now (tests_wp_secdisc.rs)
        Vec::new(),
    ] {
        assert!(
            matches!(
                link_parse_txt("Hub".into(), bad.clone()),
                Err(DspiError::InvalidArgument { .. })
            ),
            "{bad:?}"
        );
    }
}

#[test]
fn cgswsa_link_parse_txt_never_panics_on_odd_text() {
    let long = "x".repeat(10_000);
    for (k, v) in [
        ("hid", long.as_str()),
        ("d", long.as_str()),
        ("n", "-5"),
        ("n", "99999999999999"),
        ("kind", "\u{0}\u{7}"),
    ] {
        let mut t = txt(&[("v", "1"), ("hid", "ok")]);
        t.insert(
            0,
            LinkTxtEntry {
                key: k.to_owned(),
                value: v.to_owned(),
            },
        );
        let _ = link_parse_txt(long.clone(), t);
    }
    let r = link_parse_txt("A\u{7}B".into(), txt(&[("v", "1"), ("hid", "h")])).unwrap();
    assert!(!r.name.contains('\u{7}'));
}

#[test]
fn cgswsa_link_parse_txt_takes_the_first_of_a_repeated_key() {
    let r = link_parse_txt(
        "H".into(),
        txt(&[
            ("v", "1"),
            ("hid", "first"),
            ("HID", "second"),
            ("hid", "third"),
        ]),
    )
    .unwrap();
    assert_eq!(r.hub_id, "first");
}
