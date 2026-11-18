//! WP PGADAPT: the adapter on PGFAKE1/2's paged fakes (G2s `Stock117`, G2b
//! `Bofr117`), whose violations fail a test by themselves, cross-checked
//! against the 1.1.6 fake (`Stock116`, `Bofr116`) of the same capture: the
//! same 1.1.6-shaped call gives the same answer through the planner (PGPROTO's
//! rows, PGCUST's recipes) and the paged fake's own inverse and custom
//! handlers.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use dspi_proto::generated::{ext as gext, opcodes as op};
use dspi_proto::paged::legacy::{
    self as lm, Adapter, LEGACY_OPS, OldDir, VC_ERR_ADDRESS, VC_ERR_BUSY, VC_ERR_LENGTH,
    VC_ERR_UNKNOWN, VC_ERR_VALUE,
};
use dspi_proto::paged::{EXT_PAGE, EXT_TUNNEL, cmds, registry};

use super::*;
use crate::fake::{FakeGeneration, FakeOptions};
use crate::mock::Reply;
use crate::{FakeDevice, FakeRemote, MockTransport};

const PAGED: [FakeGeneration; 2] = [FakeGeneration::STOCK117, FakeGeneration::Bofr117];

/// The 1.1.6 twin of a paged generation: the same capture and extensions.
fn twin(g: FakeGeneration) -> FakeGeneration {
    match g {
        FakeGeneration::Bofr117 => FakeGeneration::Bofr116 { page_guard: true },
        _ => FakeGeneration::Stock116,
    }
}

/// A paged demo fake through `open`, with its remote.
fn paged(g: FakeGeneration) -> (Box<dyn Transport>, FakeRemote) {
    let dev = FakeDevice::demo_generation(g);
    let remote = dev.remote();
    (open(Box::new(dev)).unwrap(), remote)
}

/// The 1.1.6 twin through `open`.
fn legacy(g: FakeGeneration) -> (Box<dyn Transport>, FakeRemote) {
    let dev = FakeDevice::demo_generation(twin(g));
    let remote = dev.remote();
    (open(Box::new(dev)).unwrap(), remote)
}

/// A fake with options changed before it is opened.
fn paged_with(g: FakeGeneration, f: impl FnOnce(&FakeRemote)) -> (Box<dyn Transport>, FakeRemote) {
    let dev = FakeDevice::demo_generation(g);
    let remote = dev.remote();
    f(&remote);
    (open(Box::new(dev)).unwrap(), remote)
}

/// Same answer: equal bytes, or both refused.
fn same(what: &str, a: &Result<Vec<u8>>, b: &Result<Vec<u8>>) {
    match (a, b) {
        (Ok(x), Ok(y)) => assert_eq!(x, y, "{what}"),
        (Err(_), Err(_)) => {}
        _ => panic!("{what}: legacy {a:?}, paged {b:?}"),
    }
}

