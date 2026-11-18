//! WP PGPROTO: the vendored paged-protocol files were read completely.
//!
//! Every count here is compared with the `[protocol]` section of
//! `firmware/PROVENANCE.toml`, recorded when the JSON files were copied, never
//! with literals in a plan. A firmware bump that changes a count fails here and
//! names what moved (docs/terminal/firmware-bump.md): re-record the count only
//! after reading the change.

use std::collections::BTreeMap;

use dspi_proto::generated::ALL_OPCODES;
use dspi_proto::paged::legacy::{self, Adapter, LEGACY_OPS};
use dspi_proto::paged::registry::{self, COMMANDS, PAGES};
use dspi_proto::paged::{
    self, CmdId, EXT_PAGE, GetLen, PRIVATE_PAGES, PROTOCOL_INFO_LEN, PROTOCOL_MAJOR,
    SYS_PROTOCOL_INFO, cmds,
};

const PROVENANCE: &str = include_str!("../firmware/PROVENANCE.toml");

/// `section -> key -> value` for the `[protocol...]` tables (a tiny reader for
/// the subset this file uses: `key = int`, `key = "string"`, trailing comments).
fn provenance() -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut section = String::new();
    for line in PROVENANCE.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.to_string();
            continue;
        }
        if !section.starts_with("protocol") || line.starts_with('#') || line.is_empty() {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim();
        let v = if let Some(s) = v.strip_prefix('"') {
            s.split('"').next().unwrap_or("").to_string()
        } else {
            v.split('#').next().unwrap_or("").trim().to_string()
        };
        out.entry(section.clone())
            .or_default()
            .insert(k.trim().to_string(), v);
    }
    out
}

fn count(p: &BTreeMap<String, BTreeMap<String, String>>, section: &str, key: &str) -> usize {
    p.get(section)
        .and_then(|s| s.get(key))
        .unwrap_or_else(|| panic!("PROVENANCE.toml [{section}] has no {key}"))
        .parse()
        .unwrap_or_else(|_| panic!("PROVENANCE.toml [{section}] {key} is not a count"))
}

#[test]
fn wp_pgproto_the_generator_read_every_row() {
    assert!(
        legacy::UNREAD.is_empty(),
        "rows build.rs could not read (fix the generator or read the upstream change): {:?}",
        legacy::UNREAD
    );
}

#[test]
fn wp_pgproto_registry_counts_match_provenance() {
    let p = provenance();
    assert_eq!(PAGES.len(), count(&p, "protocol", "pages"));
    assert_eq!(COMMANDS.len(), count(&p, "protocol", "commands"));
    assert_eq!(
        COMMANDS.iter().filter(|c| c.get).count(),
        count(&p, "protocol", "get_commands")
    );
    assert_eq!(
        COMMANDS.iter().filter(|c| c.set).count(),
        count(&p, "protocol", "set_commands")
    );
    assert_eq!(
        usize::from(registry::VENDORED_MAJOR),
        count(&p, "protocol", "protocol_major")
    );
    assert_eq!(
        usize::from(registry::VENDORED_MINOR),
        count(&p, "protocol", "protocol_minor")
    );
    let per_page = p
        .get("protocol.commands_per_page")
        .expect("[protocol.commands_per_page]");
    assert_eq!(
        per_page.len(),
        PAGES.len(),
        "a page is missing from the counts"
    );
    for page in &PAGES {
        let have = COMMANDS.iter().filter(|c| c.page == page.mnemonic).count();
        assert_eq!(
            have,
            count(&p, "protocol.commands_per_page", page.mnemonic),
            "page {}",
            page.mnemonic
        );
    }
}

