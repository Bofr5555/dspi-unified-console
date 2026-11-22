//! The server: its clients, the dispatch of both eras, the rate limits and
//! the activity log.

use std::collections::BTreeMap;
use std::sync::Arc;

use dspi_core::actor::{ActorId, ActorScope, EMBEDDER, register_actor};
use dspi_core::{AutoEqLibrary, CoreSet, DeviceId, DspiCore, GuardPhase};
use serde_json::{Map, Value, json};

use crate::activity::{Activity, ActivityEntry, Outcome};
use crate::protocol::{
    self, Era, INSTRUCTIONS, INVALID_PARAMS, INVALID_REQUEST, LEGACY_VERSIONS, LIST_TTL_MS,
    META_CAPABILITIES, META_CLIENT_INFO, META_SERVER_INFO, META_VERSION, METHOD_NOT_FOUND,
    MODERN_VERSIONS, PARSE_ERROR, RpcError,
};
use crate::rate::{Bucket, READ_LIMIT, RateLimit, WRITE_LIMIT};
use crate::registry::{
    self, AgentGuards, Policy, ReadOnlyPolicy, Registry, RunCtx, Tool, ToolClass,
};
use crate::{ClientId, prompts, resources, untrusted};
// -- WP MCP2a --
use crate::confirm::{
    AnswerError, CONFIRM_HOLD_MS, ConfirmEvent, ConfirmState, Confirmation, Confirmations,
    WAITING_PER_CLIENT,
};
use crate::policy::WriteKey;

// -- WP MCP2a --
/// A drag the user has marked for this long, with none of the user's
/// changes meanwhile, is taken as stale: a control destroyed while pressed
/// never said it let go (review RA, MCP0 item 4). The server clears the mark
/// and tries the agent's write once more.
pub const STALE_DRAG_MS: u64 = 10_000;

// -- WP MCP2a --
/// [`Server::handle_held`]'s answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Handled {
    /// The reply now (`None` for a notification or a response).
    Reply(Option<String>),
    /// A tool call held for the user's word: [`Server::resume`] gives its
    /// reply once the confirmation has ended.
    Held(u64),
}

/// A drag the server watches (-- WP MCP2a --): since when it has refused the
/// agent, and the user's newest change then (id and time).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DragWatch {
    since_ms: u64,
    mark: (u64, u64),
}

/// How a [`Server`] is set up.
#[derive(Clone)]
pub struct ServerConfig {
    /// `serverInfo.name`.
    pub name: String,
    /// `serverInfo.version`: the app's version.
    pub version: String,
    /// The read tools' rate limit per client.
    pub read_limit: RateLimit,
    /// The write tools' rate limit per client.
    pub write_limit: RateLimit,
    /// The agent policy.
    pub policy: Arc<dyn Policy>,
    // -- WP MCP2b --
    /// The AutoEQ database the app keeps, for the AutoEQ tools (none: they
    /// say so).
    pub autoeq: Option<Arc<AutoEqLibrary>>,
    /// The user's template folder ("" for the shipped templates only).
    pub template_dir: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            name: "dspi-console".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            read_limit: READ_LIMIT,
            write_limit: WRITE_LIMIT,
            policy: Arc::new(ReadOnlyPolicy),
            autoeq: None,                // -- WP MCP2b --
            template_dir: String::new(), // -- WP MCP2b --
        }
    }
}

/// One client.
struct Client {
    /// The actor its calls are made as, once it is known who calls.
    actor: Option<ActorId>,
    /// Its history label.
    label: String,
    /// The legacy version `initialize` agreed, if it initialised.
    legacy: Option<&'static str>,
    reads: Bucket,
    writes: Bucket,
}

/// The MCP server over a [`CoreSet`]. Sans-IO: give it messages.
pub struct Server {
    set: Arc<CoreSet>,
    active: Option<DeviceId>,
    registry: Registry,
    config: ServerConfig,
    clients: BTreeMap<ClientId, Client>,
    next_client: u64,
    activity: Activity,
    // -- WP MCP2a --
    confirms: Confirmations,
    /// The confirmation the request being handled opened.
    held: Option<u64>,
    guards: AgentGuards,
    drags: BTreeMap<String, DragWatch>,
    // -- WP MCP2c --
    /// The agents' Installation drafts, by client and hub.
    installation: crate::tools::installation::Drafts,
}