/// The opcode whose IN (`get`) or OUT form is a `direct` row onto `id`.
fn direct_onto(id: CmdId, get: bool) -> u8 {
    LEGACY_OPS
        .iter()
        .find(|o| {
            let a = if get { o.r#in } else { o.out };
            matches!(a, Some(Adapter::Direct(d)) if d.target == id)
        })
        .map(|o| o.code)
        .unwrap()
}

// ---------------------------------------------------------------- open

#[test]
fn wp_pgadapt_open_makes_a_paged_transport_with_the_discovery() {
    for g in PAGED {
        let mut dev = FakeDevice::demo_generation(g);
        let remote = dev.remote();
        let probe = dev.control_in_paged(SYS_PROTOCOL_INFO, 0, 16).unwrap();
        let ProbeReply::Paged(info) = ProtocolInfo::decode(&probe) else {
            panic!("{probe:?}")
        };
        let raw: Box<dyn Transport> = Box::new(dev);
        let t = PagedTransport::new(raw, info).unwrap();
        let d = t.discovery();
        assert_eq!(d.info, info);
        assert!(d.pages.has_page(0x01) && d.pages.has_page(0x05), "{g:?}");
        assert!(
            !d.pages.has_page(0xF0),
            "{g:?}: a release build has no DIAG"
        );
        assert_eq!(d.has_extensions(), g == FakeGeneration::Bofr117, "{g:?}");
        assert_eq!(d.pages.has_page(EXT_PAGE), g == FakeGeneration::Bofr117);
        assert!(d.platform.len() >= 7, "{:?}", d.platform);
        assert!(d.pages.revision_is_vendored(0x05));
        assert_eq!(t.generation(), Some(Generation::Paged(info)));
        // The discovery is one batch: the platform, the page list and the
        // bitmaps of every page this build knows plus 0xE0.
        let known = registry::PAGES.len() + 1;
        assert_eq!(
            remote.paged_requests() as usize,
            1 + 2 + known,
            "{g:?}: the probe, then one batch"
        );
        assert_eq!(remote.violations(), 0);
    }
}

/// The discovery goes to the wire as one `control_in_batch_paged` (R3).
#[test]
fn wp_pgadapt_the_discovery_is_one_paged_batch() {
    let (dev, batches) = Batching::new(FakeDevice::demo_generation(FakeGeneration::STOCK117));
    let t = open(Box::new(dev)).unwrap();
    assert!(matches!(t.generation(), Some(Generation::Paged(_))));
    let b = batches.lock().unwrap().clone();
    assert_eq!(b.len(), 1, "{b:?}");
    assert_eq!(b[0], registry::PAGES.len() + 1 + 2);
}

/// A device that answers the probe but whose page list names no `SYS`
/// page, that refuses `SYS.PLATFORM`, or whose `SYS` bitmap lacks the probe,
/// is unrecognised; no page-0 request reaches it.
#[test]
fn wp_pgadapt_a_discovery_that_does_not_hold_up_is_unrecognised() {
    let probe = vec![
        0x44, 0x53, 0x50, 0x69, 0x01, 0x00, 0x0C, 0x00, 0x40, 0x00, 0x00, 0x01, 0, 0, 0, 0,
    ];
    let cases = [
        // No SYS page listed.
        MockTransport::new()
            .carrying_pages()
            .paged(SYS_PROTOCOL_INFO, Reply::Data(probe.clone()))
            .paged(
                cmds::SYS_PLATFORM,
                Reply::Data(vec![1, 1, 0x17, 9, 1, 7, 1]),
            )
            .paged(cmds::SYS_PAGE_LIST, Reply::Data(vec![0x05, 1])),
        // No platform.
        MockTransport::new()
            .carrying_pages()
            .paged(SYS_PROTOCOL_INFO, Reply::Data(probe.clone()))
            .paged(cmds::SYS_PAGE_LIST, Reply::Data(vec![0x01, 1, 0x05, 1])),
        // SYS listed, but its bitmap does not list the probe it answered.
        MockTransport::new()
            .carrying_pages()
            .paged(SYS_PROTOCOL_INFO, Reply::Data(probe.clone()))
            .paged(
                cmds::SYS_PLATFORM,
                Reply::Data(vec![1, 1, 0x17, 9, 1, 7, 1]),
            )
            .paged(cmds::SYS_PAGE_LIST, Reply::Data(vec![0x01, 1]))
            .paged(cmds::SYS_PAGE_COMMANDS, Reply::Data(vec![0; 64])),
    ];
    for t in cases {
        let log = t.log_handle();
        let e = open(Box::new(t)).err().unwrap();
        assert_eq!(Refusal::of(&e), Some(Refusal::Unrecognised), "{e:?}");
        assert!(log.lock().unwrap().iter().all(|x| x.page() != 0));
    }
    // A disconnect during the discovery is the disconnect.
    let t = MockTransport::new()
        .carrying_pages()
        .paged(SYS_PROTOCOL_INFO, Reply::Data(probe))
        .paged(cmds::SYS_PAGE_LIST, Reply::Disconnect);
    assert!(matches!(
        open(Box::new(t)).err(),
        Some(TransportError::Disconnected)
    ));
}

// ---------------------------------------------------------------- 1.1.6 calls

/// Every 1.1.6 read (`old_dir` R) at a few `wValue`s answers through the
/// adapter as the 1.1.6 twin answers it: PGPROTO's direct and split rows,
/// PGCUST's custom recipes (`GET_STATUS`, `GET_EQ_PARAM`,
/// `GET_MATRIX_ROUTE`, `PRESET_GET_STARTUP`, `GET_CS_DISPLAY_CFG`, ...)
/// against PGFAKE1's inverse and PGFAKE2's custom handlers.
#[test]
fn wp_pgadapt_every_116_read_answers_as_on_the_116_twin() {
    for g in PAGED {
        let (mut old, _) = legacy(g);
        let (mut new, remote) = paged(g);
        let mut compared = 0;
        let mut bad = Vec::new();
        for o in LEGACY_OPS.iter().filter(|o| o.old_dir == OldDir::Read) {
            if SKIP_READ.contains(&o.code) {
                continue;
            }
            for value in [0u16, 1, 2] {
                let a = old.control_in_upto(o.code, value, 64);
                let b = new.control_in_upto(o.code, value, 64);
                compared += 1;
                let ok = match (&a, &b) {
                    (Ok(x), Ok(y)) => x == y,
                    (Err(_), Err(_)) => true,
                    // The 1.1.6 fake refuses a `wValue` the 1.1.6 firmware
                    // ignored where 1.1.7 addresses nothing: the planner
                    // sends 0 there (guide 5.5), so the paged answer is the
                    // firmware's 1.1.6 answer.
                    (Err(_), Ok(_)) => value != 0,
                    (Ok(_), Err(_)) => false,
                };
                if !ok {
                    bad.push(format!(
                        "{} wValue {value}: legacy {a:?}, paged {b:?}",
                        o.name
                    ));
                }
            }
        }
        assert!(bad.is_empty(), "{g:?}:\n{}", bad.join("\n"));
        assert!(compared > 250, "{compared}");
        assert_eq!(remote.violations(), 0);
        assert_eq!(remote.zero_length_gets(), 0);
    }
}

/// Reads whose answer is not the same on purpose: the firmware version in
/// `GET_PLATFORM` and `GET_BUILD_INFO` (1.1.7 on the paged fakes), the
/// bulk image (its own test), and the live counters that move between
/// two reads of two devices.
const SKIP_READ: &[u8] = &[
    op::REQ_GET_PLATFORM,
    op::REQ_GET_BUILD_INFO,
    op::REQ_GET_ALL_PARAMS,
    op::REQ_GET_ALL_PARAMS_CHUNK,
];

/// The connect's identity reads: `GET_PLATFORM` comes from the discovery
/// (no request), 8 bytes asked; the version is 1.1.7's.
#[test]
fn wp_pgadapt_get_platform_is_answered_from_the_discovery() {
    let (mut t, remote) = paged(FakeGeneration::STOCK117);
    let before = remote.paged_requests();
    let p = t
        .control_in_upto(op::REQ_GET_PLATFORM, 0, PLATFORM_ASK)
        .unwrap();
    assert_eq!(p.get(1..3), Some(&[1, 0x17][..]), "{p:?}");
    assert_eq!(t.control_in(op::REQ_GET_PLATFORM, 0, 4).unwrap(), p[..4]);
    assert_eq!(remote.paged_requests(), before, "nothing on the wire");
}

/// The bulk image both ways: the single-shot and the chunked form, then a
/// write of the whole image (custom `SET_ALL_PARAMS`), read back equal.
#[test]
fn wp_pgadapt_the_bulk_image_round_trips() {
    for g in PAGED {
        let (mut old, _) = legacy(g);
        let (mut new, remote) = paged(g);
        let a = old.control_in(op::REQ_GET_ALL_PARAMS, 0, 6136).unwrap();
        let b = new.control_in(op::REQ_GET_ALL_PARAMS, 0, 6136).unwrap();
        assert_eq!(a, b, "{g:?}");
        let mut chunked = Vec::new();
        while chunked.len() < 6136 {
            let at = u16::try_from(chunked.len()).unwrap();
            let want = (6136 - chunked.len()).min(4096) as u16;
            chunked.extend(
                new.control_in(op::REQ_GET_ALL_PARAMS_CHUNK, at, want)
                    .unwrap(),
            );
        }
        assert_eq!(chunked, b);
        // Change a byte the device keeps (the master volume is in the
        // image), write it whole, read it back.
        let mut image = remote.image();
        let before = image.clone();
        new.control_out(op::REQ_SET_ALL_PARAMS, 0, &image).unwrap();
        assert_eq!(remote.image(), before, "{g:?}: the same image");
        image = new.control_in(op::REQ_GET_ALL_PARAMS, 0, 6136).unwrap();
        assert_eq!(image, before);
        assert_eq!(remote.violations(), 0);
    }
}

/// Writes through the adapter leave the device as on the 1.1.6 twin: a
/// direct row (master volume), the custom ones (`SET_EQ_PARAM`,
/// `SET_MATRIX_ROUTE`, `SET_PREAMP`), a former write-as-read (`SET_BYPASS`'s
/// sibling, `CLEAR_CLIPS`).
#[test]
fn wp_pgadapt_writes_leave_the_same_device_as_on_the_116_twin() {
    for g in PAGED {
        let (mut old, old_remote) = legacy(g);
        let (mut new, remote) = paged(g);
        let master = (-12.5f32).to_le_bytes();
        let mut eq = vec![0u8, 1, 0, 0];
        eq.extend((1000f32).to_le_bytes());
        eq.extend((0.7f32).to_le_bytes());
        eq.extend((3.0f32).to_le_bytes());
        let mut route = vec![0u8, 1, 1, 0];
        route.extend((-3.0f32).to_le_bytes());
        let writes: Vec<(u8, u16, Vec<u8>)> = vec![
            (op::REQ_SET_MASTER_VOLUME, 0, master.to_vec()),
            (op::REQ_SET_PREAMP, 0, (-2.0f32).to_le_bytes().to_vec()),
            (op::REQ_SET_DELAY, 1, (1.5f32).to_le_bytes().to_vec()),
            (op::REQ_SET_MATRIX_ROUTE, 0, route.clone()),
        ];
        for (opcode, value, data) in &writes {
            let a = old.control_out(*opcode, *value, data);
            let b = new.control_out(*opcode, *value, data);
            assert_eq!(a.is_ok(), b.is_ok(), "{g:?} 0x{opcode:02X}: {a:?} {b:?}");
        }
        assert_eq!(remote.image(), old_remote.image(), "{g:?}");
        // CLEAR_CLIPS (custom, a write-as-read): the status byte.
        let a = old.control_in(op::REQ_CLEAR_CLIPS, 0, 1);
        let b = new.control_in(op::REQ_CLEAR_CLIPS, 0, 1);
        same(&format!("{g:?} CLEAR_CLIPS"), &a, &b);
        assert_eq!(remote.violations(), 0);
    }
}

/// Preset save and load, the gated flash save (`SAVE_PARAMS` becomes
/// `STORE.PRESET_ACTIVE` then `STORE.PRESET_SAVE`), each answered as the
/// 1.1.6 twin answers.
#[test]
fn wp_pgadapt_presets_and_the_flash_save_answer_as_on_the_116_twin() {
    for g in PAGED {
        let (mut old, _) = legacy(g);
        let (mut new, remote) = paged(g);
        for (opcode, value) in [
            (op::REQ_PRESET_SAVE, 1u16),
            (op::REQ_PRESET_LOAD, 1),
            (op::REQ_SAVE_PARAMS, 0),
            (op::REQ_PRESET_GET_ACTIVE, 0),
        ] {
            let a = old.control_in(opcode, value, 1);
            let b = new.control_in(opcode, value, 1);
            same(&format!("{g:?} 0x{opcode:02X}"), &a, &b);
        }
        assert_eq!(remote.violations(), 0);
    }
}

/// The bootloader entry: `0xF0` becomes `SYS.ENTER_BOOTLOADER`, a
/// zero-length SET, and its 1.1.6 status byte comes back. On the paged
/// sender (PGFAKE2's dual-stack board, which models the reboot) the fake
/// leaves for BOOTSEL; on the Pico fakes, whose 1.1.6 model has no loader,
/// it answers as the 1.1.6 twin.
#[test]
fn wp_pgadapt_the_bootloader_entry_reaches_the_fakes_reboot() {
    let dev = FakeDevice::sender(true);
    let remote = dev.remote();
    let mut t = open(Box::new(dev)).unwrap();
    assert!(matches!(t.generation(), Some(Generation::Paged(_))));
    assert_eq!(t.control_in(op::REQ_ENTER_BOOTLOADER, 0, 1).unwrap(), [1]);
    assert_eq!(remote.rebooted_to(), Some(crate::RebootTarget::Bootsel));
    assert_eq!(remote.violations(), 0);
    for g in PAGED {
        let (mut old, _) = legacy(g);
        let (mut new, remote) = paged(g);
        let a = old.control_in(op::REQ_ENTER_BOOTLOADER, 0, 1);
        let b = new.control_in(op::REQ_ENTER_BOOTLOADER, 0, 1);
        same(&format!("{g:?} 0xF0"), &a, &b);
        assert_eq!(remote.violations(), 0);
    }
}

/// G2b: the extension reads go through the tunnel and answer as on G1b; G2s
/// has no channel and the refusal is local.
#[test]
fn wp_pgadapt_extension_reads_tunnel_on_g2b_and_are_local_stalls_on_g2s() {
    let (mut old, _) = legacy(FakeGeneration::Bofr117);
    let (mut new, remote) = paged(FakeGeneration::Bofr117);
    let list_old = old
        .control_in_upto(op::REQ_EXTENSION, gext::EXT_LIST, 64)
        .unwrap();
    let list_new = new
        .control_in_upto(op::REQ_EXTENSION, gext::EXT_LIST, 64)
        .unwrap();
    assert_eq!(list_old, list_new);
    assert!(!list_new.is_empty());
    assert_eq!(remote.violations(), 0);

    let (mut stock, remote) = paged(FakeGeneration::STOCK117);
    let before = remote.paged_requests();
    let r = stock.control_in_upto(op::REQ_EXTENSION, gext::EXT_LIST, 64);
    assert!(
        matches!(r, Err(TransportError::Stalled { opcode: 0 })),
        "{r:?}"
    );
    assert_eq!(remote.paged_requests(), before, "nothing on the wire");
}

// ---------------------------------------------------------------- local refusals

/// A command the device does not list is refused locally: as a STALL to a
/// 1.1.6-shaped call, as `Refused { VC_ERR_UNKNOWN }` to a paged one; nothing
/// on the wire either way.
#[test]
fn wp_pgadapt_absent_commands_are_refused_locally() {
    let get = direct_onto(cmds::ROUTE_MASTER_VOLUME, true);
    let set = direct_onto(cmds::ROUTE_MASTER_VOLUME, false);
    for g in PAGED {
        let (mut t, remote) = paged_with(g, |r| r.hide_paged(cmds::ROUTE_MASTER_VOLUME));
        let before = remote.paged_requests();
        assert!(matches!(
            t.control_in(get, 0, 4),
            Err(TransportError::Stalled { opcode }) if opcode == get
        ));
        assert!(matches!(
            t.control_out(set, 0, &(-3f32).to_le_bytes()),
            Err(TransportError::Stalled { .. }) | Ok(())
        ));
        assert!(matches!(
            t.control_in_paged(cmds::ROUTE_MASTER_VOLUME, 0, 4),
            Err(TransportError::Refused { id, result }) if id == cmds::ROUTE_MASTER_VOLUME && result == VC_ERR_UNKNOWN
        ));
        // A direction the command does not have.
        assert!(matches!(
            t.control_out_paged(cmds::SYS_SERIAL, 0, &[1]),
            Err(TransportError::Refused { result, .. }) if result == lm::VC_ERR_DIRECTION
        ));
        assert_eq!(
            remote.paged_requests(),
            before,
            "{g:?}: nothing on the wire"
        );
    }
}

/// Never sent: a GET with `wLength` 0, a page-0 id, a NaN or an infinity
/// in a field the registry types `f32`, a SET of the wrong length at the
/// vendored revision.
#[test]
fn wp_pgadapt_never_sent_locally_refused() {
    for g in PAGED {
        let (mut t, remote) = paged(g);
        let before = remote.paged_requests();
        assert!(matches!(
            t.control_in_paged(cmds::SYS_SERIAL, 0, 0),
            Err(TransportError::Refused { result, .. }) if result == VC_ERR_LENGTH
        ));
        assert!(matches!(
            t.control_in_paged(CmdId::new(0, op::REQ_GET_SERIAL), 0, 16),
            Err(TransportError::Incompatible(_))
        ));
        assert!(matches!(
            t.control_out_paged(CmdId::new(0, op::REQ_SET_MASTER_VOLUME), 0, &[0; 4]),
            Err(TransportError::Incompatible(_))
        ));
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(matches!(
                t.control_out_paged(cmds::ROUTE_MASTER_VOLUME, 0, &bad.to_le_bytes()),
                Err(TransportError::Refused { result, .. }) if result == VC_ERR_VALUE
            ));
            let mut band = [0u8; 16];
            band[12..].copy_from_slice(&bad.to_le_bytes());
            assert!(matches!(
                t.control_out_paged(cmds::FILTER_BAND, 0, &band),
                Err(TransportError::Refused { result, .. }) if result == VC_ERR_VALUE
            ));
            // The 1.1.6 call with a NaN: answered as 1.1.6 would (an ACK
            // or a STALL), with nothing sent.
            let _ = t.control_out(op::REQ_SET_MASTER_VOLUME, 0, &bad.to_le_bytes());
        }
        assert!(matches!(
            t.control_out_paged(cmds::ROUTE_MASTER_VOLUME, 0, &[0; 6]),
            Err(TransportError::Refused { result, .. }) if result == VC_ERR_LENGTH
        ));
        assert_eq!(
            remote.paged_requests(),
            before,
            "{g:?}: nothing on the wire"
        );
        // A finite value goes.
        t.control_out_paged(cmds::ROUTE_MASTER_VOLUME, 0, &(-6f32).to_le_bytes())
            .unwrap();
        assert_eq!(remote.paged_requests(), before + 1);
    }
}

