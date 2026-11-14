//! Safe apply with rollback (WP P1SA, PRO1 W6's core; PLAN-CLIENTS 5.9).
//!
//! A guarded apply captures the device as a snapshot (P1SN), applies a change
//! and then waits for the user's **Keep** ([`DspiCore::apply_confirm`]). A
//! timeout, **Revert** ([`DspiCore::apply_revert`]) or a failed apply puts the
//! snapshot back: one recall, the settings outside the bulk image first, the
//! image last (so the undo history before it ends, `docs/integrator/undo.md`).
//!
//! What can be guarded:
//! - an import of a document, by block ([`DspiCore::apply_guarded_import`],
//!   P1DF's options);
//! - a snapshot recall ([`DspiCore::apply_guarded_snapshot`], and
//!   [`DspiCore::apply_guarded_snapshot_document`] for the web);
//! - any multi-step flow the client runs itself, between
//!   [`DspiCore::apply_guard_begin`] and the Keep (output enables, a
//!   control-surface edit, a hardware change).
//!
//! An A/B toggle is never guarded (owner, OD-P1g). Nothing here asks for a
//! confirmation token: a guarded apply has its own way back (P1TK); the import
//! mints and spends its own.
//!
//! **The timer.** On USB, MIDI and the demo device the core runs it on the
//! caller's poll ([`DspiCore::apply_poll`]): no thread, no sleep, the time from
//! the clock seam (the web rule). Over DSPi Link, when the hub offers
//! `apply_guard` (P1HA), the hub runs it too: `apply.begin` makes the hub
//! capture the image, with a deadline a little after the core's. A client that
//! keeps polling reverts by itself at its own deadline (every part of the
//! snapshot), then ends the hub's guard with `apply.cancel` (the hub writes
//! the same image again: the end everyone hears is "reverted"). A client that
//! vanishes is reverted by the hub, image only.
//!
//! **A lost connection** (the lost-Link rule: nothing is replayed, a revert is
//! never queued):
//! - over Link with the hub's guard, the hub puts the image back when the
//!   session ends. The status is [`GuardPhase::Lost`] until the core can ask
//!   the hub (`apply.status`) after the reconnect; then it says how it ended
//!   (`last`), and the event log has a record. Nothing is sent but the
//!   question.
//! - on USB (or a hub without the guard) nothing can be written: the status
//!   ends [`GuardPhase::Lost`] and says the device may still hold the change.
//!   **Revert** stays offered: once the same device (serial) is open again,
//!   [`DspiCore::apply_revert`] recalls the captured snapshot, on the user's
//!   word only.
//!
//! **Reverts are never refused by a lock or a limit** set since the capture
//! (PLAN-CLIENTS 5.9): the revert is a snapshot recall, which P1LM lets pass,
//! and the hub's own write passes every lock (P1HA).
//!
//! The countdown and the outcome are a state the client reads
//! ([`DspiCore::apply_status`], no I/O, for the countdown bar) and drives
//! ([`DspiCore::apply_poll`], off the UI thread: it may revert).

use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};
use std::time::Duration;

use dspi_session::events::EventKind;
use dspi_transport::TransportKind;
use dspi_transport::clock::Instant;
use serde_json::Value;

use crate::{DspiCore, DspiError, Result};

/// The timeout a guarded apply takes when the caller gives 0 (owner, OD-P1g).
pub const APPLY_GUARD_DEFAULT_TIMEOUT_S: u32 = 30;

/// The longest timeout granted, in seconds; a longer one is shortened to it.
/// The hub's deadline is [`HUB_MARGIN`] later, inside its 600 s.
pub const APPLY_GUARD_MAX_TIMEOUT_S: u32 = 590;

/// How much later than the core's own deadline the hub's falls: a client that
/// keeps polling reverts by itself, every part of the snapshot, before the hub
/// writes the image back.
// Without DSPi Link there is no hub to guard with.
#[cfg_attr(not(feature = "link"), allow(dead_code))]
const HUB_MARGIN: Duration = Duration::from_secs(5);

/// The name of the snapshot taken before an apply (its note is the label).
const BEFORE_NAME: &str = "Before a guarded apply";

/// Why the second begin is refused.
const BUSY: &str = "a guarded apply is already waiting for Keep or Revert";

/// Why a Keep or Revert has nothing to act on.
const NOTHING_WAITING: &str = "no guarded apply is waiting for Keep or Revert";

