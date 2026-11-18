//! The command tables (`policy/commands.json`, spec 9.3) and what a role may do.
//! Owned by WP HB2; re-keyed on `(page, cmd)` by WP PGHPOL.
//!
//! **Two tables (PLAN-PG section 1.6).** Page 0, the 1.1.6 generation, is classed by
//! the 1.2 table keyed by opcode (`commands-1.2.json`, as before); any other page by
//! upstream's 1.3 table keyed by command id (`commands-1.3.json`, DSPi-Link
//! `4aec5ab`) plus our private rows (`private.json`: the extension tunnel `0xE010`).
//! A command id or a direction neither lists is `config` (spec 1.3 section 9.3).
//! The other judges (lock scope, the core-voltage rule, limits, the apply guard) see
//! a paged request through its 1.1.6 equivalent ([`judge_view`], OD-PGi), and a
//! page-0 request the hub translates for a paged device is judged by the stricter of
//! its 1.2 class and the 1.3 classes of what it reaches ([`Policy::classify_translated`],
//! OD-PGh). The opcode-keyed functions stay until PGHDEV switches the router.
//!
//! **How `0x00` is classed.** `REQ_EXTENSION` is not in upstream's table, so rule 7
//! of spec section 10 makes it `config` in both directions: admin only, reads
//! included, whatever the sub-command. A viewer or control client is answered
//! `DENIED`, and the core takes that as "no extension channel" on a read
//! (`dspi-core/src/ext.rs`). Its IN side only reads (`ext_handle_get`), so
//! [`Policy::mutates`] does not count it as a write. HUB.md section 13 has this for
//! client authors.
//!
//! **`EXT_CORE_VREG`.** Being admin is not enough for a raise the voltage rule would
//! ask a word for: see [`core_vreg_write`] and `Router::before_exec`.

use std::collections::{BTreeMap, BTreeSet};

use dspi_link::Dir;
use dspi_link::frames::CmdRequest;
use dspi_proto::ext::VregOp;
use dspi_proto::generated::opcodes as op;
// -- WP PGHPOL --
use dspi_proto::paged::plan::{UsbDir, reaches};
use dspi_proto::paged::{CmdId, PRIVATE_PAGES, PagedDir, legacy_equivalent};
use dspi_proto::registry::REGISTRY;
use dspi_session::overvolt;
use serde::Deserialize;

use crate::session::Role;

/// The table this build ships.
// -- WP PGHPOL -- renamed from `commands.json`: the 1.2 table, keyed by opcode.
const COMMANDS_JSON: &str = include_str!("../../../tests/fixtures/link/policy/commands-1.2.json");
// -- WP PGHPOL --
/// Upstream's 1.3 table, keyed by command id (DSPi-Link `4aec5ab`, MIT).
const COMMANDS_13_JSON: &str =
    include_str!("../../../tests/fixtures/link/policy/commands-1.3.json");
/// Our rows for the private pages (0xE0-0xEF), in the 1.3 table's shape.
const PRIVATE_JSON: &str = include_str!("../../../tests/fixtures/link/policy/private.json");

/// How much authority a command needs (spec 6.3). Ordered from the laxest to the
/// strictest (WP PGHPOL), so `max` is "the stricter of".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
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
    /// The directions the firmware accepts. Absent, nothing is accepted.
    #[serde(default)]
    dirs: Vec<String>,
    class: Class,
    #[serde(default)]
    classes: BTreeMap<String, Class>,
}

/// A row's key for a direction.
fn dir_key(dir: Dir) -> &'static str {
    match dir {
        Dir::Set => "set",
        Dir::Get => "get",
    }
}

#[derive(Debug, Deserialize)]
struct Table {
    commands: Vec<Row>,
}

// -- WP PGHPOL --
/// A row of the 1.3 table (spec 1.3 section 9.3): one class per direction the
/// command has; a direction it does not list is `config`.
#[derive(Debug, Deserialize)]
struct IdRow {
    id: u16,
    #[serde(default)]
    classes: BTreeMap<String, Class>,
}

#[derive(Debug, Deserialize)]
struct IdTable {
    commands: Vec<IdRow>,
}

/// The hub's command table.
#[derive(Debug)]
pub struct Policy {
    rows: BTreeMap<u8, Row>,
    /// See [`Policy::retries_stall`].
    busy_reads: BTreeSet<u8>,
    // -- WP PGHPOL --
    /// Pages != 0: the 1.3 table and our private rows, by command id.
    id_rows: BTreeMap<u16, IdRow>,
}

// -- WP PGHPOL --
/// Why a set of tables cannot be the hub's.
#[derive(Debug, thiserror::Error)]
pub enum TableError {
    /// A table is not JSON of the expected shape.
    #[error("the {table} table does not parse: {source}")]
    Parse {
        /// `1.2`, `1.3` or `private`.
        table: &'static str,
        /// What `serde_json` said.
        source: serde_json::Error,
    },
    /// A command id appears twice in the 1.3 and private tables together.
    #[error("command {id:#06x} is listed twice (in the {table} table)")]
    Duplicate {
        /// The table of the second row.
        table: &'static str,
        /// The command id.
        id: u16,
    },
    /// A by-id row on page 0, which only the 1.2 table classes.
    #[error("the {table} table lists command {id:#06x}, which is on page 0")]
    PageZero {
        /// `1.3` or `private`.
        table: &'static str,
        /// The command id.
        id: u16,
    },
    /// A private row outside pages 0xE0-0xEF.
    #[error("the private table lists {id:#06x}, which is not on a private page")]
    NotPrivate {
        /// The command id.
        id: u16,
    },
}

/// The parameter reads the direct path repeats through a busy window: the GET of each
/// registry entry (`readout.rs`, `write.rs`), and the platform read (`probe.rs`). Bulk
/// chunks have their own loop (`device.rs`).
fn busy_reads() -> BTreeSet<u8> {
    REGISTRY
        .iter()
        .filter_map(|d| d.get)
        .chain([op::REQ_GET_PLATFORM])
        .collect()
}

impl Policy {
    /// The table bundled with this build. A table that does not parse leaves every
    /// command `config` (rule 7), and a unit test keeps that from shipping.
    pub fn embedded() -> Self {
        // -- WP PGHPOL -- the three tables together; any one bad leaves all empty.
        match Self::from_tables(COMMANDS_JSON, COMMANDS_13_JSON, PRIVATE_JSON) {
            Ok(p) => p,
            Err(e) => {
                log::error!("the bundled command tables: {e}");
                Self {
                    rows: BTreeMap::new(),
                    busy_reads: busy_reads(),
                    id_rows: BTreeMap::new(),
                }
            }
        }
    }

