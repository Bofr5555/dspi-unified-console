//! WP ROMODE: the client read-only mode refuses every write family with
//! nothing sent, and reads, notifications and the meters go on.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dspi_proto::generated::{self as g, opcodes as op};
use dspi_transport::{
    DeviceDescriptor, FakeDevice, FakeRemote, NotificationSource, Transport, TransportCaps,
};

use crate::{
    AbSide, BootloaderDrive, ConfirmOp, DspiCore, DspiError, FirmwareGuardBoard,
    FirmwareGuardBoardSource, ParamValue, SiggenConfig,
};

/// Every transfer the core made: (OUT, opcode, wValue).
type Log = Arc<Mutex<Vec<(bool, u8, u16)>>>;

/// The demo device, every transfer logged.
struct Spy {
    inner: FakeDevice,
    log: Log,
}

impl Spy {
    fn note(&self, out: bool, op: u8, value: u16) {
        self.log.lock().unwrap().push((out, op, value));
    }
}

impl Transport for Spy {
    fn control_in(&mut self, op: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.note(false, op, value);
        self.inner.control_in(op, value, len)
    }
    fn control_in_upto(&mut self, op: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        self.note(false, op, value);
        self.inner.control_in_upto(op, value, len)
    }
    fn control_out(&mut self, op: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        self.note(true, op, value);
        self.inner.control_out(op, value, data)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        self.inner.descriptor()
    }
    fn max_transfer(&self) -> usize {
        self.inner.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.inner.notifications()
    }
    fn caps(&self) -> TransportCaps {
        self.inner.caps()
    }
}

fn demo_of(dev: FakeDevice) -> (Arc<DspiCore>, FakeRemote, Log) {
    let remote = dev.remote();
    let log = Log::default();
    let core = DspiCore::new();
    core.attach_probed(
        Box::new(Spy {
            inner: dev,
            log: log.clone(),
        }),
        "DEMO".into(),
        0,
    )
    .unwrap();
    settle(&core);
    (core, remote, log)
}

fn demo() -> (Arc<DspiCore>, FakeRemote, Log) {
    demo_of(FakeDevice::demo())
}

fn settle(core: &DspiCore) {
    let mut quiet = 0;
    while quiet < 10 {
        std::thread::sleep(Duration::from_millis(5));
        let r = core.process_notifications();
        quiet = if r.flags == 0 { quiet + 1 } else { 0 };
    }
}

/// What reached the device that was not a pure read: every OUT, and every
/// IN of an opcode the registry does not call a pure read.
fn writes(log: &Log) -> Vec<(bool, u8, u16)> {
    log.lock()
        .unwrap()
        .iter()
        .copied()
        .filter(|&(out, op, _)| out || !dspi_proto::registry::is_pure_read(op))
        .collect()
}

fn refused<T: std::fmt::Debug>(name: &str, r: crate::Result<T>) {
    assert!(matches!(r, Err(DspiError::ReadOnly)), "{name}: {r:?}");
}

/// A JSON reply that is a refusal.
fn refused_reply(name: &str, reply: &str) {
    let v: serde_json::Value = serde_json::from_str(reply).unwrap();
    assert_eq!(v["ok"], false, "{name}: {reply}");
    assert!(
        reply.contains("read-only") || reply.contains("read_only"),
        "{name}: {reply}"
    );
}

fn tok(core: &DspiCore, op: ConfirmOp) -> Option<String> {
    Some(core.confirm_token(op))
}

#[test]
fn wp_romode_the_flag_is_off_by_default_and_stays_across_a_reconnect() {
    let core = DspiCore::new();
    assert!(!core.client_read_only());
    core.set_client_read_only(true).unwrap();
    assert!(core.client_read_only());
    core.open_demo().unwrap();
    assert!(core.client_read_only());
    refused("no device swap", core.set_master_volume(-3.0));
    core.disconnect();
    core.open_demo().unwrap();
    refused("after a reconnect", core.set_master_volume(-3.0));
    core.set_client_read_only(false).unwrap();
    assert!(!core.client_read_only());
    core.set_master_volume(-3.0).unwrap();
    assert_eq!(core.get_state().master_volume_db, -3.0);
}

