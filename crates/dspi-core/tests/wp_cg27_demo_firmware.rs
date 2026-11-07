//! WP CG27: the demo device (`open_demo`, `dspi_transport::fake`) reports the
//! final 1.1.6, the release every console targets and bundles, so no client
//! shows a stale-firmware banner on it. A beta of 1.1.6 sorts below the
//! bundled images.

use std::path::Path;
use std::sync::Arc;

use dspi_core::{DspiCore, uf2_inspect};
use dspi_proto::packets::FirmwareVersion;

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo().expect("the demo opens");
    core
}

#[test]
fn wp_cg27_the_demo_reports_final_1_1_6() {
    let core = demo();
    let s = core.get_state();
    assert_eq!(
        (s.fw_major, s.fw_minor, s.fw_patch, s.fw_beta),
        (1, 1, 6, 0)
    );
    assert_eq!(core.firmware_version_matches("1.1.6"), Ok(true));
    assert_eq!(core.firmware_version_matches("1.1.6-beta4"), Ok(false));
    assert_eq!(core.firmware_version_matches("1.1.6-beta5"), Ok(false));
}

#[test]
fn wp_cg27_the_demo_runs_what_this_build_expects() {
    let core = demo();
    let want = FirmwareVersion::expected();
    assert_eq!(
        want.beta(),
        Some(0),
        "the vendored headers are a final release"
    );
    assert_eq!(core.firmware_version_matches(&want.to_string()), Ok(true));
}

/// The Qt app's bundled images, which its banner compares against.
#[test]
fn wp_cg27_the_demo_runs_the_bundled_images_version() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/qt/firmware");
    let core = demo();
    let mut seen = 0;
    for e in std::fs::read_dir(&dir)
        .expect("the bundled images")
        .flatten()
    {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".uf2") {
            continue;
        }
        let info = uf2_inspect(std::fs::read(e.path()).expect("readable"));
        assert_eq!(
            core.firmware_version_matches(&info.firmware_version),
            Ok(true),
            "{name}: {}",
            info.firmware_version
        );
        seen += 1;
    }
    assert!(
        seen >= 2,
        "the RP2040 and RP2350 images in {}",
        dir.display()
    );
}
