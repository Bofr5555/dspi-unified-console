//! WP DISC2: a hub that says goodbye stays listed for a grace, unless the network
//! was asked about it. Against the fake advertiser; no socket.

use std::net::Ipv4Addr;

use super::*;

const HID: &str = "3f2a7c1e-0000-4000-8000-0000000000d2";
const WAIT: Duration = Duration::from_secs(5);

fn advert(name: &str, id: &str, count: u32) -> HubAdvert {
    HubAdvert {
        name: name.into(),
        port: 11915,
        hub_id: id.into(),
        kind: "console".into(),
        auth: "pin".into(),
        device_count: count,
        ..HubAdvert::default()
    }
}

fn addr() -> [IpAddr; 1] {
    [IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))]
}

fn record(name: &str, id: &str) -> ServiceRecord {
    let a = advert(name, id, 1);
    ServiceRecord {
        instance: FakeAdvertiser::instance(name),
        port: a.port,
        addresses: addr().to_vec(),
        txt: a.txt(),
        ..ServiceRecord::default()
    }
}

/// Every snapshot the listener was given, as hub names.
#[derive(Default)]
struct Seen(Mutex<Vec<Vec<String>>>);

impl BrowseListener for Seen {
    fn on_hubs_changed(&self, snapshot: BrowseSnapshot) {
        self.0.lock().unwrap().push(snapshot.hubs.into_iter().map(|h| h.name).collect());
    }
}

fn with_grace(grace: Duration) -> BrowseOptions {
    BrowseOptions { retry_first: Duration::from_millis(5), removal_grace: grace, ..BrowseOptions::default() }
}

fn browse(net: &FakeAdvertiser, seen: &Arc<Seen>, options: BrowseOptions) -> HubBrowser {
    HubBrowser::start(Arc::new(net.clone()), Some(Arc::clone(seen) as Arc<dyn BrowseListener>), options).unwrap()
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

/// Windows' TXT update: a goodbye, then the same name with new data.
#[test]
fn wp_disc2_a_goodbye_and_a_quick_return_never_leave_the_list() {
    let net = FakeAdvertiser::new();
    let seen = Arc::new(Seen::default());
    let b = browse(&net, &seen, with_grace(Duration::from_secs(2)));
    net.announce(&advert("Hub", HID, 1), &addr());
    until("found", || names(&b) == ["Hub"]);

    net.withdraw("Hub");
    thread::sleep(Duration::from_millis(300));
    assert_eq!(names(&b), ["Hub"], "listed through the gap");
    net.announce(&advert("Hub", HID, 2), &addr());
    until("new data", || b.resolve(HID).is_some_and(|h| h.device_count == 2));
    // Past the first goodbye's grace: the return cancelled it.
    thread::sleep(Duration::from_millis(2200));
    assert_eq!(names(&b), ["Hub"]);
    let seen = seen.0.lock().unwrap();
    let first = seen.iter().position(|s| !s.is_empty()).unwrap();
    assert!(seen[first..].iter().all(|s| s == &["Hub"]), "the listener saw it vanish: {seen:?}");
}

#[test]
fn wp_disc2_a_hub_that_does_not_come_back_leaves_after_the_grace() {
    let net = FakeAdvertiser::new();
    let seen = Arc::new(Seen::default());
    let grace = Duration::from_millis(400);
    let b = browse(&net, &seen, with_grace(grace));
    net.announce(&advert("Hub", HID, 1), &addr());
    until("found", || names(&b) == ["Hub"]);

    let said = Instant::now();
    net.withdraw("Hub");
    until("gone", || b.snapshot().hubs.is_empty());
    assert!(said.elapsed() >= grace, "left after {:?}", said.elapsed());
    until("the listener hears", || seen.0.lock().unwrap().last().is_some_and(Vec::is_empty));
}

/// A second goodbye during the grace does not put the end off, and a busy network
/// does not either.
#[test]
fn wp_disc2_repeated_goodbyes_and_other_traffic_do_not_stretch_the_grace() {
    let net = FakeAdvertiser::new();
    let seen = Arc::new(Seen::default());
    let grace = Duration::from_millis(400);
    let b = browse(&net, &seen, with_grace(grace));
    net.announce(&advert("Hub", HID, 1), &addr());
    until("found", || names(&b) == ["Hub"]);

    let said = Instant::now();
    net.withdraw("Hub");
    let mut n = 0;
    while said.elapsed() < Duration::from_millis(1200) && names(&b).contains(&"Hub".to_owned()) {
        net.withdraw("Hub");
        n += 1;
        net.announce(&advert("Other", "id-other", n), &addr());
        thread::sleep(Duration::from_millis(20));
    }
    assert!(!names(&b).contains(&"Hub".to_owned()), "still listed after {:?}", said.elapsed());
    assert!(said.elapsed() < Duration::from_millis(1000), "left after {:?}", said.elapsed());
}

/// A dial failed and the client asked: the answer is final, not a flap.
#[test]
fn wp_disc2_a_goodbye_after_a_verify_counts_at_once() {
    let net = FakeAdvertiser::new();
    let seen = Arc::new(Seen::default());
    let b = browse(&net, &seen, with_grace(Duration::from_secs(30)));
    net.announce(&advert("Hub", HID, 1), &addr());
    until("found", || names(&b) == ["Hub"]);

    b.verify(HID);
    until("asked", || !net.verified().is_empty());
    net.withdraw("Hub");
    until("gone at once", || b.snapshot().hubs.is_empty());
}

#[test]
fn wp_disc2_a_verify_of_a_hub_already_leaving_removes_it() {
    let net = FakeAdvertiser::new();
    let seen = Arc::new(Seen::default());
    let b = browse(&net, &seen, with_grace(Duration::from_secs(30)));
    net.announce(&advert("Hub", HID, 1), &addr());
    until("found", || names(&b) == ["Hub"]);
    net.withdraw("Hub");
    net.announce(&advert("Marker", "id-m", 1), &addr());
    until("marker", || names(&b) == ["Hub", "Marker"]);

    b.verify(HID);
    until("gone at once", || names(&b) == ["Marker"]);
}

#[test]
fn wp_disc2_a_failed_source_forgets_at_once_grace_or_not() {
    let net = FakeAdvertiser::new();
    let seen = Arc::new(Seen::default());
    let b = browse(
        &net,
        &seen,
        BrowseOptions { retry_first: Duration::from_secs(5), ..with_grace(Duration::from_secs(30)) },
    );
    net.announce(&advert("Hub", HID, 1), &addr());
    until("found", || names(&b) == ["Hub"]);
    net.withdraw("Hub");
    net.fail("gone");
    until("cleared", || b.snapshot().hubs.is_empty() && !b.snapshot().browsing);
}

/// The list itself, on a clock of its own.
#[test]
fn wp_disc2_the_list_keeps_a_leaving_hub_exactly_its_grace() {
    let grace = Duration::from_secs(3);
    let mut list = HubList::new(String::new(), grace);
    let t0 = Instant::now();
    list.apply(SourceEvent::Resolved(record("A", "id-a")), t0);
    list.apply(SourceEvent::Resolved(record("B", "id-b")), t0);
    let a = FakeAdvertiser::instance("A");

    list.apply(SourceEvent::Removed(a.clone()), t0);
    assert_eq!(list.next_expiry(), Some(t0 + grace));
    // A second goodbye later does not move it.
    list.apply(SourceEvent::Removed(a.clone()), t0 + Duration::from_secs(1));
    assert_eq!(list.next_expiry(), Some(t0 + grace));
    assert!(!list.expire(t0 + grace - Duration::from_millis(1)));
    assert_eq!(list.sorted().len(), 2, "still listed");
    assert!(list.expire(t0 + grace));
    assert_eq!(list.sorted().iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), ["B"]);
    assert_eq!(list.next_expiry(), None);

    // Unknown instances leave nothing behind.
    list.apply(SourceEvent::Removed("nobody._dspi._tcp.local.".into()), t0);
    assert_eq!(list.next_expiry(), None);
}

