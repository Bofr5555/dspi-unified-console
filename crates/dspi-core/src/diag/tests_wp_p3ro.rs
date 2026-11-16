//! WP P3RO: the hub's rollout records in the diagnostics bundle; the images never.

use super::*;
use crate::records::DiagOptions;

/// A fresh directory. `ci/run.sh` points `TMPDIR` at the home volume, not tmpfs.
fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir()
        .join("wp-p3ro-core")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("firmware")).unwrap();
    d
}

#[test]
fn wp_p3ro_the_bundle_carries_rollout_json_and_no_image() {
    let d = dir("rollout");
    std::fs::write(
        d.join("rollout.json"),
        "{\"v\":1,\"next_id\":2,\"rollouts\":[{\"id\":\"ro1\",\"target\":\"BB66A44F2AC1E7B6\",\
         \"state\":\"ok\",\"image\":{\"name\":\"dspi-rp2350-1.2.0.uf2\",\"sha256\":\"ab12\",\
         \"firmware_version\":\"1.2.0\"},\"devices\":[]}]}\n",
    )
    .unwrap();
    // An image and the store's index: neither goes in.
    std::fs::write(
        d.join("firmware").join("dspi-rp2350-1.2.0.uf2"),
        b"UF2\nIMAGE-BYTES-P3RO-MARKER",
    )
    .unwrap();
    std::fs::write(
        d.join("firmware").join("index.json"),
        "{\"images\":[{\"name\":\"INDEX-P3RO-MARKER\"}]}",
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
    assert!(names.iter().any(|n| n == "hub/rollout.json"), "{names:?}");
    assert!(!names.iter().any(|n| n.contains("firmware")), "{names:?}");
    let zip = std::fs::read(&out).unwrap();
    let raw = String::from_utf8_lossy(&zip);
    assert!(raw.contains("BB66A44F2AC1E7B6"));
    assert!(raw.contains("dspi-rp2350-1.2.0.uf2"));
    assert!(
        !raw.contains("IMAGE-BYTES-P3RO-MARKER"),
        "an image is in the bundle"
    );
    assert!(
        !raw.contains("INDEX-P3RO-MARKER"),
        "the image index is in the bundle"
    );
}
