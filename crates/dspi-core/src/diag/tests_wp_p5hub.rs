//! WP P5HUB: the hub's measurement documents in the diagnostics bundle: the store's
//! index only (kinds, names, serials, sizes), never a document, a note or a curve.

use super::*;
use crate::records::DiagOptions;

/// A fresh directory. `ci/run.sh` points `TMPDIR` at the home volume, not tmpfs.
fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir()
        .join("wp-p5hub-core")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn wp_p5hub_the_bundle_carries_the_measurement_index_only() {
    let d = dir("measurements");
    let root = d.join("measurements");
    // A document as the hub files it, and its index (which also holds the note).
    let folder = root.join("s-DIAGP5HUB");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("1-0.dspipreset"),
        "{\"measurement\":{\"outputs\":[{\"response\":{\"mag\":\"CURVE-P5HUB-MARKER\"}}]}}",
    )
    .unwrap();
    std::fs::write(
        folder.join("index.json"),
        "{\"version\":1,\"documents\":[{\"id\":\"1-0\",\"kind\":\"measurement\",\
         \"name\":\"Living room P5HUB\",\"note\":\"NOTE-P5HUB-MARKER\",\"serial\":\"DIAGP5HUB\",\
         \"savedMs\":1,\"bytes\":9000}]}",
    )
    .unwrap();
    let mic = root.join("mic");
    std::fs::create_dir_all(&mic).unwrap();
    std::fs::write(mic.join("index.json"), "not json").unwrap();
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
    assert!(
        names.iter().any(|n| n == "hub/measurements.json"),
        "{names:?}"
    );
    assert!(!names.iter().any(|n| n.contains("dspipreset")), "{names:?}");
    let zip = std::fs::read(&out).unwrap();
    let raw = String::from_utf8_lossy(&zip);
    assert!(raw.contains("Living room P5HUB"));
    assert!(raw.contains("DIAGP5HUB"));
    assert!(raw.contains("9000"));
    assert!(
        !raw.contains("CURVE-P5HUB-MARKER"),
        "a curve is in the bundle"
    );
    assert!(
        !raw.contains("NOTE-P5HUB-MARKER"),
        "a note is in the bundle"
    );
    assert!(
        raw.contains("is not JSON and was left out"),
        "the torn index's note"
    );
}

#[test]
fn wp_p5hub_no_store_no_file() {
    let d = dir("none");
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
    assert!(
        !names.iter().any(|n| n == "hub/measurements.json"),
        "{names:?}"
    );
}
