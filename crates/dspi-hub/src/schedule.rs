//! Schedules (WP P2SC, PLAN-P2 I5 and OD-P2h; `docs/integrator/hub-api.md` section 12).
//!
//! Rules in `<data>/schedule.json` (Admin): on given days at a local time, or once at a
//! local date and time, a list of actions (`preset.load`, `snapshot.recall`,
//! `system.scene.apply`, `param.set` for `vol.master`, `vol.user`, `vol.mute` and
//! `out.mute`) on a list of targets (serials; `group:<id>` is phase 3's and answers
//! `unsupported`). At most [`MAX_RULES`] rules of at most [`MAX_ACTIONS`] actions and
//! [`MAX_TARGETS`] targets.
//!
//! **Who writes (OD-P2m, the session-id rule).** A run opens the principal
//! `Schedule: <name>` with the Admin role ([`HubHandle::open_principal`]: a session id
//! from the remote range) and runs each action through P2PA's action executor
//! (`crate::api::exec`), as a child session of that principal, so the limits, the
//! installer lock, the device lock and the voltage rule judge it as any Admin client's
//! write (an image is held after the fact, as P1HL does). The principal hears nothing: its
//! relay subscription goes as soon as it opens (nobody reads its outbox), and it is closed
//! when the run ends. A target that is offline fails at once and is reported, never tried
//! later; a target with a pending guarded apply is `conflict` (review S4). `run_now`
//! runs as the same principal, with the asking Admin in `by`.
//!
//! **The clock (OD-P2h)**, in [`Engine::tick`], pure and tested with a given wall time,
//! monotonic time and zone:
//! - local time in the hub machine's time zone (`jiff`, the system's zone database), or
//!   the one [`ScheduleConfig::zone`] names;
//! - at start, and when the clock becomes readable, a rule missed by up to [`GRACE_S`]
//!   runs; older ones do not (no catch-up after downtime);
//! - a forward jump of the wall clock (more wall time than monotonic time passed) skips
//!   what it passed and records it (`last.skipped`, a history line per target); a
//!   backward jump runs nothing again: a rule runs at most once per local day;
//! - a local time that does not exist (spring forward) runs at the first valid minute
//!   after the gap; one that happens twice (fall back) runs once, at the first;
//! - while the wall clock reads before the build date ([`build_floor_s`]) nothing runs and
//!   the `clock_unset` alarm is raised on the bus (cleared once the clock is set).
//!
//! **Threads.** The decisions are made on the hub's timer (`run_due`, short: no file or
//! device I/O); the runs, the file writes and the records are made on a thread of this
//! service (`hub-schedule`), one run at a time, at most [`MAX_PENDING`] waiting. `run_now`
//! runs on the asking connection's reader and answers when the run is done.
//!
//! Each run goes on the bus as `ScheduleRan {rule, name, at, by, results}` (the history
//! records it per device, P2EV; notification targets deliver it, P2WH; a failure raises
//! `schedule_failed`, P2AL) and to every session as `schedule.ran`.

// Without feature `api` the service is off (it runs actions through the API's executor):
// the rules, the engine and the file are still built, and tested, but not used.
#![cfg_attr(not(feature = "api"), allow(dead_code, unused_imports))]

use std::collections::VecDeque;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use jiff::Timestamp;
use jiff::civil::{Date, DateTime, Time};
use jiff::tz::{AmbiguousOffset, TimeZone};
use serde_json::{Map, Value, json};

use crate::bus::{Bus, BusMessage, TargetResult};
use crate::clock::HubClock;
use crate::hub::HubHandle;
use crate::service::{HubService, ServiceConfig, ServiceSetup, Waker};
use crate::session::{Role, Session};
use crate::system::alarms::AlarmState;

/// The `hello` cap.
pub const CAP: &str = "schedule";
/// `v` in every answer and event.
pub const VERSION: u32 = 1;
/// The rules' file in the hub's data folder.
pub const FILE_NAME: &str = "schedule.json";
/// The most rules (review S3).
pub const MAX_RULES: usize = 256;
/// The most actions in one rule (review S3).
pub const MAX_ACTIONS: usize = 16;
/// The most targets in one rule: 16 actions on 16 devices is 256 result rows, which
/// fit one DSPi Link frame.
pub const MAX_TARGETS: usize = 16;
/// The longest rule name, in characters (`Schedule: <name>` is a session label).
pub const MAX_NAME_CHARS: usize = 48;
/// A rule missed by up to this long (downtime, a stalled timer) still runs.
pub const GRACE_S: i64 = 300;
/// More wall time than monotonic time by this much between two looks is a jump.
pub const JUMP_SLACK_S: i64 = 60;
/// The longest the timer waits between two looks at the clock.
pub const TICK: Duration = Duration::from_secs(30);
/// Runs waiting for the schedule's thread; past it a run is dropped (and logged).
pub const MAX_PENDING: usize = 64;
/// The day this code was written (2026-11-15T00:00:00Z): the least build date. A build
/// with `SOURCE_DATE_EPOCH` set takes that when it is later.
pub const SOURCE_FLOOR_S: i64 = 1_794_700_800;
/// The messages of this family.
pub const MESSAGES: [&str; 4] = [
    "schedule.list",
    "schedule.set",
    "schedule.delete",
    "schedule.run_now",
];
/// The actions a rule may run.
pub const ACTIONS: [&str; 4] = [
    "preset.load",
    "snapshot.recall",
    "system.scene.apply",
    "param.set",
];
/// The parameters `param.set` may write in a rule.
pub const SCHEDULED_PATHS: [&str; 4] = ["vol.master", "vol.user", "vol.mute", "out.mute"];

/// Preset slots on a DSPi (0 to 9).
const PRESET_SLOTS: u64 = 10;
/// The largest file read.
const MAX_FILE_BYTES: u64 = 4 << 20;
/// How far back a jump or a stall is looked at for what it skipped.
const WINDOW_S: i64 = 8 * 86_400;
/// A result row's `msg`, at most (256 rows must fit one frame).
const MAX_MSG_CHARS: usize = 80;
const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
/// The alarm's id on the bus (the alarm book's own ids are P2AL's).
pub const CLOCK_ALARM_ID: &str = "schedule:clock_unset";

/// The least wall-clock time this build believes: [`SOURCE_FLOOR_S`], or the build's
/// `SOURCE_DATE_EPOCH` when that is later.
pub fn build_floor_s() -> i64 {
    option_env!("SOURCE_DATE_EPOCH")
        .and_then(|s| s.trim().parse::<i64>().ok())
        .map_or(SOURCE_FLOOR_S, |e| e.max(SOURCE_FLOOR_S))
}

