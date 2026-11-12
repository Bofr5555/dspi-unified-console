//! WP TM22: `uconsole --version` credits who made it, from the About table.

use std::process::{Command, Stdio};

const DSPI: &str = env!(concat!("CARGO_BIN_EXE_", dspi_cmd::command!()));

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
    assert!(
        lines[0].starts_with(concat!(dspi_cmd::command!(), " ")),
        "{text}"
    );
    assert!(lines[1].starts_with("wire format"), "{text}");
    let credits: Vec<&str> = table.credit_lines.iter().map(|l| l.text.as_str()).collect();
    // -- WP LIC1 -- then where the licence is
    assert_eq!(&lines[2..lines.len() - 1], credits.as_slice(), "{text}");
    assert_eq!(
        lines[lines.len() - 1],
        concat!(
            "GPL-3.0-only, with additional terms: ",
            dspi_cmd::command!(),
            " --license"
        )
    );
}

/// WP LIC1: `uconsole --license` prints the attributions, the licence notice and the three
/// additional terms, as the About table words them.
#[test]
fn wp_lic1_license_prints_the_notice_and_the_terms() {
    let table = dspi_session::about::about().expect("the built-in table");
    let home = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("about-lic1");
    std::fs::create_dir_all(&home).expect("scratch dir");
    for flag in ["--license", "--licence"] {
        let out = Command::new(DSPI)
            .arg(flag)
            .current_dir(&home)
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join("config"))
            .stdin(Stdio::null())
            .output()
            .expect("spawn");
        assert!(out.status.success());
        let text = String::from_utf8_lossy(&out.stdout);
        let flat = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ");
        let all = flat(&text);
        for line in &table.credit_lines {
            assert!(all.contains(&line.text), "{text}");
        }
        assert!(all.contains(&flat(&table.licence.statement)), "{text}");
        assert_eq!(table.licence.terms.len(), 3);
        for term in &table.licence.terms {
            assert!(all.contains(&flat(&term.text)), "{text}");
        }
    }
}
