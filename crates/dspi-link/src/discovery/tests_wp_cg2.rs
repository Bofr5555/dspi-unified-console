//! WP CG2: the TXT record both ways, and the browser against a fake advertiser.
//! Nothing here opens a socket, except the one ignored test at the end.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::atomic::AtomicUsize;

use super::*;

const HID: &str = "3f2a7c1e-0000-4000-8000-000000000001";
const WAIT: Duration = Duration::from_secs(5);

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap()
}

fn txt(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

fn record(pairs: &[(&str, &str)]) -> ServiceRecord {
    ServiceRecord {
        instance: "Studio Mac._dspi._tcp.local.".into(),
        port: 11915,
        addresses: vec![ip("192.168.1.20")],
        txt: txt(pairs),
        ..ServiceRecord::default() // -- WP DISC2 --
    }
}

const SPEC: &[(&str, &str)] = &[("v", "1"), ("hid", HID), ("kind", "console"), ("auth", "pin"), ("n", "1")];

fn advert(name: &str, id: &str) -> HubAdvert {
    HubAdvert {
        name: name.into(),
        port: 11915,
        hub_id: id.into(),
        kind: "console".into(),
        auth: "pin".into(),
        device_count: 1,
        serials: vec!["E46058388B1A2E2C".into()],
        web: false,
        path: None,
        tls: false, // -- WP SECDISC --
    }
}

#[test]
fn wp_cg2_a_spec_record_reads_as_a_hub() {
    let mut r = record(SPEC);
    r.txt.extend(txt(&[("d", "AA, BB,,CC"), ("web", "1"), ("future", "ignored")]));
    let hub = parse_service(&r).unwrap();
    assert_eq!(hub.id, HID);
    assert_eq!(hub.name, "Studio Mac");
    assert_eq!(hub.kind, "console");
    assert_eq!(hub.auth, "pin");
    assert_eq!(hub.device_count, 1);
    assert_eq!(hub.serials, ["AA", "BB", "CC"]);
    assert_eq!(hub.host, "192.168.1.20");
    assert_eq!(hub.port, 11915);
    assert_eq!(hub.path, "/dspi/v1");
    assert_eq!(hub.url, "ws://192.168.1.20:11915/dspi/v1");
}

#[test]
fn wp_cg2_keys_ignore_case_and_the_first_wins_and_ids_are_lowered() {
    let r = record(&[("V", "1"), ("HID", "ABC-DEF"), ("hid", "other"), ("AUTH", "none"), ("Kind", "bridge")]);
    let hub = parse_service(&r).unwrap();
    assert_eq!(hub.id, "abc-def");
    assert_eq!(hub.auth, "none");
    assert_eq!(hub.kind, "bridge");
}

#[test]
fn wp_cg2_a_later_major_version_is_listed_since_v_is_the_highest_served() {
    assert!(parse_service(&record(&[("v", "2"), ("hid", HID)])).is_ok());
}

#[test]
fn wp_cg2_what_cannot_be_used_is_rejected_with_its_reason() {
    assert_eq!(parse_service(&record(&[("hid", HID)])), Err(Rejected::Version));
    assert_eq!(parse_service(&record(&[("v", "0"), ("hid", HID)])), Err(Rejected::Version));
    assert_eq!(parse_service(&record(&[("v", "x"), ("hid", HID)])), Err(Rejected::Version));
    assert_eq!(parse_service(&record(&[("v", "1")])), Err(Rejected::HubId));
    assert_eq!(parse_service(&record(&[("v", "1"), ("hid", "")])), Err(Rejected::HubId));
    assert_eq!(parse_service(&record(&[("v", "1"), ("hid", "a b")])), Err(Rejected::HubId));
    assert_eq!(parse_service(&record(&[("v", "1"), ("hid", "a\u{1b}[2J")])), Err(Rejected::HubId));
    let long = "a".repeat(MAX_HUB_ID_BYTES + 1);
    assert_eq!(parse_service(&record(&[("v", "1"), ("hid", &long)])), Err(Rejected::HubId));
    // -- WP SECDISC -- `tls=1` is listed now (tests_wp_secdisc.rs)
    assert!(parse_service(&record(&[("v", "1"), ("hid", HID), ("tls", "1")])).unwrap().tls_advertised);
    assert!(!parse_service(&record(&[("v", "1"), ("hid", HID), ("tls", "0")])).unwrap().tls_advertised);
    let mut r = record(SPEC);
    r.port = 0;
    assert_eq!(parse_service(&r), Err(Rejected::Port));
    r = record(SPEC);
    r.addresses = vec![ip("fe80::1"), ip("0.0.0.0"), ip("224.0.0.251"), ip("::")];
    assert_eq!(parse_service(&r), Err(Rejected::Address));
    r.addresses.clear();
    assert_eq!(parse_service(&r), Err(Rejected::Address));
}

#[test]
fn wp_cg2_nonsense_in_optional_keys_falls_back() {
    let hub = parse_service(&record(&[
        ("v", "1"),
        ("hid", HID),
        ("kind", "toaster"),
        ("auth", "maybe"),
        ("n", "many"),
        ("d", "A,B"),
        ("path", "no-slash"),
    ]))
    .unwrap();
    assert_eq!(hub.kind, "unknown");
    assert_eq!(hub.auth, "pin", "an unknown mode must not read as open");
    assert_eq!(hub.device_count, 2, "n falls back to the list");
    assert_eq!(hub.path, DEFAULT_PATH);
}

#[test]
fn wp_cg2_a_custom_path_goes_in_the_url_and_a_hostile_one_does_not() {
    let hub = parse_service(&record(&[("v", "1"), ("hid", HID), ("path", "/link/v1")])).unwrap();
    assert_eq!(hub.url, "ws://192.168.1.20:11915/link/v1");
    for bad in ["/a b", "/a\r\nHost: evil", "/a?b=c", "/%2e%2e", "//@evil/"] {
        let hub = parse_service(&record(&[("v", "1"), ("hid", HID), ("path", bad)])).unwrap();
        assert_eq!(hub.path, DEFAULT_PATH, "{bad:?}");
        assert!(!hub.url.contains(['\r', '\n', ' ', '@', '?']), "{bad:?}: {}", hub.url);
    }
    let long = format!("/{}", "a".repeat(MAX_PATH_BYTES));
    let hub = parse_service(&record(&[("v", "1"), ("hid", HID), ("path", &long)])).unwrap();
    assert_eq!(hub.path, DEFAULT_PATH);
}

#[test]
fn wp_cg2_what_a_hub_says_is_made_safe_and_bounded() {
    let mut r = record(SPEC);
    r.instance = format!("Evil\u{1b}[2J\n{}._dspi._tcp.local.", "x".repeat(1000));
    let many: Vec<String> = (0..1000).map(|i| format!("S{i}")).collect();
    let mut with_junk = many.join(",");
    with_junk.push_str(&format!(",{}", "y".repeat(MAX_SERIAL_BYTES + 1)));
    r.txt.push(("d".into(), with_junk));
    r.addresses = (0..=255).map(|i| IpAddr::V4(Ipv4Addr::new(10, 0, 0, i))).collect();
    let hub = parse_service(&r).unwrap();
    assert!(!hub.name.chars().any(char::is_control), "{:?}", hub.name);
    assert!(hub.name.len() <= MAX_NAME_BYTES);
    assert_eq!(hub.serials.len(), MAX_SERIALS);
    assert_eq!(hub.addresses.len(), MAX_ADDRESSES);

    let mut r = record(&[("v", "1"), ("hid", HID), ("d", "A\u{7}B")]);
    r.instance = "\u{1b}\u{1b}._dspi._tcp.local.".into();
    let hub = parse_service(&r).unwrap();
    assert_eq!(hub.serials, ["A\u{fffd}B"]);
    assert_eq!(hub.name, "\u{fffd}\u{fffd}");

    r.instance = "._dspi._tcp.local.".into();
    assert_eq!(parse_service(&r).unwrap().name, HID, "a nameless hub shows its id");
}

#[test]
fn wp_cg2_no_input_panics() {
    // Multi-byte text cut anywhere, escapes cut short, and odd numbers.
    let fragments = ["\\", "\\1", "\\12", "\\999", "\\.", "é", "日本", "._dspi._tcp.local.", ".", ""];
    for a in fragments {
        for b in fragments {
            let mut r = record(SPEC);
            r.instance = format!("{a}{b}");
            let hub = parse_service(&r).unwrap();
            assert!(hub.name.len() <= MAX_NAME_BYTES);
            r.txt = txt(&[("v", a), ("hid", b), ("n", a), ("d", b), ("path", a), ("kind", b)]);
            let _ = parse_service(&r);
        }
    }
    assert_eq!(bounded("日本", 4), "日");
    assert_eq!(bounded("日本", 0), "");
    assert_eq!(bounded("abc", 99), "abc");
}

#[test]
fn wp_cg2_instance_names_are_unescaped() {
    assert_eq!(instance_label("Studio\\.Mac._dspi._tcp.local."), "Studio.Mac");
    assert_eq!(instance_label("Back\\\\slash._dspi._tcp.local."), "Back\\slash");
    assert_eq!(instance_label("DSPi\\032Bridge._DSPI._TCP.LOCAL."), "DSPi Bridge");
    assert_eq!(instance_label("Caf\\195\\169._dspi._tcp.local."), "Café");
    assert_eq!(instance_label("over\\999._dspi._tcp.local."), "over?");
    assert_eq!(instance_label("trailing\\"), "trailing\\");
    assert_eq!(instance_label("no service type"), "no service type");
}

#[test]
fn wp_cg2_addresses_prefer_what_a_url_can_reach() {
    let mut r = record(SPEC);
    r.addresses = vec![
        ip("fe80::1"),
        ip("2001:db8::5"),
        ip("169.254.3.4"),
        ip("192.168.1.20"),
        ip("10.0.0.7"),
        ip("10.0.0.7"),
        ip("fd00::9"),
    ];
    let hub = parse_service(&r).unwrap();
    assert_eq!(hub.addresses, ["10.0.0.7", "192.168.1.20", "2001:db8::5", "fd00::9", "169.254.3.4"]);
    assert_eq!(hub.host, "10.0.0.7");

    r.addresses = vec![IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 5)), ip("fe80::1")];
    let hub = parse_service(&r).unwrap();
    assert_eq!(hub.url, "ws://[2001:db8::5]:11915/dspi/v1", "IPv6 is bracketed");
    assert_eq!(address::parse_address("[2001:db8::5]:11915"), Some(("2001:db8::5".into(), 11915)));
}

