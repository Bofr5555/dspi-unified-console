//! Output labels and dashboard cards for every client (BUG1, owner decision
//! 2026-11-09): `dspi_session::output_label` over the core's mirror.
//!
//! While an output has its firmware default name (the DAC slot, "I2S 1 L"),
//! it is labelled from the matrix and its filters ("Out 1 · R · Low"); a name
//! the user set is shown as it is. Clients call these when the routing, the
//! filters or the names change, and keep the result: never per frame.
//!
//! [`DspiCore::output_labels`] and [`DspiCore::output_cards`] read the mirror
//! (Android, Qt through the C ABI, the web through wasm). The `_for` functions
//! take the state as an [`OutputLabelInput`], for a client that keeps its own
//! (the Swift console's view model).

use dspi_session::output_label::{self as ol, LabelFilter, LabelSource};

use crate::DspiCore;
use crate::records::{DspState, FilterParams};

/// One output's label.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputLabel {
    /// 0-based output index.
    pub output: u8,
    /// What to show: the user's name, else "Out 1 · R · Low".
    pub label: String,
    /// The same for a phone row, beside the output's "OUTn" button: the
    /// user's name, else "R · Low" (owner decision, review c86 F3).
    pub short_label: String,
    /// The firmware's name for the slot ("I2S 1 L", "PDM"), for a tooltip or
    /// a detail line.
    pub slot_name: String,
    /// The output has a name the user set; `label` is that name.
    pub user_named: bool,
    /// "L", "R", "L+R", an input's name, "Mix", or "None".
    pub channel: String,
    /// "Low", "High", "Band" or "Full".
    pub band: String,
}

/// One dashboard card: two outputs side by side, or one alone.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputCard {
    /// 0-based output index of the left (or only) column.
    pub left: u8,
    /// The right column's output; none for a card of one output.
    pub right: Option<u8>,
    /// "Woofers", "Tweeters" or "Mids" for a pair on one band with default
    /// names whose left output plays L and right output R by the routing;
    /// empty otherwise.
    pub title: String,
    /// The columns' headings: "Woofers L" / "Woofers R" under a title, else
    /// the outputs' labels.
    pub left_name: String,
    /// The right column's heading; empty for a card of one output.
    pub right_name: String,
    /// The left column's heading on a phone: "Woofers L" under a title, else
    /// the output's short label ("R · Low").
    pub left_short_name: String,
    /// The right column's heading on a phone: "R" under a title, else the
    /// output's short label; empty for a card of one output.
    pub right_short_name: String,
}

/// The state the labels read, for a client that keeps its own copy.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OutputLabelInput {
    /// Live input channels.
    pub num_inputs: u8,
    /// Live output channels, the PDM sub last.
    pub num_outputs: u8,
    /// Per slot: 1 = I2S, else S/PDIF.
    pub output_types: Vec<u8>,
    /// Channel names in wire order: inputs, then outputs.
    pub channel_names: Vec<String>,
    /// Crosspoint on, `[input][output]`.
    pub matrix_routing: Vec<Vec<bool>>,
    /// Each output's bands: PEQ and crossover, in any order.
    pub output_filters: Vec<Vec<FilterParams>>,
    /// Output enabled, per output.
    pub output_enabled: Vec<bool>,
    /// The K340 crossfeed pair map, 4 bytes; empty or zero elsewhere.
    pub crossfeed_pair_map: Vec<u8>,
}

fn filter(f: &FilterParams) -> LabelFilter {
    LabelFilter {
        filter_type: f.filter_type,
        bypass: f.bypass,
        freq: f.freq,
    }
}

fn pair_map(v: &[u8]) -> [u8; 4] {
    let mut m = [0u8; 4];
    for (d, s) in m.iter_mut().zip(v) {
        *d = *s;
    }
    m
}

impl From<&OutputLabelInput> for LabelSource {
    fn from(i: &OutputLabelInput) -> Self {
        LabelSource {
            num_inputs: usize::from(i.num_inputs),
            num_outputs: usize::from(i.num_outputs),
            output_types: i.output_types.clone(),
            names: i.channel_names.clone(),
            routing: i.matrix_routing.clone(),
            filters: i
                .output_filters
                .iter()
                .map(|bands| bands.iter().map(filter).collect())
                .collect(),
            output_enabled: i.output_enabled.clone(),
            pair_map: pair_map(&i.crossfeed_pair_map),
        }
    }
}

