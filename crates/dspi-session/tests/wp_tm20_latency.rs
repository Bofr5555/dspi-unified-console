//! WP TM20: `out.latency` is applied by the firmware's main loop, so its
//! confirmation has to wait for `pending` to clear; and `dev.name ""` clears.

use std::time::{Duration, Instant};

use dspi_proto::generated::opcodes as op;
use dspi_proto::value::Value;
use dspi_session::{Outcome, Session, WriteError, probe};
use dspi_transport::{FakeDevice, FakeRemote, TransportError};

/// The demo's default option, `OUTPUT_LATENCY_DEFAULT`.
const DEFAULT_ID: u8 = 5;
const WANTED: u8 = 3;

fn session() -> (Session, FakeRemote) {
    let mut d = FakeDevice::demo();
    let remote = d.remote();
    let caps = probe(&mut d).unwrap();
    (Session::new(Box::new(d), caps).unwrap(), remote)
}

fn set(s: &mut Session, id: u8) -> Result<Outcome, WriteError> {
    s.write("out.latency", &[], Value::Choice(id))
}

fn requested(s: &mut Session) -> u8 {
    s.read("out.latency", &[]).unwrap().as_u8().unwrap()
}

#[test]
fn wp_tm20_a_prompt_apply_confirms() {
    let (mut s, remote) = session();
    assert!(matches!(
        set(&mut s, WANTED).unwrap(),
        Outcome::Confirmed(_)
    ));
    assert_eq!(requested(&mut s), WANTED);
    assert_eq!(remote.latency_sets(), 1);
}

#[test]
fn wp_tm20_a_deferred_apply_confirms() {
    let (mut s, remote) = session();
    remote.defer_latency(3);
    assert!(matches!(
        set(&mut s, WANTED).unwrap(),
        Outcome::Confirmed(_)
    ));
    assert_eq!(requested(&mut s), WANTED);
    assert_eq!(remote.latency_sets(), 1, "the write went once");
}

/// The bug itself: right after the SET the status still has the old id.
#[test]
fn wp_tm20_an_immediate_read_sees_the_old_id_and_pending() {
    let (mut s, remote) = session();
    remote.hold_latency();
    s.send("out.latency", &[], Value::Choice(WANTED)).unwrap();
    let status = s
        .with_transport(|t| t.control_in(op::REQ_OUTPUT_LATENCY, 0, 12))
        .unwrap();
    assert_eq!((status[0], status[8]), (DEFAULT_ID, 1));
}

/// The flash write takes the control endpoint away; that is not a failure.
#[test]
fn wp_tm20_a_blackout_during_the_apply_is_waited_out() {
    let (mut s, remote) = session();
    remote.defer_latency(1);
    remote.black_out_latency(3);
    assert!(matches!(
        set(&mut s, WANTED).unwrap(),
        Outcome::Confirmed(_)
    ));
    assert_eq!(remote.latency_sets(), 1);
}

#[test]
fn wp_tm20_a_real_refusal_is_rejected() {
    let (mut s, remote) = session();
    remote.refuse_latency();
    let out = set(&mut s, WANTED).unwrap();
    assert!(!out.is_confirmed());
    assert!(
        matches!(&out, Outcome::Rejected { actual, .. } if actual.as_u8() == Some(DEFAULT_ID)),
        "{out:?}"
    );
}

#[test]
fn wp_tm20_a_refusal_after_the_wait_is_rejected() {
    let (mut s, remote) = session();
    remote.defer_latency(2);
    remote.refuse_latency();
    let out = set(&mut s, WANTED).unwrap();
    assert!(matches!(out, Outcome::Rejected { .. }), "{out:?}");
}

