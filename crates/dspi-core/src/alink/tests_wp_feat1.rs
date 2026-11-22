//! WP FEAT1: the installation's programme sources, a unit's slots, the
//! audio link sources, a zone's source that routes (its units' slots), and
//! scenes with a `source` op, as typed edits on a draft: validated, refused
//! on a clash, all or none, in the diff's words, and round tripped. The hub
//! end to end is dspi-hub's `tests/wp_sw2_system.rs` (`wp_feat1_*`).

use serde_json::json;

use super::*;
use crate::DspiCore;

/// A sender on two rooms; A and B in the lounge (B a repeater), C in the
/// bar, a USB-direct unit in the lounge, a second sender with D in the
/// office; a stereo source on channels 0 and 1.
fn site() -> SystemModel {
    serde_json::from_value(json!({
        "rooms": [
            {"id": "lounge", "name": "Lounge"},
            {"id": "bar", "name": "Bar"},
            {"id": "office", "name": "Office"}
        ],
        "zones": [
            {"id": "down", "name": "Downstairs", "rooms": ["lounge"]},
            {"id": "bars", "name": "Bars", "rooms": ["bar"]}
        ],
        "senders": [
            {"id": "tx", "name": "Sender", "serial": "S-TX"},
            {"id": "tx2", "name": "Office sender", "serial": "S-TX2"}
        ],
        "sources": [{"id": "radio", "name": "Radio", "sender": "tx", "channels": [0, 1]}],
        "units": [
            {"id": "A", "name": "Lounge left", "serial": "S-A", "room": "lounge", "upstream": "tx"},
            {"id": "B", "name": "Lounge right", "serial": "S-B", "room": "lounge", "upstream": "tx", "repeater": true},
            {"id": "C", "name": "Bar", "serial": "S-C", "room": "bar", "upstream": "B"},
            {"id": "U", "name": "Lounge sub", "serial": "S-U", "room": "lounge", "transport": "usb_direct"},
            {"id": "D", "name": "Office", "serial": "S-D", "room": "office", "upstream": "tx2"}
        ]
    }))
    .unwrap()
}

fn draft() -> InstallationDraft {
    InstallationDraft::new(3, &site()).unwrap()
}

/// `edits` as JSON (the agents' and the web's shape) made on `d`.
fn edit(d: InstallationDraft, edits: serde_json::Value) -> crate::Result<InstallationDraft> {
    let ops: Vec<InstallationEdit> = serde_json::from_value(edits).unwrap();
    DspiCore::new().installation_draft_edit(d, ops)
}

fn refused(d: InstallationDraft, edits: serde_json::Value) -> String {
    match edit(d, edits) {
        Err(DspiError::InvalidArgument { msg }) => msg,
        other => panic!("not refused: {other:?}"),
    }
}

fn changes(d: &InstallationDraft) -> Vec<String> {
    DspiCore::new().installation_diff(d.clone()).unwrap()
}

fn valid(d: &InstallationDraft) -> bool {
    DspiCore::new()
        .installation_validate(d.clone())
        .unwrap()
        .is_empty()
}

const S: u8 = ALINK_SLOT_SILENT;

