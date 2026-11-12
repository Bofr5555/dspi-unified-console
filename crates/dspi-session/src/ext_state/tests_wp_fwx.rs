//! WP FWX: the probe reads `EXT_RESET_INFO` and the clip-event flag, and the
//! identify and reset-info I/O, against a mock device.

use super::*;
use crate::ext::{self as sx, ExtError, IdentifyRequest, ResetReason};
use dspi_transport::MockTransport;
use dspi_transport::mock::Reply;

/// A warm reset after the watchdog fired: hang 1, 12 s up, 3600 s before.
fn reset_reply() -> Vec<u8> {
    let mut b = vec![4, 1, 1, 1];
    b.extend(12u32.to_le_bytes());
    b.extend(3600u32.to_le_bytes());
    b.extend(0x0100_0000u32.to_le_bytes());
    b.extend(0x101u32.to_le_bytes());
    b
}

fn list(ids: &[u16]) -> Vec<u8> {
    ids.iter().flat_map(|id| id.to_le_bytes()).collect()
}

#[test]
fn wp_fwx_the_probe_reads_reset_info_and_the_clip_flag() {
    let mut t = MockTransport::new().reply(
        op::REQ_EXTENSION,
        Reply::Sequence(vec![
            Reply::Data(list(&[3, 4, 5])),
            Reply::Data(vec![3, 0, 0, 0]),
            Reply::Data(reset_reply()),
        ]),
    );
    let e = ExtState::probe(&mut t).unwrap();
    assert_eq!(e.supported, vec![3, 4, 5]);
    assert!(e.bulk_source());
    assert!(e.clip_events());
    let r = e.reset_info.unwrap();
    assert_eq!(r.reason, ResetReason::Watchdog);
    assert_eq!(r.prev_uptime_s, Some(3600));
}

#[test]
fn wp_fwx_old_firmware_has_neither() {
    // bofr-main 04122d3: 1, 2, 3 with only the bulk-source bit.
    let e = from_replies(
        supported_from_list(&list(&[1, 2, 3])),
        &[(g::EXT_NOTIFY_FEATURES, vec![1, 0, 0, 0])],
    );
    assert!(e.bulk_source());
    assert!(!e.clip_events());
    assert_eq!(e.reset_info, None);
    // Stock firmware: nothing.
    assert!(!ExtState::default().clip_events());
    // Listed, but the read was short or refused: absent, no panic.
    let e = from_replies(
        supported_from_list(&list(&[5])),
        &[(g::EXT_RESET_INFO, vec![4, 1])],
    );
    assert_eq!(e.reset_info, None);
    // Not listed: the reply is ignored.
    let e = from_replies(Vec::new(), &[(g::EXT_RESET_INFO, reset_reply())]);
    assert_eq!(e.reset_info, None);
}

#[test]
fn wp_fwx_reads_and_the_identify_write() {
    let mut status = vec![5, 0xFF, 1, 2, 0x27, 0x49, 0, 0x20];
    status.extend(
        b"pico2"
            .iter()
            .copied()
            .chain(std::iter::repeat(0))
            .take(16),
    );
    status.extend([0u8; 32]);
    let mut t = MockTransport::new().reply(op::REQ_EXTENSION, Reply::Data(status));
    let s = sx::read_identify(&mut t).unwrap();
    assert_eq!(
        (s.remaining_s, s.board.as_str(), s.hw_rev.as_str()),
        (5, "pico2", "")
    );

    let mut t = MockTransport::new().reply(op::REQ_EXTENSION, Reply::Data(reset_reply()));
    assert_eq!(sx::read_reset_info(&mut t).unwrap().uptime_s, 12);

    // A short reply is a short read, not a decode of garbage.
    let mut t = MockTransport::new().reply(op::REQ_EXTENSION, Reply::Data(vec![0; 19]));
    assert!(matches!(
        sx::read_reset_info(&mut t),
        Err(ExtError::Transport(TransportError::ShortRead { .. }))
    ));

    // Stock firmware stalls: unsupported on a read, refused on the write.
    let mut t = MockTransport::new().reply(op::REQ_EXTENSION, Reply::Stall);
    assert!(matches!(
        sx::read_identify(&mut t),
        Err(ExtError::Unsupported { id: 4 })
    ));
    let req = IdentifyRequest {
        seconds: 10,
        tone_output: None,
    };
    assert!(matches!(
        sx::identify(&mut t, req),
        Err(ExtError::Refused { id: 4 })
    ));
}
