//! WP FEATUI (AIVIS's web part): who works on the hub's devices beside the
//! people, so every console shows it. A console that runs an AI agent says so
//! with `presence.set` while the agent works; every session hears
//! `presence.changed` with the whole list; `presence.list` reads it.
//!
//! - `presence.set {kind: "agent", label, serial?, what?, ttl_s}` (Control or
//!   Admin): this session's entry, kept `ttl_s` seconds (5 to 60) unless set
//!   again; `ttl_s: 0` clears it. `label` (the agent's name) and `what` are
//!   text from elsewhere: control and format characters are dropped and they
//!   are bounded. `by` is the hub's own label of the session, never the
//!   sender's word.
//! - `presence.list` (every role): `{presence: [{kind, label, by, serial, what,
//!   ttl_s}]}`, by session.
//! - An entry ends at its time, or when its session closes (the next turn).
//!
//! In memory only; the cap is [`CAP`]. Answers carry `"v": 1`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::clock::HubClock;
use crate::hub::HubHandle;
use crate::service::{HubService, ServiceConfig, ServiceSetup};
use crate::session::{Role, Session, SessionId};

/// The `hello` cap.
pub const CAP: &str = "presence";
/// The messages' version.
pub const VERSION: u32 = 1;
/// The longest an entry is kept without being set again, seconds.
pub const TTL_MAX_S: u64 = 60;
/// The shortest, seconds.
pub const TTL_MIN_S: u64 = 5;
/// The longest label (the agent's name) kept, in characters.
pub const LABEL_MAX: usize = 64;
/// The longest description of what it does, in characters.
pub const WHAT_MAX: usize = 160;
/// The most entries the hub keeps (one per session).
pub const ENTRIES_MAX: usize = 64;

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    kind: String,
    label: String,
    by: String,
    serial: String,
    what: String,
    until: Instant,
}

/// The service: always on (it keeps nothing on disk and costs nothing idle).
pub(crate) fn service(
    _config: &ServiceConfig,
    setup: &ServiceSetup<'_>,
) -> Option<Arc<dyn HubService>> {
    Some(Arc::new(Presence {
        clock: Arc::clone(setup.clock),
        entries: Mutex::new(BTreeMap::new()),
    }) as Arc<dyn HubService>)
}

struct Presence {
    clock: Arc<dyn HubClock>,
    entries: Mutex<BTreeMap<SessionId, Entry>>,
}

/// `text` without control and format characters, trimmed, at most `max` characters.
fn clean(text: &str, max: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control() && !is_format(*c))
        .collect::<String>()
        .trim()
        .chars()
        .take(max)
        .collect()
}

/// Unicode's format characters that reorder or hide text (bidi controls, zero-width ones).
fn is_format(c: char) -> bool {
    matches!(c, '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}' | '\u{E0000}'..='\u{E007F}')
}

impl Presence {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<SessionId, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn list(&self, now: Instant) -> Value {
        let rows: Vec<Value> = self
            .lock()
            .values()
            .map(|e| {
                json!({"kind": e.kind, "label": e.label, "by": e.by, "serial": e.serial, "what": e.what,
                       "ttl_s": e.until.saturating_duration_since(now).as_secs()})
            })
            .collect();
        Value::Array(rows)
    }

    fn changed(&self, hub: &HubHandle, now: Instant) {
        let event = json!({"t": "presence.changed", "v": VERSION, "hub": hub.hub_id(), "presence": self.list(now)});
        hub.inner.relay.broadcast_event(&event.to_string());
    }

    /// Entries whose time is up or whose session is gone; true when any was.
    fn sweep(&self, hub: &HubHandle, now: Instant) -> bool {
        let live = hub.session_ids();
        let mut entries = self.lock();
        let before = entries.len();
        entries.retain(|id, e| e.until > now && live.contains(id));
        entries.len() != before
    }

