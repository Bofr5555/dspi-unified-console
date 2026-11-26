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
    // -- INT016 -- (WP UB12) the demo is the tuned example: its outputs by their speakers' names
    assert!(
        out.lines()
            .any(|l| l.starts_with("Left woofer: ") && l.contains(", gain ")),
        "an output's sentence: {out}"
    );
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

// -- FIX016 -- (REVG F10) a script's lines are commands, never answers: its `ab` ends where `--keep`
// says (it took `t` and `b` from the next lines before, and so swallowed the script's commands)
#[test]
fn ub11_an_ab_in_plain_keeps_the_side_asked_and_the_next_command_still_runs() {
    let o = run(
        "ab",
        &["--demo", "--plain"],
        "snapshot save Before\nout.gain 0 -6\nab Before --keep b\nvol.user -23\nquit\nquit\n",
    );
    let out = text(&o.stdout);
    let err = text(&o.stderr);
    assert_eq!(o.status.code(), Some(0), "{out}\n{err}");
    assert!(out.contains("On A (A)"), "the compare began on A: {out}");
    assert!(out.contains("Kept B (Before)"), "--keep b kept B: {out}");
    assert!(
        out.lines().any(|l| l.starts_with("vol.user -23")),
        "the line after the compare ran: {out}"
    );
}

// -- FIX016 -- (REVG F10, reviewer C's probe) `ab` in a piped script takes no answers: it keeps the side
// it is on, a blank line does not toggle it, every later line runs and the writes happen
#[test]
fn fix016_a_scripts_ab_takes_no_answers_and_every_line_runs() {
    let o = run(
        "fix016-ab",
        &["--demo", "--plain"],
        "snapshot save Before\nout.gain 0 -6\nab Before\n\nvol.user -23\nout.gain 1 -4\nquit\n",
    );
    let out = text(&o.stdout);
    let err = text(&o.stderr);
    assert_eq!(o.status.code(), Some(0), "nothing failed: {out}\n{err}");
    assert!(out.contains("On A (A)"), "the compare began on A: {out}");
    assert!(!out.contains("On B"), "a blank line does not toggle: {out}");
    assert!(out.contains("Kept A (A)"), "the side it is on: {out}");
    assert!(
        !err.contains("type t, d, a, b or q"),
        "no line taken as an answer: {err}"
    );
    for write in ["vol.user -23", "out.gain 1 -4"] {
        assert!(
            out.lines().any(|l| l.starts_with(write)),
            "`{write}` ran: {out}"
        );
    }
    // `quit` asked once with the changes unsaved, then the input ended
    assert!(
        out.contains("Type save to keep them on the device"),
        "{out}"
    );
    assert!(out.contains("They are not saved to the device."), "{out}");
}

// -- FIX016 -- (REVG F10) a guarded recall in a script is kept at once (nothing waits out its
// countdown or reverts at the end), the next lines run, and the code is the first failure's
#[test]
fn fix016_a_scripts_guarded_recall_is_kept_and_the_code_is_the_first_failures() {
    let o = run(
        "fix016-guard",
        &["--demo", "--plain"],
        "snapshot save Before\nout.gain 0 -6\nsnapshot recall Before --guard 10\nsideways 3\nvol.user -24\nquit\n",
    );
    let out = text(&o.stdout);
    let err = text(&o.stderr);
    assert_eq!(
        o.status.code(),
        Some(1),
        "the first failure's (sideways): {out}\n{err}"
    );
    assert!(
        out.lines().any(|l| l.starts_with("Kept")),
        "the recall kept: {out}"
    );
    assert!(
        !err.contains("Type keep or revert"),
        "no line taken as an answer: {err}"
    );
    assert!(
        err.contains("no parameter or command called `sideways`"),
        "{err}"
    );
    assert!(
        out.lines().any(|l| l.starts_with("vol.user -24")),
        "the line after the mistake ran: {out}"
    );
}
