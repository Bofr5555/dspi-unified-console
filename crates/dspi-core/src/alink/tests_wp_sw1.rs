//! WP SW1: the audio link through `DspiCore`, against `FakeDevice`'s alink
//! model and as firmware without the alink (no traffic, `Unsupported`), and
//! its events in WP P0M's event log.

use std::time::{Duration, Instant};

use dspi_proto::generated::notify as nfy;
use dspi_transport::fake::{ALINK_SIM_LOSE, ALINK_SIM_REANCHOR};
use dspi_transport::{Capture, FakeOptions, FakeRemote};

use super::*;
use crate::consts::NOTIFY_INPUT_FORMAT;
use crate::ext::tests_wp_c8::{Faults, Probed, k340_options, probed};
use crate::{EVENT_READ_MAX, EventEntry};

const WAIT: Duration = Duration::from_secs(8);

fn alink_unit(alink: bool) -> Probed {
    let options = FakeOptions {
        alink,
        ..k340_options()
    };
    probed(Capture::demo().bulk, options, Faults::default())
}

/// Every alink call, each expected to fail `Unsupported`.
fn every_call(core: &DspiCore) -> Vec<(&'static str, Result<()>)> {
    vec![
        ("status", core.fetch_alink_status().map(drop)),
        ("info", core.fetch_alink_info().map(drop)),
        ("slots", core.fetch_alink_slots().map(drop)),
        ("set slots", core.set_alink_slots(vec![0; 8])),
        ("phase", core.fetch_alink_phase().map(drop)),
        ("set phase", core.set_alink_phase(1, 128, 0)),
        ("counter", core.fetch_alink_frame_counter().map(drop)),
        ("apply", core.apply_at_frame(1, 1, 0, 0)),
        ("apply status", core.fetch_apply_at_frame_status().map(drop)),
        ("stats reset", core.reset_alink_stats()),
    ]
}