#[test]
fn wp_cg2_an_advertisement_reads_back_as_itself() {
    let mut a = advert("Studio", HID);
    a.serials = vec!["AA".into(), "BB".into()];
    a.device_count = 2;
    a.path = Some("/x".into());
    a.web = true;
    let t = a.txt();
    assert_eq!(
        t.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
        ["v", "hid", "kind", "auth", "n", "web", "path", "d"]
    );
    let hub = parse_service(&ServiceRecord {
        instance: FakeAdvertiser::instance("Studio"),
        port: 4000,
        addresses: vec![ip("10.0.0.1")],
        txt: t,
        ..ServiceRecord::default() // -- WP DISC2 --
    })
    .unwrap();
    assert_eq!(
        (hub.id.as_str(), hub.name.as_str(), hub.device_count, hub.path.as_str(), hub.port),
        (HID, "Studio", 2, "/x", 4000)
    );
    assert_eq!(hub.serials, ["AA", "BB"]);
}

#[test]
fn wp_cg2_a_long_device_list_stays_out_of_the_record() {
    let size = |t: &[(String, String)]| t.iter().map(|(k, v)| 2 + k.len() + v.len()).sum::<usize>();
    let mut a = advert("Studio", HID);
    let others = {
        a.serials.clear();
        size(&a.txt())
    };
    // `d=` and its length byte are 3 bytes: fits at 399 in all, not at 400.
    let room = TXT_LIMIT - others - 3;
    a.serials = vec!["x".repeat(room - 1)];
    assert!(a.txt().iter().any(|(k, _)| k == "d"));
    assert_eq!(size(&a.txt()), TXT_LIMIT - 1);
    a.serials = vec!["x".repeat(room)];
    assert!(!a.txt().iter().any(|(k, _)| k == "d"));
}

