//! WP REVDFIX tests (review REVD B3): a statement's top-fall checks never
//! come from an import, another layout or the caller.

use std::sync::Arc;

use dspi_transport::FakeDevice;

use super::*;
use crate::consts::{IMPORT_BY_BLOCK, IMPORT_CROSSOVER};

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
    // The caller's statement is not kept by the user's set.
    let mut forged = users(&LMH, false);
    forged.stated = Some(Statement {
        checks: vec![top_falls(0, MeasureRole::Low)],
        device_outputs: 8,
        ..Statement::default()
    });
    let l = core.speaker_layout_set(forged).unwrap();
    assert!(l.stated.is_none());
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
