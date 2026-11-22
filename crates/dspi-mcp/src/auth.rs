//! Who may call the server (MCPDES 3.1, WP MCP3): one token per client,
//! pairing, and the store that keeps the tokens' hashes.
//!
//! - **A client is a named principal** ("Claude Code on this computer") with
//!   its own 256-bit token. Only the token's SHA-256 is kept, in
//!   `mcp-clients.json` in the app's folder, a file only this user can read.
//!   A token is shown once, when it is made, and never logged.
//! - **Checking a token** hashes it and compares the hash with every stored
//!   one, in constant time and without stopping at a match, so a wrong token
//!   costs the same as a right one and tells nothing about the others.
//! - **Pairing** is how the stdio relay gets a token without the user
//!   copying one: it asks under a name, the app shows the name and a 4-digit
//!   code, which the relay also prints, and the user allows or denies. One
//!   request waits at a time, a few a minute, each for at most
//!   [`PAIRING_TTL_MS`]. The token goes back once, to the holder of the
//!   request's ticket; one not picked up in time is revoked.
//! - Clients the user adds by hand (the HTTP agents, whose configuration
//!   takes a header) are [`Clients::mint`]ed by the app.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::hint::black_box;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use data_encoding::{BASE64URL_NOPAD, HEXLOWER};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::untrusted;

/// What every token starts with, so a leaked one is recognisable (secret
/// scanners, the user reading a config file).
pub const TOKEN_PREFIX: &str = "dspi-mcp-";

/// The most clients there can be.
pub const MAX_CLIENTS: usize = 16;

/// How long a pairing request waits for the user, and then for the relay
/// to pick up its token.
pub const PAIRING_TTL_MS: u64 = 120_000;

/// Pairing requests allowed per [`PAIRING_WINDOW_MS`]: a caller that is
/// refused cannot fill the app with questions.
pub const PAIRING_PER_WINDOW: usize = 3;
/// See [`PAIRING_PER_WINDOW`].
pub const PAIRING_WINDOW_MS: u64 = 60_000;

/// The store's file name in the folder the app gives.
pub const STORE_FILE: &str = "mcp-clients.json";

type Hash = [u8; 32];

fn sha256(bytes: &[u8]) -> Hash {
    Sha256::digest(bytes).into()
}

/// Every byte is looked at, whatever the first difference.
fn hashes_equal(a: &Hash, b: &Hash) -> bool {
    let difference = a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y));
    black_box(difference) == 0
}

fn random<const N: usize>() -> Result<[u8; N], AuthError> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|_| AuthError::NoRandom)?;
    Ok(bytes)
}

/// A new token.
fn new_token() -> Result<String, AuthError> {
    let raw: [u8; 32] = random()?;
    Ok(format!("{TOKEN_PREFIX}{}", BASE64URL_NOPAD.encode(&raw)))
}

/// Four digits, every code equally likely.
fn new_code() -> Result<String, AuthError> {
    // 65536 - 65536 % 10000: values at or above it would favour low codes.
    const FAIR: u16 = 60_000;
    loop {
        let n = u16::from_le_bytes(random::<2>()?);
        if n < FAIR {
            return Ok(format!("{:04}", n % 10_000));
        }
    }
}

/// A client's name as the user sees it: cleaned and capped, never empty.
fn client_name(name: &str) -> String {
    let n = untrusted::clean(name, untrusted::CLIENT_MAX_BYTES);
    if n.is_empty() { "AI agent".into() } else { n }
}

/// Why an auth operation did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// There are [`MAX_CLIENTS`] already.
    Full,
    /// The system gave no random numbers.
    NoRandom,
    /// The store could not be read or written; the words say which file.
    Store(String),
    /// A pairing request is already waiting for the user.
    Busy,
    /// Too many pairing requests lately.
    TooMany,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => write!(
                f,
                "there are {MAX_CLIENTS} AI clients already: remove one first"
            ),
            Self::NoRandom => write!(f, "the system gave no random numbers for a token"),
            Self::Store(why) => write!(f, "the AI clients' file: {why}"),
            Self::Busy => write!(f, "another pairing request is waiting in DSPi Console"),
            Self::TooMany => write!(f, "too many pairing requests: wait a minute"),
        }
    }
}

