//! Confirmations (WP MCP2a): a tool call the policy holds for the user's
//! word, for up to [`CONFIRM_HOLD_MS`].
//!
//! The server opens one when a call's writes need asking
//! (`policy::Judging::asked`); nothing has been sent then. The app hears of
//! it as a [`ConfirmEvent`] (`Server::take_confirm_events`), shows the
//! sheet from [`Confirmation::to_json`], and answers with
//! `Server::answer_confirmation`. **Allow** runs the call again at once with
//! a grant for exactly the writes the user saw, on the device the sheet named
//! (-- WP RBFIX --); anything else it would write is judged afresh. **Deny**,
//! no answer within the hold, the agent gone or that device closed end it with
//! nothing changed.
//!
//! The agent gets the outcome one of two ways: a transport that can hold a
//! request (`Server::handle_held`) answers it once the confirmation ends
//! (`Server::resume`); any other gets "awaiting confirmation" at once and
//! asks `dspi_get_confirmation`.
//!
//! -- WP UB5 -- (UXP B5, D2) One agent's questions raised together are one
//! sheet: a group. A question joins its agent's open group while that group's
//! newest question is less than [`GROUP_QUIET_MS`] old and its first less
//! than [`GROUP_MS`]; then the group seals ([`Confirmations::seal_due`]). The
//! app shows a group only once it is sealed (`sealed` in the JSON), and a
//! sealed group never takes another question, so one Allow over a sheet
//! answers exactly the questions it shows: one raised after the sheet came up
//! is a group, and a sheet, of its own. Each confirmation keeps its own
//! grant and its own answer.

use std::collections::VecDeque;

use serde_json::{Map, Value, json};

use crate::ClientId;
use crate::policy::{Risk, WriteKey};

/// How long a confirmation waits for the user (MCPDES 3.3).
pub const CONFIRM_HOLD_MS: u64 = 120_000;
/// The most confirmations one client may have waiting at once. -- WP UB5 --
/// (UXP B5) 8, was 3: one sheet shows those raised together (night mode on
/// three devices is six questions: loudness and the leveller on each).
pub const WAITING_PER_CLIENT: usize = 8;
/// Ended confirmations kept for `dspi_get_confirmation`.
const ENDED_KEPT: usize = 64;
// -- WP UB5 -- (UXP D2)
/// A group takes questions for at most this long after its first.
pub const GROUP_MS: u64 = 1_500;
/// A group seals once no question has joined it for this long.
pub const GROUP_QUIET_MS: u64 = 400;
// -- FIX016 -- (REVG R2, owner: "one Allow answers exactly the rows shown")
/// The most rows one grouped sheet takes: every row of its questions, the
/// asked ones (folded as the sheet folds them) and the unasked rest, plus
/// the devices' headings, as [`Confirmations::sheet`] lists them. A question
/// that would take its agent's open group past it starts the next group, and
/// so the next sheet; a question that alone has more rows is a sheet of its
/// own (its own unasked rest may then fold under "and N more"). It is the
/// rows a sheet shows: AiHost.qml's `rowsShown` must match it.
pub const GROUP_SHEET_ROWS: usize = 12;

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
    /// The agent disconnected or its device was closed before the answer;
    /// nothing was changed.
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
    /// The call's arguments, for the re-run, with `device` filled in.
    pub(crate) args: Map<String, Value>,
    /// The tool result the agent gets, once it has ended.
    pub(crate) result: Option<Value>,
    /// A held request: its JSON-RPC id and whether it was modern-era.
    pub(crate) request: Option<(Value, bool)>,
    // -- WP ODEC --
    /// It asks for test signals: Allow also grants them on its device for
    /// the task (`crate::grant`).
    pub grouped: bool,
    // -- WP UAG -- (UXP A6)
    /// The call's other changes in the app's words (`policy::Judging::rest`):
    /// what the same Allow also does. The app's alone.
    pub rest: Vec<String>,
    // -- WP UB5 -- (UXP D2)
    /// Its group: the id of the group's first question (its own id when it
    /// is the first). The app's alone.
    pub group: u64,
    /// Its group takes no more questions: the app may show it.
    pub sealed: bool,
}

