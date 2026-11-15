//! WP P1LM tests: the profile, its document, the plain-value checks, the
//! files (fallback, atomic writes) and the PIN's hash.
//!
//! Files live under `std::env::temp_dir()`, which `ci/run.sh` points at
//! `~/.cache/dspi-build/work/<branch>/tmp` (never tmpfs).

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};

use super::*;

fn store_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("wp-p1lm-limits").join(format!(
        "{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&d);
    d
}

fn profile() -> LimitsProfile {
    LimitsProfile {
        serial: "DSPI-1".into(),
        outputs: vec![OutputLimits {
            output: 2,
            max_gain_db: Some(3.0),
            highpass_min_hz: Some(1500.0),
            highpass_min_slope_db: Some(24),
            delay_min_ms: Some(1.0),
            delay_max_ms: Some(5.0),
            limiter_ceiling_db: Some(-3.0),
        }],
        max_peq_boost_db: Some(6.0),
        master_volume_max_db: Some(-10.0),
        locked_pages: vec!["outputs".into()],
        note: "Tweeters on 3 and 4".into(),
        contact: "installer@example.org".into(),
    }
}

#[test]
fn wp_p1lm_normalise_holds_a_gain_limit_to_the_registry() {
    let p = LimitsProfile {
        outputs: vec![
            OutputLimits {
                output: 0,
                max_gain_db: Some(40.0),
                delay_min_ms: Some(9.0),
                delay_max_ms: Some(2.0),
                limiter_ceiling_db: Some(f32::NAN),
                highpass_min_slope_db: Some(200),
                ..Default::default()
            },
            OutputLimits {
                output: 0,
                max_gain_db: Some(25.0),
                ..Default::default()
            },
        ],
        max_peq_boost_db: Some(-3.0),
        ..Default::default()
    }
    .normalised();
    assert_eq!(p.outputs.len(), 1, "an output named twice: the last wins");
    let o = p.output(0).unwrap();
    assert_eq!(
        o.max_gain_db,
        Some(20.0),
        "never above the registry's +20 dB"
    );
    let p0 = LimitsProfile {
        outputs: vec![OutputLimits {
            output: 1,
            max_gain_db: Some(40.0),
            delay_min_ms: Some(9.0),
            delay_max_ms: Some(2.0),
            limiter_ceiling_db: Some(f32::NAN),
            highpass_min_slope_db: Some(200),
            ..Default::default()
        }],
        ..Default::default()
    }
    .normalised();
    let o = p0.output(1).unwrap();
    assert_eq!((o.delay_min_ms, o.delay_max_ms), (Some(2.0), Some(9.0)));
    assert_eq!(o.limiter_ceiling_db, None, "NaN is no limit");
    assert_eq!(o.highpass_min_slope_db, Some(48));
    assert_eq!(p.max_peq_boost_db, Some(0.0));
}

#[test]
fn wp_p1lm_checks_refuse_past_each_limit_and_pass_within() {
    let p = profile().normalised();
    assert!(p.check_output_gain(2, 3.0).is_ok());
    assert_eq!(p.check_output_gain(2, 3.1).unwrap_err().code, "out_gain");
    assert!(p.check_output_gain(0, 20.0).is_ok(), "an output not named");
    assert_eq!(p.check_delay(2, 0.5).unwrap_err().code, "delay");
    assert_eq!(p.check_delay(2, 5.5).unwrap_err().code, "delay");
    assert!(p.check_delay(2, 4.0).is_ok());
    assert_eq!(
        p.check_master_volume(-5.0).unwrap_err().code,
        "master_volume"
    );
    assert!(
        p.check_master_volume(-128.0).is_ok(),
        "muting always passes"
    );
    assert!(p.check_master_volume(-10.0).is_ok());
    assert_eq!(p.check_boost(1, 7.0).unwrap_err().code, "eq_boost");
    assert!(p.check_boost(1, 6.0).is_ok());
    assert!(
        p.check_boost(11, 900.0).is_ok(),
        "a Linkwitz gain is a frequency"
    );
    assert!(p.check_boost(5, 12.0).is_ok(), "a high-pass does not boost");
    assert_eq!(p.check_limiter(2, false, -6.0).unwrap_err().code, "limiter");
    assert_eq!(p.check_limiter(2, true, -1.0).unwrap_err().code, "limiter");
    assert!(p.check_limiter(2, true, -3.0).is_ok());
    assert!(p.check_limiter(1, false, 0.0).is_ok());
    assert_eq!(p.clamp_output_gain(2, 10.0), 3.0);
    assert_eq!(p.clamp_delay(2, 9.0), 5.0);
    assert_eq!(p.clamp_delay(2, 0.0), 1.0);
    assert_eq!(p.clamp_master_volume(0.0), -10.0);
    assert_eq!(p.clamp_boost(2, 12.0), 6.0);
    assert_eq!(p.clamp_boost(11, 900.0), 900.0);
}

#[test]
fn wp_p1lm_highpass_slopes_and_the_kept_rule() {
    assert_eq!(highpass_slope(5), Some(12));
    assert_eq!(highpass_slope(13), Some(6));
    assert_eq!(highpass_slope(33), Some(12)); // LR2 HP
    assert_eq!(highpass_slope(35), Some(24)); // LR4 HP
    assert_eq!(highpass_slope(39), Some(48)); // LR8 HP
    assert_eq!(highpass_slope(41), Some(6)); // BW1 HP
    assert_eq!(highpass_slope(55), Some(48)); // BW8 HP
    assert_eq!(highpass_slope(57), Some(12)); // Bessel 2 HP
    assert_eq!(highpass_slope(63), Some(48)); // Bessel 8 HP
    assert_eq!(highpass_slope(34), None, "LR4 low-pass");
    assert_eq!(highpass_slope(4), None, "PEQ low-pass");
    for s in [6, 12, 13, 24, 36, 48] {
        assert!(highpass_slope(lr_highpass_for(s)).unwrap() >= s.max(12));
    }
    let p = profile().normalised();
    let hp = |t: u8, f: f32, bypass: bool| BandView {
        filter_type: t,
        bypass,
        freq: f,
    };
    let good = [hp(1, 100.0, false), hp(35, 1800.0, false)];
    assert!(p.highpass_kept(2, &good));
    assert!(!p.highpass_kept(2, &[hp(35, 1000.0, false)]), "too low");
    assert!(!p.highpass_kept(2, &[hp(33, 2000.0, false)]), "too shallow");
    assert!(!p.highpass_kept(2, &[hp(35, 2000.0, true)]), "bypassed");
    assert!(p.highpass_kept(0, &[]), "an output not named");
    let lowered = [hp(1, 100.0, false), hp(35, 900.0, false)];
    assert_eq!(
        p.check_highpass(2, &good, &lowered).unwrap_err().code,
        "highpass"
    );
    // Already outside: a change is not refused for what was so before.
    assert!(p.check_highpass(2, &lowered, &lowered).is_ok());
}

#[test]
fn wp_p1lm_the_document_round_trips_and_carries_no_pin() {
    let p = profile().normalised();
    let text = p.to_document();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["meta"]["kind"], "limits");
    assert_eq!(from_document(&text).unwrap(), p);
    assert!(!text.to_lowercase().contains("pin\""), "{text}");
    // Carried in an exported document's meta.
    let mut export = json!({"schemaVersion": 1, "meta": {"name": "x"}, "channels": []});
    p.embed(&mut export);
    assert_eq!(from_value(&export).unwrap(), p);
    // A preset without one, and things that are not documents.
    assert_eq!(
        from_value(&json!({"schemaVersion": 1, "meta": {}})),
        Err(LimitsError::NoProfile)
    );
    assert!(matches!(
        from_document("{"),
        Err(LimitsError::Unreadable(_))
    ));
    assert!(matches!(
        from_document(r#"{"meta":{"kind":"limits"},"limits":{"outputs":"x"}}"#),
        Err(LimitsError::Unreadable(_))
    ));
    // A limits block without the kind word is not taken as a profile.
    assert_eq!(
        from_value(&json!({"meta": {"kind": "preset"}, "limits": {}})),
        Err(LimitsError::NoProfile)
    );
}

#[test]
fn wp_p1lm_hmac_and_pbkdf2_match_the_published_vectors() {
    // RFC 4231 test case 2.
    let mac = kdf::hmac_for_test(b"Jefe", b"what do ya want for nothing?");
    assert_eq!(
        hex(&mac),
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
    );
    // PBKDF2-HMAC-SHA256, P="password", S="salt", c=1 and c=2 (RFC 7914's
    // companion vectors, first 32 bytes).
    let one = PinRecord::make("", "password", b"salt", 1);
    assert_eq!(
        one.hash,
        "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
    );
    let two = PinRecord::make("", "password", b"salt", 2);
    assert_eq!(
        two.hash,
        "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43"
    );
}

#[test]
fn wp_p1lm_a_pin_record_is_salted_per_device() {
    let a = PinRecord::make("DSPI-1", "1234", &[7u8; 16], 50);
    let b = PinRecord::make("DSPI-2", "1234", &[7u8; 16], 50);
    assert_ne!(a.hash, b.hash, "the serial is part of the salt");
    assert!(a.matches("DSPI-1", "1234"));
    assert!(!a.matches("DSPI-1", "1235"));
    assert!(!a.matches("DSPI-2", "1234"), "another device's record");
    let mut torn = a.clone();
    torn.hash.truncate(10);
    assert!(!torn.matches("DSPI-1", "1234"));
}

#[test]
fn wp_p1lm_store_saves_loads_and_clears() {
    let store = LimitsStore::new(store_dir("save"));
    assert_eq!(store.load("DSPI-1").state, LoadState::None);
    let p = profile().normalised();
    store.save(&p).unwrap();
    let l = store.load("DSPI-1");
    assert_eq!(l.state, LoadState::Ok);
    assert_eq!(l.profile, Some(p.clone()));
    // Another device has none.
    assert_eq!(store.load("DSPI-2").profile, None);
    store.clear("DSPI-1").unwrap();
    assert_eq!(store.load("DSPI-1").state, LoadState::None);
}

#[test]
fn wp_p1lm_a_corrupt_profile_falls_back_to_the_last_good_copy() {
    let store = LimitsStore::new(store_dir("corrupt"));
    let p = profile().normalised();
    store.save(&p).unwrap();
    let dir = store.dir("DSPI-1");
    // A torn file (cut mid-way) and a hand edit of the wrong shape.
    for bad in [
        &p.to_document()[..40],
        r#"{"meta":{"kind":"limits"},"limits":[]}"#,
    ] {
        fs::write(dir.join(PROFILE_FILE), bad).unwrap();
        let l = store.load("DSPI-1");
        assert!(
            matches!(l.state, LoadState::FellBack { .. }),
            "{:?}",
            l.state
        );
        assert_eq!(l.profile, Some(p.clone()), "fails closed");
    }
    // Missing: also the good copy.
    fs::remove_file(dir.join(PROFILE_FILE)).unwrap();
    assert!(matches!(
        store.load("DSPI-1").state,
        LoadState::FellBack { .. }
    ));
    // No good copy either: unlimited, and said so.
    fs::write(dir.join(PROFILE_FILE), "not json").unwrap();
    fs::remove_file(dir.join(GOOD_FILE)).unwrap();
    let l = store.load("DSPI-1");
    assert!(matches!(l.state, LoadState::Unreadable { .. }));
    assert_eq!(l.profile, None);
    // Binary junk never panics.
    fs::write(dir.join(PROFILE_FILE), [0xffu8, 0, 0xfe, 0x80]).unwrap();
    assert!(matches!(
        store.load("DSPI-1").state,
        LoadState::Unreadable { .. }
    ));
}

#[test]
fn wp_p1lm_a_torn_write_leaves_the_previous_file() {
    let store = LimitsStore::new(store_dir("torn"));
    let p = profile().normalised();
    store.save(&p).unwrap();
    let dir = store.dir("DSPI-1");
    // A write that died before its rename leaves only its temporary file.
    fs::write(
        dir.join(format!("{PROFILE_FILE}.tmp-{}", std::process::id())),
        "{\"meta\":{\"kin",
    )
    .unwrap();
    let l = store.load("DSPI-1");
    assert_eq!(l.state, LoadState::Ok);
    assert_eq!(l.profile, Some(p.clone()));
    // A completed write leaves nothing behind but its own files.
    let mut q = p.clone();
    q.note = "second".into();
    store.save(&q).unwrap();
    assert_eq!(store.load("DSPI-1").profile.unwrap().note, "second");
    let names: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| !n.contains(".tmp-"))
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
}

