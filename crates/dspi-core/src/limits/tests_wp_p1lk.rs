//! WP P1LK: the installer's PIN and lock on the demo device. Files live
//! under `std::env::temp_dir()`, which `ci/run.sh` points at the home volume
//! (never tmpfs).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use dspi_session::limits::{LimitsStore, PIN_FILE, RECOVERY_FILE};

use super::*;
use crate::consts::IMPORT_ALL_BLOCKS;
use crate::notice_text::notice_for;
use crate::write_param::ParamValue;

const PIN: &str = "2468";

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    core
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("wp-p1lk-core").join(format!(
        "{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn code_of(e: &DspiError) -> &str {
    match e {
        DspiError::InstallerLock { code, .. } => code,
        _ => "",
    }
}

/// A PIN set, the lock covering the front page, and the console locked.
fn locked(core: &DspiCore) -> String {
    let set = core.installer_pin_set(PIN.into()).unwrap();
    core.installer_lock_cover(vec!["front".into()]).unwrap();
    let st = core.installer_lock().unwrap();
    assert!(st.locked && st.pin_set && !st.unlocked, "{st:?}");
    set.recovery_code
}

/// Every kind of write but volume, mute and preset, each `Err` or `Ok`.
fn others(core: &DspiCore) -> Vec<(&'static str, Result<()>)> {
    let ch = core.get_state().num_input_channels;
    let band = FilterParams {
        filter_type: 1,
        bypass: false,
        freq: 1000.0,
        q: 1.0,
        gain: 1.0,
        qp: 0.707,
    };
    let import = |core: &DspiCore| {
        let doc = core.config_export("x".into(), "t".into(), 0).unwrap();
        let r: serde_json::Value =
            serde_json::from_str(&core.config_import(doc, IMPORT_ALL_BLOCKS)).unwrap();
        if r["ok"] == true {
            Ok(())
        } else {
            Err(DspiError::InstallerLock {
                code: r["noticeKey"]
                    .as_str()
                    .and_then(|k| k.strip_prefix("installer_lock."))
                    .unwrap_or("")
                    .to_owned(),
                msg: r["error"].as_str().unwrap_or("").to_owned(),
            })
        }
    };
    vec![
        ("set_output_gain", core.set_output_gain(1, 1.5)),
        ("set_filter", core.set_filter(ch, 0, band)),
        ("set_output_delay", core.set_output_delay(1, 1.0)),
        ("set_channel_name", core.set_channel_name(ch, "Left".into())),
        ("set_device_name", core.set_device_name("Room".into())),
        (
            "write_param out.gain",
            core.write_param("out.gain".into(), vec![1], ParamValue::Float(2.0))
                .map(|_| ()),
        ),
        ("set_master_volume_mode", core.set_master_volume_mode(1)),
        ("apply_all_params", core.apply_all_params()),
        ("config_import", import(core)),
        ("save_preset", core.save_preset(1)),
        ("factory_reset", core.factory_reset()),
    ]
}

/// The writes the locked-user state allows.
fn allowed(core: &DspiCore) -> Vec<(&'static str, Result<()>)> {
    vec![
        ("set_master_volume", core.set_master_volume(-20.0)),
        ("set_user_volume", core.set_user_volume(-12.0)),
        ("set_user_mute", core.set_user_mute(true)),
        ("set_output_mute", core.set_output_mute(0, true)),
        ("load_preset", core.load_preset(0)),
        (
            "write_param vol.master",
            core.write_param("vol.master".into(), vec![], ParamValue::Float(-25.0))
                .map(|_| ()),
        ),
        (
            "write_param vol.mute",
            core.write_param("vol.mute".into(), vec![], ParamValue::Bool(false))
                .map(|_| ()),
        ),
    ]
}

#[test]
fn wp_p1lk_no_pin_every_mode_and_every_write_no_prompt() {
    let core = demo();
    let st = core.installer_lock_state();
    assert!(!st.pin_set && !st.locked && st.installer_open, "{st:?}");
    for platform in ["phone", "desktop", "web"] {
        for mode in ["simple", "advanced", "installer"] {
            let c = core.mode_resolve(platform.into(), mode.into()).unwrap();
            assert_eq!(c.mode, mode, "{platform} {c:?}");
            assert!(!c.needs_pin && c.switchable, "{c:?}");
            assert_eq!(c.open_modes, ["simple", "advanced", "installer"]);
        }
    }
    // Nothing asks for a PIN: an unlock with anything passes, unchanged.
    assert_eq!(core.installer_unlock("whatever".into()).unwrap(), st);
    assert_eq!(code_of(&core.installer_lock().unwrap_err()), "no_pin");
    for (what, r) in allowed(&core).into_iter().chain(others(&core)) {
        assert!(r.is_ok(), "{what}: {r:?}");
    }
    // A lock's cover without a PIN locks nothing.
    core.installer_lock_cover(vec!["front".into()]).unwrap();
    assert!(!core.installer_lock_state().locked);
    assert!(core.set_output_gain(1, 0.5).is_ok());
}

#[test]
fn wp_p1lk_locked_allows_volume_mute_preset_and_refuses_the_rest() {
    let core = demo();
    core.set_output_gain(1, 0.5).unwrap(); // an entry to undo
    locked(&core);
    // An undo of a write the lock refuses is refused too, as `locked`, and
    // the entry stays (-- WP P1DKFIX -- F10: the refusal itself asserted,
    // before a preset load ends the history).
    let before = core.get_state();
    let e = core.undo().unwrap_err();
    assert_eq!(code_of(&e), "locked", "{e:?}");
    assert_eq!(core.get_state().output_gain_db, before.output_gain_db);
    let e = core.undo().unwrap_err();
    assert_eq!(code_of(&e), "locked", "still there to undo: {e:?}");
    for (what, r) in allowed(&core) {
        assert!(r.is_ok(), "{what}: {r:?}");
    }
    let before = core.get_state();
    for (what, r) in others(&core) {
        let e = r.expect_err(what);
        assert_eq!(code_of(&e), "locked", "{what}: {e:?}");
        let n = notice_for(e);
        assert_eq!(n.key, "installer_lock.locked", "{what}");
        assert!(!n.title.is_empty() && !n.detail.is_empty(), "{what}: {n:?}");
    }
    assert_eq!(core.get_state().output_gain_db, before.output_gain_db);
    assert_eq!(core.get_state().filters, before.filters);
    // The installer's actions need the unlock.
    for r in [
        core.limits_apply_profile(LimitsProfile::default().to_document())
            .map(|_| ()),
        core.limits_clear(),
        core.installer_pin_set("1357".into()).map(|_| ()),
        core.installer_pin_clear().map(|_| ()),
        core.installer_lock_cover(Vec::new()).map(|_| ()),
    ] {
        assert_eq!(code_of(&r.unwrap_err()), "pin_required");
    }
    // The installer's mode needs the PIN; the lock shows the locked user.
    let c = core
        .mode_resolve("desktop".into(), "installer".into())
        .unwrap();
    assert_eq!(
        (c.mode.as_str(), c.reason.as_str()),
        ("locked_user", "locked")
    );
    assert!(!c.switchable);
    assert_eq!(c.screens, ["front"]);
}

#[test]
fn wp_p1lk_unlock_then_everything_passes() {
    let core = demo();
    locked(&core);
    let st = core.installer_unlock(PIN.into()).unwrap();
    assert!(
        st.unlocked && !st.locked && st.installer_open && !st.recovered,
        "{st:?}"
    );
    for (what, r) in allowed(&core).into_iter().chain(others(&core)) {
        assert!(r.is_ok(), "{what}: {r:?}");
    }
    let c = core
        .mode_resolve("phone".into(), "installer".into())
        .unwrap();
    assert_eq!(c.mode, "installer");
    assert!(!c.needs_pin);
    // Locked again: refused again.
    core.installer_lock().unwrap();
    assert_eq!(
        code_of(&core.set_output_gain(1, 1.0).unwrap_err()),
        "locked"
    );
    // The unlock lasts for the session, across a reconnect.
    core.installer_unlock(PIN.into()).unwrap();
    core.disconnect();
    core.open_demo().unwrap();
    assert!(core.installer_lock_state().unlocked);
    assert!(core.set_output_gain(1, 1.0).is_ok());
}

#[test]
fn wp_p1lk_wrong_pins_open_a_refusal_window_on_the_clock_seam() {
    let core = demo();
    locked(&core);
    for n in 1..=LOCK_FREE_TRIES {
        let e = core.installer_unlock("0000".into()).unwrap_err();
        assert_eq!(code_of(&e), "wrong_pin");
        let st = core.installer_lock_state();
        assert_eq!((st.wrong_tries, st.retry_after_ms), (n, 0));
    }
    // The next wrong one opens the window ...
    let e = core.installer_unlock("0000".into()).unwrap_err();
    assert_eq!(code_of(&e), "wrong_pin");
    let st = core.installer_lock_state();
    assert!(
        st.retry_after_ms > 0 && st.retry_after_ms <= LOCK_FIRST_WAIT_MS,
        "{st:?}"
    );
    // ... and inside it even the right PIN is refused at once, unchecked.
    let t = std::time::Instant::now();
    let e = core.installer_unlock(PIN.into()).unwrap_err();
    assert_eq!(code_of(&e), "wait");
    assert_eq!(notice_for(e).key, "installer_lock.wait");
    assert!(t.elapsed() < Duration::from_millis(500), "no sleep");
    assert!(core.installer_lock_state().locked);
    // Once the window has passed, the right PIN unlocks and the count resets.
    core.lock()
        .limits
        .lock
        .age_for_test(Duration::from_millis(LOCK_FIRST_WAIT_MS + 1));
    let st = core.installer_unlock(PIN.into()).unwrap();
    assert!(st.unlocked && st.wrong_tries == 0 && st.retry_after_ms == 0);
}

#[test]
fn wp_p1lk_a_profile_without_a_pin_is_editable_by_anyone() {
    let core = demo();
    let doc = LimitsProfile {
        master_volume_max_db: Some(-6.0),
        locked_pages: vec!["front".into()],
        ..Default::default()
    }
    .to_document();
    core.limits_apply_profile(doc).unwrap();
    assert!(!core.installer_lock_state().locked);
    core.limits_apply_profile(LimitsProfile::default().to_document())
        .unwrap();
    core.limits_clear().unwrap();
    // A PIN with nothing covered locks no write, but the profile needs it.
    core.installer_pin_set(PIN.into()).unwrap();
    core.installer_lock().unwrap();
    let st = core.installer_lock_state();
    assert!(st.pin_set && !st.locked && !st.installer_open, "{st:?}");
    assert!(core.set_output_gain(1, 1.0).is_ok());
    let r = core.limits_apply_profile(LimitsProfile::default().to_document());
    assert_eq!(code_of(&r.unwrap_err()), "pin_required");
    let c = core
        .mode_resolve("desktop".into(), "installer".into())
        .unwrap();
    assert_eq!((c.mode.as_str(), c.needs_pin), ("advanced", true));
    assert_eq!(c.open_modes, ["simple", "advanced"]);
}

#[test]
fn wp_p1lk_the_recovery_code_unlocks_once_shown() {
    let core = demo();
    let code = locked(&core);
    assert_eq!(code.len(), 23, "{code}");
    let st = core
        .installer_unlock(code.to_lowercase().replace('-', " "))
        .unwrap();
    assert!(st.unlocked && st.recovered, "{st:?}");
    // A new PIN; the old code no longer opens it.
    let new = core.installer_pin_set("97531".into()).unwrap();
    assert_ne!(new.recovery_code, code);
    assert!(!new.state.recovered);
    core.installer_lock().unwrap();
    assert_eq!(
        code_of(&core.installer_unlock(code).unwrap_err()),
        "wrong_pin"
    );
    assert!(core.installer_unlock(PIN.into()).is_err());
    assert!(core.installer_unlock("97531".into()).unwrap().unlocked);
    // Cleared: back to no PIN, no lock, no prompt.
    let st = core.installer_pin_clear().unwrap();
    assert!(!st.pin_set && !st.locked && st.installer_open, "{st:?}");
}

#[test]
fn wp_p1lk_the_pin_lives_in_files_beside_the_profile_and_fails_closed() {
    let d = dir("files");
    let core = demo();
    core.limits_set_dir(d.to_string_lossy().into_owned())
        .unwrap();
    let serial = core.installer_lock_state().serial;
    locked(&core);
    let store = LimitsStore::new(&d);
    let pin = std::fs::read_to_string(store.dir(&serial).join(PIN_FILE)).unwrap();
    let rec = std::fs::read_to_string(store.dir(&serial).join(RECOVERY_FILE)).unwrap();
    assert!(!pin.contains(PIN) && pin.contains("pbkdf2"), "{pin}");
    assert!(rec.contains("pbkdf2"), "{rec}");
    // No document carries the hash.
    let export = core.config_export("x".into(), "t".into(), 0);
    let hash = store.pin(&serial).unwrap().unwrap().hash;
    assert!(export.is_err() || !export.unwrap().contains(&hash));
    assert!(!core.limits_profile_document().contains(&hash));

    // Another session (a new core) finds the device locked.
    let again = demo();
    again
        .limits_set_dir(d.to_string_lossy().into_owned())
        .unwrap();
    assert!(again.installer_lock_state().locked);
    assert_eq!(
        code_of(&again.set_output_gain(1, 1.0).unwrap_err()),
        "locked"
    );
    // A torn PIN file still counts as set: the lock holds, with a reason.
    std::fs::write(store.dir(&serial).join(PIN_FILE), "{").unwrap();
    let torn = demo();
    torn.limits_set_dir(d.to_string_lossy().into_owned())
        .unwrap();
    let st = torn.installer_lock_state();
    assert!(st.pin_set && st.locked && !st.detail.is_empty(), "{st:?}");
    assert!(torn.installer_unlock(PIN.into()).is_err());
    // Deleting the device's folder is the documented last resort.
    std::fs::remove_dir_all(store.dir(&serial)).unwrap();
    let fresh = demo();
    fresh
        .limits_set_dir(d.to_string_lossy().into_owned())
        .unwrap();
    assert!(!fresh.installer_lock_state().pin_set);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn wp_p1lk_a_hub_pinned_mode_overrides_the_choice_and_the_lock_comes_first() {
    let core = demo();
    core.mode_set_pinned("simple".into()).unwrap();
    assert_eq!(core.mode_pinned(), "simple");
    let c = core
        .mode_resolve("desktop".into(), "advanced".into())
        .unwrap();
    assert_eq!((c.mode.as_str(), c.reason.as_str()), ("simple", "pinned"));
    assert!(!c.switchable);
    core.mode_set_pinned("installer".into()).unwrap();
    assert_eq!(
        core.mode_resolve("web".into(), String::new()).unwrap().mode,
        "installer"
    );
    locked(&core);
    let c = core.mode_resolve("web".into(), String::new()).unwrap();
    assert_eq!(c.mode, "locked_user");
    core.installer_unlock(PIN.into()).unwrap();
    core.installer_lock_cover(Vec::new()).unwrap();
    core.installer_lock().unwrap();
    // Pinned to the Installer, PIN set, not unlocked: the PIN is asked.
    let c = core.mode_resolve("web".into(), String::new()).unwrap();
    assert_eq!((c.mode.as_str(), c.needs_pin), ("advanced", true));
    core.mode_set_pinned(String::new()).unwrap();
    assert_eq!(core.mode_pinned(), "");
    assert!(core.mode_set_pinned("pro".into()).is_err());
}

#[test]
fn wp_p1lk_a_guarded_revert_passes_the_lock() {
    // The revert path runs inside `unlimited`, which the lock honours too.
    let core = demo();
    locked(&core);
    let r = unlimited(|| core.set_output_gain(1, 2.0));
    assert!(r.is_ok(), "{r:?}");
}

// -- WP TMCOREF -- (review tmcore F2)
/// The bootloader is an installer's action: refused while locked, before the
/// token is spent; the same token goes through once unlocked.
#[test]
fn wp_tmcoref_the_bootloader_is_held_by_the_lock() {
    let core = demo();
    core.require_confirmation(true);
    locked(&core);
    let token = core.confirm_token(crate::ConfirmOp::EnterBootloader);
    let e = core
        .enter_bootloader_confirmed(Some(token.clone()))
        .unwrap_err();
    assert_eq!(code_of(&e), "locked", "{e:?}");
    assert!(core.is_connected(), "nothing was sent");
    core.installer_unlock(PIN.into()).unwrap();
    core.enter_bootloader_confirmed(Some(token)).unwrap();
}

// -- WP TMCOREF -- (review tmcore F2)
/// A firmware write with the device still open is held the same way; once
/// unlocked it goes on to its own checks (here the image is refused).
#[test]
fn wp_tmcoref_a_firmware_write_is_held_by_the_lock() {
    let core = demo();
    core.require_confirmation(true);
    locked(&core);
    let drive = || crate::BootloaderDrive {
        path: "/nonexistent/tmcoref-drive".into(),
        platform: 1,
        info: String::new(),
    };
    let flash = |core: &DspiCore| {
        let token = core.confirm_token(crate::ConfirmOp::FirmwareFlash);
        core.flash_confirmed(
            vec![0; 16],
            drive(),
            crate::firmware_guard_board_in_bootloader(),
            String::new(),
            Some(token),
        )
    };
    let e = flash(&core).unwrap_err();
    assert_eq!(code_of(&e), "locked", "{e:?}");
    core.installer_unlock(PIN.into()).unwrap();
    let e = flash(&core).unwrap_err();
    assert!(
        !matches!(e, DspiError::InstallerLock { .. }),
        "past the lock: {e:?}"
    );
}