/// Collects listener calls.
#[derive(Default)]
struct Heard {
    calls: AtomicUsize,
    last: Mutex<Option<BrowseSnapshot>>,
}

impl BrowseListener for Heard {
    fn on_hubs_changed(&self, snapshot: BrowseSnapshot) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.last.lock().unwrap() = Some(snapshot);
    }
}

fn fast() -> BrowseOptions {
    BrowseOptions {
        own_hub_id: String::new(),
        retry_first: Duration::from_millis(5),
        retry_max: Duration::from_millis(40),
        // -- WP DISC2 -- These tests are about the list, not the grace (tests_wp_disc2.rs).
        removal_grace: Duration::ZERO,
    }
}

fn browse(net: &FakeAdvertiser, heard: &Arc<Heard>, options: BrowseOptions) -> HubBrowser {
    HubBrowser::start(Arc::new(net.clone()), Some(Arc::clone(heard) as Arc<dyn BrowseListener>), options).unwrap()
}

fn until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + WAIT;
    while !f() {
        assert!(Instant::now() < end, "timed out waiting: {what}");
        thread::sleep(Duration::from_millis(2));
    }
}

fn names(b: &HubBrowser) -> Vec<String> {
    b.snapshot().hubs.into_iter().map(|h| h.name).collect()
}