#[test]
fn wp_p1lm_the_pin_file_holds_a_slow_salted_hash_only() {
    // -- WP FLAKE4 -- a PIN the file's hex salt and hash can never hold by chance
    // ("1234" could); '~' is in no hex or number.
    const PIN: &str = "~flake4~1234~";
    let store = LimitsStore::new(store_dir("pin"));
    assert_eq!(store.check_pin("DSPI-1", PIN).unwrap(), None, "no PIN set");
    store.set_pin("DSPI-1", PIN).unwrap();
    assert_eq!(store.check_pin("DSPI-1", PIN).unwrap(), Some(true));
    assert_eq!(store.check_pin("DSPI-1", "4321").unwrap(), Some(false));
    let text = fs::read_to_string(store.dir("DSPI-1").join(PIN_FILE)).unwrap();
    assert!(!text.contains(PIN), "never the PIN itself");
    let rec = store.pin("DSPI-1").unwrap().unwrap();
    assert_eq!(rec.iterations, PIN_ITERATIONS);
    assert_eq!(rec.salt.len(), 32, "16 random bytes");
    // The same PIN again gets another salt.
    store.set_pin("DSPI-1", PIN).unwrap();
    assert_ne!(store.pin("DSPI-1").unwrap().unwrap().salt, rec.salt);
    // The profile's document never holds it.
    store.save(&profile().normalised()).unwrap();
    let doc = fs::read_to_string(store.dir("DSPI-1").join(PROFILE_FILE)).unwrap();
    assert!(!doc.contains(&rec.hash));
    // A corrupt PIN file is an error, never "no PIN".
    fs::write(store.dir("DSPI-1").join(PIN_FILE), "{").unwrap();
    assert!(store.check_pin("DSPI-1", PIN).is_err());
    store.clear_pin("DSPI-1").unwrap();
    assert_eq!(store.check_pin("DSPI-1", PIN).unwrap(), None);
}

