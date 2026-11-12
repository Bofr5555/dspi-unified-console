//! WP TM12: `uconsole autoeq` update commands as a script sees them. None of these
//! touches the network: `rebuild` without `--yes` stops first, and the fetching
//! paths are tested in the binary's own unit tests, on a scripted fetch.
//! Scratch lives in the target directory, never a tmpfs.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const DSPI: &str = env!(concat!("CARGO_BIN_EXE_", dspi_cmd::command!()));
const DB: &str = r#"{"version":1,"generatedAt":"2026","entries":[{"id":"a"},{"id":"b"}]}"#;

fn home(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tm12-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(concat!("config/", dspi_cmd::command!()))).unwrap();
    dir
}

fn dspi(home: &Path, args: &[&str]) -> Output {
    Command::new(DSPI)
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn db_path(home: &Path) -> PathBuf {
    home.join(concat!(
        "config/",
        dspi_cmd::command!(),
        "/autoeq_database.json"
    ))
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn tm12_import_then_reset() {
    let home = home("import");
    let file = home.join("mine.json");
    std::fs::write(&file, DB).unwrap();

    let o = dspi(&home, &["autoeq", "import", file.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    assert!(stdout(&o).contains("Entries: 2"), "{o:?}");
    assert_eq!(std::fs::read_to_string(db_path(&home)).unwrap(), DB);

    let o = dspi(&home, &["autoeq", "reset"]);
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    assert!(stdout(&o).contains("Reset to built-in"), "{o:?}");
    assert!(!db_path(&home).exists());
}

/// A database that will not load is exactly when these are wanted.
#[test]
fn tm12_reset_mends_a_broken_database() {
    let home = home("broken");
    std::fs::write(db_path(&home), "not json").unwrap();
    assert_eq!(dspi(&home, &["autoeq"]).status.code(), Some(1));
    let o = dspi(&home, &["autoeq", "reset"]);
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    assert_eq!(dspi(&home, &["autoeq"]).status.code(), Some(0));
}

#[test]
fn tm12_rebuild_asks_first() {
    let home = home("asks");
    let o = dspi(&home, &["autoeq", "rebuild"]);
    assert_eq!(o.status.code(), Some(1), "{o:?}");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("api.github.com"), "{err}");
    assert!(err.contains("--yes"), "{err}");
    assert!(!db_path(&home).exists());
}

#[test]
fn tm12_missing_arguments_are_usage_errors() {
    let home = home("usage");
    for args in [&["autoeq", "update"], &["autoeq", "import"]] {
        assert_eq!(dspi(&home, args).status.code(), Some(1), "{args:?}");
    }
    let o = dspi(&home, &["autoeq", "import", "no-such-file"]);
    assert_eq!(o.status.code(), Some(1), "{o:?}");
    assert!(!db_path(&home).exists());
}
