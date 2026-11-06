//! Wording every client shares (WP CG10): what to say for each [`LinkPhase`],
//! and the onboarding data.
//!
//! **Link wording.** Each entry has a stable key and its English. A client
//! localises by the key and falls back to the English. Keys are the phase names
//! `dspi_link_status` reports in JSON, so a client has one set, with the
//! `needs_pairing` key split by `pin_source`:
//! `needs_pairing.window`, `needs_pairing.fixed`. Keys are never reused.
//! The text says nothing about a particular hub; `close_reason` and the hub's
//! name are the client's to add.
//!
//! **Onboarding data.** `assets/onboarding/whatsnew.json` and
//! `getting_started.json`, built in, handed over as the JSON documents they
//! are. What was already read and when to show it is each client's own state.
//! `whatsnew.json` is `[{version, headline, items}]`, newest first, and is the
//! macOS console's `WhatsNew.json` as it is. In `getting_started.json`, an
//! object with a `when` is for clients where it holds (`can_install_firmware`,
//! `cannot_install_firmware`): an iOS client takes the second and so has no
//! `board` stage. `id`s are stable keys, as above. `{name}` in a text is the
//! client's to fill in; `icon` is a name the client maps to its own icon set.

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

/// `assets/onboarding/whatsnew.json`: see the module docs.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn onboarding_whats_new_json() -> String {
    WHATS_NEW_JSON.to_owned()
}

/// `assets/onboarding/getting_started.json`: see the module docs.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn onboarding_getting_started_json() -> String {
    GETTING_STARTED_JSON.to_owned()
}

fn entry(phase: LinkPhase, fixed_pin: bool) -> LinkPhaseText {
    let (key, text, action) = match phase {
        LinkPhase::Idle => ("idle", "Not connected to a hub", None),
        LinkPhase::NeedsPairing if fixed_pin => {
            ("needs_pairing.fixed", "Enter the hub's PIN.", None)
        }
        LinkPhase::NeedsPairing => (
            "needs_pairing.window",
            "Enter the PIN shown on the hub. On a DSPi Console, open Settings > Networking \
             and choose Allow a New Client to get one.",
            None,
        ),
        LinkPhase::NoDevice => ("no_device", "Connected to hub, no DSPi found", None),
        LinkPhase::ChooseDevice => ("choose_device", "Connected to hub, choose a DSPi", None),
        LinkPhase::Bound => ("bound", "Connected", None),
        LinkPhase::Reconnecting => (
            "reconnecting",
            "Connection lost, reconnecting",
            Some(("reconnecting.disconnect", "Disconnect")),
        ),
        LinkPhase::Closed => ("closed", "The connection to the hub has ended", None),
        LinkPhase::Refused => ("refused", "The hub refused this connection", None),
    };
    LinkPhaseText {
        key: key.to_owned(),
        phase: link_phase_key(phase).to_owned(),
        text: text.to_owned(),
        action_key: action.map(|(k, _)| k.to_owned()),
        action: action.map(|(_, a)| a.to_owned()),
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
        struct Release {
            version: String,
            headline: String,
            items: Vec<String>,
        }
        let releases: Vec<Release> = serde_json::from_str(&onboarding_whats_new_json()).unwrap();
        assert!(!releases.is_empty());
        for r in &releases {
            assert!(!r.headline.is_empty(), "{}", r.version);
            assert!(!r.items.is_empty(), "{}", r.version);
            assert!(r.items.iter().all(|i| !i.is_empty()), "{}", r.version);
        }
        let orders: Vec<_> = releases.iter().map(|r| order(&r.version)).collect();
        assert!(orders.windows(2).all(|w| w[0] > w[1]), "{orders:?}");
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
        let doc: Value = serde_json::from_str(&onboarding_getting_started_json()).unwrap();
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
                if let Some(when) = o.get("when").and_then(Value::as_str) {
                    assert!(
                        ["can_install_firmware", "cannot_install_firmware"].contains(&when),
                        "{when}"
                    );
                }
            }
            _ => {}
        });

        let stages = doc["stages"].as_array().unwrap();
        let stage_ids: Vec<&str> = stages.iter().map(|s| s["id"].as_str().unwrap()).collect();
        assert_eq!(stage_ids, ["welcome", "board", "done"]);
        assert_eq!(stages[1]["when"], "can_install_firmware");
    }

    /// A client that cannot install firmware reads a whole wizard that never
    /// asks it to, and one that can reads none of the other's lines.
    #[test]
    fn wp_cg10_getting_started_reads_whole_with_and_without_install() {
        let doc: Value = serde_json::from_str(&onboarding_getting_started_json()).unwrap();
        let shown = |item: &Value, can: &str| item.get("when").is_none_or(|w| w == can);
        for can in ["can_install_firmware", "cannot_install_firmware"] {
            let stages = doc["stages"].as_array().unwrap();
            let stages: Vec<&Value> = stages.iter().filter(|s| shown(s, can)).collect();
            assert_eq!(stages.len(), if can.starts_with("can_") { 3 } else { 2 });
            for stage in stages {
                if stage["id"] == "board" {
                    continue;
                }
                for list in ["paragraphs", "rows"] {
                    let items = stage[list].as_array().unwrap();
                    assert!(items.iter().any(|i| shown(i, can)), "{list} for {can}");
                }
            }
        }
    }
}
