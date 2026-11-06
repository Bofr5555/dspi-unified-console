//! The echo filter (PLAN §1.2): our own bulk write's `BULK_SET`
//! invalidation is claimed at most once, and nothing else is claimed by it.

use std::time::{Duration, Instant};

use dspi_proto::generated::notify as n;
use dspi_session::notify::EchoFilter;
use dspi_transport::TransportKind;
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Step {
    Started,
    Finished(bool),
    Wait(u16),
    Note(u8),
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        Just(Step::Started),
        any::<bool>().prop_map(Step::Finished),
        (0u16..1500).prop_map(Step::Wait),
        prop_oneof![
            Just(n::PARAM_SRC_BULK_SET as u8),
            Just(n::PARAM_SRC_HOST_SET as u8),
            Just(n::PARAM_SRC_MIDI as u8),
            any::<u8>(),
        ]
        .prop_map(Step::Note),
    ]
}

proptest! {
    /// However starts, replies, waits and notifications interleave, a
    /// BULK_SET is claimed at most once per started bulk write, never
    /// without one, and never after a failed reply.
    #[test]
    fn proptest_echo_filter_claims_each_bulk_write_at_most_once(
        steps in prop::collection::vec(step(), 0..60),
        midi in any::<bool>(),
    ) {
        let kind = if midi { TransportKind::Midi } else { TransportKind::Usb };
        let mut f = EchoFilter::default();
        let mut now = Instant::now();
        let mut writes = 0usize;
        let mut claimed = 0usize;
        let mut armed = false;
        for s in steps {
            match s {
                Step::Started => {
                    f.bulk_write_started(now);
                    writes += 1;
                    armed = true;
                }
                Step::Finished(ok) => {
                    f.bulk_write_finished(now, ok);
                    if !ok {
                        armed = false;
                    }
                }
                Step::Wait(ms) => now += Duration::from_millis(u64::from(ms)),
                Step::Note(src) => {
                    let own = f.classify_invalidation(now, kind, src, None, None);
                    if u16::from(src) == n::PARAM_SRC_BULK_SET && own {
                        prop_assert!(armed, "claimed without a pending write of ours");
                        claimed += 1;
                        armed = false;
                    }
                }
            }
            prop_assert!(claimed <= writes);
        }
    }

    /// Tagged firmware: only our path's source with a SET of ours out is
    /// ever claimed, and it settles everything.
    #[test]
    fn proptest_echo_filter_tagged_only_claims_our_path_when_outstanding(
        steps in prop::collection::vec(step(), 0..60),
        midi in any::<bool>(),
    ) {
        let kind = if midi { TransportKind::Midi } else { TransportKind::Usb };
        let path = if midi { n::PARAM_SRC_MIDI } else { n::PARAM_SRC_HOST_SET };
        let mut f = EchoFilter::default();
        f.set_bulk_source(true);
        let mut now = Instant::now();
        let mut outstanding = 0usize;
        for s in steps {
            match s {
                Step::Started => {
                    f.bulk_write_started(now);
                    outstanding += 1;
                }
                Step::Finished(ok) => {
                    f.bulk_write_finished(now, ok);
                    if !ok {
                        outstanding = outstanding.saturating_sub(1);
                    }
                }
                Step::Wait(ms) => now += Duration::from_millis(u64::from(ms)),
                Step::Note(src) => {
                    if f.classify_invalidation(now, kind, src, None, None) {
                        prop_assert_eq!(u16::from(src), path, "another source claimed");
                        prop_assert!(outstanding > 0, "claimed with nothing outstanding");
                        outstanding = 0;
                        prop_assert!(!f.own_bulk_pending(now), "a match settles all");
                    }
                }
            }
        }
    }

    /// Over Link the origin decides; the filter's bulk state changes nothing.
    #[test]
    fn proptest_echo_filter_leaves_link_to_the_origin(
        src in any::<u8>(),
        origin in prop::option::of(any::<u16>()),
        session in prop::option::of(any::<u16>()),
        started in any::<bool>(),
    ) {
        let mut f = EchoFilter::default();
        let now = Instant::now();
        if started {
            f.bulk_write_started(now);
        }
        let own = f.classify_invalidation(now, TransportKind::Link, src, origin, session);
        let expected = matches!((origin, session), (Some(o), Some(s)) if o != 0 && o == s);
        prop_assert_eq!(own, expected);
    }
}
