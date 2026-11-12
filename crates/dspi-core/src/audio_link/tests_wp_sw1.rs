//! WP SW1: the audio link through `DspiCore`, against `FakeDevice`'s link
//! model and as firmware without the link (no traffic, `Unsupported`), and
//! its events in WP P0M's event log.

use std::time::{Duration, Instant};

use dspi_proto::generated::notify as nfy;
use dspi_transport::fake::{LINK_SIM_LOSE, LINK_SIM_REANCHOR};
use dspi_transport::{Capture, FakeOptions, FakeRemote};

use super::*;
use crate::consts::NOTIFY_INPUT_FORMAT;
use crate::ext::tests_wp_c8::{Faults, Probed, k340_options, probed};
use crate::{EVENT_READ_MAX, EventEntry};

const WAIT: Duration = Duration::from_secs(8);

fn link(audio_link: bool) -> Probed {
    let options = FakeOptions {
        audio_link,
        ..k340_options()
    };
    probed(Capture::demo().bulk, options, Faults::default())
}

/// Every link call, each expected to fail `Unsupported`.
fn every_call(core: &DspiCore) -> Vec<(&'static str, Result<()>)> {
    vec![
        ("status", core.fetch_audio_link_status().map(drop)),
        ("info", core.fetch_audio_link_info().map(drop)),
        ("slots", core.fetch_audio_link_slots().map(drop)),
        ("set slots", core.set_audio_link_slots(vec![0; 8])),
        ("phase", core.fetch_audio_link_phase().map(drop)),
        ("set phase", core.set_audio_link_phase(1, 128, 0)),
        ("counter", core.fetch_link_frame_counter().map(drop)),
        ("apply", core.apply_at_frame(1, 1, 0, 0)),
        ("apply status", core.fetch_apply_at_frame_status().map(drop)),
        ("stats reset", core.reset_audio_link_stats()),
    ]
}

#[test]
fn wp_sw1_without_the_link_every_call_is_unsupported_with_no_traffic() {
    // bofr-main de9614c's extension channel, no link.
    let p = link(false);
    assert_eq!(p.core.audio_link_info().unwrap(), None);
    let before = p.traffic();
    for (name, r) in every_call(&p.core) {
        assert!(matches!(r, Err(DspiError::Unsupported)), "{name}: {r:?}");
    }
    // Input source 7 too.
    assert!(matches!(
        p.core.set_input_source(7),
        Err(DspiError::Unsupported)
    ));
    assert_eq!(p.traffic(), before, "nothing was sent");

    // Stock firmware (no extension channel at all).
    let p = probed(
        Capture::demo().bulk,
        FakeOptions {
            ack_unknown_sets: true,
            ..FakeOptions::default()
        },
        Faults::default(),
    );
    let before = p.traffic();
    for (name, r) in every_call(&p.core) {
        assert!(matches!(r, Err(DspiError::Unsupported)), "{name}: {r:?}");
    }
    assert_eq!(p.traffic(), before);
    assert!(p.core.set_input_source(8).is_err());
}

#[test]
fn wp_sw1_the_probe_adds_one_read_only_for_a_link_unit() {
    // The connect probe of a unit without the link is unchanged by SW1: the
    // list names no 0x8000, so it is never read. With the link, one more
    // (both list EXT_NOTIFY_FEATURES here, which a link build always does).
    let probe = |audio_link| {
        let options = FakeOptions {
            audio_link,
            bulk_source: true,
            ..k340_options()
        };
        probed(Capture::demo().bulk, options, Faults::default()).traffic()
    };
    let (without, with) = (probe(false), probe(true));
    assert_eq!(with.0, without.0 + 1, "{without:?} -> {with:?}");
    assert_eq!(with.1, without.1);
}

#[test]
fn wp_sw1_the_calls_against_the_fake_link() {
    let p = link(true);
    let c = &p.core;
    let info = c.audio_link_info().unwrap().unwrap();
    assert_eq!((info.role.as_str(), info.role_code), ("receiver", 1));
    assert!(info.link_input && info.state_events && info.apply_events);
    assert_eq!((info.d_local_min, info.d_local_max), (112, 1024));
    assert_eq!(c.fetch_audio_link_info().unwrap(), info);

    let s = c.fetch_audio_link_status().unwrap();
    assert_eq!(
        (s.state.as_str(), s.link_class, s.channels),
        ("locked", 1, 8)
    );
    assert!(s.frame > 0);

    assert_eq!(
        c.fetch_audio_link_slots().unwrap(),
        (0..8).collect::<Vec<u8>>()
    );
    c.set_audio_link_slots(vec![4, 5, 255, 255, 255, 255, 255, 255])
        .unwrap();
    assert_eq!(c.fetch_audio_link_slots().unwrap()[..3], [4, 5, 255]);

    c.set_audio_link_phase(1, 256, -2).unwrap();
    let ph = c.fetch_audio_link_phase().unwrap();
    assert_eq!(
        (ph.mode, ph.d_local, ph.hops_offset, ph.state),
        (1, 256, -2, 2)
    );
    // Out of the unit's range: the device refuses.
    assert!(matches!(
        c.set_audio_link_phase(1, 100, 0),
        Err(DspiError::Refused { .. })
    ));

    let f1 = c.fetch_link_frame_counter().unwrap();
    std::thread::sleep(Duration::from_millis(5));
    let f2 = c.fetch_link_frame_counter().unwrap();
    assert!(f2.frame > f1.frame, "the counter runs");

    c.apply_at_frame(f2.frame + 48_000_000, 1, 0, 0).unwrap();
    assert_eq!(c.fetch_apply_at_frame_status().unwrap().pending, 1);
    c.apply_at_frame(0, 0, 0, 0).unwrap();
    let st = c.fetch_apply_at_frame_status().unwrap();
    assert_eq!(
        (st.pending, st.last_op.as_str(), st.last_result.as_str()),
        (0, "mute", "cancelled")
    );

    c.reset_audio_link_stats().unwrap();
    assert_eq!(c.fetch_audio_link_status().unwrap().lock_count, 0);

    c.set_input_source(INPUT_SOURCE_AUDIO_LINK).unwrap();
    assert_eq!(c.get_state().input_source, 7);
}

