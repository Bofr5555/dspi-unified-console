//! CLEAN1 tests: configuration documents and filter files, on their own and
//! through the demo device.

use std::sync::Arc;

use dspi_transport::{Capture, FakeDevice, FakeOptions};
use proptest::prelude::*;

use super::*;
use crate::state::{apply_platform, decode_bulk, default_state};

const PLATFORM: &[u8] = include_bytes!("../../../../tests/fixtures/link/device/platform.bin");
const BULK: &[u8] = include_bytes!("../../../../tests/fixtures/link/device/bulk_v32.bin");
const QT: &str = include_str!("../../../../tests/fixtures/preset/qt-core-v32.dspipreset");
const MAC: &str = include_str!("../../../../tests/fixtures/preset/mac-swift-v32.dspipreset");
const DSPI_FILTERS: &str = include_str!("../../../../tests/fixtures/DSPiFilters.txt");
const MAC_FILTERS: &str = include_str!("../../../../tests/fixtures/MacOSFilters.txt");
const WINDOWS_FILTERS: &str = include_str!("../../../../tests/fixtures/WindowsFilters.txt");
const REW_FILTERS: &str = include_str!("../../../../tests/fixtures/REWFilters.txt");
const ALL: u32 = IMPORT_VOLUMES | IMPORT_HARDWARE;

/// The demo device's state (RP2350, V32), without a connection.
fn capture() -> DspState {
    let mut s = default_state();
    apply_platform(PLATFORM, &mut s).unwrap();
    decode_bulk(BULK, &mut s).unwrap();
    s
}

/// A connected demo device, read.
fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    core.fetch_all().unwrap();
    core
}

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

/// A document's JSON with its time stamp blanked.
fn stamped_out(text: &str) -> Value {
    let mut v = json_of(text);
    v["meta"]["savedUtc"] = json!("");
    v
}

fn band(t: u8, freq: f32, q: f32, gain: f32) -> FilterParams {
    FilterParams {
        filter_type: t,
        bypass: false,
        freq,
        q,
        gain,
        qp: DEFAULT_Q,
    }
}

