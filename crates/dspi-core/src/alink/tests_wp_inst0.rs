//! WP INST0: the Installation setup's pure part: edits on a draft, the
//! hub's validator run locally, the diff in words. The hub end to end is
//! dspi-hub's `tests/wp_inst0_installation.rs`.

use serde_json::json;

use super::*;
use crate::DspiCore;

fn site() -> SystemModel {
    serde_json::from_value(json!({
        "rooms": [{"id": "main", "name": "Main room"}],
        "senders": [{"id": "tx", "name": "Sender", "serial": "S-TX"}],
        "units": [
            {"id": "A", "name": "Left", "serial": "S-A", "room": "main", "upstream": "tx"}
        ],
        "scenes": [{"id": "quiet", "name": "Quiet", "actions": [
            {"target": "unit:A", "op": "mute"}, {"target": "room:main", "op": "mute"}
        ]}]
    }))
    .unwrap()
}

fn draft() -> InstallationDraft {
    InstallationDraft::new(7, &site()).unwrap()
}

fn edit(d: InstallationDraft, edits: Vec<InstallationEdit>) -> crate::Result<InstallationDraft> {
    DspiCore::new().installation_draft_edit(d, edits)
}

fn add_unit(
    serial: &str,
    room: Option<&str>,
    role: InstallationRole,
    up: Option<&str>,
) -> InstallationEdit {
    InstallationEdit::AddUnit {
        serial: serial.into(),
        id: None,
        name: String::new(),
        room: room.map(Into::into),
        role,
        upstream: up.map(Into::into),
    }
}

#[test]
fn wp_inst0_add_a_unit_with_room_role_and_upstream_then_a_zone_validates() {
    let core = DspiCore::new();
    let d = edit(
        draft(),
        vec![
            InstallationEdit::AddRoom {
                id: None,
                name: "Bar".into(),
            },
            add_unit("S-B", Some("Bar"), InstallationRole::Repeater, Some("tx")),
            add_unit("S-C", Some("Bar"), InstallationRole::Receiver, Some("S-B")),
            InstallationEdit::AddZone {
                id: Some("downstairs".into()),
                name: "Downstairs".into(),
                rooms: vec!["main".into(), "Bar".into()],
                source: None,
            },
            InstallationEdit::AddGroup {
                id: None,
                name: "Bar units".into(),
                units: vec!["S-B".into(), "S-C".into()],
            },
        ],
    )
    .unwrap();
    let m = d.model_parsed().unwrap();
    // Ids made from the name, else the serial.
    assert_eq!(m.rooms[1].id, "Bar");
    let b = m.unit("S-B").unwrap();
    assert!(b.repeater && b.on_alink() && b.upstream == "tx" && b.room.as_deref() == Some("Bar"));
    assert_eq!(m.unit("S-C").unwrap().upstream, "S-B");
    assert_eq!(m.groups[0].id, "Bar units");
    assert_eq!(core.installation_validate(d.clone()).unwrap(), vec![]);
    // The base is untouched; the revision stays the one read.
    assert_eq!((d.revision, d.base_parsed().unwrap()), (7, site()));
    let changes = core.installation_diff(d).unwrap();
    assert_eq!(
        changes,
        [
            "Adds room Bar",
            "Adds zone Downstairs",
            "Adds group Bar units",
            "Adds unit S-B",
            "Adds unit S-C",
        ]
    );
}

#[test]
fn wp_inst0_edits_that_cannot_be_made_are_refused_all_or_none() {
    let refuse = |e: InstallationEdit| match edit(draft(), vec![e]) {
        Err(crate::DspiError::InvalidArgument { msg }) => msg,
        other => panic!("{other:?}"),
    };
    assert!(
        refuse(add_unit(
            "S-A",
            None,
            InstallationRole::Receiver,
            Some("tx")
        ))
        .contains("already")
    );
    assert!(
        refuse(add_unit(
            "S-X",
            Some("nowhere"),
            InstallationRole::Receiver,
            None
        ))
        .contains("no room")
    );
    assert!(
        refuse(add_unit(
            "S-X",
            None,
            InstallationRole::UsbDirect,
            Some("tx")
        ))
        .contains("no upstream")
    );
    assert!(
        refuse(add_unit(
            "S-X",
            None,
            InstallationRole::Receiver,
            Some("ghost")
        ))
        .contains("ghost")
    );
    let taken = InstallationEdit::AddRoom {
        id: Some("A".into()),
        name: String::new(),
    };
    assert!(refuse(taken).contains("taken"));
    let bad_id = InstallationEdit::AddRoom {
        id: Some("a:b".into()),
        name: String::new(),
    };
    assert!(refuse(bad_id).contains("no ':'"));
    let long = InstallationEdit::RenameElement {
        id: "A".into(),
        name: "x".repeat(65),
    };
    assert!(refuse(long).contains("64"));
    let mode = InstallationEdit::SetUnitUsbMode {
        id: "A".into(),
        mode: Some("fast".into()),
        play_unaligned: false,
    };
    assert!(refuse(mode).contains("USB mode"));
    // The second edit fails: nothing of the first is kept, and the error
    // names the edit.
    let err = edit(
        draft(),
        vec![
            InstallationEdit::AddRoom {
                id: None,
                name: "Bar".into(),
            },
            InstallationEdit::SetUnitRoom {
                id: "nobody".into(),
                room: None,
            },
        ],
    )
    .unwrap_err();
    assert!(err.to_string().contains("edit 2"), "{err}");
}

