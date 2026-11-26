//! WP P1CFIX: the review p1c fixups, on the demo device, adopted from the
//! reviewer's probes (`review-p1c/tests_review_p1c.rs`) as tests of the fixed
//! behaviour.
//!
//! - F1: a guarded revert (Revert, and the poll's timeout) restores a value
//!   above a limit set during the countdown.
//! - F3: an A/B re-capture keeps a side's stored value for every row its
//!   recall had to hold; an edit made on the side since stays.
//! - F4: the A/B calls are refused while a guard waits; a revert ends the
//!   compare.
//! - F5: "protect all outputs" takes a lower profile ceiling per output (a
//!   linked pair, its lower one), reports it, and never fails half-way.
//! - N3: a lost guard the hub cannot be asked about is not asked again on the
//!   same session and connection.
//!
//! The timeout runs on this thread's guard clock ([`super::skew`]). Stores
//! live under `std::env::temp_dir()`, which `ci/run.sh` points at the home
//! volume (never tmpfs).

use std::time::Duration;

use dspi_proto::generated::limiter;
use dspi_session::limits::{LimitsProfile, OutputLimits};
use serde_json::Value;

use super::*;
use crate::AbSide;

const O: u8 = 2;

fn demo() -> std::sync::Arc<DspiCore> {
    let core = DspiCore::new();
    core.open_demo().unwrap();
    core
}

fn gain(core: &DspiCore) -> f32 {
    core.get_state().output_gain_db[usize::from(O)]
}

