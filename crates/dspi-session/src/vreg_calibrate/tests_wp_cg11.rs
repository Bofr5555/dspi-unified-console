//! WP CG11: the state machine alone. The same paths on `FakeDevice` are in
//! `dspi-core`'s `vreg/tests_wp_cg11.rs`.

use proptest::prelude::*;

use super::*;

const SOAK: u32 = DEFAULT_SOAK_S;
const WORD: &str = overvolt::WORD;

/// A K340: build default 1.20 V, up for an hour, nothing on record.
fn k340() -> CoreVregStatus {
    CoreVregStatus {
        live_vreg: VREG_1_20,
        stored_sel: 0,
        default_vreg: VREG_1_20,
        flags: 0,
        hang_count: 0,
        trial_active: false,
        trial_vreg: 0,
        failed_trial_vreg: 0,
        failed_trial_uptime_s: 0,
        uptime_s: 3600,
    }
}

fn with(f: impl FnOnce(&mut CoreVregStatus)) -> CoreVregStatus {
    let mut s = k340();
    f(&mut s);
    s
}

fn running(code: u8, uptime_s: u32) -> CoreVregStatus {
    with(|s| {
        s.trial_active = true;
        s.trial_vreg = code;
        s.live_vreg = code;
        s.uptime_s = uptime_s;
    })
}

/// The `TRIAL` of a run on a K340: the word was typed if it was needed.
fn ft(code: u8) -> FlowTrial {
    FlowTrial {
        code,
        word: code > VREG_1_20,
    }
}

/// Open, start at `target`, past the `CLEAR` and into the soak at uptime 3600.
fn soaking(target: u8) -> Calibrator {
    let mut c = Calibrator::new();
    c.open(&k340(), 0).unwrap();
    let typed = needs_word(target, &k340()).then_some(WORD);
    let op = c.start(&k340(), target, SOAK, typed, 0).unwrap();
    assert_eq!(op, VregOp::Clear);
    assert_eq!(c.on_status(&k340(), 10), Some(ft(target)));
    assert_eq!(c.on_status(&running(target, 3600), 20), None);
    assert!(matches!(c.phase(), Phase::Soaking { .. }));
    c
}

#[test]
fn wp_cg11_the_range_runs_from_the_build_default_to_1v25_only() {
    assert_eq!(offered(0x0C), vec![0x0C, 0x0D, 0x0E]);
    assert_eq!(offered(0x0D), vec![0x0D, 0x0E]);
    assert_eq!(offered(0x0E), vec![0x0E]);
    // A build above the flow's ceiling, and a nonsense default, offer nothing.
    for bad in [0x0F, 0x0B, 0x00, 0xFF] {
        assert_eq!(offered(bad), Vec::<u8>::new());
    }
    assert!(!in_flow_range(0x0F, 0x0C), "1.30 V is never offered");
    assert!(!in_flow_range(0x0C, 0x0D), "never below the build default");
}

#[test]
fn wp_cg14_the_flow_judges_the_word_on_the_units_status() {
    let on = |live: u8, stored: u8, default_vreg: u8| {
        with(|s| {
            s.live_vreg = live;
            s.stored_sel = stored;
            s.default_vreg = default_vreg;
        })
    };
    assert!(needs_word(0x0E, &on(0x0D, 0, 0x0D)), "1.20 to 1.25");
    assert!(needs_word(0x0E, &on(0x0C, 0, 0x0C)), "1.15 to 1.25");
    assert!(!needs_word(0x0D, &on(0x0C, 0, 0x0C)), "1.15 to 1.20");
    assert!(!needs_word(0x0E, &on(0x0E, 0x0E, 0x0D)), "1.25 stays");
    assert!(!needs_word(0x0E, &on(0x0D, 0x0E, 0x0D)), "stored at 1.25");
    assert!(!needs_word(0x0E, &on(0x0E, 0, 0x0D)), "on trial at 1.25");
}

