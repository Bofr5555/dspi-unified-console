//! The tools, and the one place a tool call runs.
//!
//! A [`Tool`] is static data plus a handler. [`Registry::standard`] lists
//! them in a fixed order (`tools/list` order, a SHOULD of 2026-07-28); each
//! later WP adds its module's list under its own marker.
//!
//! **Every call runs in [`run`].** For a tool that addresses a device,
//! `run` resolves the `device` argument to an open core and makes the call
//! inside an [`ActorScope`] on that core for the client's actor, with the
//! [`Policy`]'s pre-write hook. The handler gets the core only through
//! [`Call::core`], which exists only inside that scope, so no tool can reach
//! a core as the user (a thread without a scope acts as the embedder, review
//! RA MCP0 item 7). A tool that addresses no device gets no core at all.

use std::sync::Arc;

use dspi_core::actor::{ActorId, ActorScope, WriteHook};
use dspi_core::{CoreSet, DeviceId, DspiCore, DspiError, OpenDevice};
use serde_json::{Map, Value, json};

use crate::activity::Activity;
use crate::untrusted;
use crate::{ClientId, tools};

/// What a tool may do. Whether a given write is risky is the policy's
/// judgement per write (MCP0's hook), not the tool's class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolClass {
    /// Reads only; never asks.
    Read,
    /// Writes through the core, each write judged by the policy's hook.
    Write,
}

/// A tool's handler: the call's context and its checked arguments, to a
/// result or an error the model can read.
pub type Handler = fn(&mut Call<'_>, &Map<String, Value>) -> ToolResult;

/// One tool.
pub struct Tool {
    /// `dspi_…`.
    pub name: &'static str,
    /// A title for a host's UI.
    pub title: &'static str,
    /// What it does, for the model: the server's own words only.
    pub description: &'static str,
    /// Its class.
    pub class: ToolClass,
    /// It addresses a device: it takes `device` and runs in a scope on it.
    pub device: bool,
    /// Its arguments' JSON Schema, without `device` (added for a device tool).
    pub input_schema: fn() -> Value,
    /// Its result's JSON Schema.
    pub output_schema: fn() -> Value,
    /// The handler.
    pub handler: Handler,
}

impl Tool {
    /// The arguments' schema as listed: `device` added for a device tool.
    pub fn listed_input_schema(&self) -> Value {
        let mut schema = (self.input_schema)();
        if self.device
            && let Some(props) = schema.get_mut("properties").and_then(Value::as_object_mut)
        {
            props.insert(
                "device".into(),
                json!({"type": "string", "description":
                    "The device's id from dspi_list_devices; default: the device the app shows"}),
            );
        }
        schema
    }

    /// The `tools/list` entry.
    pub fn listing(&self) -> Value {
        let read = self.class == ToolClass::Read;
        json!({
            "name": self.name,
            "title": self.title,
            "description": self.description,
            "inputSchema": self.listed_input_schema(),
            "outputSchema": (self.output_schema)(),
            "annotations": {
                "title": self.title,
                "readOnlyHint": read,
                "destructiveHint": false,
                "idempotentHint": read,
                "openWorldHint": false,
            },
        })
    }
}

/// The tools in `tools/list` order.
pub struct Registry {
    tools: Vec<&'static Tool>,
}

impl Registry {
    /// Every tool this build serves.
    pub fn standard() -> Self {
        let mut tools: Vec<&'static Tool> = Vec::new();
        // -- WP MCP1 -- the read tools
        tools.extend(tools::read::TOOLS.iter().copied());
        // -- WP MCP2a --
        // -- WP MCP2b --
        // -- WP MCP2c --
        // -- WP MCP5 --
        Self { tools }
    }

    /// These tools only (tests).
    pub fn of(tools: Vec<&'static Tool>) -> Self {
        Self { tools }
    }

    /// The tools, in order.
    pub fn tools(&self) -> &[&'static Tool] {
        &self.tools
    }

    /// The tool named `name`.
    pub fn get(&self, name: &str) -> Option<&'static Tool> {
        self.tools.iter().copied().find(|t| t.name == name)
    }
}

/// Who calls, for the [`Policy`].
#[derive(Debug, Clone)]
pub struct CallInfo<'a> {
    /// The client.
    pub client: ClientId,
    /// Its actor on every core.
    pub actor: ActorId,
    /// Its label ("AI · Claude Code").
    pub client_label: &'a str,
    /// The tool.
    pub tool: &'a str,
    /// The tool's class.
    pub class: ToolClass,
    /// The device addressed, for a device tool.
    pub device: Option<&'a DeviceId>,
}