impl std::error::Error for AuthError {}

/// A paired client as the app shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedClient {
    /// Its id, for revoking; never reused.
    pub id: u64,
    /// The name it was paired or added under (cleaned).
    pub name: String,
    /// When it was made, Unix seconds.
    pub created_s: u64,
    /// When its token last opened a request, Unix seconds.
    pub last_used_s: Option<u64>,
    hash: Hash,
}

impl PairedClient {
    /// `{id, name, created, lastUsed}`: never the hash.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "created": self.created_s,
            "lastUsed": self.last_used_s,
        })
    }
}

/// A token made for a client: shown once, then only its hash is kept.
#[derive(Clone, PartialEq, Eq)]
pub struct Minted {
    /// The client's id.
    pub id: u64,
    /// The token.
    pub token: String,
}

impl std::fmt::Debug for Minted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Minted")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// The paired clients, kept in a file (or only in memory, for tests).
#[derive(Debug)]
pub struct Clients {
    path: Option<PathBuf>,
    list: Vec<PairedClient>,
    next_id: u64,
}

impl Clients {
    /// No clients, kept nowhere.
    pub fn in_memory() -> Self {
        Self {
            path: None,
            list: Vec::new(),
            next_id: 1,
        }
    }

    /// The clients stored in `dir`'s [`STORE_FILE`]; none when it does not
    /// exist yet. A file that is there and cannot be read is an error, so a
    /// damaged file is not overwritten with an empty list.
    pub fn load(dir: &Path) -> Result<Self, AuthError> {
        let path = dir.join(STORE_FILE);
        let mut clients = Self {
            path: Some(path.clone()),
            ..Self::in_memory()
        };
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(clients),
            Err(e) => {
                return Err(AuthError::Store(format!(
                    "{} cannot be read: {e}",
                    path.display()
                )));
            }
        };
        let bad = || AuthError::Store(format!("{} is damaged", path.display()));
        let doc: Value = serde_json::from_str(&text).map_err(|_| bad())?;
        clients.next_id = doc
            .get("nextId")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1);
        for c in doc
            .get("clients")
            .and_then(Value::as_array)
            .ok_or_else(bad)?
        {
            let hash = c
                .get("hash")
                .and_then(Value::as_str)
                .and_then(|h| HEXLOWER.decode(h.as_bytes()).ok())
                .and_then(|h| Hash::try_from(h).ok())
                .ok_or_else(bad)?;
            let id = c.get("id").and_then(Value::as_u64).ok_or_else(bad)?;
            clients.next_id = clients.next_id.max(id.saturating_add(1));
            clients.list.push(PairedClient {
                id,
                name: client_name(c.get("name").and_then(Value::as_str).unwrap_or("")),
                created_s: c.get("created").and_then(Value::as_u64).unwrap_or(0),
                last_used_s: c.get("lastUsed").and_then(Value::as_u64),
                hash,
            });
        }
        clients.list.truncate(MAX_CLIENTS);
        Ok(clients)
    }

    /// Write the file, whole or not at all, never readable by others.
    pub fn save(&self) -> Result<(), AuthError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let clients: Vec<Value> = self
            .list
            .iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "name": c.name,
                    "created": c.created_s,
                    "lastUsed": c.last_used_s,
                    "hash": HEXLOWER.encode(&c.hash),
                })
            })
            .collect();
        let doc = json!({"version": 1, "nextId": self.next_id, "clients": clients});
        let fail =
            |e: io::Error| AuthError::Store(format!("{} cannot be written: {e}", path.display()));
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(fail)?;
        }
        let tmp = path.with_extension("json.new");
        let mut f = private_file(&tmp).map_err(fail)?;
        f.write_all(doc.to_string().as_bytes()).map_err(fail)?;
        f.sync_all().map_err(fail)?;
        drop(f);
        fs::rename(&tmp, path).map_err(fail)
    }

    /// The clients, oldest first.
    pub fn list(&self) -> &[PairedClient] {
        &self.list
    }

    /// A new client called `name`, with its token. Saved before the token is
    /// handed out: a token that would not survive a restart is not given.
    pub fn mint(&mut self, name: &str, now_s: u64) -> Result<Minted, AuthError> {
        if self.list.len() >= MAX_CLIENTS {
            return Err(AuthError::Full);
        }
        let token = new_token()?;
        let id = self.next_id;
        self.next_id = id.saturating_add(1);
        self.list.push(PairedClient {
            id,
            name: client_name(name),
            created_s: now_s,
            last_used_s: None,
            hash: sha256(token.as_bytes()),
        });
        if let Err(e) = self.save() {
            self.list.retain(|c| c.id != id);
            return Err(e);
        }
        Ok(Minted { id, token })
    }

    /// Remove client `id`; false if there was none. The file is written; a
    /// failure to write it is logged, and the client is gone for this run
    /// all the same.
    pub fn revoke(&mut self, id: u64) -> bool {
        let before = self.list.len();
        self.list.retain(|c| c.id != id);
        if self.list.len() == before {
            return false;
        }
        if let Err(e) = self.save() {
            log::error!("mcp: a revoked client is still in the file: {e}");
        }
        true
    }

    /// The client whose token this is. Every stored hash is compared, in
    /// constant time; a token of any length or shape is hashed the same way.
    pub fn verify(&mut self, token: &str, now_s: u64) -> Option<u64> {
        let presented = sha256(token.as_bytes());
        let mut found = None;
        for c in &self.list {
            if hashes_equal(&presented, &c.hash) {
                found = Some(c.id);
            }
        }
        let id = black_box(found)?;
        if let Some(c) = self.list.iter_mut().find(|c| c.id == id) {
            c.last_used_s = Some(now_s);
        }
        Some(id)
    }
}