/// Where a guarded apply stands ([`GuardedApply::phase`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardPhase {
    /// None has been begun on this core.
    #[default]
    Idle,
    /// Applied; waiting for Keep or Revert. The countdown runs.
    Waiting,
    /// Kept: by Keep, or because another console took the device over.
    Kept,
    /// The settings from before are back.
    Reverted,
    /// Putting them back failed; it is not tried again.
    RevertFailed,
    /// The hub went to put them back and the device was not there: nothing
    /// was written.
    NotWritten,
    /// The connection was lost. With the hub's guard this lasts until the
    /// hub says how it ended; without it, the device may still hold the
    /// change, and Revert works once the same device is open again.
    Lost,
}

/// A guarded apply's state: [`DspiCore::apply_status`] and every guarded call.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardedApply {
    /// This core's number for it (1, 2, …); 0 when none was begun.
    pub id: u64,
    /// Where it stands.
    pub phase: GuardPhase,
    /// What was applied: the document's or snapshot's name, or the client's
    /// label for a flow.
    pub label: String,
    /// The hub guards it too (DSPi Link with `apply_guard`): a vanished
    /// client is reverted by the hub.
    pub hub: bool,
    /// The timeout, ms.
    pub timeout_ms: u32,
    /// What is left of it, ms; 0 once it has ended.
    pub remaining_ms: u32,
    /// How it ended: "" while waiting, `confirmed`, `timeout`, `reverted`
    /// (Revert), `failed` (the apply itself failed), `connection_lost`,
    /// `taken_over` (another console took the device's lock) or
    /// `hub_stopping`.
    pub reason: String,
    /// One plain sentence for the user, "" while waiting.
    pub message: String,
    /// What the apply changed, in the unsaved-changes list's words.
    pub changes: Vec<String>,
    /// What the apply or the revert did not do, and why.
    pub notes: Vec<String>,
}

/// The hub's side of a running guard.
// Without DSPi Link there is no hub to guard with.
#[cfg_attr(not(feature = "link"), allow(dead_code))]
#[derive(Debug, Clone, Copy)]
struct HubGuard {
    /// The hub's guard id (`apply.begin`'s `guard`).
    guard: u64,
    /// Our hub session when it was begun: another one means ours ended.
    session: u16,
}

/// A guard waiting for Keep or Revert.
#[derive(Debug, Clone)]
struct Running {
    id: u64,
    label: String,
    /// The snapshot document taken before.
    before: String,
    serial: String,
    /// The connection it was begun on.
    generation: u64,
    deadline: Instant,
    timeout: Duration,
    hub: Option<HubGuard>,
    /// The connection was lost; the hub is asked how it ended.
    lost: bool,
    changes: Vec<String>,
    notes: Vec<String>,
}

/// What a guard lost without the hub's guard captured: Revert recalls it
/// once the same device is open again.
#[derive(Debug)]
struct Orphan {
    serial: String,
    before: String,
}

#[derive(Debug, Default)]
struct State {
    next_id: u64,
    running: Option<Running>,
    /// The last one's end, or the default.
    last: GuardedApply,
    orphan: Option<Orphan>,
}

/// The core's guarded apply: one at a time.
#[derive(Debug, Default)]
pub(crate) struct GuardSlot {
    /// Held through a begin, Keep, Revert or poll (they do I/O). Taken before
    /// the core lock, never after it.
    op: Mutex<()>,
    /// Held briefly, never across I/O or another lock.
    state: Mutex<State>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every update under these locks is whole before anything that could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn invalid(msg: impl Into<String>) -> DspiError {
    DspiError::InvalidArgument { msg: msg.into() }
}

fn ms(d: Duration) -> u32 {
    u32::try_from(d.as_millis()).unwrap_or(u32::MAX)
}

/// The timeout for `timeout_s` (0: the default), at most the longest.
fn timeout_of(timeout_s: u32) -> Duration {
    let s = if timeout_s == 0 {
        APPLY_GUARD_DEFAULT_TIMEOUT_S
    } else {
        timeout_s.min(APPLY_GUARD_MAX_TIMEOUT_S)
    };
    Duration::from_secs(u64::from(s))
}

/// The clock seam's now (with a test's skew).
fn now() -> Instant {
    let n = Instant::now();
    #[cfg(test)]
    let n = n.checked_add(skew::get()).unwrap_or(n);
    n
}

/// A test moves the guard's clock on, on its own thread.
#[cfg(test)]
pub(crate) mod skew {
    use std::cell::Cell;
    use std::time::Duration;

    thread_local! {
        static BY: Cell<Duration> = const { Cell::new(Duration::ZERO) };
    }

    pub(crate) fn get() -> Duration {
        BY.with(Cell::get)
    }

