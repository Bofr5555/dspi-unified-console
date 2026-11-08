//! WP DL3 tests: an imported delay past the line is shortened and named.

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

const LONG: &str = r#"{"schemaVersion": 1,
  "channels": [
    {"inputIndex": 0, "name": "Master L", "delayMs": 40},
    {"inputIndex": 1, "name": "Master R", "delayMs": 3},
    {"isOutput": true, "outputIndex": 0, "name": "Sub", "delayMs": 85, "outputDelayMs": 30.5},
    {"isOutput": true, "outputIndex": 1, "outputDelayMs": 21}]}"#;

/// On an RP2040 (21 ms) the import shortens the three delays past the line,
/// names each in the report and is not clean; the ones within it are not named.
#[test]
fn wp_dl3_a_delay_past_the_line_is_named_in_the_import_report() {
    let v = parse_config(LONG).unwrap();
    let b = build_config(&default_state(), &v, 0);
    assert_eq!(b.state.channel_delays[0], 21.0);
    assert_eq!(b.state.channel_delays[1], 3.0);
    assert_eq!(b.state.output_delay_ms[0], 21.0);
    assert_eq!(b.state.output_delay_ms[1], 21.0);
    let (lines, clean) = config_report(&b);
    assert!(!clean);
    let line = lines
        .iter()
        .find(|l| l.starts_with("Delays shortened"))
        .expect("a line names the shortened delays");
    assert_eq!(
        line,
        "Delays shortened to the 21 ms this device (RP2040) holds: Master L (40 ms), \
         Sub (85 ms), Sub output delay (30.5 ms)."
    );
    assert_eq!(lines.last().unwrap(), NOT_STORED);
}

/// On an RP2350 (42 ms) only the 85 ms delay is past the line.
#[test]
fn wp_dl3_the_rp2350_shortens_only_what_is_past_42_ms() {
    let v = parse_config(LONG).unwrap();
    let b = build_config(&rp2350(), &v, 0);
    assert_eq!(b.state.channel_delays[0], 40.0);
    assert_eq!(b.state.output_delay_ms[0], 30.5);
    let (lines, _) = config_report(&b);
    assert!(lines.contains(
        &"Delays shortened to the 42 ms this device (RP2350) holds: Sub (85 ms).".to_string()
    ));
}

/// Delays within the line leave the report as it was: no line, still clean.
#[test]
fn wp_dl3_delays_within_the_line_add_nothing() {
    let v = parse_config(r#"{"schemaVersion": 1, "channels": [{"inputIndex": 0, "delayMs": 21}]}"#)
        .unwrap();
    let b = build_config(&default_state(), &v, 0);
    let (lines, _) = config_report(&b);
    assert!(!lines.iter().any(|l| l.contains("shortened")), "{lines:?}");
    assert!(b.shortened.is_empty());
}
