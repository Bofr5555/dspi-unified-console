//! WP P2SC: the schedule's clock rules (OD-P2h) on the pure engine, in two time zones
//! across both DST changes, jumps, the grace and the unset clock; the rules' checks and
//! caps; the file read entry by entry.

use super::*;

/// Central Europe: CEST from the last Sunday of March 02:00, CET from the last Sunday of
/// October 03:00. A POSIX string, so no zone database is needed.
const CET: &str = "CET-1CEST,M3.5.0,M10.5.0/3";
/// US Eastern: EDT from the second Sunday of March 02:00, EST from the first Sunday of
/// November 02:00.
const EASTERN: &str = "EST5EDT,M3.2.0,M11.1.0";

fn tz(s: &str) -> TimeZone {
    TimeZone::posix(s).unwrap()
}

/// Unix seconds of a UTC time.
fn utc(s: &str) -> i64 {
    s.parse::<Timestamp>().unwrap().as_second()
}

fn local(tz: &TimeZone, s: i64) -> String {
    let dt = tz.to_datetime(Timestamp::from_second(s).unwrap());
    format!("{}T{:02}:{:02}", dt.date(), dt.hour(), dt.minute())
}

fn rule(when: Value) -> Value {
    json!({"name": "Doors", "when": when,
           "actions": [{"t": "param.set", "path": "vol.master", "value": -12.0}],
           "targets": ["E46058388B1A2E2C"]})
}

fn daily(time: &str) -> Value {
    rule(json!({"days": ["mon", "tue", "wed", "thu", "fri", "sat", "sun"], "time": time}))
}

fn engine_with(rules: &[Value]) -> Engine {
    let mut e = Engine::default();
    for r in rules {
        e.set(None, r).unwrap();
    }
    e
}

/// The hub's timer: a look every `step` seconds from `from` to `to`, wall and monotonic
/// time moving together. Every run, as (rule id, when it ran).
struct Sim {
    engine: Engine,
    tz: TimeZone,
    base: Instant,
    base_wall: i64,
    wall: i64,
    mono: i64,
    runs: Vec<(String, i64)>,
    skips: Vec<(String, i64, SkipWhy)>,
    unset: Vec<bool>,
}

impl Sim {
    fn new(engine: Engine, zone: &str, start: i64) -> Self {
        Self {
            engine,
            tz: tz(zone),
            base: Instant::now(),
            base_wall: start,
            wall: start,
            mono: 0,
            runs: Vec::new(),
            skips: Vec::new(),
            unset: Vec::new(),
        }
    }

    fn look(&mut self) {
        let mono = self.base + Duration::from_secs(self.mono as u64);
        let t = self.engine.tick(self.wall, mono, &self.tz, 0);
        self.collect(t);
    }

    fn look_floor(&mut self, floor: i64) {
        let mono = self.base + Duration::from_secs(self.mono as u64);
        let t = self.engine.tick(self.wall, mono, &self.tz, floor);
        self.collect(t);
    }

    fn collect(&mut self, t: Tick) {
        if let Some(u) = t.clock_unset {
            self.unset.push(u);
        }
        for d in t.due {
            match d {
                Due::Run { rule, .. } => self.runs.push((rule.id, self.wall)),
                Due::Skip {
                    rule, slot, why, ..
                } => self.skips.push((rule.id, slot, why)),
            }
        }
    }

    /// Both clocks on by `by` seconds, a look every `step`.
    fn run_for(&mut self, by: i64, step: i64) {
        let end = self.wall + by;
        while self.wall < end {
            self.wall += step;
            self.mono += step;
            self.look();
        }
    }

    /// The wall clock alone moves to `to`; a little monotonic time passes.
    fn jump_to(&mut self, to: i64) {
        self.wall = to;
        self.mono += 30;
        self.look();
    }

    fn local_runs(&self) -> Vec<String> {
        self.runs.iter().map(|(_, s)| local(&self.tz, *s)).collect()
    }
}

