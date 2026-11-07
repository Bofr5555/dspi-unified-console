//! WP CG18 tests: an export holds what the device reported, and an import leaves
//! alone what a file does not hold.

use dspi_proto::generated::SECTIONS;
use dspi_transport::{Capture, FakeDevice, FakeOptions};

use super::*;
use crate::DspiError;
use crate::state::{apply_platform, decode_bulk, default_state};

const PLATFORM: &[u8] = include_bytes!("../../../../tests/fixtures/link/device/platform.bin");
const BULK: &[u8] = include_bytes!("../../../../tests/fixtures/link/device/bulk_v32.bin");
const QT: &str = include_str!("../../../../tests/fixtures/preset/qt-core-v32.dspipreset");
const MAC: &str = include_str!("../../../../tests/fixtures/preset/mac-swift-v32.dspipreset");
const EVERY_OPTION: u32 = IMPORT_VOLUMES | IMPORT_HARDWARE;

fn off(name: &str) -> usize {
    SECTIONS.iter().find(|(n, _, _)| *n == name).unwrap().1
}

fn json_of(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

fn capture() -> DspState {
    let mut s = default_state();
    apply_platform(PLATFORM, &mut s).unwrap();
    decode_bulk(BULK, &mut s).unwrap();
    s
}

/// The capture in the V17 layout: the inverse of `wire_compat::normalize` for
/// that format. The bytes V17 held as reserved are zero, as on the firmware, except
/// the first two I2S extension pins, which V16 already had; the third stays unset.
fn v17_image() -> Vec<u8> {
    let size = wire_compat::bulk_size(17).unwrap();
    let mut v = BULK.to_vec();
    let (leveller, preamp, input, adat) = (
        off("leveller"),
        off("preamp"),
        off("input_config"),
        off("adat_config"),
    );
    v[off("global") + 6..off("global") + 8].fill(0); // loudness mask, V19
    v[off("crossfeed") + 3] = 0; // pair mask, V20
    v[off("crossfeed") + 12..off("crossfeed") + 16].fill(0); // pair map
    v[off("i2s_config") + 8..off("i2s_config") + 10].fill(0); // clock pins, V21
    v[input + 5] = 40;
    v[input + 6] = 41;
    v[input + 7] = 0; // the third I2S pin: unset
    v[input + 8..input + 16].fill(0); // S/PDIF pins, mask, clock, ADAT input
    let mut out = vec![0u8; size];
    out[..leveller + 16].copy_from_slice(&v[..leveller + 16]);
    out[preamp - 4..adat + 8 - 4].copy_from_slice(&v[preamp..adat + 8]);
    out[0] = 17;
    out[6..8].copy_from_slice(&(size as u16).to_le_bytes());
    out
}

fn device_with(image: Vec<u8>) -> std::sync::Arc<DspiCore> {
    let cap = Capture {
        platform: Capture::demo().platform,
        bulk: image,
    };
    let core = DspiCore::new();
    core.attach(
        Box::new(FakeDevice::new(cap, FakeOptions::default())),
        "DEV".into(),
        0,
    );
    core.fetch_all().unwrap();
    core
}

/// A connected demo device (V32).
fn demo() -> std::sync::Arc<DspiCore> {
    let core = DspiCore::new();
    core.attach_probed(Box::new(FakeDevice::demo()), "DEMO".into(), 0)
        .unwrap();
    core.fetch_all().unwrap();
    core
}

/// Every setting a complete document holds, moved off its captured value.
fn moved(s: &DspState) -> DspState {
    let mut m = s.clone();
    m.bypass = !m.bypass;
    m.input_source = (m.input_source + 1) % 7;
    m.lg_sound_sync_enabled = !m.lg_sound_sync_enabled;
    m.master_volume_db -= 1.0;
    m.user_volume_db -= 1.0;
    m.input_preamp_db.iter_mut().for_each(|x| *x -= 1.0);
    m.preamp_db -= 1.0;
    m.loudness_enabled = !m.loudness_enabled;
    m.loudness_ref_spl += 1.0;
    m.loudness_intensity += 1.0;
    m.loudness_output_mask ^= 1;
    m.crossfeed_enabled = !m.crossfeed_enabled;
    m.crossfeed_preset = (m.crossfeed_preset + 1) % 4;
    m.crossfeed_freq += 1.0;
    m.crossfeed_feed += 1.0;
    m.crossfeed_itd = !m.crossfeed_itd;
    m.crossfeed_output_pair_mask ^= 1;
    m.leveller_enabled = !m.leveller_enabled;
    m.leveller_speed = (m.leveller_speed + 1) % 3;
    m.leveller_lookahead = !m.leveller_lookahead;
    m.leveller_amount += 1.0;
    m.leveller_max_gain_db += 1.0;
    m.leveller_gate_db += 1.0;
    m.leveller_detector_mask ^= 1;
    m.leveller_apply_mask ^= 1;
    m.psybass_enabled = !m.psybass_enabled;
    m.psybass_cutoff_hz += 1.0;
    m.psybass_harmonics_db += 1.0;
    m.psybass_drive_db += 1.0;
    m.psybass_character_pct += 1.0;
    m.psybass_original_db += 1.0;
    m.psybass_output_mask ^= 1;
    for x in m.upmix.iter_mut().chain(&mut m.subharm).chain(&mut m.tube) {
        *x += 1.0;
    }
    // The switches stay 0 or 1, as a device reports them.
    let flip = |x: f32| if x == 0.0 { 1.0 } else { 0.0 };
    m.upmix[usize::from(UPMIX_PARAM_ENABLED)] = flip(s.upmix[usize::from(UPMIX_PARAM_ENABLED)]);
    for id in [SUBHARM_PARAM_ENABLED, SUBHARM_PARAM_LINK] {
        m.subharm[usize::from(id)] = flip(s.subharm[usize::from(id)]);
    }
    for i in [0, 9] {
        m.tube[i] = flip(s.tube[i]);
    }
    m.channel_delays.iter_mut().for_each(|x| *x += 1.0);
    m.output_gain_db.iter_mut().for_each(|x| *x -= 1.0);
    m.output_muted.iter_mut().for_each(|x| *x = !*x);
    m.output_enabled.iter_mut().for_each(|x| *x = !*x);
    m.output_delay_ms.iter_mut().for_each(|x| *x += 1.0);
    m.limiter_enabled.iter_mut().for_each(|x| *x = !*x);
    m.limiter_threshold_db.iter_mut().for_each(|x| *x -= 1.0);
    m.limiter_release_ms.iter_mut().for_each(|x| *x += 1.0);
    m.limiter_link_group
        .iter_mut()
        .for_each(|x| *x = (*x + 1) % 5);
    // Inputs have no crossover in any document.
    let n_in = usize::from(m.num_input_channels);
    let outputs = m.xover.iter_mut().skip(n_in);
    for bank in m.filters.iter_mut().chain(outputs) {
        bank.iter_mut().for_each(|b| b.freq += 1.0);
    }
    for r in &mut m.matrix_routing {
        r.iter_mut().for_each(|x| *x = !*x);
    }
    for r in &mut m.matrix_invert {
        r.iter_mut().for_each(|x| *x = !*x);
    }
    for r in &mut m.matrix_gain {
        r.iter_mut().for_each(|x| *x -= 1.0);
    }
    m.mck_enabled = !m.mck_enabled;
    m.mck_pin += 1;
    m.mck_multiplier ^= 1;
    m.i2s_bck_pin += 1;
    m.i2s_bck_pin_slave += 1;
    m.i2s_clock_mode ^= 1;
    m.i2s_clock_pin_mode ^= 1;
    m.i2s_input_channels += 2;
    m.i2s_input_rate = (m.i2s_input_rate + 1) % 3;
    m.spdif_inputs_enabled ^= 0x02;
    let pins = usize::from(m.num_pin_outputs);
    m.output_pins.iter_mut().take(pins).for_each(|x| *x += 1);
    m.output_types.iter_mut().for_each(|x| *x ^= 1);
    m.spdif_rx_pins.iter_mut().for_each(|x| *x += 1);
    m.i2s_rx_pins.iter_mut().for_each(|x| *x += 1);
    m.adat_out_enabled = !m.adat_out_enabled;
    m.adat_out_pin += 1;
    m.adat_input_enabled = !m.adat_input_enabled;
    m.adat_input_pin = m.adat_input_pin.wrapping_add(1);
    m.adat_input_clock_mode ^= 1;
    m
}

/// Everything but `bulk_raw`, reporting only the fields that differ.
#[track_caller]
fn assert_same(a: &DspState, b: &DspState, what: &str) {
    let (mut a, mut b) = (a.clone(), b.clone());
    a.bulk_raw.clear();
    b.bulk_raw.clear();
    let text = |s: &DspState| format!("{s:#?}");
    let (a, b) = (text(&a), text(&b));
    let mut field = "";
    let mut differ = Vec::new();
    for (x, y) in a.lines().zip(b.lines()) {
        if x.starts_with("    ") && !x.starts_with("     ") {
            field = x.trim();
        }
        if x != y && differ.last() != Some(&field) {
            differ.push(field);
        }
    }
    assert!(differ.is_empty(), "{what}: {differ:?}");
}

/// `{"schemaVersion": 1, "channels": [...]}` with `rest` added.
fn doc(channels: &str, rest: &str) -> Value {
    parse_config(&format!(
        r#"{{"schemaVersion": 1, "channels": [{channels}] {rest}}}"#
    ))
    .unwrap()
}

// ── Complete documents keep their meaning ──

/// Every key of a complete document is read, so the result does not depend on
/// what the device held before; the old defaults never came into it. This is what
/// holds "exactly as before" for a Mac- or Qt-written document.
#[test]
fn wp_cg18_a_complete_document_does_not_depend_on_the_device() {
    let base = capture();
    let other = moved(&base);
    assert_ne!(base, other);
    for (who, text) in [("Qt", QT), ("Mac", MAC)] {
        let v = parse_config(text).unwrap();
        let a = build_config(&base, &v, EVERY_OPTION);
        let b = build_config(&other, &v, EVERY_OPTION);
        assert_same(&a.state, &b.state, who);
        assert_eq!(a.left_alone, Vec::<String>::new(), "{who}");
        assert!(!a.unreadable, "{who}");
        let (lines, clean) = config_report(&a);
        assert!(clean, "{who}: {lines:?}");
        assert_eq!(lines.len(), 2, "{who}: {lines:?}");
    }
}

/// A Qt document on the device it came from changes no setting at all.
#[test]
fn wp_cg18_round_trip_at_v32() {
    let core = demo();
    let before = core.get_state();
    let text = core.config_export("x".into(), "1".into(), 0).unwrap();
    assert!(text.contains("\"limiter\"") && text.contains("\"tube\""));
    let r = json_of(&core.config_import(text.clone(), EVERY_OPTION));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["clean"], true, "{r}");
    assert_eq!(r["leftAlone"], json!([]), "{r}");
    assert_same(&core.get_state(), &before, "after the import");
    // And back on a device that had moved, it restores every setting.
    let v = parse_config(&text).unwrap();
    let built = build_config(&moved(&before), &v, EVERY_OPTION);
    assert_same(&built.state, &before, "restored");
}

