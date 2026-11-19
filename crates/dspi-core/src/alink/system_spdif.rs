//! WP ATQT: the S/PDIF transport in the native System editor (PLAN-AT 1.2,
//! the hub's model as WP ATHUBM made it; the web's `SpdifUnits.tsx`, WP
//! ATWEB, is the same panel). Between DSPi units S/PDIF carries the time
//! base in its user bits: a **source** stamps its frame counter on the
//! output slots of `spdifTxMask` (never a speaker), a **follower**
//! (`"transport": "spdif"`, its `upstream` the source) plays each sample its
//! D after it arrived. Foreign S/PDIF carries none, and a follower then says
//! so ("this S/PDIF source does not carry a DSPi time base").
//!
//! Two parts, both over what [`DspiCore::alink_system_view`] already read
//! (no request of their own: the hub answers `system.status` once a second):
//! - `DspiCore::alink_system_spdif`: the S/PDIF rows of the last reading
//!   (kept with the view, so a `stale` view has them too);
//! - `DspiCore::alink_system_save_spdif`: the "Clock and audio links" edits
//!   and the S/PDIF ones as one `system.set`, guarded by the revision as
//!   `DspiCore::alink_system_save_report`.
//!
//! Neither is exported over UniFFI (the C ABI's `dspi_alink_system_view` and
//! `_save` carry them for Qt); the hub validates the model it is sent (no
//! speaker on a source's time-base slot, no follower of a follower).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::alink_system::{AlinkSystemEdits, AlinkSystemSaved, apply_edits};
use crate::{DspiCore, DspiError, Result};

/// One S/PDIF unit of the installation (a source or a follower).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlinkSpdifUnit {
    /// The model's id.
    pub id: String,
    /// Its name, or the id when it has none.
    pub name: String,
    /// Its serial; "" when the model has none.
    pub serial: String,
    /// `source` or `follower`.
    pub role: String,
    /// A follower's source (its `upstream`); "" for a source.
    pub source: String,
    /// That source's name; "" for none.
    pub source_name: String,
    /// A source's time-base slots: bit n is outputs 2n + 1 and 2n + 2.
    pub tx_mask: u8,
    /// [`Self::tx_mask`] in words ("Outputs 7-8"; "None").
    pub slots_word: String,
    /// The D a source asks for, samples; none: the hub's.
    pub d: Option<u16>,
    /// The D the unit reports, samples.
    pub unit_d: Option<u16>,
    /// The D range a follower takes now.
    pub d_min: Option<u16>,
    /// Its top.
    pub d_max: Option<u16>,
    /// Attached to the hub now.
    pub connected: bool,
    /// The contract state as the hub read it (`none`, `acquiring`,
    /// `anchoring`, `locked`, `holdover`, `lock_error`), or `offline`,
    /// `no_spdif` (its firmware has no S/PDIF time base), `not_read`.
    pub state: String,
    /// [`Self::state`] in words.
    pub state_word: String,
    /// In lock error now.
    pub lock_error: bool,
    /// A follower that hears no DSPi time base: other equipment's S/PDIF, or
    /// a DSPi not set as a source (notice `spdif_no_time_base`).
    pub no_time_base: bool,
}

/// One S/PDIF edit ("Clock and audio links", Installer).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AlinkSpdifEdit {
    /// The unit's id.
    pub unit_id: String,
    /// `source` (a time-base source on `tx_mask`, default outputs 7 and 8),
    /// `follower` (fed by S/PDIF from `source`), `off` (back on the audio
    /// link, fed by the model's first sender); none: the role unchanged.
    pub role: Option<String>,
    /// A follower's source (a unit that is one).
    pub source: Option<String>,
    /// A source's time-base slots (not 0).
    pub tx_mask: Option<u8>,
    /// Change a source's D: `d` is the new one, none the hub's.
    pub set_d: bool,
    /// The D, samples.
    pub d: Option<u16>,
}

/// The slot mask a new source gets: outputs 7 and 8.
const DEFAULT_TX_MASK: u8 = 0b1000;

fn str_at<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn u16_at(v: Option<&Value>, key: &str) -> Option<u16> {
    v.and_then(|v| v.get(key))
        .and_then(Value::as_u64)
        .and_then(|n| u16::try_from(n).ok())
}

