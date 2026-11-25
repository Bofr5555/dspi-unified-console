//! -- WP SHOWFIX -- (findings 4, 5, 6, 13) The names a person knows a
//! device's channels by, one set for every text that names a channel: History,
//! the agents' activity, Auto-tune's layout, the Equaliser's list.
//!
//! - An input: the name stored on the device ("C", "LFE"), else "In N".
//! - An output: its speaker and role from the speaker layout when that row
//!   is someone's word, confirmed, detected or stated from the user's words
//!   ("Centre woofer", "Left tweeter", "Sub"), not a suggestion; else the name
//!   the user gave it; else, when nothing is routed to it, the core's label
//!   "3 · None" (-- WP INTE -- owner 2026-11-24, as every edit view names it);
//!   else "Out N".
//!
//! Numbers count from 1, as the screens do.

use serde::Serialize;

use crate::DspiCore;
use crate::measure::MeasureRole;
use crate::speaker_layout::{LayoutState, RoleSource, SpeakerLayout};

/// Every channel's name ([`DspiCore::channel_words`]).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelWords {
    /// The device's inputs, in order.
    pub inputs: Vec<String>,
    /// The device's outputs, in order.
    pub outputs: Vec<String>,
    /// Outputs whose name comes from the speaker layout (0-based).
    pub from_layout: Vec<u8>,
}

impl ChannelWords {
    /// Input `i` (0-based).
    pub fn input(&self, i: usize) -> String {
        self.inputs
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("In {}", i.saturating_add(1)))
    }

    /// Output `o` (0-based).
    pub fn output(&self, o: usize) -> String {
        self.outputs
            .get(o)
            .cloned()
            .unwrap_or_else(|| format!("Out {}", o.saturating_add(1)))
    }

    /// Wire channel `ch`: the inputs first, then the outputs.
    pub fn wire(&self, ch: usize) -> String {
        match ch.checked_sub(self.inputs.len()) {
            Some(o) if !self.inputs.is_empty() => self.output(o),
            _ => self.input(ch),
        }
    }
}

/// A speaker's name and the role its output drives, as the Overview words it:
/// "Centre woofer", "Left tweeter", "Sub". A one-letter side reads as a word.
pub fn speaker_role_name(speaker: &str, role: MeasureRole) -> Option<String> {
    let speaker = match speaker.trim() {
        "" => return None,
        "L" | "l" => "Left",
        "R" | "r" => "Right",
        "C" | "c" => "Centre",
        other => other,
    };
    Some(match role {
        MeasureRole::Low => format!("{speaker} woofer"),
        MeasureRole::Mid => format!("{speaker} mid"),
        MeasureRole::High => format!("{speaker} tweeter"),
        MeasureRole::Sub if speaker.to_lowercase().starts_with("sub") => speaker.to_owned(),
        MeasureRole::Sub => format!("{speaker} sub"),
        MeasureRole::Full => speaker.to_owned(),
    })
}

/// The names from the parts: the device's channel names (wire order), its
/// input count, the outputs the user named, the unused ones and the layout.
pub(crate) fn words_of(
    names: &[String],
    inputs: u8,
    outputs: u8,
    user_named: &[bool],
    unused: &[bool],
    layout: Option<&SpeakerLayout>,
) -> ChannelWords {
    let stored = |ch: usize| {
        names
            .get(ch)
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty())
    };
    let inputs_n = usize::from(inputs);
    let input_names = (0..inputs_n)
        .map(|i| stored(i).unwrap_or_else(|| format!("In {}", i.saturating_add(1))))
        .collect();
    // A row is someone's word when the layout is confirmed, or the row was
    // detected, stated from the user's words or set by the user.
    let confirmed = layout.is_some_and(|l| l.state() == LayoutState::Confirmed);
    let mut from_layout = Vec::new();
    let output_names = (0..outputs)
        .map(|o| {
            let row = layout
                .and_then(|l| l.outputs.iter().find(|r| r.output == o))
                .filter(|r| confirmed || r.source != RoleSource::Suggested);
            if let Some(name) = row.and_then(|r| speaker_role_name(&r.speaker, r.role)) {
                from_layout.push(o);
                return name;
            }
            let ch = inputs_n.saturating_add(usize::from(o));
            if user_named.get(usize::from(o)).copied().unwrap_or(false)
                && let Some(name) = stored(ch)
            {
                return name;
            }
            // -- WP INTE -- an unused output by the core's label (output_label.rs)
            if unused.get(usize::from(o)).copied().unwrap_or(false) {
                return format!("{} · None", u16::from(o).saturating_add(1));
            }
            format!("Out {}", u16::from(o).saturating_add(1))
        })
        .collect();
    ChannelWords {
        inputs: input_names,
        outputs: output_names,
        from_layout,
    }
}

