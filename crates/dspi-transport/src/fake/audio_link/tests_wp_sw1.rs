//! WP SW1: the fake's audio link answers as SYS2 5.3 specifies, and is absent
//! without the option.

use super::super::{Capture, FakeDevice, FakeOptions};
use super::*;
use crate::{NotificationSource, Transport, TransportError};
use dspi_proto::generated::opcodes as op;
use std::time::Duration;

fn device(audio_link: bool) -> FakeDevice {
    FakeDevice::new(
        Capture::demo(),
        FakeOptions {
            extensions: true,
            audio_link,
            ..FakeOptions::default()
        },
    )
}

fn get(d: &mut FakeDevice, id: u16, len: u16) -> crate::Result<Vec<u8>> {
    d.control_in(op::REQ_EXTENSION, id, len)
}

fn set(d: &mut FakeDevice, id: u16, data: &[u8]) -> crate::Result<()> {
    d.control_out(op::REQ_EXTENSION, id, data)
}

/// Every packet queued now.
fn drain(src: &mut Box<dyn NotificationSource>) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let p = src.read(Duration::from_millis(1)).unwrap();
        if p.is_empty() {
            return out;
        }
        out.push(p);
    }
}

fn list(d: &mut FakeDevice) -> Vec<u16> {
    let b = d
        .control_in_upto(op::REQ_EXTENSION, dspi_proto::generated::ext::EXT_LIST, 64)
        .unwrap();
    dspi_proto::ext::decode_list(&b)
}

#[test]
fn wp_sw1_without_the_option_there_is_no_link() {
    let mut d = device(false);
    assert!(list(&mut d).iter().all(|id| *id < 0x8000));
    assert!(matches!(
        get(&mut d, alink::EXT_LINK_INFO, 32),
        Err(TransportError::Stalled { .. })
    ));
    assert!(matches!(
        set(&mut d, EXT_FAKE_LINK_SIM, &[LINK_SIM_LOSE]),
        Err(TransportError::Stalled { .. })
    ));
}

