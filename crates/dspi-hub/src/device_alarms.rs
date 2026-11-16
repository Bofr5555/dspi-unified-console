//! Device alarms (WP P2AL, PLAN-P2 M3, `docs/integrator/hub-api.md` section 7): the
//! hub raises alarms about its devices beside the audio link's, on an
//! [`AlarmBook`] of their own (the alink's book is used, never edited).
//!
//! | Code | Raised | Cleared | Default rule |
//! |---|---|---|---|
//! | `device_offline` | a device seen this run detached for `after_s` | attached again | on, 30 s |
//! | `unresponsive` | attached, silent for `after_s` (P2HP's `Unresponsive`) | its next reading (`Health`), or detached | on, 10 s |
//! | `device_restarted` | `Facts` with a watchdog, glitch or brown-out reset that happened in this run or in the last 10 minutes | acknowledged | on |
//! | `clip` | extension firmware: `per_minute` clip events (0xE0) within 60 s; other firmware: the sticky latch set ("clipped since cleared"), once | events: a minute under the threshold; the latch: someone cleared it (the hub never does) | on, 10 a minute |
//! | `temperature` | at or above `above_c` | 5 °C under it | on, 70 °C |
//! | `cpu` | at or above `above_pct` for `for_s` | a reading under it | on, 90 % for 10 s |
//! | `schedule_failed` | a schedule run with a failed action (`ScheduleRan`) | acknowledged | on |
//! | `clock_unset` | the hub's wall clock reads before [`BUILD_FLOOR_S`] | the clock is set | on |
//! | `rollout_failed` (WP P3RO) | a device's firmware rollout failed (`RolloutEnded`), critical | a later rollout of the device ends ok, or (unless it failed its verify) the device attaches with the image's firmware | always on: listed in the rules (P3CC), its `on` cannot be turned off |
//!
//! **Mute on clip storm** (OD-P2n, off by default): while a `clip` storm (events) is on
//! and the rule's `mute_on_storm` is set, the hub mutes the device once per storm, as one
//! of its own writes (PLAN-P2 M3, OD-P2o): `REQ_SET_USER_MUTE` 1 as
//! [`INTERNAL_SESSION_ID`], let past the device's lock once (`Router::pass_hold`), never
//! an unmute. The write is recorded in the history (a bus `Event`, `by` the rule's name,
//! [`MUTE_BY`]) and the alarm's message says so.
//!
//! Every change of an alarm (raised, raised again, acknowledged, cleared) goes onto the
//! bus as [`BusMessage::Alarm`] (so the history records it and P2WH and P2MQ deliver it)
//! and to every session as `alarm.changed {v, hub, alarm}`. The audio link's alarms are
//! read from the hub's system book: `alarm.list` merges them (`source: "alink"`),
//! `alarm.ack` acknowledges either, and their changes are announced the same way, read
//! once a second. `system.alarms` and `system.alarm.ack` keep working.
//!
//! Rules live in `<data>/alarms.json` (at most [`MAX_FILE_BYTES`]); a file that does not
//! read whole is kept aside as `alarms.json.bad-<time>` and the rules it held that read
//! are kept. Messages: `alarm.list` (viewer), `alarm.ack` (control), `alarm.rules.get`
//! (viewer), `alarm.rules.set` (admin).
//!
//! Pure state, thin glue: [`DeviceAlarms`] is the state machine, given the time and the
//! bus messages; [`AlarmService`] runs it on the hub's timer.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use dspi_link::frames::CmdRequest;
use dspi_link::{Dir, status};
use dspi_proto::ext::{NOTIFY_EVT_EXT_CLIP, ResetReason};
use dspi_proto::generated::opcodes as op;
use dspi_session::events::{EventKind, EventRecord, ExtEvent, decode_ext_event};
use dspi_session::notify::{self, Event};
use serde_json::{Map, Value, json};

use crate::bus::{Bus, BusMessage, Subscription};
use crate::clock::HubClock;
use crate::hub::HubHandle;
use crate::inventory::iso8601;
use crate::registry::DeviceState;
use crate::service::{DeviceEvent, HubService, ServiceConfig, ServiceSetup, Waker};
use crate::session::{Completion, INTERNAL_SESSION_ID, Role, Session, SessionCtx};
use crate::system::alarms::{Alarm, AlarmBook, AlarmState};

/// The family's cap in `hello.caps`.
pub const CAP: &str = "alarms";
/// The rules' file in the hub's data folder.
pub const FILE_NAME: &str = "alarms.json";
/// The rules' file is never longer (PLAN-P2 S3).
pub const MAX_FILE_BYTES: u64 = 64 * 1024;
/// The event every session gets.
pub const CHANGED_EVENT: &str = "alarm.changed";
/// The messages of the family.
pub const MESSAGES: [&str; 4] = [
    "alarm.list",
    "alarm.ack",
    "alarm.rules.get",
    "alarm.rules.set",
];
/// The codes of device alarms, in the order the rules list them.
pub const CODES: [&str; 9] = [
    "device_offline",
    "unresponsive",
    "device_restarted",
    "clip",
    "temperature",
    "cpu",
    "schedule_failed",
    "clock_unset",
    // -- WP P3CC -- now that the core lists it (its notice row `alarm.rollout_failed`)
    ROLLOUT_FAILED,
];
// -- WP P3RO --
/// The firmware rollout's alarm (`hub-api.md` section 21.3). In [`CODES`] since P3CC, so
/// `alarm.rules.get` lists it, but always on: its rule's `on` stays true whatever
/// `alarm.rules.set` says.
pub const ROLLOUT_FAILED: &str = "rollout_failed";
/// `by` of the hub's mute on a clip storm (its history record).
pub const MUTE_BY: &str = "Alarm rule: mute on clip storm";
/// `source` of that record.
pub const MUTE_SOURCE: &str = "mute_on_storm";
/// The earliest wall clock this build believes: 2026-11-15T00:00:00Z, the date of its
/// source. A clock before it is unset (`clock_unset`; schedules wait, OD-P2h).
pub const BUILD_FLOOR_S: u64 = 1_794_700_800;
/// The window of the clip rate, and how long under the threshold clears a storm.
pub const CLIP_WINDOW: Duration = Duration::from_secs(60);
/// How far under `above_c` a temperature alarm clears.
pub const TEMPERATURE_HYSTERESIS_C: f64 = 5.0;
/// A restart older than this, seen on the first attach of a hub run, is no alarm.
pub const RECENT_RESTART_S: u64 = 600;
/// The most devices tracked (the inventory's cap).
pub const MAX_DEVICES: usize = 1024;
/// The most alarms on at once; past it a new one is logged, not raised.
pub const MAX_ACTIVE: usize = 8192;
/// The most alarms one `alarm.list` answers (active first).
pub const MAX_LIST: usize = 500;
/// `"v"` of every answer and event.
const VERSION: u8 = 1;
/// The bus is read, and the clock and the audio link's book looked at, this often.
const TICK: Duration = Duration::from_secs(1);
/// What the subscription holds.
const BUS_CAPACITY: usize = 1024;
/// Two readings of one restart differ by at most this (the uptime's 1 s steps).
const RESTART_SLACK_S: u64 = 10;
/// Clip events remembered per device (the window's), far past any rule.
const MAX_CLIP_EVENTS: usize = 1024;
/// A serial longer than this is not one.
const MAX_SERIAL_BYTES: usize = 64;
/// A subject or text from the bus longer than this is cut.
const MAX_TEXT_CHARS: usize = 256;
/// Who acknowledged what, remembered for at most this many alarms.
const MAX_ACKED: usize = 2 * MAX_ACTIVE;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every section under these locks leaves its data whole before it could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn cut(text: &str) -> String {
    text.chars().take(MAX_TEXT_CHARS).collect()
}