    /// Move this thread's guard clock on by `d`.
    pub(crate) fn advance(d: Duration) {
        BY.with(|b| b.set(b.get().saturating_add(d)));
    }
}

/// `doc`'s `meta.name`, or `fallback`.
fn doc_name(doc: &Value, fallback: &str) -> String {
    doc.get("meta")
        .and_then(|m| m.get("name"))
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

/// The core error an import's refusal names (its `noticeKey`), with its words.
fn import_error(reply: &Value) -> DspiError {
    let msg = reply
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("the import failed")
        .to_owned();
    let key = reply
        .get("noticeKey")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match key {
        "disconnected" => DspiError::Disconnected,
        "read_only" => DspiError::ReadOnly,
        "locked" => DspiError::Locked,
        "denied" => DspiError::Denied,
        "confirmation_required" => DspiError::ConfirmationRequired {
            code: "refused".into(),
            msg,
        },
        k if k.starts_with("dlink") => DspiError::Link {
            code: k.strip_prefix("dlink.").unwrap_or("link").to_owned(),
            msg,
        },
        k if k.starts_with("config.") || k == "invalid" => DspiError::InvalidArgument { msg },
        _ => DspiError::Transfer { msg },
    }
}

/// An import's reply as the apply's notes (its report when not clean), or
/// its refusal.
fn import_result(reply: &str) -> Result<Vec<String>> {
    let v: Value = serde_json::from_str(reply).map_err(|e| DspiError::Transfer {
        msg: format!("the import's reply could not be read: {e}"),
    })?;
    if v.get("ok") != Some(&Value::Bool(true)) {
        return Err(import_error(&v));
    }
    let clean = v.get("clean") != Some(&Value::Bool(false));
    if clean {
        return Ok(Vec::new());
    }
    Ok(v.get("lines")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default())
}

/// A failed apply that sent nothing: a refusal before the first write (the
/// document, the selection, a snapshot of another device). A recall that
/// stopped part way says "partly recalled".
fn sent_nothing(e: &DspiError) -> bool {
    matches!(e, DspiError::InvalidArgument { msg } if !msg.contains("partly recalled"))
}

/// The hub's answer about our guard (`apply.status`).
// Without DSPi Link there is no hub to guard with.
#[cfg_attr(not(feature = "link"), allow(dead_code))]
#[derive(Debug)]
enum HubView {
    /// Still running, for this session.
    Ours,
    /// Still running for a session that has ended: the hub is putting it back.
    Ending,
    /// Ended: the outcome (`apply.ended`'s fields).
    Ended(Value),
    /// The hub no longer knows it (another guard ran since, or the hub
    /// restarted).
    Unknown,
    /// The hub could not be asked.
    Unreachable,
}

// ─── The API ────────────────────────────────────────────────────────────

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Import `text` (a `.dspipreset` document) with a way back: capture a
    /// snapshot, apply the blocks `options` and `eq_channels` choose (as
    /// [`DspiCore::config_import_selected`] takes them), then wait
    /// `timeout_s` seconds (0: 30, at most 590) for
    /// [`DspiCore::apply_confirm`]. A timeout, [`DspiCore::apply_revert`] or a
    /// failed import puts the snapshot back; over DSPi Link the hub guards it
    /// too. No confirmation token: a guarded apply has its own way back.
    ///
    /// Returns the state, [`GuardPhase::Waiting`] with the changes.
    ///
    /// Errors, with nothing applied: [`DspiError::InvalidArgument`] for a text
    /// that is not a document, an empty selection, or while another guarded
    /// apply waits; [`DspiError::Disconnected`], [`DspiError::ReadOnly`];
    /// [`DspiError::Link`] for the hub's refusal (`busy`, `locked`,
    /// `denied`, `no_device`). The import's own errors, after the snapshot
    /// was put back (see [`DspiCore::apply_status`]).
    pub fn apply_guarded_import(
        &self,
        text: String,
        options: u32,
        eq_channels: u32,
        timeout_s: u32,
    ) -> Result<GuardedApply> {
        let doc = crate::files::parse_config(&text).map_err(invalid)?;
        if crate::files::Selection::new(options, eq_channels).blocks() == 0 {
            return Err(invalid(
                "Nothing was selected to import; nothing was changed.",
            ));
        }
        let label = doc_name(&doc, "a document");
        self.guard_begin(label, timeout_s, move |core| {
            let token = core.confirm_token(crate::ConfirmOp::ConfigImport);
            import_result(&core.config_import_confirmed(
                text,
                options,
                eq_channels,
                Some(token),
            ))
        })
    }

    /// Recall snapshot `id` from the store with a way back, as
    /// [`DspiCore::apply_guarded_import`] does for a document. Errors as
    /// there, and [`DspiCore::snapshot_recall`]'s.
    pub fn apply_guarded_snapshot(&self, id: String, timeout_s: u32) -> Result<GuardedApply> {
        let text = self.snapshot_export(id)?;
        self.apply_guarded_snapshot_document(text, timeout_s)
    }

    /// Recall a snapshot document's text with a way back (the web's, which
    /// keeps snapshots as files the page holds). Only a snapshot: any other
    /// document is refused with [`DspiError::InvalidArgument`], nothing sent.
    pub fn apply_guarded_snapshot_document(
        &self,
        text: String,
        timeout_s: u32,
    ) -> Result<GuardedApply> {
        let doc = crate::files::parse_config(&text).map_err(invalid)?;
        let kind = doc
            .get("meta")
            .and_then(|m| m.get("kind"))
            .and_then(Value::as_str);
        if kind != Some("snapshot") || !doc.get("snapshot").is_some_and(Value::is_object) {
            return Err(invalid(crate::snapshot::NOT_A_SNAPSHOT));
        }
        let label = doc_name(&doc, "a snapshot");
        self.guard_begin(label, timeout_s, move |core| {
            core.snapshot_recall_document(text).map(|r| r.notes)
        })
    }

    /// Begin a guard around a flow the client runs itself (output enables, a
    /// control-surface edit, several hardware settings): capture a snapshot
    /// now, then the client writes as usual and calls
    /// [`DspiCore::apply_confirm`] or [`DspiCore::apply_revert`]; the timeout
    /// and a lost connection work as for an import. `label` names the flow
    /// in the status. Errors as [`DspiCore::apply_guarded_import`]'s before
    /// anything is applied.
    pub fn apply_guard_begin(&self, label: String, timeout_s: u32) -> Result<GuardedApply> {
        self.guard_begin(label, timeout_s, |_| Ok(Vec::new()))
    }

    /// Keep the change. Returns the state: [`GuardPhase::Kept`]; or, past
    /// the timeout, the revert's end; or [`GuardPhase::Lost`] when the Keep
    /// could not reach the hub (it is not sent again: the hub reverts).
    ///
    /// Errors: [`DspiError::InvalidArgument`] when nothing is waiting.
    pub fn apply_confirm(&self) -> Result<GuardedApply> {
        let _op = lock(&self.guarded.op);
        let r = self.running().ok_or_else(|| invalid(NOTHING_WAITING))?;
        if r.lost {
            return Ok(self.resolve_lost(r));
        }
        if now() >= r.deadline {
            return Ok(self.revert_now(r, "timeout"));
        }
        if self.connection_changed(&r) {
            return Ok(self.on_lost(r));
        }
        let Some(h) = r.hub else {
            return Ok(self.finish(r, GuardPhase::Kept, "confirmed", "Kept.", Vec::new()));
        };
        match self.hub_end(h, "apply.confirm") {
            Ok(_) => Ok(self.finish(r, GuardPhase::Kept, "confirmed", "Kept.", Vec::new())),
            // The hub ended it first (its timeout, another console's lock).
            Err(DspiError::Link { code, .. })
                if matches!(code.as_str(), "no_guard" | "conflict" | "denied") =>
            {
                Ok(self.after_hub_refusal(r))
            }
            Err(_) => Ok(self.on_lost(r)),
        }
    }

    /// Put the settings from before back now. Returns the state:
    /// [`GuardPhase::Reverted`], [`GuardPhase::RevertFailed`], or
    /// [`GuardPhase::Lost`] when the connection went. After a lost USB
    /// connection, it recalls the captured snapshot once the same device is
    /// open again (the user's word, never queued).
    ///
    /// Errors: [`DspiError::InvalidArgument`] when nothing is waiting and no
    /// lost apply can be reverted on the open device.
    pub fn apply_revert(&self) -> Result<GuardedApply> {
        let _op = lock(&self.guarded.op);
        if let Some(r) = self.running() {
            if r.lost {
                return Ok(self.resolve_lost(r));
            }
            if self.connection_changed(&r) {
                return Ok(self.on_lost(r));
            }
            return Ok(self.revert_now(r, "reverted"));
        }
        self.revert_orphan()
    }

    /// Drive the guard: revert at the timeout, notice a lost connection, ask
    /// the hub how a lost one ended once the connection is back. Call it
    /// from the client's poll, off the UI thread (it may write). Returns the
    /// state; at once, unchanged, while another guarded call runs.
    pub fn apply_poll(&self) -> GuardedApply {
        let _op = match self.guarded.op.try_lock() {
            Ok(g) => g,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => return self.apply_status(),
        };
        let Some(r) = self.running() else {
            return self.apply_status();
        };
        if r.lost {
            return self.resolve_lost(r);
        }
        if self.connection_changed(&r) {
            return self.on_lost(r);
        }
        if now() >= r.deadline {
            return self.revert_now(r, "timeout");
        }
        self.apply_status()
    }

    /// The guard's state, for the countdown bar: no I/O, never blocks on a
    /// guarded call.
    pub fn apply_status(&self) -> GuardedApply {
        let st = lock(&self.guarded.state);
        match &st.running {
            Some(r) => GuardedApply {
                id: r.id,
                phase: if r.lost {
                    GuardPhase::Lost
                } else {
                    GuardPhase::Waiting
                },
                label: r.label.clone(),
                hub: r.hub.is_some(),
                timeout_ms: ms(r.timeout),
                remaining_ms: if r.lost {
                    0
                } else {
                    ms(r.deadline.saturating_duration_since(now()))
                },
                reason: if r.lost {
                    "connection_lost".into()
                } else {
                    String::new()
                },
                message: if r.lost {
                    "The connection was lost. The hub puts the settings from before back; \
                     how it ended is shown once the connection is back."
                        .into()
                } else {
                    String::new()
                },
                changes: r.changes.clone(),
                notes: r.notes.clone(),
            },
            None => st.last.clone(),
        }
    }

    /// Whether an import or a recall is guarded by default here (owner,
    /// OD-P1g): over DSPi Link to a hub that offers the guard, yes; on USB
    /// it is the user's choice. An A/B toggle never is.
    pub fn apply_guard_default(&self) -> bool {
        #[cfg(feature = "link")]
        {
            let link = self
                .lock()
                .conn
                .as_ref()
                .is_some_and(|c| c.caps.kind == TransportKind::Link);
            link && self.link_guard_target().is_some_and(|t| t.has_guard)
        }
        #[cfg(not(feature = "link"))]
        false
    }
}

// ─── The flow ───────────────────────────────────────────────────────────

impl DspiCore {
    /// The running guard, if any (a copy: the state lock is never held
    /// across I/O).
    fn running(&self) -> Option<Running> {
        lock(&self.guarded.state).running.clone()
    }

