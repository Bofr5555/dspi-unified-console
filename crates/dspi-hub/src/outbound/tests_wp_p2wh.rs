//! WP P2WH: the outbound state without threads or sockets, the file, and the roles.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::*;
use crate::bus::TargetResult;
use crate::clock::TestClock;
use std::sync::atomic::AtomicU32;

fn scratch(what: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work, not tmpfs.
    let dir = std::env::temp_dir().join("wp-p2wh-unit").join(format!(
        "{what}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn target(id: &str, kinds: &[&str]) -> (Target, HttpUrl) {
    let url = format!("http://127.0.0.1:9/{id}");
    let t = Target {
        id: id.into(),
        name: id.into(),
        kind: TargetKind::Webhook,
        url: url.clone(),
        enabled: true,
        kinds: kinds.iter().map(|k| (*k).to_owned()).collect(),
        secret: None,
    };
    (t, http::parse_url(&url).unwrap())
}

fn alarm(state: AlarmState, code: &str, n: u64) -> BusMessage {
    BusMessage::Alarm {
        id: format!("device:{n}"),
        source: "device".into(),
        code: code.into(),
        subject: "E466".into(),
        serial: Some("E466".into()),
        severity: "warning".into(),
        state,
        message: format!("message {n}"),
        at: 1_800_000_000 + n,
        raised_at: 1_800_000_000,
        cleared_at: (state == AlarmState::Cleared).then_some(1_800_000_000 + n),
        acknowledged_by: None,
        count: 1,
    }
}

fn notice(n: u64) -> Notice {
    Notice::from_bus(&alarm(AlarmState::Raised, "clip", n)).unwrap()
}

fn ready(outbox: &mut Outbox, epoch: u64, now: Instant) -> Job {
    match outbox.take(epoch, now) {
        Take::Ready(job) => job,
        other => panic!("not ready: {other:?}"),
    }
}

fn epoch_of(outbox: &Outbox, id: &str) -> u64 {
    outbox
        .slots
        .iter()
        .find(|s| s.target.id == id)
        .unwrap()
        .epoch
}

#[test]
fn wp_p2wh_notices_from_the_bus_and_the_body_shape() {
    let n = notice(5);
    assert_eq!(n.kinds, vec!["alarm.raised"]);
    assert_eq!(n.serial.as_deref(), Some("E466"));
    assert_eq!(n.text, "Alarm on E466: message 5");
    let body: Value = serde_json::from_str(&n.body("hub-1", "alarm.raised")).unwrap();
    assert_eq!(body["v"], 1);
    assert_eq!(body["hub"], "hub-1");
    assert_eq!(body["kind"], "alarm.raised");
    assert_eq!(body["serial"], "E466");
    assert_eq!(body["at"], "2027-01-15T08:00:05Z");
    assert_eq!(body["alarm"]["id"], "device:5");
    assert_eq!(body["alarm"]["state"], "raised");
    assert_eq!(body["alarm"]["raised_at"], "2027-01-15T08:00:00Z");
    assert!(body["alarm"].get("cleared_at").is_none());
    assert!(body.get("schedule").is_none());

    let restart = Notice::from_bus(&alarm(AlarmState::Raised, "device_restarted", 1)).unwrap();
    assert_eq!(restart.kinds, vec!["restart", "alarm.raised"]);
    let cleared = Notice::from_bus(&alarm(AlarmState::Cleared, "clip", 2)).unwrap();
    assert_eq!(cleared.kinds, vec!["alarm.cleared"]);
    assert!(cleared.alarm.unwrap()["cleared_at"].is_string());
    assert_eq!(
        Notice::from_bus(&alarm(AlarmState::Acknowledged, "clip", 3)),
        None,
        "an acknowledgement is not sent"
    );
    assert_eq!(
        Notice::from_bus(&BusMessage::DeviceOnline {
            serial: "S".into(),
            handle: 0
        }),
        None
    );

    let row = |ok: bool| TargetResult {
        serial: "E466".into(),
        action: Some("0:preset.load".into()),
        ok,
        code: (!ok).then(|| "offline".to_owned()),
        msg: None,
    };
    let ran = |results: Vec<TargetResult>| BusMessage::ScheduleRan {
        rule: "r1".into(),
        name: "Morning".into(),
        at: 1_800_000_000,
        by: None,
        results,
    };
    let good = Notice::from_bus(&ran(vec![row(true)])).unwrap();
    assert_eq!(good.kinds, vec!["schedule.ran"]);
    let bad = Notice::from_bus(&ran(vec![row(true), row(false)])).unwrap();
    assert_eq!(bad.kinds, vec!["schedule.failed", "schedule.ran"]);
    assert_eq!(
        bad.text,
        "Schedule Morning failed: 1 of 2 actions went through"
    );
    let body: Value = serde_json::from_str(&bad.body("h", "schedule.failed")).unwrap();
    assert_eq!(body["schedule"]["rule"], "r1");
    assert_eq!(body["schedule"]["results"][1]["code"], "offline");
    assert!(body.get("serial").is_none());
    // Bounded: a long list is cut in the body, with its total.
    let many = Notice::from_bus(&ran((0..500).map(|_| row(true)).collect())).unwrap();
    let body: Value = serde_json::from_str(&many.body("h", "schedule.ran")).unwrap();
    assert_eq!(body["schedule"]["results"].as_array().unwrap().len(), 64);
    assert_eq!(body["schedule"]["results_total"], 500);
}

#[test]
fn wp_p2wh_each_target_gets_the_first_kind_it_wants_once() {
    let mut o = Outbox::default();
    let mut off = target("off", &KINDS);
    off.0.enabled = false;
    o.set_targets(vec![
        target("all", &KINDS),
        target("alarms", &["alarm.raised"]),
        target("restarts", &["restart"]),
        target("cleared", &["alarm.cleared"]),
        off,
    ]);
    let restart = Notice::from_bus(&alarm(AlarmState::Raised, "device_restarted", 1)).unwrap();
    assert_eq!(o.enqueue(&restart, "h"), 3);
    assert_eq!(o.queued("all")[0].kind, "restart", "once, as the best kind");
    assert_eq!(o.queued("all").len(), 1);
    assert_eq!(o.queued("alarms")[0].kind, "alarm.raised");
    assert_eq!(o.queued("restarts")[0].kind, "restart");
    assert!(o.queued("cleared").is_empty());
    assert!(o.queued("off").is_empty(), "a disabled target gets nothing");
    // A push target is sent the text.
    let mut push = target("push", &KINDS);
    push.0.kind = TargetKind::Push;
    o.set_targets(vec![push]);
    o.enqueue(&notice(1), "h");
    assert_eq!(&*o.queued("push")[0].body, "Alarm on E466: message 1");
}

#[test]
fn wp_p2wh_a_full_queue_drops_its_oldest_with_a_count() {
    let mut o = Outbox::default();
    o.set_targets(vec![target("a", &KINDS)]);
    for n in 0..70 {
        o.enqueue(&notice(n), "h");
    }
    assert_eq!(o.counts("a"), Some((QUEUE_CAP, 6)));
    let q = o.queued("a");
    assert!(
        q[0].body.contains("message 6"),
        "the oldest went: {}",
        q[0].body
    );
    assert!(q[63].body.contains("message 69"));
    let view = o.view();
    assert_eq!(view[0]["queued"], 64);
    assert_eq!(view[0]["dropped"], 6);
}

#[test]
fn wp_p2wh_a_failing_target_backs_off_doubling_to_five_minutes() {
    let mut o = Outbox::default();
    o.set_targets(vec![target("a", &KINDS)]);
    let e = epoch_of(&o, "a");
    let t0 = Instant::now();
    assert!(matches!(o.take(e, t0), Take::Wait(None)), "nothing queued");
    o.enqueue(&notice(0), "h");
    let mut now = t0;
    let mut waits = Vec::new();
    // One delivery, nine failures: each time back at the front, later.
    for _ in 0..9 {
        let job = ready(&mut o, e, now);
        assert!(matches!(o.take(e, now), Take::Wait(None)), "one in flight");
        assert_eq!(o.counts("a"), Some((1, 0)), "in flight counts as queued");
        o.finish(job, &Outcome::Failed("refused".into()), now, 1);
        let Take::Wait(Some(due)) = o.take(e, now) else {
            panic!("not backed off")
        };
        waits.push((due - now).as_secs());
        // Not a moment early.
        assert!(matches!(
            o.take(e, due - Duration::from_millis(1)),
            Take::Wait(Some(_))
        ));
        now = due;
    }
    assert_eq!(waits, vec![1, 2, 4, 8, 16, 32, 64, 128, 256]);
    // The tenth failure drops it; the backoff stays at its cap.
    let job = ready(&mut o, e, now);
    assert_eq!(job.delivery.attempts, 9);
    o.finish(job, &Outcome::Refused(500), now, 2);
    assert_eq!(
        o.counts("a"),
        Some((0, 1)),
        "dropped after {MAX_ATTEMPTS} tries"
    );
    let last = &o.view()[0]["last"];
    assert_eq!(last["ok"], false);
    assert_eq!(last["status"], 500);
    o.enqueue(&notice(1), "h");
    let Take::Wait(Some(due)) = o.take(e, now) else {
        panic!()
    };
    assert_eq!((due - now).as_secs(), 300, "capped at 5 min");
    // A success resets it.
    let job = ready(&mut o, e, due);
    o.finish(job, &Outcome::Delivered(204), due, 3);
    assert_eq!(o.counts("a"), Some((0, 1)));
    assert_eq!(
        o.view()[0]["last"],
        json!({"at": iso8601(3), "ok": true, "status": 204})
    );
    o.enqueue(&notice(2), "h");
    let job = ready(&mut o, e, due);
    o.finish(job, &Outcome::Failed("x".into()), due, 4);
    let Take::Wait(Some(next)) = o.take(e, due) else {
        panic!()
    };
    assert_eq!((next - due).as_secs(), 1);
}

#[test]
fn wp_p2wh_a_retry_into_a_queue_that_filled_meanwhile_is_dropped() {
    let mut o = Outbox::default();
    o.set_targets(vec![target("a", &KINDS)]);
    let e = epoch_of(&o, "a");
    let now = Instant::now();
    o.enqueue(&notice(0), "h");
    let job = ready(&mut o, e, now);
    for n in 1..=64 {
        o.enqueue(&notice(n), "h");
    }
    o.finish(job, &Outcome::Failed("x".into()), now, 1);
    assert_eq!(o.counts("a"), Some((64, 1)), "the oldest, the retry, went");
    assert!(o.queued("a")[0].body.contains("message 1"));
}

#[test]
fn wp_p2wh_replacing_targets_keeps_a_kept_ids_queue_and_ends_a_removed_one() {
    let mut o = Outbox::default();
    o.set_targets(vec![target("a", &KINDS), target("b", &KINDS)]);
    let (ea, eb) = (epoch_of(&o, "a"), epoch_of(&o, "b"));
    assert_eq!(o.needing_workers(), vec![ea, eb]);
    assert!(o.needing_workers().is_empty());
    o.enqueue(&notice(0), "h");
    let mut a2 = target("a", &KINDS);
    a2.0.name = "renamed".into();
    let mut b_off = target("b", &KINDS);
    b_off.0.enabled = false;
    o.set_targets(vec![a2, b_off, target("c", &KINDS)]);
    assert_eq!(epoch_of(&o, "a"), ea);
    assert_eq!(o.queued("a").len(), 1, "a kept id keeps its queue");
    assert!(o.queued("b").is_empty(), "a disabled one loses its queue");
    assert_eq!(o.needing_workers(), vec![epoch_of(&o, "c")]);
    o.set_targets(vec![target("c", &KINDS)]);
    assert!(matches!(o.take(ea, Instant::now()), Take::Gone));
    // Added again: a new slot, a new worker.
    o.set_targets(vec![target("a", &KINDS)]);
    assert_ne!(epoch_of(&o, "a"), ea);
    assert_eq!(o.needing_workers().len(), 1);
}

#[test]
fn wp_p2wh_next_due_is_a_backed_off_target_and_never_one_already_woken_for() {
    let mut o = Outbox::default();
    o.set_targets(vec![target("a", &KINDS)]);
    let e = epoch_of(&o, "a");
    let now = Instant::now();
    o.enqueue(&notice(0), "h");
    let job = ready(&mut o, e, now);
    o.finish(job, &Outcome::Failed("x".into()), now, 1);
    assert_eq!(o.next_due(None), None, "no worker, no timer");
    o.needing_workers();
    let due = now + BACKOFF_FIRST;
    assert_eq!(o.next_due(None), Some(due));
    assert_eq!(o.next_due(Some(now)), Some(due));
    assert_eq!(o.next_due(Some(due)), None, "woken for it already: no spin");
}

fn entry(v: Value) -> Vec<Value> {
    vec![v]
}

#[test]
fn wp_p2wh_set_checks_every_field_and_makes_secrets() {
    let hook = json!({"name": "HA", "kind": "webhook", "url": "http://10.0.0.2:8123/hook", "secret": "new"});
    let (targets, secrets) = plan(&entry(hook.clone()), &[]).unwrap();
    let t = &targets[0].0;
    assert_eq!(t.id, "t1", "an id is given when none is");
    assert_eq!(t.name, "HA");
    assert!(t.enabled);
    assert_eq!(t.kinds, KINDS.to_vec(), "every kind by default");
    let secret = t.secret.clone().unwrap();
    assert!(valid_secret(&secret));
    assert_eq!(secrets["t1"], secret.as_str(), "shown in this answer");
    assert!(!format!("{t:?}").contains(&secret), "never in a log line");

    // Kept when the list is sent again with "set" or no secret; replaced by "new";
    // cleared by "not set".
    let current = vec![t.clone()];
    for keep in [json!("set"), Value::Null] {
        let mut e = hook.clone();
        e["id"] = "t1".into();
        if keep.is_null() {
            e.as_object_mut().unwrap().remove("secret");
        } else {
            e["secret"] = keep;
        }
        let (again, secrets) = plan(&entry(e), &current).unwrap();
        assert_eq!(again[0].0.secret.as_ref(), Some(&secret));
        assert!(secrets.is_empty(), "a kept secret is not shown again");
    }
    let mut e = hook.clone();
    e["id"] = "t1".into();
    let (again, secrets) = plan(&entry(e.clone()), &current).unwrap();
    assert_ne!(again[0].0.secret.as_ref(), Some(&secret));
    assert_eq!(secrets.len(), 1);
    e["secret"] = "not set".into();
    assert_eq!(plan(&entry(e), &current).unwrap().0[0].0.secret, None);

    let refused = |v: Value| plan(&entry(v), &[]).err().unwrap();
    let (i, why) = refused(json!({"kind": "webhook", "url": "https://ntfy.sh/x"}));
    assert_eq!(i, Some(0));
    assert!(
        why.contains("https:// is not supported") && why.contains("OD-P2f"),
        "{why}"
    );
    let (_, why) = refused(json!({"kind": "webhook", "url": "http://h/", "secret": "0123abcd"}));
    assert!(why.contains("made by the hub"), "a typed secret: {why}");
    for bad in [
        json!({"kind": "email", "url": "http://h/"}),
        json!({"kind": "webhook"}),
        json!({"kind": "webhook", "url": "http://h/", "kinds": ["alarm.everything"]}),
        json!({"kind": "webhook", "url": "http://h/", "kinds": "alarm.raised"}),
        json!({"kind": "webhook", "url": "http://h/", "enabled": "yes"}),
        json!({"kind": "webhook", "url": "http://h/", "id": "a b"}),
        json!({"kind": "webhook", "url": "http://h/", "name": "n".repeat(65)}),
        json!("not an object"),
    ] {
        refused(bad);
    }
    let dup = vec![
        json!({"id": "a", "kind": "push", "url": "http://h/"}),
        json!({"id": "a", "kind": "push", "url": "http://h/"}),
    ];
    assert_eq!(plan(&dup, &[]).err().unwrap().0, Some(1));
    let mut list: Vec<Value> = (0..17)
        .map(|_| json!({"kind": "push", "url": "http://h/"}))
        .collect();
    assert!(plan(&list, &[]).err().unwrap().1.contains("at most 16"));
    list.pop();
    let (sixteen, _) = plan(
        &[
            vec![json!({"id": "t2", "kind": "push", "url": "http://h/"})],
            list[1..].to_vec(),
        ]
        .concat(),
        &[],
    )
    .unwrap();
    let mut ids: Vec<_> = sixteen.iter().map(|(t, _)| t.id.clone()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 16, "given ids are unique");
}

#[test]
fn wp_p2wh_the_file_is_private_and_a_bad_one_is_set_aside() {
    let dir = scratch("file");
    let (mut t, _) = target("a", &["alarm.raised"]);
    t.secret = Some(new_secret().unwrap());
    save(&dir, std::slice::from_ref(&t)).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(dir.join(FILE_NAME))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let loaded = load(&dir, 1);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].0, t, "the secret survives a restart");

    // One good entry, one bad one: the file is set aside, the good one kept and written.
    let text = json!({"v": 1, "targets": [t, {"id": "b", "kind": "webhook", "url": "https://x/"}]});
    fs::write(dir.join(FILE_NAME), text.to_string()).unwrap();
    let loaded = load(&dir, 77);
    assert_eq!(loaded.len(), 1);
    assert!(dir.join(format!("{FILE_NAME}.bad-77")).exists());
    assert_eq!(load(&dir, 78).len(), 1, "the file written back is whole");
    assert!(!dir.join(format!("{FILE_NAME}.bad-78")).exists());

    // Garbage, and a file too large: no targets, no panic.
    fs::write(dir.join(FILE_NAME), b"{\"v\": 1, \"targ").unwrap();
    assert!(load(&dir, 79).is_empty());
    assert!(dir.join(format!("{FILE_NAME}.bad-79")).exists());
    fs::write(dir.join(FILE_NAME), vec![b' '; (MAX_FILE + 1) as usize]).unwrap();
    assert!(load(&dir, 80).is_empty());
    assert!(dir.join(format!("{FILE_NAME}.bad-80")).exists());
    // None at all is no targets, nothing set aside.
    let empty = scratch("none");
    assert!(load(&empty, 1).is_empty());
    assert!(!empty.exists());
}

