//! WP P2HP: the health tracker alone, in virtual time.

use dspi_proto::ext::{IDENTIFY_TONE_NONE, ResetReason};

use super::*;

const T: Duration = Duration::from_millis(1);
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

fn published(out: &[Effect]) -> Vec<BusMessage> {
    out.iter()
        .filter_map(|e| match e {
            Effect::Publish(m) => Some((**m).clone()),
            _ => None,
        })
        .collect()
}

fn ext_list(ids: &[u16]) -> Vec<u8> {
    ids.iter().flat_map(|id| id.to_le_bytes()).collect()
}

fn identify_bytes() -> Vec<u8> {
    let mut b = vec![0, IDENTIFY_TONE_NONE, 1, 2];
    b.extend(0x2000_4927u32.to_le_bytes());
    let mut board = b"pico2".to_vec();
    board.resize(16, 0);
    let mut rev = b"K340 rev B".to_vec();
    rev.resize(32, 0);
    b.extend(board);
    b.extend(rev);
    b
}

fn reset_bytes(reason: u8, uptime: u32, prev: Option<u32>, hangs: u8) -> Vec<u8> {
    let mut b = vec![reason, u8::from(prev.is_some()), hangs, 1];
    b.extend(uptime.to_le_bytes());
    b.extend(prev.unwrap_or(0).to_le_bytes());
    b.extend([0u8; 8]);
    b
}

fn meters(cpu0: u8, cpu1: u8, clip: u32) -> Vec<u8> {
    SystemStatus {
        peaks: vec![0; 17],
        cpu0,
        cpu1,
        clip_flags: clip,
        active_inputs: 2,
    }
    .encode()
}

/// A tracker that has read its list and answered every attach read.
fn running(ids: &[u16], start: Instant) -> Tracker {
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
        &meters(10, 20, 0),
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
    t.step(start, WALL, &c, &mut out);
    if ids.contains(&gext::EXT_IDENTIFY) {
        t.answer(
            Read::Identify,
            status::OK,
            &identify_bytes(),
            start,
            WALL,
            &c,
            &mut out,
        );
    }
    if ids.contains(&gext::EXT_RESET_INFO) {
        t.answer(
            Read::ResetInfo,
            status::OK,
            &reset_bytes(1, 100, None, 0),
            start,
            WALL,
            &c,
            &mut out,
        );
    }
    t.step(start, WALL, &c, &mut out);
    assert!(t.facts_sent);
    t
}

#[test]
fn wp_p2hp_attach_reads_the_list_then_what_it_lists_then_the_facts() {
    let c = cfg();
    let now = Instant::now();
    let mut t = Tracker::new("S1".into(), 3, "1.1.7".into(), now);
    let mut out = Vec::new();
    t.step(now, WALL, &c, &mut out);
    assert_eq!(
        submits(&out),
        vec![Read::ExtList, Read::Probe, Read::Temperature],
        "the list first, the meter block's length and a temperature beside it"
    );
    assert!(published(&out).is_empty());
    out.clear();
    let ids = [
        gext::EXT_CORE_VREG,
        gext::EXT_IDENTIFY,
        gext::EXT_RESET_INFO,
    ];
    t.answer(
        Read::ExtList,
        status::OK,
        &ext_list(&ids),
        now,
        WALL,
        &c,
        &mut out,
    );
    t.step(now + T, WALL, &c, &mut out);
    assert_eq!(submits(&out), vec![Read::Identify, Read::ResetInfo]);
    assert!(published(&out).is_empty(), "no facts before both answered");
    out.clear();
    t.answer(
        Read::Identify,
        status::OK,
        &identify_bytes(),
        now,
        WALL,
        &c,
        &mut out,
    );
    t.step(now + T, WALL, &c, &mut out);
    assert!(published(&out).is_empty());
    t.answer(
        Read::ResetInfo,
        status::OK,
        &reset_bytes(4, 30, Some(5000), 1),
        now,
        WALL,
        &c,
        &mut out,
    );
    t.step(now + T, WALL, &c, &mut out);
    let facts = published(&out);
    let [
        BusMessage::Facts {
            serial,
            identify: Some(id),
            reset_info: Some(reset),
            fw,
        },
    ] = facts.as_slice()
    else {
        panic!("one Facts with both: {facts:?}");
    };
    assert_eq!((serial.as_str(), fw.as_str()), ("S1", "1.1.7"));
    assert_eq!(
        (id.board.as_str(), id.hw_rev.as_str()),
        ("pico2", "K340 rev B")
    );
    assert_eq!(reset.reason, ResetReason::Watchdog);
    assert_eq!(reset.prev_uptime_s, Some(5000));
    let r = t.record();
    assert_eq!(
        r["reset"],
        json!({"reason": "watchdog", "prev_uptime_s": 5000})
    );
    assert_eq!(r["uptime_s"], 30);
    assert_eq!(r["hang_count"], 1);
    out.clear();
    t.step(now + T, WALL, &c, &mut out);
    assert!(published(&out).is_empty(), "the facts go once");
}

