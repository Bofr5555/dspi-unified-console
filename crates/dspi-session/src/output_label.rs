//! What an output is called on screen (BUG1, owner decision 2026-11-09).
//!
//! The firmware names an output after its DAC slot ("I2S 1 L": the left half
//! of slot 1), which says nothing about what the output plays: on the owner's
//! K340 every "L" is a woofer and every "R" a tweeter. While an output still
//! has that default name, it is labelled from the matrix and its filters
//! instead: "Out 1 · R · Low" (output, channel, band). A name the user set is
//! shown as it is. The slot name stays available for a tooltip or detail line.
//!
//! The dashboard's cards pair outputs by the K340's crossfeed pair map when the
//! device has one ("Woofers L" | "Woofers R"), else by slot as the Console does.
//!
//! Pure functions of a [`LabelSource`]: clients compute them when the routing,
//! the filters or the names change, never per frame. The macOS Console shows
//! the stored name (docs/UPSTREAM-DEVIATIONS.md, BUG1).

use dspi_proto::enums::FilterType;
use dspi_proto::ext::PairMap;

use crate::state::DeviceState;

/// A high pass at or above this splits the band; below it, it is a subsonic
/// or satellite filter and the output still plays the full range.
pub const HIGH_PASS_SPLIT_HZ: f32 = 150.0;
/// A low pass at or below this splits the band; above it, it only trims the
/// top octave.
pub const LOW_PASS_SPLIT_HZ: f32 = 12_000.0;
/// An input's name is cut to this many characters in a label.
const INPUT_NAME_CHARS: usize = 6;

/// One filter band of an output, PEQ or crossover.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelFilter {
    /// `FILTER_*`: 0..13 PEQ, 32..63 crossover.
    pub filter_type: u8,
    pub bypass: bool,
    pub freq: f32,
}

/// What the labels are worked out from: the parts of the device state they
/// read. Short vectors read as off, unnamed or default.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LabelSource {
    pub num_inputs: usize,
    pub num_outputs: usize,
    /// `output_types` per slot: 1 = I2S, else S/PDIF.
    pub output_types: Vec<u8>,
    /// Channel names in wire order: inputs, then outputs.
    pub names: Vec<String>,
    /// Crosspoint on, `[input][output]`.
    pub routing: Vec<Vec<bool>>,
    /// Each output's PEQ and crossover bands.
    pub filters: Vec<Vec<LabelFilter>>,
    pub output_enabled: Vec<bool>,
    /// The K340 firmware's crossfeed pair map; all zero on other firmware.
    pub pair_map: [u8; PairMap::SIZE],
}

/// The pass band an output's filters leave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    /// No band-splitting filter.
    Full,
    /// Only a low pass: a woofer or a sub.
    Low,
    /// Only a high pass: a tweeter.
    High,
    /// Both: a midrange.
    Mid,
}

impl Band {
    /// The word in a label.
    pub fn word(self) -> &'static str {
        match self {
            Band::Full => "Full",
            Band::Low => "Low",
            Band::High => "High",
            Band::Mid => "Band",
        }
    }

    /// The drivers a pair of outputs on this band feeds, for a card's title.
    pub fn drivers(self) -> Option<&'static str> {
        match self {
            Band::Full => None,
            Band::Low => Some("Woofers"),
            Band::High => Some("Tweeters"),
            Band::Mid => Some("Mids"),
        }
    }
}

/// One output's label.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputLabel {
    /// 0-based output index.
    pub output: usize,
    /// What to show: the user's name, else "Out 1 · R · Low".
    pub label: String,
    /// The firmware's name for the slot, "I2S 1 L" or "PDM".
    pub slot_name: String,
    /// The output has a name the user set, shown as `label`.
    pub user_named: bool,
    /// "L", "R", "L+R", an input's name, "Mix", or "None" when nothing is routed.
    pub channel: String,
    pub band: Band,
}

/// One dashboard card: a pair of outputs side by side, or one alone.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputCard {
    pub left: usize,
    pub right: Option<usize>,
    /// "Woofers" when both outputs carry default names and the same band;
    /// empty otherwise.
    pub title: String,
    /// The column headings: "Woofers L" / "Woofers R" under a title, else the
    /// outputs' labels. `right_name` is empty for a single output.
    pub left_name: String,
    pub right_name: String,
}

