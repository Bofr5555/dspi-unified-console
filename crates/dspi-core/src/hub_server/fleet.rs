//! The hub's fleet families (WP P3CC, the fourth section of `hub_server`):
//! groups, group writes and bulk apply, the firmware image store and the
//! rollout, as typed calls. The wire contract is `docs/integrator/hub-api.md`
//! sections 19 to 21; each call's name follows its message.
//!
//! - Groups (cap `groups`): [`DspiCore::hub_groups`] (`group.list`, every
//!   role), [`DspiCore::hub_group_set`] and [`DspiCore::hub_group_delete`]
//!   (Admin); `group.changed` through [`super::HubServerListener`]
//!   ([`super::HubServerEvent::group`]).
//! - Group writes (cap `group_params`): [`DspiCore::hub_group_param_get`]
//!   (every role), [`DspiCore::hub_group_param_set`] and
//!   [`DspiCore::hub_group_preset_load`] (Control, Admin): one row per
//!   member, in member order ([`HubGroupRow`]).
//! - Bulk apply (cap `group_apply`): [`DspiCore::hub_group_apply`],
//!   [`DspiCore::hub_group_apply_confirm`], [`DspiCore::hub_group_apply_cancel`]
//!   (Control, Admin).
//! - Firmware (cap `rollout`, TLS only): [`DspiCore::hub_firmware_images`]
//!   (every role), [`DspiCore::hub_firmware_put`] (the upload in chunks; the
//!   bytes never logged) and [`DspiCore::hub_firmware_delete`] (Admin);
//!   [`DspiCore::hub_rollout_start`], [`DspiCore::hub_rollouts`] (every role),
//!   [`DspiCore::hub_rollout_cancel`]; `fw.rollout.changed` through the
//!   listener ([`super::HubServerEvent::rollout`]). Each step of a record
//!   has its notice row `rollout.<step>` ([`ROLLOUT_STEPS`],
//!   [`hub_rollout_step_notice`]).
//!
//! As the first section: [`DspiError::Unsupported`] with nothing sent
//! without the family's cap (or on no hub), [`DspiError::Disconnected`]
//! while the hub session is down, the hub's refusals typed (a `fw.*` request
//! over a plaintext session is the hub's `tls_required`, as
//! [`DspiError::Link`] with that code), and the client read-only mode lets
//! out only the reads ([`super::HUB_SERVER_READS`]) and what settles or stops
//! ([`super::HUB_SERVER_SETTLES`]: a run's confirm and cancel, a rollout's
//! cancel). A fan-out or an apply
//! answers after its last member: these calls wait up to
//! [`GROUP_REQUEST_TIMEOUT_S`] seconds. Off the UI thread.

use serde::{Deserialize, Serialize};
#[cfg_attr(not(feature = "link"), allow(unused_imports))]
use serde_json::{Map, Value};

#[cfg_attr(not(feature = "link"), allow(unused_imports))]
use super::{cut, hub_code, small, text, texts};
use crate::{DspiCore, DspiError, Notice, ParamValue, Result};

/// Cap: `group.*` (hub-api.md section 19).
pub const CAP_GROUPS: &str = "groups";
/// Cap: `group.param.*` and `group.preset.load` (section 20.2).
pub const CAP_GROUP_PARAMS: &str = "group_params";
/// Cap: `group.apply*` (section 20.3).
pub const CAP_GROUP_APPLY: &str = "group_apply";
/// Cap: `fw.image.*` and `fw.rollout.*` (section 21).
pub const CAP_ROLLOUT: &str = "rollout";

/// The hub's code: a `fw.*` request over a plaintext session (section 21.1).
pub const HUB_ERR_TLS_REQUIRED: &str = "tls_required";

/// How long a group request may take, s: the answer comes after the last
/// member (64 at most), one after another.
pub const GROUP_REQUEST_TIMEOUT_S: u64 = 300;

/// The steps of a rollout, in order (section 21.3). Each has a notice row
/// `rollout.<step>`.
pub const ROLLOUT_STEPS: [&str; 8] = [
    "check",
    "guard",
    "export",
    "bootloader",
    "find",
    "flash",
    "wait",
    "verify",
];

/// The notice rows' family of the steps: `rollout.<step>`.
pub const NOTICE_ROLLOUT_FAMILY: &str = "rollout";

/// The notice row of a step this build does not know.
pub const NOTICE_ROLLOUT_UNKNOWN: &str = "rollout.unknown";

/// A stored image is at most this many bytes (the hub's own cap).
pub const FIRMWARE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Bytes of image in one `fw.image.put` chunk: 49 152 base64 characters.
#[cfg_attr(not(feature = "link"), allow(dead_code))]
const CHUNK_BYTES: usize = 36 * 1024;

/// Groups taken from the hub at most (its own cap is 64).
#[cfg_attr(not(feature = "link"), allow(dead_code))]
const GROUPS_MAX: usize = 256;
/// Members, rows and devices taken from the hub for one list at most.
const ROWS_MAX: usize = 256;
/// Rollout records taken at most (the hub keeps 64).
#[cfg_attr(not(feature = "link"), allow(dead_code))]
const ROLLOUTS_MAX: usize = 64;
/// Steps of one device's entry at most.
const STEPS_MAX: usize = 32;
/// Firmware images taken at most (the hub keeps 4).
#[cfg_attr(not(feature = "link"), allow(dead_code))]
const IMAGES_MAX: usize = 16;

// ── Records ────────────────────────────────────────────────────────────────

/// A group of the hub's ([`DspiCore::hub_groups`]; [`DspiCore::hub_group_set`]
/// takes one).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubGroup {
    /// The hub's id (`g1`, `g2`, ...; never reused); "" for a new group.
    pub id: String,
    /// 1 to 48 characters.
    pub name: String,
    /// At most 256 characters; "" when unset.
    pub note: String,
    /// 0 to 64 inventory serials, each once, in the order fan-outs use.
    pub members: Vec<String>,
}

