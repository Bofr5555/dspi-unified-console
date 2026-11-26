//! The palette's search (`Ctrl-P`, WP UTM, UXP A13).
//!
//! The palette used to be the `:` line's completer: a prefix of a parameter
//! path, so "mute" found nothing (the path is `vol.mute`) and "crossover"
//! found nothing at all. Here a query is words, each matched against the
//! start of any word of a place's name, its description, its path and a
//! short list of the words people use for it ("crossover" for the XO tab,
//! "low pass" for `lp`). Places and actions come first, then the
//! interface's verbs, then parameters; a query that finds nothing says so.

use dspi_cmd::Context;
use dspi_proto::registry::REGISTRY;

use crate::shell::{Selection, ShellEvent, Tool};

/// What choosing a hit does.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// What the shell would have asked for by its key.
    Event(ShellEvent),
    /// A line to run as typed on the `:` line.
    Line(String),
    /// The `:` line, opened with this text for the rest of the command.
    Complete(String),
    /// The `?` overlay.
    Help,
    /// The volume row, with the keyboard.
    Volume,
    /// An output's page on its XO tab.
    Crossover,
}

/// Where a hit sorts: places first, parameters last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rank {
    Place,
    Verb,
    Parameter,
}

/// One thing the palette can find.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// What the palette lists: a place's name, a verb, a path.
    pub value: String,
    /// The words beside it: what it is and its key.
    pub detail: String,
    pub action: Action,
    pub rank: Rank,
    /// Words that find it without being shown.
    keywords: &'static str,
}

impl Hit {
    fn new(
        value: impl Into<String>,
        detail: impl Into<String>,
        action: Action,
        rank: Rank,
    ) -> Self {
        Self {
            value: value.into(),
            detail: detail.into(),
            action,
            rank,
            keywords: "",
        }
    }

    fn also(mut self, keywords: &'static str) -> Self {
        self.keywords = keywords;
        self
    }

    /// How well every word of the query matches, lower is better; `None`
    /// when a word matches nothing.
    fn score(&self, query: &[String]) -> Option<u32> {
        let name = words(&self.value);
        let rest: Vec<String> = words(&self.detail)
            .into_iter()
            .chain(words(self.keywords))
            .collect();
        let mut total = 0;
        for q in query {
            total += if name.iter().any(|w| w == q) {
                0
            } else if name.iter().any(|w| w.starts_with(q.as_str())) {
                1
            } else if rest.iter().any(|w| w.starts_with(q.as_str())) {
                3
            } else {
                return None;
            };
        }
        Some(total)
    }
}