#[test]
fn wp_sw1_without_the_link_every_call_is_unsupported_with_no_traffic() {
    // bofr-main de9614c's extension channel, no alink.
    let p = alink_unit(false);
    assert_eq!(p.core.alink_info().unwrap(), None);
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
    // The connect probe of a unit without the alink is unchanged by SW1: the
    // list names no 0x8000, so it is never read. With the alink, one more
    // (both list EXT_NOTIFY_FEATURES here, which an alink build always does).
    let probe = |alink| {
        let options = FakeOptions {
            alink,
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
    let p = alink_unit(true);
    let c = &p.core;
    let info = c.alink_info().unwrap().unwrap();
    assert_eq!((info.role.as_str(), info.role_code), ("receiver", 1));
    assert!(info.alink_input && info.state_events && info.apply_events);
    // The firmware's range at the default 4 ms option (WP FIXS1).
    assert_eq!((info.d_local_min, info.d_local_max), (257, 399));
    assert_eq!(c.fetch_alink_info().unwrap(), info);

    let s = c.fetch_alink_status().unwrap();
    assert_eq!(
        (s.state.as_str(), s.alink_class, s.channels),
        ("locked", 1, 8)
    );
    assert!(s.frame > 0);

    assert_eq!(c.fetch_alink_slots().unwrap(), (0..8).collect::<Vec<u8>>());
    c.set_alink_slots(vec![4, 5, 255, 255, 255, 255, 255, 255])
        .unwrap();
    assert_eq!(c.fetch_alink_slots().unwrap()[..3], [4, 5, 255]);

    c.set_alink_phase(1, 320, -2).unwrap();
    let ph = c.fetch_alink_phase().unwrap();
    assert_eq!(
        (ph.mode, ph.d_local, ph.hops_offset, ph.state),
        (1, 320, -2, 2)
    );
    // Out of the unit's range: the device refuses.
    assert!(matches!(
        c.set_alink_phase(1, 100, 0),
        Err(DspiError::Refused { .. })
    ));

    let f1 = c.fetch_alink_frame_counter().unwrap();
    std::thread::sleep(Duration::from_millis(5));
    let f2 = c.fetch_alink_frame_counter().unwrap();
    assert!(f2.frame > f1.frame, "the counter runs");

    c.apply_at_frame(f2.frame + 48_000_000, 1, 0, 0).unwrap();
    assert_eq!(c.fetch_apply_at_frame_status().unwrap().pending, 1);
    c.apply_at_frame(0, 0, 0, 0).unwrap();
    let st = c.fetch_apply_at_frame_status().unwrap();
    assert_eq!(
        (st.pending, st.last_op.as_str(), st.last_result.as_str()),
        (0, "mute", "cancelled")
    );

    c.reset_alink_stats().unwrap();
    assert_eq!(c.fetch_alink_status().unwrap().lock_count, 0);

    c.set_input_source(INPUT_SOURCE_ALINK).unwrap();
    assert_eq!(c.get_state().input_source, 7);
}

#[test]
fn wp_sw1_bad_arguments_send_nothing() {
    let p = alink_unit(true);
    let c = &p.core;
    let before = p.traffic();
    let invalid = |r: Result<()>| matches!(r, Err(DspiError::InvalidArgument { .. }));
    assert!(invalid(c.set_alink_slots(vec![0; 7])));
    assert!(invalid(c.set_alink_slots(vec![64; 8])));
    assert!(invalid(c.set_alink_phase(2, 128, 0)));
    assert!(invalid(c.set_alink_phase(1, 128, 65)));
    assert!(invalid(c.set_alink_phase(1, 128, i16::MIN)));
    // -- WP ATPROTO -- ops 9 and 10 are known now (2.8.7.1): 11 is the
    // first unknown op, and their arguments are judged.
    assert!(invalid(c.apply_at_frame(1, 11, 0, 0)));
    assert!(invalid(c.apply_at_frame(1, 9, 1, 0)));
    assert!(invalid(c.apply_at_frame(1, 10, 2, 0)));
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
            alink: true,
            ..k340_options()
        },
        Faults {
            deny_ext_reads_from: Some(0x8000),
            ..Faults::default()
        },
    );
    assert_eq!(p.core.alink_info().unwrap(), None);
    assert!(matches!(
        p.core.fetch_alink_status(),
        Err(DspiError::Unsupported)
    ));
}

