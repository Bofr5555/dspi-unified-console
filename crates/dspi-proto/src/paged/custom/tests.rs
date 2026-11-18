//! WP PGCUST: the 14 custom rows, each with vectors transcribed from the
//! migration guide (sections 6.3, 6.5, 6.7; 4.6 for `SYS.PLATFORM`), scripted
//! replies with a refusal in the middle of every multi-step recipe, the
//! `GET_STATUS` table held equal to the vendored row's data, the reverse view,
//! and the shared properties.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::super::legacy::{
    self, Adapter, VC_ERR_ADDRESS, VC_ERR_BUSY, VC_ERR_STATE, VC_ERR_UNKNOWN, VC_ERR_VALUE,
};
use super::super::plan::{
    LegacyBody, Next, Outcome, PagedBody, PagedRequest, Plan, PlanError, UsbDir, WireAnswer, plan,
    reaches,
};
use super::super::registry::{self, cmds};
use super::super::reverse::legacy_equivalent;
use super::super::{CmdId, PageSet, PagedDir};
use super::{Sel, Ty, ZERO_IF_ABSENT, status_selector};
use proptest::prelude::*;
use std::collections::VecDeque;

const CUSTOM_OPS: [u8; 14] = [
    0x28, 0x42, 0x43, 0x44, 0x50, 0x51, 0x70, 0x71, 0x83, 0x97, 0xA1, 0xB2, 0xB3, 0xEE,
];

fn full() -> PageSet {
    PageSet::vendored(|_| true)
}

/// A release build of an RP2350 (no `DIAG` page).
fn release() -> PageSet {
    PageSet::vendored(|c| c.build.is_none() || c.build == Some("PICO_RP2350"))
}

fn data(b: &[u8]) -> WireAnswer {
    WireAnswer::Data(b.to_vec())
}

fn refused(code: u8) -> WireAnswer {
    WireAnswer::Refused { result: Some(code) }
}

/// Run a plan against scripted answers; the requests it sent and its outcome.
/// Running out of script answers a refusal without a result.
fn run_plan(p: Plan, script: Vec<WireAnswer>) -> (Vec<PagedRequest>, Outcome) {
    let mut script: VecDeque<_> = script.into();
    let mut sent = Vec::new();
    let out = p.run(32, |r| {
        sent.push(r.clone());
        script
            .pop_front()
            .unwrap_or(WireAnswer::Refused { result: None })
    });
    (sent, out)
}

fn run(
    opcode: u8,
    value: u16,
    body: LegacyBody<'_>,
    script: Vec<WireAnswer>,
) -> (Vec<PagedRequest>, Outcome) {
    run_plan(plan(opcode, value, body, &full()).unwrap(), script)
}

fn get(id: CmdId, value: u16, len: u16) -> PagedRequest {
    PagedRequest {
        id,
        value,
        body: PagedBody::Get { len },
    }
}

fn set(id: CmdId, value: u16, d: &[u8]) -> PagedRequest {
    PagedRequest {
        id,
        value,
        body: PagedBody::Set(d.to_vec()),
    }
}

