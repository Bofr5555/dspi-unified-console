//! WP AN2: what the Android app is given on each firmware, through the real
//! core and the demo device: older and newer wire formats are shown read-only,
//! and V32 firmware (stock final, the K340's fork) connects writable.
//!
//! The old images are derived from the V32 capture by inverting
//! `wire_compat`'s mapping, section by section: not captures from units on old
//! firmware. `apps/android/fixtures/wire/*.json` are these results, which the
//! Android tests start from (regenerate on purpose with
//! `DSPI_WRITE_ANDROID_FIXTURE=1`).

use std::path::PathBuf;
use std::sync::Arc;

use dspi_core::consts::{COMPAT_FIRMWARE_TOO_NEW, COMPAT_FIRMWARE_TOO_OLD, COMPAT_OK};
use dspi_core::{DspiCore, DspiError};
use dspi_proto::generated::SECTIONS;
use dspi_proto::wire_compat::bulk_size;
use dspi_transport::{Capture, FakeDevice, FakeOptions};
use serde_json::{Map, Value, json};

const V32: u8 = 32;

fn off(name: &str) -> usize {
    SECTIONS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, o, _)| *o)
        .unwrap_or_else(|| panic!("no section {name}"))
}

/// The V32 capture with a psychoacoustic bass and a limiter switched on, so a
/// test can see which of them a version's image carries.
fn v32_image() -> Vec<u8> {
    let mut image = Capture::demo().bulk;
    image[off("psybass")] = 1;
    image[off("limiter")] = 1;
    image
}

/// `image` in the layout of wire format `v` (17..=31): the inverse of
/// `wire_compat::normalize_bytes`.
fn downgrade(image: &[u8], v: u8) -> Vec<u8> {
    let size = bulk_size(v).expect("a mapped version");
    let (leveller, preamp, ic, adat) = (
        off("leveller"),
        off("preamp"),
        off("input_config"),
        off("adat_config"),
    );
    let mut d = vec![0u8; size];
    d[..leveller + 16].copy_from_slice(&image[..leveller + 16]);
    let shift = if v < 18 { 4 } else { 0 };
    if v >= 18 {
        d[leveller + 16..preamp].copy_from_slice(&image[leveller + 16..preamp]);
    }
    d[preamp - shift..adat + 8 - shift].copy_from_slice(&image[preamp..adat + 8]);
    if v < 28 {
        // One S/PDIF pin fewer: the tail of the input config sits a byte earlier.
        let at = ic - shift;
        d[at + 10..at + 15].copy_from_slice(&image[ic + 11..ic + 16]);
        d[at + 15] = 0;
    }
    let appended = [
        (23, "psybass", off("upmix") - off("psybass")),
        (25, "upmix", off("subharm") - off("upmix")),
        (
            29,
            "subharm",
            if v == 29 {
                16
            } else {
                off("tube") - off("subharm")
            },
        ),
        (31, "tube", off("limiter") - off("tube")),
    ];
    for (since, name, len) in appended {
        if v >= since {
            let at = off(name);
            d[at..at + len].copy_from_slice(&image[at..at + len]);
        }
    }
    d[0] = v;
    d[6..8].copy_from_slice(&u16::try_from(size).unwrap().to_le_bytes());
    d
}

/// A `REQ_GET_PLATFORM` reply of 7 bytes (firmware from 1.1.6-beta3 on).
fn platform7(minor: u8, patch: u8, beta: u8) -> Vec<u8> {
    let mut p = Capture::demo().platform;
    p[2] = (minor << 4) | patch;
    p[4] = minor;
    p[5] = patch;
    p[6] = beta;
    p
}

/// A `REQ_GET_PLATFORM` reply of 4 bytes, BCD only (older firmware).
fn platform4(minor: u8, patch: u8) -> Vec<u8> {
    let mut p = platform7(minor, patch, 0);
    p.truncate(4);
    p
}

struct Opened {
    core: Arc<DspiCore>,
    device: dspi_transport::FakeRemote,
}

