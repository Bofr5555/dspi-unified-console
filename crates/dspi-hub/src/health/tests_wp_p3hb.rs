//! WP P3HB: the board's three facts on the health tracker alone, in virtual time:
//! the stream counters (one read a period, the two in turn), the clip events of the
//! last minute (only where announced) and the last change.

use dspi_session::events::{EventKind, EventRecord};

use super::*;

const WALL: u64 = 1_800_000_000;

fn cfg() -> HealthConfig {
    HealthConfig::default()
}

fn submits(out: &[Effect]) -> Vec<Read> {
    out.iter()
        .filter_map(|e| match e {
            Effect::Submit(r) => Some(*r),
            _ => None,
        })
        .collect()
}

fn healths(out: &[Effect]) -> Vec<BusMessage> {
    out.iter()
        .filter_map(|e| match e {
            Effect::Publish(m) if matches!(**m, BusMessage::Health { .. }) => Some((**m).clone()),
            _ => None,
        })
        .collect()
}

fn ext_list(ids: &[u16]) -> Vec<u8> {
    ids.iter().flat_map(|id| id.to_le_bytes()).collect()
}

fn meters(clip: u32) -> Vec<u8> {
    SystemStatus {
        peaks: vec![0; 17],
        cpu0: 10,
        cpu1: 20,
        clip_flags: clip,
        active_inputs: 2,
    }
    .encode()
}

/// A tracker past its attach reads, with `features` answered for
/// `EXT_NOTIFY_FEATURES` when it is in `ids`.
fn running(ids: &[u16], features: u32, start: Instant) -> Tracker {
    let c = cfg();
    let mut t = Tracker::new("S1".into(), 3, "1.1.7".into(), start);
    let mut out = Vec::new();
    t.step(start, WALL, &c, &mut out);
    t.answer(
        Read::ExtList,
        status::OK,
        &ext_list(ids),
        start,
        WALL,
        &c,
        &mut out,
    );
    t.answer(
        Read::Probe,
        status::OK,
        &meters(0),
        start,
        WALL,
        &c,
        &mut out,
    );
    t.poll_answer(true, start, &c);
    t.answer(
        Read::Temperature,
        status::OK,
        &3500u32.to_le_bytes(),
        start,
        WALL,
        &c,
        &mut out,
    );
    out.clear();
    t.step(start, WALL, &c, &mut out);
    let asked = submits(&out);
    assert_eq!(
        asked.contains(&Read::NotifyFeatures),
        ids.contains(&gext::EXT_NOTIFY_FEATURES),
        "the features once, where listed: {asked:?}"
    );
    if ids.contains(&gext::EXT_NOTIFY_FEATURES) {
        t.answer(
            Read::NotifyFeatures,
            status::OK,
            &features.to_le_bytes(),
            start,
            WALL,
            &c,
            &mut out,
        );
    }
    t
}

/// Run `t` for `span` from `start`, answering every read at once (`counter` gives the
/// counters' values at that time), and return every read made.
fn run_for(
    t: &mut Tracker,
    start: Instant,
    span: Duration,
    counter: impl Fn(Read, Duration) -> u32,
) -> Vec<Read> {
    let c = cfg();
    let mut reads = Vec::new();
    let mut now = start;
    while now < start + span {
        now = t.due().max(now);
        let mut out = Vec::new();
        t.status_reading(&meters(0), now, WALL);
        t.step(now, WALL, &c, &mut out);
        for r in submits(&out) {
            reads.push(r);
            let payload = match r {
                Read::Temperature => 3600u32.to_le_bytes().to_vec(),
                r if r.is_counter() => counter(r, now - start).to_le_bytes().to_vec(),
                _ => Vec::new(),
            };
            t.answer(r, status::OK, &payload, now, WALL, &c, &mut out);
        }
    }
    reads
}

// ── Counters ────────────────────────────────────────────────────────────────

fn is_counter(r: &Read) -> bool {
    r.is_counter()
}

#[test]
fn wp_p3hb_one_counter_read_a_period_the_four_in_turn_and_nothing_else_more() {
    let start = Instant::now();
    let mut t = running(&[], 0, start);
    let reads = run_for(&mut t, start, Duration::from_secs(600), |_, _| 0);
    let count = |r: Read| reads.iter().filter(|x| **x == r).count();
    // Ten minutes: twenty counter reads, five of each, one at a time.
    let counters: Vec<Read> = reads.iter().copied().filter(is_counter).collect();
    assert!((19..=21).contains(&counters.len()), "{reads:?}");
    for r in COUNTER_READS {
        assert!((4..=6).contains(&count(r)), "{r:?}: {reads:?}");
    }
    assert!(
        counters.windows(2).all(|w| w[0] != w[1]),
        "in turn: {counters:?}"
    );
    // The other reads are what P2HP's test counts: the counters are the only addition.
    assert!((19..=21).contains(&count(Read::Temperature)), "{reads:?}");
    assert_eq!(count(Read::Status), 0);
    assert_eq!(count(Read::NotifyFeatures), 0, "not listed: never asked");
}