#[test]
fn wp_cg14_already_at_1v25_it_can_trial_with_no_word() {
    let at = with(|s| {
        s.stored_sel = 0x0E;
        s.live_vreg = 0x0E;
    });
    let mut c = Calibrator::new();
    c.open(&at, 0).unwrap();
    assert_eq!(c.start(&at, 0x0E, SOAK, None, 0), Ok(VregOp::Clear));
    let t = c.on_status(&at, 1).unwrap();
    assert_eq!(t.word(), None);
}

#[test]
fn wp_cg14_a_unit_that_came_down_since_the_start_stops_a_wordless_trial() {
    let at = with(|s| {
        s.stored_sel = 0x0E;
        s.live_vreg = 0x0E;
    });
    let mut c = Calibrator::new();
    c.open(&at, 0).unwrap();
    c.start(&at, 0x0E, SOAK, None, 0).unwrap();
    // Someone set it back to 1.20 V between the CLEAR and the TRIAL.
    assert_eq!(c.on_status(&k340(), 1), None);
    assert!(matches!(c.phase(), Phase::Unknown { .. }));
}

#[test]
fn wp_cg14_a_save_is_judged_on_the_status_at_that_moment() {
    let mut c = soaking(0x0E);
    c.on_status(&running(0x0E, 3600 + SOAK), 50);
    // The trial gone from a stale status: 1.20 V now, so 1.25 V is a raise.
    assert!(matches!(
        c.commit(&k340(), None, 60),
        Err(FlowError::WordRequired { .. })
    ));
    assert!(matches!(c.phase(), Phase::Passed { .. }));
    // Still on trial at 1.25 V: persisting it asks again, and stays in Passed.
    let on_trial = running(0x0E, 3600 + SOAK);
    assert!(save_needs_word(0x0E, &on_trial));
    assert!(!needs_word(0x0E, &on_trial), "a trial of it would be level");
    assert!(matches!(
        c.commit(&on_trial, None, 61),
        Err(FlowError::WordRequired { .. })
    ));
    assert!(matches!(c.phase(), Phase::Passed { .. }));
    assert_eq!(c.commit(&on_trial, Some(WORD), 62), Ok(VregOp::Set(0x0E)));
}

#[test]
fn wp_cg11_nothing_starts_without_opening_and_idle_polls_never_write() {
    let mut c = Calibrator::new();
    assert!(matches!(
        c.start(&k340(), 0x0D, SOAK, None, 0),
        Err(FlowError::WrongPhase { .. })
    ));
    for _ in 0..3 {
        assert_eq!(c.on_status(&k340(), 0), None);
    }
    assert_eq!(*c.phase(), Phase::Idle);
    c.open(&k340(), 0).unwrap();
    assert_eq!(c.on_status(&k340(), 0), None);
    assert_eq!(*c.phase(), Phase::Review);
}

#[test]
fn wp_cg11_start_checks_range_soak_and_typing_before_anything_changes() {
    let mut c = Calibrator::new();
    c.open(&k340(), 0).unwrap();
    for (target, soak, typed, ok) in [
        (0x0C, SOAK, None, false),       // below the build default
        (0x0F, SOAK, Some(WORD), false), // 1.30 V, word or not
        (0x0D, MIN_SOAK_S - 1, None, false),
        (0x0E, SOAK, None, false),
        (0x0E, SOAK, Some("yes"), false),
        (0x0E, SOAK, Some("overvolt"), false),
    ] {
        assert_eq!(c.start(&k340(), target, soak, typed, 0).is_ok(), ok);
        assert_eq!(*c.phase(), Phase::Review, "{target:#x}");
    }
    assert_eq!(
        c.start(&k340(), 0x0E, MIN_SOAK_S, Some(WORD), 0),
        Ok(VregOp::Clear)
    );
}