impl DspiCore {
    /// The names a person knows this device's channels by (module doc), for
    /// any text that names a channel. Reads the mirror and the speaker
    /// layout's file; never fails (no device: no channels).
    pub fn channel_words(&self) -> ChannelWords {
        let state = self.get_state();
        let labels = self.output_labels();
        let named: Vec<bool> = (0..state.num_output_channels)
            .map(|o| labels.iter().any(|l| l.output == o && l.user_named))
            .collect();
        let unused: Vec<bool> = (0..state.num_output_channels)
            .map(|o| labels.iter().any(|l| l.output == o && l.unused))
            .collect();
        let layout = self.speaker_layout().ok().flatten();
        words_of(
            &state.channel_names,
            state.num_input_channels,
            state.num_output_channels,
            &named,
            &unused,
            layout.as_ref(),
        )
    }
}

#[cfg(test)]
mod tests_wp_showfix {
    use super::*;
    use crate::speaker_layout::SpeakerLayoutOutput;

    fn row(
        output: u8,
        role: MeasureRole,
        speaker: &str,
        source: RoleSource,
    ) -> SpeakerLayoutOutput {
        SpeakerLayoutOutput {
            output,
            role,
            speaker: speaker.into(),
            group: 0,
            source,
        }
    }

    #[test]
    fn wp_showfix_channels_are_named_as_the_person_knows_them() {
        let names: Vec<String> = ["L", "R", "C", "LFE", "", "", "", "", "", "Bass", "", ""]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let layout = SpeakerLayout {
            outputs: vec![
                row(0, MeasureRole::Low, "Centre", RoleSource::Stated),
                row(1, MeasureRole::High, "C", RoleSource::Stated),
                row(2, MeasureRole::Sub, "Sub", RoleSource::Stated),
            ],
            ..SpeakerLayout::default()
        };
        let w = words_of(
            &names,
            8,
            4,
            &[false, true, false, true],
            &[],
            Some(&layout),
        );
        assert_eq!(w.input(2), "C");
        assert_eq!(w.input(4), "In 5");
        assert_eq!(w.output(0), "Centre woofer");
        assert_eq!(w.output(1), "Centre tweeter");
        assert_eq!(w.output(2), "Sub");
        // Not in the layout: the user's name, else the number.
        assert_eq!(w.wire(11), "Out 4");
        assert_eq!(w.from_layout, vec![0, 1, 2]);
        // A suggestion is nobody's word: no layout names.
        let suggested = SpeakerLayout {
            outputs: vec![row(0, MeasureRole::Low, "L", RoleSource::Suggested)],
            ..SpeakerLayout::default()
        };
        let w = words_of(&names, 8, 2, &[false, true], &[], Some(&suggested));
        assert_eq!(w.output(0), "Out 1");
        assert_eq!(w.output(1), "Bass");
        assert_eq!(
            speaker_role_name("L", MeasureRole::Low).unwrap(),
            "Left woofer"
        );
        assert_eq!(w.wire(3), "LFE");
    }

    // -- WP INTE -- (owner 2026-11-24) an output nothing is routed to reads as the
    // core's label, "3 · None"; a layout row or the user's name still wins.
    #[test]
    fn wp_inte_an_unused_output_is_named_as_its_label() {
        let names: Vec<String> = ["L", "R", "", "", "Bass", ""]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let layout = SpeakerLayout {
            outputs: vec![row(0, MeasureRole::Low, "L", RoleSource::Stated)],
            ..SpeakerLayout::default()
        };
        let unused = [true, false, true, true];
        let w = words_of(
            &names,
            2,
            4,
            &[false, false, true, false],
            &unused,
            Some(&layout),
        );
        assert_eq!(w.output(0), "Left woofer");
        assert_eq!(w.output(1), "Out 2");
        assert_eq!(w.output(2), "Bass");
        assert_eq!(w.output(3), "4 · None");
    }
}
