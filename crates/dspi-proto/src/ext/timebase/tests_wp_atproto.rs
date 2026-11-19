//! WP ATPROTO: the time-base codecs against the firmware's normative byte
//! examples (`extension_channel_spec.md` 2.8.1, 2.8.7.1, 2.10, 2.11;
//! `alink_protocol_spec.md` 6.1, 12.2; FWTB1's revision of the specs,
//! `DSPi-wt/int-fwb` eec48ba), and FWSP1's vector file as a third oracle.
#![allow(clippy::arithmetic_side_effects)]

use super::super::alink::{
    self, AlinkInfo, ApplyOp, ApplyRequest, ApplyResult, ApplyResultEvent, ApplyStatus, feature,
};
use super::super::alink_usb;
use super::*;
use proptest::prelude::*;

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn wp_atproto_the_alink_info_example_has_siggen_sync() {
    // 2.8.1 (FWTB1): `... 63 00 00 00 70 00 00 04 ...`, bit 6 since FWTB1.
    let mut raw = hex("01 01 07 01 40 08 00 00 00 ff 00 00 63 00 00 00 70 00 00 04");
    raw.resize(AlinkInfo::SIZE, 0);
    let info = AlinkInfo::decode(&raw).unwrap();
    assert_eq!(info.feature_bits, 0x63);
    assert_ne!(info.feature_bits & feature::SIGGEN_SYNC, 0);
    assert_eq!(feature::SIGGEN_SYNC, FEATURE_SIGGEN_SYNC);
    assert_eq!(
        info.feature_bits,
        feature::PHASE_LOCK | feature::APPLY_AT_FRAME | feature::LOS_PIN | feature::SIGGEN_SYNC
    );
    assert_eq!(info.encode().to_vec(), raw);
}

#[test]
fn wp_atproto_op_9_on_the_usb_time_base_encodes_as_the_example() {
    // 2.8.7.1: op 9 at S = 80 012 000, time base 1.
    let raw = hex("e0 e2 c4 04 00 00 00 00 09 00 00 00 01 00 00 00");
    let req = ApplyRequest {
        frame: 80_012_000,
        op: ApplyOp::SiggenStart,
        arg: 0,
        value: 0,
    };
    assert_eq!(encode_apply_tb(&req, TB_USB_SAMPLE).to_vec(), raw);
    assert_eq!(decode_apply_tb(&raw), Some((req, TB_USB_SAMPLE)));
    // The same bytes as the USB block's own encoder.
    assert_eq!(
        alink_usb::encode_apply(&req, alink_usb::TimeBase::UsbSample).to_vec(),
        raw
    );
    assert!(apply_form_valid(&req, TB_USB_SAMPLE));
    // The started event: `02 e2 00 <seq> 09 00 00 00 64 07 20 00`, bus
    // frame 1892, sample 32.
    let ev = hex("02 e2 00 07 09 00 00 00 64 07 20 00");
    let e = ApplyResultEvent::decode(&ev[4..]).unwrap();
    assert_eq!(
        (e.op, e.result),
        (ApplyOp::SiggenStart, ApplyResult::Applied)
    );
    let (bus, k) = alink_usb::bus_of_sample(80_012_000, 48).unwrap();
    assert_eq!((bus, k), (1892, 32));
    assert_eq!(e.frame_low, u32::from(bus) | (u32::from(k) << 16));
    assert_eq!(e.encode().to_vec(), ev[4..].to_vec());
}

