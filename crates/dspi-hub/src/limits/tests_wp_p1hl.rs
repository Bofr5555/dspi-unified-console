//! WP P1HL unit tests: the wire judge, the hold, the PIN, the store, scenes.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::*;
use dspi_session::limits::OutputLimits;
use dspi_transport::{FakeDevice, Transport};

fn profile() -> LimitsProfile {
    LimitsProfile {
        serial: "S".into(),
        outputs: vec![OutputLimits {
            output: 0,
            max_gain_db: Some(3.0),
            highpass_min_hz: Some(80.0),
            highpass_min_slope_db: Some(12),
            delay_min_ms: Some(1.0),
            delay_max_ms: Some(5.0),
            limiter_ceiling_db: Some(-6.0),
        }],
        max_peq_boost_db: Some(6.0),
        master_volume_max_db: Some(-10.0),
        ..LimitsProfile::default()
    }
    .normalised()
}

fn set(code: u8, w_value: u16, payload: Vec<u8>) -> CmdRequest {
    write(1, code, w_value, payload)
}

fn f(v: f32) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

fn image(d: &FakeDevice) -> Image {
    Image::decode(d.remote().image()).unwrap()
}

fn packet(ch: u8, band: u8, t: u8, freq: f32, gain: f32, bypass: bool) -> EqParamPacket {
    EqParamPacket {
        channel: ch,
        band,
        filter_type: FilterType::from_raw(t),
        bypass,
        freq,
        q: 0.707,
        gain_db: gain,
        qp: None,
    }
}

const PEAKING: u8 = 1;
const LR2_HP: u8 = 33;

/// The demo device with output 0 inside the profile: every band of it flat
/// but a 12 dB/oct high-pass at 100 Hz in its first crossover slot, its
/// limiter on at -10 dBFS, its delay 2 ms.
fn compliant() -> (FakeDevice, u8) {
    let mut d = FakeDevice::demo();
    let ch = image(&d).n_in();
    for b in (0..PEQ_BANDS).chain(XOVER_BASE..XOVER_BASE + 4) {
        let flat = packet(ch, b, PEAKING, 1000.0, 0.0, false);
        d.control_out(op::REQ_SET_EQ_PARAM, 0, &flat.encode())
            .unwrap();
    }
    let hp = packet(ch, XOVER_BASE, LR2_HP, 100.0, 0.0, false);
    d.control_out(op::REQ_SET_EQ_PARAM, 0, &hp.encode())
        .unwrap();
    d.control_out(op::REQ_LIMITER, limiter::LIMITER_PARAM_ENABLED, &f(1.0))
        .unwrap();
    d.control_out(
        op::REQ_LIMITER,
        limiter::LIMITER_PARAM_THRESHOLD_DB,
        &f(-10.0),
    )
    .unwrap();
    d.control_out(op::REQ_SET_OUTPUT_DELAY, 0, &f(2.0)).unwrap();
    (d, ch)
}

fn refused(r: Result<(), Result<Violation, Need>>) -> &'static str {
    match r {
        Err(Ok(v)) => v.code,
        other => panic!("not refused: {other:?}"),
    }
}

#[test]
fn wp_p1hl_plain_values_are_judged_on_the_wire() {
    let p = profile();
    let (d, _) = compliant();
    let img = image(&d);
    let ok = |req: CmdRequest| assert_eq!(judge(&p, &req, Some(&img)), Ok(()), "{req:?}");
    ok(set(op::REQ_SET_OUTPUT_GAIN, 0, f(3.0)));
    ok(set(op::REQ_SET_OUTPUT_GAIN, 1, f(12.0)));
    ok(set(op::REQ_SET_MASTER_VOLUME, 0, f(-12.0)));
    ok(set(op::REQ_SET_MASTER_VOLUME, 0, f(-128.0)));
    ok(set(op::REQ_SET_OUTPUT_DELAY, 0, f(4.0)));
    assert_eq!(
        refused(judge(&p, &set(op::REQ_SET_OUTPUT_GAIN, 0, f(3.5)), None)),
        "out_gain"
    );
    assert_eq!(
        refused(judge(&p, &set(op::REQ_SET_MASTER_VOLUME, 0, f(-3.0)), None)),
        "master_volume"
    );
    assert_eq!(
        refused(judge(&p, &set(op::REQ_SET_OUTPUT_DELAY, 0, f(6.0)), None)),
        "delay"
    );
    // A channel delay: output 0 is wire channel n_in, which needs the image.
    let ch = u16::from(img.n_in());
    assert_eq!(
        judge(&p, &set(op::REQ_SET_DELAY, ch, f(9.0)), None),
        Err(Err(Need::Image))
    );
    assert_eq!(
        refused(judge(&p, &set(op::REQ_SET_DELAY, ch, f(9.0)), Some(&img))),
        "delay"
    );
    ok(set(op::REQ_SET_DELAY, 0, f(9.0)));
    // A short payload is not this rule's to refuse: the device stalls on it.
    ok(set(op::REQ_SET_OUTPUT_GAIN, 0, vec![1, 2]));
}

