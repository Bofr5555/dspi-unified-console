//! WP CG27: the demo device runs the final 1.1.6 this build targets, so the
//! Terminal's demo run names that release in its title row and shows no
//! firmware banner above it.

use std::path::PathBuf;
use std::process::{Command, Stdio};

const DSPI: &str = env!("CARGO_BIN_EXE_dspi");

/// A scratch directory under the target directory, never on tmpfs, doubling as
/// `HOME` so that the config, undo history and AutoEQ store stay out of the way.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("demo-firmware-cg27")
        .join(name);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn wp_cg27_the_demo_run_shows_no_firmware_banner() {
    let home = scratch("screenshot");
    let out = Command::new(DSPI)
        .args(["--demo", "screenshot", "120", "40"])
        .current_dir(&home)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env_remove("DSPI_PIN")
        .stdin(Stdio::null())
        .output()
        .expect("dspi runs");
    let frame = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{frame}");
    let title = frame.lines().next().unwrap_or_default();
    assert!(title.contains("fw 1.1.6 ·"), "{title}");
    assert!(!frame.contains("beta"), "{frame}");
    assert!(!frame.contains("READ-ONLY"), "{frame}");
    assert!(!frame.to_lowercase().contains("update"), "{frame}");
}