fn doc(rest: &str) -> Value {
    parse_config(&format!(r#"{{"schemaVersion": 1, {rest}}}"#)).unwrap()
}

// ── The document ──

#[test]
fn clean1_channel_ids_follow_the_shared_table() {
    let v = json_of(&export_config(&capture(), "x", "1", 0, None));
    let ch = v["channels"].as_array().unwrap();
    let ids: Vec<i64> = ch
        .iter()
        .map(|c| c["channelId"].as_i64().unwrap())
        .collect();
    assert_eq!(
        ids,
        [0, 1, 11, 12, 13, 14, 15, 16, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    );
    for (k, c) in ch.iter().enumerate() {
        assert_eq!(c["eqChannel"], json!(k));
        if k < 8 {
            assert_eq!(c["inputIndex"], json!(k));
            assert_eq!(c["isOutput"], false);
            assert!(c.get("outputIndex").is_none());
        } else {
            assert_eq!(c["outputIndex"], json!(k - 8));
            assert_eq!(c["isOutput"], true);
        }
    }
    // An id read back places each entry where it came from.
    for (k, id) in ids.iter().enumerate() {
        let want = if k < 8 {
            ChannelRef::Input(k as u8)
        } else {
            ChannelRef::Output(k as u8 - 8)
        };
        assert_eq!(
            preset_file::channel_id::resolve(*id as i32),
            Some(want),
            "{id}"
        );
    }
}

#[test]
fn clean1_the_mac_document_lands_where_it_says() {
    let b = build_config(&capture(), &parse_config(MAC).unwrap(), ALL);
    let s = &b.state;
    assert_eq!(
        (s.filters[0][0].filter_type, s.filters[0][0].freq),
        (1, 100.0)
    );
    assert!(s.filters[0][3].bypass);
    assert_eq!(s.filters[8][0].freq, 108.0, "output 0 is wire 8");
    assert_eq!(s.xover[16][0].filter_type, 34, "the PDM crossover");
    assert_eq!(s.channel_delays[1], 0.5);
    assert_eq!(s.output_delay_ms[1], 1.5);
    assert_eq!(s.output_gain_db[8], -4.0);
    assert_eq!(s.input_preamp_db[0], -3.0);
    assert_eq!(s.preamp_db, -3.0);
    assert_eq!(s.crossfeed_pair_map, vec![2, 19, 0, 0]);
    assert_eq!((s.loudness_enabled, s.loudness_output_mask), (true, 3));
    assert_eq!(s.psybass_output_mask, 256);
    assert_eq!((s.master_volume_db, s.user_volume_db), (-18.0, -12.0));
    assert!(s.limiter_enabled[0]);
    assert_eq!(s.limiter_threshold_db[0], -6.0);
    assert_eq!(b.links, Some(1));
    assert_eq!(
        (b.channels, b.bands, b.xover, b.crosspoints),
        (17, 170, 36, 72)
    );
    let (lines, clean) = config_report(&b);
    assert!(clean, "{lines:?}");
    assert_eq!(
        lines[0],
        "Applied 17 channels, 170 EQ bands, 36 crossover bands, 72 crosspoints."
    );
    assert_eq!(lines.last().unwrap(), NOT_STORED);
}

#[test]
fn clean1_empty_newer_and_oversized_files_are_refused() {
    for (text, says) in [
        ("", "empty"),
        ("  \n", "empty"),
        ("{", "not a DSPi configuration"),
        ("[1, 2]", "not a DSPi configuration"),
        (r#"{"channels": [{}]}"#, "schema version"),
        (
            r#"{"schemaVersion": 0, "channels": [{}]}"#,
            "schema version",
        ),
        (
            r#"{"schemaVersion": "1", "channels": [{}]}"#,
            "schema version",
        ),
        (r#"{"schemaVersion": 2, "channels": [{}]}"#, "newer version"),
        (r#"{"schemaVersion": 1}"#, "no channel"),
        (r#"{"schemaVersion": 1, "channels": []}"#, "no channel"),
    ] {
        let e = parse_config(text).unwrap_err();
        assert!(e.contains(says), "{text}: {e}");
        let r = json_of(&config_inspect(text.into()));
        assert_eq!(r["ok"], false, "{text}");
        assert_eq!(r["error"], json!(e));
    }
    let big = format!(
        r#"{{"schemaVersion": 1, "channels": [{{}}], "pad": "{}"}}"#,
        " ".repeat(MAX_FILE_BYTES)
    );
    assert_eq!(parse_config(&big).unwrap_err(), TOO_LARGE);
    let r = json_of(&demo().config_import(big, 0));
    assert_eq!(r["ok"], false);
}

#[test]
fn clean1_inspect_describes_a_document() {
    let r = json_of(&config_inspect(MAC.into()));
    assert_eq!(r["ok"], true);
    assert_eq!(r["error"], "");
    assert_eq!(r["name"], "Mac reference");
    assert_eq!(r["platform"], "RP2350");
    assert_eq!(r["firmware"], "1.1.6 beta 5");
    assert_eq!(r["savedUtc"], "2026-11-06T10:00:00Z");
    // No meta at all is still a document.
    let r = json_of(&config_inspect(
        r#"{"schemaVersion": 1, "channels": [{}]}"#.into(),
    ));
    assert_eq!(
        (r["ok"].clone(), r["name"].clone()),
        (json!(true), json!(""))
    );
}

#[test]
fn clean1_band_keys_a_document_lacks_take_the_flat_defaults() {
    let v = doc(r#""schemaVersion": 1.0, "channels": [
            {"isOutput": true, "outputIndex": 0, "eq": [{"type": 1.0, "gain": 2}, {}, {"type": 2}]}]"#);
    let s = build_config(&capture(), &v, 0).state;
    let o = 8;
    assert_eq!(s.filters[o][0], {
        let mut b = flat();
        b.filter_type = 1;
        b.gain = 2.0;
        b
    });
    assert_eq!(s.filters[o][1], flat());
    assert_eq!(s.filters[o][2].filter_type, 2);
    assert_eq!(s.filters[o][2].freq, 1000.0);
    assert_eq!(s.filters[o][3], flat());
}

#[test]
fn clean1_the_rp2040_has_no_upmixer() {
    let s = default_state();
    assert_eq!(s.platform_id, 0);
    let mut s32 = s.clone();
    s32.format_version = 32;
    let v = json_of(&export_config(&s32, "x", "1", 0, None));
    assert!(v.get("upmix").is_none());
    assert!(v.get("tube").is_some() && v.get("psybass").is_some());

    let d = doc(r#""channels": [{"channelId": 0}], "upmix": {"strengthPct": 50}"#);
    let b = build_config(&s, &d, 0);
    assert_eq!(b.state.upmix, s.upmix);
    let (lines, clean) = config_report(&b);
    assert!(!clean);
    assert!(
        lines.contains(&"Skipped: Stereo upmixer (not supported by this device)".to_string()),
        "{lines:?}"
    );
    // The RP2350 takes it.
    let b = build_config(&capture(), &d, 0);
    assert_eq!(b.state.upmix[usize::from(UPMIX_PARAM_STRENGTH)], 50.0);
}

#[test]
fn clean1_an_export_of_the_capture_reads_back_as_the_capture() {
    let base = capture();
    let text = export_config(&base, "x", "1", 0b1010, None);
    let v = parse_config(&text).unwrap();
    // Onto a device that holds something else entirely.
    let mut other = base.clone();
    for bank in other
        .filters
        .iter_mut()
        .chain(other.xover.iter_mut().skip(8))
    {
        bank.iter_mut().for_each(|b| *b = band(1, 77.0, 2.0, 3.0));
    }
    other.output_gain_db.iter_mut().for_each(|g| *g = -9.0);
    other.loudness_ref_spl = 70.0;
    other.tube[0] = 1.0;
    other.channel_names[3] = "elsewhere".into();
    let b = build_config(&other, &v, ALL);
    assert_eq!(b.state.filters, base.filters);
    assert_eq!(b.state.xover, base.xover);
    assert_eq!(b.state.output_gain_db, base.output_gain_db);
    assert_eq!(b.state.loudness_ref_spl, base.loudness_ref_spl);
    assert_eq!(b.state.tube, base.tube);
    assert_eq!(b.state.channel_names, base.channel_names);
    assert_eq!(b.links, Some(0b1010));
    assert!(config_report(&b).1);
}

#[test]
fn clean1_an_edited_device_moves_to_another_whole() {
    let a = demo();
    a.set_filter(8, 2, band(1, 250.0, 1.5, -4.5)).unwrap();
    a.set_crossover(9, 0, band(35, 2500.0, DEFAULT_Q, 0.0))
        .unwrap();
    a.set_output_gain(1, -6.0).unwrap();
    a.set_input_preamp(1, -2.5).unwrap();
    a.set_delay(0, 3.0).unwrap();
    let text = a.config_export("edited".into(), "1".into(), 1).unwrap();

    let b = demo();
    let r = json_of(&b.config_import(text.clone(), ALL));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["links"], 1);
    let again = b.config_export("edited".into(), "1".into(), 1).unwrap();
    assert_eq!(stamped_out(&again), stamped_out(&text));
    let s = b.get_state();
    assert_eq!(s.filters[8][2].freq, 250.0);
    assert_eq!(s.xover[9][0].filter_type, 35);
    assert_eq!(s.output_gain_db[1], -6.0);
}

#[test]
fn clean1_the_pair_map_is_taken_as_the_console_takes_it() {
    let map = |m: &str| {
        let d = doc(&format!(
            r#""channels": [{{"channelId": 0}}], "crossfeed": {{"pairMap": {m}}}"#
        ));
        let b = build_config(&capture(), &d, 0);
        (b.pair_map, b.state.crossfeed_pair_map, b.unreadable)
    };
    let k340 = [0x02, 0x13, 0, 0];
    assert_eq!(map("[2, 19, 0, 0]"), (Some(k340), k340.to_vec(), false));
    // An explicit default byte is written as 0, as the firmware reads it back.
    assert_eq!(map("[16, 50, 0, 0]").0, Some([0, 0, 0, 0]));
    // A short map leaves the rest at the default.
    assert_eq!(map("[2, 19]").0, Some(k340));
    // Invalid, or wider than the firmware's: the default pairing.
    assert_eq!(map("[2, 2, 0, 0]").0, Some([0; 4]));
    assert_eq!(map("[0, 0, 0, 0, 0]").0, Some([0; 4]));
    // Not a list: no map, and the report says something could not be read.
    let (m, now, bad) = map(r#""K340""#);
    assert_eq!((m, bad), (None, true));
    assert_eq!(now, capture().crossfeed_pair_map);
    // On RP2040 (two pairs) a pair past output 3 is invalid.
    let mut small = default_state();
    small.platform_id = 0;
    let d = doc(r#""channels": [{"channelId": 0}], "crossfeed": {"pairMap": [84, 0, 0, 0]}"#);
    assert_eq!(build_config(&small, &d, 0).pair_map, Some([0; 4]));
}

#[test]
fn clean1_the_demo_export_is_the_qt_fixture() {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    let text = core
        .config_export("Qt reference".into(), "1.0".into(), 0b0101)
        .unwrap();
    assert_eq!(stamped_out(&text), stamped_out(QT));
    // Keys sorted, two-space indent, no trailing newline: what serde writes.
    assert!(text.starts_with("{\n  \"channels\": [\n    {\n      \"channelId\": 0,"));
    assert!(text.ends_with("\n  }\n}"));
    assert!(text.contains("\"q\": 0.7070000171661377"));
    assert!(text.contains("\"dacHwMute\": null"));
}

#[test]
fn clean1_utc_strings() {
    for (secs, want) in [
        (0, "1970-01-01T00:00:00Z"),
        (951_782_400, "2000-02-29T00:00:00Z"),
        (1_709_210_096, "2024-02-29T12:34:56Z"),
        (1_794_436_360, "2026-11-11T22:32:40Z"),
        (4_102_444_800, "2100-01-01T00:00:00Z"),
    ] {
        assert_eq!(utc_text(secs), want);
    }
    let v = json_of(&export_config(&capture(), "x", "1", 0, None));
    let saved = v["meta"]["savedUtc"].as_str().unwrap();
    assert_eq!(saved.len(), 20, "{saved}");
    assert!(saved.ends_with('Z'));
}

// ── Filter files ──

#[test]
fn clean1_every_type_code_reads_back() {
    for t in FilterType::all_defined() {
        match type_code(t) {
            None => assert_eq!(t, FilterType::Flat),
            Some(code) => {
                assert_eq!(code_type(&code), Some(t), "{code}");
                assert_eq!(code_type(&code.to_ascii_lowercase()), Some(t), "{code}");
            }
        }
    }
    assert_eq!(type_code(FilterType::from_raw(34)).unwrap(), "LR4LP");
    assert_eq!(type_code(FilterType::from_raw(63)).unwrap(), "BES8HP");
    assert_eq!(type_code(FilterType::from_raw(14)), None);
    assert_eq!(code_type("XX"), None);
}

#[test]
fn clean1_band_lines_read_back() {
    let mut lt = band(11, 40.0, 0.5, 28.0);
    lt.qp = 0.71;
    let mut bypassed = band(2, 120.5, 0.5, -2.5);
    bypassed.bypass = true;
    for (bank, b, line) in [
        (
            Bank::Peq,
            band(1, 100.0, 1.25, 3.0),
            "Filter  1: ON  PK      Fc   100.0 Hz  Gain  +3.000 dB  Q  1.250\n",
        ),
        (
            Bank::Peq,
            bypassed,
            "Filter  1: ON  LS      Fc   120.5 Hz  Gain  -2.500 dB  Q  0.500  [Bypassed]\n",
        ),
        (
            Bank::Peq,
            lt,
            "Filter  1: ON  LT      Fc    40.0 Hz  Q  0.500  Fp    28.0 Hz  Qp  0.710\n",
        ),
        (
            Bank::Peq,
            band(13, 80.0, DEFAULT_Q, 0.0),
            "Filter  1: ON  HP1     Fc    80.0 Hz\n",
        ),
        (
            Bank::Xover,
            band(34, 2000.0, DEFAULT_Q, 0.0),
            "Xover   1: ON  LR4LP   Fc  2000.0 Hz\n",
        ),
        (Bank::Xover, flat(), "Xover   1: OFF\n"),
    ] {
        assert_eq!(format_band(bank, 1, &b), line);
        let (got_bank, got) = band_line(line).unwrap();
        assert_eq!(got_bank, bank);
        if b.filter_type == 0 {
            assert_eq!(got, None);
        } else {
            assert_eq!(got, Some(b), "{line}");
        }
    }
    assert_eq!(format_band(Bank::Peq, 10, &flat()), "Filter 10: OFF\n");
}

#[test]
fn clean1_other_tools_spellings_are_read() {
    let read = |l: &str| band_line(l).unwrap().1.unwrap();
    let b = read("Filter  1: ON  PEQ  Fc 1,234.5 Hz  Gain 8,8 dB  Q 0,70");
    assert_eq!((b.filter_type, b.freq, b.gain, b.q), (1, 1234.5, 8.8, 0.7));
    assert_eq!(
        read("Filter 2: ON LSC Fc 105 Hz Gain 3 dB Q 0.707").filter_type,
        2
    );
    assert_eq!(read("Filter 2: ON HSC Fc 105 Hz Gain 3 dB").filter_type, 3);
    let n = read("Filter 3: ON NO Fc 60,0 Hz Q 8,00 BYP");
    assert_eq!((n.filter_type, n.q, n.bypass), (6, 8.0, true));
    let x = read("Crossover 1: ON LR HP Fc 2000,0 Hz Slope 24 dB/oct");
    assert_eq!((x.filter_type, x.freq), (35, 2000.0));
    assert_eq!(
        read("Crossover 2: ON BW LP Fc 4000 Hz Slope 12 dB/oct").filter_type,
        42
    );
    assert_eq!(
        read("Crossover 1: ON Bessel LP Fc 120 Hz Slope 24 dB/oct").filter_type,
        58
    );
    // An order the family does not have, REW's None, and a lowercase line.
    assert_eq!(
        band_line("Crossover 1: ON LR HP Fc 1 Hz Slope 18 dB/oct")
            .unwrap()
            .1,
        None
    );
    assert_eq!(band_line("Filter 5: ON  None").unwrap().1, None);
    assert_eq!(read("filter 1: on pk fc 100hz gain -1 db q 2").gain, -1.0);
    // Not band lines.
    for l in [
        "Filters: 5",
        "Filter Settings file",
        "Filter x: ON PK",
        "Xover: ON PK",
    ] {
        assert!(band_line(l).is_none(), "{l}");
    }
    assert_eq!(preamp_line("Preamp: -6.5 dB"), Some(-6.5));
    assert_eq!(preamp_line("Preamp -6,5 dB"), Some(-6.5));
    assert_eq!(preamp_line("preamp"), None);
    assert_eq!(number("1.234,5"), Some(1234.5));
    assert_eq!(number("100Hz"), Some(100.0));
    assert_eq!(number("inf"), None);
    assert_eq!(number("dB"), None);
}

#[test]
fn clean1_off_lines_keep_the_slots_of_the_bands_after_them() {
    let s = capture();
    let text = "# DSPi Console Filter Settings\n# Format: 2\n\n[Output 2: X]\n\
        Filter  1: OFF\nFilter  2: ON  PK      Fc   300.0 Hz  Gain  +1.000 dB  Q  1.000\n\
        Filter  3: ON  XYZ     Fc   300.0 Hz\nFilter  4: ON  PK      Fc   400.0 Hz  Gain  +1.000 dB  Q  1.000\n";
    let plan = plan_filters(&s, text, &[10]);
    let f = plan.writes[0].filters.as_ref().unwrap();
    assert_eq!(f[0], flat());
    assert_eq!(f[1].freq, 300.0);
    assert_eq!(f[2], flat(), "an unknown type keeps its slot, flat");
    assert_eq!(f[3].freq, 400.0);
    assert_eq!(plan.writes[0].enable, None, "no state in the header");
}

#[test]
fn clean1_the_legacy_console_file() {
    let s = capture();
    let r = json_of(&inspect_filters(&s, DSPI_FILTERS, 2).to_string());
    assert_eq!(r["kind"], "native");
    assert_eq!(r["newer"], false);
    let wires: Vec<u64> = r["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["wire"].as_u64().unwrap())
        .collect();
    // [Master L], [Master R], [Out L], [Out R], [Sub] (the PDM output).
    assert_eq!(wires, [0, 1, 8, 9, 16]);
    assert_eq!(r["channels"][0]["bands"], 10);
    assert_eq!(r["channels"][0]["checked"], true);
    let plan = plan_filters(&s, DSPI_FILTERS, &[0, 8]);
    assert_eq!(plan.writes[0].filters.as_ref().unwrap()[0], {
        let mut b = flat();
        b.filter_type = 2;
        b.freq = 105.0;
        b.gain = 8.8;
        b
    });
    assert_eq!(plan.writes[0].preamp, None);
    assert_eq!(plan.writes[1].filters.as_ref().unwrap()[0].filter_type, 5);
    assert!(plan.notes.iter().any(|n| n == NO_XOVER_BANK));
}

#[test]
fn clean1_the_macos_file() {
    let s = capture();
    let plan = plan_filters(&s, MAC_FILTERS, &[0, 1, 8, 16]);
    assert_eq!(plan.writes.len(), 4);
    let w = &plan.writes[0];
    assert_eq!(w.preamp, Some(-5.3));
    let f = w.filters.as_ref().unwrap();
    assert!(f[2].bypass);
    assert_eq!(f[3], flat());
    let out0 = &plan.writes[2];
    assert_eq!(out0.enable, Some((0, true)));
    let f = out0.filters.as_ref().unwrap();
    assert_eq!(f[0].filter_type, 13);
    assert_eq!((f[1].filter_type, f[1].gain, f[1].qp), (11, 28.0, 0.71));
    let x = out0.xover.as_ref().unwrap();
    assert_eq!((x[0].filter_type, x[1].filter_type, x[2]), (35, 42, flat()));
    let pdm = &plan.writes[3];
    assert_eq!(pdm.xover.as_ref().unwrap()[0].filter_type, 58);
}

#[test]
fn clean1_the_windows_file() {
    let s = capture();
    let plan = plan_filters(&s, WINDOWS_FILTERS, &[0, 1, 8, 9, 16]);
    let w = &plan.writes[0];
    assert_eq!(w.preamp, Some(-6.5));
    let f = w.filters.as_ref().unwrap();
    assert_eq!((f[0].freq, f[0].gain, f[0].q), (105.0, 8.8, 0.7));
    assert_eq!(f[1].freq, 1234.5);
    assert_eq!((f[2].filter_type, f[2].bypass), (6, true));
    assert_eq!(f[3], flat());
    let spdif1l = plan.writes.iter().find(|w| w.wire == 8).unwrap();
    let x = spdif1l.xover.as_ref().unwrap();
    assert_eq!((x[0].filter_type, x[1].filter_type), (35, 42));
    let pdm = plan.writes.iter().find(|w| w.wire == 16).unwrap();
    assert_eq!(pdm.xover.as_ref().unwrap()[0].filter_type, 58);
    // Its output headers carry no state: enables are left alone.
    assert!(plan.writes.iter().all(|w| w.enable.is_none()));
}

#[test]
fn clean1_the_rew_file() {
    let s = capture();
    let r = inspect_filters(&s, REW_FILTERS, 2);
    assert_eq!(r["kind"], "rew");
    assert_eq!(r["filters"], 4);
    assert_eq!(r["preamp"], json!(-6.5));
    let ch = r["channels"].as_array().unwrap();
    assert_eq!(ch.len(), 2 + 9);
    assert_eq!(
        (ch[0]["checked"].clone(), ch[2]["checked"].clone()),
        (json!(true), json!(false))
    );
    assert_eq!(ch[2]["wire"], 8);
    assert_eq!(ch[0]["bands"], 4);

    let plan = plan_filters(&s, REW_FILTERS, &[0, 8]);
    let f = plan.writes[0].filters.as_ref().unwrap();
    assert_eq!((f[0].freq, f[0].gain, f[0].q), (31.5, -4.2, 3.5));
    assert_eq!((f[2].filter_type, f[3].filter_type), (2, 3));
    assert_eq!(f[4], flat(), "None lines are dropped, the rest flat");
    assert_eq!(plan.writes[0].preamp, Some(-6.5));
    assert_eq!(plan.writes[1].preamp, None);
    assert!(plan.notes.iter().any(|n| n == PREAMP_ON_OUTPUT));
}

#[test]
fn clean1_a_rew_list_longer_than_a_bank() {
    let s = capture();
    let text: String = (1..=12)
        .map(|k| format!("Filter {k}: ON PK Fc {} Hz Gain -1 dB Q 1\n", k * 100))
        .collect();
    assert_eq!(inspect_filters(&s, &text, 1)["channels"][0]["bands"], 10);
    let plan = plan_filters(&s, &text, &[0]);
    let f = plan.writes[0].filters.as_ref().unwrap();
    assert_eq!(f.len(), BANDS_PER_CHANNEL);
    assert_eq!(f[9].freq, 1000.0);
    assert!(
        plan.notes
            .iter()
            .any(|n| n.starts_with("2 filter lines past the 10")),
        "{:?}",
        plan.notes
    );
}

#[test]
fn clean1_a_filter_export_reads_back() {
    let mut s = capture();
    s.filters[1][3] = band(1, 1234.5, 2.5, -3.25);
    s.filters[9][0] = {
        let mut b = band(11, 30.0, 0.6, 20.0);
        b.qp = 0.5;
        b
    };
    s.xover[9][1] = band(47, 3000.0, DEFAULT_Q, 0.0);
    s.input_preamp_db[1] = -4.5;
    s.output_enabled[1] = false;
    let text = filter_text(&s, 2, 1_794_436_360);
    assert!(text.starts_with(
        "# DSPi Console Filter Settings\n# Exported: 1794436360 (Unix time)\n# Format: 2\n\n[Input 0: "
    ));
    assert!(!text.contains("[Input 2:"));
    assert!(text.contains("[Output 1: "));
    assert!(text.contains(" (Disabled)]\n"));
    let wires: Vec<u8> = (0..17).collect();
    let plan = plan_filters(&s, &text, &wires);
    assert_eq!(plan.writes.len(), 2 + 9, "inputs 2..7 were not written");
    assert!(plan.notes.is_empty(), "{:?}", plan.notes);
    for w in &plan.writes {
        let c = usize::from(w.wire);
        assert_eq!(w.filters.as_ref().unwrap(), &s.filters[c]);
        if c < 8 {
            assert_eq!(w.preamp, Some(s.input_preamp_db[c]));
        } else {
            assert_eq!(w.xover.as_ref().unwrap(), &s.xover[c]);
            assert_eq!(w.enable, Some((w.wire - 8, s.output_enabled[c - 8])));
        }
    }
}

#[test]
fn clean1_inputs_take_no_crossover_and_no_linkwitz() {
    let s = capture();
    let text = "# DSPi Console Filter Settings\n# Format: 2\n\n[Input 0: A]\n\
        Filter  1: ON  LT      Fc    40.0 Hz  Q  0.500  Fp    28.0 Hz  Qp  0.710\n\
        Filter  2: ON  LR4LP   Fc   100.0 Hz\n\
        Xover   1: ON  LR4LP   Fc  2000.0 Hz\n\
        [Output 0: B]\nXover   1: ON  PK      Fc   100.0 Hz  Gain  +1.000 dB  Q  1.000\n";
    let plan = plan_filters(&s, text, &[0, 8]);
    let input = &plan.writes[0];
    assert_eq!(input.xover, None);
    let f = input.filters.as_ref().unwrap();
    assert_eq!((f[0], f[1]), (flat(), flat()));
    assert_eq!(plan.writes[1].xover.as_ref().unwrap()[0], flat());
    for n in [
        "Linkwitz",
        "Inputs have no crossover",
        "Crossover types",
        "EQ types",
    ] {
        assert!(
            plan.notes.iter().any(|x| x.contains(n)),
            "{n}: {:?}",
            plan.notes
        );
    }
    // An output keeps its Linkwitz Transform.
    let plan = plan_filters(&s, &text.replace("[Input 0: A]", "[Output 1: A]"), &[9]);
    assert_eq!(plan.writes[0].filters.as_ref().unwrap()[0].filter_type, 11);
}

#[test]
fn clean1_newer_and_oversized_files_are_noted() {
    let s = capture();
    let text = "# DSPi Console Filter Settings\n# Format: 3\n\n[Input 0: A]\nPreamp -1.0 dB\n";
    assert_eq!(inspect_filters(&s, text, 2)["newer"], true);
    let plan = plan_filters(&s, text, &[0]);
    assert!(plan.notes.iter().any(|n| n.contains("newer version")));
    let big = "#".repeat(MAX_FILE_BYTES + 1);
    let r = inspect_filters(&s, &big, 2);
    assert_eq!(
        (r["kind"].clone(), r["error"].clone()),
        (json!("error"), json!(TOO_LARGE))
    );
    let r = json_of(&demo().filters_import(big, vec![0]));
    assert_eq!(
        (r["ok"].clone(), r["error"].clone()),
        (json!(false), json!(TOO_LARGE))
    );
}

#[test]
fn clean1_a_byte_order_mark_is_read_past() {
    let s = capture();
    let bom = format!("\u{feff}{MAC_FILTERS}");
    assert_eq!(inspect_filters(&s, &bom, 2)["kind"], "native");
    assert_eq!(plan_filters(&s, &bom, &[0]).writes[0].preamp, Some(-5.3));
    let doc = format!("\u{feff}{MAC}");
    assert!(parse_config(&doc).is_ok());
    assert_eq!(json_of(&config_inspect(doc))["ok"], true);
}

#[test]
fn clean1_bare_windows_input_headers_count_from_one() {
    let s = capture();
    let place = |h: &str| header_place(h, &s).map(|(r, _)| r);
    assert_eq!(place("Input 3"), Some(ChannelRef::Input(2)));
    assert_eq!(place("Input 8"), Some(ChannelRef::Input(7)));
    assert_eq!(place("Input 2"), None, "Master R is the Windows name");
    assert_eq!(place("Input 9"), None);
    assert_eq!(
        place("Input 3: USB 4"),
        Some(ChannelRef::Input(3)),
        "ours, from 0"
    );
    assert_eq!(place("SPDIF 2 R"), Some(ChannelRef::Output(3)));
    assert_eq!(place("PDM"), Some(ChannelRef::Output(8)));
    assert_eq!(place("Something Else"), None);
    // On an RP2040 (two inputs) input 3 is not there.
    assert_eq!(header_place("Input 3", &default_state()), None);
    assert_eq!(
        header_place("Sub", &default_state()).map(|(r, _)| r),
        Some(ChannelRef::Output(default_state().num_output_channels - 1))
    );
}

#[test]
fn clean1_numbers_from_a_file_never_wrap() {
    let s = capture();
    let d = doc(
        r#""channels": [{"isOutput": true, "outputIndex": 256, "name": "Far"},
                        {"inputIndex": 4294967296, "name": "Farther"}],
           "io": {"mckPin": 300, "outputPins": [-5, 1e30]},
           "crossfeed": {"preset": 99999999999999999999},
           "loudness": {"outputMask": 70000}"#,
    );
    let b = build_config(&s, &d, IMPORT_HARDWARE);
    assert_eq!(b.state.mck_pin, 255);
    assert_eq!(&b.state.output_pins[..2], &[0, 255]);
    assert_eq!(b.state.crossfeed_preset, 255);
    assert_eq!(b.state.loudness_output_mask, 65_535);
    assert_eq!(b.missing.names, ["Far", "Farther"]);
    assert_eq!(b.channels, 0);
    // Headers and stamps too large for their field are not read at all.
    let text = "# DSPi Console Filter Settings\n# Format: 99999999999\n\n[Input 256: A]\nPreamp -1.0 dB\n\
        [Output 264: B]\nFilter  1: OFF\n";
    let file = read_native(&s, text);
    assert_eq!(file.format, 1);
    assert!(file.sections.is_empty());
}

#[test]
fn clean1_missing_channels_are_capped_and_negative_indices_missing() {
    let channels: Vec<String> = (0..12)
        .map(|k| {
            format!(
                r#"{{"isOutput": true, "outputIndex": {}, "name": "Out {k}"}}"#,
                20 + k
            )
        })
        .chain([
            r#"{"inputIndex": -1, "name": "Negative"}"#.to_string(),
            r#"{"channelId": -3}"#.to_string(),
            r#"{"channelId": 0}"#.to_string(),
        ])
        .collect();
    let d = doc(&format!(r#""channels": [{}]"#, channels.join(",")));
    let b = build_config(&capture(), &d, 0);
    assert_eq!(b.missing.names.len(), 14);
    assert!(b.missing.names.contains(&"Negative".to_string()));
    assert!(b.missing.names.contains(&"channel -3".to_string()));
    assert_eq!(b.channels, 1);
    let (lines, clean) = config_report(&b);
    assert!(!clean);
    assert_eq!(
        lines[1],
        "Not present on this device: Out 0, Out 1, Out 2, Out 3, Out 4, Out 5, Out 6, Out 7, \
         and 6 more"
    );
}

// ── Through the device ──

#[test]
fn clean1_imports_need_a_writable_device() {
    let none = DspiCore::new();
    let r = json_of(&none.config_import(MAC.into(), 0));
    assert_eq!(
        (r["ok"].clone(), r["error"].clone()),
        (json!(false), json!("No device connected."))
    );
    let r = json_of(&none.filters_import(MAC_FILTERS.into(), vec![0]));
    assert_eq!(r["ok"], false);

    // A V31 device is shown read-only.
    let size = wire_compat::bulk_size(31).unwrap();
    let mut image = BULK.to_vec();
    image.truncate(size);
    image[0] = 31;
    image[6..8].copy_from_slice(&(size as u16).to_le_bytes());
    let core = DspiCore::new();
    let cap = Capture {
        platform: Capture::demo().platform,
        bulk: image,
    };
    core.attach(
        Box::new(FakeDevice::new(cap, FakeOptions::default())),
        "OLD".into(),
        0,
    );
    core.fetch_all().unwrap();
    let before = core.get_state();
    for r in [
        core.config_import(MAC.into(), ALL),
        core.filters_import(MAC_FILTERS.into(), vec![0]),
    ] {
        let r = json_of(&r);
        assert_eq!(r["ok"], false, "{r}");
        assert!(r["error"].as_str().unwrap().contains("read-only"), "{r}");
    }
    assert_eq!(core.get_state(), before);
    // A file that is not a document is refused before the device is asked.
    let r = json_of(&demo().config_import("nonsense".into(), 0));
    assert!(
        r["error"]
            .as_str()
            .unwrap()
            .contains("not a DSPi configuration")
    );
}

#[test]
fn clean1_exports_and_inspects_read_the_mirror() {
    let core = demo();
    core.set_output_gain(2, -7.5).unwrap();
    core.set_channel_name(8, "Woofer".into()).unwrap();
    let v = json_of(&core.config_export("n".into(), "2.0".into(), 0).unwrap());
    assert_eq!(v["channels"][10]["gainDb"], json!(-7.5));
    assert_eq!(v["channels"][8]["name"], "Woofer");
    assert_eq!(v["meta"]["name"], "n");
    assert_eq!(v["meta"]["appVersion"], "2.0");
    let text = core.filters_export(1).unwrap();
    assert!(text.contains("[Output 0: Woofer (Enabled)]"), "{text}");
    let r = json_of(&core.filters_inspect(MAC_FILTERS.into(), 2));
    assert_eq!(r["channels"][2]["label"], "Woofer");
    assert_eq!(r["channels"][2]["wire"], 8);
}

#[test]
fn clean1_a_document_import_writes_the_demo_device() {
    let core = demo();
    let r = json_of(&core.config_import(MAC.into(), ALL));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["clean"], true, "{r}");
    assert_eq!(r["links"], 1);
    assert_eq!(r["hardwareWritten"], false, "outputs are kept with presets");
    assert_eq!(r["leftAlone"], json!([]));
    assert_eq!(r["lines"].as_array().unwrap().last().unwrap(), NOT_STORED);
    // What the device holds now, read back.
    core.refresh_params().unwrap();
    let s = core.get_state();
    assert_eq!(s.filters[0][0].freq, 100.0);
    assert_eq!(s.xover[16][0].filter_type, 34);
    assert_eq!(s.output_gain_db[0], -1.5);
    assert_eq!(s.master_volume_db, -18.0);
    assert_eq!(s.crossfeed_pair_map, vec![2, 19, 0, 0]);
    assert!(s.limiter_enabled[0]);

    // Without the options the volumes and limiters stay.
    let core = demo();
    let before = core.get_state();
    let r = json_of(&core.config_import(MAC.into(), 0));
    assert_eq!(r["ok"], true, "{r}");
    let s = core.get_state();
    assert_eq!(s.master_volume_db, before.master_volume_db);
    assert_eq!(s.limiter_enabled, before.limiter_enabled);
    assert_eq!(s.filters[0][0].freq, 100.0);
}

#[test]
fn clean1_hardware_kept_apart_from_presets_is_reported_written() {
    let core = demo();
    core.set_output_config_mode(0).unwrap();
    let text = core.config_export("x".into(), "1".into(), 0).unwrap();
    let mut v = json_of(&text);
    v["channels"][8]["limiter"]["thresholdDb"] = json!(-9.0);
    let r = json_of(&core.config_import(v.to_string(), IMPORT_HARDWARE));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["hardwareWritten"], true, "{r}");
    assert_eq!(core.get_state().limiter_threshold_db[0], -9.0);
    // The same document again changes nothing.
    let r = json_of(&core.config_import(v.to_string(), IMPORT_HARDWARE));
    assert_eq!(r["hardwareWritten"], false, "{r}");
    // And without the option nothing of it is written.
    v["channels"][8]["limiter"]["thresholdDb"] = json!(-3.0);
    let r = json_of(&core.config_import(v.to_string(), 0));
    assert_eq!(r["hardwareWritten"], false, "{r}");
    assert_eq!(core.get_state().limiter_threshold_db[0], -9.0);
}

#[test]
fn clean1_a_filter_import_writes_the_demo_device() {
    let core = demo();
    let r = json_of(&core.filters_import(MAC_FILTERS.into(), vec![0, 8, 16]));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["lines"][0], "Filters imported to 3 channels.");
    core.refresh_params().unwrap();
    let s = core.get_state();
    assert_eq!(s.input_preamp_db[0], -5.3);
    assert_eq!(s.filters[0][0].filter_type, 2);
    assert!(s.filters[0][2].bypass);
    assert_eq!(s.filters[8][1].filter_type, 11);
    assert_eq!(s.xover[8][0].filter_type, 35);
    assert_eq!(s.xover[16][0].filter_type, 58);
    assert!(s.output_enabled[0]);
    // Nothing for the chosen channels: refused, nothing written.
    let r = json_of(&core.filters_import(MAC_FILTERS.into(), vec![5]));
    assert_eq!(r["ok"], false, "{r}");
    let r = json_of(&core.filters_import(REW_FILTERS.into(), vec![1]));
    assert_eq!(r["lines"][0], "Filters imported to 1 channel.");
    assert_eq!(core.get_state().input_preamp_db[1], -6.5);
}

// ── Property tests ──

proptest! {
    #[test]
    fn proptest_clean1_parsing_a_document_never_panics(text in ".{0,400}") {
        let _ = parse_config(&text);
        let _ = config_inspect(text);
    }

    #[test]
    fn proptest_clean1_building_a_document_never_panics(
        cut in 0usize..4000,
        noise in proptest::collection::vec(any::<u8>(), 0..40),
        options in 0u32..4,
    ) {
        // A real document, cut and spliced with noise: whatever still parses
        // is built.
        let mut text = MAC.chars().take(cut).collect::<String>();
        text.push_str(&String::from_utf8_lossy(&noise));
        text.push_str(&MAC.chars().skip(cut).collect::<String>());
        if let Ok(v) = parse_config(&text) {
            let b = build_config(&capture(), &v, options);
            let _ = config_report(&b);
        }
        for t in [-1e30, 0.5, 300.0, 1e30] {
            let v = doc(&format!(
                r#""channels": [{{"isOutput": true, "outputIndex": {cut}, "delayMs": {t},
                    "eq": [{{"type": {t}}}], "limiter": {{"linkGroup": {t}}}}}],
                   "matrix": [{{"input": {t}, "output": 0}}],
                   "io": {{"outputPins": [{t}], "i2sInputRateHz": {t}}}"#
            ));
            let _ = config_report(&build_config(&default_state(), &v, options));
        }
    }

    #[test]
    fn proptest_clean1_reading_a_filter_file_never_panics(
        lines in proptest::collection::vec(
            prop_oneof![
                Just("# DSPi Console Filter Settings".to_string()),
                Just("# Format: 2".to_string()),
                "\\[(Input|Output|SPDIF|Master) [0-9]{0,4}:? ?[A-Za-z ()]{0,12}\\]",
                "(Filter|Xover|Crossover) {1,2}[0-9]{1,3}: (ON|OFF)  [A-Z0-9]{0,6} +Fc +[-0-9.,]{0,8} Hz( +Gain +[-+0-9.,]{0,7} dB)?( +Q +[0-9.,]{0,6})?",
                "Preamp:? [-+0-9.,]{0,8} dB",
                ".{0,40}",
            ],
            0..30,
        ),
        wires in proptest::collection::vec(any::<u8>(), 0..20),
    ) {
        let text = lines.join("\n");
        for s in [capture(), default_state()] {
            let _ = inspect_filters(&s, &text, 8);
            let plan = plan_filters(&s, &text, &wires);
            for w in &plan.writes {
                prop_assert!(wire_place(&s, w.wire).is_some());
                for b in w.filters.iter().chain(w.xover.iter()).flatten() {
                    prop_assert!(b.freq.is_finite() && b.q.is_finite() && b.gain.is_finite());
                }
            }
        }
    }

    #[test]
    fn proptest_clean1_filter_text_never_panics(text in ".{0,120}") {
        let _ = band_line(&text);
        let _ = preamp_line(&text);
        let _ = format_line(&text);
        let _ = number(&text);
        let _ = header_place(&text, &capture());
        let _ = labelled(&text.split_whitespace().collect::<Vec<_>>());
    }
}
