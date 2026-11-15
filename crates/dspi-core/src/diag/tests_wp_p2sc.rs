//! WP P2SC: the hub's schedule rules in the diagnostics bundle, with nothing secret.

use super::*;
use crate::records::DiagOptions;

/// A fresh directory. `ci/run.sh` points `TMPDIR` at the home volume, not tmpfs.
fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir()
        .join("wp-p2sc-core")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn wp_p2sc_the_bundle_carries_the_schedule_without_secrets() {
    let d = dir("schedule");
    // As the hub writes it: one rule per line. `secret` stands for any field a later
    // build might add that names one.
    std::fs::write(
        d.join("schedule.json"),
        "{\"v\":1,\"next_id\":2,\"rules\":[\n\
         {\"id\":\"r1\",\"name\":\"Doors open\",\"enabled\":true,\
         \"when\":{\"days\":[\"mon\"],\"time\":\"18:30\"},\
         \"actions\":[{\"t\":\"param.set\",\"path\":\"vol.master\",\"value\":-12.0}],\
         \"targets\":[\"E46058388B1A2E2C\"],\"secret\":\"sch-sekrit-91\",\
         \"state\":{\"last_date\":null,\"last\":null}}\n]}\n",
    )
    .unwrap();
    let out = d.join("bundle.zip");
    let names = DspiCore::new()
        .diagnostics_save(
            out.to_string_lossy().into_owned(),
            DiagOptions {
                hub_store_dir: d.to_string_lossy().into_owned(),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(names.iter().any(|n| n == "hub/schedule.json"), "{names:?}");
    // Entries are stored, not compressed: the text is in the bytes.
    let zip = std::fs::read(&out).unwrap();
    let raw = String::from_utf8_lossy(&zip);
    assert!(raw.contains("Doors open"));
    assert!(raw.contains("E46058388B1A2E2C"));
    assert!(!raw.contains("sch-sekrit-91"), "a secret is in the bundle");
}

#[test]
fn wp_p2sc_no_schedule_is_no_entry_and_a_bad_one_is_left_out() {
    let d = dir("no-schedule");
    let options = || DiagOptions {
        hub_store_dir: d.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let names = DspiCore::new()
        .diagnostics_save(d.join("b.zip").to_string_lossy().into_owned(), options())
        .unwrap();
    assert!(!names.iter().any(|n| n == "hub/schedule.json"));
    std::fs::write(d.join("schedule.json"), "{\"rules\":[ key=abc123secret").unwrap();
    let zip = DspiCore::new().diagnostics_zip(options()).unwrap();
    let raw = String::from_utf8_lossy(&zip);
    assert!(!raw.contains("abc123secret"));
    assert!(raw.contains("schedule.json is not JSON and was left out"));
}
