//! WP LKUI1: the System screen's rows from the hub's `system.*` answers
//! (shapes as dspi-hub's serde writes them). The end-to-end test against a
//! real hub is dspi-hub's `tests/wp_lkui1_system_view.rs`.

use serde_json::{Value, json};

use super::alink_system::{align_outcome, apply_edits, build_view, recall_outcome};
use super::{AlinkRoomLipSync, AlinkSystemEdits, AlinkUnitLatency};
use crate::{DspiCore, DspiError};

fn model() -> Value {
    json!({
        "rooms": [{"id": "main", "name": "Main room"}, {"id": "bar", "name": "Bar", "lipSyncUs": 500}],
        "zones": [], "groups": [], "sources": [],
        "senders": [{"id": "tx", "name": "Sender", "serial": "TX"}],
        "fanOuts": [{"id": "fan", "name": "Fan-out", "serial": "FAN", "upstream": "tx"}],
        "units": [
            {"id": "A", "name": "Main left", "serial": "A", "room": "main", "upstream": "tx"},
            {"id": "B", "name": "", "serial": "B", "room": "main", "upstream": "fan", "measuredLatencyUs": 120},
            {"id": "C", "name": "Bar", "serial": "C", "room": "bar", "upstream": "fan"}
        ],
        "scenes": [
            {"id": "evening", "name": "Evening", "actions": []},
            {"id": "closed", "name": "Closed", "systemMute": true, "actions": []}
        ]
    })
}

fn status() -> Value {
    json!({
        "revision": 4,
        "role": "admin",
        "observed": {
            "TX": {"online": true, "role": "sender", "systemMute": false, "pendingApplyFrame": 9000,
                   "superframePath": true, "stateEvents": true},
            "FAN": {"online": true, "role": "fan_out", "state": "locked", "hopDelayFrames": 3,
                    "superframePath": false, "stateEvents": true},
            "A": {"online": true, "role": "receiver", "state": "locked", "dLocalMin": 64, "dLocalMax": 400,
                  "phaseErrorFrames": 0.25, "hops": 0, "phase": [1, 120, 0, 0],
                  "superframePath": false, "stateEvents": true},
            "B": {"online": true, "role": "receiver", "state": "holdover", "dLocalMin": 64, "dLocalMax": 400,
                  "hops": 1, "phase": [1, 128, 0, 0], "superframePath": false, "stateEvents": true},
            "C": {"online": false, "superframePath": false, "stateEvents": false}
        },
        "alignment": {
            "dFrames": 120, "dAuto": true, "rateHz": 48000, "latencyMaxFrames": 0,
            "units": [
                {"unit": "A", "serial": "A", "hops": 0, "pathDelayFrames": 0, "latencyFrames": 0,
                 "lipSyncFrames": 0, "effective": 120, "dLocal": 120, "hopsOffset": 0, "outDelays": [], "writable": true},
                {"unit": "B", "serial": "B", "hops": 1, "pathDelayFrames": 3, "latencyFrames": 6,
                 "lipSyncFrames": 0, "effective": 117, "dLocal": 117, "hopsOffset": 3, "outDelays": [], "writable": true}
            ],
            "issues": [{"subject": "C", "code": "offline", "message": "Bar is not connected"}]
        },
        "alarms": [
            {"id": 1, "code": "alink_lost", "subject": "B", "severity": "warning", "message": "Link lost",
             "state": "raised", "raisedAt": 1, "count": 1},
            {"id": 2, "code": "x", "subject": "C", "severity": "critical", "message": "Gone",
             "state": "acknowledged", "raisedAt": 2, "count": 1},
            {"id": 3, "code": "y", "subject": "A", "severity": "warning", "message": "Old",
             "state": "cleared", "raisedAt": 0, "count": 1}
        ]
    })
}