#[test]
fn wp_atproto_ops_9_and_10_are_valid_up_to_max_and_never_in_the_bus_frame_form() {
    assert_eq!(ApplyOp::MAX, 10);
    assert_eq!(ApplyOp::MAX, APPLY_OP_MAX);
    for op in 0..=10 {
        assert!(alink::superframe_op_valid(op), "{op}");
    }
    assert!(!alink::superframe_op_valid(11));
    let start = |frame| ApplyRequest {
        frame,
        op: ApplyOp::SiggenStart,
        arg: 0,
        value: 0,
    };
    let stop = |arg| ApplyRequest {
        frame: 5,
        op: ApplyOp::SiggenStop,
        arg,
        value: 0,
    };
    for tb in [TB_ALINK_LINE, TB_USB_SAMPLE, TB_SPDIF] {
        assert!(apply_form_valid(&start(5), tb), "{tb}");
        assert!(apply_form_valid(&stop(0), tb), "{tb}");
        assert!(apply_form_valid(&stop(1), tb), "{tb}");
    }
    // R5: op 9 never as a bus frame; op 10 may.
    assert!(!apply_form_valid(&start(5), TB_USB_BUS_FRAME));
    assert!(apply_form_valid(&stop(1), TB_USB_BUS_FRAME));
    // Arguments: op 9 arg 0 value 0; op 10 arg 0..1 value 0.
    assert!(!apply_form_valid(
        &ApplyRequest { arg: 1, ..start(5) },
        TB_USB_SAMPLE
    ));
    assert!(!apply_form_valid(
        &ApplyRequest {
            value: 1,
            ..start(5)
        },
        TB_USB_SAMPLE
    ));
    assert!(!apply_form_valid(&stop(2), TB_USB_SAMPLE));
    // Op 6 is the sender's, 11 unknown, time base 4 and 0xFF not writable.
    let rate = ApplyRequest {
        frame: 5,
        op: ApplyOp::Rate,
        arg: 1,
        value: 0,
    };
    assert!(!apply_form_valid(&rate, TB_ALINK_LINE));
    let other = ApplyRequest {
        op: ApplyOp::from_raw(11),
        ..stop(0)
    };
    assert!(!apply_form_valid(&other, TB_ALINK_LINE));
    assert!(!apply_form_valid(&stop(0), 4));
    assert!(!apply_form_valid(&stop(0), TB_NONE));
}

#[test]
fn wp_atproto_the_op_9_lead_window() {
    // 150 ms of arm lead at 48 kHz is 7200 samples; at most 120 s ahead.
    assert_eq!(SIGGEN_START_MAX_LEAD_MS, 120_000);
    let now = 1_000_000;
    assert!(siggen_start_lead_ok(now, now + 7200, 150, 48_000));
    assert!(!siggen_start_lead_ok(now, now + 7199, 150, 48_000));
    assert!(siggen_start_lead_ok(now, now + 5_760_000, 150, 48_000));
    assert!(!siggen_start_lead_ok(now, now + 5_760_001, 150, 48_000));
    assert!(!siggen_start_lead_ok(now, now - 1, 150, 48_000));
    assert!(!siggen_start_lead_ok(u64::MAX, 0, 150, 48_000));
}

#[test]
fn wp_atproto_the_alink_spdif_examples() {
    // 2.10.1: follower, auto re-anchor, D 192, mute on lock error, locked.
    let raw = hex("01 01 c0 00 00 00 00 03");
    let f = AlinkSpdif::decode(&raw).unwrap();
    assert_eq!(
        f,
        AlinkSpdif {
            mode: SpdifMode::Follower,
            flags: SPDIF_FLAG_AUTO_REANCHOR,
            d: 192,
            on_lock_error: SPDIF_ON_LOCK_ERROR_MUTE,
            tx_mask: 0,
            state: ContractState::Locked,
        }
    );
    assert_eq!(f.encode().to_vec(), raw);
    assert_eq!(f.encode_out().to_vec(), hex("01 01 c0 00 00 00 00 00"));
    assert!(f.is_valid());
    // Source on slot 0 only, outputs running.
    let raw = hex("02 01 00 00 00 00 01 03");
    let s = AlinkSpdif::decode(&raw).unwrap();
    assert_eq!(
        (s.mode, s.tx_mask, s.d, s.state),
        (SpdifMode::Source, 1, 0, ContractState::Locked)
    );
    assert_eq!(s.encode().to_vec(), raw);
    assert!(s.is_valid());
    // The host recipe's writes (2.10.5).
    assert_eq!(
        AlinkSpdif {
            mode: SpdifMode::Source,
            d: 0,
            tx_mask: 0x01,
            ..AlinkSpdif::DEFAULT
        }
        .encode_out()
        .to_vec(),
        hex("02 01 00 00 00 00 01 00")
    );
    assert_eq!(
        AlinkSpdif::DEFAULT.encode().to_vec(),
        hex("00 01 c0 00 00 00 00 00")
    );
    // What the unit refuses on its own.
    let bad = [
        AlinkSpdif {
            mode: SpdifMode::Other(3),
            ..f
        },
        AlinkSpdif { flags: 0x03, ..f },
        AlinkSpdif {
            on_lock_error: 2,
            ..f
        },
        AlinkSpdif { tx_mask: 1, ..f },
    ];
    for b in bad {
        assert!(!b.is_valid(), "{b:?}");
    }
    // A short reply is None; extra bytes are ignored.
    assert_eq!(AlinkSpdif::decode(&raw[..7]), None);
    let mut long = raw.clone();
    long.push(0xEE);
    assert_eq!(AlinkSpdif::decode(&long), Some(s));
}