fn f32s(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

// ---------------------------------------------------------------------------
// Every custom row is planned now

#[test]
fn wp_pgcust_every_custom_row_is_handled() {
    let mut seen = Vec::new();
    for o in &legacy::LEGACY_OPS {
        for (side, dir) in [(o.out, UsbDir::Out), (o.r#in, UsbDir::In)] {
            if let Some(Adapter::Custom(_)) = side {
                seen.push(o.code);
                let body = match dir {
                    UsbDir::In => LegacyBody::In { len: 64 },
                    UsbDir::Out => LegacyBody::Out(&[0; 18]),
                };
                let r = plan(o.code, 0, body, &full());
                assert!(
                    !matches!(
                        r,
                        Err(PlanError::Unsupported { .. } | PlanError::Table { .. })
                    ),
                    "0x{:02X}: {r:?}",
                    o.code
                );
            }
        }
    }
    assert_eq!(seen, CUSTOM_OPS);
}

// ---------------------------------------------------------------------------
// GET_CS_DISPLAY_CFG 0x28 (guide 6.5)

#[test]
fn wp_pgcust_cs_display_cfg() {
    let caps = [16u8, 9, 0, 0]; // page count 16, model count 9, 2 reserved
    let cfg: Vec<u8> = (0x40..0x4C).collect();
    let (sent, out) = run(
        0x28,
        0,
        LegacyBody::In { len: 16 },
        vec![data(&caps), data(&cfg)],
    );
    assert_eq!(
        sent,
        [
            get(cmds::CS_DISPLAY_CAPS, 0, 4),
            get(cmds::CS_DISPLAY_CONFIG, 0, 12)
        ]
    );
    assert_eq!(sent[0].setup_packet(), [0xC1, 0x71, 0, 0, 0x02, 0x0A, 4, 0]);
    assert_eq!(out, Outcome::Reply([&caps[..], &cfg].concat()));
    // A refusal in the middle raises; no half reply.
    let (sent, out) = run(
        0x28,
        0,
        LegacyBody::In { len: 16 },
        vec![data(&caps), refused(VC_ERR_BUSY)],
    );
    assert_eq!(sent.len(), 2);
    assert_eq!(
        out,
        Outcome::Stall {
            result: Some(VC_ERR_BUSY)
        }
    );
    // Four bytes need only the caps, as one batchable request.
    let p = plan(0x28, 0, LegacyBody::In { len: 4 }, &full()).unwrap();
    assert_eq!(p.single(), Some(&get(cmds::CS_DISPLAY_CAPS, 0, 4)));
    assert_eq!(
        run_plan(p, vec![data(&caps)]).1,
        Outcome::Reply(caps.to_vec())
    );
}

// ---------------------------------------------------------------------------
// SET_EQ_PARAM 0x42 and GET_EQ_PARAM 0x43 (guide 6.5)

/// The old 18-byte `EqParamPacket`: channel 2, band 3, type 1, bypass 0,
/// 1000 Hz, Q 0.707, -3 dB, Qp 0x0200.
fn eq_packet18() -> Vec<u8> {
    [
        &[2u8, 3, 1, 0][..],
        &f32s(&[1000.0, 0.707, -3.0]),
        &[0x00, 0x02],
    ]
    .concat()
}

#[test]
fn wp_pgcust_set_eq_param_18_bytes_is_one_set() {
    let p = eq_packet18();
    let pl = plan(0x42, 0, LegacyBody::Out(&p), &full()).unwrap();
    // wValue = (p[0] << 8) | p[1]; payload p[2], p[3], qp low, qp high, p[4..16].
    let want = [&[1u8, 0, 0x00, 0x02][..], &p[4..16]].concat();
    assert_eq!(pl.single(), Some(&set(cmds::FILTER_BAND, 0x0203, &want)));
    assert_eq!(
        pl.single().unwrap().setup_packet(),
        [0x41, 0x10, 0x03, 0x02, 0x02, 0x06, 16, 0]
    );
    assert_eq!(run_plan(pl, vec![WireAnswer::Ack]).1, Outcome::Ack);
    // A band in the gap 10-19 is VC_ERR_ADDRESS now; 1.1.6 ignored the write.
    let (_, out) = run(0x42, 0, LegacyBody::Out(&p), vec![refused(VC_ERR_ADDRESS)]);
    assert_eq!(
        out,
        Outcome::Swallowed {
            result: Some(VC_ERR_ADDRESS)
        }
    );
}

#[test]
fn wp_pgcust_set_eq_param_16_bytes_keeps_the_stored_qp() {
    let p = &eq_packet18()[..16];
    // The stored band (another Qp, 0x015A) as FILTER.BAND answers it.
    let stored = [&[4u8, 0, 0x5A, 0x01][..], &f32s(&[50.0, 1.0, 0.0])].concat();
    let (sent, out) = run(
        0x42,
        0,
        LegacyBody::Out(p),
        vec![data(&stored), WireAnswer::Ack],
    );
    let want = [&[1u8, 0, 0x5A, 0x01][..], &p[4..16]].concat();
    assert_eq!(
        sent,
        [
            get(cmds::FILTER_BAND, 0x0203, 16),
            set(cmds::FILTER_BAND, 0x0203, &want)
        ]
    );
    assert_eq!(out, Outcome::Ack);
    // Read refused: nothing written, swallowed as 1.1.6 ignored it.
    let (sent, out) = run(0x42, 0, LegacyBody::Out(p), vec![refused(VC_ERR_ADDRESS)]);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        out,
        Outcome::Swallowed {
            result: Some(VC_ERR_ADDRESS)
        }
    );
    // Write refused (busy) after the read: not an ACK.
    let (sent, out) = run(
        0x42,
        0,
        LegacyBody::Out(p),
        vec![data(&stored), refused(VC_ERR_BUSY)],
    );
    assert_eq!(sent.len(), 2);
    assert_eq!(
        out,
        Outcome::Swallowed {
            result: Some(VC_ERR_BUSY)
        }
    );
    // A short read cannot give the Qp: raised, nothing written.
    let (sent, out) = run(0x42, 0, LegacyBody::Out(p), vec![data(&[4, 0, 0x5A])]);
    assert_eq!(sent.len(), 1);
    assert_eq!(out, Outcome::Stall { result: None });
    // Fewer than 16 bytes: 1.1.6 ignored it; nothing sent.
    let (sent, out) = run(0x42, 0, LegacyBody::Out(&p[..15]), vec![]);
    assert!(sent.is_empty());
    assert_eq!(out, Outcome::Swallowed { result: None });
}

#[test]
fn wp_pgcust_get_eq_param_every_param() {
    // FILTER.BAND reply: type 1, bypass 1, Qp 0x0200, 1000 Hz, Q 0.707, -3 dB.
    let band = [&[1u8, 1, 0x00, 0x02][..], &f32s(&[1000.0, 0.707, -3.0])].concat();
    let cases: [(u16, Vec<u8>); 8] = [
        (0, vec![1, 0, 0, 0]),
        (1, band[4..8].to_vec()),
        (2, band[8..12].to_vec()),
        (3, band[12..16].to_vec()),
        (4, vec![1, 0, 0, 0]),
        (5, vec![0x00, 0x02, 0, 0]),
        (6, vec![0; 4]),
        (7, vec![0; 4]),
    ];
    for (param, want) in cases {
        // Old wValue (channel << 8) | (band << 3) | param: channel 2, band 3.
        let old = (2 << 8) | (3 << 3) | param;
        let (sent, out) = run(0x43, old, LegacyBody::In { len: 4 }, vec![data(&band)]);
        assert_eq!(sent, [get(cmds::FILTER_BAND, 0x0203, 16)], "param {param}");
        assert_eq!(out, Outcome::Reply(want), "param {param}");
    }
    // Crossover band 20 of output channel 4 (the 5-bit band field).
    let p = plan(
        0x43,
        (4 << 8) | (20 << 3) | 1,
        LegacyBody::In { len: 4 },
        &full(),
    )
    .unwrap();
    assert_eq!(p.single().unwrap().value, 0x0414);
    let (_, out) = run(
        0x43,
        0x0218,
        LegacyBody::In { len: 4 },
        vec![refused(VC_ERR_ADDRESS)],
    );
    assert_eq!(
        out,
        Outcome::Stall {
            result: Some(VC_ERR_ADDRESS)
        }
    );
}

// ---------------------------------------------------------------------------
// SET_PREAMP 0x44 (guide 6.7, SYS.PLATFORM 4.6)

#[test]
fn wp_pgcust_set_preamp_writes_every_input() {
    let db = (-6.5f32).to_le_bytes();
    // Guide 4.6: RP2350 answers 01 01 17 09 01 07 01, an RP2040 00 01 17 05 01 07 01.
    for (platform, inputs) in [
        ([0x01u8, 0x01, 0x17, 0x09, 0x01, 0x07, 0x01], 8u16),
        ([0x00, 0x01, 0x17, 0x05, 0x01, 0x07, 0x01], 2),
    ] {
        let mut script = vec![data(&platform)];
        script.extend((0..inputs).map(|_| WireAnswer::Ack));
        let (sent, out) = run(0x44, 0, LegacyBody::Out(&db), script);
        let mut want = vec![get(cmds::SYS_PLATFORM, 0, 7)];
        want.extend((0..inputs).map(|ch| set(cmds::ROUTE_PREAMP, ch, &db)));
        assert_eq!(sent, want);
        assert_eq!(out, Outcome::Ack);
    }
}

#[test]
fn wp_pgcust_set_preamp_refusal_in_the_middle_is_never_success() {
    let db = 3.0f32.to_le_bytes();
    let rp2350 = [0x01u8, 0x01, 0x17, 0x09, 0x01, 0x07, 0x01];
    let script = vec![
        data(&rp2350),
        WireAnswer::Ack,
        WireAnswer::Ack,
        WireAnswer::Ack,
        refused(VC_ERR_BUSY),
        WireAnswer::Ack,
    ];
    let (sent, out) = run(0x44, 0, LegacyBody::Out(&db), script);
    assert_eq!(sent.len(), 5, "stops at the refusal: {sent:?}");
    assert_eq!(
        out,
        Outcome::Stall {
            result: Some(VC_ERR_BUSY)
        }
    );
    // The first channel refused: nothing applied, as 1.1.6 ignoring it.
    let (_, out) = run(
        0x44,
        0,
        LegacyBody::Out(&db),
        vec![data(&rp2350), refused(VC_ERR_VALUE)],
    );
    assert_eq!(
        out,
        Outcome::Swallowed {
            result: Some(VC_ERR_VALUE)
        }
    );
    // An unknown platform: nothing written.
    let (sent, out) = run(0x44, 0, LegacyBody::Out(&db), vec![data(&[7, 1, 0x17, 9])]);
    assert_eq!(sent.len(), 1);
    assert_eq!(out, Outcome::Stall { result: None });
    // Fewer than 4 bytes was ignored.
    let (sent, out) = run(0x44, 0, LegacyBody::Out(&db[..3]), vec![]);
    assert!(sent.is_empty());
    assert_eq!(out, Outcome::Swallowed { result: None });
}

// ---------------------------------------------------------------------------
// GET_STATUS 0x50 (guide 6.3)

/// The vendored `GET_STATUS` row's data.
fn status_data() -> serde_json::Value {
    let Some(Adapter::Custom(c)) = legacy::op(0x50).unwrap().r#in else {
        panic!("GET_STATUS is custom");
    };
    serde_json::from_str(c.data).unwrap()
}

