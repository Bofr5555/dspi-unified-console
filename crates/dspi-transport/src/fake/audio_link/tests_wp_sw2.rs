//! WP SW2: the fake at other places on the link: behind re-emitters, as a
//! re-emitter reporting its hop delay, and as the sender build.

use super::super::FakeDevice;
use super::*;
use crate::{Transport, TransportError};
use dspi_proto::generated::opcodes as op;

fn get(d: &mut FakeDevice, id: u16, len: u16) -> crate::Result<Vec<u8>> {
    d.control_in(op::REQ_EXTENSION, id, len)
}

fn set(d: &mut FakeDevice, id: u16, data: &[u8]) -> crate::Result<()> {
    d.control_out(op::REQ_EXTENSION, id, data)
}

fn status(d: &mut FakeDevice) -> LinkStatus {
    LinkStatus::decode(&get(d, alink::EXT_LINK_STATUS, 52).unwrap()).unwrap()
}

fn stalled<T>(r: crate::Result<T>) -> bool {
    matches!(r, Err(TransportError::Stalled { .. }))
}

#[test]
fn wp_sw2_a_unit_behind_a_fan_out_and_the_fan_out_itself() {
    let mut unit = FakeDevice::demo_audio_link_as(AudioLinkSetup {
        hops: 1,
        sender_id: 7,
        d_local_min: Some(64),
        ..AudioLinkSetup::default()
    })
    .with_serial("UNIT-B");
    assert_eq!(unit.descriptor().serial, "UNIT-B");
    let s = status(&mut unit);
    assert_eq!((s.hops, s.sender_id, s.hop_delay_frames), (1, 7, 0));
    let i = LinkInfo::decode(&get(&mut unit, alink::EXT_LINK_INFO, 32).unwrap()).unwrap();
    assert_eq!(i.d_local_min, 64);

    let mut fan = FakeDevice::demo_audio_link_as(AudioLinkSetup {
        role: LinkRole::FanOut,
        hop_delay_frames: 3,
        ..AudioLinkSetup::default()
    });
    assert_eq!(status(&mut fan).hop_delay_frames, 3);
    // A sender build lists no slots, phase or apply queue.
    assert!(stalled(get(&mut fan, alink::EXT_LINK_SLOTS, 8)));
    assert!(stalled(set(
        &mut fan,
        alink::EXT_LINK_PHASE,
        &[1, 0, 128, 0, 0, 0, 0, 0]
    )));
    // Not re-emitting while the input is lost.
    assert!(fan.remote().link_sim(LINK_SIM_LOSE));
    assert_eq!(status(&mut fan).hop_delay_frames, 0);
}

#[test]
fn wp_sw2_a_repeater_reports_its_hop_delay() {
    let mut r = FakeDevice::demo_audio_link_as(AudioLinkSetup {
        role: LinkRole::Repeater,
        hop_delay_frames: 1,
        ..AudioLinkSetup::default()
    });
    let i = LinkInfo::decode(&get(&mut r, alink::EXT_LINK_INFO, 32).unwrap()).unwrap();
    assert_eq!(i.role, LinkRole::Repeater);
    assert_ne!(i.feature_bits & feature::REPEATER, 0);
    assert_eq!(status(&mut r).hop_delay_frames, 1);
}

#[test]
fn wp_sw2_the_sender_takes_one_superframe_apply_with_lead() {
    let mut s = FakeDevice::demo_audio_link_as(AudioLinkSetup {
        role: LinkRole::Sender,
        ..AudioLinkSetup::default()
    });
    let st = status(&mut s);
    assert_eq!(st.state, LinkState::Off, "a master has no input");
    let c = FrameCounter::decode(&get(&mut s, alink::EXT_FRAME_COUNTER, 16).unwrap()).unwrap();
    assert!(c.frame >= 1_000_000);
    let req = |frame, op| ApplyRequest {
        frame,
        op,
        arg: 0,
        value: 0,
    };
    // Too little lead.
    let near = c.frame;
    assert!(stalled(set(
        &mut s,
        alink::EXT_LINK_CTRL_MSG,
        &req(near, ApplyOp::Mute).encode_ctrl_msg(0)
    )));
    let at = c.frame + 2400;
    set(
        &mut s,
        alink::EXT_LINK_CTRL_MSG,
        &req(at, ApplyOp::Mute).encode_ctrl_msg(0),
    )
    .unwrap();
    // One at a time.
    assert!(stalled(set(
        &mut s,
        alink::EXT_LINK_CTRL_MSG,
        &req(at + 64, ApplyOp::Unmute).encode_ctrl_msg(0)
    )));
    let snap =
        SuperframeSnapshot::decode(&get(&mut s, alink::EXT_LINK_CTRL_MSG, 24).unwrap()).unwrap();
    assert_eq!((snap.apply.frame, snap.apply.op), (at, ApplyOp::Mute));
    assert_ne!(snap.flags & 0b100, 0, "apply pending");
    // Op 0 only sets the flags and leaves the pending apply alone.
    set(
        &mut s,
        alink::EXT_LINK_CTRL_MSG,
        &req(0, ApplyOp::CancelAll).encode_ctrl_msg(1),
    )
    .unwrap();
    let snap =
        SuperframeSnapshot::decode(&get(&mut s, alink::EXT_LINK_CTRL_MSG, 24).unwrap()).unwrap();
    assert_eq!((snap.apply.op, snap.flags & 1), (ApplyOp::Mute, 1));
    // A receiver build has no CTRL_MSG OUT and no TX.
    let mut unit = FakeDevice::demo_audio_link();
    assert!(stalled(set(
        &mut unit,
        alink::EXT_LINK_CTRL_MSG,
        &req(at, ApplyOp::Mute).encode_ctrl_msg(0)
    )));
    assert!(stalled(get(&mut unit, alink::EXT_LINK_TX, 16)));
    assert!(get(&mut s, alink::EXT_LINK_TX, 16).is_ok());
}

// -- fixup sw2x --
/// One flags byte: a system mute set through `EXT_LINK_TX` shows in the
/// superframe, and every accepted `EXT_LINK_CTRL_MSG` write sets the bit from
/// its `system_flags`, as the firmware's `set_ctrl` does.
#[test]
fn wp_sw2x_the_system_mute_is_one_bit() {
    let mut s = FakeDevice::demo_audio_link_as(AudioLinkSetup {
        role: LinkRole::Sender,
        ..AudioLinkSetup::default()
    });
    let mut tx = LinkTx::decode(&get(&mut s, alink::EXT_LINK_TX, 16).unwrap()).unwrap();
    tx.flags = 1;
    set(&mut s, alink::EXT_LINK_TX, &tx.encode()).unwrap();
    let flags = |s: &mut FakeDevice| {
        SuperframeSnapshot::decode(&get(s, alink::EXT_LINK_CTRL_MSG, 24).unwrap())
            .unwrap()
            .flags
            & 1
    };
    assert_eq!(flags(&mut s), 1);
    let c = FrameCounter::decode(&get(&mut s, alink::EXT_FRAME_COUNTER, 16).unwrap()).unwrap();
    let mute = ApplyRequest {
        frame: c.frame + 4800,
        op: ApplyOp::Mute,
        arg: 0,
        value: 0,
    };
    set(&mut s, alink::EXT_LINK_CTRL_MSG, &mute.encode_ctrl_msg(0)).unwrap();
    assert_eq!(flags(&mut s), 0, "a write with system_flags 0 clears it");
}
