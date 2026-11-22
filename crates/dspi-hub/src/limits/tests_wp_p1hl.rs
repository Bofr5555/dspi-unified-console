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
    // A short payload fails closed (-- WP P1DKFIX -- F1).
    assert_eq!(
        refused(judge(
            &p,
            &set(op::REQ_SET_OUTPUT_GAIN, 0, vec![1, 2]),
            None
        )),
        "out_gain"
    );
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
    // -- WP FLAKE4 -- a PIN the hex hash can never hold by chance ("4711" could).
    const PIN: &str = "~flake4~4711~";
    let l = HubLimits::new(None);
    assert!(!l.pin_set("S"));
    // No PIN: unlocking passes, nothing is locked.
    assert!(l.unlock("S", 5, "anything", Instant::now()).is_ok());
    assert!(matches!(
        l.set_pin("S", Some("12")),
        Err(PinRefused::BadPin)
    ));
    l.set_pin("S", Some(PIN)).unwrap();
    let rec = guard(&l.state).stored.pins["S"].clone();
    assert_eq!(rec.kdf, "pbkdf2-sha256");
    assert!(rec.iterations >= 100_000);
    assert!(!rec.hash.contains(PIN) && !rec.salt.is_empty());
    assert!(rec.hash.len() == 64 && rec.hash.bytes().all(|b| b.is_ascii_hexdigit()));
    // Another device's record never matches this one's PIN.
    assert!(!rec.matches("T", PIN));
    let t0 = Instant::now();
    // -- WP P1DKFIX -- the core's rule: the free tries, then the window.
    for _ in 0..=LOCK_FREE_TRIES {
        assert!(matches!(
            l.unlock("S", 5, "0000", t0),
            Err(PinRefused::Wrong)
        ));
    }
    assert!(matches!(
        l.unlock("S", 5, PIN, t0),
        Err(PinRefused::Later(_))
    ));
    let later = t0 + Duration::from_millis(LOCK_FIRST_WAIT_MS);
    l.unlock("S", 5, PIN, later).unwrap();
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
            source: None, // -- WP FEAT1 --
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
        (Refusal::Locked, "installer_lock"),
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

// ── WP P1DKFIX: review p1d-core's probes, adopted as tests of the fixes ─────

/// F3: wrong PINs sent at once are counted before the hash, so no more reach
/// it than the free tries and the one that opens the window.
#[test]
fn wp_p1dkfix_parallel_wrong_pins_are_counted_before_the_hash() {
    let l = Arc::new(HubLimits::new(None));
    l.set_pin("S", Some("4711")).unwrap();
    let t0 = Instant::now();
    let threads: Vec<_> = (0..20u16)
        .map(|i| {
            let l = Arc::clone(&l);
            std::thread::spawn(move || {
                matches!(l.unlock("S", i, "0000", t0), Err(PinRefused::Wrong))
            })
        })
        .collect();
    let checked = threads
        .into_iter()
        .map(|t| t.join().unwrap())
        .filter(|&w| w)
        .count();
    assert_eq!(
        checked,
        LOCK_FREE_TRIES as usize + 1,
        "{checked} wrong PINs reached the hash"
    );
    // The right PIN inside the window is refused unchecked too.
    assert!(matches!(
        l.unlock("S", 1, "4711", t0),
        Err(PinRefused::Later(_))
    ));
}

