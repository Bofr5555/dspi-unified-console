//! WP HB5: a hub on the network, by its real responder and only on the loopback interface
//! (`Advertise::LoopbackOnly`, `AdvertiseScope::LoopbackOnly`). The browser here is kept to
//! loopback too, which is why it is `mdns-sd` itself and not `MdnsSource`. Each test has
//! names of its own, since a binary's tests share the interface.
//!
//! -- WP DISC2 -- Not on Windows: the responder there is Windows' own, which registers on
//! every interface or none.
#![cfg(not(windows))]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dspi_hub::advertise::Advertiser;
use dspi_hub::server::{Advertise, NetworkConfig};
use dspi_hub::{Hub, HubConfig};
use dspi_link::discovery::{
    AdvertiseScope, AdvertisedNames, BrowsedHub, HubAdvert, SERVICE_TYPE, ServiceRecord,
    mdns_host_name, parse_service,
};
use dspi_transport::FakeDevice;
use mdns_sd::{IfKind, Receiver, ServiceDaemon, ServiceEvent};
use socket2::{Domain, Protocol, Socket, Type};

const WAIT: Duration = Duration::from_secs(15);

/// Tests of a conflict run alone. Between a responder's rename and our re-registering it
/// may answer for the old name, which only shows if something is asking in those few
/// milliseconds, and here every other test's browser is. The source's header says more.
static WIRE: RwLock<()> = RwLock::new(());

fn alone() -> RwLockWriteGuard<'static, ()> {
    WIRE.write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn shared() -> RwLockReadGuard<'static, ()> {
    WIRE.read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn store_dir() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-hb5-adv").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Ids differ in their first digits, since a hub's host name is made from those, and two
/// hubs on one host name take each other's addresses with them when one says goodbye.
///
/// A browser on loopback. What it has heard, by full instance name.
struct Browser {
    daemon: ServiceDaemon,
    events: Receiver<ServiceEvent>,
    heard: BTreeMap<String, ServiceRecord>,
}

impl Browser {
    fn new() -> Self {
        let daemon = ServiceDaemon::new().unwrap();
        daemon.disable_interface(IfKind::All).unwrap();
        daemon.enable_interface(IfKind::LoopbackV4).unwrap();
        let events = daemon.browse(SERVICE_TYPE).unwrap();
        Self {
            daemon,
            events,
            heard: BTreeMap::new(),
        }
    }

    fn pump(&mut self, wait: Duration) {
        let Ok(event) = self.events.recv_timeout(wait) else {
            return;
        };
        match event {
            ServiceEvent::ServiceResolved(s) => {
                let record = ServiceRecord {
                    instance: s.get_fullname().to_owned(),
                    port: s.get_port(),
                    addresses: s
                        .get_addresses()
                        .iter()
                        .map(mdns_sd::ScopedIp::to_ip_addr)
                        .collect(),
                    txt: s
                        .get_properties()
                        .iter()
                        .map(|p| {
                            let value = String::from_utf8_lossy(p.val().unwrap_or_default());
                            (p.key().to_owned(), value.into_owned())
                        })
                        .collect(),
                    ..ServiceRecord::default() // -- WP DISC2 --
                };
                self.heard.insert(record.instance.clone(), record);
            }
            ServiceEvent::ServiceRemoved(_, instance) => {
                self.heard.remove(&instance);
            }
            _ => {}
        }
    }

    /// The hub with this id, as a client reads it, if it has been heard and `ok`.
    fn hub(&self, hub_id: &str) -> Option<BrowsedHub> {
        self.heard
            .values()
            .filter_map(|r| parse_service(r).ok())
            .find(|h| h.id == hub_id)
    }

    fn until(&mut self, what: &str, mut ok: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + WAIT;
        while !ok(self) {
            assert!(
                Instant::now() < deadline,
                "never heard: {what}. Heard: {:?}",
                self.heard
            );
            self.pump(Duration::from_millis(100));
        }
    }

    fn until_hub(&mut self, hub_id: &str, mut ok: impl FnMut(&BrowsedHub) -> bool) -> BrowsedHub {
        self.until(hub_id, |b| b.hub(hub_id).as_ref().is_some_and(&mut ok));
        self.hub(hub_id).unwrap()
    }

