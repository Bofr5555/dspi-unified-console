//! WP PGADAPT: the whole core on paged firmware through the adapter
//! (`dspi_transport::paged::PagedTransport`), on PGFAKE1/2's G2s and G2b
//! fakes, cross-checked against the 1.1.6 twin of the same capture (G1s,
//! G1b) opened the same way: the connect and the readout, a bulk round trip,
//! a typed write of every writable registry path, presets, the gated flash
//! save, the bootloader entry and the extension reads answer the same. The
//! fakes fail a test by themselves on any violation of PLAN-PG section 1.1
//! (a page-0 request to the paged ones, a page to the 1.1.6 ones).

use std::sync::Arc;
use std::time::Duration;

use dspi_core::{ConfirmOp, DspiCore, ParamValue};
use dspi_proto::registry::{Kind, ParamDesc, REGISTRY};
use dspi_transport::fake::FakeGeneration;
use dspi_transport::{FakeDevice, FakeRemote, Generation, Transport, paged};

const PAGED: [FakeGeneration; 2] = [FakeGeneration::STOCK117, FakeGeneration::Bofr117];

fn twin(g: FakeGeneration) -> FakeGeneration {
    match g {
        FakeGeneration::Bofr117 => FakeGeneration::Bofr116 { page_guard: true },
        _ => FakeGeneration::Stock116,
    }
}

/// A core connected to `dev` through `paged::open`, as every construction
/// site opens a device.
fn connect(dev: FakeDevice) -> (Arc<DspiCore>, FakeRemote, Option<Generation>) {
    let remote = dev.remote();
    let serial = dev.descriptor().serial.clone();
    let t = paged::open(Box::new(dev)).expect("open");
    let generation = t.generation();
    let core = DspiCore::new();
    core.open_transport(t, serial, 0)
        .expect("the connect sequence");
    settle(&core);
    (core, remote, generation)
}

fn settle(core: &DspiCore) {
    let mut quiet = 0;
    while quiet < 5 {
        std::thread::sleep(Duration::from_millis(5));
        let r = core.process_notifications();
        quiet = if r.flags == 0 { quiet + 1 } else { 0 };
    }
}

/// The paged core and its 1.1.6 twin.
fn pair(g: FakeGeneration) -> [(Arc<DspiCore>, FakeRemote); 2] {
    let (old, old_remote, old_gen) = connect(FakeDevice::demo_generation(twin(g)));
    let (new, remote, new_gen) = connect(FakeDevice::demo_generation(g));
    assert_eq!(old_gen, Some(Generation::Legacy));
    assert!(matches!(new_gen, Some(Generation::Paged(_))), "{new_gen:?}");
    [(old, old_remote), (new, remote)]
}

/// The mirror as JSON, without what differs on purpose: the firmware
/// version (1.1.7 on the paged fakes).
fn state(core: &DspiCore) -> serde_json::Value {
    let mut v = serde_json::to_value(core.get_state()).unwrap();
    if let Some(m) = v.as_object_mut() {
        m.retain(|k, _| !k.starts_with("fw"));
    }
    v
}

fn no_violation(remote: &FakeRemote) {
    assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());
    assert_eq!(remote.zero_length_gets(), 0);
}

#[test]
fn wp_pgadapt_the_connect_and_readout_match_the_116_twin() {
    for g in PAGED {
        let [(old, old_remote), (new, remote)] = pair(g);
        let s = new.get_state();
        assert_eq!((s.fw_major, s.fw_minor, s.fw_patch), (1, 1, 7), "{g:?}");
        assert_eq!(state(&new), state(&old), "{g:?}");
        assert_eq!(remote.image(), old_remote.image());
        no_violation(&remote);
        no_violation(&old_remote);
    }
}

/// The extension probe goes through the tunnel on G2b and finds what G1b's
/// `REQ_EXTENSION` finds; on G2s it finds nothing, as on G1s.
#[test]
fn wp_pgadapt_the_extension_reads_match_the_116_twin() {
    for g in PAGED {
        let [(old, _), (new, remote)] = pair(g);
        assert_eq!(
            new.ext_status().unwrap(),
            old.ext_status().unwrap(),
            "{g:?}"
        );
        assert_eq!(
            new.ext_status().unwrap().supported.is_empty(),
            g == FakeGeneration::STOCK117
        );
        assert_eq!(
            new.fetch_crossfeed_pair_map().is_ok(),
            old.fetch_crossfeed_pair_map().is_ok()
        );
        no_violation(&remote);
    }
}

/// A value of `d`'s kind inside its range (as WP CG32's test picks one).
fn sample(d: &ParamDesc) -> ParamValue {
    match d.kind {
        Kind::Bool => ParamValue::Bool(true),
        Kind::Float { min, max, .. } => {
            ParamValue::Float((min.max(0.0) * 0.5 + max * 0.5).min(max))
        }
        Kind::Int { min, .. } => ParamValue::Int(min),
        Kind::Choice(list) => ParamValue::Choice(list.last().map_or(0, |c| c.0)),
        Kind::Mask => ParamValue::Mask(1),
        Kind::Text { .. } => ParamValue::Text("pg".into()),
        Kind::Packet => ParamValue::Bytes(vec![0; 8]),
        Kind::Trigger | Kind::Status => ParamValue::Trigger,
    }
}

fn indices(d: &ParamDesc) -> Vec<u8> {
    match d.path {
        "in.spdif.enable" => vec![1],
        _ => vec![0; d.target.arity()],
    }
}

