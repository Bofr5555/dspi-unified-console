//! WP UB12 (UXP B12): `uconsole --demo` opens the core's tuned example, so a
//! frame of the demo shows the named stereo two-way, not seven "None" rows.

use std::path::PathBuf;
use std::process::{Command, Stdio};

const DSPI: &str = env!(concat!("CARGO_BIN_EXE_", dspi_cmd::command!()));

/// A scratch directory under the target directory, doubling as `HOME`.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("demo-example-ub12")
        .join(name);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn wp_ub12_the_demo_frame_shows_the_tuned_example() {
    let home = scratch("screenshot");
    let out = Command::new(DSPI)
        .args(["--demo", "screenshot", "132", "40"])
        .current_dir(&home)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env_remove("DSPI_PIN")
        .stdin(Stdio::null())
        .output()
        .expect("the binary runs");
    let frame = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{frame}");
    for name in [
        "Left woofer",
        "Right woofer",
        "Left tweeter",
        "Right tweeter",
    ] {
        assert!(frame.contains(name), "{name}:\n{frame}");
    }
}
