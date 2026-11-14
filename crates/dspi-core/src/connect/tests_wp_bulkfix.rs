//! WP BULKFIX: a bulk write plays every gain as requested on firmware with the
//! bulk dB bug, with no moment where a gain plays louder than asked.

use std::sync::Arc;

use dspi_proto::bulk_gain::TOLERANCE_DB;
use dspi_proto::generated::{self as g, opcodes as op};
use dspi_transport::{BulkDbDevice, BulkDbOptions, FakeDevice, FakeRemote, GainProbe};

use crate::DspiCore;
use crate::consts::IMPORT_HARDWARE;

const K340: &str = include_str!("../../../../tests/fixtures/preset/k340-bug1.dspipreset");

/// The demo device on firmware whose bulk apply waits `apply_delay`
/// requests (the main loop), read with `fetch_all`.
fn core_on(options: BulkDbOptions) -> (Arc<DspiCore>, GainProbe, FakeRemote) {
    let dev = BulkDbDevice::new(FakeDevice::demo(), options);
    let (probe, remote) = (dev.probe(), dev.remote());
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), "BULKFIX".into(), 0)
        .unwrap();
    core.fetch_all().unwrap();
    (core, probe, remote)
}

fn buggy() -> BulkDbOptions {
    BulkDbOptions {
        fixed: false,
        apply_delay: 3,
    }
}

/// A −60 dB crosspoint, a −40 dB output, a −20 dB preamp and a +12 dB boost.
fn deep_cuts(core: &DspiCore) {
    let mut inner = core.lock();
    let s = &mut inner.state;
    s.matrix_routing[0][0] = true;
    s.matrix_gain[0][0] = -60.0;
    s.matrix_routing[1][1] = true;
    s.matrix_gain[1][1] = 12.0;
    s.output_enabled[0] = true;
    s.output_enabled[1] = true;
    s.output_gain_db[1] = -40.0;
    s.input_preamp_db[0] = -20.0;
}

fn mutes(remote: &FakeRemote, outputs: usize) -> Vec<u8> {
    let img = remote.image();
    (0..outputs)
        .map(|o| img[g::OFF_OUTPUTS + o * 12 + 1])
        .collect()
}

fn assert_plays_right(probe: &GainProbe) {
    assert!(probe.bulk_applies() >= 1);
    assert!(
        probe.worst_excess_db() <= TOLERANCE_DB,
        "a gain played {} dB too loud",
        probe.worst_excess_db()
    );
    assert!(
        probe.mismatch_db() <= TOLERANCE_DB,
        "a gain plays {} dB off",
        probe.mismatch_db()
    );
}

#[test]
fn wp_bulkfix_deep_cuts_play_as_requested_after_a_bulk_write() {
    let (core, probe, remote) = core_on(buggy());
    deep_cuts(&core);
    let want_mutes: Vec<u8> = core
        .lock()
        .state
        .output_muted
        .iter()
        .map(|m| u8::from(*m))
        .collect();
    core.apply_all_params().unwrap();
    assert_plays_right(&probe);
    assert!((probe.crosspoint_db(0, 0) + 60.0).abs() <= TOLERANCE_DB);
    // The device and the mirror hold what was asked, mutes included.
    assert_eq!(mutes(&remote, want_mutes.len()), want_mutes);
    let s = core.lock().state.clone();
    assert_eq!(s.matrix_gain[0][0], -60.0);
    assert_eq!(s.output_gain_db[1], -40.0);
    // The image went muted, then the single SETs, unmutes last.
    let sets = probe.sets();
    let first_single = sets
        .iter()
        .position(|o| *o != op::REQ_SET_ALL_PARAMS_CHUNK)
        .unwrap();
    assert!(
        sets[..first_single]
            .iter()
            .all(|o| *o == op::REQ_SET_ALL_PARAMS_CHUNK)
    );
    let tail = &sets[first_single..];
    let unmute_at = tail
        .iter()
        .position(|o| *o == op::REQ_SET_OUTPUT_MUTE)
        .unwrap();
    assert!(
        tail[unmute_at..]
            .iter()
            .all(|o| *o == op::REQ_SET_OUTPUT_MUTE)
    );
    for o in [
        op::REQ_SET_MATRIX_ROUTE,
        op::REQ_SET_OUTPUT_GAIN,
        op::REQ_SET_PREAMP_CH,
    ] {
        assert!(tail[..unmute_at].contains(&o), "0x{o:02X} re-sent");
    }
}