fn id_of(name: &serde_json::Value) -> CmdId {
    registry::by_name(name.as_str().unwrap()).unwrap().id
}

fn n(v: &serde_json::Value) -> u64 {
    v.as_u64().unwrap()
}

#[test]
fn wp_pgcust_status_table_equals_the_vendored_row() {
    let d = status_data();
    let mut listed = std::collections::BTreeSet::new();
    for (k, v) in d["fields"].as_object().unwrap() {
        let sel: u16 = k.parse().unwrap();
        let ty = match v[1].as_str().unwrap() {
            "u8" => Ty::U8,
            "u16" => Ty::U16,
            "i16" => Ty::I16,
            "u32" => Ty::U32,
            "i32" => Ty::I32,
            other => panic!("type {other}"),
        };
        let want = Sel::Field {
            id: id_of(&v[0]),
            ty,
            off: n(&v[2]) as u8,
        };
        assert_eq!(status_selector(sel), want, "selector {sel}");
        assert!(listed.insert(sel));
    }
    for (k, v) in d["peaks"].as_object().unwrap() {
        let sel: u16 = k.parse().unwrap();
        assert_eq!(
            status_selector(sel),
            Sel::Peaks(n(&v[0]) as u8, n(&v[1]) as u8)
        );
        assert!(listed.insert(sel));
    }
    for (k, v) in d["peak_cpu"].as_object().unwrap() {
        let sel: u16 = k.parse().unwrap();
        assert_eq!(
            status_selector(sel),
            Sel::PeakCpu {
                ch: n(&v[0]) as u8,
                id: id_of(&v[1]),
                lo: n(&v[2]) as u8,
                hi: n(&v[3]) as u8,
            }
        );
        assert!(listed.insert(sel));
    }
    for v in d["levels_raw"].as_array().unwrap() {
        assert_eq!(status_selector(n(v) as u16), Sel::Levels);
        assert!(listed.insert(n(v) as u16));
    }
    for v in d["zero"].as_array().unwrap() {
        assert_eq!(status_selector(n(v) as u16), Sel::Zero);
        assert!(listed.insert(n(v) as u16));
    }
    let absent: Vec<CmdId> = d["zero_if_absent"]
        .as_array()
        .unwrap()
        .iter()
        .map(id_of)
        .collect();
    assert_eq!(absent, ZERO_IF_ABSENT);
    // Guide 6.3: 0-33 are all listed; any other selector answers 4 zero bytes.
    assert_eq!(listed, (0..=33).collect());
    for sel in 34..=u16::MAX {
        assert_eq!(status_selector(sel), Sel::Zero, "selector {sel}");
    }
}