impl Confirmation {
    /// The writes the user allows by Allow: those the risks name.
    pub(crate) fn grant(&self) -> Vec<WriteKey> {
        self.risks.iter().flat_map(|r| r.keys.clone()).collect()
    }

    /// As JSON, for the app's sheet (agents read the risks through
    /// `dspi_get_confirmation`, which builds its own): numbers and the
    /// server's words, and (-- WP UAG --) the app's words of each risk
    /// (`said`) and of the whole call (`batch`), which agents never read. -- WP FIN015 -- (REVF S8) two counting origins, on
    /// purpose: `risks[].output` counts from 1, as the risk's words and the
    /// agent do; `roles.rows[].output` is the wire output, from 0 (the app
    /// shows it as Out N+1), as the app's own speaker rows are.
    pub fn to_json(&self) -> Value {
        json!({
            "confirmation_id": self.id,
            "client": self.client.0,
            "client_name": self.client_label,
            "tool": self.tool,
            "device": self.device,
            "risks": self.risks.iter().map(|r| {
                let mut v = r.to_json();
                if let Some(o) = v.as_object_mut() {
                    // -- WP UAG --
                    o.insert("said".into(), json!(r.said));
                    o.insert("channel".into(), json!(r.channel));
                }
                v
            }).collect::<Vec<_>>(),
            "batch": self.batch(), // -- WP UAG --
            // -- WP UB5 -- (UXP D2) one sheet per group, shown once sealed
            "group": self.group,
            "sealed": self.sealed,
            "asked_ms": self.asked_ms,
            "expires_ms": self.expires_ms,
            "state": self.state.as_str(),
            // -- WP ODEC -- what Allow grants beyond this call
            "grant": if self.grouped {
                json!({"kind": crate::grant::TEST_SIGNALS, "device": self.device,
                       "minutes": crate::grant::GRANT_MS / 60_000,
                       "detect": crate::grant::asks_detect(&self.risks)}) // -- WP ADET1 --
            } else {
                Value::Null
            },
            // -- WP ADET1 -- the speaker detection's warning on the mechanism
            "warning": crate::grant::asks_detect(&self.risks)
                .then_some(crate::tools::measure::DETECT_WARNING),
            // -- WP ROLES -- speaker roles from the user's words: the table
            // (`after`: the question's words after the table)
            "roles": (self.tool == crate::tools::roles::SET_ROLES_NAME)
                .then(|| crate::tools::roles::table_json(&self.args))
                .flatten()
                .map(|mut t| {
                    let after = self
                        .risks
                        .iter()
                        .find(|r| r.kind == crate::policy::RiskKind::SpeakerRoles)
                        .and_then(|r| r.what.split_once('\n'))
                        .map_or("", |(_, a)| a);
                    if let Some(o) = t.as_object_mut() {
                        o.insert("after".into(), json!(after));
                    }
                    t
                }),
        })
    }
}

// -- WP UAG -- (UXP A6)
/// Folded rows name at most this many channels ("Left woofer, Right woofer,
/// Sub: limiter off"); more read "9 outputs: limiter off".
const FOLD_NAMES: usize = 3;

