//! WP SECDISC: `tls=1` both ways (PLAN-SEC 4.4, as the owner simplified it on
//! 2026-11-15: one TXT key, written whenever TLS is served), upstream records
//! still read, and TLS ranked first when two records claim one hub. No socket.

use std::net::Ipv4Addr;

use super::*;

const HID: &str = "3f2a7c1e-0000-4000-8000-00000000secd";
const WAIT: Duration = Duration::from_secs(5);

fn advert(name: &str, tls: bool) -> HubAdvert {
    HubAdvert {
        name: name.into(),
        port: 11915,
        hub_id: HID.into(),
        kind: "console".into(),
        auth: "pin".into(),
        device_count: 1,
        serials: vec!["E46058388B1A2E2C".into()],
        web: true,
        path: None,
        tls,
    }
}

fn addr() -> Vec<IpAddr> {
    vec![IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20))]
}

fn record_of(instance: &str, txt: Vec<(String, String)>) -> ServiceRecord {
    ServiceRecord {
        instance: FakeAdvertiser::instance(instance),
        port: 11915,
        addresses: addr(),
        txt,
        ..ServiceRecord::default()
    }
}

fn txt(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

#[test]
fn wp_secdisc_tls_round_trips() {
    let on = advert("Studio", true).txt();
    let keys: Vec<&str> = on.iter().map(|(k, _)| k.as_str()).collect();
    // Where macOS writes it: after `n`, before `web` (LinkDiscovery.swift).
    assert_eq!(keys, ["v", "hid", "kind", "auth", "n", "tls", "web", "d"]);
    assert_eq!(txt_value(&on, "tls"), Some("1"));
    let hub = parse_service(&record_of("Studio", on)).unwrap();
    assert!(hub.tls_advertised);
    assert_eq!(hub.id, HID);
    // The plaintext form: the connect rule picks the scheme.
    assert_eq!(hub.url, "ws://192.168.1.20:11915/dspi/v1");

    let off = advert("Studio", false).txt();
    assert_eq!(txt_value(&off, "tls"), None, "plaintext writes no key, as upstream");
    assert!(!parse_service(&record_of("Studio", off)).unwrap().tls_advertised);
}

/// An upstream WeebLabs hub (ref-swift `LinkDiscovery.swift` `txtData`, which never
/// sets `tls`) is listed exactly as before.
#[test]
fn wp_secdisc_an_upstream_record_without_tls_still_reads() {
    let r = record_of(
        "Kitchen Mac",
        txt(&[("v", "1"), ("hid", HID), ("kind", "console"), ("auth", "pin"), ("n", "1"), ("web", "1"), ("d", "X1")]),
    );
    let hub = parse_service(&r).unwrap();
    assert!(!hub.tls_advertised);
    assert_eq!(hub.name, "Kitchen Mac");
    assert_eq!(hub.auth, "pin");
    assert_eq!(hub.serials, ["X1"]);
    assert_eq!(hub.url, "ws://192.168.1.20:11915/dspi/v1");
}

/// `tls` counts only as the spec writes it; anything else is plaintext, and the hub
/// stays listed. A `tlsk` (the key the plan once had, which nothing writes) is an
/// unknown key: ignored, whatever it holds.
#[test]
fn wp_secdisc_odd_values_keep_the_hub_and_read_plaintext() {
    for value in ["0", "", "yes", "true", "11", "1\u{0}", "\u{1b}[2J"] {
        let r = record_of("Hub", txt(&[("v", "1"), ("hid", HID), ("tls", value)]));
        let hub = parse_service(&r).unwrap_or_else(|e| panic!("{value:?}: {e}"));
        assert!(!hub.tls_advertised, "{value:?}");
    }
    // Keys ignore case and the value is trimmed, as every other key.
    assert!(parse_service(&record_of("Hub", txt(&[("v", "1"), ("hid", HID), ("TLS", " 1 ")]))).unwrap().tls_advertised);
    // Of a repeated key the first counts (RFC 6763 6.4).
    let first_off = txt(&[("v", "1"), ("hid", HID), ("tls", "0"), ("tls", "1")]);
    assert!(!parse_service(&record_of("Hub", first_off)).unwrap().tls_advertised);
    let tlsk = txt(&[("v", "1"), ("hid", HID), ("tlsk", "!!not base64!!\u{7f}"), ("tlsk", "")]);
    let hub = parse_service(&record_of("Hub", tlsk)).unwrap();
    assert!(!hub.tls_advertised);
}

#[test]
fn wp_secdisc_tls_still_leaves_room_for_the_serials() {
    let mut a = advert("Studio", true);
    a.serials = (0..40).map(|i| format!("SERIAL{i:04}")).collect();
    let pairs = a.txt();
    assert_eq!(txt_value(&pairs, "tls"), Some("1"), "tls is never the one dropped");
    assert_eq!(txt_value(&pairs, "d"), None, "past 400 bytes `d` goes, as before");
    a.serials.truncate(3);
    assert!(txt_value(&a.txt(), "d").is_some());
}

#[test]
fn wp_secdisc_the_json_form_carries_the_flag() {
    let hub = parse_service(&record_of("Studio", advert("Studio", true).txt())).unwrap();
    let v = serde_json::to_value(&hub).unwrap();
    assert_eq!(v["tlsAdvertised"], true);
    let hub = parse_service(&record_of("Studio", advert("Studio", false).txt())).unwrap();
    assert_eq!(serde_json::to_value(&hub).unwrap()["tlsAdvertised"], false);
}

fn listed(list: &HubList) -> Vec<(String, bool)> {
    list.sorted().into_iter().map(|h| (h.name, h.tls_advertised)).collect()
}

/// A plaintext record does not push out another instance's TLS record for the same
/// `hid` (a one-packet downgrade), but the hub's own record may change either way.
#[test]
fn wp_secdisc_tls_is_ranked_first_for_one_hub() {
    let mut list = HubList::new(String::new(), Duration::from_secs(3));
    let t0 = Instant::now();
    list.apply(SourceEvent::Resolved(record_of("Studio", advert("Studio", true).txt())), t0);
    // Another instance claims the hub without TLS: ignored.
    list.apply(SourceEvent::Resolved(record_of("Studio (2)", advert("Studio (2)", false).txt())), t0);
    assert_eq!(listed(&list), [("Studio".to_owned(), true)]);
    // The same instance turning TLS off is the hub's own news (`--tls off`, or the
    // upstream compatibility mode): it applies.
    list.apply(SourceEvent::Resolved(record_of("Studio", advert("Studio", false).txt())), t0);
    assert_eq!(listed(&list), [("Studio".to_owned(), false)]);
    // A TLS record replaces a plaintext one, renamed or not: the latest news, as before.
    list.apply(SourceEvent::Resolved(record_of("Studio (2)", advert("Studio (2)", true).txt())), t0);
    assert_eq!(listed(&list), [("Studio (2)".to_owned(), true)]);
    // Once the TLS record has said goodbye, a plaintext successor is listed.
    list.apply(SourceEvent::Removed(FakeAdvertiser::instance("Studio (2)")), t0);
    list.apply(SourceEvent::Resolved(record_of("Studio", advert("Studio", false).txt())), t0);
    assert_eq!(listed(&list), [("Studio".to_owned(), false)]);
    assert!(list.leaving.is_empty(), "no timer left behind");
    // Other hubs are not affected by the rule.
    let mut other = advert("Other", false);
    other.hub_id = "id-other".into();
    list.apply(SourceEvent::Resolved(record_of("Other", other.txt())), t0);
    assert_eq!(listed(&list), [("Other".to_owned(), false), ("Studio".to_owned(), false)]);
}

/// The browser hands the flag out, and `resolve` (the reconnect path) keeps it.
#[test]
fn wp_secdisc_the_browser_reports_it() {
    let net = FakeAdvertiser::new();
    let options = BrowseOptions { removal_grace: Duration::ZERO, ..BrowseOptions::default() };
    let b = HubBrowser::start(Arc::new(net.clone()), None, options).unwrap();
    net.announce(&advert("Studio", true), &addr());
    let end = Instant::now() + WAIT;
    while b.resolve(HID).is_none() {
        assert!(Instant::now() < end, "timed out");
        thread::sleep(Duration::from_millis(2));
    }
    assert!(b.resolve(HID).unwrap().tls_advertised);
    assert!(b.snapshot().hubs.iter().all(|h| h.tls_advertised));
}
