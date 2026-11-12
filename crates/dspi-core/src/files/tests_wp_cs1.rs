//! WP CS1 tests: an imported number past what the device takes is clamped
//! and named, with WP P0U's rows and wording.

use super::*;
use crate::state::default_state;

fn rp2350() -> DspState {
    let mut s = default_state();
    s.platform_id = 1;
    s.num_input_channels = 8;
    s.num_output_channels = 9;
    s.num_channels = 17;
    s
}

/// WP P0U's review file for the upmixer (release 3000, attack 0.5,
/// threshold 100, surround delay 30, detector HPF 1500; strength 50 in
/// range), plus an output gain, an EQ band, a preamp and a negative delay.
const WILD: &str = r#"{"schemaVersion": 1,
  "global": {"inputPreampsDb": [40, 0]},
  "upmix": {"strengthPct": 50, "releaseMs": 3000, "attackMs": 0.5,
            "thresholdPct": 100, "surroundDelayMs": 30, "detectorHpfHz": 1500},
  "channels": [
    {"inputIndex": 0, "delayMs": -5,
     "eq": [{"type": 1, "freqHz": 1000, "q": 1, "gain": 99}]},
    {"isOutput": true, "outputIndex": 2, "gainDb": 40}]}"#;

fn rows(b: &Built) -> Vec<(String, f64, Option<f64>)> {
    b.out_of_range
        .iter()
        .map(|o| (o.path.clone(), o.value, o.applied))
        .collect()
}

#[test]
fn wp_cs1_out_of_range_numbers_are_clamped_and_named() {
    let v = parse_config(WILD).unwrap();
    let b = build_config(&rp2350(), &v, 0);
    let up = |id: u8| b.state.upmix[usize::from(id)];
    assert_eq!(up(UPMIX_PARAM_RELEASE), 2000.0);
    assert_eq!(up(UPMIX_PARAM_ATTACK), 1.0);
    assert_eq!(up(UPMIX_PARAM_THRESHOLD), 95.0);
    assert_eq!(up(UPMIX_PARAM_SURROUND_DELAY), 20.0);
    assert_eq!(up(UPMIX_PARAM_DETECTOR_HPF), 1000.0);
    assert_eq!(up(UPMIX_PARAM_STRENGTH), 50.0, "in range, as it is");
    assert_eq!(b.state.channel_delays[0], 0.0);

    let r = rows(&b);
    let has = |path: &str, value: f64| {
        r.iter()
            .any(|(p, v, a)| p == path && *v == value && a.is_some())
    };
    for (path, value) in [
        ("up.release", 3000.0),
        ("up.attack", 0.5),
        ("up.threshold", 100.0),
        ("up.sur_delay", 30.0),
        ("up.det_hpf", 1500.0),
        ("pre[0]", 40.0),
        ("eq.gain[0,0]", 99.0),
        ("out.gain[2]", 40.0),
        ("ch.delay[0]", -5.0),
    ] {
        assert!(has(path, value), "{path} {value}: {r:?}");
    }
    assert_eq!(r.len(), 9, "{r:?}");
    // What was written is the row's applied value.
    let applied = |path: &str| r.iter().find(|(p, ..)| p == path).and_then(|(_, _, a)| *a);
    assert_eq!(
        f64::from(b.state.output_gain_db[2]),
        applied("out.gain[2]").unwrap()
    );
    assert_eq!(
        f64::from(b.state.input_preamp_db[0]),
        applied("pre[0]").unwrap()
    );
    assert_eq!(
        f64::from(b.state.filters[0][0].gain),
        applied("eq.gain[0,0]").unwrap()
    );

    let (lines, clean) = config_report(&b);
    assert!(!clean);
    let line = lines
        .iter()
        .find(|l| l.starts_with("Clamped to what this device takes: "))
        .expect("a line names the clamps");
    assert!(line.contains("up.release 3000 → 2000 ms"), "{line}");
}

/// On an RP2040 the preamp stops at +18 dB, as `Session::write_range` says.
#[test]
fn wp_cs1_the_platform_narrows_the_preamp() {
    let v = parse_config(
        r#"{"schemaVersion": 1, "global": {"inputPreampsDb": [20, 18]},
            "channels": [{"inputIndex": 0}]}"#,
    )
    .unwrap();
    let b = build_config(&default_state(), &v, 0);
    assert_eq!(b.state.input_preamp_db[0], 18.0);
    assert_eq!(b.state.input_preamp_db[1], 18.0);
    assert_eq!(rows(&b), [("pre[0]".to_string(), 20.0, Some(18.0))]);
}

/// A document within every range adds no row and stays clean.
#[test]
fn wp_cs1_numbers_in_range_add_nothing() {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    let text = core.config_export("demo".into(), "test".into(), 0).unwrap();
    let r: Value = serde_json::from_str(&core.config_import(text, 0)).unwrap();
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["outOfRange"], json!([]), "{r}");
}

/// The reply carries the rows for a client to show.
#[test]
fn wp_cs1_the_import_reply_carries_the_rows() {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    let doc = r#"{"schemaVersion": 1,
      "channels": [{"isOutput": true, "outputIndex": 0, "gainDb": 40}]}"#;
    let r: Value = serde_json::from_str(&core.config_import(doc.into(), 0)).unwrap();
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["clean"], false);
    let row = &r["outOfRange"][0];
    assert_eq!(row["path"], "out.gain[0]");
    assert_eq!(row["value"], 40.0);
    let applied = row["applied"].as_f64().unwrap();
    assert!(applied < 40.0, "{applied}");
    assert_eq!(f64::from(core.get_state().output_gain_db[0]), applied);
}

/// Every key the importer ranges names a registry number.
#[test]
fn wp_cs1_every_ranged_key_is_a_registry_number() {
    let blocks: [(&str, Vec<Field>); 9] = [
        ("global", global_fields()),
        ("global", volume_fields()),
        ("loudness", loudness_fields()),
        ("crossfeed", crossfeed_fields()),
        ("leveller", leveller_fields()),
        ("psybass", psybass_fields()),
        ("upmix", upmix_fields()),
        ("subharm", subharm_fields()),
        ("tube", tube_fields()),
    ];
    let mut s = default_state();
    for (block, fields) in blocks {
        for f in fields {
            let level = level_of(&mut s, f.slot).is_some();
            match block_path(block, f.key) {
                Some(path) => {
                    let d = dspi_proto::registry::by_path(path)
                        .unwrap_or_else(|| panic!("{block}.{}: {path} is not a path", f.key));
                    assert!(
                        matches!(
                            d.kind,
                            dspi_proto::registry::Kind::Float { .. }
                                | dspi_proto::registry::Kind::Int { .. }
                        ),
                        "{path} is not a number"
                    );
                    assert!(level, "{block}.{} has a path but is not a level", f.key);
                }
                None => assert!(!level, "{block}.{} is a level without a path", f.key),
            }
        }
    }
}
