//! WP ROMODE: `--read-only` puts the Terminal's core in the client read-only
//! mode. Every write is refused before it is sent, by every route (a `set`,
//! a script, `raw`, an import); reads go on. The real binary on the demo
//! device, with no terminal on either side.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

const DSPI: &str = env!(concat!("CARGO_BIN_EXE_", dspi_cmd::command!()));

/// A scratch directory under the target directory, never on tmpfs, doubling
/// as `HOME` so that the config and undo history stay out of the way.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("read-only-romode")
        .join(name);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn command(name: &str, args: &[&str]) -> Command {
    let home = scratch(name);
    let mut c = Command::new(DSPI);
    c.args(args)
        .current_dir(&home)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env_remove("DSPI_PIN");
    c
}

fn run(name: &str, args: &[&str]) -> Output {
    command(name, args)
        .stdin(Stdio::null())
        .output()
        .expect("spawn")
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Exit 3: refused, in the read-only mode's words.
fn assert_refused(o: &Output, what: &str) {
    let err = text(&o.stderr);
    assert!(!err.contains("panicked"), "{what}: {err}");
    assert_eq!(o.status.code(), Some(3), "{what}: {err}");
    assert!(err.contains("read-only mode"), "{what}: {err}");
}

#[test]
fn wp_romode_the_terminal_refuses_every_write_with_read_only() {
    // Without the flag the write goes out (the contrast).
    let plain = run("plain", &["--demo", "set", "vol.user", "-18"]);
    assert_eq!(plain.status.code(), Some(0), "{}", text(&plain.stderr));

    assert_refused(
        &run("set", &["--demo", "--read-only", "set", "vol.user", "-18"]),
        "set",
    );
    // The flag reads the same after the command.
    assert_refused(
        &run(
            "set-after",
            &["--demo", "set", "vol.user", "-18", "--read-only"],
        ),
        "set after",
    );
    assert_refused(
        &run("bare", &["--read-only", "--demo", "vol.user", "-18"]),
        "a bare path",
    );
    let raw = run(
        "raw",
        &[
            "--demo",
            "--read-only",
            "raw",
            "out",
            "0x42",
            "0",
            "08",
            "03",
            "01",
            "00",
        ],
    );
    assert_ne!(raw.status.code(), Some(0), "raw: {}", text(&raw.stderr));
    assert!(!text(&raw.stderr).contains("panicked"));

    // A script on standard input.
    let mut child = command("script", &["--demo", "--read-only"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"vol.user -18\n")
        .unwrap();
    let script = child.wait_with_output().unwrap();
    assert_ne!(script.status.code(), Some(0), "{}", text(&script.stderr));
    assert!(
        text(&script.stderr).contains("read-only mode"),
        "{}",
        text(&script.stderr)
    );
}

#[test]
fn wp_romode_reads_go_on_with_read_only() {
    let get = run("get", &["--demo", "--read-only", "get", "vol.user"]);
    assert_eq!(get.status.code(), Some(0), "{}", text(&get.stderr));
    assert!(!text(&get.stdout).trim().is_empty());
    let dump = run("dump", &["--demo", "--read-only", "--json", "dump"]);
    assert_eq!(dump.status.code(), Some(0), "{}", text(&dump.stderr));
    let v: serde_json::Value = serde_json::from_slice(&dump.stdout).expect("JSON");
    assert!(v.is_object(), "{v}");
}
