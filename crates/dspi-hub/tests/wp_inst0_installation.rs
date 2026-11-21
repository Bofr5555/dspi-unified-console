//! WP INST0: the core's Installation setup (`installation_view`, `_draft_edit`,
//! `_validate`, `_diff`, `_save`, dspi-core `alink/installation.rs`) against a
//! real hub with the fake devices' audio link: the units the hub sees and the
//! site does not hold, a unit added with its room, role and upstream, a zone,
//! the save as one `system.set` at the revision, a stale draft, the client
//! read-only mode, and the hub's validator run locally giving the hub's verdict.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use dspi_core::link::{InstallationDraft, InstallationEdit, InstallationRole};
use dspi_core::{DspiCore, DspiError};
use dspi_hub::system::model::SystemModel;
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, ServiceConfig};
use dspi_proto::ext::alink::AlinkRole;
use dspi_transport::FakeDevice;
use dspi_transport::fake::AlinkSetup;
use serde_json::{Map, Value, json};

fn store_dir() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
    let dir = std::env::temp_dir().join("wp-inst0").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn site() -> Value {
    json!({
        "rooms": [{"id": "main", "name": "Main room"}],
        "senders": [{"id": "tx", "name": "Sender", "serial": "IN-TX"}],
        "fanOuts": [{"id": "fan", "name": "Fan-out", "serial": "IN-FAN", "upstream": "tx", "output": 1}],
        "units": [
            {"id": "A", "name": "Main left", "serial": "IN-A", "room": "main", "upstream": "tx", "output": 0}
        ],
        "scenes": [{"id": "closed", "name": "Closed", "systemMute": true, "actions": [{"target": "all", "op": "mute"}]}]
    })
}

/// A networked hub (open: every session is admin) with five units on the
/// audio link attached, of which the site holds the sender, the fan-out and A;
/// with the inventory service or without.
fn hub(inventory: bool) -> Hub {
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Open,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            ..NetworkConfig::new(store_dir())
        }),
        initial_scan: false,
        services: ServiceConfig {
            inventory,
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("INST0 hub")
    })
    .unwrap();
    let at = |role, hops, hop_delay_frames| AlinkSetup {
        role,
        hops,
        hop_delay_frames,
        ..AlinkSetup::default()
    };
    for (serial, setup) in [
        ("IN-TX", at(AlinkRole::Sender, 0, 0)),
        ("IN-FAN", at(AlinkRole::FanOut, 0, 3)),
        ("IN-A", at(AlinkRole::Receiver, 0, 0)),
        ("IN-B", at(AlinkRole::Receiver, 1, 0)),
        ("IN-C", at(AlinkRole::Receiver, 2, 0)),
    ] {
        hub.attach(
            Box::new(FakeDevice::demo_alink_as(setup).with_serial(serial)),
            0,
        )
        .unwrap();
    }
    hub.handle()
        .system_set_model(serde_json::from_value(site()).unwrap())
        .unwrap();
    hub
}

