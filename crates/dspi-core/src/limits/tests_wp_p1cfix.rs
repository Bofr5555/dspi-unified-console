//! WP P1CFIX, N1: the profile's files are read with the core lock released,
//! when the folder is named and when a device is installed, not by the first
//! write judged under the lock. Stores live under `std::env::temp_dir()`,
//! which `ci/run.sh` points at the home volume (never tmpfs).

use dspi_session::limits::{LimitsProfile, OutputLimits};

use crate::DspiCore;

fn dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("wp-p1cfix-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn loaded(core: &DspiCore) -> bool {
    let inner = core.lock();
    let serial = inner.conn.as_ref().map(|c| c.serial.clone());
    inner
        .limits
        .current
        .as_ref()
        .is_some_and(|c| Some(&c.serial) == serial.as_ref() && c.profile.is_some())
}

#[test]
fn wp_p1cfix_n1_the_profile_is_read_before_any_write_is_judged() {
    let d = dir("n1");
    let core = DspiCore::new();
    core.open_demo().unwrap();
    core.limits_set_dir(d.to_string_lossy().into()).unwrap();
    let p = LimitsProfile {
        outputs: vec![OutputLimits {
            output: 2,
            max_gain_db: Some(3.0),
            ..Default::default()
        }],
        ..Default::default()
    };
    core.limits_apply_profile(p.to_document()).unwrap();

    // Naming the folder reads the files (outside the lock).
    let named = DspiCore::new();
    named.open_demo().unwrap();
    named.limits_set_dir(d.to_string_lossy().into()).unwrap();
    assert!(loaded(&named), "read when the folder was named");
    assert!(named.set_output_gain(2, 6.0).is_err());

    // Opening a device with the folder named reads them at install.
    let opened = DspiCore::new();
    opened.limits_set_dir(d.to_string_lossy().into()).unwrap();
    assert!(!loaded(&opened), "no device yet");
    opened.open_demo().unwrap();
    assert!(loaded(&opened), "read when the device was installed");
    assert!(opened.set_output_gain(2, 6.0).is_err());
    let _ = std::fs::remove_dir_all(&d);
}
