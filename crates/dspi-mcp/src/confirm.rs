//! Confirmations (WP MCP2a): a tool call the policy holds for the user's
//! word, for up to [`CONFIRM_HOLD_MS`].
//!
//! The server opens one when a call's writes need asking
//! (`policy::Judging::asked`); nothing has been sent then. The app hears of
//! it as a [`ConfirmEvent`] (`Server::take_confirm_events`), shows the
//! sheet from [`Confirmation::to_json`], and answers with
//! `Server::answer_confirmation`. **Allow** runs the call again at once with
//! a grant for exactly the writes the user saw; anything else it would write
//! is judged afresh. **Deny**, no answer within the hold, or the agent gone
//! end it with nothing changed.
//!
//! The agent gets the outcome one of two ways: a transport that can hold a
//! request (`Server::handle_held`) answers it once the confirmation ends
//! (`Server::resume`); any other gets "awaiting confirmation" at once and
//! asks `dspi_get_confirmation`.

use std::collections::VecDeque;

use serde_json::{Map, Value, json};

use crate::ClientId;
use crate::policy::{Risk, WriteKey};

/// How long a confirmation waits for the user (MCPDES 3.3).
pub const CONFIRM_HOLD_MS: u64 = 120_000;
/// The most confirmations one client may have waiting at once.
pub const WAITING_PER_CLIENT: usize = 3;
/// Ended confirmations kept for `dspi_get_confirmation`.
const ENDED_KEPT: usize = 64;

/// Where a confirmation stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmState {
    /// Waiting for the user.
    Waiting,
    /// The user allowed it; the call ran (its result may still be an
    /// error, from the device or a change since).
    Allowed,
    /// The user declined it; nothing was changed.
    Denied,
    /// Nobody answered within the hold; nothing was changed.
    Expired,
    /// The agent disconnected before the answer; nothing was changed.
    Gone,
}

impl ConfirmState {
    /// The word in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::Expired => "expired",
            Self::Gone => "gone",
        }
    }
}

/// One held call.
#[derive(Debug, Clone)]
pub struct Confirmation {
    /// Its number, from 1, unique for the server's life.
    pub id: u64,
    /// The client that called.
    pub client: ClientId,
    /// Its label ("AI · Claude Code"), cleaned.
    pub client_label: String,
    /// The tool.
    pub tool: &'static str,
    /// The device it addresses.
    pub device: Option<String>,
    /// What the user is asked about.
    pub risks: Vec<Risk>,
    /// When it was asked (the server's clock).
    pub asked_ms: u64,
    /// When it lapses unanswered.
    pub expires_ms: u64,
    /// Where it stands.
    pub state: ConfirmState,
    /// The call's arguments, for the re-run.
    pub(crate) args: Map<String, Value>,
    /// The tool result the agent gets, once it has ended.
    pub(crate) result: Option<Value>,
    /// A held request: its JSON-RPC id and whether it was modern-era.
    pub(crate) request: Option<(Value, bool)>,
}

impl Confirmation {
    /// The writes the user allows by Allow: those the risks name.
    pub(crate) fn grant(&self) -> Vec<WriteKey> {
        self.risks.iter().flat_map(|r| r.keys.clone()).collect()
    }

    /// As JSON, for the app's sheet and the agent: numbers and the server's
    /// words only.
    pub fn to_json(&self) -> Value {
        json!({
            "confirmation_id": self.id,
            "client": self.client.0,
            "client_name": self.client_label,
            "tool": self.tool,
            "device": self.device,
            "risks": self.risks.iter().map(Risk::to_json).collect::<Vec<_>>(),
            "asked_ms": self.asked_ms,
            "expires_ms": self.expires_ms,
            "state": self.state.as_str(),
        })
    }
}

/// What the app hears of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmEvent {
    /// A confirmation waits for the user: show it.
    Asked(u64),
    /// A confirmation ended: take its sheet down.
    Ended(u64, ConfirmState),
}