/// F4: the window doubles with each wrong PIN past the free ones, to 15 min.
#[test]
fn wp_p1dkfix_the_refusal_window_grows_to_fifteen_minutes() {
    let l = HubLimits::new(None);
    l.set_pin("S", Some("4711")).unwrap();
    let mut now = Instant::now();
    for _ in 0..LOCK_FREE_TRIES {
        assert!(matches!(
            l.unlock("S", 1, "0000", now),
            Err(PinRefused::Wrong)
        ));
        assert_eq!(l.tries("S", now).1, None, "a free try opens no window");
    }
    let mut windows = Vec::new();
    for _ in 0..8 {
        assert!(matches!(
            l.unlock("S", 1, "0000", now),
            Err(PinRefused::Wrong)
        ));
        let Err(PinRefused::Later(d)) = l.unlock("S", 1, "0000", now) else {
            panic!("no window");
        };
        windows.push(d.as_millis() as u64);
        now += d;
    }
    let s = LOCK_FIRST_WAIT_MS;
    assert_eq!(
        windows,
        [
            s,
            2 * s,
            4 * s,
            8 * s,
            16 * s,
            LOCK_MAX_WAIT_MS,
            LOCK_MAX_WAIT_MS,
            LOCK_MAX_WAIT_MS
        ]
    );
    // A right PIN after the window clears the count.
    l.unlock("S", 1, "4711", now).unwrap();
    assert_eq!(l.tries("S", now), (0, None));
}

/// F8 (decided): the count is per device, whichever session guessed; an Admin
/// is never held by it.
#[test]
fn wp_p1dkfix_the_count_is_per_device_and_an_admin_is_unaffected() {
    let l = HubLimits::new(None);
    l.set_pin("S", Some("4711")).unwrap();
    let t0 = Instant::now();
    for i in 0..=LOCK_FREE_TRIES {
        let _ = l.unlock("S", u16::try_from(i).unwrap(), "0000", t0);
    }
    assert!(matches!(
        l.unlock("S", 99, "4711", t0),
        Err(PinRefused::Later(_))
    ));
    assert!(l.unlocked("S", 99, Role::Admin));
}

/// F6: the count and the window survive a restart of the hub. -- WP FLAKE2 -- on a
/// clock the test sets: the five wrong PINs are hashed in real time, and on a loaded
/// host they took longer than the 5 s the window was allowed to shrink by.
#[test]
fn wp_p1dkfix_the_wrong_pin_window_survives_a_restart() {
    #[derive(Default)]
    struct Mem(Mutex<StoredLimits>);
    impl LimitsPersist for Mem {
        fn load(&self) -> StoredLimits {
            self.0.lock().unwrap().clone()
        }
        fn save(&self, data: &StoredLimits) -> Result<(), HubError> {
            *self.0.lock().unwrap() = data.clone();
            Ok(())
        }
    }
    let t0 = Instant::now();
    let w0 = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let at = Arc::new(Mutex::new((t0, w0)));
    let clock: Clock = {
        let at = Arc::clone(&at);
        Arc::new(move || *at.lock().unwrap())
    };
    let mem = Arc::new(Mem::default());
    let l = HubLimits::with_clock(Some(mem.clone()), Arc::clone(&clock));
    l.set_pin("S", Some("4711")).unwrap();
    for _ in 0..=LOCK_FREE_TRIES {
        let _ = l.unlock("S", 1, "0000", t0);
    }
    let kept = mem.load().tries["S"];
    assert_eq!(kept.wrong, LOCK_FREE_TRIES + 1);
    assert_eq!(kept.until_unix_ms, 1_800_000_000_000 + LOCK_FIRST_WAIT_MS);
    drop(l);
    // The restart, 1.5 s later on the wall clock, with a new monotonic origin.
    let t1 = t0 + Duration::from_secs(3600);
    *at.lock().unwrap() = (t1, w0 + Duration::from_millis(1_500));
    let again = HubLimits::with_clock(Some(mem.clone()), Arc::clone(&clock));
    let (wrong, left) = again.tries("S", t1);
    assert_eq!(wrong, LOCK_FREE_TRIES + 1);
    assert_eq!(
        left,
        Some(Duration::from_millis(LOCK_FIRST_WAIT_MS - 1_500)),
        "the window survives"
    );
    assert!(matches!(
        again.unlock("S", 1, "4711", t1),
        Err(PinRefused::Later(_))
    ));
    // A window kept far in the future (a clock set back) is never longer
    // than the longest.
    let mut far = mem.load();
    far.tries.get_mut("S").unwrap().until_unix_ms = u64::MAX;
    *mem.0.lock().unwrap() = far;
    let capped = HubLimits::with_clock(Some(mem.clone()), clock);
    assert!(capped.tries("S", t1).1.unwrap() <= Duration::from_millis(LOCK_MAX_WAIT_MS));
    // A PIN set anew clears it, in the store too.
    capped.set_pin("S", Some("2468")).unwrap();
    assert!(!mem.load().tries.contains_key("S"));
}