#[test]
fn wp_p2hp_firmware_without_the_ids_gives_bare_facts_and_is_asked_nothing_more() {
    let c = cfg();
    for (status_, payload) in [
        // Stock firmware: EXT_LIST stalls.
        (status::ERROR, Vec::new()),
        // The extension channel without identify and reset info.
        (status::OK, ext_list(&[gext::EXT_CORE_VREG])),
    ] {
        let now = Instant::now();
        let mut t = Tracker::new("S2".into(), 0, "1.1.6".into(), now);
        let mut out = Vec::new();
        t.step(now, WALL, &c, &mut out);
        out.clear();
        t.answer(Read::ExtList, status_, &payload, now, WALL, &c, &mut out);
        t.step(now + T, WALL, &c, &mut out);
        assert_eq!(
            published(&out),
            vec![BusMessage::Facts {
                serial: "S2".into(),
                identify: None,
                reset_info: None,
                fw: "1.1.6".into()
            }]
        );
        assert!(
            !submits(&out).contains(&Read::Identify) && !submits(&out).contains(&Read::ResetInfo)
        );
        // Never a reset read later either.
        out.clear();
        t.step(now + Duration::from_secs(3600), WALL, &c, &mut out);
        assert!(!submits(&out).contains(&Read::ResetInfo));
        assert!(t.record().get("reset").is_none());
    }
}

#[test]
fn wp_p2hp_a_list_read_that_times_out_is_tried_again_later() {
    let c = cfg();
    let now = Instant::now();
    let mut t = Tracker::new("S3".into(), 0, String::new(), now);
    let mut out = Vec::new();
    t.step(now, WALL, &c, &mut out);
    t.answer(Read::ExtList, status::TIMEOUT, &[], now, WALL, &c, &mut out);
    out.clear();
    t.step(now + c.retry_after - T, WALL, &c, &mut out);
    assert!(
        !submits(&out).contains(&Read::ExtList),
        "not before the retry"
    );
    assert!(
        t.due() <= now + c.retry_after + MIN_WAIT,
        "it wakes for the retry"
    );
    t.step(now + c.retry_after, WALL, &c, &mut out);
    assert!(submits(&out).contains(&Read::ExtList));
}

#[test]
fn wp_p2hp_the_probe_sets_the_polls_length_and_rate() {
    let c = cfg();
    assert_eq!(c.poll_hz(), 1, "every 2 s: the least a poll runs at");
    let fast = HealthConfig {
        status_every: Duration::from_millis(200),
        ..c
    };
    assert_eq!(fast.poll_hz(), 5);
    let fastest = HealthConfig {
        status_every: Duration::ZERO,
        ..c
    };
    assert_eq!(fastest.poll_hz(), POLL_MAX_HZ);

    let now = Instant::now();
    let mut t = Tracker::new("S4".into(), 0, String::new(), now);
    let mut out = Vec::new();
    t.answer(
        Read::Probe,
        status::OK,
        &meters(1, 2, 0),
        now,
        WALL,
        &c,
        &mut out,
    );
    assert_eq!(out, vec![Effect::Subscribe { len: 41, hz: 1 }]);
    // A reply that cannot be a meter block: no poll, no panic.
    let mut odd = Tracker::new("S5".into(), 0, String::new(), now);
    let mut out = Vec::new();
    odd.answer(Read::Probe, status::OK, &[1, 2, 3], now, WALL, &c, &mut out);
    odd.answer(Read::Probe, status::OK, &[0; 8], now, WALL, &c, &mut out);
    assert!(out.is_empty());
    assert_eq!(odd.poll, PollState::Unavailable);
    assert_eq!(meter_block_len(21), Some(21));
    assert_eq!(meter_block_len(6), None);
}