    fn until_gone(&mut self, hub_id: &str) {
        self.until("it leaving", |b| b.hub(hub_id).is_none());
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        // -- WP HUBFLAKE -- Until its thread has ended, so a browser that is going asks
        // nothing during the next test, which may be one that runs `alone()`.
        if let Ok(done) = self.daemon.shutdown() {
            let _ = done.recv_timeout(Duration::from_secs(5));
        }
    }
}

fn advert(name: &str, hub_id: &str) -> HubAdvert {
    HubAdvert {
        name: name.to_owned(),
        port: 11915,
        hub_id: hub_id.to_owned(),
        kind: "console".to_owned(),
        auth: "pin".to_owned(),
        device_count: 1,
        serials: vec!["E46058388B1A2E2C".to_owned()],
        web: false,
        path: None,
        tls: false, // -- WP SECDISC --
    }
}

/// An `Advertiser` being ticked, as the hub's supervisor does. Dropping it stops it.
struct Ticked {
    advertiser: Arc<Mutex<Option<Advertiser>>>,
    thread: Option<JoinHandle<()>>,
}

impl Ticked {
    fn update(&self, advert: &HubAdvert) {
        let mut slot = self.advertiser.lock().unwrap();
        slot.as_mut().unwrap().update(advert).unwrap();
    }

    fn names(&self) -> AdvertisedNames {
        let mut slot = self.advertiser.lock().unwrap();
        slot.as_mut().unwrap().names()
    }
}

