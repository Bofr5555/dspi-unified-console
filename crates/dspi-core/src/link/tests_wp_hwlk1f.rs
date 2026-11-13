//! WP HWLK1F: the System view and outcomes with a unit whose firmware has no
//! audio link (the hub's `observed.alink: "none"`), from the hub's answers.
//! The end-to-end test on a real hub is dspi-hub's `tests/wp_hwlk1f_mixed.rs`.

use serde_json::{Value, json};

use super::system::{align_outcome, build_view, recall_outcome};

fn model() -> Value {
    json!({
        "senders": [{"id": "tx", "name": "Sender", "serial": "TX"}],
        "units": [
            {"id": "A", "name": "Main left", "serial": "A", "upstream": "tx"},
            {"id": "K", "name": "K340", "serial": "K", "upstream": "tx"},
            {"id": "D", "name": "Bar", "serial": "D", "upstream": "tx"}
        ],
        "scenes": [{"id": "evening", "name": "Evening", "actions": []}]
    })
}

fn status(k: Value) -> Value {
    json!({
        "revision": 1,
        "observed": {
            "TX": {"online": true, "role": "sender", "alink": "present"},
            "A": {"online": true, "role": "receiver", "state": "locked", "alink": "present",
                  "dLocalMin": 64, "dLocalMax": 400, "phase": [1, 120, 0, 0]},
            "K": k,
            "D": {"online": false}
        },
        "alignment": {"dFrames": 120, "rateHz": 48000, "units": [
            {"unit": "A", "dLocal": 120, "hopsOffset": 0, "writable": true},
            {"unit": "K", "dLocal": 128, "hopsOffset": 0, "writable": false}
        ], "issues": [{"subject": "K", "code": "no_alink",
                       "message": "K: its firmware has no audio link; it is not aligned and takes scenes at once"}]},
        "alarms": []
    })
}

#[test]
fn wp_hwlk1f_a_unit_without_the_link_is_not_unknown_nor_counted() {
    let v = build_view(
        &model(),
        1,
        &status(json!({"online": true, "alink": "none"})),
        "admin",
    );
    let k = v.units.iter().find(|u| u.id == "K").unwrap();
    assert_eq!(
        (k.state.as_str(), k.state_word.as_str(), k.role.as_str()),
        ("no_alink", "No audio link (firmware)", "No audio link")
    );
    assert!(k.link_delay_frames.is_none() && k.link_delay_ms.is_none() && !k.pending);
    assert_eq!((v.unit_count, v.locked), (2, 1));
    assert_eq!(
        v.summary,
        "Audio link: 1 of 2 locked, 1 without the audio link (firmware), 1 not connected"
    );
    assert_eq!(v.health, "problem");

    // A unit that did not answer stays Unknown, with the model's role.
    let v = build_view(&model(), 1, &status(json!({"online": true})), "admin");
    let k = v.units.iter().find(|u| u.id == "K").unwrap();
    assert_eq!(
        (k.state_word.as_str(), k.role.as_str()),
        ("Unknown", "Receiver")
    );
    assert_eq!(v.unit_count, 3);

    // Every unit with the link locked, one without: a warning, not ok.
    let mut m = model();
    m["units"].as_array_mut().unwrap().pop();
    let v = build_view(
        &m,
        1,
        &status(json!({"online": true, "alink": "none"})),
        "admin",
    );
    assert_eq!(
        v.summary,
        "Audio link: 1 of 1 locked, 1 without the audio link (firmware)"
    );
    assert_eq!(v.health, "warning");
}