/// What [`DspiCore::hub_group_set`] stored.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubGroupSet {
    /// The group as the hub now has it.
    pub group: HubGroup,
    /// A new group (no id was given).
    pub created: bool,
}

/// Which devices a group request addresses: exactly one of a group's id or
/// 1 to 16 serials (section 20.1).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubFleetTarget {
    /// A group's id; "" when [`HubFleetTarget::serials`] is used.
    pub group: String,
    /// Serials, each once; empty when [`HubFleetTarget::group`] is used.
    pub serials: Vec<String>,
}

/// One member's offset of [`DspiCore::hub_group_param_set`] (dB paths only).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubOffset {
    /// The member.
    pub serial: String,
    /// Added to the value, dB.
    pub db: f64,
}

/// One member's own preset of [`DspiCore::hub_group_preset_load`].
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubPresetPick {
    /// The member.
    pub serial: String,
    /// Its slot (0 to 9); `None` when [`HubPresetPick::name`] is used.
    pub slot: Option<u8>,
    /// A preset's name; "" when [`HubPresetPick::slot`] is used.
    pub name: String,
}

/// One device's row of a group request (sections 20.1 to 20.3). The fields a
/// request does not use are empty, `None` or false.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubGroupRow {
    /// The device.
    pub serial: String,
    /// Written, read or applied.
    pub ok: bool,
    /// The hub's code when not (`offline`, `limit_exceeded`, ...); "" when ok.
    pub code: String,
    /// The hub's words; "" when none.
    pub msg: String,
    /// The notice row for [`HubGroupRow::code`] ("" when ok).
    pub notice_key: String,
    /// The limit crossed (`limit_exceeded`).
    pub limit: String,
    /// A parameter's value: what was read, or what was sent and read back.
    pub value: Option<ParamValue>,
    /// A choice's label, where the path has labels.
    pub choice: String,
    /// A structured value, base64 (as `param.get` answers it).
    pub value_b64: String,
    /// The member's offset was clamped into its range.
    pub clamped: bool,
    /// The preset slot loaded.
    pub slot: Option<u8>,
    /// The preset's name.
    pub name: String,
    /// An apply: the whole image was written.
    pub image: bool,
    /// An apply: what changed.
    pub lines: Vec<String>,
    /// An apply: what was not written, and why.
    pub notes: Vec<String>,
    /// An apply with a guard: the device's guard id.
    pub guard: Option<u64>,
    /// The guard reverted (ran out before the run ended, or cancelled).
    pub reverted: bool,
}

/// A parameter fan-out's answer ([`DspiCore::hub_group_param_get`],
/// [`DspiCore::hub_group_param_set`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubGroupParams {
    /// The path.
    pub path: String,
    /// Its indices.
    pub idx: Vec<u32>,
    /// One row per member, in order.
    pub results: Vec<HubGroupRow>,
}

/// A bulk apply's run ([`DspiCore::hub_group_apply`], its confirm and
/// cancel).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubGroupRun {
    /// The hub's id of the run (`a1`, ...).
    pub run: String,
    /// The earliest guard deadline (ISO-8601 UTC); "" without a guard.
    pub confirm_by: String,
    /// The hub's note (guards that ran out: raise the timeout); "" when none.
    pub msg: String,
    /// One row per member (apply) or per guard (confirm, cancel).
    pub results: Vec<HubGroupRow>,
}

/// A firmware image in the hub's store ([`DspiCore::hub_firmware_images`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubFirmwareImage {
    /// As uploaded (`...uf2`).
    pub name: String,
    /// Bytes.
    pub size: u64,
    /// Of the whole file, lowercase hex.
    pub sha256: String,
    /// The DSPi version in the image; "" when none.
    pub firmware_version: String,
    /// `RP2040` or `RP2350`.
    pub platform: String,
    /// The core-voltage stamp, mV; `None` when unstamped.
    pub stamp_mv: Option<u16>,
    /// The release's manifest lists this name with these bytes.
    pub verified: bool,
    /// When (ISO-8601 UTC).
    pub added_at: String,
    /// Who: a client's name.
    pub added_by: String,
}

/// The hub's image store ([`DspiCore::hub_firmware_images`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubFirmwareImages {
    /// Oldest first.
    pub images: Vec<HubFirmwareImage>,
    /// Images the store keeps at most.
    pub max_images: u32,
    /// Bytes an image may have at most.
    pub max_bytes: u64,
}

/// What [`DspiCore::hub_firmware_put`] stored.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubFirmwarePut {
    /// The image as the hub has it.
    pub image: HubFirmwareImage,
    /// False: the hub already held this name with these bytes.
    pub created: bool,
}

/// One step a rollout's device went through.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubRolloutStep {
    /// One of [`ROLLOUT_STEPS`].
    pub step: String,
    /// When (ISO-8601 UTC).
    pub at: String,
    /// It went through.
    pub ok: bool,
    /// The hub's line (for a log); a client shows [`HubRolloutStep::notice_key`].
    pub text: String,
    /// The session the step's device request went out as, where it has one.
    pub session: Option<u32>,
    /// `rollout.<step>` ([`hub_rollout_step_notice`]).
    pub notice_key: String,
}

/// `EXT_CORE_VREG` as the rollout read it, mV.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubRolloutVreg {
    /// The live setting.
    pub live_mv: Option<u16>,
    /// The stored selection.
    pub stored_mv: Option<u16>,
    /// The build default.
    pub default_mv: Option<u16>,
}

