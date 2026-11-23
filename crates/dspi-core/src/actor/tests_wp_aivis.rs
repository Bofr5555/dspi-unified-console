//! WP AIVIS: the user's changes are logged by cursor for the other actors,
//! jobs' own writes are not, and a control the user took back from an actor
//! is refused to it until it has had the time to ask.

use std::sync::Arc;

use super::*;
use crate::actor::{ActorScope, JobActor, WriteHook, register_actor};

fn demo() -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    core
}

fn judged() -> Option<WriteHook> {
    Some(Arc::new(|_| Ok(())))
}

fn code(e: DspiError) -> String {
    match e {
        DspiError::ConfirmationRequired { code, .. } => code,
        other => panic!("{other}"),
    }
}

fn user(core: &DspiCore, since: u64) -> Vec<UserChange> {
    let mut c = changes_since(core, since);
    c.retain(|c| c.by == ChangedBy::User);
    c
}

#[test]
fn wp_aivis_the_users_changes_are_listed_by_cursor() {
    let core = demo();
    let agent = register_actor("AI · t");
    let start = latest();
    core.set_output_gain(0, -3.0).unwrap();
    let first = user(&core, start);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].path, "out.gain");
    assert_eq!(first[0].indices, [0]);
    assert_eq!(first[0].after, ParamValue::Float(-3.0));
    assert_eq!(first[0].label(), by_path("out.gain").unwrap().label);

    // The agent's own writes are not the user's; a job's neither
    {
        let _s = ActorScope::enter(&core, agent, judged());
        core.set_output_gain(1, -4.0).unwrap();
    }
    {
        let _s = JobActor::of(EMBEDDER).attribute(&core);
        core.set_output_gain(1, -5.0).unwrap();
    }
    assert!(user(&core, first[0].seq).is_empty());

    // A drag is one change, from where it began
    let before = latest();
    core.set_dragging(true);
    for g in [-4.0, -5.0, -6.0] {
        core.set_output_gain(0, g).unwrap();
    }
    core.set_dragging(false);
    let drag = user(&core, before);
    assert_eq!(drag.len(), 1);
    assert_eq!(drag[0].after, ParamValue::Float(-6.0));
    assert_eq!(drag[0].before, Some(ParamValue::Float(-3.0)));

    // An undo is the user's; so is a preset load, and the log outlives it
    core.undo().unwrap();
    assert!(user(&core, drag[0].seq).iter().all(|c| c.undo));
    let before = latest();
    core.load_preset(1).unwrap();
    assert!(user(&core, before).iter().any(|c| wholesale(&c.path)));
}

#[test]
fn wp_aivis_a_control_the_user_took_back_is_refused_to_the_agent() {
    let core = demo();
    let agent = register_actor("AI · t");
    let other = register_actor("AI · u");
    let by = |actor, out, db| {
        let _s = ActorScope::enter(&core, actor, judged());
        core.set_output_gain(out, db)
    };
    by(agent, 0, -3.0).unwrap();
    by(agent, 0, -4.0).unwrap(); // its own tuning goes on
    core.set_output_gain(0, -10.0).unwrap();
    assert_eq!(code(by(agent, 0, -5.0).unwrap_err()), USER_CHANGED);
    assert_eq!(core.get_state().output_gain_db[0], -10.0);
    // A blind retry is refused too; another control and another agent are not
    assert_eq!(code(by(agent, 0, -5.0).unwrap_err()), USER_CHANGED);
    by(agent, 1, -5.0).unwrap();
    by(other, 0, -11.0).unwrap();
    // The user's yes lifts it
    allowed(&core, agent);
    by(agent, 0, -5.0).unwrap();

    // A control the agent never set is free
    core.set_output_gain(2, -7.0).unwrap();
    by(agent, 2, -8.0).unwrap();
}