#[test]
fn wp_p2sc_a_daily_rule_runs_once_a_day_at_its_local_time_in_two_zones() {
    for (zone, start, want_utc) in [
        (CET, "2027-01-11T00:00:00Z", "2027-01-11T17:30:00Z"),
        (EASTERN, "2027-01-11T00:00:00Z", "2027-01-11T23:30:00Z"),
    ] {
        let mut sim = Sim::new(engine_with(&[daily("18:30")]), zone, utc(start));
        sim.look();
        sim.run_for(3 * 86_400, 60);
        assert_eq!(sim.runs.len(), 3, "{zone}: {:?}", sim.local_runs());
        assert_eq!(sim.runs[0].1, utc(want_utc), "{zone}");
        assert!(sim.local_runs().iter().all(|l| l.ends_with("T18:30")));
        assert!(sim.skips.is_empty());
    }
}

#[test]
fn wp_p2sc_spring_forward_runs_a_missing_time_at_the_first_valid_minute() {
    // CET 2027-03-28: 02:00 CET becomes 03:00 CEST; 02:30 does not exist that day.
    let mut sim = Sim::new(
        engine_with(&[daily("02:30")]),
        CET,
        utc("2027-03-26T12:00:00Z"),
    );
    sim.look();
    sim.run_for(3 * 86_400, 30);
    assert_eq!(
        sim.local_runs(),
        ["2027-03-27T02:30", "2027-03-28T03:00", "2027-03-29T02:30"],
        "the gap's run is at 03:00, the first minute that exists"
    );
    assert_eq!(sim.runs[1].1, utc("2027-03-28T01:00:00Z"));
    // US Eastern 2027-03-14: 02:00 EST becomes 03:00 EDT.
    let mut sim = Sim::new(
        engine_with(&[daily("02:15")]),
        EASTERN,
        utc("2027-03-13T12:00:00Z"),
    );
    sim.look();
    sim.run_for(2 * 86_400, 30);
    assert_eq!(sim.local_runs(), ["2027-03-14T03:00", "2027-03-15T02:15"]);
    assert_eq!(sim.runs[0].1, utc("2027-03-14T07:00:00Z"));
}

#[test]
fn wp_p2sc_fall_back_runs_a_repeated_time_once_at_the_first() {
    // CET 2027-10-31: 03:00 CEST becomes 02:00 CET; 02:30 happens twice.
    let mut sim = Sim::new(
        engine_with(&[daily("02:30")]),
        CET,
        utc("2027-10-30T12:00:00Z"),
    );
    sim.look();
    sim.run_for(2 * 86_400, 30);
    assert_eq!(sim.runs.len(), 2, "{:?}", sim.local_runs());
    assert_eq!(
        sim.runs[0].1,
        utc("2027-10-31T00:30:00Z"),
        "the first 02:30 (CEST)"
    );
    assert_eq!(sim.runs[1].1, utc("2027-11-01T01:30:00Z"));
    // US Eastern 2027-11-07: 02:00 EDT becomes 01:00 EST; 01:30 happens twice.
    let mut sim = Sim::new(
        engine_with(&[daily("01:30")]),
        EASTERN,
        utc("2027-11-06T12:00:00Z"),
    );
    sim.look();
    sim.run_for(2 * 86_400, 30);
    assert_eq!(sim.runs.len(), 2, "{:?}", sim.local_runs());
    assert_eq!(
        sim.runs[0].1,
        utc("2027-11-07T05:30:00Z"),
        "the first 01:30 (EDT)"
    );
    assert!(sim.skips.is_empty());
}

#[test]
fn wp_p2sc_a_forward_jump_skips_and_records_what_it_passed() {
    let mut sim = Sim::new(
        engine_with(&[daily("18:30"), daily("21:00")]),
        CET,
        utc("2027-01-11T16:00:00Z"), // 17:00 local
    );
    sim.look();
    sim.run_for(600, 30);
    // The clock is set two hours on (an SBC whose network time arrived).
    sim.jump_to(utc("2027-01-11T18:10:00Z")); // 19:10 local
    assert!(
        sim.runs.is_empty(),
        "nothing it passed runs: {:?}",
        sim.runs
    );
    assert_eq!(sim.skips.len(), 1);
    assert_eq!(sim.skips[0].0, "r1");
    assert_eq!(sim.skips[0].1, utc("2027-01-11T17:30:00Z"));
    assert_eq!(sim.skips[0].2, SkipWhy::Jump);
    let last = sim.engine.rules()[0].1.last.unwrap();
    assert!(last.skipped && !last.ok, "recorded: {last:?}");
    // What lies ahead still runs, once.
    sim.run_for(2 * 3600, 30);
    assert_eq!(sim.runs.len(), 1);
    assert_eq!(sim.runs[0].0, "r2");
    assert_eq!(local(&sim.tz, sim.runs[0].1), "2027-01-11T21:00");
}