#[test]
fn wp_disc2_a_leaving_hub_that_renames_leaves_no_timer_behind() {
    let mut list = HubList::new(String::new(), Duration::from_secs(3));
    let t0 = Instant::now();
    list.apply(SourceEvent::Resolved(record("Old", HID)), t0);
    list.apply(SourceEvent::Removed(FakeAdvertiser::instance("Old")), t0);
    list.apply(SourceEvent::Resolved(record("New", HID)), t0);
    assert_eq!(list.next_expiry(), None);
    assert_eq!(list.sorted().iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), ["New"]);
}

#[test]
fn wp_disc2_a_zero_grace_removes_at_once() {
    let mut list = HubList::new(String::new(), Duration::ZERO);
    let t0 = Instant::now();
    list.apply(SourceEvent::Resolved(record("A", "id-a")), t0);
    list.apply(SourceEvent::Removed(FakeAdvertiser::instance("A")), t0);
    assert!(list.sorted().is_empty());
    assert_eq!(list.next_expiry(), None);
}

#[test]
fn wp_disc2_a_verify_window_ends() {
    let mut list = HubList::new(String::new(), Duration::from_secs(3));
    let t0 = Instant::now();
    let a = FakeAdvertiser::instance("A");
    list.apply(SourceEvent::Resolved(record("A", "id-a")), t0);
    assert!(!list.verifying(&a, t0), "not leaving, so not removed by asking");
    // It answered; much later a goodbye is a goodbye like any other.
    let later = t0 + VERIFY_WINDOW + Duration::from_secs(1);
    list.expire(later);
    list.apply(SourceEvent::Removed(a), later);
    assert_eq!(list.sorted().len(), 1, "in its grace");
}

/// The embedded responder notices when it has no socket or no thread, says so, and
/// opens again. On the loopback interface alone, as HB5's tests are.
#[cfg(all(feature = "mdns", not(windows)))]
mod responder {
    use std::net::UdpSocket;

    use mdns_sd::{IfKind, ServiceDaemon, ServiceEvent};