// ── Export per format ──

/// `path` = block, key.
fn has(v: &Value, block: &str, key: &str) -> bool {
    v.get(block).and_then(|b| b.get(key)).is_some()
}

#[test]
fn wp_cg18_each_key_is_exported_from_the_format_that_carries_it() {
    let keys: [(&str, &str, u8); 25] = [
        ("io", "adatEnabled", 17),
        ("io", "adatPin", 17),
        ("leveller", "detectorMask", 18),
        ("leveller", "applyMask", 18),
        ("loudness", "outputMask", 19),
        ("crossfeed", "outputPairMask", 20),
        ("io", "i2sClockMode", 21),
        ("io", "i2sClockPinMode", 21),
        ("io", "i2sBckPinSlave", 21),
        ("io", "spdifEnabledExt", 21),
        ("psybass", "cutoffHz", 23),
        ("io", "adatInputEnabled", 24),
        ("io", "adatInputPin", 24),
        ("io", "adatInputClockMode", 24),
        ("upmix", "strengthPct", 25),
        ("upmix", "presenceDb", 26),
        ("io", "spdifRxPin4", 28),
        ("subharm", "lowDb", 29),
        ("subharm", "outputMask", 29),
        ("subharm", "topDb", 30),
        ("subharm", "ceilingDb", 30),
        ("subharm", "linkPairs", 30),
        ("tube", "driveDb", 31),
        ("crossfeed", "pairMap", 32),
        ("crossfeed", "enabled", 16),
    ];
    let mut s = capture();
    for wire in 16u8..=33 {
        s.format_version = wire;
        let v = json_of(&export_config(&s, "x", "1", 0));
        for (block, key, since) in keys {
            assert_eq!(
                has(&v, block, key),
                wire >= since,
                "{block}.{key} at V{wire}"
            );
        }
        let output = &v["channels"].as_array().unwrap()[8];
        assert_eq!(output.get("limiter").is_some(), wire >= 32, "V{wire}");
        assert_eq!(output["eq"][0].get("qp").is_some(), wire >= 22, "V{wire}");
    }
}

