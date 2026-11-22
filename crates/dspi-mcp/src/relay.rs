//! The stdio relay (WP MCP3, MCPDES 1.4): what an agent host that starts
//! local servers as programs (Claude Desktop, most IDE agents) runs. It reads
//! MCP messages from standard input, one per line, posts each to the app's
//! loopback [`http`](crate::http) endpoint, and writes the replies to
//! standard output, one per line. Standard error gets the relay's own words
//! (pairing, the app not running); nothing else ever reaches standard output.
//!
//! - **The token.** Kept by a [`TokenKeeper`]: the Qt app's is the system's
//!   secret store, the Terminal's a file only this user can read
//!   ([`FileTokenKeeper`]). With none, or one the app refuses (401), the
//!   relay pairs: the app shows the agent's name and a code, the relay prints
//!   the same code, the user allows, and the new token is kept.
//! - **Headers.** It sets what each era needs from the message itself:
//!   `MCP-Protocol-Version`, `Mcp-Method` and `Mcp-Name` for 2026-07-28; the
//!   session id and the agreed version after a legacy `initialize`. When the
//!   app has forgotten the session (it restarted), the relay repeats the
//!   agent's `initialize` once and sends the request again.
//! - **The app not running.** A request is answered with an error that says
//!   to start DSPi Console and turn AI agents on, and the relay keeps going:
//!   the next request may find the app.
//! - It ends when standard input closes.

use std::io::{self, BufRead, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::http::{DEFAULT_PORT, MCP_PATH, PAIR_PATH, PAIR_WAIT_PATH};
use crate::protocol::{self, META_CLIENT_INFO, META_VERSION, RpcError};

/// The error code of the relay's own failures (the app not there, pairing
/// refused): JSON-RPC's implementation-defined server error range.
pub const RELAY_ERROR: i64 = -32000;

/// What the agent is told when the app does not answer.
pub const NOT_RUNNING_TEXT: &str = "DSPi Console is not running, or AI agents are turned off in it. \
Start DSPi Console and turn on Settings, AI agents, then try again.";

/// How long the relay waits after a refused pairing before it asks again.
pub const PAIR_RETRY_AFTER: Duration = Duration::from_secs(60);

/// Where the relay keeps its token.
pub trait TokenKeeper {
    /// The token kept, if any.
    fn load(&mut self) -> Option<String>;
    /// Keep `token` (it replaces any other).
    fn save(&mut self, token: &str);
    /// The app refused the kept token: drop it.
    fn forget(&mut self);
}

/// A token in a file only this user can read (the Terminal's way, as its
/// DSPi Link tokens).
#[derive(Debug, Clone)]
pub struct FileTokenKeeper {
    path: PathBuf,
}

impl FileTokenKeeper {
    /// Keep the token in `path`.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl TokenKeeper for FileTokenKeeper {
    fn load(&mut self) -> Option<String> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        let t = text.trim();
        (!t.is_empty()).then(|| t.to_owned())
    }

    fn save(&mut self, token: &str) {
        let write = || -> io::Result<()> {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = self.path.with_extension("new");
            let mut f = private_file(&tmp)?;
            f.write_all(token.as_bytes())?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&tmp, &self.path)
        };
        if let Err(e) = write() {
            log::warn!(
                "mcp relay: the token was not kept ({e}); the agent will pair again next time"
            );
        }
    }

    fn forget(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(unix)]
fn private_file(path: &std::path::Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn private_file(path: &std::path::Path) -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

/// A token kept only while the relay runs (tests; a keeper that cannot keep).
#[derive(Debug, Default, Clone)]
pub struct MemoryTokenKeeper {
    /// The token.
    pub token: Option<String>,
}

impl TokenKeeper for MemoryTokenKeeper {
    fn load(&mut self) -> Option<String> {
        self.token.clone()
    }
    fn save(&mut self, token: &str) {
        self.token = Some(token.to_owned());
    }
    fn forget(&mut self) {
        self.token = None;
    }
}

/// How the relay finds the app.
#[derive(Debug, Clone)]
pub struct RelayConfig {
    /// The app's port.
    pub port: u16,
    /// The name to pair under when the agent gives none.
    pub fallback_name: String,
    /// How long one HTTP exchange may take (a tool call included).
    pub timeout: Duration,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            fallback_name: "AI agent (stdio)".into(),
            timeout: Duration::from_secs(120),
        }
    }
}

