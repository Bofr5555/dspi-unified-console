//! WP PGCORE: the core's surface for the protocol generation (PLAN-PG
//! sections 1.1, 1.2 and 1.6), on PGFAKE1/2's fakes of every generation,
//! each opened through `paged::open` as every construction site opens a
//! device: the device info shows each generation through the core's
//! wrappers, the raw-id calls meet section 1.6's gates, and a
//! `PIN_MAP_CHANGED` notification re-reads the pins. The fakes fail a test
//! by themselves on a violation of section 1.1.

use std::sync::Arc;
use std::time::Duration;

use dspi_core::{DeviceProtocol, DspiCore, DspiError, ExtEnvelope, ProtocolGeneration};
use dspi_proto::generated::ext as g;
use dspi_proto::paged::{EXT_TUNNEL, cmds};
use dspi_transport::fake::FakeGeneration;
use dspi_transport::{FakeDevice, FakeRemote, Transport, paged};

/// A core connected to the demo fake of `generation` through `paged::open`.
fn connect(generation: FakeGeneration) -> (Arc<DspiCore>, FakeRemote) {
    let dev = FakeDevice::demo_generation(generation);
    let remote = dev.remote();
    let serial = dev.descriptor().serial.clone();
    let t = paged::open(Box::new(dev)).expect("open");
    let core = DspiCore::new();
    core.open_transport(t, serial, 0)
        .expect("the connect sequence");
    settle(&core);
    (core, remote)
}

fn settle(core: &DspiCore) {
    let mut quiet = 0;
    while quiet < 5 {
        std::thread::sleep(Duration::from_millis(5));
        let r = core.process_notifications();
        quiet = if r.flags == 0 { quiet + 1 } else { 0 };
    }
}

fn no_violation(remote: &FakeRemote) {
    assert_eq!(remote.violations(), 0, "{:?}", remote.last_violation());
}

#[test]
fn wp_pgcore_the_device_info_shows_each_generation() {
    for generation in FakeGeneration::ALL {
        let (core, remote) = connect(generation);
        let p: DeviceProtocol = core.device_protocol().unwrap();
        let ext = generation.has_extensions();
        if generation.is_paged() {
            assert_eq!(p.generation, ProtocolGeneration::Paged, "{generation:?}");
            assert_eq!(p.protocol_major, 1, "{generation:?}");
            assert!(
                p.pages
                    .iter()
                    .any(|pg| pg.page == cmds::SYS_PLATFORM.page()),
                "{generation:?}: {p:?}"
            );
            assert_eq!(
                p.pages.iter().any(|pg| pg.page == 0xE0),
                ext,
                "{generation:?}: page 0xE0 only on our port"
            );
            let want = if ext {
                ExtEnvelope::Tunnel
            } else {
                ExtEnvelope::NoChannel
            };
            assert_eq!(p.ext_envelope, want, "{generation:?}");
        } else {
            assert_eq!(p.generation, ProtocolGeneration::Legacy, "{generation:?}");
            assert_eq!((p.protocol_major, p.protocol_minor), (0, 0));
            assert!(p.pages.is_empty());
            let want = if ext {
                ExtEnvelope::Legacy
            } else {
                ExtEnvelope::NoChannel
            };
            assert_eq!(p.ext_envelope, want, "{generation:?}");
        }
        no_violation(&remote);
        core.disconnect();
        assert_eq!(core.device_protocol(), Err(DspiError::Disconnected));
    }
}

fn volume(db: f32) -> Vec<u8> {
    db.to_le_bytes().to_vec()
}