/// What the schedule runs with ([`ServiceConfig::schedule`]).
#[derive(Debug, Clone, Default)]
pub struct ScheduleConfig {
    /// The time zone: an IANA name (`Europe/Stockholm`, from the system's database) or a
    /// POSIX TZ string (`CET-1CEST,M3.5.0,M10.5.0/3`). `None`: the hub machine's own.
    pub zone: Option<String>,
    /// The least wall-clock time believed, Unix seconds; `None`: [`build_floor_s`].
    pub not_before_s: Option<i64>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every section under these locks leaves its data whole before it could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn cut(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

/// ISO-8601 UTC with `Z`, from Unix seconds (none before the epoch).
fn iso(secs: i64) -> String {
    crate::inventory::iso8601(u64::try_from(secs).unwrap_or(0))
}

// ── The zone ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Zone {
    /// The machine's, read again on each look (jiff caches it and notices a change).
    System,
    Fixed {
        tz: TimeZone,
        name: String,
    },
}

impl Zone {
    fn new(name: Option<&str>) -> Self {
        let Some(name) = name else {
            return Self::System;
        };
        match TimeZone::get(name).or_else(|_| TimeZone::posix(name)) {
            Ok(tz) => Self::Fixed {
                tz,
                name: name.to_owned(),
            },
            Err(e) => {
                log::error!("hub: schedule time zone {name:?} unknown ({e}); using UTC");
                Self::Fixed {
                    tz: TimeZone::UTC,
                    name: "UTC".into(),
                }
            }
        }
    }

    fn tz(&self) -> TimeZone {
        match self {
            Self::System => TimeZone::system(),
            Self::Fixed { tz, .. } => tz.clone(),
        }
    }

    fn name(&self) -> String {
        match self {
            Self::System => TimeZone::system().iana_name().unwrap_or("local").to_owned(),
            Self::Fixed { name, .. } => name.clone(),
        }
    }
}

/// The local date at `secs`.
fn local_date(tz: &TimeZone, secs: i64) -> Option<Date> {
    Timestamp::from_second(secs)
        .ok()
        .map(|t| tz.to_datetime(t).date())
}

/// When the local `dt` happens in `tz`, Unix seconds: in a gap (spring forward) the
/// first valid instant after it; in a fold (fall back) the first of the two.
pub fn resolve(tz: &TimeZone, dt: DateTime) -> Option<i64> {
    let ts = match tz.to_ambiguous_timestamp(dt).offset() {
        AmbiguousOffset::Unambiguous { offset } => offset.to_timestamp(dt).ok()?,
        AmbiguousOffset::Fold { before, .. } => before.to_timestamp(dt).ok()?,
        AmbiguousOffset::Gap { before, .. } => {
            // Read with the offset before the gap, `dt` is past the transition; the
            // transition itself is the first minute that exists.
            let late = before.to_timestamp(dt).ok()?;
            tz.preceding(late).next()?.timestamp()
        }
    };
    Some(ts.as_second())
}

// ── Rules ───────────────────────────────────────────────────────────────────

/// When a rule runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum When {
    /// On these days (bit 0 Monday … bit 6 Sunday) at this local time.
    Weekly {
        /// The days.
        days: u8,
        /// The local time (whole minutes).
        time: Time,
    },
    /// Once, at this local date and time.
    Once {
        /// The local date and time (whole minutes).
        at: DateTime,
    },
}

fn hhmm(s: &str) -> Option<(i8, i8)> {
    let (h, m) = s.split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    let h: i8 = h.parse().ok()?;
    let m: i8 = m.parse().ok()?;
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some((h, m))
}

impl When {
    fn parse(v: &Value) -> Result<Self, String> {
        let o = v
            .as_object()
            .ok_or("when is {days, time} or {once}".to_owned())?;
        if let Some(once) = o.get("once") {
            let s = once
                .as_str()
                .ok_or("once is a local date and time, \"2026-12-24T17:00\"")?;
            let (date, time) = s
                .split_once('T')
                .ok_or("once is a local date and time, \"2026-12-24T17:00\"")?;
            let date: Date = date
                .parse()
                .map_err(|_| format!("once: {date:?} is not a date"))?;
            let (h, m) = hhmm(time).ok_or(format!("once: {time:?} is not HH:MM"))?;
            let time = Time::new(h, m, 0, 0).map_err(|e| e.to_string())?;
            return Ok(Self::Once {
                at: date.to_datetime(time),
            });
        }
        let days = o
            .get("days")
            .and_then(Value::as_array)
            .ok_or("when needs days (\"mon\" … \"sun\") and a time")?;
        let mut mask = 0u8;
        for d in days {
            let i = d
                .as_str()
                .and_then(|d| DAYS.iter().position(|x| x.eq_ignore_ascii_case(d)))
                .ok_or(format!("{d} is not a day (\"mon\" … \"sun\")"))?;
            mask |= 1u8.checked_shl(u32::try_from(i).unwrap_or(0)).unwrap_or(0);
        }
        if mask == 0 {
            return Err("when needs at least one day".into());
        }
        let time = o
            .get("time")
            .and_then(Value::as_str)
            .and_then(hhmm)
            .ok_or("time is a local HH:MM")?;
        let time = Time::new(time.0, time.1, 0, 0).map_err(|e| e.to_string())?;
        Ok(Self::Weekly { days: mask, time })
    }

    fn to_json(&self) -> Value {
        match self {
            Self::Weekly { days, time } => {
                let list: Vec<&str> = DAYS
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| days & 1u8.checked_shl(*i as u32).unwrap_or(0) != 0)
                    .map(|(_, d)| *d)
                    .collect();
                json!({"days": list, "time": format!("{:02}:{:02}", time.hour(), time.minute())})
            }
            Self::Once { at } => json!({"once": format!(
                "{}T{:02}:{:02}",
                at.date(),
                at.hour(),
                at.minute()
            )}),
        }
    }

    fn on(days: u8, date: Date) -> bool {
        let i = date.weekday().to_monday_zero_offset();
        days & 1u8.checked_shl(u32::try_from(i).unwrap_or(7)).unwrap_or(0) != 0
    }
}

/// One action of a rule: its `t` and the fields of that message it runs with.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    /// One of [`ACTIONS`].
    pub t: &'static str,
    /// Its fields, the device left out.
    pub fields: Map<String, Value>,
}

fn short_text(v: Option<&Value>, key: &str, max: usize) -> Result<Option<String>, String> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if !s.is_empty() && s.chars().count() <= max => Ok(Some(s.clone())),
        Some(_) => Err(format!("{key} is text of 1 to {max} characters")),
    }
}