#[test]
fn wp_p2sc_a_backward_jump_never_runs_a_rule_twice_in_a_day() {
    let mut sim = Sim::new(
        engine_with(&[daily("18:30")]),
        CET,
        utc("2027-01-11T17:00:00Z"), // 18:00 local
    );
    sim.look();
    sim.run_for(3600, 30);
    assert_eq!(sim.runs.len(), 1);
    // Back by an hour: 18:30 comes round again the same day.
    sim.jump_to(utc("2027-01-11T17:05:00Z"));
    sim.run_for(3600, 30);
    assert_eq!(
        sim.runs.len(),
        1,
        "not twice in a day: {:?}",
        sim.local_runs()
    );
    assert!(sim.skips.is_empty());
    // Back by two days: those days ran already.
    sim.jump_to(utc("2027-01-09T17:00:00Z"));
    sim.run_for(3 * 86_400 + 3600, 60);
    assert_eq!(
        sim.local_runs(),
        ["2027-01-11T18:30", "2027-01-12T18:30"],
        "the next run is the next day's, after the day that ran"
    );
}

#[test]
fn wp_p2sc_the_grace_runs_what_downtime_missed_by_up_to_five_minutes_only() {
    // The hub starts 4 minutes after a time: it runs.
    let mut sim = Sim::new(
        engine_with(&[daily("18:30")]),
        CET,
        utc("2027-01-11T17:34:00Z"),
    );
    sim.look();
    assert_eq!(sim.runs.len(), 1, "inside the grace");
    // 6 minutes after: it does not, and it is not caught up later.
    let mut sim = Sim::new(
        engine_with(&[daily("18:30")]),
        CET,
        utc("2027-01-11T17:36:00Z"),
    );
    sim.look();
    sim.run_for(3600, 30);
    assert!(sim.runs.is_empty() && sim.skips.is_empty());
    // A timer that stalled for 10 minutes across a time: missed, recorded, not run.
    let mut sim = Sim::new(
        engine_with(&[daily("18:30")]),
        CET,
        utc("2027-01-11T17:25:00Z"),
    );
    sim.look();
    sim.run_for(900, 900);
    assert!(sim.runs.is_empty());
    assert_eq!(sim.skips.len(), 1);
    assert_eq!(sim.skips[0].2, SkipWhy::Missed);
    // A rule saved with a last date of today is not run again by a restart.
    let mut e = engine_with(&[daily("18:30")]);
    e.rules[0].1.last_date = Some("2027-01-11".parse().unwrap());
    let mut sim = Sim::new(e, CET, utc("2027-01-11T17:31:00Z"));
    sim.look();
    assert!(sim.runs.is_empty(), "it ran before the restart");
    let _ = sim.base_wall;
}

#[test]
fn wp_p2sc_nothing_runs_while_the_clock_reads_before_the_build_date() {
    let floor = utc("2026-11-15T00:00:00Z");
    let mut sim = Sim::new(
        engine_with(&[rule(json!({"days": ["thu"], "time": "00:01"}))]),
        CET,
        utc("1970-01-01T00:00:30Z"), // a Thursday 00:00 local, no battery clock
    );
    for _ in 0..10 {
        sim.wall += 30;
        sim.mono += 30;
        sim.look_floor(floor);
    }
    assert!(sim.runs.is_empty() && sim.skips.is_empty());
    assert_eq!(sim.unset, [true], "clock_unset raised once");
    assert!(sim.engine.clock_unset());
    // The network sets the clock: a Thursday 00:03 local; the grace window applies,
    // as after a start.
    sim.wall = utc("2027-01-13T23:03:00Z");
    sim.mono += 30;
    sim.look_floor(floor);
    assert_eq!(sim.unset, [true, false], "cleared once set");
    assert_eq!(sim.runs.len(), 1, "00:01 was 2 minutes ago");
    assert_eq!(
        floor, SOURCE_FLOOR_S,
        "the floor is the day this was written"
    );
    assert!(build_floor_s() >= SOURCE_FLOOR_S);
}