/// One HTTP reply.
#[derive(Debug)]
struct Reply {
    status: u16,
    session: Option<String>,
    body: Vec<u8>,
}

/// POST `body` to `path` on the app; `headers` are added as given.
fn post(
    cfg: &RelayConfig,
    path: &str,
    headers: &[(&str, String)],
    body: &[u8],
) -> io::Result<Reply> {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), cfg.port);
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
    s.set_read_timeout(Some(cfg.timeout))?;
    s.set_write_timeout(Some(cfg.timeout))?;
    let mut head = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n",
        cfg.port,
        body.len()
    );
    for (n, v) in headers {
        // A value with a line break would split the head: never sent.
        if v.contains(['\r', '\n']) {
            continue;
        }
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes())?;
    s.write_all(body)?;
    s.flush()?;
    let mut raw = Vec::new();
    let deadline = Instant::now().checked_add(cfg.timeout);
    let mut chunk = [0u8; 8192];
    // Up to the end of the head, then by Content-Length (or to the end).
    loop {
        let n = match s.read(&mut chunk) {
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if n == 0 {
            break;
        }
        raw.extend_from_slice(chunk.get(..n).unwrap_or_default());
        if raw.len() > 64 * 1024 * 1024 {
            return Err(io::Error::other("the app's reply is too large"));
        }
        if let Some(done) = complete(&raw) {
            raw.truncate(done);
            break;
        }
        if deadline.is_some_and(|d| Instant::now() > d) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the app took too long",
            ));
        }
    }
    parse_reply(&raw).ok_or_else(|| io::Error::other("the app's reply is not HTTP"))
}

/// The length of a whole reply in `raw`, once it has arrived.
fn complete(raw: &[u8]) -> Option<usize> {
    let head_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")?
        .checked_add(4)?;
    let mut slots = [httparse::EMPTY_HEADER; 32];
    let mut r = httparse::Response::new(&mut slots);
    r.parse(raw.get(..head_end)?).ok()?;
    let length = r
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("Content-Length"))
        .and_then(|h| {
            std::str::from_utf8(h.value)
                .ok()?
                .trim()
                .parse::<usize>()
                .ok()
        })?;
    let total = head_end.checked_add(length)?;
    (raw.len() >= total).then_some(total)
}

fn parse_reply(raw: &[u8]) -> Option<Reply> {
    let head_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")?
        .checked_add(4)?;
    let mut slots = [httparse::EMPTY_HEADER; 32];
    let mut r = httparse::Response::new(&mut slots);
    r.parse(raw.get(..head_end)?).ok()?;
    let session = r
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("Mcp-Session-Id"))
        .and_then(|h| std::str::from_utf8(h.value).ok())
        .map(|v| v.trim().to_owned());
    Some(Reply {
        status: r.code?,
        session,
        body: raw.get(head_end..)?.to_vec(),
    })
}

/// The relay's state between messages.
struct Relay<'a> {
    cfg: RelayConfig,
    keeper: &'a mut dyn TokenKeeper,
    log: &'a mut dyn Write,
    token: Option<String>,
    /// Legacy: the session and agreed version from `initialize`, and the
    /// agent's `initialize` itself, to repeat if the app forgot the session.
    session: Option<String>,
    legacy_version: Option<String>,
    initialize: Option<Value>,
    /// Do not ask to pair again before this.
    pair_after: Option<Instant>,
}

/// What came of sending one message.
enum Sent {
    /// The app's reply body (a JSON-RPC message) or nothing (202).
    Reply(Option<Value>),
    /// The relay's own failure, in its words.
    Failed(String),
}