/// A struct whose byte k is k + `base`, so every offset is visible in the reply.
fn ramp(len: usize, base: u8) -> Vec<u8> {
    (0..len).map(|k| (k as u8).wrapping_add(base)).collect()
}

#[test]
fn wp_pgcust_get_status_every_selector_from_its_struct() {
    for sel in 0u16..=40 {
        let s = status_selector(sel);
        let (sent, out) = run(
            0x50,
            sel,
            LegacyBody::In { len: 4 },
            vec![data(&ramp(64, 0x10)), data(&ramp(64, 0x80))],
        );
        match s {
            Sel::Field { id, ty, off } => {
                let len = match registry::command(id).unwrap().get_len {
                    registry::GetLen::Fixed(n) => n,
                    _ => unreachable!(),
                };
                assert_eq!(sent, [get(id, 0, len)], "selector {sel}");
                let o = usize::from(off);
                let raw = ramp(64, 0x10);
                let v: u32 = match ty {
                    Ty::U8 => u32::from(raw[o]),
                    Ty::U16 => u32::from(u16::from_le_bytes([raw[o], raw[o + 1]])),
                    Ty::I16 => i16::from_le_bytes([raw[o], raw[o + 1]]) as i32 as u32,
                    Ty::U32 | Ty::I32 => u32::from_le_bytes(raw[o..o + 4].try_into().unwrap()),
                };
                assert_eq!(out, Outcome::Reply(v.to_le_bytes().to_vec()), "{sel}");
            }
            Sel::Zero => {
                assert!(sent.is_empty(), "selector {sel}");
                assert_eq!(out, Outcome::Reply(vec![0; 4]));
            }
            Sel::Peaks(a, b) => {
                assert_eq!(sent, [get(cmds::MEAS_LEVELS, 0, 2 * (u16::from(b) + 1))]);
                let r = ramp(64, 0x10);
                let (a, b) = (usize::from(a) * 2, usize::from(b) * 2);
                assert_eq!(out, Outcome::Reply(vec![r[a], r[a + 1], r[b], r[b + 1]]));
            }
            Sel::PeakCpu { .. } => {
                assert_eq!(
                    sent,
                    [
                        get(cmds::MEAS_LEVELS, 0, 10),
                        get(cmds::SYS_SYSTEM_STATUS, 0, 16)
                    ]
                );
                // Peak of channel 4 (bytes 8, 9), then CPU loads (bytes 12, 13).
                assert_eq!(out, Outcome::Reply(vec![0x18, 0x19, 0x8C, 0x8D]));
            }
            Sel::Levels => {
                assert_eq!(sent, [get(cmds::MEAS_LEVELS, 0, 4)]);
                assert_eq!(out, Outcome::Reply(ramp(4, 0x10)));
            }
        }
    }
}

#[test]
fn wp_pgcust_get_status_guide_examples() {
    // SYS.SYSTEM_STATUS: clk_sys 307.2 MHz, 48 kHz, 1200 mV, -12.34 C, loads 40/55.
    let ss = [
        &307_200_000u32.to_le_bytes()[..],
        &48_000u32.to_le_bytes(),
        &1200u16.to_le_bytes(),
        &(-1234i16).to_le_bytes(),
        &[40, 55, 0, 0],
    ]
    .concat();
    // Selector 15 is the sample rate at offset 4, not the clk_sys a 4-byte read gives.
    let (sent, out) = run(0x50, 15, LegacyBody::In { len: 4 }, vec![data(&ss)]);
    assert_eq!(sent, [get(cmds::SYS_SYSTEM_STATUS, 0, 16)]);
    assert_eq!(out, Outcome::Reply(48_000u32.to_le_bytes().to_vec()));
    // 16: temperature sign-extended; 14: core voltage zero-extended.
    let (_, out) = run(0x50, 16, LegacyBody::In { len: 4 }, vec![data(&ss)]);
    assert_eq!(out, Outcome::Reply((-1234i32).to_le_bytes().to_vec()));
    let (_, out) = run(0x50, 14, LegacyBody::In { len: 4 }, vec![data(&ss)]);
    assert_eq!(out, Outcome::Reply(vec![0xB0, 0x04, 0, 0]));
    // 9: MEAS.LEVELS is byte for byte the old combined packet (41 bytes on RP2350).
    let levels = ramp(41, 1);
    let p = plan(0x50, 9, LegacyBody::In { len: 41 }, &full()).unwrap();
    assert_eq!(p.single(), Some(&get(cmds::MEAS_LEVELS, 0, 41)));
    assert_eq!(run_plan(p, vec![data(&levels)]).1, Outcome::Reply(levels));
    // 2: the CPU loads come from SYS.SYSTEM_STATUS bytes 12 and 13; a refusal of
    // that second read raises.
    let (sent, out) = run(
        0x50,
        2,
        LegacyBody::In { len: 4 },
        vec![data(&ramp(41, 0)), refused(VC_ERR_BUSY)],
    );
    assert_eq!(sent.len(), 2);
    assert_eq!(
        out,
        Outcome::Stall {
            result: Some(VC_ERR_BUSY)
        }
    );
    // A short struct reply is not a value.
    let (_, out) = run(0x50, 15, LegacyBody::In { len: 4 }, vec![data(&ss[..6])]);
    assert_eq!(out, Outcome::Stall { result: None });
    // A 2-byte read of a scalar gets the prefix of the old 4 bytes.
    let (_, out) = run(0x50, 15, LegacyBody::In { len: 2 }, vec![data(&ss)]);
    assert_eq!(out, Outcome::Reply(48_000u32.to_le_bytes()[..2].to_vec()));
}