    // -- WP PGHPOL --
    /// A policy from the 1.2 table (by opcode), the 1.3 table (by command id) and
    /// our private rows. The 1.3 and private rows must be off page 0, the private
    /// ones on a private page, and no id may be listed twice.
    pub fn from_tables(t12: &str, t13: &str, private: &str) -> Result<Self, TableError> {
        let t12: Table = serde_json::from_str(t12).map_err(|source| TableError::Parse {
            table: "1.2",
            source,
        })?;
        let mut id_rows = BTreeMap::new();
        for (table, json) in [("1.3", t13), ("private", private)] {
            let t: IdTable =
                serde_json::from_str(json).map_err(|source| TableError::Parse { table, source })?;
            for row in t.commands {
                let id = row.id;
                if CmdId(id).is_legacy() {
                    return Err(TableError::PageZero { table, id });
                }
                if table == "private" && !PRIVATE_PAGES.contains(&CmdId(id).page()) {
                    return Err(TableError::NotPrivate { id });
                }
                if id_rows.insert(id, row).is_some() {
                    return Err(TableError::Duplicate { table, id });
                }
            }
        }
        Ok(Self {
            rows: t12.commands.into_iter().map(|r| (r.code, r)).collect(),
            busy_reads: busy_reads(),
            id_rows,
        })
    }

    /// The class of a command in one direction. A command the table does not list
    /// is `config` (spec 10, rule 7), and so is a direction the table does not list
    /// for it: a `SET` of a read-only opcode must not pass as a read.
    pub fn classify(&self, code: u8, dir: Dir) -> Class {
        let key = dir_key(dir);
        match self.rows.get(&code) {
            Some(row) if row.dirs.iter().any(|d| d == key) => {
                row.classes.get(key).copied().unwrap_or(row.class)
            }
            _ => Class::Config,
        }
    }

    /// Whether a command can change the device, whichever direction carries it
    /// (write-as-read included). Such a command is attributed to its session.
    pub fn mutates(&self, code: u8, dir: Dir) -> bool {
        if code == op::REQ_EXTENSION && dir == Dir::Get {
            return false;
        }
        self.classify(code, dir) != Class::Read
    }

    /// Whether this command is one the hub may repeat through the device's busy window:
    /// a parameter read. The opcode alone does not decide: the device's worker also asks
    /// whether a write has just run (HUB.md section 4), since the same opcodes are also
    /// the core's probes, for which a stall is the answer.
    pub fn retries_stall(&self, code: u8, dir: Dir) -> bool {
        dir == Dir::Get && self.busy_reads.contains(&code) && !self.mutates(code, dir)
    }

    /// Whether this role may issue this command (spec 6.3).
    pub fn allows(&self, role: Role, code: u8, dir: Dir) -> bool {
        role_permits(role, self.classify(code, dir))
    }

    /// The effective denial list for a role (`policy.denied`, spec 7.3), as
    /// `[bRequest, dir]` pairs, `dir` as on the wire. Each listed direction of each
    /// row the role may not use, and `0x00` both ways, since a client that greys
    /// controls out should know. Other unlisted codes are denied too, but no client
    /// has a control for them. Does not say which voltage writes are refused: that
    /// depends on the unit (HUB.md section 13).
    pub fn denied(&self, role: Role) -> Vec<[u16; 2]> {
        let mut denied: Vec<[u16; 2]> = Vec::new();
        for dir in [Dir::Set, Dir::Get] {
            if !self.rows.contains_key(&op::REQ_EXTENSION) && !self.allows(role, 0, dir) {
                denied.push([u16::from(op::REQ_EXTENSION), dir as u16]);
            }
        }
        for row in self.rows.values() {
            for dir in [Dir::Set, Dir::Get] {
                let listed = row.dirs.iter().any(|d| d == dir_key(dir));
                if listed && !self.allows(role, row.code, dir) {
                    denied.push([u16::from(row.code), dir as u16]);
                }
            }
        }
        denied
    }
}

/// Roles are cumulative (spec 6.3).
fn role_permits(role: Role, class: Class) -> bool {
    match role {
        Role::Viewer => class == Class::Read,
        Role::Control => class != Class::Config,
        Role::Admin => true,
    }
}

// -- WP P1HA --
/// Whether this role may send this `apply.*` message (`apply_guard.rs`):
/// beginning, confirming and cancelling a guarded apply is Admin and Control, as
/// applying is a listening-state change at the least (spec 6.3); reading its
/// status is every role's. Anything else is not an `apply.*` message: false.
pub fn apply_guard_allows(role: Role, message: &str) -> bool {
    match message {
        "apply.begin" | "apply.confirm" | "apply.cancel" => role != Role::Viewer,
        "apply.status" => true,
        _ => false,
    }
}

// -- WP P1HL --
/// Whether this role may send this `limits.*` or `mode.*` message (`limits.rs`):
/// reading a device's limits and lock, and the mode pinned for one's own role, is
/// every role's; unlocking and locking again is Admin's and Control's (a Viewer
/// changes nothing to unlock for); setting a profile, a PIN or a role's mode is
/// Admin's (PLAN-CLIENTS 5.9, P1HL). Anything else: false.
pub fn limits_allows(role: Role, message: &str) -> bool {
    match message {
        "limits.get" | "mode.pinned" => true,
        "limits.unlock" | "limits.lock" => role != Role::Viewer,
        "limits.set" | "limits.pin" | "mode.pin" => role == Role::Admin,
        _ => false,
    }
}

// -- WP P1HL --
/// Whether a command is within a locked device's scope for a Control client
/// (`docs/UI-MODES.md` section 3: volume, mute and preset; OD-P1i): the master and
/// the user volume, the user and an output's mute, a preset load. Reads are never
/// judged by the lock; any other change is outside it.
///
/// The analyser too (WP P1DKFIX, F7, decided): starting, stopping and
/// configuring it (`RTA_CONTROL`, `RTA_SET_CONFIG`) changes no audio, and the
/// core's lock lets it run as well.
pub fn in_lock_scope(code: u8, dir: Dir) -> bool {
    matches!(
        (code, dir),
        (
            op::REQ_SET_MASTER_VOLUME
                | op::REQ_SET_USER_VOLUME
                | op::REQ_SET_USER_MUTE
                | op::REQ_SET_OUTPUT_MUTE
                | op::REQ_RTA_SET_CONFIG,
            Dir::Set
        ) | (op::REQ_PRESET_LOAD | op::REQ_RTA_CONTROL, Dir::Get)
    )
}

