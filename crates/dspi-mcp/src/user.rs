//! What the user did meanwhile (WP AIVIS): an agent that works beside a
//! person has to know when the person steps in. The server gathers it from
//! the open cores (`dspi_core::actor::user`), the set and the app, and gives
//! it to each agent:
//!
//! - with **every tool result**, what is new since that agent's last one: one
//!   more text ("Since your last call, the user ...") and, in a structured
//!   result, `user_activity`;
//! - on request, with `dspi_get_user_activity`, from any cursor.
//!
//! In the server's words: numbers, switches, filter types and the device's
//! id. Never a name or any other text the user or a device wrote.

use std::collections::{BTreeMap, VecDeque};

use dspi_core::actor::user::{self, ChangedBy, UserChange};
use dspi_core::{CoreSet, DspiCore, ParamValue};
use dspi_proto::packets::MatrixRoutePacket;
use dspi_proto::registry::{Kind, by_path};
use dspi_proto::value::Unit;
use serde_json::{Map, Value, json};

use crate::ClientId;
use crate::registry;
use crate::untrusted;

/// The most events kept.
pub const EVENTS_CAP: usize = 400;
/// The most events one result carries.
pub const BLOCK_MAX: usize = 8;

/// One thing the user (or another writer) did.
#[derive(Debug, Clone, PartialEq)]
pub struct UserEvent {
    /// Its number, the cursor; rising. A control moved again gets a new one.
    pub id: u64,
    /// When the server saw it, on its clock.
    pub at_ms: u64,
    /// The device it is about (a `DeviceId`).
    pub device: Option<String>,
    /// What kind: `change`, `undo`, `replaced` (everything at once),
    /// `elsewhere`, `speakers`, `measure_stop`, `device_opened`,
    /// `device_closed`, `device_shown`, `paused`, `resumed`, `answer`,
    /// `stopped` (-- WP ODEC --: the user pressed Stop before this run).
    pub kind: &'static str,
    /// In words, as a verb phrase ("changed Out 3 output gain -3 dB to -9 dB").
    pub what: String,
    /// What was changed: `path`, `output`, `input`, `band`, `crossover_slot`
    /// (numbers from 1, as the app shows them).
    pub target: Map<String, Value>,
    /// Told to this client only (the answer to its own question).
    only: Option<ClientId>,
}

impl UserEvent {
    /// As JSON.
    pub fn to_json(&self) -> Value {
        let mut o = self.target.clone();
        o.insert("id".into(), json!(self.id));
        o.insert("at_ms".into(), json!(self.at_ms));
        o.insert("device".into(), json!(self.device));
        o.insert("kind".into(), json!(self.kind));
        o.insert("what".into(), json!(self.what));
        Value::Object(o)
    }

    fn for_client(&self, client: ClientId) -> bool {
        self.only.is_none_or(|c| c == client)
    }
}

/// What the user did, newest last.
#[derive(Debug, Default)]
pub struct UserActivity {
    events: VecDeque<UserEvent>,
    last: u64,
    /// The core log's cursor of each open device.
    cores: BTreeMap<String, u64>,
    /// The devices open at the first look are no news.
    looked: bool,
    shown: Option<String>,
}

impl UserActivity {
    /// The newest event's number, 0 for none.
    pub fn cursor(&self) -> u64 {
        self.last
    }

    /// The events after `since` that `client` is told, oldest first.
    pub fn since(&self, since: u64, client: ClientId) -> Vec<&UserEvent> {
        self.events
            .iter()
            .filter(|e| e.id > since && e.for_client(client))
            .collect()
    }

    /// Whether events after `since` were dropped from the ring.
    pub fn truncated(&self, since: u64) -> bool {
        self.events.len() >= EVENTS_CAP
            && self
                .events
                .front()
                .is_some_and(|e| e.id > since.saturating_add(1))
    }