// ── Rules ───────────────────────────────────────────────────────────────────

/// A rule field's type and range.
#[derive(Debug, Clone, Copy)]
enum Kind {
    Bool,
    Int(u64, u64),
    Num(f64, f64),
}

/// Each code's fields beside `on`, with their defaults.
fn spec(code: &str) -> &'static [(&'static str, Kind, f64)] {
    match code {
        "device_offline" => &[("after_s", Kind::Int(1, 86_400), 30.0)],
        "unresponsive" => &[("after_s", Kind::Int(1, 86_400), 10.0)],
        "clip" => &[
            ("per_minute", Kind::Int(1, 1000), 10.0),
            ("mute_on_storm", Kind::Bool, 0.0),
        ],
        "temperature" => &[("above_c", Kind::Num(0.0, 150.0), 70.0)],
        "cpu" => &[
            ("above_pct", Kind::Int(1, 100), 90.0),
            ("for_s", Kind::Int(1, 3600), 10.0),
        ],
        _ => &[],
    }
}

/// The alarm rules: every code with `on` and its fields, each valid.
#[derive(Debug, Clone, PartialEq)]
pub struct Rules(Map<String, Value>);

impl Default for Rules {
    /// PLAN-P2's defaults: every code on, mute on clip storm off.
    fn default() -> Self {
        let mut all = Map::new();
        for code in CODES {
            let mut rule = Map::new();
            rule.insert("on".into(), Value::Bool(true));
            for (name, kind, default) in spec(code) {
                let v = match kind {
                    Kind::Bool => Value::Bool(*default != 0.0),
                    Kind::Int(..) => json!(*default as u64),
                    Kind::Num(..) => json!(*default),
                };
                rule.insert((*name).into(), v);
            }
            all.insert(code.into(), Value::Object(rule));
        }
        Self(all)
    }
}

impl Rules {
    fn field(&self, code: &str, name: &str) -> Option<&Value> {
        self.0.get(code)?.get(name)
    }

    /// Whether the code's rule is on.
    pub fn on(&self, code: &str) -> bool {
        self.field(code, "on").and_then(Value::as_bool) == Some(true)
    }

    /// A whole-number field (seconds, a count, a percentage).
    pub fn int(&self, code: &str, name: &str) -> u64 {
        self.field(code, name).and_then(Value::as_u64).unwrap_or(0)
    }

    /// A number field (°C).
    pub fn num(&self, code: &str, name: &str) -> f64 {
        self.field(code, name)
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
    }

    /// A yes-or-no field.
    pub fn flag(&self, code: &str, name: &str) -> bool {
        self.field(code, name).and_then(Value::as_bool) == Some(true)
    }

    /// The rules as `alarm.rules.get` answers them and the file keeps them.
    pub fn to_json(&self) -> Value {
        Value::Object(self.0.clone())
    }

    /// One code's rule changed by `v` (an object of its fields; one left out keeps its
    /// value; a field this build does not know is ignored, as section 1.1 says).
    fn merge_code(&mut self, code: &str, v: &Value) -> Result<(), String> {
        let Some(given) = v.as_object() else {
            return Err(format!("rules.{code} is not an object"));
        };
        let mut rule = self
            .0
            .get(code)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(on) = given.get("on") {
            let on = on
                .as_bool()
                .ok_or_else(|| format!("rules.{code}.on is not true or false"))?;
            // -- WP P3CC -- `rollout_failed` cannot be turned off
            let on = on || code == ROLLOUT_FAILED;
            rule.insert("on".into(), Value::Bool(on));
        }
        for (name, kind, _) in spec(code) {
            let Some(x) = given.get(*name) else { continue };
            let ok = match kind {
                Kind::Bool => x.as_bool().map(Value::Bool),
                Kind::Int(lo, hi) => x
                    .as_u64()
                    .filter(|n| (*lo..=*hi).contains(n))
                    .map(|n| json!(n)),
                Kind::Num(lo, hi) => x
                    .as_f64()
                    .filter(|n| n.is_finite() && (*lo..=*hi).contains(n))
                    .map(|n| json!(n)),
            };
            let Some(x) = ok else {
                let range = match kind {
                    Kind::Bool => "true or false".to_owned(),
                    Kind::Int(lo, hi) => format!("a whole number {lo} to {hi}"),
                    Kind::Num(lo, hi) => format!("a number {lo} to {hi}"),
                };
                return Err(format!("rules.{code}.{name} must be {range}"));
            };
            rule.insert((*name).into(), x);
        }
        self.0.insert(code.into(), Value::Object(rule));
        Ok(())
    }

    /// `alarm.rules.set`'s `rules`, whole or in part: all or nothing. A code this build
    /// does not know is ignored.
    pub fn merged(&self, v: &Value) -> Result<Self, String> {
        let Some(given) = v.as_object() else {
            return Err("rules is not an object".into());
        };
        let mut out = self.clone();
        for (code, rule) in given {
            if CODES.contains(&code.as_str()) {
                out.merge_code(code, rule)?;
            }
        }
        Ok(out)
    }

    /// The rules of a file's text, entry by entry: a code that does not read keeps its
    /// default. The rules and whether the whole file read.
    pub fn from_file(text: &str) -> (Self, bool) {
        let mut rules = Self::default();
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return (rules, false);
        };
        let Some(given) = v.get("rules").and_then(Value::as_object) else {
            return (rules, false);
        };
        let mut whole = true;
        for (code, rule) in given {
            if !CODES.contains(&code.as_str()) {
                continue;
            }
            let mut one = rules.clone();
            match one.merge_code(code, rule) {
                Ok(()) => rules = one,
                Err(e) => {
                    log::warn!("hub alarms: {FILE_NAME}: {e}; its default kept");
                    whole = false;
                }
            }
        }
        (rules, whole)
    }

    /// The file's text.
    pub fn to_file(&self) -> String {
        let v = json!({"v": VERSION, "rules": self.to_json()});
        serde_json::to_string_pretty(&v).unwrap_or_default()
    }
}

// ── The alarm object ────────────────────────────────────────────────────────

/// `warning` or `critical`, by code (the book's own say for the audio link's).
pub fn severity(code: &str) -> &'static str {
    match code {
        "device_offline" | "unresponsive" | "temperature" => "critical",
        // -- WP P3RO --
        ROLLOUT_FAILED => "critical",
        _ => "warning",
    }
}

/// Whether a code is about one device (`serial` is its subject).
fn per_device(code: &str) -> bool {
    !matches!(code, "schedule_failed" | "clock_unset")
}