    fn set(
        &self,
        hub: &HubHandle,
        session: &Session,
        msg: &Value,
        now: Instant,
    ) -> Result<Value, (&'static str, String)> {
        let ttl = msg
            .get("ttl_s")
            .and_then(Value::as_u64)
            .ok_or(("bad_request", "ttl_s: seconds, 0 to clear".to_owned()))?;
        if ttl == 0 {
            self.lock().remove(&session.id());
            return Ok(json!({}));
        }
        if !(TTL_MIN_S..=TTL_MAX_S).contains(&ttl) {
            return Err((
                "bad_request",
                format!("ttl_s: {TTL_MIN_S} to {TTL_MAX_S} seconds, or 0 to clear"),
            ));
        }
        let kind = msg.get("kind").and_then(Value::as_str).unwrap_or("agent");
        if kind != "agent" {
            return Err(("bad_request", "kind: \"agent\"".to_owned()));
        }
        let label = clean(
            msg.get("label").and_then(Value::as_str).unwrap_or(""),
            LABEL_MAX,
        );
        if label.is_empty() {
            return Err(("bad_request", "label: the agent's name".to_owned()));
        }
        let entry = Entry {
            kind: kind.to_owned(),
            label,
            by: hub.session_label(session.id()).by().unwrap_or_default(),
            serial: clean(
                msg.get("serial").and_then(Value::as_str).unwrap_or(""),
                LABEL_MAX,
            ),
            what: clean(
                msg.get("what").and_then(Value::as_str).unwrap_or(""),
                WHAT_MAX,
            ),
            until: now.checked_add(Duration::from_secs(ttl)).unwrap_or(now),
        };
        let mut entries = self.lock();
        if !entries.contains_key(&session.id()) && entries.len() >= ENTRIES_MAX {
            return Err((
                "busy",
                format!("the hub keeps {ENTRIES_MAX} entries at most"),
            ));
        }
        entries.insert(session.id(), entry);
        Ok(json!({}))
    }
}

impl HubService for Presence {
    fn name(&self) -> &'static str {
        "presence"
    }

    fn caps(&self) -> Vec<&'static str> {
        vec![CAP]
    }

    fn handle_message(
        &self,
        hub: &HubHandle,
        session: &Arc<Session>,
        msg: &Value,
    ) -> Option<String> {
        let t = msg.get("t")?.as_str()?;
        if t != "presence.set" && t != "presence.list" {
            return None;
        }
        let id = msg.get("id");
        let now = self.clock.now();
        let swept = self.sweep(hub, now);
        let answer = if t == "presence.list" {
            ok(id, json!({"presence": self.list(now)}))
        } else if session.role() < Role::Control {
            err(id, "denied", "control or admin")
        } else {
            let before = self.list(now);
            match self.set(hub, session, msg, now) {
                Ok(body) => {
                    // What others see changed (not only the time left): said to every session
                    let strip = |v: Value| -> Value {
                        Value::Array(
                            v.as_array()
                                .into_iter()
                                .flatten()
                                .map(|r| {
                                    let mut r = r.clone();
                                    if let Some(o) = r.as_object_mut() {
                                        o.remove("ttl_s");
                                    }
                                    r
                                })
                                .collect(),
                        )
                    };
                    if swept || strip(before) != strip(self.list(now)) {
                        self.changed(hub, now);
                    }
                    hub.wake_services();
                    return Some(ok(id, body));
                }
                Err((code, why)) => err(id, code, &why),
            }
        };
        if swept {
            self.changed(hub, now);
        }
        Some(answer)
    }

    fn next_due(&self) -> Option<Instant> {
        self.lock().values().map(|e| e.until).min()
    }

    fn run_due(&self, hub: &HubHandle, now: Instant) {
        if self.sweep(hub, now) {
            self.changed(hub, now);
        }
    }
}

fn err(id: Option<&Value>, code: &str, msg: &str) -> String {
    json!({"t": "err", "id": id, "code": code, "msg": msg}).to_string()
}

fn ok(id: Option<&Value>, body: Value) -> String {
    let mut out = json!({"t": "ok", "id": id, "v": VERSION});
    if let (Some(map), Value::Object(body)) = (out.as_object_mut(), body) {
        map.extend(body);
    }
    out.to_string()
}

#[cfg(test)]
#[path = "presence/tests_wp_featui.rs"]
mod tests_wp_featui;