/// Paths whose command neither fake models: the 1.1.6 fake STALLs
/// `TEST_DAC_HW_MUTE`; the paged fake runs its own inverse onto that same
/// unmodelled opcode and answers the STALL as `VC_ERR_VALUE` (PGFAKE1's
/// rule for a 1.1.6-model refusal of a SET), which the `pin` status family
/// turns into status 5 (guide 5.4). Both are refusals; real firmware runs
/// the test.
const UNMODELLED: &[&str] = &["dev.dacmute.test"];

/// Every writable path of the parameter registry through `write_param`
/// answers on the paged core as on its 1.1.6 twin, and the two devices end
/// up with the same image and the same mirror.
#[test]
fn wp_pgadapt_a_typed_write_of_every_registry_entry_matches_the_116_twin() {
    for g in PAGED {
        let [(old, old_remote), (new, remote)] = pair(g);
        let mut bad = Vec::new();
        let mut written = 0;
        for d in REGISTRY.iter().filter(|d| d.is_writable()) {
            let a = old.write_param(d.path.into(), indices(d), sample(d));
            let b = new.write_param(d.path.into(), indices(d), sample(d));
            if a.is_ok() {
                written += 1;
            }
            let same = match (&a, &b) {
                (Ok(x), Ok(y)) => x == y,
                (Err(x), Err(y)) => std::mem::discriminant(x) == std::mem::discriminant(y),
                _ => false,
            };
            if !same && !UNMODELLED.contains(&d.path) {
                bad.push(format!("{}: legacy {a:?}, paged {b:?}", d.path));
            }
        }
        settle(&old);
        settle(&new);
        assert!(bad.is_empty(), "{g:?}:\n{}", bad.join("\n"));
        assert!(written > 40, "{written}");
        assert_eq!(remote.image(), old_remote.image(), "{g:?}");
        assert_eq!(state(&new), state(&old), "{g:?}");
        no_violation(&remote);
    }
}

/// A bulk round trip: capture a snapshot document, change the device,
/// recall it (the whole image written back), on both.
#[test]
fn wp_pgadapt_a_bulk_round_trip_matches_the_116_twin() {
    for g in PAGED {
        for (core, remote) in pair(g) {
            let before = remote.image();
            let doc = core
                .snapshot_capture_document("s".into(), String::new())
                .unwrap();
            core.set_master_volume(-30.0).unwrap();
            assert_ne!(remote.image(), before);
            let r = core.snapshot_recall_document(doc).unwrap();
            assert!(r.image, "{g:?}");
            settle(&core);
            assert_eq!(remote.image(), before, "{g:?}");
            no_violation(&remote);
        }
    }
}

/// Preset save and load, and the gated flash save, as on the twin.
#[test]
fn wp_pgadapt_presets_and_the_gated_save_match_the_116_twin() {
    for g in PAGED {
        let [(old, old_remote), (new, remote)] = pair(g);
        for (which, core) in [("1.1.6", &old), ("paged", &new)] {
            core.set_output_gain(0, -7.0).unwrap();
            core.save_preset_confirmed(2, Some(core.confirm_token(ConfirmOp::SavePreset)))
                .unwrap();
            settle(core);
            let saved = state(core);
            core.set_output_gain(0, -1.0).unwrap();
            settle(core);
            assert_ne!(state(core), saved, "{g:?} {which}");
            core.load_preset(2).unwrap();
            settle(core);
            assert_eq!(state(core), saved, "{g:?} {which}: the preset came back");
            core.save_params_confirmed(Some(core.confirm_token(ConfirmOp::SaveParams)))
                .unwrap();
        }
        settle(&old);
        settle(&new);
        assert_eq!(state(&new), state(&old), "{g:?}");
        assert_eq!(remote.image(), old_remote.image());
        no_violation(&remote);
    }
}

/// The gated bootloader entry: on the paged sender (the fake that models a
/// reboot) the device leaves for BOOTSEL through `SYS.ENTER_BOOTLOADER`; on
/// the paged Pico fakes the call answers as on the twin.
#[test]
fn wp_pgadapt_the_bootloader_entry_reaches_the_fakes_reboot() {
    let dev = FakeDevice::sender(true);
    let remote = dev.remote();
    let t = paged::open(Box::new(dev)).unwrap();
    assert!(matches!(t.generation(), Some(Generation::Paged(_))));
    let mut t = t;
    // The core's gated call sends exactly this (hardware.rs: `get_upto`
    // of 0xF0, one byte); through the adapter it is the paged action.
    assert_eq!(
        t.control_in_upto(dspi_proto::generated::opcodes::REQ_ENTER_BOOTLOADER, 0, 1)
            .unwrap(),
        [1]
    );
    assert_eq!(
        remote.rebooted_to(),
        Some(dspi_transport::RebootTarget::Bootsel)
    );
    no_violation(&remote);

    for g in PAGED {
        let [(old, _), (new, remote)] = pair(g);
        let a = old.enter_bootloader_confirmed(Some(old.confirm_token(ConfirmOp::EnterBootloader)));
        let b = new.enter_bootloader_confirmed(Some(new.confirm_token(ConfirmOp::EnterBootloader)));
        assert_eq!(a.is_ok(), b.is_ok(), "{g:?}: {a:?} {b:?}");
        no_violation(&remote);
    }
}