#[test]
fn wp_cg2_hubs_come_and_go_and_the_listener_hears() {
    let net = FakeAdvertiser::new();
    let heard = Arc::new(Heard::default());
    let b = browse(&net, &heard, fast());
    until("browsing", || b.snapshot().browsing);

    net.announce(&advert("beta", "id-b"), &[ip("10.0.0.2")]);
    net.announce(&advert("Alpha", "id-a"), &[ip("10.0.0.1")]);
    until("both hubs", || b.snapshot().hubs.len() == 2);
    assert_eq!(names(&b), ["Alpha", "beta"], "sorted by name, ignoring case");
    until("listener", || heard.last.lock().unwrap().as_ref().is_some_and(|s| s.hubs.len() == 2));

    net.withdraw("beta");
    until("goodbye", || names(&b) == ["Alpha"]);
    until("listener hears", || heard.last.lock().unwrap().as_ref().is_some_and(|s| s.hubs.len() == 1));
}

#[test]
fn wp_cg2_hubs_already_there_are_found() {
    let net = FakeAdvertiser::new();
    net.announce(&advert("Early", "id-e"), &[ip("10.0.0.1")]);
    let heard = Arc::new(Heard::default());
    let b = browse(&net, &heard, fast());
    until("found", || names(&b) == ["Early"]);
}

#[test]
fn wp_cg2_the_listener_hears_only_changes() {
    let net = FakeAdvertiser::new();
    let heard = Arc::new(Heard::default());
    let b = browse(&net, &heard, fast());
    net.announce(&advert("A", "id-a"), &[ip("10.0.0.1")]);
    until("found", || names(&b) == ["A"]);
    until("heard", || heard.last.lock().unwrap().as_ref().is_some_and(|s| s.hubs.len() == 1));
    let calls = heard.calls.load(Ordering::SeqCst);
    for _ in 0..20 {
        net.announce(&advert("A", "id-a"), &[ip("10.0.0.1")]);
    }
    // A marker after the repeats: when it arrives they have all been handled.
    net.announce(&advert("Z", "id-z"), &[ip("10.0.0.9")]);
    until("marker", || names(&b) == ["A", "Z"]);
    until("heard", || heard.calls.load(Ordering::SeqCst) > calls);
    assert_eq!(heard.calls.load(Ordering::SeqCst), calls + 1);
}

/// F5: the hub's DHCP lease changes under a session.
#[test]
fn wp_cg2_resolve_by_hub_id_follows_a_new_address() {
    let net = FakeAdvertiser::new();
    let heard = Arc::new(Heard::default());
    let b = browse(&net, &heard, fast());
    assert_eq!(b.resolve(HID), None);
    net.announce(&advert("Hub", HID), &[ip("192.168.1.20")]);
    until("found", || b.resolve(HID).is_some());
    assert_eq!(b.resolve(&HID.to_uppercase()).unwrap().host, "192.168.1.20", "any case");

    net.announce(&advert("Hub", HID), &[ip("192.168.1.77")]);
    until("new lease", || b.resolve(HID).is_some_and(|h| h.url == "ws://192.168.1.77:11915/dspi/v1"));
    assert_eq!(b.snapshot().hubs.len(), 1);

    net.withdraw("Hub");
    until("gone", || b.resolve(HID).is_none());
}

