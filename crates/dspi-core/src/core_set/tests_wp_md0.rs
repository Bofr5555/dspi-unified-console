//! WP MD0: the set of cores, one per open device.

#![allow(clippy::arithmetic_side_effects)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::Duration;

use dspi_transport::{FakeDevice, FakeRemote};

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

impl Heard {
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

fn set_with_events() -> (Arc<CoreSet>, Arc<Heard>) {
    let set = CoreSet::new();
    let heard = Arc::new(Heard::default());
    set.set_listener(Some(heard.clone()));
    (set, heard)
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

/// Open a demo fake as USB unit `serial` through the set, keeping its remote.
fn open_fake_usb(set: &CoreSet, serial: &str) -> (DeviceId, FakeRemote) {
    let dev = FakeDevice::demo();
    let remote = dev.remote();
    let id = set
        .open_with(DeviceId::usb(serial), String::new(), |c| {
            c.attach_probed(Box::new(dev), serial.to_owned(), VID)
        })
        .unwrap();
    (id, remote)
}

fn wait_for_reader_end(core: &DspiCore) {
    let start = std::time::Instant::now();
    while !core
        .lock()
        .conn
        .as_ref()
        .and_then(|c| c.notifications.as_ref())
        .is_some_and(|n| n.is_disconnected())
    {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the reader never saw the unplug"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn wp_md0_ids_name_their_kind_and_serial() {
    let cases = [
        (DeviceId::usb("A1"), DeviceTransport::Usb, Some("A1")),
        (
            DeviceId::hub("hub-7", "B2"),
            DeviceTransport::Hub,
            Some("B2"),
        ),
        (
            DeviceId::link("10.0.0.2:8080", "C3"),
            DeviceTransport::Link,
            Some("C3"),
        ),
        (DeviceId::demo(2), DeviceTransport::Demo, None),
        (DeviceId::file(1), DeviceTransport::File, None),
    ];
    for (id, transport, serial) in cases {
        assert_eq!(id.transport(), transport, "{id}");
        assert_eq!(id.serial(), serial, "{id}");
        assert_eq!(DeviceId::parse(id.as_str()), Some(id.clone()));
    }
    for bad in ["", "usb:", "hub:x", "hub:/A", "demo:x", "tape:1", "nothing"] {
        assert_eq!(DeviceId::parse(bad), None, "{bad}");
    }
    #[cfg(feature = "link")]
    assert_eq!(host_port("ws://10.0.0.2:8080/dspi/v1"), "10.0.0.2:8080");
}

#[test]
fn wp_md0_three_demos_each_keep_their_own_mirror_and_journal() {
    let (set, heard) = set_with_events();
    let ids: Vec<DeviceId> = (0..3)
        .map(|_| set.open(DeviceTarget::Demo).unwrap())
        .collect();
    assert_eq!(
        ids,
        [DeviceId::demo(1), DeviceId::demo(2), DeviceId::demo(3)]
    );
    assert_eq!(
        heard.take(),
        ["opened demo:1", "opened demo:2", "opened demo:3"]
    );

    let cores: Vec<_> = ids.iter().map(|id| set.core(id).unwrap()).collect();
    assert!(!Arc::ptr_eq(&cores[0], &cores[1]), "a core each");
    for (n, core) in cores.iter().enumerate() {
        for k in (0..=n).rev() {
            core.set_master_volume(-3.0 * (n as f32 + 1.0) - k as f32)
                .unwrap();
        }
    }
    for (n, core) in cores.iter().enumerate() {
        assert_eq!(core.get_state().master_volume_db, -3.0 * (n as f32 + 1.0));
        assert_eq!(
            core.history(10).len(),
            n + 1,
            "{} has its own journal",
            ids[n]
        );
    }
    cores[2].undo().unwrap();
    assert_eq!(cores[2].get_state().master_volume_db, -10.0);
    assert_eq!(
        cores[1].get_state().master_volume_db,
        -6.0,
        "undo stays on its device"
    );

    let list = set.list();
    assert_eq!(list.len(), 3);
    assert_eq!(list[1].label, "Demo 2");
    assert_eq!(list[1].transport, DeviceTransport::Demo);
    assert!(list.iter().all(|d| d.state == DeviceLiveness::Live));
}

#[test]
fn wp_md0_closing_one_leaves_the_others_live() {
    let (set, heard) = set_with_events();
    let a = set.open(DeviceTarget::Demo).unwrap();
    let b = set.open(DeviceTarget::Demo).unwrap();
    let (u, _remote) = open_fake_usb(&set, "U1");
    heard.take();
    let b_core = set.core(&b).unwrap();

    set.close(&b).unwrap();
    assert_eq!(heard.take(), ["closed demo:2"]);
    assert!(
        !b_core.is_connected(),
        "the closed one's core has nothing open"
    );
    assert!(set.core(&b).is_none());
    assert!(matches!(set.close(&b), Err(CoreSetError::Unknown { .. })));
    for id in [&a, &u] {
        let core = set.core(id).unwrap();
        assert!(core.is_connected(), "{id} stays open");
        core.set_master_volume(-12.0).unwrap();
    }
    assert_eq!(
        set.list().iter().map(|d| d.id.clone()).collect::<Vec<_>>(),
        [a, u.clone()]
    );
    set.close(&u).unwrap();
    assert_eq!(set.find_serial("U1"), None, "its serial is free again");
    open_fake_usb(&set, "U1");
}

#[test]
fn wp_md0_the_same_serial_twice_is_refused() {
    let set = CoreSet::new();
    let (first, _remote) = open_fake_usb(&set, "S1");
    let dialled = AtomicUsize::new(0);
    let again = set.open_with(DeviceId::hub("hub-1", "S1"), String::new(), |_| {
        dialled.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });
    assert_eq!(
        again,
        Err(CoreSetError::AlreadyOpen {
            serial: "S1".into(),
            id: first.clone(),
        })
    );
    assert_eq!(dialled.load(Ordering::SeqCst), 0, "refused before any dial");
    assert_eq!(
        set.open(DeviceTarget::Usb {
            serial: "S1".into()
        }),
        Err(CoreSetError::AlreadyOpen {
            serial: "S1".into(),
            id: first.clone(),
        })
    );
    assert_eq!(set.find_serial("S1"), Some(first));

    // A core of the set cannot open it behind the set's back either.
    let demo = set.core(&set.open(DeviceTarget::Demo).unwrap()).unwrap();
    assert!(matches!(
        demo.refuse_held_by_another("S1"),
        Err(DspiError::InvalidArgument { msg }) if msg.contains("already open as usb:S1")
    ));
    #[cfg(feature = "usb")]
    assert!(matches!(
        demo.select_device("S1".into()),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert!(demo.is_connected(), "the refusal closed nothing");
    assert_eq!(demo.set_reserved(), ["S1"]);
    assert_eq!(DspiCore::new().set_reserved(), Vec::<String>::new());

    // A failed open gives its serial back.
    let failed = set.open_with(DeviceId::usb("S2"), String::new(), |_| {
        Err(DspiError::Disconnected)
    });
    assert_eq!(failed, Err(CoreSetError::Device(DspiError::Disconnected)));
    let unopened = set.open_with(DeviceId::usb("S2"), String::new(), |_| Ok(()));
    assert_eq!(
        unopened,
        Err(CoreSetError::Device(DspiError::Disconnected)),
        "nothing connected"
    );
    assert_eq!(set.find_serial("S2"), None);
    assert_eq!(set.list().len(), 2);
}

#[test]
fn wp_md0_one_scan_wakes_only_the_right_core() {
    let (set, heard) = set_with_events();
    let (a, _ra) = open_fake_usb(&set, "A");
    let (b, rb) = open_fake_usb(&set, "B");
    let (c, _rc) = open_fake_usb(&set, "C");
    let scans = AtomicUsize::new(0);
    let scan = |serials: &'static [&'static str]| {
        let scans = &scans;
        move || {
            scans.fetch_add(1, Ordering::SeqCst);
            bus(serials)
        }
    };
    let never =
        |_: &DspiCore, s: &str| -> crate::Result<()> { panic!("no reopen of {s} expected") };
    set.poll_with(scan(&["A", "B", "C"]), never);
    heard.take();

    // A member's own poll does not scan, and leaves its device alone.
    set.core(&b).unwrap().poll_hotplug();

    // B is unplugged: one scan, B alone is closed and lost.
    rb.disconnect();
    wait_for_reader_end(&set.core(&b).unwrap());
    set.poll_with(scan(&["A", "C"]), never);
    assert_eq!(scans.load(Ordering::SeqCst), 2, "one enumeration per poll");
    assert_eq!(heard.take(), ["departed B", "lost usb:B"]);
    let states: Vec<_> = set.list().iter().map(|d| d.state).collect();
    assert_eq!(
        states,
        [
            DeviceLiveness::Live,
            DeviceLiveness::Lost,
            DeviceLiveness::Live
        ]
    );
    assert!(set.core(&a).unwrap().is_connected() && set.core(&c).unwrap().is_connected());
    assert_eq!(
        set.find_serial("B"),
        Some(b.clone()),
        "a lost unit keeps its place"
    );

    // B comes back: only B's core reopens it.
    let reopened = Mutex::new(Vec::new());
    set.poll_with(scan(&["A", "B", "C"]), |core, serial| {
        reopened.lock().unwrap().push(serial.to_owned());
        assert!(Arc::ptr_eq(
            &set.core(&b).unwrap(),
            &core.me.upgrade().unwrap()
        ));
        core.attach_probed(Box::new(FakeDevice::demo()), serial.to_owned(), VID)
    });
    assert_eq!(*reopened.lock().unwrap(), ["B"]);
    assert_eq!(heard.take(), ["arrived B", "restored usb:B"]);
    assert!(set.list().iter().all(|d| d.state == DeviceLiveness::Live));

    // Other units coming and going wake nobody.
    set.poll_with(scan(&["A", "B", "C", "D"]), never);
    set.poll_with(scan(&["A", "B", "C"]), never);
    assert_eq!(heard.take(), ["arrived D", "departed D"]);
}

#[test]
fn wp_md0_the_set_holds_eight() {
    let set = CoreSet::new();
    for _ in 0..CORE_SET_CAP {
        set.open(DeviceTarget::Demo).unwrap();
    }
    assert_eq!(set.open(DeviceTarget::Demo), Err(CoreSetError::Full));
    assert_eq!(
        set.open_with(DeviceId::usb("X"), String::new(), |_| Ok(())),
        Err(CoreSetError::Full)
    );
    set.close(&DeviceId::demo(3)).unwrap();
    let again = set.open(DeviceTarget::Demo).unwrap();
    assert!(
        !(1..=8).map(DeviceId::demo).any(|d| d == again),
        "a new number: {again}"
    );
    assert_eq!(set.list().len(), CORE_SET_CAP);
}

#[test]
fn wp_md0_a_document_opens_as_a_file() {
    let set = CoreSet::new();
    let list: serde_json::Value =
        serde_json::from_str(&crate::template_list(String::new())).unwrap();
    let first = list[0]["id"].as_str().unwrap().to_owned();
    let text = crate::template_text(first, String::new()).unwrap();
    let id = set
        .open(DeviceTarget::Document {
            text,
            file_name: "Living room.dspipreset".into(),
        })
        .unwrap();
    assert_eq!(id, DeviceId::file(1));
    let d = &set.list()[0];
    assert_eq!(
        (d.label.as_str(), d.transport),
        ("Living room.dspipreset", DeviceTransport::File)
    );
    assert_eq!(
        set.open(DeviceTarget::Document {
            text: "no document".into(),
            file_name: "x".into(),
        })
        .map_err(|e| matches!(e, CoreSetError::Device(DspiError::InvalidArgument { .. }))),
        Err(true)
    );
    assert_eq!(set.list().len(), 1);
}

#[test]
fn wp_md0_every_core_gets_the_shared_folders() {
    let root = std::env::temp_dir()
        .join("wp-md0-core-set")
        .join(format!("{}", std::process::id()));
    let dir = |n: &str| {
        let d = root.join(n);
        std::fs::create_dir_all(&d).unwrap();
        d.to_string_lossy().into_owned()
    };
    let set = CoreSet::new();
    let early = set.core(&set.open(DeviceTarget::Demo).unwrap()).unwrap();
    let config = CoreSetConfig {
        limits_dir: dir("limits"),
        snapshot_dir: dir("snapshots"),
        measure_dir: dir("measure"),
        event_log_dir: dir("events"),
        link_dir: dir("link"),
    };
    set.set_config(config.clone()).unwrap();
    let late = set.core(&set.open(DeviceTarget::Demo).unwrap()).unwrap();
    for core in [&early, &late] {
        core.snapshot_capture("wp-md0".into(), String::new())
            .unwrap();
    }
    // Both demos share a serial: one folder holds both cores' snapshots.
    assert_eq!(
        late.snapshot_list(String::new()).len(),
        2,
        "one folder for both"
    );
    assert!(
        std::fs::read_dir(&config.snapshot_dir)
            .unwrap()
            .next()
            .is_some()
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn wp_md0_a_listener_calling_back_into_the_set_does_not_deadlock() {
    struct Back(std::sync::Weak<CoreSet>, AtomicUsize);
    impl CoreSetListener for Back {
        fn on_event(&self, _: CoreSetEvent) {
            if let Some(set) = self.0.upgrade() {
                let _ = set.list();
                let _ = set.find_serial("A");
            }
            self.1.fetch_add(1, Ordering::SeqCst);
        }
    }
    let set = CoreSet::new();
    let back = Arc::new(Back(Arc::downgrade(&set), AtomicUsize::new(0)));
    set.set_listener(Some(back.clone()));
    let (tx, rx) = mpsc::channel();
    let s = Arc::clone(&set);
    std::thread::spawn(move || {
        let id = s.open(DeviceTarget::Demo).unwrap();
        s.poll_with(|| bus(&["A"]), |_, _| Ok(()));
        s.close(&id).unwrap();
        tx.send(()).unwrap();
    });
    rx.recv_timeout(Duration::from_secs(10))
        .expect("a re-entrant listener deadlocked the set");
    assert_eq!(back.1.load(Ordering::SeqCst), 3, "opened, arrived, closed");
}

#[cfg(feature = "link")]
mod hub {
    use dspi_link::fakehub::{FakeHub, HubConfig};

    use super::*;

    fn target(hub: &FakeHub, cfg: &HubConfig) -> HubTarget {
        HubTarget {
            url: hub.url(),
            hub_id: cfg.hub_id.clone(),
            serial: cfg.serial.clone(),
            client_name: "wp-md0".into(),
            app_version: "test".into(),
            credential: String::new(),
            tls_advertised: false,
        }
    }

    #[test]
    fn wp_md0_three_demos_and_a_hub_unit_are_open_at_once() {
        let cfg = HubConfig::default();
        let hub = FakeHub::start(cfg.clone()).unwrap();
        let (set, heard) = set_with_events();
        let demos: Vec<_> = (0..3)
            .map(|_| set.open(DeviceTarget::Demo).unwrap())
            .collect();
        let unit = set.open(DeviceTarget::Hub(target(&hub, &cfg))).unwrap();
        assert_eq!(unit, DeviceId::hub(&cfg.hub_id, &cfg.serial));
        assert_eq!(heard.take().last().unwrap(), &format!("opened {unit}"));
        let d = set.list().into_iter().find(|d| d.id == unit).unwrap();
        assert_eq!(
            (d.transport, d.serial.as_str()),
            (DeviceTransport::Hub, cfg.serial.as_str())
        );
        assert!(!d.label.is_empty());

        let hub_core = set.core(&unit).unwrap();
        assert_eq!(hub_core.link_status().phase, crate::LinkPhase::Bound);
        hub_core.set_master_volume(-27.5).unwrap();
        for id in &demos {
            let core = set.core(id).unwrap();
            assert_ne!(
                core.get_state().master_volume_db,
                -27.5,
                "{id} has its own mirror"
            );
            assert!(core.history(10).is_empty(), "{id} has its own journal");
        }
        assert_eq!(hub_core.history(10).len(), 1);
        assert_eq!(hub.sessions().len(), 1);

        // The same unit over USB, or the hub again, is refused.
        assert!(matches!(
            set.open(DeviceTarget::Usb { serial: cfg.serial.clone() }),
            Err(CoreSetError::AlreadyOpen { id, .. }) if id == unit
        ));
        assert!(matches!(
            set.open(DeviceTarget::Hub(target(&hub, &cfg))),
            Err(CoreSetError::AlreadyOpen { .. })
        ));
        assert_eq!(hub.sessions().len(), 1, "no second session was dialled");

        // Closing a demo leaves the hub unit bound; closing the unit leaves the hub.
        set.close(&demos[0]).unwrap();
        assert_eq!(hub_core.link_status().phase, crate::LinkPhase::Bound);
        set.close(&unit).unwrap();
        assert!(!hub_core.is_connected());
        assert_eq!(hub_core.link_status().phase, crate::LinkPhase::Idle);
        assert!(set.core(&demos[1]).unwrap().is_connected());
    }

    /// Eight cores of one set go for one fake hub's unit at once: one session
    /// binds it, the other seven are refused before they dial. The fake hub
    /// serves a single board, so eight bound units on one hub cannot be
    /// played; the set's cap with a hub unit in it is checked instead.
    #[test]
    fn wp_md0_eight_cores_on_one_fake_hub() {
        let cfg = HubConfig::default();
        let hub = FakeHub::start(cfg.clone()).unwrap();
        let set = CoreSet::new();
        let go = Arc::new(std::sync::Barrier::new(CORE_SET_CAP));
        let tries: Vec<_> = (0..CORE_SET_CAP)
            .map(|_| {
                let (set, go, t) = (Arc::clone(&set), Arc::clone(&go), target(&hub, &cfg));
                std::thread::spawn(move || {
                    go.wait();
                    set.open(DeviceTarget::Hub(t))
                })
            })
            .collect();
        let results: Vec<_> = tries.into_iter().map(|t| t.join().unwrap()).collect();
        let opened: Vec<_> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
        assert_eq!(opened.len(), 1, "{results:?}");
        assert!(
            results
                .iter()
                .filter(|r| r.is_err())
                .all(|r| matches!(r, Err(CoreSetError::AlreadyOpen { .. }))),
            "{results:?}"
        );
        assert_eq!(hub.sessions().len(), 1);

        for _ in 1..CORE_SET_CAP {
            set.open(DeviceTarget::Demo).unwrap();
        }
        assert_eq!(set.list().len(), CORE_SET_CAP);
        assert_eq!(set.open(DeviceTarget::Demo), Err(CoreSetError::Full));
        set.poll_with(Vec::new, |_, _| Ok(()));
        assert!(set.list().iter().all(|d| d.state == DeviceLiveness::Live));
        assert_eq!(
            set.core(opened[0]).unwrap().link_status().phase,
            crate::LinkPhase::Bound
        );
    }

    #[test]
    fn wp_md0_a_reconnecting_hub_unit_is_reported_and_restored() {
        let cfg = HubConfig::default();
        let hub = FakeHub::start(cfg.clone()).unwrap();
        let (set, heard) = set_with_events();
        let unit = set.open(DeviceTarget::Hub(target(&hub, &cfg))).unwrap();
        let core = set.core(&unit).unwrap();
        heard.take();
        hub.drop_connections();
        let start = std::time::Instant::now();
        let mut seen = Vec::new();
        while !seen.contains(&format!("restored {unit}")) {
            assert!(start.elapsed() < Duration::from_secs(30), "{seen:?}");
            set.poll_with(Vec::new, |_, _| Ok(()));
            seen.extend(heard.take());
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            seen.first(),
            Some(&format!("reconnecting {unit}")),
            "{seen:?}"
        );
        assert_eq!(core.link_status().phase, crate::LinkPhase::Bound);
    }

    #[test]
    fn wp_md0_a_hub_without_the_unit_adds_nothing() {
        let cfg = HubConfig::default();
        let hub = FakeHub::start(cfg.clone()).unwrap();
        let set = CoreSet::new();
        let mut t = target(&hub, &cfg);
        t.serial = "NOT-THERE".into();
        t.hub_id = String::new();
        let started = std::time::Instant::now();
        assert_eq!(
            set.open(DeviceTarget::Hub(t)),
            Err(CoreSetError::Device(DspiError::Disconnected))
        );
        assert!(started.elapsed() >= HUB_LIST_WAIT);
        assert!(set.list().is_empty());
        assert_eq!(set.find_serial("NOT-THERE"), None);
        let start = std::time::Instant::now();
        while !hub.sessions().is_empty() {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "the failed core's session stays"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
