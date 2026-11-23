//! WP FUNC2c: what the function test's stories S7 (an installer-locked
//! device) and S6 (a role the user has not confirmed) found in the MCP server.
//!
//! S7: the lock's refusal said only "ask the user to unlock in the app", and
//! `dspi_get_limits` did not show the lock at all, so no agent of three
//! offered the mute that the lock lets through. The refusal, the overview and
//! the limits now say what passes while it is locked, and none of them gives
//! the agent the PIN, its recovery code or its hash.

use serde_json::json;

use crate::policy::ConfirmMode;
use crate::tests_wp_ffix::{demo, err, ok, server_of};

const PIN: &str = "4711";

#[test]
fn wp_func2c_the_lock_says_what_passes_and_never_gives_the_pin() {
    let (set, core) = demo("func2c-lock");
    let code = core.installer_pin_set(PIN.into()).unwrap().recovery_code;
    core.installer_lock_cover(vec!["channels".into()]).unwrap();
    assert!(core.installer_lock().unwrap().locked);
    let mut s = server_of(&set, ConfirmMode::AskRisky);
    let c = s.connect();

    let refused = err(
        &mut s,
        c,
        "dspi_set_gain",
        json!({"output": 4, "gain_db": -3.0}),
    );
    assert!(refused.starts_with("the installer lock is on"), "{refused}");
    for passes in [
        "dspi_set_mute",
        "dspi_set_volume",
        "dspi_preset_load",
        "Settings › Installer",
        "PIN",
    ] {
        assert!(refused.contains(passes), "{passes}: {refused}");
    }
    let over = ok(&mut s, c, "dspi_get_overview", json!({}));
    let limits = ok(&mut s, c, "dspi_get_limits", json!({}));
    for v in [&over, &limits] {
        assert_eq!(v["installer_lock"]["locked"], json!(true), "{v}");
        let text = v["installer_lock"]["while_locked"]
            .as_str()
            .unwrap_or_default();
        assert!(text.contains("dspi_set_mute"), "{v}");
        let all = v.to_string();
        assert!(
            !all.contains(PIN) && !all.contains(&code),
            "a secret in {all}"
        );
    }
    // What the refusal names does pass, quietly (a mute is never risky)
    ok(
        &mut s,
        c,
        "dspi_set_mute",
        json!({"output": 4, "muted": true}),
    );
    assert!(core.get_state().output_muted[4]);
    assert!((core.get_state().output_gain_db[4]).abs() < 0.01);

    // Unlocked (a PIN set, the lock covering nothing): no `while_locked`
    core.installer_unlock(PIN.into()).unwrap();
    core.installer_lock_cover(Vec::new()).unwrap();
    core.installer_lock().unwrap();
    let limits = ok(&mut s, c, "dspi_get_limits", json!({}));
    assert_eq!(
        limits["installer_lock"],
        json!({"pin_set": true, "locked": false}),
        "{limits}"
    );
}

#[test]
fn wp_func2c_what_the_profile_held_reads_in_the_apps_words() {
    use crate::tools::library::held_words;
    assert_eq!(
        held_words("eq.freq[9,20] 600 → 1000 Hz", 8),
        "Out 2 crossover 1 frequency: 600 → 1000 Hz"
    );
    assert_eq!(
        held_words("eq.gain[2,3] 9 → 6 dB", 8),
        "In 3 band 4 gain: 9 → 6 dB"
    );
    assert_eq!(
        held_words("out.gain[4] 2 → 0 dB", 8),
        "Out 5 gain: 2 → 0 dB"
    );
    assert_eq!(
        held_words(
            "output 2: a 12 dB/oct high-pass at 1000 Hz in crossover 1",
            8
        ),
        "Out 2: a 12 dB/oct high-pass at 1000 Hz in crossover 1"
    );
    assert_eq!(
        held_words("vol.master -3 → -6 dB", 8),
        "vol.master -3 → -6 dB"
    );
}