/// An image a V17 device really sends, through the connection, into a document,
/// the Terminal's reader and a V32 device.
#[test]
fn wp_cg18_round_trip_from_v17() {
    let old = device_with(v17_image());
    let s = old.get_state();
    assert_eq!(s.format_version, 17);
    assert_eq!(s.compat, COMPAT_FIRMWARE_TOO_OLD);
    let text = old.config_export("old".into(), "1".into(), 0).unwrap();
    let v = json_of(&text);
    assert_eq!(v["meta"]["wireFormatVersion"], 17);

    for absent in ["psybass", "subharm", "tube"] {
        assert!(v.get(absent).is_none(), "{absent}");
    }
    assert!(v["upmix"].is_null());
    assert!(!has(&v, "loudness", "outputMask"));
    assert!(!has(&v, "crossfeed", "pairMap"));
    assert!(!has(&v, "leveller", "applyMask"));
    for gone in [
        "i2sClockMode",
        "spdifEnabledExt",
        "spdifRxPin4",
        "adatInputPin",
    ] {
        assert!(!has(&v, "io", gone), "{gone}");
    }
    // Only the first S/PDIF pin and the I2S pins the image set: the mirror shows
    // defaults for the rest.
    assert_eq!(v["io"]["spdifRxPins"].as_array().unwrap().len(), 1);
    assert_eq!(v["io"]["i2sRxPins"].as_array().unwrap().len(), 3);
    assert_eq!(v["io"]["i2sRxPins"][1], 40);
    // What the format carries is there, and is the device's.
    assert_eq!(v["io"]["adatPin"], s.adat_out_pin);
    assert_eq!(v["global"]["masterVolumeDb"], json!(s.master_volume_db));
    assert_eq!(v["channels"][0]["name"], s.channel_names[0]);
    assert_eq!(
        v["channels"][8]["gainDb"],
        json!(s.output_gain_db[0]),
        "first output"
    );
    assert_eq!(
        v["channels"][0]["eq"].as_array().unwrap().len(),
        BANDS_PER_CHANNEL
    );
    assert_eq!(v["loudness"]["refSpl"], json!(s.loudness_ref_spl));

    // The Terminal reads it and names the same holes.
    let typed = preset_file::parse(&text).unwrap();
    let named = preset_file::omitted_settings(&typed);
    for key in [
        "loudness.outputMask",
        "crossfeed.outputPairMask",
        "leveller.applyMask",
    ] {
        assert!(named.iter().any(|k| k == key), "{key}: {named:?}");
    }

    // A V32 device keeps its own values for all of them.
    let core = demo();
    let before = core.get_state();
    let r = json_of(&core.config_import(text, 0));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["clean"], false, "{r}");
    assert!(
        r["leftAlone"]
            .as_array()
            .unwrap()
            .contains(&json!("loudness.outputMask"))
    );
    let lines = r["lines"].to_string();
    assert!(lines.contains("Left as they are"), "{lines}");
    assert!(lines.contains("older firmware"), "{lines}");
    let after = core.get_state();
    assert_eq!(after.loudness_output_mask, before.loudness_output_mask);
    assert_eq!(
        after.crossfeed_output_pair_mask,
        before.crossfeed_output_pair_mask
    );
    assert_eq!(after.crossfeed_pair_map, before.crossfeed_pair_map);
    assert_eq!(after.leveller_apply_mask, before.leveller_apply_mask);
    assert_eq!(after.psybass_cutoff_hz, before.psybass_cutoff_hz);
    assert_eq!(after.subharm, before.subharm);
    assert_eq!(after.tube, before.tube);
    assert_eq!(after.upmix, before.upmix);
    assert_eq!(after.limiter_threshold_db, before.limiter_threshold_db);
    // What it does hold has moved.
    assert_eq!(after.channel_names[0], s.channel_names[0]);
    assert_eq!(
        after.master_volume_db, before.master_volume_db,
        "not asked for"
    );
}

