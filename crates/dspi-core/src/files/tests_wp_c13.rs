//! WP C13 tests: configuration documents and filter files.

use super::*;
use crate::state::{apply_platform, decode_bulk, default_state};
use proptest::prelude::*;

const PLATFORM: &[u8] = include_bytes!("../../../../tests/fixtures/link/device/platform.bin");
const BULK: &[u8] = include_bytes!("../../../../tests/fixtures/link/device/bulk_v32.bin");
const DSPI_FILTERS: &str = include_str!("../../../../tests/fixtures/DSPiFilters.txt");
const MACOS_FILTERS: &str = include_str!("../../../../tests/fixtures/MacOSFilters.txt");
const WINDOWS_FILTERS: &str = include_str!("../../../../tests/fixtures/WindowsFilters.txt");
const REW_FILTERS: &str = include_str!("../../../../tests/fixtures/REWFilters.txt");

/// A document written by the macOS Console (the Windows Console's interop
/// fixture `mac-shared-schema.dspipreset`, as the old core's test has it).
const MAC_DOC: &str = r#"{"schemaVersion": 1, "meta": {"name": "Desk", "platform": "RP2350", "firmwareVersion": "1.1.5",
    "savedUtc": "2026-07-14T09:31:07.4821563+00:00", "wireFormatVersion": 27},
  "global": {"inputPreampsDb": [-3, -3, 0, 0, 0, 0, 0, 0], "bypass": false, "masterVolumeDb": -20,
             "inputSource": 1, "inputPairLinked": [true, false, false, false]},
  "loudness": {"enabled": true, "refSpl": 80, "intensityPct": 60, "outputMask": 3},
  "crossfeed": {"enabled": false, "preset": 1, "freqHz": 700, "feedDb": 4.5, "itd": true, "outputPairMask": 1},
  "psybass": null, "upmix": null,
  "channels": [{"channelId": 0, "name": "Master L", "isOutput": false, "delayMs": 0,
      "eq": [{"type": 1, "freqHz": 100, "q": 1, "gain": 3, "qp": 0.707, "bypass": false}], "crossover": []},
    {"channelId": 11, "name": "Input 3", "isOutput": false, "delayMs": 0, "eq": [], "crossover": []},
    {"channelId": 10, "name": "PDM", "isOutput": true, "delayMs": 2, "gainDb": -4,
      "crossover": [{"type": 34, "freqHz": 80, "q": 0.707, "gain": 0, "qp": 0.707, "bypass": false}]}],
  "matrix": [{"input": 0, "output": 8, "enabled": true, "invert": false, "gainDb": -3}],
  "io": {"outputPins": [6, 7, 8, 9, 10], "outputSlotTypes": [0, 1, 0, 0], "mckMultiplier": 256,
         "spdifRxPins": [5, 20, 21], "spdifEnabledExt": 1, "i2sInputRateHz": 96000, "dacHwMute": null}}"#;

/// The capture (RP2350, 8 inputs, 9 outputs), decoded.
fn capture() -> DspState {
    let mut s = default_state();
    apply_platform(PLATFORM, &mut s).unwrap();
    decode_bulk(BULK, &mut s).unwrap();
    s
}

/// An RP2350 with 8 inputs and 9 outputs, otherwise defaults.
fn rp2350() -> DspState {
    let mut s = default_state();
    s.platform_id = 1;
    s.num_input_channels = 8;
    s.num_output_channels = 9;
    s.num_channels = 17;
    s.num_pin_outputs = 5;
    s.format_version = 32;
    s
}

fn pk(freq: f32, gain: f32, q: f32) -> FilterParams {
    FilterParams {
        filter_type: 1,
        freq,
        gain,
        q,
        ..flat()
    }
}