/// R8b: a page above its vendored revision may have grown a payload, so a
/// longer SET is sent and the device judges (the fake accepts it).
#[test]
fn wp_pgadapt_a_higher_page_revision_takes_a_longer_set() {
    let vendored = registry::page(cmds::ROUTE_MASTER_VOLUME.page())
        .unwrap()
        .revision;
    let (mut t, remote) = paged(FakeGeneration::Stock117 {
        revision: vendored + 1,
    });
    let mut long = (-6f32).to_le_bytes().to_vec();
    long.extend([0, 0]);
    let before = remote.paged_requests();
    t.control_out_paged(cmds::ROUTE_MASTER_VOLUME, 0, &long)
        .unwrap();
    assert_eq!(remote.paged_requests(), before + 1);
    // The same SET at the vendored revision is refused before the wire.
    let (mut t, remote) = paged(FakeGeneration::STOCK117);
    let before = remote.paged_requests();
    assert!(
        t.control_out_paged(cmds::ROUTE_MASTER_VOLUME, 0, &long)
            .is_err()
    );
    assert_eq!(remote.paged_requests(), before);
}

// ---------------------------------------------------------------- LAST_RESULT

/// A paged STALL whose `SYS.LAST_RESULT` matches (id, direction, `wValue`)
/// is `Refused` with the device's code; the read is one request.
#[test]
fn wp_pgadapt_a_matching_last_result_types_the_refusal() {
    for g in PAGED {
        let (mut t, remote) = paged(g);
        let before = remote.paged_requests();
        let r = t.control_out_paged(cmds::ROUTE_OUTPUT_GAIN, 99, &(-1f32).to_le_bytes());
        assert!(
            matches!(r, Err(TransportError::Refused { id, result }) if id == cmds::ROUTE_OUTPUT_GAIN && result == VC_ERR_ADDRESS),
            "{r:?}"
        );
        assert_eq!(
            remote.paged_requests(),
            before + 2,
            "the SET and LAST_RESULT"
        );
        let r = t.control_in_paged(cmds::ROUTE_OUTPUT_GAIN, 99, 4);
        assert!(
            matches!(r, Err(TransportError::Refused { result, .. }) if result == VC_ERR_ADDRESS),
            "{r:?}"
        );
        assert_eq!(
            r.unwrap_err().to_string(),
            "the device refused command 0x0521: VC_ERR_ADDRESS"
        );
    }
}