    /// Capture, begin the hub's guard, apply; see the API.
    fn guard_begin(
        &self,
        label: String,
        timeout_s: u32,
        apply: impl FnOnce(&Self) -> Result<Vec<String>>,
    ) -> Result<GuardedApply> {
        let _op = lock(&self.guarded.op);
        // A guard still waiting, or lost and not yet settled, holds the slot.
        if let Some(r) = self.running() {
            let left = if r.lost {
                self.resolve_lost(r)
            } else {
                self.apply_status()
            };
            if matches!(left.phase, GuardPhase::Waiting | GuardPhase::Lost) {
                return Err(invalid(BUSY));
            }
        }
        let timeout = timeout_of(timeout_s);
        let (serial, generation, link) = {
            let mut inner = self.lock();
            inner.writable_conn()?;
            let c = inner.conn()?;
            (
                c.serial.clone(),
                c.generation,
                c.caps.kind == TransportKind::Link,
            )
        };
        let before = self.snapshot_capture_document(BEFORE_NAME.into(), label.clone())?;
        let hub = if link { self.hub_begin(timeout)? } else { None };
        let start = now();
        let previous = {
            let mut st = lock(&self.guarded.state);
            st.next_id = st.next_id.wrapping_add(1).max(1);
            st.orphan = None;
            st.running = Some(Running {
                id: st.next_id,
                label,
                before,
                serial,
                generation,
                deadline: start.checked_add(timeout).unwrap_or(start),
                timeout,
                hub,
                lost: false,
                changes: Vec::new(),
                notes: Vec::new(),
            });
            st.last.clone()
        };
        let (mirror, sources) = self.mirror();
        match apply(self) {
            Ok(notes) => {
                let (after, _) = self.mirror();
                let changes = crate::presets::changes(&mirror, &after, &sources);
                if let Some(r) = lock(&self.guarded.state).running.as_mut() {
                    r.changes = changes;
                    r.notes = notes;
                }
                Ok(self.apply_status())
            }
            Err(e) => {
                let Some(r) = self.running() else {
                    return Err(e);
                };
                if sent_nothing(&e) {
                    // Nothing to put back: end the hub's guard and forget it.
                    if let Some(h) = r.hub {
                        let _ = self.hub_end(h, "apply.confirm");
                    }
                    let mut st = lock(&self.guarded.state);
                    st.running = None;
                    st.last = previous;
                    return Err(e);
                }
                if e == DspiError::Disconnected
                    || self.connection_changed(&r)
                    || !self.is_connected()
                {
                    self.on_lost(r);
                } else {
                    self.revert_now(r, "failed");
                }
                Err(e)
            }
        }
    }