    /// Note an event of the app's.
    pub fn note(
        &mut self,
        now_ms: u64,
        kind: &'static str,
        what: &str,
        device: Option<&str>,
        only: Option<ClientId>,
    ) {
        self.push(UserEvent {
            id: 0,
            at_ms: now_ms,
            device: device.map(str::to_owned),
            kind,
            what: what.to_owned(),
            target: Map::new(),
            only,
        });
    }

    fn push(&mut self, mut e: UserEvent) {
        // A control moved again is one event, its newest
        if !e.target.is_empty() {
            self.events
                .retain(|o| o.kind != e.kind || o.device != e.device || o.target != e.target);
        }
        self.last = self.last.saturating_add(1);
        e.id = self.last;
        if self.events.len() >= EVENTS_CAP {
            self.events.pop_front();
        }
        self.events.push_back(e);
    }

    /// The app shows `id` now.
    pub fn shown(&mut self, id: Option<&str>, now_ms: u64) {
        if self.shown.as_deref() == id {
            return;
        }
        let first = self.shown.is_none();
        self.shown = id.map(str::to_owned);
        if !first && let Some(id) = id {
            self.note(
                now_ms,
                "device_shown",
                "switched the app to this device",
                Some(id),
                None,
            );
        }
    }

    /// Read what is new on the open devices.
    pub fn collect(&mut self, set: &CoreSet, now_ms: u64) {
        let open = registry::devices(set);
        let gone: Vec<String> = self
            .cores
            .keys()
            .filter(|k| !open.iter().any(|d| d.id.as_str() == k.as_str()))
            .cloned()
            .collect();
        for id in gone {
            self.cores.remove(&id);
            self.note(
                now_ms,
                "device_closed",
                "closed this device (or it went)",
                Some(&id),
                None,
            );
        }
        for d in &open {
            let id = d.id.as_str();
            let Some(core) = set.core(&d.id) else {
                continue;
            };
            let Some(cursor) = self.cores.get(id).copied() else {
                self.cores.insert(id.to_owned(), user::latest());
                if self.looked {
                    self.note(
                        now_ms,
                        "device_opened",
                        "opened this device",
                        Some(id),
                        None,
                    );
                }
                continue;
            };
            let new = user::changes_since(&core, cursor);
            if let Some(newest) = new.last() {
                self.cores.insert(id.to_owned(), newest.seq);
            }
            for c in &new {
                let (kind, what, target) = describe(&core, c);
                self.push(UserEvent {
                    id: 0,
                    at_ms: now_ms,
                    device: Some(id.to_owned()),
                    kind,
                    what,
                    target,
                    only: None,
                });
            }
        }
        self.looked = true;
    }

    /// `client`'s block for a result: what is new after `since`, `None` for
    /// nothing. Text and structure say the same.
    pub fn block(&self, since: u64, client: ClientId) -> Option<(String, Value)> {
        let new = self.since(since, client);
        if new.is_empty() {
            return None;
        }
        let more = new.len().saturating_sub(BLOCK_MAX);
        let newest: Vec<&UserEvent> = new.into_iter().skip(more).collect();
        let say = |elsewhere: bool| -> String {
            let parts: Vec<String> = newest
                .iter()
                .filter(|e| (e.kind == "elsewhere") == elsewhere)
                .map(|e| match &e.device {
                    Some(d) => format!("{} ({})", e.what, untrusted::clean(d, 64)),
                    None => e.what.clone(),
                })
                .collect();
            parts.join("; ")
        };
        let (mine, other) = (say(false), say(true));
        let mut text = String::new();
        if !mine.is_empty() {
            text.push_str(&format!("Since your last call, the user {mine}."));
        }
        if !other.is_empty() {
            let gap = if text.is_empty() { "" } else { " " };
            text.push_str(&format!("{gap}Elsewhere since your last call: {other}."));
        }
        if more > 0 {
            text.push_str(&format!(
                " {more} more before that: dspi_get_user_activity lists them."
            ));
        }
        text.push_str(" Read what you work on again before you change it.");
        let data = json!({
            "note": text,
            "events": newest.iter().map(|e| e.to_json()).collect::<Vec<_>>(),
            "more": more,
            "cursor": self.last,
        });
        Some((text, data))
    }
}