#[test]
fn wp_p1hl_eq_writes_keep_the_boost_and_the_high_pass() {
    let p = profile();
    let (d, ch) = compliant();
    let img = image(&d);
    assert!(p.highpass_kept(0, &img.views(ch, None)), "the fixture");
    let band = |ch: u8, b: u8, t: u8, freq: f32, gain: f32, bypass: bool| {
        set(
            op::REQ_SET_EQ_PARAM,
            0,
            packet(ch, b, t, freq, gain, bypass).encode(),
        )
    };
    // A boost on any channel, input included.
    assert_eq!(
        refused(judge(
            &p,
            &band(0, 2, PEAKING, 1000.0, 7.0, false),
            Some(&img)
        )),
        "eq_boost"
    );
    assert_eq!(
        judge(&p, &band(0, 2, PEAKING, 1000.0, 6.0, false), Some(&img)),
        Ok(())
    );
    // The output's high-pass: lowered below the corner, bypassed, retyped.
    for bad in [
        band(ch, XOVER_BASE, LR2_HP, 60.0, 0.0, false),
        band(ch, XOVER_BASE, LR2_HP, 100.0, 0.0, true),
        band(ch, XOVER_BASE, PEAKING, 100.0, 0.0, false),
    ] {
        assert_eq!(refused(judge(&p, &bad, Some(&img))), "highpass");
    }
    assert_eq!(
        judge(&p, &band(ch, XOVER_BASE, 35, 120.0, 0.0, false), Some(&img)),
        Ok(()),
        "steeper and higher keeps it"
    );
    let bypass = set(
        op::REQ_SET_BAND_BYPASS,
        (u16::from(ch) << 8) | u16::from(XOVER_BASE),
        vec![1],
    );
    assert_eq!(refused(judge(&p, &bypass, Some(&img))), "highpass");
    assert_eq!(
        judge(&p, &bypass, None),
        Err(Err(Need::Image)),
        "the high-pass rule needs the image"
    );
}

#[test]
fn wp_p1hl_limiter_writes_follow_the_cores_rule() {
    let p = profile();
    let (d, _) = compliant();
    let img = image(&d);
    let lim = |out: u8, idx: u16, v: f32| set(op::REQ_LIMITER, (u16::from(out) << 8) | idx, f(v));
    let (on, thr) = (
        limiter::LIMITER_PARAM_ENABLED,
        limiter::LIMITER_PARAM_THRESHOLD_DB,
    );
    assert_eq!(refused(judge(&p, &lim(0, on, 0.0), Some(&img))), "limiter");
    assert_eq!(
        refused(judge(&p, &lim(0, thr, -3.0), Some(&img))),
        "limiter"
    );
    assert_eq!(judge(&p, &lim(0, thr, -8.0), Some(&img)), Ok(()));
    // Every output at once reaches output 0 too.
    assert_eq!(
        refused(judge(&p, &lim(ALL_OUTPUTS, on, 0.0), Some(&img))),
        "limiter"
    );
    // Another output's limiter is not limited.
    assert_eq!(judge(&p, &lim(1, on, 0.0), Some(&img)), Ok(()));
    // A read is never judged.
    let read = CmdRequest {
        dir: Dir::Get,
        ..lim(0, on, 0.0)
    };
    assert_eq!(judge(&p, &read, None), Ok(()));
}