#[test]
fn wp_sw1_bad_arguments_send_nothing() {
    let p = link(true);
    let c = &p.core;
    let before = p.traffic();
    let invalid = |r: Result<()>| matches!(r, Err(DspiError::InvalidArgument { .. }));
    assert!(invalid(c.set_audio_link_slots(vec![0; 7])));
    assert!(invalid(c.set_audio_link_slots(vec![64; 8])));
    assert!(invalid(c.set_audio_link_phase(2, 128, 0)));
    assert!(invalid(c.set_audio_link_phase(1, 128, 65)));
    assert!(invalid(c.set_audio_link_phase(1, 128, i16::MIN)));
    assert!(invalid(c.apply_at_frame(1, 9, 0, 0)));
    assert!(invalid(c.apply_at_frame(1, 3, 10, 0)));
    assert!(invalid(c.apply_at_frame(1, 4, 0, 1)));
    assert!(invalid(c.apply_at_frame(1, 1, 1, 0)));
    assert!(matches!(
        c.set_input_source(8),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert_eq!(p.traffic(), before);
}

#[test]
fn wp_sw1_a_hub_viewer_reads_no_link() {
    // A DSPi Link viewer: the hub refuses the link's reads.
    let p = probed(
        Capture::demo().bulk,
        FakeOptions {
            audio_link: true,
            ..k340_options()
        },
        Faults {
            deny_ext_reads_from: Some(0x8000),
            ..Faults::default()
        },
    );
    assert_eq!(p.core.audio_link_info().unwrap(), None);
    assert!(matches!(
        p.core.fetch_audio_link_status(),
        Err(DspiError::Unsupported)
    ));
}

#[test]
fn wp_sw1_the_demo_with_the_link_opens() {
    let core = DspiCore::new();
    core.open_demo_audio_link().unwrap();
    assert!(core.audio_link_info().unwrap().unwrap().link_input);
    core.open_demo().unwrap();
    assert_eq!(core.audio_link_info().unwrap(), None);
}

// ---------------------------------------------------------------- events

/// Push `INPUT_FORMAT` and process until it is seen: everything pushed
/// before it has been processed too.
fn settle(core: &DspiCore, remote: &FakeRemote) {
    remote.push(vec![
        2,
        nfy::NOTIFY_EVT_INPUT_FORMAT as u8,
        0,
        0,
        2,
        0,
        0,
        0,
    ]);
    let start = Instant::now();
    while core.process_notifications().flags & NOTIFY_INPUT_FORMAT == 0 {
        assert!(start.elapsed() < WAIT, "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn entries(core: &DspiCore, kind: &str) -> Vec<EventEntry> {
    core.event_log_read(String::new(), 0, EVENT_READ_MAX)
        .entries
        .into_iter()
        .filter(|e| e.kind == kind)
        .collect()
}

#[test]
fn wp_sw1_loss_resync_and_apply_results_reach_the_event_log() {
    let p = link(true);
    let (c, remote) = (&p.core, &p.remote);
    settle(c, remote);
    assert!(remote.link_sim(LINK_SIM_LOSE));
    assert!(remote.link_sim(LINK_SIM_REANCHOR));
    let now = c.fetch_link_frame_counter().unwrap().frame;
    c.apply_at_frame(now.saturating_sub(10), 1, 0, 0).unwrap();
    settle(c, remote);
    let log = entries(c, "link");
    let seen: Vec<_> = log
        .iter()
        .map(|e| (e.source.as_str(), e.text.as_str()))
        .collect();
    assert_eq!(
        seen,
        [
            ("lost", "Audio link lost (LOS): outputs muted"),
            ("resync", "Audio link re-anchored (a short silence), L1"),
            (
                "apply",
                "Apply at frame: mute applied late (its frame had passed)"
            ),
        ],
        "{log:?}"
    );
    // The apply record carries its target frame's low 32 bits.
    assert_eq!(
        log[2].value,
        Some(f64::from((now.saturating_sub(10) & 0xFFFF_FFFF) as u32))
    );
    // Nothing else went into the log as a plain notification.
    assert!(
        entries(c, "notification")
            .iter()
            .all(|e| !e.text.contains("0xE1") && !e.text.contains("0xE2")),
    );
}

#[test]
fn wp_sw1_link_events_without_the_feature_bits_are_not_logged() {
    // A device sending 0xE1 but not announcing bit 2 (stock firmware, or a
    // later upstream use of the id): not read as a link event.
    let p = link(false);
    let (c, remote) = (&p.core, &p.remote);
    settle(c, remote);
    remote.push(vec![2, 0xE1, 0, 0, 3, 1, 2, 0, 0, 0, 0, 0]);
    remote.push(vec![2, 0xE2, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
    settle(c, remote);
    assert!(entries(c, "link").is_empty());
}