#[test]
fn wp_p2sc_a_once_rule_runs_once_and_never_again() {
    let mut sim = Sim::new(
        engine_with(&[rule(json!({"once": "2027-12-24T17:00"}))]),
        CET,
        utc("2027-12-24T15:00:00Z"),
    );
    sim.look();
    sim.run_for(3 * 3600, 30);
    assert_eq!(sim.local_runs(), ["2027-12-24T17:00"]);
    sim.jump_to(utc("2027-12-24T15:00:00Z"));
    sim.run_for(3 * 3600, 30);
    assert_eq!(sim.runs.len(), 1, "a backward jump does not run it again");
    assert_eq!(sim.engine.next_run("r1", &sim.tz, sim.wall), None);
}

#[test]
fn wp_p2sc_next_run_and_a_changed_time_runs_today() {
    let tz = tz(CET);
    let mut e = engine_with(&[daily("18:30")]);
    let now = utc("2027-01-11T17:45:00Z"); // 18:45 local, after today's
    assert_eq!(
        e.next_run("r1", &tz, utc("2027-01-11T12:00:00Z")),
        Some(utc("2027-01-11T17:30:00Z"))
    );
    e.rules[0].1.last_date = Some("2027-01-11".parse().unwrap());
    assert_eq!(
        e.next_run("r1", &tz, utc("2027-01-11T12:00:00Z")),
        Some(utc("2027-01-12T17:30:00Z")),
        "it ran today"
    );
    // Moved to 20:00 after today's run: its new time is still ahead today.
    e.set(Some("r1"), &daily("20:00")).unwrap();
    assert_eq!(
        e.next_run("r1", &tz, now),
        Some(utc("2027-01-11T19:00:00Z"))
    );
    // Disabled: no next run.
    let mut off = daily("20:00");
    off["enabled"] = json!(false);
    e.set(Some("r1"), &off).unwrap();
    assert_eq!(e.next_run("r1", &tz, now), None);
}

#[test]
fn wp_p2sc_rules_are_checked_and_capped() {
    let mut e = Engine::default();
    let bad = |e: &mut Engine, v: Value| e.set(None, &v).unwrap_err().code;
    // 16 actions fit, 17 do not.
    let mut r = daily("08:00");
    r["actions"] = json!(vec![
        json!({"t": "param.set", "path": "vol.mute", "value": true});
        16
    ]);
    e.set(None, &r).unwrap();
    r["actions"] = json!(vec![
        json!({"t": "param.set", "path": "vol.mute", "value": true});
        17
    ]);
    assert_eq!(bad(&mut e, r), "bad_request");
    // Only the four paths.
    let mut r = daily("08:00");
    r["actions"] = json!([{"t": "param.set", "path": "out.gain", "idx": [0], "value": 0.0}]);
    assert_eq!(bad(&mut e, r), "bad_request");
    // Only the four actions.
    let mut r = daily("08:00");
    r["actions"] = json!([{"t": "preset.save", "slot": 1, "confirm": true}]);
    assert_eq!(bad(&mut e, r), "bad_request");
    // -- WP P3GR -- a well-formed group is taken (resolved when the rule runs); a
    // malformed one is refused.
    let mut r = daily("08:00");
    r["targets"] = json!(["group:hall"]);
    e.set(None, &r).unwrap();
    r["targets"] = json!(["group:a b"]);
    assert_eq!(bad(&mut e, r), "bad_request");
    // Device actions need targets; a scene does not.
    let mut r = daily("08:00");
    r["targets"] = json!([]);
    assert_eq!(bad(&mut e, r.clone()), "bad_request");
    r["actions"] = json!([{"t": "system.scene.apply", "scene": "show"}]);
    e.set(None, &r).unwrap();
    // 17 targets do not fit; a serial twice is refused.
    let mut r = daily("08:00");
    r["targets"] = json!((0..17).map(|i| format!("S{i}")).collect::<Vec<_>>());
    assert_eq!(bad(&mut e, r), "bad_request");
    let mut r = daily("08:00");
    r["targets"] = json!(["A1", "A1"]);
    assert_eq!(bad(&mut e, r), "bad_request");
    // Times and days.
    for when in [
        json!({"days": [], "time": "08:00"}),
        json!({"days": ["mon"], "time": "24:00"}),
        json!({"days": ["mon"], "time": "8:0"}),
        json!({"days": ["moon"], "time": "08:00"}),
        json!({"once": "2027-02-30T08:00"}),
        json!({"once": "2027-02-03"}),
    ] {
        assert_eq!(bad(&mut e, rule(when.clone())), "bad_request", "{when}");
    }
    let mut r = daily("08:00");
    r["name"] = json!("x".repeat(MAX_NAME_CHARS + 1));
    assert_eq!(bad(&mut e, r), "bad_request");
    // 256 rules, then `conflict`.
    while e.rules().len() < MAX_RULES {
        e.set(None, &daily("09:00")).unwrap();
    }
    assert_eq!(bad(&mut e, daily("09:00")), "conflict");
    assert_eq!(
        e.set(Some("r999"), &daily("09:00")).unwrap_err().code,
        "not_found"
    );
    assert_eq!(e.delete("r999").unwrap_err().code, "not_found");
    assert_eq!(e.delete("r1").unwrap().id, "r1");
    let new = e.set(None, &daily("09:00")).unwrap();
    assert_ne!(new.id, "r1", "ids are not reused");
}