#[test]
fn wp_p2hp_each_period_gives_one_health_from_the_newest_reading() {
    let c = cfg();
    let start = Instant::now();
    let mut t = running(&[gext::EXT_RESET_INFO], start);
    let mut out = Vec::new();
    // The probe's own reading, at the first period.
    t.step(start + c.status_every, WALL, &c, &mut out);
    assert!(
        matches!(
            published(&out).as_slice(),
            [BusMessage::Health { cpu: Some(20), .. }]
        ),
        "{out:?}"
    );
    out.clear();
    t.step(start + c.status_every + T, WALL, &c, &mut out);
    assert!(published(&out).is_empty(), "nothing new since");
    t.status_reading(&meters(40, 91, 0b101), start + c.status_every, WALL + 2);
    t.status_reading(&meters(12, 30, 0b101), start + c.status_every, WALL + 2);
    t.step(start + c.status_every * 2, WALL + 4, &c, &mut out);
    assert_eq!(
        published(&out),
        vec![BusMessage::Health {
            serial: "S1".into(),
            cpu: Some(30),
            clip_mask: 0b101,
            temp_c: Some(35.0),
            uptime_s: Some(100),
            hang_count: Some(0),
        }]
    );
    let r = t.record();
    assert_eq!(r["cpu_pct"], 30);
    assert_eq!(r["clip_mask"], 5);
    assert_eq!(r["temp_c"], 35.0);
    assert_eq!(r["unresponsive"], false);
    assert_eq!(r["at"], iso8601(WALL + 2));
}

#[test]
fn wp_p2hp_temperature_and_reset_info_come_at_their_own_periods() {
    let c = cfg();
    let start = Instant::now();
    let mut t = running(&[gext::EXT_RESET_INFO], start);
    let mut reads = Vec::new();
    let mut now = start;
    // Ten minutes, each read answered at once.
    while now < start + Duration::from_secs(600) {
        now = t.due().max(now);
        let mut out = Vec::new();
        // The poll keeps answering.
        t.status_reading(&meters(1, 1, 0), now, WALL);
        t.step(now, WALL, &c, &mut out);
        for r in submits(&out) {
            reads.push(r);
            let payload = match r {
                Read::Temperature => 3600u32.to_le_bytes().to_vec(),
                Read::ResetInfo => reset_bytes(1, 200, None, 0),
                _ => Vec::new(),
            };
            t.answer(r, status::OK, &payload, now, WALL, &c, &mut out);
        }
    }
    let count = |r: Read| reads.iter().filter(|x| **x == r).count();
    assert!((19..=21).contains(&count(Read::Temperature)), "{reads:?}");
    assert!((9..=11).contains(&count(Read::ResetInfo)), "{reads:?}");
    assert_eq!(count(Read::Status), 0, "the poll gives the status");
    assert_eq!(t.temp_c, Some(36.0));
}

#[test]
fn wp_p2hp_an_uptime_that_went_backwards_is_a_reset_and_new_facts() {
    let c = cfg();
    let start = Instant::now();
    let mut t = running(&[gext::EXT_RESET_INFO], start);
    let mut out = Vec::new();
    t.answer(
        Read::ResetInfo,
        status::OK,
        &reset_bytes(1, 160, None, 0),
        start,
        WALL,
        &c,
        &mut out,
    );
    assert!(published(&out).is_empty(), "uptime went on: nothing to say");
    t.answer(
        Read::ResetInfo,
        status::OK,
        &reset_bytes(4, 3, Some(160), 1),
        start,
        WALL,
        &c,
        &mut out,
    );
    let facts = published(&out);
    let [
        BusMessage::Facts {
            reset_info: Some(r),
            ..
        },
    ] = facts.as_slice()
    else {
        panic!("{facts:?}");
    };
    assert_eq!(
        (r.reason, r.uptime_s, r.hang_count),
        (ResetReason::Watchdog, 3, 1)
    );
    assert_eq!(t.record()["reset"]["reason"], "watchdog");
}