#[test]
fn wp_atproto_the_spdif_status_example() {
    // 2.10.2: a locked follower.
    let raw = hex(
        "03 02 01 03 42 37 03 00 00 a6 0e 00 00 00 00 00 00 00 80 3e 00 00 48 41 96 00 8f 01 60 00 00 00 \
         02 00 00 00 01 00 00 00 05 00 00 00 01 00 00 00",
    );
    let s = AlinkSpdifStatus::decode(&raw).unwrap();
    assert_eq!(s.state, ContractState::Locked);
    assert_eq!((s.phase_state, s.rate_code), (2, 1));
    assert!(s.has(spdif_flag::SUPERFRAME) && s.has(spdif_flag::TIME_BASE));
    assert!(!s.has(spdif_flag::HOLDOVER) && !s.has(spdif_flag::SOURCE));
    assert_eq!((s.source_id, s.epoch, s.frame), (0x3742, 3, 960_000));
    assert_eq!((s.phase_error, s.vcxo_ppm), (0.25, 12.5));
    assert_eq!((s.d_min, s.d_max, s.fill), (150, 399, 96));
    assert_eq!(
        (s.lock_count, s.loss_count, s.sf_errors, s.reanchors),
        (2, 1, 5, 1)
    );
    assert_eq!(s.encode().to_vec(), raw);
    assert_eq!(AlinkSpdifStatus::decode(&raw[..47]), None);
}

#[test]
fn wp_atproto_the_spdif_ctrl_example() {
    // 2.10.3: a broadcast SIGGEN_STOP now at F = 1 000 000.
    let raw = hex("40 42 0f 00 00 00 00 00 0a 01 00 00 03 00 00 00");
    let req = ApplyRequest {
        frame: 1_000_000,
        op: ApplyOp::SiggenStop,
        arg: 1,
        value: 0,
    };
    assert_eq!(encode_spdif_ctrl(&req).to_vec(), raw);
    assert_eq!(decode_apply_tb(&raw), Some((req, TB_SPDIF)));
    assert!(spdif_ctrl_valid(&req));
    // Once applied.
    let st = hex("00 0a 00 00 40 42 0f 00 00 00 00 00 00 00 00 00");
    let s = decode_spdif_ctrl_status(&st).unwrap();
    assert_eq!(
        s,
        ApplyStatus {
            pending: 0,
            last_op: ApplyOp::SiggenStop,
            last_result: ApplyResult::Applied,
            last_frame: 1_000_000,
        }
    );
    assert_eq!(s.encode().to_vec(), st);
    // Ops never carried on S/PDIF, and CANCEL_ALL, are refused.
    for op in [0u8, 3, 5, 6, 8, 11] {
        let r = ApplyRequest {
            frame: 1_000_000,
            op: ApplyOp::from_raw(op),
            arg: 0,
            value: 0,
        };
        assert!(!spdif_ctrl_valid(&r), "op {op}");
    }
    for op in SPDIF_APPLY_OPS {
        assert!(spdif_op_allowed(op));
    }
    // The 896-frame lead (FWSP1's lead vectors).
    for (now, target, ok) in [
        (1000u64, 1896u64, true),
        (1000, 1895, false),
        (1000, 1000, false),
        (1000, 999, false),
        (0, 896, true),
        (1 << 40, (1 << 40) + 10_000, true),
    ] {
        assert_eq!(spdif_ctrl_lead_ok(now, target), ok, "{now} {target}");
    }
}

