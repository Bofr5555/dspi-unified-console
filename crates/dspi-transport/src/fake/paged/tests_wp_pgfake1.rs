//! WP PGFAKE1: the generations, the paged front, its inverse and the
//! violation rules. Vectors are the migration guide's (upstream
//! `release/v1.1.7` 103ba37, `vendor_protocol_migration_guide.md`) and the
//! fork's `extension_channel_spec.md` 1.3 (WP FWPG0), transcribed by hand.

use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};

use dspi_proto::paged::{
    LegacyBody, PageSet, PagedBody, ProbeReply, ProtocolInfo, SYS_PROTOCOL_INFO, legacy_equivalent,
    plan,
};
use proptest::prelude::*;

use super::*;
use crate::fake::{Capture, FakeOptions};
use crate::paged::open;
use crate::{FakeDevice, Generation, Transport};

const GUIDE_PROBE: [u8; 16] = [
    0x44, 0x53, 0x50, 0x69, 0x01, 0x00, 0x0C, 0x00, 0x40, 0x00, 0x00, 0x01, 0, 0, 0, 0,
];

fn stock() -> FakeDevice {
    FakeDevice::demo_generation(FakeGeneration::STOCK117)
}

fn bofr() -> FakeDevice {
    FakeDevice::demo_generation(FakeGeneration::Bofr117)
}

/// A generation's demo device that lets violations through.
fn lax(g: FakeGeneration) -> FakeDevice {
    let dev = FakeDevice::demo_generation(g);
    dev.shared.lock().options.allow_violation = true;
    dev
}

fn stalled<T: std::fmt::Debug>(r: Result<T>) {
    assert!(
        matches!(r, Err(TransportError::Stalled { .. })),
        "not a STALL: {r:?}"
    );
}

/// `SYS.LAST_RESULT` as the host reads it, 8 bytes.
fn last(d: &mut FakeDevice) -> Vec<u8> {
    d.control_in_paged(cmds::SYS_LAST_RESULT, 0, 8).unwrap()
}

// ------------------------------------------------------------ the inverse