/// `None`: this command is no `EXT_CORE_VREG` write. Otherwise the request it
/// carries, `Some(None)` if it is not one the rule can judge: a wrong length or an
/// op it does not know. The firmware refuses those (`ext_handle_set`,
/// `core_vreg_request_valid`), and the hub treats them as a code that is not a
/// voltage, which needs the word, in case a later build does not. `wValue` is
/// compared whole, as the firmware does.
pub fn core_vreg_write(req: &CmdRequest) -> Option<Option<VregOp>> {
    if req.dir != Dir::Set || !overvolt::is_core_vreg_write(req.b_request, req.w_value) {
        return None;
    }
    Some(match *req.payload.as_slice() {
        [0, value] => Some(VregOp::Set(value)),
        [1, value] => Some(VregOp::Trial(value)),
        [2, _] => Some(VregOp::TrialEnd),
        [3, value @ (0 | 1)] => Some(VregOp::Auto(value == 1)),
        [4, _] => Some(VregOp::Clear),
        _ => None,
    })
}

// -- WP PGHPOL --
/// The Link direction as a paged one.
fn paged_dir(dir: Dir) -> PagedDir {
    match dir {
        Dir::Set => PagedDir::Set,
        Dir::Get => PagedDir::Get,
    }
}

/// The Link direction as a 1.1.6 USB one (SET is OUT, GET is IN).
fn usb_dir(dir: Dir) -> UsbDir {
    match dir {
        Dir::Set => UsbDir::Out,
        Dir::Get => UsbDir::In,
    }
}

// -- WP PGHPOL --
impl Policy {
    /// The class of a command by `(page, cmd)` (PLAN-PG section 1.6): page 0 by the
    /// 1.2 table ([`Policy::classify`]); any other page by the 1.3 table and our
    /// private rows. An id, or a direction of one, neither lists is `config`
    /// (spec 1.3 section 9.3): today the pin commands upstream's table lacks.
    pub fn classify_id(&self, id: CmdId, dir: Dir) -> Class {
        if id.is_legacy() {
            return self.classify(id.cmd(), dir);
        }
        self.id_rows
            .get(&id.0)
            .and_then(|row| row.classes.get(dir_key(dir)).copied())
            .unwrap_or(Class::Config)
    }

    /// Whether a command can change the device, by `(page, cmd)`: page 0 as
    /// [`Policy::mutates`]; any other page a SET only, since every 1.1.7 GET only
    /// reads (and the tunnel's GET only reads, as `0x00`'s IN side does).
    pub fn mutates_id(&self, id: CmdId, dir: Dir) -> bool {
        if id.is_legacy() {
            return self.mutates(id.cmd(), dir);
        }
        dir == Dir::Set
    }

    /// Whether this role may issue this command, by `(page, cmd)`.
    pub fn allows_id(&self, role: Role, id: CmdId, dir: Dir) -> bool {
        role_permits(role, self.classify_id(id, dir))
    }

    /// The class of a page-0 request the hub translates for a paged device (OD-PGh):
    /// the stricter of its 1.2 class and the 1.3 class of every paged command its
    /// plan reaches (`dspi_proto::paged::plan::reaches`), so a later upstream
    /// tightening cannot be bypassed with the page-0 form. A request whose plan
    /// reaches nothing sends nothing to the device (the adapter answers it or
    /// refuses it locally) and keeps its 1.2 class.
    pub fn classify_translated(&self, opcode: u8, value: u16, dir: Dir) -> Class {
        reaches(opcode, value, usb_dir(dir))
            .into_iter()
            .map(|(id, pdir)| {
                self.classify_id(
                    id,
                    match pdir {
                        PagedDir::Set => Dir::Set,
                        PagedDir::Get => Dir::Get,
                    },
                )
            })
            .fold(self.classify(opcode, dir), Ord::max)
    }

    /// The effective denial list for a role by command id (`policy.denied_ids`,
    /// PLAN-PG section 1.4), as `[id, dir]` pairs, `dir` as on the wire: each
    /// direction of each 1.3 or private row the role may not use. Ids neither
    /// table lists are denied too (`config`) but are not listed, as in
    /// [`Policy::denied`]. Page 0 is [`Policy::denied`]'s.
    pub fn denied_ids(&self, role: Role) -> Vec<[u16; 2]> {
        let mut denied = Vec::new();
        for row in self.id_rows.values() {
            for dir in [Dir::Set, Dir::Get] {
                if row.classes.contains_key(dir_key(dir))
                    && !self.allows_id(role, CmdId(row.id), dir)
                {
                    denied.push([row.id, dir as u16]);
                }
            }
        }
        denied
    }
}

// -- WP PGHPOL --
/// What the judges (lock scope, the core-voltage rule, limits, the apply guard,
/// bulk gain) see of a request (OD-PGi): a page-0 request as it is; a paged one as
/// its 1.1.6 equivalent (`dspi_proto::paged::legacy_equivalent`) in a 1.1.6-shaped
/// `CmdRequest` (`wIndex` 2, the equivalent's direction, `wValue` and payload; a
/// GET keeps the `wLength` asked); `None` when the paged request has no 1.1.6
/// equivalent, which the judges treat conservatively (outside the lock scope; a
/// write the limits profile would cover is `limits_unknown`).
pub fn judge_view(req: &CmdRequest) -> Option<CmdRequest> {
    let id = CmdId::from_request(req.b_request, req.w_index);
    if id.is_legacy() {
        return Some(req.clone());
    }
    let payload = match req.dir {
        Dir::Set => req.payload.as_slice(),
        Dir::Get => &[],
    };
    let eq = legacy_equivalent(id, paged_dir(req.dir), req.w_value, payload)?;
    let (dir, w_length) = match eq.dir {
        UsbDir::Out => (Dir::Set, u16::try_from(eq.data.len()).unwrap_or(u16::MAX)),
        UsbDir::In => (Dir::Get, if req.dir == Dir::Get { req.w_length } else { 0 }),
    };
    Some(CmdRequest {
        tag: req.tag,
        handle: req.handle,
        dir,
        b_request: eq.opcode,
        w_value: eq.value,
        w_index: 2,
        w_length,
        payload: if dir == Dir::Set { eq.data } else { Vec::new() },
    })
}

/// [`in_lock_scope`] by `(page, cmd)`: a page-0 request as it is, a paged one by
/// its 1.1.6 equivalent; a paged request with no equivalent is outside the scope
/// (refused for Control while locked).
pub fn in_lock_scope_id(req: &CmdRequest) -> bool {
    judge_view(req).is_some_and(|v| in_lock_scope(v.b_request, v.dir))
}