/// A device before its flash (step 3).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubRolloutBefore {
    /// Its firmware version.
    pub fw: String,
    /// `EXT_LIST`.
    pub ext_list: Vec<u16>,
    /// The lowest core voltage the board may be left on, mV (0 unknown).
    pub required_mv: u16,
    /// Where that came from.
    pub board_source: String,
    /// `EXT_CORE_VREG`, where it has one.
    pub vreg: Option<HubRolloutVreg>,
    /// The export's snapshot id in the hub's store; "" when none.
    pub export: String,
}

/// A device after it came back (step 8).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubRolloutAfter {
    /// Its firmware version.
    pub fw: String,
    /// `EXT_LIST`.
    pub ext_list: Vec<u16>,
    /// `EXT_CORE_VREG`, where it has one.
    pub vreg: Option<HubRolloutVreg>,
    /// Against the export: `unchanged`, `changed` or `unknown`.
    pub settings: String,
}

/// Why a device's part ended badly.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubRolloutFailed {
    /// The step.
    pub step: String,
    /// The hub's code, where a refusal had one; "" otherwise.
    pub code: String,
    /// The hub's line.
    pub text: String,
    /// The notice row of the code (`dlink.<code>`, `image_refused`, ...);
    /// "" without a code (then the step's row says it).
    pub notice_key: String,
}

/// One device of a rollout.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubRolloutDevice {
    /// The device.
    pub serial: String,
    /// `pending`, `running`, `ok`, `failed`, `cancelled`, `not_tried`.
    pub state: String,
    /// The step it is at or ended at ("" before its first).
    pub step: String,
    /// The steps done, in order.
    pub steps: Vec<HubRolloutStep>,
    /// Before its flash.
    pub before: Option<HubRolloutBefore>,
    /// After it came back.
    pub after: Option<HubRolloutAfter>,
    /// Why it failed.
    pub failed: Option<HubRolloutFailed>,
}

/// A rollout ([`DspiCore::hub_rollouts`], `fw.rollout.changed`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubRollout {
    /// The hub's id (`ro1`, ...; never reused).
    pub id: String,
    /// The serial, or `group:<id>`.
    pub target: String,
    /// The image's name.
    pub image: String,
    /// Its SHA-256.
    pub image_sha256: String,
    /// Its DSPi version.
    pub image_version: String,
    /// The Admin's client name.
    pub by: String,
    /// Started without the mute check.
    pub while_live: bool,
    /// `running`, `ok`, `failed`, `cancelled`.
    pub state: String,
    /// ISO-8601 UTC.
    pub started_at: String,
    /// ISO-8601 UTC; "" while running.
    pub ended_at: String,
    /// One entry per device, in order.
    pub devices: Vec<HubRolloutDevice>,
}

// ── Notices ────────────────────────────────────────────────────────────────

fn step_notice_key(step: &str) -> String {
    if ROLLOUT_STEPS.contains(&step) {
        format!("{NOTICE_ROLLOUT_FAMILY}.{step}")
    } else {
        NOTICE_ROLLOUT_UNKNOWN.to_owned()
    }
}

/// What to tell the user about a rollout's step: its row `rollout.<step>`
/// (what the step does, and what to do when it fails there). A step this
/// build does not know gets [`NOTICE_ROLLOUT_UNKNOWN`]'s row. The detail is
/// `text` (the hub's line). No I/O.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn hub_rollout_step_notice(step: String, text: String) -> Notice {
    crate::notice_text::notice_for_key(step_notice_key(&step), text)
}

// ── Reading the hub's answers ─────────────────────────────────────────────

