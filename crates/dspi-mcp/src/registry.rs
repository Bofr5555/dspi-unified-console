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

use std::collections::BTreeMap;
use std::sync::Arc;

use dspi_core::actor::{ActorId, ActorScope, WriteHook};
use dspi_core::{AutoEqLibrary, CoreSet, DeviceId, DspiCore, DspiError, OpenDevice};
use serde_json::{Map, Value, json};

use crate::activity::Activity;
use crate::confirm::Confirmations; // -- WP MCP2a --
use crate::host_audio::HostAudioSource; // -- WP FEAT3 --
use crate::policy::{self, Judging, JudgingCell, WriteKey}; // -- WP MCP2a --
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
        // -- WP MCP2a -- the edit tools, the batch and its guard
        tools.extend(tools::edit::TOOLS.iter().copied());
        tools.extend(tools::apply::TOOLS.iter().copied());
        // -- WP MCP2b -- effects, presets and snapshots, files, AutoEQ, names
        tools.extend(tools::effects::TOOLS.iter().copied());
        tools.extend(tools::library::TOOLS.iter().copied());
        tools.extend(tools::files::TOOLS.iter().copied());
        tools.extend(tools::autoeq::TOOLS.iter().copied());
        tools.extend(tools::names::TOOLS.iter().copied());
        // -- WP MCP2c -- the audio link, Installation setup, hubs, analysis,
        // limits and firmware (the hub-side ones need the feature `link`)
        tools.extend(tools::alink::TOOLS.iter().copied());
        tools.extend(tools::installation::TOOLS.iter().copied());
        tools.extend(tools::hub::TOOLS.iter().copied());
        tools.extend(tools::analysis::TOOLS.iter().copied());
        tools.extend(tools::info::TOOLS.iter().copied());
        // -- WP MCP5 -- measuring, the auto-tune, measuring across units
        tools.extend(tools::measure::TOOLS.iter().copied());
        // -- WP FEAT1 -- routing and sources in the installation draft
        tools.extend(tools::installation::FEAT1_TOOLS.iter().copied());
        // -- WP FEAT2 -- a room's volume, the hub's schedule (feature `link`),
        // opening known devices
        tools.extend(tools::site::TOOLS.iter().copied());
        tools.extend(tools::devices::TOOLS.iter().copied());
        // -- WP FEAT3 -- this computer's audio, read only
        tools.extend(tools::host::TOOLS.iter().copied());
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

    // -- WP MCP2a --
    /// The hook for one call on `core`, sharing `judging` with the tool
    /// ([`policy::Judging`]: the setting, the user's roles, the grant, what
    /// was asked). Called before the scope, outside the core's lock, so it
    /// may read the core. The default ignores both and gives
    /// [`Policy::write_hook`].
    fn judging_hook(
        &self,
        call: &CallInfo<'_>,
        core: &DspiCore,
        judging: &JudgingCell,
    ) -> Option<WriteHook> {
        let _ = (core, judging);
        self.write_hook(call)
    }

    // -- WP FEAT2 --
    /// The setting for a call that addresses no device (opening one), which
    /// has no core to hook: `None` (the default) lets it change nothing.
    fn setting(&self) -> Option<policy::ConfirmMode> {
        None
    }
}

// -- WP MCP2a --
/// The guarded applies agents began, by device id: the client and the
/// guard's id.
pub type AgentGuards = BTreeMap<String, (ClientId, u64)>;

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
    // -- WP MCP2c --
    /// For a device on a DSPi Link hub: this session's role and encryption.
    pub hub: Option<HubRow>,
}

// -- WP MCP2c --
/// What a device-less tool may know of a device's hub session.
#[derive(Debug, Clone, Default)]
pub struct HubRow {
    /// The role the hub gave the session: `viewer`, `control`, `admin`.
    pub role: String,
    /// `verified`, `offered`, `off`, `local` or `unknown`.
    pub encryption: String,
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
    // -- WP MCP2a --
    /// This call's judgement, shared with the hook.
    pub judging: JudgingCell,
    /// The held calls (`dspi_get_confirmation`).
    pub confirmations: Option<&'a Confirmations>,
    /// The guarded apply this client began on the device, if one waits.
    pub agent_guard: Option<u64>,
    // -- WP MCP2b --
    /// The app's AutoEQ database.
    pub autoeq: Option<&'a Arc<AutoEqLibrary>>,
    /// The user's template folder.
    pub template_dir: &'a str,
    // -- WP MCP2c --
    /// The agents' Installation drafts.
    pub installation: Option<&'a tools::installation::Drafts>,
    // -- WP MCP5 --
    /// The agents' requests to the app's Measure view.
    pub measure: Option<&'a tools::measure::Jobs>,
    // -- WP FEAT2 --
    /// The agents' requests to open and close devices.
    pub opens: Option<&'a tools::devices::Opens>,
    /// The client's label ("AI · Claude Code").
    pub client_label: String,
    /// The devices, for a tool over several units (measuring across units,
    /// -- WP FEAT2 -- a room's volume, the known devices): it reaches another
    /// unit's core only inside a scope of its own.
    pub set: Option<&'a CoreSet>,
    // -- WP FEAT3 --
    /// Reads this computer's audio (`dspi_get_host_audio`).
    pub host_audio: Option<&'a Arc<dyn HostAudioSource>>,
    core: Option<&'a DspiCore>,
}