#[test]
fn wp_sw1_a_receiver_lists_and_answers_the_link() {
    let mut d = device(true);
    let ids = list(&mut d);
    for id in alink::RECEIVER_IDS {
        assert!(ids.contains(&id), "{id:#x} in {ids:x?}");
    }
    assert!(!ids.contains(&alink::EXT_LINK_TX));
    // 16 ids at most: the list still fits the host's 64-byte read.
    assert!(ids.len() <= 32);
    let f = get(&mut d, dspi_proto::generated::ext::EXT_NOTIFY_FEATURES, 4).unwrap();
    let f = u32::from_le_bytes(f.try_into().unwrap());
    assert_eq!(f & 0b1100, 0b1100);

    // 2.8.1's example bytes, but the firmware's D_local range for the
    // default 4 ms option, 257..399 (WP FIXS1).
    let info = get(&mut d, 0x8000, 32).unwrap();
    assert_eq!(
        info[..20],
        [
            1, 1, 7, 1, 0x40, 8, 0, 0, 0, 0xff, 0, 0, 0x23, 0, 0, 0, 0x01, 1, 0x8f, 1
        ]
    );
    let s = LinkStatus::decode(&get(&mut d, 0x8001, 52).unwrap()).unwrap();
    assert_eq!(
        (s.state, s.channels, s.sender_id),
        (LinkState::Locked, 8, 0x42)
    );
    assert!(s.frame >= FRAME_BASE);
    assert_eq!(get(&mut d, 0x8003, 8).unwrap(), LinkSlots::IDENTITY.0);
    assert_eq!(get(&mut d, 0x800A, 64).unwrap()[0], 1);
    let snap = SuperframeSnapshot::decode(&get(&mut d, 0x8008, 24).unwrap()).unwrap();
    assert_eq!((snap.channels, snap.sender_id), (8, 0x42));
    assert!(get(&mut d, alink::EXT_LINK_TX, 16).is_err());

    // Slots: written and read back; 64..0xFE stalls, at or above C does not.
    set(&mut d, 0x8003, &[4, 5, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]).unwrap();
    assert_eq!(get(&mut d, 0x8003, 8).unwrap()[..2], [4, 5]);
    set(&mut d, 0x8003, &[9; 8]).unwrap();
    assert!(set(&mut d, 0x8003, &[64; 8]).is_err());
    assert!(set(&mut d, 0x8003, &[0; 7]).is_err());

    // Config: 2.8.3's refusals; tx_pin needs REPEATER, which it lacks.
    assert!(set(&mut d, 0x8002, &[2, 0xFF, 0, 0xFF, 0xFF, 0xFF, 3, 0]).is_err());
    assert!(set(&mut d, 0x8002, &[1, 0xFF, 4, 0xFF, 0xFF, 0xFF, 3, 0]).is_err());
    assert!(set(&mut d, 0x8002, &[1, 0xFF, 0, 0xFF, 0xFF, 0xFF, 4, 0]).is_err());
    assert!(set(&mut d, 0x8002, &[1, 23, 0, 0xFF, 0xFF, 0xFF, 3, 0]).is_err());
    assert!(set(&mut d, 0x8002, &[1, 0, 0, 0xFF, 0, 0xFF, 3, 0]).is_err());
    assert!(set(&mut d, 0x8002, &[1, 0, 1, 22, 21, 0xFF, 1, 0]).is_err());
    set(&mut d, 0x8002, &[1, 0, 1, 0xFF, 21, 0xFF, 1, 0]).unwrap();
    assert_eq!(
        get(&mut d, 0x8002, 8).unwrap(),
        [1, 0, 1, 0xFF, 21, 0xFF, 1, 0]
    );

    // Phase: within 257..399 and +-64; OUT flags must be 0.
    let ph = |mode, d_local, hops_offset, flags| {
        LinkPhase {
            mode,
            d_local,
            hops_offset,
            flags,
            state: 0,
        }
        .encode()
    };
    set(&mut d, 0x8004, &ph(0, 257, 0, 0)).unwrap();
    let p = LinkPhase::decode(&get(&mut d, 0x8004, 8).unwrap()).unwrap();
    assert_eq!((p.mode, p.d_local, p.state), (0, 257, 0));
    assert!(set(&mut d, 0x8004, &ph(2, 300, 0, 0)).is_err());
    assert!(set(&mut d, 0x8004, &ph(1, 256, 0, 0)).is_err());
    assert!(set(&mut d, 0x8004, &ph(1, 400, 0, 0)).is_err());
    assert!(set(&mut d, 0x8004, &ph(1, 300, 65, 0)).is_err());
    assert!(set(&mut d, 0x8004, &ph(1, 300, 0, 1)).is_err());
    set(&mut d, 0x8004, &ph(1, 300, 0, 0)).unwrap();
    assert_eq!(get(&mut d, 0x8004, 8).unwrap()[7], 2);

    // Stats reset takes exactly [1].
    assert!(set(&mut d, 0x8009, &[0]).is_err());
    set(&mut d, 0x8009, &[1]).unwrap();
    let s = LinkStatus::decode(&get(&mut d, 0x8001, 52).unwrap()).unwrap();
    assert_eq!(s.lock_count, 0);

    // Input source 7 is selectable.
    d.control_out(op::REQ_SET_INPUT_SOURCE, 0, &[7]).unwrap();
    assert_eq!(
        d.control_in(op::REQ_GET_INPUT_SOURCE, 0, 1).unwrap(),
        vec![7]
    );
}

#[test]
fn wp_sw1_input_source_7_needs_the_link() {
    let mut d = device(false);
    d.control_out(op::REQ_SET_INPUT_SOURCE, 0, &[7]).unwrap();
    assert_ne!(
        d.control_in(op::REQ_GET_INPUT_SOURCE, 0, 1).unwrap(),
        vec![7]
    );
}