/// One alarm of this family as hub-api.md section 7 has it.
#[derive(Debug, Clone, PartialEq)]
pub struct AlarmView {
    /// `"device:<n>"` or `"alink:<n>"`.
    pub id: String,
    /// `device` or `alink`.
    pub source: &'static str,
    /// Its code.
    pub code: String,
    /// What it is about.
    pub subject: String,
    /// The device, for a device alarm about one.
    pub serial: Option<String>,
    /// `warning` or `critical`.
    pub severity: String,
    /// For people.
    pub message: String,
    /// Its state.
    pub state: AlarmState,
    /// Unix seconds.
    pub raised_at: u64,
    /// Unix seconds, once cleared.
    pub cleared_at: Option<u64>,
    /// Who acknowledged it.
    pub acknowledged_by: Option<String>,
    /// How often it was raised while on.
    pub count: u32,
}

impl AlarmView {
    fn device(a: &Alarm, by: Option<&String>) -> Self {
        Self {
            id: format!("device:{}", a.id),
            source: "device",
            code: a.code.clone(),
            subject: a.subject.clone(),
            serial: per_device(&a.code).then(|| a.subject.clone()),
            severity: severity(&a.code).into(),
            message: a.message.clone(),
            state: a.state,
            raised_at: a.raised_at,
            cleared_at: a.cleared_at,
            acknowledged_by: by.cloned(),
            count: a.count,
        }
    }

    /// An audio link alarm of the system's book.
    pub fn alink(a: &Alarm, by: Option<&String>) -> Self {
        Self {
            id: format!("alink:{}", a.id),
            source: "alink",
            code: a.code.clone(),
            subject: a.subject.clone(),
            serial: None,
            severity: a.severity.clone(),
            message: a.message.clone(),
            state: a.state,
            raised_at: a.raised_at,
            cleared_at: a.cleared_at,
            acknowledged_by: by.cloned(),
            count: a.count,
        }
    }

    /// The object of `alarm.list`, `alarm.ack` and `alarm.changed` (ISO times).
    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "id": self.id,
            "source": self.source,
            "code": self.code,
            "subject": self.subject,
            "severity": self.severity,
            "message": self.message,
            "state": self.state,
            "raised_at": iso8601(self.raised_at),
            "count": self.count,
        });
        if let Some(map) = v.as_object_mut() {
            if let Some(s) = &self.serial {
                map.insert("serial".into(), json!(s));
            }
            if let Some(c) = self.cleared_at {
                map.insert("cleared_at".into(), json!(iso8601(c)));
            }
            if let Some(by) = &self.acknowledged_by {
                map.insert("acknowledged_by".into(), json!(by));
            }
        }
        v
    }

    /// The bus message of this change, at `at`.
    pub fn to_bus(&self, at: u64) -> BusMessage {
        BusMessage::Alarm {
            id: self.id.clone(),
            source: self.source.into(),
            code: self.code.clone(),
            subject: self.subject.clone(),
            serial: self.serial.clone(),
            severity: self.severity.clone(),
            state: self.state,
            message: self.message.clone(),
            at,
            raised_at: self.raised_at,
            cleared_at: self.cleared_at,
            acknowledged_by: self.acknowledged_by.clone(),
            count: self.count,
        }
    }

    fn active(&self) -> bool {
        self.state != AlarmState::Cleared
    }
}

// ── The state machine ───────────────────────────────────────────────────────

/// The time, on both clocks.
#[derive(Debug, Clone, Copy)]
pub struct Now {
    /// The monotonic clock: how long things last.
    pub mono: Instant,
    /// The wall clock, Unix seconds: when things happened.
    pub wall_s: u64,
}

impl Now {
    /// The time on `clock`.
    pub fn of(clock: &dyn HubClock) -> Self {
        Self {
            mono: clock.now(),
            wall_s: clock.unix_seconds(),
        }
    }
}

/// What the state machine asks of its glue.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// An alarm changed: onto the bus and to every session.
    Changed(AlarmView),
    /// Mute this device (mute on clip storm).
    Mute {
        /// The device.
        serial: String,
    },
}

/// A refusal of a request: its code and a line for people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    /// The `code`.
    pub code: &'static str,
    /// The `msg`.
    pub msg: String,
}

impl Refused {
    fn new(code: &'static str, msg: impl Into<String>) -> Self {
        Self {
            code,
            msg: msg.into(),
        }
    }
}

/// What the alarms keep of one device.
#[derive(Debug)]
struct Dev {
    online: bool,
    /// When it went offline.
    offline_at: Option<Instant>,
    /// Silent since (wall seconds), and when that becomes an alarm.
    unresponsive: Option<(u64, Instant)>,
    /// Its `Facts` of this attach arrived.
    facts: bool,
    /// Its firmware has the extension, so it sends clip events (0xE0); without it the
    /// latch is all there is.
    ext: bool,
    /// Clip events within the window.
    clips: VecDeque<Instant>,
    /// The storm's rate has been under the threshold since.
    clip_under_since: Option<Instant>,
    /// The hub muted it in this storm.
    muted_this_storm: bool,
    /// CPU at or over the threshold since.
    cpu_over_since: Option<Instant>,
    /// When its last restart alarmed happened (wall seconds).
    last_restart_s: Option<u64>,
    /// When it was last heard of.
    seen: Instant,
}

impl Dev {
    fn new(now: Instant) -> Self {
        Self {
            online: false,
            offline_at: None,
            unresponsive: None,
            facts: false,
            ext: false,
            clips: VecDeque::new(),
            clip_under_since: None,
            muted_this_storm: false,
            cpu_over_since: None,
            last_restart_s: None,
            seen: now,
        }
    }
}

/// The device alarms of one hub run. No threads, no sockets: the caller passes the time.
pub struct DeviceAlarms {
    book: AlarmBook,
    rules: Rules,
    /// The book's alarms on, by (code, subject): their ids.
    on: BTreeMap<(String, String), u64>,
    devices: BTreeMap<String, Dev>,
    /// Who acknowledged an alarm, by id.
    acked_by: BTreeMap<u64, String>,
    /// When this run began (wall seconds).
    started_s: u64,
    /// The clock floor ([`BUILD_FLOOR_S`]; tests move it).
    floor_s: u64,
    // -- WP P3RO --
    /// The firmware a device with `rollout_failed` on is expected back with: its
    /// attach with it clears the alarm. Bounded by the alarms on.
    rollout_expect: BTreeMap<String, String>,
}

impl DeviceAlarms {
    /// No alarm yet, these rules, started at `now`. The clock floor is the schedule's
    /// ([`crate::schedule::build_floor_s`]: [`BUILD_FLOOR_S`], or a later
    /// `SOURCE_DATE_EPOCH`), so the alarm and the schedules judge the clock alike
    /// (review/p2c merge glue).
    pub fn new(rules: Rules, now: Now) -> Self {
        let floor = u64::try_from(crate::schedule::build_floor_s()).unwrap_or(BUILD_FLOOR_S);
        Self::with_floor(rules, now, floor)
    }

    /// [`DeviceAlarms::new`] with another clock floor.
    pub fn with_floor(rules: Rules, now: Now, floor_s: u64) -> Self {
        Self {
            book: AlarmBook::default(),
            rules,
            on: BTreeMap::new(),
            devices: BTreeMap::new(),
            acked_by: BTreeMap::new(),
            started_s: now.wall_s,
            floor_s,
            // -- WP P3RO --
            rollout_expect: BTreeMap::new(),
        }
    }

    /// The rules in force.
    pub fn rules(&self) -> &Rules {
        &self.rules
    }