/// Whether a change moves what a measurement measures (a name, a preset
/// saved or a clip mark cleared do not).
pub fn audible(c: &UserChange) -> bool {
    !(c.path.starts_with("did.")
        || c.path.ends_with("name")
        || c.path.contains(".name.")
        || matches!(
            c.path.as_str(),
            "preset.save" | "preset.delete" | "preset.startup" | "preset.dir" | "vol.master.save"
        ))
}

/// A change in words: its kind, a verb phrase and its target.
pub fn describe(core: &DspiCore, c: &UserChange) -> (&'static str, String, Map<String, Value>) {
    let mut target = Map::new();
    if let Some(act) = c.path.strip_prefix("did.") {
        let (kind, what) = match act {
            "speakers_confirmed" => ("speakers", "confirmed the speaker setup"),
            "speakers" => (
                "speakers",
                "changed the speaker setup without confirming it",
            ),
            "measure_stop" => ("measure_stop", "stopped the measurement"),
            _ => ("change", "did something in the app"),
        };
        return (kind, what.to_owned(), target);
    }
    if c.by == ChangedBy::Elsewhere {
        let who = match &c.after {
            ParamValue::Text(t) if !t.is_empty() => untrusted::clean(t, 64),
            _ => "another writer".to_owned(),
        };
        target.insert("path".into(), json!(c.path));
        let what = format!("{who} changed {}", c.path.replace('_', " "));
        return ("elsewhere", what, target);
    }
    let verb = if c.undo { "undid" } else { "changed" };
    let kind = if c.undo { "undo" } else { "change" };
    target.insert("path".into(), json!(c.path));
    match c.path.as_str() {
        "bulk" | "bulk.chunk" => {
            return (
                "replaced",
                "replaced the settings as a whole (a recall, an import or a revert)".into(),
                target,
            );
        }
        "dev.reset" => return ("replaced", "reset the device".into(), target),
        "preset.load" => {
            return ("replaced", format!("loaded a preset{}", slot(c)), target);
        }
        "raw" => return (kind, "sent the device a raw command".into(), target),
        _ => {}
    }
    let n_in = core.get_state().num_input_channels;
    let at = place(&c.path, &c.indices, n_in, &mut target);
    let desc = by_path(&c.path);
    let label = match (c.path.as_str(), desc) {
        ("eq.band", _) => "filter".to_owned(),
        (_, Some(d)) => d.label.to_lowercase(),
        (p, None) => p.replace(['.', '_'], " "),
    };
    let mut what = verb.to_owned();
    if !at.is_empty() {
        what.push(' ');
        what.push_str(&at);
    }
    what.push(' ');
    what.push_str(&label);
    let kind_of = desc.map(|d| d.kind);
    let words = |v: &ParamValue| value(v, kind_of);
    match (c.before.as_ref().and_then(words), words(&c.after)) {
        (Some(b), Some(a)) if b != a => what.push_str(&format!(" from {b} to {a}")),
        (_, Some(a)) => what.push_str(&format!(" to {a}")),
        _ => {}
    }
    (kind, what, target)
}

/// " 3" for a preset entry's slot, "" without one.
fn slot(c: &UserChange) -> String {
    let n = c.indices.first().map(|n| u64::from(*n)).or(match c.after {
        ParamValue::Choice(n) => Some(u64::from(n)),
        ParamValue::Int(n) => u64::try_from(n).ok(),
        _ => None,
    });
    n.map(|n| format!(" ({})", n.saturating_add(1)))
        .unwrap_or_default()
}