#[test]
fn wp_cg2_changed_data_replaces_the_old() {
    let net = FakeAdvertiser::new();
    let heard = Arc::new(Heard::default());
    let b = browse(&net, &heard, fast());
    let mut a = advert("Hub", HID);
    net.announce(&a, &[ip("10.0.0.1")]);
    until("found", || b.resolve(HID).is_some());
    a.device_count = 3;
    a.serials = vec!["A".into(), "B".into(), "C".into()];
    net.announce(&a, &[ip("10.0.0.1")]);
    until("updated", || b.resolve(HID).is_some_and(|h| h.device_count == 3 && h.serials.len() == 3));
}

#[test]
fn wp_cg2_our_own_hub_is_not_listed() {
    let net = FakeAdvertiser::new();
    let heard = Arc::new(Heard::default());
    let b = browse(&net, &heard, BrowseOptions { own_hub_id: HID.to_uppercase(), ..fast() });
    net.announce(&advert("Me", HID), &[ip("10.0.0.1")]);
    net.announce(&advert("Other", "id-o"), &[ip("10.0.0.2")]);
    until("other", || names(&b) == ["Other"]);
    assert_eq!(b.resolve(HID), None);
}

#[test]
fn wp_cg2_a_renamed_hub_is_one_hub() {
    let net = FakeAdvertiser::new();
    let heard = Arc::new(Heard::default());
    let b = browse(&net, &heard, fast());
    net.announce(&advert("Old name", HID), &[ip("10.0.0.1")]);
    until("old", || names(&b) == ["Old name"]);
    net.announce(&advert("New name", HID), &[ip("10.0.0.1")]);
    until("new", || names(&b) == ["New name"]);
    net.withdraw("Old name");
    net.announce(&advert("Marker", "id-m"), &[ip("10.0.0.9")]);
    until("old's goodbye does not take the new", || names(&b) == ["Marker", "New name"]);
}

#[test]
fn wp_cg2_a_service_that_stops_making_sense_leaves() {
    let net = FakeAdvertiser::new();
    let heard = Arc::new(Heard::default());
    let b = browse(&net, &heard, fast());
    net.announce(&advert("Hub", HID), &[ip("10.0.0.1")]);
    until("found", || names(&b) == ["Hub"]);
    net.announce_record(ServiceRecord {
        instance: FakeAdvertiser::instance("Hub"),
        port: 11915,
        addresses: vec![ip("10.0.0.1")],
        txt: txt(&[("v", "1")]),
        ..ServiceRecord::default() // -- WP DISC2 --
    });
    until("gone", || b.snapshot().hubs.is_empty());
}

#[test]
fn wp_cg2_a_flood_of_hubs_is_capped() {
    let net = FakeAdvertiser::new();
    let heard = Arc::new(Heard::default());
    let b = browse(&net, &heard, fast());
    for i in 0..MAX_HUBS + 50 {
        net.announce(&advert(&format!("h{i}"), &format!("id{i}")), &[ip("10.0.0.1")]);
    }
    net.withdraw("h0");
    net.announce(&advert("after", "id-after"), &[ip("10.0.0.1")]);
    until("a place freed is used", || b.resolve("id-after").is_some());
    until("as many as allowed, no more", || b.snapshot().hubs.len() == MAX_HUBS);
    net.announce(&advert("marker", "id-marker"), &[ip("10.0.0.1")]);
    net.withdraw("marker");
    thread::sleep(Duration::from_millis(20));
    assert_eq!(b.snapshot().hubs.len(), MAX_HUBS);
}