fn open(platform: Vec<u8>, bulk: Vec<u8>, extensions: bool) -> Opened {
    let dev = FakeDevice::new(
        Capture { platform, bulk },
        FakeOptions {
            extensions,
            ..FakeOptions::default()
        },
    );
    let device = dev.remote();
    let core = DspiCore::new();
    core.open_transport(Box::new(dev), "AN2".into(), 0x2E8B)
        .expect("opens");
    Opened { core, device }
}

fn state_json(core: &DspiCore) -> Value {
    serde_json::to_value(core.get_state()).expect("json")
}

/// Every write is refused by the core and nothing reaches the device.
fn assert_nothing_written(o: &Opened) {
    let before = o.device.image();
    for result in [
        o.core.set_preamp(-1.0),
        o.core.set_master_volume(-3.0),
        o.core.apply_all_params(),
    ] {
        assert!(matches!(result, Err(DspiError::ReadOnly)), "{result:?}");
    }
    assert_eq!(o.device.image(), before, "the device's image changed");
}

/// The fixture `name`: the fields the Android tests read. `bulkRaw` is only the
/// image's first byte, which says whether an image was decoded.
fn check_fixture(name: &str, core: &DspiCore) {
    const KEYS: &[&str] = &[
        "platformId",
        "numChannels",
        "numInputChannels",
        "numOutputChannels",
        "formatVersion",
        "fwMajor",
        "fwMinor",
        "fwPatch",
        "fwBeta",
        "compat",
        "bulkValid",
        "masterVolumeDb",
    ];
    let state = state_json(core);
    let mut new = Map::new();
    for key in KEYS {
        new.insert((*key).to_string(), state[key].clone());
    }
    new.insert("bulkRaw".into(), json!([state["bulkRaw"][0]]));
    let new = Value::Object(new);

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/android/fixtures/wire")
        .join(format!("{name}.json"));
    if std::env::var_os("DSPI_WRITE_ANDROID_FIXTURE").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text = serde_json::to_string_pretty(&new).unwrap();
        std::fs::write(&path, text + "\n").unwrap();
        return;
    }
    let old: Value = serde_json::from_slice(
        &std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    )
    .unwrap();
    assert_eq!(old, new, "{name}");
}

#[test]
fn wp_an2_stock_final_firmware_connects_writable() {
    let o = open(platform7(1, 6, 0), Capture::demo().bulk, false);
    let s = o.core.get_state();
    assert_eq!(s.compat, COMPAT_OK);
    assert!(s.bulk_valid);
    assert_eq!(s.format_version, V32);
    assert_eq!(
        (s.fw_major, s.fw_minor, s.fw_patch, s.fw_beta),
        (1, 1, 6, 0)
    );
    o.core.set_preamp(-1.0).expect("writable");
    check_fixture("stock_final_v32", &o.core);
}

#[test]
fn wp_an2_k340_firmware_connects_writable() {
    // bofr-main: V32 and 1.1.6 too, with the extension channel answering and a
    // crossfeed pair map in what was the section's reserved word.
    let mut image = Capture::demo().bulk;
    image[off("crossfeed") + 12..off("crossfeed") + 16].copy_from_slice(&[0x02, 0x13, 0, 0]);
    let o = open(platform7(1, 6, 0), image, true);
    let s = o.core.get_state();
    assert_eq!(s.compat, COMPAT_OK);
    assert!(s.bulk_valid);
    assert_eq!(s.format_version, V32);
    assert_eq!(
        (s.fw_major, s.fw_minor, s.fw_patch, s.fw_beta),
        (1, 1, 6, 0)
    );
    o.core.set_preamp(-1.0).expect("writable");
    check_fixture("k340_v32", &o.core);
}