#[test]
fn wp_atproto_the_time_base_example() {
    // 2.11.1: a USB-direct follower that is also an S/PDIF source.
    let raw = hex("01 02 00 00 00 e1 f5 05 00 3e 49 00 \
         01 00 03 01 00 00 00 01 00 b4 c4 04 00 00 00 00 01 00 c0 00 00 00 00 3e \
         03 01 03 0b 42 37 03 03 00 a6 0e 00 00 00 00 00 04 00 00 00 00 00 00 00 \
         00 00 00 00");
    assert_eq!(raw.len(), TIME_BASE_LEN);
    let r = TimeBaseRead::decode(&raw).unwrap();
    assert_eq!((r.time_us, r.output_frames), (100_000_000, 4_800_000));
    assert_eq!(r.entries.len(), 2);
    let f = r.follower().unwrap();
    assert_eq!(
        *f,
        TimeBaseEntry {
            time_base: TB_USB_SAMPLE,
            role: TimeBaseRole::Follower,
            state: ContractState::Locked,
            last_reason: TimeBaseReason::Locked,
            domain: 0x0100_0000,
            t: 80_000_000,
            disturbances: 1,
            d: 192,
            phase_error: 0.125,
        }
    );
    assert_eq!(f.domain_time_base(), TB_USB_SAMPLE);
    let s = r.source().unwrap();
    assert_eq!(
        (s.time_base, s.role, s.state, s.last_reason),
        (
            TB_SPDIF,
            TimeBaseRole::Source,
            ContractState::Locked,
            TimeBaseReason::Epoch
        )
    );
    assert_eq!(s.domain, 0x0303_3742);
    assert_eq!(
        (s.domain_time_base(), s.domain_source(), s.domain_epoch()),
        (TB_SPDIF, 0x3742, 3)
    );
    assert_eq!(
        (s.t, s.disturbances, s.d, s.phase_error),
        (960_000, 4, 0, 0.0)
    );
    // A source's t - output_frames is its origin, a multiple of 192 (here
    // negative modulo 2^32: 960 000 - 4 800 000).
    let origin = r.t_minus_output(s);
    assert_eq!(origin, 960_000u32.wrapping_sub(4_800_000));
    assert_eq!(r.t_minus_output(f), 80_000_000 - 4_800_000);
    assert_eq!(r.entry(TB_SPDIF), Some(s));
    assert_eq!(r.entry(TB_ALINK_LINE), None);
    assert_eq!(r.encode().to_vec(), raw);
}

#[test]
fn wp_atproto_time_base_replies_short_versioned_and_overcounted() {
    let empty = TimeBaseRead {
        time_us: 7,
        output_frames: 9,
        entries: vec![],
    };
    let raw = empty.encode();
    assert_eq!(&raw[..2], &[1, 0]);
    assert_eq!(TimeBaseRead::decode(&raw), Some(empty));
    assert_eq!(TimeBaseRead::decode(&raw[..63]), None);
    let mut long = raw.to_vec();
    long.extend_from_slice(&[0xAA; 8]);
    assert!(TimeBaseRead::decode(&long).is_some());
    let mut v2 = raw;
    v2[0] = 2;
    assert_eq!(TimeBaseRead::decode(&v2), None);
    let mut three = raw;
    three[1] = 3;
    assert_eq!(TimeBaseRead::decode(&three), None);
}

#[test]
fn wp_atproto_the_siggen_timeline_example() {
    // 2.11.2: running since S = 80 012 000, 1 s at 48 kHz, run 7.
    let raw = hex(
        "01 01 03 01 00 00 96 00 e0 e2 c4 04 00 00 00 00 00 00 00 00 00 00 00 00 \
         80 bb 00 00 00 00 00 00 07 00 00 00 00 00 00 00",
    );
    let t = SiggenTimeline::decode(&raw).unwrap();
    assert_eq!(
        t,
        SiggenTimeline {
            time_base: TB_USB_SAMPLE,
            state: TimelineState::Running,
            last_result: TimelineResult::Started,
            discontinuities: 0,
            arm_lead_ms: 150,
            start: 80_012_000,
            pending: 0,
            samples: 48_000,
            held: 0,
            run_id: 7,
        }
    );
    assert_eq!(t.encode().to_vec(), raw);
    assert_eq!(SiggenTimeline::decode(&raw[..39]), None);
    let mut v0 = raw.clone();
    v0[0] = 0;
    assert_eq!(SiggenTimeline::decode(&v0), None);
    for r in 0..=7 {
        assert_eq!(TimelineResult::from_raw(r).raw(), r);
        assert_ne!(TimelineResult::from_raw(r).as_str(), "unknown");
    }
    assert_eq!(TimelineResult::from_raw(7), TimelineResult::StoppedLoss);
    assert_eq!(TimelineResult::from_raw(8).as_str(), "unknown");
}

