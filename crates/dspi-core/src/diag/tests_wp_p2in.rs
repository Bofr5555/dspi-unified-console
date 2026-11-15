//! WP P2IN: the hub's inventory in the diagnostics bundle, with nothing secret.

use super::*;
use crate::records::DiagOptions;

/// A fresh directory. `ci/run.sh` points `TMPDIR` at the home volume, not tmpfs.
fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir()
        .join("wp-p2in-core")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn wp_p2in_the_bundle_carries_the_inventory_without_secrets() {
    let d = dir("inventory");
    // As the hub writes it: one record per line. `token` stands for any field a later
    // build might add that names a secret.
    std::fs::write(
        d.join("inventory.json"),
        "{\"v\":1,\"devices\":[\n\
         {\"serial\":\"E46058388B1A2E2C\",\"name\":\"Left\",\"platform\":1,\"fw\":\"1.1.7\",\
         \"outputs\":9,\"location\":\"Hall B\",\"tags\":[\"main\"],\"first_seen_s\":1,\
         \"last_seen_s\":2,\"token\":\"tok-sekrit-77\"}\n]}\n",
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
    assert!(names.iter().any(|n| n == "hub/inventory.json"), "{names:?}");
    // Entries are stored, not compressed: the text is in the bytes.
    let zip = std::fs::read(&out).unwrap();
    let raw = String::from_utf8_lossy(&zip);
    assert!(raw.contains("E46058388B1A2E2C"));
    assert!(raw.contains("Hall B"));
    assert!(!raw.contains("tok-sekrit-77"), "a secret is in the bundle");
}

#[test]
fn wp_p2in_no_inventory_is_no_entry_and_a_bad_one_is_left_out() {
    let d = dir("no-inventory");
    let options = || DiagOptions {
        hub_store_dir: d.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let out = d.join("bundle.zip");
    let names = DspiCore::new()
        .diagnostics_save(out.to_string_lossy().into_owned(), options())
        .unwrap();
    assert!(!names.iter().any(|n| n == "hub/inventory.json"));

    std::fs::write(
        d.join("inventory.json"),
        "{\"devices\":[ token=abc123secret",
    )
    .unwrap();
    let zip = DspiCore::new().diagnostics_zip(options()).unwrap();
    let raw = String::from_utf8_lossy(&zip);
    assert!(!raw.contains("abc123secret"));
    assert!(raw.contains("inventory.json is not JSON and was left out"));
}