impl ConfirmEvent {
    /// As JSON.
    pub fn to_json(&self) -> Value {
        match self {
            Self::Asked(id) => json!({"event": "asked", "confirmation_id": id}),
            Self::Ended(id, s) => {
                json!({"event": "ended", "confirmation_id": id, "state": s.as_str()})
            }
        }
    }
}

/// Why an answer was not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerError {
    /// No confirmation has that id (or it has been forgotten).
    Unknown,
    /// It has already ended (answered, lapsed or the agent gone).
    Ended(ConfirmState),
}

/// The table.
#[derive(Debug, Default)]
pub struct Confirmations {
    list: VecDeque<Confirmation>,
    next: u64,
    events: Vec<ConfirmEvent>,
}

impl Confirmations {
    /// Open one, waiting from `now_ms`; its id.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn open(
        &mut self,
        client: ClientId,
        client_label: &str,
        tool: &'static str,
        device: Option<String>,
        risks: Vec<Risk>,
        args: Map<String, Value>,
        now_ms: u64,
    ) -> u64 {
        self.next = self.next.saturating_add(1);
        let id = self.next;
        self.list.push_back(Confirmation {
            id,
            client,
            client_label: client_label.to_owned(),
            tool,
            device,
            risks,
            asked_ms: now_ms,
            expires_ms: now_ms.saturating_add(CONFIRM_HOLD_MS),
            state: ConfirmState::Waiting,
            args,
            result: None,
            request: None,
        });
        self.events.push(ConfirmEvent::Asked(id));
        self.prune();
        id
    }

    /// Confirmation `id`.
    pub fn get(&self, id: u64) -> Option<&Confirmation> {
        self.list.iter().find(|c| c.id == id)
    }

    pub(crate) fn get_mut(&mut self, id: u64) -> Option<&mut Confirmation> {
        self.list.iter_mut().find(|c| c.id == id)
    }

    /// The ones waiting, oldest first.
    pub fn waiting(&self) -> Vec<&Confirmation> {
        self.list
            .iter()
            .filter(|c| c.state == ConfirmState::Waiting)
            .collect()
    }

    /// How many of `client`'s wait.
    pub(crate) fn waiting_of(&self, client: ClientId) -> usize {
        self.list
            .iter()
            .filter(|c| c.client == client && c.state == ConfirmState::Waiting)
            .count()
    }

    /// The waiting ones that lapse by `now_ms`.
    pub(crate) fn lapsed(&self, now_ms: u64) -> Vec<u64> {
        self.list
            .iter()
            .filter(|c| c.state == ConfirmState::Waiting && now_ms >= c.expires_ms)
            .map(|c| c.id)
            .collect()
    }

    /// The waiting ones of `client`.
    pub(crate) fn of_client(&self, client: ClientId) -> Vec<u64> {
        self.list
            .iter()
            .filter(|c| c.client == client && c.state == ConfirmState::Waiting)
            .map(|c| c.id)
            .collect()
    }

    /// End `id` as `state` with the agent's `result`.
    pub(crate) fn end(&mut self, id: u64, state: ConfirmState, result: Value) {
        if let Some(c) = self.get_mut(id) {
            c.state = state;
            c.result = Some(result);
            self.events.push(ConfirmEvent::Ended(id, state));
        }
    }

    /// Forget `id` (its held request has been answered).
    pub(crate) fn remove(&mut self, id: u64) -> Option<Confirmation> {
        let at = self.list.iter().position(|c| c.id == id)?;
        self.list.remove(at)
    }

    /// The events since the last take, oldest first.
    pub(crate) fn take_events(&mut self) -> Vec<ConfirmEvent> {
        std::mem::take(&mut self.events)
    }

    /// Drop the oldest ended ones past [`ENDED_KEPT`].
    fn prune(&mut self) {
        let ended = self
            .list
            .iter()
            .filter(|c| c.state != ConfirmState::Waiting)
            .count();
        let mut extra = ended.saturating_sub(ENDED_KEPT);
        self.list.retain(|c| {
            if extra > 0 && c.state != ConfirmState::Waiting && c.request.is_none() {
                extra = extra.saturating_sub(1);
                return false;
            }
            true
        });
    }
}