#[test]
fn wp_pgcust_get_status_diag_selectors_on_release_and_bench_builds() {
    for sel in [24u16, 25] {
        // A release build does not list DIAG: nothing sent, 4 zero bytes as 1.1.6.
        let p = plan(0x50, sel, LegacyBody::In { len: 4 }, &release()).unwrap();
        assert!(p.single().is_none());
        let (sent, out) = run_plan(p, vec![]);
        assert!(sent.is_empty());
        assert_eq!(out, Outcome::Reply(vec![0; 4]));
        // Listed but answering VC_ERR_UNKNOWN: 0 too (guide 6.3).
        let (sent, out) = run(
            0x50,
            sel,
            LegacyBody::In { len: 4 },
            vec![refused(VC_ERR_UNKNOWN)],
        );
        assert_eq!(sent, [get(cmds::DIAG_LOOPBACK_STATS, 0, 8)]);
        assert_eq!(out, Outcome::Reply(vec![0; 4]));
        // Any other refusal is raised.
        let (_, out) = run(
            0x50,
            sel,
            LegacyBody::In { len: 4 },
            vec![refused(VC_ERR_BUSY)],
        );
        assert_eq!(
            out,
            Outcome::Stall {
                result: Some(VC_ERR_BUSY)
            }
        );
        // A DSPI_LOOPBACK build answers the counters.
        let counts = [&7u32.to_le_bytes()[..], &9u32.to_le_bytes()].concat();
        let (_, out) = run(0x50, sel, LegacyBody::In { len: 4 }, vec![data(&counts)]);
        let want = if sel == 24 { 7u32 } else { 9 };
        assert_eq!(out, Outcome::Reply(want.to_le_bytes().to_vec()));
    }
    // Other absent structs are refused locally like any absent command.
    let no_pdm = PageSet::vendored(|c| c.id != cmds::SYS_PDM_STATS);
    assert_eq!(
        plan(0x50, 3, LegacyBody::In { len: 4 }, &no_pdm).unwrap_err(),
        PlanError::Absent {
            id: cmds::SYS_PDM_STATS,
            dir: PagedDir::Get
        }
    );
}

// ---------------------------------------------------------------------------
// SAVE_PARAMS 0x51 (guide 6.7)

#[test]
fn wp_pgcust_save_params_saves_the_active_slot() {
    let (sent, out) = run(
        0x51,
        0,
        LegacyBody::In { len: 1 },
        vec![data(&[3]), WireAnswer::Ack],
    );
    assert_eq!(
        sent,
        [
            get(cmds::STORE_PRESET_ACTIVE, 0, 1),
            set(cmds::STORE_PRESET_SAVE, 3, &[])
        ]
    );
    // STORE.PRESET_SAVE, slot 3: a zero-length SET (guide 6.6 pattern).
    assert_eq!(
        sent[1].setup_packet(),
        [0x41, 0x10, 0x03, 0, 0x02, 0x02, 0, 0]
    );
    assert_eq!(out, Outcome::Reply(vec![0x00]));
    // The save refused: never FLASH_OK.
    let (_, out) = run(
        0x51,
        0,
        LegacyBody::In { len: 1 },
        vec![data(&[3]), refused(VC_ERR_STATE)],
    );
    assert_eq!(
        out,
        Outcome::Stall {
            result: Some(VC_ERR_STATE)
        }
    );
    let (sent, out) = run(
        0x51,
        0,
        LegacyBody::In { len: 1 },
        vec![refused(VC_ERR_BUSY)],
    );
    assert_eq!(sent.len(), 1);
    assert_eq!(
        out,
        Outcome::Stall {
            result: Some(VC_ERR_BUSY)
        }
    );
}

// ---------------------------------------------------------------------------
// SET_MATRIX_ROUTE 0x70 and GET_MATRIX_ROUTE 0x71 (guide 6.5)

#[test]
fn wp_pgcust_matrix_route() {
    let gain = (-6.0f32).to_le_bytes();
    // Old packet: input 1, output 4, enabled, phase invert, -6 dB (sent with wValue 0).
    let p = [&[1u8, 4, 1, 1][..], &gain].concat();
    let pl = plan(0x70, 0, LegacyBody::Out(&p), &full()).unwrap();
    let want = [&[1u8, 1, 0, 0][..], &gain].concat();
    assert_eq!(
        pl.single(),
        Some(&set(cmds::ROUTE_MATRIX_ROUTE, 0x0104, &want))
    );
    assert_eq!(
        pl.single().unwrap().setup_packet(),
        [0x41, 0x10, 0x04, 0x01, 0x02, 0x05, 8, 0]
    );
    let (_, out) = run(0x70, 0, LegacyBody::Out(&p), vec![refused(VC_ERR_ADDRESS)]);
    assert_eq!(
        out,
        Outcome::Swallowed {
            result: Some(VC_ERR_ADDRESS)
        }
    );
    let (sent, out) = run(0x70, 0, LegacyBody::Out(&p[..7]), vec![]);
    assert!(sent.is_empty());
    assert_eq!(out, Outcome::Swallowed { result: None });
    // GET: the old reply rebuilt as input, output, r[0], r[1], r[4..8].
    let r = [&[1u8, 0, 0, 0][..], &gain].concat();
    let (sent, out) = run(0x71, 0x0104, LegacyBody::In { len: 8 }, vec![data(&r)]);
    assert_eq!(sent, [get(cmds::ROUTE_MATRIX_ROUTE, 0x0104, 8)]);
    assert_eq!(out, Outcome::Reply([&[1u8, 4, 1, 0][..], &gain].concat()));
    let (_, out) = run(
        0x71,
        0x0909,
        LegacyBody::In { len: 8 },
        vec![refused(VC_ERR_ADDRESS)],
    );
    assert_eq!(
        out,
        Outcome::Stall {
            result: Some(VC_ERR_ADDRESS)
        }
    );
}