    /// The mirror and the input source names, for the changes' words.
    fn mirror(&self) -> (crate::records::DspState, Vec<String>) {
        let inner = self.lock();
        let names = inner
            .conn
            .as_ref()
            .map(|c| c.source_names.to_vec())
            .unwrap_or_default();
        (inner.state.clone(), names)
    }

    /// Whether the connection the guard was begun on is gone (closed, or
    /// another one, or another device), or, over Link, our hub session.
    fn connection_changed(&self, r: &Running) -> bool {
        let same = self
            .lock()
            .conn
            .as_ref()
            .is_some_and(|c| c.generation == r.generation && c.serial == r.serial);
        if !same {
            return true;
        }
        match r.hub {
            Some(h) => self.hub_session() != Some(h.session),
            None => false,
        }
    }

    /// End the running guard as `phase`, and keep that as the last state.
    fn finish(
        &self,
        r: Running,
        phase: GuardPhase,
        reason: &str,
        message: &str,
        notes: Vec<String>,
    ) -> GuardedApply {
        let mut all = r.notes;
        all.extend(notes);
        let end = GuardedApply {
            id: r.id,
            phase,
            label: r.label,
            hub: r.hub.is_some(),
            timeout_ms: ms(r.timeout),
            remaining_ms: 0,
            reason: reason.to_owned(),
            message: message.to_owned(),
            changes: r.changes,
            notes: all,
        };
        let mut st = lock(&self.guarded.state);
        if st.running.as_ref().is_some_and(|x| x.id == end.id) {
            st.running = None;
        }
        st.last = end.clone();
        end
    }

