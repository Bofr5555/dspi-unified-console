//! Who made what (WP CG15): the About table, for a client that does not link
//! `dspi-core`.
//!
//! `assets/about.json` is the only copy of these words, and this is the only place
//! that reads the file. `dspi_core::about_json` serves it to the clients on the core;
//! the Terminal reads [`about`] here, so it needs no copy either. The file holds no
//! placeholder, because this route has no renderer; a test in `dspi_core::link_text`
//! fails if one is added.
//!
//! **The wording is a draft** until the owner approves it (`draft` in the file). No
//! legal text belongs here but the project's own licence statement (`licence`, WP
//! LIC1): the owner's GPL-3.0-or-later notice and section 7 terms, which every client
//! shows in its About. The same words are in `NOTICE.md`, `apps/android/NOTICE.md`
//! and the packages' `TERMS.txt`; a test holds them equal. Third-party licences stay
//! in `NOTICE.md`.
//!
//! Sources: makers and credits from `NOTICE.md` and `docs/licensing/`, links from the
//! Swift console's About (`DSPi_ConsoleApp.swift:1042-1050`), their details from
//! `AboutPage.qml`.

use serde::Deserialize;

/// The table, as JSON. Its fields are those of [`About`].
pub const ABOUT_JSON: &str = include_str!("../../../assets/about.json");

/// The whole About table.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct About {
    /// True until the owner approves the wording.
    pub draft: bool,
    /// The lines to show under the app's name.
    pub credit_lines: Vec<CreditLine>,
    /// Who made what.
    pub makers: Vec<Maker>,
    /// The heading of [`links`](Self::links), which says whose links they are
    /// (WP CG29).
    pub links_title: String,
    /// Shown under that heading: whose channels these are, and who made this console.
    pub links_note: String,
    /// Where to follow and support the work.
    pub links: Vec<Link>,
    /// Who and what else the work includes.
    pub credits: Vec<Credit>,
    // -- WP LIC1 --
    /// The project's licence statement and its additional terms.
    pub licence: Licence,
}

// -- WP LIC1 --
/// The project's licence: GPL-3.0-or-later with additional terms under its section 7.
/// Every client shows all of it where it shows licences.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Licence {
    /// Stable key of the wording.
    pub id: String,
    /// The section's heading.
    pub title: String,
    /// The SPDX expression.
    pub spdx: String,
    /// Who holds the copyright in what, a line each.
    pub copyright: Vec<String>,
    /// The licence notice.
    pub statement: String,
    /// The heading of [`terms`](Self::terms).
    pub terms_title: String,
    /// What the terms apply to.
    pub terms_note: String,
    /// The additional terms, in order.
    pub terms: Vec<Term>,
    /// What stays allowed.
    pub freedoms: String,
    /// Where the third-party licences are.
    pub notices: String,
}

/// One additional term.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Term {
    /// Stable key.
    pub id: String,
    /// Its name and the paragraph of section 7 it uses.
    pub title: String,
    /// The term.
    pub text: String,
}

/// One line under the app's name.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreditLine {
    /// Stable key of `text`.
    pub id: String,
    /// The English.
    pub text: String,
}

/// One maker and their parts.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Maker {
    /// Stable key.
    pub id: String,
    /// The maker's name.
    pub name: String,
    /// The maker's page.
    pub url: String,
    /// What they made.
    pub made: Vec<String>,
}

/// One link.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    /// Stable key.
    pub id: String,
    /// The link's name.
    pub title: String,
    /// What is there.
    pub detail: String,
    /// The address.
    pub url: String,
}

/// One credit.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credit {
    /// Stable key.
    pub id: String,
    /// Who or what is credited.
    pub name: String,
    /// What for.
    pub what: String,
    /// Its page, where the sources give one.
    #[serde(default)]
    pub url: Option<String>,
}

/// The table, parsed. A test holds that the built-in file parses, so an error here
/// means a broken build of this crate.
pub fn about() -> Result<About, serde_json::Error> {
    serde_json::from_str(ABOUT_JSON)
}

#[cfg(test)]
mod tests_wp_cg15 {
    use super::*;