// ---------------------------------------------------------------------------
// CLEAR_CLIPS 0x83 (guide 6.5)

#[test]
fn wp_pgcust_clear_clips_reads_then_clears() {
    let flags = 0x0001_0005u32.to_le_bytes();
    let (sent, out) = run(
        0x83,
        0,
        LegacyBody::In { len: 4 },
        vec![data(&flags), WireAnswer::Ack],
    );
    assert_eq!(
        sent,
        [
            get(cmds::MEAS_CLIP_FLAGS, 0, 4),
            set(cmds::MEAS_CLIP_CLEAR, 0, &[])
        ]
    );
    assert_eq!(sent[1].setup_packet(), [0x41, 0x12, 0, 0, 0x02, 0x09, 0, 0]);
    assert_eq!(out, Outcome::Reply(flags.to_vec()));
    // The clear refused after the read: the flags are not reported as cleared.
    let (_, out) = run(
        0x83,
        0,
        LegacyBody::In { len: 4 },
        vec![data(&flags), refused(VC_ERR_BUSY)],
    );
    assert_eq!(
        out,
        Outcome::Stall {
            result: Some(VC_ERR_BUSY)
        }
    );
    // Absent clear: refused before anything is sent.
    let no_clear = PageSet::vendored(|c| c.id != cmds::MEAS_CLIP_CLEAR);
    assert_eq!(
        plan(0x83, 0, LegacyBody::In { len: 4 }, &no_clear).unwrap_err(),
        PlanError::Absent {
            id: cmds::MEAS_CLIP_CLEAR,
            dir: PagedDir::Set
        }
    );
}

// ---------------------------------------------------------------------------
// PRESET_GET_STARTUP 0x97 (guide 6.5)

#[test]
fn wp_pgcust_preset_get_startup() {
    // STORE.DIRECTORY, 7 bytes: byte 4 is the last active slot (5).
    let dir = [0xFF, 0x03, 1, 2, 5, 0, 0];
    let (sent, out) = run(
        0x97,
        0,
        LegacyBody::In { len: 3 },
        vec![data(&[1, 2]), data(&dir)],
    );
    assert_eq!(
        sent,
        [
            get(cmds::STORE_STARTUP, 0, 2),
            get(cmds::STORE_DIRECTORY, 0, 7)
        ]
    );
    assert_eq!(out, Outcome::Reply(vec![1, 2, 5]));
    let (_, out) = run(
        0x97,
        0,
        LegacyBody::In { len: 3 },
        vec![data(&[1, 2]), refused(VC_ERR_BUSY)],
    );
    assert_eq!(
        out,
        Outcome::Stall {
            result: Some(VC_ERR_BUSY)
        }
    );
    let p = plan(0x97, 0, LegacyBody::In { len: 2 }, &full()).unwrap();
    assert_eq!(p.single(), Some(&get(cmds::STORE_STARTUP, 0, 2)));
}

// ---------------------------------------------------------------------------
// SET_ALL_PARAMS 0xA1 (guide 6.5, 9)

#[test]
fn wp_pgcust_set_all_params_length_rules() {
    let blob = vec![0x5A; 6136];
    let p = plan(0xA1, 0, LegacyBody::Out(&blob), &full()).unwrap();
    assert_eq!(p.single(), Some(&set(cmds::STORE_ALL_PARAMS, 0, &blob)));
    assert_eq!(
        p.single().unwrap().setup_packet(),
        [0x41, 0x40, 0, 0, 0x02, 0x02, 0xF8, 0x17]
    );
    assert_eq!(run_plan(p, vec![WireAnswer::Ack]).1, Outcome::Ack);
    // A header the apply refuses STALLs now: raised, never reported as applied.
    let (_, out) = run(0xA1, 0, LegacyBody::Out(&blob), vec![refused(VC_ERR_VALUE)]);
    assert_eq!(
        out,
        Outcome::Stall {
            result: Some(VC_ERR_VALUE)
        }
    );
    // 1.1.6: 64 bytes or fewer ACKed and ignored; other wrong lengths STALLed.
    for (len, want) in [
        (0usize, Outcome::Swallowed { result: None }),
        (64, Outcome::Swallowed { result: None }),
        (65, Outcome::Stall { result: None }),
        (6135, Outcome::Stall { result: None }),
        (6137, Outcome::Stall { result: None }),
    ] {
        let (sent, out) = run(0xA1, 0, LegacyBody::Out(&vec![0; len]), vec![]);
        assert!(sent.is_empty(), "{len}");
        assert_eq!(out, want, "{len}");
    }
}

// ---------------------------------------------------------------------------
// GET_USB_ERROR_STATS 0xB2, RESET_USB_ERROR_STATS 0xB3 (guide 6.7)

#[test]
fn wp_pgcust_usb_error_stats_are_local() {
    let (sent, out) = run(0xB2, 0, LegacyBody::In { len: 64 }, vec![]);
    assert!(sent.is_empty());
    assert_eq!(out, Outcome::Reply(vec![0; 24]));
    let (sent, out) = run(0xB3, 0, LegacyBody::In { len: 1 }, vec![]);
    assert!(sent.is_empty());
    assert_eq!(out, Outcome::Reply(vec![0x01]));
    assert!(reaches(0xB2, 0, UsbDir::In).is_empty());
}

// ---------------------------------------------------------------------------
// GET_INPUT_RATE 0xEE (guide 6.5)

