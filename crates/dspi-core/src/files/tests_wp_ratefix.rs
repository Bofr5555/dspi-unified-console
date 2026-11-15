//! WP RATEFIX tests: a document holds the STORED input rate (the bulk
//! `input_config.i2s_input_rate`, word 1 of REQ_GET_INPUT_RATE), never the
//! live pipeline rate (word 0), in export, import and snapshot recall.
//!
//! The demo device's live rate is 48 kHz whatever is stored, so a stored
//! 96 kHz or 44.1 kHz is a device whose live rate differs from its setting.

use std::sync::Arc;

use dspi_transport::FakeDevice;

use super::*;

const MAC: &str = include_str!("../../../../tests/fixtures/preset/mac-swift-v32.dspipreset");

fn json_of(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

/// The demo device with `stored` (0 = 44.1k, 1 = 48k, 2 = 96k) as its
/// stored rate, the live rate staying 48 kHz.
fn demo(stored: u8) -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.attach_probed(Box::new(FakeDevice::demo()), "RATEFIX-DEMO".into(), 0)
        .unwrap();
    core.fetch_all().unwrap();
    core.set_input_rate(stored).unwrap();
    core
}

fn stored(core: &DspiCore) -> u8 {
    core.fetch_all().unwrap();
    core.get_state().i2s_input_rate
}

#[test]
fn wp_ratefix_export_holds_the_stored_rate_and_import_keeps_it() {
    let core = demo(2);
    assert_eq!(
        core.fetch_input_rate(),
        Ok(48_000),
        "live differs from stored"
    );
    let doc = core.config_export("r".into(), "t".into(), 0).unwrap();
    assert_eq!(json_of(&doc)["io"]["i2sInputRateHz"], 96_000);

    // Back onto the same device: the stored rate is unchanged.
    let r = json_of(&core.config_import_tok(doc.clone(), IMPORT_HARDWARE));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(stored(&core), 2);
    assert_eq!(
        core.fetch_input_rate(),
        Ok(48_000),
        "the live rate is untouched"
    );

    // Onto a device storing another rate: the document's stored rate lands.
    let other = demo(0);
    let r = json_of(&other.config_import_tok(doc, IMPORT_HARDWARE));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(stored(&other), 2);
}

/// An older document's `i2sInputRateHz` is read as the stored rate, which is
/// what every client wrote there except the Terminal before this WP.
#[test]
fn wp_ratefix_an_old_document_sets_the_stored_rate() {
    let mut v = json_of(MAC);
    assert_eq!(v["io"]["i2sInputRateHz"], 48_000, "the fixture has the key");
    v["io"]["i2sInputRateHz"] = 44_100.into();
    let core = demo(2);
    let r = json_of(&core.config_import_tok(v.to_string(), IMPORT_HARDWARE));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(stored(&core), 0);

    // A document without the key keeps the device's stored rate.
    v["io"].as_object_mut().unwrap().remove("i2sInputRateHz");
    let r = json_of(&core.config_import_tok(v.to_string(), IMPORT_HARDWARE));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(stored(&core), 0);
}

#[test]
fn wp_ratefix_snapshot_recall_restores_the_stored_rate() {
    let core = demo(0);
    let doc = core
        .snapshot_capture_document("rate".into(), String::new())
        .unwrap();
    core.set_input_rate(2).unwrap();
    assert_eq!(stored(&core), 2);
    core.snapshot_recall_document(doc).unwrap();
    assert_eq!(
        stored(&core),
        0,
        "the snapshot's stored rate, not the live 48 kHz"
    );
    assert_eq!(core.fetch_input_rate(), Ok(48_000));
}