/// The agent policy (MCP2a): what an agent's writes may do. MCP1 fixes the
/// interface; [`ReadOnlyPolicy`] is the default.
pub trait Policy: Send + Sync {
    /// The pre-write hook for this call's scope. It runs under the core's
    /// lock and must not call the core (`dspi_core::actor`). `None` lets
    /// every write through as far as the core's own rules allow.
    fn write_hook(&self, call: &CallInfo<'_>) -> Option<WriteHook>;

    /// What the overview says an agent may do ("agent may" flags).
    fn agent_may(&self) -> Value;
}

/// [`DspiError::ConfirmationRequired`]'s code from [`ReadOnlyPolicy`]: an
/// agent may not write at all. No confirmation allows it.
pub const AGENT_READ_ONLY: &str = "agent_read_only";

/// Refuses every write: the policy until MCP2a's. Read tools run under it
/// too, so a read tool that wrote by mistake would be refused.
#[derive(Debug, Default, Clone, Copy)]
pub struct ReadOnlyPolicy;

impl Policy for ReadOnlyPolicy {
    fn write_hook(&self, _call: &CallInfo<'_>) -> Option<WriteHook> {
        Some(Arc::new(|_p| {
            Err(DspiError::ConfirmationRequired {
                code: AGENT_READ_ONLY.into(),
                msg: "AI agents may only read in this version".into(),
            })
        }))
    }

    fn agent_may(&self) -> Value {
        json!({"read": true, "write": false, "measure": false})
    }
}

/// A tool's outcome: structured content, or a message for the model.
pub type ToolResult = Result<Value, ToolError>;

/// A tool-level error (`isError: true`): the call was well formed, and the
/// text says why it failed and what to do. Never untrusted text.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolError(pub String);

impl ToolError {
    /// From the server's own words.
    pub fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

impl From<DspiError> for ToolError {
    /// A core error in the core's words, cleaned (they carry no names, but
    /// a device's own text could reach one).
    fn from(e: DspiError) -> Self {
        Self(untrusted::note(&format!("the device refused: {e}")))
    }
}

/// A device-less tool's view of the set: the devices, never a core.
#[derive(Debug, Clone)]
pub struct DeviceRow {
    /// As the set lists it.
    pub open: OpenDevice,
    /// The firmware version ("1.1.6"), "" for none.
    pub firmware: String,
    /// It is the device the app shows.
    pub active: bool,
}

/// One tool call's context.
pub struct Call<'a> {
    /// The client.
    pub client: ClientId,
    /// Its actor.
    pub actor: ActorId,
    /// The device addressed (a device tool).
    pub device: Option<DeviceId>,
    /// Its unit's serial, as the set lists it ("" for a device-less tool).
    pub serial: String,
    /// The open devices (a device-less tool; empty for a device tool).
    pub devices: Vec<DeviceRow>,
    /// The activity log, for `dspi_get_activity`.
    pub activity: &'a Activity,
    /// The policy, for the overview's flags.
    pub policy: &'a dyn Policy,
    core: Option<&'a DspiCore>,
}

impl Call<'_> {
    /// The addressed core. Only a device tool has one, and only inside its
    /// scope.
    pub fn core(&self) -> Result<&DspiCore, ToolError> {
        self.core
            .ok_or_else(|| ToolError::new("this tool addresses no device"))
    }

    /// The device's id as text.
    pub fn device_text(&self) -> String {
        self.device
            .as_ref()
            .map(|d| d.as_str().to_owned())
            .unwrap_or_default()
    }
}

/// What [`run`] needs from the server.
pub struct RunCtx<'a> {
    /// The devices.
    pub set: &'a CoreSet,
    /// The device the app shows.
    pub active: Option<&'a DeviceId>,
    /// The policy.
    pub policy: &'a dyn Policy,
    /// The activity log.
    pub activity: &'a Activity,
    /// The client.
    pub client: ClientId,
    /// Its actor.
    pub actor: ActorId,
    /// Its label.
    pub client_label: &'a str,
}

/// What [`run`] did.
#[derive(Debug)]
pub struct RunOutcome {
    /// The device addressed, as text.
    pub device: Option<String>,
    /// The tool's result.
    pub result: ToolResult,
    /// The journal ids of the changes the call made on its device.
    pub journal_ids: Vec<u64>,
}

/// The device `args` name, or the default: the app's device when it is
/// open, else the only device open.
fn resolve(
    set: &CoreSet,
    active: Option<&DeviceId>,
    args: &Map<String, Value>,
) -> Result<(DeviceId, String, Arc<DspiCore>), ToolError> {
    let open = set.list();
    let id = match args.get("device") {
        Some(Value::String(text)) => {
            let found = DeviceId::parse(text).filter(|id| open.iter().any(|d| d.id == *id));
            found.ok_or_else(|| {
                ToolError::new("no open device has that id; dspi_list_devices lists them")
            })?
        }
        Some(_) => return Err(ToolError::new("`device` must be a string")),
        None => match active.filter(|a| open.iter().any(|d| d.id == **a)) {
            Some(a) => a.clone(),
            None => match open.as_slice() {
                [only] => only.id.clone(),
                [] => return Err(ToolError::new("no device is open in the app")),
                _ => {
                    return Err(ToolError::new(
                        "several devices are open; pass `device` (see dspi_list_devices)",
                    ));
                }
            },
        },
    };
    let core = set
        .core(&id)
        .ok_or_else(|| ToolError::new("that device has just been closed"))?;
    let serial = open
        .iter()
        .find(|d| d.id == id)
        .map(|d| d.serial.clone())
        .unwrap_or_default();
    Ok((id, serial, core))
}

