//! WP REVDFIX tests (review REVD B1, B3): a tweeter in the user's own rows
//! stays a tweeter for agents after an agent's statement replaces them (the
//! ratchet), until the user changes it; and a statement's top-fall checks
//! never come from an import, another layout or the caller.

use std::sync::Arc;

use dspi_transport::FakeDevice;

use super::*;
use crate::consts::{IMPORT_BY_BLOCK, IMPORT_CROSSOVER};
use crate::measure::trusted_roles;

/// A demo device opened as `serial` (layouts kept in memory).
fn core_as(serial: &str) -> Arc<DspiCore> {
    let core = DspiCore::new();
    core.attach_probed(Box::new(FakeDevice::demo()), serial.into(), 0)
        .unwrap();
    core.fetch_all().unwrap();
    core
}

fn row(output: u8, role: MeasureRole) -> SpeakerLayoutOutput {
    SpeakerLayoutOutput {
        output,
        role,
        speaker: "L".into(),
        group: 0,
        source: RoleSource::Suggested,
    }
}

fn users(roles: &[MeasureRole], confirmed: bool) -> SpeakerLayout {
    SpeakerLayout {
        outputs: roles
            .iter()
            .enumerate()
            .map(|(o, r)| row(o as u8, *r))
            .collect(),
        confirmed_by_user: confirmed,
        ..SpeakerLayout::default()
    }
}

fn top_falls(output: u8, role: MeasureRole) -> RoleCheck {
    RoleCheck {
        output,
        role,
        verdict: RoleCheckVerdict::TopFalls,
        floor_hz: 1_000.0,
        ..RoleCheck::default()
    }
}

const LMH: [MeasureRole; 3] = [MeasureRole::Low, MeasureRole::Mid, MeasureRole::High];

#[test]
fn wp_revdfix_the_users_confirmed_tweeter_stays_one_for_agents() {
    let core = core_as("REVDFIX-B1");
    core.speaker_layout_set(users(&LMH, true)).unwrap();
    let l = core
        .speaker_roles_state(
            vec![
                row(0, MeasureRole::Low),
                row(1, MeasureRole::Mid),
                row(2, MeasureRole::Mid),
            ],
            "out 3 is a midrange now".into(),
            "the user rewired".into(),
        )
        .unwrap();
    assert_eq!(l.state(), LayoutState::Stated);
    let st = l.stated.as_ref().unwrap();
    assert_eq!(
        st.replaced
            .iter()
            .map(|r| (r.output, r.role))
            .collect::<Vec<_>>(),
        LMH.iter()
            .enumerate()
            .map(|(o, r)| (o as u8, *r))
            .collect::<Vec<_>>(),
        "the confirmed rows are on record"
    );
    assert!(l.user_tweeter_held(2));
    assert!(!l.user_tweeter_held(0) && !l.user_tweeter_held(1));
    assert_eq!(trusted_roles(&l).unwrap()[2], Some(MeasureRole::High));
    assert_eq!(trusted_roles(&l).unwrap()[1], Some(MeasureRole::Mid));
    assert!(l.agent_floor_held(2));
    assert_eq!(l.unchecked(&[0, 1, 2]), vec![0, 1], "no check clears it");
    // A top that falls does not clear the user's tweeter.
    core.speaker_roles_checked(top_falls(2, MeasureRole::Mid));
    let l = core.speaker_layout().unwrap().unwrap();
    assert!(l.agent_floor_held(2));
    assert_eq!(trusted_roles(&l).unwrap()[2], Some(MeasureRole::High));
    // Another statement carries it forward.
    let l = core
        .speaker_roles_state(
            vec![row(0, MeasureRole::Low), row(2, MeasureRole::Low)],
            "out 3 is the woofer".into(),
            String::new(),
        )
        .unwrap();
    assert!(l.user_tweeter_held(2));
    assert_eq!(trusted_roles(&l).unwrap()[2], Some(MeasureRole::High));
    assert!(l.agent_floor_held(2));
    // The user changes the row themselves (Settings › Speakers): their word
    // ends it, for that row.
    let mut mine = l.clone();
    mine.outputs[1].role = MeasureRole::Sub;
    let l = core.speaker_layout_set(mine).unwrap();
    assert_eq!(l.output(2).unwrap().source, RoleSource::User);
    assert!(!l.user_tweeter_held(2));
    // And a confirmation ends it for every row.
    let core = core_as("REVDFIX-B1b");
    core.speaker_layout_set(users(&LMH, true)).unwrap();
    let mut l = core
        .speaker_roles_state(vec![row(2, MeasureRole::Mid)], "w".into(), "rewired".into())
        .unwrap();
    assert!(l.user_tweeter_held(2));
    l.confirmed_by_user = true;
    let l = core.speaker_layout_set(l).unwrap();
    assert_eq!(l.state(), LayoutState::Confirmed);
    assert!(!l.user_tweeter_held(2));
    assert_eq!(trusted_roles(&l).unwrap()[2], Some(MeasureRole::Mid));
}