pub(crate) fn group_from(v: &Value) -> HubGroup {
    HubGroup {
        id: text(v.get("id")),
        name: text(v.get("name")),
        note: text(v.get("note")),
        members: v
            .get("members")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .take(ROWS_MAX)
                    .filter_map(Value::as_str)
                    .map(|s| cut(s, 300))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// The notice row of the hub's code `code` ("" for none).
fn code_notice(code: &str) -> String {
    if code.is_empty() {
        return String::new();
    }
    hub_code(code.to_owned(), String::new()).notice_key()
}

/// A wire value of `path` as a [`ParamValue`], by the registry's kind.
fn value_from(path: &str, v: Option<&Value>) -> Option<ParamValue> {
    use dspi_proto::registry::{Kind, by_path};
    let v = v?;
    let kind = by_path(path).map(|d| d.kind);
    match (v, kind) {
        (Value::Null, Some(Kind::Trigger)) => Some(ParamValue::Trigger),
        (Value::Null, _) => None,
        (Value::Bool(b), _) => Some(ParamValue::Bool(*b)),
        (Value::String(s), _) => Some(ParamValue::Text(cut(s, 300))),
        (Value::Number(n), Some(Kind::Float { .. })) => {
            #[allow(clippy::cast_possible_truncation)] // a parameter's value, small
            n.as_f64().map(|f| ParamValue::Float(f as f32))
        }
        (Value::Number(n), Some(Kind::Choice(_))) => n
            .as_u64()
            .and_then(|c| u8::try_from(c).ok())
            .map(ParamValue::Choice),
        (Value::Number(n), Some(Kind::Mask)) => n
            .as_u64()
            .and_then(|m| u32::try_from(m).ok())
            .map(ParamValue::Mask),
        (Value::Number(n), _) => match n.as_i64() {
            Some(i) => Some(ParamValue::Int(i)),
            #[allow(clippy::cast_possible_truncation)]
            None => n.as_f64().map(|f| ParamValue::Float(f as f32)),
        },
        _ => None,
    }
}

/// A [`ParamValue`] as the wire takes it (section 1.4); a structured value
/// is not taken by a fan-out.
#[cfg_attr(not(feature = "link"), allow(dead_code))]
fn value_to_wire(v: &ParamValue) -> Result<Value> {
    Ok(match v {
        ParamValue::Bool(b) => Value::Bool(*b),
        // The shortest text of the f32, so -0.1 goes as -0.1.
        ParamValue::Float(f) => f
            .to_string()
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .ok_or_else(|| DspiError::InvalidArgument {
                msg: "the value is not a finite number".into(),
            })?,
        ParamValue::Int(i) => Value::from(*i),
        ParamValue::Choice(c) => Value::from(*c),
        ParamValue::Mask(m) => Value::from(*m),
        ParamValue::Text(t) => Value::String(t.clone()),
        ParamValue::Trigger => Value::Null,
        ParamValue::Bytes(_) => {
            return Err(DspiError::InvalidArgument {
                msg: "a group write takes no structured value".into(),
            });
        }
    })
}

pub(crate) fn row_from(path: &str, v: &Value) -> HubGroupRow {
    let code = text(v.get("code"));
    let guard = match v.get("guard") {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) => s.parse().ok(),
        _ => None,
    };
    HubGroupRow {
        serial: text(v.get("serial")),
        ok: v.get("ok").and_then(Value::as_bool).unwrap_or(false),
        notice_key: code_notice(&code),
        code,
        msg: text(v.get("msg")),
        limit: text(v.get("limit")),
        value: value_from(path, v.get("value")),
        choice: text(v.get("choice")),
        value_b64: v
            .get("value_b64")
            .and_then(Value::as_str)
            .map(|s| cut(s, 4096))
            .unwrap_or_default(),
        clamped: v.get("clamped").and_then(Value::as_bool).unwrap_or(false),
        slot: small(v.get("slot")),
        name: text(v.get("name")),
        image: v.get("image").and_then(Value::as_bool).unwrap_or(false),
        lines: texts(v.get("lines")),
        notes: texts(v.get("notes")),
        guard,
        reverted: v.get("reverted").and_then(Value::as_bool).unwrap_or(false),
    }
}

fn rows_from(path: &str, v: &Value) -> Vec<HubGroupRow> {
    v.get("results")
        .and_then(Value::as_array)
        .map(|a| a.iter().take(ROWS_MAX).map(|r| row_from(path, r)).collect())
        .unwrap_or_default()
}

#[cfg_attr(not(feature = "link"), allow(dead_code))]
fn run_from(v: &Value) -> HubGroupRun {
    HubGroupRun {
        run: text(v.get("run")),
        confirm_by: text(v.get("confirm_by")),
        msg: text(v.get("msg")),
        results: rows_from("", v),
    }
}

#[cfg_attr(not(feature = "link"), allow(dead_code))]
fn image_from(v: &Value) -> HubFirmwareImage {
    HubFirmwareImage {
        name: text(v.get("name")),
        size: v.get("size").and_then(Value::as_u64).unwrap_or(0),
        sha256: text(v.get("sha256")),
        firmware_version: text(v.get("firmware_version")),
        platform: text(v.get("platform")),
        stamp_mv: small(v.get("stamp_mv")),
        verified: v.get("verified").and_then(Value::as_bool).unwrap_or(false),
        added_at: text(v.get("added_at")),
        added_by: text(v.get("added_by")),
    }
}

fn vreg_from(v: Option<&Value>) -> Option<HubRolloutVreg> {
    v.filter(|v| v.is_object()).map(|v| HubRolloutVreg {
        live_mv: small(v.get("live_mv")),
        stored_mv: small(v.get("stored_mv")),
        default_mv: small(v.get("default_mv")),
    })
}

fn ext_list(v: Option<&Value>) -> Vec<u16> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .take(ROWS_MAX)
                .filter_map(|x| small::<u16>(Some(x)))
                .collect()
        })
        .unwrap_or_default()
}