/// The lowercase words of `text`, split at anything that is not a letter or
/// a digit, so `vol.mute` is "vol" and "mute" and "Out 1 · L" is three.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// The places and actions, with the words people use for them.
fn places() -> Vec<Hit> {
    let ev = |e: ShellEvent| Action::Event(e);
    let mut out = vec![
        Hit::new(
            "Crossover",
            "an output's XO tab (x on its page)",
            Action::Crossover,
            Rank::Place,
        )
        .also("xo high low pass hp lp lr4 lr2 linkwitz riley butterworth split tweeter woofer"),
        Hit::new(
            "Mute",
            "the listening volume's mute (m)",
            ev(ShellEvent::VolumeMute),
            Rank::Place,
        )
        .also("silence quiet unmute"),
        Hit::new(
            "Volume",
            "the listening volume ([ ] change it anywhere)",
            Action::Volume,
            Rank::Place,
        )
        .also("level loudness master user louder softer"),
        Hit::new(
            "Overview",
            "every channel at once (Esc)",
            ev(ShellEvent::Select(Selection::Overview)),
            Rank::Place,
        )
        .also("home channels"),
        Hit::new(
            "Equaliser",
            "a channel's filter bands (select it in the list)",
            ev(ShellEvent::Select(Selection::Overview)),
            Rank::Place,
        )
        .also("eq equalizer peq filter band bands bell peak shelf notch"),
        Hit::new("Settings", "the device's settings (,)", ev(ShellEvent::OpenSettings), Rank::Place)
            .also("preferences options config configuration i2s inputs outputs graphing about advanced groups macros surfaces interfaces aux"),
        Hit::new(
            "Presets",
            "load, save, rename (the preset row)",
            ev(ShellEvent::Preset(None)),
            Rank::Place,
        )
        .also("preset slot load default"),
        Hit::new(
            "Save to Device",
            "commit the parameters to the preset (Ctrl-S)",
            ev(ShellEvent::SavePreset),
            Rank::Place,
        )
        .also("commit store write flash"),
        Hit::new("Undo", "the last change (Ctrl-Z)", ev(ShellEvent::Undo), Rank::Place)
            .also("back revert"),
        Hit::new("Redo", "what undo put back (Ctrl-Y)", ev(ShellEvent::Redo), Rank::Place),
        Hit::new(
            "Bypass",
            "the master EQ, on or off (b)",
            ev(ShellEvent::BypassToggle),
            Rank::Place,
        )
        .also("eq flat compare"),
        Hit::new(
            "Device",
            "the device picker (Ctrl-D)",
            ev(ShellEvent::DevicePicker),
            Rank::Place,
        )
        .also("devices switch picker usb serial"),
        Hit::new("Help", "the keys here and everywhere (?)", Action::Help, Rank::Place)
            .also("keys shortcuts keyboard"),
        Hit::new(
            "Snapshots",
            "save, list, recall (:snapshot)",
            Action::Complete("snapshot ".into()),
            Rank::Place,
        )
        .also("snapshot capture recall compare before"),
        Hit::new(
            "A/B",
            "compare two snapshots (:ab)",
            Action::Complete("ab ".into()),
            Rank::Place,
        )
        .also("ab compare snapshot"),
        Hit::new(
            "Measurements",
            "the measurement documents (:measure docs)",
            Action::Line("measure docs".into()),
            Rank::Place,
        )
        .also("measure measuring mic microphone sweep room auto tune autotune"),
        Hit::new(
            "Installer lock",
            "lock, unlock, limits (:lock, :limits)",
            Action::Complete("lock ".into()),
            Rank::Place,
        )
        .also("lock unlock pin installer limits locked"),
        Hit::new(
            "Status",
            "the device, its link and its lock (:status)",
            Action::Line("status".into()),
            Rank::Place,
        )
        .also("state info connection"),
    ];
    // The tools by their titles. The two that are also verbs of their own
    // (`subharm`, `tube`) are listed once, as those verbs.
    for tool in [
        Tool::Matrix,
        Tool::Loudness,
        Tool::Crossfeed,
        Tool::Psybass,
        Tool::Upmixer,
        Tool::Leveller,
        Tool::Signals,
        Tool::Stats,
        Tool::Monitor,
        Tool::AutoEq,
        Tool::Spectrum,
    ] {
        let keywords = match tool {
            Tool::Matrix => "routing route mix mixer crosspoint",
            Tool::Signals => "test tone noise sweep pink white sine",
            Tool::Spectrum => "rta analyzer analyser fft",
            Tool::AutoEq => "headphone headphones profile",
            Tool::Stats => "cpu load",
            Tool::Monitor => "notifications events",
            Tool::Leveller => "agc compressor",
            Tool::Psybass => "bass",
            Tool::Crossfeed => "xf xfeed headphone headphones",
            _ => "",
        };
        out.push(
            Hit::new(
                tool.title(),
                format!("a tool ({})", tool.key()),
                ev(ShellEvent::OpenTool(tool)),
                Rank::Place,
            )
            .also(keywords),
        );
    }
    out
}

/// The verbs: the interface's own, the grammar's that make sense here, and
/// the shell's that the `:` line now runs.
fn verbs() -> Vec<Hit> {
    let mut out: Vec<Hit> = crate::live::APP_VERBS
        .iter()
        .chain(crate::live::SHELL_VERBS)
        .map(|(name, detail)| {
            let action = if crate::live::verb_takes_words(name) {
                Action::Complete(format!("{name} "))
            } else {
                Action::Line((*name).to_string())
            };
            Hit::new(*name, *detail, action, Rank::Verb)
        })
        .collect();
    for (name, detail) in dspi_cmd::VERBS {
        if out.iter().any(|h| h.value == *name) {
            continue;
        }
        let action = match *name {
            "undo" | "redo" => Action::Line((*name).to_string()),
            _ => Action::Complete(format!("{name} ")),
        };
        out.push(Hit::new(*name, *detail, action, Rank::Verb));
    }
    out
}

