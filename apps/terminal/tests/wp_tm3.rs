//! WP TM3: `dspi flash` as a user runs it, on the demo device and a fake
//! bootloader drive under Cargo's target-side scratch directory, never on tmpfs.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

#[allow(dead_code, clippy::arithmetic_side_effects)]
mod support_tm3;
use support_tm3::{RP2040, RP2350, RP2350_INFO, image, mount};

const PHRASE: &str = "I ACCEPT THE CORE VOLTAGE RISK";

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("wp_tm3")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn dspi(home: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_dspi"))
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// A scratch directory, an image in it, and where the drive goes.
fn setup(name: &str, bytes: &[u8]) -> (PathBuf, String, PathBuf) {
    let dir = scratch(name);
    let file = dir.join("test.uf2");
    std::fs::write(&file, bytes).unwrap();
    let drive = dir.join("RP2350");
    (dir, file.to_str().unwrap().to_owned(), drive)
}

fn plain() -> Vec<u8> {
    image(RP2350, "1.1.6-beta4", None)
}

#[test]
fn wp_tm3_force_alone_is_refused_on_a_mounted_drive() {
    let (dir, file, drive) = setup("force", &plain());
    mount(&drive, RP2350_INFO);
    let d = drive.to_str().unwrap();
    let o = dspi(
        &dir,
        &["--demo", "flash", &file, "--drive", d, "--force"],
        "",
    );
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
    assert!(
        text(&o).contains("--force does not override"),
        "{}",
        text(&o)
    );
    assert!(!drive.join("firmware.uf2").exists());
}

#[test]
fn wp_tm3_the_flag_with_a_wrong_or_missing_answer_writes_nothing() {
    let (dir, file, drive) = setup("wrong", &plain());
    mount(&drive, RP2350_INFO);
    let d = drive.to_str().unwrap();
    let args = [
        "--demo",
        "flash",
        &file,
        "--drive",
        d,
        "--accept-core-voltage-risk",
    ];
    for stdin in [
        "",
        "\n",
        "yes\n",
        "OVERVOLT\n",
        " I ACCEPT THE CORE VOLTAGE RISK\n",
    ] {
        let o = dspi(&dir, &args, stdin);
        assert_eq!(o.status.code(), Some(3), "{stdin:?}: {}", text(&o));
        assert!(!drive.join("firmware.uf2").exists(), "{stdin:?}");
    }
}

#[test]
fn wp_tm3_the_flag_and_the_typed_phrase_write_the_exact_bytes() {
    let bytes = plain();
    let (dir, file, drive) = setup("phrase", &bytes);
    mount(&drive, RP2350_INFO);
    let d = drive.to_str().unwrap();
    let o = dspi(
        &dir,
        &[
            "--demo",
            "flash",
            &file,
            "--drive",
            d,
            "--accept-core-voltage-risk",
            "--json",
        ],
        &format!("{PHRASE}\r\n"),
    );
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert_eq!(std::fs::read(drive.join("firmware.uf2")).unwrap(), bytes);
    let report: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(report["flashed"], true);
    assert_eq!(report["confirmed"], true);
    assert_eq!(report["guard"]["decision"], "confirmed");
    assert_eq!(report["board"]["source"], "bootloader");
}

#[test]
fn wp_tm3_the_whole_flow_with_a_drive_that_arrives() {
    // The demo device reports a 1.20 V board, and this image keeps it there.
    let bytes = image(RP2350, "1.1.6-beta4", Some("1200"));
    let (dir, file, drive) = setup("arrives", &bytes);
    let d = drive.to_str().unwrap().to_owned();
    let arriving = drive.clone();
    // Its drive turns up during the wait.
    let boot_rom = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        mount(&arriving, RP2350_INFO);
    });
    let o = dspi(
        &dir,
        &["--demo", "flash", &file, "--drive", &d, "--force"],
        "",
    );
    boot_rom.join().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert_eq!(std::fs::read(drive.join("firmware.uf2")).unwrap(), bytes);
    let said = text(&o);
    assert!(said.contains("dspi export"), "suggests a backup: {said}");
    assert!(said.contains("Done."), "{said}");
}

#[test]
fn wp_tm3_force_alone_is_refused_on_an_at_risk_board() {
    // The demo device is one: 1.20 V, against an image that does not say.
    let (dir, file, drive) = setup("at_risk", &plain());
    let d = drive.to_str().unwrap();
    let o = dspi(
        &dir,
        &["--demo", "flash", &file, "--drive", d, "--force"],
        "",
    );
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
    assert!(text(&o).contains("This board needs 1.20 V"), "{}", text(&o));
    assert!(!drive.exists(), "it never got as far as the reboot");
}

#[test]
fn wp_tm3_dry_run_touches_nothing() {
    let (dir, file, drive) = setup("dry", &plain());
    let d = drive.to_str().unwrap();
    let o = dspi(
        &dir,
        &["--demo", "flash", &file, "--drive", d, "--dry-run"],
        "",
    );
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
    assert!(text(&o).contains(PHRASE), "{}", text(&o));
    assert!(!drive.exists());

    let (dir, file, drive) = setup("dry_clear", &image(RP2350, "1.1.6-beta4", Some("1200")));
    let d = drive.to_str().unwrap();
    let o = dspi(
        &dir,
        &["--demo", "flash", &file, "--drive", d, "--dry-run"],
        "",
    );
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(!drive.exists());
}

#[test]
fn wp_tm3_a_bad_file_is_refused_before_anything_else() {
    let (dir, file, drive) = setup("bad", b"not firmware");
    mount(&drive, RP2350_INFO);
    let d = drive.to_str().unwrap();
    let args = ["--demo", "flash", &file, "--drive", d, "--force"];
    let o = dspi(&dir, &args, "");
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("not a firmware file"), "{}", text(&o));
    assert!(!drive.join("firmware.uf2").exists());

    let o = dspi(&dir, &["--demo", "flash", "/no/such.uf2", "--force"], "");
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
}

#[test]
fn wp_tm3_the_wrong_chip_is_refused_by_the_drive() {
    let (dir, file, drive) = setup("chip", &image(RP2040, "1.1.6", None));
    mount(&drive, RP2350_INFO);
    let d = drive.to_str().unwrap();
    let o = dspi(
        &dir,
        &[
            "--demo",
            "flash",
            &file,
            "--drive",
            d,
            "--accept-core-voltage-risk",
        ],
        &format!("{PHRASE}\n"),
    );
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(!drive.join("firmware.uf2").exists());
}

#[test]
fn wp_tm3_demo_without_a_drive_and_a_hub_are_refused() {
    let (dir, file, _) = setup("targets", &plain());
    let o = dspi(&dir, &["--demo", "flash", &file, "--force"], "");
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    let o = dspi(
        &dir,
        &["--link", "127.0.0.1", "flash", &file, "--force"],
        "",
    );
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
}