/// The firmware's name for output `out`'s slot (`get_default_channel_name`,
/// usb_audio.c): the last output is "PDM", the others "<I2S|SPDIF> <slot> <L|R>".
pub fn slot_name(out: usize, num_outputs: usize, output_types: &[u8]) -> String {
    if out.saturating_add(1) == num_outputs {
        return "PDM".into();
    }
    let slot = out / 2;
    let kind = if output_types.get(slot) == Some(&1) {
        "I2S"
    } else {
        "SPDIF"
    };
    let side = if out.is_multiple_of(2) { "L" } else { "R" };
    format!("{kind} {} {side}", slot.saturating_add(1))
}

/// Whether `name` is a firmware default for output `out`: empty, its slot's
/// name, or the slot's name under the other slot type (what an old Reset
/// Channel Names wrote on an I2S slot).
fn is_default_name(name: &str, out: usize, src: &LabelSource) -> bool {
    if name.is_empty() || name == slot_name(out, src.num_outputs, &src.output_types) {
        return true;
    }
    let flipped: Vec<u8> = (0..PairMap::SIZE)
        .map(|s| u8::from(src.output_types.get(s) != Some(&1)))
        .collect();
    name == slot_name(out, src.num_outputs, &flipped)
}

fn name_of(src: &LabelSource, wire: usize) -> &str {
    src.names.get(wire).map(String::as_str).unwrap_or("")
}

fn routed(src: &LabelSource, input: usize, out: usize) -> bool {
    src.routing
        .get(input)
        .and_then(|r| r.get(out))
        .copied()
        .unwrap_or(false)
}

/// The channel an output plays, from the inputs routed to it.
fn channel(src: &LabelSource, out: usize) -> String {
    let ins: Vec<usize> = (0..src.num_inputs)
        .filter(|&i| routed(src, i, out))
        .collect();
    match ins.as_slice() {
        [] => "None".into(),
        [0] => "L".into(),
        [1] => "R".into(),
        [0, 1] => "L+R".into(),
        [i] => {
            let name = name_of(src, *i);
            let name = if name.is_empty() {
                format!("In {}", i.saturating_add(1))
            } else {
                name.to_string()
            };
            if name.chars().count() > INPUT_NAME_CHARS {
                let cut: String = name
                    .chars()
                    .take(INPUT_NAME_CHARS.saturating_sub(1))
                    .collect();
                format!("{}…", cut.trim_end())
            } else {
                name
            }
        }
        _ => "Mix".into(),
    }
}

/// The band an output's active filters pass.
pub fn band(filters: &[LabelFilter]) -> Band {
    let (mut low, mut high) = (false, false);
    for f in filters.iter().filter(|f| !f.bypass) {
        let t = FilterType::from_raw(f.filter_type);
        let high_pass = match t {
            FilterType::LowPass | FilterType::LowPass1 => false,
            FilterType::HighPass | FilterType::HighPass1 => true,
            _ => match dspi_proto::xover::meta(f.filter_type) {
                Some(m) => m.high_pass,
                None => continue,
            },
        };
        if high_pass {
            high |= f.freq >= HIGH_PASS_SPLIT_HZ;
        } else {
            low |= f.freq <= LOW_PASS_SPLIT_HZ;
        }
    }
    match (low, high) {
        (false, false) => Band::Full,
        (true, false) => Band::Low,
        (false, true) => Band::High,
        (true, true) => Band::Mid,
    }
}

/// Every output's label, in output order.
pub fn output_labels(src: &LabelSource) -> Vec<OutputLabel> {
    (0..src.num_outputs)
        .map(|out| {
            let name = name_of(src, src.num_inputs.saturating_add(out));
            let user_named = !is_default_name(name, out, src);
            let channel = channel(src, out);
            let band = band(src.filters.get(out).map(Vec::as_slice).unwrap_or(&[]));
            let label = if user_named {
                name.to_string()
            } else {
                format!(
                    "Out {} · {channel} · {}",
                    out.saturating_add(1),
                    band.word()
                )
            };
            OutputLabel {
                output: out,
                label,
                slot_name: slot_name(out, src.num_outputs, &src.output_types),
                user_named,
                channel,
                band,
            }
        })
        .collect()
}