    fn view(&self, a: &Alarm) -> AlarmView {
        AlarmView::device(a, self.acked_by.get(&a.id))
    }

    fn find(&self, id: u64) -> Option<Alarm> {
        self.book.list().into_iter().find(|a| a.id == id)
    }

    /// Whether (code, subject) is on.
    pub fn is_on(&self, code: &str, subject: &str) -> bool {
        self.on.contains_key(&(code.to_owned(), subject.to_owned()))
    }

    /// Raise (code, subject) if its rule is on. One already on counts again only if
    /// `again` (a new occurrence, not the same condition seen once more).
    fn raise(
        &mut self,
        code: &str,
        subject: &str,
        message: String,
        again: bool,
        now: Now,
        out: &mut Vec<Effect>,
    ) {
        // -- WP P3RO -- `rollout_failed` has no rule: always on.
        if !self.rules.on(code) && code != ROLLOUT_FAILED {
            return;
        }
        let key = (code.to_owned(), subject.to_owned());
        if let Some(&id) = self.on.get(&key) {
            if again {
                let _ = self.book.raise(code, subject, message, now.wall_s);
                if let Some(a) = self.find(id) {
                    out.push(Effect::Changed(self.view(&a)));
                }
            }
            return;
        }
        if self.on.len() >= MAX_ACTIVE {
            log::warn!("hub alarms: {MAX_ACTIVE} alarms on; {code} for {subject} not raised");
            return;
        }
        if let Some(a) = self.book.raise(code, subject, message, now.wall_s) {
            log::warn!("hub alarm raised: {code} {subject}: {}", a.message);
            self.on.insert(key, a.id);
            out.push(Effect::Changed(self.view(&a)));
        }
    }

    fn clear(&mut self, code: &str, subject: &str, now: Now, out: &mut Vec<Effect>) {
        if self
            .on
            .remove(&(code.to_owned(), subject.to_owned()))
            .is_none()
        {
            return;
        }
        if let Some(a) = self.book.clear(code, subject, now.wall_s) {
            log::info!("hub alarm cleared: {code} {subject}");
            out.push(Effect::Changed(self.view(&a)));
        }
        if self.acked_by.len() > MAX_ACKED {
            let kept: Vec<u64> = self.book.list().iter().map(|a| a.id).collect();
            self.acked_by.retain(|id, _| kept.contains(id));
        }
    }

    /// The device's entry, made on first sight (`None` past [`MAX_DEVICES`] with none
    /// offline to forget).
    fn dev(&mut self, serial: &str, now: Now) -> Option<&mut Dev> {
        if serial.is_empty() || serial.len() > MAX_SERIAL_BYTES {
            return None;
        }
        if !self.devices.contains_key(serial) {
            if self.devices.len() >= MAX_DEVICES {
                let oldest = self
                    .devices
                    .iter()
                    .filter(|(_, d)| !d.online)
                    .min_by_key(|(_, d)| d.seen)
                    .map(|(s, _)| s.clone())?;
                self.devices.remove(&oldest);
            }
            self.devices.insert(serial.to_owned(), Dev::new(now.mono));
        }
        let d = self.devices.get_mut(serial)?;
        d.seen = now.mono;
        Some(d)
    }

    /// One bus message. The effects, in order.
    pub fn on_message(&mut self, msg: &BusMessage, now: Now) -> Vec<Effect> {
        let mut out = Vec::new();
        match msg {
            BusMessage::DeviceOnline { serial, .. } => self.came_online(serial, now, &mut out),
            BusMessage::DeviceOffline { serial, .. } => self.went_offline(serial, now, &mut out),
            BusMessage::Facts {
                serial,
                identify,
                reset_info,
                fw,
            } => {
                // -- WP P3RO --
                self.rollout_back(serial, fw, now, &mut out);
                let restart = reset_info.map(|r| (r.reason, r.uptime_s));
                self.facts(
                    serial,
                    identify.is_some() || reset_info.is_some(),
                    restart,
                    now,
                    &mut out,
                );
            }
            BusMessage::Health {
                serial,
                cpu,
                clip_mask,
                temp_c,
                ..
            } => self.health(serial, *cpu, *clip_mask, *temp_c, now, &mut out),
            BusMessage::Unresponsive { serial, since } => {
                let after = Duration::from_secs(self.rules.int("unresponsive", "after_s"));
                let silent = Duration::from_secs(now.wall_s.saturating_sub(*since));
                let due = now
                    .mono
                    .checked_add(after.saturating_sub(silent))
                    .unwrap_or(now.mono);
                if let Some(d) = self.dev(serial, now)
                    && d.online
                {
                    d.unresponsive = Some((*since, due));
                }
                self.tick(now, &mut out);
            }
            BusMessage::Notification { serial, packet, .. } => {
                if let Some(mask) = clip_event(packet) {
                    self.clip_event(serial, mask, now, &mut out);
                }
            }
            BusMessage::ScheduleRan {
                rule,
                name,
                results,
                ..
            } => {
                let failed: Vec<_> = results.iter().filter(|r| !r.ok).collect();
                if let Some(first) = failed.first() {
                    let why = first.code.as_deref().unwrap_or("failed");
                    let message = cut(&format!(
                        "Schedule \"{name}\": {} of {} actions failed (first: {} {why})",
                        failed.len(),
                        results.len(),
                        first.serial
                    ));
                    self.raise("schedule_failed", &cut(rule), message, true, now, &mut out);
                }
            }
            // -- WP P3RO --
            BusMessage::RolloutEnded {
                serial,
                ok,
                step,
                text,
                firmware_version,
            } => self.rollout_ended(serial, *ok, step, text, firmware_version, now, &mut out),
            BusMessage::DeviceRemoved { .. }
            // -- WP P2DFIXA --
            | BusMessage::DeviceForgotten { .. }
            | BusMessage::Alarm { .. }
            | BusMessage::Event { .. } => {}
        }
        out
    }

    // -- WP P3RO --
    /// A device's rollout ended: a failure raises `rollout_failed` (critical) with the
    /// step, a success clears it.
    #[allow(clippy::too_many_arguments)]
    fn rollout_ended(
        &mut self,
        serial: &str,
        ok: bool,
        step: &str,
        text: &str,
        firmware_version: &str,
        now: Now,
        out: &mut Vec<Effect>,
    ) {
        if serial.is_empty() || serial.len() > MAX_SERIAL_BYTES {
            return;
        }
        if ok {
            self.rollout_expect.remove(serial);
            self.clear(ROLLOUT_FAILED, serial, now, out);
            return;
        }
        let message = cut(&format!("Firmware rollout failed at {step}: {text}"));
        self.raise(ROLLOUT_FAILED, serial, message, true, now, out);
        // A device that failed its verify came back with the image and still failed:
        // a replug clears nothing then.
        if step != "verify" && !firmware_version.is_empty() && self.is_on(ROLLOUT_FAILED, serial) {
            self.rollout_expect
                .insert(serial.to_owned(), cut(firmware_version));
        } else {
            self.rollout_expect.remove(serial);
        }
    }

    // -- WP P3RO --
    /// A device attached with `fw`: back with the firmware its failed rollout put on it.
    fn rollout_back(&mut self, serial: &str, fw: &str, now: Now, out: &mut Vec<Effect>) {
        if fw.is_empty() || self.rollout_expect.get(serial).map(String::as_str) != Some(fw) {
            return;
        }
        self.rollout_expect.remove(serial);
        self.clear(ROLLOUT_FAILED, serial, now, out);
    }