#[test]
fn wp_an2_every_older_format_decodes_read_only() {
    let image = v32_image();
    let reference = open(platform7(1, 6, 0), image.clone(), false);
    let want = state_json(&reference.core);
    // Fields whose bytes every version since V17 has, some of them moved.
    const SAME: &[&str] = &[
        "masterVolumeDb",
        "preampDb",
        "inputPreampDb",
        "filters",
        "xover",
        "matrixRouting",
        "matrixGain",
        "outputEnabled",
        "outputGainDb",
        "channelDelays",
        "loudnessEnabled",
        "crossfeedEnabled",
        "levellerEnabled",
    ];
    for v in 17..V32 {
        let o = open(platform7(1, 5, 0), downgrade(&image, v), false);
        let s = o.core.get_state();
        assert_eq!(s.compat, COMPAT_FIRMWARE_TOO_OLD, "V{v}");
        assert!(!s.bulk_valid, "V{v}");
        assert_eq!(s.format_version, v, "the device's own format");
        assert_eq!(s.bulk_raw[0], V32, "V{v}: decoded onto the V32 layout");
        let got = state_json(&o.core);
        for key in SAME {
            assert_eq!(got[key], want[key], "V{v}: {key}");
        }
        // What the version does not carry reads as absent, for the UI to hide.
        assert_eq!(s.psybass_enabled, v >= 23, "V{v}: psychoacoustic bass");
        assert!(!s.limiter_enabled[0], "V{v}: limiter");
        assert_nothing_written(&o);
    }
    assert!(reference.core.get_state().limiter_enabled[0]);
}

#[test]
fn wp_an2_released_older_firmware_gives_its_fixtures() {
    let image = v32_image();
    for (name, v, platform) in [
        ("v17_1_1_5_beta3", 17, platform4(1, 5)),
        ("v26_1_1_5_beta6", 26, platform4(1, 5)),
        ("v28_1_1_5", 28, platform4(1, 5)),
        ("v28_1_1_6_early_beta", 28, platform4(1, 6)),
        ("v30_1_1_6_beta3", 30, platform7(1, 6, 3)),
        ("v31_dev", 31, platform7(1, 6, 3)),
    ] {
        let o = open(platform, downgrade(&image, v), false);
        assert_eq!(o.core.get_state().compat, COMPAT_FIRMWARE_TOO_OLD);
        check_fixture(name, &o.core);
    }
}

#[test]
fn wp_an2_a_newer_format_shows_its_v32_prefix_read_only() {
    let mut image = Capture::demo().bulk;
    image.resize(image.len() + 40, 0);
    image[0] = 33;
    let len = u16::try_from(image.len()).unwrap();
    image[6..8].copy_from_slice(&len.to_le_bytes());
    let o = open(platform7(1, 7, 0), image, false);
    let s = o.core.get_state();
    assert_eq!(s.compat, COMPAT_FIRMWARE_TOO_NEW);
    assert!(!s.bulk_valid);
    assert_eq!(s.format_version, 33);
    assert_eq!(s.bulk_raw[0], V32, "decoded");
    assert_nothing_written(&o);
    check_fixture("v33_mock", &o.core);
}

#[test]
fn wp_an2_a_format_that_cannot_be_mapped_leaves_the_image_unread() {
    let mut old = Capture::demo().bulk;
    old[0] = 15;
    let o = open(platform4(1, 4), old, false);
    let s = o.core.get_state();
    assert_eq!(s.compat, COMPAT_FIRMWARE_TOO_OLD);
    assert_eq!(s.bulk_raw[0], 0, "the mirror keeps its defaults");
    assert_nothing_written(&o);
    check_fixture("v15_unmapped", &o.core);

    let mut new = Capture::demo().bulk;
    new.resize(6200, 0);
    new[0] = 34;
    new[5] = 20; // band slots changed: the V32 prefix does not line up
    new[6..8].copy_from_slice(&6200u16.to_le_bytes());
    let o = open(platform7(1, 8, 0), new, false);
    let s = o.core.get_state();
    assert_eq!(s.compat, COMPAT_FIRMWARE_TOO_NEW);
    assert_eq!(s.bulk_raw[0], 0, "the mirror keeps its defaults");
    assert_nothing_written(&o);
    check_fixture("v34_unmapped", &o.core);
}