    /// The connection went. With the hub's guard: wait for the hub's word.
    /// Without: nothing can be written; keep the snapshot for Revert.
    fn on_lost(&self, r: Running) -> GuardedApply {
        if r.hub.is_some() {
            if let Some(x) = lock(&self.guarded.state).running.as_mut() {
                x.lost = true;
            }
            let mut lost = r;
            lost.lost = true;
            return self.resolve_lost(lost);
        }
        lock(&self.guarded.state).orphan = Some(Orphan {
            serial: r.serial.clone(),
            before: r.before.clone(),
        });
        self.finish(
            r,
            GuardPhase::Lost,
            "connection_lost",
            "The connection was lost before Keep or Revert. Nothing was written back, so the \
             device may still hold the change; Revert puts the settings from before back once \
             the same device is open again.",
            Vec::new(),
        )
    }

    /// Put the snapshot back now (the timeout, Revert, a failed apply).
    fn revert_now(&self, r: Running, reason: &str) -> GuardedApply {
        // Over Link: is the hub's guard still ours? If it ended (its timeout,
        // another console's lock), its outcome is what happened.
        if let Some(h) = r.hub {
            match self.hub_view(&r, h) {
                HubView::Ours => {}
                HubView::Ended(o) => return self.finish_from_hub(r, &o),
                HubView::Unreachable | HubView::Ending => return self.on_lost(r),
                HubView::Unknown => {
                    return self.finish(
                        r,
                        GuardPhase::Lost,
                        "connection_lost",
                        "The hub no longer knows how this apply ended; check the settings.",
                        Vec::new(),
                    );
                }
            }
        }
        let recall = self.snapshot_recall_document(r.before.clone());
        // The hub's guard ends with its own write of the same image: the end
        // everyone hears is "reverted".
        let hub_end = r.hub.map(|h| self.hub_end(h, "apply.cancel"));
        let message = match reason {
            "timeout" => format!(
                "No Keep within {} s: the settings from before are back.",
                r.timeout.as_secs()
            ),
            "failed" => "The apply failed part way: the settings from before are back.".into(),
            _ => "Reverted: the settings from before are back.".into(),
        };
        match recall {
            Ok(rec) => self.finish(r, GuardPhase::Reverted, reason, &message, rec.notes),
            Err(e) => {
                let hub_reverted = matches!(
                    &hub_end,
                    Some(Ok(v)) if v.get("result").and_then(Value::as_str) == Some("reverted")
                );
                if hub_reverted {
                    let note = format!(
                        "Only the settings in the device's image were put back (by the hub); \
                         the rest could not be ({e})"
                    );
                    return self.finish(r, GuardPhase::Reverted, reason, &message, vec![note]);
                }
                // An unplugged device keeps its connection until it is closed:
                // its transfers fail as `Disconnected`.
                if r.hub.is_none()
                    && (e == DspiError::Disconnected
                        || self.connection_changed(&r)
                        || !self.is_connected())
                {
                    return self.on_lost(r);
                }
                if r.hub.is_some() && hub_end.as_ref().is_some_and(|x| x.is_err()) {
                    // The hub could not be told: it reverts by itself.
                    return self.on_lost(r);
                }
                let message = format!("The settings from before could not be put back: {e}.");
                self.finish(r, GuardPhase::RevertFailed, reason, &message, Vec::new())
            }
        }
    }

