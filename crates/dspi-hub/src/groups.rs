//! Groups (WP P3GR, PLAN-P3 F3): named sets of inventory serials, kept in
//! `<data>/groups.json`.
//!
//! | Message | Role | Answer |
//! |---|---|---|
//! | `group.list {group?}` | every role | `ok {groups}` (the one group named; `not_found` when there is none) |
//! | `group.set {group: {id?, name, note?, members}}` | Admin | `ok {group, created}`: a new group without `id`, otherwise that group replaced whole |
//! | `group.delete {group}` | Admin | `ok {group}` (the id) |
//!
//! Field names follow hub-api.md 1.1: a request's `id` is its correlation id, and a field
//! naming a group is `group`; a group's own object (`{id, name, note, members}`) carries
//! `id`. Each change sends `group.changed {v, hub, group, change}` to every session:
//! the whole group as it is now (as it was, for a delete), `change` one of `set`,
//! `delete`, `forgot` (a member was forgotten).
//!
//! A group has a name, a note and members, nothing else: no role (OD-P3a) and no
//! nesting (a `group:` member is `bad_request`). Ids are the hub's (`g1`, `g2`, ...), never reused. A device
//! may be in many groups. `inventory.list` is unchanged (OD-P3b): the client joins.
//!
//! **Members are inventory serials.** `group.set` asks the inventory (`inventory.list`,
//! as the asking Admin's session) and answers `not_found` for a serial it does not
//! hold; a hub that runs without the inventory checks against the registry (the
//! devices attached or in their grace). When an Admin forgets a device
//! (`inventory.forget`, [`BusMessage::DeviceForgotten`]) it leaves every group it was
//! in, each with a `group.changed`.
//!
//! **Resolution.** Other families take `group:<id>` through [`GroupResolver`], handed
//! over [`crate::service::ServiceSetup::group_resolver`]: the schedule expands its
//! `group:<id>` targets at run time with [`expand`] (P3GR); the group writes (P3GP) and
//! bulk apply (P3BA) take `group` the same way. A multi-device run answers at most
//! [`MAX_RESULT_ROWS`] rows, the last one [`truncated_row`] when there was more.
//!
//! **Bounded.** [`MAX_GROUPS`] groups of [`MAX_MEMBERS`] members; a name is 1 to
//! [`MAX_NAME_CHARS`] characters, a note at most [`MAX_NOTE_CHARS`]. The file is read
//! entry by entry (one group per line, as the hub writes it): a torn or corrupt one is
//! kept aside as `groups.json.bad-<time>`, the groups that still read are kept and the
//! hub runs on, with a line in the log. Written atomically, 0600 on Unix. Nothing here
//! panics on the file or on a message.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::bus::{BusMessage, Subscription, TargetResult};
use crate::clock::HubClock;
use crate::hub::HubHandle;
use crate::inventory::valid_serial;
use crate::service::{HubService, ServiceConfig, ServiceSetup};
use crate::session::{Role, Session};