#[test]
fn wp_romode_every_write_family_is_refused_with_nothing_sent() {
    let (core, remote, log) = demo();
    // Before the mode: documents to import and recall, and one undo step.
    let config = core.config_export("x".into(), "t".into(), 0).unwrap();
    let filters = core.filters_export(0).unwrap();
    let snapshot = core
        .snapshot_capture_document("s".into(), String::new())
        .unwrap();
    core.set_master_volume(-10.0).unwrap();
    assert!(core.can_undo());
    let state = core.get_state();
    let band = state.filters[0][0];

    core.set_client_read_only(true).unwrap();
    log.lock().unwrap().clear();
    let image = remote.image();
    let boot_token = tok(&core, ConfirmOp::EnterBootloader);

    // Typed setters, one or more per family.
    refused("set_master_volume", core.set_master_volume(-6.0));
    refused("set_user_volume", core.set_user_volume(-6.0));
    refused("save_master_volume", core.save_master_volume());
    refused("set_preamp", core.set_preamp(-1.0));
    refused(
        "set_filter",
        core.set_filter(0, 0, crate::FilterParams { gain: 3.0, ..band }),
    );
    refused("set_bypass", core.set_bypass(true));
    refused("set_output_gain", core.set_output_gain(0, -2.0));
    refused("set_output_mute", core.set_output_mute(0, true));
    refused("set_output_delay", core.set_output_delay(0, 1.0));
    refused(
        "set_matrix_route",
        core.set_matrix_route(0, 0, true, -1.0, false),
    );
    refused("set_channel_name", core.set_channel_name(0, "x".into()));
    refused("set_device_name", core.set_device_name("x".into()));
    refused("set_output_pin", core.set_output_pin(0, 6));
    refused("set_dac_mute", core.set_dac_mute(true, false, 5, 10, 10));
    refused("set_limiter_param", core.set_limiter_param(0, 0, -1.0));
    refused("set_loudness", core.set_loudness(true));
    refused("set_crossfeed", core.set_crossfeed(true));
    refused("set_leveller_enabled", core.set_leveller_enabled(true));
    refused("set_output_latency", core.set_output_latency(1));
    refused(
        "write_param",
        core.write_param("vol.master".into(), vec![], ParamValue::Float(-6.0)),
    );
    // Presets.
    refused("load_preset", core.load_preset(1));
    refused("set_preset_name", core.set_preset_name(1, "x".into()));
    refused("load_params", core.load_params());
    // The gated calls, each with a good token.
    refused(
        "write_param_confirmed",
        core.write_param_confirmed(
            "dev.save".into(),
            vec![],
            ParamValue::Trigger,
            tok(&core, ConfirmOp::SaveParams),
        ),
    );
    refused(
        "save_preset_confirmed",
        core.save_preset_confirmed(1, tok(&core, ConfirmOp::SavePreset)),
    );
    refused(
        "copy_preset_confirmed",
        core.copy_preset_confirmed(1, 2, tok(&core, ConfirmOp::CopyPreset)),
    );
    refused(
        "delete_preset_confirmed",
        core.delete_preset_confirmed(1, tok(&core, ConfirmOp::DeletePreset)),
    );
    refused(
        "save_params_confirmed",
        core.save_params_confirmed(tok(&core, ConfirmOp::SaveParams)),
    );
    refused(
        "save_output_config_confirmed",
        core.save_output_config_confirmed(tok(&core, ConfirmOp::SaveOutputConfig)),
    );
    refused(
        "factory_reset_confirmed",
        core.factory_reset_confirmed(tok(&core, ConfirmOp::FactoryReset)),
    );
    refused(
        "enter_bootloader_confirmed",
        core.enter_bootloader_confirmed(boot_token.clone()),
    );
    refused(
        "core_vreg_op_confirmed",
        core.core_vreg_op_confirmed(0, 10, None, tok(&core, ConfirmOp::VregSave)),
    );
    refused(
        "core_vreg_manual_set_confirmed",
        core.core_vreg_manual_set_confirmed(10, None, tok(&core, ConfirmOp::VregSave)),
    );
    refused(
        "flash_confirmed",
        core.flash_confirmed(
            vec![0; 512],
            BootloaderDrive {
                path: "nowhere".into(),
                platform: 1,
                info: String::new(),
            },
            FirmwareGuardBoard {
                required_mv: 1150,
                source: FirmwareGuardBoardSource::NoDevice,
            },
            String::new(),
            tok(&core, ConfirmOp::FirmwareFlash),
        ),
    );
    // The core voltage without a token, the signal generator (identify, absent
    // on the demo, takes the same `writable_conn`).
    refused("core_vreg_manual_auto", core.core_vreg_manual_auto(true));
    refused("siggen_control", core.siggen_control(1));
    refused(
        "siggen_set_config",
        core.siggen_set_config(SiggenConfig {
            channel_mask: 1,
            p: vec![1000.0; 4],
            ..SiggenConfig::default()
        }),
    );
    // Imports, recalls, A/B, the guarded apply, the whole image, undo.
    refused_reply(
        "config_import_confirmed",
        &core.config_import_confirmed(
            config.clone(),
            0,
            u32::MAX,
            tok(&core, ConfirmOp::ConfigImport),
        ),
    );
    refused_reply("filters_import", &core.filters_import(filters, vec![]));
    refused(
        "snapshot_recall_document",
        core.snapshot_recall_document(snapshot.clone()),
    );
    refused(
        "ab_start_documents",
        core.ab_start_documents(String::new(), snapshot.clone()),
    );
    refused("ab_toggle", core.ab_toggle());
    assert!(!core.ab_state().active);
    refused(
        "apply_guarded_snapshot_document",
        core.apply_guarded_snapshot_document(snapshot.clone(), 30),
    );
    refused(
        "apply_guarded_import",
        core.apply_guarded_import(config, 0, u32::MAX, 30),
    );
    refused("apply_guard_begin", core.apply_guard_begin("x".into(), 30));
    refused("apply_all_params", core.apply_all_params());
    refused("undo", core.undo());
    assert!(core.can_undo(), "the step stays to be undone");
    // A control surface.
    refused_reply(
        "cs_apply",
        &core.cs_apply(r#"{"op":"name","index":0,"name":"x"}"#.into()),
    );
    // -- WP ROMODEF -- the device's counters: not even §1.2's housekeeping.
    refused("clear_clips", core.clear_clips());
    refused("reset_buffer_stats", core.reset_buffer_stats());
    // Raw: no write, no write-as-read, not even the §1.2 housekeeping.
    refused(
        "raw_set",
        core.raw_set(
            op::REQ_SET_MASTER_VOLUME,
            0,
            (-6.0f32).to_le_bytes().to_vec(),
        ),
    );
    for code in [
        op::REQ_PRESET_LOAD,
        op::REQ_RTA_CONTROL,
        op::REQ_SIGGEN_CONTROL,
        op::REQ_CLEAR_CLIPS,
        op::REQ_RESET_BUFFER_STATS,
        op::REQ_LIMITER,
        0x01,
    ] {
        refused(&format!("raw_get 0x{code:02X}"), core.raw_get(code, 1, 1));
    }

    assert_eq!(
        writes(&log),
        [],
        "nothing but pure reads reached the device"
    );
    assert_eq!(remote.image(), image, "the device is unchanged");
    assert!(core.client_read_only_refusals() >= 50);
    assert_eq!(core.read_only_refusals(), 0, "the connection is writable");
    assert_eq!(core.get_state().master_volume_db, -10.0, "the mirror too");

    // A refused gated call did not spend its token.
    core.set_client_read_only(false).unwrap();
    core.enter_bootloader_confirmed(boot_token).unwrap();
}

#[test]
fn wp_romode_reads_notifications_and_meters_go_on() {
    let (core, remote, log) = demo();
    core.set_client_read_only(true).unwrap();
    log.lock().unwrap().clear();
    core.fetch_all().unwrap();
    core.refresh_params().unwrap();
    core.fetch_status().unwrap();
    core.get_preset_directory().unwrap();
    assert!(!core.raw_get(op::REQ_GET_PLATFORM, 0, 7).unwrap().is_empty());
    assert!(!core.read_image_bytes().unwrap().is_empty());
    // Another console's change reaches the mirror.
    assert!(remote.poke(
        g::OFF_MASTER_VOLUME,
        &(-12.0f32).to_le_bytes(),
        g::notify::PARAM_SRC_UAC1 as u8
    ));
    let until = Instant::now() + Duration::from_secs(5);
    while core.get_state().master_volume_db != -12.0 {
        assert!(Instant::now() < until, "the notification never arrived");
        core.process_notifications();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(core.client_read_only_refusals(), 0);
    assert_eq!(
        writes(&log),
        [],
        "nothing but pure reads reached the device"
    );
    // Reads of local state: a snapshot, an export, the history.
    core.snapshot_capture_document("s".into(), String::new())
        .unwrap();
    core.config_export("x".into(), "t".into(), 0).unwrap();
    let _ = core.history(10);
}

#[test]
fn wp_romode_the_audio_link_setters_are_refused() {
    let (core, remote, log) = demo_of(FakeDevice::demo_alink());
    core.set_client_read_only(true).unwrap();
    log.lock().unwrap().clear();
    let image = remote.image();
    let slots = core.fetch_alink_slots().unwrap();
    refused("set_alink_slots", core.set_alink_slots(slots));
    refused("set_alink_phase", core.set_alink_phase(1, 0, 0));
    refused("apply_at_frame", core.apply_at_frame(1_000_000, 0, 0, 0));
    refused("reset_alink_stats", core.reset_alink_stats());
    assert_eq!(writes(&log), []);
    assert_eq!(remote.image(), image);
}

#[test]
fn wp_romode_a_refusal_has_the_client_row_and_the_firmware_one_stays() {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    let firmware = core.notice_for_error(DspiError::ReadOnly);
    assert_eq!(firmware.key, "read_only");
    core.set_client_read_only(true).unwrap();
    let e = core.set_master_volume(-1.0).unwrap_err();
    let n = core.notice_for_error(e.clone());
    assert_eq!(n.key, super::NOTICE_READ_ONLY_CLIENT);
    assert_eq!(n.title, "Read-only mode");
    assert_eq!(n.kind, crate::NoticeKind::Warning);
    assert_eq!(
        core.client_notice_key(&e.notice_key()),
        Some(super::NOTICE_READ_ONLY_CLIENT)
    );
    // Other errors keep their own rows.
    assert_eq!(core.notice_for_error(DspiError::Locked).key, "locked");
    assert_eq!(core.client_notice_key("locked"), None);
    core.set_client_read_only(false).unwrap();
    assert_eq!(core.client_notice_key("read_only"), None);
}

#[test]
fn wp_romode_hub_requests_are_refused_unless_they_read() {
    let core = DspiCore::new();
    core.set_client_read_only(true).unwrap();
    let mut inner = core.lock();
    for t in super::HUB_READS {
        assert_eq!(super::check_hub_request(&mut inner, t), Ok(()), "{t}");
    }
    for t in [
        "system.set",
        "system.align",
        "system.mute",
        "system.scene.apply",
        "system.alarm.ack",
        "apply.begin",
        "apply.confirm",
        "limits.set",
        "limits.lock",
        "limits.unlock",
        "limits.pin",
        "mode.pin",
    ] {
        assert_eq!(
            super::check_hub_request(&mut inner, t),
            Err(DspiError::ReadOnly),
            "{t}"
        );
    }
    inner.client_read_only.on = false;
    assert_eq!(super::check_hub_request(&mut inner, "system.set"), Ok(()));
}

#[test]
fn wp_romode_viewing_writes_pass_and_nothing_else_does() {
    let (core, _remote, _log) = demo();
    core.set_client_read_only(true).unwrap();
    assert!(matches!(
        core.lock().writable_conn(),
        Err(DspiError::ReadOnly)
    ));
    {
        let _v = super::Viewing::allow();
        assert!(core.lock().writable_conn().is_ok());
    }
    assert!(core.lock().writable_conn().is_err());
    // The ending of an A/B on its live side sends nothing either way.
    assert!(core.ab_keep(AbSide::A).is_err());
}

#[test]
fn wp_romode_the_mode_is_refused_while_a_guarded_apply_waits_and_the_way_back_stays() {
    let (core, remote, log) = demo();
    let before = core.get_state().master_volume_db;
    core.apply_guard_begin("flow".into(), 30).unwrap();
    core.set_master_volume(before - 6.0).unwrap();

    // Refused, with its own row, and nothing changed: writes still go out.
    let e = core.set_client_read_only(true).unwrap_err();
    assert_eq!(
        e,
        DspiError::InvalidArgument {
            msg: super::GUARD_PENDING.into()
        }
    );
    let n = core.notice_for_error(e);
    assert_eq!(n.key, super::NOTICE_READ_ONLY_GUARD_PENDING);
    assert_eq!(n.title, "A trial is waiting"); // -- WP UFW -- (UXP D10)
    assert!(!core.client_read_only());
    // Turning it off is never refused.
    core.set_client_read_only(false).unwrap();

    // The guard's revert puts the device back, then the mode goes on.
    log.lock().unwrap().clear();
    core.apply_revert().unwrap();
    assert_eq!(core.get_state().master_volume_db, before, "reverted");
    assert!(
        log.lock().unwrap().iter().any(|&(out, _, _)| out),
        "the revert wrote"
    );
    core.set_client_read_only(true).unwrap();
    assert!(core.client_read_only());

    // A guard can no longer begin; after Keep the mode may go on too.
    refused("apply_guard_begin", core.apply_guard_begin("x".into(), 30));
    core.set_client_read_only(false).unwrap();
    core.apply_guard_begin("again".into(), 30).unwrap();
    assert!(core.set_client_read_only(true).is_err());
    core.apply_confirm().unwrap();
    core.set_client_read_only(true).unwrap();
    let image = remote.image();
    refused("after Keep", core.set_master_volume(-1.0));
    assert_eq!(remote.image(), image);
}

// -- WP ROMODEF -- (review romode F3, the reviewer's probe 2)
#[test]
fn wp_romodef_the_meters_do_not_clear_the_devices_clips_in_the_mode() {
    let (core, remote, log) = demo();
    core.set_client_read_only(true).unwrap();
    let n = usize::from(core.get_state().num_channels);
    core.set_meter_ballistics(crate::MeterBallistics {
        hold_ms: 1000,
        decay_per_s: 0.5,
        clip_hold_ms: 50,
    });
    let inputs = core.get_state().num_input_channels;
    let out1 = 1u32 << inputs;
    let block = |clips: u32| {
        let mut b = Vec::new();
        for _ in 0..n {
            b.extend_from_slice(&0u16.to_le_bytes());
        }
        b.extend_from_slice(&[10, 20]);
        b.extend_from_slice(&clips.to_le_bytes());
        b.push(2);
        b
    };
    remote.set_status(block(0));
    core.fetch_status().unwrap();
    remote.set_status(block(out1));
    assert_eq!(core.fetch_status().unwrap().clip_hold, out1);
    core.fetch_status().unwrap();
    std::thread::sleep(Duration::from_millis(80));
    log.lock().unwrap().clear();
    let st = core.fetch_status().unwrap();
    let clears: Vec<_> = log
        .lock()
        .unwrap()
        .iter()
        .filter(|&&(_, o, _)| o == op::REQ_CLEAR_CLIPS)
        .copied()
        .collect();
    assert_eq!(clears, [], "no clear went out in the mode");
    // The refused clear: the hold follows the device's sticky flag from the
    // next read on.
    let _ = st;
    let st = core.fetch_status().unwrap();
    assert_eq!(st.clip_hold & out1, out1, "{st:?}");
    assert_eq!(
        log.lock()
            .unwrap()
            .iter()
            .filter(|&&(_, o, _)| o == op::REQ_CLEAR_CLIPS)
            .count(),
        0
    );
    refused("clear_clips", core.clear_clips());
    refused("reset_buffer_stats", core.reset_buffer_stats());
    assert_eq!(writes(&log), []);
    // Off again: the meters clear as before.
    core.set_client_read_only(false).unwrap();
    core.clear_clips().unwrap();
}

// -- WP ROMODEF -- (review romode F2, the reviewer's hub probe)
#[cfg(feature = "link")]
#[test]
fn wp_romodef_the_audio_link_system_writes_over_the_hub_are_refused() {
    use dspi_link::fakehub::{Auth, FakeHub, HubConfig};
    let hub = FakeHub::start(HubConfig {
        auth: Auth::None,
        ..HubConfig::bridge()
    })
    .unwrap();
    let core = DspiCore::new();
    core.link_connect(
        hub.url(),
        "romodef".into(),
        "test".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    core.set_client_read_only(true).unwrap();
    let before = core.client_read_only_refusals();
    let save = core.alink_system_save(0, crate::link::alink_system::AlinkSystemEdits::default());
    let align = core.alink_system_align();
    let recall = core.alink_system_recall("x".into());
    assert_eq!(
        core.link_hub_request("system.set", serde_json::Map::new()),
        Err(DspiError::ReadOnly)
    );
    assert_eq!(
        align.map(|_| ()),
        Err(DspiError::ReadOnly),
        "system.align went out"
    );
    assert_eq!(
        recall.map(|_| ()),
        Err(DspiError::ReadOnly),
        "system.scene.apply went out"
    );
    assert_eq!(
        save.map(|_| ()),
        Err(DspiError::ReadOnly),
        "system.set went out"
    );
    assert!(core.client_read_only_refusals() >= before + 3);
    // The view only reads: it reaches the hub (which refuses an unknown type).
    let view = core.alink_system_view();
    assert!(!matches!(view, Err(DspiError::ReadOnly)), "{view:?}");
    // Off: the write reaches the hub.
    core.set_client_read_only(false).unwrap();
    assert!(!matches!(
        core.alink_system_align(),
        Err(DspiError::ReadOnly)
    ));
}
