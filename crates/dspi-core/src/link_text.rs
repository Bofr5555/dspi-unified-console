//! Wording every client shares (WP CG10): what to say for each [`LinkPhase`],
//! and the onboarding data.
//!
//! **Link wording.** Each entry has a stable key and its English. A client
//! localises by the key and falls back to the English. Keys are the phase names
//! `dspi_link_status` reports in JSON, so a client has one set, with the
//! `needs_pairing` key split by `pin_source`:
//! `needs_pairing.window`, `needs_pairing.fixed`. Keys are never reused.
//! The text says nothing about a particular hub; `close_reason` and the hub's
//! name are the client's to add. No term in it differs by platform.
//! The sentences themselves are in `dspi_session::link_words`, so the Terminal,
//! which has no `dspi-core`, says the same thing; this module is their interface.
//!
//! **Onboarding data.** `assets/onboarding/whatsnew.json` and
//! `getting_started.json`, built in. The functions return JSON resolved for the
//! platform this library was compiled for ([`Platform::COMPILED`]): a client sees
//! neither the syntax below nor a choice of platform.
//! - What's New is `[{version, headline, items}]`, newest first, plain strings.
//!   On macOS it is the Swift console's `WhatsNew.json`, byte for byte.
//! - In Getting Started `id`s are stable keys, as above, and `icon` is a name the
//!   client maps to its own icon set. `{name}` in a text is a value only the client
//!   knows at run time (`{chip}`, `{version}`), for it to fill in.
//! - What was already read and when to show it is each client's own state.
//!
//! **About data** (WP CG15). `assets/about.json`, through [`about_json`]: who made
//! what, the links and the credits. The Terminal reads the same file through
//! `dspi_session::about`, without this renderer, so the file holds no placeholder.
//!
//! **The syntax in the assets**, resolved here and nowhere else:
//! - `[[term]]`: a word that differs by platform. See [`term`].
//! - `[[shortcut:Shift+D]]`: " (Cmd+Shift+D)" or " (Ctrl+Shift+D)", and nothing on
//!   a touch platform. The space is its own, so write it against the word before.
//! - `[[if condition:text]]`: `text` where the condition holds, else nothing. It
//!   holds no placeholder.
//! - `"when": "condition"` on an object in an array: the object is left out where
//!   the condition does not hold. An object left with `text` alone becomes a string.
//!
//! Conditions are in [`Platform::holds`]. Add a term or condition only when a text
//! needs one.
//!
//! **No text names what the user is holding** (owner: "The users generally know if
//! they are using a computer or a phone"). Say the thing without the noun. Only
//! macOS keeps such words, because there the text is Weeb Labs' original.

use dspi_session::link_words;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::LinkPhase;

const WHATS_NEW_JSON: &str = include_str!("../../../assets/onboarding/whatsnew.json");
const GETTING_STARTED_JSON: &str = include_str!("../../../assets/onboarding/getting_started.json");

/// Every phase, in the order of the wording table.
const PHASES: [LinkPhase; 8] = [
    LinkPhase::Idle,
    LinkPhase::NeedsPairing,
    LinkPhase::NoDevice,
    LinkPhase::ChooseDevice,
    LinkPhase::Bound,
    LinkPhase::Reconnecting,
    LinkPhase::Closed,
    LinkPhase::Refused,
];

/// What to tell the user about a [`LinkPhase`].
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPhaseText {
    /// Stable key of `text`, for localising: `no_device`, `needs_pairing.fixed`.
    pub key: String,
    /// The phase's name ([`link_phase_key`]), also for an entry split by
    /// `pin_source`.
    pub phase: String,
    /// The English.
    pub text: String,
    /// Stable key of `action`, if the phase has a button of its own.
    pub action_key: Option<String>,
    /// The English of that button.
    /// [`LinkPhase::Reconnecting`] has one, which calls `link_close`: the owner's
    /// rule is that a lost connection always offers the way out.
    pub action: Option<String>,
}

/// A phase's name, as `dspi_link_status` reports it in JSON.
pub fn link_phase_key(phase: LinkPhase) -> &'static str {
    match phase {
        LinkPhase::Idle => "idle",
        LinkPhase::NeedsPairing => "needs_pairing",
        LinkPhase::NoDevice => "no_device",
        LinkPhase::ChooseDevice => "choose_device",
        LinkPhase::Bound => "bound",
        LinkPhase::Reconnecting => "reconnecting",
        LinkPhase::Closed => "closed",
        LinkPhase::Refused => "refused",
    }
}

/// The phase with this name ([`link_phase_key`]); `None` for any other text.
pub fn link_phase_from_key(key: &str) -> Option<LinkPhase> {
    PHASES.into_iter().find(|p| link_phase_key(*p) == key)
}

/// The wording of `phase`. `pin_source` is [`crate::LinkStatus::pin_source`]:
/// it matters only to [`LinkPhase::NeedsPairing`], and anything but `fixed`
/// means `window`, as a hub that does not say does (spec 7.2).
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn link_phase_text(phase: LinkPhase, pin_source: String) -> LinkPhaseText {
    entry(phase, pin_source == "fixed")
}