impl Action {
    fn parse(v: &Value) -> Result<Self, String> {
        let o = v.as_object().ok_or("an action is an object with its t")?;
        let t = o
            .get("t")
            .and_then(Value::as_str)
            .ok_or("an action needs its t")?;
        let t = *ACTIONS.iter().find(|a| **a == t).ok_or(format!(
            "{t} is not an action a schedule runs ({})",
            ACTIONS.join(", ")
        ))?;
        let mut fields = Map::new();
        let mut put = |k: &str, v: Value| {
            fields.insert(k.to_owned(), v);
        };
        match t {
            "preset.load" => {
                let slot = match o.get("slot") {
                    None | Some(Value::Null) => None,
                    Some(s) => Some(
                        s.as_u64()
                            .filter(|s| *s < PRESET_SLOTS)
                            .ok_or("slot is 0 to 9")?,
                    ),
                };
                let name = short_text(o.get("name"), "name", 64)?;
                match (slot, name) {
                    (Some(s), _) => put("slot", json!(s)),
                    (None, Some(n)) => put("name", json!(n)),
                    (None, None) => return Err("preset.load needs a slot or a name".into()),
                }
            }
            "snapshot.recall" => {
                match (
                    short_text(o.get("snapshot"), "snapshot", 128)?,
                    short_text(o.get("name"), "name", 256)?,
                ) {
                    (Some(s), _) => put("snapshot", json!(s)),
                    (None, Some(n)) => put("name", json!(n)),
                    (None, None) => {
                        return Err("snapshot.recall needs a snapshot or a name".into());
                    }
                }
            }
            "system.scene.apply" => {
                let scene = short_text(o.get("scene"), "scene", 64)?
                    .ok_or("system.scene.apply needs its scene")?;
                put("scene", json!(scene));
            }
            // "param.set"
            _ => {
                let path = o.get("path").and_then(Value::as_str).unwrap_or_default();
                if !SCHEDULED_PATHS.contains(&path) {
                    return Err(format!(
                        "a schedule sets only {} (not {path:?})",
                        SCHEDULED_PATHS.join(", ")
                    ));
                }
                put("path", json!(path));
                match o.get("idx") {
                    None | Some(Value::Null) => {}
                    Some(Value::Array(idx))
                        if idx.len() <= 2
                            && idx.iter().all(|i| i.as_u64().is_some_and(|i| i < 256)) =>
                    {
                        put("idx", Value::Array(idx.clone()));
                    }
                    Some(_) => return Err("idx is a list of whole numbers".into()),
                }
                match o.get("value") {
                    Some(v @ (Value::Number(_) | Value::Bool(_))) => put("value", v.clone()),
                    _ => return Err("param.set needs a value (a number or true/false)".into()),
                }
            }
        }
        Ok(Self { t, fields })
    }

    fn to_json(&self) -> Value {
        let mut o = self.fields.clone();
        o.insert("t".into(), json!(self.t));
        Value::Object(o)
    }

    fn per_device(&self) -> bool {
        self.t != "system.scene.apply"
    }
}

/// A schedule rule.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// The hub's id for it (`r1`, `r2`, …).
    pub id: String,
    /// Its name (`Schedule: <name>` is who writes).
    pub name: String,
    /// Off: kept, never run on its schedule (`run_now` still runs it).
    pub enabled: bool,
    /// When.
    pub when: When,
    /// What, in order.
    pub actions: Vec<Action>,
    /// On which devices, in order.
    pub targets: Vec<String>,
}

/// A refusal: a code of hub-api.md 1.6 and a line for people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The code.
    pub code: &'static str,
    /// For people.
    pub msg: String,
}

impl Refusal {
    fn new(code: &'static str, msg: impl Into<String>) -> Self {
        Self {
            code,
            msg: msg.into(),
        }
    }

    fn bad(msg: impl Into<String>) -> Self {
        Self::new("bad_request", msg)
    }
}

impl Rule {
    /// A rule from a message's or the file's object, with `id`.
    pub fn parse(v: &Value, id: String) -> Result<Self, Refusal> {
        let o = v
            .as_object()
            .ok_or_else(|| Refusal::bad("rule is an object"))?;
        let name = o
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|n| !n.is_empty() && n.chars().count() <= MAX_NAME_CHARS)
            .filter(|n| !n.chars().any(char::is_control))
            .ok_or_else(|| {
                Refusal::bad(format!("name is text of 1 to {MAX_NAME_CHARS} characters"))
            })?
            .to_owned();
        let enabled = match o.get("enabled") {
            None | Some(Value::Null) => true,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(Refusal::bad("enabled is true or false")),
        };
        let when = When::parse(o.get("when").unwrap_or(&Value::Null)).map_err(Refusal::bad)?;
        let actions = o
            .get("actions")
            .and_then(Value::as_array)
            .filter(|a| !a.is_empty())
            .ok_or_else(|| Refusal::bad("actions is a list of 1 to 16 actions"))?;
        if actions.len() > MAX_ACTIONS {
            return Err(Refusal::bad(format!(
                "at most {MAX_ACTIONS} actions in a rule"
            )));
        }
        let actions = actions
            .iter()
            .enumerate()
            .map(|(i, a)| Action::parse(a).map_err(|e| Refusal::bad(format!("action {i}: {e}"))))
            .collect::<Result<Vec<_>, _>>()?;
        let targets = match o.get("targets") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(t)) => t
                .iter()
                .map(|t| t.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| Refusal::bad("targets is a list of serials"))?,
            Some(_) => return Err(Refusal::bad("targets is a list of serials")),
        };
        if targets.len() > MAX_TARGETS {
            return Err(Refusal::bad(format!(
                "at most {MAX_TARGETS} targets in a rule"
            )));
        }
        for t in &targets {
            if t.starts_with("group:") {
                return Err(Refusal::new(
                    "unsupported",
                    "group targets come with phase 3's groups",
                ));
            }
            if !crate::inventory::valid_serial(t) {
                return Err(Refusal::bad(format!("{t:?} is not a serial")));
            }
        }
        let mut unique = targets.clone();
        unique.sort();
        unique.dedup();
        if unique.len() != targets.len() {
            return Err(Refusal::bad("a target is named twice"));
        }
        if targets.is_empty() && actions.iter().any(Action::per_device) {
            return Err(Refusal::bad("the rule's device actions need targets"));
        }
        Ok(Self {
            id,
            name,
            enabled,
            when,
            actions,
            targets,
        })
    }

    /// Its object, as `schedule.set` takes it.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "enabled": self.enabled,
            "when": self.when.to_json(),
            "actions": self.actions.iter().map(Action::to_json).collect::<Vec<_>>(),
            "targets": self.targets,
        })
    }

    /// Who writes for it.
    pub fn principal(&self) -> String {
        format!("Schedule: {}", self.name)
    }
}

// ── The engine: what is due, by the clock rules ─────────────────────────────

/// What a rule last did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Last {
    /// When, Unix seconds: the run's start, or the skipped time.
    pub at: i64,
    /// Every action on every target went through.
    pub ok: bool,
    /// It was skipped (a forward jump, or missed by more than the grace).
    pub skipped: bool,
}

/// A rule's memory, kept in the file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleState {
    /// The local date of the last time it ran or was skipped: it never runs again on that
    /// date or before (a backward jump).
    pub last_date: Option<Date>,
    /// What it last did.
    pub last: Option<Last>,
}