#[test]
fn wp_pgproto_legacy_counts_match_provenance() {
    let p = provenance();
    assert_eq!(
        usize::from(legacy::LEGACY_FORMAT),
        count(&p, "protocol", "legacy_format")
    );
    assert_eq!(LEGACY_OPS.len(), count(&p, "protocol", "opcodes"));
    let ins = LEGACY_OPS.iter().filter(|o| o.r#in.is_some()).count();
    let outs = LEGACY_OPS.iter().filter(|o| o.out.is_some()).count();
    assert_eq!(ins, count(&p, "protocol", "adapters_in"));
    assert_eq!(outs, count(&p, "protocol", "adapters_out"));
    assert_eq!(ins + outs, count(&p, "protocol", "adapters"));

    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    let mut cases = 0;
    for a in LEGACY_OPS.iter().flat_map(|o| [o.out, o.r#in]).flatten() {
        *kinds.entry(a.kind()).or_default() += 1;
        if let Adapter::Split(s) = a {
            cases += s.cases.len() + usize::from(s.default.is_some());
        }
    }
    let recorded = p
        .get("protocol.adapters_by_kind")
        .expect("[protocol.adapters_by_kind]");
    assert_eq!(kinds.len(), recorded.len(), "kinds {kinds:?}");
    for (k, n) in &kinds {
        assert_eq!(*n, count(&p, "protocol.adapters_by_kind", k), "kind {k}");
    }
    assert_eq!(cases, count(&p, "protocol", "split_cases"));
}

#[test]
fn wp_pgproto_provenance_names_the_pinned_commit() {
    let p = provenance();
    let s = &p["protocol"];
    assert_eq!(s["short"], "103ba37");
    assert!(s["commit"].starts_with(&s["short"]));
    assert_eq!(s["branch"], "release/v1.1.7");
}

/// The seam's literals (PLAN-PG section 2, "The seam") equal the generated table.
#[test]
fn wp_pgproto_seam_literals_are_the_generated_ones() {
    assert_eq!(SYS_PROTOCOL_INFO, CmdId(0x0110));
    assert_eq!(SYS_PROTOCOL_INFO, cmds::SYS_PROTOCOL_INFO);
    let probe = registry::command(SYS_PROTOCOL_INFO).expect("SYS.PROTOCOL_INFO");
    assert_eq!(probe.get_len, GetLen::Fixed(PROTOCOL_INFO_LEN));
    assert!(probe.get && !probe.set);
    assert_eq!(PROTOCOL_MAJOR, 1);
    assert_eq!(PROTOCOL_MAJOR, registry::VENDORED_MAJOR);
    // Our extension page is a private one and upstream allocates none of them.
    assert!(PRIVATE_PAGES.contains(&EXT_PAGE));
    assert!(PAGES.iter().all(|p| !PRIVATE_PAGES.contains(&p.page)));
    assert!(registry::page(paged::LEGACY_PAGE).is_none());
}

/// Every 1.1.6 opcode of our pinned headers is in the map, under the same name
/// and code, except our own `REQ_EXTENSION`, which is the planner's extension row.
#[test]
fn wp_pgproto_every_header_opcode_has_a_row() {
    for (name, code) in ALL_OPCODES {
        if name == "REQ_EXTENSION" {
            assert!(legacy::op(code).is_none());
            continue;
        }
        let op = legacy::op(code).unwrap_or_else(|| panic!("{name} 0x{code:02X} has no row"));
        assert_eq!(Some(op.name), name.strip_prefix("REQ_"), "0x{code:02X}");
    }
    assert_eq!(LEGACY_OPS.len() + 1, ALL_OPCODES.len());
}

/// No adapter targets page 0, and every target exists in the registry (build.rs
/// refuses otherwise; this states it).
#[test]
fn wp_pgproto_no_target_on_page_zero() {
    fn walk(a: &Adapter, out: &mut Vec<CmdId>) {
        match a {
            Adapter::Direct(d) => out.push(d.target),
            Adapter::ActValue(x) | Adapter::ActAddr(x) | Adapter::ActPacked(x) => {
                out.push(x.target)
            }
            Adapter::Split(s) => {
                for (_, c) in s.cases {
                    walk(c, out);
                }
                if let Some(d) = s.default {
                    walk(d, out);
                }
            }
            Adapter::Custom(c) => out.extend(c.uses.iter().map(|(id, _)| *id)),
        }
    }
    let mut targets = Vec::new();
    for a in LEGACY_OPS.iter().flat_map(|o| [o.out, o.r#in]).flatten() {
        walk(a, &mut targets);
    }
    assert!(!targets.is_empty());
    for t in targets {
        assert!(!t.is_legacy(), "{t:?}");
        assert!(registry::command(t).is_some(), "{t:?}");
    }
}