#[test]
fn wp_cg18_a_v31_device_exports_everything_but_the_limiter() {
    let size = wire_compat::bulk_size(31).unwrap();
    let mut image = BULK.to_vec();
    image.truncate(size);
    image[0] = 31;
    image[6..8].copy_from_slice(&(size as u16).to_le_bytes());
    let core = device_with(image);
    let v = json_of(&core.config_export("x".into(), "1".into(), 0).unwrap());
    assert!(v.get("tube").is_some());
    assert!(has(&v, "subharm", "ceilingDb"));
    assert!(v["channels"][8].get("limiter").is_none());
    assert!(!has(&v, "crossfeed", "pairMap"));
}

// ── Nothing to export ──

#[test]
fn wp_cg18_no_device_state_is_refused_with_a_typed_reason() {
    let core = DspiCore::new();
    let name = || "x".to_string();
    assert_eq!(
        core.config_export(name(), name(), 0),
        Err(DspiError::Disconnected)
    );
    assert_eq!(core.filters_export(8), Err(DspiError::Disconnected));

    // Attached, nothing read.
    core.attach(Box::new(FakeDevice::demo()), "DEV".into(), 0);
    assert!(matches!(
        core.config_export(name(), name(), 0),
        Err(DspiError::Transfer { .. })
    ));
    assert!(matches!(
        core.filters_export(8),
        Err(DspiError::Transfer { .. })
    ));

    // Firmware whose image cannot be mapped: the mirror is defaults.
    let mut image = BULK.to_vec();
    image[0] = 15;
    let cap = Capture {
        platform: Capture::demo().platform,
        bulk: image,
    };
    let core = DspiCore::new();
    core.attach(
        Box::new(FakeDevice::new(cap, FakeOptions::default())),
        "DEV".into(),
        0,
    );
    let _ = core.fetch_all();
    assert_eq!(
        core.config_export(name(), name(), 0),
        Err(DspiError::Unsupported)
    );

    // Read-only is not "nothing": a filter export from V17 has its bands.
    let old = device_with(v17_image());
    assert!(old.filters_export(2).unwrap().contains("[Output 0"));
}