fn state_events(src: &mut Box<dyn NotificationSource>) -> Vec<LinkStateEvent> {
    drain(src)
        .into_iter()
        .filter(|p| p.get(1) == Some(&0xE1))
        .map(|p| LinkStateEvent::decode(&p[4..]).unwrap())
        .collect()
}

#[test]
fn wp_sw1_loss_lock_and_reanchor_send_events() {
    let mut d = device(true);
    let mut src = d.notifications().unwrap();
    drain(&mut src);
    set(&mut d, EXT_FAKE_LINK_SIM, &[LINK_SIM_LOSE]).unwrap();
    let s1 = LinkStatus::decode(&get(&mut d, 0x8001, 52).unwrap()).unwrap();
    assert_eq!((s1.state, s1.loss_count), (LinkState::Holdover, 1));
    // Extrapolated in holdover: still valid, still running.
    assert!(s1.frame >= FRAME_BASE);
    assert_eq!(get(&mut d, 0x8004, 8).unwrap()[7], 3);
    // No apply without a lock.
    let r = ApplyRequest {
        frame: s1.frame + 48_000,
        op: ApplyOp::Mute,
        arg: 0,
        value: 0,
    };
    assert!(set(&mut d, 0x8006, &r.encode()).is_err());
    set(&mut d, EXT_FAKE_LINK_SIM, &[LINK_SIM_REANCHOR]).unwrap();
    set(&mut d, EXT_FAKE_LINK_SIM, &[LINK_SIM_BANK_ERROR]).unwrap();
    assert!(set(&mut d, EXT_FAKE_LINK_SIM, &[9]).is_err());
    let s = LinkStatus::decode(&get(&mut d, 0x8001, 52).unwrap()).unwrap();
    assert_eq!(
        (s.state, s.crc16_errors, s.lock_count),
        (LinkState::Locked, 1, 2)
    );

    let events = state_events(&mut src);
    assert_eq!(events.len(), 2);
    assert_eq!(
        (events[0].state, events[0].reason),
        (LinkState::Holdover, LinkEventReason::Los)
    );
    assert_eq!(
        (events[1].state, events[1].reason),
        (LinkState::Locked, LinkEventReason::Reanchor)
    );

    // A new delay while phase-locked re-anchors; the same one does nothing.
    let p = LinkPhase {
        mode: 1,
        d_local: 320,
        hops_offset: 0,
        flags: 0,
        state: 0,
    };
    set(&mut d, 0x8004, &p.encode()).unwrap();
    set(&mut d, 0x8004, &p.encode()).unwrap();
    let events = state_events(&mut src);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].reason, LinkEventReason::Reanchor);

    // Disabling a locked link: off, reason DISABLED; enabling locks again.
    set(&mut d, 0x8002, &[0, 0xFF, 0, 0xFF, 0xFF, 0xFF, 3, 0]).unwrap();
    assert_eq!(get(&mut d, 0x8001, 52).unwrap()[0], 0);
    assert_eq!(
        FrameCounter::decode(&get(&mut d, 0x8005, 16).unwrap())
            .unwrap()
            .frame,
        0
    );
    set(&mut d, 0x8002, &[1, 0xFF, 0, 0xFF, 0xFF, 0xFF, 3, 0]).unwrap();
    let events = state_events(&mut src);
    let reasons: Vec<_> = events.iter().map(|e| e.reason).collect();
    assert_eq!(
        reasons,
        [LinkEventReason::Disabled, LinkEventReason::Locked]
    );
}