#[test]
fn wp_pgcust_get_input_rate() {
    let mut ss = vec![0u8; 16];
    ss[4..8].copy_from_slice(&48_000u32.to_le_bytes());
    let i2s = 96_000u32.to_le_bytes();
    let (sent, out) = run(
        0xEE,
        0,
        LegacyBody::In { len: 8 },
        vec![data(&ss), data(&i2s)],
    );
    assert_eq!(
        sent,
        [
            get(cmds::SYS_SYSTEM_STATUS, 0, 16),
            get(cmds::INPUT_I2S_RX_RATE, 0, 4)
        ]
    );
    assert_eq!(
        out,
        Outcome::Reply([&48_000u32.to_le_bytes()[..], &i2s].concat())
    );
    let (_, out) = run(
        0xEE,
        0,
        LegacyBody::In { len: 8 },
        vec![data(&ss), refused(VC_ERR_BUSY)],
    );
    assert_eq!(
        out,
        Outcome::Stall {
            result: Some(VC_ERR_BUSY)
        }
    );
}

// ---------------------------------------------------------------------------
// The reverse view

#[test]
fn wp_pgcust_reverse_of_one_request_rows_is_exact() {
    let p18 = eq_packet18();
    let gain = (-6.0f32).to_le_bytes();
    let matrix = [&[1u8, 4, 1, 1][..], &gain].concat();
    let blob = vec![7u8; 6136];
    let cases: Vec<(u8, u16, LegacyBody<'_>)> = vec![
        (0x42, 0, LegacyBody::Out(&p18)),
        (0x43, 0x0219, LegacyBody::In { len: 4 }),
        (0x50, 9, LegacyBody::In { len: 41 }),
        (0x50, 15, LegacyBody::In { len: 4 }),
        (0x50, 24, LegacyBody::In { len: 4 }),
        (0x70, 0, LegacyBody::Out(&matrix)),
        (0x71, 0x0104, LegacyBody::In { len: 8 }),
        (0xA1, 0, LegacyBody::Out(&blob)),
    ];
    for (op, value, body) in cases {
        let p = plan(op, value, body, &full()).unwrap();
        let r = p.single().unwrap().clone();
        let payload = match &r.body {
            PagedBody::Set(d) => d.clone(),
            PagedBody::Get { .. } => Vec::new(),
        };
        let eq = legacy_equivalent(r.id, r.dir(), r.value, &payload)
            .unwrap_or_else(|| panic!("0x{op:02X}: no reverse for {r:?}"));
        let body2 = match eq.dir {
            UsbDir::In => LegacyBody::In { len: 64 },
            UsbDir::Out => LegacyBody::Out(&eq.data),
        };
        let p2 = plan(eq.opcode, eq.value, body2, &full()).unwrap();
        let r2 = p2.single().unwrap();
        assert_eq!((r2.id, r2.value), (r.id, r.value), "0x{op:02X}");
        if let (PagedBody::Set(a), PagedBody::Set(b)) = (&r2.body, &r.body) {
            assert_eq!(a, b);
        }
    }
    // The 18-byte packet comes back whole.
    let eq = legacy_equivalent(
        cmds::FILTER_BAND,
        PagedDir::Set,
        0x0203,
        &[&[1u8, 0, 0x00, 0x02][..], &p18[4..16]].concat(),
    )
    .unwrap();
    assert_eq!((eq.opcode, eq.dir, eq.data), (0x42, UsbDir::Out, p18));
}

#[test]
fn wp_pgcust_reverse_of_multi_request_rows() {
    let cases = [
        (cmds::CS_DISPLAY_CAPS, PagedDir::Get, 0x28u8, false),
        (cmds::CS_DISPLAY_CONFIG, PagedDir::Get, 0x28, false),
        (cmds::MEAS_CLIP_CLEAR, PagedDir::Set, 0x83, true),
        (cmds::STORE_STARTUP, PagedDir::Get, 0x97, false),
        (cmds::INPUT_I2S_RX_RATE, PagedDir::Get, 0xEE, false),
    ];
    for (id, dir, op, action) in cases {
        let eq = legacy_equivalent(id, dir, 0, &[]).unwrap_or_else(|| panic!("{id:?}"));
        assert_eq!((eq.opcode, eq.dir, eq.action), (op, UsbDir::In, action));
        assert!(reaches(eq.opcode, eq.value, eq.dir).contains(&(id, dir)));
        // Only the request the recipe sends.
        assert_eq!(legacy_equivalent(id, dir, 1, &[]), None, "{id:?}");
    }
    // The one-to-one rows still win where they exist.
    let eq = legacy_equivalent(cmds::STORE_PRESET_SAVE, PagedDir::Set, 3, &[]).unwrap();
    assert_eq!((eq.opcode, eq.value), (0x90, 3));
    // No equivalent: a pure read of the clip flags (CLEAR_CLIPS also clears), a
    // SET with reserved bytes 1.1.6 could not send, a bulk of the wrong length.
    assert_eq!(
        legacy_equivalent(cmds::MEAS_CLIP_FLAGS, PagedDir::Get, 0, &[]),
        None
    );
    assert_eq!(
        legacy_equivalent(
            cmds::ROUTE_MATRIX_ROUTE,
            PagedDir::Set,
            0x0104,
            &[1, 0, 1, 0, 0, 0, 0, 0]
        ),
        None
    );
    assert_eq!(
        legacy_equivalent(cmds::STORE_ALL_PARAMS, PagedDir::Set, 0, &[0; 100]),
        None
    );
    assert_eq!(
        legacy_equivalent(cmds::FILTER_BAND, PagedDir::Get, 0x0220, &[]),
        None,
        "band 32 has no 5-bit old field"
    );
}

// ---------------------------------------------------------------------------
// Properties

fn custom_index() -> impl Strategy<Value = usize> {
    0..CUSTOM_OPS.len()
}

