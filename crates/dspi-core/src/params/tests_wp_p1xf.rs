//! WP P1XF: a named crossfeed preset gives its own frequency and feed to
//! every client, before and after the device's echo and a re-read; the custom
//! pair is kept (device, record, export, undo) and comes back with Custom.

use std::sync::Arc;
use std::time::Duration;

use dspi_proto::generated::{OFF_CROSSFEED, notify as nfy};
use dspi_transport::{FakeDevice, FakeRemote};
use serde_json::Value;

use super::CROSSFEED_PRESETS;
use crate::consts::{IMPORT_HARDWARE, IMPORT_VOLUMES};
use crate::records::DspState;
use crate::{DspiCore, state};

const MAC: &str = include_str!("../../../../tests/fixtures/preset/mac-swift-v32.dspipreset");

/// `WireCrossfeedParams` offsets in the image.
const PRESET_AT: usize = OFF_CROSSFEED + 1;
const FREQ_AT: usize = OFF_CROSSFEED + 4;

fn demo() -> (Arc<DspiCore>, FakeRemote) {
    let dev = FakeDevice::demo();
    let remote = dev.remote();
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), "DEMO".into(), 0).unwrap();
    (core, remote)
}

/// (preset, freq, feed) as clients read them.
fn shown(core: &DspiCore) -> (u8, f32, f32) {
    let s = core.get_state();
    (s.crossfeed_preset, s.crossfeed_freq, s.crossfeed_feed)
}

/// The core's own record.
fn kept(core: &DspiCore) -> DspState {
    core.lock().state.clone()
}

/// (preset, custom freq, custom feed) in the device's image.
fn on_device(remote: &FakeRemote) -> (u8, f32, f32) {
    let mut s = crate::state::default_state();
    state::decode_bulk(&remote.image(), &mut s).unwrap();
    (s.crossfeed_preset, s.crossfeed_freq, s.crossfeed_feed)
}