    use super::super::mdns::MdnsAdvertiser;
    use super::*;

    /// One at a time: one of them holds the mDNS port.
    static PORT: Mutex<()> = Mutex::new(());
    const LONG: Duration = Duration::from_secs(30);

    fn loopback_browser() -> (ServiceDaemon, mdns_sd::Receiver<ServiceEvent>) {
        let daemon = ServiceDaemon::new().unwrap();
        daemon.disable_interface(IfKind::All).unwrap();
        daemon.enable_interface(IfKind::LoopbackV4).unwrap();
        let events = daemon.browse(SERVICE_TYPE).unwrap();
        (daemon, events)
    }

    /// Call `names()` as the hub's tick does, until the browser resolves `hid`.
    fn until_found(adv: &mut MdnsAdvertiser, events: &mdns_sd::Receiver<ServiceEvent>, hid: &str) {
        let end = Instant::now() + LONG;
        loop {
            assert!(Instant::now() < end, "not found again");
            adv.names();
            if let Ok(ServiceEvent::ServiceResolved(s)) = events.recv_timeout(Duration::from_millis(50)) {
                if s.get_property_val_str("hid") == Some(hid) {
                    return;
                }
            }
        }
    }

    #[test]
    fn wp_disc2_a_responder_whose_thread_ended_is_opened_again() {
        let _one = PORT.lock().unwrap_or_else(PoisonError::into_inner);
        let hid = format!("{HID}-thread-{}", std::process::id());
        let mut adv = MdnsAdvertiser::start_on(&advert("DISC2 ended", &hid, 1), AdvertiseScope::LoopbackOnly).unwrap();
        let (browser, events) = loopback_browser();
        until_found(&mut adv, &events, &hid);

        // As if its thread had died: no socket, no answer.
        adv.daemon.shutdown().unwrap();
        while events.recv_timeout(Duration::from_millis(300)).is_ok() {}
        until_found(&mut adv, &events, &hid);
        drop(adv);
        let _ = browser.shutdown();
    }

    #[test]
    fn wp_disc2_a_responder_that_got_no_socket_says_so_and_tries_again() {
        let _one = PORT.lock().unwrap_or_else(PoisonError::into_inner);
        // Held without SO_REUSEADDR/SO_REUSEPORT: the responder cannot bind beside it.
        let blocker = UdpSocket::bind("[::]:5353").or_else(|_| UdpSocket::bind("0.0.0.0:5353")).unwrap();
        let hid = format!("{HID}-socket-{}", std::process::id());
        let mut adv = MdnsAdvertiser::start_on(&advert("DISC2 nosock", &hid, 1), AdvertiseScope::LoopbackOnly).unwrap();
        let started = Instant::now();
        let end = started + LONG;
        while adv.reopen_at.is_none() {
            assert!(Instant::now() < end, "never noticed it had no socket");
            adv.names();
            thread::sleep(Duration::from_millis(50));
        }
        assert!(started.elapsed() >= Duration::from_secs(4), "noticed after {:?}", started.elapsed());

        drop(blocker);
        let (browser, events) = loopback_browser();
        until_found(&mut adv, &events, &hid);
        assert!(adv.unproven.is_none() && adv.reopen_at.is_none(), "healthy again");
        drop(adv);
        let _ = browser.shutdown();
    }
}

/// A source that gives a host name (Windows') and nothing to dial lists the hub by name.
#[test]
fn wp_disc2_a_hub_with_only_a_link_local_address_is_dialled_by_name() {
    let mut r = record("Linux Hub", HID);
    r.addresses = vec!["fe80::58f7:c2ff:fe62:4874".parse().unwrap()];
    assert_eq!(parse_service(&r), Err(Rejected::Address), "no name: as before");
    r.host_name = "DSPi-A5D5FE0C5A53.local".into();
    let hub = parse_service(&r).unwrap();
    assert_eq!(hub.host, "dspi-a5d5fe0c5a53.local");
    assert_eq!(hub.addresses, ["dspi-a5d5fe0c5a53.local"]);
    assert_eq!(hub.url, "ws://dspi-a5d5fe0c5a53.local:11915/dspi/v1");

    // Any address that can be dialled comes first, and alone.
    r.addresses.push("10.77.0.20".parse().unwrap());
    let hub = parse_service(&r).unwrap();
    assert_eq!((hub.host.as_str(), hub.addresses.len()), ("10.77.0.20", 1));
}

#[test]
fn wp_disc2_only_a_plain_local_name_is_dialled() {
    let mut r = record("Hub", HID);
    r.addresses.clear();
    for bad in ["", ".local", "x.local:80", "x/y.local", "-x.local", "a..local", "x.example.com", "x local.local"] {
        r.host_name = bad.into();
        assert_eq!(parse_service(&r), Err(Rejected::Address), "{bad:?}");
    }
    for good in ["x.local", "x.local.", "my-pc.lan.local", "STUDIO.LOCAL"] {
        r.host_name = good.into();
        assert!(parse_service(&r).is_ok(), "{good:?}");
    }
}