#[test]
fn wp_sw1_the_demo_with_the_link_opens() {
    let core = DspiCore::new();
    core.open_demo_alink().unwrap();
    assert!(core.alink_info().unwrap().unwrap().alink_input);
    core.open_demo().unwrap();
    assert_eq!(core.alink_info().unwrap(), None);
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
    let p = alink_unit(true);
    let (c, remote) = (&p.core, &p.remote);
    settle(c, remote);
    assert!(remote.alink_sim(ALINK_SIM_LOSE));
    assert!(remote.alink_sim(ALINK_SIM_REANCHOR));
    let now = c.fetch_alink_frame_counter().unwrap().frame;
    c.apply_at_frame(now.saturating_sub(10), 1, 0, 0).unwrap();
    settle(c, remote);
    let log = entries(c, "audio_link");
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
    // later upstream use of the id): not read as an alink event.
    let p = alink_unit(false);
    let (c, remote) = (&p.core, &p.remote);
    settle(c, remote);
    remote.push(vec![2, 0xE1, 0, 0, 3, 1, 2, 0, 0, 0, 0, 0]);
    remote.push(vec![2, 0xE2, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
    settle(c, remote);
    assert!(entries(c, "audio_link").is_empty());
}

// ---------------------------------------------------------------- history

/// Fixup sw1 (F1): the four writes are journalled; slots undo, a phase change
/// is stepped over (it re-anchors), apply and the counter reset are actions.
mod tests_fixup_sw1 {
    use super::*;
    use crate::{HistorySource, ParamValue};

    #[test]
    fn wp_sw1_fixup_slots_are_journalled_and_undone() {
        let p = alink_unit(true);
        let c = &p.core;
        let identity: Vec<u8> = (0..8).collect();
        let mine = vec![4, 5, 255, 255, 255, 255, 255, 255];
        c.set_alink_slots(mine.clone()).unwrap();
        let h = c.history(10);
        assert_eq!(h.len(), 1, "{h:?}");
        let e = &h[0];
        assert_eq!(e.path, "ext.alink.slots");
        assert!(e.undoable);
        assert_eq!(e.before, Some(ParamValue::Bytes(identity.clone())));
        assert_eq!(e.after, ParamValue::Bytes(mine.clone()));
        // The same slots again: nothing moved, nothing recorded.
        c.set_alink_slots(mine.clone()).unwrap();
        assert_eq!(c.history(10).len(), 1);

        let undone = c.undo().unwrap();
        assert_eq!(
            undone.entry.map(|e| e.path).as_deref(),
            Some("ext.alink.slots")
        );
        assert_eq!(c.fetch_alink_slots().unwrap(), identity);
        assert_eq!(c.history(1)[0].source, HistorySource::Undo);
        let redone = c.redo().unwrap();
        assert!(redone.entry.is_some());
        assert_eq!(c.fetch_alink_slots().unwrap(), mine);
    }

    #[test]
    fn wp_sw1_fixup_a_phase_change_is_recorded_and_stepped_over() {
        let p = alink_unit(true);
        let c = &p.core;
        c.set_alink_slots(vec![3; 8]).unwrap();
        c.set_alink_phase(1, 320, -2).unwrap();
        let e = c.history(1).pop().unwrap();
        assert_eq!(e.path, "ext.alink.phase");
        assert!(!e.undoable);
        // [mode, d_local LE, hops LE]: 300 (the fake's start, WP FIXS1) to 320/-2.
        assert_eq!(e.before, Some(ParamValue::Bytes(vec![1, 44, 1, 0, 0])));
        assert_eq!(e.after, ParamValue::Bytes(vec![1, 64, 1, 0xFE, 0xFF]));
        // Undo steps over the phase (the delay stays) and undoes the slots.
        let m = c.undo().unwrap();
        assert_eq!(m.skipped.len(), 1);
        assert_eq!(m.skipped[0].path, "ext.alink.phase");
        assert_eq!(m.entry.unwrap().path, "ext.alink.slots");
        assert_eq!(c.fetch_alink_phase().unwrap().d_local, 320);
        assert_eq!(c.fetch_alink_slots().unwrap(), (0..8).collect::<Vec<u8>>());
    }

    #[test]
    fn wp_sw1_fixup_apply_and_stats_reset_are_actions() {
        let p = alink_unit(true);
        let c = &p.core;
        let f = c.fetch_alink_frame_counter().unwrap().frame + 48_000_000;
        c.apply_at_frame(f, 7, 2, 0).unwrap();
        c.reset_alink_stats().unwrap();
        let h = c.history(10);
        assert_eq!(h.len(), 2, "{h:?}");
        // Newest first.
        let (reset, apply) = (&h[0], &h[1]);
        assert_eq!(
            (apply.path.as_str(), apply.indices.as_slice()),
            ("ext.alink.apply", &[7u8, 2][..])
        );
        assert!(!apply.undoable && apply.before.is_none());
        let ParamValue::Bytes(req) = &apply.after else {
            panic!("{apply:?}");
        };
        assert_eq!(req[..8], f.to_le_bytes());
        assert_eq!(reset.path, "ext.alink.stats_reset");
        assert_eq!(reset.after, ParamValue::Trigger);
        assert!(!reset.undoable);
        // Nothing to undo: both are stepped over.
        let m = c.undo().unwrap();
        assert!(m.entry.is_none());
        assert_eq!(m.skipped.len(), 2);
        assert_eq!(c.fetch_apply_at_frame_status().unwrap().pending, 1);
    }

    #[test]
    fn wp_sw1_fixup_a_refused_write_records_nothing() {
        let p = alink_unit(true);
        let c = &p.core;
        // Out of the unit's delay range: the device refuses, no entry.
        assert!(c.set_alink_phase(1, 100, 0).is_err());
        assert!(c.history(10).is_empty());
    }
}