fn answer() -> impl Strategy<Value = WireAnswer> {
    prop_oneof![
        Just(WireAnswer::Ack),
        proptest::collection::vec(any::<u8>(), 0..48).prop_map(WireAnswer::Data),
        proptest::option::of(0x10u8..0x1B).prop_map(|result| WireAnswer::Refused { result }),
    ]
}

/// Plausible answers for a request: the registry's reply length for a GET.
fn fitting(r: &PagedRequest, fill: u8) -> WireAnswer {
    match r.body {
        PagedBody::Get { len } => {
            let n = match registry::command(r.id).map(|c| c.get_len) {
                Some(registry::GetLen::Fixed(n)) => n.min(len),
                _ => len,
            };
            // A plausible platform byte for SYS.PLATFORM.
            let mut d = vec![fill; usize::from(n)];
            if r.id == cmds::SYS_PLATFORM {
                d[0] = fill & 1;
            }
            WireAnswer::Data(d)
        }
        PagedBody::Set(_) => WireAnswer::Ack,
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2048, ..ProptestConfig::default() })]

    /// Whatever the caller asks and the device answers, a custom recipe never
    /// panics, finishes within a bound, sends only well-formed requests the device
    /// lists, a one-request plan finishes on its one answer, and a refusal is
    /// never answered as an ACK.
    #[test]
    fn proptest_wp_pgcust_recipes_are_well_formed(
        i in custom_index(), value in any::<u16>(), len in 0u16..300,
        payload in proptest::collection::vec(any::<u8>(), 0..40),
        exact_eq in any::<bool>(),
        answers in proptest::collection::vec(answer(), 0..12),
        release_build in any::<bool>(),
    ) {
        let op = CUSTOM_OPS[i];
        let pages = if release_build { release() } else { full() };
        let o = legacy::op(op).unwrap();
        let blob;
        let body = if o.out.is_some() {
            blob = if op == 0xA1 && exact_eq { vec![0; 6136] } else { payload.clone() };
            LegacyBody::Out(&blob)
        } else {
            LegacyBody::In { len }
        };
        let Ok(mut p) = plan(op, value, body, &pages) else { return Ok(()); };
        let one = p.single().cloned();
        let mut next = p.start();
        if let Some(r) = &one {
            prop_assert_eq!(&next, &Next::Send(r.clone()));
        }
        let mut answers: VecDeque<_> = answers.into();
        let mut steps = 0;
        let mut refused_any = false;
        while let Next::Send(r) = next {
            steps += 1;
            prop_assert!(steps <= 10, "0x{op:02X} does not finish");
            prop_assert!(!r.id.is_legacy());
            prop_assert!(pages.contains(r.id, r.dir()), "{r:?} not listed");
            let c = registry::command(r.id).unwrap();
            match &r.body {
                PagedBody::Get { len } => prop_assert!(*len > 0),
                PagedBody::Set(d) => prop_assert!(c.set_len_ok(d.len()), "{r:?}"),
            }
            let a = answers.pop_front().unwrap_or(WireAnswer::Ack);
            refused_any |= matches!(a, WireAnswer::Refused { .. });
            next = p.feed(a.as_answer());
            if one.is_some() {
                prop_assert!(matches!(next, Next::Done(_)), "one-step plan sent again");
            }
        }
        if refused_any {
            let Next::Done(out) = &next else { unreachable!() };
            prop_assert!(!matches!(out, Outcome::Ack), "0x{op:02X}: a refusal answered as ACK");
        }
    }

    /// A refusal after a SET was accepted is always raised (never success, never
    /// swallowed): no half-applied write is reported as applied.
    #[test]
    fn proptest_wp_pgcust_partial_writes_are_raised(
        i in custom_index(), value in any::<u16>(), fill in any::<u8>(),
        refuse_at in 1usize..10, code in 0x10u8..0x1B,
        payload in proptest::collection::vec(any::<u8>(), 16..20),
    ) {
        let op = CUSTOM_OPS[i];
        let pages = full();
        let o = legacy::op(op).unwrap();
        let body = if o.out.is_some() { LegacyBody::Out(&payload) } else { LegacyBody::In { len: 64 } };
        let Ok(mut p) = plan(op, value, body, &pages) else { return Ok(()); };
        let mut next = p.start();
        let mut n = 0;
        let mut acked_set = false;
        let mut refused_after_set = false;
        while let Next::Send(r) = next {
            n += 1;
            prop_assert!(n <= 10);
            let a = if n == refuse_at {
                refused_after_set = acked_set;
                WireAnswer::Refused { result: Some(code) }
            } else {
                fitting(&r, fill)
            };
            if matches!(r.body, PagedBody::Set(_)) && matches!(a, WireAnswer::Ack) {
                acked_set = true;
            }
            next = p.feed(a.as_answer());
        }
        if refused_after_set {
            prop_assert_eq!(next, Next::Done(Outcome::Stall { result: Some(code) }), "0x{:02X}", op);
        }
    }

    /// The custom reverse view never panics and only names requests whose plan
    /// reaches the command.
    #[test]
    fn proptest_wp_pgcust_reverse_reaches_the_command(
        k in 0usize..registry::COMMANDS.len(), set in any::<bool>(), value in any::<u16>(),
        payload in proptest::collection::vec(any::<u8>(), 0..20),
    ) {
        let id = registry::COMMANDS[k].id;
        let dir = if set { PagedDir::Set } else { PagedDir::Get };
        if let Some(eq) = super::legacy_equivalent(id, dir, value, &payload) {
            prop_assert!(reaches(eq.opcode, eq.value, eq.dir).contains(&(id, dir)), "{eq:?}");
            prop_assert!(CUSTOM_OPS.contains(&eq.opcode));
        }
    }
}
