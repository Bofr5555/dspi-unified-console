//! What the agents did: one entry per tool call, newest last, the oldest
//! dropped past [`ACTIVITY_CAP`]. The app's AI panel (MCP4) shows it, and an
//! agent reads its own through `dspi_get_activity`. An entry names its
//! journal ids, so the panel's per-row Undo is `revert_entry(id)`.
//!
//! No untrusted text is kept here beyond the client's cleaned label: the
//! summary is the server's own words.

use std::collections::VecDeque;

use serde_json::{Value, json};

use crate::ClientId;

/// The most entries kept.
pub const ACTIVITY_CAP: usize = 1000;

/// How a call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// It ran.
    Ok,
    /// It ran and reported an error (a bad argument, a device error, a refusal
    /// by the core or the policy).
    Error,
    /// The rate limiter held it back; nothing ran.
    RateLimited,
    // -- WP MCP2a --
    /// The policy held it for the user's confirmation; nothing ran yet. The
    /// answer is an entry of its own.
    Held,
}

impl Outcome {
    /// The word in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::RateLimited => "rate_limited",
            Self::Held => "held", // -- WP MCP2a --
        }
    }
}

/// One tool call.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityEntry {
    /// Its number, from 1, unique for the server's life.
    pub id: u64,
    /// When, in the clock's milliseconds (`Server::handle`'s `now_ms`).
    pub at_ms: u64,
    /// The client that called.
    pub client: ClientId,
    /// The client's label ("AI · Claude Code"), cleaned.
    pub client_label: String,
    /// The tool.
    pub tool: String,
    /// The device it addressed (a `DeviceId`), if any.
    pub device: Option<String>,
    /// Whether it wrote (a write-class tool).
    pub write: bool,
    /// How it ended.
    pub outcome: Outcome,
    /// The journal ids of the changes it made, for `revert_entry`.
    pub journal_ids: Vec<u64>,
    /// In the server's words: the error's text, or "".
    pub detail: String,
}

impl ActivityEntry {
    /// As JSON, for a tool result or the app.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "at_ms": self.at_ms,
            "client": self.client.0,
            "client_name": self.client_label,
            "tool": self.tool,
            "device": self.device,
            "write": self.write,
            "outcome": self.outcome.as_str(),
            "journal_ids": self.journal_ids,
            "detail": self.detail,
        })
    }
}

/// The ring.
#[derive(Debug, Default)]
pub struct Activity {
    entries: VecDeque<ActivityEntry>,
    next: u64,
}

impl Activity {
    /// Add `entry` (its `id` is set here) and give its id.
    pub fn push(&mut self, mut entry: ActivityEntry) -> u64 {
        self.next = self.next.saturating_add(1);
        entry.id = self.next;
        if self.entries.len() >= ACTIVITY_CAP {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
        self.next
    }

    /// The newest `n`, oldest first; only `client`'s with `Some`.
    pub fn recent(&self, n: usize, client: Option<ClientId>) -> Vec<ActivityEntry> {
        let mut out: Vec<ActivityEntry> = self
            .entries
            .iter()
            .rev()
            .filter(|e| client.is_none_or(|c| e.client == c))
            .take(n)
            .cloned()
            .collect();
        out.reverse();
        out
    }

    /// How many entries are kept.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether none are.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