/// The file in the hub's data folder.
pub const FILE_NAME: &str = "groups.json";
/// The cap `hello.caps` announces.
pub const CAP: &str = "groups";
/// The event a change sends.
pub const CHANGED_EVENT: &str = "group.changed";
/// The messages this family answers.
pub const MESSAGES: [&str; 3] = ["group.list", "group.set", "group.delete"];
/// How a multi-device target names a group: `group:<id>`.
pub const TARGET_PREFIX: &str = "group:";
/// The most groups a hub keeps.
pub const MAX_GROUPS: usize = 64;
/// The most members of one group.
pub const MAX_MEMBERS: usize = 64;
/// The longest group name, in characters.
pub const MAX_NAME_CHARS: usize = 48;
/// The longest note, in characters.
pub const MAX_NOTE_CHARS: usize = 256;
/// The most `results` rows one run answers (PLAN-P3 section 2): the last says
/// "truncated" when there was more.
pub const MAX_RESULT_ROWS: usize = 256;
/// The longest group id taken in a request or a target, in bytes.
const MAX_ID_BYTES: usize = 24;
/// The largest file read; a larger one is kept aside (64 groups of 64 serials of 64
/// bytes is about 270 KB).
const MAX_FILE_BYTES: u64 = 1 << 20;
/// How often the service reads the bus when nothing woke it.
const TICK: Duration = Duration::from_secs(1);
/// The bus queue the service reads (forgotten devices only).
const BUS_CAPACITY: usize = 1024;
/// The inventory's pages read to learn its serials (1024 devices in pages of 500).
const MAX_INVENTORY_PAGES: usize = 8;
/// The answers' version.
const VERSION: u32 = 1;
/// The file format's version.
const FILE_VERSION: u32 = 1;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every section under this lock leaves the book whole before it could panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn cut(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

// ── The seam other families resolve groups through ──────────────────────────

/// How a family resolves `group:<id>` (the schedule, P3GP's group writes, P3BA's bulk
/// apply). Handed over [`crate::service::ServiceSetup::group_resolver`]; a test may
/// give its own.
pub trait GroupResolver: Send + Sync {
    /// The members of group `id` in group order, or `None` when the hub has no such
    /// group. At most [`MAX_MEMBERS`].
    fn members(&self, id: &str) -> Option<Vec<String>>;
}

/// One target of a multi-device run after [`expand`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expanded {
    /// A device to run on.
    Serial(String),
    /// A `group:<id>` the hub does not have (or a hub without groups): one row,
    /// `not_found`, with the target as its serial.
    Unknown(String),
}

/// The group id of a `group:<id>` target, if `target` is one.
pub fn group_id(target: &str) -> Option<&str> {
    target.strip_prefix(TARGET_PREFIX)
}

/// Whether `target` is a `group:<id>` a request may name: an id of 1 to 24 letters,
/// digits, `-` or `_`. Whether the group exists is a question for run time.
pub fn valid_group_target(target: &str) -> bool {
    group_id(target).is_some_and(valid_id_syntax)
}

fn valid_id_syntax(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_BYTES
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `targets` (serials and `group:<id>`s) as the devices to run on, in order: each group
/// by its members in group order, at the time of the call. A device named twice (by
/// itself and in a group, or in two groups) is run on once, at its first place.
pub fn expand(targets: &[String], resolver: Option<&dyn GroupResolver>) -> Vec<Expanded> {
    let mut out = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for t in targets {
        match group_id(t) {
            Some(id) => match resolver.and_then(|r| r.members(id)) {
                Some(members) => {
                    for m in members.into_iter().take(MAX_MEMBERS) {
                        if seen.insert(m.clone()) {
                            out.push(Expanded::Serial(m));
                        }
                    }
                }
                None => out.push(Expanded::Unknown(t.clone())),
            },
            None => {
                if seen.insert(t.clone()) {
                    out.push(Expanded::Serial(t.clone()));
                }
            }
        }
    }
    out
}

/// The row of a `group:<id>` the hub does not have.
pub fn unknown_row(target: &str, action: Option<String>) -> TargetResult {
    TargetResult {
        serial: target.to_owned(),
        action,
        ok: false,
        code: Some("not_found".into()),
        msg: Some("no such group".into()),
    }
}

/// The last row of a run that had more than [`MAX_RESULT_ROWS`] rows to give:
/// `bad_request`, "truncated". Nothing after it ran.
pub fn truncated_row(action: Option<String>) -> TargetResult {
    TargetResult {
        serial: String::new(),
        action,
        ok: false,
        code: Some("bad_request".into()),
        msg: Some(format!(
            "truncated: a run answers at most {MAX_RESULT_ROWS} rows; the rest did not run"
        )),
    }
}

// ── Records ─────────────────────────────────────────────────────────────────

/// One group, as the file keeps it and the messages show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    /// The hub's id: `g1`, `g2`, ...
    pub id: String,
    /// 1 to [`MAX_NAME_CHARS`] characters.
    pub name: String,
    /// At most [`MAX_NOTE_CHARS`] characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Inventory serials, each once, at most [`MAX_MEMBERS`].
    #[serde(default)]
    pub members: Vec<String>,
}