    #[test]
    fn wp_cg15_the_terminal_route_reads_the_whole_table() {
        let table = about().unwrap();
        assert!(!table.credit_lines.is_empty());
        assert!(!table.makers.is_empty());
        assert!(!table.links.is_empty());
        assert!(table.credits.iter().any(|c| c.name == "Jonathan Rosser"));
        let autoeq = table.credits.iter().find(|c| c.name == "AutoEq").unwrap();
        assert!(autoeq.url.is_some());
    }
}

#[cfg(test)]
mod tests_wp_cg29 {
    use super::*;

    /// The links are Weeb Labs', and the table says so: every client shows this title
    /// and note over them.
    #[test]
    fn wp_cg29_the_links_section_is_weeb_labs() {
        let table = about().unwrap();
        assert_eq!(table.links_title, "Weeb Labs: links and support");
        assert_eq!(
            table.links_note,
            "Weeb Labs makes DSPi, its firmware and the original DSPi Consoles; these are their \
             channels, and Patreon and Ko-fi are where to support their work. This console is \
             made by Bofr5555, not Weeb Labs."
        );
        assert!(!table.links.is_empty());
    }
}

#[cfg(test)]
mod tests_wp_lic1 {
    use super::*;

    /// `text` with runs of whitespace as single spaces and Markdown's emphasis removed,
    /// so a paragraph reads the same however a file wraps it.
    fn flat(text: &str) -> String {
        text.replace("**", "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Every copy of the statement and the terms, where users and packagers read them.
    const COPIES: &[(&str, &str)] = &[
        ("NOTICE.md", include_str!("../../../NOTICE.md")),
        (
            "apps/android/NOTICE.md",
            include_str!("../../../apps/android/NOTICE.md"),
        ),
        (
            "apps/desktop/packaging/licences/TERMS.txt",
            include_str!("../../../apps/desktop/packaging/licences/TERMS.txt"),
        ),
    ];

    #[test]
    fn wp_lic1_the_licence_is_gpl3_or_later_with_three_terms() {
        let l = about().unwrap().licence;
        // -- WP CLEAN1 -- the program as a whole is GPL-3.0; Bofr5555's parts "or later"
        assert_eq!(l.spdx, "GPL-3.0-only");
        assert!(
            l.statement
                .contains("either version 3 of the License, or (at your option) any later version")
        );
        let ids: Vec<_> = l.terms.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            ["attributions", "modified_versions", "no_trademark_rights"]
        );
        // The attributions the first term names are the lines every About shows
        let table = about().unwrap();
        for line in &table.credit_lines {
            assert!(
                l.terms[0].text.contains(&format!("\"{}\"", line.text)),
                "{} is not in the terms",
                line.text
            );
        }
    }

    /// One statement for the whole project: each copy holds the table's words.
    #[test]
    fn wp_lic1_every_copy_says_the_same() {
        let l = about().unwrap().licence;
        let mut parts = vec![
            l.statement.clone(),
            l.terms_note.clone(),
            l.freedoms.clone(),
        ];
        parts.extend(l.terms.iter().map(|t| format!("{}. {}", t.title, t.text)));
        parts.extend(l.copyright.iter().cloned());
        for (path, text) in COPIES {
            let text = flat(text);
            for part in &parts {
                assert!(text.contains(&flat(part)), "{path} does not say: {part}");
            }
        }
    }
}

// -- WP CLEAN1 --
#[cfg(test)]
mod tests_wp_clean1 {
    use super::*;

    /// The texts are drafts until the owner approves them, and say so.
    #[test]
    fn wp_clean1_the_notices_are_marked_draft() {
        assert!(about().unwrap().draft);
        for (path, text) in [
            ("NOTICE.md", include_str!("../../../NOTICE.md")),
            (
                "apps/android/NOTICE.md",
                include_str!("../../../apps/android/NOTICE.md"),
            ),
            (
                "apps/desktop/packaging/licences/TERMS.txt",
                include_str!("../../../apps/desktop/packaging/licences/TERMS.txt"),
            ),
        ] {
            assert!(text.contains("DRAFT"), "{path} is not marked as a draft");
        }
    }

    /// The source pointer is the release's own archive, not a repository link.
    #[test]
    fn wp_clean1_the_source_is_the_release_archive() {
        let l = about().unwrap().licence;
        assert!(l.notices.contains("-source.tar.gz"), "{}", l.notices);
        assert!(!l.notices.contains("github.com"), "{}", l.notices);
    }
}