#[test]
fn wp_cg11_the_trial_waits_for_the_clear_to_show() {
    let mut c = Calibrator::new();
    let old = with(|s| {
        s.hang_count = 1;
        s.failed_trial_vreg = 0x0C;
        s.failed_trial_uptime_s = 40;
    });
    c.open(&old, 0).unwrap();
    assert_eq!(
        TrialFailure::on_record(&old).map(|f| f.vreg),
        Some(0x0C),
        "the record is there to show first"
    );
    c.start(&old, 0x0D, SOAK, None, 0).unwrap();
    assert_eq!(c.on_status(&old, 100), None, "not applied yet");
    assert_eq!(c.on_status(&k340(), 200), Some(ft(0x0D)));
}

#[test]
fn wp_cg11_a_clear_that_never_shows_sends_no_trial() {
    let mut c = Calibrator::new();
    let old = with(|s| s.hang_count = 1);
    c.open(&old, 0).unwrap();
    c.start(&old, 0x0D, SOAK, None, 0).unwrap();
    assert_eq!(c.on_status(&old, VERIFY_TIMEOUT_MS), None);
    assert_eq!(
        *c.phase(),
        Phase::Unknown {
            target: 0x0D,
            reason: UnknownReason::NotStarted
        }
    );
    assert_eq!(c.on_status(&k340(), VERIFY_TIMEOUT_MS + 1), None);
}

#[test]
fn wp_cg11_a_long_uptime_is_not_a_soak() {
    // Up for an hour, a 30-minute soak: the plan's "uptime at least the soak"
    // would pass this at once.
    let mut c = soaking(0x0D);
    assert_eq!(c.on_status(&running(0x0D, 3601), 30), None);
    assert!(matches!(c.phase(), Phase::Soaking { .. }));
    c.on_status(&running(0x0D, 3600 + SOAK - 1), 40);
    assert!(matches!(c.phase(), Phase::Soaking { .. }));
    assert_eq!(c.soak_elapsed_s(&running(0x0D, 3600 + SOAK - 1)), SOAK - 1);
    c.on_status(&running(0x0D, 3600 + SOAK), 50);
    assert!(matches!(c.phase(), Phase::Passed { .. }));
}

#[test]
fn wp_cg11_a_pass_commits_nothing_until_asked() {
    let mut c = soaking(0x0D);
    let at = 3600 + SOAK;
    for i in 0..5 {
        assert_eq!(c.on_status(&running(0x0D, at + i), 50), None);
    }
    assert!(matches!(c.phase(), Phase::Passed { .. }));
    assert_eq!(c.commit(&k340(), None, 60), Ok(VregOp::Set(0x0D)));
    let saved = with(|s| s.stored_sel = 0x0D);
    c.on_status(&saved, 70);
    assert_eq!(*c.phase(), Phase::Committed { value: 0x0D });
}

#[test]
fn wp_cg11_a_set_is_verified_on_stored_and_live() {
    let mut c = soaking(0x0E);
    c.on_status(&running(0x0E, 3600 + SOAK), 50);
    assert_eq!(
        c.commit(&running(0x0E, 3600 + SOAK), Some(WORD), 60),
        Ok(VregOp::Set(0x0E))
    );
    // Still the trial: the main loop has not run.
    c.on_status(&running(0x0E, 3600 + SOAK), 61);
    assert!(matches!(c.phase(), Phase::Committing { .. }));
    // Stored but not live.
    let half = with(|s| {
        s.stored_sel = 0x0E;
        s.uptime_s = 3600 + SOAK;
    });
    c.on_status(&half, 62);
    assert!(matches!(c.phase(), Phase::Committing { .. }));
    c.on_status(&half, 60 + VERIFY_TIMEOUT_MS);
    assert_eq!(
        *c.phase(),
        Phase::VerifyFailed {
            set_value: Some(0x0E)
        }
    );
}

