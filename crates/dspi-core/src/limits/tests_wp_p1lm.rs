//! WP P1LM: limits profiles in the core, on the demo device. Files live
//! under `std::env::temp_dir()`, which `ci/run.sh` points at the home volume
//! (never tmpfs).

use std::path::PathBuf;
use std::sync::Arc;

use dspi_session::limits::{GOOD_FILE, LimitsStore, OutputLimits, PIN_FILE, PROFILE_FILE};
use serde_json::{Value, json};

use super::*;
use crate::consts::{IMPORT_ALL_BLOCKS, IMPORT_HARDWARE, IMPORT_VOLUMES};
use crate::write_param::ParamValue;

const ON: u8 = limiter::LIMITER_PARAM_ENABLED as u8;
const THR: u8 = limiter::LIMITER_PARAM_THRESHOLD_DB as u8;
const ALL: u8 = limiter::LIMITER_ALL_OUTPUTS as u8;

/// The output every test limits, and the one it leaves alone.
const O: u8 = 2;
const FREE: u8 = 0;

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    core
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("wp-p1lm-core").join(format!(
        "{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn serial(core: &DspiCore) -> String {
    core.lock().conn.as_ref().unwrap().serial.clone()
}

/// Output `O`: gain at most 3 dB, delay 1 to 5 ms, a limiter at -3 dBFS, a
/// 24 dB/oct high-pass at 1.5 kHz or above; every PEQ boost at most 6 dB;
/// the master volume at most -10 dB.
fn profile() -> LimitsProfile {
    LimitsProfile {
        outputs: vec![OutputLimits {
            output: O,
            max_gain_db: Some(3.0),
            highpass_min_hz: Some(1500.0),
            highpass_min_slope_db: Some(24),
            delay_min_ms: Some(1.0),
            delay_max_ms: Some(5.0),
            limiter_ceiling_db: Some(-3.0),
        }],
        max_peq_boost_db: Some(6.0),
        master_volume_max_db: Some(-10.0),
        note: "Tweeters on 3".into(),
        contact: "installer@example.org".into(),
        ..Default::default()
    }
}

fn ch(core: &DspiCore, o: u8) -> u8 {
    core.get_state().num_input_channels + o
}

fn band(filter_type: u8, freq: f32, gain: f32) -> FilterParams {
    FilterParams {
        filter_type,
        bypass: false,
        freq,
        q: 0.707,
        gain,
        qp: 0.707,
    }
}

/// The device inside every limit of [`profile`]: an LR4 high-pass at 2 kHz
/// on output `O`'s first crossover slot, its limiter on at -6 dB.
fn inside(core: &DspiCore) {
    let c = ch(core, O);
    core.set_crossover(c, 0, band(35, 2000.0, 0.0)).unwrap();
    core.set_limiter_param(O, ON, 1.0).unwrap();
    core.set_limiter_param(O, THR, -6.0).unwrap();
    core.set_output_gain(O, 0.0).unwrap();
    core.set_output_delay(O, 2.0).unwrap();
    core.set_master_volume(-20.0).unwrap();
}

fn code(e: DspiError) -> String {
    match e {
        DspiError::LimitExceeded { code, .. } => code,
        other => panic!("not a limit refusal: {other:?}"),
    }
}

fn apply(core: &DspiCore, p: &LimitsProfile) -> LimitsApply {
    core.limits_apply_profile(p.to_document()).unwrap()
}

#[test]
fn wp_p1lm_no_profile_changes_nothing() {
    let core = demo();
    core.limits_set_dir(dir("none").to_string_lossy().into())
        .unwrap();
    core.set_output_gain(O, 20.0).unwrap();
    core.set_master_volume(0.0).unwrap();
    core.set_filter(0, 0, band(1, 1000.0, 20.0)).unwrap();
    assert_eq!(core.get_state().output_gain_db[usize::from(O)], 20.0);
    let st = core.limits_status();
    assert!(!st.active);
    assert_eq!(st.state, "none");
    assert_eq!(st.notice_key, "");
    assert_eq!(core.limits_profile_document(), "");
}

/// Each limit refuses a typed write, `write_param`, and an undo that would
/// cross it; nothing moves.
#[test]
fn wp_p1lm_each_limit_refuses_a_typed_write_write_param_and_an_undo() {
    let core = demo();
    // Values an undo will want back, set before the profile.
    let c = ch(&core, O);
    core.set_output_gain(O, 10.0).unwrap();
    core.set_output_delay(O, 9.0).unwrap();
    core.set_master_volume(0.0).unwrap();
    core.set_filter(0, 1, band(1, 1000.0, 12.0)).unwrap();
    core.set_crossover(c, 0, band(35, 800.0, 0.0)).unwrap();
    inside(&core);
    core.set_filter(0, 1, band(1, 1000.0, 3.0)).unwrap();
    let held = apply(&core, &profile());
    assert!(held.held.is_empty(), "inside every limit: {:?}", held.held);
    let before = core.get_state();

    let typed: Vec<(&str, Result<()>)> = vec![
        ("out_gain", core.set_output_gain(O, 6.0)),
        ("delay", core.set_output_delay(O, 7.0)),
        ("delay", core.set_output_delay(O, 0.5)),
        ("delay", core.set_delay(c, 7.0)),
        ("master_volume", core.set_master_volume(-3.0)),
        ("eq_boost", core.set_filter(0, 1, band(1, 1000.0, 9.0))),
        ("eq_boost", core.set_filter(c, 2, band(2, 100.0, 7.0))),
        ("highpass", core.set_crossover(c, 0, band(35, 900.0, 0.0))),
        ("highpass", core.set_crossover(c, 0, band(33, 2000.0, 0.0))),
        ("highpass", core.set_crossover(c, 0, band(0, 2000.0, 0.0))),
        ("highpass", core.set_band_bypass(c, 20, true)),
        ("limiter", core.set_limiter_param(O, ON, 0.0)),
        ("limiter", core.set_limiter_param(ALL, ON, 0.0)),
        ("limiter", core.set_limiter_param(O, THR, -1.0)),
    ];
    for (want, r) in typed {
        assert_eq!(code(r.unwrap_err()), want);
    }
    let via = |path: &str, ix: &[u8], v: ParamValue| {
        code(core.write_param(path.into(), ix.to_vec(), v).unwrap_err())
    };
    assert_eq!(via("out.gain", &[O], ParamValue::Float(6.0)), "out_gain");
    assert_eq!(via("out.delay", &[O], ParamValue::Float(7.0)), "delay");
    assert_eq!(
        via("vol.master", &[], ParamValue::Float(-1.0)),
        "master_volume"
    );
    assert_eq!(via("eq.gain", &[0, 1], ParamValue::Float(9.0)), "eq_boost");
    assert_eq!(
        via("eq.freq", &[c, 20], ParamValue::Float(500.0)),
        "highpass"
    );
    assert_eq!(via("limit.on", &[O], ParamValue::Bool(false)), "limiter");
    assert_eq!(
        via("limit.threshold", &[O], ParamValue::Float(0.0)),
        "limiter"
    );
    assert_eq!(core.get_state(), before, "nothing moved");

    // Within the limits, writes pass.
    core.set_output_gain(O, 3.0).unwrap();
    core.set_output_gain(FREE, 12.0).unwrap();
    core.set_crossover(c, 0, band(39, 2500.0, 0.0)).unwrap();
    core.set_limiter_param(O, THR, -4.0).unwrap();

    // Undo walks back to the values set before the profile: each refused.
    let mut refused = Vec::new();
    for _ in 0..40 {
        match core.undo() {
            Ok(m) if m.entry.is_none() => break,
            Ok(_) => {}
            Err(DspiError::LimitExceeded { code, .. }) => {
                refused.push(code);
                break;
            }
            Err(e) => panic!("{e:?}"),
        }
    }
    assert_eq!(refused.len(), 1, "an undo past a limit is refused");
    let s = core.get_state();
    let k = usize::from(O);
    assert!(s.output_gain_db[k] <= 3.0);
    assert!(s.output_delay_ms[k] <= 5.0);
    assert!(s.master_volume_db <= -10.0);
}

/// An undo that crosses a limit is refused, by name.
#[test]
fn wp_p1lm_an_undo_across_a_limit_is_refused() {
    let core = demo();
    core.set_output_gain(O, 10.0).unwrap();
    core.set_output_gain(O, 0.0).unwrap();
    let gain_only = LimitsProfile {
        outputs: vec![OutputLimits {
            output: O,
            max_gain_db: Some(3.0),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert!(apply(&core, &gain_only).held.is_empty());
    let e = core.undo().unwrap_err();
    assert_eq!(code(e), "out_gain");
    assert_eq!(core.get_state().output_gain_db[usize::from(O)], 0.0);
    // Without the profile the same undo goes through.
    core.limits_clear().unwrap();
    core.undo().unwrap();
    assert_eq!(core.get_state().output_gain_db[usize::from(O)], 10.0);
}

/// Narrowing a limit while the device is outside it clamps at once: writes,
/// journalled as one undo group, reported in the import's words.
#[test]
fn wp_p1lm_narrowing_a_limit_clamps_and_reports() {
    let core = demo();
    let c = ch(&core, O);
    // No high-pass anywhere on the output.
    for x in 0..4 {
        core.set_crossover(c, x, band(0, 1000.0, 0.0)).unwrap();
    }
    for b in 0..10 {
        core.set_filter(c, b, band(1, 1000.0, 0.0)).unwrap();
    }
    core.set_output_gain(O, 10.0).unwrap();
    core.set_output_delay(O, 9.0).unwrap();
    core.set_master_volume(0.0).unwrap();
    core.set_filter(0, 1, band(1, 1000.0, 12.0)).unwrap();
    core.set_limiter_param(O, ON, 0.0).unwrap();
    core.set_limiter_param(O, THR, 0.0).unwrap();
    let r = apply(&core, &profile());
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    let s = core.get_state();
    let k = usize::from(O);
    assert_eq!(s.output_gain_db[k], 3.0);
    assert_eq!(s.output_delay_ms[k], 5.0);
    assert_eq!(s.master_volume_db, -10.0);
    assert_eq!(s.filters[0][1].gain, 6.0);
    assert!(s.limiter_enabled[k]);
    assert_eq!(s.limiter_threshold_db[k], -3.0);
    // No high-pass at all: one is made in a flat crossover slot.
    let xo = s.xover[usize::from(c)]
        .iter()
        .find(|b| shared::highpass_slope(b.filter_type).is_some())
        .copied()
        .expect("a high-pass was added");
    assert!(shared::highpass_slope(xo.filter_type).unwrap() >= 24);
    assert_eq!(xo.freq, 1500.0);
    assert!(!xo.bypass);
    // Reported, row by row and as one line.
    assert_eq!(r.lines.len(), 1);
    assert!(
        r.lines[0].starts_with("Held to the limits profile: "),
        "{:?}",
        r.lines
    );
    assert!(
        r.held.iter().any(|l| l.starts_with("out.gain[2] 10 → 3")),
        "{:?}",
        r.held
    );
    assert!(
        r.held.iter().any(|l| l.contains("high-pass at 1500 Hz")),
        "{:?}",
        r.held
    );
    assert_eq!(core.limits_status().last_held, r.held);
    // Journalled under the group's label.
    let labelled = core
        .history(100)
        .into_iter()
        .filter(|e| e.label.as_deref() == Some(HOLD_LABEL))
        .count();
    assert!(labelled >= 1, "{labelled}");
    // One undo puts the group's undoable values back (the limiter's enable,
    // audible, stands apart, as in a drag).
    core.limits_clear().unwrap();
    core.undo().unwrap();
    assert_eq!(core.get_state().output_gain_db[k], 10.0);
    apply(&core, &profile());
    // Applying the same profile again holds nothing more.
    assert!(apply(&core, &profile()).held.is_empty());
}

/// An import is held to the limits and says so.
#[test]
fn wp_p1lm_an_import_is_clamped_and_reported() {
    let core = demo();
    core.set_output_gain(O, 10.0).unwrap();
    core.set_master_volume(0.0).unwrap();
    core.set_filter(0, 1, band(1, 1000.0, 12.0)).unwrap();
    let doc = core.config_export("loud".into(), "test".into(), 0).unwrap();
    inside(&core);
    apply(&core, &profile());
    let r: Value = serde_json::from_str(
        &core.config_import_tok(doc, IMPORT_VOLUMES | IMPORT_HARDWARE | IMPORT_ALL_BLOCKS),
    )
    .unwrap();
    assert_eq!(r["ok"], true, "{r}");
    let lines: Vec<String> = serde_json::from_value(r["lines"].clone()).unwrap();
    let held = lines
        .iter()
        .find(|l| l.contains("Held to the limits profile"))
        .unwrap_or_else(|| panic!("{lines:?}"));
    assert!(held.contains("out.gain[2] 10 → 3"), "{held}");
    assert!(held.contains("eq.gain[0,1] 12 → 6"), "{held}");
    assert_eq!(r["clean"], false);
    let s = core.get_state();
    assert_eq!(s.output_gain_db[usize::from(O)], 3.0);
    assert_eq!(s.filters[0][1].gain, 6.0);
    assert_eq!(s.master_volume_db, -10.0);
    assert!(!core.limits_status().last_held.is_empty());
}

/// A snapshot recall (an A/B toggle is one) is held and says so.
#[test]
fn wp_p1lm_a_recall_is_clamped_and_reported() {
    let core = demo();
    core.set_output_gain(O, 10.0).unwrap();
    core.set_filter(0, 1, band(1, 1000.0, 12.0)).unwrap();
    let snap = core
        .snapshot_capture_document("loud".into(), String::new())
        .unwrap();
    inside(&core);
    apply(&core, &profile());
    let r = core.snapshot_recall_document(snap).unwrap();
    assert!(r.image);
    let held = r
        .notes
        .iter()
        .find(|l| l.starts_with("Held to the limits profile"))
        .unwrap_or_else(|| panic!("{:?}", r.notes));
    assert!(held.contains("out.gain[2] 10 → 3"), "{held}");
    let s = core.get_state();
    assert_eq!(s.output_gain_db[usize::from(O)], 3.0);
    assert_eq!(s.filters[0][1].gain, 6.0);
}

/// A preset slot load: the firmware loads what the host never held. It is
/// read back, held and reported (OD-P1j).
#[test]
fn wp_p1lm_a_slot_load_outside_the_profile_is_clamped_and_reported() {
    let core = demo();
    core.set_output_gain(O, 10.0).unwrap();
    core.save_preset_tok(3).unwrap();
    core.set_output_gain(O, 0.0).unwrap();
    apply(&core, &profile());
    core.load_preset(3).unwrap();
    assert_eq!(core.get_state().output_gain_db[usize::from(O)], 3.0);
    let st = core.limits_status();
    assert!(st.active);
    assert!(
        st.last_held
            .iter()
            .any(|l| l.starts_with("out.gain[2] 10 → 3")),
        "{:?}",
        st.last_held
    );
}

/// A corrupt profile with a last good copy keeps that copy (fails closed),
/// with the notice; without one, unlimited with the notice standing.
#[test]
fn wp_p1lm_a_corrupt_profile_falls_back_or_runs_unlimited_with_a_notice() {
    let d = dir("corrupt");
    let core = demo();
    core.limits_set_dir(d.to_string_lossy().into()).unwrap();
    inside(&core);
    apply(&core, &profile());
    assert_eq!(core.limits_status().state, "ok");
    let folder = LimitsStore::new(&d).dir(&serial(&core));
    std::fs::write(folder.join(PROFILE_FILE), "{\"meta\":{\"kind\":\"lim").unwrap();
    // A fresh read (another start of the app).
    core.limits_set_dir(d.to_string_lossy().into()).unwrap();
    let st = core.limits_status();
    assert_eq!(st.state, "fell_back");
    assert_eq!(st.notice_key, "limits.fell_back");
    assert!(st.active);
    assert!(!st.detail.is_empty());
    assert_eq!(code(core.set_output_gain(O, 6.0).unwrap_err()), "out_gain");
    let n = crate::notice_for_key(st.notice_key, st.detail);
    assert_eq!(n.title, "Limits profile unreadable");

    std::fs::remove_file(folder.join(GOOD_FILE)).unwrap();
    core.limits_set_dir(d.to_string_lossy().into()).unwrap();
    let st = core.limits_status();
    assert_eq!(st.state, "unreadable");
    assert_eq!(st.notice_key, "limits.unreadable");
    assert!(!st.active);
    core.set_output_gain(O, 6.0).unwrap();
    assert_eq!(
        core.limits_status().notice_key,
        "limits.unreadable",
        "standing"
    );
}

/// A profile kept in files survives a new core; another device has none.
#[test]
fn wp_p1lm_a_profile_is_kept_per_serial() {
    let d = dir("kept");
    let core = demo();
    core.limits_set_dir(d.to_string_lossy().into()).unwrap();
    inside(&core);
    apply(&core, &profile());
    let doc = core.limits_profile_document();
    assert!(doc.contains("\"kind\": \"limits\""), "{doc}");
    let again = demo();
    again.limits_set_dir(d.to_string_lossy().into()).unwrap();
    assert!(again.limits_status().active);
    assert_eq!(code(again.set_output_gain(O, 6.0).unwrap_err()), "out_gain");
    // Cleared: gone from the files too.
    again.limits_clear().unwrap();
    assert!(!again.limits_status().active);
    let third = demo();
    third.limits_set_dir(d.to_string_lossy().into()).unwrap();
    assert!(!third.limits_status().active);
}

/// The PIN's hash never leaves the host: not in an export (which carries
/// the profile in `meta.limits`), a snapshot, a template or a diagnostics
/// bundle.
#[test]
fn wp_p1lm_no_document_or_bundle_holds_the_pin_hash() {
    let d = dir("pin");
    let core = demo();
    core.limits_set_dir(d.to_string_lossy().into()).unwrap();
    inside(&core);
    apply(&core, &profile());
    let store = LimitsStore::new(&d);
    let s = serial(&core);
    // -- WP FLAKE3 -- a PIN no document can hold by chance: "4711" matched digits in a
    // snapshot's own numbers. '~' is in no number, hex, base64 or name the core writes,
    // and JSON writes it as it is.
    let pin = "~flake3~4711~";
    store.set_pin(&s, pin).unwrap();
    let rec = store.pin(&s).unwrap().unwrap();
    assert!(store.dir(&s).join(PIN_FILE).exists());

    let export = core.config_export("x".into(), "t".into(), 0).unwrap();
    let v: Value = serde_json::from_str(&export).unwrap();
    assert_eq!(v["meta"]["limits"]["outputs"][0]["maxGainDb"], json!(3.0));
    let snapshot = core
        .snapshot_capture_document("s".into(), String::new())
        .unwrap();
    let template = core.save_as_template("t".into(), "t".into(), 0).unwrap();
    let profile_doc = core.limits_profile_document();
    let zip = core
        .diagnostics_zip(crate::records::DiagOptions {
            app: "test".into(),
            hub_store_dir: String::new(),
            files: Vec::new(),
        })
        .unwrap();
    let zip = String::from_utf8_lossy(&zip).into_owned();
    // The PIN as JSON would write it too, should an encoder escape it.
    let escaped = serde_json::to_string(pin).unwrap();
    let escaped = escaped.trim_matches('"').to_owned();
    let leak = |text: &str| -> Option<&'static str> {
        if text.contains(&rec.hash) {
            Some("the PIN hash")
        } else if text.contains(&rec.salt) {
            Some("the PIN salt")
        } else if text.contains(pin) || text.contains(&escaped) {
            Some("the PIN")
        } else {
            None
        }
    };
    // The check catches a real leak: each secret, set into a document as a field.
    for secret in [pin, rec.hash.as_str(), rec.salt.as_str()] {
        let mut doc: Value = serde_json::from_str(&snapshot).unwrap();
        doc["meta"]["leak"] = json!(secret);
        assert!(
            leak(&doc.to_string()).is_some(),
            "a leaked {secret} is found"
        );
    }
    for (what, text) in [
        ("export", &export),
        ("snapshot", &snapshot),
        ("template", &template),
        ("profile", &profile_doc),
        ("bundle", &zip),
    ] {
        assert_eq!(leak(text), None, "{what} holds a PIN secret");
    }
}

/// A guarded revert restores a state captured before, past a limit set
/// during its countdown: it runs inside `unlimited`.
#[test]
fn wp_p1lm_a_guarded_revert_passes_a_limit_set_during_its_countdown() {
    let core = demo();
    core.set_output_gain(O, 10.0).unwrap();
    let before = core
        .snapshot_capture_document("before".into(), String::new())
        .unwrap();
    core.set_output_gain(O, 0.0).unwrap();
    // The countdown runs; a profile is set meanwhile.
    apply(&core, &profile());
    // The revert: not refused, not held.
    let r = unlimited(|| core.snapshot_recall_document(before.clone())).unwrap();
    assert!(
        r.notes.iter().all(|n| !n.contains("limits")),
        "{:?}",
        r.notes
    );
    assert_eq!(core.get_state().output_gain_db[usize::from(O)], 10.0);
    unlimited(|| core.set_output_gain(O, 12.0)).unwrap();
    // Outside `unlimited` the limit holds again.
    assert_eq!(code(core.set_output_gain(O, 12.0).unwrap_err()), "out_gain");
}

/// The notice table has the refusal's row.
#[test]
fn wp_p1lm_a_refusal_carries_its_notice() {
    let core = demo();
    inside(&core);
    apply(&core, &profile());
    let e = core.set_output_gain(O, 6.0).unwrap_err();
    assert_eq!(e.notice_key(), "limit_exceeded");
    let n = crate::notice_for(e.clone());
    assert_eq!(n.title, "Outside the limits");
    assert!(n.detail.contains("output 3 gain 6 dB"), "{}", n.detail);
    for k in NOTICE_KEYS {
        assert_ne!(
            crate::notice_for_key(k.into(), String::new()).key,
            crate::NOTICE_KEY_UNKNOWN,
            "{k}"
        );
    }
}