/// F1: an index the firmware truncates (`wValue & 0xFF`) is judged as the
/// output it reaches: output 0x0100 is output 0.
#[test]
fn wp_p1dkfix_an_output_index_past_255_is_judged_as_the_firmware_reads_it() {
    let p = profile();
    for (what, code) in [
        ("gain", op::REQ_SET_OUTPUT_GAIN),
        ("delay", op::REQ_SET_OUTPUT_DELAY),
    ] {
        for w_value in [0x0100, 0xFF00, 0xAB00] {
            let r = judge(&p, &set(code, w_value, f(10.0)), None);
            assert!(matches!(r, Err(Ok(_))), "{what} to {w_value:#06x}: {r:?}");
        }
        // Another output's low byte is another output, judged as such.
        assert!(
            judge(&p, &set(code, 0x0101, f(10.0)), None).is_ok(),
            "{what}"
        );
    }
    // A channel delay: the low byte is the wire channel.
    let (d, _) = compliant();
    let img = image(&d);
    let ch = u16::from(img.n_in());
    let r = judge(
        &p,
        &set(op::REQ_SET_DELAY, 0x0100 | ch, f(10.0)),
        Some(&img),
    );
    assert!(matches!(r, Err(Ok(_))), "{r:?}");
}

/// F1: a write the judge cannot read is refused, never passed: a short
/// payload, a value that is not a number.
#[test]
fn wp_p1dkfix_an_unreadable_write_fails_closed() {
    let p = profile();
    let (d, _) = compliant();
    let img = image(&d);
    let cases: Vec<(&str, CmdRequest)> = vec![
        ("short gain", set(op::REQ_SET_OUTPUT_GAIN, 0, vec![0, 0])),
        ("NaN gain", set(op::REQ_SET_OUTPUT_GAIN, 0, f(f32::NAN))),
        (
            "inf delay",
            set(op::REQ_SET_OUTPUT_DELAY, 0, f(f32::INFINITY)),
        ),
        ("NaN delay", set(op::REQ_SET_DELAY, 0, f(f32::NAN))),
        ("short master", set(op::REQ_SET_MASTER_VOLUME, 0, vec![])),
        ("NaN master", set(op::REQ_SET_MASTER_VOLUME, 0, f(f32::NAN))),
        ("short EQ", set(op::REQ_SET_EQ_PARAM, 0, vec![0; 15])),
        (
            "NaN EQ gain",
            set(op::REQ_SET_EQ_PARAM, 0, {
                let mut b = packet(0, 0, PEAKING, 1000.0, 0.0, false).encode();
                b[12..16].copy_from_slice(&f32::NAN.to_le_bytes());
                b
            }),
        ),
        ("empty bypass", set(op::REQ_SET_BAND_BYPASS, 0, vec![])),
        ("short limiter", set(op::REQ_LIMITER, 0, vec![1])),
    ];
    for (what, req) in cases {
        let r = judge(&p, &req, Some(&img));
        assert!(matches!(r, Err(Ok(_))), "{what}: {r:?}");
    }
    // The same writes well formed and within the limits pass.
    assert!(judge(&p, &set(op::REQ_SET_OUTPUT_GAIN, 0, f(1.0)), Some(&img)).is_ok());
    assert!(judge(&p, &set(op::REQ_SET_MASTER_VOLUME, 0, f(-20.0)), Some(&img)).is_ok());
}