#[test]
fn wp_p3hb_the_first_counter_read_is_half_a_period_after_attach() {
    let c = cfg();
    let start = Instant::now();
    let mut t = Tracker::new("S1".into(), 0, String::new(), start);
    let mut out = Vec::new();
    t.step(start, WALL, &c, &mut out);
    assert!(
        !submits(&out).iter().any(is_counter),
        "not with the attach reads"
    );
    out.clear();
    t.step(start + c.counters_every / 2, WALL, &c, &mut out);
    assert_eq!(
        submits(&out)
            .into_iter()
            .filter(is_counter)
            .collect::<Vec<_>>(),
        vec![Read::Underruns]
    );
}

/// The device's totals in this test: each rises by one every 10 s.
fn total(r: Read, at: Duration) -> u32 {
    let base = match r {
        Read::Underruns => 100,
        Read::PdmRingUnderruns => 20,
        Read::PdmDmaUnderruns => 1,
        _ => 7,
    };
    base + u32::try_from(at.as_secs() / 10).unwrap()
}

#[test]
fn wp_p3hb_counters_step_when_the_device_steps_them() {
    let c = cfg();
    let start = Instant::now();
    let mut t = running(&[], 0, start);
    assert!(
        t.record().get("counters").is_none(),
        "none before the first read"
    );
    // Reads at 15 s (S/PDIF), 45 s (starvation), 75 s (PDM ring), 105 s (PDM DMA).
    run_for(&mut t, start, Duration::from_secs(50), total);
    assert_eq!(
        t.record()["counters"],
        json!({"starvation": 11}),
        "the sum waits for its three parts"
    );
    run_for(
        &mut t,
        start + Duration::from_secs(50),
        Duration::from_secs(70),
        |r, at| total(r, at + Duration::from_secs(50)),
    );
    let first = t.record()["counters"].clone();
    assert_eq!(
        first,
        json!({"underruns": 101 + 27 + 11, "starvation": 11}),
        "{first}"
    );
    run_for(
        &mut t,
        start + Duration::from_secs(120),
        Duration::from_secs(120),
        |r, at| total(r, at + Duration::from_secs(120)),
    );
    let later = t.record()["counters"].clone();
    assert_eq!(
        later,
        json!({"underruns": 113 + 39 + 23, "starvation": 23}),
        "{later}"
    );
    // And in the bus's Health.
    let mut out = Vec::new();
    t.status_reading(&meters(0), start + Duration::from_secs(241), WALL);
    t.step(start + Duration::from_secs(242), WALL, &c, &mut out);
    let h = healths(&out);
    let [
        BusMessage::Health {
            counters: Some(sc), ..
        },
    ] = h.as_slice()
    else {
        panic!("{h:?}");
    };
    assert_eq!((sc.underruns, sc.starvation), (Some(175), Some(23)));
}

#[test]
fn wp_p3hb_a_counter_the_firmware_stalls_is_not_asked_again_and_counts_zero() {
    let c = cfg();
    let start = Instant::now();
    let mut t = running(&[], 0, start);
    let mut out = Vec::new();
    t.answer(
        Read::PdmRingUnderruns,
        status::ERROR,
        &[],
        start,
        WALL,
        &c,
        &mut out,
    );
    let reads = run_for(&mut t, start, Duration::from_secs(300), |_, _| 5);
    assert!(!reads.contains(&Read::PdmRingUnderruns), "{reads:?}");
    assert!(
        reads.iter().filter(|r| is_counter(r)).count() >= 9,
        "{reads:?}"
    );
    assert_eq!(
        t.record()["counters"],
        json!({"underruns": 10, "starvation": 5})
    );
    for r in COUNTER_READS {
        t.answer(r, status::ERROR, &[], start, WALL, &c, &mut out);
    }
    let reads = run_for(
        &mut t,
        start + Duration::from_secs(300),
        Duration::from_secs(120),
        |_, _| 5,
    );
    assert!(
        !reads.iter().any(is_counter),
        "all refused: none at all {reads:?}"
    );
}

// ── Clip per minute ─────────────────────────────────────────────────────────