impl Relay<'_> {
    fn say(&mut self, text: &str) {
        let _ = writeln!(self.log, "dspi-mcp relay: {text}");
        let _ = self.log.flush();
    }

    fn headers(&self, msg: &Value) -> Vec<(&'static str, String)> {
        let mut h = Vec::new();
        if let Some(t) = &self.token {
            h.push(("Authorization", format!("Bearer {t}")));
        }
        let method = msg.get("method").and_then(Value::as_str);
        let modern = msg
            .pointer("/params/_meta")
            .and_then(|m| m.get(META_VERSION))
            .and_then(Value::as_str);
        if let Some(v) = modern {
            h.push(("MCP-Protocol-Version", v.to_owned()));
            if let Some(m) = method {
                h.push(("Mcp-Method", m.to_owned()));
                let name = match m {
                    "tools/call" | "prompts/get" => msg.pointer("/params/name"),
                    "resources/read" => msg.pointer("/params/uri"),
                    _ => None,
                };
                if let Some(n) = name.and_then(Value::as_str) {
                    h.push(("Mcp-Name", n.to_owned()));
                }
            }
        } else if method != Some("initialize") {
            if let Some(s) = &self.session {
                h.push(("Mcp-Session-Id", s.clone()));
            }
            if let Some(v) = &self.legacy_version {
                h.push(("MCP-Protocol-Version", v.clone()));
            }
        }
        h
    }

    fn send_once(&mut self, msg: &Value) -> io::Result<Reply> {
        let body = msg.to_string();
        post(&self.cfg, MCP_PATH, &self.headers(msg), body.as_bytes())
    }

    /// Send `msg`, pairing first when there is no token, again when the app
    /// refuses it, and starting a new session when the app forgot this one.
    fn send(&mut self, msg: &Value) -> Sent {
        if self.token.is_none()
            && let Err(why) = self.pair(msg)
        {
            return Sent::Failed(why);
        }
        let mut reply = match self.send_once(msg) {
            Ok(r) => r,
            Err(_) => return Sent::Failed(NOT_RUNNING_TEXT.into()),
        };
        if reply.status == 401 {
            self.say("DSPi Console no longer knows this agent's token; pairing again");
            self.keeper.forget();
            self.token = None;
            if let Err(why) = self.pair(msg) {
                return Sent::Failed(why);
            }
            reply = match self.send_once(msg) {
                Ok(r) => r,
                Err(_) => return Sent::Failed(NOT_RUNNING_TEXT.into()),
            };
        }
        if reply.status == 404 && self.session.is_some() && self.renew_session() {
            reply = match self.send_once(msg) {
                Ok(r) => r,
                Err(_) => return Sent::Failed(NOT_RUNNING_TEXT.into()),
            };
        }
        if msg.get("method").and_then(Value::as_str) == Some("initialize") && reply.status == 200 {
            self.session = reply.session.clone();
            self.legacy_version = serde_json::from_slice::<Value>(&reply.body)
                .ok()
                .and_then(|v| {
                    v.pointer("/result/protocolVersion")?
                        .as_str()
                        .map(str::to_owned)
                });
            self.initialize = Some(msg.clone());
        }
        if reply.body.is_empty() {
            return Sent::Reply(None);
        }
        match serde_json::from_slice::<Value>(&reply.body) {
            Ok(v) if v.get("jsonrpc").is_some() => Sent::Reply(Some(v)),
            _ => Sent::Failed(format!("DSPi Console answered HTTP {}", reply.status)),
        }
    }

    /// The app restarted: initialise again as the agent did, quietly.
    fn renew_session(&mut self) -> bool {
        let Some(init) = self.initialize.clone() else {
            return false;
        };
        self.session = None;
        let Ok(r) = self.send_once(&init) else {
            return false;
        };
        if r.status != 200 || r.session.is_none() {
            return false;
        }
        self.session = r.session;
        let done = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let _ = self.send_once(&done);
        true
    }

    /// The agent's name, from the message that asks.
    fn name_in(&self, msg: &Value) -> String {
        let info = msg.pointer("/params/clientInfo").or_else(|| {
            msg.pointer("/params/_meta")
                .and_then(|m| m.get(META_CLIENT_INFO))
        });
        info.and_then(|i| i.get("title").or_else(|| i.get("name")))
            .and_then(Value::as_str)
            .filter(|n| !n.trim().is_empty())
            .map_or_else(|| self.cfg.fallback_name.clone(), str::to_owned)
    }

    /// Ask the app for a token and wait for the user's answer.
    fn pair(&mut self, msg: &Value) -> Result<(), String> {
        if self.pair_after.is_some_and(|t| Instant::now() < t) {
            return Err(
                "pairing with DSPi Console was refused a moment ago; try again in a minute".into(),
            );
        }
        let name = self.name_in(msg);
        let body = json!({"name": name}).to_string();
        let r = post(&self.cfg, PAIR_PATH, &[], body.as_bytes())
            .map_err(|_| NOT_RUNNING_TEXT.to_owned())?;
        let v: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
        if r.status != 200 {
            let why = v
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("pairing was refused")
                .to_owned();
            self.say(&why);
            return Err(format!("DSPi Console: {why}"));
        }
        let ticket = v
            .get("ticket")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let code = v.get("code").and_then(Value::as_str).unwrap_or("");
        self.say(&format!(
            "DSPi Console asks you to allow \"{name}\". Check that it shows the code {code}, then click Allow."
        ));
        let wait = json!({"ticket": ticket}).to_string();
        loop {
            let r = post(&self.cfg, PAIR_WAIT_PATH, &[], wait.as_bytes())
                .map_err(|_| NOT_RUNNING_TEXT.to_owned())?;
            let v: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
            match v.get("status").and_then(Value::as_str) {
                Some("pending") => continue,
                Some("allowed") => {
                    let token = v
                        .get("token")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned();
                    if token.is_empty() {
                        return Err("DSPi Console allowed the agent but sent no token".into());
                    }
                    self.keeper.save(&token);
                    self.token = Some(token);
                    self.say("paired with DSPi Console");
                    return Ok(());
                }
                Some("denied") => {
                    self.pair_after = Instant::now().checked_add(PAIR_RETRY_AFTER);
                    self.say("the request was denied in DSPi Console");
                    return Err("the user denied this agent in DSPi Console".into());
                }
                _ => {
                    self.pair_after = Instant::now().checked_add(PAIR_RETRY_AFTER);
                    self.say("nobody answered the request in DSPi Console in time");
                    return Err("nobody allowed this agent in DSPi Console in time".into());
                }
            }
        }
    }
}