/// Why a time was not run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipWhy {
    /// The wall clock jumped forward past it.
    Jump,
    /// It was missed by more than [`GRACE_S`] (the timer stalled).
    Missed,
}

/// What a look at the clock found due.
#[derive(Debug, Clone, PartialEq)]
pub enum Due {
    /// Run the rule now; `slot` is the time it was due.
    Run {
        /// The rule as it is now.
        rule: Rule,
        /// When it was due, Unix seconds.
        slot: i64,
    },
    /// The rule's times were passed without running.
    Skip {
        /// The rule.
        rule: Rule,
        /// The last such time, Unix seconds.
        slot: i64,
        /// How many (in the last week looked at).
        count: u32,
        /// Why.
        why: SkipWhy,
    },
}

/// What [`Engine::tick`] found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tick {
    /// What is due, in order.
    pub due: Vec<Due>,
    /// `Some(true)` when the clock just became unset, `Some(false)` when it just became set.
    pub clock_unset: Option<bool>,
    /// The wall clock jumped by this many seconds more (or less) than time passed.
    pub jump_s: Option<i64>,
}

/// The rules and their clock, without threads or files.
#[derive(Debug, Clone)]
pub struct Engine {
    rules: Vec<(Rule, RuleState)>,
    next_id: u64,
    /// The wall time and the monotonic time of the last look; `None` before the first,
    /// and while the clock is unset.
    checked: Option<(i64, Instant)>,
    clock_unset: bool,
}

fn occurrences(
    when: &When,
    last: Option<Date>,
    tz: &TimeZone,
    lo: i64,
    hi: i64,
) -> Vec<(i64, Date)> {
    let mut out = Vec::new();
    let lo = lo.max(hi.saturating_sub(WINDOW_S));
    if hi <= lo {
        return out;
    }
    match when {
        When::Once { at } => {
            if last.is_none()
                && let Some(ts) = resolve(tz, *at)
                && ts > lo
                && ts <= hi
            {
                out.push((ts, at.date()));
            }
        }
        When::Weekly { days, time } => {
            let first = local_date(tz, lo).and_then(|d| d.yesterday().ok());
            let end = local_date(tz, hi).and_then(|d| d.tomorrow().ok());
            let (Some(mut d), Some(end)) = (first, end) else {
                return out;
            };
            // At most the window's eight days and the two around them.
            for _ in 0..12 {
                if d > end {
                    break;
                }
                if When::on(*days, d)
                    && last.is_none_or(|l| d > l)
                    && let Some(ts) = resolve(tz, d.to_datetime(*time))
                    && ts > lo
                    && ts <= hi
                {
                    out.push((ts, d));
                }
                match d.tomorrow() {
                    Ok(n) => d = n,
                    Err(_) => break,
                }
            }
        }
    }
    out
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(Vec::new(), 1)
    }
}

fn secs_between(later: Instant, earlier: Instant) -> i64 {
    i64::try_from(later.saturating_duration_since(earlier).as_secs()).unwrap_or(i64::MAX)
}

impl Engine {
    /// An engine with these rules and their memory (the file's).
    pub fn new(rules: Vec<(Rule, RuleState)>, next_id: u64) -> Self {
        let next_id = rules
            .iter()
            .filter_map(|(r, _)| r.id.strip_prefix('r')?.parse::<u64>().ok())
            .map(|n| n.saturating_add(1))
            .max()
            .unwrap_or(1)
            .max(next_id);
        Self {
            rules,
            next_id,
            checked: None,
            clock_unset: false,
        }
    }

    /// The rules and their memory.
    pub fn rules(&self) -> &[(Rule, RuleState)] {
        &self.rules
    }

    /// The id the next new rule gets.
    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    /// Whether the clock read before the build date at the last look.
    pub fn clock_unset(&self) -> bool {
        self.clock_unset
    }

    /// The rule `id`.
    pub fn get(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|(r, _)| r.id == id).map(|(r, _)| r)
    }

    /// Add (`id: None`) or replace a rule. A rule whose `when` changed forgets its last
    /// date, so its new time runs today if it is still ahead.
    pub fn set(&mut self, id: Option<&str>, v: &Value) -> Result<Rule, Refusal> {
        match id {
            Some(id) => {
                let i = self
                    .rules
                    .iter()
                    .position(|(r, _)| r.id == id)
                    .ok_or_else(|| Refusal::new("not_found", format!("no rule {id}")))?;
                let rule = Rule::parse(v, id.to_owned())?;
                let Some((old, state)) = self.rules.get_mut(i) else {
                    return Err(Refusal::new("not_found", format!("no rule {id}")));
                };
                if old.when != rule.when {
                    state.last_date = None;
                }
                *old = rule.clone();
                Ok(rule)
            }
            None => {
                if self.rules.len() >= MAX_RULES {
                    return Err(Refusal::new(
                        "conflict",
                        format!("the hub holds {MAX_RULES} rules, its most"),
                    ));
                }
                let rule = Rule::parse(v, format!("r{}", self.next_id))?;
                self.next_id = self.next_id.saturating_add(1);
                self.rules.push((rule.clone(), RuleState::default()));
                Ok(rule)
            }
        }
    }

    /// Remove the rule `id`.
    pub fn delete(&mut self, id: &str) -> Result<Rule, Refusal> {
        let i = self
            .rules
            .iter()
            .position(|(r, _)| r.id == id)
            .ok_or_else(|| Refusal::new("not_found", format!("no rule {id}")))?;
        Ok(self.rules.remove(i).0)
    }

    /// Note what a run of `id` did (it may have been deleted meanwhile).
    pub fn ran(&mut self, id: &str, last: Last) {
        if let Some((_, s)) = self.rules.iter_mut().find(|(r, _)| r.id == id) {
            s.last = Some(last);
        }
    }

    /// Look at the clock: `wall_s` the wall clock (Unix seconds), `mono` the monotonic
    /// one, `tz` the zone, `floor_s` the least believable wall time. Marks what it
    /// answers as handled (a rule's last date), so a time is answered once.
    pub fn tick(&mut self, wall_s: i64, mono: Instant, tz: &TimeZone, floor_s: i64) -> Tick {
        let mut out = Tick::default();
        if wall_s < floor_s {
            if !self.clock_unset {
                self.clock_unset = true;
                out.clock_unset = Some(true);
            }
            self.checked = None;
            return out;
        }
        if self.clock_unset {
            self.clock_unset = false;
            out.clock_unset = Some(false);
        }
        // (lo, wall_s] is looked at; up to `normal` is time that passed, after it a jump.
        let (lo, normal) = match self.checked {
            None => (wall_s.saturating_sub(GRACE_S), wall_s),
            Some((w, m)) => {
                let dm = secs_between(mono, m);
                let dw = wall_s.saturating_sub(w);
                if dw < 0 {
                    // Back: nothing runs again (each rule's last date), nothing is due.
                    out.jump_s = Some(dw.saturating_sub(dm));
                    self.checked = Some((wall_s, mono));
                    return out;
                }
                if dw > dm.saturating_add(JUMP_SLACK_S) {
                    out.jump_s = Some(dw.saturating_sub(dm));
                    (w, w.saturating_add(dm))
                } else {
                    (w, wall_s)
                }
            }
        };
        self.checked = Some((wall_s, mono));
        for (rule, state) in self.rules.iter_mut().filter(|(r, _)| r.enabled) {
            let mut skip: Option<(i64, u32, SkipWhy)> = None;
            let mut run = None;
            for (ts, date) in occurrences(&rule.when, state.last_date, tz, lo, wall_s) {
                state.last_date = Some(date);
                if ts > normal {
                    let n = skip.map_or(0, |s| s.1).saturating_add(1);
                    skip = Some((ts, n, SkipWhy::Jump));
                } else if wall_s.saturating_sub(ts) > GRACE_S {
                    let n = skip.map_or(0, |s| s.1).saturating_add(1);
                    skip = Some((ts, n, skip.map_or(SkipWhy::Missed, |s| s.2)));
                } else {
                    run = Some(ts);
                }
            }
            if let Some((slot, count, why)) = skip {
                if run.is_none_or(|r| r < slot) {
                    state.last = Some(Last {
                        at: slot,
                        ok: false,
                        skipped: true,
                    });
                }
                out.due.push(Due::Skip {
                    rule: rule.clone(),
                    slot,
                    count,
                    why,
                });
            }
            if let Some(slot) = run {
                out.due.push(Due::Run {
                    rule: rule.clone(),
                    slot,
                });
            }
        }
        out
    }

    /// When `rule` runs next after `after_s` (Unix seconds), within a week and a day.
    pub fn next_run(&self, id: &str, tz: &TimeZone, after_s: i64) -> Option<i64> {
        let (rule, state) = self.rules.iter().find(|(r, _)| r.id == id)?;
        if !rule.enabled {
            return None;
        }
        occurrences(
            &rule.when,
            state.last_date,
            tz,
            after_s,
            after_s.saturating_add(WINDOW_S),
        )
        .first()
        .map(|o| o.0)
    }

    /// The earliest next run of any rule after the last look; `None` with none, or
    /// before the first look.
    pub fn next_due_wall(&self, tz: &TimeZone) -> Option<i64> {
        let (w, _) = self.checked?;
        self.rules
            .iter()
            .filter_map(|(r, _)| self.next_run(&r.id, tz, w))
            .min()
    }

    /// The wall and monotonic time of the last look.
    pub fn checked(&self) -> Option<(i64, Instant)> {
        self.checked
    }
}