fn connect(hub: &Hub) -> Arc<DspiCore> {
    let port = hub.network().unwrap().port();
    let core = DspiCore::new();
    core.link_connect(
        format!("ws://127.0.0.1:{port}/dspi/v1"),
        "wp-inst0".into(),
        "0".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    core
}

fn add(serial: &str, name: &str, role: InstallationRole, upstream: &str) -> InstallationEdit {
    InstallationEdit::AddUnit {
        serial: serial.into(),
        id: None,
        name: name.into(),
        room: Some("bar".into()),
        role,
        upstream: Some(upstream.into()),
    }
}

#[test]
fn wp_inst0_unseen_units_add_a_unit_and_a_zone_save_and_a_stale_draft() {
    let hub = hub(true);
    let core = connect(&hub);

    let v = core.installation_view().unwrap();
    assert_eq!(
        (v.role.as_str(), v.may_edit, v.inventory),
        ("admin", true, true)
    );
    let (_, revision) = hub.handle().system_model();
    assert_eq!(v.revision, revision);
    let mut unseen: Vec<&str> = v.unseen.iter().map(|u| u.serial.as_str()).collect();
    unseen.sort_unstable();
    assert_eq!(unseen, ["IN-B", "IN-C"], "{:?}", v.unseen);
    assert!(v.unseen.iter().all(|u| u.online));

    let draft = core
        .installation_draft_edit(
            v.draft.clone(),
            vec![
                InstallationEdit::AddRoom {
                    id: Some("bar".into()),
                    name: "Bar".into(),
                },
                add("IN-B", "Bar left", InstallationRole::Repeater, "fan"),
                add("IN-C", "Bar right", InstallationRole::Receiver, "Bar left"),
                InstallationEdit::AddZone {
                    id: None,
                    name: "Downstairs".into(),
                    rooms: vec!["main".into(), "bar".into()],
                    source: None,
                },
            ],
        )
        .unwrap();
    assert_eq!(core.installation_validate(draft.clone()).unwrap(), vec![]);
    assert_eq!(
        core.installation_diff(draft.clone()).unwrap(),
        [
            "Adds room Bar",
            "Adds zone Downstairs",
            "Adds unit Bar left",
            "Adds unit Bar right"
        ]
    );

    let saved = core.installation_save(draft.clone()).unwrap();
    assert_eq!(saved.revision, revision + 1);
    let (model, now) = hub.handle().system_model();
    assert_eq!(now, revision + 1);
    assert_eq!(
        model,
        draft.model_parsed().unwrap(),
        "the hub keeps the draft"
    );
    let c = model.unit("Bar right").unwrap();
    assert_eq!(
        (c.serial.as_str(), c.room.as_deref(), c.upstream.as_str()),
        ("IN-C", Some("bar"), "Bar left")
    );
    assert_eq!(model.path_to("Bar right").unwrap().1, ["fan", "Bar left"]);
    assert_eq!(model.zones[0].rooms, ["main", "bar"]);

    // Read again: nothing unseen, nothing to change.
    let v2 = core.installation_view().unwrap();
    assert!(v2.unseen.is_empty(), "{:?}", v2.unseen);
    assert_eq!(v2.revision, revision + 1);
    assert!(core.installation_diff(v2.draft).unwrap().is_empty());

    // The first draft again, edited at the old revision: `conflict`, and the
    // hub's model is as saved.
    let stale = core
        .installation_draft_edit(
            v.draft,
            vec![InstallationEdit::RenameElement {
                id: "A".into(),
                name: "Front left".into(),
            }],
        )
        .unwrap();
    match core.installation_save(stale) {
        Err(DspiError::Link { code, .. }) => assert_eq!(code, "conflict"),
        other => panic!("{other:?}"),
    }
    assert_eq!(hub.handle().system_model(), (model, revision + 1));

    // A draft the hub would refuse is refused here, with nothing sent.
    let bad = core
        .installation_draft_edit(
            v2_draft(&core),
            vec![InstallationEdit::RemoveElement { id: "tx".into() }],
        )
        .unwrap();
    assert!(matches!(
        core.installation_save(bad),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert_eq!(hub.handle().system_model().1, revision + 1);
}

fn v2_draft(core: &DspiCore) -> InstallationDraft {
    core.installation_view().unwrap().draft
}

/// The client read-only mode refuses a save before its read: a draft at a
/// revision the hub has left would otherwise come back `conflict` from that
/// read. The view (a read) still works.
#[test]
fn wp_inst0_a_read_only_client_is_refused_before_the_read() {
    let hub = hub(false);
    let core = connect(&hub);
    let v = core.installation_view().unwrap();
    // A hub without the inventory: none unseen, and said so.
    assert!(!v.inventory && v.unseen.is_empty());
    let draft = v.draft;
    hub.handle()
        .system_set_model(serde_json::from_value(site()).unwrap())
        .unwrap();
    core.set_client_read_only(true).unwrap();
    let refusals = core.client_read_only_refusals();
    assert_eq!(
        core.installation_save(draft.clone()),
        Err(DspiError::ReadOnly)
    );
    assert_eq!(core.client_read_only_refusals(), refusals + 1);
    assert!(core.installation_view().is_ok());
    core.set_client_read_only(false).unwrap();
    match core.installation_save(draft) {
        Err(DspiError::Link { code, .. }) => assert_eq!(code, "conflict"),
        other => panic!("{other:?}"),
    }
}

/// The models of the hub's own suites (LKUI1, AUSB1, ATHUBM, the Terminal's
/// cinema) and broken variants of them: the core's local verdict is the
/// hub's, word for word.
#[test]
fn wp_inst0_local_validation_equals_the_hubs_on_the_hub_model_fixtures() {
    let hub = hub(false);
    let core = connect(&hub);
    let lkui1 = json!({
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
    let ausb1 = json!({
        "rooms": [{"id": "main", "name": "Main room"}],
        "senders": [{"id": "tx", "name": "Sender", "serial": "HW-TX"}],
        "fanOuts": [{"id": "fan", "name": "Fan-out", "serial": "HW-FAN", "upstream": "tx", "output": 1}],
        "units": [
            {"id": "A", "name": "Main left", "serial": "HW-A", "room": "main", "upstream": "tx"},
            {"id": "U1", "name": "Rear L/R", "serial": "HW-U1", "room": "main", "transport": "usb_direct",
             "usbMode": "phase_lock",
             "speakers": [{"output": 0, "distanceM": 3.0}, {"output": 1, "distanceM": 3.0}]},
            {"id": "U2", "name": "Height", "serial": "HW-U2", "room": "main", "transport": "usb_direct",
             "speakers": [{"output": 0, "distanceM": 3.0}]}
        ],
        "scenes": [{"id": "rear", "name": "Rear quiet", "actions": [{"target": "unit:U1", "op": "mute"}]}]
    });
    let athubm = json!({
        "rooms": [{"id": "main", "name": "Main room"}],
        "units": [
            {"id": "SRC", "name": "Front", "serial": "HW-SRC", "room": "main", "transport": "spdif",
             "spdifSource": true, "spdifTxMask": 1, "speakers": [{"output": 2, "distanceM": 3.0}]},
            {"id": "F1", "name": "Rear", "serial": "HW-F1", "room": "main", "transport": "spdif",
             "upstream": "SRC", "speakers": [{"output": 0, "distanceM": 3.0}]},
            {"id": "F2", "name": "Height", "serial": "HW-F2", "room": "main", "transport": "spdif",
             "upstream": "SRC", "onLockError": "play_unaligned", "speakers": [{"output": 0, "distanceM": 3.0}]}
        ]
    });
    let cinema: Value = serde_json::from_str(include_str!(
        "../../../apps/terminal/tests/fixtures/ausb3/cinema-7.1.expected/hub-system-model.json"
    ))
    .unwrap();
    let broken = |base: &Value, path: &[&str], v: Value| {
        let mut m = base.clone();
        let mut at = &mut m;
        for k in path {
            at = match k.parse::<usize>() {
                Ok(i) => &mut at[i],
                Err(_) => &mut at[*k],
            };
        }
        *at = v;
        m
    };
    let fixtures = [
        lkui1.clone(),
        ausb1.clone(),
        athubm.clone(),
        cinema,
        broken(&lkui1, &["units", "1", "upstream"], json!("ghost")),
        broken(&lkui1, &["fanOuts", "0", "upstream"], json!("B")),
        broken(&lkui1, &["units", "2", "serial"], json!("LK-A")),
        broken(&lkui1, &["rateHz"], json!(1)),
        broken(
            &lkui1,
            &["scenes", "0", "actions", "0", "op"],
            json!("explode"),
        ),
        broken(
            &lkui1,
            &["scenes", "2", "actions", "0", "target"],
            json!("room:nowhere"),
        ),
        broken(&lkui1, &["rooms", "1", "name"], json!("x".repeat(65))),
        broken(&lkui1, &["units", "0", "id"], json!("fan")),
        broken(&ausb1, &["units", "1", "upstream"], json!("tx")),
        broken(&ausb1, &["units", "1", "usbMode"], json!("warp")),
        broken(&athubm, &["units", "1", "upstream"], json!("F2")),
        broken(&athubm, &["units", "0", "spdifTxMask"], json!(2)),
    ];
    let mut taken = 0;
    for (i, fixture) in fixtures.iter().enumerate() {
        let model: SystemModel = serde_json::from_value(fixture.clone()).unwrap();
        let local = core
            .installation_validate(InstallationDraft::new(0, &model).unwrap())
            .unwrap();
        let (_, revision) = hub.handle().system_model();
        let mut f = Map::new();
        f.insert("model".into(), fixture.clone());
        f.insert("revision".into(), revision.into());
        match core.link_hub_request("system.set", f) {
            Ok(_) => {
                assert_eq!(local, vec![], "fixture {i}: the hub took it");
                taken += 1;
            }
            Err(DspiError::Link { code, msg }) => {
                assert_eq!(code, "bad_request", "fixture {i}: {msg}");
                assert_eq!(local.len(), 1, "fixture {i}: the hub refused it: {msg}");
                assert!(
                    msg.ends_with(&local[0].message),
                    "fixture {i}: hub {msg:?}, local {:?}",
                    local[0].message
                );
            }
            Err(e) => panic!("fixture {i}: {e:?}"),
        }
    }
    assert_eq!(taken, 4, "the four suites' models are taken, no broken one");
}
