//! WP LKUI1: the core's typed System calls (`link_system_view`, `_save`,
//! `_align`, `_recall`, dspi-core `link/system.rs`) against a real hub with
//! the fake devices' audio link: a sender, a fan-out and three units. What
//! the native System screens (Qt, Android) show and do goes through these.
#![allow(clippy::arithmetic_side_effects)]

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use dspi_core::link::{LinkSystemEdits, LinkUnitLatency};
use dspi_core::{DspiCore, DspiError};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, Role};
use dspi_proto::ext::audio_link::LinkRole;
use dspi_transport::FakeDevice;
use dspi_transport::fake::AudioLinkSetup;
use serde_json::json;

fn store_dir() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-lkui1").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

/// A networked hub with the installation attached and its model set.
fn hub(access: AccessMode) -> Hub {
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(store_dir())
        }),
        initial_scan: false,
        ..HubConfig::in_process("LKUI1 hub")
    })
    .unwrap();
    let at = |role, hops, hop_delay_frames| AudioLinkSetup {
        role,
        hops,
        hop_delay_frames,
        ..AudioLinkSetup::default()
    };
    for (serial, setup) in [
        ("LK-TX", at(LinkRole::Sender, 0, 0)),
        ("LK-FAN", at(LinkRole::FanOut, 0, 3)),
        ("LK-A", at(LinkRole::Receiver, 0, 0)),
        ("LK-B", at(LinkRole::Receiver, 1, 0)),
        ("LK-C", at(LinkRole::Receiver, 1, 0)),
    ] {
        hub.attach(
            Box::new(FakeDevice::demo_audio_link_as(setup).with_serial(serial)),
            0,
        )
        .unwrap();
    }
    let model = json!({
        "rooms": [{"id": "main", "name": "Main room"}, {"id": "bar", "name": "Bar"}],
        "senders": [{"id": "tx", "name": "Sender", "serial": "LK-TX"}],
        "fanOuts": [{"id": "fan", "name": "Fan-out", "serial": "LK-FAN", "upstream": "tx", "output": 1}],
        "units": [
            {"id": "A", "name": "Main left", "serial": "LK-A", "room": "main", "upstream": "tx", "output": 0},
            {"id": "B", "name": "Main right", "serial": "LK-B", "room": "main", "upstream": "fan"},
            {"id": "C", "name": "Bar", "serial": "LK-C", "room": "bar", "upstream": "fan"}
        ],
        "scenes": [
            {"id": "evening", "name": "Evening", "actions": [{"target": "all", "op": "master_volume", "value": -1200}]},
            {"id": "closed", "name": "Closed", "systemMute": true, "actions": [{"target": "all", "op": "mute"}]},
            {"id": "click", "name": "Test click", "actions": [{"target": "unit:A", "op": "click", "arg": 0}]}
        ]
    });
    hub.handle()
        .system_set_model(serde_json::from_value(model).unwrap())
        .unwrap();
    hub
}