/// [`core_vreg_write`] by `(page, cmd)`: a page-0 request as it is, a paged one by
/// its 1.1.6 equivalent, so the tunnel `0xE010` SET with `wValue` `EXT_CORE_VREG`
/// is judged exactly as `0x00`'s. No other paged command is a voltage write.
pub fn core_vreg_write_id(req: &CmdRequest) -> Option<Option<VregOp>> {
    judge_view(req).and_then(|v| core_vreg_write(&v))
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
    fn wp_hb0_only_parameter_reads_retry_a_stall() {
        let p = Policy::embedded();
        assert!(p.retries_stall(op::REQ_GET_PLATFORM, Dir::Get));
        assert!(p.retries_stall(op::REQ_GET_MASTER_VOLUME, Dir::Get));
        assert!(p.retries_stall(op::REQ_GET_EQ_PARAM, Dir::Get));
        assert!(!p.retries_stall(op::REQ_SET_MASTER_VOLUME, Dir::Set));
        assert!(!p.retries_stall(0xFE, Dir::Get));
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

#[cfg(test)]
mod tests_wp_hb2 {
    use super::*;
    use dspi_proto::generated::ext as g;

    const ROLES: [Role; 3] = [Role::Viewer, Role::Control, Role::Admin];

    /// Every row, both ways, for every role, by spec 6.3, against the JSON itself.
    #[test]
    fn wp_hb2_every_row_follows_spec_6_3() {
        let p = Policy::embedded();
        let doc: serde_json::Value = serde_json::from_str(COMMANDS_JSON).unwrap();
        let rows = doc["commands"].as_array().unwrap();
        assert_eq!(rows.len(), p.rows.len());
        for row in rows {
            let code = u8::try_from(row["code"].as_u64().unwrap()).unwrap();
            for (dir, key) in [(Dir::Set, "set"), (Dir::Get, "get")] {
                let listed = row["dirs"].as_array().unwrap().iter().any(|d| d == key);
                let class = row["classes"][key].as_str().or(row["class"].as_str());
                let class = if listed { class.unwrap() } else { "config" };
                for role in ROLES {
                    let want = match role {
                        Role::Viewer => class == "read",
                        Role::Control => class != "config",
                        Role::Admin => true,
                    };
                    assert_eq!(
                        p.allows(role, code, dir),
                        want,
                        "{role:?} {code:#04x} {key} ({class})"
                    );
                }
            }
        }
    }

    /// `REQ_GET_STATUS` is a read; a `SET` of it has no handler and must not be taken
    /// for one (`LinkPolicyTests.swift`).
    #[test]
    fn wp_hb2_a_direction_the_table_does_not_list_is_config() {
        let p = Policy::embedded();
        assert_eq!(p.classify(op::REQ_GET_STATUS, Dir::Get), Class::Read);
        assert_eq!(p.classify(op::REQ_GET_STATUS, Dir::Set), Class::Config);
        assert!(!p.allows(Role::Viewer, op::REQ_GET_STATUS, Dir::Set));
        assert!(!p.allows(Role::Control, op::REQ_GET_STATUS, Dir::Set));
    }

    #[test]
    fn wp_hb2_a_write_as_read_is_judged_by_what_it_does() {
        let p = Policy::embedded();
        assert!(!p.allows(Role::Viewer, op::REQ_RTA_CONTROL, Dir::Get));
        assert!(p.allows(Role::Control, op::REQ_RTA_CONTROL, Dir::Get));
        assert!(!p.allows(Role::Control, op::REQ_ENTER_BOOTLOADER, Dir::Get));
        assert!(p.allows(Role::Viewer, op::REQ_RTA_GET_BANDS, Dir::Get));
    }

    #[test]
    fn wp_hb2_0x00_is_admin_only_both_ways() {
        let p = Policy::embedded();
        for dir in [Dir::Set, Dir::Get] {
            assert!(!p.allows(Role::Viewer, op::REQ_EXTENSION, dir));
            assert!(!p.allows(Role::Control, op::REQ_EXTENSION, dir));
            assert!(p.allows(Role::Admin, op::REQ_EXTENSION, dir));
        }
        for role in [Role::Viewer, Role::Control] {
            let denied = p.denied(role);
            assert!(denied.contains(&[0, 0]) && denied.contains(&[0, 1]));
        }
        assert!(p.denied(Role::Admin).is_empty());
    }

    /// The list says exactly what `allows` does, for the opcodes it covers.
    #[test]
    fn wp_hb2_the_denial_list_agrees_with_the_table() {
        let p = Policy::embedded();
        let viewer = p.denied(Role::Viewer);
        assert!(viewer.contains(&[u16::from(op::REQ_SET_MASTER_VOLUME), 0]));
        assert!(viewer.contains(&[u16::from(op::REQ_RTA_CONTROL), 1]));
        assert!(!viewer.contains(&[u16::from(op::REQ_GET_PLATFORM), 1]));
        // One opcode, two classes.
        assert!(viewer.contains(&[u16::from(op::REQ_LIMITER), 0]));
        assert!(!viewer.contains(&[u16::from(op::REQ_LIMITER), 1]));
        let control = p.denied(Role::Control);
        assert!(!control.contains(&[u16::from(op::REQ_SET_MASTER_VOLUME), 0]));
        assert!(control.contains(&[u16::from(op::REQ_ENTER_BOOTLOADER), 1]));
        for role in ROLES {
            for [code, dir] in p.denied(role) {
                let dir = if dir == 0 { Dir::Set } else { Dir::Get };
                assert!(!p.allows(role, u8::try_from(code).unwrap(), dir));
            }
        }
    }

    fn write(value: u16, payload: &[u8]) -> CmdRequest {
        CmdRequest {
            tag: 0,
            handle: 0,
            dir: Dir::Set,
            b_request: op::REQ_EXTENSION,
            w_value: value,
            w_index: 2,
            w_length: u16::try_from(payload.len()).unwrap(),
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn wp_hb2_voltage_writes_are_read_the_way_the_firmware_reads_them() {
        let v = g::EXT_CORE_VREG;
        for o in [
            VregOp::Set(0x0E),
            VregOp::Trial(0x0F),
            VregOp::TrialEnd,
            VregOp::Auto(true),
            VregOp::Auto(false),
            VregOp::Clear,
        ] {
            assert_eq!(core_vreg_write(&write(v, &o.encode())), Some(Some(o)));
        }
        // Not to be judged, so refused.
        for bad in [&[0u8][..], &[0, 0x0E, 0], &[], &[5, 0], &[3, 2]] {
            assert_eq!(core_vreg_write(&write(v, bad)), Some(None), "{bad:?}");
        }
        // No voltage write: another sub-command, another opcode, the read.
        assert_eq!(
            core_vreg_write(&write(g::EXT_CROSSFEED_PAIR_MAP, &[0, 1])),
            None
        );
        assert_eq!(core_vreg_write(&write(v | 0x0100, &[0, 0x0E])), None);
        let other = CmdRequest {
            b_request: op::REQ_SET_MASTER_VOLUME,
            ..write(v, &[0, 0x0E])
        };
        assert_eq!(core_vreg_write(&other), None);
        let read = CmdRequest {
            dir: Dir::Get,
            ..write(v, &[])
        };
        assert_eq!(core_vreg_write(&read), None);
    }
}

#[cfg(test)]
mod tests_wp_p1ha {
    use super::*;

    #[test]
    fn wp_p1ha_guarded_apply_is_admin_and_control() {
        for t in ["apply.begin", "apply.confirm", "apply.cancel"] {
            assert!(apply_guard_allows(Role::Admin, t));
            assert!(apply_guard_allows(Role::Control, t));
            assert!(!apply_guard_allows(Role::Viewer, t));
        }
        for role in [Role::Admin, Role::Control, Role::Viewer] {
            assert!(apply_guard_allows(role, "apply.status"));
            assert!(!apply_guard_allows(role, "apply.other"));
        }
    }
}

#[cfg(test)]
mod tests_wp_p1hl {
    use super::*;

    #[test]
    fn wp_p1hl_limits_messages_by_role() {
        for role in [Role::Admin, Role::Control, Role::Viewer] {
            assert!(limits_allows(role, "limits.get"));
            assert!(limits_allows(role, "mode.pinned"));
            assert!(!limits_allows(role, "limits.other"));
        }
        for t in ["limits.unlock", "limits.lock"] {
            assert!(limits_allows(Role::Admin, t));
            assert!(limits_allows(Role::Control, t));
            assert!(!limits_allows(Role::Viewer, t));
        }
        for t in ["limits.set", "limits.pin", "mode.pin"] {
            assert!(limits_allows(Role::Admin, t));
            assert!(!limits_allows(Role::Control, t));
            assert!(!limits_allows(Role::Viewer, t));
        }
    }

    /// Volume, mute and preset, every one a command a Control client may send at
    /// all; nothing else that changes the device.
    #[test]
    fn wp_p1hl_the_lock_scope_is_volume_mute_and_preset() {
        let p = Policy::embedded();
        let inside = [
            (op::REQ_SET_MASTER_VOLUME, Dir::Set),
            (op::REQ_SET_USER_VOLUME, Dir::Set),
            (op::REQ_SET_USER_MUTE, Dir::Set),
            (op::REQ_SET_OUTPUT_MUTE, Dir::Set),
            (op::REQ_PRESET_LOAD, Dir::Get),
        ];
        for (code, dir) in inside {
            assert!(in_lock_scope(code, dir), "{code:#04x}");
            assert!(p.allows(Role::Control, code, dir), "{code:#04x}");
        }
        for (code, dir) in [
            (op::REQ_SET_OUTPUT_GAIN, Dir::Set),
            (op::REQ_SET_EQ_PARAM, Dir::Set),
            (op::REQ_SET_ALL_PARAMS, Dir::Set),
            (op::REQ_PRESET_SAVE, Dir::Get),
            (op::REQ_SET_MASTER_VOLUME, Dir::Get),
        ] {
            assert!(!in_lock_scope(code, dir), "{code:#04x}");
        }
    }

    // -- WP P1DKFIX -- F7: the analyser runs while locked, as in the core.
    #[test]
    fn wp_p1dkfix_the_analyser_is_inside_the_lock_scope() {
        let p = Policy::embedded();
        for (code, dir) in [
            (op::REQ_RTA_CONTROL, Dir::Get),
            (op::REQ_RTA_SET_CONFIG, Dir::Set),
        ] {
            assert!(in_lock_scope(code, dir), "{code:#04x}");
            assert!(p.allows(Role::Control, code, dir), "{code:#04x}");
        }
        assert!(!in_lock_scope(op::REQ_RTA_CONTROL, Dir::Set));
    }
}

// -- WP PGHPOL --
#[cfg(test)]
mod tests_wp_pghpol {
    use super::*;
    use dspi_proto::generated::ext as g;
    use dspi_proto::paged::legacy::{Adapter, LEGACY_OPS};
    use dspi_proto::paged::registry::{self, COMMANDS};
    use dspi_proto::paged::{EXT_TUNNEL, PagedDir};
    use proptest::prelude::*;
    use serde_json::Value;

    const ROLES: [Role; 3] = [Role::Viewer, Role::Control, Role::Admin];

    fn id(name: &str) -> CmdId {
        registry::by_name(name)
            .unwrap_or_else(|| panic!("{name} is not in the registry"))
            .id
    }

    fn dir_of(p: PagedDir) -> Dir {
        match p {
            PagedDir::Set => Dir::Set,
            PagedDir::Get => Dir::Get,
        }
    }

    fn rows13() -> Vec<Value> {
        let doc: Value = serde_json::from_str(COMMANDS_13_JSON).unwrap();
        doc["commands"].as_array().unwrap().clone()
    }

    fn req(dir: Dir, b_request: u8, w_index: u16, w_value: u16, payload: &[u8]) -> CmdRequest {
        CmdRequest {
            tag: 7,
            handle: 1,
            dir,
            b_request,
            w_value,
            w_index,
            w_length: if dir == Dir::Set {
                u16::try_from(payload.len()).unwrap()
            } else {
                64
            },
            payload: payload.to_vec(),
        }
    }

    fn paged(dir: Dir, id: CmdId, w_value: u16, payload: &[u8]) -> CmdRequest {
        req(dir, id.cmd(), id.w_index(), w_value, payload)
    }

    #[test]
    fn wp_pghpol_the_three_tables_parse() {
        let p = Policy::embedded();
        assert!(p.rows.len() > 200, "{}", p.rows.len());
        let private: Value = serde_json::from_str(PRIVATE_JSON).unwrap();
        let n_private = private["commands"].as_array().unwrap().len();
        assert_eq!(p.id_rows.len(), rows13().len() + n_private);
        assert!(p.id_rows.contains_key(&EXT_TUNNEL.0));
    }

    /// Every row of upstream's 1.3 table, both ways, for every role, against the
    /// JSON itself: a listed direction by its class, an unlisted one `config`.
    #[test]
    fn wp_pghpol_every_1_3_row_classes_as_upstreams() {
        let p = Policy::embedded();
        for row in rows13() {
            let cid = CmdId(u16::try_from(row["id"].as_u64().unwrap()).unwrap());
            assert!(!cid.is_legacy());
            for (dir, key) in [(Dir::Set, "set"), (Dir::Get, "get")] {
                let want = match row["classes"][key].as_str() {
                    Some("read") => Class::Read,
                    Some("control") => Class::Control,
                    Some("config") | None => Class::Config,
                    Some(other) => panic!("unknown class {other}"),
                };
                assert_eq!(p.classify_id(cid, dir), want, "{} {key}", row["name"]);
                for role in ROLES {
                    assert_eq!(p.allows_id(role, cid, dir), role_permits(role, want));
                }
            }
        }
    }

    /// R8a: every registry id the vendored 1.3 table lacks (today the 13 pin
    /// commands of 2026-11-18) is `config` both ways; every 1.3 row is a registry id.
    #[test]
    fn wp_pghpol_registry_ids_missing_from_the_1_3_table_are_config() {
        let p = Policy::embedded();
        let listed: BTreeSet<u16> = rows13()
            .iter()
            .map(|r| u16::try_from(r["id"].as_u64().unwrap()).unwrap())
            .collect();
        for i in &listed {
            assert!(registry::command(CmdId(*i)).is_some(), "{i:#06x}");
        }
        let absent: Vec<_> = COMMANDS
            .iter()
            .filter(|c| !listed.contains(&c.id.0))
            .collect();
        assert!(!absent.is_empty(), "the registry is newer than the table");
        assert_eq!(absent.len(), COMMANDS.len() - listed.len());
        for c in absent {
            assert!(
                c.name.contains("PIN"),
                "{}: not a pin command",
                c.full_name()
            );
            for dir in [Dir::Set, Dir::Get] {
                assert_eq!(p.classify_id(c.id, dir), Class::Config, "{}", c.full_name());
                assert!(!p.allows_id(Role::Control, c.id, dir));
            }
        }
        // An id nobody lists, on a public page and on a private one.
        for unknown in [
            CmdId::new(0x05, 0xFF),
            CmdId::new(0xE0, 0x11),
            CmdId::new(0xEF, 0x10),
        ] {
            for dir in [Dir::Set, Dir::Get] {
                assert_eq!(p.classify_id(unknown, dir), Class::Config);
            }
        }
    }

    /// Page 0 is the 1.2 table, whatever `wIndex` low byte the client sends.
    #[test]
    fn wp_pghpol_page_0_is_the_1_2_table() {
        let p = Policy::embedded();
        for code in 0..=u8::MAX {
            for dir in [Dir::Set, Dir::Get] {
                let want = p.classify(code, dir);
                for w_index in [0, 2] {
                    let cid = CmdId::from_request(code, w_index);
                    assert_eq!(p.classify_id(cid, dir), want, "{code:#04x}");
                    assert_eq!(p.mutates_id(cid, dir), p.mutates(code, dir));
                }
            }
        }
    }

    /// The hazard PGHUB0 closed on the wire, here in the table: a paged GET 0x0553
    /// is a read of the saved master volume, never the 1.1.6 FACTORY_RESET 0x53.
    #[test]
    fn wp_pghpol_master_volume_saved_get_is_a_read_never_factory_reset() {
        let p = Policy::embedded();
        let saved = CmdId::from_request(0x53, 0x0502);
        assert_eq!(saved, id("ROUTE.MASTER_VOLUME_SAVED"));
        assert_eq!(p.classify_id(saved, Dir::Get), Class::Read);
        assert!(!p.mutates_id(saved, Dir::Get));
        assert!(p.allows_id(Role::Viewer, saved, Dir::Get));
        assert_eq!(op::REQ_FACTORY_RESET, 0x53);
        assert_eq!(p.classify(0x53, Dir::Get), Class::Config);
        assert_eq!(p.classify_id(CmdId::new(0, 0x53), Dir::Get), Class::Config);
    }

    #[test]
    fn wp_pghpol_paged_gets_only_read_paged_sets_mutate() {
        let p = Policy::embedded();
        for c in &COMMANDS {
            assert!(!p.mutates_id(c.id, Dir::Get), "{}", c.full_name());
            assert!(p.mutates_id(c.id, Dir::Set), "{}", c.full_name());
        }
        assert!(!p.mutates_id(EXT_TUNNEL, Dir::Get));
        assert!(p.mutates_id(EXT_TUNNEL, Dir::Set));
    }

    /// Every paged command a `legacy_map` adapter can reach, splits walked into
    /// every case and default, with the direction it reaches it in.
    fn targets(a: &Adapter, dir: Dir, out: &mut Vec<(CmdId, PagedDir)>) {
        match a {
            Adapter::Direct(d) => out.push((
                d.target,
                match dir {
                    Dir::Set => PagedDir::Set,
                    Dir::Get => PagedDir::Get,
                },
            )),
            Adapter::ActValue(x) | Adapter::ActAddr(x) | Adapter::ActPacked(x) => {
                out.push((x.target, PagedDir::Set));
            }
            Adapter::Custom(c) => out.extend_from_slice(c.uses),
            Adapter::Split(s) => {
                for (_, c) in s.cases {
                    targets(c, dir, out);
                }
                if let Some(d) = s.default {
                    targets(d, dir, out);
                }
            }
        }
    }

    /// R5b: for every `legacy_map` row and direction, the 1.2 class is at least as
    /// strict as the 1.3 class of every paged command its plan can reach. True
    /// today with 0 exceptions; a re-vendored table that tightens a command fails
    /// here (and `classify_translated` covers it meanwhile).
    #[test]
    fn wp_pghpol_the_1_2_class_is_never_laxer_than_the_1_3_classes() {
        let p = Policy::embedded();
        let mut exceptions = Vec::new();
        let mut checked = 0usize;
        for o in &LEGACY_OPS {
            for (dir, side) in [(Dir::Set, o.out), (Dir::Get, o.r#in)] {
                let Some(a) = side else { continue };
                let mut reached = Vec::new();
                targets(a, dir, &mut reached);
                // A custom row that 1.1.7 answers locally (`GET_USB_ERROR_STATS`)
                // reaches nothing and keeps its 1.2 class.
                let c12 = p.classify(o.code, dir);
                for (cid, pdir) in reached {
                    checked += 1;
                    let c13 = p.classify_id(cid, dir_of(pdir));
                    if c12 < c13 {
                        exceptions.push(format!(
                            "{} {dir:?} ({c12:?}) reaches {cid:?} {pdir:?} ({c13:?})",
                            o.name
                        ));
                    }
                }
            }
        }
        assert!(checked > 248, "{checked}");
        assert!(exceptions.is_empty(), "{exceptions:#?}");
        // The extension row: 0x00 reaches the tunnel, `config` in both.
        for dir in [Dir::Set, Dir::Get] {
            assert_eq!(p.classify(op::REQ_EXTENSION, dir), Class::Config);
            assert_eq!(p.classify_id(EXT_TUNNEL, dir), Class::Config);
        }
    }

    /// The walk above sees what the planner's own `reaches` does, for rows
    /// without a split.
    #[test]
    fn wp_pghpol_the_walk_agrees_with_reaches() {
        for o in &LEGACY_OPS {
            for (dir, side) in [(Dir::Set, o.out), (Dir::Get, o.r#in)] {
                let Some(a) = side else { continue };
                if matches!(a, Adapter::Split(_)) {
                    continue;
                }
                let mut walked = Vec::new();
                targets(a, dir, &mut walked);
                assert_eq!(reaches(o.code, 0, usb_dir(dir)), walked, "{}", o.name);
            }
        }
    }

    proptest! {
        /// R5b as a property over every request a minor-2 client can send: the
        /// translated class is never laxer than the 1.2 class or than any 1.3 class
        /// the plan reaches, and with today's tables it is the 1.2 class.
        #[test]
        fn proptest_wp_pghpol_translated_is_the_stricter(
            opcode in any::<u8>(), value in any::<u16>(), set in any::<bool>()
        ) {
            let p = Policy::embedded();
            let dir = if set { Dir::Set } else { Dir::Get };
            let t = p.classify_translated(opcode, value, dir);
            prop_assert!(t >= p.classify(opcode, dir));
            for (cid, pdir) in reaches(opcode, value, usb_dir(dir)) {
                prop_assert!(t >= p.classify_id(cid, dir_of(pdir)));
            }
            prop_assert_eq!(t, p.classify(opcode, dir));
        }

        /// The judges' view never panics on any request and keeps page 0 as it is.
        #[test]
        fn proptest_wp_pghpol_judge_view_is_total(
            b in any::<u8>(), w_index in any::<u16>(), value in any::<u16>(),
            set in any::<bool>(), payload in proptest::collection::vec(any::<u8>(), 0..72)
        ) {
            let dir = if set { Dir::Set } else { Dir::Get };
            let r = req(dir, b, w_index, value, &payload);
            let v = judge_view(&r);
            if CmdId::from_request(b, w_index).is_legacy() {
                prop_assert_eq!(v.as_ref(), Some(&r));
            } else if let Some(v) = v {
                prop_assert_eq!(v.w_index, 2);
                if v.dir == Dir::Set {
                    prop_assert_eq!(usize::from(v.w_length), v.payload.len());
                } else {
                    prop_assert!(v.payload.is_empty());
                }
            }
            let _ = in_lock_scope_id(&r);
            let _ = core_vreg_write_id(&r);
        }
    }

    /// A synthetic upstream tightening: the translation is judged by the stricter.
    #[test]
    fn wp_pghpol_classify_translated_takes_the_stricter_of_a_tightened_row() {
        let mut doc: Value = serde_json::from_str(COMMANDS_13_JSON).unwrap();
        for row in doc["commands"].as_array_mut().unwrap() {
            if row["name"] == "ROUTE.MASTER_VOLUME" {
                row["classes"]["set"] = "config".into();
                row["classes"]["get"] = "control".into();
            }
        }
        let tight = Policy::from_tables(COMMANDS_JSON, &doc.to_string(), PRIVATE_JSON).unwrap();
        let today = Policy::embedded();
        let (set, get) = (op::REQ_SET_MASTER_VOLUME, op::REQ_GET_MASTER_VOLUME);
        assert_eq!(tight.classify(set, Dir::Set), Class::Control);
        assert_eq!(tight.classify_translated(set, 0, Dir::Set), Class::Config);
        assert_eq!(tight.classify(get, Dir::Get), Class::Read);
        assert_eq!(tight.classify_translated(get, 0, Dir::Get), Class::Control);
        assert_eq!(today.classify_translated(set, 0, Dir::Set), Class::Control);
        assert_eq!(today.classify_translated(get, 0, Dir::Get), Class::Read);
        // The other way round, a 1.2 class stricter than the 1.3 one, stays.
        assert_eq!(
            today.classify_translated(op::REQ_EXTENSION, g::EXT_LIST, Dir::Get),
            Class::Config
        );
    }

    fn vreg_ops() -> Vec<Vec<u8>> {
        let mut v: Vec<Vec<u8>> = [
            VregOp::Set(0x0E),
            VregOp::Trial(0x0F),
            VregOp::TrialEnd,
            VregOp::Auto(true),
            VregOp::Auto(false),
            VregOp::Clear,
        ]
        .iter()
        .map(|o| o.encode().to_vec())
        .collect();
        v.extend([vec![0u8], vec![0, 0x0E, 0], vec![], vec![5, 0], vec![3, 2]]);
        v
    }

    /// The tunnel's SET with `EXT_CORE_VREG` triggers the voltage rule exactly as
    /// `0x00` does; its GET and other extension ids do not.
    #[test]
    fn wp_pghpol_the_tunnel_triggers_the_voltage_rule_as_0x00_does() {
        let v = g::EXT_CORE_VREG;
        for payload in vreg_ops() {
            let legacy = req(Dir::Set, op::REQ_EXTENSION, 2, v, &payload);
            let tunnel = paged(Dir::Set, EXT_TUNNEL, v, &payload);
            assert!(core_vreg_write(&legacy).is_some());
            assert_eq!(
                core_vreg_write_id(&tunnel),
                core_vreg_write(&legacy),
                "{payload:?}"
            );
            assert_eq!(core_vreg_write_id(&legacy), core_vreg_write(&legacy));
            // A Mac-style wIndex 0 is page 0 too.
            let mac = req(Dir::Set, op::REQ_EXTENSION, 0, v, &payload);
            assert_eq!(core_vreg_write_id(&mac), core_vreg_write(&legacy));
        }
        assert_eq!(
            core_vreg_write_id(&paged(Dir::Get, EXT_TUNNEL, v, &[])),
            None
        );
        assert_eq!(
            core_vreg_write_id(&paged(
                Dir::Set,
                EXT_TUNNEL,
                g::EXT_CROSSFEED_PAIR_MAP,
                &[0, 1]
            )),
            None
        );
        // The same cmd number on another page is no tunnel.
        let other = req(Dir::Set, 0x10, 0xE102, v, &[0, 0x0E]);
        assert_eq!(core_vreg_write_id(&other), None);
        let page0 = req(Dir::Set, 0x10, 2, v, &[0, 0x0E]);
        assert_eq!(core_vreg_write_id(&page0), None);
    }

    #[test]
    fn wp_pghpol_the_tunnel_is_admin_only_both_ways() {
        let p = Policy::embedded();
        for dir in [Dir::Set, Dir::Get] {
            assert_eq!(p.classify_id(EXT_TUNNEL, dir), Class::Config);
            assert!(p.allows_id(Role::Admin, EXT_TUNNEL, dir));
            assert!(!p.allows_id(Role::Control, EXT_TUNNEL, dir));
            assert!(!p.allows_id(Role::Viewer, EXT_TUNNEL, dir));
        }
    }

    /// Lock scope by the 1.1.6 equivalent: the paged master volume, mute and
    /// preset load are inside it as their 1.1.6 forms are; a paged write with no
    /// equivalent, and everything the 1.1.6 scope leaves out, is outside.
    #[test]
    fn wp_pghpol_lock_scope_by_equivalent() {
        let vol = 0.5f32.to_le_bytes();
        let inside = [
            paged(Dir::Set, id("ROUTE.MASTER_VOLUME"), 0, &vol),
            paged(Dir::Set, id("ROUTE.USER_MUTE"), 0, &[1]),
            paged(Dir::Set, id("STORE.PRESET_LOAD"), 2, &[]),
        ];
        for r in &inside {
            let v = judge_view(r).unwrap_or_else(|| panic!("{r:?}: no equivalent"));
            assert!(in_lock_scope(v.b_request, v.dir), "{v:?}");
            assert!(in_lock_scope_id(r), "{r:?}");
        }
        let preset = judge_view(&inside[2]).unwrap();
        assert_eq!(
            (preset.b_request, preset.dir),
            (op::REQ_PRESET_LOAD, Dir::Get)
        );
        // Outside: a read of the volume, an id with no equivalent, the tunnel.
        let outside = [
            paged(Dir::Get, id("ROUTE.MASTER_VOLUME"), 0, &[]),
            paged(Dir::Set, CmdId::new(0x05, 0xFF), 0, &vol),
            paged(Dir::Set, EXT_TUNNEL, g::EXT_CORE_VREG, &[0, 0x0E]),
        ];
        for r in &outside {
            assert!(!in_lock_scope_id(r), "{r:?}");
        }
        assert_eq!(judge_view(&outside[1]), None);
        // Page 0 is unchanged.
        for code in 0..=u8::MAX {
            for dir in [Dir::Set, Dir::Get] {
                let r = req(dir, code, 2, 0, &[]);
                assert_eq!(in_lock_scope_id(&r), in_lock_scope(code, dir));
            }
        }
    }

    /// The judges' view: page 0 as sent, a paged request as its 1.1.6 form.
    #[test]
    fn wp_pghpol_judge_view_feeds_the_equivalent() {
        let vol = (-6.0f32).to_le_bytes();
        let r = req(Dir::Set, op::REQ_SET_MASTER_VOLUME, 0, 0, &vol);
        assert_eq!(judge_view(&r), Some(r.clone()));
        let v = judge_view(&paged(Dir::Set, id("ROUTE.MASTER_VOLUME"), 0, &vol)).unwrap();
        assert_eq!(
            (
                v.dir,
                v.b_request,
                v.w_index,
                v.w_length,
                v.payload.as_slice(),
                v.tag,
                v.handle
            ),
            (Dir::Set, op::REQ_SET_MASTER_VOLUME, 2, 4, &vol[..], 7, 1)
        );
        let g = judge_view(&paged(Dir::Get, id("ROUTE.MASTER_VOLUME"), 0, &[])).unwrap();
        assert_eq!(
            (g.dir, g.b_request, g.w_length),
            (Dir::Get, op::REQ_GET_MASTER_VOLUME, 64)
        );
        assert!(g.payload.is_empty());
        // The tunnel is 0x00 with the same wValue and payload.
        let t = judge_view(&paged(Dir::Set, EXT_TUNNEL, g::EXT_CORE_VREG, &[0, 0x0E])).unwrap();
        assert_eq!(
            (t.b_request, t.w_value, t.payload.as_slice()),
            (op::REQ_EXTENSION, g::EXT_CORE_VREG, &[0u8, 0x0E][..])
        );
        // A command 1.1.7 added has none.
        assert_eq!(
            judge_view(&paged(Dir::Get, id("SYS.PIN_STATE"), 0, &[])),
            None
        );
    }

    /// `denied_ids` lists exactly what `allows_id` refuses, for the listed rows.
    #[test]
    fn wp_pghpol_the_id_denial_list_agrees_with_the_tables() {
        let p = Policy::embedded();
        let vol = id("ROUTE.MASTER_VOLUME").0;
        let viewer = p.denied_ids(Role::Viewer);
        assert!(viewer.contains(&[vol, 0]));
        assert!(!viewer.contains(&[vol, 1]));
        assert!(viewer.contains(&[EXT_TUNNEL.0, 0]) && viewer.contains(&[EXT_TUNNEL.0, 1]));
        let control = p.denied_ids(Role::Control);
        assert!(!control.contains(&[vol, 0]));
        assert!(control.contains(&[id("SYS.ENTER_BOOTLOADER").0, 0]));
        assert!(control.contains(&[EXT_TUNNEL.0, 1]));
        assert!(p.denied_ids(Role::Admin).is_empty());
        for role in ROLES {
            for [i, dir] in p.denied_ids(role) {
                assert!(!CmdId(i).is_legacy());
                let dir = if dir == 0 { Dir::Set } else { Dir::Get };
                assert!(!p.allows_id(role, CmdId(i), dir));
            }
        }
    }

    #[test]
    fn wp_pghpol_bad_tables_are_refused() {
        let row = |i: u32| format!(r#"{{"commands":[{{"id":{i},"classes":{{"get":"read"}}}}]}}"#);
        let empty = r#"{"commands":[]}"#;
        assert!(matches!(
            Policy::from_tables(COMMANDS_JSON, empty, &row(0x0550)),
            Err(TableError::NotPrivate { id: 0x0550 })
        ));
        assert!(matches!(
            Policy::from_tables(COMMANDS_JSON, &row(0x0053), empty),
            Err(TableError::PageZero { id: 0x0053, .. })
        ));
        assert!(matches!(
            Policy::from_tables(COMMANDS_JSON, &row(0xE010), &row(0xE010)),
            Err(TableError::Duplicate { id: 0xE010, .. })
        ));
        assert!(matches!(
            Policy::from_tables(COMMANDS_JSON, "{", empty),
            Err(TableError::Parse { table: "1.3", .. })
        ));
        assert!(matches!(
            Policy::from_tables(COMMANDS_JSON, &row(0x1_0000), empty),
            Err(TableError::Parse { .. })
        ));
    }
}