#[cfg(unix)]
fn private_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn private_file(path: &Path) -> io::Result<File> {
    // The user's own profile folder: other users cannot read it on Windows.
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

// ── Pairing ─────────────────────────────────────────────────────────────

/// A pairing request for the app to show.
#[derive(Clone, PartialEq, Eq)]
pub struct PairingRequest {
    /// Its id, for [`Pairing::answer`].
    pub id: u64,
    /// The name it asks under (cleaned).
    pub name: String,
    /// The 4-digit code the requester prints too.
    pub code: String,
    /// The requester's secret for [`Pairing::poll`]; only the requester sees it.
    pub ticket: String,
}

impl std::fmt::Debug for PairingRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairingRequest")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("code", &self.code)
            .finish_non_exhaustive()
    }
}

/// How a pairing request stands, for its requester.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairPoll {
    /// The user has not answered yet.
    Waiting,
    /// Allowed: the token, handed out this once.
    Allowed(Minted),
    /// The user said no.
    Denied,
    /// Not answered in time, or no such request (a wrong ticket, one already
    /// picked up).
    Gone,
}

/// How a pairing request ended, for the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairEnd {
    /// The requester has its token.
    Paired,
    /// The user denied it.
    Denied,
    /// Nobody answered in time, or the token was not picked up (it is
    /// revoked).
    Expired,
}

impl PairEnd {
    /// `paired`, `denied`, `expired`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Paired => "paired",
            Self::Denied => "denied",
            Self::Expired => "expired",
        }
    }
}

#[derive(Debug)]
enum PairState {
    Waiting,
    Allowed(Minted),
    Denied,
}

#[derive(Debug)]
struct Pending {
    id: u64,
    name: String,
    code: String,
    ticket: Hash,
    asked_ms: u64,
    state: PairState,
}

/// The pairing desk: at most one request at a time.
#[derive(Debug, Default)]
pub struct Pairing {
    pending: Option<Pending>,
    next_id: u64,
    recent: VecDeque<u64>,
}

