//! WP INTI (owner 2026-11-23, FUNC2b/c question 2): the person's own preset
//! load gets the guard an agent's has (WP FFIX, in `dspi-mcp`).
//!
//! A preset load runs the stored preset on the device before anything can
//! look at it. With a confirmed tweeter or midrange in the user's speaker
//! setup and the master volume independent of presets, [`DspiCore::load_preset`]
//! first lowers the master volume to [`LOAD_GUARD_DB`] (silent), loads, lets
//! the limits profile hold the result (as every load), then judges each
//! confirmed tweeter and midrange against its floor (the tweeter rule, raised
//! by the profile). One left playing without its high-pass is held: muted.
//! Then the volume comes back, unless something set it meanwhile. What was
//! held, and a volume that could not come back, is the load's notice
//! ([`DspiCore::preset_load_notice`]) for the app to show.
//!
//! An agent's load is silenced by the MCP server before it calls the core,
//! and judged there: with the master already silent this guard stays out.
//! With the master volume saved in each preset the load sets the volume
//! itself and nothing can be guarded (the person's load goes as before; an
//! agent's is refused, WP ODEC).

use crate::{DspState, DspiCore};

/// The master volume a guarded load runs at, dB (the core's floor: silent).
pub(crate) const LOAD_GUARD_DB: f32 = -127.0;

/// A tolerance on a corner, Hz (as the measurement preflight's).
const HZ_SLACK: f32 = 0.5;

/// What [`DspiCore::load_guard_begin`] lowered and what it judges after.
pub(crate) struct LoadGuard {
    /// The master volume to bring back, dB.
    level: f32,
    /// Each confirmed tweeter or midrange: (output, floor Hz, slope dB/octave).
    rules: Vec<(u8, f32, u8)>,
}

/// Whether output `o` plays programme: on, unmuted and routed from an input.
fn plays(st: &DspState, o: u8) -> bool {
    let at = usize::from(o);
    st.output_enabled.get(at).copied().unwrap_or(false)
        && !st.output_muted.get(at).copied().unwrap_or(false)
        && st
            .matrix_routing
            .iter()
            .take(usize::from(st.num_input_channels))
            .any(|row| row.get(at).copied().unwrap_or(false))
}

/// Whether output `o` has an active high-pass at `floor` or above, `slope`
/// dB/octave or steeper, in its own EQ or crossover.
fn protected(st: &DspState, o: u8, floor: f32, slope: u8) -> bool {
    let ch = usize::from(st.num_input_channels.saturating_add(o));
    st.filters
        .get(ch)
        .into_iter()
        .flatten()
        .chain(st.xover.get(ch).into_iter().flatten())
        .any(|b| {
            !b.bypass
                && b.freq.is_finite()
                && b.freq >= floor - HZ_SLACK
                && dspi_session::limits::highpass_slope(b.filter_type).is_some_and(|s| s >= slope)
        })
}

/// -- INT016 -- Hold in `st` what [`DspiCore::load_guard_end`] holds after a
/// load: each of `rules`' outputs that plays without its high-pass is muted.
/// The outputs held, from 0. For a state about to be put on the device
/// whole (the A/B compare's "as saved", ab.rs), so it plays what a load of
/// that preset would.
pub(crate) fn hold_unprotected(st: &mut DspState, rules: &[(u8, f32, u8)]) -> Vec<u8> {
    let mut held = Vec::new();
    for &(o, floor, slope) in rules {
        if !plays(st, o) || protected(st, o, floor, slope) {
            continue;
        }
        if let Some(m) = st.output_muted.get_mut(usize::from(o)) {
            *m = true;
            held.push(o);
        }
    }
    held
}

impl DspiCore {
    /// The guard's rules now: each trusted tweeter or midrange of the speaker
    /// layout as (output, floor Hz, slope dB/octave); none without a trusted
    /// layout. -- INT016 -- (load_guard_begin's, for the A/B compare too)
    pub(crate) fn load_guard_rules(&self) -> Vec<(u8, f32, u8)> {
        let st = self.get_state();
        // -- WP ADET0 -- a detected layout's roles guard the load too
        let Some(layout) = self.trusted_layout_now() else {
            return Vec::new();
        };
        let profile = crate::limits::profile(&mut self.lock());
        layout
            .outputs
            .iter()
            .filter(|o| o.output < st.num_output_channels)
            .filter_map(|o| {
                crate::measure::tweeter_rule(o.role, o.output, profile.as_ref())
                    .map(|(hz, slope)| (o.output, hz, slope))
            })
            .collect()
    }

