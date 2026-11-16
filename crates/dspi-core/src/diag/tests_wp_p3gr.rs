//! WP P3GR: the hub's groups in the diagnostics bundle, with nothing secret.

use super::*;
use crate::records::DiagOptions;

/// A fresh directory. `ci/run.sh` points `TMPDIR` at the home volume, not tmpfs.
fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir()
        .join("wp-p3gr-core")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn wp_p3gr_the_bundle_carries_the_groups_without_secrets() {
    let d = dir("groups");
    // As the hub writes it: the header and one group per line. `secret` stands for any
    // field a later build might add that names one.
    std::fs::write(
        d.join("groups.json"),
        "{\"v\":1,\"next_id\":3,\"groups\":[\n\
         {\"id\":\"g1\",\"name\":\"Main hall\",\"note\":\"left stack\",\
         \"members\":[\"E46058388B1A2E2C\",\"BB66A44F2AC1E7B6\"],\"secret\":\"grp-sekrit-17\"},\n\
         {\"id\":\"g2\",\"name\":\"Bar\",\"members\":[]}\n]}\n",
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
    assert!(names.iter().any(|n| n == "hub/groups.json"), "{names:?}");
    // Entries are stored, not compressed: the text is in the bytes.
    let zip = std::fs::read(&out).unwrap();
    let raw = String::from_utf8_lossy(&zip);
    assert!(raw.contains("Main hall"));
    assert!(raw.contains("BB66A44F2AC1E7B6"));
    assert!(!raw.contains("grp-sekrit-17"), "a secret is in the bundle");
}

#[test]
fn wp_p3gr_no_groups_is_no_entry_and_a_bad_file_is_left_out() {
    let d = dir("no-groups");
    let options = || DiagOptions {
        hub_store_dir: d.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let names = DspiCore::new()
        .diagnostics_save(d.join("b.zip").to_string_lossy().into_owned(), options())
        .unwrap();
    assert!(!names.iter().any(|n| n == "hub/groups.json"));
    std::fs::write(d.join("groups.json"), "{\"groups\":[ key=grp123secret").unwrap();
    let zip = DspiCore::new().diagnostics_zip(options()).unwrap();
    let raw = String::from_utf8_lossy(&zip);
    assert!(!raw.contains("grp123secret"));
    assert!(raw.contains("groups.json is not JSON and was left out"));
}