/// Every parameter the device can read or write, by its path and its line.
fn parameters() -> Vec<Hit> {
    REGISTRY
        .iter()
        .filter(|d| d.set.is_some() || d.get.is_some())
        .map(|d| {
            let keywords = match d.path {
                "vol.mute" | "out.mute" => "silence quiet",
                "eq" | "eq.type" => "crossover xo filter band",
                "mix" => "routing route matrix",
                _ => "",
            };
            Hit::new(
                d.path,
                d.plain,
                Action::Complete(format!("{} ", d.path)),
                Rank::Parameter,
            )
            .also(keywords)
        })
        .collect()
}

/// Everything that matches `query`, best first; everything for an empty one.
pub fn search(query: &str, _ctx: &Context) -> Vec<Hit> {
    let q = words(query);
    let mut scored: Vec<(u32, usize, Hit)> = places()
        .into_iter()
        .chain(verbs())
        .chain(parameters())
        .enumerate()
        .filter_map(|(i, h)| h.score(&q).map(|s| (s, i, h)))
        .collect();
    // A path typed whole is the parameter meant; then places before verbs
    // before parameters, then the closer match; ties keep the order above.
    let typed = query.trim().to_lowercase();
    scored.sort_by_key(|(s, i, h)| {
        let whole = h.rank == Rank::Parameter && h.value == typed;
        (!whole, h.rank, *s, *i)
    });
    scored.into_iter().map(|(_, _, h)| h).collect()
}

/// The line the palette shows when nothing matches.
pub fn no_match(query: &str) -> String {
    format!("No match for \u{201c}{}\u{201d} · try : or ?", query.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context {
        Context {
            channel_slugs: Vec::new(),
            num_inputs: 0,
            num_outputs: 0,
            max_bands: 0,
        }
    }

    fn first(query: &str) -> Hit {
        search(query, &ctx())
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("nothing for {query}"))
    }

    /// UXP A13: the words the examiner typed found nothing; each finds its
    /// place now, first.
    #[test]
    fn utm_the_words_people_use_find_their_place_first() {
        assert_eq!(first("crossover").action, Action::Crossover);
        assert_eq!(first("cross").action, Action::Crossover);
        assert_eq!(first("mute").action, Action::Event(ShellEvent::VolumeMute));
        assert_eq!(first("snapshot").value, "Snapshots");
        assert_eq!(first("measure").value, "Measurements");
        assert_eq!(first("low pass").action, Action::Crossover);
        assert_eq!(first("high pass").action, Action::Crossover);
        assert_eq!(
            first("matrix").action,
            Action::Event(ShellEvent::OpenTool(Tool::Matrix))
        );
        assert_eq!(
            first("routing").action,
            Action::Event(ShellEvent::OpenTool(Tool::Matrix))
        );
        assert_eq!(first("help").action, Action::Help);
        assert_eq!(first("volume").action, Action::Volume);
        assert_eq!(first("lock").value, "Installer lock");
        assert_eq!(
            first("xf").action,
            Action::Event(ShellEvent::OpenTool(Tool::Crossfeed))
        );
    }

    /// A word inside a path finds the parameter: `vol.mute` by "mute".
    #[test]
    fn utm_a_word_inside_a_path_finds_the_parameter() {
        let hits = search("mute", &ctx());
        assert!(hits.iter().any(|h| h.value == "vol.mute"), "{hits:?}");
        assert_eq!(first("vol.mute").value, "vol.mute", "a path typed whole");
        let places = hits.iter().position(|h| h.value == "Mute").unwrap();
        let param = hits.iter().position(|h| h.value == "vol.mute").unwrap();
        assert!(places < param, "places first");
        assert_eq!(
            hits[param].action,
            Action::Complete("vol.mute ".into()),
            "a parameter goes to the : line for its value"
        );
    }

    #[test]
    fn utm_nothing_found_is_said() {
        assert!(search("zzzz", &ctx()).is_empty());
        assert_eq!(
            no_match("zzzz "),
            "No match for \u{201c}zzzz\u{201d} · try : or ?"
        );
    }

    /// An empty query lists everything, places first.
    #[test]
    fn utm_an_empty_query_lists_everything() {
        let hits = search("", &ctx());
        assert_eq!(hits[0].rank, Rank::Place);
        assert!(hits.iter().any(|h| h.rank == Rank::Parameter));
        assert!(hits.iter().any(|h| h.value == "import"));
    }
}