/// The mirror's view: output `o` is wire channel `num_input_channels + o`.
pub(crate) fn source(s: &DspState) -> LabelSource {
    let ni = usize::from(s.num_input_channels);
    let no = usize::from(s.num_output_channels);
    let bands = |wire: usize| -> Vec<LabelFilter> {
        let peq = s.filters.get(wire).map(Vec::as_slice).unwrap_or(&[]);
        let xo = s.xover.get(wire).map(Vec::as_slice).unwrap_or(&[]);
        peq.iter().chain(xo).map(filter).collect()
    };
    LabelSource {
        num_inputs: ni,
        num_outputs: no,
        output_types: s.output_types.clone(),
        names: s.channel_names.clone(),
        routing: s.matrix_routing.clone(),
        filters: (0..no).map(|o| bands(ni.saturating_add(o))).collect(),
        output_enabled: s.output_enabled.clone(),
        pair_map: pair_map(&s.crossfeed_pair_map),
    }
}

fn index(i: usize) -> u8 {
    u8::try_from(i).unwrap_or(u8::MAX)
}

fn labels(src: &LabelSource) -> Vec<OutputLabel> {
    ol::output_labels(src)
        .into_iter()
        .map(|l| OutputLabel {
            output: index(l.output),
            label: l.label,
            short_label: l.short_label,
            slot_name: l.slot_name,
            user_named: l.user_named,
            channel: l.channel,
            band: l.band.word().into(),
        })
        .collect()
}

fn cards(src: &LabelSource) -> Vec<OutputCard> {
    ol::output_cards(src)
        .into_iter()
        .map(|c| OutputCard {
            left: index(c.left),
            right: c.right.map(index),
            title: c.title,
            left_name: c.left_name,
            right_name: c.right_name,
            left_short_name: c.left_short_name,
            right_short_name: c.right_short_name,
        })
        .collect()
}

/// Every output's label for the state `input`, in output order.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn output_labels_for(input: OutputLabelInput) -> Vec<OutputLabel> {
    labels(&LabelSource::from(&input))
}

/// The dashboard's output cards for the state `input`, in order.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn output_cards_for(input: OutputLabelInput) -> Vec<OutputCard> {
    cards(&LabelSource::from(&input))
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Every output's label from the mirror, in output order: the user's name,
    /// else "Out N · channel · band" from the routing and the filters. Call it
    /// when the state changes, not per frame. Never fails.
    pub fn output_labels(&self) -> Vec<OutputLabel> {
        let src = source(&self.lock().state);
        labels(&src)
    }

    /// The dashboard's output cards from the mirror: paired by the crossfeed
    /// pair map when the device has one, else by slot. Never fails.
    pub fn output_cards(&self) -> Vec<OutputCard> {
        let src = source(&self.lock().state);
        cards(&src)
    }
}

#[cfg(test)]
mod tests_bug1 {
    use super::*;
    use crate::consts::IMPORT_HARDWARE;
    use std::sync::Arc;

    const K340: &str = include_str!("../../../tests/fixtures/preset/k340-bug1.dspipreset");

    fn demo() -> Arc<DspiCore> {
        let core = DspiCore::new();
        core.open_demo().expect("demo");
        core
    }

    fn k340() -> Arc<DspiCore> {
        let core = demo();
        let r: serde_json::Value =
            serde_json::from_str(&core.config_import_tok(K340.into(), IMPORT_HARDWARE)).unwrap();
        assert_eq!(r["ok"], true, "{r}");
        core
    }

    fn shown(core: &DspiCore) -> Vec<String> {
        core.output_labels().into_iter().map(|l| l.label).collect()
    }

    #[test]
    fn bug1_the_k340_fixture_on_the_demo_device() {
        let core = k340();
        let l = core.output_labels();
        let first: Vec<&str> = l.iter().take(4).map(|l| l.label.as_str()).collect();
        assert_eq!(
            first,
            [
                "Out 1 · R · Low",
                "Out 2 · R · High",
                "Out 3 · L · Low",
                "Out 4 · L · High"
            ]
        );
        let slots: Vec<&str> = l.iter().take(4).map(|l| l.slot_name.as_str()).collect();
        assert_eq!(slots, ["I2S 1 L", "I2S 1 R", "I2S 2 L", "I2S 2 R"]);
        assert_eq!((l[0].channel.as_str(), l[0].band.as_str()), ("R", "Low"));
        // -- review c86 F3 -- a phone's form beside the OUTn button
        let short: Vec<&str> = l.iter().take(4).map(|l| l.short_label.as_str()).collect();
        assert_eq!(short, ["R · Low", "R · High", "L · Low", "L · High"]);
        let c = core.output_cards();
        assert_eq!(
            (
                c[0].left_short_name.as_str(),
                c[0].right_short_name.as_str()
            ),
            ("Woofers L", "R")
        );
        let shape: Vec<(u8, Option<u8>, &str, &str)> = c
            .iter()
            .map(|c| (c.left, c.right, c.left_name.as_str(), c.right_name.as_str()))
            .collect();
        assert_eq!(
            shape,
            [
                (2, Some(0), "Woofers L", "Woofers R"),
                (3, Some(1), "Tweeters L", "Tweeters R")
            ]
        );
    }