#[test]
fn wp_pgcore_the_raw_id_calls_meet_the_gates_on_paged_firmware() {
    for generation in [FakeGeneration::STOCK117, FakeGeneration::Bofr117] {
        let (core, remote) = connect(generation);
        let id = cmds::ROUTE_MASTER_VOLUME.0;
        // A value SET with a 1.1.6 equivalent goes out and reads back.
        core.raw_set_id(id, 0, volume(-6.0)).unwrap();
        let back = core.raw_get_id(id, 0, 4).unwrap();
        assert_eq!(back, volume(-6.0), "{generation:?}");

        let sent = remote.paged_requests();
        let gated = |what: &str, r: Result<(), DspiError>| {
            assert!(
                matches!(r, Err(DspiError::InvalidArgument { .. })),
                "{generation:?} {what}: {r:?}"
            );
            assert_eq!(remote.paged_requests(), sent, "{what}: nothing sent");
        };
        // The bootloader entry by its paged id, the flash save, the bulk
        // and a core-voltage request through the tunnel: nothing is sent.
        gated(
            "SYS.ENTER_BOOTLOADER",
            core.raw_set_id(cmds::SYS_ENTER_BOOTLOADER.0, 0, Vec::new()),
        );
        gated(
            "STORE.PRESET_SAVE",
            core.raw_set_id(cmds::STORE_PRESET_SAVE.0, 1, Vec::new()),
        );
        gated(
            "STORE.ALL_PARAMS",
            core.raw_set_id(cmds::STORE_ALL_PARAMS.0, 0, vec![0; 8]),
        );
        gated(
            "EXT_CORE_VREG through the tunnel",
            core.raw_set_id(EXT_TUNNEL.0, g::EXT_CORE_VREG, vec![0x10]),
        );
        // A page-0 id is the 1.1.6 calls'; a GET of 0 bytes is never sent.
        gated("page 0", core.raw_set_id(0x0050, 0, vec![1]));
        assert!(matches!(
            core.raw_get_id(0x00D2, 0, 4),
            Err(DspiError::InvalidArgument { .. })
        ));
        assert!(matches!(
            core.raw_get_id(id, 0, 0),
            Err(DspiError::InvalidArgument { .. })
        ));
        assert_eq!(remote.paged_requests(), sent);

        // The client read-only mode: a raw GET passes, a raw SET does not.
        core.set_client_read_only(true).unwrap();
        assert_eq!(core.raw_get_id(id, 0, 4).unwrap(), volume(-6.0));
        let read = remote.paged_requests();
        assert_eq!(
            core.raw_set_id(id, 0, volume(-3.0)),
            Err(DspiError::ReadOnly)
        );
        assert_eq!(remote.paged_requests(), read, "nothing sent");
        core.set_client_read_only(false).unwrap();
        assert_eq!(core.raw_get_id(id, 0, 4).unwrap(), volume(-6.0));
        no_violation(&remote);
    }
}

#[test]
fn wp_pgcore_a_raw_id_call_to_a_legacy_device_sends_nothing() {
    for generation in [
        FakeGeneration::Stock116,
        FakeGeneration::Bofr116 { page_guard: true },
    ] {
        let (core, remote) = connect(generation);
        // The probe of `paged::open` is the one paged request it ever gets
        assert_eq!(remote.paged_requests(), 1);
        let id = cmds::ROUTE_MASTER_VOLUME.0;
        assert_eq!(core.raw_get_id(id, 0, 4), Err(DspiError::Unsupported));
        assert_eq!(
            core.raw_set_id(id, 0, volume(-6.0)),
            Err(DspiError::Unsupported)
        );
        assert_eq!(remote.paged_requests(), 1, "{generation:?}: no page sent");
        no_violation(&remote);
    }
}

/// `NOTIFY_EVT_PIN_MAP_CHANGED` with pin map generation `n`.
fn pin_map_changed(n: u32) -> Vec<u8> {
    let mut p = vec![2, 0x11, 0, 0];
    p.extend_from_slice(&n.to_le_bytes());
    p
}

#[test]
fn wp_pgcore_a_pin_map_change_re_reads_the_pins() {
    let (core, remote) = connect(FakeGeneration::STOCK117);
    let before = remote.paged_requests();
    remote.push(pin_map_changed(7));
    let mut flags = 0;
    for _ in 0..200 {
        std::thread::sleep(Duration::from_millis(5));
        flags |= core.process_notifications().flags;
        if flags & dspi_core::consts::NOTIFY_REFRESHED != 0 {
            break;
        }
    }
    assert_ne!(flags & dspi_core::consts::NOTIFY_REFRESHED, 0, "re-read");
    assert!(
        remote.paged_requests() > before,
        "the image and the pins were read again"
    );
    // Another unknown event is only logged.
    settle(&core);
    let quiet = remote.paged_requests();
    remote.push(vec![2, 0x12, 0, 0, 0, 0, 0, 0]);
    settle(&core);
    assert_eq!(remote.paged_requests(), quiet, "no re-read for 0x12");
    no_violation(&remote);
}