    fn came_online(&mut self, serial: &str, now: Now, out: &mut Vec<Effect>) {
        if let Some(d) = self.dev(serial, now) {
            d.online = true;
            d.offline_at = None;
            d.unresponsive = None;
            d.facts = false;
            d.clips.clear();
            d.clip_under_since = None;
            d.cpu_over_since = None;
        }
        self.clear("device_offline", serial, now, out);
        self.clear("unresponsive", serial, now, out);
    }

    fn went_offline(&mut self, serial: &str, now: Now, out: &mut Vec<Effect>) {
        let storm = self.devices.get(serial).is_some_and(|d| d.ext);
        if let Some(d) = self.dev(serial, now) {
            d.online = false;
            d.offline_at = Some(now.mono);
            d.unresponsive = None;
            d.cpu_over_since = None;
            d.clips.clear();
            d.muted_this_storm = false;
        }
        // What only a running device has is over; a restart and the latch stay.
        self.clear("unresponsive", serial, now, out);
        self.clear("cpu", serial, now, out);
        self.clear("temperature", serial, now, out);
        if storm {
            self.clear("clip", serial, now, out);
        }
    }

    fn facts(
        &mut self,
        serial: &str,
        ext: bool,
        restart: Option<(ResetReason, u32)>,
        now: Now,
        out: &mut Vec<Effect>,
    ) {
        let started = self.started_s;
        let Some(d) = self.dev(serial, now) else {
            return;
        };
        d.facts = true;
        d.ext = ext;
        let Some((reason, uptime_s)) = restart else {
            return;
        };
        if !matches!(
            reason,
            ResetReason::Watchdog | ResetReason::Glitch | ResetReason::Brownout
        ) {
            return;
        }
        let at = now.wall_s.saturating_sub(u64::from(uptime_s));
        let fresh =
            at.saturating_add(RESTART_SLACK_S) >= started || u64::from(uptime_s) < RECENT_RESTART_S;
        let same = d
            .last_restart_s
            .is_some_and(|s| s.abs_diff(at) <= RESTART_SLACK_S);
        if !fresh || same {
            return;
        }
        d.last_restart_s = Some(at);
        let message = format!(
            "Restarted at {} ({}: {})",
            iso8601(at),
            reason.as_str(),
            reason.describe()
        );
        self.raise("device_restarted", serial, cut(&message), true, now, out);
    }

    fn health(
        &mut self,
        serial: &str,
        cpu: Option<u8>,
        clip_mask: u32,
        temp_c: Option<f32>,
        now: Now,
        out: &mut Vec<Effect>,
    ) {
        let above_c = self.rules.num("temperature", "above_c");
        let above_pct = self.rules.int("cpu", "above_pct");
        let for_s = Duration::from_secs(self.rules.int("cpu", "for_s"));
        let Some(d) = self.dev(serial, now) else {
            return;
        };
        // A reading: it answers.
        let answered = d.unresponsive.take().is_some();
        let latch = d.facts && !d.ext;
        let cpu_over = match cpu {
            Some(c) if u64::from(c) >= above_pct => {
                let since = *d.cpu_over_since.get_or_insert(now.mono);
                Some(now.mono.saturating_duration_since(since) >= for_s)
            }
            Some(_) => {
                d.cpu_over_since = None;
                None
            }
            None => Some(false),
        };
        if answered || self.is_on("unresponsive", serial) {
            self.clear("unresponsive", serial, now, out);
        }
        if let Some(c) = temp_c.map(f64::from).filter(|c| c.is_finite()) {
            if c >= above_c {
                let message = format!("{c:.1} °C, at or above {above_c:.0} °C");
                self.raise("temperature", serial, message, false, now, out);
            } else if c < above_c - TEMPERATURE_HYSTERESIS_C {
                self.clear("temperature", serial, now, out);
            }
        }
        match (cpu_over, cpu) {
            (Some(true), Some(c)) => {
                let message = format!(
                    "CPU {c} %, at or above {above_pct} % for {} s",
                    for_s.as_secs()
                );
                self.raise("cpu", serial, message, false, now, out);
            }
            (None, _) => self.clear("cpu", serial, now, out),
            _ => {}
        }
        if latch {
            if clip_mask != 0 {
                let message = format!(
                    "Clipped since the clip indicator was last cleared ({})",
                    channel_words(clip_mask)
                );
                self.raise("clip", serial, message, false, now, out);
            } else {
                self.clear("clip", serial, now, out);
            }
        }
    }

    fn clip_event(&mut self, serial: &str, mask: u32, now: Now, out: &mut Vec<Effect>) {
        let per_minute = self.rules.int("clip", "per_minute");
        let mute = self.rules.flag("clip", "mute_on_storm") && self.rules.on("clip");
        let Some(d) = self.dev(serial, now) else {
            return;
        };
        if !d.online || !d.ext {
            return;
        }
        d.clips.push_back(now.mono);
        trim(&mut d.clips, now.mono);
        while d.clips.len() > MAX_CLIP_EVENTS {
            d.clips.pop_front();
        }
        let n = d.clips.len();
        if u64::try_from(n).unwrap_or(u64::MAX) < per_minute {
            return;
        }
        d.clip_under_since = None;
        let do_mute = mute && !d.muted_this_storm;
        if do_mute {
            d.muted_this_storm = true;
        }
        let mut message = format!(
            "Clip storm: {n} clip events within a minute (the latest on {})",
            channel_words(mask)
        );
        if do_mute {
            message.push_str("; the hub muted the device (mute on clip storm)");
        }
        self.raise("clip", serial, message, false, now, out);
        if do_mute {
            out.push(Effect::Mute {
                serial: serial.to_owned(),
            });
        }
    }

    /// What time alone decides: an offline or silent device past its rule, a CPU over
    /// for long enough, a clip storm over, the clock.
    pub fn tick(&mut self, now: Now, out: &mut Vec<Effect>) {
        let offline_after = Duration::from_secs(self.rules.int("device_offline", "after_s"));
        let per_minute = self.rules.int("clip", "per_minute");
        let mut offline = Vec::new();
        let mut silent = Vec::new();
        let mut calm = Vec::new();
        for (serial, d) in &mut self.devices {
            if let Some(at) = d.offline_at
                && now.mono.saturating_duration_since(at) >= offline_after
            {
                offline.push(serial.clone());
            }
            if let Some((since, due)) = d.unresponsive
                && now.mono >= due
            {
                silent.push((serial.clone(), since));
            }
            trim(&mut d.clips, now.mono);
            if d.ext && u64::try_from(d.clips.len()).unwrap_or(u64::MAX) < per_minute {
                let under = *d.clip_under_since.get_or_insert(now.mono);
                if now.mono.saturating_duration_since(under) >= CLIP_WINDOW {
                    calm.push(serial.clone());
                }
            }
        }
        for serial in offline {
            if !self.is_on("device_offline", &serial) {
                let secs = offline_after.as_secs();
                let since = now.wall_s.saturating_sub(secs);
                let message = format!("Not attached to the hub since {}", iso8601(since));
                self.raise("device_offline", &serial, message, false, now, out);
            }
        }
        for (serial, since) in silent {
            if !self.is_on("unresponsive", &serial) {
                let message = format!("Attached but not answering since {}", iso8601(since));
                self.raise("unresponsive", &serial, message, false, now, out);
            }
        }
        for serial in calm {
            if self.is_on("clip", &serial) {
                self.clear("clip", &serial, now, out);
            }
            if let Some(d) = self.devices.get_mut(&serial) {
                d.muted_this_storm = false;
            }
        }
        if now.wall_s < self.floor_s {
            let message = format!(
                "The hub's clock reads {}, before this build's date: schedules wait until it is set",
                iso8601(now.wall_s)
            );
            self.raise("clock_unset", "hub", message, false, now, out);
        } else {
            self.clear("clock_unset", "hub", now, out);
        }
        // CPU: over for long enough without a new reading.
        let for_s = Duration::from_secs(self.rules.int("cpu", "for_s"));
        let hot: Vec<String> = self
            .devices
            .iter()
            .filter(|(_, d)| {
                d.online
                    && d.cpu_over_since
                        .is_some_and(|s| now.mono.saturating_duration_since(s) >= for_s)
            })
            .map(|(s, _)| s.clone())
            .collect();
        let above_pct = self.rules.int("cpu", "above_pct");
        for serial in hot {
            let message = format!("CPU at or above {above_pct} % for {} s", for_s.as_secs());
            self.raise("cpu", &serial, message, false, now, out);
        }
    }