#[test]
fn wp_atproto_the_time_base_event_example_behind_bit_5() {
    // 2.11.3: an S/PDIF follower phase-locked at F = 960 000.
    let raw = hex("02 e4 00 05 03 03 01 00 00 a6 0e 00");
    let p = &raw[4..];
    let e = time_base_event(Some(NOTIFY_FEAT_TIME_BASE), NOTIFY_EVT_TIME_BASE, p).unwrap();
    assert_eq!(
        e,
        TimeBaseEvent {
            time_base: TB_SPDIF,
            state: ContractState::Locked,
            reason: TimeBaseReason::Locked,
            t_low: 960_000,
        }
    );
    assert!(!e.is_loss());
    assert_eq!(e.encode().to_vec(), p.to_vec());
    // Only with bit 5, only for 0xE4, never short.
    assert_eq!(time_base_event(Some(1 << 4), 0xE4, p), None);
    assert_eq!(time_base_event(None, 0xE4, p), None);
    assert_eq!(time_base_event(Some(u32::MAX), 0xE3, p), None);
    assert_eq!(time_base_event(Some(u32::MAX), 0xE4, &p[..7]), None);
    for r in 0..=11 {
        assert_eq!(TimeBaseReason::from_raw(r).raw(), r);
        assert_ne!(TimeBaseReason::from_raw(r).as_str(), "unknown");
    }
    for s in 0..=5 {
        assert_eq!(ContractState::from_raw(s).raw(), s);
    }
}

#[test]
fn wp_atproto_the_crc_matches_the_line_superframe_example() {
    // 6.1 `sf/typical`: CRC `d7 47` over bytes 0-29; and FWSP1's CRC vectors.
    let sf = hex(
        "a5 01 08 01 08 01 00 42 00 00 00 00 00 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 d7 47",
    );
    assert_eq!(crc16(&sf[..30]).to_le_bytes(), [0xd7, 0x47]);
    assert_eq!(crc16(b"123456789"), 10673);
    assert_eq!(crc16(&[]), 0xFFFF);
}