#[test]
fn wp_p2wh_only_an_admin_reaches_the_family() {
    let clock: Arc<dyn HubClock> = Arc::new(TestClock::new());
    let hub = crate::Hub::start(crate::HubConfig {
        services: ServiceConfig {
            clock,
            extra: Vec::new(),
        },
        ..crate::HubConfig::in_process("P2WH roles")
    })
    .unwrap();
    let h = hub.handle();
    assert!(h.service_caps().contains(&CAP), "announced");
    let set = json!({"t": "outbound.set", "id": 1, "targets": [
        {"id": "a", "kind": "webhook", "url": "http://127.0.0.1:9/"}]});
    for role in [Role::Viewer, Role::Control] {
        let s = h.open_session(role, None).unwrap();
        for msg in [
            json!({"t": "outbound.get", "id": 1}),
            set.clone(),
            json!({"t": "outbound.test", "id": 1, "target": "a"}),
        ] {
            let answer: Value =
                serde_json::from_str(&h.service_message(&s, &msg).unwrap()).unwrap();
            assert_eq!(answer["code"], "denied", "{role:?} {msg}: {answer}");
            assert_eq!(answer["id"], 1);
        }
        h.close_session(s.id());
    }
    let admin = h.open_session(Role::Admin, None).unwrap();
    let get: Value = serde_json::from_str(
        &h.service_message(&admin, &json!({"t": "outbound.get", "id": 2}))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        get,
        json!({"t": "ok", "id": 2, "v": 1, "targets": []}),
        "none by default"
    );
    let ok: Value = serde_json::from_str(&h.service_message(&admin, &set).unwrap()).unwrap();
    assert_eq!(ok["t"], "ok", "{ok}");
    assert_eq!(ok["targets"][0]["secret"], "not set");
    let missing: Value = serde_json::from_str(
        &h.service_message(
            &admin,
            &json!({"t": "outbound.test", "id": 3, "target": "zz"}),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(missing["code"], "not_found");
    // Another family's message is not this one's.
    assert!(
        h.service_message(&admin, &json!({"t": "outbound.other", "id": 4}))
            .is_none()
    );
    hub.stop();
}