/// The whole wording table: every key once, for a client's translation files.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn link_phase_texts() -> Vec<LinkPhaseText> {
    let mut table = Vec::with_capacity(PHASES.len().saturating_add(1));
    for phase in PHASES {
        table.push(entry(phase, false));
        if phase == LinkPhase::NeedsPairing {
            table.push(entry(phase, true));
        }
    }
    table
}

/// The release notes for this build's platform: see the module docs.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn onboarding_whats_new_json() -> String {
    render_whats_new(Platform::COMPILED)
}

/// The Getting Started wizard's data for this build's platform: see the module
/// docs.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn onboarding_getting_started_json() -> String {
    render_getting_started(Platform::COMPILED)
}

// -- WP CG15 --
/// The About table as a JSON document. `draft` is true until the owner approves
/// the wording:
/// `{draft, credit_lines: [{id, text}], makers: [{id, name, url, made}],
/// links: [{id, title, detail, url}], credits: [{id, name, what, url?}]}`.
/// `id`s are stable keys, as above. Show it as it is; it holds no legal text.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn about_json() -> String {
    render_about(Platform::COMPILED)
}

fn render_about(platform: Platform) -> String {
    render_asset(dspi_session::about::ABOUT_JSON, platform)
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| "{}".to_owned())
}

/// What the wording in the assets depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Platform {
    MacOs,
    Windows,
    Linux,
    Android,
    Ios,
}

impl Platform {
    /// The platform this library is compiled for. Any other target reads as
    /// Linux, the plainest desktop wording.
    const COMPILED: Self = if cfg!(target_os = "macos") {
        Self::MacOs
    } else if cfg!(target_os = "windows") {
        Self::Windows
    } else if cfg!(target_os = "android") {
        Self::Android
    } else if cfg!(target_os = "ios") {
        Self::Ios
    } else {
        Self::Linux
    };

    /// A condition, `None` if there is no such condition.
    fn holds(self, condition: &str) -> Option<bool> {
        let mobile = matches!(self, Self::Android | Self::Ios);
        match condition {
            "macos" => Some(self == Self::MacOs),
            "not_macos" => Some(self != Self::MacOs),
            "desktop" => Some(!mobile),
            // -- WP QD32 -- Where the Qt Console is the app.
            "windows_or_linux" => Some(matches!(self, Self::Windows | Self::Linux)),
            "mobile" => Some(mobile),
            // iOS has no route to the bootloader (SW18).
            "can_install_firmware" => Some(self != Self::Ios),
            "cannot_install_firmware" => Some(self == Self::Ios),
            _ => None,
        }
    }
}

/// A `[[term]]`, `None` if there is no such term.
fn term(name: &str, platform: Platform) -> Option<&'static str> {
    use Platform::{Android, Ios, Linux, MacOs, Windows};
    let word = match (name, platform) {
        // The settings page that holds outputs and pins.
        ("hardware_page", MacOs | Android | Ios) => "Hardware",
        ("hardware_page", Windows | Linux) => "Outputs",
        // -- WP QD32 -- The menu that holds the effect windows. Qt's has its own.
        ("effects_menu", MacOs | Android | Ios) => "Tools",
        ("effects_menu", Windows | Linux) => "Effects",
        // Only in `desktop` lines: mobile systems have no such choice.
        ("sound_settings", MacOs) => "macOS Sound settings",
        ("sound_settings", Windows) => "Windows Sound settings",
        ("sound_settings", Linux) => "your desktop's sound settings",
        _ => return None,
    };
    Some(word)
}

/// What is between `[[` and `]]`, `None` if it means nothing.
fn expand(inner: &str, platform: Platform) -> Option<String> {
    if let Some(rest) = inner.strip_prefix("if ") {
        let (condition, text) = rest.split_once(':')?;
        let shown = platform.holds(condition)?;
        return Some(if shown {
            text.to_owned()
        } else {
            String::new()
        });
    }
    if let Some(keys) = inner.strip_prefix("shortcut:") {
        return Some(match platform {
            Platform::MacOs => format!(" (Cmd+{keys})"),
            Platform::Windows | Platform::Linux => format!(" (Ctrl+{keys})"),
            Platform::Android | Platform::Ios => String::new(),
        });
    }
    term(inner, platform).map(str::to_owned)
}