#[test]
fn wp_aivis_the_refusal_ends_after_the_time_to_ask() {
    let mut log = UserLog::default();
    let c = Control::of("out.gain", &[0]);
    log.touch(c.clone(), 7, 1_000);
    log.contest("out.gain", &[0], 2_000);
    let ask = |log: &UserLog, at| log.may_write(7, std::slice::from_ref(&c), at);
    assert!(!ask(&log, 3_000));
    assert!(!ask(&log, 3_000 + ASK_FIRST_MS - 1));
    assert!(ask(&log, 3_000 + ASK_FIRST_MS));
    // and is over
    assert!(ask(&log, 3_000));

    // A band is one control; a preset load contests them all; an old write
    // is not "in the middle of tuning"
    log.touch(Control::of("eq.freq", &[4, 1]), 7, 10_000);
    log.contest("eq.gain", &[4, 1], 11_000);
    assert!(!log.may_write(7, &[Control::of("eq.band", &[4, 1])], 11_500));
    log.touch(c.clone(), 7, 20_000);
    log.contest("preset.load", &[], 21_000);
    assert!(!ask(&log, 22_000));
    log.touch(Control::of("out.mute", &[3]), 7, 30_000);
    log.contest("out.mute", &[3], 30_001 + TUNING_MS);
    assert!(log.may_write(7, &[Control::of("out.mute", &[3])], 30_002 + TUNING_MS));
    // and one nobody was told of lapses
    assert!(!log.contested_by(8));
}

#[test]
fn wp_aivis_a_plan_is_checked_whole_and_acts_are_logged() {
    let core = demo();
    let agent = register_actor("AI · t");
    {
        let _s = ActorScope::enter(&core, agent, judged());
        core.set_output_gain(0, -3.0).unwrap();
    }
    core.set_output_gain(0, -9.0).unwrap();
    let plan: [(&str, &[u8]); 2] = [("out.gain", &[1]), ("out.gain", &[0])];
    assert_eq!(
        code(check_plan(&core, agent, plan.into_iter()).unwrap_err()),
        USER_CHANGED
    );
    // Reverts are never refused: no hook
    {
        let _s = ActorScope::enter(&core, agent, None);
        core.set_output_gain(0, -3.0).unwrap();
    }
    let before = latest();
    did(&core, "measure_stop");
    {
        let _s = ActorScope::enter(&core, agent, None);
        did(&core, "measure_stop");
    }
    let acts = user(&core, before);
    assert_eq!(acts.len(), 1);
    assert_eq!(acts[0].path, "did.measure_stop");
}

#[test]
fn wp_aivis_another_writers_change_is_named_apart() {
    let mut log = UserLog::default();
    let (name, base, _) = SECTIONS[0];
    elsewhere(&mut log, Some(base), "a control surface");
    elsewhere(&mut log, None, "another app");
    let paths: Vec<&str> = log.changes.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(paths, [name, "everything"]);
    assert!(log.changes.iter().all(|c| c.by == ChangedBy::Elsewhere));
}

#[test]
fn wp_aivis_a_recall_of_the_agents_counts_for_every_control_and_a_reopen_for_none() {
    let core = demo();
    let agent = register_actor("AI · t");
    {
        let _s = ActorScope::enter(&core, agent, judged());
        core.load_preset(1).unwrap();
    }
    check_keep(&core, agent).unwrap();
    core.set_output_gain(0, -9.0).unwrap();
    assert_eq!(code(check_keep(&core, agent).unwrap_err()), USER_CHANGED);
    let by = |out| {
        let _s = ActorScope::enter(&core, agent, judged());
        core.set_output_gain(out, -5.0)
    };
    assert_eq!(code(by(0).unwrap_err()), USER_CHANGED);
    by(1).unwrap();
    // Another device on the same core
    core.open_demo().unwrap();
    by(0).unwrap();
}

#[test]
fn wp_aivis_a_change_counts_as_a_disturbance_until_the_session_ends() {
    let core = demo();
    let start = latest();
    assert_eq!(disturbed(&core, start, |_| true), None);
    core.set_output_gain(0, -9.0).unwrap();
    assert!(disturbed(&core, start, |_| true).is_some());
    assert_eq!(disturbed(&core, start, |_| false), None);
    let start = latest();
    measure_ended(&mut core.lock());
    core.set_output_gain(0, -8.0).unwrap();
    assert_eq!(disturbed(&core, start, |_| true), None);
}