#[test]
fn wp_revdfix_a_row_the_user_set_counts_unconfirmed_too() {
    let core = core_as("REVDFIX-B1c");
    // An unconfirmed layout whose Out 3 the user set as a tweeter.
    let mut l = users(&LMH, false);
    l.outputs[2].source = RoleSource::User;
    core.speaker_layout_set(l).unwrap();
    let l = core
        .speaker_roles_state(vec![row(2, MeasureRole::Low)], "w".into(), String::new())
        .unwrap();
    assert_eq!(
        l.stated.as_ref().unwrap().replaced.len(),
        1,
        "the suggestions are nobody's word"
    );
    assert!(l.user_tweeter_held(2));
}

#[test]
fn wp_revdfix_an_imported_statement_brings_no_checks() {
    let src = core_as("REVDFIX-SRC");
    src.speaker_roles_state(vec![row(1, MeasureRole::Low)], "w".into(), String::new())
        .unwrap();
    src.speaker_roles_checked(top_falls(1, MeasureRole::Low));
    assert!(!src.speaker_layout().unwrap().unwrap().agent_floor_held(1));
    let text = src.config_export("R".into(), "t".into(), 0).unwrap();
    assert!(text.contains("\"stated\""), "the export carries it");
    let core = core_as("REVDFIX-DST");
    let t = core.confirm_token(crate::ConfirmOp::ConfigImport);
    let r =
        core.config_import_confirmed(text, IMPORT_BY_BLOCK | IMPORT_CROSSOVER, u32::MAX, Some(t));
    let l = core
        .speaker_layout()
        .unwrap()
        .unwrap_or_else(|| panic!("{r}"));
    assert!(l.stated.is_none(), "{:?}", l.stated);
    // The agent's statement on this device waits for its own check.
    let l = core
        .speaker_roles_state(
            vec![row(1, MeasureRole::Low)],
            "out 2 is the woofer".into(),
            String::new(),
        )
        .unwrap();
    assert!(l.check_of(1).is_none());
    assert!(l.agent_floor_held(1));
    assert_eq!(l.unchecked(&[1]), vec![1]);
}

#[test]
fn wp_revdfix_a_statement_keeps_only_its_own_rows_checks() {
    let core = core_as("REVDFIX-B3");
    // The user's set on a fresh core (the web page's kept copy) keeps a
    // caller's statement, but none of its checks that would clear an output.
    let mut forged = users(&LMH, false);
    let holds = RoleCheck {
        verdict: RoleCheckVerdict::TopHolds,
        ..top_falls(1, MeasureRole::Mid)
    };
    forged.stated = Some(Statement {
        words: "w".into(),
        checks: vec![top_falls(0, MeasureRole::Low), holds.clone()],
        device_outputs: 8,
        ..Statement::default()
    });
    let l = core.speaker_layout_set(forged.clone()).unwrap();
    let st = l.stated.as_ref().unwrap();
    assert_eq!(st.words, "w");
    assert_eq!(st.checks, vec![holds], "the clearing check is dropped");
    // With a statement on record, that one stays, whatever the caller says.
    forged.stated.as_mut().unwrap().words = "other".into();
    let l = core.speaker_layout_set(forged).unwrap();
    assert_eq!(l.stated.as_ref().unwrap().words, "w");
    assert_eq!(l.stated.as_ref().unwrap().checks.len(), 1);
    // Over a layout that is not a statement, no checks are kept...
    let l = core
        .speaker_roles_state(vec![row(0, MeasureRole::Low)], "w".into(), String::new())
        .unwrap();
    assert!(l.check_of(0).is_none());
    core.speaker_roles_checked(top_falls(0, MeasureRole::Low));
    assert!(!core.speaker_layout().unwrap().unwrap().agent_floor_held(0));
    // ...over a statement, a check stays with its unchanged row...
    let l = core
        .speaker_roles_state(vec![row(0, MeasureRole::Low)], "w".into(), String::new())
        .unwrap();
    assert!(l.check_of(0).is_some());
    // ...but not when the row moved to another speaker.
    let mut r = row(0, MeasureRole::Low);
    r.speaker = "R".into();
    let l = core
        .speaker_roles_state(vec![r], "w".into(), String::new())
        .unwrap();
    assert!(l.check_of(0).is_none());
    assert!(l.agent_floor_held(0));
    // A check made before the user confirmed the layout is not the next
    // statement's: the user's rows were never checked as stated.
    let core = core_as("REVDFIX-B3b");
    core.speaker_roles_state(vec![row(0, MeasureRole::Low)], "w".into(), String::new())
        .unwrap();
    core.speaker_roles_checked(top_falls(0, MeasureRole::Low));
    let mut l = core.speaker_layout().unwrap().unwrap();
    l.confirmed_by_user = true;
    let l = core.speaker_layout_set(l).unwrap();
    assert_eq!(l.state(), LayoutState::Confirmed);
    assert!(l.stated.as_ref().is_some_and(|s| !s.checks.is_empty()));
    let l = core
        .speaker_roles_state(
            vec![row(0, MeasureRole::Low)],
            "w".into(),
            "the user rewired".into(),
        )
        .unwrap();
    assert!(l.check_of(0).is_none());
    assert!(l.agent_floor_held(0));
}