#[test]
fn wp_cg11_reset_is_set_zero_and_verifies_against_the_build_default() {
    let mut c = Calibrator::new();
    let raised = with(|s| {
        s.stored_sel = 0x0E;
        s.live_vreg = 0x0E;
    });
    assert_eq!(c.manual_set(&raised, 0, None, 0), Ok(VregOp::Set(0)));
    c.on_status(&raised, 1);
    assert!(matches!(c.phase(), Phase::Committing { .. }));
    c.on_status(&k340(), 2);
    assert_eq!(*c.phase(), Phase::Committed { value: 0 });
}

#[test]
fn wp_cg11_a_hang_is_failed_with_the_trials_own_time_and_the_next_step() {
    let mut c = soaking(0x0D);
    c.on_status(&running(0x0D, 3700), 30);
    c.on_link_lost();
    assert!(c.link_lost());
    assert!(matches!(c.phase(), Phase::Soaking { .. }));
    let hung = with(|s| {
        s.failed_trial_vreg = 0x0D;
        s.failed_trial_uptime_s = 3900;
        s.uptime_s = 9;
    });
    c.on_status(&hung, 40);
    assert!(!c.link_lost());
    let Phase::Failed(f) = *c.phase() else {
        panic!("{:?}", c.phase());
    };
    assert_eq!(f.vreg, 0x0D);
    assert_eq!(f.ran_s, Some(300), "not the 3900 s the device reports");
    assert_eq!(f.uptime_at_hang_s, 3900);
    assert_eq!(f.next_vreg, Some(0x0E));
    assert!(!f.hardware_problem);
    assert!(c.can_start(), "the next step up is the user's to take");
    assert!(c.phase().message().contains("1.25 V"));
}

#[test]
fn wp_cg11_a_hang_at_the_cap_stops_with_hardware_problem() {
    let mut c = soaking(0x0E);
    let hung = with(|s| {
        s.failed_trial_vreg = 0x0E;
        s.failed_trial_uptime_s = 3700;
        s.uptime_s = 5;
    });
    c.on_status(&hung, 40);
    let Phase::Failed(f) = *c.phase() else {
        panic!("{:?}", c.phase());
    };
    assert_eq!((f.next_vreg, f.hardware_problem), (None, true));
    assert!(c.phase().message().contains("hardware problem"));
}

#[test]
fn wp_cg11_a_failure_whose_uptime_was_lost_says_how_long_is_not_known() {
    let mut c = soaking(0x0D);
    let later = with(|s| {
        s.failed_trial_vreg = 0x0D;
        s.failed_trial_uptime_s = 0; // a further warm reset, RAM gone
        s.uptime_s = 5;
    });
    c.on_status(&later, 40);
    let Phase::Failed(f) = *c.phase() else {
        panic!("{:?}", c.phase());
    };
    assert_eq!(f.ran_s, None);
    assert!(!c.phase().message().contains(" s."));
}

#[test]
fn wp_cg11_power_cycle_and_reboot_are_unknown() {
    // Both look the same: the trial gone, no record, uptime back near zero.
    let mut c = soaking(0x0D);
    c.on_status(&with(|s| s.uptime_s = 12), 40);
    assert_eq!(
        *c.phase(),
        Phase::Unknown {
            target: 0x0D,
            reason: UnknownReason::RecordGone
        }
    );
    assert!(c.can_start(), "run again");
}

#[test]
fn wp_cg11_a_trial_gone_while_uptime_went_on_is_unknown_too() {
    let mut c = soaking(0x0D);
    c.on_status(&with(|s| s.uptime_s = 3700), 40);
    assert!(matches!(
        c.phase(),
        Phase::Unknown {
            reason: UnknownReason::RecordGone,
            ..
        }
    ));
}