// ── The file ────────────────────────────────────────────────────────────────

fn state_json(s: &RuleState) -> Value {
    json!({
        "last_date": s.last_date.map(|d| d.to_string()),
        "last": s.last.map(last_json),
    })
}

fn last_json(l: Last) -> Value {
    let mut v = json!({"at": iso(l.at), "ok": l.ok});
    if l.skipped
        && let Some(o) = v.as_object_mut()
    {
        o.insert("skipped".into(), json!(true));
    }
    v
}

fn parse_iso(s: &str) -> Option<i64> {
    s.parse::<Timestamp>().ok().map(Timestamp::as_second)
}

fn state_from(v: Option<&Value>) -> RuleState {
    let Some(v) = v else {
        return RuleState::default();
    };
    let last_date = v
        .get("last_date")
        .and_then(Value::as_str)
        .and_then(|d| d.parse::<Date>().ok());
    let last = v.get("last").and_then(|l| {
        Some(Last {
            at: parse_iso(l.get("at")?.as_str()?)?,
            ok: l.get("ok")?.as_bool()?,
            skipped: l.get("skipped").and_then(Value::as_bool).unwrap_or(false),
        })
    });
    RuleState { last_date, last }
}

/// The file's text for `engine`.
pub fn to_file(engine: &Engine) -> String {
    let rules: Vec<Value> = engine
        .rules()
        .iter()
        .map(|(r, s)| {
            let mut v = r.to_json();
            if let Some(o) = v.as_object_mut() {
                o.insert("state".into(), state_json(s));
            }
            v
        })
        .collect();
    let mut text = String::from("{\"v\":1,\"next_id\":");
    text.push_str(&engine.next_id().to_string());
    text.push_str(",\"rules\":[");
    for (i, r) in rules.iter().enumerate() {
        text.push_str(if i == 0 { "\n" } else { ",\n" });
        text.push_str(&r.to_string());
    }
    text.push_str("\n]}\n");
    text
}

/// An engine from the file's text, read entry by entry: a rule that does not read is
/// left out. Answers whether every entry read (else the file is kept aside).
pub fn from_file(text: &str) -> (Engine, bool) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return (Engine::default(), false);
    };
    let Some(list) = v.get("rules").and_then(Value::as_array) else {
        return (Engine::default(), false);
    };
    let next_id = v.get("next_id").and_then(Value::as_u64).unwrap_or(1);
    let mut whole = true;
    let mut rules: Vec<(Rule, RuleState)> = Vec::new();
    for entry in list {
        let id = entry.get("id").and_then(Value::as_str).filter(|id| {
            !id.is_empty() && id.len() <= 16 && !rules.iter().any(|(r, _)| r.id == *id)
        });
        let parsed = id.map(|id| Rule::parse(entry, id.to_owned()));
        match parsed {
            Some(Ok(rule)) if rules.len() < MAX_RULES => {
                rules.push((rule, state_from(entry.get("state"))));
            }
            _ => whole = false,
        }
    }
    (Engine::new(rules, next_id), whole)
}

