//! The command table (`policy/commands.json`, spec 9.3) and what a role may do.
//! Owned by WP HB2.
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
use dspi_proto::registry::REGISTRY;
use dspi_session::overvolt;
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

/// The hub's command table.
#[derive(Debug)]
pub struct Policy {
    rows: BTreeMap<u8, Row>,
    /// See [`Policy::retries_stall`].
    busy_reads: BTreeSet<u8>,
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
        match serde_json::from_str::<Table>(COMMANDS_JSON) {
            Ok(t) => Self {
                rows: t.commands.into_iter().map(|r| (r.code, r)).collect(),
                busy_reads: busy_reads(),
            },
            Err(e) => {
                log::error!("the bundled command table does not parse: {e}");
                Self {
                    rows: BTreeMap::new(),
                    busy_reads: busy_reads(),
                }
            }
        }
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
