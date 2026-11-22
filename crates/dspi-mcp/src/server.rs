//! The server: its clients, the dispatch of both eras, the rate limits and
//! the activity log.

use std::collections::BTreeMap;
use std::sync::Arc;

use dspi_core::actor::{ActorId, register_actor};
use dspi_core::{CoreSet, DeviceId};
use serde_json::{Map, Value, json};

use crate::activity::{Activity, ActivityEntry, Outcome};
use crate::protocol::{
    self, Era, INSTRUCTIONS, INVALID_PARAMS, INVALID_REQUEST, LEGACY_VERSIONS, LIST_TTL_MS,
    META_CAPABILITIES, META_CLIENT_INFO, META_SERVER_INFO, META_VERSION, METHOD_NOT_FOUND,
    MODERN_VERSIONS, PARSE_ERROR, RpcError,
};
use crate::rate::{Bucket, READ_LIMIT, RateLimit, WRITE_LIMIT};
use crate::registry::{self, Policy, ReadOnlyPolicy, Registry, RunCtx, ToolClass};
use crate::{ClientId, prompts, resources, untrusted};

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
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            name: "dspi-console".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            read_limit: READ_LIMIT,
            write_limit: WRITE_LIMIT,
            policy: Arc::new(ReadOnlyPolicy),
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

    /// The client is gone. Its activity stays.
    pub fn disconnect(&mut self, client: ClientId) {
        self.clients.remove(&client);
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
            self.notification(client, method);
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

    fn notification(&mut self, client: ClientId, method: &str) {
        // `notifications/initialized` confirms what `initialize` set up, and
        // calls run to the end before the next message, so a cancel finds
        // nothing to stop. Others are not for this server.
        let _ = (client, method);
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
        let Some((actor, label)) = self.actor(client) else {
            return Err(RpcError::new(INVALID_REQUEST, "unknown client"));
        };
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
        let ctx = RunCtx {
            set: &self.set,
            active: self.active.as_ref(),
            policy: self.config.policy.as_ref(),
            activity: &self.activity,
            client,
            actor,
            client_label: &label,
        };
        let out = registry::run(&ctx, tool, args);
        entry.device = out.device;
        entry.journal_ids = out.journal_ids;
        let result = match out.result {
            Ok(data) => tool_ok(data),
            Err(e) => {
                entry.outcome = Outcome::Error;
                entry.detail = e.0.clone();
                tool_error(&e.0)
            }
        };
        self.activity.push(entry);
        Ok(result)
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
