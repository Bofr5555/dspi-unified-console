//! WP T2: a MIDI notification queue overflow, seen end to end through the
//! session's notify reader. The transport drops the oldest packets
//! (`midi.rs`, `NOTIFY_QUEUE`); the gap in the `seq` byte must make
//! `dspi_session::Notifications` mark the next delivered packet `lost`, so
//! the client re-reads the bulk image.

use std::sync::Arc;
use std::time::{Duration, Instant};

use dspi_proto::frame::{DEFAULT_MFR_ID, Frame, encode_sysex};
use dspi_proto::generated::notify as n;
use dspi_session::{Event, Notification, Notifications};
use dspi_transport::Transport;
use dspi_transport::midi::{MidiPipe, MidiTransport, PipeError};

/// The tests send no requests.
struct NullPipe;

impl MidiPipe for NullPipe {
    fn send(&self, _: &[u8]) -> Result<(), PipeError> {
        Ok(())
    }
}

/// A v2 `BULK_INVALIDATED` (source MIDI) as the device sends it on the
/// MIDI port: tag 0, frame type 0x40.
fn bulk_invalidated(seq: u8) -> Vec<u8> {
    let packet = vec![
        2,
        n::NOTIFY_EVT_BULK_INVALIDATED as u8,
        0,
        seq,
        n::PARAM_SRC_MIDI as u8,
        0,
        0,
        0,
    ];
    encode_sysex(DEFAULT_MFR_ID, 0, &Frame::Notification { packet })
}

/// Drain until `count` notifications arrived or 5 s passed.
fn collect(notes: &Notifications, count: usize) -> Vec<Notification> {
    let started = Instant::now();
    let mut got = Vec::new();
    while got.len() < count && started.elapsed() < Duration::from_secs(5) {
        got.extend(notes.drain());
        std::thread::sleep(Duration::from_millis(5));
    }
    got
}

#[test]
fn wp_t2_a_midi_queue_overflow_marks_the_next_notification_lost() {
    const QUEUE: usize = 1024; // the transport's NOTIFY_QUEUE
    const SENT: usize = QUEUE + 76;

    let (t, input) = MidiTransport::new(Arc::new(NullPipe), DEFAULT_MFR_ID, "MIDI-E2E");
    let notes = Notifications::start(t.notifications().unwrap()).unwrap();

    // One packet first, so the reader has a sequence to compare against.
    input.receive(&bulk_invalidated(1));
    let first = collect(&notes, 1);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].seq, 1);
    assert!(!first[0].lost);
    assert!(matches!(first[0].event, Event::BulkInvalidated { .. }));

    // A burst the reader cannot keep up with: one `receive` call holds the
    // transport's lock throughout, so its queue overflows and drops the
    // oldest 76 packets (seq 2..=77).
    let burst: Vec<u8> = (0..SENT)
        .flat_map(|i| bulk_invalidated((2 + i) as u8))
        .collect();
    input.receive(&burst);

    let got = collect(&notes, QUEUE);
    assert_eq!(got.len(), QUEUE, "the newest packets are kept");
    assert_eq!(got[0].seq, 78, "the oldest were dropped");
    assert!(got[0].lost, "the gap is reported as lost");
    assert!(
        got[1..].iter().all(|n| !n.lost),
        "only the packet after the gap is marked"
    );
    assert_eq!(got.last().unwrap().seq, (1 + SENT) as u8);
    drop(t);
}