#[test]
fn wp_feat1_a_programme_source_is_described_validated_and_in_the_diff() {
    let d = edit(
        draft(),
        json!([
            {"op": "addProgrammeSource", "name": "Plex on the HTPC", "kind": "computer_usb",
             "feeds": "A", "inputs": [0, 1]},
            {"op": "addProgrammeSource", "id": "dj", "name": "DJ booth", "kind": "alink_sender",
             "feeds": "tx", "inputs": [6, 7, 40]}
        ]),
    )
    .unwrap();
    let m = d.model_parsed().unwrap();
    assert_eq!(m.programme_sources.len(), 2);
    let p = &m.programme_sources[0];
    assert_eq!(
        (p.id.as_str(), p.kind, p.feeds.as_str(), &p.inputs[..]),
        (
            "Plex on the HTPC",
            ProgrammeKind::ComputerUsb,
            "A",
            &[0u8, 1][..]
        )
    );
    assert!(valid(&d));
    assert_eq!(
        changes(&d),
        [
            "Adds programme source Plex on the HTPC",
            "Adds programme source DJ booth"
        ]
    );
    // It routes nothing: no unit's slots moved.
    assert!(m.units.iter().all(|u| u.slots.is_none()));

    // Kind, what it feeds and inputs, replaced; renamed; in words.
    let d2 = edit(
        d.clone(),
        json!([
            {"op": "setProgrammeSource", "id": "dj", "kind": "spdif", "feeds": "C", "inputs": [2, 3]},
            {"op": "renameElement", "id": "dj", "name": "Turntables"}
        ]),
    )
    .unwrap();
    let d2 = InstallationDraft {
        base: d.model.clone(),
        ..d2
    };
    assert_eq!(
        changes(&d2),
        [
            "Programme source Turntables: name \"DJ booth\" → \"Turntables\"; kind audio link sender → \
          S/PDIF; feeds Sender → Bar; inputs 6 7 40 → 2 3"
        ]
    );

    // What it feeds must be of its kind; its inputs the element's, once.
    let e = refused(
        draft(),
        json!([{"op": "addProgrammeSource", "name": "TV", "kind": "spdif", "feeds": "tx"}]),
    );
    assert!(e.contains("edit 1") && e.contains("not a unit"), "{e}");
    let e = refused(
        draft(),
        json!([{"op": "addProgrammeSource", "name": "Box", "kind": "alink_sender", "feeds": "A"}]),
    );
    assert!(e.contains("not a sender"), "{e}");
    for inputs in [json!([8]), json!([1, 1])] {
        let e = refused(
            draft(),
            json!([{"op": "addProgrammeSource", "name": "Mixer", "kind": "analogue",
                    "feeds": "A", "inputs": inputs}]),
        );
        assert!(e.contains("inputs below 8, each once"), "{e}");
    }
    let e = refused(
        draft(),
        json!([{"op": "addProgrammeSource", "name": "PC", "kind": "alink_sender",
                "feeds": "tx", "inputs": [64]}]),
    );
    assert!(e.contains("inputs below 64"), "{e}");
    // A kind the core does not know is not an edit.
    let bad: std::result::Result<Vec<InstallationEdit>, _> = serde_json::from_value(json!([
        {"op": "addProgrammeSource", "name": "X", "kind": "bluetooth", "feeds": "A"}
    ]));
    assert!(bad.is_err());

    // Removed with removeElement; the unit it fed removed: the validator says so.
    let d3 = edit(d.clone(), json!([{"op": "removeElement", "id": "dj"}])).unwrap();
    assert_eq!(d3.model_parsed().unwrap().programme_sources.len(), 1);
    let d4 = edit(d, json!([{"op": "removeElement", "id": "A"}])).unwrap();
    let found = DspiCore::new().installation_validate(d4).unwrap();
    assert_eq!(found[0].subject, "Plex on the HTPC", "{found:?}");
    assert!(found[0].message.contains("not a unit"), "{found:?}");
}

#[test]
fn wp_feat1_a_zones_source_routes_its_units_slots() {
    let d = edit(
        draft(),
        json!([{"op": "setZoneSource", "id": "down", "source": "radio"}]),
    )
    .unwrap();
    let m = d.model_parsed().unwrap();
    assert_eq!(m.zones[0].source.as_deref(), Some("radio"));
    // Both lounge units on the audio link take the source's channels.
    let routed = vec![0, 1, S, S, S, S, S, S];
    assert_eq!(m.unit("A").unwrap().slots.as_ref(), Some(&routed));
    assert_eq!(m.unit("B").unwrap().slots.as_ref(), Some(&routed));
    // The USB-direct unit takes no slots; the bar is another zone.
    assert!(m.unit("U").unwrap().slots.is_none());
    assert!(m.unit("C").unwrap().slots.is_none());
    assert!(valid(&d));
    assert_eq!(
        changes(&d),
        [
            "Zone Downstairs: source none → Radio",
            "Unit Lounge left: slots the unit's own → 0 1 - - - - - -",
            "Unit Lounge right: slots the unit's own → 0 1 - - - - - -",
        ]
    );

    // A unit moved into the zone's room is routed; the source's channels
    // changed: every unit of the zones playing it follows.
    let d = edit(
        d,
        json!([
            {"op": "setUnitRoom", "id": "C", "room": "lounge"},
            {"op": "setSource", "id": "radio", "sender": "tx", "channels": [4, 5]}
        ]),
    )
    .unwrap();
    let m = d.model_parsed().unwrap();
    let moved = vec![4, 5, S, S, S, S, S, S];
    for u in ["A", "B", "C"] {
        assert_eq!(m.unit(u).unwrap().slots.as_ref(), Some(&moved), "{u}");
    }
    // A unit added to the room is routed too.
    let d = edit(
        d,
        json!([{"op": "addUnit", "serial": "S-E", "name": "Lounge back", "room": "lounge",
                "role": "receiver", "upstream": "tx"}]),
    )
    .unwrap();
    let m = d.model_parsed().unwrap();
    assert_eq!(m.unit("Lounge back").unwrap().slots.as_ref(), Some(&moved));

    // No source: the zone names none and the slots stay as they are.
    let d = edit(d, json!([{"op": "setZoneSource", "id": "down"}])).unwrap();
    let m = d.model_parsed().unwrap();
    assert!(m.zones[0].source.is_none());
    assert_eq!(m.unit("A").unwrap().slots.as_ref(), Some(&moved));
}