#[test]
fn wp_cg11_a_hang_after_the_pass_takes_the_pass_back() {
    let mut c = soaking(0x0D);
    c.on_status(&running(0x0D, 3600 + SOAK), 50);
    assert!(c.can_commit());
    let hung = with(|s| {
        s.failed_trial_vreg = 0x0D;
        s.failed_trial_uptime_s = 3600 + SOAK + 60;
        s.uptime_s = 3;
    });
    c.on_status(&hung, 60);
    assert!(matches!(c.phase(), Phase::Failed(_)));
    assert!(!c.can_commit());
    assert!(c.commit(&hung, None, 61).is_err());
}

#[test]
fn wp_cg11_another_voltage_on_trial_is_unknown_and_then_found() {
    let mut c = soaking(0x0D);
    c.on_status(&running(0x0E, 3700), 40);
    assert_eq!(
        *c.phase(),
        Phase::Unknown {
            target: 0x0D,
            reason: UnknownReason::TrialChanged
        }
    );
    c.on_status(&running(0x0E, 3701), 41);
    assert_eq!(*c.phase(), Phase::TrialFound { vreg: 0x0E });
}

#[test]
fn wp_cg11_a_reboot_with_a_new_trial_is_not_the_same_soak() {
    let mut c = soaking(0x0D);
    c.on_status(&running(0x0D, 3700), 40);
    c.on_status(&running(0x0D, 5), 50); // same voltage, but a new run
    assert!(matches!(c.phase(), Phase::Unknown { .. }));
    c.on_status(&running(0x0D, 6), 51);
    assert_eq!(*c.phase(), Phase::TrialFound { vreg: 0x0D });
}

#[test]
fn wp_cg11_a_link_drop_before_the_trial_means_no_trial_on_return() {
    let mut c = Calibrator::new();
    c.open(&k340(), 0).unwrap();
    c.start(&k340(), 0x0D, SOAK, None, 0).unwrap();
    c.on_link_lost();
    assert_eq!(c.on_status(&k340(), 10), None, "no write is replayed");
    assert!(matches!(
        c.phase(),
        Phase::Unknown {
            reason: UnknownReason::NotStarted,
            ..
        }
    ));
}

#[test]
fn wp_cg11_a_failed_write_is_not_retried() {
    let mut c = Calibrator::new();
    c.open(&k340(), 0).unwrap();
    c.start(&k340(), 0x0D, SOAK, None, 0).unwrap();
    assert_eq!(c.on_status(&k340(), 10), Some(ft(0x0D)));
    c.on_write_failed();
    assert_eq!(c.on_status(&k340(), 20), None);
    assert!(matches!(c.phase(), Phase::Unknown { .. }));
}

#[test]
fn wp_cg11_a_found_trial_offers_end_or_keep_and_keep_counts_from_now() {
    let mut c = Calibrator::new();
    assert_eq!(c.on_status(&running(0x0E, 7200), 0), None);
    assert_eq!(*c.phase(), Phase::TrialFound { vreg: 0x0E });
    assert!(c.can_keep());
    assert!(!c.can_commit(), "uptime alone proves nothing");
    assert_eq!(
        c.keep_trial(&running(0x0E, 7200), 1),
        Err(FlowError::SoakTooShort)
    );
    c.keep_trial(&running(0x0E, 7200), SOAK).unwrap();
    c.on_status(&running(0x0E, 7200 + SOAK - 1), 10);
    assert!(matches!(c.phase(), Phase::Soaking { .. }));
    c.on_status(&running(0x0E, 7200 + SOAK), 20);
    assert!(matches!(c.phase(), Phase::Passed { .. }));
    assert!(
        c.commit(&k340(), None, 30).is_err(),
        "the unit shows 1.20 V: a raise"
    );
}