/// The dashboard's output cards, in order. Outputs pair by the crossfeed pair
/// map when the device has one, else by slot (1+2, 3+4, ..). A pair with both
/// outputs on is one card, a pair with one on is that output's card, and an
/// output in no pair (the PDM sub) has a card of its own when it is on.
pub fn output_cards(src: &LabelSource) -> Vec<OutputCard> {
    let labels = output_labels(src);
    let pdm = src.num_outputs.checked_sub(1);
    let paired = |o: usize| Some(o) != pdm && o < src.num_outputs;
    let mut used = vec![false; src.num_outputs];
    let mut pairs: Vec<(usize, Option<usize>)> = Vec::new();
    let map = PairMap(src.pair_map);
    if src.pair_map.iter().any(|&b| b != 0) {
        for p in 0..PairMap::PAIRS {
            let (l, r) = map.pair(p);
            let (l, r) = (usize::from(l), usize::from(r));
            if l != r && paired(l) && paired(r) && !used[l] && !used[r] {
                used[l] = true;
                used[r] = true;
                pairs.push((l, Some(r)));
            }
        }
    } else {
        let mut o: usize = 0;
        while o.saturating_add(1) < src.num_outputs && paired(o) && paired(o + 1) {
            used[o] = true;
            used[o + 1] = true;
            pairs.push((o, Some(o + 1)));
            o += 2;
        }
    }
    for (o, u) in used.iter().enumerate() {
        if !u {
            pairs.push((o, None));
        }
    }
    pairs.sort_by_key(|&(l, r)| l.min(r.unwrap_or(l)));

    let on = |o: usize| src.output_enabled.get(o).copied().unwrap_or(false);
    let mut cards = Vec::new();
    for (l, r) in pairs {
        let card = match (on(l), r.filter(|&r| on(r))) {
            (true, Some(r)) => {
                let (a, b) = (&labels[l], &labels[r]);
                match a
                    .band
                    .drivers()
                    .filter(|_| a.band == b.band && !a.user_named && !b.user_named)
                {
                    Some(t) => OutputCard {
                        left: l,
                        right: Some(r),
                        title: t.into(),
                        left_name: format!("{t} L"),
                        right_name: format!("{t} R"),
                    },
                    None => OutputCard {
                        left: l,
                        right: Some(r),
                        title: String::new(),
                        left_name: a.label.clone(),
                        right_name: b.label.clone(),
                    },
                }
            }
            (true, None) => single(&labels[l]),
            (false, Some(r)) => single(&labels[r]),
            (false, None) => continue,
        };
        cards.push(card);
    }
    cards
}

fn single(l: &OutputLabel) -> OutputCard {
    OutputCard {
        left: l.output,
        right: None,
        title: String::new(),
        left_name: l.label.clone(),
        right_name: String::new(),
    }
}

impl LabelSource {
    /// The source for a session's device state (the Terminal's).
    pub fn from_state(s: &DeviceState) -> Self {
        let (ni, no) = (
            usize::from(s.caps.num_inputs),
            usize::from(s.caps.num_outputs),
        );
        let filter = |b: &dspi_proto::value::EqParamPacket| LabelFilter {
            filter_type: b.filter_type.to_raw(),
            bypass: b.bypass,
            freq: b.freq,
        };
        let filters = (0..no)
            .map(|o| {
                let ch = u8::try_from(ni.saturating_add(o)).unwrap_or(u8::MAX);
                s.bands(ch)
                    .iter()
                    .chain(s.xover_bands(ch).iter())
                    .map(filter)
                    .collect()
            })
            .collect();
        LabelSource {
            num_inputs: ni,
            num_outputs: no,
            output_types: s.i2s().output_types.to_vec(),
            names: s.channel_names(),
            routing: (0..ni)
                .map(|i| (0..no).map(|o| s.crosspoint(i, o).enabled).collect())
                .collect(),
            filters,
            output_enabled: (0..no).map(|o| s.output(o).enabled).collect(),
            pair_map: s.crossfeed_pair_map().0,
        }
    }
}

#[cfg(test)]
mod tests_bug1 {
    use super::*;

    const LP: u8 = 4;
    const HP: u8 = 5;

    fn f(filter_type: u8, freq: f32) -> LabelFilter {
        LabelFilter {
            filter_type,
            bypass: false,
            freq,
        }
    }

