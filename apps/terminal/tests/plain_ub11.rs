//! WP UB11 (UXP B11, owner D16): `uconsole --plain`, the interface as lines.
//! These run the real binary on the demo device with standard input a pipe:
//! a script's view of the plain mode (no prompt, the first failure's code).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

const DSPI: &str = env!(concat!("CARGO_BIN_EXE_", dspi_cmd::command!()));

/// A scratch directory under the target directory, never on tmpfs, doubling
/// as `HOME` so that the config, the snapshots and the undo history stay
/// out of the way.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("plain-ub11")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn run(name: &str, args: &[&str], input: &str) -> Output {
    let home = scratch(name);
    let mut child = Command::new(DSPI)
        .args(args)
        .current_dir(&home)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("TERM", "xterm-256color")
        .env_remove("DSPI_PIN")
        .env_remove("DSPI_INSTALLER_PIN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write the lines");
    child.wait_with_output().expect("wait")
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Box drawing, block elements, braille or an escape: what a screen reader
/// spells out, or a log keeps as noise.
fn glyphs(s: &str) -> Vec<char> {
    s.chars()
        .filter(|c| {
            ('\u{2500}'..='\u{259F}').contains(c)
                || ('\u{2800}'..='\u{28FF}').contains(c)
                || *c == '\u{1b}'
        })
        .collect()
}

#[test]
fn ub11_plain_greets_with_sentences_and_says_each_change_back() {
    let o = run(
        "greet",
        &["--demo", "--plain"],
        "overview\nvol.user -20\nshow out 1\nquit\nquit\n",
    );
    let out = text(&o.stdout);
    let err = text(&o.stderr);
    assert_eq!(o.status.code(), Some(0), "{out}\n{err}");
    assert!(
        out.contains("plain mode. Connected to the demo device."),
        "{out}"
    );
    assert!(out.contains("Volume "), "{out}");
    assert!(out.contains("Out 1"), "an output's sentence: {out}");
    assert!(out.contains("not muted"), "{out}");
    assert!(
        out.contains("Type a command, help for the list, or quit."),
        "{out}"
    );
    // the echo line: the command that makes the change
    assert!(
        out.lines().any(|l| l.starts_with("vol.user -20")),
        "the change said back: {out}"
    );
    assert!(glyphs(&out).is_empty(), "{:?} in {out}", glyphs(&out));
    // no prompt without a person
    assert!(!out.contains("dspi> "), "{out}");
}

#[test]
fn ub11_plain_carries_on_past_a_mistake_and_a_script_gets_its_code() {
    let o = run(
        "mistake",
        &["--demo", "--plain"],
        "sideways 3\nhelp\nvol.user -21\n",
    );
    let out = text(&o.stdout);
    let err = text(&o.stderr);
    assert_eq!(o.status.code(), Some(1), "the first failure's code: {err}");
    assert!(
        err.contains("no parameter or command called `sideways`; help lists"),
        "{err}"
    );
    assert!(!err.contains("Try `uconsole params`"), "no way out: {err}");
    assert!(out.contains("The plain mode's words:"), "{out}");
    assert!(
        out.lines().any(|l| l.starts_with("vol.user -21")),
        "the line after the mistake ran: {out}"
    );
}

#[test]
fn ub11_the_shells_verbs_run_in_the_plain_session_and_keep_the_next_line() {
    let o = run(
        "verbs",
        &["--demo", "--plain"],
        "snapshot save Before\nsnapshot list\nvol.user -22\nstatus\nquit\nquit\n",
    );
    let out = text(&o.stdout);
    let err = text(&o.stderr);
    assert_eq!(o.status.code(), Some(0), "{out}\n{err}");
    assert!(out.contains("Before"), "the snapshot listed: {out}");
    assert!(
        out.lines().any(|l| l.starts_with("vol.user -22")),
        "the line after the verbs ran: {out}"
    );
}

#[test]
fn ub11_without_plain_a_piped_input_is_still_a_script() {
    let o = run("script", &["--demo"], "vol.user -20\n");
    let out = text(&o.stdout);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    assert!(!out.contains("plain mode"), "{out}");
}