impl Pairing {
    /// A request from `name`. One waits at a time ([`AuthError::Busy`]),
    /// [`PAIRING_PER_WINDOW`] a minute ([`AuthError::TooMany`]).
    pub fn request(
        &mut self,
        name: &str,
        now_ms: u64,
        clients: &mut Clients,
    ) -> Result<PairingRequest, AuthError> {
        self.expire(now_ms, clients);
        if self.pending.is_some() {
            return Err(AuthError::Busy);
        }
        while self
            .recent
            .front()
            .is_some_and(|t| now_ms.saturating_sub(*t) >= PAIRING_WINDOW_MS)
        {
            self.recent.pop_front();
        }
        if self.recent.len() >= PAIRING_PER_WINDOW {
            return Err(AuthError::TooMany);
        }
        if clients.list().len() >= MAX_CLIENTS {
            return Err(AuthError::Full);
        }
        let ticket = BASE64URL_NOPAD.encode(&random::<32>()?);
        let code = new_code()?;
        self.recent.push_back(now_ms);
        self.next_id = self.next_id.saturating_add(1);
        let name = client_name(name);
        self.pending = Some(Pending {
            id: self.next_id,
            name: name.clone(),
            code: code.clone(),
            ticket: sha256(ticket.as_bytes()),
            asked_ms: now_ms,
            state: PairState::Waiting,
        });
        Ok(PairingRequest {
            id: self.next_id,
            name,
            code,
            ticket,
        })
    }

    /// The request waiting for the user, if any: `(id, name, code)`.
    pub fn waiting(&mut self, now_ms: u64, clients: &mut Clients) -> Option<(u64, String, String)> {
        self.expire(now_ms, clients);
        self.pending
            .as_ref()
            .filter(|p| matches!(p.state, PairState::Waiting))
            .map(|p| (p.id, p.name.clone(), p.code.clone()))
    }

    /// The user's answer to request `id`: Allow mints the client's token
    /// under the name it asked with, Deny refuses. False when `id` is not
    /// waiting.
    pub fn answer(
        &mut self,
        id: u64,
        allow: bool,
        now_ms: u64,
        now_s: u64,
        clients: &mut Clients,
    ) -> Result<bool, AuthError> {
        self.expire(now_ms, clients);
        let Some(p) = self.pending.as_mut().filter(|p| p.id == id) else {
            return Ok(false);
        };
        if !matches!(p.state, PairState::Waiting) {
            return Ok(false);
        }
        p.state = if allow {
            PairState::Allowed(clients.mint(&p.name, now_s)?)
        } else {
            PairState::Denied
        };
        Ok(true)
    }

    /// The requester's view, by its ticket. An answer is given once; then the
    /// request is gone. Also says how the request ended, for the app.
    pub fn poll(
        &mut self,
        ticket: &str,
        now_ms: u64,
        clients: &mut Clients,
    ) -> (PairPoll, Option<(u64, PairEnd)>) {
        if let Some(ended) = self.expire(now_ms, clients) {
            // Whoever asks about an expired request learns it is gone.
            return (PairPoll::Gone, Some(ended));
        }
        let presented = sha256(ticket.as_bytes());
        let Some(p) = self.pending.as_ref() else {
            return (PairPoll::Gone, None);
        };
        if !hashes_equal(&presented, &p.ticket) {
            return (PairPoll::Gone, None);
        }
        if matches!(p.state, PairState::Waiting) {
            return (PairPoll::Waiting, None);
        }
        let Some(p) = self.pending.take() else {
            return (PairPoll::Gone, None);
        };
        match p.state {
            PairState::Allowed(m) => (PairPoll::Allowed(m), Some((p.id, PairEnd::Paired))),
            PairState::Denied => (PairPoll::Denied, Some((p.id, PairEnd::Denied))),
            PairState::Waiting => (PairPoll::Waiting, None),
        }
    }

    /// End a request older than [`PAIRING_TTL_MS`]; a token nobody picked up
    /// is revoked.
    pub fn expire(&mut self, now_ms: u64, clients: &mut Clients) -> Option<(u64, PairEnd)> {
        let p = self.pending.as_ref()?;
        if now_ms.saturating_sub(p.asked_ms) < PAIRING_TTL_MS {
            return None;
        }
        let p = self.pending.take()?;
        match p.state {
            PairState::Allowed(m) => {
                clients.revoke(m.id);
                Some((p.id, PairEnd::Expired))
            }
            PairState::Denied => Some((p.id, PairEnd::Denied)),
            PairState::Waiting => Some((p.id, PairEnd::Expired)),
        }
    }
}