#[test]
fn wp_feat1_a_source_that_cannot_reach_a_zone_is_refused() {
    // The office's unit hears the other sender.
    let d = edit(
        draft(),
        json!([{"op": "setZoneRooms", "id": "bars", "rooms": ["bar", "office"]}]),
    )
    .unwrap();
    let e = refused(
        d,
        json!([{"op": "setZoneSource", "id": "bars", "source": "radio"}]),
    );
    assert!(
        e.contains("edit 1") && e.contains("hears sender \"tx2\""),
        "{e}"
    );
    // More channels than a unit takes; no channels at all.
    let e = refused(
        draft(),
        json!([
            {"op": "addSource", "id": "film", "name": "Film", "sender": "tx", "channels": [10, 11, 12, 13, 14, 15, 16, 17, 18]},
            {"op": "setZoneSource", "id": "down", "source": "film"}
        ]),
    );
    assert!(e.contains("edit 2") && e.contains("a unit takes 8"), "{e}");
    let e = refused(
        draft(),
        json!([
            {"op": "addSource", "id": "none", "name": "Nothing", "sender": "tx"},
            {"op": "setZoneSource", "id": "down", "source": "none"}
        ]),
    );
    assert!(e.contains("no channels"), "{e}");
    let e = refused(
        draft(),
        json!([{"op": "setZoneSource", "id": "down", "source": "tv"}]),
    );
    assert!(e.contains("no source \"tv\""), "{e}");
}

