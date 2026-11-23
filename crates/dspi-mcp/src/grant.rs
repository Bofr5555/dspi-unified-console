//! Grouped grants (WP ODEC; owner 2026-11-23, answer 6): one question lets
//! an agent play test signals on one device for the task at hand, for
//! [`GRANT_MS`], instead of a question for every measurement, low-end search,
//! sum check and verify.
//!
//! Under "ask about risky changes" and "ask about every change", a held call
//! whose risks include test signals asks it this way (the confirmation's
//! `grant`); the user's Allow then also opens a grant for that agent on that
//! device. While it lasts, that agent's test signals on that device are not
//! asked again. Louder changes, preset saves and every other risk are still
//! asked one at a time. The app shows each grant with the time left and a
//! Revoke ([`Grants::revoke`]); it ends with its time, a Revoke, the agent
//! going, or Stop. Under "never ask" nothing is asked, so nothing is granted.

use serde_json::{Value, json};

use crate::ClientId;
use crate::policy::{Risk, RiskKind, WriteKey};

/// How long a grant lasts, ms.
pub const GRANT_MS: u64 = 600_000;

/// The kind of grant in JSON: test signals.
pub const TEST_SIGNALS: &str = "test_signals";

/// The key a call's judgement gets while its agent holds a grant on its
/// device: `policy::require` lets test signals pass on it.
pub fn standing_key() -> WriteKey {
    WriteKey::new(
        "standing.test_signal",
        &[],
        dspi_core::ParamValue::Trigger,
    )
}

/// Whether a held call's risks would open a grant when allowed.
pub fn asks_grant(risks: &[Risk]) -> bool {
    risks.iter().any(|r| r.kind == RiskKind::TestSignal)
}

/// One grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// Its number, from 1.
    pub id: u64,
    /// The agent it is for.
    pub client: ClientId,
    /// Its label, for the app.
    pub client_label: String,
    /// The device (a `DeviceId`'s text).
    pub device: String,
    /// When it was given and when it ends, on the server's clock.
    pub since_ms: u64,
    /// When it ends.
    pub until_ms: u64,
}

impl Grant {
    /// As JSON for the app: `{id, kind, client, client_name, device,
    /// since_ms, until_ms}`.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "kind": TEST_SIGNALS,
            "client": self.client.0,
            "client_name": self.client_label,
            "device": self.device,
            "since_ms": self.since_ms,
            "until_ms": self.until_ms,
        })
    }
}

/// The grants that last.
#[derive(Debug, Default)]
pub struct Grants {
    list: Vec<Grant>,
    next: u64,
}

impl Grants {
    /// Give `client` test signals on `device` from `now_ms`: a grant it held
    /// there already starts again. Its id.
    pub fn give(&mut self, client: ClientId, label: &str, device: &str, now_ms: u64) -> u64 {
        self.list
            .retain(|g| !(g.client == client && g.device == device));
        self.next = self.next.saturating_add(1);
        self.list.push(Grant {
            id: self.next,
            client,
            client_label: label.to_owned(),
            device: device.to_owned(),
            since_ms: now_ms,
            until_ms: now_ms.saturating_add(GRANT_MS),
        });
        self.next
    }

    /// Whether `client` holds a grant on `device`.
    pub fn holds(&self, client: ClientId, device: &str) -> bool {
        self.list
            .iter()
            .any(|g| g.client == client && g.device == device)
    }

    /// End grant `id` (the user's Revoke): the grant, `None` for none.
    pub fn revoke(&mut self, id: u64) -> Option<Grant> {
        let at = self.list.iter().position(|g| g.id == id)?;
        Some(self.list.remove(at))
    }

    /// End the grants whose time is up, and those of agents that are gone
    /// or devices that are closed (`keep` false): the ended ones.
    pub fn prune(&mut self, now_ms: u64, keep: impl Fn(&Grant) -> bool) -> Vec<Grant> {
        let (ended, kept): (Vec<Grant>, Vec<Grant>) = std::mem::take(&mut self.list)
            .into_iter()
            .partition(|g| g.until_ms <= now_ms || !keep(g));
        self.list = kept;
        ended
    }

    /// End every grant of `client`.
    pub fn forget(&mut self, client: ClientId) {
        self.list.retain(|g| g.client != client);
    }

    /// The grants, oldest first.
    pub fn list(&self) -> &[Grant] {
        &self.list
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests_wp_odec {
    use super::*;

    #[test]
    fn wp_odec_a_grant_lasts_ten_minutes_and_is_per_agent_and_device() {
        let mut g = Grants::default();
        let a = ClientId(1);
        let id = g.give(a, "AI · A", "demo:1", 1_000);
        assert!(g.holds(a, "demo:1"));
        assert!(!g.holds(a, "demo:2"));
        assert!(!g.holds(ClientId(2), "demo:1"));
        assert!(g.prune(1_000 + GRANT_MS - 1, |_| true).is_empty());
        assert_eq!(g.prune(1_000 + GRANT_MS, |_| true)[0].id, id);
        assert!(!g.holds(a, "demo:1"));
        let id = g.give(a, "AI · A", "demo:1", 5_000);
        assert_eq!(g.revoke(id).unwrap().device, "demo:1");
        assert!(g.revoke(id).is_none());
    }
}
