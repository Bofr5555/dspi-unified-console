//! WP P1NT: every error has a key and a row; the wording's rules.

use std::collections::BTreeSet;

use super::*;

const ALL: [Platform; 5] = [
    Platform::MacOs,
    Platform::Windows,
    Platform::Linux,
    Platform::Android,
    Platform::Ios,
];

/// The variants of `DspiError` as `error.rs` declares them.
fn declared_variants() -> Vec<String> {
    let src = include_str!("../error.rs");
    let body = src.split("pub enum DspiError {").nth(1).unwrap();
    let body = body.split("\n}").next().unwrap();
    body.lines()
        .filter_map(|l| l.strip_prefix("    "))
        .filter(|l| l.starts_with(|c: char| c.is_ascii_uppercase()))
        .map(|l| {
            l.split(|c: char| !c.is_alphanumeric())
                .next()
                .unwrap()
                .to_owned()
        })
        .collect()
}

/// One of each variant. A new variant missing here fails
/// `wp_p1nt_every_error_has_a_key_and_a_row`.
fn samples() -> Vec<DspiError> {
    let msg = || "words".to_owned();
    vec![
        DspiError::Disconnected,
        DspiError::Transfer { msg: msg() },
        DspiError::Refused { status: 4 },
        DspiError::InvalidArgument { msg: msg() },
        DspiError::Unsupported,
        DspiError::ReadOnly,
        DspiError::Locked,
        DspiError::Denied,
        DspiError::Link {
            code: "io".into(),
            msg: msg(),
        },
        DspiError::Io { msg: msg() },
        DspiError::AutoEq {
            code: "cancelled".into(),
            msg: msg(),
            retry_after_s: None,
        },
        DspiError::FirmwareGuardBlocked { msg: msg() },
        DspiError::OvervoltWordRequired { msg: msg() },
        // -- WP P1TK --
        DspiError::ConfirmationRequired {
            code: "missing".into(),
            msg: msg(),
        },
        // -- WP P1LM --
        DspiError::LimitExceeded {
            code: "out_gain".into(),
            msg: msg(),
        },
        // -- WP P1LK --
        DspiError::InstallerLock {
            code: "locked".into(),
            msg: msg(),
        },
        // -- WP P1DKFIX --
        DspiError::ImageRefused { msg: msg() },
        DspiError::LimitsUnknown { msg: msg() },
    ]
}

fn variant_name(e: &DspiError) -> String {
    let debug = format!("{e:?}");
    debug
        .split(|c: char| !c.is_alphanumeric())
        .next()
        .unwrap()
        .to_owned()
}

#[test]
fn wp_p1nt_every_error_has_a_key_and_a_row() {
    let sampled: Vec<String> = samples().iter().map(variant_name).collect();
    assert_eq!(
        sampled,
        declared_variants(),
        "add the new variant to samples()"
    );
    let mut keys = BTreeSet::new();
    for e in samples() {
        let key = e.notice_key();
        assert!(has_row(&key), "no row for {key} ({e:?})");
        assert_ne!(key, NOTICE_KEY_UNKNOWN, "{e:?}");
        assert!(keys.insert(e.kind_key()), "{key} twice");
        let n = notice_for(e.clone());
        assert_eq!(n.key, key);
        assert_eq!(n.detail, e.notice_detail());
    }
}

#[test]
fn wp_p1nt_codes_refine_the_key_where_the_table_has_a_row() {
    let dlink = |code: &str| DspiError::Link {
        code: code.into(),
        msg: "hub words".into(),
    };
    assert_eq!(dlink("wrong_pin").notice_key(), "dlink.wrong_pin");
    assert_eq!(dlink("still_reading").notice_key(), "dlink.still_reading");
    assert_eq!(dlink("something new").notice_key(), "dlink");
    let n = notice_for(dlink("wrong_pin"));
    assert_eq!(n.title, "Wrong PIN");
    assert_eq!(n.detail, "hub words");
    let autoeq = DspiError::AutoEq {
        code: "rate_limited".into(),
        msg: "later".into(),
        retry_after_s: Some(60),
    };
    assert_eq!(autoeq.notice_key(), "autoeq.rate_limited");
    // Every refined row is reachable from its code, and its family exists.
    for n in notice_texts() {
        if let Some((family, code)) = n.key.split_once('.') {
            // -- WP P1AFIX -- the configuration calls' own refusals: no
            // error variant makes these keys, `files` does
            if family == "config" {
                assert!(
                    [
                        crate::files::NOTICE_CONFIG_UNREADABLE,
                        crate::files::NOTICE_CONFIG_NOTHING_SELECTED,
                        crate::files::NOTICE_CONFIG_NOT_READ,
                    ]
                    .contains(&n.key.as_str()),
                    "{}: not a key files.rs answers",
                    n.key
                );
                continue;
            }
            // -- WP P1LM -- the limits profile's standing notices: no error
            // variant makes these keys, `limits` does
            if family == "limits" {
                assert!(
                    crate::limits::NOTICE_KEYS.contains(&n.key.as_str()),
                    "{}: not a key limits.rs answers",
                    n.key
                );
                continue;
            }
            // -- WP P1DCFIX -- the safe apply's countdown: no error makes
            // this key, the clients ask for it while a guard waits
            if family == "guard" {
                // -- WP P1WB2 -- and the hub's backstop beside it
                assert!(
                    [
                        crate::guarded::NOTICE_GUARD_WAITING,
                        crate::guarded::NOTICE_GUARD_HUB_BACKSTOP,
                    ]
                    .contains(&n.key.as_str()),
                    "{}: not a key guarded.rs names",
                    n.key
                );
                continue;
            }
            let e = match family {
                "dlink" => dlink(code),
                "autoeq" => DspiError::AutoEq {
                    code: code.into(),
                    msg: String::new(),
                    retry_after_s: None,
                },
                // -- WP P1LK --
                "installer_lock" => DspiError::InstallerLock {
                    code: code.into(),
                    msg: String::new(),
                },
                other => panic!("{other}: no family"),
            };
            assert_eq!(e.notice_key(), n.key);
            assert!(has_row(family), "{family}");
        }
    }
    // The owner's naming: DSPi Link is `dlink`, never bare `link`.
    assert!(notice_texts().iter().all(|n| !n.key.starts_with("link")));
}