#[test]
fn wp_cg11_a_kept_trial_is_not_failed_by_an_older_record() {
    let old = |up: u32| {
        with(|s| {
            s.trial_active = true;
            s.trial_vreg = 0x0D;
            s.failed_trial_vreg = 0x0D;
            s.failed_trial_uptime_s = 500;
            s.uptime_s = up;
        })
    };
    let mut c = Calibrator::new();
    c.on_status(&old(100), 0);
    c.keep_trial(&old(100), SOAK).unwrap();
    c.on_status(&old(200), 1);
    assert!(matches!(c.phase(), Phase::Soaking { .. }));
    // It hangs in its turn: the code is the same, the reboot gives it away.
    let again = with(|s| {
        s.failed_trial_vreg = 0x0D;
        s.failed_trial_uptime_s = 300;
        s.uptime_s = 2;
    });
    c.on_status(&again, 2);
    assert!(matches!(c.phase(), Phase::Failed(_)));
}

#[test]
fn wp_cg11_a_found_trial_outside_the_range_can_be_kept_but_never_passes() {
    // On a K340: a CLI's 1.15 V, and a CLI's 1.30 V.
    for code in [0x0C, 0x0F] {
        let mut c = Calibrator::new();
        c.on_status(&running(code, 100), 0);
        c.keep_trial(&running(code, 100), SOAK).unwrap();
        assert_eq!(*c.phase(), Phase::TrialLeftRunning { vreg: code });
        c.on_status(&running(code, 100 + 2 * SOAK), 1);
        assert_eq!(*c.phase(), Phase::TrialLeftRunning { vreg: code });
        assert!(c.commit(&running(code, 200), None, 2).is_err());
        c.on_status(&k340(), 3);
        assert_eq!(*c.phase(), Phase::Idle);
    }
}

#[test]
fn wp_cg11_abort_works_in_every_phase_of_a_run() {
    let start = |c: &mut Calibrator| {
        c.open(&k340(), 0).unwrap();
        c.start(&k340(), 0x0D, SOAK, None, 0).unwrap();
    };
    let mut clearing = Calibrator::new();
    start(&mut clearing);
    let mut starting = clearing.clone();
    starting.on_status(&k340(), 1);
    let soak = soaking(0x0D);
    let mut passed = soaking(0x0D);
    passed.on_status(&running(0x0D, 3600 + SOAK), 2);
    for mut c in [clearing, starting, soak, passed] {
        assert_eq!(c.end_trial(10), Ok(VregOp::TrialEnd));
        assert_eq!(c.on_status(&k340(), 11), None);
        assert_eq!(*c.phase(), Phase::Ended);
    }
}

#[test]
fn wp_cg11_an_end_that_never_shows_is_stated() {
    let mut c = soaking(0x0D);
    c.end_trial(10).unwrap();
    c.on_status(&running(0x0D, 3700), 11);
    assert!(matches!(c.phase(), Phase::Ending { .. }));
    c.on_status(&running(0x0D, 3701), 10 + VERIFY_TIMEOUT_MS);
    assert_eq!(*c.phase(), Phase::VerifyFailed { set_value: None });
}

#[test]
fn wp_cg11_manual_ops_keep_the_limits_and_stay_out_of_a_run() {
    let mut c = Calibrator::new();
    let s = k340();
    assert!(c.manual_set(&s, 0x0C, None, 0).is_err());
    assert!(c.manual_set(&s, 0x0F, Some(WORD), 0).is_err());
    assert!(c.manual_set(&s, 0x0E, None, 0).is_err());
    assert!(c.manual_trial(&s, 0x0C, None).is_err());
    assert!(c.manual_trial(&s, 0x0F, Some(WORD)).is_err());
    assert!(c.manual_trial(&s, 0x0E, None).is_err());
    assert_eq!(c.manual_trial(&s, 0x0D, None), Ok(VregOp::Trial(0x0D)));
    assert_eq!(c.manual_clear(), Ok(VregOp::Clear));

    let mut c = soaking(0x0D);
    assert!(c.manual_set(&s, 0x0D, None, 0).is_err());
    assert!(c.manual_trial(&s, 0x0D, None).is_err());
    assert!(c.manual_clear().is_err());
    assert!(c.close().is_err());
    assert!(c.open(&s, 0).is_err());
}