/// The cross-test of PGFAKE1's entry: on every `direct`, `act_*` and `split`
/// row the fake's inverse equals `dspi_proto::paged::legacy_equivalent`, and
/// what it finds plans back to the same paged request.
#[test]
fn wp_pgfake1_inverse_equals_legacy_equivalent_on_every_simple_row() {
    let all = PageSet::vendored(|_| true);
    let bases: [u16; 16] = [
        0, 1, 2, 3, 7, 0x0F, 0x80, 0xFF, 0x100, 0x101, 0x201, 0x302, 0x800, 0x8000, 0xFF00, 0xFFFF,
    ];
    let mut compared = 0usize;
    let mut leaves = 0usize;
    let mut missed = Vec::new();
    for op in LEGACY_OPS.iter() {
        for (usb, side) in [(UsbDir::Out, op.out), (UsbDir::In, op.r#in)] {
            let Some(a) = side else { continue };
            for (leaf, key) in flatten(a) {
                let target = match leaf {
                    Adapter::Direct(d) => d.target,
                    Adapter::ActValue(x) | Adapter::ActAddr(x) | Adapter::ActPacked(x) => x.target,
                    Adapter::Split(_) | Adapter::Custom(_) => continue,
                };
                leaves += 1;
                let row = registry::command(target).unwrap();
                let mut lens = BTreeSet::from([usize::from(row.set_min), usize::from(row.set_max)]);
                if let Adapter::Direct(d) = leaf
                    && let Some(t) = d.trunc
                {
                    lens.insert(usize::from(t) + 4);
                }
                let mut hit = false;
                for b in bases {
                    let old = match key {
                        Some((sel, Some(k))) => {
                            let m = u32::from(sel.mask) << sel.shift;
                            let placed = u32::from(k) << sel.shift;
                            ((u32::from(b) & !m) | placed) as u16
                        }
                        _ => b,
                    };
                    let payloads: Vec<Vec<u8>> = match usb {
                        UsbDir::In => vec![Vec::new()],
                        UsbDir::Out => lens
                            .iter()
                            .map(|n| (0..*n).map(|i| (i as u8).wrapping_mul(37) | 1).collect())
                            .collect(),
                    };
                    for p in payloads {
                        let body = match usb {
                            UsbDir::In => LegacyBody::In { len: 64 },
                            UsbDir::Out => LegacyBody::Out(&p),
                        };
                        let Ok(pl) = plan(op.code, old, body, &all) else {
                            continue;
                        };
                        let Some(req) = pl.single() else { continue };
                        let (dir, payload) = match &req.body {
                            PagedBody::Get { .. } => (PagedDir::Get, Vec::new()),
                            PagedBody::Set(d) => (PagedDir::Set, d.clone()),
                        };
                        let ours = inverse(req.id, dir, req.value, &payload);
                        let theirs = legacy_equivalent(req.id, dir, req.value, &payload);
                        assert_eq!(
                            ours.as_ref().map(|i| (
                                i.opcode,
                                i.dir,
                                i.action,
                                i.value,
                                i.data.clone()
                            )),
                            theirs.map(|e| (e.opcode, e.dir, e.action, e.value, e.data)),
                            "{} {usb:?} old 0x{old:04X} -> {req:?}",
                            op.name
                        );
                        compared += 1;
                        if let Some(inv) = ours {
                            hit = true;
                            // What the fake runs plans back to the same request.
                            let back = match inv.dir {
                                UsbDir::In => LegacyBody::In { len: 64 },
                                UsbDir::Out => LegacyBody::Out(&inv.data),
                            };
                            let again = plan(inv.opcode, inv.value, back, &all).unwrap();
                            assert_eq!(
                                again.single().map(|r| (r.id, r.value)),
                                Some((req.id, req.value))
                            );
                        }
                    }
                }
                if !hit {
                    missed.push(format!("{} {usb:?} -> {target:?}", op.name));
                }
            }
        }
    }
    eprintln!(
        "{leaves} simple leaves, {compared} requests compared, {} leaves never won a request",
        missed.len()
    );
    assert!(leaves > 200, "{leaves} leaves");
    assert!(compared > 2000, "{compared} comparisons");
    // Every simple row is reached by the inverse from at least one of its own
    // requests, unless another row wins the tie for the same request.
    assert!(missed.is_empty(), "{missed:#?}");
}

#[test]
fn wp_pgfake1_inverse_has_nothing_for_new_and_custom_commands() {
    assert_eq!(
        inverse(cmds::SYS_PROTOCOL_INFO, PagedDir::Get, 0, &[]),
        None
    );
    assert_eq!(inverse(cmds::SYS_PIN_STATE, PagedDir::Get, 0, &[]), None);
    assert_eq!(inverse(EXT_TUNNEL, PagedDir::Get, 0, &[]), None);
    assert_eq!(inverse(CmdId(0x0053), PagedDir::Get, 0, &[]), None);
    // ROUTE.MASTER_VOLUME_SAVED GET is GET_MASTER_VOLUME_SAVED, never FACTORY_RESET.
    let inv = inverse(cmds::ROUTE_MASTER_VOLUME_SAVED, PagedDir::Get, 0, &[]).unwrap();
    assert_ne!(inv.opcode, op::REQ_FACTORY_RESET);
    assert!(!inv.action);
}

// ------------------------------------------------------------ G2s

#[test]
fn wp_pgfake1_stock117_answers_the_guides_discovery_vectors() {
    let mut d = stock();
    // 4.2: the probe.
    let probe = d.control_in_paged(SYS_PROTOCOL_INFO, 0, 16).unwrap();
    assert_eq!(probe, GUIDE_PROBE);
    assert!(matches!(ProtocolInfo::decode(&probe), ProbeReply::Paged(_)));
    // 4.6: SYS.PLATFORM, an RP2350 on 1.1.7 beta 1.
    assert_eq!(
        d.control_in_paged(cmds::SYS_PLATFORM, 0, 7).unwrap(),
        [0x01, 0x01, 0x17, 0x09, 0x01, 0x07, 0x01]
    );
    // 4.7: SYS.PAGE_LIST, twelve pages at revision 1, no DIAG.
    assert_eq!(
        d.control_in_paged(cmds::SYS_PAGE_LIST, 0, 256).unwrap(),
        [
            1, 1, 2, 1, 3, 1, 4, 1, 5, 1, 6, 1, 7, 1, 8, 1, 9, 1, 10, 1, 11, 1, 12, 1
        ]
    );
    // 4.7: SYS.PAGE_COMMANDS of FILTER.
    let mut filter = [0u8; 64];
    filter[2] = 0x07;
    filter[34] = 0x07;
    assert_eq!(
        d.control_in_paged(cmds::SYS_PAGE_COMMANDS, 6, 64).unwrap(),
        filter
    );
    // A page this build lacks, page 0 included, is VC_ERR_ADDRESS.
    for page in [0, 0xF0, 0xE0] {
        stalled(d.control_in_paged(cmds::SYS_PAGE_COMMANDS, page, 64));
        assert_eq!(last(&mut d)[3], lm::VC_ERR_ADDRESS);
    }
    // 4.7: SYS.COMMAND_INFO of STORE.PRESET_LOAD.
    assert_eq!(
        d.control_in_paged(cmds::SYS_COMMAND_INFO, 0x0211, 8)
            .unwrap(),
        [0x38, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x02]
    );
    stalled(d.control_in_paged(cmds::SYS_COMMAND_INFO, 0x01FF, 8));
}

#[test]
fn wp_pgfake1_stock117_answers_the_guides_request_vectors() {
    let mut d = stock();
    let remote = d.remote();
    // 3.2: a SET and a GET with an address.
    d.control_out_paged(cmds::ROUTE_OUTPUT_GAIN, 1, &[0x00, 0x00, 0xC0, 0xC0])
        .unwrap();
    assert_eq!(
        d.control_in_paged(cmds::ROUTE_OUTPUT_GAIN, 1, 4).unwrap(),
        [0x00, 0x00, 0xC0, 0xC0]
    );
    // 3.2: a refused read and its reason.
    stalled(d.control_in_paged(cmds::ROUTE_OUTPUT_GAIN, 9, 4));
    assert_eq!(
        last(&mut d),
        [0x21, 0x05, 0x01, 0x13, 0x09, 0x00, 0x00, 0x00]
    );
    // Reading SYS.LAST_RESULT never overwrites it (5.2).
    assert_eq!(
        last(&mut d),
        [0x21, 0x05, 0x01, 0x13, 0x09, 0x00, 0x00, 0x00]
    );
    // 3.2: a former write-as-read, and a zero-length action.
    d.control_out_paged(cmds::SYS_STATS_RESET, 0, &[0x01])
        .unwrap();
    d.control_out_paged(cmds::MEAS_RTA_RESET, 0, &[]).unwrap();
    assert_eq!(
        last(&mut d),
        [0x2A, 0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
    );
    // 6.6 worked example 1: OUTPUT.TYPE slot 0, type 5.
    stalled(d.control_out_paged(cmds::OUTPUT_TYPE, 0, &[5]));
    assert_eq!(
        last(&mut d),
        [0x10, 0x04, 0x00, 0x14, 0x00, 0x00, 0x00, 0x00]
    );
    // 6.6: STORE.PRESET_LOAD of slot 10.
    stalled(d.control_out_paged(cmds::STORE_PRESET_LOAD, 10, &[]));
    assert_eq!(
        last(&mut d),
        [0x11, 0x02, 0x00, 0x13, 0x0A, 0x00, 0x00, 0x00]
    );
    // 13.1.8: a NaN user volume is VC_ERR_VALUE, and nothing is stored.
    let before = d.control_in_paged(cmds::ROUTE_USER_VOLUME, 0, 4).unwrap();
    stalled(d.control_out_paged(cmds::ROUTE_USER_VOLUME, 0, &[0x00, 0x00, 0xC0, 0x7F]));
    assert_eq!(last(&mut d)[3], lm::VC_ERR_VALUE);
    assert_eq!(
        d.control_in_paged(cmds::ROUTE_USER_VOLUME, 0, 4).unwrap(),
        before
    );
    // An infinity too, except where the setter clamps it (5.6).
    stalled(d.control_out_paged(cmds::ROUTE_OUTPUT_GAIN, 0, &f32::INFINITY.to_le_bytes()));
    // 13.1.8: MEAS.CLIP_CLEAR sent as a GET is VC_ERR_DIRECTION.
    stalled(d.control_in_paged(cmds::MEAS_CLIP_CLEAR, 0, 4));
    assert_eq!(last(&mut d)[3], lm::VC_ERR_DIRECTION);
    // 5.5: exact lengths, a whole 16-bit address, wValue 0 where nothing is addressed.
    stalled(d.control_out_paged(cmds::ROUTE_OUTPUT_GAIN, 1, &[0; 5]));
    assert_eq!(last(&mut d)[3], lm::VC_ERR_LENGTH);
    stalled(d.control_in_paged(cmds::ROUTE_OUTPUT_GAIN, 0x0100, 4));
    assert_eq!(last(&mut d)[3], lm::VC_ERR_ADDRESS);
    stalled(d.control_in_paged(cmds::ROUTE_MASTER_VOLUME, 1, 4));
    assert_eq!(last(&mut d)[3], lm::VC_ERR_ADDRESS);
    // No such command, and the private pages are absent on stock.
    stalled(d.control_in_paged(CmdId(0x05FE), 0, 4));
    assert_eq!(last(&mut d)[3], lm::VC_ERR_UNKNOWN);
    stalled(d.control_in_paged(EXT_TUNNEL, 0, 64));
    assert_eq!(
        last(&mut d),
        [0x10, 0xE0, 0x01, 0x10, 0x00, 0x00, 0x00, 0x00]
    );
    // The fake's strict rule: a GET with wLength 0 is refused and counted.
    stalled(d.control_in_paged(cmds::ROUTE_OUTPUT_GAIN, 1, 0));
    assert_eq!(last(&mut d)[3], lm::VC_ERR_LENGTH);
    assert_eq!(remote.zero_length_gets(), 1);
    assert_eq!(remote.violations(), 0);
    assert_eq!(
        remote.last_result().map(|r| r.id),
        Some(cmds::ROUTE_OUTPUT_GAIN)
    );
}

#[test]
fn wp_pgfake1_a_write_through_the_paged_front_is_the_same_state() {
    let mut d = stock();
    let remote = d.remote();
    d.control_out_paged(cmds::ROUTE_MASTER_VOLUME, 0, &(-12.5f32).to_le_bytes())
        .unwrap();
    let off = crate::fake::section("master_volume").unwrap().0;
    assert_eq!(remote.image()[off..off + 4], (-12.5f32).to_le_bytes());
    // A write-as-read's refusal byte becomes its result code (guide 5.4).
    d.control_out_paged(cmds::STORE_PRESET_SAVE, 3, &[])
        .unwrap();
    d.control_out_paged(cmds::STORE_PRESET_LOAD, 3, &[])
        .unwrap();
    assert_eq!(remote.unmodelled(), 0);
}

#[test]
fn wp_pgfake1_unmodelled_and_hidden_commands() {
    let mut d = stock();
    let remote = d.remote();
    // -- WP PGFAKE2 -- FILTER.BAND is answered now (fake/paged_custom.rs); a pin
    // availability query is still listed but unmodelled: refused and counted.
    stalled(d.control_in_paged(cmds::INPUT_SPDIF_RX_PIN_AVAIL, 0, 24));
    assert_eq!(last(&mut d)[3], lm::VC_ERR_UNSUPPORTED);
    assert_eq!(remote.unmodelled(), 1);
    // A hidden command leaves the build.
    remote.hide_paged(cmds::ROUTE_OUTPUT_GAIN);
    stalled(d.control_in_paged(cmds::ROUTE_OUTPUT_GAIN, 0, 4));
    assert_eq!(last(&mut d)[3], lm::VC_ERR_UNKNOWN);
    let page =
        PageCommands::decode(&d.control_in_paged(cmds::SYS_PAGE_COMMANDS, 5, 64).unwrap()).unwrap();
    assert!(!page.has(cmds::ROUTE_OUTPUT_GAIN.cmd(), PagedDir::Get));
    assert!(page.has(cmds::ROUTE_MASTER_VOLUME.cmd(), PagedDir::Set));
}

#[test]
fn wp_pgfake1_an_rp2040_build_lacks_the_rp2350_commands() {
    let mut cap = Capture::demo();
    cap.platform[0] = 0;
    let mut d = FakeDevice::new(cap, FakeGeneration::STOCK117.options());
    stalled(d.control_in_paged(cmds::INPUT_ADAT_RX_STATUS, 0, 20));
    assert_eq!(last(&mut d)[3], lm::VC_ERR_UNKNOWN);
    let page =
        PageCommands::decode(&d.control_in_paged(cmds::SYS_PAGE_COMMANDS, 3, 64).unwrap()).unwrap();
    assert!(!page.has(cmds::INPUT_ADAT_RX_STATUS.cmd(), PagedDir::Get));
}

#[test]
fn wp_pgfake1_a_higher_page_revision_takes_a_longer_set() {
    let mut newer = FakeDevice::demo_generation(FakeGeneration::Stock117 { revision: 2 });
    let list = newer.control_in_paged(cmds::SYS_PAGE_LIST, 0, 256).unwrap();
    assert!(list.chunks(2).all(|p| p[1] == 2), "{list:?}");
    // One byte more than the vendored 4: accepted, the vendored part applied.
    let mut five = (-3.0f32).to_le_bytes().to_vec();
    five.push(0);
    newer
        .control_out_paged(cmds::ROUTE_OUTPUT_GAIN, 2, &five)
        .unwrap();
    assert_eq!(
        newer
            .control_in_paged(cmds::ROUTE_OUTPUT_GAIN, 2, 4)
            .unwrap(),
        (-3.0f32).to_le_bytes()
    );
    // Below the minimum is still refused; at the vendored revision so is longer.
    stalled(newer.control_out_paged(cmds::ROUTE_OUTPUT_GAIN, 2, &[0; 3]));
    stalled(stock().control_out_paged(cmds::ROUTE_OUTPUT_GAIN, 2, &five));
}

#[test]
#[should_panic(expected = "reached stock 1.1.7 firmware")]
fn wp_pgfake1_stock117_fails_a_test_that_sends_page_0() {
    let mut d = stock();
    let _ = d.control_in(op::REQ_GET_PLATFORM, 0, 7);
}

#[test]
fn wp_pgfake1_stock117_allowed_refuses_page_0_as_unknown() {
    let mut d = lax(FakeGeneration::STOCK117);
    let remote = d.remote();
    // 4.3: the old GET_PLATFORM STALLs, VC_ERR_UNKNOWN; nothing changes.
    stalled(d.control_in(op::REQ_GET_PLATFORM, 0, 7));
    assert_eq!(
        last(&mut d),
        [0x7F, 0x00, 0x01, 0x10, 0x00, 0x00, 0x00, 0x00]
    );
    stalled(d.control_out(op::REQ_SET_MASTER_VOLUME, 0, &0f32.to_le_bytes()));
    stalled(d.control_in(op::REQ_FACTORY_RESET, 0, 1));
    stalled(d.control_in_paged(CmdId(0x007F), 0, 7));
    assert_eq!(remote.violations(), 4);
    assert!(remote.last_violation().unwrap().contains("0x7F"));
}

// ------------------------------------------------------------ G2b

#[test]
fn wp_pgfake1_bofr117_lists_the_tunnel() {
    let mut d = bofr();
    let probe = d.control_in_paged(SYS_PROTOCOL_INFO, 0, 16).unwrap();
    assert_eq!(probe[6], 13, "page 0xE0 counts");
    let list = d.control_in_paged(cmds::SYS_PAGE_LIST, 0, 256).unwrap();
    assert_eq!(list.len(), 26);
    assert_eq!(list[22..], [0x0C, 0x01, 0xE0, 0x01]);
    // extension_channel_spec.md 1.3.6: byte 2 bit 0 and byte 34 bit 0.
    let mut ext = [0u8; 64];
    ext[2] = 1;
    ext[34] = 1;
    assert_eq!(
        d.control_in_paged(cmds::SYS_PAGE_COMMANDS, 0xE0, 64)
            .unwrap(),
        ext
    );
    // 1.3.2: SYS.COMMAND_INFO of 0xE010.
    assert_eq!(
        d.control_in_paged(cmds::SYS_COMMAND_INFO, 0xE010, 8)
            .unwrap(),
        [0x00, 0x00, 0xFF, 0xFF, 0x01, 0x40, 0x18, 0x03]
    );
    // Other commands and pages of the private range are unknown.
    stalled(d.control_in_paged(CmdId(0xE011), 0, 4));
    stalled(d.control_in_paged(CmdId(0xE110), 0, 4));
    assert_eq!(last(&mut d)[3], lm::VC_ERR_UNKNOWN);
}

#[test]
fn wp_pgfake1_the_tunnel_is_the_extension_channel() {
    // EXT_LIST through the legacy stack of an unprobed device ...
    let mut plain = bofr();
    let legacy_list = plain
        .control_in_upto(op::REQ_EXTENSION, gext::EXT_LIST, 64)
        .unwrap();
    // ... and through the tunnel, after the probe (1.3.1: byte-identical).
    let mut d = bofr();
    let remote = d.remote();
    d.control_in_paged(SYS_PROTOCOL_INFO, 0, 16).unwrap();
    assert_eq!(
        d.control_in_paged(EXT_TUNNEL, gext::EXT_LIST, 64).unwrap(),
        legacy_list
    );
    // 1.3.1: the crossfeed pair map written through the tunnel reads back.
    let map = d
        .control_in_paged(EXT_TUNNEL, gext::EXT_CROSSFEED_PAIR_MAP, 4)
        .unwrap();
    d.control_out_paged(EXT_TUNNEL, gext::EXT_CROSSFEED_PAIR_MAP, &map)
        .unwrap();
    assert_eq!(
        d.control_in_paged(EXT_TUNNEL, gext::EXT_CROSSFEED_PAIR_MAP, 4)
            .unwrap(),
        map
    );
    // The core voltage: 16 bytes, and a valid request is applied.
    assert_eq!(
        d.control_in_paged(EXT_TUNNEL, gext::EXT_CORE_VREG, 16)
            .unwrap()
            .len(),
        16
    );
    d.control_out_paged(EXT_TUNNEL, gext::EXT_CORE_VREG, &[1, 0x0E])
        .unwrap();
    // 1.3.4: every refusal is a VC_ERR_* code, with wValue = the sub-command.
    stalled(d.control_in_paged(EXT_TUNNEL, 0x7777, 4));
    assert_eq!(
        last(&mut d),
        [0x10, 0xE0, 0x01, 0x13, 0x77, 0x77, 0x00, 0x00]
    );
    stalled(d.control_out_paged(EXT_TUNNEL, gext::EXT_LIST, &[0]));
    assert_eq!(last(&mut d)[3], lm::VC_ERR_DIRECTION);
    stalled(d.control_out_paged(EXT_TUNNEL, gext::EXT_CORE_VREG, &[1, 0x0E, 0]));
    assert_eq!(last(&mut d)[3], lm::VC_ERR_LENGTH);
    stalled(d.control_out_paged(EXT_TUNNEL, gext::EXT_CORE_VREG, &[9, 0]));
    assert_eq!(
        last(&mut d),
        [0x10, 0xE0, 0x00, 0x14, 0x02, 0x00, 0x00, 0x00]
    );
    stalled(d.control_out_paged(EXT_TUNNEL, gext::EXT_CORE_VREG, &[]));
    assert_eq!(last(&mut d)[3], lm::VC_ERR_LENGTH);
    // The upstream commands work beside it.
    assert_eq!(
        d.control_in_paged(cmds::SYS_PLATFORM, 0, 7).unwrap()[1..3],
        [0x01, 0x17]
    );
    assert_eq!(remote.violations(), 0);
}

#[test]
fn wp_pgfake1_bofr117_serves_legacy_hosts_on_page_0() {
    // A host that never sends the probe (an old app, the ESP32 panel, an L12 hub).
    let mut d = bofr();
    let remote = d.remote();
    let p = d.control_in_upto(op::REQ_GET_PLATFORM, 0, 8).unwrap();
    assert_eq!(p[1..3], [0x01, 0x17]);
    d.control_out(op::REQ_SET_MASTER_VOLUME, 0, &(-6.0f32).to_le_bytes())
        .unwrap();
    assert!(
        !d.control_in_upto(op::REQ_EXTENSION, gext::EXT_LIST, 64)
            .unwrap()
            .is_empty()
    );
    assert_eq!(remote.violations(), 0);
    // A page-0 request is not recorded in SYS.LAST_RESULT (1.3.3).
    assert_eq!(remote.last_result(), None);
}

#[test]
#[should_panic(expected = "after it answered the probe as paged")]
fn wp_pgfake1_bofr117_fails_page_0_after_a_paged_probe() {
    let mut d = bofr();
    d.control_in_paged(SYS_PROTOCOL_INFO, 0, 16).unwrap();
    let _ = d.control_in(op::REQ_EXTENSION, gext::EXT_LIST, 4);
}

#[test]
fn wp_pgfake1_bofr117_allowed_still_answers_page_0_after_the_probe() {
    let mut d = lax(FakeGeneration::Bofr117);
    let remote = d.remote();
    d.control_in_paged(SYS_PROTOCOL_INFO, 0, 16).unwrap();
    assert!(d.control_in_upto(op::REQ_GET_PLATFORM, 0, 8).is_ok());
    assert_eq!(remote.violations(), 1);
}

// ------------------------------------------------------------ G1s, G1b

#[test]
fn wp_pgfake1_the_116_generations_set_their_options() {
    let mut cap_ext = FakeGeneration::Stock116.options();
    cap_ext.extensions = true; // overwritten by the generation
    let mut s116 = FakeDevice::new(Capture::demo(), cap_ext);
    stalled(s116.control_in_upto(op::REQ_EXTENSION, gext::EXT_LIST, 64));
    let mut b116 = FakeDevice::new(
        Capture::demo(),
        FakeGeneration::Bofr116 { page_guard: false }.options(),
    );
    assert!(
        b116.control_in_upto(op::REQ_EXTENSION, gext::EXT_LIST, 64)
            .is_ok()
    );
    // The platform still says 1.1.6 on the 1.1.6 line.
    assert_eq!(
        b116.control_in_upto(op::REQ_GET_PLATFORM, 0, 8).unwrap()[2],
        0x16
    );
    // The probe: a STALL on every 1.1.6 generation, PAGEGUARD or not, no violation.
    for g in [
        FakeGeneration::Stock116,
        FakeGeneration::Bofr116 { page_guard: false },
        FakeGeneration::Bofr116 { page_guard: true },
    ] {
        let mut d = FakeDevice::demo_generation(g);
        let remote = d.remote();
        stalled(d.control_in_paged(SYS_PROTOCOL_INFO, 0, 16));
        assert_eq!(remote.violations(), 0, "{g:?}");
    }
}

#[test]
fn wp_pgfake1_violations_are_counted_on_every_generation() {
    // A paged request on 1.1.6: the guard STALLs it, the unguarded line runs
    // the same-number opcode; both count.
    for g in [
        FakeGeneration::Stock116,
        FakeGeneration::Bofr116 { page_guard: false },
        FakeGeneration::Bofr116 { page_guard: true },
    ] {
        let mut d = lax(g);
        let remote = d.remote();
        // GET_PLATFORM's number on page 5: 1.1.6 answers it as GET_PLATFORM.
        let r = d.control_in_paged(CmdId::new(5, op::REQ_GET_PLATFORM), 0, 8);
        assert_eq!(
            r.is_err(),
            g == FakeGeneration::Bofr116 { page_guard: true },
            "{g:?}"
        );
        assert_eq!(remote.violations(), 1, "{g:?}");
    }
}

#[test]
fn wp_pgfake1_a_poisoned_fake_fails_in_drop() {
    // The violation panics where it happens (here caught, as a worker thread
    // would swallow it) ...
    let mut d = stock();
    let remote = d.remote();
    let caught = catch_unwind(AssertUnwindSafe(|| {
        let _ = d.control_in(op::REQ_GET_PLATFORM, 0, 7);
    }));
    assert!(caught.is_err());
    assert_eq!(remote.violations(), 1);
    // ... and the device fails again when it is dropped.
    let dropped = catch_unwind(AssertUnwindSafe(move || drop(d)));
    let msg = dropped.unwrap_err();
    let msg = msg.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(msg.contains("protocol violation"), "{msg}");
    // Allowed violations do not fail the drop.
    let mut ok = lax(FakeGeneration::STOCK117);
    let _ = ok.control_in(op::REQ_GET_PLATFORM, 0, 7);
    drop(ok);
}

// ------------------------------------------------------------ with `open`

/// Every generation through `paged::open`: the 1.1.6 ones are legacy after
/// one probe request; the paged ones are paged after the probe and the
/// discovery (WP PGADAPT; until then they were refused). No violation
/// anywhere.
#[test]
fn wp_pgfake1_open_decides_every_generation() {
    for g in FakeGeneration::ALL {
        let dev = FakeDevice::demo_generation(g);
        let remote = dev.remote();
        let t = open(Box::new(dev)).unwrap();
        if g.is_paged() {
            assert!(
                matches!(t.generation(), Some(Generation::Paged(_))),
                "{g:?}"
            );
            assert!(remote.paged_requests() > 1, "{g:?}");
        } else {
            assert_eq!(t.generation(), Some(Generation::Legacy), "{g:?}");
            assert_eq!(remote.paged_requests(), 1, "{g:?}");
        }
        assert_eq!(remote.violations(), 0, "{g:?}");
    }
}

// ------------------------------------------------------------ robustness

proptest! {
    /// Any paged request with a non-zero page is answered or refused, never a
    /// panic, on both paged generations.
    #[test]
    fn proptest_wp_pgfake1_any_paged_request_is_answered_without_panic(
        page in 1u8..=255,
        cmd in any::<u8>(),
        value in any::<u16>(),
        payload in proptest::collection::vec(any::<u8>(), 0..80),
        get in any::<bool>(),
        len in 0u16..300,
        bofr in any::<bool>(),
    ) {
        let mut d = if bofr { bofr_dev() } else { stock() };
        let remote = d.remote();
        let id = CmdId::new(page, cmd);
        if get {
            let _ = d.control_in_paged(id, value, len);
        } else {
            let _ = d.control_out_paged(id, value, &payload);
        }
        prop_assert_eq!(remote.violations(), 0);
    }
}

fn bofr_dev() -> FakeDevice {
    bofr()
}

#[test]
fn wp_pgfake1_options_default_to_the_classic_fake() {
    assert_eq!(FakeOptions::default().generation, None);
    let d = FakeDevice::demo();
    assert_eq!(d.shared.lock().generation(), None);
}