#[test]
fn wp_hwlk1f_a_mixed_recall_says_what_went_at_once() {
    let report = |writes: Value, path: &str, on_time: bool| {
        json!({
            "plan": {"scene": "evening", "frame": 9600, "path": path, "leadFrames": 4800,
                     "ops": [{"serial": "TX", "element": "tx", "op": "master_volume"}]},
            "writes": writes, "onTime": on_time
        })
    };
    let k_now = json!({"serial": "K", "element": "K", "what": "master_volume", "result": "applied_now",
                       "reason": "no audio link (firmware)"});
    let tx = json!({"serial": "TX", "element": "tx", "what": "master_volume", "result": "queued"});
    let d = json!({"serial": "D", "element": "D", "what": "master_volume", "result": "no_device",
                   "reason": "not connected"});
    let r = recall_outcome(
        "evening",
        &report(json!([tx, k_now]), "superframe", false),
        &model(),
        None,
        48_000,
    );
    assert!(r.taken && !r.on_time);
    assert_eq!(
        r.message,
        "Evening recalled: applied now on K340 (no audio link); at link frame 9600 through the sender on the others."
    );
    // The sender's write is the scene; K's keeps its op.
    assert_eq!(r.writes[0].what, "Scene Evening");
    assert_eq!(
        (r.writes[1].what.as_str(), r.writes[1].result_word.as_str()),
        ("Master volume", "Applied now")
    );

    // An absent unit is named, never "on time on every unit".
    let r = recall_outcome(
        "evening",
        &report(json!([tx, d]), "superframe", false),
        &model(),
        None,
        48_000,
    );
    assert_eq!(
        r.message,
        "Evening recalled: at link frame 9600 through the sender; Bar not connected."
    );
    assert_eq!(r.writes[1].what, "Master volume");

    // Only K: applied now. K and a unit with no frame: said.
    let r = recall_outcome(
        "evening",
        &report(json!([k_now]), "direct", false),
        &model(),
        None,
        48_000,
    );
    assert_eq!(
        r.message,
        "Evening recalled: applied now on K340 (no audio link)."
    );
    let nf = json!({"serial": "A", "element": "A", "what": "mute", "result": "no_frame"});
    let r = recall_outcome(
        "evening",
        &report(json!([k_now, nf]), "direct", false),
        &model(),
        None,
        48_000,
    );
    assert_eq!(
        r.message,
        "Evening recalled: applied now on K340 (no audio link); not applied on Main left (no link frame)."
    );
    assert_eq!(r.writes[1].result_word, "No link frame");
    // K could not do it (a click): nothing taken, refused.
    let un = json!({"serial": "K", "element": "K", "what": "click", "result": "unsupported",
                    "reason": "no audio link (firmware)"});
    let r = recall_outcome(
        "evening",
        &report(json!([un]), "direct", false),
        &model(),
        None,
        48_000,
    );
    assert!(!r.taken);
    assert_eq!(
        r.message,
        "Evening was refused by every unit; nothing changed."
    );
    assert_eq!(
        (r.writes[0].result_word.as_str(), r.writes[0].tone.as_str()),
        ("Needs the audio link", "bad")
    );
    // A late unit with K now: the frame part not on time.
    let late = json!({"serial": "A", "element": "A", "what": "mute", "result": "late"});
    let r = recall_outcome(
        "evening",
        &report(json!([late, k_now]), "per_unit", false),
        &model(),
        None,
        48_000,
    );
    assert_eq!(
        r.message,
        "Evening recalled: applied now on K340 (no audio link); at link frame 9600 to each unit on the others. \
         Not every unit at the frame on time."
    );
}

#[test]
fn wp_hwlk1f_align_says_nothing_was_written_and_why() {
    let w = |el: &str, what: &str, result: &str, reason: Option<&str>| {
        let mut v = json!({"serial": el, "element": el, "what": what, "result": result});
        if let Some(r) = reason {
            v["reason"] = r.into();
        }
        v
    };
    let report = |writes: Vec<Value>| json!({"applied": true, "alignment": {"issues": []}, "writes": writes});
    let all_skipped = report(vec![
        w("K", "phase", "skipped", Some("no audio link (firmware)")),
        w("D", "phase", "skipped", Some("not connected")),
    ]);
    let a = align_outcome(&all_skipped, &model());
    assert_eq!(
        a.message,
        "Nothing was written: skipped on K340 (no audio link (firmware)), Bar (not connected)."
    );
    let partial = report(vec![
        w("A", "phase", "ok", None),
        w("K", "phase", "skipped", Some("no audio link (firmware)")),
    ]);
    assert_eq!(
        align_outcome(&partial, &model()).message,
        "The link delays were written on Main left (each re-anchored with a short silence); \
         skipped on K340 (no audio link (firmware))."
    );
    let kept = report(vec![
        w("A", "phase", "unchanged", None),
        w("K", "phase", "skipped", Some("no audio link (firmware)")),
    ]);
    assert_eq!(
        align_outcome(&kept, &model()).message,
        "Nothing was written: already set on Main left; skipped on K340 (no audio link (firmware))."
    );
    let none = report(vec![w("A", "phase", "unchanged", None)]);
    assert_eq!(
        align_outcome(&none, &model()).message,
        "Nothing was written: every unit already has its link delays."
    );
    // An older hub without reasons: the unit alone.
    let old = report(vec![w("K", "phase", "skipped", None)]);
    assert_eq!(
        align_outcome(&old, &model()).message,
        "Nothing was written: skipped on K340."
    );
}

// -- fixup hwlk1f --
/// F1: a unit the sender's write reaches but whose frame was not read is not
/// known to have taken it; F3: every write failed is "Nothing was written".
#[test]
fn fixup_hwlk1f_unverified_units_and_all_failed_writes_are_said() {
    let report = json!({
        "plan": {"scene": "evening", "frame": 9600, "path": "superframe", "leadFrames": 4800,
                 "ops": [{"serial": "TX", "element": "tx", "op": "mute"}]},
        "writes": [
            {"serial": "TX", "element": "tx", "what": "mute", "result": "queued"},
            {"serial": "A", "element": "A", "what": "mute", "result": "no_frame", "reason": "no link frame read"}
        ],
        "onTime": false
    });
    let r = recall_outcome("evening", &report, &model(), None, 48_000);
    assert_eq!(
        r.message,
        "Evening recalled: at link frame 9600 through the sender; \
         not known to have taken it on Main left (no link frame read)."
    );
    assert_eq!(r.writes[1].what, "Mute", "a unit's own result keeps its op");
    let failed = json!({"applied": true, "alignment": {"issues": []}, "writes": [
        {"serial": "A", "element": "A", "what": "phase", "result": "timeout"}
    ]});
    assert_eq!(
        align_outcome(&failed, &model()).message,
        "Nothing was written: 1 writes failed."
    );
}