#[test]
fn wp_p1nt_an_unknown_key_reads_as_the_unknown_row() {
    let n = notice_for_key("no such key".into(), "d".into());
    assert_eq!(n.key, NOTICE_KEY_UNKNOWN);
    assert_eq!(n.detail, "d");
    assert_eq!(
        notice_for_key("locked".into(), String::new()),
        notice_for(DspiError::Locked)
    );
}

/// Every row is whole on every platform: unique keys, a known kind, no empty
/// text, no placeholder left.
#[test]
fn wp_p1nt_the_table_is_whole_on_every_platform() {
    let raw: Value = serde_json::from_str(NOTICES_JSON).unwrap();
    let rows = raw["notices"].as_array().unwrap().len();
    for platform in ALL {
        let table = table_for(platform);
        assert_eq!(table.len(), rows, "{platform:?}");
        let mut keys = BTreeSet::new();
        for r in &table {
            assert!(keys.insert(r.key.clone()), "{} twice", r.key);
            for text in [&r.title, &r.reason, &r.action] {
                assert!(!text.is_empty(), "{}", r.key);
                assert!(!text.contains("[[") && !text.contains("]]"), "{text}");
            }
        }
        assert!(keys.contains(NOTICE_KEY_UNKNOWN));
    }
}

/// The owner's rules: no text names what the user is holding, on any platform
/// (the table is not the Mac's original, so macOS too), and there is no success
/// notice.
#[test]
fn wp_p1nt_no_host_nouns_and_no_success() {
    const HOST: [&str; 12] = [
        "computer",
        "computers",
        "phone",
        "phones",
        "tablet",
        "tablets",
        "desktop",
        "laptop",
        "mac",
        "pc",
        "iphone",
        "ipad",
    ];
    for platform in ALL {
        for r in table_for(platform) {
            for text in [&r.title, &r.reason, &r.action] {
                let lower = text.to_lowercase();
                for word in lower.split(|c: char| !c.is_alphanumeric() && c != '\'') {
                    assert!(!HOST.contains(&word), "{platform:?}: {text}");
                }
                assert!(!lower.contains("success"), "{text}");
            }
        }
    }
    let raw = NOTICES_JSON.to_lowercase();
    assert!(!raw.contains("\"success\""));
}

#[test]
fn wp_p1nt_kinds_serialise_as_words() {
    let n = notice_for(DspiError::Locked);
    let v = serde_json::to_value(&n).unwrap();
    assert_eq!(v["kind"], "warning");
    assert_eq!(v["key"], "locked");
    assert!(v.get("detail").is_some());
    assert_eq!(notice_for(DspiError::Disconnected).kind, NoticeKind::Error);
    let info = notice_for_key("dlink.still_reading".into(), String::new());
    assert_eq!(info.kind, NoticeKind::Info);
}

/// WP P1DCFIX (F7): the countdown sentence every client shows while a guard
/// waits: an info row, the seconds as `{s}` exactly once.
#[test]
fn wp_p1dcfix_the_countdown_has_one_sentence_with_the_seconds() {
    let n = notice_for_key(crate::guarded::NOTICE_GUARD_WAITING.into(), String::new());
    assert_eq!(n.key, "guard.waiting");
    assert_eq!(n.kind, NoticeKind::Info);
    assert_eq!(n.title, "Keep these settings?");
    assert_eq!(n.reason, "They go back in {s} s unless you press Keep.");
    let all = format!("{} {} {}", n.title, n.reason, n.action);
    assert_eq!(all.matches("{s}").count(), 1, "{all}");
}

/// WP P1WB2: the hub's backstop sentence beside the countdown, in shared
/// words (a console, never the host's own noun).
#[test]
fn wp_p1wb2_the_hub_backstop_has_its_row() {
    let n = notice_for_key(
        crate::guarded::NOTICE_GUARD_HUB_BACKSTOP.into(),
        String::new(),
    );
    assert_eq!(n.key, "guard.hub_backstop");
    assert_eq!(n.kind, NoticeKind::Info);
    assert_eq!(
        n.reason,
        "The hub puts them back too if this console goes away."
    );
}
