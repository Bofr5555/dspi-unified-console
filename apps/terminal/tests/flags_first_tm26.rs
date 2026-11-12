//! WP TM26: a boolean flag before the command (hw-1 D1).
//!
//! `uconsole --dry-run set ...` used to read `--dry-run` as "no command", open the
//! interface and, with no terminal, panic in ratatui (exit 101). These run the
//! real binary on the demo device with no terminal on either side.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

const DSPI: &str = env!(concat!("CARGO_BIN_EXE_", dspi_cmd::command!()));

/// A scratch directory under the target directory, never on tmpfs, doubling as
/// `HOME` so that the config and undo history stay out of the way.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("flags-first-tm26")
        .join(name);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Run `uconsole` with standard input and output that are not terminals.
fn run(name: &str, args: &[&str]) -> Output {
    let home = scratch(name);
    Command::new(DSPI)
        .args(args)
        .current_dir(&home)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env_remove("DSPI_PIN")
        .stdin(Stdio::null())
        .output()
        .expect("spawn")
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn assert_no_panic(o: &Output) {
    let err = text(&o.stderr);
    assert!(!err.contains("panicked"), "panicked:\n{err}");
    assert_ne!(o.status.code(), Some(101), "{err}");
}

#[test]
fn tm26_dry_run_before_the_command_runs_the_command() {
    let first = run(
        "dry-first",
        &["--demo", "--dry-run", "set", "vol.user", "-18"],
    );
    let last = run(
        "dry-last",
        &["--demo", "set", "vol.user", "-18", "--dry-run"],
    );
    assert_no_panic(&first);
    assert_eq!(first.status.code(), Some(0), "{}", text(&first.stderr));
    assert_eq!(text(&first.stdout), text(&last.stdout));
    assert_eq!(first.status.code(), last.status.code());
}

#[test]
fn tm26_json_before_the_command_prints_json() {
    let o = run("json-first", &["--demo", "--json", "get", "vol.user"]);
    assert_no_panic(&o);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).expect("JSON on stdout");
    assert_eq!(v["path"], "vol.user", "{v}");

    // And with `--demo` after it, as hw-1 also wrote it.
    let o = run("json-first-2", &["--json", "--demo", "get", "vol.user"]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let _: serde_json::Value = serde_json::from_slice(&o.stdout).expect("JSON on stdout");
}

#[test]
fn tm26_quiet_before_the_command_is_quiet() {
    let o = run(
        "quiet-first",
        &["--quiet", "--demo", "set", "vol.user", "-18"],
    );
    assert_no_panic(&o);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
}

/// The interface itself, with no terminal: a message and exit 1, not a panic.
#[test]
fn tm26_the_interface_without_a_terminal_fails_cleanly() {
    let o = run("no-tty", &["--fixture"]);
    assert_no_panic(&o);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o.stderr));
    let err = text(&o.stderr);
    assert!(err.contains("the interface needs a terminal"), "{err}");
    assert!(
        err.contains(concat!(dspi_cmd::command!(), " --help")),
        "{err}"
    );
    assert!(o.stdout.is_empty(), "drew into a pipe: {}", text(&o.stdout));
}