#[test]
fn wp_p3hb_ten_clip_events_in_a_minute_count_ten_and_are_forgotten_after() {
    let c = cfg();
    let start = Instant::now();
    let mut t = running(
        &[gext::EXT_NOTIFY_FEATURES],
        NOTIFY_FEAT_CLIP_EVENT | 1,
        start,
    );
    assert_eq!(t.record()["clip_per_min"], 0, "announced, none yet");
    for i in 0..10u32 {
        t.clip_event(start + Duration::from_secs(u64::from(i) * 5), &c);
    }
    let at_50 = start + Duration::from_secs(50);
    t.forget_clips(at_50, &c);
    assert_eq!(t.record()["clip_per_min"], 10);
    let mut out = Vec::new();
    t.status_reading(&meters(1), at_50, WALL);
    t.step(at_50, WALL, &c, &mut out);
    let h = healths(&out);
    assert!(
        matches!(
            h.as_slice(),
            [BusMessage::Health {
                clip_per_min: Some(10),
                ..
            }]
        ),
        "{h:?}"
    );
    // The first went 60 s after it came; all of them a minute after the last.
    t.forget_clips(start + Duration::from_secs(61), &c);
    assert_eq!(t.record()["clip_per_min"], 9);
    t.forget_clips(start + Duration::from_secs(45 + 60), &c);
    assert_eq!(t.record()["clip_per_min"], 0);
}

#[test]
fn wp_p3hb_latch_only_firmware_has_no_clip_per_min() {
    let c = cfg();
    let start = Instant::now();
    // Not listed; listed without the bit; listed and stalled.
    let mut cases = vec![
        running(&[], 0, start),
        running(&[gext::EXT_NOTIFY_FEATURES], 1, start),
    ];
    let mut stalled = Tracker::new("S2".into(), 0, String::new(), start);
    let mut out = Vec::new();
    stalled.answer(
        Read::ExtList,
        status::OK,
        &ext_list(&[gext::EXT_NOTIFY_FEATURES]),
        start,
        WALL,
        &c,
        &mut out,
    );
    stalled.answer(
        Read::NotifyFeatures,
        status::ERROR,
        &[],
        start,
        WALL,
        &c,
        &mut out,
    );
    cases.push(stalled);
    for mut t in cases {
        t.clip_event(start, &c);
        assert!(t.record().get("clip_per_min").is_none(), "{}", t.record());
        let mut out = Vec::new();
        t.status_reading(&meters(1), start, WALL);
        t.step(start + c.status_every, WALL, &c, &mut out);
        assert!(
            healths(&out).iter().all(|m| matches!(
                m,
                BusMessage::Health {
                    clip_per_min: None,
                    ..
                }
            )),
            "{out:?}"
        );
        // Asked once at most, never again.
        let mut later = Vec::new();
        t.step(start + Duration::from_secs(3600), WALL, &c, &mut later);
        assert!(!submits(&later).contains(&Read::NotifyFeatures));
    }
}

#[test]
fn wp_p3hb_a_features_read_that_times_out_is_tried_again() {
    let c = cfg();
    let start = Instant::now();
    let mut t = Tracker::new("S3".into(), 0, String::new(), start);
    let mut out = Vec::new();
    t.answer(
        Read::ExtList,
        status::OK,
        &ext_list(&[gext::EXT_NOTIFY_FEATURES]),
        start,
        WALL,
        &c,
        &mut out,
    );
    t.step(start, WALL, &c, &mut out);
    assert!(submits(&out).contains(&Read::NotifyFeatures));
    t.answer(
        Read::NotifyFeatures,
        status::TIMEOUT,
        &[],
        start,
        WALL,
        &c,
        &mut out,
    );
    out.clear();
    t.step(start + c.retry_after, WALL, &c, &mut out);
    assert!(submits(&out).contains(&Read::NotifyFeatures));
}

#[test]
fn wp_p3hb_the_clip_events_kept_are_bounded() {
    let c = cfg();
    let start = Instant::now();
    let mut t = running(&[gext::EXT_NOTIFY_FEATURES], NOTIFY_FEAT_CLIP_EVENT, start);
    for _ in 0..(CLIP_EVENTS_MAX + 500) {
        t.clip_event(start, &c);
    }
    assert_eq!(t.clip_events.len(), CLIP_EVENTS_MAX);
    assert_eq!(t.record()["clip_per_min"], CLIP_EVENTS_MAX);
}

#[test]
fn wp_p3hb_only_extension_clip_events_count() {
    let clip = {
        let mut p = vec![2, NOTIFY_EVT_EXT_CLIP, 0, 0, 3, 1, 0, 0];
        p.extend(8u32.to_le_bytes());
        p
    };
    assert!(is_clip_event(&clip));
    let msg = |packet: Vec<u8>| BusMessage::Notification {
        serial: "S1".into(),
        session: 0,
        by: None,
        packet: packet.into(),
    };
    assert!(wanted(&msg(clip.clone())));
    // A parameter change, a short packet, an empty mask: not clip events.
    let param = vec![2, 2, 0, 0, 0, 0, 0, 0];
    assert!(!wanted(&msg(param.clone())) && !is_clip_event(&param));
    assert!(!is_clip_event(&[2, NOTIFY_EVT_EXT_CLIP]));
    let mut empty = vec![2, NOTIFY_EVT_EXT_CLIP, 0, 0, 0, 0, 0, 0];
    empty.extend(0u32.to_le_bytes());
    assert!(!is_clip_event(&empty));
}