#[test]
fn wp_cg11_a_status_changing_the_default_stops_the_trial() {
    // The device behind the link changed: its default is now above the target.
    let mut c = Calibrator::new();
    let stock = with(|s| {
        s.default_vreg = 0x0C;
        s.live_vreg = 0x0C;
    });
    c.open(&stock, 0).unwrap();
    c.start(&stock, 0x0C, SOAK, None, 0).unwrap();
    assert_eq!(c.on_status(&k340(), 1), None);
    assert!(matches!(c.phase(), Phase::Unknown { .. }));
}

/// An arbitrary status: anything the wire can carry.
fn any_status() -> impl Strategy<Value = CoreVregStatus> {
    (
        any::<[u8; 6]>(),
        any::<bool>(),
        any::<u32>(),
        prop_oneof![Just(0u32), 0u32..20_000, any::<u32>()],
    )
        .prop_map(|(b, trial_active, failed_up, uptime_s)| CoreVregStatus {
            live_vreg: b[0],
            stored_sel: b[1],
            default_vreg: b[2],
            flags: b[3],
            hang_count: b[4],
            trial_active,
            trial_vreg: b[5],
            failed_trial_vreg: b[5].wrapping_add(failed_up as u8) & 0x1F,
            failed_trial_uptime_s: failed_up,
            uptime_s,
        })
}

/// A status like a device's: real codes, so runs get somewhere.
fn real_status() -> impl Strategy<Value = CoreVregStatus> {
    (
        0x0Cu8..=0x0F,
        prop_oneof![Just(0u8), 0x0Cu8..=0x0F],
        prop_oneof![Just(0u8), 0x0Cu8..=0x0F],
        any::<bool>(),
        prop_oneof![Just(0u8), 0x0Cu8..=0x0F],
        0u32..20_000,
    )
        .prop_map(
            |(default_vreg, stored_sel, trial, active, failed, up)| CoreVregStatus {
                live_vreg: if active {
                    trial
                } else {
                    resolve(stored_sel, default_vreg)
                },
                stored_sel,
                default_vreg,
                flags: 0,
                hang_count: 0,
                trial_active: active,
                trial_vreg: trial,
                failed_trial_vreg: failed,
                failed_trial_uptime_s: up / 2,
                uptime_s: up,
            },
        )
}

#[derive(Debug, Clone)]
enum Action {
    Status(CoreVregStatus),
    Open,
    Start(u8, u32, bool),
    Commit(bool),
    End,
    Keep(u32),
    ManualSet(u8, bool),
    ManualTrial(u8, bool),
    ManualClear,
    LinkLost,
    WriteFailed,
    Close,
}