/// Where a path's indices are, in words, and into `target`: `output`,
/// `input`, `band` (a PEQ band) or `crossover_slot`, each counted from 1 as
/// the app shows them (-- WP SHOWFIX --); a path of another shape gives its
/// `indices`, each from 1 too.
pub(crate) fn place(path: &str, ix: &[u8], n_in: u8, target: &mut Map<String, Value>) -> String {
    let nth = |n: u8| u16::from(n).saturating_add(1);
    let mut channel = |ch: u8| -> String {
        match ch.checked_sub(n_in) {
            Some(o) => {
                target.insert("output".into(), json!(nth(o)));
                format!("Out {}", nth(o))
            }
            None => {
                target.insert("input".into(), json!(nth(ch)));
                format!("In {}", nth(ch))
            }
        }
    };
    match (path, ix) {
        (p, [ch, band]) if p.starts_with("eq.") => {
            let at = channel(*ch);
            match band.checked_sub(20) {
                Some(x) => {
                    target.insert("crossover_slot".into(), json!(nth(x)));
                    format!("{at} crossover slot {}", nth(x))
                }
                None => {
                    target.insert("band".into(), json!(nth(*band)));
                    format!("{at} band {}", nth(*band))
                }
            }
        }
        ("ch.delay", [ch]) => channel(*ch),
        ("mix", [i, o]) => {
            target.insert("input".into(), json!(nth(*i)));
            target.insert("output".into(), json!(nth(*o)));
            format!("In {} to Out {}", nth(*i), nth(*o))
        }
        ("pre", [i]) => {
            target.insert("input".into(), json!(nth(*i)));
            format!("In {}", nth(*i))
        }
        (p, [0xFF]) if p.starts_with("limit.") => {
            target.insert("output".into(), json!("all"));
            "every output".to_owned()
        }
        (p, [o]) if p.starts_with("out.") || p.starts_with("limit.") => {
            target.insert("output".into(), json!(nth(*o)));
            format!("Out {}", nth(*o))
        }
        (_, []) => String::new(),
        (_, more) => {
            let more: Vec<u16> = more.iter().map(|n| nth(*n)).collect();
            target.insert("indices".into(), json!(more));
            String::new()
        }
    }
}

/// A value in words: a number, a switch, a choice, a filter or a route; never
/// text.
fn value(v: &ParamValue, kind: Option<Kind>) -> Option<String> {
    let unit = |k: Option<Kind>| match k.map(Kind::unit) {
        Some(Unit::Db) => " dB",
        Some(Unit::Hz) => " Hz",
        Some(Unit::Ms) => " ms",
        Some(Unit::Percent) => " %",
        _ => "",
    };
    let number = |x: f64, u: &str| {
        let t = format!("{x:.2}");
        let t = t.trim_end_matches('0').trim_end_matches('.');
        let t = if t == "-0" { "0" } else { t };
        format!("{t}{u}")
    };
    match v {
        ParamValue::Bool(b) => Some(if *b { "on" } else { "off" }.to_owned()),
        ParamValue::Float(f) if f.is_finite() => Some(number(f64::from(*f), unit(kind))),
        ParamValue::Int(i) => Some(format!("{i}{}", unit(kind))),
        ParamValue::Choice(n) => match kind {
            Some(Kind::Choice(list)) => list
                .iter()
                .find(|(raw, _)| raw == n)
                .map(|(_, l)| (*l).to_owned()),
            _ => Some(n.to_string()),
        },
        ParamValue::Bytes(b) => {
            if let Some(p) = crate::policy::band_of_bytes(b) {
                let hz = number(f64::from(p.freq), " Hz");
                return Some(format!("{hz} {}", number(f64::from(p.gain), " dB")));
            }
            MatrixRoutePacket::decode(b).ok().map(|p| match p.enabled {
                true => number(f64::from(p.gain_db), " dB"),
                false => "off".to_owned(),
            })
        }
        _ => None,
    }
}