#[test]
fn wp_bulkfix_an_apply_waits_however_long_the_device_takes() {
    for apply_delay in [0, 1, 8] {
        let (core, probe, _) = core_on(BulkDbOptions {
            fixed: false,
            apply_delay,
        });
        deep_cuts(&core);
        core.apply_all_params().unwrap();
        assert_plays_right(&probe);
    }
}

#[test]
fn wp_bulkfix_firmware_with_the_fix_gets_the_image_alone() {
    let (core, probe, _) = core_on(BulkDbOptions {
        fixed: true,
        apply_delay: 3,
    });
    assert!(
        core.lock()
            .conn()
            .unwrap()
            .ext
            .has(dspi_proto::bulk_gain::EXT_FIRMWARE_FIXES)
    );
    deep_cuts(&core);
    core.apply_all_params().unwrap();
    assert!(
        probe
            .sets()
            .iter()
            .all(|o| *o == op::REQ_SET_ALL_PARAMS_CHUNK)
    );
    // Applied on the device's next request.
    core.fetch_all().unwrap();
    assert_plays_right(&probe);
}

#[test]
fn wp_bulkfix_the_k340_configuration_imports_and_plays_right() {
    let (core, probe, _) = core_on(buggy());
    let r: serde_json::Value =
        serde_json::from_str(&core.config_import_tok(K340.into(), IMPORT_HARDWARE)).unwrap();
    assert_eq!(r["ok"], true, "{r}");
    // Past the apply however it was sent (a bulk read waits for it).
    core.fetch_all().unwrap();
    assert_plays_right(&probe);
    // Every K340 gain is 0 dB: nothing but the mute bracket is added.
    let sets = probe.sets();
    assert!(!sets.iter().any(|o| matches!(
        *o,
        op::REQ_SET_MATRIX_ROUTE | op::REQ_SET_OUTPUT_GAIN | op::REQ_SET_PREAMP_CH
    )));
    // The owner's other unit runs its two inputs at +0.7 dB: within the
    // series' accuracy, still nothing to re-send.
    {
        let mut inner = core.lock();
        inner.state.input_preamp_db[0] = 0.7;
        inner.state.input_preamp_db[1] = 0.7;
    }
    let before = probe.sets().len();
    core.apply_all_params().unwrap();
    assert!(!probe.sets()[before..].contains(&op::REQ_SET_PREAMP_CH));
    assert_plays_right(&probe);
}

#[test]
fn wp_bulkfix_a_preset_recall_plays_right() {
    let (core, probe, _) = core_on(buggy());
    deep_cuts(&core);
    core.apply_all_params().unwrap();
    core.save_preset_tok(1).unwrap();
    // Another state, then the recall: the device loads the slot itself
    // (`REQ_PRESET_LOAD`, converted with powf), no bulk SET.
    {
        let mut inner = core.lock();
        inner.state.matrix_gain[0][0] = -3.0;
        inner.state.output_gain_db[1] = -1.0;
    }
    core.apply_all_params().unwrap();
    let before = probe.sets().len();
    core.load_preset(1).unwrap();
    assert!(!probe.sets()[before..].contains(&op::REQ_SET_ALL_PARAMS_CHUNK));
    assert_eq!(core.lock().state.matrix_gain[0][0], -60.0);
    assert_plays_right(&probe);
}

#[test]
fn wp_bulkfix_a_failed_repair_leaves_the_outputs_muted() {
    let (core, probe, remote) = core_on(buggy());
    deep_cuts(&core);
    remote.stall(op::REQ_SET_OUTPUT_GAIN);
    assert!(core.apply_all_params().is_err());
    let n = usize::from(core.lock().state.num_output_channels);
    assert!(mutes(&remote, n).iter().all(|m| *m == 1));
    assert!(core.lock().state.output_muted.iter().take(n).all(|m| *m));
    assert!(
        probe.worst_excess_db() <= TOLERANCE_DB,
        "nothing audible played loud"
    );
    assert!(
        probe.mismatch_db() > 50.0,
        "the series' −40 dB output is still in"
    );
}