impl Server {
    /// A server over `set` with every tool this build has.
    pub fn new(set: Arc<CoreSet>, config: ServerConfig) -> Self {
        Self::with_registry(set, config, Registry::standard())
    }

    /// A server with these tools only.
    pub fn with_registry(set: Arc<CoreSet>, config: ServerConfig, registry: Registry) -> Self {
        Self {
            set,
            active: None,
            registry,
            config,
            clients: BTreeMap::new(),
            next_client: 0,
            activity: Activity::default(),
            confirms: Confirmations::default(),
            held: None,
            guards: AgentGuards::new(),
            drags: BTreeMap::new(),
            installation: crate::tools::installation::Drafts::default(), // -- WP MCP2c --
        }
    }

    /// The device the app shows, which tools use when no `device` is passed.
    pub fn set_active_device(&mut self, id: Option<DeviceId>) {
        self.active = id;
    }

    /// A new client (a connection, or a paired token's first request).
    pub fn connect(&mut self) -> ClientId {
        self.next_client = self.next_client.saturating_add(1);
        let id = ClientId(self.next_client);
        self.clients.insert(
            id,
            Client {
                actor: None,
                label: untrusted::actor_label(""),
                legacy: None,
                reads: Bucket::new(self.config.read_limit),
                writes: Bucket::new(self.config.write_limit),
            },
        );
        id
    }

    // -- WP MCP4 --
    /// A new client the user paired as `name`: its label in the History, the
    /// activity and the confirmation sheets is that name, whatever the agent
    /// calls itself.
    pub fn connect_as(&mut self, name: &str) -> ClientId {
        let id = self.connect();
        if let Some(c) = self.clients.get_mut(&id) {
            c.label = untrusted::actor_label(name);
            c.actor = Some(register_actor(&c.label));
        }
        id
    }

    /// The client is gone. Its activity stays; its waiting confirmations end
    /// with nothing changed (-- WP MCP2a --).
    pub fn disconnect(&mut self, client: ClientId) {
        self.clients.remove(&client);
        self.installation.forget(client); // -- WP MCP2c --
        for id in self.confirms.of_client(client) {
            self.confirms.end(
                id,
                ConfirmState::Gone,
                tool_error("the agent disconnected; nothing was changed"),
            );
        }
    }

    /// The client's actor, once it has called or named itself.
    pub fn actor_of(&self, client: ClientId) -> Option<ActorId> {
        self.clients.get(&client).and_then(|c| c.actor)
    }

    /// The client's history label.
    pub fn label_of(&self, client: ClientId) -> Option<String> {
        self.clients.get(&client).map(|c| c.label.clone())
    }

    /// The newest `n` activity entries of every client, oldest first.
    pub fn activity(&self, n: usize) -> Vec<ActivityEntry> {
        self.activity.recent(n, None)
    }

    /// One message as text: the reply as text, `None` for a notification or a
    /// response.
    pub fn handle(&mut self, client: ClientId, now_ms: u64, text: &str) -> Option<String> {
        let reply = match serde_json::from_str::<Value>(text) {
            Ok(msg) => self.handle_value(client, now_ms, msg),
            Err(_) => Some(protocol::error_reply(
                &Value::Null,
                &RpcError::new(PARSE_ERROR, "not JSON"),
            )),
        };
        reply.map(|r| r.to_string())
    }

