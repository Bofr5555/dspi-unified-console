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
    // -- WP FIN015 -- (REVF R1)
    /// Per output: a test signal plays on it now (the signal generator, which
    /// a measurement, a live run and Identify use too). It writes after the
    /// matrix, so an output with nothing routed to it can still be loud.
    pub playing: Vec<bool>,
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
    /// What to show: the user's name, else "Out 1 · R · Low" (no "· Full" for
    /// a full-range one: "Out 1 · L"); an unused output is its number and
    /// "None", "3 · None" (owner 2026-11-24, REVEFIX).
    pub label: String,
    /// The same for a phone row, beside the output's "OUTn" button: the
    /// user's name, else channel and band alone, "R · Low" (owner, c86 F3), or
    /// "None" for an unused output.
    pub short_label: String,
    /// The firmware's name for the slot, "I2S 1 L" or "PDM".
    pub slot_name: String,
    /// The output has a name the user set, shown as `label`.
    pub user_named: bool,
    /// "L", "R", "L+R", an input's name, "Mix", or "None" when nothing is routed.
    pub channel: String,
    pub band: Band,
    /// -- WP REVEFIX -- No input is routed to it (owner 2026-11-24): views that
    /// only show (the Overview's cards and tiles, a phone's Home, summaries)
    /// leave it out; views that edit it show `label`, "3 · None".
    pub unused: bool,
    // -- WP FIN015 -- (REVF R1)
    /// `unused`, and no test signal plays on it: what a view that only
    /// shows levels (meters, a Spectrum chooser, Stats) leaves out. An unused
    /// output the generator plays on keeps its meter.
    pub hidden: bool,
}

/// One dashboard card: a pair of outputs side by side, or one alone.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputCard {
    pub left: usize,
    pub right: Option<usize>,
    /// "Woofers" when both outputs carry default names and the same band,
    /// and the left one plays L and the right one R by the routing; empty
    /// otherwise.
    pub title: String,
    /// The column headings: "Woofers L" / "Woofers R" under a title, else the
    /// outputs' labels. `right_name` is empty for a single output.
    pub left_name: String,
    pub right_name: String,
    /// The same for a phone's narrow columns: "Woofers L" / "R" under a
    /// title, else the outputs' short labels ("R · Low").
    pub left_short_name: String,
    pub right_short_name: String,
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