// -- fixup bf --

fn suspects(core: &DspiCore) -> u32 {
    core.lock().conn.as_ref().unwrap().gain_guard.suspects()
}

/// Review bf's probe: the deep cuts, with the repair failing on the output
/// gains, so output 1 keeps the series' +19.5 dB for its −40 dB behind its
/// mute. The device works again afterwards.
fn failed_repair() -> (Arc<DspiCore>, GainProbe, FakeRemote) {
    let (core, probe, remote) = core_on(buggy());
    deep_cuts(&core);
    probe.fail(op::REQ_SET_OUTPUT_GAIN, true);
    assert!(core.apply_all_params().is_err());
    probe.fail(op::REQ_SET_OUTPUT_GAIN, false);
    assert!(probe.mismatch_db() > 50.0, "the series' gain is still in");
    assert_ne!(suspects(&core), 0);
    (core, probe, remote)
}

#[test]
fn wp_bulkfix_bf_a_manual_unmute_after_a_failed_repair_plays_right() {
    let (core, probe, remote) = failed_repair();
    core.set_output_mute(1, false).unwrap();
    core.set_output_mute(0, false).unwrap();
    assert!(
        probe.worst_excess_db() <= TOLERANCE_DB,
        "the unmute played {} dB too loud",
        probe.worst_excess_db()
    );
    assert!(
        probe.mismatch_db() <= TOLERANCE_DB,
        "{}",
        probe.mismatch_db()
    );
    assert_eq!(mutes(&remote, 2), vec![0, 0]);
    assert_eq!(
        suspects(&core) & 0b11,
        0,
        "verified outputs are not suspect any more"
    );
    // Muting and unmuting again sends nothing but the mutes.
    let before = probe.sets().len();
    core.set_output_mute(1, true).unwrap();
    core.set_output_mute(1, false).unwrap();
    assert_eq!(probe.sets()[before..], [op::REQ_SET_OUTPUT_MUTE; 2]);
}

#[test]
fn wp_bulkfix_bf_an_unmute_that_cannot_resend_the_gains_is_refused() {
    let (core, probe, remote) = failed_repair();
    probe.fail(op::REQ_SET_OUTPUT_GAIN, true);
    let e = core.set_output_mute(1, false).unwrap_err();
    assert!(e.to_string().contains("stays muted"), "{e}");
    assert_eq!(mutes(&remote, 2)[1], 1);
    assert!(probe.worst_excess_db() <= TOLERANCE_DB);
}

#[test]
fn wp_bulkfix_bf_a_raw_unmute_resends_the_gains_too() {
    let (core, probe, _) = failed_repair();
    core.raw_set(op::REQ_SET_OUTPUT_MUTE, 1, vec![0]).unwrap();
    assert!(probe.worst_excess_db() <= TOLERANCE_DB);
    assert!(probe.mismatch_db() <= TOLERANCE_DB);
}

#[test]
fn wp_bulkfix_bf_a_preset_load_or_a_new_apply_clears_the_suspicion() {
    let (core, probe, _) = core_on(buggy());
    core.save_preset_tok(2).unwrap();
    deep_cuts(&core);
    probe.fail(op::REQ_SET_OUTPUT_GAIN, true);
    assert!(core.apply_all_params().is_err());
    probe.fail(op::REQ_SET_OUTPUT_GAIN, false);
    assert_ne!(suspects(&core), 0);
    core.load_preset(2).unwrap();
    assert_eq!(suspects(&core), 0);
    assert!(probe.mismatch_db() <= TOLERANCE_DB);

    let (core, probe, _) = failed_repair();
    core.apply_all_params().unwrap();
    assert_eq!(suspects(&core), 0);
    assert!(probe.mismatch_db() <= TOLERANCE_DB);
}