impl Group {
    /// The group as the messages show it.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "note": self.note,
            "members": self.members,
        })
    }

    /// A group read from the file, made to fit the bounds; `None` for one without an id
    /// or a name the hub takes.
    fn sanitized(mut self) -> Option<Self> {
        id_number(&self.id)?;
        self.name = cut(self.name.trim(), MAX_NAME_CHARS);
        if self.name.is_empty() || self.name.chars().any(char::is_control) {
            return None;
        }
        self.note = self
            .note
            .map(|n| cut(&n, MAX_NOTE_CHARS))
            .filter(|n| !n.is_empty());
        let mut seen = BTreeSet::new();
        self.members = self
            .members
            .into_iter()
            .filter(|m| valid_serial(m) && seen.insert(m.clone()))
            .take(MAX_MEMBERS)
            .collect();
        Some(self)
    }
}

/// The number of a hub id (`g12` is 12); `None` for anything else.
fn id_number(id: &str) -> Option<u64> {
    let digits = id.strip_prefix('g')?;
    if digits.is_empty() || digits.starts_with('0') || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// What an Admin sends with `group.set`, checked for its shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupEdit {
    /// The group to replace; `None` for a new one.
    pub id: Option<String>,
    /// The name.
    pub name: String,
    /// The note.
    pub note: Option<String>,
    /// The members, in order.
    pub members: Vec<String>,
}

/// Why the book refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GroupError {
    /// A malformed request or a value past a bound.
    #[error("{0}")]
    BadRequest(String),
    /// No such group, or a member that is not in the inventory.
    #[error("{0}")]
    NotFound(String),
    /// The hub holds [`MAX_GROUPS`] already.
    #[error("{0}")]
    Full(String),
}

impl GroupError {
    /// The error code of hub-api.md 1.6.
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::NotFound(_) => "not_found",
            Self::Full(_) => "conflict",
        }
    }
}

impl GroupEdit {
    /// The edit `msg` carries (`id?`, `name`, `note?`, `members`), checked whole.
    pub fn parse(msg: &Value) -> Result<Self, GroupError> {
        let bad = |m: &str| GroupError::BadRequest(m.to_owned());
        let id = match msg.get("id") {
            None | Some(Value::Null) => None,
            Some(Value::String(id)) if valid_id_syntax(id) => Some(id.clone()),
            Some(_) => return Err(bad("group is a group's id")),
        };
        let name = msg
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|n| !n.is_empty() && n.chars().count() <= MAX_NAME_CHARS)
            .ok_or_else(|| {
                GroupError::BadRequest(format!("name is text of 1 to {MAX_NAME_CHARS} characters"))
            })?;
        if name.chars().any(char::is_control) {
            return Err(bad("name has a control character"));
        }
        let note = match msg.get("note") {
            None | Some(Value::Null) => None,
            Some(Value::String(n)) => {
                if n.chars().count() > MAX_NOTE_CHARS {
                    return Err(GroupError::BadRequest(format!(
                        "note is over {MAX_NOTE_CHARS} characters"
                    )));
                }
                if n.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
                    return Err(bad("note has a control character"));
                }
                Some(n.clone()).filter(|n| !n.is_empty())
            }
            Some(_) => return Err(bad("note is text")),
        };
        let items = msg
            .get("members")
            .and_then(Value::as_array)
            .ok_or_else(|| bad("members is a list of serials"))?;
        if items.len() > MAX_MEMBERS {
            return Err(GroupError::BadRequest(format!(
                "at most {MAX_MEMBERS} members in a group"
            )));
        }
        let mut members: Vec<String> = Vec::with_capacity(items.len());
        for m in items {
            let Some(serial) = m.as_str().filter(|s| valid_serial(s)) else {
                return Err(bad("members is a list of serials"));
            };
            if serial.starts_with(TARGET_PREFIX) {
                return Err(bad("a member is a serial: groups do not nest"));
            }
            if members.iter().any(|x| x == serial) {
                return Err(GroupError::BadRequest(format!("{serial} is named twice")));
            }
            members.push(serial.to_owned());
        }
        Ok(Self {
            id,
            name: name.to_owned(),
            note,
            members,
        })
    }
}

