//! WP P1LK: the mode manifest's asset and the resolution rules.

use super::*;

#[test]
fn wp_p1lk_the_manifest_reads_whole_and_consistent() {
    let m: ModeManifest = serde_json::from_str(MODES_JSON).expect("modes.json reads");
    assert_eq!(m, mode_manifest());
    assert_eq!(m.version, 1);
    assert_eq!(m.platforms, ["phone", "desktop", "web"]);
    let ids: Vec<&str> = m.modes.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["simple", "advanced", "installer"]);
    // Every platform has exactly one default; only the Installer needs a PIN.
    for p in &m.platforms {
        let n = m.modes.iter().filter(|e| e.default_on.contains(p)).count();
        assert_eq!(n, 1, "{p}");
    }
    let pinned: Vec<&str> = m
        .modes
        .iter()
        .filter(|e| e.needs_pin_when_set)
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(pinned, ["installer"]);
    // Every reference names something the manifest has.
    let mut states: Vec<&str> = ids.clone();
    states.push(LOCKED_USER);
    let screens: Vec<&str> = m.screens.iter().map(|s| s.id.as_str()).collect();
    for s in &m.screens {
        assert!(!s.title.is_empty(), "{}", s.id);
        for x in s.modes.iter().chain(&s.first_in) {
            assert!(states.contains(&x.as_str()), "{}: {x}", s.id);
        }
        for x in &s.first_in {
            assert!(s.modes.contains(x), "{}: first in {x} but not shown", s.id);
        }
        assert!(s.layer.is_empty() || s.layer == "pro", "{}", s.id);
        for p in &s.platforms {
            assert!(m.platforms.contains(p), "{}: {p}", s.id);
        }
    }
    for o in &m.operations {
        for x in &o.modes {
            assert!(ids.contains(&x.as_str()), "{}: {x}", o.id);
        }
        for p in &o.platforms {
            assert!(m.platforms.contains(p), "{}: {p}", o.id);
        }
    }
    for x in &m.locked_user.screens {
        assert!(screens.contains(&x.as_str()), "{x}");
    }
    // Each mode and the locked user has a first screen on every platform.
    for mode in &states {
        for p in &m.platforms {
            let c = resolve(&m, p, "", *mode == LOCKED_USER, true, Some(mode)).unwrap();
            assert!(!c.first_screen.is_empty(), "{mode} on {p}");
        }
    }
    // The writes the locked user may make are the core's own list.
    assert_eq!(
        m.locked_user.writes,
        crate::limits::LOCKED_USER_WRITES
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    );
    for w in &m.locked_user.writes {
        assert!(
            dspi_proto::registry::by_path(w).is_some_and(|d| d.is_writable()),
            "{w}"
        );
    }
}

#[test]
fn wp_p1lk_defaults_per_platform_class() {
    let m = mode_manifest();
    let mode = |p: &str| resolve(&m, p, "", false, true, None).unwrap();
    // -- WP UAN -- owner D9 (2026-11-25): phones open in Advanced until Simple means something there
    assert_eq!(mode("phone").mode, "advanced");
    assert_eq!(mode("desktop").mode, "advanced");
    assert_eq!(mode("web").mode, "advanced");
    assert_eq!(mode("phone").reason, "default");
    assert_eq!(mode("phone").first_screen, "system");
    assert_eq!(
        resolve(&m, "phone", "simple", false, true, None)
            .unwrap()
            .first_screen,
        "front"
    );
    assert_eq!(mode("desktop").first_screen, "system");
    // Firmware is a desktop operation only.
    let inst = |p: &str| resolve(&m, p, "installer", false, true, None).unwrap();
    assert!(inst("desktop").operations.contains(&"firmware".to_owned()));
    assert!(!inst("phone").operations.contains(&"firmware".to_owned()));
    assert!(inst("phone").screens.contains(&"limits".to_owned()));
    assert!(
        !resolve(&m, "phone", "simple", false, true, None)
            .unwrap()
            .operations
            .contains(&"diagnostics".to_owned())
    );
    assert!(
        mode("desktop")
            .operations
            .contains(&"diagnostics".to_owned())
    );
}

#[test]
fn wp_p1lk_resolution_order_and_the_pin_rule() {
    let m = mode_manifest();
    // No PIN (installer open): never a prompt, whatever is asked.
    for chosen in ["", "simple", "advanced", "installer"] {
        let c = resolve(&m, "web", chosen, false, true, None).unwrap();
        assert!(!c.needs_pin && c.switchable, "{c:?}");
    }
    // A PIN set and not unlocked: the Installer asks for it.
    let c = resolve(&m, "web", "installer", false, false, None).unwrap();
    assert_eq!((c.mode.as_str(), c.needs_pin), ("advanced", true));
    assert_eq!(c.open_modes, ["simple", "advanced"]);
    // Pinned beats chosen; locked beats pinned.
    let c = resolve(&m, "desktop", "advanced", false, true, Some("simple")).unwrap();
    assert_eq!(
        (c.mode.as_str(), c.reason.as_str(), c.switchable),
        ("simple", "pinned", false)
    );
    let c = resolve(&m, "desktop", "advanced", true, false, Some("simple")).unwrap();
    assert_eq!(
        (c.mode.as_str(), c.reason.as_str()),
        ("locked_user", "locked")
    );
    assert_eq!(c.screens, ["front"]);
    assert!(c.operations.is_empty());
    let c = resolve(&m, "desktop", "", false, true, Some(LOCKED_USER)).unwrap();
    assert_eq!(c.mode, LOCKED_USER);
    // Unknown names are refused, not guessed.
    assert!(resolve(&m, "tablet", "", false, true, None).is_err());
    assert!(resolve(&m, "web", "pro", false, true, None).is_err());
}

#[test]
fn wp_p1lk_the_manifest_serialises_in_the_assets_shape() {
    let v = serde_json::to_value(mode_manifest()).unwrap();
    let asset: serde_json::Value = serde_json::from_str(MODES_JSON).unwrap();
    let back: ModeManifest = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(back, mode_manifest());
    assert_eq!(v["modes"][2]["needsPinWhenSet"], true);
    assert_eq!(v["lockedUser"]["screens"], asset["lockedUser"]["screens"]);
}

/// The UniFFI clients get the same record: lowered as a binding sends it and
/// lifted back, field for field (and the lock's state and error with it).
#[cfg(feature = "uniffi")]
#[test]
fn wp_p1lk_the_manifest_round_trips_through_uniffi() {
    use uniffi::{Lift, Lower};
    fn round<T: Lower<crate::UniFfiTag> + Lift<crate::UniFfiTag>>(v: T) -> T {
        T::try_lift_from_rust_buffer(T::lower_into_rust_buffer(v)).unwrap()
    }
    let m = mode_manifest();
    assert_eq!(round(m.clone()), m);
    let core = DspiCore::new();
    core.open_demo().unwrap();
    let c = core.mode_resolve("phone".into(), String::new()).unwrap();
    assert_eq!(round(c.clone()), c);
    let st = core.installer_lock_state();
    assert_eq!(round(st.clone()), st);
    let e = DspiError::InstallerLock {
        code: "wait".into(),
        msg: "later".into(),
    };
    assert_eq!(round(e.clone()), e);
}