fn device_entry_from(v: &Value) -> HubRolloutDevice {
    let steps = v
        .get("steps")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .take(STEPS_MAX)
                .map(|s| {
                    let step = text(s.get("step"));
                    HubRolloutStep {
                        notice_key: step_notice_key(&step),
                        step,
                        at: text(s.get("at")),
                        ok: s.get("ok").and_then(Value::as_bool).unwrap_or(false),
                        text: text(s.get("text")),
                        session: small(s.get("session")),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    HubRolloutDevice {
        serial: text(v.get("serial")),
        state: text(v.get("state")),
        step: text(v.get("step")),
        steps,
        before: v.get("before").filter(|b| b.is_object()).map(|b| {
            let board = b.get("board");
            HubRolloutBefore {
                fw: text(b.get("fw")),
                ext_list: ext_list(b.get("ext_list")),
                required_mv: small(board.and_then(|x| x.get("required_mv"))).unwrap_or(0),
                board_source: text(board.and_then(|x| x.get("source"))),
                vreg: vreg_from(b.get("vreg")),
                export: text(b.get("export")),
            }
        }),
        after: v
            .get("after")
            .filter(|a| a.is_object())
            .map(|a| HubRolloutAfter {
                fw: text(a.get("fw")),
                ext_list: ext_list(a.get("ext_list")),
                vreg: vreg_from(a.get("vreg")),
                settings: text(a.get("settings")),
            }),
        failed: v.get("failed").filter(|f| f.is_object()).map(|f| {
            let code = text(f.get("code"));
            HubRolloutFailed {
                step: text(f.get("step")),
                notice_key: code_notice(&code),
                code,
                text: text(f.get("text")),
            }
        }),
    }
}

pub(crate) fn rollout_from(v: &Value) -> HubRollout {
    let image = v.get("image");
    HubRollout {
        id: text(v.get("id")),
        target: text(v.get("target")),
        image: text(image.and_then(|i| i.get("name"))),
        image_sha256: text(image.and_then(|i| i.get("sha256"))),
        image_version: text(image.and_then(|i| i.get("firmware_version"))),
        by: text(v.get("by")),
        while_live: v
            .get("while_live")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        state: text(v.get("state")),
        started_at: text(v.get("started_at")),
        ended_at: text(v.get("ended_at")),
        devices: v
            .get("devices")
            .and_then(Value::as_array)
            .map(|a| a.iter().take(ROWS_MAX).map(device_entry_from).collect())
            .unwrap_or_default(),
    }
}

#[cfg(feature = "link")]
fn protocol(msg: &str) -> DspiError {
    DspiError::Link {
        code: "protocol".into(),
        msg: msg.into(),
    }
}

/// `target` into `fields` (`group` or `targets`), checked: exactly one.
#[cfg_attr(not(feature = "link"), allow(dead_code))]
fn address(target: HubFleetTarget, fields: &mut Map<String, Value>) -> Result<()> {
    match (target.group.is_empty(), target.serials.is_empty()) {
        (false, true) => {
            fields.insert("group".into(), target.group.into());
            Ok(())
        }
        (true, false) => {
            fields.insert("targets".into(), target.serials.into());
            Ok(())
        }
        _ => Err(DspiError::InvalidArgument {
            msg: "a group request names a group or serials, one of the two".into(),
        }),
    }
}

/// A preset given by slot or by name, checked: exactly one.
#[cfg_attr(not(feature = "link"), allow(dead_code))]
fn preset_ref(slot: Option<u8>, name: &str) -> Result<Value> {
    match (slot, name.is_empty()) {
        (Some(s), true) => Ok(Value::from(s)),
        (None, false) => Ok(Value::String(name.to_owned())),
        _ => Err(DspiError::InvalidArgument {
            msg: "a preset is given by slot or by name, one of the two".into(),
        }),
    }
}

// ── The calls ──────────────────────────────────────────────────────────────

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// The hub's groups, by id (`group.list`). Every role. Slow: off the UI
    /// thread.
    ///
    /// Errors: [`DspiError::Unsupported`] without the `groups` cap (nothing
    /// sent); [`DspiError::Disconnected`]; the hub's refusal.
    pub fn hub_groups(&self) -> Result<Vec<HubGroup>> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_GROUPS)?;
            let reply = self.hub_ask("group.list", Map::new())?;
            Ok(reply
                .get("groups")
                .and_then(Value::as_array)
                .map(|a| a.iter().take(GROUPS_MAX).map(group_from).collect())
                .unwrap_or_default())
        }
        #[cfg(not(feature = "link"))]
        Err(DspiError::Unsupported)
    }

    /// Store `group` on the hub (`group.set`): without an id a new group,
    /// with one that group replaced whole (an empty note clears it). The
    /// hub's bounds: a name of 1 to 48 characters, a note of at most 256,
    /// at most 64 members, each an inventory serial and each once; 64 groups.
    /// Admin of the hub. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] past a bound (`bad_request`);
    /// [`DspiError::Link`] `not_found` (a member not in the inventory, an
    /// unknown id), `conflict` (a 65th group); [`DspiError::Denied`] below
    /// Admin; [`DspiError::ReadOnly`] (nothing sent); as
    /// [`DspiCore::hub_groups`].
    pub fn hub_group_set(&self, group: HubGroup) -> Result<HubGroupSet> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_GROUPS)?;
            let mut g = Map::new();
            if !group.id.is_empty() {
                g.insert("id".into(), group.id.into());
            }
            g.insert("name".into(), group.name.into());
            if !group.note.is_empty() {
                g.insert("note".into(), group.note.into());
            }
            g.insert("members".into(), group.members.into());
            let mut f = Map::new();
            f.insert("group".into(), Value::Object(g));
            let reply = self.hub_ask("group.set", f)?;
            let stored = reply
                .get("group")
                .filter(|g| g.is_object())
                .map(group_from)
                .ok_or_else(|| protocol("group.set's answer has no group"))?;
            Ok(HubGroupSet {
                group: stored,
                created: reply
                    .get("created")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = group;
            Err(DspiError::Unsupported)
        }
    }

    /// Delete group `id` (`group.delete`; its id is never given again).
    /// Admin of the hub. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for an empty id (nothing sent);
    /// [`DspiError::Link`] `not_found`; as [`DspiCore::hub_group_set`].
    pub fn hub_group_delete(&self, id: String) -> Result<()> {
        if id.is_empty() {
            return Err(DspiError::InvalidArgument {
                msg: "delete names the group".into(),
            });
        }
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_GROUPS)?;
            let mut f = Map::new();
            f.insert("group".into(), id.into());
            self.hub_ask("group.delete", f).map(|_| ())
        }
        #[cfg(not(feature = "link"))]
        Err(DspiError::Unsupported)
    }

    /// Read `path` (`idx` its indices) on every device of `target`
    /// (`group.param.get`): one row per member, in member order, each with
    /// its value or its code. Every role. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Unsupported`] without the `group_params` cap
    /// (nothing sent); [`DspiError::InvalidArgument`] for a target that is
    /// both or neither (nothing sent) and the hub's `bad_request`;
    /// [`DspiError::Link`] `not_found` (the group), `busy` (this session's
    /// other group request runs); [`DspiError::Disconnected`].
    pub fn hub_group_param_get(
        &self,
        target: HubFleetTarget,
        path: String,
        idx: Vec<u32>,
    ) -> Result<HubGroupParams> {
        #[cfg(feature = "link")]
        {
            let mut f = Map::new();
            address(target, &mut f)?;
            self.hub_cap(CAP_GROUP_PARAMS)?;
            f.insert("path".into(), path.clone().into());
            f.insert("idx".into(), idx.into());
            let reply = self.hub_ask_long("group.param.get", f)?;
            Ok(params_from(&path, &reply))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = (target, path, idx);
            Err(DspiError::Unsupported)
        }
    }

    /// Write `value` to `path` on every device of `target`, one after
    /// another as this console's child session (`group.param.set`): each
    /// judged on its own by role, limits and lock; a refused row stops no
    /// other. `offsets` (dB paths only) add to the value per member, clamped
    /// into that device's range (`clamped` in its row). Control or Admin.
    /// Slow: off the UI thread.
    ///
    /// Errors: as [`DspiCore::hub_group_param_get`];
    /// [`DspiError::InvalidArgument`] for a structured value (nothing sent),
    /// a value outside the registry's range, offsets on a path not in dB;
    /// [`DspiError::Denied`] for a Viewer; [`DspiError::ReadOnly`] (nothing
    /// sent).
    pub fn hub_group_param_set(
        &self,
        target: HubFleetTarget,
        path: String,
        idx: Vec<u32>,
        value: ParamValue,
        offsets: Vec<HubOffset>,
    ) -> Result<HubGroupParams> {
        #[cfg(feature = "link")]
        {
            let mut f = Map::new();
            address(target, &mut f)?;
            let wire = value_to_wire(&value)?;
            self.hub_cap(CAP_GROUP_PARAMS)?;
            f.insert("path".into(), path.clone().into());
            f.insert("idx".into(), idx.into());
            f.insert("value".into(), wire);
            if !offsets.is_empty() {
                let mut o = Map::new();
                for x in offsets {
                    let db = serde_json::Number::from_f64(x.db).ok_or_else(|| {
                        DspiError::InvalidArgument {
                            msg: "an offset is a finite number of dB".into(),
                        }
                    })?;
                    o.insert(x.serial, Value::Number(db));
                }
                f.insert("offsets".into(), Value::Object(o));
            }
            let reply = self.hub_ask_long("group.param.set", f)?;
            Ok(params_from(&path, &reply))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = (target, path, idx, value, offsets);
            Err(DspiError::Unsupported)
        }
    }

    /// Load a preset on every device of `target` (`group.preset.load`): by
    /// `slot` (0 to 9) or by `name`, one of the two; `map` gives a member
    /// its own. One row per member, with the slot and name loaded. Control
    /// or Admin. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for both or neither of slot
    /// and name (nothing sent); as [`DspiCore::hub_group_param_set`].
    pub fn hub_group_preset_load(
        &self,
        target: HubFleetTarget,
        slot: Option<u8>,
        name: String,
        map: Vec<HubPresetPick>,
    ) -> Result<Vec<HubGroupRow>> {
        #[cfg(feature = "link")]
        {
            let mut f = Map::new();
            address(target, &mut f)?;
            match preset_ref(slot, &name)? {
                v @ Value::Number(_) => f.insert("slot".into(), v),
                v => f.insert("name".into(), v),
            };
            if !map.is_empty() {
                let mut m = Map::new();
                for p in map {
                    m.insert(p.serial, preset_ref(p.slot, &p.name)?);
                }
                f.insert("map".into(), Value::Object(m));
            }
            self.hub_cap(CAP_GROUP_PARAMS)?;
            let reply = self.hub_ask_long("group.preset.load", f)?;
            Ok(rows_from("", &reply))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = (target, slot, name, map);
            Err(DspiError::Unsupported)
        }
    }

    /// Apply one snapshot to every device of `target` (`group.apply`):
    /// exactly one of `snapshot` (an id in the hub's store), `name` (one
    /// snapshot's name in the store) or `document` (a snapshot document's
    /// text, at most 61 440 bytes compact). Each member is the hub's recall
    /// as this console's child session. With `guard`, one guarded apply per
    /// device (`timeout_ms` 1000 to 600 000; 0 for the hub's 30 s): confirm
    /// the run ([`DspiCore::hub_group_apply_confirm`]) or each reverts at its
    /// deadline. Control or Admin. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for a source that is not
    /// exactly one, or a document that is no JSON object (nothing sent);
    /// [`DspiError::Link`] `conflict` for a document over the wire cap
    /// (nothing sent) and two snapshots of that name; `not_found`; as
    /// [`DspiCore::hub_group_param_set`] (cap `group_apply`).
    #[allow(clippy::too_many_arguments)]
    pub fn hub_group_apply(
        &self,
        target: HubFleetTarget,
        snapshot: String,
        name: String,
        document: String,
        guard: bool,
        timeout_ms: u32,
    ) -> Result<HubGroupRun> {
        #[cfg(feature = "link")]
        {
            let mut f = Map::new();
            address(target, &mut f)?;
            let given = [&snapshot, &name, &document]
                .iter()
                .filter(|s| !s.is_empty())
                .count();
            if given != 1 {
                return Err(DspiError::InvalidArgument {
                    msg: "an apply takes one of a snapshot, a name or a document".into(),
                });
            }
            if !document.is_empty() {
                f.insert("document".into(), super::wire_document(&document)?);
            } else if !snapshot.is_empty() {
                f.insert("snapshot".into(), snapshot.into());
            } else {
                f.insert("name".into(), name.into());
            }
            self.hub_cap(CAP_GROUP_APPLY)?;
            if guard {
                f.insert("guard".into(), true.into());
                if timeout_ms > 0 {
                    f.insert("timeout_ms".into(), timeout_ms.into());
                }
            }
            let reply = self.hub_ask_long("group.apply", f)?;
            Ok(run_from(&reply))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = (target, snapshot, name, document, guard, timeout_ms);
            Err(DspiError::Unsupported)
        }
    }

    /// Keep every pending guard of bulk-apply run `run`
    /// (`group.apply.confirm`): one row per guard, `conflict` for one that
    /// had already reverted. Only the session that ran it. Control or Admin.
    /// Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Link`] `not_found` (an unknown run, or another
    /// session's); as [`DspiCore::hub_group_apply`].
    pub fn hub_group_apply_confirm(&self, run: String) -> Result<HubGroupRun> {
        self.hub_settle("group.apply.confirm", run)
    }

    /// Revert every pending guard of run `run` now (`group.apply.cancel`):
    /// one row per guard, `reverted` set. Errors: as
    /// [`DspiCore::hub_group_apply_confirm`].
    pub fn hub_group_apply_cancel(&self, run: String) -> Result<HubGroupRun> {
        self.hub_settle("group.apply.cancel", run)
    }

    /// The hub's firmware images (`fw.image.list`). Every role, over an
    /// encrypted session only. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Unsupported`] without the `rollout` cap (nothing
    /// sent); [`DspiError::Link`] `tls_required` over a plaintext session;
    /// [`DspiError::Disconnected`].
    pub fn hub_firmware_images(&self) -> Result<HubFirmwareImages> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_ROLLOUT)?;
            let reply = self.hub_ask("fw.image.list", Map::new())?;
            Ok(HubFirmwareImages {
                images: reply
                    .get("images")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().take(IMAGES_MAX).map(image_from).collect())
                    .unwrap_or_default(),
                max_images: small(reply.get("max_images")).unwrap_or(0),
                max_bytes: reply.get("max_bytes").and_then(Value::as_u64).unwrap_or(0),
            })
        }
        #[cfg(not(feature = "link"))]
        Err(DspiError::Unsupported)
    }

    /// Put firmware image `bytes` in the hub's store as `name` (`fw.image.put`,
    /// 36 KiB a chunk, one upload per session). The hub checks the whole
    /// image on the last chunk (`uf2_inspect`, the manifest by name and
    /// bytes). Nothing goes over a plaintext session: the store is read first
    /// (its `tls_required`). The bytes are never logged. Admin of the hub.
    /// Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for no bytes or more than
    /// [`FIRMWARE_MAX_BYTES`] (nothing sent), and the hub's `bad_request` (a
    /// name or image it refuses); [`DspiError::Link`] `conflict` (the name
    /// with other bytes; a fifth image), `busy` (an upload is open);
    /// [`DspiError::Denied`] below Admin; [`DspiError::ReadOnly`] (nothing
    /// sent); as [`DspiCore::hub_firmware_images`].
    pub fn hub_firmware_put(&self, name: String, bytes: Vec<u8>) -> Result<HubFirmwarePut> {
        if bytes.is_empty() || bytes.len() > FIRMWARE_MAX_BYTES {
            return Err(DspiError::InvalidArgument {
                msg: format!(
                    "a firmware image is 1 to {FIRMWARE_MAX_BYTES} bytes; this one is {}",
                    bytes.len()
                ),
            });
        }
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_ROLLOUT)?;
            // Read-only mode refuses the upload before anything is sent.
            crate::read_only::check_hub_request(&mut self.lock(), "fw.image.put")?;
            // The read answers tls_required over plaintext: no byte goes then.
            self.hub_ask("fw.image.list", Map::new())?;
            let chunks = bytes.chunks(CHUNK_BYTES);
            let n = chunks.len();
            for (seq, chunk) in chunks.enumerate() {
                let last = seq.saturating_add(1) == n;
                let mut f = Map::new();
                f.insert("name".into(), name.clone().into());
                f.insert("seq".into(), seq.into());
                f.insert("data".into(), base64(chunk).into());
                f.insert("last".into(), last.into());
                let reply = self.hub_ask("fw.image.put", f)?;
                if last {
                    let image = reply
                        .get("image")
                        .filter(|i| i.is_object())
                        .map(image_from)
                        .ok_or_else(|| protocol("fw.image.put's answer has no image"))?;
                    return Ok(HubFirmwarePut {
                        image,
                        created: reply
                            .get("created")
                            .and_then(Value::as_bool)
                            .unwrap_or(true),
                    });
                }
            }
            Err(protocol("the upload ended without its last chunk"))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = name;
            Err(DspiError::Unsupported)
        }
    }

    /// Delete image `name` from the hub's store (`fw.image.delete`). Admin
    /// of the hub. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Link`] `not_found`, `conflict` (a running
    /// rollout uses it); as [`DspiCore::hub_firmware_put`].
    pub fn hub_firmware_delete(&self, name: String) -> Result<()> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_ROLLOUT)?;
            let mut f = Map::new();
            f.insert("image".into(), name.into());
            self.hub_ask("fw.image.delete", f).map(|_| ())
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = name;
            Err(DspiError::Unsupported)
        }
    }

    /// Start a firmware rollout of stored image `image` on `target`: one
    /// serial, or a group (one member after another, stopped at the first
    /// failure) (`fw.rollout.start`). The device must be muted unless
    /// `while_live`. The answer comes after the first device's check and
    /// guard steps; follow the rest with `fw.rollout.changed`
    /// ([`super::HubServerEvent::rollout`]) or [`DspiCore::hub_rollouts`].
    /// Admin of the hub, over an encrypted session. Slow: off the UI thread.
    ///
    /// Errors (nothing touched): [`DspiError::InvalidArgument`] for a target
    /// that is not one serial or one group (nothing sent);
    /// [`DspiError::Unsupported`] (not on the hub's own USB);
    /// [`DspiError::Locked`]; [`DspiError::ConfirmationRequired`] (not
    /// muted); [`DspiError::ImageRefused`] (another platform, not verified,
    /// the firmware guard's refusal); [`DspiError::Link`] `offline`,
    /// `conflict`, `busy`, `not_found`, `tls_required`; as
    /// [`DspiCore::hub_firmware_put`].
    pub fn hub_rollout_start(
        &self,
        target: HubFleetTarget,
        image: String,
        while_live: bool,
    ) -> Result<HubRollout> {
        #[cfg(feature = "link")]
        {
            let mut f = Map::new();
            match (target.group.is_empty(), target.serials.as_slice()) {
                (false, []) => f.insert("group".into(), target.group.into()),
                (true, [one]) => f.insert("serial".into(), one.clone().into()),
                _ => {
                    return Err(DspiError::InvalidArgument {
                        msg: "a rollout names one serial or one group".into(),
                    });
                }
            };
            self.hub_cap(CAP_ROLLOUT)?;
            f.insert("image".into(), image.into());
            if while_live {
                f.insert("while_live".into(), true.into());
            }
            let reply = self.hub_ask("fw.rollout.start", f)?;
            reply
                .get("rollout")
                .filter(|r| r.is_object())
                .map(rollout_from)
                .ok_or_else(|| protocol("fw.rollout.start's answer has no rollout"))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = (target, image, while_live);
            Err(DspiError::Unsupported)
        }
    }

    /// The hub's rollouts, newest first (`fw.rollout.status`; the hub keeps
    /// 64), or only rollout `id` ("" for all). Every role, over an encrypted
    /// session. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Link`] `not_found` for an unknown id; as
    /// [`DspiCore::hub_firmware_images`].
    pub fn hub_rollouts(&self, id: String) -> Result<Vec<HubRollout>> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_ROLLOUT)?;
            let mut f = Map::new();
            if !id.is_empty() {
                f.insert("rollout".into(), id.into());
            }
            let reply = self.hub_ask("fw.rollout.status", f)?;
            Ok(reply
                .get("rollouts")
                .and_then(Value::as_array)
                .map(|a| a.iter().take(ROLLOUTS_MAX).map(rollout_from).collect())
                .unwrap_or_default())
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = id;
            Err(DspiError::Unsupported)
        }
    }

    /// Cancel rollout `id` (`fw.rollout.cancel`): before the bootloader entry
    /// of the device it is working on, never during a flash. The record as
    /// it is now. Admin of the hub. Slow: off the UI thread.
    ///
    /// Errors: [`DspiError::Link`] `conflict` (past the entry, or ended),
    /// `not_found`; as [`DspiCore::hub_rollout_start`].
    pub fn hub_rollout_cancel(&self, id: String) -> Result<HubRollout> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_ROLLOUT)?;
            let mut f = Map::new();
            f.insert("rollout".into(), id.into());
            let reply = self.hub_ask("fw.rollout.cancel", f)?;
            reply
                .get("rollout")
                .filter(|r| r.is_object())
                .map(rollout_from)
                .ok_or_else(|| protocol("fw.rollout.cancel's answer has no rollout"))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = id;
            Err(DspiError::Unsupported)
        }
    }
}