#[test]
fn wp_cg2_a_source_that_cannot_open_is_an_error_state_that_recovers() {
    let net = FakeAdvertiser::new();
    net.fail_next_opens(&["no network", "still none"]);
    net.announce(&advert("Hub", HID), &[ip("10.0.0.1")]);
    let seen = Arc::new(Mutex::new(Vec::new()));
    struct Errors(Arc<Mutex<Vec<Option<String>>>>);
    impl BrowseListener for Errors {
        fn on_hubs_changed(&self, s: BrowseSnapshot) {
            self.0.lock().unwrap().push(s.error);
        }
    }
    let b = HubBrowser::start(Arc::new(net.clone()), Some(Arc::new(Errors(Arc::clone(&seen)))), fast()).unwrap();
    until("recovered", || names(&b) == ["Hub"] && b.snapshot().browsing);
    assert_eq!(b.snapshot().error, None, "cleared by itself");
    assert_eq!(net.opens(), 3);
    let seen = seen.lock().unwrap();
    assert!(seen[0].as_deref().is_some_and(|e| e.contains("no network")), "{seen:?}");
    assert!(seen.iter().any(|e| e.as_deref().is_some_and(|e| e.contains("still none"))), "{seen:?}");
}

#[test]
fn wp_cg2_a_source_that_fails_is_opened_again_and_reports_afresh() {
    let net = FakeAdvertiser::new();
    let heard = Arc::new(Heard::default());
    // A wait long enough to see the failed state in between.
    let b = browse(
        &net,
        &heard,
        BrowseOptions { retry_first: Duration::from_millis(300), retry_max: Duration::from_millis(300), ..fast() },
    );
    net.announce(&advert("Hub", HID), &[ip("10.0.0.1")]);
    until("found", || names(&b) == ["Hub"]);
    let stale = net.stale_sink().unwrap();

    net.fail("daemon died");
    until("failed state", || {
        let s = b.snapshot();
        !s.browsing && s.hubs.is_empty() && s.error.as_deref() == Some("daemon died")
    });
    assert_eq!(net.open_now(), 0, "the failed session is closed");

    // The dead source's last words do not count.
    stale.send(SourceEvent::Resolved(record(SPEC)));
    until("reopened", || b.snapshot().browsing);
    until("afresh", || names(&b) == ["Hub"]);
    assert_eq!(b.snapshot().error, None);
    assert_eq!(net.opens(), 2);
    assert_eq!(net.open_now(), 1);
    stale.send(SourceEvent::Resolved(record(&[("v", "1"), ("hid", "stale")])));
    net.announce(&advert("Marker", "id-m"), &[ip("10.0.0.9")]);
    until("marker", || names(&b) == ["Hub", "Marker"]);
    assert_eq!(b.resolve("stale"), None);
}

#[test]
fn wp_cg2_the_wait_before_reopening_grows_to_its_limit() {
    let net = FakeAdvertiser::new();
    net.fail_next_opens(&["1", "2", "3", "4", "5", "6"]);
    let heard = Arc::new(Heard::default());
    let started = Instant::now();
    let b = browse(
        &net,
        &heard,
        BrowseOptions {
            retry_first: Duration::from_millis(10),
            retry_max: Duration::from_millis(40),
            ..BrowseOptions::default()
        },
    );
    until("recovered", || b.snapshot().browsing);
    // 10 + 20 + 40 + 40 + 40 + 40.
    assert!(started.elapsed() >= Duration::from_millis(190), "{:?}", started.elapsed());
    assert_eq!(net.opens(), 7);
}

#[test]
fn wp_cg2_default_options_wait_as_macos_does() {
    let o = BrowseOptions::default();
    assert_eq!((o.retry_first, o.retry_max), (Duration::from_secs(1), Duration::from_secs(30)));
    assert_eq!(o.own_hub_id, "");
    // -- WP DISC2 --
    assert_eq!(o.removal_grace, Duration::from_secs(3));
}

#[test]
fn wp_cg2_stop_closes_forgets_and_silences() {
    let net = FakeAdvertiser::new();
    let heard = Arc::new(Heard::default());
    let mut b = browse(&net, &heard, fast());
    net.announce(&advert("Hub", HID), &[ip("10.0.0.1")]);
    until("found", || names(&b) == ["Hub"]);
    let stale = net.stale_sink().unwrap();

    b.stop();
    assert_eq!(b.snapshot(), BrowseSnapshot::default());
    assert_eq!(b.resolve(HID), None);
    assert_eq!(net.open_now(), 0);
    let calls = heard.calls.load(Ordering::SeqCst);
    assert!(!stale.send(SourceEvent::Removed("x".into())), "the sink knows the browser has gone");
    net.announce(&advert("Late", "id-l"), &[ip("10.0.0.2")]);
    thread::sleep(Duration::from_millis(20));
    assert_eq!(heard.calls.load(Ordering::SeqCst), calls);
    b.stop();
    b.verify(HID);
}