    /// Revert a guard whose USB connection was lost, on the same device now
    /// open again: the user's word.
    fn revert_orphan(&self) -> Result<GuardedApply> {
        let open = self.lock().conn.as_ref().map(|c| c.serial.clone());
        let (before, last) = {
            let st = lock(&self.guarded.state);
            match (&st.orphan, open) {
                (Some(o), Some(serial)) if o.serial == serial => (o.before.clone(), st.last.clone()),
                _ => return Err(invalid(NOTHING_WAITING)),
            }
        };
        let end = match self.snapshot_recall_document(before) {
            Ok(rec) => GuardedApply {
                phase: GuardPhase::Reverted,
                reason: "reverted".into(),
                message: "Reverted: the settings from before are back.".into(),
                notes: rec.notes,
                ..last
            },
            Err(e) => GuardedApply {
                phase: GuardPhase::RevertFailed,
                reason: "reverted".into(),
                message: format!("The settings from before could not be put back: {e}."),
                ..last
            },
        };
        let mut st = lock(&self.guarded.state);
        if end.phase == GuardPhase::Reverted {
            st.orphan = None;
        }
        st.last = end.clone();
        Ok(end)
    }

    /// A lost guard over Link: ask the hub how it ended, once it can be asked.
    fn resolve_lost(&self, r: Running) -> GuardedApply {
        let Some(h) = r.hub else {
            return self.on_lost(Running { lost: false, ..r });
        };
        match self.hub_view(&r, h) {
            HubView::Ended(o) => self.finish_from_hub(r, &o),
            HubView::Ours => {
                // Our session survived (the device came back on the same
                // session): the guard runs on, on the connection now open.
                let generation = self.lock().conn.as_ref().map(|c| c.generation);
                if let Some(x) = lock(&self.guarded.state).running.as_mut()
                    && x.id == r.id
                    && let Some(g) = generation
                {
                    x.lost = false;
                    x.generation = g;
                }
                self.apply_status()
            }
            HubView::Ending | HubView::Unreachable => self.apply_status(),
            HubView::Unknown => self.finish(
                r,
                GuardPhase::Lost,
                "connection_lost",
                "The connection was lost, and the hub no longer knows how this apply ended; \
                 check the settings.",
                Vec::new(),
            ),
        }
    }

    /// The hub ended our guard first (a refused Keep): how?
    fn after_hub_refusal(&self, r: Running) -> GuardedApply {
        let Some(h) = r.hub else {
            return self.apply_status();
        };
        match self.hub_view(&r, h) {
            HubView::Ended(o) => self.finish_from_hub(r, &o),
            HubView::Ours => self.revert_now(r, "timeout"),
            _ => self.on_lost(r),
        }
    }