impl DspiCore {
    /// [`DspiCore::hub_firmware_put`] with the image as base64 (RFC 4648,
    /// padded), for a binding that carries text (the web).
    pub fn hub_firmware_put_base64(&self, name: String, data: &str) -> Result<HubFirmwarePut> {
        let bytes = unbase64(data).ok_or_else(|| DspiError::InvalidArgument {
            msg: "the image is not base64 (RFC 4648, padded)".into(),
        })?;
        self.hub_firmware_put(name, bytes)
    }

    fn hub_settle(&self, t: &str, run: String) -> Result<HubGroupRun> {
        if run.is_empty() {
            return Err(DspiError::InvalidArgument {
                msg: "confirm and cancel name the run".into(),
            });
        }
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_GROUP_APPLY)?;
            let mut f = Map::new();
            f.insert("run".into(), run.into());
            let reply = self.hub_ask_long(t, f)?;
            Ok(run_from(&reply))
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = t;
            Err(DspiError::Unsupported)
        }
    }
}

#[cfg(feature = "link")]
impl DspiCore {
    /// [`Self::hub_ask`] for a request that answers after every member.
    fn hub_ask_long(&self, t: &str, fields: Map<String, Value>) -> Result<Value> {
        self.link_hub_request_within(
            t,
            fields,
            std::time::Duration::from_secs(GROUP_REQUEST_TIMEOUT_S),
        )
        .map_err(super::hub_error)
    }
}