#[test]
fn wp_atproto_the_spdif_superframe_examples() {
    // 12.2: source 0x42/0x37, epoch 3, 48 kHz, F = 960 000, op 9 at N = 1 000 000.
    let raw = hex(
        "a5 01 04 01 02 10 00 42  00 a6 0e 00 00 00 00 00  40 42 0f 00 00 00 00 00  09 00 00 00 03 37 a7 80 \
         00 00 00 00 00 00 00 00  00 00 00 00 00 00 00 00",
    );
    let sf = SpdifSuperframe {
        flags: 0,
        rate: 1,
        hops: 0,
        sender: 0x42,
        frame: 960_000,
        apply: ApplyRequest {
            frame: 1_000_000,
            op: ApplyOp::SiggenStart,
            arg: 0,
            value: 0,
        },
        epoch: 3,
        sender2: 0x37,
    };
    assert_eq!(sf.pack().to_vec(), raw);
    let back = SpdifSuperframe::unpack(&raw).unwrap();
    assert_eq!(back, SpdifSuperframe { flags: 0x04, ..sf });
    assert_eq!(back.source_id(), 0x3742);
    assert!(SpdifSuperframe::carries(960_000, 1_000_000));
    // The next block, no apply pending.
    let mut next = hex("a5 01 00 01 02 10 00 42 c0 a6 0e 00 00 00 00 00");
    next.extend_from_slice(&[0; 12]);
    next.extend_from_slice(&hex("03 37 90 dd"));
    next.extend_from_slice(&[0; 16]);
    let none = SpdifSuperframe {
        frame: 960_192,
        apply: ApplyRequest {
            frame: 0,
            op: ApplyOp::CancelAll,
            arg: 0,
            value: 0,
        },
        ..sf
    };
    assert_eq!(none.pack().to_vec(), next);
    assert_eq!(SpdifSuperframe::unpack(&next), Ok(none));
    // The first 16 user bits: `1 0 1 0 0 1 0 1 1 0 0 0 0 0 0 0` (A0 = 1, B0 = 0).
    let block: [u8; 48] = raw.clone().try_into().unwrap();
    let first: Vec<u8> = (0..16)
        .map(|i| u8::from(user_bit(&block, i).unwrap()))
        .collect();
    assert_eq!(first, vec![1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(user_bit(&block, 384), None);
    let bits: Vec<bool> = (0..384).map(|i| user_bit(&block, i).unwrap()).collect();
    assert_eq!(superframe_from_user_bits(&bits), Some(block));
    assert_eq!(superframe_from_user_bits(&bits[..383]), None);
}

#[test]
fn wp_atproto_apply_carriage_edge() {
    // `frame + 192 <= N`.
    assert!(SpdifSuperframe::carries(0, 192));
    assert!(!SpdifSuperframe::carries(0, 191));
    assert!(!SpdifSuperframe::carries(u64::MAX - 100, u64::MAX));
}

/// FWSP1's `alink_spdif_vectors.json` (`DSPi-wt/int-fwb` eec48ba), every `sf`
/// row: name, the reference codec's result, the 48 bytes.
const SF_VECTORS: &[(&str, u8, &str)] = &[
    (
        "spdif_sf/typical",
        0,
        "a50104010210004200a60e000000000040420f0000000000090000000337a78000000000000000000000000000000000",
    ),
    (
        "spdif_sf/no_apply",
        0,
        "a501000102100042c0a60e0000000000000000000000000000000000033790dd00000000000000000000000000000000",
    ),
    (
        "spdif_sf/op1",
        0,
        "a5010c02021000fe0000000000030000881300000003000001000000ff01d4af00000000000000000000000000000000",
    ),
    (
        "spdif_sf/op2",
        0,
        "a5010c02021000fe0000000000030000881300000003000002000000ff01346100000000000000000000000000000000",
    ),
    (
        "spdif_sf/op4",
        0,
        "a5010c02021000fe00000000000300008813000000030000040050fbff014d8a00000000000000000000000000000000",
    ),
    (
        "spdif_sf/op7",
        0,
        "a5010c02021000fe0000000000030000881300000003000007030000ff01e7cc00000000000000000000000000000000",
    ),
    (
        "spdif_sf/op9",
        0,
        "a5010c02021000fe0000000000030000881300000003000009000000ff0196a200000000000000000000000000000000",
    ),
    (
        "spdif_sf/op10",
        0,
        "a5010c02021000fe000000000003000088130000000300000a010000ff0127c600000000000000000000000000000000",
    ),
    (
        "spdif_sf/padding_ignored",
        0,
        "a50104010210004200a60e000000000040420f0000000000090000000337a7800000000000000000ff00000000000000",
    ),
    (
        "spdif_sf/err_crc",
        1,
        "a50104010210004201a60e000000000040420f0000000000090000000337a78000000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_crc_field",
        1,
        "a50104010210004200a60e000000000040420f0000000000090000000337a68000000000000000000000000000000000",
    ),
    (
        "spdif_sf/all_zero",
        1,
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_magic",
        2,
        "5a0104010210004200a60e000000000040420f0000000000090000000337b5fd00000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_magic_and_frame",
        2,
        "5a0104010210004201a60e000000000040420f00000000000900000003376d9000000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_version0",
        3,
        "a50004010210004200a60e000000000040420f0000000000090000000337417000000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_version2",
        3,
        "a50204010210004200a60e000000000040420f0000000000090000000337ac8100000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_version_and_class",
        3,
        "a50204010201004200a60e000000000040420f0000000000090000000337dcdf00000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_frame_f1",
        4,
        "a50104010210004201a60e000000000040420f00000000000900000003377fed00000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_frame_f64",
        4,
        "a50104010210004240a60e000000000040420f0000000000090000000337fd5500000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_frame_f128",
        4,
        "a50104010210004280a60e000000000040420f0000000000090000000337323a00000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_frame_2p32",
        4,
        "a501040102100042000000000100000040420f0000000000090000000337a49700000000000000000000000000000000",
    ),
    (
        "spdif_sf/frame_3x2p32",
        0,
        "a501040102100042000000000300000040420f0000000000090000000337e76700000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_frame_and_class",
        4,
        "a50104010201004201a60e000000000040420f00000000000900000003370fb300000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_class_00",
        5,
        "a50104010200004200a60e000000000040420f0000000000090000000337639b00000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_class_01",
        5,
        "a50104010201004200a60e000000000040420f0000000000090000000337d7de00000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_class_02",
        5,
        "a50104010202004200a60e000000000040420f00000000000900000003370b1000000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_class_03",
        5,
        "a50104010203004200a60e000000000040420f0000000000090000000337bf5500000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_class_11",
        5,
        "a50104010211004200a60e000000000040420f000000000009000000033713c500000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_class_ff",
        5,
        "a501040102ff004200a60e000000000040420f0000000000090000000337d0c300000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_rate3",
        5,
        "a50104030210004200a60e000000000040420f00000000000900000003376baa00000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_channels0",
        5,
        "a50104010010004200a60e000000000040420f000000000009000000033744c800000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_channels1",
        5,
        "a50104010110004200a60e000000000040420f0000000000090000000337256400000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_channels3",
        5,
        "a50104010310004200a60e000000000040420f0000000000090000000337c62c00000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_channels8",
        5,
        "a50104010810004200a60e000000000040420f0000000000090000000337e9fb00000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_layout_and_apply",
        5,
        "a50104010201004200a60e000000000040420f0000000000030000000337d55800000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_op3",
        6,
        "a50104010210004200a60e000000000040420f0000000000030000000337a50600000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_op5",
        6,
        "a50104010210004200a60e000000000040420f0000000000050000000337448b00000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_op6",
        6,
        "a50104010210004200a60e000000000040420f0000000000060000000337a44500000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_op8",
        6,
        "a50104010210004200a60e000000000040420f000000000008000000033707c500000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_op11",
        6,
        "a50104010210004200a60e000000000040420f00000000000b0000000337e70b00000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_op31",
        6,
        "a50104010210004200a60e000000000040420f00000000001f0000000337c21700000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_op32",
        6,
        "a50104010210004200a60e000000000040420f00000000002000000003374dfd00000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_op255",
        6,
        "a50104010210004200a60e000000000040420f0000000000ff0000000337fa9c00000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_op_no_target",
        6,
        "a50104010210004200a60e00000000000000000000000000090000000337ed6800000000000000000000000000000000",
    ),
    (
        "spdif_sf/err_target_no_op",
        6,
        "a501000102100042c0a60e000000000080841e0000000000000000000337251d00000000000000000000000000000000",
    ),
    (
        "spdif_sf/arg_not_judged",
        0,
        "a50104010210004200a60e000000000040420f000000000009ee340003371b9200000000000000000000000000000000",
    ),
    (
        "spdif_sf/hops_flags_ignored",
        0,
        "a501f2010210094200a60e000000000040420f0000000000090000000337b9b300000000000000000000000000000000",
    ),
];

#[test]
fn wp_atproto_agrees_with_fwsp1s_vectors() {
    for (name, want, bytes) in SF_VECTORS {
        let b = hex(bytes);
        let got = match SpdifSuperframe::unpack(&b) {
            Ok(sf) => {
                // A valid one re-packs to itself, padding aside, unless its
                // flags byte disagrees with the op (the flags are not checked;
                // pack derives bit 2 from the op).
                if (b[2] & SF_FLAG_APPLY_PENDING != 0) == (b[24] != 0) {
                    assert_eq!(&sf.pack()[..32], &b[..32], "{name}");
                }
                0
            }
            Err(e) => e.code(),
        };
        assert_eq!(got, *want, "{name}");
    }
    assert_eq!(
        SpdifSuperframe::unpack(&[0; 47]),
        Err(SuperframeError::Short)
    );
}

#[test]
fn wp_atproto_names() {
    assert_eq!(crate::ext::name(EXT_TIME_BASE), "time base");
    assert_eq!(crate::ext::name(EXT_SIGGEN_TIMELINE), "generator timeline");
    assert_eq!(crate::ext::name(EXT_ALINK_SPDIF), "audio link S/PDIF");
    assert_eq!(
        crate::ext::name(EXT_ALINK_SPDIF_STATUS),
        "audio link S/PDIF status"
    );
    assert_eq!(
        crate::ext::name(EXT_ALINK_SPDIF_CTRL),
        "audio link S/PDIF broadcast"
    );
    assert_eq!(crate::ext::name(0x8023), "experimental");
    assert_eq!(crate::ext::name(0x8032), "experimental");
    assert_eq!(time_base_name(TB_SPDIF), "spdif");
    assert_eq!(time_base_name(7), "unknown");
}

fn entry() -> impl Strategy<Value = TimeBaseEntry> {
    (
        any::<[u8; 4]>(),
        any::<u32>(),
        any::<u64>(),
        any::<u16>(),
        any::<u16>(),
        -1.0e6f32..1.0e6,
    )
        .prop_map(
            |(h, domain, t, disturbances, d, phase_error)| TimeBaseEntry {
                time_base: h[0],
                role: TimeBaseRole::from_raw(h[1]),
                state: ContractState::from_raw(h[2]),
                last_reason: TimeBaseReason::from_raw(h[3]),
                domain,
                t,
                disturbances,
                d,
                phase_error,
            },
        )
}

proptest! {
    #[test]
    fn proptest_wp_atproto_decoders_never_panic(b in proptest::collection::vec(any::<u8>(), 0..80)) {
        let _ = TimeBaseRead::decode(&b);
        let _ = TimeBaseEntry::decode(&b);
        let _ = SiggenTimeline::decode(&b);
        let _ = AlinkSpdif::decode(&b).map(|s| s.is_valid());
        let _ = AlinkSpdifStatus::decode(&b);
        let _ = decode_spdif_ctrl_status(&b);
        let _ = decode_apply_tb(&b).map(|(r, tb)| apply_form_valid(&r, tb));
        let _ = SpdifSuperframe::unpack(&b);
        for id in 0xE0..=0xE5u8 {
            let _ = time_base_event(Some(u32::MAX), id, &b);
        }
        prop_assert_eq!(AlinkSpdifStatus::decode(&b).is_some(), b.len() >= AlinkSpdifStatus::SIZE);
        prop_assert_eq!(AlinkSpdif::decode(&b).is_some(), b.len() >= AlinkSpdif::SIZE);
        prop_assert_eq!(TimeBaseEvent::decode(&b).is_some(), b.len() >= TimeBaseEvent::PAYLOAD);
    }

    #[test]
    fn proptest_wp_atproto_time_base_round_trips(
        time_us in any::<u32>(), output_frames in any::<u32>(),
        entries in proptest::collection::vec(entry(), 0..=2),
        tail in proptest::collection::vec(any::<u8>(), 0..8),
    ) {
        let r = TimeBaseRead { time_us, output_frames, entries };
        let mut raw = r.encode().to_vec();
        raw.extend_from_slice(&tail);
        prop_assert_eq!(TimeBaseRead::decode(&raw), Some(r));
    }

    #[test]
    fn proptest_wp_atproto_timeline_and_status_round_trip(b in any::<[u8; 48]>()) {
        let mut tl = b[..40].to_vec();
        tl[0] = 1;
        tl[36..40].fill(0);
        let t = SiggenTimeline::decode(&tl).unwrap();
        prop_assert_eq!(t.encode().to_vec(), tl);
        let mut st = b.to_vec();
        st[7] = 0;
        st[30] = 0;
        st[31] = 0;
        // Compare bytes: a NaN bit pattern round-trips, but is not equal.
        let s = AlinkSpdifStatus::decode(&st).unwrap();
        prop_assert_eq!(s.encode().to_vec(), st);
        let mut cfg = b[..8].to_vec();
        cfg[5] = 0;
        prop_assert_eq!(AlinkSpdif::decode(&cfg).unwrap().encode().to_vec(), cfg);
        let ev = TimeBaseEvent::decode(&b[..8]).map(|e| e.encode());
        let mut want = b[..8].to_vec();
        want[3] = 0;
        prop_assert_eq!(ev.map(|e| e.to_vec()), Some(want));
    }

    #[test]
    fn proptest_wp_atproto_superframe_round_trips_and_detects_one_bit(
        rate in 0u8..=2, flags in any::<u8>(), sender in any::<u8>(), sender2 in any::<u8>(),
        epoch in any::<u8>(), block in 0u64..(u64::MAX / 192),
        op_i in 0usize..7, arg in 0u8..2, value in any::<i16>(), target in 1u64..,
        bit in 0usize..(32 * 8),
    ) {
        let op = if op_i == 0 { 0 } else { SPDIF_APPLY_OPS[op_i - 1] };
        let sf = SpdifSuperframe {
            flags, rate, hops: 0, sender, frame: block * 192,
            apply: ApplyRequest {
                frame: if op == 0 { 0 } else { target },
                op: ApplyOp::from_raw(op), arg, value,
            },
            epoch, sender2,
        };
        let raw = sf.pack();
        let want_flags = if op == 0 { flags & !SF_FLAG_APPLY_PENDING } else { flags | SF_FLAG_APPLY_PENDING };
        prop_assert_eq!(SpdifSuperframe::unpack(&raw), Ok(SpdifSuperframe { flags: want_flags, ..sf }));
        // CRC-16 catches every single-bit error in bytes 0-31.
        let mut bad = raw;
        bad[bit / 8] ^= 1 << (bit % 8);
        prop_assert!(SpdifSuperframe::unpack(&bad).is_err());
        // The user bits carry the bytes back.
        let bits: Vec<bool> = (0..384).map(|i| user_bit(&raw, i).unwrap()).collect();
        prop_assert_eq!(superframe_from_user_bits(&bits), Some(raw));
    }

    #[test]
    fn proptest_wp_atproto_leads_never_panic(now in any::<u64>(), t in any::<u64>(), arm in any::<u16>(), rate in any::<u32>()) {
        let _ = siggen_start_lead_ok(now, t, arm, rate);
        prop_assert_eq!(spdif_ctrl_lead_ok(now, t), t >= now && t - now >= 896);
    }
}