#[test]
fn wp_inst0_ids_are_made_free() {
    let d = edit(
        draft(),
        vec![
            InstallationEdit::AddRoom {
                id: None,
                name: "Main room".into(),
            },
            InstallationEdit::AddRoom {
                id: None,
                name: "main".into(),
            },
            InstallationEdit::AddRoom {
                id: None,
                name: "a:ll".into(),
            },
            InstallationEdit::AddRoom {
                id: None,
                name: String::new(),
            },
        ],
    )
    .unwrap();
    let ids: Vec<String> = d
        .model_parsed()
        .unwrap()
        .rooms
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, ["main", "Main room", "main-2", "room", "room-2"]);
}

#[test]
fn wp_inst0_roles_drop_what_they_cannot_have() {
    let d = edit(
        draft(),
        vec![
            InstallationEdit::SetUnitRole {
                id: "A".into(),
                role: InstallationRole::UsbDirect,
            },
            InstallationEdit::SetUnitUsbMode {
                id: "A".into(),
                mode: Some("phase_lock".into()),
                play_unaligned: true,
            },
        ],
    )
    .unwrap();
    let core = DspiCore::new();
    let m = d.model_parsed().unwrap();
    let a = m.unit("A").unwrap();
    assert!(a.usb_direct() && a.upstream.is_empty());
    assert_eq!(core.installation_validate(d.clone()).unwrap(), vec![]);
    assert_eq!(
        core.installation_diff(d.clone()).unwrap(),
        [
            "Unit Left: role receiver → USB-direct; fed by Sender → none; USB mode its own → phase_lock; in lock error mute → play unaligned"
        ]
    );
    // Back to a receiver: the lock-error action goes (an alink unit has
    // none); no upstream yet, which the validator names.
    let d = edit(
        d,
        vec![InstallationEdit::SetUnitRole {
            id: "A".into(),
            role: InstallationRole::Receiver,
        }],
    )
    .unwrap();
    let issues = core.installation_validate(d.clone()).unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].subject, "A");
    assert!(issues[0].message.contains("upstream"), "{issues:?}");
    let d = edit(
        d,
        vec![InstallationEdit::SetUnitUpstream {
            id: "A".into(),
            upstream: "tx".into(),
            output: Some(0),
        }],
    )
    .unwrap();
    assert_eq!(core.installation_validate(d).unwrap(), vec![]);

    // An S/PDIF follower of a source.
    let d = edit(
        draft(),
        vec![
            add_unit(
                "S-F",
                Some("main"),
                InstallationRole::SpdifFollower,
                Some("A"),
            ),
            InstallationEdit::SetSpdifSource {
                id: "A".into(),
                tx_mask: 1,
            },
        ],
    )
    .unwrap();
    assert_eq!(core.installation_validate(d.clone()).unwrap(), vec![]);
    assert!(
        core.installation_diff(d).unwrap()[0]
            .contains("role receiver → receiver and S/PDIF source")
    );
}