/// When another request lands between the STALL and the read (the record is
/// per transport, not per host), the record does not match: a plain STALL.
#[test]
fn wp_pgadapt_a_non_matching_last_result_is_a_plain_stall() {
    let dev = FakeDevice::demo_generation(FakeGeneration::STOCK117);
    let remote = dev.remote();
    let interloper = Interloper {
        inner: dev,
        armed: Arc::new(AtomicBool::new(false)),
    };
    let armed = Arc::clone(&interloper.armed);
    let mut t = open(Box::new(interloper)).unwrap();
    armed.store(true, Ordering::SeqCst);
    let r = t.control_out_paged(cmds::ROUTE_OUTPUT_GAIN, 99, &(-1f32).to_le_bytes());
    assert!(
        matches!(r, Err(TransportError::Stalled { opcode }) if opcode == cmds::ROUTE_OUTPUT_GAIN.cmd()),
        "{r:?}"
    );
    assert_eq!(remote.violations(), 0);
}

/// A 1.1.6-shaped write-as-read whose status byte depends on the code:
/// the planner gets it from the matching record (`PRESET_LOAD` of an
/// empty or out-of-range slot answers its 1.1.6 status byte, not a STALL).
#[test]
fn wp_pgadapt_a_status_byte_comes_from_the_matching_record() {
    for g in PAGED {
        let (mut old, _) = legacy(g);
        let (mut new, _) = paged(g);
        for slot in [0u16, 7, 200] {
            let a = old.control_in(op::REQ_PRESET_LOAD, slot, 1);
            let b = new.control_in(op::REQ_PRESET_LOAD, slot, 1);
            same(&format!("{g:?} PRESET_LOAD {slot}"), &a, &b);
        }
    }
}