/// F1 audit: a limiter link group moves settings (joining a group adopts
/// its settings), so it is judged as the firmware applies it.
#[test]
fn wp_p1dkfix_a_limiter_link_group_cannot_switch_off_a_kept_limiter() {
    let p = profile();
    let (mut d, _) = compliant();
    // Output 1: limiter off, in link group 2. Output 0 (kept on at -10 dBFS
    // under a -6 ceiling) joining group 2 would adopt "off".
    d.control_out(
        op::REQ_LIMITER,
        (1 << 8) | limiter::LIMITER_PARAM_LINK_GROUP,
        &f(2.0),
    )
    .unwrap();
    d.control_out(
        op::REQ_LIMITER,
        (1 << 8) | limiter::LIMITER_PARAM_ENABLED,
        &f(0.0),
    )
    .unwrap();
    let img = image(&d);
    assert_eq!(img.limiter(0).map(|l| l.0), Some(true));
    assert_eq!(img.limiter(1).map(|l| (l.0, l.1)), Some((false, 2)));
    let join = set(op::REQ_LIMITER, limiter::LIMITER_PARAM_LINK_GROUP, f(2.0));
    assert!(matches!(judge(&p, &join, Some(&img)), Err(Ok(v)) if v.code == "limiter"));
    // A group value past the largest is clamped to it, as the firmware does.
    d.control_out(
        op::REQ_LIMITER,
        (1 << 8) | limiter::LIMITER_PARAM_LINK_GROUP,
        &f(4.0),
    )
    .unwrap();
    let img = image(&d);
    let join = set(op::REQ_LIMITER, limiter::LIMITER_PARAM_LINK_GROUP, f(9.0));
    assert!(matches!(judge(&p, &join, Some(&img)), Err(Ok(_))));
    // Every output into one group: each adopts the first member's (output 0's,
    // on at -10): passes.
    let all = set(
        op::REQ_LIMITER,
        (u16::from(ALL_OUTPUTS) << 8) | limiter::LIMITER_PARAM_LINK_GROUP,
        f(3.0),
    );
    assert!(judge(&p, &all, Some(&img)).is_ok());
    // A group of its own (no peer): nothing adopted, passes.
    let alone = set(op::REQ_LIMITER, limiter::LIMITER_PARAM_LINK_GROUP, f(1.0));
    assert!(judge(&p, &alone, Some(&img)).is_ok());
    // The release moves nothing a profile limits.
    let release = set(op::REQ_LIMITER, limiter::LIMITER_PARAM_RELEASE_MS, f(500.0));
    assert!(judge(&p, &release, Some(&img)).is_ok());
}

// -- WP FEAT1 --
#[test]
fn wp_feat1_a_source_switch_on_a_locked_unit_is_for_the_installer() {
    let l = HubLimits::new(None);
    let scene = crate::system::model::Scene {
        id: "s".into(),
        name: String::new(),
        actions: vec![crate::system::model::SceneAction {
            target: "zone:z".into(),
            op: "source".into(),
            arg: 0,
            value: 0,
            source: Some("radio".into()),
        }],
        system_mute: None,
    };
    let units = vec![vec!["K".to_owned()]];
    // Unlocked, or no profile: anyone who may recall it.
    assert!(judge_scene(&l, &scene, &units, Role::Control).is_ok());
    l.set_profile("K", Some(profile())).unwrap();
    assert!(judge_scene(&l, &scene, &units, Role::Control).is_ok());
    // Locked: not volume, mute or a preset, so not for a control role.
    let mut p = profile();
    p.locked_pages = vec!["eq".into()];
    l.set_profile("K", Some(p)).unwrap();
    l.set_pin("K", Some("4711")).unwrap();
    let e = judge_scene(&l, &scene, &units, Role::Control).unwrap_err();
    assert!(
        e.contains("op source on zone:z") && e.contains("locked"),
        "{e}"
    );
    assert!(judge_scene(&l, &scene, &units, Role::Admin).is_ok());
}