/// Let the device's echoes of our writes arrive and be handled.
fn settle(core: &DspiCore) {
    for _ in 0..20 {
        core.process_notifications();
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Handle notifications until the record satisfies `done`.
fn wait_for(core: &DspiCore, done: impl Fn(&DspState) -> bool) {
    let t = std::time::Instant::now();
    while !done(&kept(core)) {
        assert!(t.elapsed() < Duration::from_secs(5), "no notification");
        core.process_notifications();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A custom pair the presets do not have, chosen as Custom.
fn custom_800_5(core: &DspiCore) {
    core.set_crossfeed_preset(3).unwrap();
    core.set_crossfeed_freq(800.0).unwrap();
    core.set_crossfeed_feed(5.0).unwrap();
    assert_eq!(shown(core), (3, 800.0, 5.0));
}

#[test]
fn wp_p1xf_chu_moy_reads_its_values_before_and_after_the_echo_and_a_re_read() {
    let (core, remote) = demo();
    custom_800_5(&core);
    core.set_crossfeed_preset(1).unwrap();
    assert_eq!(shown(&core), (1, 700.0, 6.0), "before the echo");
    settle(&core);
    assert_eq!(shown(&core), (1, 700.0, 6.0), "after the echo");
    core.fetch_all().unwrap();
    assert_eq!(shown(&core), (1, 700.0, 6.0), "after fetch_all");
    // The device and the core's record keep the custom pair
    assert_eq!(on_device(&remote), (1, 800.0, 5.0));
    let k = kept(&core);
    assert_eq!((k.crossfeed_freq, k.crossfeed_feed), (800.0, 5.0));
}

#[test]
fn wp_p1xf_custom_restores_the_custom_pair() {
    let (core, remote) = demo();
    custom_800_5(&core);
    core.set_crossfeed_preset(2).unwrap();
    assert_eq!(shown(&core), (2, 650.0, 9.5));
    core.set_crossfeed_preset(3).unwrap();
    assert_eq!(shown(&core), (3, 800.0, 5.0));
    settle(&core);
    core.fetch_all().unwrap();
    assert_eq!(
        shown(&core),
        (3, 800.0, 5.0),
        "after the echo and fetch_all"
    );
    assert_eq!(on_device(&remote), (3, 800.0, 5.0));
}

#[test]
fn wp_p1xf_each_named_preset_shows_its_own_values() {
    let (core, _remote) = demo();
    custom_800_5(&core);
    for (p, &(freq, feed)) in (0u8..).zip(CROSSFEED_PRESETS.iter()) {
        core.set_crossfeed_preset(p).unwrap();
        assert_eq!(shown(&core), (p, freq, feed), "preset {p}");
    }
    assert_eq!(
        CROSSFEED_PRESETS,
        [(700.0, 4.5), (700.0, 6.0), (650.0, 9.5)],
        "the firmware's table (crossfeed.c)"
    );
}

#[test]
fn wp_p1xf_a_custom_value_written_under_a_named_preset_waits_for_custom() {
    let (core, remote) = demo();
    custom_800_5(&core);
    core.set_crossfeed_preset(1).unwrap();
    core.set_crossfeed_freq(900.0).unwrap();
    core.set_crossfeed_feed(7.0).unwrap();
    settle(&core);
    assert_eq!(shown(&core), (1, 700.0, 6.0), "the preset's values stay");
    assert_eq!(on_device(&remote), (1, 900.0, 7.0));
    core.set_crossfeed_preset(3).unwrap();
    assert_eq!(shown(&core), (3, 900.0, 7.0));
}

#[test]
fn wp_p1xf_a_preset_chosen_at_the_device_shows_its_values() {
    let (core, remote) = demo();
    custom_800_5(&core);
    settle(&core);
    let uart = nfy::PARAM_SRC_UART as u8;
    // Another host picks Meier
    assert!(remote.poke(PRESET_AT, &[2], uart));
    wait_for(&core, |s| s.crossfeed_preset == 2);
    assert_eq!(shown(&core), (2, 650.0, 9.5));
    // ... changes the custom frequency under it
    assert!(remote.poke(FREQ_AT, &1000.0f32.to_le_bytes(), uart));
    wait_for(&core, |s| s.crossfeed_freq == 1000.0);
    assert_eq!(shown(&core), (2, 650.0, 9.5));
    // ... and goes back to Custom
    assert!(remote.poke(PRESET_AT, &[3], uart));
    wait_for(&core, |s| s.crossfeed_preset == 3);
    assert_eq!(shown(&core), (3, 1000.0, 5.0));
}

#[test]
fn wp_p1xf_undo_of_a_preset_choice_brings_the_custom_pair_back() {
    let (core, _remote) = demo();
    custom_800_5(&core);
    core.set_crossfeed_preset(1).unwrap();
    core.undo().unwrap();
    assert_eq!(shown(&core), (3, 800.0, 5.0));
    core.redo().unwrap();
    assert_eq!(shown(&core), (1, 700.0, 6.0));
    // An undone custom write under a named preset restores the custom value
    core.set_crossfeed_freq(900.0).unwrap();
    core.undo().unwrap();
    assert_eq!(kept(&core).crossfeed_freq, 800.0);
    assert_eq!(shown(&core), (1, 700.0, 6.0));
}

fn crossfeed_of(doc: &str) -> Value {
    let v: Value = serde_json::from_str(doc).unwrap();
    v["crossfeed"].clone()
}

/// The export writes the custom pair under a named preset, as the Mac's
/// document does (`mac-swift-v32`: Chu Moy with 700 Hz / 4.5 dB, not Chu
/// Moy's own 6 dB), so an import puts the custom pair back.
#[test]
fn wp_p1xf_the_export_matches_the_mac_fixture() {
    let (core, remote) = demo();
    let r: Value =
        serde_json::from_str(&core.config_import(MAC.into(), IMPORT_VOLUMES | IMPORT_HARDWARE))
            .unwrap();
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(shown(&core), (1, 700.0, 6.0), "the graph draws Chu Moy");
    assert_eq!(on_device(&remote), (1, 700.0, 4.5));
    let out = core.config_export("x".into(), "1".into(), 0).unwrap();
    let (mac, ours) = (crossfeed_of(MAC), crossfeed_of(&out));
    for key in ["enabled", "preset", "itd"] {
        assert_eq!(ours[key], mac[key], "crossfeed.{key}: {ours}");
    }
    // The Mac writes 700, the core 700.0: the same number
    for key in ["freqHz", "feedDb"] {
        assert_eq!(
            ours[key].as_f64(),
            mac[key].as_f64(),
            "crossfeed.{key}: {ours}"
        );
    }
}

#[test]
fn wp_p1xf_an_exported_custom_pair_under_a_named_preset_round_trips() {
    let (core, _remote) = demo();
    custom_800_5(&core);
    core.set_crossfeed_preset(2).unwrap();
    let out = core.config_export("x".into(), "1".into(), 0).unwrap();
    let cf = crossfeed_of(&out);
    assert_eq!(
        (&cf["preset"], cf["freqHz"].as_f64(), cf["feedDb"].as_f64()),
        (&Value::from(2), Some(800.0), Some(5.0)),
        "{cf}"
    );
    let (other, remote) = demo();
    let r: Value = serde_json::from_str(&other.config_import(out, 0)).unwrap();
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(shown(&other), (2, 650.0, 9.5));
    assert_eq!(on_device(&remote), (2, 800.0, 5.0));
    other.set_crossfeed_preset(3).unwrap();
    assert_eq!(shown(&other), (3, 800.0, 5.0));
}