/// The firmware version of `core`, "" when nothing is connected.
fn firmware(core: &DspiCore) -> String {
    if !core.is_connected() {
        return String::new();
    }
    let s = core.get_state();
    format!("{}.{}.{}", s.fw_major, s.fw_minor, s.fw_patch)
}

/// The newest journal id on `core`, 0 for none.
fn newest_id(core: &DspiCore) -> u64 {
    core.history(1).first().map_or(0, |e| e.id)
}

/// Run `tool` with `args` (already checked against its schema): the one
/// place a tool call runs (module docs).
pub fn run(ctx: &RunCtx<'_>, tool: &Tool, args: &Map<String, Value>) -> RunOutcome {
    if !tool.device {
        let devices = ctx
            .set
            .list()
            .into_iter()
            .map(|open| {
                let firmware = ctx
                    .set
                    .core(&open.id)
                    .map(|c| firmware(&c))
                    .unwrap_or_default();
                let active = ctx.active == Some(&open.id);
                DeviceRow {
                    open,
                    firmware,
                    active,
                }
            })
            .collect();
        let mut call = Call {
            client: ctx.client,
            actor: ctx.actor,
            device: None,
            serial: String::new(),
            devices,
            activity: ctx.activity,
            policy: ctx.policy,
            core: None,
        };
        let result = (tool.handler)(&mut call, args);
        return RunOutcome {
            device: None,
            result,
            journal_ids: Vec::new(),
        };
    }
    let (id, serial, core) = match resolve(ctx.set, ctx.active, args) {
        Ok(found) => found,
        Err(e) => {
            return RunOutcome {
                device: None,
                result: Err(e),
                journal_ids: Vec::new(),
            };
        }
    };
    let info = CallInfo {
        client: ctx.client,
        actor: ctx.actor,
        client_label: ctx.client_label,
        tool: tool.name,
        class: tool.class,
        device: Some(&id),
    };
    let hook = ctx.policy.write_hook(&info);
    let before = newest_id(&core);
    let result = {
        let _scope = ActorScope::enter(&core, ctx.actor, hook);
        let mut call = Call {
            client: ctx.client,
            actor: ctx.actor,
            device: Some(id.clone()),
            serial,
            devices: Vec::new(),
            activity: ctx.activity,
            policy: ctx.policy,
            core: Some(&core),
        };
        (tool.handler)(&mut call, args)
    };
    // The changes this call made: this actor's entries newer than before.
    let mut journal_ids: Vec<u64> = core
        .history(HISTORY_SCAN)
        .into_iter()
        .filter(|e| e.id > before && e.actor == ctx.actor)
        .map(|e| e.id)
        .collect();
    journal_ids.sort_unstable();
    journal_ids.dedup();
    RunOutcome {
        device: Some(id.as_str().to_owned()),
        result,
        journal_ids,
    }
}

/// How many history entries [`run`] looks through for a call's changes.
const HISTORY_SCAN: u32 = 200;

// ------------------------------------------------------------ the arguments

/// Check `args` against a tool's input schema, in the subset the tools use:
/// an object of named properties, `required`, `additionalProperties: false`,
/// and per property `type` (`integer`, `number`, `string`, `boolean`),
/// `enum`, `minimum` and `maximum`. The message names the argument, never
/// its value.
pub fn check_args(schema: &Value, args: &Map<String, Value>) -> Result<(), String> {
    let props = schema.get("properties").and_then(Value::as_object);
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for r in required.iter().filter_map(Value::as_str) {
            if !args.contains_key(r) {
                return Err(format!("`{r}` is required"));
            }
        }
    }
    let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
    for (key, value) in args {
        let Some(spec) = props.and_then(|p| p.get(key)) else {
            if closed {
                return Err(
                    if key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && key.len() <= 40
                    {
                        format!("no argument `{key}`")
                    } else {
                        "an argument this tool does not have".to_owned()
                    },
                );
            }
            continue;
        };
        check_value(key, spec, value)?;
    }
    Ok(())
}

fn check_value(key: &str, spec: &Value, value: &Value) -> Result<(), String> {
    if let Some(ty) = spec.get("type").and_then(Value::as_str) {
        let ok = match ty {
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "number" => value.as_f64().is_some_and(f64::is_finite),
            "string" => value.is_string(),
            "boolean" => value.is_boolean(),
            "array" => value.is_array(),
            "object" => value.is_object(),
            _ => true,
        };
        if !ok {
            return Err(format!("`{key}` must be of type {ty}"));
        }
    }
    if let Some(options) = spec.get("enum").and_then(Value::as_array)
        && !options.contains(value)
    {
        let words: Vec<String> = options.iter().map(Value::to_string).collect();
        return Err(format!("`{key}` must be one of {}", words.join(", ")));
    }
    if let Some(n) = value.as_f64() {
        if let Some(min) = spec.get("minimum").and_then(Value::as_f64)
            && n < min
        {
            return Err(format!("`{key}` must be at least {min}"));
        }
        if let Some(max) = spec.get("maximum").and_then(Value::as_f64)
            && n > max
        {
            return Err(format!("`{key}` must be at most {max}"));
        }
    }
    Ok(())
}
