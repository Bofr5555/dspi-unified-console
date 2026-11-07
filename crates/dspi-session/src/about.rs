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
//! legal text belongs here: licences are in `NOTICE.md`.
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
    /// Where to follow and support the work.
    pub links: Vec<Link>,
    /// Who and what else the work includes.
    pub credits: Vec<Credit>,
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
        assert_eq!(table.makers.len(), 2);
        assert_eq!(table.links.len(), 5);
        assert!(table.credits.iter().any(|c| c.name == "Jonathan Rosser"));
        let autoeq = table.credits.iter().find(|c| c.name == "AutoEq").unwrap();
        assert!(autoeq.url.is_some());
    }
}