// ── Last change ─────────────────────────────────────────────────────────────

fn written(kind: EventKind, t: i64, text: &str, by: Option<&str>) -> EventRecord {
    let mut r = EventRecord::new(kind, t, text);
    r.id = 7;
    r.by = by.map(str::to_owned);
    r
}

#[test]
fn wp_p3hb_last_change_is_the_newest_written_change_and_who_made_it() {
    let start = Instant::now();
    let mut t = running(&[], 0, start);
    assert!(t.record().get("last_change").is_none());
    let ms = i64::try_from(WALL).unwrap() * 1000;
    let a = written(
        EventKind::Notification,
        ms,
        "Master volume -20.0 dB",
        Some("Front of house"),
    );
    t.set_last_change(change_of(&a).unwrap());
    assert_eq!(
        t.record()["last_change"],
        json!({"at": iso8601(WALL), "by": "Front of house", "text": "Master volume -20.0 dB"})
    );
    // An older record does not replace it; a newer one does, with no `by` when none.
    let old = written(
        EventKind::Recall,
        ms - 5000,
        "Preset 2 recalled",
        Some("Old"),
    );
    t.set_last_change(change_of(&old).unwrap());
    assert_eq!(t.record()["last_change"]["by"], "Front of house");
    let knob = written(EventKind::Notification, ms + 3000, "Output 1 muted", None);
    t.set_last_change(change_of(&knob).unwrap());
    let lc = &t.record()["last_change"];
    assert!(
        lc.get("by").is_none() && lc["text"] == "Output 1 muted",
        "{lc}"
    );
    assert_eq!(lc["at"], iso8601(WALL + 3));
}

#[test]
fn wp_p3hb_only_written_changes_are_last_changes() {
    let ms = 1_800_000_000_000;
    // Not written yet (another service's, for the history to write): the bus's
    // filter leaves it out, and it is no change either way.
    let mut unwritten = written(EventKind::Notification, ms, "x", Some("A"));
    unwritten.id = 0;
    assert!(change_of(&unwritten).is_none());
    let ev = |record: EventRecord| BusMessage::Event {
        serial: "S1".into(),
        record,
    };
    assert!(!wanted(&ev(unwritten)));
    for kind in [
        EventKind::Notification,
        EventKind::Recall,
        EventKind::Import,
        EventKind::Schedule,
        EventKind::Firmware,
    ] {
        assert!(
            change_of(&written(kind, ms, "x", None)).is_some(),
            "{kind:?}"
        );
        assert!(wanted(&ev(written(kind, ms, "x", None))));
    }
    for kind in [
        EventKind::Clip,
        EventKind::Temperature,
        EventKind::Counter,
        EventKind::Reset,
        EventKind::Connected,
        EventKind::Disconnected,
        EventKind::Alarm,
        EventKind::Alink,
        EventKind::Access,
        EventKind::Unresponsive,
        EventKind::EventsDropped,
        EventKind::Unknown,
    ] {
        assert!(
            change_of(&written(kind, ms, "x", None)).is_none(),
            "{kind:?}"
        );
        assert!(!wanted(&ev(written(kind, ms, "x", None))), "{kind:?}");
    }
    assert!(wanted(&BusMessage::DeviceForgotten {
        serial: "S1".into()
    }));
    // Long texts and names are cut, not kept whole.
    let long = written(
        EventKind::Import,
        ms,
        &"t".repeat(5000),
        Some(&"b".repeat(5000)),
    );
    let c = change_of(&long).unwrap();
    assert_eq!(c.text.chars().count(), CHANGE_TEXT_MAX);
    assert_eq!(c.by.unwrap().chars().count(), CHANGE_BY_MAX);
    // An empty `by` is none.
    assert!(
        change_of(&written(EventKind::Import, ms, "x", Some("")))
            .unwrap()
            .by
            .is_none()
    );
}

#[test]
fn wp_p3hb_the_last_changes_kept_are_bounded_oldest_out() {
    let mut book = BTreeMap::new();
    let change = |at_ms: i64| LastChange {
        at_ms,
        by: None,
        text: "x".into(),
    };
    for i in 0..LAST_CHANGES_MAX + 10 {
        let at = i64::try_from(i).unwrap();
        remember_change(&mut book, &format!("S{i}"), change(at));
    }
    assert_eq!(book.len(), LAST_CHANGES_MAX);
    assert!(!book.contains_key("S0") && book.contains_key(&format!("S{}", LAST_CHANGES_MAX + 9)));
    // An older change of a known device does not replace its newer one.
    remember_change(&mut book, "S20", change(1));
    assert_eq!(book["S20"].at_ms, 20);
}