#[test]
fn wp_bulkfix_bf_outputs_muted_at_connect_are_suspect_without_the_fix() {
    // A repair cut short before this connection: output 1 muted, −40 dB,
    // playing the series' value.
    let mut dev = BulkDbDevice::new(FakeDevice::demo(), buggy());
    let probe = dev.probe();
    let mut img = dev.remote().image();
    let out = g::OFF_OUTPUTS + 12;
    img[out] = 1;
    img[out + 1] = 1;
    img[out + 4..out + 8].copy_from_slice(&(-40.0f32).to_le_bytes());
    use dspi_transport::Transport;
    dev.control_out(op::REQ_SET_ALL_PARAMS, 0, &img).unwrap();
    while probe.bulk_applies() == 0 {
        let _ = dev.control_in(op::REQ_GET_PLATFORM, 0, 7);
    }
    assert!(probe.mismatch_db() > 50.0);
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), "BULKFIX".into(), 0)
        .unwrap();
    core.fetch_all().unwrap();
    assert_eq!(suspects(&core), 0b10);
    core.set_output_mute(1, false).unwrap();
    assert!(
        probe.worst_excess_db() <= TOLERANCE_DB,
        "{}",
        probe.worst_excess_db()
    );
    assert!(probe.mismatch_db() <= TOLERANCE_DB);
}

#[test]
fn wp_bulkfix_bf_a_failed_import_repair_shows_the_device_muted_and_why() {
    let (core, probe, remote) = core_on(buggy());
    probe.fail(op::REQ_SET_OUTPUT_MUTE, true);
    let r: serde_json::Value =
        serde_json::from_str(&core.config_import_tok(K340.into(), IMPORT_HARDWARE)).unwrap();
    assert_eq!(r["ok"], false, "{r}");
    let why = r["error"].as_str().unwrap_or_default();
    assert!(why.contains("left muted"), "{r}");
    let s = core.get_state();
    let n = usize::from(s.num_output_channels);
    assert!(
        s.output_muted.iter().take(n).all(|m| *m),
        "the mirror shows the device muted"
    );
    assert!(mutes(&remote, n).iter().all(|m| *m == 1));
    // The import's matrix is what the device holds (USB 2 feeds output 1).
    assert!(s.matrix_routing[1][0]);
    let logged = core.event_log_read(String::new(), 0, 100).entries;
    assert!(
        logged.iter().any(|e| e.text.contains("left muted")),
        "{:?}",
        logged.iter().map(|e| &e.text).collect::<Vec<_>>()
    );
}

// -- WP SW2 --

/// SW1's note: an apply-at-frame UNMUTE from a unit on the audio link goes
/// past the per-output unmute, so the guard takes it as an unmute of every
/// suspect output: the gains are re-sent first, or the op is refused.
#[test]
fn wp_sw2_an_apply_at_frame_unmute_is_gated_as_an_unmute() {
    let link_core = |fail: bool| {
        let dev = BulkDbDevice::new(FakeDevice::demo_alink(), buggy());
        let (probe, remote) = (dev.probe(), dev.remote());
        let core = DspiCore::new();
        core.attach_probed(Box::new(dev), "SW2".into(), 0).unwrap();
        core.fetch_all().unwrap();
        deep_cuts(&core);
        probe.fail(op::REQ_SET_OUTPUT_GAIN, true);
        assert!(core.apply_all_params().is_err());
        probe.fail(op::REQ_SET_OUTPUT_GAIN, fail);
        assert_ne!(suspects(&core), 0);
        (core, probe, remote)
    };
    let frame = |core: &DspiCore| core.fetch_alink_frame_counter().unwrap().frame + 48_000;

    let (core, probe, _) = link_core(false);
    core.apply_at_frame(frame(&core), 2, 0, 0).unwrap();
    assert_eq!(suspects(&core), 0);
    assert!(
        probe.mismatch_db() <= TOLERANCE_DB,
        "{}",
        probe.mismatch_db()
    );

    let (core, probe, _) = link_core(true);
    let e = core.apply_at_frame(frame(&core), 2, 0, 0).unwrap_err();
    assert!(e.to_string().contains("stays muted"), "{e}");
    assert_eq!(core.fetch_apply_at_frame_status().unwrap().pending, 0);
    // A MUTE is not gated.
    core.apply_at_frame(frame(&core), 1, 0, 0).unwrap();
    assert!(probe.worst_excess_db() <= TOLERANCE_DB);
}