fn units(model: &Value) -> &[Value] {
    model
        .get("units")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// A unit fed by S/PDIF (a follower, or a source fed by nothing the model holds).
fn spdif_fed(u: &Value) -> bool {
    str_at(u, "transport") == "spdif"
}

fn spdif_source(u: &Value) -> bool {
    u.get("spdifSource").and_then(Value::as_bool) == Some(true)
}

/// The slots in words: "Outputs 1-2, 7-8", "None".
pub(crate) fn slots_word(mask: u8) -> String {
    let on: Vec<String> = (0u8..4)
        .filter(|n| mask & (1 << n) != 0)
        .map(|n| {
            let first = u32::from(n).saturating_mul(2).saturating_add(1);
            format!("{first}-{}", first.saturating_add(1))
        })
        .collect();
    if on.is_empty() {
        "None".into()
    } else {
        format!("Outputs {}", on.join(", "))
    }
}

/// The contract state in words.
pub(crate) fn spdif_state_word(state: &str) -> &'static str {
    match state {
        "locked" => "Locked",
        "acquiring" => "Acquiring",
        "anchoring" => "Anchoring",
        "holdover" => "Holdover",
        "lock_error" => "Lock error",
        "none" => "No S/PDIF time base",
        "offline" => "Not connected",
        "no_spdif" => "No S/PDIF time base (firmware)",
        _ => "Not read",
    }
}

/// The S/PDIF rows of `model` as `status` observed them.
pub(crate) fn spdif_rows(model: &Value, status: &Value) -> Vec<AlinkSpdifUnit> {
    let all = units(model);
    let name_of = |u: &Value| -> String {
        let n = str_at(u, "name");
        if n.is_empty() { str_at(u, "id") } else { n }.to_owned()
    };
    all.iter()
        .filter(|u| spdif_fed(u) || spdif_source(u))
        .map(|u| {
            let serial = str_at(u, "serial");
            let o = (!serial.is_empty())
                .then(|| status.get("observed").and_then(|o| o.get(serial)))
                .flatten();
            let connected = o.and_then(|o| o.get("online")).and_then(Value::as_bool) == Some(true);
            let seen = o.and_then(|o| o.get("spdif")).filter(|v| v.is_object());
            let block_none = o.map(|o| str_at(o, "spdifBlock")) == Some("none");
            let source = spdif_source(u);
            let follower = spdif_fed(u) && !source;
            let upstream = if follower { str_at(u, "upstream") } else { "" };
            let state = if !connected {
                "offline".to_owned()
            } else if block_none {
                "no_spdif".to_owned()
            } else {
                seen.map(|s| str_at(s, "state"))
                    .filter(|s| !s.is_empty())
                    .unwrap_or("not_read")
                    .to_owned()
            };
            let flag = |k: &str| seen.and_then(|s| s.get(k)).and_then(Value::as_bool) == Some(true);
            let lock_error = flag("lockError");
            let mode_follower = seen.map(|s| str_at(s, "mode")) == Some("follower");
            let tx_mask = u
                .get("spdifTxMask")
                .and_then(Value::as_u64)
                .and_then(|n| u8::try_from(n).ok())
                .unwrap_or(0);
            AlinkSpdifUnit {
                id: str_at(u, "id").to_owned(),
                name: name_of(u),
                serial: serial.to_owned(),
                role: if source { "source" } else { "follower" }.to_owned(),
                source: upstream.to_owned(),
                source_name: all
                    .iter()
                    .find(|x| !upstream.is_empty() && str_at(x, "id") == upstream)
                    .map(name_of)
                    .unwrap_or_default(),
                tx_mask,
                slots_word: if source {
                    slots_word(tx_mask)
                } else {
                    String::new()
                },
                d: u16_at(Some(u), "spdifD"),
                unit_d: u16_at(seen, "d"),
                d_min: u16_at(seen, "dMin").filter(|d| *d > 0),
                d_max: u16_at(seen, "dMax").filter(|d| *d > 0),
                connected,
                state_word: spdif_state_word(&state).to_owned(),
                state,
                lock_error,
                no_time_base: connected
                    && mode_follower
                    && (flag("foreign") || (lock_error && !flag("superframe"))),
            }
        })
        .collect()
}

fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