    /// Silence the device for a load, when it needs guarding (module docs).
    pub(crate) fn load_guard_begin(&self) -> Option<LoadGuard> {
        let st = self.get_state();
        if st.master_volume_mode != 0 || st.master_volume_db <= LOAD_GUARD_DB + 0.5 {
            return None;
        }
        let rules = self.load_guard_rules();
        if rules.is_empty() {
            return None;
        }
        self.set_master_volume(LOAD_GUARD_DB).ok()?;
        Some(LoadGuard {
            level: st.master_volume_db,
            rules,
        })
    }

    /// After the load (`loaded`: it went through): hold what plays without
    /// its high-pass, bring the volume back, and keep what the person must
    /// know for [`DspiCore::preset_load_notice`].
    pub(crate) fn load_guard_end(&self, guard: LoadGuard, slot: u8, loaded: bool) {
        let mut said = Vec::new();
        let mut keep_silent = false;
        if loaded {
            let st = self.get_state();
            for &(o, floor, slope) in &guard.rules {
                if !plays(&st, o) || protected(&st, o, floor, slope) {
                    continue;
                }
                let out = u16::from(o).saturating_add(1);
                if self.set_output_mute(o, true).is_ok() {
                    said.push(format!(
                        "Out {out} is muted: your speaker setup has a tweeter or midrange there, and \
                         preset {} gives it no high-pass at {floor:.0} Hz or above ({slope} dB/octave). \
                         Give it its high-pass, then unmute it.",
                        u16::from(slot).saturating_add(1)
                    ));
                } else {
                    keep_silent = true;
                    said.push(format!(
                        "Out {out} plays without its high-pass at {floor:.0} Hz and could not be \
                         muted, so the master volume stays down. Give it its high-pass, then set \
                         the master volume again."
                    ));
                }
            }
        }
        // The volume back, unless something (the person, a recall) set it meanwhile
        if !keep_silent && self.get_state().master_volume_db <= LOAD_GUARD_DB + 0.5 {
            #[cfg(test)]
            let back = if tests::RESTORE_FAILS.with(std::cell::Cell::get) {
                Err(crate::DspiError::Disconnected)
            } else {
                self.set_master_volume(guard.level)
            };
            #[cfg(not(test))]
            let back = self.set_master_volume(guard.level);
            if back.is_err() {
                said.push(format!(
                    "The master volume was lowered to {LOAD_GUARD_DB:.0} dB for the preset load \
                     and could not be brought back to {:.1} dB. Set the master volume again.",
                    guard.level
                ));
            }
        }
        self.lock().preset_load_notice = said.join(" ");
    }