/// `VC_ERR_BUSY` (the request did not run) is retried; any other refusal is
/// not.
#[test]
fn wp_pgadapt_only_busy_is_retried() {
    let dev = FakeDevice::demo_generation(FakeGeneration::STOCK117);
    let busy = Busy {
        inner: dev,
        left: Arc::new(AtomicU32::new(0)),
        code: Arc::new(AtomicU32::new(u32::from(VC_ERR_BUSY))),
    };
    let (left, code) = (Arc::clone(&busy.left), Arc::clone(&busy.code));
    let mut t = open(Box::new(busy)).unwrap();
    left.store(2, Ordering::SeqCst);
    t.control_out_paged(cmds::ROUTE_MASTER_VOLUME, 0, &(-6f32).to_le_bytes())
        .unwrap();
    assert_eq!(left.load(Ordering::SeqCst), 0);
    code.store(u32::from(VC_ERR_VALUE), Ordering::SeqCst);
    left.store(2, Ordering::SeqCst);
    let r = t.control_out_paged(cmds::ROUTE_MASTER_VOLUME, 0, &(-6f32).to_le_bytes());
    assert!(matches!(r, Err(TransportError::Refused { result, .. }) if result == VC_ERR_VALUE));
    assert_eq!(left.load(Ordering::SeqCst), 1, "sent once");
}