impl Confirmation {
    /// The asked rows of the call for the app's sheet: `{said, kind, risky:
    /// true}` each. Rows of one kind that differ only in their channel fold
    /// into one ("Left woofer, Right woofer: limiter off").
    fn asked_batch(&self) -> Vec<Value> {
        // (kind, channels, the words after them, or the whole row)
        let mut rows: Vec<(&'static str, Vec<&str>, &str)> = Vec::new();
        for r in &self.risks {
            let kind = r.kind.as_str();
            let split = r.output.and_then(|_| r.said.split_once(": "));
            match split {
                Some((ch, rest)) => {
                    if let Some(row) = rows
                        .iter_mut()
                        .find(|(k, chs, w)| *k == kind && !chs.is_empty() && *w == rest)
                    {
                        if !row.1.contains(&ch) {
                            row.1.push(ch);
                        }
                    } else {
                        rows.push((kind, vec![ch], rest));
                    }
                }
                None => rows.push((kind, Vec::new(), r.said.as_str())),
            }
        }
        rows.into_iter()
            .map(|(kind, chs, words)| {
                let said = match chs.len() {
                    0 => words.to_owned(),
                    n if n > FOLD_NAMES => format!("{n} outputs: {words}"),
                    _ => format!("{}: {words}", chs.join(", ")),
                };
                json!({"said": said, "kind": kind, "risky": true})
            })
            .collect()
    }

    /// The call's other changes, unasked: `{said, kind: null, risky: false}`
    /// each.
    fn rest_batch(&self) -> Vec<Value> {
        self.rest
            .iter()
            .map(|said| json!({"said": said, "kind": null, "risky": false}))
            .collect()
    }

    /// The whole call for the app's sheet, the asked rows first: `{said,
    /// kind, risky}` each. Asked rows of one kind that differ only in their
    /// channel fold into one ("Left woofer, Right woofer: limiter off").
    fn batch(&self) -> Vec<Value> {
        let mut out = self.asked_batch();
        out.extend(self.rest_batch());
        out
    }
}

// -- FIX016 -- (REVG R2)
/// The sheet of `members` (one group's questions, oldest first), in the order
/// [`Confirmations::sheet`] gives.
fn sheet_of(members: &[&Confirmation]) -> Vec<Value> {
    let mut devices: Vec<&Option<String>> = Vec::new();
    for c in members {
        if !devices.contains(&&c.device) {
            devices.push(&c.device);
        }
    }
    let tagged = |c: &Confirmation, mut row: Value| {
        if let Some(o) = row.as_object_mut() {
            o.insert("confirmation_id".into(), json!(c.id));
        }
        row
    };
    let head = |d: &Option<String>| json!({"head": true, "device": d, "risky": false});
    let mut out = Vec::new();
    for d in &devices {
        out.push(head(d));
        for c in members.iter().filter(|c| &&c.device == d) {
            out.extend(c.asked_batch().into_iter().map(|r| tagged(c, r)));
        }
    }
    for d in &devices {
        let rest: Vec<Value> = members
            .iter()
            .filter(|c| &&c.device == d)
            .flat_map(|c| c.rest_batch().into_iter().map(|r| tagged(c, r)))
            .collect();
        if devices.len() > 1 && !rest.is_empty() {
            out.push(head(d));
        }
        out.extend(rest);
    }
    out
}

// -- WP UB5 -- (UXP D2)
/// Whether a question may share a sheet: not a speaker detection's (its
/// warning and what the person does) nor the speaker roles' (their table).
fn groupable(tool: &str, risks: &[Risk]) -> bool {
    tool != crate::tools::roles::SET_ROLES_NAME && !crate::grant::asks_detect(risks)
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
    // -- FIX016 -- (REVG R2, B10)
    /// Its group may still take questions: the app has not shown it yet
    /// (`sealed` is false), so nobody can have seen what an Allow approves.
    Unsealed,
}

/// The table.
#[derive(Debug, Default)]
pub struct Confirmations {
    list: VecDeque<Confirmation>,
    next: u64,
    events: Vec<ConfirmEvent>,
}

impl Confirmations {
    /// Open one, waiting from `now_ms`; its id. -- FIX016 -- (REVG R2) The
    /// tests' form: the server opens with the call's rest
    /// ([`Confirmations::open_with_rest`]).
    #[cfg(test)]
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
        self.open_with_rest(
            client,
            client_label,
            tool,
            device,
            risks,
            Vec::new(),
            args,
            now_ms,
        )
    }