    /// Acknowledge alarm `n` for `by`. A `device_restarted` or `schedule_failed`
    /// acknowledged is over (cleared). The changes, the acknowledged alarm first.
    pub fn acknowledge(
        &mut self,
        n: u64,
        by: Option<String>,
        now: Now,
    ) -> Result<Vec<Effect>, Refused> {
        let Some(before) = self.find(n).filter(|a| a.state != AlarmState::Cleared) else {
            return Err(Refused::new("not_found", "no such alarm on"));
        };
        let mut out = Vec::new();
        if before.state == AlarmState::Raised {
            let Some(a) = self.book.acknowledge(n, now.wall_s) else {
                return Err(Refused::new("not_found", "no such alarm on"));
            };
            if let Some(by) = by {
                self.acked_by.insert(n, by);
            }
            out.push(Effect::Changed(self.view(&a)));
        }
        if matches!(before.code.as_str(), "device_restarted" | "schedule_failed") {
            let (code, subject) = (before.code.clone(), before.subject.clone());
            self.clear(&code, &subject, now, &mut out);
        }
        if out.is_empty() {
            out.push(Effect::Changed(self.view(&before)));
        }
        Ok(out)
    }

    /// New rules: the alarms of a code turned off are cleared. The changes.
    pub fn set_rules(&mut self, rules: Rules, now: Now) -> Vec<Effect> {
        self.rules = rules;
        let mut out = Vec::new();
        let off: Vec<(String, String)> = self
            .on
            .keys()
            // -- WP P3RO -- `rollout_failed` has no rule to turn off
            .filter(|(code, _)| !self.rules.on(code) && code != ROLLOUT_FAILED)
            .cloned()
            .collect();
        for (code, subject) in off {
            self.clear(&code, &subject, now, &mut out);
        }
        if !self.rules.flag("clip", "mute_on_storm") {
            for d in self.devices.values_mut() {
                d.muted_this_storm = false;
            }
        }
        out
    }

    /// Every alarm: those on (newest first), then the cleared ones kept.
    pub fn list(&self) -> Vec<AlarmView> {
        self.book.list().iter().map(|a| self.view(a)).collect()
    }

    /// Whether the hub's devices it tracks include `serial` online.
    pub fn online(&self, serial: &str) -> bool {
        self.devices.get(serial).is_some_and(|d| d.online)
    }

    /// The registry says these serials are online: catch up after lost bus messages.
    pub fn reconcile(&mut self, online: &[String], now: Now) -> Vec<Effect> {
        let mut out = Vec::new();
        for s in online {
            if !self.online(s) {
                self.came_online(s, now, &mut out);
            }
        }
        let gone: Vec<String> = self
            .devices
            .iter()
            .filter(|(s, d)| d.online && !online.contains(s))
            .map(|(s, _)| s.clone())
            .collect();
        for s in gone {
            self.went_offline(&s, now, &mut out);
        }
        out
    }
}

/// Drop clip events older than the window.
fn trim(clips: &mut VecDeque<Instant>, now: Instant) {
    while clips
        .front()
        .is_some_and(|t| now.saturating_duration_since(*t) >= CLIP_WINDOW)
    {
        clips.pop_front();
    }
}

/// "channel 1, 3" for a mask (channels counted from 1).
fn channel_words(mask: u32) -> String {
    let chans: Vec<String> = (0u32..32)
        .filter(|c| mask & 1u32.checked_shl(*c).unwrap_or(0) != 0)
        .map(|c| c.saturating_add(1).to_string())
        .collect();
    match chans.len() {
        0 => "no channel".into(),
        1 => format!("channel {}", chans.join("")),
        _ => format!("channels {}", chans.join(", ")),
    }
}

/// The channel mask of an extension clip event (0xE0), if `packet` is one. The bus
/// carries only the notifications of firmware that may send it ([`wanted`]); whether
/// the device has the extension is the caller's.
fn clip_event(packet: &[u8]) -> Option<u32> {
    match notify::decode(packet).ok()?.1 {
        Event::Unknown { id, bytes } => match decode_ext_event(Some(u32::MAX), id, &bytes)? {
            ExtEvent::Clip { mask } => Some(mask),
            _ => None,
        },
        _ => None,
    }
}

/// What the alarms take from the bus: no history records and no alarms (their own
/// echoes), and of the notifications only the clip events.
fn wanted(m: &BusMessage) -> bool {
    match m {
        BusMessage::Notification { packet, .. } => packet.get(1) == Some(&NOTIFY_EVT_EXT_CLIP),
        BusMessage::Alarm { .. } | BusMessage::Event { .. } | BusMessage::DeviceRemoved { .. } => {
            false
        }
        // -- WP P2DFIXA --
        BusMessage::DeviceForgotten { .. } => false,
        _ => true,
    }
}

// ── The file ────────────────────────────────────────────────────────────────