#[test]
fn wp_lkui1_the_view_has_every_element_with_its_link_state() {
    let v = build_view(&model(), 4, &status(), "admin");
    assert!(v.installed && v.may_edit && v.may_recall);
    assert_eq!(v.revision, 4);
    let ids: Vec<&str> = v.units.iter().map(|u| u.id.as_str()).collect();
    assert_eq!(ids, ["tx", "fan", "A", "B", "C"]);
    let by = |id: &str| v.units.iter().find(|u| u.id == id).unwrap();
    assert_eq!(
        (by("tx").role.as_str(), by("tx").state_word.as_str()),
        ("Sender", "Clock")
    );
    assert_eq!(
        (by("fan").role.as_str(), by("fan").hop_delay_frames),
        ("Fan-out", Some(3))
    );
    let a = by("A");
    assert_eq!(
        (a.role.as_str(), a.state_word.as_str(), a.room.as_str()),
        ("Receiver", "Locked", "Main room")
    );
    assert_eq!(a.alink_delay_frames, Some(120));
    assert!((a.alink_delay_ms.unwrap() - 2.5).abs() < 1e-9);
    assert_eq!(
        (a.alink_delay_min, a.alink_delay_max),
        (Some(64), Some(400))
    );
    assert_eq!(a.phase_error_frames, Some(0.25));
    assert!(!a.pending, "A has the planned delay");
    // B: the plan asks 117 + 3, the unit has 128 + 0: pending; a name falls back to the id.
    let b = by("B");
    assert_eq!((b.name.as_str(), b.state.as_str()), ("B", "holdover"));
    assert_eq!(
        (b.alink_delay_frames, b.unit_alink_delay_frames),
        (Some(117), Some(128))
    );
    assert_eq!((b.hop_offset, b.unit_hop_offset), (Some(3), Some(0)));
    assert!(b.pending);
    assert_eq!((b.measured_latency_us, b.alarms), (Some(120), 1));
    let c = by("C");
    assert_eq!(
        (c.connected, c.state_word.as_str(), c.pending),
        (false, "Not connected", false)
    );
    assert_eq!(
        (v.locked, v.holdover, v.pending, v.unit_count),
        (1, 1, 1, 3)
    );
    assert_eq!(
        v.summary,
        "Audio link: 1 of 3 locked, 1 in holdover, 1 not connected, audio link delay to write on 1"
    );
    assert_eq!(v.health, "problem");
    // Alarms still on, critical first, with the element's name.
    let alarms: Vec<(&str, &str)> = v
        .alarms
        .iter()
        .map(|a| (a.subject.as_str(), a.severity.as_str()))
        .collect();
    assert_eq!(alarms, [("Bar", "critical"), ("B", "warning")]);
    assert_eq!(
        (v.system_mute, v.pending_apply_frame),
        (Some(false), Some(9000))
    );
    assert_eq!((v.system_delay_frames, v.system_delay_auto), (120, true));
    assert_eq!(v.issues, ["Bar is not connected"]);
    assert_eq!(v.rooms[1].lip_sync_us, Some(500));
    assert_eq!(v.scenes[1].system_mute, Some(true));
}

#[test]
fn wp_lkui1_roles_decide_what_is_offered() {
    let control = build_view(&model(), 4, &status(), "control");
    assert!(!control.may_edit && control.may_recall);
    assert_eq!(
        control.edit_note,
        "Changing the installation needs the hub's Admin role."
    );
    let viewer = build_view(&model(), 4, &status(), "viewer");
    assert!(!viewer.may_edit && !viewer.may_recall);
    assert_eq!(
        viewer.recall_note,
        "Recalling a scene needs the hub's Control role."
    );
}

#[test]
fn wp_lkui1_a_hub_without_an_installation_says_so() {
    let empty = json!({"rooms": [], "zones": [], "groups": [], "sources": [], "senders": [],
                       "fanOuts": [], "units": [], "scenes": []});
    let v = build_view(
        &empty,
        0,
        &json!({"revision": 0, "observed": {}, "alarms": []}),
        "admin",
    );
    assert!(!v.installed);
    assert_eq!(
        (v.summary.as_str(), v.health.as_str()),
        ("No audio link installation on this hub", "none")
    );
    assert_eq!(v.rate_hz, 48_000);
    // A healthy installation is ok.
    let mut s = status();
    s["observed"]["B"]["state"] = json!("locked");
    s["observed"]["B"]["phase"] = json!([1, 117, 3, 0]);
    s["observed"]["C"] = json!({"online": true, "state": "locked", "phase": [1, 100, 0, 0]});
    let v = build_view(&model(), 4, &s, "admin");
    assert_eq!(
        (v.summary.as_str(), v.health.as_str()),
        ("Audio link: 3 of 3 locked", "ok")
    );
}