/// Output O: gain at most 3 dB.
fn profile() -> LimitsProfile {
    LimitsProfile {
        outputs: vec![OutputLimits {
            output: O,
            max_gain_db: Some(3.0),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn after(s: u64) {
    skew::advance(Duration::from_secs(s) + Duration::from_millis(1));
}

fn held_note(notes: &[String]) -> bool {
    notes.iter().any(|n| n.contains("Held to the limits"))
}

// ── F1 ──────────────────────────────────────────────────────────────────

/// PLAN-CLIENTS 5.9 / P1LM's done criterion through P1SA's Revert: a limit
/// set during the countdown neither refuses nor holds the revert.
#[test]
fn wp_p1cfix_f1_revert_restores_a_value_above_a_limit_set_during_the_countdown() {
    let core = demo();
    core.set_output_gain(O, 10.0).unwrap();
    core.apply_guard_begin("flow".into(), 30).unwrap();
    core.set_output_gain(O, 0.0).unwrap();
    // The installer sets a profile while the countdown runs.
    core.limits_apply_profile(profile().to_document()).unwrap();
    assert!(
        core.set_output_gain(O, 10.0).is_err(),
        "the limit is in force"
    );
    let end = core.apply_revert().unwrap();
    assert_eq!(end.phase, GuardPhase::Reverted, "{end:?}");
    assert!(
        !held_note(&end.notes),
        "the revert was held: {:?}",
        end.notes
    );
    assert_eq!(gain(&core), 10.0, "the state from before is back");
    // The exemption ended with the revert: the profile judges writes again.
    assert!(core.set_output_gain(O, 9.0).is_err());
}

/// The same through the timeout on the poll.
#[test]
fn wp_p1cfix_f1_timeout_revert_restores_a_value_above_a_limit() {
    let core = demo();
    core.set_output_gain(O, 10.0).unwrap();
    core.apply_guard_begin("flow".into(), 30).unwrap();
    core.set_output_gain(O, 0.0).unwrap();
    core.limits_apply_profile(profile().to_document()).unwrap();
    after(31);
    let end = core.apply_poll();
    assert_eq!(
        (end.phase, end.reason.as_str()),
        (GuardPhase::Reverted, "timeout")
    );
    assert!(!held_note(&end.notes), "{:?}", end.notes);
    assert_eq!(gain(&core), 10.0, "notes {:?}", end.notes);
}

// ── F3 ──────────────────────────────────────────────────────────────────

/// B holds a gain above the limit. Every toggle to B is held and says so;
/// the toggle away keeps B's 10 dB, so the diff is unchanged and Keep(B)
/// after a toggle away recalls (and holds) B again.
#[test]
fn wp_p1cfix_f3_ab_recapture_keeps_the_value_the_recall_held() {
    let core = demo();
    core.set_output_gain(O, 10.0).unwrap();
    let b = core
        .snapshot_capture_document("B".into(), String::new())
        .unwrap();
    core.set_output_gain(O, 0.0).unwrap();
    core.limits_apply_profile(profile().to_document()).unwrap();
    core.ab_start_documents(String::new(), b).unwrap();
    let diff0: Value = serde_json::from_str(&core.ab_diff().unwrap()).unwrap();
    let t1 = core.ab_toggle().unwrap();
    assert_eq!(t1.side, Some(AbSide::B));
    assert_eq!(gain(&core), 3.0, "held to the limit");
    assert!(held_note(&t1.recall.unwrap().notes));
    let t2 = core.ab_toggle().unwrap(); // back to A: B keeps its 10 dB
    assert_eq!(t2.side, Some(AbSide::A));
    assert_eq!(gain(&core), 0.0);
    let diff2: Value = serde_json::from_str(&core.ab_diff().unwrap()).unwrap();
    assert_eq!(diff0, diff2, "the compare's B changed without an edit on B");
    let t3 = core.ab_toggle().unwrap();
    assert!(
        held_note(&t3.recall.unwrap().notes),
        "every recall of B reports the hold again"
    );
    assert_eq!(gain(&core), 3.0);
    // Two more round trips change nothing.
    core.ab_toggle().unwrap();
    core.ab_toggle().unwrap();
    let diff4: Value = serde_json::from_str(&core.ab_diff().unwrap()).unwrap();
    assert_eq!(diff0, diff4);
}

/// An edit on a held row while on B is the user's: it is captured (P1AB
/// round 2) and nothing is held on the way back.
#[test]
fn wp_p1cfix_f3_an_edit_on_a_held_row_is_kept() {
    let core = demo();
    core.set_output_gain(O, 10.0).unwrap();
    let b = core
        .snapshot_capture_document("B".into(), String::new())
        .unwrap();
    core.set_output_gain(O, 0.0).unwrap();
    core.limits_apply_profile(profile().to_document()).unwrap();
    core.ab_start_documents(String::new(), b).unwrap();
    core.ab_toggle().unwrap(); // on B, held to 3
    core.set_output_gain(O, 2.0).unwrap(); // the user's edit on B
    core.ab_toggle().unwrap(); // to A
    assert_eq!(gain(&core), 0.0);
    let t = core.ab_toggle().unwrap(); // to B
    assert_eq!(gain(&core), 2.0, "the edit came back");
    assert!(!held_note(&t.recall.unwrap().notes), "nothing to hold");
}

/// Without a profile the re-capture is P1AB's: the device as it stands.
#[test]
fn wp_p1cfix_f3_without_a_profile_the_recapture_is_unchanged() {
    let core = demo();
    core.set_output_gain(O, 10.0).unwrap();
    let b = core
        .snapshot_capture_document("B".into(), String::new())
        .unwrap();
    core.set_output_gain(O, 0.0).unwrap();
    core.ab_start_documents(String::new(), b).unwrap();
    core.ab_toggle().unwrap();
    assert_eq!(gain(&core), 10.0);
    core.set_output_gain(O, 4.0).unwrap();
    core.ab_toggle().unwrap();
    core.ab_toggle().unwrap();
    assert_eq!(gain(&core), 4.0);
}

// ── F4 ──────────────────────────────────────────────────────────────────

/// The reviewer's scenario: a toggle during the countdown is refused (it
/// would have captured the guarded change into A); the revert ends the
/// compare, so no later toggle can bring the reverted change back.
#[test]
fn wp_p1cfix_f4_ab_calls_are_refused_while_a_guard_waits() {
    let core = demo();
    core.set_output_gain(O, 0.0).unwrap();
    let b = core
        .snapshot_capture_document("B".into(), String::new())
        .unwrap();
    core.ab_start_documents(String::new(), b.clone()).unwrap(); // on A (gain 0)
    core.apply_guard_begin("flow".into(), 30).unwrap(); // before = A, gain 0
    core.set_output_gain(O, 5.0).unwrap(); // the guarded change
    let refused = |r: Result<crate::AbCompare>| match r {
        Err(DspiError::InvalidArgument { msg }) => {
            assert!(msg.contains("finish the trial first"), "{msg}"); // -- WP UFW --
        }
        other => panic!("not refused: {other:?}"),
    };
    refused(core.ab_toggle());
    refused(core.ab_keep(AbSide::B));
    refused(core.ab_start_documents(String::new(), b));
    assert!(core.ab_state().active, "the compare runs on");
    assert_eq!(gain(&core), 5.0, "nothing was sent");
    after(31);
    let end = core.apply_poll();
    assert_eq!(end.phase, GuardPhase::Reverted, "{end:?}");
    assert_eq!(gain(&core), 0.0, "the guard put gain 0 back");
    assert!(!core.ab_state().active, "the revert ended the compare");
    assert!(core.ab_toggle().is_err());
    assert_eq!(gain(&core), 0.0);
}

/// After Keep the compare runs on and the A/B calls work again.
#[test]
fn wp_p1cfix_f4_keep_leaves_the_compare_running() {
    let core = demo();
    core.set_output_gain(O, 0.0).unwrap();
    let b = core
        .snapshot_capture_document("B".into(), String::new())
        .unwrap();
    core.ab_start_documents(String::new(), b).unwrap();
    core.apply_guard_begin("flow".into(), 30).unwrap();
    assert!(core.ab_toggle().is_err());
    core.apply_confirm().unwrap();
    assert!(core.ab_state().active);
    assert_eq!(core.ab_toggle().unwrap().side, Some(AbSide::B));
}

/// The reviewer's guarded snapshot recall with A/B active, then Revert: the
/// device is back on A's state and the compare has ended (F4), so it cannot
/// disagree with the device.
#[test]
fn wp_p1cfix_f4_a_guarded_recall_then_revert_ends_the_compare() {
    let core = demo();
    core.set_output_gain(O, 0.0).unwrap();
    let b = core
        .snapshot_capture_document("B".into(), String::new())
        .unwrap();
    core.set_output_gain(O, 7.0).unwrap();
    let s = core
        .snapshot_capture_document("S".into(), String::new())
        .unwrap();
    core.set_output_gain(O, 0.0).unwrap();
    core.ab_start_documents(String::new(), b).unwrap();
    core.apply_guarded_snapshot_document(s, 30).unwrap();
    assert_eq!(gain(&core), 7.0);
    let end = core.apply_revert().unwrap();
    assert_eq!(end.phase, GuardPhase::Reverted);
    assert_eq!(gain(&core), 0.0);
    assert!(!core.ab_state().active);
}

// ── F5 ──────────────────────────────────────────────────────────────────

const ALL: u8 = limiter::LIMITER_ALL_OUTPUTS as u8;
const ON: u8 = limiter::LIMITER_PARAM_ENABLED as u8;
const THR: u8 = limiter::LIMITER_PARAM_THRESHOLD_DB as u8;
const LINK: u8 = limiter::LIMITER_PARAM_LINK_GROUP as u8;

fn ceiling_profile(output: u8, ceiling: f32) -> LimitsProfile {
    LimitsProfile {
        outputs: vec![OutputLimits {
            output,
            limiter_ceiling_db: Some(ceiling),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// The reviewer's case: protect at -1 under a -3 ceiling on output 2. It
/// succeeds; output 2 and its pair (3) take -3, the others -1; every limiter
/// is on and paired; the clamp is reported.
#[test]
fn wp_p1cfix_f5_protect_all_clamps_to_a_lower_profile_ceiling() {
    let core = demo();
    core.limits_apply_profile(ceiling_profile(O, -3.0).to_document())
        .unwrap();
    core.protect_all_outputs(-1.0).unwrap();
    let s = core.get_state();
    let n = usize::from(s.num_output_channels);
    let groups = crate::params_fx::protect_groups_for_tests(s.num_output_channels);
    for k in 0..n {
        assert!(s.limiter_enabled[k], "output {k} on");
        assert_eq!(s.limiter_link_group[k], groups[k], "output {k}'s group");
        let want = if k == 2 || k == 3 { -3.0 } else { -1.0 };
        assert_eq!(s.limiter_threshold_db[k], want, "output {k}");
    }
    let held = core.limits_status().last_held;
    assert!(
        held.iter().any(|r| r.starts_with("limit.threshold[2]")),
        "{held:?}"
    );
    assert!(
        held.iter().any(|r| r.starts_with("limit.threshold[3]")),
        "the pair: {held:?}"
    );
    // Protecting again sends nothing.
    let h = core.history(1000).len();
    core.protect_all_outputs(-1.0).unwrap();
    assert_eq!(core.history(1000).len(), h);
}

/// From a messy start under the profile (no pairs, output 2 alone in group
/// 4 with output 6, every limiter off at -6): still one call, nothing
/// refused half-way, one undo step plus the enable.
#[test]
fn wp_p1cfix_f5_protect_all_from_a_messy_start_under_a_profile() {
    let core = demo();
    core.set_limiter_param(ALL, LINK, 0.0).unwrap();
    core.set_limiter_param(ALL, ON, 0.0).unwrap();
    core.set_limiter_param(ALL, THR, -6.0).unwrap();
    core.set_limiter_param(2, LINK, 4.0).unwrap();
    core.set_limiter_param(6, LINK, 4.0).unwrap();
    core.limits_apply_profile(ceiling_profile(O, -3.0).to_document())
        .unwrap();
    let n0 = core.history(1000).len();
    core.protect_all_outputs(0.0).unwrap();
    let s = core.get_state();
    for k in 0..usize::from(s.num_output_channels) {
        assert!(s.limiter_enabled[k]);
        let want = if k == 2 || k == 3 { -3.0 } else { 0.0 };
        assert_eq!(s.limiter_threshold_db[k], want, "output {k}");
    }
    assert_eq!(
        core.history(1000).len(),
        n0 + 2,
        "the enable, then one step"
    );
    core.undo().unwrap();
    let u = core.get_state();
    assert!(u.limiter_threshold_db.iter().take(4).all(|&t| t == -6.0));
}

// ── N3 ──────────────────────────────────────────────────────────────────

/// A lost guard with the hub's guard whose hub cannot be asked records where
/// it could not; the next poll on the same session and connection returns
/// the state without asking again.
#[test]
fn wp_p1cfix_n3_a_lost_guard_is_not_asked_again_on_the_same_connection() {
    let core = demo();
    core.apply_guard_begin("flow".into(), 30).unwrap();
    if let Some(r) = lock(&core.guarded.state).running.as_mut() {
        r.hub = Some(HubGuard {
            guard: 1,
            session: 7,
        });
        r.lost = true;
    }
    let st = core.apply_poll();
    assert_eq!(st.phase, GuardPhase::Lost);
    let key = core.lost_key();
    let seen = lock(&core.guarded.state)
        .running
        .as_ref()
        .and_then(|r| r.unreachable_on);
    assert_eq!(seen, Some(key), "where the hub could not be asked");
    assert_eq!(core.apply_poll().phase, GuardPhase::Lost);
}