    /// [`Confirmations::open`] with `rest`, the call's other changes in the
    /// app's words (-- WP UAG --): -- FIX016 -- (REVG R2) its rows count
    /// towards the sheet's ([`GROUP_SHEET_ROWS`]) when it joins a group.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn open_with_rest(
        &mut self,
        client: ClientId,
        client_label: &str,
        tool: &'static str,
        device: Option<String>,
        risks: Vec<Risk>,
        rest: Vec<String>,
        mut args: Map<String, Value>,
        now_ms: u64,
    ) -> u64 {
        // -- WP RBFIX -- (review RB R1) the re-run goes to the device the user
        // was asked about, whichever one the app shows by then: a grant names
        // no device.
        if let Some(d) = &device {
            args.insert("device".into(), json!(d));
        }
        self.next = self.next.saturating_add(1);
        let id = self.next;
        let grouped = device.is_some() && crate::grant::asks_grant(&risks); // -- WP ODEC --
        // -- WP UB5 -- (UXP D2) its agent's open group, else a new one; a
        // speaker detection's or the roles' question is a sheet of its own
        let alone = !groupable(tool, &risks);
        let mut c = Confirmation {
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
            grouped,   // -- WP ODEC --
            rest,      // -- WP UAG --
            group: id, // -- WP UB5 --
            sealed: alone,
        };
        if !alone {
            let open = self
                .list
                .iter()
                .rev()
                .find(|o| {
                    o.client == client
                        && o.state == ConfirmState::Waiting
                        && !o.sealed
                        && groupable(o.tool, &o.risks)
                        && now_ms < self.seals_at(o.group)
                })
                .map(|o| o.group);
            // -- FIX016 -- (REVG R2) only while the whole sheet still fits:
            // else it starts the next group, and the next sheet
            if let Some(g) = open.filter(|g| {
                let members: Vec<&Confirmation> = self.members(*g).chain([&c]).collect();
                sheet_of(&members).len() <= GROUP_SHEET_ROWS
            }) {
                c.group = g;
            }
        }
        self.list.push_back(c);
        self.events.push(ConfirmEvent::Asked(id));
        self.prune();
        id
    }

    // -- FIX016 -- (REVG R2)
    /// The waiting questions of `group`, oldest first.
    fn members(&self, group: u64) -> impl Iterator<Item = &Confirmation> {
        self.list
            .iter()
            .filter(move |c| c.group == group && c.state == ConfirmState::Waiting)
    }

    /// `group`'s sheet in the order to show it: every asked row of the whole
    /// group first, under its device's heading (the devices in the order
    /// they first asked), then the unasked rest, so a row the app folds away
    /// is never one an Allow approves. Each row is `{said, kind, risky,
    /// confirmation_id}`; a heading is `{head: true, device, risky: false}`
    /// (the app names the device). The rest's headings repeat only when the
    /// group spans several devices. A group holds at most
    /// [`GROUP_SHEET_ROWS`] rows, unless one question alone has more.
    pub fn sheet(&self, group: u64) -> Vec<Value> {
        let members: Vec<&Confirmation> = self.members(group).collect();
        sheet_of(&members)
    }

    // -- WP UB5 -- (UXP D2)
    /// When `group` seals, on the server's clock: [`GROUP_QUIET_MS`] after
    /// its newest question, at most [`GROUP_MS`] after its first.
    pub fn seals_at(&self, group: u64) -> u64 {
        let asked = self
            .list
            .iter()
            .filter(|c| c.group == group)
            .map(|c| c.asked_ms);
        let (first, last) = asked.fold((u64::MAX, 0), |(f, l), a| (f.min(a), l.max(a)));
        last.saturating_add(GROUP_QUIET_MS)
            .min(first.saturating_add(GROUP_MS))
    }

    /// Seal the groups whose time is up by `now_ms`: from then on they take
    /// no question. Call it before the app reads the waiting ones.
    pub(crate) fn seal_due(&mut self, now_ms: u64) {
        let due: Vec<u64> = self
            .list
            .iter()
            .filter(|c| !c.sealed && now_ms >= self.seals_at(c.group))
            .map(|c| c.group)
            .collect();
        for c in &mut self.list {
            if due.contains(&c.group) {
                c.sealed = true;
            }
        }
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

    // -- WP MCP4 --
    /// `client`'s waiting ones that hold its JSON-RPC request `rpc_id`.
    pub(crate) fn held_as(&self, client: ClientId, rpc_id: &Value) -> Vec<u64> {
        self.list
            .iter()
            .filter(|c| {
                c.client == client
                    && c.state == ConfirmState::Waiting
                    && c.request.as_ref().is_some_and(|(id, _)| id == rpc_id)
            })
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