#[test]
fn wp_p2hp_a_silent_device_is_unresponsive_once_and_recovers() {
    let c = cfg();
    let start = Instant::now();
    let mut t = running(&[], start);
    let mut out = Vec::new();
    let mut now = start;
    // Twenty seconds without an answer; the reads it makes time out.
    while now < start + Duration::from_secs(20) {
        now = t.due().max(now);
        t.step(
            now,
            WALL + now.duration_since(start).as_secs(),
            &c,
            &mut out,
        );
        for r in submits(&out) {
            t.answer(r, status::TIMEOUT, &[], now, WALL, &c, &mut Vec::new());
        }
        out.retain(|e| matches!(e, Effect::Publish(_)));
    }
    let gone: Vec<_> = published(&out)
        .into_iter()
        .filter(|m| matches!(m, BusMessage::Unresponsive { .. }))
        .collect();
    assert_eq!(
        gone,
        vec![BusMessage::Unresponsive {
            serial: "S1".into(),
            since: WALL
        }],
        "once, since its last answer"
    );
    assert_eq!(t.record()["unresponsive"], true);
    // While it is silent the single reads wait.
    let mut quiet = Vec::new();
    t.step(now + Duration::from_secs(60), WALL, &c, &mut quiet);
    assert!(submits(&quiet).is_empty(), "{quiet:?}");
    // The poll's next frame ends it.
    let back = now + Duration::from_secs(60);
    t.status_reading(&meters(5, 5, 0), back, WALL + 81);
    assert_eq!(t.record()["unresponsive"], false);
    assert!(t.record().get("unresponsive_since").is_none());
    let mut after = Vec::new();
    t.step(back + c.status_every, WALL, &c, &mut after);
    assert!(
        published(&after)
            .iter()
            .any(|m| matches!(m, BusMessage::Health { .. }))
    );
    assert!(
        !published(&after)
            .iter()
            .any(|m| matches!(m, BusMessage::Unresponsive { .. }))
    );
}

#[test]
fn wp_p2hp_a_refused_poll_reads_the_status_itself_and_asks_again() {
    let c = cfg();
    let now = Instant::now();
    let mut t = Tracker::new("S6".into(), 0, String::new(), now);
    let mut out = Vec::new();
    t.answer(
        Read::Probe,
        status::OK,
        &meters(1, 2, 0),
        now,
        WALL,
        &c,
        &mut out,
    );
    t.poll_answer(false, now, &c);
    out.clear();
    t.step(now + c.status_every, WALL, &c, &mut out);
    assert!(submits(&out).contains(&Read::Status));
    out.clear();
    t.step(now + c.retry_after, WALL, &c, &mut out);
    assert!(
        out.contains(&Effect::Subscribe { len: 41, hz: 1 }),
        "{out:?}"
    );
}

#[test]
fn wp_p2hp_odd_payloads_are_ignored_never_a_panic() {
    let c = cfg();
    let now = Instant::now();
    let mut t = running(&[gext::EXT_IDENTIFY, gext::EXT_RESET_INFO], now);
    let mut out = Vec::new();
    for r in Read::ALL {
        for payload in [&[][..], &[1, 2, 3][..], &[0xFF; 200][..]] {
            t.answer(r, status::OK, payload, now, WALL, &c, &mut out);
        }
    }
    t.status_reading(&[9], now, WALL);
    assert_eq!(parse_temperature(&(-1250i32).to_le_bytes()), Some(-12.5));
    assert_eq!(parse_temperature(&[1, 2]), None);
}

#[test]
fn wp_p2hp_times_are_iso_utc() {
    assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
    assert_eq!(iso8601(1_763_199_000), "2025-11-15T09:30:00Z");
}
