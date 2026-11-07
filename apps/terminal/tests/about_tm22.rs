//! WP TM22: `dspi --version` credits who made it, from the About table.

use std::process::{Command, Stdio};

const DSPI: &str = env!("CARGO_BIN_EXE_dspi");

#[test]
fn wp_tm22_version_prints_the_tables_credit_lines_after_the_versions() {
    let table = dspi_session::about::about().expect("the built-in table");
    let home = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("about-tm22");
    std::fs::create_dir_all(&home).expect("scratch dir");
    let out = Command::new(DSPI)
        .arg("--version")
        .current_dir(&home)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .stdin(Stdio::null())
        .output()
        .expect("spawn");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("dspi "), "{text}");
    assert!(lines[1].starts_with("wire format"), "{text}");
    let credits: Vec<&str> = table.credit_lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(&lines[2..], credits.as_slice(), "{text}");
}