/// The groups. Pure: no files, no hub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupBook {
    /// In the order they were made.
    groups: Vec<Group>,
    /// The next new group's number: ids are never reused.
    next_id: u64,
}

impl Default for GroupBook {
    fn default() -> Self {
        Self::new()
    }
}

impl GroupBook {
    /// An empty book.
    pub fn new() -> Self {
        Self {
            groups: Vec::new(),
            next_id: 1,
        }
    }

    /// Every group, in the order they were made.
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    /// How many it holds.
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// Whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// One group.
    pub fn get(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// The number the next new group takes.
    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    /// Whether `edit` may be made (the group exists, or there is room for a new one),
    /// before the members are looked up.
    pub fn check(&self, edit: &GroupEdit) -> Result<(), GroupError> {
        match &edit.id {
            Some(id) if self.get(id).is_none() => {
                Err(GroupError::NotFound(format!("no group {id}")))
            }
            Some(_) => Ok(()),
            None if self.groups.len() >= MAX_GROUPS => Err(GroupError::Full(format!(
                "the hub holds {MAX_GROUPS} groups already"
            ))),
            // -- WP P3GP -- ids are never reused: the last number is never given.
            None if self.next_id == u64::MAX => Err(GroupError::Full(
                "the hub has given every group id it has; no new group can be made".into(),
            )),
            None => Ok(()),
        }
    }

    /// Make or replace a group. The members are the caller's to have looked up.
    pub fn set(&mut self, edit: GroupEdit) -> Result<Group, GroupError> {
        self.check(&edit)?;
        if edit.members.len() > MAX_MEMBERS {
            return Err(GroupError::BadRequest(format!(
                "at most {MAX_MEMBERS} members in a group"
            )));
        }
        let id = match edit.id {
            Some(id) => id,
            None => {
                // -- WP P3GP -- `check` refused u64::MAX, so this never wraps or repeats.
                let n = self.next_id;
                self.next_id = n.checked_add(1).ok_or_else(|| {
                    GroupError::Full("the hub has given every group id it has".into())
                })?;
                format!("g{n}")
            }
        };
        let group = Group {
            id,
            name: edit.name,
            note: edit.note,
            members: edit.members,
        };
        match self.groups.iter_mut().find(|g| g.id == group.id) {
            Some(slot) => *slot = group.clone(),
            None => self.groups.push(group.clone()),
        }
        Ok(group)
    }

    /// Delete a group.
    pub fn delete(&mut self, id: &str) -> Result<Group, GroupError> {
        let i = self
            .groups
            .iter()
            .position(|g| g.id == id)
            .ok_or_else(|| GroupError::NotFound(format!("no group {id}")))?;
        Ok(self.groups.remove(i))
    }

    /// A device was forgotten: it leaves every group. The ids of the groups it left.
    pub fn forget(&mut self, serial: &str) -> Vec<String> {
        let mut changed = Vec::new();
        for g in &mut self.groups {
            let before = g.members.len();
            g.members.retain(|m| m != serial);
            if g.members.len() != before {
                changed.push(g.id.clone());
            }
        }
        changed
    }

    // ── The file ────────────────────────────────────────────────────────────

    /// The file's text: one JSON object, the header and each group on a line of its
    /// own, so a torn file still reads line by line.
    pub fn to_file(&self) -> String {
        let mut out = format!(
            "{{\"v\":{FILE_VERSION},\"next_id\":{},\"groups\":[\n",
            self.next_id
        );
        let mut first = true;
        for g in &self.groups {
            let Ok(line) = serde_json::to_string(g) else {
                continue;
            };
            if !first {
                out.push_str(",\n");
            }
            first = false;
            out.push_str(&line);
        }
        out.push_str("\n]}\n");
        out
    }

    /// The book from the file's text, and whether all of it read. A whole file reads as
    /// one object; otherwise each line that is a group is read on its own. Past
    /// [`MAX_GROUPS`] the later groups go. `next_id` is never below a kept group's.
    pub fn from_file(text: &str) -> (Self, bool) {
        let mut whole = false;
        let mut next_id: u64 = 1;
        let mut groups: Vec<Group> = Vec::new();
        let kept = |g: Group, groups: &mut Vec<Group>| -> bool {
            if groups.len() >= MAX_GROUPS || groups.iter().any(|x| x.id == g.id) {
                return false;
            }
            groups.push(g);
            true
        };
        if let Ok(v) = serde_json::from_str::<Value>(text)
            && let Some(list) = v.get("groups").and_then(Value::as_array)
        {
            whole = true;
            next_id = v.get("next_id").and_then(Value::as_u64).unwrap_or(1);
            for g in list {
                let ok = serde_json::from_value::<Group>(g.clone())
                    .ok()
                    .and_then(Group::sanitized)
                    .is_some_and(|g| kept(g, &mut groups));
                whole &= ok;
            }
        } else {
            for line in text.lines() {
                let line = line.trim().trim_end_matches(',');
                if line.starts_with("{\"v\"") {
                    // The header line: `{"v":1,"next_id":N,"groups":[`.
                    if let Ok(v) = serde_json::from_str::<Value>(&format!("{line}]}}"))
                        && let Some(n) = v.get("next_id").and_then(Value::as_u64)
                    {
                        next_id = n;
                    }
                    continue;
                }
                if !line.starts_with('{') {
                    continue;
                }
                if let Some(g) = serde_json::from_str::<Group>(line)
                    .ok()
                    .and_then(Group::sanitized)
                {
                    kept(g, &mut groups);
                }
            }
        }
        // -- WP P3GP -- capped at u64::MAX, which `check` never gives: a kept `g<MAX>`
        // (or a file's `next_id` of MAX) leaves the book unable to make a group rather
        // than give an id again.
        let past = groups
            .iter()
            .filter_map(|g| id_number(&g.id))
            .max()
            .map_or(1, |n| n.saturating_add(1));
        (
            Self {
                groups,
                next_id: next_id.max(past).max(1),
            },
            whole,
        )
    }
}

// ── Files ───────────────────────────────────────────────────────────────────

/// Write `text` to `path` atomically (a temporary file, then a rename), 0600 on Unix.
fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut o = fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    {
        use std::io::Write;
        let mut f = o.open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// The book from `path`: empty when there is none; a file that does not read whole is
/// kept aside as `groups.json.bad-<time>` and what could be read is kept. Returns the
/// book and whether it must be written again.
fn load(path: &Path, now_s: u64) -> (GroupBook, bool) {
    let mut bytes = Vec::new();
    let read = fs::File::open(path).and_then(|f| {
        f.take(MAX_FILE_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
    });
    match read {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return (GroupBook::new(), false),
        Err(e) => {
            log::error!(
                "hub: groups {} unread ({e}); starting empty",
                path.display()
            );
            set_aside(path, now_s);
            return (GroupBook::new(), true);
        }
    }
    let too_large = u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES;
    let (book, whole) = if too_large {
        (GroupBook::new(), false)
    } else {
        GroupBook::from_file(&String::from_utf8_lossy(&bytes))
    };
    if whole {
        return (book, false);
    }
    log::error!(
        "hub: groups {} did not read whole; kept aside, {} group(s) recovered",
        path.display(),
        book.len()
    );
    set_aside(path, now_s);
    (book, true)
}

fn set_aside(path: &Path, now_s: u64) {
    let mut aside = path.as_os_str().to_owned();
    aside.push(format!(".bad-{now_s}"));
    if let Err(e) = fs::rename(path, PathBuf::from(&aside)) {
        log::error!("hub: cannot keep {} aside: {e}", path.display());
    }
}

// ── The service ─────────────────────────────────────────────────────────────

struct State {
    book: GroupBook,
    /// The file is behind the book.
    dirty: bool,
    /// When the bus is read next without a wake.
    next_tick: Instant,
}

/// The `group.*` family as a hub service, and the hub's [`GroupResolver`].
pub struct GroupsService {
    state: Mutex<State>,
    sub: Subscription,
    clock: Arc<dyn HubClock>,
    file: Option<PathBuf>,
}

/// The service, if the configuration asks for it. Made before the others
/// (`Services::new`), so they get it as their resolver.
pub(crate) fn make(config: &ServiceConfig, setup: &ServiceSetup<'_>) -> Option<Arc<GroupsService>> {
    config.groups.then(|| Arc::new(GroupsService::new(setup)))
}

/// The service for the list (the one `registered` line).
pub(crate) fn service(setup: &ServiceSetup<'_>) -> Option<Arc<dyn HubService>> {
    setup
        .groups
        .as_ref()
        .map(|g| Arc::clone(g) as Arc<dyn HubService>)
}

fn reply_err(id: &Value, code: &str, msg: &str) -> String {
    json!({"t": "err", "id": id, "v": VERSION, "code": code, "msg": msg}).to_string()
}

fn reply_ok(id: &Value, body: Value) -> String {
    let mut v = json!({"t": "ok", "id": id, "v": VERSION});
    if let (Some(map), Value::Object(body)) = (v.as_object_mut(), body) {
        // The envelope is the request's: a body never names its `t`, `id` or `v`.
        for (k, val) in body {
            if !matches!(k.as_str(), "t" | "id" | "v") {
                map.insert(k, val);
            }
        }
    }
    v.to_string()
}

fn forgotten(m: &BusMessage) -> bool {
    matches!(m, BusMessage::DeviceForgotten { .. })
}

impl GroupsService {
    /// The service: its book from `<data>/groups.json` (none in process), and its bus
    /// subscription (forgotten devices).
    pub fn new(setup: &ServiceSetup<'_>) -> Self {
        let clock = Arc::clone(setup.clock);
        let file = setup.data_dir.map(|d| d.join(FILE_NAME));
        let (book, dirty) = match &file {
            Some(f) => load(f, clock.unix_seconds()),
            None => (GroupBook::new(), false),
        };
        Self {
            state: Mutex::new(State {
                book,
                dirty,
                next_tick: clock.now(),
            }),
            sub: setup
                .bus
                .subscribe_filtered("groups", BUS_CAPACITY, forgotten),
            clock,
            file,
        }
    }

    fn save(&self, st: &mut State) {
        if !st.dirty {
            return;
        }
        let Some(path) = &self.file else {
            st.dirty = false;
            return;
        };
        if let Some(dir) = path.parent()
            && let Err(e) = fs::create_dir_all(dir)
        {
            log::error!("hub: cannot make {}: {e}", dir.display());
            return;
        }
        match write_atomic(path, &st.book.to_file()) {
            Ok(()) => st.dirty = false,
            // Tried again on the next tick.
            Err(e) => log::error!("hub: cannot save {}: {e}", path.display()),
        }
    }

    /// `group.changed {v, hub, group, change}`: the whole group as it is now (as it was,
    /// for a delete); `change` is `set`, `delete` or `forgot`.
    fn changed_event(hub: &HubHandle, group: &Group, change: &str) -> String {
        json!({
            "t": CHANGED_EVENT,
            "v": VERSION,
            "hub": hub.hub_id(),
            "group": group.to_json(),
            "change": change,
        })
        .to_string()
    }

    /// Take in the forgotten devices, save, and tell everyone. With no hub lock held.
    fn process(&self, hub: &HubHandle) {
        let mut serials = Vec::new();
        while let Some(m) = self.sub.try_recv() {
            if let BusMessage::DeviceForgotten { serial } = &*m {
                serials.push(serial.clone());
            }
        }
        if self.sub.take_dropped() > 0 {
            // Only forgotten devices reach this queue: 1024 of them between two ticks
            // is not a hub this runs on. Said, not hidden.
            log::warn!("hub: groups missed forgotten devices on the bus");
        }
        let events = {
            let mut st = lock(&self.state);
            st.next_tick = self.clock.now().checked_add(TICK).unwrap_or(st.next_tick);
            let mut changed: Vec<String> = Vec::new();
            for serial in &serials {
                for id in st.book.forget(serial) {
                    if !changed.contains(&id) {
                        changed.push(id);
                    }
                }
            }
            if !changed.is_empty() {
                st.dirty = true;
                log::info!(
                    "hub: forgotten device(s) left group(s) {}",
                    changed.join(", ")
                );
            }
            self.save(&mut st);
            changed
                .iter()
                .filter_map(|id| st.book.get(id))
                .map(|g| Self::changed_event(hub, g, "forgot"))
                .collect::<Vec<_>>()
        };
        for e in events {
            hub.inner.relay.broadcast_event(&e);
        }
    }

    /// Every serial a member may be: the inventory's (asked as `session`), or, with no
    /// inventory on this hub, the registry's.
    fn known_serials(
        hub: &HubHandle,
        session: &Arc<Session>,
    ) -> Result<BTreeSet<String>, (&'static str, String)> {
        let mut known = BTreeSet::new();
        let mut offset: u64 = 0;
        for _ in 0..MAX_INVENTORY_PAGES {
            let ask = json!({"t": "inventory.list", "offset": offset, "limit": 500});
            let Some(text) = hub.service_request(session, &ask) else {
                // No inventory on this hub: the devices it knows now.
                return Ok(hub
                    .devices()
                    .into_iter()
                    .map(|d| d.identity.serial)
                    .collect());
            };
            let page: Value = serde_json::from_str(&text)
                .map_err(|_| ("internal", "the inventory's answer did not read".to_owned()))?;
            if page.get("t").and_then(Value::as_str) != Some("ok") {
                let code = match page.get("code").and_then(Value::as_str) {
                    Some("denied") => "denied",
                    _ => "internal",
                };
                return Err((code, "the inventory did not answer".to_owned()));
            }
            let rows = page
                .get("devices")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            for r in rows {
                if let Some(s) = r.get("serial").and_then(Value::as_str) {
                    known.insert(s.to_owned());
                }
            }
            match page.get("next_offset").and_then(Value::as_u64) {
                Some(next) if next > offset => offset = next,
                _ => return Ok(known),
            }
        }
        Ok(known)
    }

    fn list(&self, msg: &Value) -> Result<Value, (&'static str, String)> {
        let st = lock(&self.state);
        match msg.get("group") {
            None | Some(Value::Null) => Ok(json!({
                "groups": st.book.groups().iter().map(Group::to_json).collect::<Vec<_>>(),
            })),
            Some(Value::String(id)) => st
                .book
                .get(id)
                .map(|g| json!({"groups": [g.to_json()]}))
                .ok_or(("not_found", format!("no group {id}"))),
            Some(_) => Err(("bad_request", "group is a group's id".into())),
        }
    }

    fn set(
        &self,
        hub: &HubHandle,
        session: &Arc<Session>,
        msg: &Value,
    ) -> Result<Value, (&'static str, String)> {
        let err = |e: GroupError| (e.code(), e.to_string());
        let body = msg
            .get("group")
            .filter(|g| g.is_object())
            .ok_or(("bad_request", "group is the group's object".to_owned()))?;
        let edit = GroupEdit::parse(body).map_err(err)?;
        let created = edit.id.is_none();
        lock(&self.state).book.check(&edit).map_err(err)?;
        // The inventory is asked with no lock of ours held.
        let known = Self::known_serials(hub, session)?;
        if let Some(missing) = edit.members.iter().find(|m| !known.contains(*m)) {
            return Err(("not_found", format!("no device {missing} in the inventory")));
        }
        let (group, event) = {
            let mut st = lock(&self.state);
            let group = st.book.set(edit).map_err(err)?;
            st.dirty = true;
            self.save(&mut st);
            let event = Self::changed_event(hub, &group, "set");
            (group, event)
        };
        hub.inner.relay.broadcast_event(&event);
        Ok(json!({"group": group.to_json(), "created": created}))
    }

    fn delete(&self, hub: &HubHandle, msg: &Value) -> Result<Value, (&'static str, String)> {
        let Some(id) = msg.get("group").and_then(Value::as_str) else {
            return Err(("bad_request", "group is a group's id".into()));
        };
        let event = {
            let mut st = lock(&self.state);
            let gone = st.book.delete(id).map_err(|e| (e.code(), e.to_string()))?;
            st.dirty = true;
            self.save(&mut st);
            Self::changed_event(hub, &gone, "delete")
        };
        hub.inner.relay.broadcast_event(&event);
        Ok(json!({"group": id}))
    }
}

impl GroupResolver for GroupsService {
    fn members(&self, id: &str) -> Option<Vec<String>> {
        lock(&self.state).book.get(id).map(|g| g.members.clone())
    }
}

impl HubService for GroupsService {
    fn name(&self) -> &'static str {
        "groups"
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
        if t != "group.list" && session.role() < Role::Admin {
            return Some(reply_err(&id, "denied", "an admin's"));
        }
        // Answer from a book that has seen every forgotten device up to now.
        self.process(hub);
        let answer = match t {
            "group.list" => self.list(msg),
            "group.set" => self.set(hub, session, msg),
            _ => self.delete(hub, msg),
        };
        Some(match answer {
            Ok(body) => reply_ok(&id, body),
            Err((code, text)) => reply_err(&id, code, &text),
        })
    }

    fn next_due(&self) -> Option<Instant> {
        let st = lock(&self.state);
        if st.dirty || !self.sub.is_empty() {
            Some(self.clock.now().min(st.next_tick))
        } else {
            Some(st.next_tick)
        }
    }

    fn run_due(&self, hub: &HubHandle, _now: Instant) {
        self.process(hub);
    }

    fn stop(&self) {
        let mut st = lock(&self.state);
        self.save(&mut st);
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p3gr;

// -- WP P3GP --
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests_wp_p3gp {
    use super::*;

    fn edit(name: &str) -> GroupEdit {
        GroupEdit {
            id: None,
            name: name.into(),
            note: None,
            members: Vec::new(),
        }
    }

    #[test]
    fn wp_p3gp_the_last_group_id_is_never_given_so_none_is_reused() {
        let text = format!(
            "{{\"v\":1,\"next_id\":{},\"groups\":[\n{{\"id\":\"g7\",\"name\":\"Hall\"}}\n]}}\n",
            u64::MAX - 1
        );
        let (mut book, whole) = GroupBook::from_file(&text);
        assert!(whole);
        let g = book.set(edit("Last")).unwrap();
        assert_eq!(g.id, format!("g{}", u64::MAX - 1));
        assert_eq!(book.next_id(), u64::MAX);
        let e = book.set(edit("One more")).unwrap_err();
        assert_eq!(e.code(), "conflict", "{e}");
        assert_eq!(book.len(), 2, "nothing made");
        // A group may still be replaced.
        let again = GroupEdit {
            id: Some("g7".into()),
            ..edit("Hall 2")
        };
        assert_eq!(book.set(again).unwrap().name, "Hall 2");
        // Read back, the book stays where it was.
        let (book, _) = GroupBook::from_file(&book.to_file());
        assert_eq!(book.next_id(), u64::MAX);
        assert_eq!(book.clone().set(edit("x")).unwrap_err().code(), "conflict");
    }

    #[test]
    fn wp_p3gp_a_kept_group_at_the_last_id_caps_next_id() {
        let text = format!(
            "{{\"v\":1,\"next_id\":3,\"groups\":[\n{{\"id\":\"g{}\",\"name\":\"Top\"}}\n]}}\n",
            u64::MAX
        );
        let (mut book, _) = GroupBook::from_file(&text);
        assert_eq!(book.next_id(), u64::MAX);
        assert_eq!(book.set(edit("x")).unwrap_err().code(), "conflict");
        // A torn file's line reading gives the same.
        let torn = text.replace("\n]}\n", "");
        let (mut book, _) = GroupBook::from_file(&torn);
        assert_eq!(book.next_id(), u64::MAX);
        assert_eq!(book.set(edit("x")).unwrap_err().code(), "conflict");
    }

    #[test]
    fn wp_p3gp_a_group_as_a_member_says_groups_do_not_nest() {
        let e = GroupEdit::parse(&json!({"name": "Bar", "members": ["group:g1"]})).unwrap_err();
        assert_eq!(e.code(), "bad_request");
        assert!(e.to_string().contains("groups do not nest"), "{e}");
    }
}