/// The Terminal's import path (`preset_file::apply_limited`) holds a
/// document's values to the profile and names each.
#[test]
fn wp_p1lm_a_document_is_held_to_the_profile_and_reported() {
    let text = json!({
        "schemaVersion": 1,
        "meta": {"name": "loud"},
        "global": {"masterVolumeDb": 0.0},
        "channels": [
            {"channelId": 0, "isOutput": false, "inputIndex": 0,
             "eq": [{"type": 1, "freqHz": 1000.0, "q": 1.0, "gain": 12.0}]},
            {"channelId": 9, "isOutput": true, "outputIndex": 2, "gainDb": 10.0,
             "outputDelayMs": 9.0,
             "crossover": [{"type": 35, "freqHz": 800.0, "q": 0.707, "gain": 0.0}],
             "limiter": {"enabled": false, "thresholdDb": 0.0}},
            {"channelId": 10, "isOutput": true, "outputIndex": 3, "gainDb": 10.0}
        ]
    })
    .to_string();
    let mut doc = crate::preset_file::parse(&text).unwrap();
    let rows = crate::preset_file::hold_document(&profile().normalised(), &mut doc, 2, 9);
    let out2 = doc
        .channels
        .iter()
        .find(|c| c.output_index == Some(2))
        .unwrap();
    assert_eq!(out2.gain_db, Some(3.0));
    assert_eq!(out2.output_delay_ms, Some(5.0));
    assert_eq!(out2.crossover[0].freq_hz, 1500.0);
    let lim = out2.limiter.as_ref().unwrap();
    assert_eq!((lim.enabled, lim.threshold_db), (Some(true), Some(-3.0)));
    let out3 = doc
        .channels
        .iter()
        .find(|c| c.output_index == Some(3))
        .unwrap();
    assert_eq!(out3.gain_db, Some(10.0), "an output not named");
    assert_eq!(doc.channels[0].eq[0].gain, 6.0);
    assert_eq!(doc.global.master_volume_db, Some(-10.0));
    for want in [
        "out.gain[2] 10 → 3 dB",
        "eq.gain[0,0] 12 → 6 dB",
        "limit.on[2] off → on",
    ] {
        assert!(rows.iter().any(|r| r == want), "{want}: {rows:?}");
    }
    let report = crate::preset_file::ApplyReport {
        limited: rows,
        ..Default::default()
    };
    assert!(!report.is_clean());
    assert!(
        report
            .lines(false)
            .iter()
            .any(|l| l.starts_with("Held to the limits profile: ")),
        "{:?}",
        report.lines(false)
    );
}