impl Drop for Ticked {
    fn drop(&mut self) {
        // The thread ends at an empty slot.
        let goodbye = self.advertiser.lock().unwrap().take();
        drop(goodbye);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn start_on_loopback(advert: &HubAdvert) -> Ticked {
    let advertiser = Advertiser::start(advert, AdvertiseScope::LoopbackOnly).unwrap();
    let advertiser = Arc::new(Mutex::new(Some(advertiser)));
    let thread = {
        let advertiser = Arc::clone(&advertiser);
        std::thread::spawn(move || {
            loop {
                match advertiser.lock().unwrap().as_mut() {
                    Some(advertiser) => advertiser.names(),
                    None => return,
                };
                std::thread::sleep(Duration::from_millis(10));
            }
        })
    };
    Ticked {
        advertiser,
        thread: Some(thread),
    }
}

fn hub(name: &str, hub_id: &str, web: bool) -> Hub {
    Hub::start(HubConfig {
        hub_id: hub_id.to_owned(),
        network: Some(NetworkConfig {
            port: 0,
            web,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            advertise: Advertise::LoopbackOnly,
            ..NetworkConfig::new(store_dir())
        }),
        ..HubConfig::in_process(name)
    })
    .unwrap()
}

fn upgrade_status(port: u16, origin: &str) -> u16 {
    let mut tcp = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(
        tcp,
        "GET /dspi/v1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: Upgrade\r\n\
         Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Protocol: dspi-link-1\r\n\
         Origin: {origin}\r\n\r\n"
    )
    .unwrap();
    let mut first = [0u8; 12];
    tcp.read_exact(&mut first).unwrap();
    String::from_utf8_lossy(&first[9..]).trim().parse().unwrap()
}

fn until_names(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !ok() {
        assert!(Instant::now() < deadline, "never: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

// ─────────────────── A hub ───────────────────

#[test]
fn wp_hb5_a_sharing_hub_is_found_with_what_the_spec_asks() {
    let _wire = shared();
    const ID: &str = "b5000001-0000-4000-8000-000000000000";
    let mut browser = Browser::new();
    let h = hub("HB5 found", ID, false);
    h.attach(Box::new(FakeDevice::demo()), 0).unwrap();
    let serial = h.devices()[0].identity.serial.clone();
    let port = h.network().unwrap().port();

    let found = browser.until_hub(ID, |f| f.device_count == 1);
    assert_eq!(found.name, "HB5 found");
    assert_eq!(found.port, port, "the port actually bound");
    assert_eq!(found.kind, "console");
    assert_eq!(found.auth, "pin", "PIN is the default");
    assert_eq!(found.serials, [serial]);
    assert_eq!(found.path, "/dspi/v1");
    let record = browser
        .heard
        .values()
        .find(|r| r.instance.starts_with("HB5 found"))
        .unwrap();
    assert!(!record.txt.iter().any(|(k, _)| k == "web"), "web is off");
    assert!(
        record.addresses.iter().all(IpAddr::is_loopback),
        "loopback only: {:?}",
        record.addresses
    );
    h.stop();
}

#[test]
fn wp_hb5_the_record_says_web_only_with_web_on() {
    let _wire = shared();
    const ID: &str = "b5000002-0000-4000-8000-000000000000";
    let mut browser = Browser::new();
    let h = hub("HB5 web", ID, true);
    browser.until_hub(ID, |_| true);
    let record = browser
        .heard
        .values()
        .find(|r| r.instance.starts_with("HB5 web"))
        .unwrap();
    assert!(record.txt.iter().any(|(k, v)| k == "web" && v == "1"));
    h.stop();
}

#[test]
fn wp_hb5_devices_coming_and_going_change_the_record() {
    let _wire = shared();
    const ID: &str = "b5000003-0000-4000-8000-000000000000";
    let mut browser = Browser::new();
    let h = hub("HB5 devices", ID, false);
    assert_eq!(browser.until_hub(ID, |_| true).device_count, 0);

    let device = FakeDevice::demo();
    let remote = device.remote();
    h.attach(Box::new(device), 0).unwrap();
    let found = browser.until_hub(ID, |f| f.device_count == 1);
    assert_eq!(found.serials.len(), 1);

    // Offline keeps its handle for the grace period, but is not shared.
    remote.disconnect();
    let found = browser.until_hub(ID, |f| f.device_count == 0);
    assert!(found.serials.is_empty(), "{found:?}");

    h.attach(Box::new(FakeDevice::demo()), 0).unwrap();
    browser.until_hub(ID, |f| f.device_count == 1);
    h.stop();
}

#[test]
fn wp_hb5_stopping_sends_a_goodbye_and_stops_the_responder() {
    let _wire = shared();
    const ID: &str = "b5000004-0000-4000-8000-000000000000";
    let mut browser = Browser::new();
    let h = hub("HB5 goodbye", ID, false);
    browser.until_hub(ID, |_| true);
    assert!(h.advertised().is_some());
    h.stop();
    assert_eq!(h.advertised(), None);
    // Well inside a record's TTL: this is the goodbye, not an expiry.
    let stopped = Instant::now();
    browser.until_gone(ID);
    assert!(stopped.elapsed() < Duration::from_secs(5));
}

#[test]
fn wp_hb5_a_hub_bound_to_loopback_says_nothing_to_the_network_unless_told() {
    let _wire = shared();
    const ID: &str = "b5000005-0000-4000-8000-000000000000";
    let h = Hub::start(HubConfig {
        hub_id: ID.to_owned(),
        network: Some(NetworkConfig {
            port: 0,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(store_dir())
        }),
        ..HubConfig::in_process("HB5 quiet")
    })
    .unwrap();
    assert_eq!(h.advertised(), None);
    h.stop();

    // The two ways to have no advertisement in the configuration a hub on the LAN has.
    let lan = NetworkConfig::new(store_dir());
    assert_eq!(lan.advertise, Advertise::Lan, "on by default, when sharing");
    assert!(
        lan.mdns_name.is_none(),
        "set by Hub::start, which knows the id"
    );
}

#[test]
fn wp_hb5_a_hub_with_advertising_off_says_nothing_and_has_no_local_name() {
    let _wire = shared();
    const ID: &str = "b5000006-0000-4000-8000-000000000000";
    let h = Hub::start(HubConfig {
        hub_id: ID.to_owned(),
        network: Some(NetworkConfig {
            port: 0,
            web: true,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            advertise: Advertise::Off,
            ..NetworkConfig::new(store_dir())
        }),
        ..HubConfig::in_process("HB5 off")
    })
    .unwrap();
    assert_eq!(h.advertised(), None);
    let port = h.network().unwrap().port();
    let host = mdns_host_name(ID).unwrap();
    assert_eq!(upgrade_status(port, &format!("http://{host}:{port}")), 403);
    h.stop();
}

// ─────────────────── The .local page ───────────────────

#[test]
fn wp_hb5_a_page_opened_at_the_local_name_is_not_refused() {
    let _wire = shared();
    const ID: &str = "b5000007-0000-4000-8000-000000000000";
    let h = hub("HB5 local", ID, true);
    let names = h.advertised().unwrap();
    assert_eq!(Some(names.host.clone()), mdns_host_name(ID));
    assert!(names.host.ends_with(".local"), "{names:?}");
    let port = h.network().unwrap().port();
    let host = names.host;
    assert_eq!(upgrade_status(port, &format!("http://{host}:{port}")), 101);
    assert_eq!(upgrade_status(port, &format!("http://{host}:1")), 403);
    assert_eq!(upgrade_status(port, &format!("https://{host}:{port}")), 403);
    assert_eq!(
        upgrade_status(port, &format!("http://other-{host}:{port}")),
        403
    );
    h.stop();
}

#[test]
fn wp_hb5_with_web_off_even_the_local_name_is_refused() {
    let _wire = shared();
    const ID: &str = "b5000008-0000-4000-8000-000000000000";
    let h = hub("HB5 local off", ID, false);
    let port = h.network().unwrap().port();
    let host = h.advertised().unwrap().host;
    assert_eq!(upgrade_status(port, &format!("http://{host}:{port}")), 403);
    h.stop();
}

// ─────────────────── Conflicts and renames ───────────────────

#[test]
fn wp_hb5_a_taken_instance_name_gets_another_and_the_first_keeps_its_own() {
    let _wire = alone();
    const FIRST: &str = "b5000011-0000-4000-8000-000000000000";
    const SECOND: &str = "b5000012-0000-4000-8000-000000000000";
    let mut browser = Browser::new();
    let first = start_on_loopback(&advert("HB5 clash", FIRST));
    browser.until_hub(FIRST, |_| true);

    let second = start_on_loopback(&advert("HB5 clash", SECOND));
    browser.until_hub(SECOND, |f| f.name == "HB5 clash (2)");
    assert_eq!(browser.hub(FIRST).unwrap().name, "HB5 clash");
    until_names("the responder reporting it", || {
        second.names().instance == "HB5 clash (2)"
    });
    assert_eq!(first.names().instance, "HB5 clash");

    // A change to the loser goes out under the name it holds, not back under the taken one.
    let changed = HubAdvert {
        device_count: 2,
        ..advert("HB5 clash", SECOND)
    };
    second.update(&changed);
    let found = browser.until_hub(SECOND, |f| f.device_count == 2);
    assert_eq!(found.name, "HB5 clash (2)");
    assert_eq!(browser.hub(FIRST).unwrap().device_count, 1);
    assert_eq!(second.names().instance, "HB5 clash (2)");

    // Its goodbye withdraws what it held, and leaves the other alone.
    drop(second);
    browser.until_gone(SECOND);
    assert_eq!(browser.hub(FIRST).unwrap().name, "HB5 clash");
}

#[test]
fn wp_hb5_a_conflict_over_a_name_with_a_dot_keeps_the_name_whole() {
    let _wire = alone();
    const FIRST: &str = "b5000015-0000-4000-8000-000000000000";
    const SECOND: &str = "b5000016-0000-4000-8000-000000000000";
    let mut browser = Browser::new();
    let _first = start_on_loopback(&advert("HB5 Studio 1.2", FIRST));
    browser.until_hub(FIRST, |_| true);
    let _second = start_on_loopback(&advert("HB5 Studio 1.2", SECOND));
    browser.until_hub(SECOND, |f| f.name == "HB5 Studio 1-2 (2)");
    assert_eq!(browser.hub(FIRST).unwrap().name, "HB5 Studio 1-2");
}

#[test]
fn wp_hb5_a_second_conflict_counts_on() {
    let _wire = alone();
    const IDS: [&str; 3] = [
        "b5000021-0000-4000-8000-000000000000",
        "b5000022-0000-4000-8000-000000000000",
        "b5000023-0000-4000-8000-000000000000",
    ];
    let mut browser = Browser::new();
    let _first = start_on_loopback(&advert("HB5 thrice", IDS[0]));
    browser.until_hub(IDS[0], |_| true);
    let second = start_on_loopback(&advert("HB5 thrice", IDS[1]));
    browser.until_hub(IDS[1], |f| f.name != "HB5 thrice");
    // -- WP HUBFLAKE -- The responder announces its rename before our re-registering
    // drops the old name (the source's header), and until then it answers for the first's
    // name too. The third's probe of that name, sent in that moment, had both answer it,
    // and the browser kept the second's answer as the first's: the slower the tick, the
    // wider that moment. The third starts once the second holds its new name.
    until_names("the second re-registered", || {
        second.names().instance == "HB5 thrice (2)"
    });
    let third = start_on_loopback(&advert("HB5 thrice", IDS[2]));
    // Each with a name of its own, the first undisturbed. Which of the others ends up with
    // which depends on who was probing when.
    browser.until("three hubs under three names", |b| {
        let names: BTreeSet<_> = IDS
            .iter()
            .filter_map(|id| b.hub(id))
            .map(|f| f.name)
            .collect();
        names.len() == 3
            && names.contains("HB5 thrice")
            && names.contains("HB5 thrice (2)")
            && names.contains("HB5 thrice (3)")
    });
    assert_eq!(browser.hub(IDS[0]).unwrap().name, "HB5 thrice");
    assert_ne!(third.names().instance, "HB5 thrice");
}

#[test]
fn wp_hb5_a_rename_says_goodbye_to_the_old_name_and_is_the_same_hub() {
    let _wire = shared();
    const ID: &str = "b5000031-0000-4000-8000-000000000000";
    let mut browser = Browser::new();
    let hub = start_on_loopback(&advert("HB5 before", ID));
    browser.until_hub(ID, |f| f.name == "HB5 before");

    hub.update(&advert("HB5 after", ID));
    browser.until("the old name leaving and the new one arriving", |b| {
        !b.heard.keys().any(|k| k.starts_with("HB5 before"))
            && b.hub(ID).is_some_and(|f| f.name == "HB5 after")
    });
    assert_eq!(hub.names().instance, "HB5 after");
}

#[test]
fn wp_hb5_a_rename_onto_a_taken_name_gets_another() {
    let _wire = alone();
    const HOLDER: &str = "b5000041-0000-4000-8000-000000000000";
    const MOVER: &str = "b5000042-0000-4000-8000-000000000000";
    let mut browser = Browser::new();
    let _holder = start_on_loopback(&advert("HB5 taken", HOLDER));
    browser.until_hub(HOLDER, |_| true);
    let mover = start_on_loopback(&advert("HB5 mover", MOVER));
    browser.until_hub(MOVER, |_| true);

    mover.update(&advert("HB5 taken", MOVER));
    browser.until_hub(MOVER, |f| f.name == "HB5 taken (2)");
    assert_eq!(browser.hub(HOLDER).unwrap().name, "HB5 taken");
    until_names("the responder reporting it", || {
        mover.names().instance == "HB5 taken (2)"
    });
}

/// `dns_name.local A 127.0.0.2`, as another machine claiming the name would answer.
fn answer_claiming(host: &str) -> Vec<u8> {
    let mut packet = vec![0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0];
    for label in host.split('.') {
        packet.push(u8::try_from(label.len()).unwrap());
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&[0, 1, 0, 1, 0, 0, 0, 120, 0, 4, 127, 0, 0, 2]);
    packet
}

/// A socket whose multicast goes out on the loopback interface and nowhere else.
fn loopback_sender() -> Socket {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).unwrap();
    socket.set_multicast_if_v4(&Ipv4Addr::LOCALHOST).unwrap();
    socket.set_multicast_ttl_v4(1).unwrap();
    socket
}

fn mdns_group() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(224, 0, 0, 251)), 5353)
}

/// Sends `packet` to the mDNS group on the loopback interface alone, until `stop`.
fn claim_until(packet: &[u8], mut stop: impl FnMut() -> bool) {
    let socket = loopback_sender();
    let group = mdns_group();
    let deadline = Instant::now() + WAIT;
    while !stop() {
        assert!(Instant::now() < deadline, "the responder never gave way");
        socket.send_to(packet, &group.into()).unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn wp_hb5_a_taken_host_name_moves_the_origin_check_with_it() {
    let _wire = alone();
    const ID: &str = "b5000051-0000-4000-8000-000000000000";
    let host = mdns_host_name(ID).unwrap();
    let claim = answer_claiming(&host);
    // Already claimed while the hub probes, as a machine holding the name would answer.
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let claimer = std::thread::spawn(move || {
        claim_until(&claim, || stop_rx.try_recv().is_ok());
    });
    std::thread::sleep(Duration::from_millis(100));

    let h = hub("HB5 host", ID, true);
    let renamed = format!("{}-2.local", host.strip_suffix(".local").unwrap());
    until_names("the responder taking another host name", || {
        h.advertised().unwrap().host == renamed
    });
    stop_tx.send(()).unwrap();
    claimer.join().unwrap();

    let port = h.network().unwrap().port();
    assert_eq!(
        upgrade_status(port, &format!("http://{renamed}:{port}")),
        101
    );
    assert_eq!(
        upgrade_status(port, &format!("http://{host}:{port}")),
        403,
        "that name is somebody else's now"
    );
    h.stop();
}

// ─────────────────── What DNS-SD can carry ───────────────────

#[test]
fn wp_hb5_awkward_names_read_back_as_themselves() {
    let _wire = shared();
    for (i, (name, expected)) in [
        ("HB5 Studio 1.2", "HB5 Studio 1-2"),
        ("HB5 back\\slash", "HB5 back/slash"),
        ("HB5 Åke's rum", "HB5 Åke's rum"),
        ("HB5 two\nlines", "HB5 two lines"),
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("b50006{i:02}-0000-4000-8000-000000000000");
        let mut browser = Browser::new();
        let _hub = start_on_loopback(&advert(name, &id));
        assert_eq!(browser.until_hub(&id, |_| true).name, expected);
    }
}

#[test]
fn wp_hb5_a_name_past_a_dns_label_still_advertises_and_so_does_its_conflict() {
    let _wire = alone();
    const FIRST: &str = "b5000071-0000-4000-8000-000000000000";
    const SECOND: &str = "b5000072-0000-4000-8000-000000000000";
    let long = format!("HB5 {}", "x".repeat(300));
    let mut browser = Browser::new();
    let _first = start_on_loopback(&advert(&long, FIRST));
    let name = browser.until_hub(FIRST, |_| true).name;
    assert!(name.starts_with("HB5 xxx") && name.len() <= 63, "{name}");
    let _second = start_on_loopback(&advert(&long, SECOND));
    let found = browser.until_hub(SECOND, |f| f.name.ends_with(" (2)"));
    assert!(found.name.len() <= 63, "{}", found.name);
}

#[test]
fn wp_hb5_a_device_list_past_one_txt_string_does_not_take_the_hub_off_the_network() {
    let _wire = shared();
    const ID: &str = "b5000081-0000-4000-8000-000000000000";
    let serials: Vec<String> = (0..20).map(|i| format!("{i:016X}")).collect();
    let many = HubAdvert {
        device_count: 20,
        serials,
        ..advert("HB5 many", ID)
    };
    let mut browser = Browser::new();
    let hub = start_on_loopback(&many);
    let found = browser.until_hub(ID, |_| true);
    assert_eq!(found.device_count, 20);
    assert!(found.serials.is_empty(), "d is optional");

    // And the update path, which is how a hub gets there.
    let few = HubAdvert {
        device_count: 1,
        ..advert("HB5 many", ID)
    };
    hub.update(&few);
    let found = browser.until_hub(ID, |f| f.device_count == 1);
    assert_eq!(found.serials, ["E46058388B1A2E2C"]);
    hub.update(&many);
    browser.until_hub(ID, |f| f.device_count == 20);
}

#[test]
fn wp_hb5_quick_changes_end_with_the_last_on_the_network() {
    let _wire = shared();
    const ID: &str = "b5000091-0000-4000-8000-000000000000";
    let hub = start_on_loopback(&advert("HB5 quick", ID));
    let mut browser = Browser::new();
    browser.until_hub(ID, |_| true);
    // Within the responder's one second, which sends none of what comes too soon.
    for n in [5, 6, 1, 7] {
        hub.update(&HubAdvert {
            device_count: n,
            ..advert("HB5 quick", ID)
        });
    }
    browser.until_hub(ID, |f| f.device_count == 7);
    // And back to a record it has sent before, which is not probed again.
    hub.update(&advert("HB5 quick", ID));
    browser.until_hub(ID, |f| f.device_count == 1);
}

#[test]
fn wp_hb5_an_update_with_nothing_new_sends_nothing() {
    let _wire = shared();
    const ID: &str = "b5000092-0000-4000-8000-000000000000";
    let hub = start_on_loopback(&advert("HB5 still", ID));
    let mut browser = Browser::new();
    browser.until_hub(ID, |_| true);
    let before = browser.heard.clone();
    for _ in 0..50 {
        hub.update(&advert("HB5 still", ID));
    }
    let slot = hub.advertiser.lock().unwrap();
    assert_eq!(slot.as_ref().unwrap().current(), &advert("HB5 still", ID));
    drop(slot);
    assert_eq!(browser.heard, before);
}

/// The responder's own late answers must not look like a conflict to it.
#[test]
fn wp_hb5_updates_while_being_asked_do_not_rename_the_hub() {
    let _wire = shared();
    const ID: &str = "b5000093-0000-4000-8000-000000000000";
    let hub = start_on_loopback(&advert("HB5 busy", ID));
    let mut browser = Browser::new();
    browser.until_hub(ID, |_| true);

    let socket = loopback_sender();
    // `_dspi._tcp.local PTR?`, far more often than anything real asks.
    let mut query = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in ["_dspi", "_tcp", "local"] {
        query.push(u8::try_from(label.len()).unwrap());
        query.extend_from_slice(label.as_bytes());
    }
    query.extend_from_slice(&[0, 0, 12, 0, 1]);
    let group = mdns_group();
    for n in 2..6 {
        hub.update(&HubAdvert {
            device_count: n,
            ..advert("HB5 busy", ID)
        });
        let deadline = Instant::now() + Duration::from_millis(2_200);
        while Instant::now() < deadline {
            socket.send_to(&query, &group.into()).unwrap();
            browser.pump(Duration::from_millis(30));
        }
        browser.until_hub(ID, |f| f.device_count == n);
    }
    assert_eq!(hub.names().instance, "HB5 busy");
    assert_eq!(browser.hub(ID).unwrap().name, "HB5 busy");
}

// ─────────────────── Conflicts between hubs ───────────────────

#[test]
fn wp_hb5_two_hubs_with_one_name_are_both_found_by_their_ids() {
    let _wire = alone();
    const FIRST: &str = "b50000a1-0000-4000-8000-000000000000";
    const SECOND: &str = "b50000a2-0000-4000-8000-000000000000";
    let mut browser = Browser::new();
    let first = hub("HB5 Same Name", FIRST, false);
    browser.until_hub(FIRST, |_| true);
    let second = hub("HB5 Same Name", SECOND, false);
    browser.until_hub(SECOND, |f| f.name == "HB5 Same Name (2)");
    assert_eq!(browser.hub(FIRST).unwrap().name, "HB5 Same Name");
    assert_eq!(second.advertised().unwrap().instance, "HB5 Same Name (2)");
    // What the hub calls itself to its clients is what it was given.
    assert_eq!(second.name(), "HB5 Same Name");
    // A device arriving reaches the record under the name it holds.
    second.attach(Box::new(FakeDevice::demo()), 0).unwrap();
    let found = browser.until_hub(SECOND, |f| f.device_count == 1);
    assert_eq!(found.name, "HB5 Same Name (2)");
    assert_eq!(browser.hub(FIRST).unwrap().device_count, 0);
    second.stop();
    browser.until_gone(SECOND);
    assert_eq!(browser.hub(FIRST).unwrap().name, "HB5 Same Name");
    first.stop();
}