#[test]
fn wp_feat1_slot_and_channel_clashes_are_refused_all_or_none() {
    // A unit's own slots: 8, each below 64 or 255; on the audio link only.
    let d = edit(
        draft(),
        json!([{"op": "setUnitSlots", "id": "C", "slots": [2, 3, 255, 255, 255, 255, 255, 63]}]),
    )
    .unwrap();
    let m = d.model_parsed().unwrap();
    assert_eq!(
        m.unit("C").unwrap().slots.as_ref(),
        Some(&vec![2, 3, S, S, S, S, S, 63])
    );
    assert_eq!(
        changes(&d),
        ["Unit Bar: slots the unit's own → 2 3 - - - - - 63"]
    );
    for slots in [json!([0, 1]), json!([0, 1, 2, 3, 4, 5, 6, 64])] {
        let e = refused(
            draft(),
            json!([{"op": "setUnitSlots", "id": "C", "slots": slots}]),
        );
        assert!(
            e.contains("slots: 8, each a channel below 64 or 255"),
            "{e}"
        );
    }
    let e = refused(
        draft(),
        json!([{"op": "setUnitSlots", "id": "U", "slots": [0, 1, 255, 255, 255, 255, 255, 255]}]),
    );
    assert!(e.contains("not on the audio link"), "{e}");
    // Back to the unit's own table.
    let d = edit(d, json!([{"op": "setUnitSlots", "id": "C"}])).unwrap();
    assert!(d.model_parsed().unwrap().unit("C").unwrap().slots.is_none());

    // A slot clash: a unit whose zone plays a source takes its slots only.
    let routed = edit(
        draft(),
        json!([{"op": "setZoneSource", "id": "down", "source": "radio"}]),
    )
    .unwrap();
    let e = refused(
        routed.clone(),
        json!([{"op": "setUnitSlots", "id": "A", "slots": [5, 6, 255, 255, 255, 255, 255, 255]}]),
    );
    assert!(
        e.starts_with("edit 1: slot clash") && e.contains("zone \"down\"") && e.contains("0 1 - -"),
        "{e}"
    );
    let e = refused(routed.clone(), json!([{"op": "setUnitSlots", "id": "A"}]));
    assert!(e.contains("slot clash"), "{e}");
    // The slots the source gives are no clash.
    edit(
        routed.clone(),
        json!([{"op": "setUnitSlots", "id": "A", "slots": [0, 1, 255, 255, 255, 255, 255, 255]}]),
    )
    .unwrap();

    // A slot clash: one room in two zones playing different sources.
    let e = refused(
        routed.clone(),
        json!([
            {"op": "addSource", "id": "tv", "name": "TV", "sender": "tx", "channels": [2, 3]},
            {"op": "setZoneSource", "id": "bars", "source": "tv"},
            {"op": "setZoneRooms", "id": "bars", "rooms": ["bar", "lounge"]}
        ]),
    );
    assert!(
        e.starts_with("edit 3: slot clash") && e.contains("different sources"),
        "{e}"
    );

    // A channel clash: two sources of one sender on one channel.
    let e = refused(
        draft(),
        json!([{"op": "addSource", "name": "TV", "sender": "tx", "channels": [1, 2]}]),
    );
    assert!(
        e.contains("channel clash: channel 1 is source \"radio\"'s"),
        "{e}"
    );
    // On another sender, or the source itself again, it is not.
    edit(
        draft(),
        json!([
            {"op": "addSource", "name": "Office radio", "sender": "tx2", "channels": [0, 1]},
            {"op": "setSource", "id": "radio", "sender": "tx", "channels": [1, 0]}
        ]),
    )
    .unwrap();
    for channels in [json!([64]), json!([3, 3])] {
        let e = refused(
            draft(),
            json!([{"op": "addSource", "name": "X", "sender": "tx", "channels": channels}]),
        );
        assert!(e.contains("each below 64, once"), "{e}");
    }
    let e = refused(
        draft(),
        json!([{"op": "addSource", "name": "X", "sender": "nope", "channels": [9]}]),
    );
    assert!(e.contains("no sender \"nope\""), "{e}");

    // All or none: the draft refused above is the one given.
    let before = routed.clone();
    let _ = refused(
        routed.clone(),
        json!([
            {"op": "addRoom", "name": "Porch"},
            {"op": "setUnitSlots", "id": "A", "slots": [9, 9, 9, 9, 9, 9, 9, 9]}
        ]),
    );
    assert_eq!(routed, before);
}

#[test]
fn wp_feat1_a_scene_switches_a_zones_source_and_is_checked_as_the_hub_does() {
    let d = edit(
        draft(),
        json!([
            {"op": "addSource", "id": "tv", "name": "TV", "sender": "tx", "channels": [2, 3]},
            {"op": "addScene", "id": "movie", "name": "Movie night", "systemMute": false, "actions": [
                {"target": "zone:down", "op": "source", "source": "tv"},
                {"target": "room:lounge", "op": "master_volume", "value": -2000}
            ]}
        ]),
    )
    .unwrap();
    let m = d.model_parsed().unwrap();
    let s = m.scene("movie").unwrap();
    assert_eq!(s.actions[0].source.as_deref(), Some("tv"));
    assert_eq!(s.system_mute, Some(false));
    assert!(valid(&d));
    // Stored only: the zone's source and the slots move when it is recalled.
    assert!(m.zones[0].source.is_none());
    assert!(m.unit("A").unwrap().slots.is_none());
    assert_eq!(changes(&d), ["Adds source TV", "Adds scene Movie night"]);

    // Replaced; the change in words.
    let d2 = edit(
        d.clone(),
        json!([{"op": "setScene", "id": "movie", "actions": [
            {"target": "zone:down", "op": "source", "source": "radio"},
            {"target": "room:lounge", "op": "master_volume", "value": -2000}
        ]}]),
    )
    .unwrap();
    let d2 = InstallationDraft {
        base: d.model.clone(),
        ..d2
    };
    assert_eq!(
        changes(&d2),
        ["Scene Movie night: actions changed"],
        "the system mute left out is none: {:?}",
        d2.model_parsed().unwrap().scene("movie")
    );

    // What the hub would refuse is refused here.
    for (actions, words) in [
        (
            json!([{"target": "room:lounge", "op": "source", "source": "tv"}]),
            "targets a zone",
        ),
        (
            json!([{"target": "zone:down", "op": "source"}]),
            "names its source",
        ),
        (
            json!([{"target": "zone:down", "op": "source", "source": "tv", "arg": 1}]),
            "no arg or value",
        ),
        (
            json!([{"target": "zone:down", "op": "mute", "source": "tv"}]),
            "only a source op names a source",
        ),
        (
            json!([{"target": "zone:nowhere", "op": "source", "source": "tv"}]),
            "no zone \"nowhere\"",
        ),
        (json!([{"target": "all", "op": "flash"}]), "op \"flash\""),
        (
            json!([{"target": "all", "op": "master_volume", "value": 2000}]),
            "master_volume with arg 0 and value 2000",
        ),
    ] {
        let e = refused(
            d.clone(),
            json!([{"op": "addScene", "name": "Bad", "actions": actions}]),
        );
        assert!(e.contains(words), "{words}: {e}");
    }
    let e = refused(
        draft(),
        json!([{"op": "setScene", "id": "nope", "actions": []}]),
    );
    assert!(e.contains("no scene \"nope\""), "{e}");

    // A source removed takes its scene ops with it; the model stays valid.
    let d3 = edit(d.clone(), json!([{"op": "removeElement", "id": "tv"}])).unwrap();
    let m3 = d3.model_parsed().unwrap();
    assert_eq!(m3.scene("movie").unwrap().actions.len(), 1);
    assert!(valid(&d3));

    // The hub's validator checks a scene's routing too: the zone moved to
    // a room the source cannot reach makes the model invalid.
    let mut m4 = m.clone();
    m4.zones[0].rooms = vec!["office".into()];
    let e = m4.validate().unwrap_err();
    assert!(
        e.0.starts_with("scene movie:") && e.0.contains("tx2"),
        "{e}"
    );
    assert_eq!(e.subject().as_deref(), Some("movie"));
}

