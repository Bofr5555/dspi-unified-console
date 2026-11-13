//! WP SW1: the probe reads `EXT_LINK_INFO` only when `EXT_LIST` names it, and
//! the link's I/O, against a mock device. Old firmware gets no extra traffic:
//! the transfers are counted.

use super::*;
use crate::ext::{self as sx, ApplyOp, ApplyRequest, ExtError, LinkPhase, LinkSlots, LinkState};
use dspi_proto::ext::audio_link::{LinkRole, LinkStatus};
use dspi_transport::MockTransport;
use dspi_transport::mock::{Direction, Reply};

fn list(ids: &[u16]) -> Vec<u8> {
    ids.iter().flat_map(|id| id.to_le_bytes()).collect()
}

fn info(role: LinkRole) -> LinkInfo {
    LinkInfo {
        role,
        protocol_version: 1,
        class_mask: 1,
        max_links: 1,
        max_channels: 8,
        slots_per_unit: 8,
        flags: 0,
        rx_pin: 0,
        tx_pin: 0xFF,
        feature_bits: 0,
        d_local_min: 112,
        d_local_max: 1024,
    }
}

/// The transfers a probe makes against `replies`, and its state.
fn probe(replies: Vec<Reply>) -> (ExtState, Vec<(Direction, u16)>) {
    let mut t = MockTransport::new().reply(op::REQ_EXTENSION, Reply::Sequence(replies));
    let log = t.log_handle();
    let e = ExtState::probe(&mut t).unwrap();
    let seen = log
        .lock()
        .unwrap()
        .iter()
        .map(|e| (e.direction, e.value))
        .collect();
    (e, seen)
}

#[test]
fn wp_sw1_old_firmware_gets_no_extra_transfer() {
    // Stock firmware: one EXT_LIST, which stalls.
    let (e, seen) = probe(vec![Reply::Stall]);
    assert_eq!(seen, vec![(Direction::In, 0)]);
    assert!(!e.audio_link() && !e.link_input());

    // bofr-main de9614c: 1-6 listed, no link. The same reads as before SW1:
    // the list and the four known sub-commands, nothing at 0x8000.
    let (e, seen) = probe(vec![
        Reply::Data(list(&[1, 2, 3, 4, 5, 6])),
        Reply::Data(vec![0x02, 0x13, 0, 0]),
        Reply::Data(vec![0x0D; 16]),
        Reply::Data(vec![3, 0, 0, 0]),
        Reply::Data(vec![1; 20]),
    ]);
    assert_eq!(seen.len(), 5, "{seen:?}");
    assert!(seen.iter().all(|(_, v)| *v < 0x8000));
    assert!(!e.audio_link());
    assert!(!e.link_state_events() && !e.apply_result_events());
}

#[test]
fn wp_sw1_a_listed_link_info_is_read_once() {
    let (e, seen) = probe(vec![
        Reply::Data(list(&[3, 0x8000, 0x8001, 0x8003])),
        Reply::Data(vec![0b1100, 0, 0, 0]),
        Reply::Data(info(LinkRole::Receiver).encode().to_vec()),
    ]);
    assert_eq!(
        seen,
        vec![
            (Direction::In, 0),
            (Direction::In, 3),
            (Direction::In, 0x8000)
        ]
    );
    assert!(e.audio_link() && e.link_input());
    assert!(e.link_state_events() && e.apply_result_events());
    assert_eq!(e.link_info.unwrap().max_channels, 8);
    // Listed but not read: status and slots are read on demand only.
    assert!(e.has(0x8001) && e.has(0x8003));

    // A sender has the link but no link input.
    let (e, _) = probe(vec![
        Reply::Data(list(&[0x8000])),
        Reply::Data(info(LinkRole::Sender).encode().to_vec()),
    ]);
    assert!(e.audio_link() && !e.link_input());

    // Role 0 or another protocol version: no link (2.8.13).
    let (e, _) = probe(vec![
        Reply::Data(list(&[0x8000])),
        Reply::Data(info(LinkRole::None).encode().to_vec()),
    ]);
    assert!(e.link_info.is_some() && !e.audio_link() && !e.link_input());
    let mut v2 = info(LinkRole::Receiver);
    v2.protocol_version = 2;
    let (e, _) = probe(vec![
        Reply::Data(list(&[0x8000])),
        Reply::Data(v2.encode().to_vec()),
    ]);
    assert!(!e.audio_link() && !e.link_input());

    // A short or stalled info is absent, not an error.
    let (e, _) = probe(vec![Reply::Data(list(&[0x8000])), Reply::Data(vec![1; 31])]);
    assert!(e.has(0x8000) && !e.audio_link());
    let (e, _) = probe(vec![Reply::Data(list(&[0x8000])), Reply::Stall]);
    assert!(!e.audio_link());
}