// ---------------------------------------------------------------- batches

/// R3: a batch of single-step plans is one paged batch on the wire; the
/// answers are those of the reads one by one.
#[test]
fn wp_pgadapt_a_batch_of_single_step_plans_is_one_paged_batch() {
    let (dev, batches) = Batching::new(FakeDevice::demo_generation(FakeGeneration::STOCK117));
    let mut t = open(Box::new(dev)).unwrap();
    batches.lock().unwrap().clear();
    let reads = [
        InRead::exact(op::REQ_GET_MASTER_VOLUME, 0, 4),
        InRead::exact(op::REQ_GET_OUTPUT_GAIN, 1, 4),
        InRead::upto(op::REQ_GET_SERIAL, 0, 16),
        InRead::exact(op::REQ_GET_STATUS, 9, 4),
        InRead::upto(op::REQ_GET_PLATFORM, 0, 8),
        InRead::exact(op::REQ_EXTENSION, gext::EXT_LIST, 4),
    ];
    let one_by_one: Vec<Result<Vec<u8>>> = reads
        .iter()
        .map(|r| {
            if r.exact {
                t.control_in(r.opcode, r.value, r.len)
            } else {
                t.control_in_upto(r.opcode, r.value, r.len)
            }
        })
        .collect();
    batches.lock().unwrap().clear();
    let batched = t.control_in_batch(&reads);
    let b = batches.lock().unwrap().clone();
    // Master volume, output gain, serial; GET_STATUS 9 is PGCUST's one-step
    // recipe; GET_PLATFORM is the discovery's; the extension is a local STALL.
    assert_eq!(b, vec![4], "{b:?}");
    for (i, (a, b)) in one_by_one.iter().zip(&batched).enumerate() {
        same(&format!("read {i}"), a, b);
    }
    assert!(matches!(
        batched[5],
        Err(TransportError::Stalled { opcode: 0 })
    ));
}