    /// The K340 as found (tests/fixtures/preset/k340-bug1.dspipreset): USB 1
    /// feeds outputs 3-4, USB 2 outputs 1-2; 1 and 3 high cut, 2 and 4 low
    /// cut at 3.5 kHz; slots 1-2 I2S; the K340 pair map.
    pub(crate) fn k340() -> LabelSource {
        let mut routing = vec![vec![false; 9]; 8];
        routing[0][2] = true;
        routing[0][3] = true;
        routing[1][0] = true;
        routing[1][1] = true;
        let mut names: Vec<String> = (1..=8).map(|i| format!("USB {i}")).collect();
        names.extend(
            [
                "I2S 1 L",
                "I2S 1 R",
                "I2S 2 L",
                "I2S 2 R",
                "SPDIF 3 L",
                "SPDIF 3 R",
                "SPDIF 4 L",
                "SPDIF 4 R",
                "PDM",
            ]
            .map(String::from),
        );
        let mut filters = vec![Vec::new(); 9];
        filters[0] = vec![f(LP, 3500.0)];
        filters[1] = vec![f(HP, 3500.0)];
        filters[2] = vec![f(LP, 3500.0)];
        filters[3] = vec![f(HP, 3500.0)];
        LabelSource {
            num_inputs: 8,
            num_outputs: 9,
            output_types: vec![1, 1, 0, 0],
            names,
            routing,
            filters,
            output_enabled: [true, true, true, true, false, false, false, false, false].to_vec(),
            pair_map: PairMap::K340.0,
        }
    }

    fn labels(src: &LabelSource) -> Vec<String> {
        output_labels(src).into_iter().map(|l| l.label).collect()
    }

    #[test]
    fn bug1_k340_outputs_are_labelled_by_routing_and_band() {
        let l = output_labels(&k340());
        let shown: Vec<&str> = l.iter().take(4).map(|l| l.label.as_str()).collect();
        assert_eq!(
            shown,
            [
                "Out 1 · R · Low",
                "Out 2 · R · High",
                "Out 3 · L · Low",
                "Out 4 · L · High"
            ]
        );
        let slots: Vec<&str> = l.iter().map(|l| l.slot_name.as_str()).collect();
        assert_eq!(
            slots,
            [
                "I2S 1 L",
                "I2S 1 R",
                "I2S 2 L",
                "I2S 2 R",
                "SPDIF 3 L",
                "SPDIF 3 R",
                "SPDIF 4 L",
                "SPDIF 4 R",
                "PDM"
            ]
        );
        assert!(l.iter().all(|l| !l.user_named));
        assert_eq!(l[4].label, "Out 5 · None · Full");
        assert_eq!(l[8].label, "Out 9 · None · Full");
        // Fits a phone row: no label longer than the longest stock one
        assert!(l.iter().all(|l| l.label.chars().count() <= 20), "{l:?}");
    }

    #[test]
    fn bug1_a_user_name_wins_and_is_shown_as_it_is() {
        let mut s = k340();
        s.names[8] = "Right woofer".into();
        let l = output_labels(&s);
        assert_eq!(l[0].label, "Right woofer");
        assert!(l[0].user_named);
        assert_eq!(l[0].slot_name, "I2S 1 L");
        assert_eq!(l[1].label, "Out 2 · R · High");
        // An empty name, and the S/PDIF name an old reset wrote on an I2S slot, are defaults
        s.names[9] = String::new();
        s.names[10] = "SPDIF 2 L".into();
        let l = output_labels(&s);
        assert_eq!(
            (l[1].label.as_str(), l[2].label.as_str()),
            ("Out 2 · R · High", "Out 3 · L · Low")
        );
        // A default of another output is the user's choice
        s.names[11] = "I2S 1 L".into();
        assert_eq!(output_labels(&s)[3].label, "I2S 1 L");
    }

    #[test]
    fn bug1_channel_words() {
        let mut s = k340();
        s.routing[0][4] = true;
        s.routing[1][4] = true; // L+R
        s.routing[2][5] = true; // USB 3
        s.names[3] = "Center channel".into();
        s.routing[2][6] = true;
        s.routing[3][6] = true; // two others: Mix
        s.routing[3][7] = true; // USB 4, by its name
        s.names[2] = String::new();
        s.routing[2][8] = true; // an unnamed input
        let l = labels(&s);
        assert_eq!(l[4], "Out 5 · L+R · Full");
        assert_eq!(l[5], "Out 6 · In 3 · Full");
        assert_eq!(l[6], "Out 7 · Mix · Full");
        assert_eq!(l[7], "Out 8 · Cente… · Full");
        assert_eq!(l[8], "Out 9 · In 3 · Full");
    }