/// `edits` made on `model` (the hub's JSON). An id the model lacks, a
/// follower of a unit that is no source, an empty slot mask are refused,
/// so nothing is half applied; the hub judges the rest.
pub(crate) fn apply_spdif(model: &mut Value, edits: &[AlinkSpdifEdit]) -> Result<()> {
    for e in edits {
        let snapshot = model.clone();
        let all = units(&snapshot);
        let first_sender = snapshot
            .get("senders")
            .and_then(Value::as_array)
            .and_then(|s| s.first())
            .map(|s| str_at(s, "id").to_owned())
            .unwrap_or_default();
        let before = all
            .iter()
            .find(|u| str_at(u, "id") == e.unit_id)
            .ok_or_else(|| invalid(format!("no unit {}", e.unit_id)))?;
        if str_at(before, "transport") == "usb_direct" {
            return Err(invalid(format!(
                "unit {} is fed by USB: it cannot be on S/PDIF",
                e.unit_id
            )));
        }
        let was_source = spdif_source(before);
        let was_fed = spdif_fed(before);
        let unit = model
            .get_mut("units")
            .and_then(Value::as_array_mut)
            .and_then(|us| us.iter_mut().find(|x| str_at(x, "id") == e.unit_id))
            .and_then(Value::as_object_mut)
            .ok_or_else(|| invalid(format!("no unit {}", e.unit_id)))?;
        let is_source = match e.role.as_deref() {
            None => was_source,
            Some("source") => {
                unit.insert("spdifSource".into(), Value::Bool(true));
                if !was_source {
                    unit.insert("spdifTxMask".into(), Value::from(DEFAULT_TX_MASK));
                }
                true
            }
            Some("follower") => {
                let src = e.source.as_deref().unwrap_or("");
                let ok = all
                    .iter()
                    .any(|x| str_at(x, "id") == src && src != e.unit_id && spdif_source(x));
                if !ok {
                    return Err(invalid(format!(
                        "unit {}: {src:?} is not an S/PDIF time-base source",
                        e.unit_id
                    )));
                }
                // What S/PDIF cannot carry goes (as the web's withSpdifFollower)
                for k in [
                    "slots",
                    "hopDelayFrames",
                    "usbMode",
                    "repeater",
                    "output",
                    "spdifSource",
                    "spdifTxMask",
                    "spdifD",
                ] {
                    unit.remove(k);
                }
                unit.insert("transport".into(), Value::from("spdif"));
                unit.insert("upstream".into(), Value::from(src));
                false
            }
            Some("off") => {
                for k in ["spdifSource", "spdifTxMask", "spdifD", "transport"] {
                    unit.remove(k);
                }
                if was_fed {
                    let up = if was_source {
                        let u = str_at(before, "upstream");
                        if u.is_empty() {
                            first_sender.as_str()
                        } else {
                            u
                        }
                    } else {
                        first_sender.as_str()
                    };
                    unit.insert("upstream".into(), Value::from(up));
                } else if let Some(t) = before.get("transport") {
                    // Not fed by S/PDIF: its own transport stays
                    unit.insert("transport".into(), t.clone());
                }
                false
            }
            Some(other) => {
                return Err(invalid(format!(
                    "S/PDIF role {other:?}: source, follower or off"
                )));
            }
        };
        if let Some(mask) = e.tx_mask {
            if !is_source {
                return Err(invalid(format!(
                    "unit {}: only an S/PDIF source has time-base slots",
                    e.unit_id
                )));
            }
            if mask == 0 || mask > 0b1111 {
                return Err(invalid(format!(
                    "unit {}: an S/PDIF source carries its time base on 1 to 4 output slots",
                    e.unit_id
                )));
            }
            unit.insert("spdifTxMask".into(), Value::from(mask));
        }
        if e.set_d {
            if !is_source {
                return Err(invalid(format!(
                    "unit {}: only an S/PDIF source sets the domain's D",
                    e.unit_id
                )));
            }
            match e.d {
                Some(d) => {
                    unit.insert("spdifD".into(), Value::from(d));
                }
                None => {
                    unit.remove("spdifD");
                }
            }
        }
    }
    Ok(())
}

impl DspiCore {
    /// The installation's S/PDIF units (sources and followers) as the last
    /// [`DspiCore::alink_system_view`] read them, also while that view is
    /// given again (`stale`): no request of its own. Empty before a view,
    /// off a hub, or with no S/PDIF unit.
    pub fn alink_system_spdif(&self) -> Vec<AlinkSpdifUnit> {
        self.alink_system_spdif_rows()
    }

    /// [`DspiCore::alink_system_save_report`] with the S/PDIF edits too:
    /// both made on the view of `revision`, sent as one `system.set`.
    /// Admin only. Errors as there, and
    /// [`DspiError::InvalidArgument`] for an S/PDIF edit that cannot be
    /// made (nothing is sent).
    pub fn alink_system_save_spdif(
        &self,
        revision: u64,
        edits: AlinkSystemEdits,
        spdif: Vec<AlinkSpdifEdit>,
    ) -> Result<AlinkSystemSaved> {
        self.alink_system_save_with(revision, |model| {
            apply_edits(model, &edits)?;
            apply_spdif(model, &spdif)
        })
    }
}