    /// One message: the reply, `None` for a notification or a response.
    pub fn handle_value(&mut self, client: ClientId, now_ms: u64, msg: Value) -> Option<Value> {
        let null = Value::Null;
        let Value::Object(msg) = msg else {
            let why = if msg.is_array() {
                "batches are not supported: send one message at a time"
            } else {
                "a message is a JSON object"
            };
            return Some(protocol::error_reply(
                &null,
                &RpcError::new(INVALID_REQUEST, why),
            ));
        };
        let id = msg.get("id");
        if msg.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            let id = id.filter(|i| protocol::valid_id(i)).unwrap_or(&null);
            return Some(protocol::error_reply(
                id,
                &RpcError::new(INVALID_REQUEST, "`jsonrpc` must be \"2.0\""),
            ));
        }
        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            // A response: the server sends no requests, so there is nothing
            // to match it to.
            return None;
        };
        let Some(id) = id else {
            self.notification(client, method, msg.get("params"));
            return None;
        };
        if !protocol::valid_id(id) {
            return Some(protocol::error_reply(
                &null,
                &RpcError::new(INVALID_REQUEST, "`id` must be a string or an integer"),
            ));
        }
        let empty = Value::Object(Map::new());
        let params = msg.get("params").unwrap_or(&empty);
        let Some(params) = params.as_object() else {
            return Some(protocol::error_reply(
                id,
                &RpcError::new(INVALID_PARAMS, "`params` must be an object"),
            ));
        };
        match self.request(client, now_ms, method, params) {
            Ok(result) => Some(protocol::reply(id, result)),
            Err(e) => Some(protocol::error_reply(id, &e)),
        }
    }

    fn notification(&mut self, client: ClientId, method: &str, params: Option<&Value>) {
        // `notifications/initialized` confirms what `initialize` set up.
        // Others are not for this server.
        // -- WP MCP4 -- A cancel finds nothing running (calls run to the end
        // before the next message) but a held request: its sheet goes down.
        // Two sessions of one client may use the same request id; then
        // neither is taken for the other's.
        if method != "notifications/cancelled" {
            return;
        }
        let Some(rpc_id) = params.and_then(|p| p.get("requestId")) else {
            return;
        };
        if let [id] = self.confirms.held_as(client, rpc_id).as_slice() {
            self.confirms.end(
                *id,
                ConfirmState::Gone,
                tool_error("cancelled by the agent; nothing was changed"),
            );
        }
    }

    fn request(
        &mut self,
        client: ClientId,
        now_ms: u64,
        method: &str,
        params: &Map<String, Value>,
    ) -> Result<Value, RpcError> {
        if !self.clients.contains_key(&client) {
            return Err(RpcError::new(INVALID_REQUEST, "unknown client"));
        }
        let meta = params.get("_meta").and_then(Value::as_object);
        let modern = meta.and_then(|m| m.get(META_VERSION));
        let era = if method == "initialize" {
            Era::Legacy
        } else if let Some(version) = modern {
            let version = version.as_str().unwrap_or("");
            if !MODERN_VERSIONS.contains(&version) {
                return Err(RpcError::unsupported_version(version));
            }
            let caps = meta.and_then(|m| m.get(META_CAPABILITIES));
            if !caps.is_some_and(Value::is_object) {
                return Err(RpcError::new(
                    INVALID_PARAMS,
                    format!("`_meta` lacks `{META_CAPABILITIES}`"),
                ));
            }
            if let Some(info) = meta.and_then(|m| m.get(META_CLIENT_INFO)) {
                self.name_client(client, info);
            }
            Era::Modern
        } else {
            let initialised = self
                .clients
                .get(&client)
                .is_some_and(|c| c.legacy.is_some());
            if !initialised && method != "ping" {
                return Err(RpcError::new(
                    INVALID_REQUEST,
                    format!(
                        "send `initialize` first, or the {} `_meta` in every request",
                        MODERN_VERSIONS.join("/")
                    ),
                ));
            }
            Era::Legacy
        };
        let mut result = match (era, method) {
            (Era::Legacy, "initialize") => self.initialize(client, params),
            (Era::Modern, "server/discover") => Ok(self.discover()),
            (_, "ping") => Ok(json!({})),
            (_, "tools/list") => {
                no_cursor(params)?;
                let tools: Vec<Value> = self.registry.tools().iter().map(|t| t.listing()).collect();
                Ok(json!({"tools": tools}))
            }
            (_, "tools/call") => self.call(client, now_ms, params),
            (_, "resources/list") => {
                no_cursor(params)?;
                Ok(json!({"resources": resources::list(&self.set)}))
            }
            (_, "resources/templates/list") => {
                no_cursor(params)?;
                Ok(json!({"resourceTemplates": resources::templates()}))
            }
            (_, "resources/read") => self.read_resource(client, params),
            (_, "prompts/list") => {
                no_cursor(params)?;
                Ok(json!({"prompts": prompts::list()}))
            }
            (_, "prompts/get") => prompts::get(params),
            _ => Err(RpcError::new(
                METHOD_NOT_FOUND,
                "no such method on this server",
            )),
        }?;
        if era == Era::Modern
            && let Some(r) = result.as_object_mut()
        {
            r.insert("resultType".into(), json!("complete"));
            if matches!(
                method,
                "tools/list" | "resources/list" | "resources/templates/list" | "prompts/list"
            ) {
                r.insert("ttlMs".into(), json!(LIST_TTL_MS));
                r.insert("cacheScope".into(), json!("private"));
            }
        }
        Ok(result)
    }

    fn server_info(&self) -> Value {
        json!({"name": self.config.name, "title": "DSPi Console", "version": self.config.version})
    }

    fn initialize(
        &mut self,
        client: ClientId,
        params: &Map<String, Value>,
    ) -> Result<Value, RpcError> {
        let asked = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or("");
        let version = LEGACY_VERSIONS
            .iter()
            .copied()
            .find(|v| *v == asked)
            .unwrap_or(LEGACY_VERSIONS[0]);
        if let Some(info) = params.get("clientInfo") {
            self.name_client(client, info);
        }
        if let Some(c) = self.clients.get_mut(&client) {
            c.legacy = Some(version);
        }
        Ok(json!({
            "protocolVersion": version,
            "capabilities": protocol::capabilities(Era::Legacy),
            "serverInfo": self.server_info(),
            "instructions": INSTRUCTIONS,
        }))
    }

    fn discover(&self) -> Value {
        let supported: Vec<&str> = MODERN_VERSIONS
            .iter()
            .chain(LEGACY_VERSIONS.iter())
            .copied()
            .collect();
        json!({
            "supportedVersions": supported,
            "capabilities": protocol::capabilities(Era::Modern),
            "instructions": INSTRUCTIONS,
            "ttlMs": LIST_TTL_MS,
            "cacheScope": "private",
            "_meta": {META_SERVER_INFO: self.server_info()},
        })
    }

    /// Name the client from its `clientInfo` (once: the first name sticks,
    /// so a client cannot pose as another later in the History).
    fn name_client(&mut self, client: ClientId, info: &Value) {
        let Some(c) = self.clients.get_mut(&client) else {
            return;
        };
        if c.actor.is_some() {
            return;
        }
        let name = info
            .get("title")
            .and_then(Value::as_str)
            .or_else(|| info.get("name").and_then(Value::as_str))
            .unwrap_or("");
        c.label = untrusted::actor_label(name);
        c.actor = Some(register_actor(&c.label));
    }

    /// The client's actor, registered now if it has none yet.
    fn actor(&mut self, client: ClientId) -> Option<(ActorId, String)> {
        let c = self.clients.get_mut(&client)?;
        let actor = *c.actor.get_or_insert_with(|| register_actor(&c.label));
        Some((actor, c.label.clone()))
    }

    fn call(
        &mut self,
        client: ClientId,
        now_ms: u64,
        params: &Map<String, Value>,
    ) -> Result<Value, RpcError> {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return Err(RpcError::new(INVALID_PARAMS, "`name` is required"));
        };
        let Some(tool) = self.registry.get(name) else {
            return Err(RpcError::new(
                INVALID_PARAMS,
                "no such tool; tools/list lists them",
            ));
        };
        let empty = Map::new();
        let args = match params.get("arguments") {
            None | Some(Value::Null) => &empty,
            Some(Value::Object(a)) => a,
            Some(_) => {
                return Err(RpcError::new(
                    INVALID_PARAMS,
                    "`arguments` must be an object",
                ));
            }
        };
        let Some((_, label)) = self.actor(client) else {
            return Err(RpcError::new(INVALID_REQUEST, "unknown client"));
        };
        self.tick(now_ms); // -- WP MCP2a --
        let mut entry = ActivityEntry {
            id: 0,
            at_ms: now_ms,
            client,
            client_label: label.clone(),
            tool: tool.name.to_owned(),
            device: None,
            write: tool.class == ToolClass::Write,
            outcome: Outcome::Ok,
            journal_ids: Vec::new(),
            detail: String::new(),
        };
        let taken = self.clients.get_mut(&client).map(|c| match tool.class {
            ToolClass::Read => c.reads.take(now_ms),
            ToolClass::Write => c.writes.take(now_ms),
        });
        if let Some(Err(wait_ms)) = taken {
            let msg = format!("too many calls: wait {wait_ms} ms, then try again");
            entry.outcome = Outcome::RateLimited;
            entry.detail = msg.clone();
            self.activity.push(entry);
            return Ok(tool_error(&msg));
        }
        if let Err(msg) = registry::check_args(&tool.listed_input_schema(), args) {
            entry.outcome = Outcome::Error;
            entry.detail = msg.clone();
            self.activity.push(entry);
            return Ok(tool_error(&msg));
        }
        // -- WP MCP2a -- the policy's judgement, holds and the drag retry
        self.held = None;
        Ok(self.run_tool(now_ms, tool, args, Vec::new(), entry))
    }

    // -- WP MCP2a --
    /// Run `tool` for the client of `entry` under `grant`, retry once past a
    /// stale drag, keep the guard book, hold what needs the user, and log it.
    fn run_tool(
        &mut self,
        now_ms: u64,
        tool: &'static Tool,
        args: &Map<String, Value>,
        grant: Vec<WriteKey>,
        mut entry: ActivityEntry,
    ) -> Value {
        let client = entry.client;
        let Some(actor) = self.clients.get(&client).and_then(|c| c.actor) else {
            return tool_error("unknown client");
        };
        let label = entry.client_label.clone();
        let regranted = !grant.is_empty();
        let mut out = self.run_once(client, actor, &label, tool, args, grant.clone());
        if out.judging.editing
            && let Some(dev) = out.device.clone()
            && self.drag_is_stale(&dev, now_ms)
        {
            if let Some(core) = self.device_core(&dev) {
                core.set_dragging(false);
            }
            self.drags.remove(&dev);
            out = self.run_once(client, actor, &label, tool, args, grant);
        }
        if let Some(dev) = out.device.clone() {
            if !out.judging.editing {
                self.drags.remove(&dev);
            }
            if let Some(g) = out.judging.guard_begun {
                self.guards.insert(dev.clone(), (client, g));
            }
            if out.judging.guard_ended.is_some() {
                self.guards.remove(&dev);
            }
        }
        entry.device.clone_from(&out.device);
        entry.journal_ids = out.journal_ids;
        let asked = !out.judging.asked.is_empty() && out.judging.refused.is_none();
        let result = match out.result {
            Err(_) if asked && regranted => {
                let msg = "the device changed since the user allowed this, so nothing more was \
                           changed; read it again and call the tool again";
                entry.outcome = Outcome::Error;
                entry.detail = msg.to_owned();
                tool_error(msg)
            }
            Err(_) if asked && self.confirms.waiting_of(client) >= WAITING_PER_CLIENT => {
                let msg = "the user has not yet answered this agent's earlier confirmations; \
                           wait for those (dspi_get_confirmation) before asking again";
                entry.outcome = Outcome::Error;
                entry.detail = msg.to_owned();
                tool_error(msg)
            }
            Err(_) if asked => {
                let id = self.confirms.open(
                    client,
                    &label,
                    tool.name,
                    out.device.clone(),
                    out.judging.asked,
                    args.clone(),
                    now_ms,
                );
                self.held = Some(id);
                entry.outcome = Outcome::Held;
                entry.detail = format!("waiting for the user's confirmation {id}");
                held_result(id)
            }
            Ok(data) => {
                // -- WP MCP4 -- the change in words, for the app's log
                if entry.write {
                    entry.detail = crate::activity::words(&data);
                }
                tool_ok(data)
            }
            Err(e) => {
                entry.outcome = Outcome::Error;
                entry.detail = e.0.clone();
                tool_error(&e.0)
            }
        };
        self.activity.push(entry);
        result
    }

    /// One run of `tool` through the registry (-- WP MCP2a --).
    fn run_once(
        &self,
        client: ClientId,
        actor: ActorId,
        label: &str,
        tool: &Tool,
        args: &Map<String, Value>,
        grant: Vec<WriteKey>,
    ) -> registry::RunOutcome {
        let ctx = RunCtx {
            set: &self.set,
            active: self.active.as_ref(),
            policy: self.config.policy.as_ref(),
            activity: &self.activity,
            client,
            actor,
            client_label: label,
            grant,
            confirmations: Some(&self.confirms),
            guards: Some(&self.guards),
            autoeq: self.config.autoeq.as_ref(), // -- WP MCP2b --
            template_dir: &self.config.template_dir,
            installation: Some(&self.installation), // -- WP MCP2c --
        };
        registry::run(&ctx, tool, args)
    }

    /// The open core with id text `dev`.
    fn device_core(&self, dev: &str) -> Option<Arc<DspiCore>> {
        DeviceId::parse(dev).and_then(|id| self.set.core(&id))
    }

    /// Whether the drag that refused an agent on `dev` is stale: marked for
    /// [`STALE_DRAG_MS`] with no change by the user meanwhile.
    fn drag_is_stale(&mut self, dev: &str, now_ms: u64) -> bool {
        let Some(core) = self.device_core(dev) else {
            return false;
        };
        let mark = core
            .history(200)
            .into_iter()
            .find(|e| e.actor == EMBEDDER)
            .map_or((0, 0), |e| (e.id, e.at_ms));
        let w = self.drags.entry(dev.to_owned()).or_insert(DragWatch {
            since_ms: now_ms,
            mark,
        });
        if w.mark != mark {
            *w = DragWatch {
                since_ms: now_ms,
                mark,
            };
            return false;
        }
        now_ms.saturating_sub(w.since_ms) >= STALE_DRAG_MS
    }

    // -- WP MCP2a --
    /// One message, holding a tool call that needs the user's word: the
    /// transport keeps the request open and calls [`Server::resume`] until it
    /// gives the reply (at the latest [`CONFIRM_HOLD_MS`] later).
    pub fn handle_held(&mut self, client: ClientId, now_ms: u64, text: &str) -> Handled {
        let Ok(msg) = serde_json::from_str::<Value>(text) else {
            return Handled::Reply(self.handle(client, now_ms, text));
        };
        match self.handle_held_value(client, now_ms, msg) {
            (_, Some(id)) => Handled::Held(id),
            (reply, None) => Handled::Reply(reply.map(|r| r.to_string())),
        }
    }

    // -- WP MCP4 --
    /// [`Server::handle_held`] on JSON values: the reply, or the confirmation
    /// the request is held for (the reply is then for nobody).
    pub fn handle_held_value(
        &mut self,
        client: ClientId,
        now_ms: u64,
        msg: Value,
    ) -> (Option<Value>, Option<u64>) {
        let rpc_id = msg.get("id").cloned().unwrap_or(Value::Null);
        let modern = msg
            .pointer("/params/_meta")
            .and_then(|m| m.get(META_VERSION))
            .is_some();
        self.held = None;
        let reply = self.handle_value(client, now_ms, msg);
        let held = self.held.take();
        if let Some(id) = held
            && let Some(c) = self.confirms.get_mut(id)
        {
            c.request = Some((rpc_id, modern));
        }
        (reply, held)
    }

    /// The reply to a request [`Server::handle_held`] held, once its
    /// confirmation has ended; `None` while it waits, and for an id that
    /// held no request.
    pub fn resume(&mut self, id: u64, now_ms: u64) -> Option<String> {
        self.tick(now_ms);
        self.resume_value(id).map(|r| r.to_string())
    }

    /// [`Server::resume`] as a JSON value, without the tick (-- WP MCP4 --).
    pub fn resume_value(&mut self, id: u64) -> Option<Value> {
        let c = self.confirms.get(id)?;
        if c.state == ConfirmState::Waiting || c.request.is_none() {
            return None;
        }
        let c = self.confirms.remove(id)?;
        Some(held_reply(
            c.request?,
            c.result.unwrap_or_else(|| tool_error("no outcome")),
        ))
    }

    // -- WP MCP4 --
    /// Stop holding the request of confirmation `id`, which still waits: the
    /// reply is `awaiting_confirmation`, and the agent asks
    /// `dspi_get_confirmation` from then on. For a transport that holds a
    /// request for less than [`CONFIRM_HOLD_MS`]. `None` when it holds none.
    pub fn release_held(&mut self, id: u64) -> Option<Value> {
        let c = self.confirms.get_mut(id)?;
        if c.state != ConfirmState::Waiting {
            return None;
        }
        let request = c.request.take()?;
        Some(held_reply(request, held_result(id)))
    }

    /// The connection of the request held for confirmation `id` is gone, so
    /// nobody is there for its answer: a waiting confirmation ends with
    /// nothing changed, and it is forgotten.
    pub fn abandon_held(&mut self, id: u64) {
        let Some(c) = self.confirms.get(id) else {
            return;
        };
        if c.request.is_none() {
            return;
        }
        if c.state == ConfirmState::Waiting {
            self.confirms.end(
                id,
                ConfirmState::Gone,
                tool_error("the agent went away; nothing was changed"),
            );
        }
        self.confirms.remove(id);
    }

    /// The user turned the agents off: every waiting confirmation ends with
    /// nothing changed, and every agent's timed preview is put back now.
    pub fn stop_agents(&mut self, now_ms: u64) {
        for id in self
            .confirms
            .waiting()
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>()
        {
            self.confirms.end(
                id,
                ConfirmState::Gone,
                tool_error("the user turned AI agents off; nothing was changed"),
            );
        }
        for (dev, (client, guard)) in std::mem::take(&mut self.guards) {
            let Some(core) = self.device_core(&dev) else {
                continue;
            };
            let label = self.label_of(client).unwrap_or_default();
            let actor = self.clients.get(&client).and_then(|c| c.actor);
            let status = core.apply_status();
            if status.id != guard || !matches!(status.phase, GuardPhase::Waiting | GuardPhase::Lost)
            {
                continue;
            }
            // As the agent with no hook, as `tick`: a revert is never refused.
            let done = {
                let _scope = actor.map(|a| ActorScope::enter(&core, a, None));
                core.apply_revert()
            };
            let (outcome, detail) = match done {
                Ok(g) if g.phase == GuardPhase::Reverted => {
                    (Outcome::Ok, "the timed preview was put back".to_owned())
                }
                Ok(g) => (
                    Outcome::Error,
                    format!("the timed preview was not put back: {}", g.message),
                ),
                Err(e) => (
                    Outcome::Error,
                    format!("the timed preview was not put back: {e}"),
                ),
            };
            self.activity.push(ActivityEntry {
                id: 0,
                at_ms: now_ms,
                client,
                client_label: label,
                tool: "dspi_revert_changes".to_owned(),
                device: Some(dev),
                write: true,
                outcome,
                journal_ids: Vec::new(),
                detail,
            });
        }
    }

    /// The title of tool `name`, "" for one this server does not have.
    pub fn tool_title(&self, name: &str) -> &'static str {
        self.registry.get(name).map_or("", |t| t.title)
    }

    /// The devices an agent's timed preview waits on.
    pub fn guarded_devices(&self) -> Vec<String> {
        self.guards.keys().cloned().collect()
    }

    /// The user's answer to confirmation `id` (the app's sheet). Allow runs
    /// the call now, with a grant for the writes the user saw.
    pub fn answer_confirmation(
        &mut self,
        id: u64,
        allow: bool,
        now_ms: u64,
    ) -> Result<(), AnswerError> {
        self.tick(now_ms);
        let c = self.confirms.get(id).ok_or(AnswerError::Unknown)?;
        if c.state != ConfirmState::Waiting {
            return Err(AnswerError::Ended(c.state));
        }
        let (client, tool_name, args, grant) = (c.client, c.tool, c.args.clone(), c.grant());
        let entry = ActivityEntry {
            id: 0,
            at_ms: now_ms,
            client,
            client_label: c.client_label.clone(),
            tool: tool_name.to_owned(),
            device: c.device.clone(),
            write: true,
            outcome: Outcome::Ok,
            journal_ids: Vec::new(),
            detail: String::new(),
        };
        let tool = self.registry.get(tool_name);
        let gone = !self.clients.contains_key(&client);
        let closed = c
            .device
            .as_deref()
            .is_some_and(|d| self.device_core(d).is_none());
        let (state, result) = match tool {
            _ if !allow => {
                let msg = "the user declined this change in the app; nothing was changed. \
                           Do not try it again unless the user asks.";
                self.activity.push(ActivityEntry {
                    outcome: Outcome::Error,
                    detail: format!("declined by the user (confirmation {id})"),
                    ..entry
                });
                (ConfirmState::Denied, tool_error(msg))
            }
            Some(_) if gone => (
                ConfirmState::Gone,
                tool_error("the agent disconnected; nothing was changed"),
            ),
            None => (ConfirmState::Gone, tool_error("no such tool")),
            // -- WP RBFIX -- (review RB R1) never on another device
            Some(_) if closed => {
                let msg = "the device this was asked for is no longer open; nothing was changed";
                self.activity.push(ActivityEntry {
                    outcome: Outcome::Error,
                    detail: format!("{msg} (confirmation {id})"),
                    ..entry
                });
                (ConfirmState::Gone, tool_error(msg))
            }
            Some(tool) => {
                // A re-run has a grant, so it never holds anew.
                let result = self.run_tool(now_ms, tool, &args, grant, entry);
                (ConfirmState::Allowed, result)
            }
        };
        self.confirms.end(id, state, result);
        Ok(())
    }

    /// Confirmation `id`, waiting or ended (kept a while).
    pub fn confirmation(&self, id: u64) -> Option<&Confirmation> {
        self.confirms.get(id)
    }

    /// The confirmations waiting for the user, oldest first, as the sheet
    /// shows them.
    pub fn waiting_confirmations(&self) -> Vec<Value> {
        self.confirms
            .waiting()
            .into_iter()
            .map(Confirmation::to_json)
            .collect()
    }

    /// What the app should hear of since it last asked.
    pub fn take_confirm_events(&mut self) -> Vec<ConfirmEvent> {
        self.confirms.take_events()
    }

    /// Drive the clocks: lapse unanswered confirmations, and the agents'
    /// guarded applies (the revert at their timeout). Call it from the app's
    /// poll; every request does too. A guard's poll runs as its agent with no
    /// hook (a revert is never refused), or as the app once the agent is
    /// gone.
    pub fn tick(&mut self, now_ms: u64) {
        for id in self.confirms.lapsed(now_ms) {
            if let Some(c) = self.confirms.get(id) {
                self.activity.push(ActivityEntry {
                    id: 0,
                    at_ms: now_ms,
                    client: c.client,
                    client_label: c.client_label.clone(),
                    tool: c.tool.to_owned(),
                    device: c.device.clone(),
                    write: true,
                    outcome: Outcome::Error,
                    journal_ids: Vec::new(),
                    detail: format!("not answered in time (confirmation {id})"),
                });
            }
            let secs = CONFIRM_HOLD_MS / 1000;
            self.confirms.end(
                id,
                ConfirmState::Expired,
                tool_error(&format!(
                    "the user did not answer within {secs} s; nothing was changed"
                )),
            );
        }
        let guards: Vec<(String, (ClientId, u64))> =
            self.guards.iter().map(|(d, g)| (d.clone(), *g)).collect();
        for (dev, (client, guard)) in guards {
            let Some(core) = self.device_core(&dev) else {
                self.guards.remove(&dev);
                continue;
            };
            let actor = self.clients.get(&client).and_then(|c| c.actor);
            let status = {
                let _scope = actor.map(|a| ActorScope::enter(&core, a, None));
                core.apply_poll()
            };
            if status.id != guard || !matches!(status.phase, GuardPhase::Waiting | GuardPhase::Lost)
            {
                self.guards.remove(&dev);
            }
        }
    }

    fn read_resource(
        &mut self,
        client: ClientId,
        params: &Map<String, Value>,
    ) -> Result<Value, RpcError> {
        let Some(uri) = params.get("uri").and_then(Value::as_str) else {
            return Err(RpcError::new(INVALID_PARAMS, "`uri` is required"));
        };
        let Some((actor, label)) = self.actor(client) else {
            return Err(RpcError::new(INVALID_REQUEST, "unknown client"));
        };
        let ctx = resources::ReadCtx {
            set: &self.set,
            policy: self.config.policy.as_ref(),
            client,
            actor,
            client_label: &label,
            version: &self.config.version,
        };
        let contents = resources::read(&ctx, uri)?;
        Ok(json!({"contents": contents}))
    }
}