#[test]
fn wp_sw1_link_io_uses_the_specs_ids_and_sizes() {
    let status = LinkStatus {
        state: LinkState::Locked,
        class: 1,
        rate_code: 1,
        channels: 8,
        hops: 0,
        los_pin: 0xFF,
        sender_id: 1,
        flags: 0,
        frame: 1234,
        lock_count: 1,
        loss_count: 0,
        crc8_errors: 0,
        crc16_errors: 0,
        bit_slips: 0,
        superframe_errors: 0,
        vcxo_ppm: 3.0,
        phase_error_frames: 0.0,
        fill_frames: 64,
        hop_delay_frames: 0,
    };
    let mut t = MockTransport::new().reply(
        op::REQ_EXTENSION,
        Reply::Sequence(vec![Reply::Data(status.encode().to_vec())]),
    );
    let log = t.log_handle();
    assert_eq!(sx::read_link_status(&mut t).unwrap(), status);
    let slots = LinkSlots([0, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    sx::write_link_slots(&mut t, slots).unwrap();
    sx::write_link_phase(
        &mut t,
        LinkPhase {
            mode: 1,
            d_local: 128,
            hops_offset: 0,
            flags: 0,
            state: 2,
        },
    )
    .unwrap();
    sx::apply_at_frame(
        &mut t,
        ApplyRequest {
            frame: 48_000,
            op: ApplyOp::Mute,
            arg: 0,
            value: 0,
        },
    )
    .unwrap();
    sx::reset_link_stats(&mut t).unwrap();
    let log = log.lock().unwrap();
    let seen: Vec<_> = log
        .iter()
        .map(|e| (e.direction, e.value, e.payload.clone()))
        .collect();
    assert_eq!((seen[0].0, seen[0].1), (Direction::In, 0x8001));
    assert_eq!((seen[1].1, seen[1].2.clone()), (0x8003, slots.0.to_vec()));
    // The state byte is the device's: sent as 0.
    assert_eq!(
        (seen[2].1, seen[2].2.clone()),
        (0x8004, vec![1, 0, 128, 0, 0, 0, 0, 0])
    );
    assert_eq!((seen[3].1, seen[3].2.len()), (0x8006, 16));
    assert_eq!((seen[4].1, seen[4].2.clone()), (0x8009, vec![1]));
    assert!(seen[1..].iter().all(|e| e.0 == Direction::Out));
}

#[test]
fn wp_sw1_short_or_stalled_link_reads_are_errors_not_panics() {
    let mut t = MockTransport::new().data(op::REQ_EXTENSION, vec![2; 10]);
    assert!(matches!(
        sx::read_link_status(&mut t),
        Err(ExtError::Transport(TransportError::ShortRead {
            wanted: 52,
            ..
        }))
    ));
    let mut t = MockTransport::new().reply(op::REQ_EXTENSION, Reply::Stall);
    assert!(matches!(
        sx::read_frame_counter(&mut t),
        Err(ExtError::Unsupported { id: 0x8005 })
    ));
    assert!(matches!(
        sx::write_link_slots(&mut t, LinkSlots::SILENT),
        Err(ExtError::Refused { id: 0x8003 })
    ));
    let msg = sx::read_frame_counter(&mut t).unwrap_err().to_string();
    assert!(msg.contains("frame counter"), "{msg}");
}