/// Whether `name` is a firmware default for output `out`: empty (or only
/// spaces, which show as nothing), its slot's
/// name, or the slot's name under the other slot type (what an old Reset
/// Channel Names wrote on an I2S slot).
fn is_default_name(name: &str, out: usize, src: &LabelSource) -> bool {
    if name.trim().is_empty() || name == slot_name(out, src.num_outputs, &src.output_types) {
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

// -- WP ROLE0 -- one inference for the labels and the measurement's layout
/// A low pass at or below this, with no high pass, is a subwoofer.
pub const SUB_LOW_PASS_HZ: f32 = 120.0;
/// An active high pass at or above this makes an output a tweeter for safety,
/// whatever the user called it (MPLAN 2.2, the effective role).
pub const TWEETER_HIGH_PASS_HZ: f32 = 1_000.0;
/// A corner within this of a threshold counts (the registry's quantisation).
const HZ_SLACK: f32 = 0.5;

/// What an output drives, as its filters (and, for a sub, its name) suggest
/// it, or as the user's speaker layout says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverRole {
    /// No band-splitting filter.
    Full,
    /// A low pass at or below [`SUB_LOW_PASS_HZ`] alone, or a name with "sub".
    Sub,
    /// Only a low pass: a woofer.
    Low,
    /// Both: a midrange.
    Mid,
    /// Only a high pass: a tweeter.
    High,
}

impl DriverRole {
    /// The band a label shows for it.
    pub fn band(self) -> Band {
        match self {
            DriverRole::Full => Band::Full,
            DriverRole::Sub | DriverRole::Low => Band::Low,
            DriverRole::Mid => Band::Mid,
            DriverRole::High => Band::High,
        }
    }
}

/// The active band-splitting corners: the lowest low pass at or below
/// [`LOW_PASS_SPLIT_HZ`] and the highest high pass at or above
/// [`HIGH_PASS_SPLIT_HZ`].
#[derive(Debug, Clone, Copy, Default)]
struct Split {
    low_pass: Option<f32>,
    high_pass: Option<f32>,
}

/// Whether `f` is a high pass (`Some(true)`), a low pass (`Some(false)`), or
/// neither (a bell, a shelf, an all-pass).
fn pass_kind(f: &LabelFilter) -> Option<bool> {
    match FilterType::from_raw(f.filter_type) {
        FilterType::LowPass | FilterType::LowPass1 => Some(false),
        FilterType::HighPass | FilterType::HighPass1 => Some(true),
        _ => dspi_proto::xover::meta(f.filter_type).map(|m| m.high_pass),
    }
}

fn split(filters: &[LabelFilter]) -> Split {
    let mut s = Split::default();
    for f in filters.iter().filter(|f| !f.bypass && f.freq.is_finite()) {
        match pass_kind(f) {
            Some(true) if f.freq >= HIGH_PASS_SPLIT_HZ => {
                s.high_pass = Some(s.high_pass.map_or(f.freq, |h| h.max(f.freq)));
            }
            Some(false) if f.freq <= LOW_PASS_SPLIT_HZ => {
                s.low_pass = Some(s.low_pass.map_or(f.freq, |l| l.min(f.freq)));
            }
            _ => {}
        }
    }
    s
}

/// The one inference of what an output drives (MPLAN 2.2): the labels'
/// band and the measurement's layout proposal both come from it. A name
/// containing "sub" (any case) makes it a sub.
pub fn driver_role(filters: &[LabelFilter], name: &str) -> DriverRole {
    if name.to_lowercase().contains("sub") {
        return DriverRole::Sub;
    }
    let s = split(filters);
    match (s.low_pass, s.high_pass) {
        (Some(f), None) if f <= SUB_LOW_PASS_HZ + HZ_SLACK => DriverRole::Sub,
        (Some(_), None) => DriverRole::Low,
        (Some(_), Some(_)) => DriverRole::Mid,
        (None, Some(_)) => DriverRole::High,
        (None, None) => DriverRole::Full,
    }
}

/// The role the safety rules use (MPLAN 2.2): the stricter of `user` (the
/// speaker layout's, or a caller's) and what the filters show. An output with
/// an active high pass at or above [`TWEETER_HIGH_PASS_HZ`] is a tweeter,
/// whatever it is called. Only that escalates: a lower high pass (a
/// satellite's) leaves the user's role, which would otherwise hold a
/// satellite to the tweeter's corner.
pub fn effective_role(user: DriverRole, filters: &[LabelFilter]) -> DriverRole {
    let tweeter = filters.iter().any(|f| {
        !f.bypass && pass_kind(f) == Some(true) && f.freq >= TWEETER_HIGH_PASS_HZ - HZ_SLACK
    });
    if tweeter { DriverRole::High } else { user }
}

/// The band an output's active filters pass.
pub fn band(filters: &[LabelFilter]) -> Band {
    driver_role(filters, "").band()
}

/// Every output's label, in output order.
pub fn output_labels(src: &LabelSource) -> Vec<OutputLabel> {
    (0..src.num_outputs)
        .map(|out| {
            let name = name_of(src, src.num_inputs.saturating_add(out));
            let user_named = !is_default_name(name, out, src);
            let channel = channel(src, out);
            let band = band(src.filters.get(out).map(Vec::as_slice).unwrap_or(&[]));
            // -- WP REVEFIX -- (owner 2026-11-24) an unused output is its number
            // and "None" wherever it is shown, "3 · None", whatever its band
            let unused = !(0..src.num_inputs).any(|i| routed(src, i, out));
            // -- WP INTD -- (REVD W12) a full-range output says no "· Full":
            // "Out 1 · L", "Out 4 · R · Low"
            let words: Vec<&str> = [
                Some(channel.as_str()),
                (band != Band::Full).then_some(band.word()),
            ]
            .into_iter()
            .flatten()
            .collect();
            let number = format!("Out {}", out.saturating_add(1));
            let label = if user_named {
                name.to_string()
            } else if unused {
                format!("{} · None", out.saturating_add(1))
            } else {
                std::iter::once(number.as_str())
                    .chain(words.iter().copied())
                    .collect::<Vec<_>>()
                    .join(" · ")
            };
            let short_label = if user_named {
                name.to_string()
            } else if unused {
                "None".into()
            } else {
                words.join(" · ")
            };
            OutputLabel {
                output: out,
                label,
                short_label,
                slot_name: slot_name(out, src.num_outputs, &src.output_types),
                user_named,
                channel,
                band,
                unused,
                // -- WP FIN015 --
                hidden: unused && !src.playing.get(out).copied().unwrap_or(false),
            }
        })
        .collect()
}

/// The dashboard's output cards, in order. Outputs pair by the crossfeed pair
/// map when the device has one, else by slot (1+2, 3+4, ..). A pair with both
/// outputs on is one card, a pair with one on is that output's card, and an
/// output in no pair (the PDM sub) has a card of its own when it is on. An
/// unused output counts as off (-- WP REVEFIX --).
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

    // -- WP REVEFIX -- an unused output has no card (owner 2026-11-24), as one that is off
    let on = |o: usize| src.output_enabled.get(o).copied().unwrap_or(false) && !labels[o].unused;
    let mut cards = Vec::new();
    for (l, r) in pairs {
        let card = match (on(l), r.filter(|&r| on(r))) {
            (true, Some(r)) => {
                let (a, b) = (&labels[l], &labels[r]);
                // A side letter only where the routing says it (review c86 F1):
                // the left output plays L and the right one R, not by position
                let sides = a.channel == "L" && b.channel == "R";
                match a
                    .band
                    .drivers()
                    .filter(|_| sides && a.band == b.band && !a.user_named && !b.user_named)
                {
                    Some(t) => OutputCard {
                        left: l,
                        right: Some(r),
                        title: t.into(),
                        left_name: format!("{t} L"),
                        right_name: format!("{t} R"),
                        left_short_name: format!("{t} L"),
                        right_short_name: "R".into(),
                    },
                    None => OutputCard {
                        left: l,
                        right: Some(r),
                        title: String::new(),
                        left_name: a.label.clone(),
                        right_name: b.label.clone(),
                        left_short_name: a.short_label.clone(),
                        right_short_name: b.short_label.clone(),
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
        left_short_name: l.short_label.clone(),
        right_short_name: String::new(),
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
            // -- WP FIN015 -- not in the image: what the host learnt
            playing: (0..no)
                .map(|o| {
                    u32::try_from(o)
                        .is_ok_and(|o| s.siggen_outputs.checked_shr(o).is_some_and(|m| m & 1 != 0))
                })
                .collect(),
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
            playing: Vec::new(),
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
        // -- WP REVEFIX -- (owner 2026-11-24) an unused output is its number and "None"
        assert_eq!(l[4].label, "5 · None");
        assert_eq!(l[8].label, "9 · None");
        assert_eq!(l[4].short_label, "None");
        assert!(l[4].unused && l[8].unused && !l[0].unused && !l[3].unused);
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
        assert_eq!(l[4], "Out 5 · L+R");
        assert_eq!(l[5], "Out 6 · In 3");
        assert_eq!(l[6], "Out 7 · Mix");
        assert_eq!(l[7], "Out 8 · Cente…");
        assert_eq!(l[8], "Out 9 · In 3");
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
        // One side off: the other alone; the PDM sub alone when on (and used:
        // -- WP REVEFIX --)
        s.output_enabled[3] = false;
        s.output_enabled[8] = true;
        s.routing[2][8] = true;
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
        // Every output used (-- WP REVEFIX --: an unused one has no card)
        for o in 4..9 {
            s.routing[2][o] = true;
        }
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
            ["1 · None", "2 · None", "3 · None", "4 · None", "5 · None"]
        );
        assert!(output_cards(&s).is_empty());
        assert!(output_labels(&LabelSource::default()).is_empty());
        let mut odd = s.clone();
        odd.pair_map = [0xFF; 4];
        odd.output_enabled = vec![true; 5];
        odd.routing = vec![vec![true; 5]; 2];
        assert_eq!(output_cards(&odd).len(), 5);
    }

    // -- WP REVEFIX --
    /// An unused output (nothing routed to it) is "3 · None" whatever its
    /// filters say, and has no card even when it is on (owner 2026-11-24).
    #[test]
    fn revefix_an_unused_output_is_its_number_and_none_and_has_no_card() {
        let mut s = k340();
        s.filters[4] = vec![f(LP, 300.0)];
        s.output_enabled = vec![true; 9];
        let l = output_labels(&s);
        assert_eq!(
            (l[4].label.as_str(), l[4].short_label.as_str(), l[4].unused),
            ("5 · None", "None", true)
        );
        assert_eq!(l[4].band, Band::Low);
        // A name the user set stays, still flagged
        s.names[12] = "Spare".into();
        let l = output_labels(&s);
        assert_eq!((l[4].label.as_str(), l[4].unused), ("Spare", true));
        let cards: Vec<_> = output_cards(&s).iter().map(|c| (c.left, c.right)).collect();
        assert!(
            cards.iter().all(|&(l, r)| l < 4 && r.is_none_or(|r| r < 4)),
            "{cards:?}"
        );
        // Routed again, it is shown and has its card
        s.routing[0][4] = true;
        let l = output_labels(&s);
        assert!(!l[4].unused);
        assert!(
            output_cards(&s)
                .iter()
                .any(|c| c.left == 4 || c.right == Some(4))
        );
    }

    // -- WP FIN015 --
    /// An unused output the generator plays on is not hidden: it keeps its
    /// meter, Spectrum entry and Stats rows, and still says "5 · None" and has
    /// no card (REVF R1). Routed, it is neither.
    #[test]
    fn fin015_an_unused_output_a_test_signal_plays_on_is_not_hidden() {
        let mut s = k340();
        let l = output_labels(&s);
        assert!(l[4].unused && l[4].hidden, "silent and unrouted");
        assert!(!l[0].unused && !l[0].hidden);
        s.playing = vec![false; 9];
        s.playing[4] = true;
        let l = output_labels(&s);
        assert!(l[4].unused, "still unused");
        assert!(!l[4].hidden, "but shown while the generator plays on it");
        assert_eq!(l[4].label, "5 · None");
        assert!(l[5].hidden, "the others stay hidden");
        // A short `playing` reads as silent.
        s.playing = vec![true; 2];
        assert!(output_labels(&s)[4].hidden);
    }

    // -- review c86 --
    /// Two mono subs on outputs 1 and 2, both fed L+R and low-passed, no pair
    /// map: the card must not call them L and R.
    #[test]
    fn bug1_c86_two_mono_subs_are_not_l_and_r() {
        let mut s = k340();
        s.pair_map = [0; 4];
        s.routing = vec![vec![false; 9]; 8];
        for o in 0..2 {
            s.routing[0][o] = true;
            s.routing[1][o] = true;
            s.filters[o] = vec![f(LP, 80.0)];
        }
        let c = output_cards(&s);
        assert_eq!((c[0].left, c[0].right), (0, Some(1)));
        assert_eq!(c[0].title, "");
        assert_eq!(
            (c[0].left_name.as_str(), c[0].right_name.as_str()),
            ("Out 1 · L+R · Low", "Out 2 · L+R · Low")
        );
        assert_eq!(
            (
                c[0].left_short_name.as_str(),
                c[0].right_short_name.as_str()
            ),
            ("L+R · Low", "L+R · Low")
        );
    }

    /// Outputs 1 and 2 low-passed, output 1 fed R and output 2 fed L: the
    /// left column plays R, so no "Woofers L" over it.
    #[test]
    fn bug1_c86_swapped_routing_keeps_each_outputs_side() {
        let mut s = k340();
        s.pair_map = [0; 4];
        s.routing = vec![vec![false; 9]; 8];
        s.routing[1][0] = true;
        s.routing[0][1] = true;
        s.filters[0] = vec![f(LP, 80.0)];
        s.filters[1] = vec![f(LP, 80.0)];
        let c = output_cards(&s);
        assert_eq!(c[0].title, "");
        assert_eq!(
            (c[0].left_name.as_str(), c[0].right_name.as_str()),
            ("Out 1 · R · Low", "Out 2 · L · Low")
        );
        assert_eq!(
            (
                c[0].left_short_name.as_str(),
                c[0].right_short_name.as_str()
            ),
            ("R · Low", "L · Low")
        );
        // Routed the other way round, the same pair is woofers L and R
        s.routing[1][0] = false;
        s.routing[0][1] = false;
        s.routing[0][0] = true;
        s.routing[1][1] = true;
        let c = output_cards(&s);
        assert_eq!(
            (
                c[0].title.as_str(),
                c[0].left_name.as_str(),
                c[0].right_name.as_str()
            ),
            ("Woofers", "Woofers L", "Woofers R")
        );
    }

    /// The K340: by the map, left is L and right is R, so the headings stand;
    /// the phone's forms are the short ones.
    #[test]
    fn bug1_c86_k340_headings_and_short_forms() {
        let l = output_labels(&k340());
        let short: Vec<&str> = l.iter().take(4).map(|l| l.short_label.as_str()).collect();
        assert_eq!(short, ["R · Low", "R · High", "L · Low", "L · High"]);
        let c = output_cards(&k340());
        let heads: Vec<(&str, &str, &str, &str)> = c
            .iter()
            .map(|c| {
                (
                    c.left_name.as_str(),
                    c.right_name.as_str(),
                    c.left_short_name.as_str(),
                    c.right_short_name.as_str(),
                )
            })
            .collect();
        assert_eq!(
            heads,
            [
                ("Woofers L", "Woofers R", "Woofers L", "R"),
                ("Tweeters L", "Tweeters R", "Tweeters L", "R")
            ]
        );
        // A user's name is its own short form
        let mut s = k340();
        s.names[8] = "Right woofer".into();
        assert_eq!(output_labels(&s)[0].short_label, "Right woofer");
    }

    /// A name of only spaces shows as nothing: it counts as unnamed (c86 N2).
    #[test]
    fn bug1_c86_a_blank_name_is_unnamed() {
        let mut s = k340();
        s.names[8] = "   ".into();
        let l = &output_labels(&s)[0];
        assert!(!l.user_named);
        assert_eq!(l.label, "Out 1 · R · Low");
    }
}

// -- WP ROLE0 --
#[cfg(test)]
mod tests_wp_role0 {
    use super::*;

    fn f(filter_type: u8, freq: f32) -> LabelFilter {
        LabelFilter {
            filter_type,
            bypass: false,
            freq,
        }
    }

    #[test]
    fn wp_role0_one_inference_with_the_labels_thresholds() {
        use DriverRole as D;
        let cases: &[(&[LabelFilter], &str, DriverRole)] = &[
            (&[], "", D::Full),
            (&[f(5, 100.0)], "", D::Full), // below the split: a subsonic filter
            (&[f(5, 150.0)], "", D::High),
            (&[f(4, 12_500.0)], "", D::Full), // above the split: a trim
            (&[f(4, 12_000.0)], "", D::Low),
            (&[f(4, 120.0)], "", D::Sub),
            (&[f(4, 121.0)], "", D::Low),
            (&[f(5, 300.0), f(4, 3_000.0)], "", D::Mid),
            (&[f(5, 300.0), f(4, 80.0)], "", D::Mid),
            (&[f(4, 3_000.0)], "my sub", D::Sub),
            (&[f(5, 3_000.0)], "Subwoofer", D::Sub),
        ];
        for (filters, name, want) in cases {
            assert_eq!(driver_role(filters, name), *want, "{filters:?} {name}");
            // The label's band is the same inference, name aside.
            assert_eq!(band(filters), driver_role(filters, "").band());
        }
        // A bypassed or non-finite filter is no filter.
        let mut off = f(5, 3_000.0);
        off.bypass = true;
        assert_eq!(driver_role(&[off], ""), D::Full);
        assert_eq!(driver_role(&[f(5, f32::NAN)], ""), D::Full);
    }
}