#[test]
fn wp_sw1_apply_at_frame_queues_and_reports() {
    let mut d = device(true);
    let mut src = d.notifications().unwrap();
    drain(&mut src);
    let now = FrameCounter::decode(&get(&mut d, 0x8005, 16).unwrap())
        .unwrap()
        .frame;
    // A passed frame: applied at once, late; the event carries the target.
    let late = ApplyRequest {
        frame: 1234,
        op: ApplyOp::Mute,
        arg: 0,
        value: 0,
    };
    set(&mut d, 0x8006, &late.encode()).unwrap();
    let st = ApplyStatus::decode(&get(&mut d, 0x8006, 16).unwrap()).unwrap();
    assert_eq!(
        (st.last_op, st.last_result, st.pending, st.last_frame),
        (ApplyOp::Mute, ApplyResult::Late, 0, 1234)
    );
    // Far ahead: queued, four at most.
    for i in 0..4u64 {
        let r = ApplyRequest {
            frame: now + 48_000_000 + i,
            op: ApplyOp::Click,
            arg: 1,
            value: 0,
        };
        set(&mut d, 0x8006, &r.encode()).unwrap();
    }
    let full = ApplyRequest {
        frame: now + 48_000_000,
        op: ApplyOp::Unmute,
        arg: 0,
        value: 0,
    };
    assert!(set(&mut d, 0x8006, &full.encode()).is_err());
    assert_eq!(get(&mut d, 0x8006, 16).unwrap()[0], 4);
    // Out of range or a reserved byte: refused.
    let bad = ApplyRequest {
        frame: now,
        op: ApplyOp::PresetLoad,
        arg: 10,
        value: 0,
    };
    assert!(set(&mut d, 0x8006, &bad.encode()).is_err());
    let mut reserved = late.encode();
    reserved[15] = 1;
    assert!(set(&mut d, 0x8006, &reserved).is_err());
    // CANCEL_ALL empties the queue, one "cancelled" each.
    let cancel = ApplyRequest {
        frame: 0,
        op: ApplyOp::CancelAll,
        arg: 0,
        value: 0,
    };
    set(&mut d, 0x8006, &cancel.encode()).unwrap();
    assert_eq!(get(&mut d, 0x8006, 16).unwrap()[0], 0);

    let apply: Vec<_> = drain(&mut src)
        .into_iter()
        .filter(|p| p.get(1) == Some(&0xE2))
        .map(|p| ApplyResultEvent::decode(&p[4..]).unwrap())
        .collect();
    assert_eq!(apply.len(), 5);
    assert_eq!(
        (apply[0].op, apply[0].result, apply[0].frame_low),
        (ApplyOp::Mute, ApplyResult::Late, 1234)
    );
    assert!(
        apply[1..]
            .iter()
            .all(|e| e.result == ApplyResult::Cancelled && e.op == ApplyOp::Click)
    );
}

#[test]
fn wp_sw1_the_demo_with_the_link() {
    let mut d = FakeDevice::demo_audio_link();
    assert!(list(&mut d).contains(&0x8000));
    let mut plain = FakeDevice::demo();
    assert!(!list(&mut plain).contains(&0x8000));
}

/// Fixup sw1 (F5): the info's rx_pin follows the config; disabling from
/// holdover reads state 0.
#[test]
fn wp_sw1_fixup_rx_pin_follows_the_config_and_disable_from_holdover() {
    let mut d = device(true);
    assert_eq!(
        LinkInfo::decode(&get(&mut d, 0x8000, 32).unwrap())
            .unwrap()
            .rx_pin,
        0
    );
    set(&mut d, 0x8002, &[1, 5, 0, 0xFF, 0xFF, 0xFF, 3, 0]).unwrap();
    assert_eq!(
        LinkInfo::decode(&get(&mut d, 0x8000, 32).unwrap())
            .unwrap()
            .rx_pin,
        5
    );
    set(&mut d, 0x8002, &[1, 0xFF, 0, 0xFF, 0xFF, 0xFF, 3, 0]).unwrap();
    assert_eq!(
        LinkInfo::decode(&get(&mut d, 0x8000, 32).unwrap())
            .unwrap()
            .rx_pin,
        0
    );

    let mut src = d.notifications().unwrap();
    drain(&mut src);
    set(&mut d, EXT_FAKE_LINK_SIM, &[LINK_SIM_LOSE]).unwrap();
    assert_eq!(get(&mut d, 0x8001, 52).unwrap()[0], 3);
    set(&mut d, 0x8002, &[0, 0xFF, 0, 0xFF, 0xFF, 0xFF, 3, 0]).unwrap();
    assert_eq!(get(&mut d, 0x8001, 52).unwrap()[0], 0);
    let reasons: Vec<_> = state_events(&mut src)
        .iter()
        .map(|e| (e.state, e.reason))
        .collect();
    assert_eq!(
        reasons,
        [
            (LinkState::Holdover, LinkEventReason::Los),
            (LinkState::Off, LinkEventReason::Disabled)
        ]
    );
}