fn connect(hub: &Hub) -> Arc<DspiCore> {
    let port = hub.network().unwrap().port();
    let core = DspiCore::new();
    core.link_connect(
        format!("ws://127.0.0.1:{port}/dspi/v1"),
        "wp-lkui1".into(),
        "0".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    core
}

/// The view, once the hub lets this session read its status again (1/s).
fn fresh_view(core: &DspiCore) -> dspi_core::link::LinkSystemView {
    let start = Instant::now();
    loop {
        let v = core.link_system_view().unwrap();
        if !v.stale {
            return v;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "always stale");
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn wp_lkui1_view_recall_refusal_align_and_stale_save_on_a_real_hub() {
    let hub = hub(AccessMode::Open);
    let core = connect(&hub);

    let v = fresh_view(&core);
    assert!(v.installed && v.may_edit && v.may_recall, "{v:?}");
    assert_eq!(v.role, "admin");
    assert_eq!(v.unit_count, 3);
    let roles: Vec<(&str, &str)> = v
        .units
        .iter()
        .map(|u| (u.name.as_str(), u.role.as_str()))
        .collect();
    assert_eq!(
        roles,
        [
            ("Sender", "Sender"),
            ("Fan-out", "Fan-out"),
            ("Main left", "Receiver"),
            ("Main right", "Receiver"),
            ("Bar", "Receiver")
        ]
    );
    for u in v.units.iter().filter(|u| u.kind == "unit") {
        assert!(u.connected, "{u:?}");
        assert_eq!(u.state_word, "Locked", "{u:?}");
        assert!(
            u.link_delay_frames.is_some()
                && u.link_delay_min.is_some()
                && u.link_delay_max.is_some(),
            "{u:?}"
        );
        assert!(u.link_delay_ms.unwrap() > 0.0);
    }
    assert_eq!(v.locked, 3);
    assert!(
        v.summary.starts_with("Audio link: 3 of 3 locked"),
        "{}",
        v.summary
    );
    assert_eq!(
        v.scenes.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        ["evening", "closed", "click"]
    );

    // Asked again at once: the hub's limit, the last view again.
    assert!(core.link_system_view().unwrap().stale);

    // Write the link delays: nothing pending afterwards.
    let aligned = core.link_system_align().unwrap();
    assert!(aligned.applied, "{aligned:?}");
    let v = fresh_view(&core);
    assert_eq!(v.pending, 0, "{v:?}");

    // A recall through the sender, then a second before its frame: refused, the held frame named.
    let first = core.link_system_recall("evening".into()).unwrap();
    assert!(first.taken, "{first:?}");
    assert!(
        first
            .message
            .starts_with("Evening recalled through the sender"),
        "{}",
        first.message
    );
    let second = core.link_system_recall("closed".into()).unwrap();
    assert!(!second.taken, "{second:?}");
    assert_eq!(second.held_frame, Some(first.frame));
    assert!(
        second.message.starts_with(&format!(
            "Closed was refused: a scene is already scheduled on the sender for link frame {}",
            first.frame
        )),
        "{}",
        second.message
    );
    assert!(second.writes.iter().all(|w| w.result_word == "Refused"));
    // -- fixup lk1 -- the sender's write is named as the scene; the mute is unchanged
    assert!(
        second.writes.iter().all(|w| w.what == "Scene Closed"),
        "{:?}",
        second.writes
    );
    assert!(
        second.message.ends_with(" System mute unchanged."),
        "{}",
        second.message
    );

    // A save on the view's revision is taken; one on an older revision is a conflict.
    let base = fresh_view(&core).revision;
    let edits = LinkSystemEdits {
        units: vec![LinkUnitLatency {
            unit_id: "B".into(),
            measured_latency_us: Some(150),
        }],
        ..LinkSystemEdits::default()
    };
    let saved = core.link_system_save(base, edits.clone()).unwrap();
    assert_eq!(saved, base + 1);
    match core.link_system_save(base, edits) {
        Err(DspiError::Link { code, msg }) => {
            assert_eq!(code, "conflict");
            assert!(msg.contains("Reload"), "{msg}");
        }
        other => panic!("{other:?}"),
    }
    let v = fresh_view(&core);
    assert_eq!(v.revision, base + 1);
    assert_eq!(
        v.units
            .iter()
            .find(|u| u.id == "B")
            .unwrap()
            .measured_latency_us,
        Some(150)
    );
}

#[test]
fn wp_lkui1_a_viewer_is_offered_nothing_and_refused_by_the_hub() {
    let hub = hub(AccessMode::Pin);
    let net = hub.network().unwrap();
    let core = connect(&hub);
    let pin = net.auth().begin_pairing(Instant::now()).unwrap().pin;
    core.link_pair(pin).unwrap();
    let v = fresh_view(&core);
    assert_eq!(v.role, "control");
    assert!(!v.may_edit && v.may_recall);
    // Control may not save; the admin-only test click is refused with the hub's words.
    let r = core.link_system_save(v.revision, LinkSystemEdits::default());
    assert!(
        matches!(r, Err(DspiError::Link { ref code, .. }) if code == "denied"),
        "{r:?}"
    );
    match core.link_system_recall("click".into()) {
        Err(DspiError::Link { code, msg }) => assert_eq!(code, "denied", "{msg}"),
        other => panic!("{other:?}"),
    }
    // Made a viewer live: the next view says so.
    let cid = net.auth().clients()[0].cid;
    assert!(net.set_role(cid, Role::Viewer));
    let v = fresh_view(&core);
    assert_eq!(v.role, "viewer");
    assert!(!v.may_edit && !v.may_recall);
    let r = core.link_system_recall("evening".into());
    assert!(
        matches!(r, Err(DspiError::Link { ref code, .. }) if code == "denied"),
        "{r:?}"
    );
}