/// Relay `input` to the app on `cfg.port` and its replies to `output`, until
/// `input` ends. Only a failure to write `output` ends it early (the agent
/// has gone).
pub fn run(
    cfg: RelayConfig,
    keeper: &mut dyn TokenKeeper,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
    log: &mut dyn Write,
) -> io::Result<()> {
    let token = keeper.load();
    let mut relay = Relay {
        cfg,
        keeper,
        log,
        token,
        session: None,
        legacy_version: None,
        initialize: None,
        pair_after: None,
    };
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(_) => {
                let e = protocol::error_reply(
                    &Value::Null,
                    &RpcError::new(protocol::PARSE_ERROR, "not JSON"),
                );
                write_line(output, &e)?;
                continue;
            }
        };
        let id = msg
            .get("id")
            .filter(|_| msg.get("method").is_some())
            .cloned();
        match relay.send(&msg) {
            Sent::Reply(Some(reply)) => write_line(output, &reply)?,
            Sent::Reply(None) => {}
            Sent::Failed(why) => {
                // A notification gets no reply, even an error.
                if let Some(id) = id {
                    let e = protocol::error_reply(&id, &RpcError::new(RELAY_ERROR, why));
                    write_line(output, &e)?;
                }
            }
        }
    }
}

/// One message, one line: serde_json writes no line breaks.
fn write_line(output: &mut dyn Write, v: &Value) -> io::Result<()> {
    let mut text = v.to_string();
    text.push('\n');
    output.write_all(text.as_bytes())?;
    output.flush()
}