#[test]
fn wp_p1hl_a_limiter_outside_its_ceiling_may_be_put_right_step_by_step() {
    let p = profile();
    let mut d = FakeDevice::demo();
    d.control_out(op::REQ_LIMITER, limiter::LIMITER_PARAM_ENABLED, &f(0.0))
        .unwrap();
    d.control_out(
        op::REQ_LIMITER,
        limiter::LIMITER_PARAM_THRESHOLD_DB,
        &f(0.0),
    )
    .unwrap();
    let img = image(&d);
    let lim = |idx: u16, v: f32| set(op::REQ_LIMITER, idx, f(v));
    // Off at 0 dBFS: switching on, or lowering the threshold, both pass.
    assert_eq!(
        judge(&p, &lim(limiter::LIMITER_PARAM_ENABLED, 1.0), Some(&img)),
        Ok(())
    );
    assert_eq!(
        judge(
            &p,
            &lim(limiter::LIMITER_PARAM_THRESHOLD_DB, -2.0),
            Some(&img)
        ),
        Ok(())
    );
}

#[test]
fn wp_p1hl_the_hold_writes_what_the_core_would_hold() {
    let p = profile();
    let mut d = FakeDevice::demo();
    let ch = image(&d).n_in();
    // Everything outside: gain, delay, master volume, a boost, no high-pass,
    // the limiter off and high.
    for b in (0..PEQ_BANDS).chain(XOVER_BASE..XOVER_BASE + 4) {
        // A flat crossover slot's corner is low: the hold sets the profile's.
        let (t, freq) = if b >= XOVER_BASE {
            (0, 20.0)
        } else {
            (PEAKING, 1000.0)
        };
        let flat = packet(ch, b, t, freq, 0.0, false);
        d.control_out(op::REQ_SET_EQ_PARAM, 0, &flat.encode())
            .unwrap();
    }
    d.control_out(op::REQ_SET_OUTPUT_GAIN, 0, &f(10.0)).unwrap();
    d.control_out(op::REQ_SET_OUTPUT_DELAY, 0, &f(9.0)).unwrap();
    d.control_out(op::REQ_SET_MASTER_VOLUME, 0, &f(0.0))
        .unwrap();
    let boost = packet(0, 1, PEAKING, 1000.0, 12.0, false);
    d.control_out(op::REQ_SET_EQ_PARAM, 0, &boost.encode())
        .unwrap();
    d.control_out(op::REQ_LIMITER, limiter::LIMITER_PARAM_ENABLED, &f(0.0))
        .unwrap();
    d.control_out(
        op::REQ_LIMITER,
        limiter::LIMITER_PARAM_THRESHOLD_DB,
        &f(0.0),
    )
    .unwrap();
    let (writes, rows) = hold_writes(&p, &image(&d), 1);
    let has = |s: &str| rows.iter().any(|r| r.starts_with(s));
    assert!(has("out.gain[0] 10 → 3"), "{rows:?}");
    assert!(has("out.delay[0] 9 → 5"), "{rows:?}");
    assert!(has("vol.master 0 → -10"), "{rows:?}");
    assert!(has("eq.gain[0,1] 12 → 6"), "{rows:?}");
    assert!(has("limit.on[0] off → on"), "{rows:?}");
    assert!(has("limit.threshold[0] 0 → -6"), "{rows:?}");
    assert!(
        rows.iter()
            .any(|r| r.contains("high-pass at 80 Hz in crossover 1")),
        "{rows:?}"
    );
    // Written to the device, the state is inside the profile: nothing more to
    // hold, and every write the hold made passes the judge on the way.
    for w in &writes {
        assert_eq!(judge(&p, w, Some(&image(&d))), Ok(()), "{w:?}");
        d.control_out(w.b_request, w.w_value, &w.payload).unwrap();
    }
    let after = image(&d);
    let (again, rows) = hold_writes(&p, &after, 1);
    assert!(again.is_empty() && rows.is_empty(), "{rows:?}");
    assert!(p.highpass_kept(0, &after.views(ch, None)));
    assert_eq!(after.out_gain(0), Some(3.0));
    assert_eq!(after.master_volume(), Some(-10.0));
}

