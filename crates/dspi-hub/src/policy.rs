//! The command table (`policy/commands.json`, spec 9.3) and what a role may do.
//!
//! Owned by WP HB2. WP HB0 has only what the app's path needs: the classes, since
//! they decide which commands count as writes, and the admin rule.
//!
//! `0x00` (`REQ_EXTENSION`) is not in upstream's table, so rule 7 of spec section
//! 10 makes it `config` in both directions. HB2 documents that for clients. Its IN
//! side reads (PLAN section 1.2, raw commands), so [`Policy::mutates`] says so.

use std::collections::BTreeMap;

use dspi_link::Dir;
use dspi_proto::generated::opcodes as op;
use serde::Deserialize;

use crate::session::Role;

/// The table this build ships.
const COMMANDS_JSON: &str = include_str!("../../../tests/fixtures/link/policy/commands.json");

/// How much authority a command needs (spec 6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Class {
    /// Returns state and changes nothing.
    Read,
    /// Changes listening state.
    Control,
    /// Changes the wiring or device-level flash config, or takes the device away.
    Config,
}

#[derive(Debug, Deserialize)]
struct Row {
    code: u8,
    class: Class,
    #[serde(default)]
    classes: BTreeMap<String, Class>,
}

#[derive(Debug, Deserialize)]
struct Table {
    commands: Vec<Row>,
}

/// The hub's command table.
#[derive(Debug)]
pub struct Policy {
    rows: BTreeMap<u8, Row>,
}

impl Policy {
    /// The table bundled with this build. A table that does not parse leaves every
    /// command `config` (rule 7), and a unit test keeps that from shipping.
    pub fn embedded() -> Self {
        match serde_json::from_str::<Table>(COMMANDS_JSON) {
            Ok(t) => Self {
                rows: t.commands.into_iter().map(|r| (r.code, r)).collect(),
            },
            Err(e) => {
                log::error!("the bundled command table does not parse: {e}");
                Self {
                    rows: BTreeMap::new(),
                }
            }
        }
    }

    /// The class of a command in one direction. A command the table does not list
    /// is `config` (spec 10, rule 7).
    pub fn classify(&self, code: u8, dir: Dir) -> Class {
        let Some(row) = self.rows.get(&code) else {
            return Class::Config;
        };
        let key = match dir {
            Dir::Set => "set",
            Dir::Get => "get",
        };
        row.classes.get(key).copied().unwrap_or(row.class)
    }

    /// Whether a command can change the device, whichever direction carries it
    /// (write-as-read included). Such a command is attributed to its session and
    /// is never repeated by the hub's busy retry.
    pub fn mutates(&self, code: u8, dir: Dir) -> bool {
        if code == op::REQ_EXTENSION && dir == Dir::Get {
            return false;
        }
        self.classify(code, dir) != Class::Read
    }

    /// Whether this role may issue this command (spec 6.3).
    pub fn allows(&self, role: Role, code: u8, dir: Dir) -> bool {
        match role {
            Role::Admin => true,
            Role::Viewer | Role::Control => {
                let _ = (code, dir);
                todo!("WP HB2")
            }
        }
    }

    /// The effective denial list for a role (`policy.denied`, spec 7.3), as
    /// `[bRequest, dir]` pairs.
    pub fn denied(&self, role: Role) -> Vec<[u16; 2]> {
        let _ = role;
        todo!("WP HB2")
    }
}

#[cfg(test)]
mod tests_wp_hb0 {
    use super::*;

    #[test]
    fn wp_hb0_the_bundled_table_parses() {
        let p = Policy::embedded();
        assert!(p.rows.len() > 200, "{}", p.rows.len());
    }

    #[test]
    fn wp_hb0_classes_follow_the_table() {
        let p = Policy::embedded();
        assert_eq!(p.classify(op::REQ_GET_PLATFORM, Dir::Get), Class::Read);
        assert_eq!(
            p.classify(op::REQ_SET_MASTER_VOLUME, Dir::Set),
            Class::Control
        );
        // One opcode both ways (spec 6.3).
        assert_eq!(p.classify(op::REQ_OUTPUT_LATENCY, Dir::Get), Class::Read);
        assert_eq!(p.classify(op::REQ_OUTPUT_LATENCY, Dir::Set), Class::Config);
    }

    #[test]
    fn wp_hb0_an_unlisted_command_is_config() {
        let p = Policy::embedded();
        assert_eq!(p.classify(op::REQ_EXTENSION, Dir::Set), Class::Config);
        assert_eq!(p.classify(op::REQ_EXTENSION, Dir::Get), Class::Config);
    }

    #[test]
    fn wp_hb0_what_mutates() {
        let p = Policy::embedded();
        assert!(!p.mutates(op::REQ_GET_PLATFORM, Dir::Get));
        assert!(p.mutates(op::REQ_SET_MASTER_VOLUME, Dir::Set));
        assert!(p.mutates(op::REQ_EXTENSION, Dir::Set));
        assert!(!p.mutates(op::REQ_EXTENSION, Dir::Get));
        assert!(p.mutates(0xFE, Dir::Get), "unknown counts as a write");
    }
}