/// Write `text` to `path` atomically (a temporary file, then a rename), 0600 on Unix.
fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    {
        use std::io::Write;
        let mut f = o.open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// The engine from `path`: empty when there is none; a file that does not read whole is
/// kept aside as `schedule.json.bad-<time>` with what could be read kept. Answers the
/// engine and whether it must be written again.
fn load(path: &Path, now_s: u64) -> (Engine, bool) {
    let mut bytes = Vec::new();
    let read = std::fs::File::open(path).and_then(|f| {
        f.take(MAX_FILE_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
    });
    match read {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return (Engine::default(), false),
        Err(e) => {
            log::error!("hub: schedule {} unread ({e}); no rules", path.display());
            set_aside(path, now_s);
            return (Engine::default(), true);
        }
    }
    let too_large = u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES;
    let (engine, whole) = if too_large {
        (Engine::default(), false)
    } else {
        from_file(&String::from_utf8_lossy(&bytes))
    };
    if whole {
        return (engine, false);
    }
    log::error!(
        "hub: schedule {} did not read whole; kept aside, {} rule(s) recovered",
        path.display(),
        engine.rules().len()
    );
    set_aside(path, now_s);
    (engine, true)
}

fn set_aside(path: &Path, now_s: u64) {
    let mut aside = path.as_os_str().to_owned();
    aside.push(format!(".bad-{now_s}"));
    if let Err(e) = std::fs::rename(path, PathBuf::from(&aside)) {
        log::error!("hub: cannot keep {} aside: {e}", path.display());
    }
}

// ── The service ─────────────────────────────────────────────────────────────

/// The service, when [`ServiceConfig::schedule`] asks for it (the one `registered`
/// line). It runs actions through the parameter API's executor, so it needs feature
/// `api`; without it the schedule is off.
pub(crate) fn service(
    config: &ServiceConfig,
    setup: &ServiceSetup<'_>,
) -> Option<Arc<dyn HubService>> {
    let schedule = config.schedule.as_ref()?;
    #[cfg(feature = "api")]
    {
        Some(Arc::new(ScheduleService::new(schedule, setup)) as Arc<dyn HubService>)
    }
    #[cfg(not(feature = "api"))]
    {
        let _ = (schedule, setup);
        log::info!("hub: schedules need the parameter API (feature `api`); they are off");
        None
    }
}

#[cfg(feature = "api")]
use service_impl::ScheduleService;

#[cfg(feature = "api")]
mod service_impl {
    use super::*;
    use crate::api::ApiError;
    use crate::api::exec::Executor;
    use crate::snapshots::HubSnapshots;

    enum Job {
        Run(Rule),
        Skip {
            rule: Rule,
            slot: i64,
            count: u32,
            why: SkipWhy,
        },
    }

    struct State {
        engine: Engine,
        /// When the timer looks next (the hub's monotonic clock).
        next_look: Instant,
        /// The file is behind the engine.
        dirty: bool,
    }

    #[derive(Default)]
    struct Jobs {
        queue: VecDeque<Job>,
        running: bool,
        stopping: bool,
    }

    pub(super) struct Shared {
        state: Mutex<State>,
        jobs: Mutex<Jobs>,
        ready: Condvar,
        exec: Executor,
        snaps: Option<HubSnapshots>,
        clock: Arc<dyn HubClock>,
        bus: Arc<Bus>,
        wake: Waker,
        zone: Zone,
        floor_s: i64,
        file: Option<PathBuf>,
        /// The file's own writes, one at a time.
        saving: Mutex<()>,
    }

    /// `schedule.*` as a hub service.
    pub(crate) struct ScheduleService(Arc<Shared>);

    fn row(serial: &str, action: &str, r: Result<Value, ApiError>) -> TargetResult {
        match r {
            Ok(_) => TargetResult {
                serial: serial.to_owned(),
                action: Some(action.to_owned()),
                ok: true,
                code: None,
                msg: None,
            },
            Err(e) => TargetResult {
                serial: serial.to_owned(),
                action: Some(action.to_owned()),
                ok: false,
                code: Some(e.code.to_owned()),
                msg: Some(cut(&e.msg, MAX_MSG_CHARS)),
            },
        }
    }

    fn reply(id: &Value, r: Result<Value, Refusal>) -> String {
        let mut out = match &r {
            Ok(_) => json!({"t": "ok", "id": id, "v": VERSION}),
            Err(e) => json!({"t": "err", "id": id, "v": VERSION, "code": e.code, "msg": e.msg}),
        };
        if let (Ok(Value::Object(body)), Some(map)) = (r, out.as_object_mut()) {
            map.extend(body);
        }
        out.to_string()
    }

    /// A scene's writes as rows, one per device (ok when every write of it was).
    fn scene_rows(action: &str, report: &crate::system::SceneReport) -> Vec<TargetResult> {
        const OK: [&str; 7] = [
            "ok",
            "unchanged",
            "queued",
            "applied",
            "applied_now",
            "late",
            "queued_unverified",
        ];
        let mut rows: Vec<TargetResult> = Vec::new();
        for w in &report.writes {
            let ok = OK.contains(&w.result.as_str());
            let i = match rows.iter().position(|r| r.serial == w.serial) {
                Some(i) => i,
                None => {
                    rows.push(TargetResult {
                        serial: w.serial.clone(),
                        action: Some(action.to_owned()),
                        ok: true,
                        code: None,
                        msg: None,
                    });
                    rows.len().saturating_sub(1)
                }
            };
            if let Some(r) = rows.get_mut(i)
                && r.ok
                && !ok
            {
                r.ok = false;
                r.code = Some(cut(&w.result, 32));
                r.msg = Some(cut(
                    &format!("{}: {}", w.what, w.reason.as_deref().unwrap_or(&w.result)),
                    MAX_MSG_CHARS,
                ));
            }
        }
        if rows.is_empty() {
            rows.push(TargetResult {
                serial: String::new(),
                action: Some(action.to_owned()),
                ok: true,
                code: None,
                msg: Some("nothing to write".into()),
            });
        }
        rows
    }

    impl ScheduleService {
        pub(super) fn new(config: &ScheduleConfig, setup: &ServiceSetup<'_>) -> Self {
            let clock = Arc::clone(setup.clock);
            let file = setup.data_dir.map(|d| d.join(FILE_NAME));
            let (engine, dirty) = match &file {
                Some(f) => load(f, clock.unix_seconds()),
                None => (Engine::default(), false),
            };
            let now = clock.now();
            Self(Arc::new(Shared {
                state: Mutex::new(State {
                    engine,
                    next_look: now,
                    dirty,
                }),
                jobs: Mutex::new(Jobs::default()),
                ready: Condvar::new(),
                exec: Executor::new(Arc::clone(&clock), setup.wake.clone()),
                snaps: setup.data_dir.map(HubSnapshots::open),
                clock,
                bus: Arc::clone(setup.bus),
                wake: setup.wake.clone(),
                zone: Zone::new(config.zone.as_deref()),
                floor_s: config.not_before_s.unwrap_or_else(build_floor_s),
                file,
                saving: Mutex::new(()),
            }))
        }
    }

    impl Shared {
        fn wall_s(&self) -> i64 {
            i64::try_from(self.clock.unix_seconds()).unwrap_or(i64::MAX)
        }

        /// Write the file if it is behind. Not on the timer thread.
        fn save(&self) {
            let Some(file) = &self.file else {
                lock(&self.state).dirty = false;
                return;
            };
            let _one = lock(&self.saving);
            let text = {
                let mut st = lock(&self.state);
                if !st.dirty {
                    return;
                }
                st.dirty = false;
                to_file(&st.engine)
            };
            if let Err(e) = write_atomic(file, &text) {
                log::error!("hub: schedule {} not saved: {e}", file.display());
                lock(&self.state).dirty = true;
            }
        }

        fn rule_json(&self, engine: &Engine, rule: &Rule, tz: &TimeZone, now_s: i64) -> Value {
            let mut v = rule.to_json();
            let state = engine
                .rules()
                .iter()
                .find(|(r, _)| r.id == rule.id)
                .map(|(_, s)| s.clone())
                .unwrap_or_default();
            let after = engine.checked().map_or(now_s, |c| c.0.max(now_s));
            let next = if engine.clock_unset() {
                None
            } else {
                engine.next_run(&rule.id, tz, after)
            };
            if let Some(o) = v.as_object_mut() {
                o.insert(
                    "next_run".into(),
                    next.map_or(Value::Null, |n| json!(iso(n))),
                );
                o.insert("last".into(), state.last.map_or(Value::Null, last_json));
            }
            v
        }

        fn list(&self) -> Value {
            let tz = self.zone.tz();
            let now_s = self.wall_s();
            let st = lock(&self.state);
            let rules: Vec<Value> = st
                .engine
                .rules()
                .iter()
                .map(|(r, _)| self.rule_json(&st.engine, r, &tz, now_s))
                .collect();
            let local = Timestamp::from_second(now_s)
                .ok()
                .map(|t| tz.to_datetime(t))
                .map(|dt| format!("{}T{:02}:{:02}", dt.date(), dt.hour(), dt.minute()));
            json!({
                "rules": rules,
                "zone": self.zone.name(),
                "local_time": local,
                "clock_unset": st.engine.clock_unset() || now_s < self.floor_s,
            })
        }

        fn edit(&self, msg: &Value, delete: bool) -> Result<Value, Refusal> {
            let tz = self.zone.tz();
            let now_s = self.wall_s();
            let answer = {
                let mut st = lock(&self.state);
                let answer = if delete {
                    let id = msg
                        .get("rule")
                        .and_then(Value::as_str)
                        .ok_or_else(|| Refusal::bad("rule is the rule's id"))?;
                    let gone = st.engine.delete(id)?;
                    json!({"rule": gone.id})
                } else {
                    let rule = msg
                        .get("rule")
                        .filter(|r| r.is_object())
                        .ok_or_else(|| Refusal::bad("rule is the rule's object"))?;
                    let id = match rule.get("id") {
                        None | Some(Value::Null) => None,
                        Some(Value::String(id)) => Some(id.as_str()),
                        Some(_) => return Err(Refusal::bad("a rule's id is text")),
                    };
                    let rule = st.engine.set(id, rule)?;
                    json!({"rule": self.rule_json(&st.engine, &rule, &tz, now_s)})
                };
                st.dirty = true;
                // The next run may be sooner now.
                st.next_look = self.clock.now();
                answer
            };
            self.save();
            self.wake.wake();
            Ok(answer)
        }

        /// Run `rule` as its principal; `by` the admin for `run_now`. Blocks for the
        /// transfers. Answers the run's time and results, after telling everyone.
        fn run(
            &self,
            hub: &HubHandle,
            rule: &Rule,
            by: Option<String>,
        ) -> (i64, Vec<TargetResult>) {
            let at = self.wall_s();
            let mut results = Vec::new();
            match hub.open_principal(Role::Admin, None, &rule.principal()) {
                Ok(principal) => {
                    // Nobody reads the principal's outbox: it hears nothing (the executor's
                    // child sessions get what they need through their own transports).
                    hub.inner.relay.remove_session(principal.id());
                    self.actions(hub, &principal, rule, &mut results);
                    hub.close_session(principal.id());
                    // Its child sessions' cores close on the executor's next sweep.
                    hub.wake_services();
                }
                Err(e) => {
                    log::warn!("hub: schedule {} could not run: {e}", rule.name);
                    for (i, a) in rule.actions.iter().enumerate() {
                        results.push(TargetResult {
                            serial: String::new(),
                            action: Some(format!("{i}:{}", a.t)),
                            ok: false,
                            code: Some("internal".into()),
                            msg: Some("the hub is stopping".into()),
                        });
                    }
                }
            }
            let ok = results.iter().all(|r| r.ok);
            {
                let mut st = lock(&self.state);
                st.engine.ran(
                    &rule.id,
                    Last {
                        at,
                        ok,
                        skipped: false,
                    },
                );
                st.dirty = true;
            }
            self.save();
            let event = json!({
                "t": "schedule.ran", "v": VERSION, "hub": hub.hub_id(),
                "rule": rule.id, "name": rule.name, "at": iso(at), "by": by,
                "results": results,
            });
            hub.inner.relay.broadcast_event(&event.to_string());
            self.bus.publish(BusMessage::ScheduleRan {
                rule: rule.id.clone(),
                name: rule.name.clone(),
                at: u64::try_from(at).unwrap_or(0),
                by,
                results: results.clone(),
            });
            (at, results)
        }

        fn actions(
            &self,
            hub: &HubHandle,
            principal: &Arc<Session>,
            rule: &Rule,
            results: &mut Vec<TargetResult>,
        ) {
            for (i, a) in rule.actions.iter().enumerate() {
                let label = format!("{i}:{}", a.t);
                if !a.per_device() {
                    let scene = a.fields.get("scene").and_then(Value::as_str).unwrap_or("");
                    match hub.system_apply_scene(scene, principal.role()) {
                        Ok(report) => results.extend(scene_rows(&label, &report)),
                        Err(e) => {
                            let code = match &e {
                                crate::system::SystemRefused::Denied(_) => "denied",
                                crate::system::SystemRefused::NoSender => "offline",
                                crate::system::SystemRefused::Scene(
                                    crate::system::scene::SceneRefused::NoScene,
                                ) => "not_found",
                                crate::system::SystemRefused::Scene(_) => "offline",
                            };
                            results.push(TargetResult {
                                serial: String::new(),
                                action: Some(label.clone()),
                                ok: false,
                                code: Some(code.into()),
                                msg: Some(cut(&e.to_string(), MAX_MSG_CHARS)),
                            });
                        }
                    }
                    continue;
                }
                for serial in &rule.targets {
                    let mut msg = a.fields.clone();
                    msg.insert("t".into(), json!(a.t));
                    msg.insert("serial".into(), json!(serial));
                    let r = crate::api::run_action(
                        &self.exec,
                        self.snaps.as_ref(),
                        hub,
                        principal,
                        a.t,
                        &Value::Object(msg),
                    );
                    results.push(row(serial, &label, r));
                }
            }
        }

        fn skipped(&self, rule: &Rule, slot: i64, count: u32, why: SkipWhy) {
            let tz = self.zone.tz();
            let local = Timestamp::from_second(slot)
                .ok()
                .map(|t| tz.to_datetime(t))
                .map_or_else(String::new, |dt| {
                    format!("{} {:02}:{:02}", dt.date(), dt.hour(), dt.minute())
                });
            let reason = match why {
                SkipWhy::Jump => "the hub's clock jumped forward past",
                SkipWhy::Missed => "missed by more than 5 minutes:",
            };
            let times = if count > 1 {
                format!(" ({count} times)")
            } else {
                String::new()
            };
            let text = format!("Schedule {} skipped{times}: {reason} {local}", rule.name);
            log::info!("hub: {text}");
            for serial in &rule.targets {
                let mut record = dspi_session::events::EventRecord::new(
                    dspi_session::events::EventKind::Schedule,
                    self.clock.unix_ms(),
                    text.clone(),
                );
                record.source = Some(rule.id.clone());
                record.by = Some(rule.principal());
                self.bus.publish(BusMessage::Event {
                    serial: serial.clone(),
                    record,
                });
            }
            self.save();
        }

        fn clock_alarm(&self, unset: bool) {
            let at = self.clock.unix_seconds();
            if unset {
                log::warn!("hub: the clock reads before the build date; schedules wait");
            } else {
                log::info!("hub: the clock is set; schedules run");
            }
            self.bus.publish(BusMessage::Alarm {
                id: CLOCK_ALARM_ID.into(),
                source: "device".into(),
                code: "clock_unset".into(),
                subject: "clock".into(),
                serial: None,
                severity: "warning".into(),
                state: if unset {
                    AlarmState::Raised
                } else {
                    AlarmState::Cleared
                },
                message:
                    "The hub's clock reads before its build date: schedules wait until it is set"
                        .into(),
                at,
                raised_at: at,
                cleared_at: (!unset).then_some(at),
                acknowledged_by: None,
                count: 1,
            });
        }

        fn worker(self: &Arc<Self>, hub: &HubHandle) {
            loop {
                let job = {
                    let mut jobs = lock(&self.jobs);
                    loop {
                        if jobs.stopping || hub.is_stopped() {
                            jobs.running = false;
                            return;
                        }
                        if let Some(job) = jobs.queue.pop_front() {
                            break job;
                        }
                        let (j, _) = self
                            .ready
                            .wait_timeout(jobs, Duration::from_secs(1))
                            .unwrap_or_else(PoisonError::into_inner);
                        jobs = j;
                    }
                };
                match job {
                    Job::Run(rule) => {
                        let _ = self.run(hub, &rule, None);
                    }
                    Job::Skip {
                        rule,
                        slot,
                        count,
                        why,
                    } => self.skipped(&rule, slot, count, why),
                }
            }
        }

        fn queue(self: &Arc<Self>, hub: &HubHandle, new: Vec<Job>) {
            if new.is_empty() {
                return;
            }
            let mut jobs = lock(&self.jobs);
            if jobs.stopping {
                return;
            }
            for job in new {
                if jobs.queue.len() >= MAX_PENDING {
                    if let Job::Run(rule) = &job {
                        log::error!(
                            "hub: schedule {} dropped: {MAX_PENDING} runs wait",
                            rule.name
                        );
                    }
                    continue;
                }
                jobs.queue.push_back(job);
            }
            if !jobs.running {
                let me = Arc::clone(self);
                let hub = hub.clone();
                match std::thread::Builder::new()
                    .name("hub-schedule".into())
                    .spawn(move || me.worker(&hub))
                {
                    Ok(_) => jobs.running = true,
                    Err(e) => log::error!("hub: the schedule's thread did not start: {e}"),
                }
            }
            drop(jobs);
            self.ready.notify_all();
        }

        /// The timer's look: decide, queue, and set the next look. Short: no I/O.
        fn look(self: &Arc<Self>, hub: &HubHandle, now: Instant) {
            let tz = self.zone.tz();
            let wall = self.wall_s();
            let tick = {
                let mut st = lock(&self.state);
                let tick = st.engine.tick(wall, now, &tz, self.floor_s);
                if tick
                    .due
                    .iter()
                    .any(|d| matches!(d, Due::Skip { .. } | Due::Run { .. }))
                {
                    st.dirty = true;
                }
                let next_wall = st.engine.next_due_wall(&tz);
                let wait = next_wall
                    .map(|n| {
                        Duration::from_secs(u64::try_from(n.saturating_sub(wall)).unwrap_or(0))
                    })
                    .map_or(TICK, |w| w.min(TICK));
                st.next_look = now.checked_add(wait).unwrap_or(now);
                tick
            };
            if let Some(j) = tick.jump_s {
                log::info!("hub: the wall clock jumped by {j} s; schedules follow it");
            }
            if let Some(unset) = tick.clock_unset {
                self.clock_alarm(unset);
            }
            let jobs: Vec<Job> = tick
                .due
                .into_iter()
                .map(|d| match d {
                    Due::Run { rule, .. } => Job::Run(rule),
                    Due::Skip {
                        rule,
                        slot,
                        count,
                        why,
                    } => Job::Skip {
                        rule,
                        slot,
                        count,
                        why,
                    },
                })
                .collect();
            self.queue(hub, jobs);
        }
    }

    impl HubService for ScheduleService {
        fn name(&self) -> &'static str {
            "schedule"
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
            let Some(id) = msg.get("id").filter(|id| id.is_u64() || id.is_i64()) else {
                return Some(reply(&Value::Null, Err(Refusal::bad("id is required"))));
            };
            if t != "schedule.list" && session.role() < Role::Admin {
                return Some(reply(
                    id,
                    Err(Refusal::new(
                        "denied",
                        "only an admin changes or runs schedules",
                    )),
                ));
            }
            let s = &self.0;
            let r = match t {
                "schedule.list" => Ok(s.list()),
                "schedule.set" => s.edit(msg, false),
                "schedule.delete" => s.edit(msg, true),
                // "schedule.run_now"
                _ => {
                    let rule = msg
                        .get("rule")
                        .and_then(Value::as_str)
                        .ok_or_else(|| Refusal::bad("rule is the rule's id"));
                    let unset = s.wall_s() < s.floor_s;
                    rule.and_then(|id| {
                        if unset {
                            return Err(Refusal::new(
                                "conflict",
                                "the hub's clock reads before its build date: nothing runs until it is set",
                            ));
                        }
                        lock(&s.state)
                            .engine
                            .get(id)
                            .cloned()
                            .ok_or_else(|| Refusal::new("not_found", format!("no rule {id}")))
                    })
                    .map(|rule| {
                        let by = hub.session_label(session.id()).by();
                        let (at, results) = s.run(hub, &rule, by.clone());
                        json!({"rule": rule.id, "name": rule.name, "at": iso(at), "by": by,
                               "results": results})
                    })
                }
            };
            Some(reply(id, r))
        }

        fn next_due(&self) -> Option<Instant> {
            let look = lock(&self.0.state).next_look;
            Some(self.0.exec.next_due().map_or(look, |e| e.min(look)))
        }

        fn run_due(&self, hub: &HubHandle, now: Instant) {
            if self.0.exec.next_due().is_some_and(|d| d <= now) {
                self.0.exec.sweep(hub, now);
            }
            if lock(&self.0.state).next_look <= now {
                self.0.look(hub, now);
            }
        }

        fn stop(&self) {
            lock(&self.0.jobs).stopping = true;
            self.0.ready.notify_all();
            self.0.exec.close_all();
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests_wp_p2sc;
