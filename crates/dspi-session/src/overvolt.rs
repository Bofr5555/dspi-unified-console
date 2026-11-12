//! The confirmation word for raising the core voltage (WP CG14). The owner's
//! rule, in the one place every path asks: the calibration flow, the core's
//! and C ABI's raw request, the Terminal's `uconsole ext vreg` and the firmware
//! guard.
//!
//! **The word is required exactly when the target is above 1200 mV and above
//! the unit's current voltage.** Current is read from the unit at that moment
//! and never remembered: no stored or per-serial record exists anywhere. A
//! unit whose current voltage cannot be read requires the word for any target
//! above 1200 mV. Lowering, or staying level, needs no word from this rule.
//!
//! **A `SET` leaves a running trial's voltage out of "current".** A trial lives
//! in RAM, so persisting it is an upward change in itself, even after the word
//! was typed for the trial. A `TRIAL` counts what the unit is running: a higher
//! running trial means no raise.
//!
//! It is a typed confirmation, not a lock: the firmware enforces only its own
//! limits, and a client talking to the device some other way is not stopped.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::ext::{CoreVregStatus, VregOp, vreg_mv};
use dspi_proto::generated::ext as g;
use dspi_proto::generated::opcodes as op;

/// What must be typed, exactly: case and spaces count. Not localised.
pub const WORD: &str = "OVERVOLT";

/// A target at or below this never needs the word, mV.
pub const WORD_ABOVE_MV: u16 = 1200;

/// The rule. `current_mv` is `None` when the unit's voltage cannot be read.
pub fn word_required(current_mv: Option<u16>, target_mv: u16) -> bool {
    target_mv > WORD_ABOVE_MV && current_mv.is_none_or(|current| target_mv > current)
}

/// Whether `typed` is the word, exactly.
pub fn word_matches(typed: Option<&str>) -> bool {
    typed == Some(WORD)
}

/// The unit's current voltage: the highest of live, stored and build default
/// as `EXT_CORE_VREG` reports them. `None`, meaning unreadable, if any of them
/// is not a voltage (a stored selection of 0 is the build default). What a
/// `TRIAL` and the firmware guard compare with.
pub fn current_mv(s: &CoreVregStatus) -> Option<u16> {
    let stored = if s.stored_sel == 0 {
        s.default_vreg
    } else {
        s.stored_sel
    };
    [s.live_vreg, stored, s.default_vreg]
        .into_iter()
        .map(vreg_mv)
        .collect::<Option<Vec<u16>>>()?
        .into_iter()
        .max()
}

/// What a `SET` compares with: [`current_mv`], but while a trial runs, without
/// its live voltage. What the unit would come back to, the persisted voltage,
/// is what a `SET` changes. Unreadable under the same conditions.
pub fn persisted_mv(s: &CoreVregStatus) -> Option<u16> {
    let current = current_mv(s)?;
    if !s.trial_active {
        return Some(current);
    }
    let stored = if s.stored_sel == 0 {
        s.default_vreg
    } else {
        s.stored_sel
    };
    [stored, s.default_vreg]
        .into_iter()
        .filter_map(vreg_mv)
        .max()
}

/// The rule for a request, on the status just read (`None` if it could not be
/// read). Only `SET` and `TRIAL` can raise the voltage now: `TRIAL_END` returns
/// to the stored voltage, which current already counts. `SET 0` is the build
/// default, which current counts too; on an unreadable unit nobody can say what
/// that is, so it needs the word. So does a code that is not a voltage.
pub fn op_needs_word(request: VregOp, status: Option<&CoreVregStatus>) -> bool {
    let judge = |code: u8, current: Option<u16>| {
        vreg_mv(code).is_none_or(|target| word_required(current, target))
    };
    match request {
        VregOp::Set(0) => status.and_then(current_mv).is_none(),
        VregOp::Set(code) => judge(code, status.and_then(persisted_mv)),
        VregOp::Trial(code) => judge(code, status.and_then(current_mv)),
        VregOp::TrialEnd | VregOp::Auto(_) | VregOp::Clear => false,
    }
}

