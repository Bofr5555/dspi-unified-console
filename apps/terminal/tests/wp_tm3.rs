//! WP TM3: `uconsole flash` as a user runs it, on the demo device and a fake
//! bootloader drive under Cargo's target-side scratch directory, never on tmpfs.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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
    let mut child = Command::new(env!(concat!("CARGO_BIN_EXE_", dspi_cmd::command!())))
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
    assert_eq!(
        report["confirmed"], false,
        "the demo has no board to confirm with"
    );
    assert_eq!(report["guard"]["decision"], "confirmed");
    assert_eq!(report["board"]["source"], "bootloader");
}

#[test]
fn wp_tm3_the_demo_is_not_a_board_that_came_back() {
    // Clear on the demo's own 1.20 V, if it stood in as the board.
    let bytes = image(RP2350, "1.1.6-beta4", Some("1200"));
    let (dir, file, drive) = setup("demo", &bytes);
    mount(&drive, RP2350_INFO);
    let d = drive.to_str().unwrap();
    let flash = ["--demo", "flash", &file, "--drive", d];
    let o = dspi(&dir, &[&flash[..], &["--force"]].concat(), "");
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
    assert!(!drive.join("firmware.uf2").exists());

    let accept = [&flash[..], &["--accept-core-voltage-risk"]].concat();
    let o = dspi(&dir, &accept, &format!("{PHRASE}\n"));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert_eq!(std::fs::read(drive.join("firmware.uf2")).unwrap(), bytes);
    let said = text(&o);
    assert!(
        said.contains(&format!("Written to {d}; no board to confirm with (demo)")),
        "{said}"
    );
    assert!(!said.contains("Done"), "{said}");
    assert!(!said.contains("is back"), "{said}");
}

#[test]
fn wp_tm3_the_demo_waits_for_no_drive() {
    let (dir, file, drive) = setup("demo_unmounted", &plain());
    let d = drive.to_str().unwrap();
    let o = dspi(
        &dir,
        &["--demo", "flash", &file, "--drive", d, "--force"],
        "",
    );
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(!drive.exists());
}

#[test]
fn wp_tm3_dry_run_touches_nothing() {
    let (dir, file, drive) = setup("dry", &plain());
    mount(&drive, RP2350_INFO);
    let d = drive.to_str().unwrap();
    let o = dspi(
        &dir,
        &["--demo", "flash", &file, "--drive", d, "--dry-run"],
        "",
    );
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
    assert!(text(&o).contains(PHRASE), "{}", text(&o));
    assert!(!drive.join("firmware.uf2").exists());
}

/// Exactly one JSON object on standard output, and the exit code it goes with.
fn report(dir: &Path, args: &[&str], stdin: &str, code: i32) -> serde_json::Value {
    let o = dspi(dir, &[args, &["--json"]].concat(), stdin);
    assert_eq!(o.status.code(), Some(code), "{args:?}: {}", text(&o));
    let out = String::from_utf8_lossy(&o.stdout);
    assert_eq!(out.lines().count(), 1, "{args:?}: {out}");
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("{args:?}: {e}: {out}"))
}

#[test]
fn wp_tm3_json_reports_every_outcome() {
    let bytes = plain();
    let (dir, file, drive) = setup("json", &bytes);
    let d = drive.to_str().unwrap().to_owned();
    let flash = ["--demo", "flash", &file, "--drive", &d];
    let refused = |args: &[&str], stdin: &str, code: i32| {
        let r = report(&dir, args, stdin, code);
        assert_eq!(r["flashed"], false, "{args:?}: {r}");
        assert!(
            r["error"].as_str().is_some_and(|e| !e.is_empty()),
            "{args:?}: {r}"
        );
        r
    };

    // Before any drive or device comes into it.
    refused(&["--demo", "flash", &file, "--nonsense"], "", 1);
    refused(&["--demo", "flash", "/no/such.uf2"], "", 1);
    refused(&["--demo", "flash", &file], "", 1);
    refused(&["--link", "127.0.0.1", "flash", &file], "", 1);
    refused(&flash, "", 1); // no drive at that path

    let bad = dir.join("bad.uf2");
    std::fs::write(&bad, b"not firmware").unwrap();
    let r = refused(&["--demo", "flash", bad.to_str().unwrap()], "", 1);
    assert!(
        r["error"].as_str().unwrap().contains("not a firmware file"),
        "{r}"
    );

    mount(&drive, RP2350_INFO);
    // The guard: declined, wrong answer, and a report of what it said.
    let r = refused(&flash, "", 3);
    assert_eq!(r["guard"]["decision"], "needsConfirmation");
    let accept = [&flash[..], &["--accept-core-voltage-risk"]].concat();
    let r = refused(&accept, "OVERVOLT\n", 3);
    assert_eq!(r["guard"]["decision"], "needsConfirmation");

    // Chip mismatch, in both modes.
    let other = dir.join("rp2040.uf2");
    std::fs::write(&other, image(RP2040, "1.1.6", None)).unwrap();
    let wrong = ["--demo", "flash", other.to_str().unwrap(), "--drive", &d];
    let r = refused(&wrong, "", 1);
    assert!(r["error"].as_str().unwrap().contains("RP2040"), "{r}");
    refused(&[&wrong[..], &["--dry-run"]].concat(), "", 1);

    // Dry run, then the real thing.
    let r = report(&dir, &[&flash[..], &["--dry-run"]].concat(), "", 3);
    assert_eq!(r["dryRun"], true);
    assert_eq!(r["flashed"], false);
    assert!(!drive.join("firmware.uf2").exists());
    let r = report(&dir, &accept, &format!("{PHRASE}\n"), 0);
    assert_eq!(r["flashed"], true);
    assert_eq!(r["confirmed"], false);
    assert_eq!(r["demo"], true);
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
fn wp_tm3_the_wrong_chip_is_refused_before_any_question() {
    let (dir, file, drive) = setup("chip", &image(RP2040, "1.1.6", None));
    mount(&drive, RP2350_INFO);
    let d = drive.to_str().unwrap();
    let flash = ["--demo", "flash", &file, "--drive", d];
    for extra in ["--accept-core-voltage-risk", "--dry-run"] {
        let o = dspi(
            &dir,
            &[&flash[..], &[extra]].concat(),
            &format!("{PHRASE}\n"),
        );
        assert_eq!(o.status.code(), Some(1), "{extra}: {}", text(&o));
        assert!(
            text(&o).contains("firmware for the RP2040"),
            "{extra}: {}",
            text(&o)
        );
        assert!(!text(&o).contains("Type "), "{extra}: {}", text(&o));
        assert!(!text(&o).contains("would"), "{extra}: {}", text(&o));
        assert!(!drive.join("firmware.uf2").exists());
    }
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