/// Write `text` to `path` atomically (a temporary file, then a rename), 0600 on Unix.
fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut o = OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    {
        let mut f = o.open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// The rules in `path`: the defaults when there is none; a file that does not read
/// whole is kept aside as `alarms.json.bad-<time>`, the rules that read are kept and
/// written back.
pub fn load(path: &Path, now_s: u64) -> Rules {
    let mut bytes = Vec::new();
    let read = fs::File::open(path).and_then(|f| {
        f.take(MAX_FILE_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
    });
    let (rules, whole) = match read {
        Ok(_) if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES => {
            log::error!("hub alarms: {} is over 64 KiB", path.display());
            (Rules::default(), false)
        }
        Ok(_) => Rules::from_file(&String::from_utf8_lossy(&bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Rules::default(),
        Err(e) => {
            log::error!("hub alarms: {} unread ({e})", path.display());
            (Rules::default(), false)
        }
    };
    if !whole {
        let mut aside = path.as_os_str().to_owned();
        aside.push(format!(".bad-{now_s}"));
        log::error!(
            "hub alarms: {} did not read whole; kept aside, the rules that read are kept",
            path.display()
        );
        if let Err(e) = fs::rename(path, PathBuf::from(&aside)) {
            log::error!("hub alarms: cannot keep {} aside: {e}", path.display());
        }
        if let Err(e) = write_atomic(path, &rules.to_file()) {
            log::error!("hub alarms: cannot write {}: {e}", path.display());
        }
    }
    rules
}

// ── The service ─────────────────────────────────────────────────────────────

struct State {
    alarms: DeviceAlarms,
    /// The audio link's alarms as last announced: (state, count) by id.
    alink: BTreeMap<u64, (AlarmState, u32)>,
    /// Who acknowledged an audio link alarm through `alarm.ack`.
    alink_by: BTreeMap<u64, String>,
    next_tick: Instant,
    /// When the bus was last read.
    last_run: Instant,
}

/// A mute's outcome, from the device's worker.
struct MuteDone {
    serial: String,
    status: u8,
}

/// The `alarm.*` family as a hub service.
pub struct AlarmService {
    state: Mutex<State>,
    sub: Subscription,
    bus: Arc<Bus>,
    clock: Arc<dyn HubClock>,
    wake: Waker,
    data_dir: Option<PathBuf>,
    mutes: Arc<Mutex<Vec<MuteDone>>>,
}

/// The service, when the config asks for it (the one `registered` line).
pub fn service(config: &ServiceConfig, setup: &ServiceSetup<'_>) -> Option<Arc<dyn HubService>> {
    config
        .alarms
        .then(|| Arc::new(AlarmService::new(setup)) as Arc<dyn HubService>)
}

fn reply_err(id: &Value, code: &str, msg: &str) -> String {
    json!({"t": "err", "id": id, "v": VERSION, "code": code, "msg": msg}).to_string()
}

fn reply_ok(id: &Value, body: Value) -> String {
    let mut v = json!({"t": "ok", "id": id, "v": VERSION});
    if let (Some(map), Value::Object(body)) = (v.as_object_mut(), body) {
        map.extend(body);
    }
    v.to_string()
}

/// The hub's own session (PLAN-P2 M3): Admin, never limited by in-flight.
fn internal() -> SessionCtx {
    SessionCtx {
        id: INTERNAL_SESSION_ID,
        role: Role::Admin,
        exempt_inflight: true,
    }
}

/// An `alarm` field: `("device" | "alink", n)`.
fn alarm_id(v: Option<&Value>) -> Result<(&'static str, u64), Refused> {
    let bad = || {
        Refused::new(
            "bad_request",
            "alarm must be an alarm's id (\"device:<n>\")",
        )
    };
    let s = v.and_then(Value::as_str).ok_or_else(bad)?;
    let (source, n) = s.split_once(':').ok_or_else(bad)?;
    let n: u64 = n.parse().map_err(|_| bad())?;
    match source {
        "device" => Ok(("device", n)),
        "alink" => Ok(("alink", n)),
        _ => Err(bad()),
    }
}

impl AlarmService {
    /// The service: its rules from `<data>/alarms.json` (the defaults in process).
    pub fn new(setup: &ServiceSetup<'_>) -> Self {
        let clock = Arc::clone(setup.clock);
        let data_dir = setup.data_dir.map(Path::to_path_buf);
        let rules = match &data_dir {
            Some(dir) => load(&dir.join(FILE_NAME), clock.unix_seconds()),
            None => Rules::default(),
        };
        let now = Now::of(clock.as_ref());
        Self {
            state: Mutex::new(State {
                alarms: DeviceAlarms::new(rules, now),
                alink: BTreeMap::new(),
                alink_by: BTreeMap::new(),
                next_tick: now.mono,
                last_run: now.mono,
            }),
            sub: setup.bus.subscribe_filtered("alarms", BUS_CAPACITY, wanted),
            bus: Arc::clone(setup.bus),
            clock,
            wake: setup.wake.clone(),
            data_dir,
            mutes: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn file(&self) -> Option<PathBuf> {
        self.data_dir.as_ref().map(|d| d.join(FILE_NAME))
    }

    /// The audio link's changes since they were last announced.
    fn alink_changes(st: &mut State, list: &[Alarm]) -> Vec<AlarmView> {
        let mut out = Vec::new();
        for a in list {
            let now = (a.state, a.count);
            if st.alink.insert(a.id, now) != Some(now) {
                out.push(AlarmView::alink(a, st.alink_by.get(&a.id)));
            }
        }
        st.alink.retain(|id, _| list.iter().any(|a| a.id == *id));
        st.alink_by.retain(|id, _| list.iter().any(|a| a.id == *id));
        out
    }

    /// Catch up with the bus, the clock and the audio link's book; announce and act.
    /// With no hub lock held.
    fn process(&self, hub: &HubHandle) {
        let messages = self.sub.drain();
        let dropped = self.sub.take_dropped();
        let online: Vec<String> = if dropped > 0 {
            log::warn!("hub alarms: {dropped} bus message(s) lost; reading the registry");
            hub.devices()
                .into_iter()
                .filter(|d| d.state == DeviceState::Online)
                .map(|d| d.identity.serial)
                .collect()
        } else {
            Vec::new()
        };
        let alink = hub.system_alarms();
        let now = Now::of(self.clock.as_ref());
        let (effects, alink_views) = {
            let mut st = lock(&self.state);
            st.next_tick = now.mono.checked_add(TICK).unwrap_or(now.mono);
            st.last_run = now.mono;
            let mut out = Vec::new();
            if dropped > 0 {
                out.extend(st.alarms.reconcile(&online, now));
            }
            for m in &messages {
                out.extend(st.alarms.on_message(m, now));
            }
            st.alarms.tick(now, &mut out);
            let views = Self::alink_changes(&mut st, &alink);
            (out, views)
        };
        let done: Vec<MuteDone> = std::mem::take(&mut *lock(&self.mutes));
        for m in done {
            self.mute_recorded(m);
        }
        for v in alink_views {
            self.announce(hub, &v, now.wall_s);
        }
        self.act(hub, effects, now);
    }

    fn act(&self, hub: &HubHandle, effects: Vec<Effect>, now: Now) {
        for e in effects {
            match e {
                Effect::Changed(v) => self.announce(hub, &v, now.wall_s),
                Effect::Mute { serial } => self.mute(hub, &serial),
            }
        }
    }

    /// An alarm changed: onto the bus and to every session.
    fn announce(&self, hub: &HubHandle, v: &AlarmView, at: u64) {
        self.bus.publish(v.to_bus(at));
        let event =
            json!({"t": CHANGED_EVENT, "v": VERSION, "hub": hub.hub_id(), "alarm": v.to_json()});
        hub.inner.relay.broadcast_event(&event.to_string());
    }

    /// Mute on clip storm: `REQ_SET_USER_MUTE` 1 as the hub's own write, past the lock
    /// once. Never waits: the outcome comes back through `mutes`.
    fn mute(&self, hub: &HubHandle, serial: &str) {
        let Some(dev) = hub.device_by_serial(serial) else {
            lock(&self.mutes).push(MuteDone {
                serial: serial.to_owned(),
                status: status::NO_DEVICE,
            });
            return;
        };
        let req = CmdRequest {
            tag: 0,
            handle: dev.handle,
            dir: Dir::Set,
            b_request: op::REQ_SET_USER_MUTE,
            w_value: 0,
            w_index: dspi_proto::VENDOR_INTERFACE,
            w_length: 1,
            payload: vec![1],
        };
        let mutes = Arc::clone(&self.mutes);
        let wake = self.wake.clone();
        let who = serial.to_owned();
        let done = Completion::new(0, move |r| {
            lock(&mutes).push(MuteDone {
                serial: who,
                status: r.status,
            });
            wake.wake();
        });
        log::warn!("hub alarms: clip storm on {serial}: muting it (mute on clip storm)");
        // Exactly this write, once, whatever the lock (OD-P2o): a mute a lock could
        // block would protect nothing.
        dev.router.pass_hold(&req);
        dev.submit(
            internal(),
            req,
            done,
            Instant::now().checked_add(Duration::from_secs(3)),
        );
    }

    /// The mute's history record, `by` the rule.
    fn mute_recorded(&self, m: MuteDone) {
        let text = if m.status == status::OK {
            "Muted by the hub: a clip storm (mute on clip storm)".to_owned()
        } else {
            log::error!(
                "hub alarms: the mute of {} failed ({})",
                m.serial,
                status::name(m.status)
            );
            format!(
                "Mute on clip storm failed: the device answered {}",
                status::name(m.status)
            )
        };
        let mut r = EventRecord::new(EventKind::Notification, self.clock.unix_ms(), text);
        r.source = Some(MUTE_SOURCE.into());
        r.session = Some(u32::from(INTERNAL_SESSION_ID));
        r.by = Some(MUTE_BY.into());
        self.bus.publish(BusMessage::Event {
            serial: m.serial,
            record: r,
        });
    }

    fn list(&self, hub: &HubHandle, msg: &Value) -> Result<Value, Refused> {
        let serial = match msg.get("serial") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if !s.is_empty() && s.len() <= MAX_SERIAL_BYTES => {
                Some(s.clone())
            }
            Some(_) => return Err(Refused::new("bad_request", "serial is not a serial")),
        };
        let state = match msg.get("state") {
            None | Some(Value::Null) => "all",
            Some(Value::String(s)) if matches!(s.as_str(), "active" | "cleared" | "all") => {
                s.as_str()
            }
            Some(_) => {
                return Err(Refused::new(
                    "bad_request",
                    "state is active, cleared or all",
                ));
            }
        };
        let alink = hub.system_alarms();
        let mut all = {
            let st = lock(&self.state);
            let mut all = st.alarms.list();
            all.extend(
                alink
                    .iter()
                    .map(|a| AlarmView::alink(a, st.alink_by.get(&a.id))),
            );
            all
        };
        all.retain(|a| match state {
            "active" => a.active(),
            "cleared" => !a.active(),
            _ => true,
        });
        if let Some(s) = &serial {
            all.retain(|a| a.serial.as_ref() == Some(s));
        }
        // On first, newest first; then the cleared, last cleared first.
        all.sort_by(|a, b| {
            b.active()
                .cmp(&a.active())
                .then(b.cleared_at.cmp(&a.cleared_at))
                .then(b.raised_at.cmp(&a.raised_at))
        });
        all.truncate(MAX_LIST);
        let alarms: Vec<Value> = all.iter().map(AlarmView::to_json).collect();
        Ok(json!({"alarms": alarms}))
    }

    fn ack(&self, hub: &HubHandle, session: &Session, msg: &Value) -> Result<Value, Refused> {
        let (source, n) = alarm_id(msg.get("alarm"))?;
        let by = hub.session_label(session.id()).by();
        let now = Now::of(self.clock.as_ref());
        if source == "alink" {
            let Some(a) = hub.system_acknowledge(n) else {
                return Err(Refused::new("not_found", "no such alarm on"));
            };
            let views = {
                let mut st = lock(&self.state);
                if let Some(by) = by {
                    st.alink_by.insert(n, by);
                }
                let list = hub.system_alarms();
                Self::alink_changes(&mut st, &list)
            };
            for v in &views {
                self.announce(hub, v, now.wall_s);
            }
            let view = {
                let st = lock(&self.state);
                AlarmView::alink(&a, st.alink_by.get(&a.id))
            };
            return Ok(json!({"alarm": view.to_json()}));
        }
        let effects = lock(&self.state).alarms.acknowledge(n, by, now)?;
        let first = effects.iter().find_map(|e| match e {
            Effect::Changed(v) => Some(v.to_json()),
            Effect::Mute { .. } => None,
        });
        self.act(hub, effects, now);
        Ok(json!({"alarm": first}))
    }

    fn rules_set(&self, hub: &HubHandle, msg: &Value) -> Result<Value, Refused> {
        let given = msg
            .get("rules")
            .ok_or_else(|| Refused::new("bad_request", "no rules"))?;
        let now = Now::of(self.clock.as_ref());
        let (effects, rules) = {
            let mut st = lock(&self.state);
            let rules = st
                .alarms
                .rules()
                .merged(given)
                .map_err(|e| Refused::new("bad_request", e))?;
            if let Some(path) = self.file()
                && let Err(e) = write_atomic(&path, &rules.to_file())
            {
                log::error!("hub alarms: cannot write {}: {e}", path.display());
                return Err(Refused::new("internal", "the rules could not be saved"));
            }
            let json = rules.to_json();
            (st.alarms.set_rules(rules, now), json)
        };
        self.act(hub, effects, now);
        Ok(json!({"rules": rules}))
    }
}

impl HubService for AlarmService {
    fn name(&self) -> &'static str {
        "alarms"
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
        if !MESSAGES.contains(&t) {
            return None;
        }
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let least = match t {
            "alarm.ack" => Role::Control,
            "alarm.rules.set" => Role::Admin,
            _ => Role::Viewer,
        };
        if session.role() < least {
            let who = if least == Role::Admin {
                "an admin's"
            } else {
                "needs the control role"
            };
            return Some(reply_err(&id, "denied", who));
        }
        // Answer from alarms that have seen everything up to now.
        self.process(hub);
        let answer = match t {
            "alarm.list" => self.list(hub, msg),
            "alarm.ack" => self.ack(hub, session, msg),
            "alarm.rules.get" => Ok(json!({"rules": lock(&self.state).alarms.rules().to_json()})),
            _ => self.rules_set(hub, msg),
        };
        Some(match answer {
            Ok(body) => reply_ok(&id, body),
            Err(r) => reply_err(&id, r.code, &r.msg),
        })
    }

    fn next_due(&self) -> Option<Instant> {
        let st = lock(&self.state);
        if self.sub.is_empty() && lock(&self.mutes).is_empty() {
            Some(st.next_tick)
        } else {
            // Work waiting: due since the last run (a time the timer's `now` is past,
            // which `clock.now()` read here would not be).
            Some(st.last_run)
        }
    }

    fn run_due(&self, hub: &HubHandle, _now: Instant) {
        self.process(hub);
    }

    fn on_device(&self, event: &DeviceEvent) {
        let _ = event;
        self.wake.wake();
    }
}

#[cfg(test)]
#[path = "device_alarms/tests_wp_p2al.rs"]
mod tests_wp_p2al;

// -- WP P3RO --
#[cfg(test)]
#[path = "device_alarms/tests_wp_p3ro.rs"]
mod tests_wp_p3ro;