/// A text with its placeholders replaced. One that means nothing stays as it is,
/// which the tests look for.
fn render_text(text: &str, platform: Platform) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((before, after)) = rest.split_once("[[") {
        out.push_str(before);
        let expanded = after
            .split_once("]]")
            .and_then(|(inner, tail)| Some((expand(inner, platform)?, tail)));
        if let Some((words, tail)) = expanded {
            out.push_str(&words);
            rest = tail;
        } else {
            out.push_str("[[");
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// A document with every text resolved and every `when` applied; `None` for an
/// object whose `when` does not hold, or that names no condition.
fn render_value(value: Value, platform: Platform) -> Option<Value> {
    match value {
        Value::String(s) => Some(Value::String(render_text(&s, platform))),
        Value::Array(items) => Some(Value::Array(
            items
                .into_iter()
                .filter_map(|v| render_value(v, platform))
                .collect(),
        )),
        Value::Object(mut fields) => {
            if let Some(when) = fields.remove("when") {
                let shown = when.as_str().and_then(|c| platform.holds(c));
                if shown != Some(true) {
                    return None;
                }
            }
            if fields.len() == 1
                && let Some(text) = fields.get("text")
            {
                return render_value(text.clone(), platform);
            }
            let fields = fields
                .into_iter()
                .filter_map(|(k, v)| Some((k, render_value(v, platform)?)))
                .collect();
            Some(Value::Object(fields))
        }
        other => Some(other),
    }
}

/// One release in What's New. Typed so that its fields keep the Swift file's order.
#[derive(Serialize, Deserialize)]
struct Release {
    version: String,
    headline: String,
    items: Vec<String>,
}

/// An asset is valid JSON: the tests parse and render it for every platform. If it
/// ever were not, a client gets an empty document rather than a failure over text.
fn render_asset(asset: &str, platform: Platform) -> Option<Value> {
    render_value(serde_json::from_str(asset).ok()?, platform)
}

fn render_whats_new(platform: Platform) -> String {
    let releases = render_asset(WHATS_NEW_JSON, platform)
        .and_then(|v| serde_json::from_value::<Vec<Release>>(v).ok())
        .and_then(|r| serde_json::to_string_pretty(&r).ok());
    releases.map_or_else(|| "[]".to_owned(), |text| text + "\n")
}

fn render_getting_started(platform: Platform) -> String {
    render_asset(GETTING_STARTED_JSON, platform)
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| "{}".to_owned())
}

fn entry(phase: LinkPhase, fixed_pin: bool) -> LinkPhaseText {
    let words = match phase {
        LinkPhase::Idle => link_words::IDLE,
        LinkPhase::NeedsPairing if fixed_pin => link_words::NEEDS_PAIRING_FIXED,
        LinkPhase::NeedsPairing => link_words::NEEDS_PAIRING_WINDOW,
        LinkPhase::NoDevice => link_words::NO_DEVICE,
        LinkPhase::ChooseDevice => link_words::CHOOSE_DEVICE,
        LinkPhase::Bound => link_words::BOUND,
        LinkPhase::Reconnecting => link_words::RECONNECTING,
        LinkPhase::Closed => link_words::CLOSED,
        LinkPhase::Refused => link_words::REFUSED,
    };
    LinkPhaseText {
        key: words.key.to_owned(),
        phase: link_phase_key(phase).to_owned(),
        text: words.text.to_owned(),
        action_key: words.action.map(|a| a.key.to_owned()),
        action: words.action.map(|a| a.text.to_owned()),
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg10 {
    use std::collections::BTreeSet;

    use serde::Deserialize;
    use serde_json::Value;

    use super::*;

    /// The variants of `LinkPhase` as `records.rs` declares them: a new one
    /// that is missing from [`PHASES`] fails here.
    fn declared_phases() -> Vec<String> {
        let src = include_str!("records.rs");
        let body = src.split("pub enum LinkPhase {").nth(1).unwrap();
        let body = body.split("\n}").next().unwrap();
        body.lines()
            .filter_map(|l| l.strip_prefix("    ")?.strip_suffix(','))
            .filter(|l| !l.starts_with('/') && !l.starts_with(' '))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn wp_cg10_every_phase_has_text() {
        let listed: Vec<String> = PHASES.iter().map(|p| format!("{p:?}")).collect();
        assert_eq!(listed, declared_phases());
        for phase in PHASES {
            for source in ["", "window", "fixed", "something new"] {
                let t = link_phase_text(phase, source.to_owned());
                assert!(!t.text.is_empty(), "{phase:?}");
                assert!(t.key.starts_with(link_phase_key(phase)), "{phase:?}");
                assert_eq!(t.phase, link_phase_key(phase));
            }
        }
    }

    #[test]
    fn wp_cg10_keys_are_the_phase_names_of_the_c_abi() {
        for (phase, name) in [
            (LinkPhase::Idle, "idle"),
            (LinkPhase::NeedsPairing, "needs_pairing"),
            (LinkPhase::NoDevice, "no_device"),
            (LinkPhase::ChooseDevice, "choose_device"),
            (LinkPhase::Bound, "bound"),
            (LinkPhase::Reconnecting, "reconnecting"),
            (LinkPhase::Closed, "closed"),
            (LinkPhase::Refused, "refused"),
        ] {
            assert_eq!(link_phase_key(phase), name);
            assert_eq!(link_phase_from_key(name), Some(phase));
        }
        assert_eq!(link_phase_from_key("connecting"), None);
        assert_eq!(link_phase_from_key(""), None);
    }

    #[test]
    fn wp_cg10_needs_pairing_depends_on_pin_source_only() {
        let pairing = |s: &str| link_phase_text(LinkPhase::NeedsPairing, s.to_owned());
        assert_eq!(pairing("fixed").key, "needs_pairing.fixed");
        assert_eq!(pairing("window").key, "needs_pairing.window");
        // A hub that does not say, and no session, read as `window` (spec 7.2).
        assert_eq!(pairing(""), pairing("window"));
        assert_eq!(pairing("something new"), pairing("window"));
        assert_ne!(pairing("fixed").text, pairing("window").text);
        for phase in PHASES {
            if phase != LinkPhase::NeedsPairing {
                assert_eq!(
                    link_phase_text(phase, "fixed".to_owned()),
                    link_phase_text(phase, "window".to_owned())
                );
            }
        }
    }

    #[test]
    fn wp_cg10_the_owners_words() {
        let no_device = link_phase_text(LinkPhase::NoDevice, String::new());
        assert_eq!(no_device.text, "Connected to hub, no DSPi found");

        let lost = link_phase_text(LinkPhase::Reconnecting, String::new());
        assert!(
            lost.text
                .eq_ignore_ascii_case("connection lost, reconnecting")
        );
        assert_eq!(lost.action_key.as_deref(), Some("reconnecting.disconnect"));
        assert_eq!(lost.action.as_deref(), Some("Disconnect"));
    }

    #[test]
    fn wp_cg10_the_table_has_every_key_once() {
        let table = link_phase_texts();
        let keys: Vec<&str> = table.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "idle",
                "needs_pairing.window",
                "needs_pairing.fixed",
                "no_device",
                "choose_device",
                "bound",
                "reconnecting",
                "closed",
                "refused",
            ]
        );
        for t in &table {
            assert_eq!(t.action.is_some(), t.action_key.is_some(), "{}", t.key);
        }
    }

    const ALL: [Platform; 5] = [
        Platform::MacOs,
        Platform::Windows,
        Platform::Linux,
        Platform::Android,
        Platform::Ios,
    ];

    const NOT_MACOS: [Platform; 4] = [
        Platform::Windows,
        Platform::Linux,
        Platform::Android,
        Platform::Ios,
    ];

    /// The Swift console's file, `ext-channel-116` at `48cf7c5`, unchanged.
    const SWIFT_WHATS_NEW: &str =
        include_str!("../../../tests/fixtures/onboarding/WhatsNew.swift-48cf7c5.json");

    fn strings(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => out.push(s.clone()),
            Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
            Value::Object(o) => o.values().for_each(|x| strings(x, out)),
            _ => {}
        }
    }

    /// Both rendered documents' strings, parsed back.
    fn rendered(platform: Platform) -> Vec<String> {
        let mut out = Vec::new();
        for doc in [render_whats_new(platform), render_getting_started(platform)] {
            strings(&serde_json::from_str(&doc).unwrap(), &mut out);
        }
        assert!(out.len() > 50, "{platform:?}");
        out
    }

    fn all_text(platform: Platform) -> String {
        rendered(platform).join("\n")
    }

    #[test]
    fn wp_cg10_compiled_platform_is_this_host() {
        // The containers that run these tests are Linux.
        if cfg!(target_os = "linux") {
            assert_eq!(Platform::COMPILED, Platform::Linux);
        }
        assert_eq!(
            onboarding_whats_new_json(),
            render_whats_new(Platform::COMPILED)
        );
        assert_eq!(
            onboarding_getting_started_json(),
            render_getting_started(Platform::COMPILED)
        );
    }

    #[test]
    fn wp_cg10_whats_new_on_macos_is_the_swift_file_byte_for_byte() {
        assert_eq!(render_whats_new(Platform::MacOs), SWIFT_WHATS_NEW);
    }

    #[test]
    fn wp_cg10_wizard_on_macos_says_what_swift_says() {
        let text = all_text(Platform::MacOs);
        for line in [
            "Choose which outputs your build uses, and the pins that carry them, in Settings under Hardware.",
            "Pick the DSPi as the output device in macOS Sound settings to hear your computer through it.",
            "Click an input or output in the sidebar to edit its filters.",
            "Help holds release notes and links, and this wizard can be run again.",
            "Hold the BOOTSEL button while connecting your Pico-compatible device to your computer. Once detected, it will appear here.",
        ] {
            assert!(text.contains(line), "{line}");
        }
    }

    #[test]
    fn wp_cg10_no_placeholder_survives_on_any_platform() {
        for platform in ALL {
            for s in rendered(platform) {
                assert!(!s.contains("[[") && !s.contains("]]"), "{platform:?}: {s}");
            }
            let wizard = render_getting_started(platform);
            assert!(!wizard.contains("\"when\""), "{platform:?}");
        }
    }

    #[test]
    fn wp_cg10_an_unknown_placeholder_shows_up() {
        for text in [
            "a [[nope]] b",
            "a [[if nope:x]] b",
            "a [[if macos]] b",
            "a [[x",
        ] {
            for platform in ALL {
                assert!(render_text(text, platform).contains("[["), "{text}");
            }
        }
        let unknown_when = serde_json::json!([{"id": "x", "when": "nope"}, {"id": "y"}]);
        let kept = render_value(unknown_when, Platform::Linux).unwrap();
        assert_eq!(kept, serde_json::json!([{"id": "y"}]));
    }

    /// Unknown `when`s are dropped silently when rendering, so check them here.
    #[test]
    fn wp_cg10_every_when_in_the_assets_is_known() {
        fn check(v: &Value) {
            match v {
                Value::Array(a) => a.iter().for_each(check),
                Value::Object(o) => {
                    if let Some(when) = o.get("when") {
                        let when = when.as_str().unwrap();
                        assert!(Platform::Linux.holds(when).is_some(), "{when}");
                    }
                    o.values().for_each(check);
                }
                _ => {}
            }
        }
        for asset in [WHATS_NEW_JSON, GETTING_STARTED_JSON] {
            check(&serde_json::from_str(asset).unwrap());
        }
    }

    #[test]
    fn wp_cg10_a_removed_condition_leaves_a_clean_sentence() {
        for platform in ALL {
            for s in rendered(platform) {
                for bad in ["  ", " ,", " .", ",,", "..", "( ", " )"] {
                    assert!(!s.contains(bad), "{platform:?}: {bad:?} in {s}");
                }
                assert!(
                    !s.starts_with(' ') && !s.ends_with(' '),
                    "{platform:?}: {s}"
                );
                // A sentence, not an id or an icon name, starts with a capital.
                let first = s.chars().next().unwrap();
                assert!(
                    !s.contains(' ') || !first.is_lowercase(),
                    "{platform:?}: {s}"
                );
            }
        }
    }

    #[test]
    fn wp_cg10_each_platform_has_its_words() {
        let mac = all_text(Platform::MacOs);
        for want in ["this Mac", "(Cmd+Shift+D)", "menu bar"] {
            assert!(mac.contains(want), "{want}");
        }
        let windows = all_text(Platform::Windows);
        let linux = all_text(Platform::Linux);
        for pc in [&windows, &linux] {
            for want in [
                "connected DSPi devices with browsers",
                "(Ctrl+Shift+D)",
                "under Outputs",
                "right-click",
            ] {
                assert!(pc.contains(want), "{want}");
            }
        }
        assert!(windows.contains("Windows Sound settings"));
        assert!(linux.contains("your desktop's sound settings"));
        for platform in NOT_MACOS {
            let text = all_text(platform);
            for gone in ["Cmd+", "this Mac", "macOS Sound", "menu bar"] {
                assert!(!text.contains(gone), "{platform:?}: {gone}");
            }
        }
        assert!(!windows.contains("macOS") && !linux.contains("macOS"));
        assert!(!all_text(Platform::Android).contains("macOS"));
        for platform in [Platform::Android, Platform::Ios] {
            let text = all_text(platform);
            for gone in [
                "Ctrl+",
                "right-click",
                "Click",
                "sidebar",
                "scroll to",
                "Sound settings",
            ] {
                assert!(!text.contains(gone), "{platform:?}: {gone}");
            }
            for want in [
                "touch and hold",
                "Tap an input",
                "pinch to change",
                "Connect the DSPi over USB to hear audio through it.",
            ] {
                assert!(text.contains(want), "{platform:?}: {want}");
            }
        }
    }

    /// The owner's rule: outside macOS, no text names what the user is holding.
    /// Whole words, so "Headphone" passes, and "device's" only where it is the
    /// DSPi's.
    #[test]
    fn wp_cg10_no_text_names_the_host_outside_macos() {
        const HOST: [&str; 6] = [
            "computer",
            "computers",
            "phone",
            "phones",
            "tablet",
            "tablets",
        ];
        const THE_DSPI: &str = "one device's spectrum analyser";
        for platform in NOT_MACOS {
            for s in rendered(platform) {
                let lower = s.to_lowercase();
                let words = lower.split(|c: char| !c.is_alphanumeric() && c != '\'');
                for word in words {
                    assert!(!HOST.contains(&word), "{platform:?}: {s}");
                }
                let without = lower.replace(THE_DSPI, "");
                assert!(!without.contains("device's"), "{platform:?}: {s}");
            }
        }
        // The check reads what it should: the original has them all on macOS.
        let mac = all_text(Platform::MacOs);
        for word in ["computer's", "phones", "your computer", THE_DSPI] {
            assert!(mac.contains(word), "{word}");
        }
    }

    /// A release's version as an order: a release comes after its betas.
    fn order(version: &str) -> (Vec<u32>, u32) {
        let (numbers, beta) = match version.split_once("-beta") {
            Some((n, b)) => (n, b.parse().unwrap()),
            None => (version, u32::MAX),
        };
        let numbers = numbers.split('.').map(|n| n.parse().unwrap()).collect();
        (numbers, beta)
    }

    #[test]
    fn wp_cg10_whats_new_is_well_formed_and_newest_first() {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Rendered {
            version: String,
            headline: String,
            items: Vec<String>,
        }
        for platform in ALL {
            let releases: Vec<Rendered> =
                serde_json::from_str(&render_whats_new(platform)).unwrap();
            assert!(!releases.is_empty());
            for r in &releases {
                assert!(!r.headline.is_empty(), "{}", r.version);
                assert!(!r.items.is_empty(), "{}", r.version);
                assert!(r.items.iter().all(|i| !i.is_empty()), "{}", r.version);
            }
            let orders: Vec<_> = releases.iter().map(|r| order(&r.version)).collect();
            assert!(orders.windows(2).all(|w| w[0] > w[1]), "{orders:?}");
        }
    }

    /// Walk a JSON document: `f` sees every object and string.
    fn walk(v: &Value, f: &mut impl FnMut(&Value)) {
        f(v);
        match v {
            Value::Array(a) => a.iter().for_each(|x| walk(x, f)),
            Value::Object(o) => o.values().for_each(|x| walk(x, f)),
            _ => {}
        }
    }

    /// The `{name}`s in a text.
    fn placeholders(text: &str) -> Vec<&str> {
        text.split('{')
            .skip(1)
            .map(|rest| rest.split('}').next().unwrap())
            .collect()
    }

    #[test]
    fn wp_cg10_getting_started_is_well_formed() {
        const FILLED_IN: [&str; 8] = [
            "current",
            "total",
            "chip",
            "volume",
            "version",
            "bundled_version",
            "device_version",
            "failure",
        ];
        // Ids over every platform's rows are unique within each.
        for platform in ALL {
            let doc: Value = serde_json::from_str(&render_getting_started(platform)).unwrap();
            let mut ids = BTreeSet::new();
            walk(&doc, &mut |v| match v {
                Value::String(s) => {
                    assert!(!s.is_empty());
                    for p in placeholders(s) {
                        assert!(FILLED_IN.contains(&p), "{{{p}}} in {s}");
                    }
                }
                Value::Object(o) => {
                    if let Some(id) = o.get("id").and_then(Value::as_str) {
                        assert!(ids.insert(id.to_owned()), "{id} twice");
                    }
                }
                _ => {}
            });
        }
    }

    /// A platform that cannot install firmware has no `board` stage and reads a
    /// whole wizard that never asks it to; the others have all three stages.
    #[test]
    fn wp_cg10_getting_started_reads_whole_with_and_without_install() {
        for platform in ALL {
            let doc: Value = serde_json::from_str(&render_getting_started(platform)).unwrap();
            let stages = doc["stages"].as_array().unwrap();
            let ids: Vec<&str> = stages.iter().map(|s| s["id"].as_str().unwrap()).collect();
            if platform == Platform::Ios {
                assert_eq!(ids, ["welcome", "done"]);
            } else {
                assert_eq!(ids, ["welcome", "board", "done"], "{platform:?}");
            }
            for stage in stages {
                if stage["id"] == "board" {
                    continue;
                }
                for list in ["paragraphs", "rows"] {
                    let items = stage[list].as_array().unwrap();
                    assert!(!items.is_empty(), "{platform:?}: {list}");
                }
            }
        }
        let ios = all_text(Platform::Ios);
        assert!(ios.contains("use uConsole for DSPi on macOS, Windows or Linux"));
        assert!(!ios.contains("right here"));
    }
}

// -- WP CG15 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg15 {
    use std::collections::BTreeSet;

    use serde_json::Value;

    use super::*;

    const ALL: [Platform; 5] = [
        Platform::MacOs,
        Platform::Windows,
        Platform::Linux,
        Platform::Android,
        Platform::Ios,
    ];

    /// What credits may be taken from.
    const SOURCES: [&str; 3] = [
        include_str!("../../../NOTICE.md"),
        include_str!("../../../docs/licensing/ios-inventory-notes.md"),
        include_str!("../../../docs/licensing/IOS-INVENTORY.md"),
    ];

    fn sourced(what: &str) -> bool {
        SOURCES.iter().any(|s| s.contains(what))
    }

    // -- WP LIC1 -- NOTICE.md wraps its paragraphs, so its statement is compared flat.
    static NOTICE_FLAT: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| flat(SOURCES[0]));

    fn flat(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn table() -> Value {
        serde_json::from_str(&about_json()).unwrap()
    }

    fn strings(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => out.push(s.clone()),
            Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
            Value::Object(o) => o.values().for_each(|x| strings(x, out)),
            _ => {}
        }
    }

    fn items<'a>(doc: &'a Value, list: &str) -> &'a Vec<Value> {
        doc[list].as_array().unwrap()
    }

    /// The field exists; its value and the words are the owner's to change, in the
    /// asset alone. `ci/licence-inventory.py` prints the state on every run.
    #[test]
    fn wp_cg15_has_the_draft_field_and_credit_lines() {
        let doc = table();
        assert!(doc["draft"].is_boolean());
        for line in items(&doc, "credit_lines") {
            assert!(!line["text"].as_str().unwrap().is_empty());
        }
        assert!(!items(&doc, "credit_lines").is_empty());
    }

    /// The links section names its owner: Qt and Android read these two fields from
    /// what the core serves, the Terminal from `dspi_session::about`.
    #[test]
    fn wp_cg29_the_served_table_says_whose_links_they_are() {
        let doc = table();
        assert_eq!(doc["links_title"], "Weeb Labs: links and support");
        let note = doc["links_note"].as_str().unwrap();
        assert!(note.contains("Bofr5555, not Weeb Labs"), "{note}");
    }

    /// The Terminal reads the file without the renderer, so what it reads and what
    /// the core serves must be the same on every platform. If this fails, a
    /// placeholder went in: the Terminal's route (`dspi_session::about`) has to learn
    /// it first.
    #[test]
    fn wp_cg15_the_terminal_and_the_core_read_the_same_table() {
        let raw: Value = serde_json::from_str(dspi_session::about::ABOUT_JSON).unwrap();
        for platform in ALL {
            let rendered: Value = serde_json::from_str(&render_about(platform)).unwrap();
            assert_eq!(rendered, raw, "{platform:?}");
        }
        assert!(!dspi_session::about::ABOUT_JSON.contains("[["));
        dspi_session::about::about().unwrap();
    }

    #[test]
    fn wp_cg15_who_made_what() {
        let doc = table();
        for name in ["Weeb Labs", "Bofr5555"] {
            let makers = items(&doc, "makers");
            let maker = makers.iter().find(|m| m["name"] == name).unwrap();
            assert!(!maker["made"].as_array().unwrap().is_empty(), "{name}");
        }
    }

    #[test]
    fn wp_cg15_every_credit_is_sourced() {
        let doc = table();
        let credits = items(&doc, "credits");
        for name in ["Jonathan Rosser", "AutoEq"] {
            assert!(credits.iter().any(|c| c["name"] == name), "{name}");
        }
        for credit in credits.iter().chain(items(&doc, "makers")) {
            let name = credit["name"].as_str().unwrap();
            assert!(sourced(name), "{name} is in no source");
            if let Some(url) = credit.get("url") {
                assert!(sourced(url.as_str().unwrap()), "{url} is in no source");
            }
        }
    }

    #[test]
    fn wp_cg15_ids_are_unique_and_links_are_https() {
        let doc = table();
        for list in ["credit_lines", "makers", "links", "credits"] {
            let mut ids = BTreeSet::new();
            for item in items(&doc, list) {
                assert!(ids.insert(item["id"].as_str().unwrap().to_owned()));
            }
        }
        for link in items(&doc, "links") {
            assert!(link["url"].as_str().unwrap().starts_with("https://"));
            assert!(!link["title"].as_str().unwrap().is_empty());
            assert!(!link["detail"].as_str().unwrap().is_empty());
        }
    }

    /// Owner's rules: no agent writes legal text, and no text names what the user
    /// is holding.
    #[test]
    fn wp_cg15_no_legal_text_and_no_host_nouns() {
        const LEGAL: [&str; 7] = [
            "licen",
            "copyright", // licence-inventory: not a notice (words a test forbids)
            "gpl",
            "warrant",
            "liab",
            "rights",
            "(c)",
        ];
        const HOST: [&str; 7] = [
            "computer",
            "computers",
            "phone",
            "phones",
            "tablet",
            "tablets",
            "desktop",
        ];
        let mut all = Vec::new();
        // -- WP LIC1 -- `licence` is the owner's own statement and terms (NOTICE.md),
        // which every client must show: the one legal text the table holds. Its words
        // are held equal to NOTICE.md's by dspi_session::about's tests.
        let mut doc = table();
        let licence = doc.as_object_mut().unwrap().remove("licence");
        let statement = licence
            .as_ref()
            .and_then(|l| l["statement"].as_str())
            .unwrap();
        assert!(
            NOTICE_FLAT.contains(&flat(statement)),
            "NOTICE.md does not say: {statement}"
        );
        strings(&doc, &mut all);
        assert!(all.len() > 20);
        for s in all {
            let lower = s.to_lowercase();
            for word in LEGAL {
                assert!(!lower.contains(word), "{s}");
            }
            for word in lower.split(|c: char| !c.is_alphanumeric()) {
                assert!(word != "mit" && !HOST.contains(&word), "{s}");
            }
        }
    }
}