// -- WP FIXS1 --
#[test]
fn wp_fixs1_the_delay_range_follows_the_latency_option_as_the_firmware() {
    let mut d = device(true);
    let range = |d: &mut FakeDevice| {
        let i = LinkInfo::decode(&get(d, 0x8000, 32).unwrap()).unwrap();
        (i.d_local_min, i.d_local_max)
    };
    // Spec 2.8.1 at 48 kHz: the default 4 ms option, then the 1 ms one.
    assert_eq!(range(&mut d), (257, 399));
    let p = LinkPhase::decode(&get(&mut d, 0x8004, 8).unwrap()).unwrap();
    assert_eq!((p.d_local, p.flags, p.state), (300, 0, 2), "starts inside");
    d.control_out(op::REQ_OUTPUT_LATENCY, 0, &[2]).unwrap();
    assert_eq!(range(&mut d), (81, 431));
    set(&mut d, 0x8004, &[1, 0, 81, 0, 0, 0, 0, 0]).unwrap();
    // Back to 4 ms: 81 is now below the minimum, said and free (2.8.5).
    d.control_out(op::REQ_OUTPUT_LATENCY, 0, &[5]).unwrap();
    let p = LinkPhase::decode(&get(&mut d, 0x8004, 8).unwrap()).unwrap();
    assert_eq!(
        (p.d_local, p.flags, p.state),
        (81, alink::PHASE_FLAG_D_BELOW_MIN, 0)
    );
    assert_eq!(d_local_limits((2000, 1000)), (161, 399));
}

#[test]
fn wp_fixs1_phase_error_small_when_locked_growing_in_holdover() {
    let mut d = device(true);
    let err = |d: &mut FakeDevice| {
        LinkStatus::decode(&get(d, 0x8001, 52).unwrap())
            .unwrap()
            .phase_error_frames
    };
    for _ in 0..3 {
        let e = err(&mut d);
        assert!((0.01..=0.05).contains(&e), "locked: {e}");
        std::thread::sleep(Duration::from_millis(5));
    }
    // Its curve: slow (a 30 s period) and never zero.
    let curve: Vec<f32> = (0..300)
        .map(|t| locked_phase_error(t as f32 * 0.1))
        .collect();
    assert!(curve.iter().all(|e| (0.0099..=0.0501).contains(e)));
    assert!(curve.windows(2).all(|w| (w[1] - w[0]).abs() < 0.001));
    let remote = d.remote();
    assert!(remote.link_sim(LINK_SIM_LOSE));
    let e0 = err(&mut d);
    std::thread::sleep(Duration::from_millis(50));
    let e1 = err(&mut d);
    std::thread::sleep(Duration::from_millis(50));
    let e2 = err(&mut d);
    assert!(e0 < e1 && e1 < e2, "holdover grows: {e0} {e1} {e2}");
    assert!(remote.link_sim(LINK_SIM_LOCK));
    assert!((0.01..=0.05).contains(&err(&mut d)));
}