/// The paged batch: local refusals answered here, the rest in one batch.
#[test]
fn wp_pgadapt_the_paged_batch_checks_each_read() {
    let (dev, batches) = Batching::new(FakeDevice::demo_generation(FakeGeneration::STOCK117));
    let mut t = open(Box::new(dev)).unwrap();
    batches.lock().unwrap().clear();
    let r = t.control_in_batch_paged(&[
        PagedRead::new(cmds::ROUTE_MASTER_VOLUME, 0, 4),
        PagedRead::new(cmds::SYS_SERIAL, 0, 0),
        PagedRead::new(CmdId::new(0, op::REQ_GET_SERIAL), 0, 16),
        PagedRead::new(EXT_TUNNEL, gext::EXT_LIST, 16),
        PagedRead::new(cmds::ROUTE_OUTPUT_GAIN, 99, 4),
    ]);
    assert_eq!(r[0].as_ref().unwrap().len(), 4);
    assert!(matches!(r[1], Err(TransportError::Refused { result, .. }) if result == VC_ERR_LENGTH));
    assert!(matches!(r[2], Err(TransportError::Incompatible(_))));
    assert!(
        matches!(r[3], Err(TransportError::Refused { result, .. }) if result == VC_ERR_UNKNOWN)
    );
    assert!(matches!(r[4], Err(TransportError::Stalled { .. })));
    assert_eq!(*batches.lock().unwrap(), vec![2]);
}

// ---------------------------------------------------------------- the mock

/// PG-A review note 4: an unscripted paged SET on the mock stalls; an
/// unscripted page-0 OUT is still ACKed.
#[test]
fn wp_pgadapt_the_mock_stalls_an_unscripted_paged_set() {
    let mut m = MockTransport::new().carrying_pages();
    assert!(matches!(
        m.control_out_paged(cmds::ROUTE_MASTER_VOLUME, 0, &[0; 4]),
        Err(TransportError::Stalled { .. })
    ));
    assert!(m.control_out(op::REQ_SET_MASTER_VOLUME, 0, &[0; 4]).is_ok());
    let mut m = MockTransport::new()
        .carrying_pages()
        .paged(cmds::ROUTE_MASTER_VOLUME, Reply::Data(Vec::new()));
    assert!(
        m.control_out_paged(cmds::ROUTE_MASTER_VOLUME, 0, &[0; 4])
            .is_ok()
    );
}

/// The probe-STALL decision (PG-A review note 3): a legacy device whose
/// probe STALLs gets `GET_PLATFORM` next, never `SYS.LAST_RESULT` (which a
/// 1.1.6 device would run as opcode 0x14).
#[test]
fn wp_pgadapt_a_stalled_probe_is_not_explained_by_last_result() {
    for g in [
        FakeGeneration::Bofr116 { page_guard: true },
        FakeGeneration::Stock116,
    ] {
        let dev = FakeDevice::new(
            crate::fake::Capture::demo(),
            FakeOptions {
                generation: Some(g),
                ..FakeOptions::default()
            },
        );
        let remote = dev.remote();
        let t = open(Box::new(dev)).unwrap();
        assert_eq!(t.generation(), Some(Generation::Legacy));
        assert_eq!(remote.paged_requests(), 1, "{g:?}: the probe only");
    }
}

// ---------------------------------------------------------------- doubles