#[test]
fn wp_cg2_dropping_the_browser_stops_it() {
    let net = FakeAdvertiser::new();
    let heard = Arc::new(Heard::default());
    let b = browse(&net, &heard, fast());
    until("open", || net.open_now() == 1);
    drop(b);
    assert_eq!(net.open_now(), 0);
}

/// A stop waiting for a listener call cannot be stopped by that call otherwise.
#[test]
fn wp_cg2_stop_from_the_listener_does_not_hang() {
    struct Stopper(Mutex<Option<HubBrowser>>, AtomicUsize);
    impl BrowseListener for Stopper {
        fn on_hubs_changed(&self, s: BrowseSnapshot) {
            if s.hubs.is_empty() {
                return;
            }
            self.1.fetch_add(1, Ordering::SeqCst);
            // Taken out of the mutex first, as dspi-core does, and dropped here.
            if let Some(b) = self.0.lock().unwrap().take() {
                drop(b);
            }
        }
    }
    let net = FakeAdvertiser::new();
    let stopper = Arc::new(Stopper(Mutex::new(None), AtomicUsize::new(0)));
    let b = HubBrowser::start(Arc::new(net.clone()), Some(Arc::clone(&stopper) as Arc<dyn BrowseListener>), fast())
        .unwrap();
    *stopper.0.lock().unwrap() = Some(b);
    net.announce(&advert("Hub", HID), &[ip("10.0.0.1")]);
    until("stopped from inside", || stopper.0.lock().unwrap().is_none());
    until("session closed", || net.open_now() == 0);
    assert_eq!(stopper.1.load(Ordering::SeqCst), 1);
}

/// `stop` on another thread waits out a listener call in progress.
#[test]
fn wp_cg2_stop_waits_for_a_listener_call_in_progress() {
    struct Slow(AtomicBool, AtomicBool);
    impl BrowseListener for Slow {
        fn on_hubs_changed(&self, _: BrowseSnapshot) {
            self.0.store(true, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(100));
            self.1.store(true, Ordering::SeqCst);
        }
    }
    let net = FakeAdvertiser::new();
    let slow = Arc::new(Slow(AtomicBool::new(false), AtomicBool::new(false)));
    let mut b =
        HubBrowser::start(Arc::new(net.clone()), Some(Arc::clone(&slow) as Arc<dyn BrowseListener>), fast()).unwrap();
    until("call started", || slow.0.load(Ordering::SeqCst));
    b.stop();
    assert!(slow.1.load(Ordering::SeqCst), "stop returned with the call still running");
}

#[test]
fn wp_cg2_verify_asks_about_the_hubs_instance() {
    let net = FakeAdvertiser::new();
    let heard = Arc::new(Heard::default());
    let b = browse(&net, &heard, fast());
    net.announce(&advert("Stu.dio", HID), &[ip("10.0.0.1")]);
    until("found", || b.resolve(HID).is_some());
    b.verify("unknown");
    b.verify(&HID.to_uppercase());
    until("asked", || !net.verified().is_empty());
    assert_eq!(net.verified(), ["Stu\\.dio._dspi._tcp.local."]);
}

#[test]
fn wp_cg2_without_a_listener_the_snapshot_still_works() {
    let net = FakeAdvertiser::new();
    let b = HubBrowser::start(Arc::new(net.clone()), None, fast()).unwrap();
    net.announce(&advert("Hub", HID), &[ip("10.0.0.1")]);
    until("found", || names(&b) == ["Hub"]);
}

