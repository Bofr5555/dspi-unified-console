//! WP FEATUI (AIVIS's web part): who works on the hub's devices beside the
//! people (`dspi-hub`'s `presence` family). A console that runs an AI agent
//! says so while the agent works ([`DspiCore::hub_presence_set`], again
//! within its time to keep it); every console hears `presence.changed` (a
//! [`super::HubServerEvent`] of that kind) and reads the list
//! ([`DspiCore::hub_presence_list`]) to show a strip. Rust, C and wasm only.

use serde::Serialize;
#[cfg(feature = "link")]
use serde_json::Value;

#[cfg(not(feature = "link"))]
use crate::DspiError;
use crate::{DspiCore, Result};

/// The hub's cap.
pub const CAP_PRESENCE: &str = "presence";

/// One entry of the hub's list. The texts are from elsewhere (the hub drops
/// control and format characters and bounds them): show them as names.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubPresence {
    /// `agent`.
    pub kind: String,
    /// The agent's name, as its console says it.
    pub label: String,
    /// The console it works through, in the hub's own words.
    pub by: String,
    /// The device it works on; "" for none named.
    pub serial: String,
    /// What it does now, in words; "" for nothing said.
    pub what: String,
    /// Seconds left unless set again.
    pub ttl_s: u64,
}

impl DspiCore {
    /// Say on the hub that an agent (`label`) works through this console on
    /// `serial` doing `what`, for `ttl_s` seconds (5 to 60) unless said again;
    /// `ttl_s` 0 clears it. Control or Admin. Slow: off the UI thread.
    ///
    /// Errors: [`crate::DspiError::Unsupported`] without the `presence` cap
    /// (nothing sent); [`crate::DspiError::Disconnected`]; the hub's refusal.
    pub fn hub_presence_set(
        &self,
        label: String,
        serial: String,
        what: String,
        ttl_s: u32,
    ) -> Result<()> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_PRESENCE)?;
            let mut f = serde_json::Map::new();
            f.insert("ttl_s".into(), ttl_s.into());
            if ttl_s > 0 {
                f.insert("kind".into(), "agent".into());
                f.insert("label".into(), label.into());
                f.insert("serial".into(), serial.into());
                f.insert("what".into(), what.into());
            }
            self.hub_ask("presence.set", f).map(|_| ())
        }
        #[cfg(not(feature = "link"))]
        {
            let _ = (label, serial, what, ttl_s);
            Err(DspiError::Unsupported)
        }
    }

    /// Who works on the hub's devices now, by session. Every role.
    ///
    /// Errors: as [`Self::hub_presence_set`].
    pub fn hub_presence_list(&self) -> Result<Vec<HubPresence>> {
        #[cfg(feature = "link")]
        {
            self.hub_cap(CAP_PRESENCE)?;
            let v = self.hub_ask("presence.list", serde_json::Map::new())?;
            Ok(rows(&v))
        }
        #[cfg(not(feature = "link"))]
        Err(DspiError::Unsupported)
    }
}

/// The list in the hub's answer, bounded as the hub bounds it.
#[cfg(feature = "link")]
pub(crate) fn rows(v: &Value) -> Vec<HubPresence> {
    let text = |r: &Value, k: &str, max: usize| -> String {
        r.get(k)
            .and_then(Value::as_str)
            .map(|t| t.chars().take(max).collect())
            .unwrap_or_default()
    };
    v.get("presence")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(64)
        .map(|r| HubPresence {
            kind: text(r, "kind", 16),
            label: text(r, "label", 64),
            by: text(r, "by", 64),
            serial: text(r, "serial", 64),
            what: text(r, "what", 160),
            ttl_s: r.get("ttl_s").and_then(Value::as_u64).unwrap_or(0),
        })
        .collect()
}