// ── Import leaves alone ──

#[test]
fn wp_cg18_absent_global_and_blocks_stay() {
    let base = moved(&capture());
    let v = doc(r#"{"channelId": 0}"#, "");
    let b = build_config(&base, &v, EVERY_OPTION);
    assert_same(&b.state, &base, "kept");
    for key in [
        "global.bypass",
        "global.masterVolumeDb",
        "loudness",
        "crossfeed",
        "leveller",
        "matrix",
        "channels.eq",
        "channels.delayMs",
    ] {
        assert!(
            b.left_alone.iter().any(|k| k == key),
            "{key}: {:?}",
            b.left_alone
        );
    }
    let (lines, clean) = config_report(&b);
    assert!(!clean);
    assert!(lines[1].starts_with("Left as they are"), "{lines:?}");
}

#[test]
fn wp_cg18_keys_inside_a_present_block_stay() {
    let base = moved(&capture());
    let v = doc(
        r#"{"channelId": 0}"#,
        r#", "loudness": {"refSpl": 70}, "crossfeed": {"feedDb": 2},
            "leveller": {"gateDb": -50}, "psybass": {"driveDb": 3},
            "upmix": {"strengthPct": 11}, "subharm": {"lowDb": -1}, "tube": {"driveDb": 5}"#,
    );
    let mut want = base.clone();
    want.loudness_ref_spl = 70.0;
    want.crossfeed_feed = 2.0;
    want.leveller_gate_db = -50.0;
    want.psybass_drive_db = 3.0;
    want.upmix[usize::from(UPMIX_PARAM_STRENGTH)] = 11.0;
    want.subharm[usize::from(SUBHARM_PARAM_LOW)] = -1.0;
    want.tube[3] = 5.0;
    let b = build_config(&base, &v, 0);
    assert_same(&b.state, &want, "one key each");
    assert!(b.left_alone.iter().any(|k| k == "loudness.outputMask"));
    assert!(b.left_alone.iter().any(|k| k == "tube.mixPct"));
}

#[test]
fn wp_cg18_a_channels_absent_keys_and_banks_stay() {
    let base = moved(&capture());
    let o = usize::from(base.num_input_channels);
    let v = doc(
        r#"{"isOutput": true, "outputIndex": 0, "name": "Left"},
           {"inputIndex": 1, "eq": []},
           {"isOutput": true, "outputIndex": 1, "muted": true, "gainDb": -3, "eq": "x", "crossover": null},
           {"isOutput": true, "outputIndex": 2,
            "eq": [{"type": 1, "freqHz": 50}], "crossover": [{"type": 34, "freqHz": 80}]}"#,
        "",
    );
    let b = build_config(&base, &v, 0);
    let s = &b.state;
    assert_eq!(s.channel_names[o], "Left");
    for k in [0, 1] {
        assert_eq!(s.filters[o + k], base.filters[o + k], "output {k} EQ");
        assert_eq!(s.xover[o + k], base.xover[o + k], "output {k} crossover");
        assert_eq!(s.channel_delays[o + k], base.channel_delays[o + k]);
        assert_eq!(s.output_delay_ms[k], base.output_delay_ms[k]);
        assert_eq!(s.output_enabled[k], base.output_enabled[k]);
    }
    assert_eq!(s.filters[1], base.filters[1], "an empty input bank");
    assert_eq!(s.output_gain_db[0], base.output_gain_db[0]);
    assert_eq!(s.output_muted[0], base.output_muted[0]);
    assert_eq!(s.output_gain_db[1], -3.0);
    assert!(s.output_muted[1]);
    // A bank with entries replaces, the rest going flat.
    assert_eq!(s.filters[o + 2][0].freq, 50.0);
    assert_eq!(s.filters[o + 2][1], flat());
    assert_eq!(s.xover[o + 2][0].filter_type, 34);
    assert_eq!(s.xover[o + 2][1], flat());
    assert_eq!((b.bands, b.xover), (1, 1));
}

#[test]
fn wp_cg18_limiter_crosspoint_and_io_keys_stay() {
    let mut base = moved(&capture());
    // Values a limit would change, so "stay" is not "clamped".
    base.dac_mute_supported = true;
    base.dac_mute_hold_ms = 77;
    base.limiter_release_ms[0] = 5.0;
    base.output_gain_db[0] = 40.0;
    let v = doc(
        r#"{"isOutput": true, "outputIndex": 0, "limiter": {"thresholdDb": -6}}"#,
        r#", "matrix": [{"input": 0, "output": 0, "gainDb": -9}],
            "io": {"mckPin": 3, "outputPins": [9, 7], "dacHwMute": {"pin": 9}}"#,
    );
    let b = build_config(&base, &v, IMPORT_HARDWARE);
    let s = &b.state;
    assert_eq!(s.limiter_threshold_db[0], -6.0);
    assert_eq!(s.limiter_enabled[0], base.limiter_enabled[0]);
    assert_eq!(s.limiter_release_ms[0], 5.0);
    assert_eq!(s.limiter_link_group[0], base.limiter_link_group[0]);
    assert_eq!(s.output_gain_db[0], 40.0);
    assert!(b.left_alone.iter().any(|k| k == "channels.limiter.enabled"));
    assert_eq!(s.matrix_gain[0][0], -9.0);
    assert_eq!(s.matrix_routing[0][0], base.matrix_routing[0][0]);
    assert_eq!(s.matrix_invert[0][0], base.matrix_invert[0][0]);
    assert_eq!(s.mck_pin, 3);
    assert_eq!(s.mck_multiplier, base.mck_multiplier);
    assert_eq!(s.i2s_input_rate, base.i2s_input_rate);
    assert_eq!(s.spdif_inputs_enabled, base.spdif_inputs_enabled);
    assert_eq!(s.output_pins[1], 7);
    assert_eq!(s.dac_mute_pin, 9);
    assert_eq!(s.dac_mute_enabled, base.dac_mute_enabled);
    assert_eq!(s.dac_mute_hold_ms, base.dac_mute_hold_ms);
}

#[test]
fn wp_cg18_a_mistyped_value_stays_and_the_report_says_so() {
    let base = moved(&capture());
    let v = doc(
        r#"{"channelId": 0}"#,
        r#", "global": {"bypass": "yes", "inputPreampsDb": ["x", -2]},
            "loudness": {"enabled": 1, "refSpl": "loud"},
            "io": {"outputPins": ["x", 7]}"#,
    );
    let b = build_config(&base, &v, IMPORT_HARDWARE);
    assert_eq!(b.state.output_pins[0], base.output_pins[0]);
    assert_eq!(b.state.output_pins[1], 7);
    assert_eq!(b.state.bypass, base.bypass);
    assert_eq!(b.state.input_preamp_db[0], base.input_preamp_db[0]);
    assert_eq!(b.state.input_preamp_db[1], -2.0);
    assert_eq!(b.state.loudness_enabled, base.loudness_enabled);
    assert_eq!(b.state.loudness_ref_spl, base.loudness_ref_spl);
    assert!(b.unreadable);
    let (lines, clean) = config_report(&b);
    assert!(!clean);
    assert!(
        lines.iter().any(|l| l.contains("could not be read")),
        "{lines:?}"
    );
}

// ── Filter files ──

#[test]
fn wp_cg18_a_filter_section_without_a_bank_leaves_it_alone() {
    let s = capture();
    let text = "# DSPi Console Filter Settings\n# Format: 2\n\n\
        [Input 0: A]\nPreamp -3.0 dB\n\n\
        [Input 1: B]\n\n\
        [Output 0: C (Enabled)]\nFilter  1: ON  PK      Fc   100.0 Hz  Gain  +1.000 dB  Q  1.000\n\n\
        [Output 1: D (Disabled)]\nXover  1: ON  LR4LP   Fc  2000.0 Hz\n";
    let plan = plan_filters(&s, text, &[0, 1, 8, 9]);
    assert_eq!(plan.writes.len(), 3, "Input 1 has nothing to write");
    let w = &plan.writes[0];
    assert_eq!(w.wire, 0);
    assert!(w.filters.is_none() && w.xover.is_none());
    assert_eq!(w.preamp, Some(-3.0));
    let w = &plan.writes[1];
    assert_eq!(w.wire, 8);
    assert!(w.xover.is_none(), "no crossover lines: left alone");
    assert_eq!(w.filters.as_ref().unwrap().len(), BANDS_PER_CHANNEL);
    assert_eq!(w.filters.as_ref().unwrap()[1], flat(), "the rest go flat");
    let w = &plan.writes[2];
    assert_eq!(w.wire, 9);
    assert!(w.filters.is_none(), "no filter lines: left alone");
    assert_eq!(w.enable, Some((1, false)));
    assert_eq!(w.xover.as_ref().unwrap()[1], flat());
    assert!(plan.notes.iter().any(|n| n == NO_EQ_BANK));
    assert!(plan.notes.iter().any(|n| n == NO_XOVER_BANK));
}

#[test]
fn wp_cg18_off_lines_are_a_bank_and_flatten() {
    let s = capture();
    let text = "# DSPi Console Filter Settings\n# Format: 2\n\n\
        [Output 0: C (Enabled)]\nFilter  1: OFF\nXover  1: OFF\n";
    let plan = plan_filters(&s, text, &[8]);
    let w = &plan.writes[0];
    assert_eq!(w.filters.as_ref().unwrap()[0], flat());
    assert_eq!(w.xover.as_ref().unwrap()[0], flat());
    assert!(plan.notes.is_empty(), "{:?}", plan.notes);
}

#[test]
fn wp_cg18_a_rew_file_without_filters_leaves_the_bank_alone() {
    let s = capture();
    let plan = plan_filters(&s, "Preamp: -6.0 dB\n", &[0, 8]);
    assert_eq!(plan.writes.len(), 1, "the output has nothing to write");
    assert!(plan.writes[0].filters.is_none());
    assert_eq!(plan.writes[0].preamp, Some(-6.0));
    assert!(
        plan.notes.iter().any(|n| n == NO_EQ_BANK),
        "{:?}",
        plan.notes
    );
}

#[test]
fn wp_cg18_a_filter_import_keeps_an_absent_bank_on_the_device() {
    let core = demo();
    let before = core.get_state();
    let text = "# DSPi Console Filter Settings\n# Format: 2\n\n[Input 0: A]\nPreamp -3.0 dB\n";
    let r = json_of(&core.filters_import(text.into(), vec![0]));
    assert_eq!(r["ok"], true, "{r}");
    let after = core.get_state();
    assert_eq!(after.input_preamp_db[0], -3.0);
    assert_eq!(after.filters[0], before.filters[0]);
    assert!(r["lines"].to_string().contains(NO_EQ_BANK), "{r}");
}