// -- WP QD32 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_qd32 {
    use serde_json::Value;

    use super::*;

    const APP_MENU: &str = include_str!("../../../apps/desktop/qt/qml/components/AppMenu.qml");
    const SETTINGS: &str = include_str!("../../../apps/desktop/qt/qml/SettingsWindow.qml");
    const INPUTS_PAGE: &str =
        include_str!("../../../apps/desktop/qt/qml/settings/pages/InputsPage.qml");
    const GLOBAL_PAGE: &str =
        include_str!("../../../apps/desktop/qt/qml/settings/pages/GlobalParametersPage.qml");
    const SIDEBAR: &str = include_str!("../../../apps/desktop/qt/qml/Sidebar.qml");

    /// Named in the notes, and not in Qt until QD14 adds it.
    const PENDING_PAGES: [&str; 1] = ["Networking"];

    fn release_text(platform: Platform) -> String {
        let doc: Value = serde_json::from_str(&render_whats_new(platform)).unwrap();
        doc.as_array()
            .unwrap()
            .iter()
            .flat_map(|r| {
                let items = r["items"].as_array().unwrap().iter();
                items
                    .chain([&r["headline"]])
                    .map(|t| t.as_str().unwrap().to_owned())
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// QML with `qsTr("x")` read as `"x"` (WP QD23 wrapped the labels).
    fn plain(qml: &str) -> String {
        let mut out = String::with_capacity(qml.len());
        let mut rest = qml;
        while let Some(i) = rest.find("qsTr(\"") {
            out.push_str(&rest[..i]);
            let after = &rest[i + 5..];
            // The string literal, then the call's own closing parenthesis.
            match after[1..].find('"') {
                Some(end) => {
                    out.push_str(&after[..end + 2]);
                    rest = after[end + 2..]
                        .strip_prefix(')')
                        .unwrap_or(&after[end + 2..]);
                }
                None => {
                    out.push_str(after);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// Every `text: "..."` in a slice of QML, `qsTr` or not.
    fn quoted_after(qml: &str, field: &str) -> Vec<String> {
        plain(qml)
            .split(field)
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .map(str::to_owned)
            .collect()
    }

    /// The labels of an `AppMenu.qml` group, without the ellipsis.
    fn menu_items(group: &str) -> Vec<String> {
        let start = format!("{{ text: \"{group}\", icon");
        let menu = plain(APP_MENU);
        let body = menu.split(&start).nth(1).expect(group);
        let body = body.split("\n        ]").next().unwrap();
        quoted_after(body, "text: \"")
            .into_iter()
            .map(|t| t.trim_end_matches('\u{2026}').to_owned())
            .collect()
    }

    /// What follows each `root > ` in `text`.
    fn after<'a>(text: &'a str, root: &str) -> Vec<&'a str> {
        let marker = format!("{root} > ");
        text.split(&marker).skip(1).collect()
    }

    #[test]
    fn wp_qd32_every_menu_path_exists_in_qt() {
        for platform in [Platform::Windows, Platform::Linux] {
            let text = release_text(platform);
            let mut seen = 0;
            for group in ["Tools", "Effects", "Device"] {
                let items = menu_items(group);
                assert!(!items.is_empty(), "{group}");
                for rest in after(&text, group) {
                    assert!(
                        items.iter().any(|i| rest.starts_with(i.as_str())),
                        "{group} > {rest}"
                    );
                    seen += 1;
                }
            }
            let pages = quoted_after(SETTINGS, "title: \"");
            for rest in after(&text, "Settings") {
                assert!(
                    pages
                        .iter()
                        .map(String::as_str)
                        .chain(PENDING_PAGES)
                        .any(|p| rest.starts_with(p)),
                    "Settings > {rest}"
                );
                seen += 1;
            }
            assert!(seen >= 8, "{platform:?}: only {seen} paths");
        }
    }

    #[test]
    fn wp_qd32_sections_named_in_the_notes_are_on_qts_pages() {
        let text = release_text(Platform::Linux);
        assert!(text.contains("Settings > Inputs > Source Names"));
        assert!(INPUTS_PAGE.contains("title: qsTr(\"Source Names\")"));
        assert!(text.contains("Settings > Global Parameters > Device Name"));
        assert!(GLOBAL_PAGE.contains("title: qsTr(\"Device Name\")"));
        assert!(text.contains("Settings > Global Parameters has a Latency section"));
        assert!(GLOBAL_PAGE.contains("title: qsTr(\"Latency\")"));
    }

    /// Qt's source picker has no right-click: the sidebar connects no
    /// `onContextMenuRequested` to it. When it gets one, the condition can widen.
    #[test]
    fn wp_qd32_the_source_picker_right_click_is_macos_only() {
        let picker = SIDEBAR
            .split("objectName: \"sourcePicker\"")
            .nth(1)
            .unwrap();
        let picker = picker.split("onChosen:").next().unwrap();
        assert!(!picker.contains("onContextMenuRequested"));
        const LINE: &str = "right-click the source picker";
        assert!(release_text(Platform::MacOs).contains(LINE));
        for platform in [
            Platform::Windows,
            Platform::Linux,
            Platform::Android,
            Platform::Ios,
        ] {
            assert!(!release_text(platform).contains(LINE), "{platform:?}");
        }
    }

    #[test]
    fn wp_qd32_effects_are_under_effects_only_in_qt() {
        for (name, keys) in [
            ("Tube Modeller", "Shift+D"),
            ("Subharmonic Synthesizer", "Shift+S"),
        ] {
            let qt = format!("Effects > {name} (Ctrl+{keys})");
            let mac = format!("Tools > {name} (Cmd+{keys})");
            let mobile = format!("Tools > {name}.");
            assert!(release_text(Platform::Linux).contains(&qt));
            assert!(release_text(Platform::Windows).contains(&qt));
            assert!(release_text(Platform::MacOs).contains(&mac));
            assert!(release_text(Platform::Android).contains(&mobile));
            assert!(release_text(Platform::Ios).contains(&mobile));
        }
        // The analyser really is in Tools, in Qt as well.
        assert!(release_text(Platform::Linux).contains("Tools > Spectrum Analyser (Ctrl+Shift+A)"));
    }

    /// Qt's own release notes: what the other platforms' text does not say. The
    /// rest of its old entry is in the shared lines.
    #[test]
    fn wp_qd32_qts_entry_shows_on_windows_and_linux_only() {
        const QT_ONLY: [&str; 5] = [
            "The graph can also show phase.",
            "Control Surfaces wire buttons, knobs, encoders",
            ".dspipreset) shared with the other Consoles",
            "Stats for nerbs and Interrupt Monitor have been rebuilt",
            "Find it under Device > Firmware Update.",
        ];
        for line in QT_ONLY {
            assert!(release_text(Platform::Windows).contains(line), "{line}");
            assert!(release_text(Platform::Linux).contains(line), "{line}");
            for platform in [Platform::MacOs, Platform::Android, Platform::Ios] {
                assert!(
                    !release_text(platform).contains(line),
                    "{platform:?}: {line}"
                );
            }
        }
        // One release per version, so its notes stay under one heading.
        let doc: Value = serde_json::from_str(&render_whats_new(Platform::Linux)).unwrap();
        let beta4 = doc.as_array().unwrap().iter();
        assert_eq!(beta4.filter(|r| r["version"] == "1.1.6-beta4").count(), 1);
    }
}
