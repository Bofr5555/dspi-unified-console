//! WP AUSB3: `uconsole usb-direct generate` end to end. The real binary, an
//! installation file from the fixtures, a scratch folder under the target
//! directory (never tmpfs). No unit is opened.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

const DSPI: &str = env!(concat!("CARGO_BIN_EXE_", dspi_cmd::command!()));

fn fixture(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ausb3")
        .join(name)
        .display()
        .to_string()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("wp-ausb3")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn run(home: &PathBuf, args: &[&str]) -> Output {
    Command::new(DSPI)
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .stdin(Stdio::null())
        .output()
        .expect("spawn")
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn wp_ausb3_generate_writes_every_file_and_keeps_existing_ones() {
    let home = scratch("write");
    let out = home.join("cinema");
    let out_s = out.display().to_string();
    let spec = fixture("cinema-7.1.json");

    let o = run(&home, &["usb-direct", "generate", &spec, "--out", &out_s]);
    assert!(o.status.success(), "{}", text(&o.stderr));
    assert!(
        text(&o.stdout).contains("Wrote 7 files"),
        "{}",
        text(&o.stdout)
    );
    for f in [
        "pipewire/dspi-usb-direct-cinema.conf",
        "wireplumber/dspi-usb-direct-cinema.conf",
        "hub-system-model.json",
        "SETTINGS.md",
        "units/front.dspi",
        "units/sides.dspi",
        "units/rears.dspi",
    ] {
        assert!(out.join(f).is_file(), "{f}");
    }
    // The same files as the golden ones.
    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ausb3/cinema-7.1.expected/SETTINGS.md");
    assert_eq!(
        std::fs::read_to_string(golden).unwrap(),
        std::fs::read_to_string(out.join("SETTINGS.md")).unwrap()
    );

    // A second run does not replace them without --force.
    let o = run(&home, &["usb-direct", "generate", &spec, "--out", &out_s]);
    assert_eq!(o.status.code(), Some(1));
    assert!(text(&o.stderr).contains("--force"), "{}", text(&o.stderr));
    let o = run(
        &home,
        &["usb-direct", "generate", &spec, "--out", &out_s, "--force"],
    );
    assert!(o.status.success(), "{}", text(&o.stderr));
}

#[test]
fn wp_ausb3_generate_dry_run_and_json_write_nothing() {
    let home = scratch("dry");
    let out = home.join("den");
    let out_s = out.display().to_string();
    let o = run(
        &home,
        &[
            "usb-direct",
            "generate",
            &fixture("den-5.1.json"),
            "--out",
            &out_s,
            "--dry-run",
            "--json",
        ],
    );
    assert!(o.status.success(), "{}", text(&o.stderr));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["written"], false);
    assert_eq!(v["d"], 240);
    assert_eq!(v["units"][0]["channels"], 4);
    assert_eq!(v["units"][1]["channels"], 4);
    assert!(!out.exists());
}

#[test]
fn wp_ausb3_generate_refuses_a_bad_file_with_exit_1() {
    let home = scratch("bad");
    let bad = home.join("bad.json");
    std::fs::write(
        &bad,
        r#"{"name": "x", "rate": 44100, "layout": "2.0",
            "units": [{"id": "a", "serial": "E6614C311B4A0001",
                       "channels": [{"channel": "FL", "output": 0}]}]}"#,
    )
    .unwrap();
    let o = run(
        &home,
        &["usb-direct", "generate", &bad.display().to_string()],
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(
        text(&o.stderr).contains("44.1 kHz cannot phase-lock"),
        "{}",
        text(&o.stderr)
    );
    assert!(!home.join("usb-direct-x").exists());
    // No subcommand: usage, exit 1.
    let o = run(&home, &["usb-direct"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(text(&o.stderr).contains("usb-direct generate"));
}