/// Counts `control_in_batch_paged` calls (their sizes) on a fake.
struct Batching {
    inner: FakeDevice,
    batches: Arc<Mutex<Vec<usize>>>,
}

impl Batching {
    fn new(inner: FakeDevice) -> (Self, Arc<Mutex<Vec<usize>>>) {
        let batches = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                inner,
                batches: Arc::clone(&batches),
            },
            batches,
        )
    }
}

/// Forward every call but the ones a double overrides.
macro_rules! forward {
    () => {
        fn control_in(&mut self, o: u8, v: u16, l: u16) -> Result<Vec<u8>> {
            self.inner.control_in(o, v, l)
        }
        fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> Result<Vec<u8>> {
            self.inner.control_in_upto(o, v, l)
        }
        fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> Result<()> {
            self.inner.control_out(o, v, d)
        }
        fn descriptor(&self) -> &DeviceDescriptor {
            self.inner.descriptor()
        }
        fn caps(&self) -> TransportCaps {
            self.inner.caps()
        }
        fn carries_pages(&self) -> bool {
            true
        }
    };
}

impl Transport for Batching {
    forward!();
    fn control_in_paged(&mut self, id: CmdId, v: u16, l: u16) -> Result<Vec<u8>> {
        self.inner.control_in_paged(id, v, l)
    }
    fn control_out_paged(&mut self, id: CmdId, v: u16, d: &[u8]) -> Result<()> {
        self.inner.control_out_paged(id, v, d)
    }
    fn control_in_batch_paged(&mut self, reads: &[PagedRead]) -> Vec<Result<Vec<u8>>> {
        self.batches.lock().unwrap().push(reads.len());
        reads
            .iter()
            .map(|r| self.inner.control_in_paged(r.id, r.value, r.len))
            .collect()
    }
}

/// Another host's request lands just before `SYS.LAST_RESULT` is read.
struct Interloper {
    inner: FakeDevice,
    armed: Arc<AtomicBool>,
}

impl Transport for Interloper {
    forward!();
    fn control_in_paged(&mut self, id: CmdId, v: u16, l: u16) -> Result<Vec<u8>> {
        if id == cmds::SYS_LAST_RESULT && self.armed.load(Ordering::SeqCst) {
            let _ = self.inner.control_in_paged(cmds::SYS_SERIAL, 0, 16);
        }
        self.inner.control_in_paged(id, v, l)
    }
    fn control_out_paged(&mut self, id: CmdId, v: u16, d: &[u8]) -> Result<()> {
        self.inner.control_out_paged(id, v, d)
    }
}

/// Refuses the next `left` paged SETs with `code`, as a link that names the
/// code (`TransportError::Refused`).
struct Busy {
    inner: FakeDevice,
    left: Arc<AtomicU32>,
    code: Arc<AtomicU32>,
}

impl Transport for Busy {
    forward!();
    fn control_in_paged(&mut self, id: CmdId, v: u16, l: u16) -> Result<Vec<u8>> {
        self.inner.control_in_paged(id, v, l)
    }
    fn control_out_paged(&mut self, id: CmdId, v: u16, d: &[u8]) -> Result<()> {
        if self.left.load(Ordering::SeqCst) > 0 {
            self.left.fetch_sub(1, Ordering::SeqCst);
            let result = u8::try_from(self.code.load(Ordering::SeqCst)).unwrap();
            return Err(TransportError::Refused { id, result });
        }
        self.inner.control_out_paged(id, v, d)
    }
}

// ---------------------------------------------------------------- robustness

proptest::proptest! {
    /// Any 1.1.6-shaped call on either paged generation is answered or
    /// refused: no panic, no violation (no page-0 request, no zero-length
    /// GET reaches the fake).
    #[test]
    fn proptest_wp_pgadapt_any_116_call_keeps_the_hard_rules(
        opcode in proptest::prelude::any::<u8>(),
        value in proptest::prelude::any::<u16>(),
        len in 0u16..300,
        payload in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..80),
        get in proptest::prelude::any::<bool>(),
        bofr in proptest::prelude::any::<bool>(),
    ) {
        // Never the bootloader or a factory reset: they end the fake's session.
        proptest::prop_assume!(opcode != op::REQ_ENTER_BOOTLOADER && opcode != op::REQ_FACTORY_RESET);
        let g = if bofr { FakeGeneration::Bofr117 } else { FakeGeneration::STOCK117 };
        let (mut t, remote) = paged(g);
        if get {
            let _ = t.control_in_upto(opcode, value, len);
        } else {
            let _ = t.control_out(opcode, value, &payload);
        }
        proptest::prop_assert_eq!(remote.violations(), 0);
        proptest::prop_assert_eq!(remote.zero_length_gets(), 0);
    }
}
