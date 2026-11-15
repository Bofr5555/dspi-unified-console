//! WP TM15: every command that prints, run with its output going nowhere.
//!
//! `uconsole --demo dump | head -5` used to panic: `println!` panics when standard
//! output is a pipe nobody reads any more. These run the real binary with a
//! reader that has already gone, so the first write fails with a broken pipe.
//! A closed reader ends the output, not the command: each exits with the status
//! it has on an open pipe, and only `watch`, which would never end, stops early.

use std::io::{BufRead, BufReader, pipe};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const DSPI: &str = env!(concat!("CARGO_BIN_EXE_", dspi_cmd::command!()));

/// Long enough for a loaded build machine; `watch` might otherwise never end.
const LIMIT: Duration = Duration::from_secs(60);

/// A scratch directory under the target directory, never on tmpfs, doubling as
/// `HOME` so that the config, undo history and AutoEQ store stay out of the way.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("closed-pipe-tm15")
        .join(name);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn dspi(home: &PathBuf, args: &[&str]) -> Command {
    let mut c = Command::new(DSPI);
    c.args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env_remove("DSPI_PIN")
        .stdin(Stdio::null());
    c
}

/// Wait for a child, killing it if it never notices. `what` names the command.
fn finish(mut child: Child, what: &str) -> (ExitStatus, String) {
    let started = Instant::now();
    let status = loop {
        match child.try_wait().expect("wait") {
            Some(s) => break s,
            None if started.elapsed() > LIMIT => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("`{what}` was still running after {LIMIT:?}");
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        std::io::Read::read_to_string(&mut pipe, &mut stderr).expect("stderr");
    }
    (status, stderr)
}

/// A pipe whose reader has already been closed.
fn closed_writer() -> Stdio {
    let (reader, writer) = pipe().expect("pipe");
    drop(reader);
    Stdio::from(writer)
}

fn assert_quiet(what: &str, stderr: &str) {
    assert!(!stderr.contains("panicked"), "`{what}` panicked:\n{stderr}");
    assert!(
        !stderr.contains("Broken pipe") && !stderr.contains("failed printing"),
        "`{what}` complained about the pipe:\n{stderr}"
    );
}

/// What an ordinary run of the command exits with, and whether it printed.
fn ordinary(home: &PathBuf, args: &[&str]) -> (Option<i32>, bool) {
    let out = dspi(home, args)
        .stderr(Stdio::piped())
        .output()
        .expect("spawn");
    (out.status.code(), !out.stdout.is_empty())
}

/// Every command that prints, in a form that gets as far as printing on the
/// demo device.
/// `watch` has its own test, since a normal run of it does not end.
const PRINTING: &[&[&str]] = &[
    &["--help"],
    &["-h"],
    &["--version"],
    &["-V"],
    &["--demo", "list"],
    &["--demo", "list", "--json"],
    &["params"],
    &["completions", "bash"],
    &["completions", "zsh"],
    &["completions", "fish"],
    &["completions", "powershell"],
    &["--demo", "doctor"],
    &["--demo", "--doctor"],
    &["--demo", "dump"],
    &["--demo", "dump", "--json"],
    &["--demo", "screenshot", "80", "24"],
    &["screenshot", "--fixture", "80", "24"],
    &["--demo", "export", "out.txt"],
    &["--demo", "export", "out.dspipreset"],
    &["--demo", "import", "in.txt"],
    &["--demo", "import", "in.dspipreset"],
    &["--demo", "get", "vol.user"],
    &["--demo", "get", "vol.user", "--json"],
    &["--demo", "set", "vol.user", "-18"],
    &["--demo", "set", "vol.user", "-18", "--json"],
    &["--demo", "vol.user", "-18"],
    &["--demo", "eq", "usb.1", "3", "peak", "2856", "3.58", "-8.6"],
    &["--demo", "undo"],
    &["--demo", "redo"],
    // -- WP P1TZF -- 0x51 (save params) takes a confirmation now; `raw` refuses it
    &["--demo", "raw", "0x9A", "1"],
    &["--demo", "raw", "0x9A", "1", "--json"],
    &["--demo", "raw", "out", "0x42", "0", "08", "03", "01", "00"],
    &[
        "--demo", "raw", "out", "0x42", "0", "08", "03", "01", "00", "--json",
    ],
    &["--demo", "raw", "war", "0x9A", "0"],
    &["--demo", "raw", "war", "0x9A", "0", "--json"],
    // Prints, and then exits 3 for the device's refusal.
    &["--demo", "raw", "war", "0xC3", "0"],
    &["--demo", "ext"],
    &["--demo", "ext", "list", "--json"],
    &["--demo", "ext", "xfeed-map"],
    &["--demo", "ext", "vreg"],
    &["autoeq"],
    &["autoeq", "search", "hd"],
    &["autoeq", "search", "no-such-headphone-at-all"],
    &["--demo", "-f", "script.dspi"],
    &["--demo", "--json", "-f", "script.dspi"],
];

/// The files `import` and `-f` read.
fn seed(home: &PathBuf) {
    std::fs::write(home.join("script.dspi"), "vol.user -18\nget vol.user\n").expect("script");
    for (made, name) in [("in.txt", "in.txt"), ("in.dspipreset", "in.dspipreset")] {
        let out = dspi(home, &["--demo", "export", name])
            .output()
            .expect("seed export");
        assert!(out.status.success(), "seeding {made}: {out:?}");
    }
}

#[test]
fn tm15_every_printing_command_survives_a_closed_reader() {
    let home = scratch("every-command");
    seed(&home);

    let mut failures = Vec::new();
    for args in PRINTING {
        let what = args.join(" ");
        let (code, printed) = ordinary(&home, args);
        assert!(printed, "`{what}` printed nothing, so it proves nothing");

        let child = dspi(&home, args)
            .stdout(closed_writer())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        let (status, stderr) = finish(child, &what);
        assert_quiet(&what, &stderr);
        if status.code() != code {
            failures.push(format!("`{what}`: {status}, ordinarily {code:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A verb nobody listed above would slip past the test; this catches it.
#[test]
fn tm15_the_table_names_every_verb_of_the_grammar() {
    for (verb, _) in dspi_cmd::VERBS {
        assert!(
            PRINTING.iter().any(|args| args.contains(verb)),
            "{verb} is not exercised by PRINTING"
        );
    }
}

/// `watch` loops forever, so noticing the closed reader is its only way out.
#[test]
fn tm15_watch_stops_when_the_reader_goes() {
    let home = scratch("watch");
    let child = dspi(&home, &["--demo", "watch"])
        .stdout(closed_writer())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let (status, stderr) = finish(child, "watch");
    assert_quiet("watch", &stderr);
    assert_eq!(status.code(), Some(0), "watch exited with {status}");
}

/// The shape from the bug report: the reader takes a little and then leaves.
#[test]
fn tm15_reader_that_leaves_part_way_like_head() {
    let home = scratch("head");
    for args in [&["--demo", "dump"][..], &["params"][..]] {
        let what = args.join(" ");
        let (reader, writer) = pipe().expect("pipe");
        let child = dspi(&home, args)
            .stdout(Stdio::from(writer))
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        let mut lines = BufReader::new(reader).lines();
        for _ in 0..5 {
            assert!(lines.next().is_some(), "`{what}` printed under 5 lines");
        }
        drop(lines);
        let (status, stderr) = finish(child, &what);
        assert_quiet(&what, &stderr);
        assert_eq!(status.code(), Some(0), "`{what}`");
    }
}

/// A closed reader does not leave a script half applied and call that success:
/// the later lines still run, and the failure on line 2 is still reported.
#[test]
fn tm15_script_runs_to_its_usual_status() {
    let home = scratch("script");
    std::fs::write(home.join("s.dspi"), "get vol.user\nnot-a-command\n").expect("script");
    let child = dspi(&home, &["--demo", "-f", "s.dspi"])
        .stdout(closed_writer())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let (status, stderr) = finish(child, "script");
    assert_quiet("script", &stderr);
    assert_eq!(status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("line 2"), "{stderr}");
}

/// Failing commands keep their own status, whatever happened to the error text.
#[test]
fn tm15_closed_stderr_keeps_the_commands_exit_code() {
    let home = scratch("stderr");
    for (args, want) in [
        (&["--demo", "export"][..], 1),
        (&["--demo", "get", "no.such.path"][..], 1),
        (&["--bogus-theme", "--theme", "nope"][..], 1),
    ] {
        let what = args.join(" ");
        let out = dspi(&home, args)
            .stdout(Stdio::null())
            .stderr(closed_writer())
            .status()
            .expect("spawn");
        // A panic would be 101, and its message has nowhere to go here.
        assert_eq!(out.code(), Some(want), "`{what}`");
    }
}

/// Both streams into the same closed pipe, as `2>&1 | head -1` leaves them.
#[test]
fn tm15_both_streams_closed() {
    let home = scratch("both");
    let (reader, writer) = pipe().expect("pipe");
    let second = writer.try_clone().expect("clone");
    drop(reader);
    let status = dspi(&home, &["--demo", "dump"])
        .stdout(Stdio::from(writer))
        .stderr(Stdio::from(second))
        .status()
        .expect("spawn");
    assert_eq!(status.code(), Some(0));
}

/// A full disk is a real failure, not a reader that left: it is reported on
/// stderr and exits non-zero, rather than panicking or passing for success.
#[cfg(target_os = "linux")]
#[test]
fn tm15_other_write_errors_are_reported() {
    let home = scratch("full");
    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("/dev/full");
    let out = dspi(&home, &["params"])
        .stdout(Stdio::from(full))
        .stderr(Stdio::piped())
        .output()
        .expect("spawn");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(4), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert!(stderr.contains("standard output"), "{stderr}");
}

/// Ordinary output is what it always was, byte for byte through the new path.
#[test]
fn tm15_open_pipe_output_is_unchanged() {
    let home = scratch("open");
    let out = dspi(&home, &["--version"]).output().expect("spawn");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.starts_with(concat!(dspi_cmd::command!(), " ")),
        "{text}"
    );
    assert!(text.ends_with('\n'), "{text}");
    let out = dspi(&home, &["completions", "fish"])
        .output()
        .expect("spawn");
    assert!(out.status.success());
    assert!(!out.stdout.is_empty());
}