    /// Review c86 F1 on the real core: the demo device's outputs 1 and 2
    /// low-passed, first both fed L+R (two mono subs), then fed R and L. The
    /// card names neither pair "Woofers L | R".
    #[test]
    fn bug1_c86_cards_take_sides_from_the_routing() {
        let core = demo();
        let ni = core.get_state().num_input_channels;
        let lp = FilterParams {
            filter_type: 34,
            bypass: false,
            freq: 80.0,
            q: 0.707,
            gain: 0.0,
            qp: 0.707,
        };
        for o in 0..2u8 {
            core.set_crossover(ni + o, 0, lp).expect("crossover");
            core.set_matrix_route(0, o, true, 0.0, false)
                .expect("route");
            core.set_matrix_route(1, o, true, 0.0, false)
                .expect("route");
        }
        let c = &core.output_cards()[0];
        assert_eq!((c.left, c.right, c.title.as_str()), (0, Some(1), ""));
        assert_eq!(
            (c.left_name.as_str(), c.right_name.as_str()),
            ("Out 1 · L+R · Low", "Out 2 · L+R · Low")
        );
        core.set_matrix_route(0, 0, false, 0.0, false)
            .expect("route");
        core.set_matrix_route(1, 1, false, 0.0, false)
            .expect("route");
        let c = &core.output_cards()[0];
        assert_eq!(c.title, "");
        assert_eq!(
            (c.left_short_name.as_str(), c.right_short_name.as_str()),
            ("R · Low", "L · Low")
        );
    }

    #[test]
    fn bug1_a_user_named_output_is_shown_as_named() {
        let core = k340();
        core.set_channel_name(8, "Right woofer".into())
            .expect("rename");
        let l = core.output_labels();
        assert_eq!(
            (l[0].label.as_str(), l[0].user_named),
            ("Right woofer", true)
        );
        assert_eq!(l[1].label, "Out 2 · R · High");
        // The woofer card loses its title: one side has the user's name
        let c = core.output_cards();
        assert_eq!(
            (c[0].title.as_str(), c[0].right_name.as_str()),
            ("", "Right woofer")
        );
    }

    #[test]
    fn bug1_the_default_demo() {
        let core = demo();
        let s = core.get_state();
        let l = shown(&core);
        assert_eq!(l.len(), usize::from(s.num_output_channels));
        // Every label is either the demo's own name or a derived one, and a
        // derived one starts with its output number
        for (o, x) in core.output_labels().iter().enumerate() {
            let name = &s.channel_names[usize::from(s.num_input_channels) + o];
            if x.user_named {
                assert_eq!(&x.label, name);
            } else {
                assert!(x.label.starts_with(&format!("Out {} · ", o + 1)), "{x:?}");
            }
        }
        // The demo has no pair map: cards pair by slot
        assert!(s.crossfeed_pair_map.iter().all(|&b| b == 0));
        for c in core.output_cards() {
            if let Some(r) = c.right {
                assert_eq!((c.left % 2, r), (0, c.left + 1), "{c:?}");
            }
        }
        println!("demo labels: {l:?}");
    }

    #[test]
    fn bug1_the_input_form_matches_the_mirror() {
        let core = k340();
        let s = core.get_state();
        let ni = usize::from(s.num_input_channels);
        let input = OutputLabelInput {
            num_inputs: s.num_input_channels,
            num_outputs: s.num_output_channels,
            output_types: s.output_types.clone(),
            channel_names: s.channel_names.clone(),
            matrix_routing: s.matrix_routing.clone(),
            output_filters: (0..usize::from(s.num_output_channels))
                .map(|o| [s.filters[ni + o].clone(), s.xover[ni + o].clone()].concat())
                .collect(),
            output_enabled: s.output_enabled.clone(),
            crossfeed_pair_map: s.crossfeed_pair_map.clone(),
        };
        assert_eq!(output_labels_for(input.clone()), core.output_labels());
        assert_eq!(output_cards_for(input), core.output_cards());
        // An empty input is no outputs, not a panic
        assert!(output_labels_for(OutputLabelInput::default()).is_empty());
        assert!(output_cards_for(OutputLabelInput::default()).is_empty());
    }
}