    #[test]
    fn bug1_band_words() {
        assert_eq!(band(&[]), Band::Full);
        assert_eq!(band(&[f(1, 1000.0), f(2, 100.0)]), Band::Full); // a peak and a shelf
        assert_eq!(band(&[f(LP, 80.0)]), Band::Low);
        assert_eq!(band(&[f(12, 300.0)]), Band::Low); // first-order low pass
        assert_eq!(band(&[f(HP, 2000.0)]), Band::High);
        assert_eq!(band(&[f(HP, 300.0), f(LP, 3000.0)]), Band::Mid);
        // A subsonic high pass and a top-octave low pass do not split the band
        assert_eq!(band(&[f(HP, 30.0), f(LP, 18000.0)]), Band::Full);
        // A bypassed band does not count
        assert_eq!(
            band(&[LabelFilter {
                bypass: true,
                ..f(LP, 80.0)
            }]),
            Band::Full
        );
        // Crossovers: LR4 low pass is 34, LR4 high pass 35 (enums.rs)
        assert_eq!(band(&[f(34, 2000.0)]), Band::Low);
        assert_eq!(band(&[f(35, 2000.0)]), Band::High);
        assert_eq!(band(&[f(200, 2000.0)]), Band::Full); // a type nobody knows
    }

    #[test]
    fn bug1_k340_cards_pair_by_the_crossfeed_map() {
        let c = output_cards(&k340());
        assert_eq!(c.len(), 2);
        assert_eq!((c[0].left, c[0].right), (2, Some(0)));
        assert_eq!(
            (
                c[0].left_name.as_str(),
                c[0].right_name.as_str(),
                c[0].title.as_str()
            ),
            ("Woofers L", "Woofers R", "Woofers")
        );
        assert_eq!((c[1].left, c[1].right), (3, Some(1)));
        assert_eq!(
            (c[1].left_name.as_str(), c[1].right_name.as_str()),
            ("Tweeters L", "Tweeters R")
        );
    }

    #[test]
    fn bug1_cards_without_a_map_pair_by_slot() {
        let mut s = k340();
        s.pair_map = [0; 4];
        let c = output_cards(&s);
        assert_eq!(
            (c[0].left, c[0].right, c[0].title.as_str()),
            (0, Some(1), "")
        );
        assert_eq!(
            (c[0].left_name.as_str(), c[0].right_name.as_str()),
            ("Out 1 · R · Low", "Out 2 · R · High")
        );
        assert_eq!((c[1].left, c[1].right), (2, Some(3)));
        // One side off: the other alone; the PDM sub alone when on
        s.output_enabled[3] = false;
        s.output_enabled[8] = true;
        let c = output_cards(&s);
        assert_eq!(
            (c[1].left, c[1].right, c[1].left_name.as_str()),
            (2, None, "Out 3 · L · Low")
        );
        assert_eq!((c[2].left, c[2].right), (8, None));
    }

    #[test]
    fn bug1_a_user_named_pair_keeps_its_names() {
        let mut s = k340();
        s.names[10] = "Woofer left".into();
        let c = output_cards(&s);
        assert_eq!(
            (
                c[0].title.as_str(),
                c[0].left_name.as_str(),
                c[0].right_name.as_str()
            ),
            ("", "Woofer left", "Out 1 · R · Low")
        );
    }

    #[test]
    fn bug1_a_map_with_bad_bytes_falls_back_per_pair() {
        let mut s = k340();
        // Pair 0 names the PDM output, pair 1 repeats an output: both skipped,
        // their outputs get cards of their own
        s.pair_map = [0x08, 0x22, 0, 0];
        s.output_enabled = vec![true; 9];
        let c = output_cards(&s);
        let shape: Vec<(usize, Option<usize>)> = c.iter().map(|c| (c.left, c.right)).collect();
        assert_eq!(
            shape,
            [
                (0, None),
                (1, None),
                (2, None),
                (3, None),
                (4, Some(5)),
                (6, Some(7)),
                (8, None)
            ]
        );
    }

    #[test]
    fn bug1_short_and_odd_sources_never_panic() {
        let s = LabelSource {
            num_inputs: 2,
            num_outputs: 5,
            ..LabelSource::default()
        };
        assert_eq!(
            labels(&s),
            [
                "Out 1 · None · Full",
                "Out 2 · None · Full",
                "Out 3 · None · Full",
                "Out 4 · None · Full",
                "Out 5 · None · Full"
            ]
        );
        assert!(output_cards(&s).is_empty());
        assert!(output_labels(&LabelSource::default()).is_empty());
        let mut odd = s.clone();
        odd.pair_map = [0xFF; 4];
        odd.output_enabled = vec![true; 5];
        assert_eq!(output_cards(&odd).len(), 5);
    }
}