fn any_action() -> impl Strategy<Value = Action> {
    prop_oneof![
        4 => prop_oneof![real_status(), any_status()].prop_map(Action::Status),
        1 => Just(Action::Open),
        2 => (any::<u8>(), prop_oneof![Just(SOAK), any::<u32>()], any::<bool>())
            .prop_map(|(t, s, ty)| Action::Start(t, s, ty)),
        1 => any::<bool>().prop_map(Action::Commit),
        1 => Just(Action::End),
        1 => prop_oneof![Just(SOAK), any::<u32>()].prop_map(Action::Keep),
        1 => (any::<u8>(), any::<bool>()).prop_map(|(v, t)| Action::ManualSet(v, t)),
        1 => (any::<u8>(), any::<bool>()).prop_map(|(v, t)| Action::ManualTrial(v, t)),
        1 => Just(Action::ManualClear),
        1 => Just(Action::LinkLost),
        1 => Just(Action::WriteFailed),
        1 => Just(Action::Close),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// No status panics, in any phase, and a poll's only request stays in
    /// range for the status it was decided on, as the type already limits it
    /// to a trial.
    #[test]
    fn proptest_wp_cg11_no_sequence_panics_or_leaves_the_limits(
        actions in proptest::collection::vec(any_action(), 0..60),
    ) {
        let mut c = Calibrator::new();
        let mut s = k340();
        let mut now = 0u64;
        for action in actions {
            now += 700;
            let word = |on: bool| on.then_some(WORD);
            let mut typed = false;
            let sent: Option<VregOp> = match action {
                Action::Status(next) => {
                    s = next;
                    let _ = c.phase().message();
                    let _ = c.soak_elapsed_s(&s);
                    let _ = TrialFailure::on_record(&s).map(|_| ());
                    let t = c.on_status(&s, now);
                    typed = t.is_some_and(|t| t.word().is_some());
                    t.map(FlowTrial::op)
                }
                Action::Open => { let _ = c.open(&s, now); None }
                Action::Start(t, soak, ty) => {
                    let r = c.start(&s, t, soak, word(ty), now).ok();
                    if r.is_some() {
                        prop_assert!(in_flow_range(t, s.default_vreg));
                        prop_assert!(soak >= MIN_SOAK_S);
                        prop_assert!(ty || !needs_word(t, &s), "{t:#x} on {s:?}");
                    }
                    r
                }
                Action::Commit(ty) => { typed = ty; c.commit(&s, word(ty), now).ok() }
                Action::End => c.end_trial(now).ok(),
                Action::Keep(soak) => { let _ = c.keep_trial(&s, soak); None }
                Action::ManualSet(v, ty) => { typed = ty; c.manual_set(&s, v, word(ty), now).ok() }
                Action::ManualTrial(v, ty) => { typed = ty; c.manual_trial(&s, v, word(ty)).ok() }
                Action::ManualClear => c.manual_clear().ok(),
                Action::LinkLost => { c.on_link_lost(); None }
                Action::WriteFailed => { c.on_write_failed(); None }
                Action::Close => { let _ = c.close(); None }
            };
            // Nothing that raises the unit above 1.20 V goes without the word.
            if let Some(op) = sent {
                prop_assert!(typed || !overvolt::op_needs_word(op, Some(&s)), "{op:?} on {s:?}");
            }
            match sent {
                Some(VregOp::Trial(v)) => {
                    prop_assert!(in_flow_range(v, s.default_vreg), "trial {v:#x} on {s:?}");
                    prop_assert!(v <= VREG_1_25);
                    prop_assert!(v >= s.default_vreg);
                }
                Some(VregOp::Set(v)) => {
                    prop_assert!(
                        v == 0 || in_flow_range(v, s.default_vreg),
                        "set {v:#x} on {s:?}"
                    );
                }
                _ => {}
            }
        }
    }

    /// A SET never follows from statuses alone, whatever they say: only a
    /// trial can, and only once per `start`.
    #[test]
    fn proptest_wp_cg11_polling_alone_never_leads_to_a_write(
        statuses in proptest::collection::vec(prop_oneof![real_status(), any_status()], 0..40),
    ) {
        let mut c = Calibrator::new();
        for (i, s) in statuses.iter().enumerate() {
            prop_assert_eq!(c.on_status(s, i as u64 * 700), None);
        }
    }

    /// A run sends at most one TRIAL however the statuses go.
    #[test]
    fn proptest_wp_cg11_one_start_is_at_most_one_trial(
        statuses in proptest::collection::vec(prop_oneof![real_status(), any_status()], 0..40),
    ) {
        let mut c = Calibrator::new();
        c.open(&k340(), 0).unwrap();
        c.start(&k340(), 0x0D, SOAK, None, 0).unwrap();
        let mut trials = 0;
        for (i, s) in statuses.iter().enumerate() {
            trials += c.on_status(s, i as u64 * 700).iter().count();
        }
        prop_assert!(trials <= 1);
    }
}