/// Whether a raw vendor write would be an `EXT_CORE_VREG` request. Raw paths
/// refuse these, since they could not carry the word: use the typed request.
pub fn is_core_vreg_write(opcode: u8, value: u16) -> bool {
    opcode == op::REQ_EXTENSION && value == g::EXT_CORE_VREG
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg14 {
    use super::*;
    use proptest::prelude::*;

    fn unit(live: u8, stored: u8, default_vreg: u8) -> CoreVregStatus {
        CoreVregStatus {
            live_vreg: live,
            stored_sel: stored,
            default_vreg,
            flags: 0,
            hang_count: 0,
            trial_active: false,
            trial_vreg: 0,
            failed_trial_vreg: 0,
            failed_trial_uptime_s: 0,
            uptime_s: 0,
        }
    }

    #[test]
    fn wp_cg14_the_owners_cases() {
        for (current, target, needs) in [
            (1250, 1300, true), // the owner's example
            (1150, 1200, false),
            (1200, 1250, true),
            (1150, 1250, true),
            (1250, 1250, false),
            (1300, 1250, false),
            (1300, 1300, false),
            (1200, 1200, false),
            (1200, 1150, false),
        ] {
            assert_eq!(
                word_required(Some(current), target),
                needs,
                "{current} to {target}"
            );
        }
    }

    #[test]
    fn wp_cg14_unreadable_needs_it_only_above_1200() {
        assert!(word_required(None, 1250));
        assert!(word_required(None, 1300));
        assert!(!word_required(None, 1200));
        assert!(!word_required(None, 1150));
    }

    #[test]
    fn wp_cg14_only_the_exact_word_matches() {
        assert!(word_matches(Some("OVERVOLT")));
        for bad in ["overvolt", "Overvolt", "OVERVOLT ", " OVERVOLT", "", "yes"] {
            assert!(!word_matches(Some(bad)), "{bad:?}");
        }
        assert!(!word_matches(None));
    }

    #[test]
    fn wp_cg14_current_is_the_highest_of_live_stored_and_default() {
        assert_eq!(current_mv(&unit(0x0C, 0, 0x0C)), Some(1150));
        assert_eq!(current_mv(&unit(0x0D, 0, 0x0C)), Some(1200)); // a trial
        assert_eq!(current_mv(&unit(0x0C, 0x0E, 0x0C)), Some(1250));
        assert_eq!(current_mv(&unit(0x0D, 0x0C, 0x0D)), Some(1200));
        assert_eq!(current_mv(&unit(0x0C, 0, 0x0F)), Some(1300));
        // A field that is not a voltage: nothing can be believed.
        assert_eq!(current_mv(&unit(0xFF, 0, 0x0D)), None);
        assert_eq!(current_mv(&unit(0x0D, 0x40, 0x0D)), None);
        assert_eq!(current_mv(&unit(0x0D, 0, 0)), None);
    }

    #[test]
    fn wp_cg14_requests() {
        let k340 = unit(0x0D, 0, 0x0D);
        assert!(op_needs_word(VregOp::Trial(0x0E), Some(&k340)));
        assert!(op_needs_word(VregOp::Set(0x0E), Some(&k340)));
        assert!(!op_needs_word(VregOp::Set(0x0D), Some(&k340)));
        assert!(!op_needs_word(VregOp::Set(0), Some(&k340)));
        // Unreadable: SET 0 has no known target.
        assert!(op_needs_word(VregOp::Set(0), None));
        assert!(!op_needs_word(VregOp::Set(0x0D), None));
        assert!(op_needs_word(VregOp::Set(0x0E), None));
        // Not a voltage.
        assert!(op_needs_word(VregOp::Trial(0x10), Some(&k340)));
        for other in [VregOp::TrialEnd, VregOp::Auto(true), VregOp::Clear] {
            assert!(!op_needs_word(other, None));
        }
    }

    #[test]
    fn wp_cg14_a_set_leaves_a_running_trial_out_and_a_trial_does_not() {
        let on_trial = |code: u8, stored: u8, default_vreg: u8| CoreVregStatus {
            trial_active: true,
            trial_vreg: code,
            ..unit(code, stored, default_vreg)
        };
        // On trial at 1.25 V, nothing stored: persisting it is a raise.
        let s = on_trial(0x0E, 0, 0x0D);
        assert_eq!(current_mv(&s), Some(1250));
        assert_eq!(persisted_mv(&s), Some(1200));
        assert!(op_needs_word(VregOp::Set(0x0E), Some(&s)));
        // A trial at or below what is running is no raise.
        assert!(!op_needs_word(VregOp::Trial(0x0E), Some(&s)));
        assert!(!op_needs_word(VregOp::Trial(0x0D), Some(&s)));
        // Already stored at 1.25 V: level.
        assert!(!op_needs_word(
            VregOp::Set(0x0E),
            Some(&on_trial(0x0E, 0x0E, 0x0D))
        ));
        // Above what is stored, whatever trial is running.
        assert!(op_needs_word(
            VregOp::Set(0x0F),
            Some(&on_trial(0x0F, 0x0E, 0x0D))
        ));
        // A trial at or below 1.20 V: the rule does not reach it.
        assert!(!op_needs_word(
            VregOp::Set(0x0D),
            Some(&on_trial(0x0D, 0, 0x0C))
        ));
        // No trial: the live voltage still counts.
        assert!(!op_needs_word(
            VregOp::Set(0x0E),
            Some(&unit(0x0E, 0, 0x0D))
        ));
        // A bad live voltage is unreadable, trial or not.
        assert_eq!(persisted_mv(&on_trial(0xFF, 0, 0x0D)), None);
    }

    proptest! {
        /// The rule as stated, for every pair.
        #[test]
        fn proptest_wp_cg14_the_rule_is_exactly_what_it_says(
            current in proptest::option::of(any::<u16>()),
            target in any::<u16>(),
        ) {
            let needs = word_required(current, target);
            prop_assert_eq!(
                needs,
                target > 1200 && current.is_none_or(|c| target > c)
            );
            if target <= 1200 {
                prop_assert!(!needs);
            }
        }

        /// Whatever a status says, nothing panics.
        #[test]
        fn proptest_wp_cg14_any_status_is_survived(
            b in any::<[u8; 16]>(),
            code in any::<u8>(),
        ) {
            let s = CoreVregStatus::decode(&b);
            prop_assert!(s.is_some());
            let _ = op_needs_word(VregOp::Set(code), s.as_ref());
            let _ = op_needs_word(VregOp::Trial(code), s.as_ref());
        }
    }
}