    /// What the last preset load held or could not put back, for the app to
    /// show the person (WP INTI); "" for nothing. Read once: it is cleared.
    pub fn preset_load_notice(&self) -> String {
        std::mem::take(&mut self.lock().preset_load_notice)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use crate::DspiCore;
    use crate::FilterParams;
    use crate::measure::MeasureRole;
    use crate::speaker_layout::{SpeakerLayout, SpeakerLayoutOutput};

    thread_local! {
        /// A test's injected failure of the volume's return.
        pub(super) static RESTORE_FAILS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    const LR4_HP: u8 = 35;

    fn hp(hz: f32) -> FilterParams {
        FilterParams {
            filter_type: LR4_HP,
            bypass: false,
            freq: hz,
            q: 0.707,
            gain: 0.0,
            qp: 0.707,
        }
    }

    /// A demo with output 1 a confirmed tweeter, routed from input 0; slot 9
    /// (8) holds it at 850 Hz, slot 2 (1) at 3 kHz and is active.
    fn demo(confirmed: bool) -> std::sync::Arc<DspiCore> {
        let core = DspiCore::new();
        core.open_demo().unwrap();
        let n_in = core.get_state().num_input_channels;
        let tweeter = n_in + 1;
        core.set_matrix_route(0, 1, true, 0.0, false).unwrap();
        core.set_crossover(tweeter, 0, hp(850.0)).unwrap();
        let token = core.confirm_token(crate::ConfirmOp::SavePreset);
        core.save_preset_confirmed(8, Some(token)).unwrap();
        core.set_crossover(tweeter, 0, hp(3_000.0)).unwrap();
        let token = core.confirm_token(crate::ConfirmOp::SavePreset);
        core.save_preset_confirmed(1, Some(token)).unwrap();
        core.set_master_volume(-12.0).unwrap();
        let outputs = (0..core.get_state().num_output_channels)
            .map(|n| SpeakerLayoutOutput {
                output: n,
                role: if n == 1 {
                    MeasureRole::High
                } else {
                    MeasureRole::Full
                },
                speaker: "L".into(),
                group: 0,
                source: Default::default(), // -- WP ADET0 --
            })
            .collect();
        core.speaker_layout_set(SpeakerLayout {
            outputs,
            confirmed_by_user: confirmed,
            ..SpeakerLayout::default()
        })
        .unwrap();
        core
    }

    fn master_writes(core: &DspiCore) -> Vec<f32> {
        core.history(50)
            .iter()
            .filter(|e| e.path == "vol.master")
            .filter_map(|e| match e.after {
                crate::ParamValue::Float(f) => Some(f),
                _ => None,
            })
            .collect()
    }

    /// The person's load of a preset that leaves the confirmed tweeter
    /// without its high-pass: it ran silenced, the tweeter is held (muted),
    /// the volume is back, and the notice says so once.
    #[test]
    fn wp_inti_the_users_load_is_silenced_and_an_unprotected_tweeter_held() {
        let core = demo(true);
        core.load_preset(8).unwrap();
        let st = core.get_state();
        assert_eq!(st.active_preset_slot, 8);
        assert!(st.output_muted[1], "the tweeter plays unprotected");
        assert!(
            (st.master_volume_db - -12.0).abs() < 0.01,
            "{}",
            st.master_volume_db
        );
        assert!(
            master_writes(&core).contains(&super::LOAD_GUARD_DB),
            "not silenced first: {:?}",
            master_writes(&core)
        );
        let n = core.preset_load_notice();
        assert!(
            n.contains("Out 2 is muted") && n.contains("preset 9"),
            "{n}"
        );
        assert_eq!(core.preset_load_notice(), "", "read once");
        // A protected preset: silenced too, nothing held, nothing to say.
        core.set_output_mute(1, false).unwrap();
        core.load_preset(1).unwrap();
        let st = core.get_state();
        assert!(!st.output_muted[1]);
        assert!((st.master_volume_db - -12.0).abs() < 0.01);
        assert_eq!(core.preset_load_notice(), "");
    }

    /// Nothing to guard: no confirmed setup, or the volume saved with presets.
    #[test]
    fn wp_inti_no_guard_without_a_confirmed_tweeter_or_with_the_volume_in_presets() {
        let core = demo(false);
        core.load_preset(8).unwrap();
        assert!(!core.get_state().output_muted[1]);
        assert!(!master_writes(&core).contains(&super::LOAD_GUARD_DB));
        let core = demo(true);
        core.set_master_volume_mode(1).unwrap();
        core.load_preset(8).unwrap();
        assert!(!master_writes(&core).contains(&super::LOAD_GUARD_DB));
        assert_eq!(core.preset_load_notice(), "");
    }

    /// A volume that cannot come back is the notice.
    #[test]
    fn wp_inti_a_volume_that_cannot_come_back_is_said() {
        let core = demo(true);
        RESTORE_FAILS.with(|f| f.set(true));
        core.load_preset(1).unwrap();
        RESTORE_FAILS.with(|f| f.set(false));
        let n = core.preset_load_notice();
        assert!(n.contains("could not be brought back to -12.0 dB"), "{n}");
        assert!(core.get_state().master_volume_db <= super::LOAD_GUARD_DB + 0.5);
    }
}