fn json_of(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

/// Everything but `bulk_raw`.
fn fields(s: &DspState) -> String {
    let mut s = s.clone();
    s.bulk_raw.clear();
    format!("{s:?}")
}

// ── Configuration documents ──

#[test]
fn wp_c13_channel_ids_match_the_shared_schema() {
    assert_eq!(id_for_input(0), 0);
    assert_eq!(id_for_input(2), 11);
    assert_eq!(id_for_output(0, 9, true), 2);
    assert_eq!(id_for_output(8, 9, true), 10);
    assert_eq!(id_for_output(4, 5, false), 6);
    assert_eq!(ref_for_id(11, 9, true), Some(ChannelRef::Input(2)));
    assert_eq!(ref_for_id(16, 9, true), Some(ChannelRef::Input(7)));
    assert_eq!(ref_for_id(17, 9, true), None);
    assert_eq!(ref_for_id(10, 9, true), Some(ChannelRef::Output(8)));
    assert_eq!(ref_for_id(6, 5, false), Some(ChannelRef::Output(4)));
    assert_eq!(ref_for_id(9, 5, false), None);
    assert_eq!(ref_for_id(-1, 5, false), None);
    assert_eq!(ref_for_id(i64::MIN, 5, false), None);
    assert_eq!(ref_for_id(10, 0, true), None, "no outputs: no PDM");
}

#[test]
fn wp_c13_reads_the_macos_document() {
    let v = parse_config(MAC_DOC).unwrap();
    let inspect = json_of(&config_inspect(MAC_DOC.into()));
    assert_eq!(inspect["ok"], true);
    assert_eq!(inspect["name"], "Desk");
    assert_eq!(inspect["platform"], "RP2350");
    assert_eq!(inspect["firmware"], "1.1.5");
    assert_eq!(inspect["savedUtc"], "2026-07-14T09:31:07.4821563+00:00");

    let old = rp2350();
    let b = build_config(&old, &v, 0);
    let s = &b.state;
    assert_eq!(b.links, Some(0b0001));
    assert_eq!((b.channels, b.bands, b.xover, b.points), (3, 1, 1, 1));
    assert!(b.missing.is_empty() && b.skipped.0.is_empty());
    assert_eq!(s.filters[0][0], pk(100.0, 3.0, 1.0));
    assert_eq!(s.channel_names[0], "Master L");
    assert_eq!(s.channel_names[2], "Input 3");
    // The PDM output is output 8, wire 16.
    assert_eq!(s.channel_names[16], "PDM");
    assert_eq!(
        (s.xover[16][0].filter_type, s.xover[16][0].freq),
        (34, 80.0)
    );
    assert_eq!(s.output_gain_db[8], -4.0);
    assert_eq!(s.channel_delays[16], 2.0);
    assert_eq!(s.input_preamp_db[0], -3.0);
    assert_eq!(s.preamp_db, -3.0);
    assert!(s.matrix_routing[0][8]);
    assert_eq!(s.matrix_gain[0][8], -3.0);
    assert!(s.loudness_enabled && s.loudness_output_mask == 3);
    assert_eq!(s.input_source, 1);
    // Not asked for: volumes and I/O stay.
    assert_eq!(s.master_volume_db, old.master_volume_db);
    assert_eq!(s.output_types, old.output_types);
    // Null blocks are absent: psybass and upmix untouched.
    assert_eq!(s.upmix, old.upmix);
    assert_eq!(s.psybass_cutoff_hz, old.psybass_cutoff_hz);
    // An absent pairMap keeps the current map.
    assert_eq!(s.crossfeed_pair_map, old.crossfeed_pair_map);

    let hw = build_config(&old, &v, IMPORT_HARDWARE | IMPORT_VOLUMES).state;
    assert_eq!(hw.master_volume_db, -20.0);
    assert_eq!(hw.output_types, vec![0, 1, 0, 0]);
    assert_eq!(hw.output_pins, vec![6, 7, 8, 9, 10]);
    assert_eq!(hw.mck_multiplier, 1);
    assert_eq!(hw.spdif_rx_pins[..3], [5, 20, 21]);
    assert_eq!(hw.spdif_inputs_enabled, 0b0011, "input 1 always on");
    assert_eq!(hw.i2s_input_rate, 2);
}

#[test]
fn wp_c13_rejects_empty_future_and_oversized_files() {
    assert!(parse_config("{}").unwrap_err().contains("no channel data"));
    assert!(
        parse_config(r#"{"schemaVersion": 1, "channels": []}"#)
            .unwrap_err()
            .contains("no channel data")
    );
    assert!(
        parse_config(r#"{"schemaVersion": 2, "channels": [{}]}"#)
            .unwrap_err()
            .contains("format 2")
    );
    assert!(
        parse_config("not json")
            .unwrap_err()
            .starts_with("Not a valid configuration file")
    );
    assert!(parse_config("[1, 2]").is_err());
    let big = format!(
        r#"{{"schemaVersion": 1, "channels": [{{}}], "pad": "{}"}}"#,
        " ".repeat(MAX_FILE_BYTES)
    );
    assert!(parse_config(&big).unwrap_err().contains("larger than"));
    let v = json_of(&config_inspect("nope".into()));
    assert_eq!(v["ok"], false);
    assert!(v["error"].as_str().unwrap().starts_with("Not a valid"));
}

#[test]
fn wp_c13_lenient_fields_take_defaults() {
    // Wrong types, nonsense numbers, out-of-range indices and types.
    let doc = r#"{"schemaVersion": 1.0,
      "global": {"inputPreampsDb": ["x", 1e300, -100, 99, null], "inputSource": 99},
      "loudness": "on", "leveller": {"speed": -7, "amountPct": "lots"},
      "channels": [
        {"inputIndex": 0, "name": "A very long channel name that does not fit\u0000tail",
         "delayMs": 1e9, "eq": [{"type": 99}, {"type": 11}, {"type": -1}, {"type": 1, "freqHz": 1e300}]},
        {"isOutput": true, "outputIndex": 99, "name": "Gone"},
        {"isOutput": true, "outputIndex": 0, "gainDb": 1000, "crossover": [{"type": 5}, {"type": 63}]},
        {"channelId": "x"}, 7, null],
      "matrix": [{"input": -1, "output": 0}, {"input": 0, "output": 99, "enabled": true}, "x",
                 {"input": 1, "output": 1, "enabled": true, "gainDb": "loud"}]}"#;
    let v = parse_config(doc).unwrap();
    let old = rp2350();
    let b = build_config(&old, &v, 0);
    let s = &b.state;
    assert_eq!(s.input_preamp_db[..5], [0.0, 0.0, -60.0, 24.0, 0.0]);
    assert_eq!(s.input_source, 6);
    assert_eq!(s.leveller_speed, 0);
    assert_eq!(s.leveller_amount, 50.0);
    assert_eq!(s.loudness_enabled, old.loudness_enabled, "not an object");
    assert_eq!(s.channel_names[0], "A very long channel name that d");
    assert_eq!(s.channel_names[0].len(), CHANNEL_NAME_LEN - 1);
    assert_eq!(s.channel_delays[0], 42.0);
    assert_eq!(s.filters[0][0].filter_type, 0, "unknown PEQ type");
    assert_eq!(s.filters[0][1].filter_type, 0, "no LT on inputs");
    assert_eq!(s.filters[0][2].filter_type, 0);
    assert_eq!(s.filters[0][3].freq, 1000.0, "infinite as f32: default");
    assert_eq!(s.output_gain_db[0], 12.0);
    assert_eq!(
        s.xover[8][0].filter_type, 0,
        "a PEQ type is not a crossover"
    );
    assert_eq!(s.xover[8][1].filter_type, 63);
    assert!(s.matrix_routing[1][1] && s.matrix_gain[1][1] == 0.0);
    assert_eq!(b.points, 1);
    assert_eq!(b.missing.names, ["Gone", "channel -1"], "each label once");
    let (lines, clean) = config_report(&b);
    assert!(!clean);
    assert_eq!(
        lines[0],
        "Applied 2 channels, 1 EQ band, 1 crossover band and 1 crosspoint."
    );
    assert!(lines[2].starts_with("Skipped: filter types this firmware doesn't have (set to Off); Linkwitz Transforms on inputs (set to Off); crossover types"));
    assert_eq!(lines.last().unwrap(), NOT_STORED);
}

#[test]
fn wp_c13_rp2040_skips_the_upmixer() {
    let v = parse_config(
        r#"{"schemaVersion": 1, "upmix": {"enabled": true, "strengthPct": 10},
            "channels": [{"channelId": 0}]}"#,
    )
    .unwrap();
    let old = default_state();
    let b = build_config(&old, &v, 0);
    assert_eq!(b.state.upmix, old.upmix);
    assert_eq!(b.skipped.0, ["the stereo upmixer (not on RP2040)"]);
}

#[test]
fn wp_c13_export_round_trips_the_capture() {
    let s = capture();
    let text = export_config(&s, "Capture", "0.1", 0b0101);
    let v = parse_config(&text).unwrap();
    assert_eq!(v["meta"]["name"], "Capture");
    assert_eq!(v["meta"]["platform"], "RP2350");
    assert_eq!(
        v["global"]["inputPairLinked"],
        json!([true, false, true, false])
    );
    assert!(
        text.find("\"channels\"").unwrap() < text.find("\"crossfeed\"").unwrap(),
        "keys sorted"
    );
    let b = build_config(&s, &v, IMPORT_VOLUMES | IMPORT_HARDWARE);
    assert_eq!(b.links, Some(0b0101));
    assert!(b.missing.is_empty(), "{:?}", b.missing);
    assert!(b.skipped.0.is_empty(), "{:?}", b.skipped.0);
    assert_eq!(
        fields(&b.state),
        fields(&s),
        "export then import changes nothing"
    );
    assert_eq!(b.state.bulk_raw, s.bulk_raw);
}

#[test]
fn wp_c13_export_round_trips_an_edited_state() {
    let mut s = rp2350();
    s.filters[9][0] = pk(250.0, -2.5, 1.4);
    s.filters[9][1] = FilterParams {
        filter_type: 11,
        freq: 40.0,
        q: 0.6,
        gain: 28.0,
        qp: 0.5,
        bypass: true,
    };
    s.xover[12][3] = FilterParams {
        filter_type: 57,
        ..flat()
    };
    s.channel_names[8] = "Left Woofer".into();
    s.channel_names[3] = "Ünïcödé".into();
    s.matrix_routing[1][3] = true;
    s.matrix_invert[7][8] = true;
    s.output_enabled = vec![true, false, true, true, false, false, true, true, false];
    s.limiter_enabled[2] = true;
    s.limiter_threshold_db[2] = -6.0;
    s.limiter_link_group[2] = 3;
    s.crossfeed_pair_map = vec![0x02, 0x13, 0, 0];
    s.upmix[UPMIX_PARAM_CENTER_MODE as usize] = 2.0;
    s.subharm[SUBHARM_PARAM_SELECT as usize] = 1.0;
    s.tube[2] = 5.0;
    s.dac_mute_supported = true;
    s.dac_mute_enabled = true;
    s.dac_mute_hold_ms = 50;
    s.spdif_inputs_enabled = 0b1011;
    s.output_types = vec![1, 0, 1, 0];
    s.mck_multiplier = 1;
    s.i2s_input_rate = 0;
    s.fw_major = 1;
    s.fw_minor = 1;
    s.fw_patch = 6;
    s.fw_beta = 5;
    let text = export_config(&s, "K340", "0.1", 0);
    let v = parse_config(&text).unwrap();
    assert_eq!(v["meta"]["firmwareVersion"], "1.1.6-beta5");
    assert_eq!(v["crossfeed"]["pairMap"], json!([2, 19, 0, 0]));
    let out1 = array(&v, "channels")
        .iter()
        .find(|c| c["outputIndex"] == 1)
        .unwrap();
    assert_eq!(out1["channelId"], 3);
    assert_eq!(out1["eq"][0]["freqHz"], 250.0);
    // Import into a fresh device of the same kind.
    let mut target = rp2350();
    target.dac_mute_supported = true;
    // The firmware version is the device's, not the document's.
    (
        target.fw_major,
        target.fw_minor,
        target.fw_patch,
        target.fw_beta,
    ) = (1, 1, 6, 5);
    let b = build_config(&target, &v, IMPORT_VOLUMES | IMPORT_HARDWARE);
    assert_eq!(fields(&b.state), fields(&s));
}

#[test]
fn wp_c13_pair_map_import() {
    let doc = |map: &str| {
        parse_config(&format!(
            r#"{{"schemaVersion": 1, "crossfeed": {{"pairMap": {map}}}, "channels": [{{"channelId": 0}}]}}"#
        ))
        .unwrap()
    };
    let mut old = rp2350();
    old.crossfeed_pair_map = vec![0x10, 0, 0, 0];
    // The K340 map, kept.
    let b = build_config(&old, &doc("[2, 19, 0, 0]"), 0);
    assert_eq!(b.state.crossfeed_pair_map, [0x02, 0x13, 0, 0]);
    // Explicit defaults are normalised to 0.
    let b = build_config(&old, &doc("[16, 50, 84, 118]"), 0);
    assert_eq!(b.state.crossfeed_pair_map, [0, 0, 0, 0]);
    // Invalid (output used twice, out of range, wrong type) = default.
    for bad in ["[2, 2, 0, 0]", "[300, 0, 0, 0]", "[-5]", "[255, 0, 0, 0]"] {
        let b = build_config(&old, &doc(bad), 0);
        assert_eq!(b.state.crossfeed_pair_map, [0, 0, 0, 0], "{bad}");
    }
    // A short map is padded; extra entries are never read.
    let b = build_config(&old, &doc("[1]"), 0);
    assert_eq!(b.state.crossfeed_pair_map, [1, 0, 0, 0]);
    let b = build_config(&old, &doc("[0, 0, 0, 0, 99, 99]"), 0);
    assert_eq!(b.state.crossfeed_pair_map, [0, 0, 0, 0]);
    // RP2040 has two pairs: a map using pair 2 is invalid there.
    let b = build_config(&default_state(), &doc("[0, 0, 84, 0]"), 0);
    assert_eq!(b.state.crossfeed_pair_map, [0, 0, 0, 0]);
    // Not an array: kept.
    let b = build_config(&old, &doc("null"), 0);
    assert_eq!(b.state.crossfeed_pair_map, old.crossfeed_pair_map);
}

#[test]
fn wp_c13_export_keeps_the_old_core_shape() {
    // The keys the Qt app, the Consoles and the Android app read.
    let v = parse_config(&export_config(&capture(), "", "", 0)).unwrap();
    let keys = |x: &Value| x.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(
        keys(&v),
        [
            "channels",
            "crossfeed",
            "global",
            "io",
            "leveller",
            "loudness",
            "matrix",
            "meta",
            "psybass",
            "schemaVersion",
            "subharm",
            "tube",
            "upmix"
        ]
    );
    assert_eq!(
        keys(&v["meta"]),
        [
            "appVersion",
            "firmwareVersion",
            "inputChannelCount",
            "masterVolumeMode",
            "name",
            "outputChannelCount",
            "outputConfigMode",
            "platform",
            "savedUtc",
            "wireFormatVersion"
        ]
    );
    assert_eq!(
        keys(&v["channels"][0]),
        [
            "channelId",
            "crossover",
            "delayMs",
            "enabled",
            "eq",
            "eqChannel",
            "gainDb",
            "inputIndex",
            "isOutput",
            "muted",
            "name"
        ]
    );
    assert_eq!(
        keys(&v["channels"][8]),
        [
            "channelId",
            "crossover",
            "delayMs",
            "enabled",
            "eq",
            "eqChannel",
            "gainDb",
            "isOutput",
            "limiter",
            "muted",
            "name",
            "outputDelayMs",
            "outputIndex"
        ]
    );
    assert_eq!(
        keys(&v["channels"][0]["eq"][0]),
        ["bypass", "freqHz", "gain", "q", "qp", "type"]
    );
    assert_eq!(array(&v, "channels").len(), 17);
    assert_eq!(array(&v, "matrix").len(), 72);
    assert_eq!(array(&v, "channels")[0]["eq"].as_array().unwrap().len(), 10);
    assert_eq!(
        array(&v, "channels")[8]["crossover"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(v["io"]["outputPins"].as_array().unwrap().len(), 5);
    assert_eq!(v["io"]["spdifRxPins"].as_array().unwrap().len(), 3);
    assert_eq!(
        keys(&v["crossfeed"]).len(),
        7,
        "the old core's six + pairMap"
    );
    assert_eq!(keys(&v["upmix"]).len(), 14);
    assert_eq!(keys(&v["subharm"]).len(), 11);
    assert_eq!(keys(&v["tube"]).len(), 14);
    assert!(v["io"]["dacHwMute"].is_null());
    // RP2040: no upmixer block.
    let v = parse_config(&export_config(&default_state(), "", "", 0)).unwrap();
    assert!(v["upmix"].is_null());
    assert_eq!(v["channels"][6]["channelId"], 6, "no PDM id on RP2040");
}

#[test]
fn wp_c13_utc_strings() {
    assert_eq!(utc_string(0), "1970-01-01T00:00:00Z");
    assert_eq!(utc_string(1_700_000_000), "2023-11-14T22:13:20Z");
    assert_eq!(utc_string(951_782_400), "2000-02-29T00:00:00Z");
    assert_eq!(utc_string(u64::MAX), "9999-12-31T23:59:59Z");
}

// ── Filter files ──

#[test]
fn wp_c13_codes_round_trip() {
    for t in (1..=13).chain(32..=63) {
        let code = file_code(t).unwrap();
        assert_eq!(type_for_code(&code), Some(t), "{code}");
    }
    assert_eq!(file_code(0), None);
    assert_eq!(file_code(14), None);
    assert_eq!(file_code(64), None);
    assert_eq!(type_for_code("PEQ"), Some(1));
    assert_eq!(type_for_code("lr4lp"), Some(34));
    assert_eq!(type_for_code("None"), None);
    assert_eq!(type_for_code("LR3LP"), None);
    assert_eq!(type_for_code("BW99HP"), None);
    assert_eq!(type_for_code("BW999HP"), None);
    assert_eq!(type_for_code("LRHP"), None);
    assert_eq!(crossover_type("BESSEL", 8, true), Some(63));
    assert_eq!(crossover_type("BW", 0, true), None);
}

#[test]
fn wp_c13_band_lines_round_trip() {
    let p = FilterParams {
        bypass: true,
        ..pk(100.0, 3.0, 1.0)
    };
    let line = format_band(false, 3, &p);
    assert_eq!(
        line,
        "Filter  3: ON  PK      Fc   100.0 Hz  Gain  +3.000 dB  Q  1.000  [Bypassed]\n"
    );
    assert_eq!(parse_band(&line).unwrap().params, p);
    let lt = FilterParams {
        filter_type: 11,
        bypass: false,
        freq: 40.0,
        q: 0.7,
        gain: 28.0,
        qp: 0.71,
    };
    assert_eq!(
        format_band(false, 4, &lt),
        "Filter  4: ON  LT      Fc    40.0 Hz  Q  0.700  Fp    28.0 Hz  Qp  0.710\n"
    );
    assert_eq!(parse_band(&format_band(false, 4, &lt)).unwrap().params, lt);
    assert_eq!(format_band(true, 1, &flat()), "Xover   1: OFF\n");
    let x = parse_band("Xover   1: ON  LR4LP   Fc  2000.0 Hz").unwrap();
    assert!(x.xover && x.params.filter_type == 34 && x.params.freq == 2000.0);
    assert!(!parse_band("Filter  5: OFF").unwrap().enabled);
    assert!(!parse_band("Filter  5: ON").unwrap().enabled);
    assert!(parse_band("Filters: 5").is_none());
    assert!(parse_band("Filter").is_none());
    assert!(parse_band("Filter x: ON PK").is_none());
    assert_eq!(number("1.234,5"), Some(1234.5));
    assert_eq!(number("105Hz"), Some(105.0));
    assert_eq!(number("1e40"), None, "not finite");
    assert_eq!(number("dB/oct"), None);
}

#[test]
fn wp_c13_reads_rew_and_other_spellings() {
    let rew = "Filter Settings file\nPreamp: -6,5 dB\nFilter 1: ON PK Fc 100 Hz Gain -3.5 dB Q 2.00\nFilter 2: ON None\nFilter 3: ON LSC Fc 105Hz Gain 4 dB Q 0.71\n";
    match parse_filters(rew, 8) {
        Parsed::Rew {
            filters,
            count,
            preamp,
            dropped,
        } => {
            assert_eq!((filters.len(), count), (2, 2));
            assert_eq!(preamp, Some(-6.5));
            assert_eq!(dropped, 1);
            assert_eq!((filters[1].filter_type, filters[1].freq), (2, 105.0));
        }
        p => panic!("not read as REW: {p:?}"),
    }
    let win = parse_band("Crossover 1: ON BW HP Fc 80 Hz Slope 24 dB/oct").unwrap();
    assert_eq!(
        win.params.filter_type,
        crossover_type("BW", 4, true).unwrap()
    );
    assert_eq!(windows_header("SPDIF 2 R", 8), Some(Chan::Output(3)));
    assert_eq!(windows_header("SPDIF 0 R", 8), None);
    assert_eq!(windows_header("Input 5", 8), Some(Chan::Input(4)));
    assert_eq!(windows_header("Input 9", 8), None);
    assert_eq!(windows_header("pdm", 4), Some(Chan::Output(4)));
}

#[test]
fn wp_c13_native_sections_keep_their_slots() {
    let text = "# DSPi Console Filter Settings\n# Format: 2\n\n[Input 1: Master R]\nPreamp -2.0 dB\nFilter  1: OFF\nFilter  2: ON  PK  Fc 500.0 Hz  Gain -1.000 dB  Q 1.000\n\n[Output 3: Tweeter (Disabled)]\nXover  1: ON  LR4HP  Fc 2000.0 Hz\n\n[Output 99: Nowhere]\nFilter 1: ON PK Fc 1 Hz\n";
    match parse_filters(text, 8) {
        Parsed::Native { channels, version } => {
            assert_eq!(version, 2);
            assert_eq!(channels.len(), 2, "no section for output 99");
            let i1 = &channels[&Chan::Input(1)];
            assert_eq!(
                (i1.preamp, i1.filters.len(), i1.filters[1].freq),
                (Some(-2.0), 2, 500.0)
            );
            let o3 = &channels[&Chan::Output(3)];
            assert_eq!((o3.enable, o3.xover[0].filter_type), (Some(false), 35));
        }
        p => panic!("not read as native: {p:?}"),
    }
}

#[test]
fn wp_c13_the_dspi_fixture() {
    // Format 1, name-keyed sections (the Windows Console's names).
    let s = rp2350();
    let v = inspect_filters(&s, DSPI_FILTERS, 2);
    assert_eq!(v["kind"], "native");
    assert_eq!(v["newer"], false);
    let wires: Vec<u64> = v["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["wire"].as_u64().unwrap())
        .collect();
    assert!(wires.starts_with(&[0, 1, 8]), "{wires:?}");
    assert_eq!(v["channels"][0]["bands"], 10);
    assert_eq!(v["channels"][0]["label"], "Input 1: ");
    let plan = plan_filters(&s, DSPI_FILTERS, &[0, 8]);
    assert_eq!(plan.writes.len(), 2);
    let master_l = &plan.writes[0];
    assert_eq!(master_l.wire, 0);
    assert_eq!(master_l.filters.as_ref().unwrap()[0].filter_type, 2);
    assert_eq!(master_l.filters.as_ref().unwrap()[0].freq, 105.0);
    assert_eq!(master_l.filters.as_ref().unwrap()[0].gain, 8.8);
    assert_eq!(master_l.filters.as_ref().unwrap()[1], pk(64.0, -6.2, 0.30));
    assert_eq!(master_l.filters.as_ref().unwrap()[9], pk(118.0, 0.4, 1.98));
    assert!(master_l.xover.is_none() && master_l.preamp.is_none());
    let out_l = &plan.writes[1];
    assert_eq!(
        (out_l.wire, out_l.filters.as_ref().unwrap()[0].filter_type),
        (8, 5)
    );
    // No crossover lines in this file: the bank is not the file's (WP CG18).
    assert!(out_l.xover.is_none());
    assert_eq!(out_l.enable, None);
}

#[test]
fn wp_c13_the_macos_fixture() {
    let s = rp2350();
    let v = inspect_filters(&s, MACOS_FILTERS, 8);
    assert_eq!(v["kind"], "native");
    let plan = plan_filters(&s, MACOS_FILTERS, &(0..17).collect::<Vec<_>>());
    let fl = &plan.writes[0];
    assert_eq!(fl.preamp, Some(-5.3));
    assert!(fl.filters.as_ref().unwrap()[2].bypass);
    assert_eq!(
        fl.filters.as_ref().unwrap()[3],
        flat(),
        "OFF keeps its slot"
    );
    let out0 = plan.writes.iter().find(|w| w.wire == 8).unwrap();
    assert_eq!(out0.enable, Some((0, true)));
    assert_eq!(out0.filters.as_ref().unwrap()[0].filter_type, 13);
    let lt = out0.filters.as_ref().unwrap()[1];
    assert_eq!(
        (lt.filter_type, lt.freq, lt.q, lt.gain, lt.qp),
        (11, 40.0, 0.5, 28.0, 0.71)
    );
    let x = out0.xover.as_ref().unwrap();
    assert_eq!((x[0].filter_type, x[1].filter_type), (35, 42));
    assert_eq!(x[2], flat());
}

#[test]
fn wp_c13_the_windows_fixture() {
    let s = rp2350();
    let plan = plan_filters(&s, WINDOWS_FILTERS, &(0..17).collect::<Vec<_>>());
    let master_l = &plan.writes[0];
    assert_eq!(master_l.preamp, Some(-6.5));
    assert_eq!(master_l.filters.as_ref().unwrap()[0], pk(105.0, 8.8, 0.7));
    assert_eq!(
        master_l.filters.as_ref().unwrap()[1].freq,
        1234.5,
        "grouping and decimal comma"
    );
    assert_eq!(master_l.filters.as_ref().unwrap()[2].filter_type, 6);
    assert!(master_l.filters.as_ref().unwrap()[2].bypass, "BYP");
    let spdif1_l = plan.writes.iter().find(|w| w.wire == 8).unwrap();
    let x = spdif1_l.xover.as_ref().unwrap();
    assert_eq!(x[0].filter_type, crossover_type("LR", 4, true).unwrap());
    assert_eq!(x[1].filter_type, crossover_type("BW", 2, false).unwrap());
    assert_eq!(x[1].freq, 4000.0);
}

#[test]
fn wp_c13_the_rew_fixture() {
    let s = rp2350();
    let v = inspect_filters(&s, REW_FILTERS, 2);
    assert_eq!(v["kind"], "rew");
    assert_eq!(v["filters"], 4);
    assert_eq!(v["preamp"], -6.5);
    let ch = v["channels"].as_array().unwrap();
    assert_eq!(ch.len(), 2 + 9);
    assert_eq!(ch[0]["checked"], true);
    assert_eq!(ch[2]["checked"], false);
    assert_eq!(ch[2]["label"], "Output 1: ");
    let plan = plan_filters(&s, REW_FILTERS, &[0, 1, 8, 200]);
    assert_eq!(plan.writes.len(), 3, "wire 200 is not on the device");
    let w0 = &plan.writes[0];
    assert_eq!(w0.preamp, Some(-6.5));
    assert_eq!(w0.filters.as_ref().unwrap()[0], pk(31.5, -4.2, 3.5));
    assert_eq!(w0.filters.as_ref().unwrap()[2].filter_type, 2);
    assert_eq!(w0.filters.as_ref().unwrap()[4], flat());
    assert_eq!(plan.writes[2].preamp, None, "outputs have no preamp");
    let lines = filter_report(plan.writes.len(), &plan.notes);
    assert_eq!(lines[0], "Applied filters to 3 channels.");
    assert_eq!(
        lines[1],
        "2 filter lines that were off or of an unknown type were left out."
    );
    let only_out = plan_filters(&s, REW_FILTERS, &[9]);
    assert!(only_out.notes.iter().any(|n| n.contains("inputs only")));
}

#[test]
fn wp_c13_rew_with_too_many_filters() {
    let text: String = (1..=14)
        .map(|i| format!("Filter {i}: ON PK Fc {} Hz Gain -1 dB Q 1\n", i * 100))
        .collect();
    let plan = plan_filters(&rp2350(), &text, &[0]);
    assert_eq!(plan.writes[0].filters.as_ref().unwrap()[9].freq, 1000.0);
    assert_eq!(
        plan.notes,
        ["The file has 14 filters; only the first 10 were used."]
    );
}

#[test]
fn wp_c13_filters_export_round_trips() {
    let mut s = rp2350();
    s.channel_names[0] = "FL".into();
    s.input_preamp_db[0] = -5.3;
    s.filters[0][0] = pk(105.0, 8.8, 0.71);
    s.filters[0][2] = FilterParams {
        bypass: true,
        ..pk(2856.0, -8.6, 3.58)
    };
    s.filters[8][1] = FilterParams {
        filter_type: 11,
        freq: 40.0,
        q: 0.5,
        gain: 28.0,
        qp: 0.71,
        bypass: false,
    };
    s.xover[8][0] = FilterParams {
        filter_type: 35,
        freq: 2000.0,
        ..flat()
    };
    s.output_enabled[0] = true;
    let text = export_filters(&s, 2, 1_700_000_000);
    assert!(text.starts_with(
        "# DSPi Console Filter Settings\n# Exported: 1700000000 (Unix time)\n# Format: 2\n\n[Input 0: FL]\nPreamp -5.3 dB\nFilter  1: ON  PK      Fc   105.0 Hz  Gain  +8.800 dB  Q  0.710\n"
    ));
    assert!(text.contains("[Output 0:  (Enabled)]\n"));
    assert!(text.contains("[Output 1:  (Disabled)]\n"));
    assert!(!text.contains("[Input 2"));
    let plan = plan_filters(&s, &text, &(0..17).collect::<Vec<_>>());
    assert_eq!(plan.writes.len(), 2 + 9);
    assert!(plan.notes.is_empty());
    for w in &plan.writes {
        let i = usize::from(w.wire);
        assert_eq!(w.filters.as_ref(), Some(&s.filters[i]), "wire {i}");
        if i >= 8 {
            assert_eq!(w.xover.as_ref().unwrap(), &s.xover[i], "wire {i}");
            assert_eq!(w.enable, Some(((i - 8) as u8, s.output_enabled[i - 8])));
        } else {
            assert_eq!(w.preamp, Some(s.input_preamp_db[i]));
        }
    }
}

#[test]
fn wp_c13_inputs_get_no_crossovers_or_linkwitz() {
    let text = "# DSPi Console Filter Settings\n[Input 0: A]\nFilter 1: ON LT Fc 40 Hz Q 0.5 Fp 28 Hz Qp 0.7\nFilter 2: ON LR4LP Fc 100 Hz\nXover 1: ON LR4LP Fc 100 Hz\n[Output 0: B]\nFilter 1: ON LT Fc 40 Hz Q 0.5 Fp 28 Hz Qp 0.7\nXover 1: ON PK Fc 100 Hz\n";
    let plan = plan_filters(&rp2350(), text, &[0, 8]);
    assert_eq!(plan.writes[0].filters.as_ref().unwrap()[0], flat());
    assert_eq!(plan.writes[0].filters.as_ref().unwrap()[1], flat());
    assert!(plan.writes[0].xover.is_none());
    assert_eq!(plan.writes[1].filters.as_ref().unwrap()[0].filter_type, 11);
    assert_eq!(plan.writes[1].xover.as_ref().unwrap()[0], flat());
}

#[test]
fn wp_c13_newer_and_long_files_are_noted() {
    let mut text = String::from("# DSPi Console Filter Settings\n# Format: 3\n[Input 0: A]\n");
    for i in 1..=12 {
        text.push_str(&format!("Filter {i}: ON PK Fc 100 Hz\n"));
    }
    let v = inspect_filters(&rp2350(), &text, 2);
    assert_eq!(v["newer"], true);
    assert_eq!(v["channels"][0]["bands"], 12);
    let plan = plan_filters(&rp2350(), &text, &[0]);
    assert_eq!(plan.notes.len(), 2);
    assert!(plan.notes[0].contains("format 3"));
    assert_eq!(
        plan.notes[1],
        "Only the first 10 bands of a channel were used."
    );
}

// ── Through DspiCore, without a device ──

#[test]
fn wp_c13_imports_need_a_writable_device() {
    let core = DspiCore::new();
    let r = json_of(&core.config_import(MAC_DOC.into(), 0));
    assert_eq!(r["ok"], false);
    assert_eq!(
        r["error"],
        "The device could not be written: device not connected."
    );
    let r = json_of(&core.filters_import(REW_FILTERS.into(), vec![0]));
    assert_eq!(r["ok"], false);
    // Even with nothing to write, a missing device is the error.
    let r = json_of(&core.filters_import(REW_FILTERS.into(), vec![]));
    assert_eq!(r["ok"], false);
    assert!(
        r["error"].as_str().unwrap().contains("not connected"),
        "{r}"
    );
    // A bad file is reported before the device is looked at.
    let r = json_of(&core.config_import("{}".into(), 0));
    assert!(r["error"].as_str().unwrap().contains("no channel data"));

    // A connection whose image was not read in this build's format is
    // read-only; the mirror is left alone.
    core.attach(
        Box::new(dspi_transport::FakeDevice::demo()),
        "DEMO".into(),
        0,
    );
    let before = core.lock().state.clone();
    let r = json_of(&core.config_import(MAC_DOC.into(), IMPORT_HARDWARE));
    assert_eq!(r["ok"], false);
    assert!(r["error"].as_str().unwrap().contains("read-only"), "{r}");
    assert_eq!(core.lock().state, before);
    let r = json_of(&core.filters_import(MACOS_FILTERS.into(), vec![0, 1]));
    assert!(r["error"].as_str().unwrap().contains("read-only"), "{r}");
    let r = json_of(&core.filters_import("x".repeat(MAX_FILE_BYTES + 1), vec![0]));
    assert!(r["error"].as_str().unwrap().contains("larger than"));
}

#[test]
fn wp_c13_exports_and_inspects_read_the_mirror() {
    let core = demo_core();
    let doc = core.config_export("Mine".into(), "2.0".into(), 1).unwrap();
    let v = parse_config(&doc).unwrap();
    assert_eq!(v["meta"]["appVersion"], "2.0");
    assert_eq!(v["meta"]["outputChannelCount"], 9);
    let text = core.filters_export(8).unwrap();
    assert_eq!(text.matches("[Input ").count(), 8);
    assert_eq!(text.matches("[Output ").count(), 9);
    let v = json_of(&core.filters_inspect(text, 8));
    assert_eq!(v["channels"].as_array().unwrap().len(), 17);
    let v = json_of(&core.filters_inspect("x".repeat(MAX_FILE_BYTES + 1), 8));
    assert_eq!(v["channels"].as_array().unwrap().len(), 0);
}

fn demo_core() -> std::sync::Arc<DspiCore> {
    let core = DspiCore::new();
    core.attach_probed(
        Box::new(dspi_transport::FakeDevice::demo()),
        "DEMO".into(),
        0,
    )
    .unwrap();
    core.fetch_all().unwrap();
    core
}

#[test]
fn wp_c13_config_import_writes_the_demo_device() {
    let core = demo_core();
    let doc = core.config_export("Demo".into(), "test".into(), 0).unwrap();
    // Edit the document, import it, and read the device back.
    let mut v = parse_config(&doc).unwrap();
    v["channels"][0]["eq"][0] = json!({"type": 1, "freqHz": 321.0, "q": 2.0, "gain": -4.5});
    v["channels"][1]["name"] = json!("Renamed");
    let r = json_of(&core.config_import(v.to_string(), 0));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["clean"], true, "{r}");
    assert_eq!(core.get_state().filters[0][0], pk(321.0, -4.5, 2.0));
    core.fetch_all().unwrap();
    let s = core.get_state();
    assert_eq!(s.filters[0][0], pk(321.0, -4.5, 2.0), "the device has it");
    assert_eq!(s.channel_names[1], "Renamed");
}

#[test]
fn wp_c13_filters_import_writes_the_demo_device() {
    let core = demo_core();
    let r = json_of(&core.filters_import(REW_FILTERS.into(), vec![1, 1]));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["lines"][0], "Applied filters to 1 channel.");
    assert_eq!(core.get_state().filters[1][0], pk(31.5, -4.2, 3.5));
}

#[test]
fn wp_c13_bom_files_are_read() {
    let s = rp2350();
    let bom = format!("\u{feff}{MACOS_FILTERS}");
    let v = inspect_filters(&s, &bom, 8);
    assert_eq!(v["kind"], "native", "not misread as REW");
    let plain = plan_filters(&s, MACOS_FILTERS, &[0, 8]);
    let with = plan_filters(&s, &bom, &[0, 8]);
    assert_eq!(with.writes, plain.writes);
    let doc = format!("\u{feff}{MAC_DOC}");
    assert!(parse_config(&doc).is_ok());
    assert_eq!(json_of(&config_inspect(doc))["ok"], true);
}

#[test]
fn wp_c13_bare_windows_input_headers_are_one_based() {
    assert_eq!(section_header("Input 3", 8), Some((Chan::Input(2), None)));
    assert_eq!(
        section_header("Input 3: X", 8),
        Some((Chan::Input(3), None))
    );
    assert_eq!(section_header("Output 2", 8), None);
    assert_eq!(
        section_header("Output 2: Y (Enabled)", 8),
        Some((Chan::Output(2), Some(true)))
    );
    let text =
        "# DSPi Console Filter Settings\n[Input 3]\nFilter 1: ON PK Fc 100 Hz Gain 1 dB Q 1\n";
    let plan = plan_filters(&rp2350(), text, &[2, 3]);
    assert_eq!(plan.writes.len(), 1);
    assert_eq!(plan.writes[0].wire, 2);
}

#[test]
fn wp_c13_file_integers_never_wrap() {
    let mut old = rp2350();
    old.dac_mute_supported = true;
    let v = parse_config(
        r#"{"schemaVersion": 1, "channels": [{"channelId": 0}],
            "loudness": {"outputMask": 70000}, "leveller": {"detectorMask": 300, "applyMask": -1},
            "io": {"outputPins": [999, 7, -3], "spdifRxPins": [256], "i2sRxPins": [1000],
                   "i2sBckPin": 300, "mckPin": -1, "spdifEnabledExt": 513, "adatPin": 1e12,
                   "dacHwMute": {"pin": 4096, "holdMs": 70000, "releaseMs": 20}}}"#,
    )
    .unwrap();
    let s = build_config(&old, &v, IMPORT_HARDWARE).state;
    assert_eq!(s.loudness_output_mask, old.loudness_output_mask);
    assert_eq!(
        (s.leveller_detector_mask, s.leveller_apply_mask),
        (old.leveller_detector_mask, old.leveller_apply_mask)
    );
    assert_eq!(
        s.output_pins[..3],
        [old.output_pins[0], 7, old.output_pins[2]]
    );
    assert_eq!(s.spdif_rx_pins[0], old.spdif_rx_pins[0]);
    assert_eq!(s.i2s_rx_pins[0], old.i2s_rx_pins[0]);
    assert_eq!(
        (s.i2s_bck_pin, s.mck_pin, s.adat_out_pin),
        (old.i2s_bck_pin, old.mck_pin, old.adat_out_pin)
    );
    assert_eq!(s.spdif_inputs_enabled, 0x01, "513 is out of range: kept");
    assert_eq!(
        (s.dac_mute_pin, s.dac_mute_hold_ms),
        (old.dac_mute_pin, old.dac_mute_hold_ms)
    );
    assert_eq!(s.dac_mute_release_ms, 20);
}

#[test]
fn wp_c13_missing_channels_are_capped_and_negative_indices_missing() {
    let channels: Vec<String> = (0..100)
        .map(|i| {
            format!(
                r#"{{"isOutput": true, "outputIndex": {}, "name": "o{i}"}}"#,
                50 + i
            )
        })
        .chain(std::iter::repeat_n(
            r#"{"inputIndex": -1, "name": "neg"}"#.to_string(),
            1000,
        ))
        .collect();
    let doc = format!(
        r#"{{"schemaVersion": 1, "channels": [{}]}}"#,
        channels.join(",")
    );
    let b = build_config(&rp2350(), &parse_config(&doc).unwrap(), 0);
    assert_eq!(b.channels, 0, "no channel 0 from a negative index");
    assert_eq!(b.missing.names.len(), Missing::SHOWN);
    assert_eq!(b.missing.more, 81, "80 more outputs and neg, once");
    let (lines, _) = config_report(&b);
    assert!(lines[1].ends_with(", o19 and 81 more."), "{}", lines[1]);
}

// ── Property tests (PLAN §1.9) ──

/// Any JSON value, with object keys from the document's vocabulary so the
/// lenient readers see wrong types in the right places.
fn any_json() -> impl Strategy<Value = Value> {
    const KEYS: &[&str] = &[
        "schemaVersion",
        "meta",
        "global",
        "channels",
        "matrix",
        "io",
        "crossfeed",
        "upmix",
        "subharm",
        "tube",
        "loudness",
        "leveller",
        "psybass",
        "pairMap",
        "channelId",
        "isOutput",
        "inputIndex",
        "outputIndex",
        "eq",
        "crossover",
        "type",
        "freqHz",
        "q",
        "gain",
        "qp",
        "name",
        "delayMs",
        "gainDb",
        "limiter",
        "input",
        "output",
        "enabled",
        "outputPins",
        "spdifRxPins",
        "inputPreampsDb",
        "inputPairLinked",
        "dacHwMute",
        "outputSlotTypes",
    ];
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|i| json!(i)),
        (-20i64..80).prop_map(|i| json!(i)),
        any::<f64>().prop_map(|f| json!(f)),
        ".{0,40}".prop_map(Value::String),
    ];
    leaf.prop_recursive(4, 64, 8, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..12).prop_map(Value::Array),
            proptest::collection::vec((proptest::sample::select(KEYS), inner), 0..10).prop_map(
                |kv| Value::Object(kv.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
            ),
        ]
    })
}