#[test]
fn wp_cg2_the_json_form_is_camel_case() {
    let hub = parse_service(&record(SPEC)).unwrap();
    let v = serde_json::to_value(BrowseSnapshot { browsing: true, error: None, hubs: vec![hub] }).unwrap();
    assert_eq!(v["browsing"], true);
    assert!(v["error"].is_null());
    assert_eq!(v["hubs"][0]["deviceCount"], 1);
    assert_eq!(v["hubs"][0]["url"], "ws://192.168.1.20:11915/dspi/v1");
}

// -- WP DISC2 -- the embedded daemon is not in a Windows build
#[cfg(all(feature = "mdns", not(windows)))]
mod mdns_mapping {
    use mdns_sd::{ServiceEvent, ServiceInfo};

    use super::super::mdns::{map_event, service_info};
    use super::*;

    #[test]
    fn wp_cg2_the_daemons_events_map_without_a_network() {
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            "Stu.dio",
            "hub.local.",
            "192.168.1.20",
            4000,
            [("v", "1"), ("hid", HID), ("kind", "bridge"), ("auth", "none"), ("n", "0")].as_slice(),
        )
        .unwrap();
        let event = ServiceEvent::ServiceResolved(Box::new(info.as_resolved_service()));
        let Some(SourceEvent::Resolved(record)) = map_event(event) else { panic!("not resolved") };
        assert_eq!(record.instance, "Stu\\.dio._dspi._tcp.local.");
        let hub = parse_service(&record).unwrap();
        assert_eq!(hub.name, "Stu.dio");
        assert_eq!(hub.id, HID);
        assert_eq!(hub.kind, "bridge");
        assert_eq!(hub.auth, "none");
        assert_eq!(hub.url, "ws://192.168.1.20:4000/dspi/v1");

        let gone = ServiceEvent::ServiceRemoved(SERVICE_TYPE.into(), "x._dspi._tcp.local.".into());
        assert_eq!(map_event(gone), Some(SourceEvent::Removed("x._dspi._tcp.local.".into())));
        assert_eq!(map_event(ServiceEvent::SearchStarted(SERVICE_TYPE.into())), None);
        assert_eq!(map_event(ServiceEvent::ServiceFound(SERVICE_TYPE.into(), "x".into())), None);
    }

    #[test]
    fn wp_cg2_an_advertisement_makes_a_valid_service() {
        let info = service_info(&advert("Studio Mac", HID)).unwrap();
        assert_eq!(info.get_fullname(), "Studio Mac._dspi._tcp.local.");
        assert_eq!(info.get_hostname(), "dspi-3f2a7c1e0000.local.");
        assert_eq!(info.get_property_val_str("hid"), Some(HID));
        assert_eq!(info.get_property_val_str("d"), Some("E46058388B1A2E2C"));
        assert!(matches!(service_info(&advert("x", "---")), Err(DiscoveryError::Advert(_))));
    }

    /// The real daemon, responder and browser, end to end. Ignored because it
    /// needs multicast on some interface; the build container has one of its
    /// own: `cargo test -p dspi-link --features mdns -- --ignored wp_cg2_live`.
    #[test]
    #[ignore = "needs multicast on an interface"]
    fn wp_cg2_live_the_real_daemon_finds_the_real_responder() {
        let id = format!("{HID}-{}", std::process::id());
        let name = format!("CG2.test {}", std::process::id());
        let mut a = advert(&name, &id);
        let mut advertiser = MdnsAdvertiser::start(&a).unwrap();
        let b = HubBrowser::start(Arc::new(MdnsSource), None, BrowseOptions::default()).unwrap();
        let long = Duration::from_secs(30);
        let wait = |what: &str, f: &dyn Fn(&HubBrowser) -> bool| {
            let end = Instant::now() + long;
            while !f(&b) {
                assert!(Instant::now() < end, "timed out: {what}; {:?}", b.snapshot());
                thread::sleep(Duration::from_millis(50));
            }
        };
        wait("found", &|b| b.resolve(&id).is_some());
        let hub = b.resolve(&id).unwrap();
        assert_eq!(hub.name, name);
        assert_eq!(hub.serials, ["E46058388B1A2E2C"]);

        a.device_count = 5;
        advertiser.update(&a).unwrap();
        wait("updated", &|b| b.resolve(&id).is_some_and(|h| h.device_count == 5));

        drop(advertiser);
        wait("goodbye", &|b| b.resolve(&id).is_none());
    }
}