#[test]
fn wp_feat1_the_new_fields_round_trip_and_stay_out_of_an_old_model() {
    // An installation without them encodes as before.
    let plain = InstallationDraft::new(1, &site()).unwrap();
    assert!(!plain.model.contains("programmeSources"), "{}", plain.model);

    let d = edit(
        draft(),
        json!([
            {"op": "addProgrammeSource", "id": "plex", "name": "Plex", "kind": "computer_usb", "feeds": "A", "inputs": [0, 1]},
            {"op": "setZoneSource", "id": "down", "source": "radio"},
            {"op": "addScene", "id": "radio-on", "name": "Radio", "actions": [
                {"target": "zone:bars", "op": "source", "source": "radio"},
                {"target": "zone:bars", "op": "unmute"}
            ]}
        ]),
    )
    .unwrap();
    // The C ABI's and the web's shape, back.
    let back = InstallationDraft::from_json(&d.to_json()).unwrap();
    assert_eq!(back, d);
    let j = d.to_json();
    assert_eq!(
        j["model"]["programmeSources"][0],
        json!({"id": "plex", "name": "Plex", "kind": "computer_usb", "feeds": "A", "inputs": [0, 1]})
    );
    assert_eq!(
        j["model"]["scenes"][0]["actions"][0],
        json!({"target": "zone:bars", "op": "source", "arg": 0, "value": 0, "source": "radio"})
    );
    // Every other op leaves the field out.
    assert_eq!(
        j["model"]["scenes"][0]["actions"][1],
        json!({"target": "zone:bars", "op": "unmute", "arg": 0, "value": 0})
    );
    // By serde too, and the model as the hub reads it.
    let again: InstallationDraft =
        serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
    assert_eq!(again, d);
    let m: SystemModel = serde_json::from_str(&d.model).unwrap();
    assert!(m.validate().is_ok());
    // The edits themselves round trip (what the agents' server keeps).
    let ops: Vec<InstallationEdit> = serde_json::from_value(json!([
        {"op": "setUnitSlots", "id": "A", "slots": [0, 1, 255, 255, 255, 255, 255, 255]},
        {"op": "setProgrammeSource", "id": "plex", "kind": "analogue", "feeds": "A"}
    ]))
    .unwrap();
    let text = serde_json::to_string(&ops).unwrap();
    assert_eq!(
        serde_json::from_str::<Vec<InstallationEdit>>(&text).unwrap(),
        ops
    );
    // An unknown field is refused, not dropped.
    assert!(
        serde_json::from_value::<InstallationEdit>(json!(
            {"op": "setZoneSource", "id": "down", "source": "radio", "live": true}
        ))
        .is_err()
    );
}