/// Lines in the filter grammar, valid or nearly so.
fn filter_line() -> impl Strategy<Value = String> {
    prop_oneof![
        "\\[(Input|Output) [0-9]{1,3}: [a-zA-Z ]{0,8}( \\((Enabled|Disabled)\\))?\\]",
        "\\[(Master|SPDIF [0-9] |Input |PDM|Out )[LR0-9]?\\]",
        "(Filter|Xover|Crossover) {1,2}[0-9]{1,2}: (ON|OFF) +[A-Za-z0-9]{0,6} +(Fc|Gain|Q|Qp|Fp|Slope) [-+0-9.,eE]{0,8}( Hz| dB)?( \\[Bypassed\\]| BYP)?",
        "Preamp:? [-+0-9.,]{0,8} dB",
        "# (Format|DSPi Console)[: ]*[0-9]{0,12}",
        ".{0,60}",
    ]
}

proptest! {
    #[test]
    fn proptest_wp_c13_config_parse_never_panics(text in ".{0,400}") {
        let _ = parse_config(&text);
        let v: Value = serde_json::from_str(&config_inspect(text)).unwrap();
        prop_assert!(v["ok"].is_boolean());
    }

    #[test]
    fn proptest_wp_c13_config_build_never_panics(
        extra in proptest::collection::vec((0usize..12, any_json()), 0..8),
        channels in proptest::collection::vec(any_json(), 1..6),
        options in 0u32..4,
        rp in any::<bool>(),
        n_in in 0u8..12,
        n_out in 0u8..12,
    ) {
        const BLOCKS: &[&str] = &[
            "global", "matrix", "io", "crossfeed", "upmix", "subharm", "tube", "loudness",
            "leveller", "psybass", "meta", "schemaVersion",
        ];
        let mut doc = Map::new();
        doc.insert("schemaVersion".into(), json!(1));
        doc.insert("channels".into(), Value::Array(channels));
        for (k, v) in extra {
            doc.insert(BLOCKS[k].into(), v);
        }
        let doc = Value::Object(doc);
        let mut s = if rp { rp2350() } else { default_state() };
        // The record may claim more channels than its vectors hold.
        s.num_input_channels = n_in;
        s.num_output_channels = n_out;
        let b = build_config(&s, &doc, options);
        let _ = config_report(&b);
        let _ = export_config(&b.state, "x", "y", 0xFF);
        let _ = config_inspect(doc.to_string());
    }

    #[test]
    fn proptest_wp_c13_filter_files_never_panic(
        lines in proptest::collection::vec(filter_line(), 0..40),
        native in any::<bool>(),
        chosen in proptest::collection::vec(any::<u8>(), 0..20),
        inputs in any::<u8>(),
        n_in in 0u8..12,
        n_out in 0u8..12,
    ) {
        let mut text = if native { "# DSPi Console Filter Settings\n".to_string() } else { String::new() };
        text.push_str(&lines.join("\n"));
        let mut s = rp2350();
        s.num_input_channels = n_in;
        s.num_output_channels = n_out;
        let v = inspect_filters(&s, &text, usize::from(inputs));
        prop_assert!(v["channels"].is_array());
        let plan = plan_filters(&s, &text, &chosen);
        for w in &plan.writes {
            if let Some(f) = &w.filters {
                prop_assert_eq!(f.len(), BANDS_PER_CHANNEL);
                prop_assert!(f.iter().all(|p| p.filter_type <= FILTER_PEQ_LAST));
            }
            prop_assert!(chosen.contains(&w.wire));
            prop_assert!(usize::from(w.wire) < usize::from(n_in) + usize::from(n_out));
            if let Some(x) = &w.xover {
                prop_assert!(x.iter().all(|p| p.filter_type == 0 || is_crossover(p.filter_type)));
            }
        }
        let _ = export_filters(&s, usize::from(inputs), u64::MAX);
    }

    #[test]
    fn proptest_wp_c13_filter_text_never_panics(text in "\\PC{0,300}", pdm in any::<usize>()) {
        let _ = parse_filters(&text, pdm);
        for line in text.lines() {
            let _ = parse_band(line);
            let _ = parse_preamp(line);
            let _ = windows_header(line, pdm);
            let _ = section_header(line, pdm);
            let _ = number(line);
            let _ = type_for_code(line);
        }
    }
}