#[test]
fn wp_lkui1_edits_change_only_what_was_edited() {
    let mut m = model();
    let edits = AlinkSystemEdits {
        set_system_delay: true,
        system_delay_frames: Some(200),
        rooms: vec![AlinkRoomLipSync {
            room_id: "bar".into(),
            lip_sync_us: None,
        }],
        units: vec![AlinkUnitLatency {
            unit_id: "A".into(),
            measured_latency_us: Some(80),
        }],
    };
    apply_edits(&mut m, &edits).unwrap();
    assert_eq!(m["dFrames"], 200);
    assert_eq!(m["rooms"][1]["lipSyncUs"], Value::Null);
    assert_eq!(m["units"][0]["measuredLatencyUs"], 80);
    assert_eq!(m["units"][1]["measuredLatencyUs"], 120, "B untouched");
    // Back to automatic.
    let auto = AlinkSystemEdits {
        set_system_delay: true,
        ..AlinkSystemEdits::default()
    };
    apply_edits(&mut m, &auto).unwrap();
    assert_eq!(m["dFrames"], Value::Null);
    // Not setting it leaves it.
    let mut m2 = model();
    m2["dFrames"] = json!(150);
    apply_edits(&mut m2, &AlinkSystemEdits::default()).unwrap();
    assert_eq!(m2["dFrames"], 150);
    // An id the model lacks: refused, nothing half done.
    let mut m3 = model();
    let bad = AlinkSystemEdits {
        units: vec![AlinkUnitLatency {
            unit_id: "Z".into(),
            measured_latency_us: Some(1),
        }],
        ..AlinkSystemEdits::default()
    };
    assert!(matches!(
        apply_edits(&mut m3, &bad),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert!(apply_edits(&mut Value::Null, &AlinkSystemEdits::default()).is_err());
}

#[test]
fn wp_lkui1_a_recall_through_the_sender_and_its_refusal() {
    let report = json!({
        "plan": {"scene": "evening", "frame": 10_000, "path": "superframe", "ops": [], "leadFrames": 4800},
        "writes": [{"serial": "TX", "element": "tx", "what": "superframe", "result": "queued"}],
        "frameAfterWrites": 5300, "onTime": true
    });
    let r = recall_outcome("evening", &report, &model(), None, 48_000);
    assert!(r.taken && r.on_time);
    assert_eq!(
        r.message,
        "Evening recalled through the sender, at audio link frame 10000."
    );
    assert_eq!((r.lead_ms, r.frame), (100.0, 10_000));
    assert_eq!(r.writes[0].unit, "Sender");
    assert_eq!(
        (r.writes[0].result_word.as_str(), r.writes[0].tone.as_str()),
        ("Queued", "ok")
    );

    // The next one before that frame: every write refused; the words name the held frame.
    let refused = json!({
        "plan": {"scene": "closed", "frame": 10_500, "path": "superframe", "ops": [], "leadFrames": 4800,
                 "systemMute": true},
        "writes": [{"serial": "TX", "element": "tx", "what": "superframe", "result": "refused"}],
        "frameAfterWrites": 5700, "onTime": false
    });
    let r = recall_outcome("closed", &refused, &model(), Some(10_000), 48_000);
    assert!(!r.taken);
    assert_eq!(r.held_frame, Some(10_000));
    assert_eq!(
        r.message,
        "Closed was refused: a scene is already scheduled on the sender for audio link frame 10000, \
         90 ms after this recall. The sender takes one scene at a time; recall Closed again after that. \
         System mute unchanged."
    );
    assert_eq!(
        (r.writes[0].result_word.as_str(), r.writes[0].tone.as_str()),
        ("Refused", "bad")
    );
    // -- fixup lk1 -- the sender's write carries the scene, never read as a refused mute
    assert_eq!(
        (r.writes[0].unit.as_str(), r.writes[0].what.as_str()),
        ("Sender", "Scene Closed")
    );
    // Without a frame to name.
    let r = recall_outcome("closed", &refused, &model(), None, 48_000);
    assert!(r.message.contains("still holds one") && r.held_frame.is_none());
    // Per unit, all refused.
    let mut per_unit = refused.clone();
    per_unit["plan"]["path"] = json!("per_unit");
    let r = recall_outcome("closed", &per_unit, &model(), Some(10_000), 48_000);
    assert_eq!(
        r.message,
        "Closed was refused by every unit; nothing changed. System mute unchanged."
    );
    // Taken with the system mute, a late unit.
    let mut late = refused.clone();
    late["writes"] = json!([{"serial": "A", "element": "A", "what": "mute", "result": "late"}]);
    let r = recall_outcome("closed", &late, &model(), None, 48_000);
    assert!(r.taken);
    assert_eq!(
        r.message,
        "Closed recalled through the sender, at audio link frame 10500, system mute on: not every unit on time."
    );
    assert_eq!(
        (r.writes[0].unit.as_str(), r.writes[0].what.as_str()),
        ("Main left", "Scene Closed")
    );
}

// -- fixup lk1 -- review lk1's probe kept: the held-scene words only for the sender's own
// refusal; a timeout, a lock or another answer has its own sentence and names no frame.
#[test]
fn wp_lkui1_fixup_lk1_a_sender_that_did_not_take_the_scene_for_another_reason() {
    let report = |result: &str| {
        json!({
            "plan": {"scene": "closed", "frame": 10_500, "path": "superframe", "ops": [], "leadFrames": 4800},
            "writes": [{"serial": "TX", "element": "tx", "what": "mute", "result": result}],
            "frameAfterWrites": 5700, "onTime": false
        })
    };
    let r = recall_outcome("closed", &report("timeout"), &model(), Some(10_000), 48_000);
    assert!(!r.taken);
    assert_eq!(r.held_frame, None);
    assert_eq!(
        r.message,
        "Closed was not recalled: the sender did not answer in time. Check the sender and recall Closed again."
    );
    let r = recall_outcome("closed", &report("locked"), &model(), Some(10_000), 48_000);
    assert_eq!(
        r.message,
        "Closed was not recalled: another console is applying changes to the sender. Recall Closed again when it is done."
    );
    let r = recall_outcome("closed", &report("no_device"), &model(), None, 48_000);
    assert_eq!(
        r.message,
        "Closed was not recalled: the sender's answer was \"Not connected\"."
    );
    for result in ["timeout", "locked", "no_device", "denied"] {
        let r = recall_outcome("closed", &report(result), &model(), Some(10_000), 48_000);
        assert!(
            !r.message.contains("already scheduled") && !r.message.contains("holds one"),
            "{result}: {}",
            r.message
        );
        assert_eq!(r.writes[0].what, "Scene Closed");
    }
}

#[test]
fn wp_lkui1_writing_the_link_delays() {
    let report = json!({"applied": true, "alignment": {"issues": []}, "writes": [
        {"serial": "A", "element": "A", "what": "phase", "result": "ok"},
        {"serial": "B", "element": "B", "what": "out_delay:1", "result": "unchanged"}
    ]});
    let a = align_outcome(&report, &model());
    assert!(a.applied);
    assert_eq!(
        a.message,
        "The audio link delays were written. Each unit re-anchored with a short silence."
    );
    assert_eq!(
        (a.writes[0].what.as_str(), a.writes[1].what.as_str()),
        ("Audio link delay", "Delay OUT2")
    );
    let stopped = json!({"applied": false, "writes": [],
                         "alignment": {"issues": [{"code": "range", "message": "D is out of B's range"}]}});
    assert_eq!(
        align_outcome(&stopped, &model()).message,
        "The audio link delays were not written: D is out of B's range."
    );
}

#[test]
fn wp_lkui1_without_a_hub_session_the_calls_say_disconnected() {
    let core = DspiCore::new();
    assert!(matches!(
        core.alink_system_view(),
        Err(DspiError::Disconnected)
    ));
    assert!(matches!(
        core.alink_system_save(0, AlinkSystemEdits::default()),
        Err(DspiError::Disconnected)
    ));
    assert!(matches!(
        core.alink_system_recall("x".into()),
        Err(DspiError::Disconnected)
    ));
    assert!(matches!(
        core.alink_system_align(),
        Err(DspiError::Disconnected)
    ));
}