#[test]
fn wp_tm20_a_pending_that_never_clears_ends_with_a_clear_message() {
    let (mut s, remote) = session();
    remote.hold_latency();
    let start = Instant::now();
    let e = set(&mut s, WANTED).unwrap_err();
    let took = start.elapsed();

    assert!(matches!(e, WriteError::Unsettled { .. }), "{e:?}");
    let msg = e.to_string();
    assert!(msg.contains("out.latency"), "{msg}");
    assert!(msg.contains("still reports it pending"), "{msg}");
    assert!(msg.contains("read it back"), "{msg}");
    assert!(took >= Duration::from_secs(4), "{took:?}");
    assert!(took < Duration::from_secs(6), "{took:?}");
    assert_eq!(remote.latency_sets(), 1, "the wait did not write again");
}

#[test]
fn wp_tm20_a_device_that_never_answers_ends_in_the_same_bound() {
    let (mut s, remote) = session();
    remote.hold_latency();
    remote.black_out_latency(u32::MAX);
    let start = Instant::now();
    let e = set(&mut s, WANTED).unwrap_err();
    let took = start.elapsed();
    assert!(
        matches!(&e, WriteError::Unsettled { why, .. } if why.contains("stalled")),
        "{e:?}"
    );
    assert!(took < Duration::from_secs(6), "{took:?}");
}

#[test]
fn wp_tm20_a_device_lost_mid_poll_fails_fast_without_writing_again() {
    let (mut s, remote) = session();
    remote.hold_latency();
    let unplug = remote.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        unplug.disconnect();
    });

    let start = Instant::now();
    let e = set(&mut s, WANTED).unwrap_err();
    let took = start.elapsed();
    t.join().unwrap();

    assert!(
        matches!(e, WriteError::Transport(TransportError::Disconnected)),
        "{e:?}"
    );
    assert!(took < Duration::from_secs(2), "{took:?}");
    assert_eq!(remote.latency_sets(), 1, "no second write");
    assert!(s.journal.is_empty(), "not recorded as done");
}

/// A dry run sends nothing, so there is nothing to wait for.
#[test]
fn wp_tm20_a_dry_run_does_not_wait() {
    let (mut s, remote) = session();
    remote.hold_latency();
    s.dry_run = true;
    let start = Instant::now();
    assert_eq!(set(&mut s, WANTED).unwrap(), Outcome::Accepted);
    assert!(start.elapsed() < Duration::from_millis(50));
    assert_eq!(remote.latency_sets(), 0);
}

/// `dspi-core`'s `set_output_latency` is this `send`: no read-back to get wrong
/// and no wait, on a device that has not applied it yet.
#[test]
fn wp_tm20_send_neither_reads_back_nor_waits() {
    let (mut s, remote) = session();
    remote.hold_latency();
    let start = Instant::now();
    s.send("out.latency", &[], Value::Choice(WANTED)).unwrap();
    assert!(start.elapsed() < Duration::from_millis(50));
    assert_eq!(remote.latency_sets(), 1);
}

/// `dev.name ""` through the grammar clears the name on the demo device.
#[cfg(feature = "undo")]
#[test]
fn wp_tm20_an_empty_quoted_name_clears_the_device_name() {
    let (mut s, _) = session();
    let name = |s: &mut Session| s.read("dev.name", &[]).unwrap();
    assert_eq!(name(&mut s), Value::Text("DSPi Demo".into()));

    let run = |s: &mut Session, line: &str| {
        let toks = dspi_cmd::tokenize(line);
        let refs: Vec<&str> = toks.iter().map(String::as_str).collect();
        match dspi_cmd::parse(&refs, &dspi_cmd::Context::default()).unwrap() {
            dspi_cmd::Command::Set {
                path,
                indices,
                value,
            } => s.write(path, &indices, value).unwrap(),
            other => panic!("`{line}` is {other:?}"),
        }
    };

    assert!(run(&mut s, r#"dev.name "12\" Sub""#).is_confirmed());
    assert_eq!(name(&mut s), Value::Text(r#"12" Sub"#.into()));

    assert!(run(&mut s, r#"dev.name """#).is_confirmed());
    assert_eq!(name(&mut s), Value::Text(String::new()));
}