#[test]
fn wp_inst0_removing_an_element_forgets_its_references() {
    let core = DspiCore::new();
    let d = edit(
        draft(),
        vec![
            InstallationEdit::AddGroup {
                id: Some("g".into()),
                name: String::new(),
                units: vec!["A".into()],
            },
            InstallationEdit::RemoveElement { id: "main".into() },
        ],
    )
    .unwrap();
    let m = d.model_parsed().unwrap();
    assert!(m.rooms.is_empty() && m.unit("A").unwrap().room.is_none());
    assert_eq!(m.scenes[0].actions.len(), 1, "the room's action goes");
    let d = edit(d, vec![InstallationEdit::RemoveElement { id: "A".into() }]).unwrap();
    let m = d.model_parsed().unwrap();
    assert!(m.units.is_empty() && m.groups[0].units.is_empty() && m.scenes[0].actions.is_empty());
    assert_eq!(core.installation_validate(d.clone()).unwrap(), vec![]);
    assert_eq!(
        core.installation_diff(d).unwrap(),
        [
            "Removes room Main room",
            "Adds group g",
            "Removes unit Left",
            "Scene Quiet: actions 2 → 0",
        ]
    );
    // A removed sender leaves the unit it fed for the validator to name.
    let d = edit(
        draft(),
        vec![InstallationEdit::RemoveElement { id: "tx".into() }],
    )
    .unwrap();
    let issues = core.installation_validate(d).unwrap();
    assert_eq!(issues[0].subject, "A", "{issues:?}");
}

#[test]
fn wp_inst0_local_validation_names_the_element() {
    let core = DspiCore::new();
    let mut m = site();
    m.units[0].upstream = "nowhere".into();
    let d = InstallationDraft::new(1, &m).unwrap();
    let i = core.installation_validate(d).unwrap();
    assert_eq!(i[0].subject, "A");
    let mut m = site();
    m.rate_hz = 1;
    let i = core
        .installation_validate(InstallationDraft::new(1, &m).unwrap())
        .unwrap();
    assert_eq!(
        (i[0].subject.as_str(), i[0].message.as_str()),
        ("", "rateHz: 44100, 48000 or 96000")
    );
}

#[test]
fn wp_inst0_the_draft_round_trips_as_one_json_object_and_by_serde() {
    let d = edit(
        draft(),
        vec![InstallationEdit::SetSystem {
            rate_hz: Some(96_000),
            set_d: true,
            d_frames: Some(120),
            keep_usb_aligned: Some(true),
        }],
    )
    .unwrap();
    let v = d.to_json();
    assert_eq!(v["revision"], 7);
    assert_eq!(v["model"]["rateHz"], 96_000);
    assert_eq!(v["base"]["rateHz"], 48_000);
    assert_eq!(InstallationDraft::from_json(&v).unwrap(), d);
    assert!(
        InstallationDraft::from_json(&json!({"revision": 1, "base": {}, "model": {"x": 1}}))
            .is_err()
    );
    assert_eq!(
        DspiCore::new().installation_diff(d).unwrap(),
        [
            "Installation: sample rate 48000 Hz → 96000 Hz; system delay automatic → 120 frames; keep USB-direct units aligned off → on"
        ]
    );
    // The edits' JSON, as the C ABI and the web send them.
    let e: Vec<InstallationEdit> = serde_json::from_value(json!([
        {"op": "addUnit", "serial": "S-Q", "role": "usb_direct", "room": "main"},
        {"op": "setUnitSpeakers", "id": "S-Q", "speakers": [{"output": 0, "distanceM": 2.5}]},
        {"op": "setSystem", "setD": true}
    ]))
    .unwrap();
    assert!(
        matches!(&e[1], InstallationEdit::SetUnitSpeakers { speakers, .. } if speakers[0].distance_m == 2.5)
    );
    assert!(
        serde_json::from_value::<InstallationEdit>(json!({"op": "addRoom", "colour": 1})).is_err()
    );
    let d = edit(draft(), e).unwrap();
    assert_eq!(DspiCore::new().installation_validate(d).unwrap(), vec![]);
}

#[test]
fn wp_inst0_speakers_on_outputs_that_stay_keep_their_measurement() {
    let mut m = site();
    m.units[0].speakers = vec![
        Speaker {
            output: 0,
            distance_m: 3.0,
            measured_arrival_us: Some(-12.0),
            ..Speaker::default()
        },
        Speaker {
            output: 1,
            distance_m: 3.0,
            ..Speaker::default()
        },
    ];
    let d = edit(
        InstallationDraft::new(1, &m).unwrap(),
        vec![InstallationEdit::SetUnitSpeakers {
            id: "A".into(),
            speakers: vec![InstallationSpeaker {
                output: 0,
                distance_m: 4.0,
            }],
        }],
    )
    .unwrap();
    let s = &d.model_parsed().unwrap().units[0].speakers;
    assert_eq!(s.len(), 1);
    assert_eq!(
        (s[0].distance_m, s[0].measured_arrival_us),
        (4.0, Some(-12.0))
    );
    assert_eq!(
        DspiCore::new().installation_diff(d).unwrap(),
        ["Unit Left: speakers 2 → 1"]
    );
}