impl Call<'_> {
    /// The addressed core. Only a device tool has one, and only inside its
    /// scope.
    pub fn core(&self) -> Result<&DspiCore, ToolError> {
        self.core
            .ok_or_else(|| ToolError::new("this tool addresses no device"))
    }

    // -- WP FEAT2 --
    /// What an agent may do here: the policy's flags, and whether this app
    /// takes the agents' measurement and device requests.
    pub fn agent_may(&self) -> Value {
        let mut may = self.policy.agent_may();
        if let Some(m) = may.as_object_mut() {
            let write = m.get("write").and_then(Value::as_bool).unwrap_or(false);
            let policy = m.get("measure").and_then(Value::as_bool).unwrap_or(false);
            let host = self.measure.is_some_and(tools::measure::Jobs::host);
            m.insert("measure".into(), json!(policy && host));
            let opens = self.opens.is_some_and(tools::devices::Opens::host);
            m.insert("open_devices".into(), json!(write && opens));
        }
        may
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
    // -- WP MCP2a --
    /// The writes the user allowed for this call (a re-run after Allow).
    pub grant: Vec<WriteKey>,
    /// The held calls.
    pub confirmations: Option<&'a Confirmations>,
    /// The agents' guarded applies.
    pub guards: Option<&'a AgentGuards>,
    // -- WP MCP2b --
    /// The app's AutoEQ database.
    pub autoeq: Option<&'a Arc<AutoEqLibrary>>,
    /// The user's template folder.
    pub template_dir: &'a str,
    // -- WP MCP2c --
    /// The agents' Installation drafts.
    pub installation: Option<&'a tools::installation::Drafts>,
    // -- WP MCP5 --
    /// The agents' requests to the app's Measure view.
    pub measure: Option<&'a tools::measure::Jobs>,
    // -- WP FEAT2 --
    /// The agents' requests to open and close devices.
    pub opens: Option<&'a tools::devices::Opens>,
    // -- WP FEAT3 --
    /// Reads this computer's audio.
    pub host_audio: Option<&'a Arc<dyn HostAudioSource>>,
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
    /// What the policy found (-- WP MCP2a --).
    pub judging: Judging,
}

// -- WP INT5 --
/// The devices agents are told of and may name: those the user has open. A
/// unit the app holds for a measurement across units is not one.
pub(crate) fn devices(set: &CoreSet) -> Vec<OpenDevice> {
    set.list().into_iter().filter(|d| !d.extra).collect()
}

/// The device `args` name, or the default: the app's device when it is
/// open, else the only device open.
fn resolve(
    set: &CoreSet,
    active: Option<&DeviceId>,
    args: &Map<String, Value>,
) -> Result<(DeviceId, String, Arc<DspiCore>), ToolError> {
    let open = devices(set);
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
        let devices = devices(ctx.set)
            .into_iter()
            .map(|open| {
                let core = ctx.set.core(&open.id);
                let firmware = core.as_deref().map(firmware).unwrap_or_default();
                let active = ctx.active == Some(&open.id);
                // -- WP MCP2c --
                let hub = core.as_deref().and_then(|c| tools::hub::row(&open.id, c));
                DeviceRow {
                    open,
                    firmware,
                    active,
                    hub,
                }
            })
            .collect();
        // -- WP FEAT2 -- judged by the tool itself (`policy::require_change`)
        let judging = JudgingCell::default();
        {
            let mut j = policy::lock(&judging);
            j.mode = ctx.policy.setting();
            j.grant.clone_from(&ctx.grant);
        }
        let mut call = Call {
            client: ctx.client,
            actor: ctx.actor,
            device: None,
            serial: String::new(),
            devices,
            activity: ctx.activity,
            policy: ctx.policy,
            judging,
            confirmations: ctx.confirmations,
            agent_guard: None,
            autoeq: ctx.autoeq, // -- WP MCP2b --
            template_dir: ctx.template_dir,
            installation: ctx.installation, // -- WP MCP2c --
            measure: ctx.measure,           // -- WP MCP5 --
            opens: ctx.opens,               // -- WP FEAT2 --
            client_label: ctx.client_label.to_owned(),
            set: Some(ctx.set),         // -- WP FEAT2 --
            host_audio: ctx.host_audio, // -- WP FEAT3 --
            core: None,
        };
        let result = (tool.handler)(&mut call, args);
        let judging = policy::lock(&call.judging).clone();
        return RunOutcome {
            device: None,
            result,
            journal_ids: Vec::new(),
            judging,
        };
    }
    let (id, serial, core) = match resolve(ctx.set, ctx.active, args) {
        Ok(found) => found,
        Err(e) => {
            return RunOutcome {
                device: None,
                result: Err(e),
                journal_ids: Vec::new(),
                judging: Judging::default(),
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
    // -- WP MCP2a -- the call's judgement, shared by the hook and the tool
    let judging = JudgingCell::default();
    policy::lock(&judging).grant.clone_from(&ctx.grant);
    let hook = ctx.policy.judging_hook(&info, &core, &judging);
    let agent_guard = ctx
        .guards
        .and_then(|g| g.get(id.as_str()))
        .filter(|(c, _)| *c == ctx.client)
        .map(|(_, g)| *g);
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
            judging: Arc::clone(&judging),
            confirmations: ctx.confirmations,
            agent_guard,
            autoeq: ctx.autoeq, // -- WP MCP2b --
            template_dir: ctx.template_dir,
            installation: ctx.installation, // -- WP MCP2c --
            measure: ctx.measure,           // -- WP MCP5 --
            opens: ctx.opens,               // -- WP FEAT2 --
            client_label: ctx.client_label.to_owned(),
            set: Some(ctx.set),
            host_audio: ctx.host_audio, // -- WP FEAT3 --
            core: Some(&core),
        };
        (tool.handler)(&mut call, args)
    };
    let judging = policy::lock(&judging).clone();
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
        judging,
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
