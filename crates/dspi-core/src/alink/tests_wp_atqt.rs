//! WP ATQT: the S/PDIF rows and edits of the native System editor, from the
//! hub's `system.*` answers (shapes as dspi-hub's serde writes them: the
//! model's `spdifSource`/`spdifTxMask`/`spdifD`, `observed[serial].spdif` as
//! `align::SpdifSeen`). The end-to-end check against `uconsole --demo hub
//! --alink --with-measure-units` is the Qt smoke section `atqt-spdif`.

use serde_json::{Value, json};

use super::alink_system::build_view;
use super::alink_system_spdif::{AlinkSpdifEdit, apply_spdif, slots_word, spdif_rows};
use crate::DspiCore;

fn model() -> Value {
    json!({
        "rooms": [{"id": "bar", "name": "Bar"}],
        "senders": [{"id": "tx", "name": "Sender", "serial": "TX"}],
        "fanOuts": [],
        "units": [
            {"id": "A", "name": "Main left", "serial": "A", "room": "bar", "upstream": "tx"},
            {"id": "S", "name": "S/PDIF source", "serial": "SRC", "room": "bar", "upstream": "",
             "transport": "spdif", "spdifSource": true, "spdifTxMask": 8},
            {"id": "SR", "name": "S/PDIF rear", "serial": "REAR", "room": "bar", "upstream": "S",
             "transport": "spdif"},
            {"id": "U", "name": "USB", "serial": "U", "room": "bar", "upstream": "", "transport": "usb_direct"}
        ]
    })
}

fn status() -> Value {
    json!({
        "revision": 3,
        "observed": {
            "SRC": {"online": true, "spdifBlock": "present",
                    "spdif": {"mode": "source", "d": 0, "txMask": 8, "state": "locked", "superframe": true,
                              "holdover": false, "lockError": false, "foreign": false, "dMin": 0, "dMax": 0}},
            "REAR": {"online": true, "spdifBlock": "present",
                     "spdif": {"mode": "follower", "d": 192, "txMask": 0, "state": "lock_error",
                               "superframe": false, "holdover": false, "lockError": true, "foreign": false,
                               "dMin": 129, "dMax": 399}},
            "A": {"online": true}
        }
    })
}

#[test]
fn wp_atqt_the_spdif_rows_name_the_source_its_slots_and_the_follower_without_a_time_base() {
    let rows = spdif_rows(&model(), &status());
    assert_eq!(rows.len(), 2, "{rows:?}");
    let s = &rows[0];
    assert_eq!((s.id.as_str(), s.role.as_str()), ("S", "source"));
    assert_eq!(s.tx_mask, 8);
    assert_eq!(s.slots_word, "Outputs 7-8");
    assert_eq!(s.state_word, "Locked");
    assert!(!s.no_time_base && s.d.is_none());
    let f = &rows[1];
    assert_eq!(
        (f.role.as_str(), f.source.as_str(), f.source_name.as_str()),
        ("follower", "S", "S/PDIF source")
    );
    assert_eq!(f.state, "lock_error");
    assert_eq!(f.state_word, "Lock error");
    assert_eq!(
        (f.unit_d, f.d_min, f.d_max),
        (Some(192), Some(129), Some(399))
    );
    assert!(f.lock_error && f.no_time_base, "{f:?}");
    // A foreign stream says so too; a follower that is offline is only offline
    let mut st = status();
    st["observed"]["REAR"]["spdif"]["lockError"] = json!(false);
    st["observed"]["REAR"]["spdif"]["foreign"] = json!(true);
    st["observed"]["REAR"]["spdif"]["state"] = json!("none");
    let f = &spdif_rows(&model(), &st)[1];
    assert!(f.no_time_base && f.state_word == "No S/PDIF time base");
    st["observed"]["REAR"]["online"] = json!(false);
    let f = &spdif_rows(&model(), &st)[1];
    assert!(!f.no_time_base && f.state == "offline" && f.state_word == "Not connected");
    // A firmware without the block
    let mut st = status();
    st["observed"]["SRC"]["spdifBlock"] = json!("none");
    assert_eq!(spdif_rows(&model(), &st)[0].state, "no_spdif");
    // Nothing on S/PDIF: no rows; garbage: no panic
    assert!(spdif_rows(&json!({"units": [{"id": "A"}]}), &status()).is_empty());
    assert!(spdif_rows(&Value::Null, &Value::Null).is_empty());
    assert!(
        spdif_rows(
            &json!({"units": [{"transport": "spdif", "spdifTxMask": 9999}]}),
            &json!(7)
        )
        .len()
            == 1
    );
}

#[test]
fn wp_atqt_slots_in_words() {
    assert_eq!(slots_word(0), "None");
    assert_eq!(slots_word(0b1001), "Outputs 1-2, 7-8");
}

fn unit<'a>(m: &'a Value, id: &str) -> &'a Value {
    m["units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == id)
        .unwrap()
}

fn edit(unit_id: &str) -> AlinkSpdifEdit {
    AlinkSpdifEdit {
        unit_id: unit_id.into(),
        ..AlinkSpdifEdit::default()
    }
}