#[cfg_attr(not(feature = "link"), allow(dead_code))]
fn params_from(path: &str, v: &Value) -> HubGroupParams {
    HubGroupParams {
        path: v
            .get("path")
            .and_then(Value::as_str)
            .map_or_else(|| path.to_owned(), |p| cut(p, 300)),
        idx: v
            .get("idx")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .take(8)
                    .filter_map(|x| small::<u32>(Some(x)))
                    .collect()
            })
            .unwrap_or_default(),
        results: rows_from(path, v),
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `bytes` in base64 (RFC 4648, padded).
#[cfg_attr(not(feature = "link"), allow(dead_code))]
pub(crate) fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3).saturating_mul(4));
    for c in bytes.chunks(3) {
        let n = u32::from_be_bytes([
            0,
            c.first().copied().unwrap_or(0),
            c.get(1).copied().unwrap_or(0),
            c.get(2).copied().unwrap_or(0),
        ]);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if i <= c.len() {
                let six = n.wrapping_shr(shift) & 0x3F;
                out.push(B64.get(six as usize).map_or('A', |&b| char::from(b)));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Base64 (RFC 4648, padded) as bytes; `None` when it is not.
pub(crate) fn unbase64(text: &str) -> Option<Vec<u8>> {
    let t = text.trim().as_bytes();
    let (quads, rest) = t.as_chunks::<4>();
    if !rest.is_empty() {
        return None;
    }
    let count = quads.len();
    let val = |c: u8| {
        B64.iter()
            .position(|&x| x == c)
            .and_then(|p| u32::try_from(p).ok())
    };
    let mut out = Vec::with_capacity(t.len());
    for (q, quad) in quads.iter().enumerate() {
        let pad = quad.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && q.saturating_add(1) != count) {
            return None;
        }
        let mut n = 0u32;
        for (i, &c) in quad.iter().enumerate() {
            let v = if i >= 4usize.saturating_sub(pad) {
                0
            } else {
                val(c)?
            };
            n = n.wrapping_shl(6) | v;
        }
        let b = n.to_be_bytes();
        out.extend_from_slice(b.get(1..4usize.saturating_sub(pad))?);
    }
    Some(out)
}

#[cfg(test)]
#[path = "tests_wp_p3cc.rs"]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p3cc;