#[test]
fn wp_p2sc_the_file_round_trips_and_a_bad_entry_is_left_out() {
    let mut e = engine_with(&[daily("18:30"), rule(json!({"once": "2027-12-24T17:00"}))]);
    e.rules[0].1.last_date = Some("2027-01-11".parse().unwrap());
    e.ran(
        "r1",
        Last {
            at: utc("2027-01-11T17:30:00Z"),
            ok: false,
            skipped: false,
        },
    );
    let text = to_file(&e);
    let (back, whole) = from_file(&text);
    assert!(whole);
    assert_eq!(back.rules(), e.rules());
    assert_eq!(back.next_id(), 3);
    // One entry torn: the other is kept, and the file is not whole.
    let torn = text.replacen("\"name\":\"Doors\"", "\"name\":7", 1);
    let (back, whole) = from_file(&torn);
    assert!(!whole);
    assert_eq!(back.rules().len(), 1);
    assert_eq!(back.rules()[0].0.id, "r2");
    assert_eq!(back.next_id(), 3, "ids go on past the lost one");
    // Not JSON at all: no rules, no panic.
    for junk in ["", "{", "[]", "{\"rules\":7}", "\u{0}\u{1}"] {
        let (back, whole) = from_file(junk);
        assert!(!whole && back.rules().is_empty(), "{junk:?}");
    }
}

#[test]
fn wp_p2sc_a_corrupt_file_is_kept_aside_and_the_hub_runs_with_what_read() {
    let dir = std::env::temp_dir()
        .join("wp-p2sc-unit")
        .join(format!("file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(FILE_NAME);
    assert!(load(&path, 1).0.rules().is_empty(), "none: empty");
    let e = engine_with(&[daily("18:30"), daily("19:30")]);
    write_atomic(&path, &to_file(&e)).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let (back, rewrite) = load(&path, 1);
    assert_eq!(back.rules().len(), 2);
    assert!(!rewrite);
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        text.replacen("\"time\":\"19:30\"", "\"time\":\"99:99\"", 1),
    )
    .unwrap();
    let (back, rewrite) = load(&path, 42);
    assert_eq!(back.rules().len(), 1);
    assert!(rewrite);
    assert!(dir.join("schedule.json.bad-42").exists(), "kept aside");
}

#[test]
fn wp_p2sc_resolve_handles_gaps_and_folds() {
    let cet = tz(CET);
    let dt = |s: &str| s.parse::<DateTime>().unwrap();
    assert_eq!(
        resolve(&cet, dt("2027-03-28T02:30")),
        Some(utc("2027-03-28T01:00:00Z"))
    );
    assert_eq!(
        resolve(&cet, dt("2027-10-31T02:30")),
        Some(utc("2027-10-31T00:30:00Z"))
    );
    assert_eq!(
        resolve(&cet, dt("2027-07-01T12:00")),
        Some(utc("2027-07-01T10:00:00Z"))
    );
    assert_eq!(
        resolve(&TimeZone::UTC, dt("2027-07-01T12:00")),
        Some(utc("2027-07-01T12:00:00Z"))
    );
}