#[test]
fn wp_p1hl_no_profile_and_a_compliant_device_hold_nothing() {
    let (d, _) = compliant();
    let (w, rows) = hold_writes(&profile(), &image(&d), 1);
    assert!(w.is_empty() && rows.is_empty(), "{rows:?}");
    let (w, rows) = hold_writes(&LimitsProfile::default(), &image(&FakeDevice::demo()), 1);
    assert!(w.is_empty() && rows.is_empty());
}

#[test]
fn wp_p1hl_the_pin_is_kept_salted_and_slow_and_wrong_pins_wait() {
    let l = HubLimits::new(None);
    assert!(!l.pin_set("S"));
    // No PIN: unlocking passes, nothing is locked.
    assert!(l.unlock("S", 5, "anything", Instant::now()).is_ok());
    assert!(matches!(
        l.set_pin("S", Some("12")),
        Err(PinRefused::BadPin)
    ));
    l.set_pin("S", Some("4711")).unwrap();
    let rec = guard(&l.state).stored.pins["S"].clone();
    assert_eq!(rec.kdf, "pbkdf2-sha256");
    assert!(rec.iterations >= 100_000);
    assert!(!rec.hash.contains("4711") && !rec.salt.is_empty());
    // Another device's record never matches this one's PIN.
    assert!(!rec.matches("T", "4711"));
    let t0 = Instant::now();
    for _ in 0..PIN_TRIES {
        assert!(matches!(
            l.unlock("S", 5, "0000", t0),
            Err(PinRefused::Wrong)
        ));
    }
    assert!(matches!(
        l.unlock("S", 5, "4711", t0),
        Err(PinRefused::Later(_))
    ));
    let later = t0 + PIN_WINDOW;
    l.unlock("S", 5, "4711", later).unwrap();
    assert!(l.unlocked("S", 5, Role::Control));
    assert!(!l.unlocked("S", 6, Role::Control));
    assert!(l.unlocked("S", 6, Role::Admin), "an Admin needs no PIN");
    l.session_closed(5);
    assert!(!l.unlocked("S", 5, Role::Control));
}

#[test]
fn wp_p1hl_a_device_is_locked_only_with_a_pin_and_a_covered_page() {
    let l = HubLimits::new(None);
    l.set_profile("S", Some(profile())).unwrap();
    assert!(!l.device("S").locked, "a profile alone");
    l.set_pin("S", Some("4711")).unwrap();
    assert!(!l.device("S").locked, "a PIN and nothing covered");
    let mut p = profile();
    p.locked_pages = vec!["eq".into()];
    p.serial = "another".into();
    l.set_profile("S", Some(p)).unwrap();
    assert!(l.device("S").locked);
    assert!(l.locked_for("S", 9, Role::Control));
    assert!(!l.locked_for("S", 9, Role::Admin));
    l.set_pin("S", None).unwrap();
    assert!(!l.device("S").locked, "clearing the PIN removes the lock");
    // The profile's serial is the device's, whatever it said.
    assert_eq!(l.profile("S").unwrap().serial, "S");
}

#[test]
fn wp_p1hl_the_store_keeps_limits_leniently_and_never_a_pin() {
    let dir = crate::store::tests_wp_hb1::scratch("p1hl");
    let mut store = crate::store::HubStore::load(&dir).unwrap();
    let mut data = StoredLimits::default();
    data.profiles.insert("S".into(), profile());
    data.pins.insert(
        "S".into(),
        PinRecord::make("S", "pin-Zq83-wp-p1hl", &[7; 16], 1000),
    );
    data.modes.insert("control".into(), "simple".into());
    store.set_limits(&data);
    store.save().unwrap();
    let text = std::fs::read_to_string(dir.join("hub-store.json")).unwrap();
    assert!(!text.contains("pin-Zq83-wp-p1hl"));
    assert!(text.contains("device_pins") && text.contains("device_limits"));
    let back = crate::store::HubStore::load(&dir).unwrap();
    assert_eq!(back.limits(), data);
    let id = back.hub_id().to_owned();
    // A hand-edited entry is dropped alone; a bad mode or role too.
    let mut v: Value = serde_json::from_str(&text).unwrap();
    v["device_limits"]["X"] = json!("not a profile");
    v["role_modes"]["boss"] = json!("simple");
    v["role_modes"]["viewer"] = json!("expert");
    std::fs::write(dir.join("hub-store.json"), v.to_string()).unwrap();
    let back = crate::store::HubStore::load(&dir).unwrap();
    assert_eq!(back.limits(), data);
    assert_eq!(back.hub_id(), id, "the store itself kept");
    // An empty set writes nothing.
    let mut store = back;
    store.set_limits(&StoredLimits::default());
    store.save().unwrap();
    let text = std::fs::read_to_string(dir.join("hub-store.json")).unwrap();
    assert!(!text.contains("device_pins") && !text.contains("device_limits"));
}