/// A list request with a `cursor` this server never gave (it pages nothing).
fn no_cursor(params: &Map<String, Value>) -> Result<(), RpcError> {
    match params.get("cursor") {
        None | Some(Value::Null) => Ok(()),
        Some(_) => Err(RpcError::new(INVALID_PARAMS, "unknown cursor")),
    }
}

/// A tool's success: the structure, and the same as text for hosts that
/// ignore structure.
fn tool_ok(data: Value) -> Value {
    json!({
        "content": [{"type": "text", "text": data.to_string()}],
        "structuredContent": data,
        "isError": false,
    })
}

/// A tool's error, for the model to read and correct.
fn tool_error(msg: &str) -> Value {
    json!({"content": [{"type": "text", "text": msg}], "isError": true})
}

// -- WP MCP4 --
/// The JSON-RPC reply to a held request: its id, whether modern-era.
fn held_reply((rpc_id, modern): (Value, bool), mut result: Value) -> Value {
    if modern && let Some(r) = result.as_object_mut() {
        r.insert("resultType".into(), json!("complete"));
    }
    protocol::reply(&rpc_id, result)
}

// -- WP MCP2a --
/// A held call's answer for a transport that cannot hold it: not an error;
/// the agent asks `dspi_get_confirmation` for the outcome.
fn held_result(id: u64) -> Value {
    let secs = CONFIRM_HOLD_MS / 1000;
    let data = json!({
        "status": "awaiting_confirmation",
        "confirmation_id": id,
        "expires_in_s": secs,
    });
    let text = format!(
        "The user must confirm this change in the app (confirmation {id}, up to {secs} s). \
         Nothing has been changed yet. Call dspi_get_confirmation with confirmation_id {id} \
         for the outcome."
    );
    json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": data,
        "isError": false,
    })
}