    /// End as the hub's outcome says (`apply.ended`'s fields).
    fn finish_from_hub(&self, r: Running, o: &Value) -> GuardedApply {
        let word = |k: &str| o.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
        let (hub_reason, result) = (word("reason"), word("result"));
        let reason = match hub_reason.as_str() {
            "session_closed" => "connection_lost",
            "cancelled" => "reverted",
            "confirmed" => "confirmed",
            "timeout" => "timeout",
            "taken_over" => "taken_over",
            "hub_stopping" => "hub_stopping",
            _ => "connection_lost",
        };
        let why = match reason {
            "connection_lost" => "The connection was lost",
            "timeout" => "No Keep in time",
            "hub_stopping" => "The hub stopped",
            "taken_over" => "Another console took the device over",
            _ => "Ended",
        };
        let (phase, message) = match result.as_str() {
            "kept" if reason == "taken_over" => (
                GuardPhase::Kept,
                format!("{why}: the change stays, and nothing was written back."),
            ),
            "kept" => (GuardPhase::Kept, "Kept.".to_owned()),
            "reverted" => (
                GuardPhase::Reverted,
                format!("{why}: the hub put the settings from before back."),
            ),
            "not_written" => (
                GuardPhase::NotWritten,
                format!(
                    "{why}, and the device was not there when the hub went to put the settings \
                     back: nothing was written."
                ),
            ),
            _ => {
                let status = word("status");
                (
                    GuardPhase::RevertFailed,
                    format!("{why}: the hub could not put the settings from before back ({status})."),
                )
            }
        };
        let mut notes = Vec::new();
        if phase == GuardPhase::Reverted {
            notes.push(
                "The hub puts back the device's image only: device and source names, output \
                 latency and control surfaces stay as they were."
                    .to_owned(),
            );
            self.events.applied(
                EventKind::Recall,
                None,
                &format!("The hub put back the settings from before {}", r.label),
                Vec::new(),
            );
        }
        self.finish(r, phase, reason, &message, notes)
    }
}

// ─── The hub's side ─────────────────────────────────────────────────────

#[cfg(feature = "link")]
impl DspiCore {
    /// This session's id on the hub, while it is live.
    fn hub_session(&self) -> Option<u16> {
        self.link_guard_target().map(|t| t.session)
    }

    /// `apply.begin` on the bound device, when the hub offers the guard.
    fn hub_begin(&self, timeout: Duration) -> Result<Option<HubGuard>> {
        let Some(t) = self.link_guard_target().filter(|t| t.has_guard) else {
            return Ok(None);
        };
        let mut f = serde_json::Map::new();
        f.insert("handle".into(), t.handle.into());
        let hub_ms = u64::try_from(timeout.saturating_add(HUB_MARGIN).as_millis()).unwrap_or(u64::MAX);
        f.insert("timeout_ms".into(), hub_ms.into());
        let reply = self.link_hub_request("apply.begin", f)?;
        let guard = reply
            .get("guard")
            .and_then(Value::as_u64)
            .ok_or_else(|| DspiError::Link {
                code: "protocol".into(),
                msg: "the hub's apply.begin answer has no guard".into(),
            })?;
        Ok(Some(HubGuard {
            guard,
            session: t.session,
        }))
    }

    /// `apply.confirm` or `apply.cancel` for our guard; the hub's answer.
    fn hub_end(&self, _h: HubGuard, t: &str) -> Result<Value> {
        let target = self.link_guard_target().ok_or(DspiError::Disconnected)?;
        let mut f = serde_json::Map::new();
        f.insert("handle".into(), target.handle.into());
        self.link_hub_request(t, f)
    }

    /// What the hub says of our guard.
    fn hub_view(&self, r: &Running, h: HubGuard) -> HubView {
        let Some(target) = self.link_guard_target() else {
            return HubView::Unreachable;
        };
        // Only the same device: the hub keeps outcomes per serial.
        if self.lock().conn.as_ref().map(|c| c.serial.as_str()) != Some(r.serial.as_str()) {
            return HubView::Unreachable;
        }
        let mut f = serde_json::Map::new();
        f.insert("handle".into(), target.handle.into());
        let Ok(reply) = self.link_hub_request("apply.status", f) else {
            return HubView::Unreachable;
        };
        let ours = |v: &Value| {
            v.get("guard").and_then(Value::as_u64) == Some(h.guard)
                && v.get("serial").and_then(Value::as_str) == Some(r.serial.as_str())
        };
        if let Some(a) = reply.get("active").filter(|a| ours(a)) {
            let session = a.get("session").and_then(Value::as_u64);
            let reverting = a.get("reverting") == Some(&Value::Bool(true));
            return if !reverting && session == Some(u64::from(target.session)) {
                HubView::Ours
            } else {
                HubView::Ending
            };
        }
        match reply.get("last").filter(|l| ours(l)) {
            Some(l) => HubView::Ended(l.clone()),
            None => HubView::Unknown,
        }
    }
}

#[cfg(not(feature = "link"))]
impl DspiCore {
    fn hub_session(&self) -> Option<u16> {
        None
    }
    fn hub_begin(&self, _timeout: Duration) -> Result<Option<HubGuard>> {
        Ok(None)
    }
    fn hub_end(&self, _h: HubGuard, _t: &str) -> Result<Value> {
        Err(DspiError::Unsupported)
    }
    fn hub_view(&self, _r: &Running, _h: HubGuard) -> HubView {
        HubView::Unreachable
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p1sa;