#[test]
fn wp_atqt_spdif_edits_change_only_what_was_edited() {
    // A follower's source, a source's slots and its D
    let mut m = model();
    let mut slots = edit("S");
    slots.tx_mask = Some(0b1100);
    slots.set_d = true;
    slots.d = Some(200);
    apply_spdif(&mut m, &[slots]).unwrap();
    assert_eq!(unit(&m, "S")["spdifTxMask"], 12);
    assert_eq!(unit(&m, "S")["spdifD"], 200);
    let mut back = edit("S");
    back.set_d = true;
    apply_spdif(&mut m, &[back]).unwrap();
    assert!(unit(&m, "S").get("spdifD").is_none());
    assert_eq!(unit(&m, "A"), unit(&model(), "A"));

    // A unit of the audio link made a source (outputs 7-8 by default), then
    // A follows it; the follower's source changes back to S
    let mut m = model();
    let mut src = edit("A");
    src.role = Some("source".into());
    let mut follow = edit("SR");
    follow.role = Some("follower".into());
    follow.source = Some("A".into());
    apply_spdif(&mut m, &[src, follow]).unwrap();
    assert_eq!(unit(&m, "A")["spdifSource"], true);
    assert_eq!(unit(&m, "A")["spdifTxMask"], 8);
    assert_eq!(unit(&m, "A")["upstream"], "tx");
    assert_eq!(unit(&m, "SR")["upstream"], "A");
    assert_eq!(unit(&m, "SR")["transport"], "spdif");

    // Off S/PDIF: the follower back on the audio link from the first sender,
    // the source loses its role
    let mut m = model();
    let mut off = edit("SR");
    off.role = Some("off".into());
    let mut off_src = edit("S");
    off_src.role = Some("off".into());
    apply_spdif(&mut m, &[off, off_src]).unwrap();
    for id in ["SR", "S"] {
        let u = unit(&m, id);
        assert!(
            u.get("transport").is_none()
                && u.get("spdifSource").is_none()
                && u.get("spdifTxMask").is_none(),
            "{u}"
        );
        assert_eq!(u["upstream"], "tx");
    }
}

#[test]
fn wp_atqt_spdif_edits_that_cannot_be_made_are_refused_whole() {
    let refused = |e: AlinkSpdifEdit| {
        let mut m = model();
        let before = m.clone();
        let r = apply_spdif(&mut m, &[e]);
        assert!(r.is_err(), "{m}");
        assert_eq!(m, before);
        r.unwrap_err().to_string()
    };
    assert!(refused(edit("nope")).contains("no unit nope"));
    let mut f = edit("SR");
    f.role = Some("follower".into());
    f.source = Some("A".into());
    assert!(refused(f).contains("not an S/PDIF time-base source"));
    let mut own = edit("S");
    own.role = Some("follower".into());
    own.source = Some("S".into());
    assert!(refused(own).contains("not an S/PDIF time-base source"));
    let mut mask = edit("S");
    mask.tx_mask = Some(0);
    assert!(refused(mask).contains("1 to 4 output slots"));
    let mut mask = edit("SR");
    mask.tx_mask = Some(1);
    assert!(refused(mask).contains("only an S/PDIF source"));
    let mut d = edit("SR");
    d.set_d = true;
    d.d = Some(150);
    assert!(refused(d).contains("only an S/PDIF source"));
    let mut usb = edit("U");
    usb.role = Some("source".into());
    assert!(refused(usb).contains("fed by USB"));
    let mut bad = edit("S");
    bad.role = Some("sideways".into());
    assert!(refused(bad).contains("source, follower or off"));
    assert!(apply_spdif(&mut Value::Null, &[edit("S")]).is_err());
    // No edits: nothing changes
    let mut m = model();
    apply_spdif(&mut m, &[]).unwrap();
    assert_eq!(m, model());
}

#[test]
fn wp_atqt_without_a_view_there_are_no_spdif_rows_and_a_save_says_disconnected() {
    let core = DspiCore::new();
    assert!(core.alink_system_spdif().is_empty());
    let r = core.alink_system_save_spdif(1, Default::default(), vec![edit("S")]);
    assert!(matches!(r, Err(crate::DspiError::Disconnected)), "{r:?}");
}

#[test]
fn wp_atqt_units_fed_by_spdif_are_not_counted_on_the_audio_link() {
    let mut st = status();
    st["observed"]["TX"] = json!({"online": true, "role": "sender"});
    st["observed"]["A"] =
        json!({"online": true, "role": "receiver", "state": "locked", "phase": [1, 120, 0, 0]});
    st["alignment"] = json!({"dFrames": 120, "units": [{"unit": "A", "dLocal": 120, "hopsOffset": 0, "writable": true}]});
    let mut m = model();
    m["units"]
        .as_array_mut()
        .unwrap()
        .retain(|u| u["id"] != "U");
    let v = build_view(&m, 3, &st, "admin");
    let row = |id: &str| v.units.iter().find(|u| u.id == id).unwrap().clone();
    assert_eq!(row("S").transport, "spdif");
    assert_eq!(
        (row("S").state.as_str(), row("S").state_word.as_str()),
        ("locked", "Locked")
    );
    assert_eq!(
        (row("SR").state.as_str(), row("SR").state_word.as_str()),
        ("lock_error", "Lock error")
    );
    assert!(!row("S").pending && !row("SR").pending);
    // A source on the audio link is one of its units
    assert_eq!(row("A").transport, "alink");
    assert!(
        v.summary.starts_with("Audio link: 1 of 1 locked"),
        "{}",
        v.summary
    );
}