#[test]
fn wp_p1hl_modes_are_pinned_per_role() {
    let l = HubLimits::new(None);
    assert_eq!(l.pinned_mode(Role::Control), None);
    assert!(l.pin_mode(Role::Control, Some("simple")).unwrap());
    assert!(!l.pin_mode(Role::Viewer, Some("expert")).unwrap());
    assert_eq!(l.pinned_mode(Role::Control).as_deref(), Some("simple"));
    assert_eq!(l.pinned_mode(Role::Viewer), None);
    assert!(l.pin_mode(Role::Control, None).unwrap());
    assert_eq!(l.pinned_mode(Role::Control), None);
}

#[test]
fn wp_p1hl_a_scene_past_a_units_limit_is_refused_before_anything_is_sent() {
    let l = HubLimits::new(None);
    l.set_profile("K", Some(profile())).unwrap();
    let scene = |op: &str, arg: u8, value: i16| crate::system::model::Scene {
        id: "s".into(),
        name: String::new(),
        actions: vec![crate::system::model::SceneAction {
            target: "unit:K".into(),
            op: op.into(),
            arg,
            value,
        }],
        system_mute: None,
    };
    let units = vec![vec!["K".to_owned()]];
    let r = judge_scene(&l, &scene("master_volume", 0, -500), &units, Role::Admin);
    assert!(r.unwrap_err().contains("limit_exceeded"));
    assert!(judge_scene(&l, &scene("master_volume", 0, -2000), &units, Role::Admin).is_ok());
    // 480 samples at 48 kHz: 10 ms, past the 5 ms.
    assert!(judge_scene(&l, &scene("out_delay", 0, 480), &units, Role::Control).is_err());
    assert!(judge_scene(&l, &scene("out_delay", 0, 96), &units, Role::Control).is_ok());
    // A unit without a profile is not judged.
    let other = vec![vec!["Q".to_owned()]];
    assert!(judge_scene(&l, &scene("master_volume", 0, 0), &other, Role::Admin).is_ok());
    // Locked: Control may recall volume, mute and presets only.
    let mut p = profile();
    p.locked_pages = vec!["eq".into()];
    l.set_profile("K", Some(p)).unwrap();
    l.set_pin("K", Some("4711")).unwrap();
    assert!(judge_scene(&l, &scene("out_delay", 0, 96), &units, Role::Control).is_err());
    assert!(judge_scene(&l, &scene("out_delay", 0, 96), &units, Role::Admin).is_ok());
    assert!(judge_scene(&l, &scene("preset_load", 2, 0), &units, Role::Control).is_ok());
}

#[test]
fn wp_p1hl_refusals_carry_the_notice_key() {
    let v = Violation {
        code: "out_gain",
        msg: "output 1 gain 6 dB is above the limit of 3 dB".into(),
    };
    let p: Value = serde_json::from_slice(&Refusal::Limit(v).payload()).unwrap();
    assert_eq!(p["refused"], "limit_exceeded");
    assert_eq!(p["limit"], "out_gain");
    for (r, key) in [
        (Refusal::Locked, "locked"),
        (Refusal::Image, "image_refused"),
        (Refusal::Unknown, "limits_unknown"),
    ] {
        let p: Value = serde_json::from_slice(&r.payload()).unwrap();
        assert_eq!(p["refused"], key);
        let msg = p["msg"].as_str().unwrap().to_lowercase();
        for noun in ["phone", "computer", "tablet"] {
            assert!(!msg.contains(noun), "{msg}");
        }
    }
}
