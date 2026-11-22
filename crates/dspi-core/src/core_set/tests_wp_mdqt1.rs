//! WP MDQT1: the main core, ids that follow the device, and review RA's
//! points on the set (MD0 items 4-7).

#![allow(clippy::arithmetic_side_effects)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;

use dspi_transport::FakeDevice;

use super::*;

const VID: u16 = dspi_proto::USB_VID;

#[derive(Default)]
struct Heard(Mutex<Vec<String>>);

impl CoreSetListener for Heard {
    fn on_event(&self, e: CoreSetEvent) {
        let kind = serde_json::to_value(e.kind).unwrap();
        let what = if e.id.is_empty() { e.serial } else { e.id };
        self.0
            .lock()
            .unwrap()
            .push(format!("{} {what}", kind.as_str().unwrap()));
    }
}

fn set_with_events() -> (Arc<CoreSet>, Arc<Heard>) {
    let set = CoreSet::new();
    let heard = Arc::new(Heard::default());
    set.set_listener(Some(heard.clone()));
    (set, heard)
}

impl Heard {
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

fn bus(serials: &[&str]) -> Vec<DeviceInfo> {
    serials
        .iter()
        .map(|s| DeviceInfo {
            serial: (*s).to_owned(),
            location_id: 0,
        })
        .collect()
}

fn attach(core: &DspiCore, serial: &str) {
    core.claim(&format!("usb:{serial}"), serial).unwrap();
    core.attach_probed(Box::new(FakeDevice::demo()), serial.to_owned(), VID)
        .unwrap();
}

fn ids(set: &CoreSet) -> Vec<String> {
    set.list().iter().map(|d| d.id.to_string()).collect()
}

#[test]
fn wp_mdqt1_the_main_core_is_listed_by_what_it_has_open() {
    let (set, heard) = set_with_events();
    let main = set.main_core();
    assert!(Arc::ptr_eq(&main, &set.main_core()), "one main core");
    assert!(set.list().is_empty(), "nothing open");

    main.open_demo().unwrap();
    assert_eq!(ids(&set), ["demo:1"]);
    assert!(Arc::ptr_eq(&set.core(&DeviceId::demo(1)).unwrap(), &main));
    assert_eq!(heard.take(), ["opened demo:1"]);

    // A second device has a core of its own; the main one's comes first.
    let extra = set.open(DeviceTarget::Demo).unwrap();
    assert_eq!(ids(&set), ["demo:1", extra.as_str()]);

    // Replaced on the same core: the old id is closed, its serial given back.
    attach(&main, "U1");
    assert_eq!(ids(&set), ["usb:U1", extra.as_str()]);
    assert_eq!(set.find_serial("U1"), Some(DeviceId::usb("U1")));
    assert!(set.core(&DeviceId::demo(1)).is_none());
    attach(&main, "U2");
    assert_eq!(ids(&set)[0], "usb:U2");
    assert_eq!(set.find_serial("U1"), None);
    assert_eq!(
        heard.take(),
        [
            "opened demo:2",
            "closed demo:1",
            "opened usb:U1",
            "closed usb:U1",
            "opened usb:U2"
        ]
    );

    // Closed: the core stays, with nothing, and can open again.
    set.close(&DeviceId::usb("U2")).unwrap();
    assert_eq!(heard.take(), ["closed usb:U2"]);
    assert_eq!(ids(&set), [extra.as_str()]);
    assert_eq!(set.find_serial("U2"), None);
    assert!(!main.is_connected());
    assert!(Arc::ptr_eq(&main, &set.main_core()));
    main.open_demo().unwrap();
    assert_eq!(ids(&set)[0], "demo:3");
}

#[test]
fn wp_mdqt1_the_main_core_shares_one_session_per_serial() {
    let (set, _heard) = set_with_events();
    let main = set.main_core();
    attach(&main, "S1");
    // Not listed since: an open sees it all the same.
    assert_eq!(
        set.open_with(DeviceId::usb("S1"), String::new(), |_| panic!("dialled")),
        Err(CoreSetError::AlreadyOpen {
            serial: "S1".into(),
            id: DeviceId::usb("S1"),
        })
    );
    let other = set
        .open_with(DeviceId::usb("S2"), String::new(), |c| {
            c.attach_probed(Box::new(FakeDevice::demo()), "S2".into(), VID)
        })
        .unwrap();
    assert!(matches!(
        main.claim("usb:S2", "S2"),
        Err(DspiError::InvalidArgument { msg }) if msg.contains("already open as usb:S2")
    ));
    assert!(matches!(
        main.select_device("S2".into()),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert!(main.is_connected(), "the refusal closed nothing");
    // What a core claims but has not opened yet survives a sync.
    main.claim("usb:S3", "S3").unwrap();
    set.refresh();
    assert_eq!(set.find_serial("S3"), Some(DeviceId::usb("S3")));
    main.unclaim("S3");
    assert_eq!(set.find_serial("S3"), None);
    assert_eq!(
        set.find_serial("S1"),
        Some(DeviceId::usb("S1")),
        "its own stays"
    );
    // The other one closed, the main core may have it.
    set.close(&other).unwrap();
    attach(&main, "S2");
    assert_eq!(ids(&set), ["usb:S2"]);
    assert_eq!(set.find_serial("S1"), None);
}

#[test]
fn wp_mdqt1_an_entry_replaced_on_its_core_takes_the_new_id() {
    let (set, heard) = set_with_events();
    let first = set
        .open_with(DeviceId::usb("A"), String::new(), |c| {
            c.attach_probed(Box::new(FakeDevice::demo()), "A".into(), VID)
        })
        .unwrap();
    let core = set.core(&first).unwrap();
    heard.take();
    core.open_demo().unwrap();
    // Looked up by the old id: never the core with another device on it.
    assert!(set.core(&first).is_none());
    assert_eq!(heard.take(), ["closed usb:A", "opened demo:1"]);
    assert_eq!(set.find_serial("A"), None);
    assert!(Arc::ptr_eq(&set.core(&DeviceId::demo(1)).unwrap(), &core));
    // With nothing on it the entry stays, lost, until it is closed.
    core.disconnect();
    set.refresh();
    assert_eq!(heard.take(), ["lost demo:1"]);
    assert_eq!(set.list()[0].state, DeviceLiveness::Lost);
    core.open_demo().unwrap();
    set.refresh();
    assert_eq!(heard.take(), ["restored demo:1"]);
    set.close(&DeviceId::demo(1)).unwrap();
    assert!(set.list().is_empty());
}

#[test]
fn wp_mdqt1_a_panic_in_the_open_closure_gives_everything_back() {
    let set = CoreSet::new();
    for n in 0..CORE_SET_CAP {
        let r = catch_unwind(AssertUnwindSafe(|| {
            set.open_with(DeviceId::usb("P"), String::new(), |c| {
                c.attach_probed(Box::new(FakeDevice::demo()), "P".into(), VID)
                    .unwrap();
                panic!("the embedder's dial")
            })
        }));
        assert!(r.is_err(), "{n}");
        assert_eq!(set.find_serial("P"), None, "{n}");
    }
    // The slots too: the set still holds eight.
    for _ in 0..CORE_SET_CAP {
        set.open(DeviceTarget::Demo).unwrap();
    }
}

#[test]
fn wp_mdqt1_another_serial_than_reserved_is_refused() {
    let (set, heard) = set_with_events();
    let r = set.open_with(DeviceId::usb("ASKED"), String::new(), |c| {
        c.attach_probed(Box::new(FakeDevice::demo()), "OTHER".into(), VID)
    });
    assert!(
        matches!(&r, Err(CoreSetError::Device(DspiError::InvalidArgument { msg }))
            if msg.contains("usb:ASKED") && msg.contains("OTHER")),
        "{r:?}"
    );
    assert!(set.list().is_empty());
    assert_eq!(set.find_serial("ASKED"), None);
    assert!(heard.take().is_empty());
}

#[test]
fn wp_mdqt1_a_hub_id_with_a_slash_is_refused() {
    for bad in ["hub:a/b/S", "link:h:1/x/S"] {
        assert_eq!(DeviceId::parse(bad), None, "{bad}");
    }
    let set = CoreSet::new();
    let id = DeviceId::hub("a/b", "S");
    assert!(matches!(
        set.open_with(id, String::new(), |_| panic!("dialled")),
        Err(CoreSetError::Device(DspiError::InvalidArgument { .. }))
    ));
    #[cfg(feature = "link")]
    {
        assert!(matches!(
            set.open(DeviceTarget::Hub(HubTarget {
                url: "ws://127.0.0.1:1/dspi/v1".into(),
                hub_id: "a/b".into(),
                serial: "S".into(),
                client_name: String::new(),
                app_version: String::new(),
                credential: String::new(),
                tls_advertised: false,
            })),
            Err(CoreSetError::Device(DspiError::InvalidArgument { .. }))
        ));
        // A hub dialled on a member core all the same goes by its address.
        assert_eq!(
            hub_unit_id("a/b", "ws://10.0.0.2:80/dspi/v1", "S"),
            DeviceId::link("10.0.0.2:80", "S")
        );
    }
    assert_eq!(set.find_serial("S"), None);
}

#[test]
fn wp_mdqt1_a_folder_that_cannot_be_used_does_not_keep_a_device_from_opening() {
    let set = CoreSet::new();
    // A file where the folder should go.
    let base = std::env::current_exe().unwrap();
    let blocked = base.join("not-a-folder").to_string_lossy().into_owned();
    let config = CoreSetConfig {
        snapshot_dir: blocked.clone(),
        measure_dir: blocked,
        ..CoreSetConfig::default()
    };
    let main = set.main_core();
    let said = set.set_config(config.clone());
    let direct = config.apply(&DspiCore::new());
    assert_eq!(
        said.is_err(),
        direct.is_err(),
        "set_config says what a core says"
    );
    assert!(set.open(DeviceTarget::Demo).is_ok());
    main.open_demo().unwrap();
    assert_eq!(set.list().len(), 2);
}

#[test]
fn wp_mdqt1_a_broker_taken_by_one_core_is_the_whole_sets() {
    #[derive(Default)]
    struct Hub(Mutex<Vec<String>>);
    impl DeviceBroker for Hub {
        fn devices(&self) -> Vec<DeviceInfo> {
            bus(&["A", "B"])
        }
        fn inaccessible(&self) -> u32 {
            0
        }
        fn open(&self, serial: &str) -> crate::Result<crate::BrokeredDevice> {
            self.0.lock().unwrap().push(serial.to_owned());
            Ok(crate::BrokeredDevice {
                transport: Box::new(FakeDevice::demo()),
                vid: VID,
            })
        }
    }
    let (set, heard) = set_with_events();
    let hub = Arc::new(Hub::default());
    let main = set.main_core();
    attach(&main, "A");
    set.refresh();
    // The fakes are no USB connections: what a hub's start closes is a
    // brokered one's way back, so both are opened through a first broker.
    main.set_device_broker(Some(hub.clone()));
    main.select_device("A".into()).unwrap();
    let b = set.open(DeviceTarget::Usb { serial: "B".into() }).unwrap();
    assert_eq!(*hub.0.lock().unwrap(), ["A", "B"], "a new core has it too");
    assert_eq!(set.scan_devices().len(), 2);
    heard.take();

    // Another owner: the caller's serial comes back to it, B's core is armed.
    let next = Arc::new(Hub::default());
    assert_eq!(
        main.set_device_broker(Some(next.clone())).as_deref(),
        Some("A")
    );
    assert!(!main.is_connected() && !set.core(&b).unwrap().is_connected());
    main.select_device("A".into()).unwrap();
    set.poll_hotplug();
    assert_eq!(*next.0.lock().unwrap(), ["A", "B"]);
    assert!(set.core(&b).unwrap().is_connected());
    assert!(set.list().iter().all(|d| d.state == DeviceLiveness::Live));
}

#[cfg(feature = "link")]
mod hub {
    use dspi_link::fakehub::{FakeHub, HubConfig};

    use super::*;

    fn dial(core: &DspiCore, hub: &FakeHub, cfg: &HubConfig) -> u32 {
        core.link_connect_advertised(
            hub.url(),
            "wp-mdqt1".into(),
            "test".into(),
            String::new(),
            cfg.hub_id.clone(),
            false,
        )
        .unwrap();
        let start = std::time::Instant::now();
        loop {
            if let Some(d) = core
                .link_devices()
                .into_iter()
                .find(|d| d.serial == cfg.serial)
            {
                return d.handle;
            }
            assert!(start.elapsed() < HUB_LIST_WAIT, "the hub lists its unit");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn cfg(hub: &str, serial: &str) -> HubConfig {
        HubConfig {
            hub_id: hub.into(),
            serial: serial.into(),
            ..HubConfig::default()
        }
    }

    #[test]
    fn wp_mdqt1_a_unit_bound_on_the_main_core_is_in_the_set() {
        let (c1, c2) = (cfg("hub-1", "UNIT1"), cfg("hub-2", "UNIT2"));
        let (h1, h2) = (
            FakeHub::start(c1.clone()).unwrap(),
            FakeHub::start(c2.clone()).unwrap(),
        );
        let (set, heard) = set_with_events();
        let main = set.main_core();
        let handle = dial(&main, &h1, &c1);
        assert!(set.list().is_empty(), "a session alone is no device");
        main.link_select_device(handle, c1.serial.clone()).unwrap();
        let one = DeviceId::hub("hub-1", "UNIT1");
        assert_eq!(ids(&set), [one.to_string()]);
        assert_eq!(heard.take(), [format!("opened {one}")]);

        // Units on two hubs, each on its own core and session.
        let two = set
            .open(DeviceTarget::Hub(HubTarget {
                url: h2.url(),
                hub_id: c2.hub_id.clone(),
                serial: c2.serial.clone(),
                client_name: "wp-mdqt1".into(),
                app_version: "test".into(),
                credential: String::new(),
                tls_advertised: false,
            }))
            .unwrap();
        assert_eq!((h1.sessions().len(), h2.sessions().len()), (1, 1));

        // A unit another core holds cannot be bound behind the set's back.
        let third = set.core(&set.open(DeviceTarget::Demo).unwrap()).unwrap();
        let again = dial(&third, &h1, &c1);
        let refused = third.link_select_device(again, c1.serial.clone());
        assert!(
            matches!(&refused, Err(DspiError::InvalidArgument { msg }) if msg.contains("already open as hub:hub-1/UNIT1")),
            "{refused:?}"
        );
        assert!(third.is_connected(), "its demo stays open");
        third.link_close();

        // A bind that fails gives its serial back.
        let gone = dial(&third, &h2, &c2);
        assert!(third.link_select_device(gone + 1, "NOBODY".into()).is_err());
        assert_eq!(set.find_serial("NOBODY"), None);
        third.link_close();

        // Lost and back under the same id, though the hub is not named meanwhile.
        heard.take();
        h1.drop_connections();
        let start = std::time::Instant::now();
        let mut seen = Vec::new();
        while !seen.contains(&format!("restored {one}")) {
            assert!(start.elapsed().as_secs() < 30, "{seen:?}");
            set.poll_with(Vec::new, |_, _| Ok(()));
            seen.extend(heard.take());
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            seen.iter().all(|e| !e.starts_with("closed")),
            "the id holds through it: {seen:?}"
        );

        set.close(&one).unwrap();
        assert!(main.link_status().phase == crate::LinkPhase::Idle);
        assert_eq!(set.find_serial("UNIT1"), None);
        assert!(set.core(&two).unwrap().is_connected());
    }
}
