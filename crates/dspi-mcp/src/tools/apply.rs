//! Several changes at once, with an optional timed guard, and the
//! confirmation an agent waits for (WP MCP2a).
//!
//! `dspi_apply_changes` runs the edit tools' plans as one plan: judged whole,
//! one undo group, all or none. With `guard_s` the core's guarded apply
//! (`apply_guard_begin`) captures the device first and puts it back at the
//! timeout unless kept; the server's `tick` drives the countdown. No guard
//! is the default (owner 2026-11-21: agents' changes apply directly; the user
//! undoes). A Keep after an auto-tune's failed verify stays the user's (the
//! core refuses it to anyone else); a revert is never judged.

use dspi_core::DspiError;
use serde_json::{Map, Value, json};

use crate::confirm::ConfirmState;
use crate::policy::{self, lock};
use crate::registry::{self, Call, Tool, ToolClass, ToolError, ToolResult};
use crate::tools::edit::{self, MAX_WRITES, PLANNERS, Plan};
use crate::tools::read::{object_schema, state};

/// The tools, in `tools/list` order.
pub static TOOLS: &[&Tool] = &[
    &APPLY_CHANGES,
    &KEEP_CHANGES,
    &REVERT_CHANGES,
    &GET_CONFIRMATION,
];

/// A guard's shortest and longest timeout (the core takes up to 590 s).
const GUARD_MIN_S: u64 = 10;
const GUARD_MAX_S: u64 = 590;

static APPLY_CHANGES: Tool = Tool {
    name: "dspi_apply_changes",
    title: "Apply several changes",
    description: "Several edits on one device as one change: `changes` is a list (at most 40) of \
                  {\"op\": <an edit tool's name without dspi_, e.g. \"set_peq_band\">, ...that \
                  tool's arguments}. They are judged together, applied in order as one undo \
                  step, and all or none. `guard_s` (10..590) makes it a timed preview: the \
                  device is put back after that many seconds unless dspi_keep_changes is called \
                  (or the user keeps it); dspi_revert_changes puts it back now. Without \
                  `guard_s` the changes stay; dspi_undo takes them back.",
    class: ToolClass::Write,
    device: true,
    input_schema: || {
        json!({"type": "object", "required": ["changes"], "additionalProperties": false,
               "properties": {
                   "changes": {"type": "array", "maxItems": MAX_WRITES,
                               "items": {"type": "object", "required": ["op"]}},
                   "guard_s": {"type": "integer", "minimum": GUARD_MIN_S, "maximum": GUARD_MAX_S}}})
    },
    output_schema: object_schema,
    handler: apply_changes,
};

fn apply_changes(call: &mut Call<'_>, args: &Map<String, Value>) -> ToolResult {
    let core = call.core()?;
    let before = state(core)?;
    let changes = args
        .get("changes")
        .and_then(Value::as_array)
        .ok_or_else(|| ToolError::new("`changes` is a list"))?;
    if changes.is_empty() {
        return Err(ToolError::new("`changes` is empty"));
    }
    if changes.len() > MAX_WRITES {
        return Err(ToolError::new(format!(
            "at most {MAX_WRITES} changes in one call"
        )));
    }
    let mut plan = Plan::new(before.clone());
    for (n, c) in changes.iter().enumerate() {
        // -- WP SHOWFIX -- counted from 1, as the agent counts its list
        let at = |e: String| ToolError::new(format!("change {}: {e}", n.saturating_add(1)));
        let obj = c
            .as_object()
            .ok_or_else(|| at("an object with `op`".into()))?;
        let op = obj.get("op").and_then(Value::as_str).unwrap_or("");
        let Some((_, planner)) = PLANNERS.iter().find(|(name, _)| *name == op) else {
            let ops: Vec<&str> = PLANNERS.iter().map(|(name, _)| *name).collect();
            // -- WP FFIX -- FUNC1 bug 15: agents looked for set_limiter here
            return Err(at(format!(
                "`op` is one of {}; a limiter or an effect is not one: dspi_set_limiter and the \
                 effects' tools set them, each a call of its own",
                ops.join(", ")
            )));
        };
        let mut a = obj.clone();
        a.remove("op");
        if a.contains_key("device") {
            return Err(at(
                "the changes go to one device: pass `device` once, beside `changes`".into(),
            ));
        }
        let tool = edit::TOOLS
            .iter()
            .find(|t| t.name.strip_prefix("dspi_") == Some(op))
            .ok_or_else(|| at("unknown op".into()))?;
        registry::check_args(&(tool.input_schema)(), &a).map_err(at)?;
        planner(&mut plan, &a).map_err(|e| at(e.0))?;
    }
    let Some(guard_s) = args.get("guard_s").and_then(Value::as_u64) else {
        return edit::execute(call, &before, plan, "AI: several changes");
    };
    // Judged before the guard begins, so a held call begins none.
    policy::prejudge(&call.judging, &before, &plan.writes)?;
    let timeout = u32::try_from(guard_s.clamp(GUARD_MIN_S, GUARD_MAX_S)).unwrap_or(60);
    let guard = core
        .apply_guard_begin("AI changes".into(), timeout)
        .map_err(|e| policy::core_error(&call.judging, e))?;
    lock(&call.judging).guard_begun = Some(guard.id);
    match edit::execute(call, &before, plan, "AI: several changes") {
        Ok(mut v) => {
            let status = call.core()?.apply_status();
            if let Some(o) = v.as_object_mut() {
                o.insert(
                    "guard".into(),
                    json!({"guard_id": status.id, "timeout_s": timeout,
                           "remaining_ms": status.remaining_ms,
                           "phase": serde_json::to_value(status.phase).unwrap_or(Value::Null)}),
                );
            }
            Ok(v)
        }
        Err(e) => {
            // The writes were put back; end the guard as well.
            let _ = call.core()?.apply_revert();
            let mut j = lock(&call.judging);
            j.guard_begun = None;
            j.guard_ended = Some(guard.id);
            Err(e)
        }
    }
}

/// This agent's guard on the device, checked against `guard_id` if given.
fn own_guard(call: &Call<'_>, args: &Map<String, Value>) -> Result<u64, ToolError> {
    let mine = call
        .agent_guard
        .ok_or_else(|| ToolError::new("no timed preview of this agent's waits on this device"))?;
    match args.get("guard_id").and_then(Value::as_u64) {
        Some(g) if g != mine => Err(ToolError::new(
            "that guard is not this agent's waiting preview on this device",
        )),
        _ => Ok(mine),
    }
}

fn guard_schema() -> Value {
    json!({"type": "object", "additionalProperties": false,
           "properties": {"guard_id": {"type": "integer", "minimum": 1}}})
}

fn guard_json(g: &dspi_core::GuardedApply) -> Value {
    json!({
        "guard_id": g.id,
        "phase": serde_json::to_value(g.phase).unwrap_or(Value::Null),
        "reason": g.reason,
        "message": g.message,
    })
}

static KEEP_CHANGES: Tool = Tool {
    name: "dspi_keep_changes",
    title: "Keep a timed preview",
    description: "Keep this agent's timed preview (dspi_apply_changes with guard_s) on a \
                  device: the countdown stops and the changes stay. An auto-tune only after its \
                  verify passed; otherwise only the user may keep it, in the app.",
    class: ToolClass::Write,
    device: true,
    input_schema: guard_schema,
    output_schema: object_schema,
    handler: |call, args| {
        let guard = own_guard(call, args)?;
        let core = call.core()?;
        // -- WP RCFIX -- as dspi_tune_keep, before the core is asked
        if super::measure::guard_is_failed_tunes(core, guard) {
            return Err(ToolError::new(super::measure::KEEP_IS_THE_USERS));
        }
        // -- WP ODEC -- nor one with no passed verify
        if super::measure::guard_is_unverified_tunes(core, guard) {
            return Err(ToolError::new(super::measure::KEEP_NEEDS_A_PASSED_VERIFY));
        }
        // -- WP AIVIS -- the user changed part of it meanwhile
        dspi_core::actor::user::check_keep(core, call.actor)
            .map_err(|e| policy::core_error(&call.judging, e))?;
        let kept = core.apply_confirm().map_err(|e| match &e {
            DspiError::InvalidArgument { msg } if msg.contains("measure.tune.keep_unverified") => {
                ToolError::new(super::measure::KEEP_IS_THE_USERS)
            }
            _ => policy::core_error(&call.judging, e),
        })?;
        lock(&call.judging).guard_ended = Some(guard);
        Ok(json!({"ok": true, "guard": guard_json(&kept)}))
    },
};

static REVERT_CHANGES: Tool = Tool {
    name: "dspi_revert_changes",
    title: "Revert a timed preview",
    description: "Put the device back as it was before this agent's timed preview, now. Always \
                  allowed.",
    class: ToolClass::Write,
    device: true,
    input_schema: guard_schema,
    output_schema: object_schema,
    handler: |call, args| {
        let guard = own_guard(call, args)?;
        let done = call
            .core()?
            .apply_revert()
            .map_err(|e| policy::core_error(&call.judging, e))?;
        lock(&call.judging).guard_ended = Some(guard);
        Ok(json!({"ok": true, "guard": guard_json(&done)}))
    },
};

static GET_CONFIRMATION: Tool = Tool {
    name: "dspi_get_confirmation",
    title: "A confirmation's outcome",
    description: "Whether the user has answered a change this agent asked for \
                  (`confirmation_id` from an awaiting_confirmation result): waiting, allowed \
                  (with the change's result), denied or expired. Ask again a few seconds later \
                  while it waits; it lapses after 120 s.",
    class: ToolClass::Read,
    device: false,
    input_schema: || {
        json!({"type": "object", "required": ["confirmation_id"], "additionalProperties": false,
               "properties": {"confirmation_id": {"type": "integer", "minimum": 1}}})
    },
    output_schema: object_schema,
    handler: |call, args| {
        let id = args
            .get("confirmation_id")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let c = call
            .confirmations
            .and_then(|t| t.get(id))
            .filter(|c| c.client == call.client)
            .ok_or_else(|| ToolError::new("this agent has no confirmation with that id"))?;
        let mut out = json!({
            "confirmation_id": id,
            "status": c.state.as_str(),
            "risks": c.risks.iter().map(policy::Risk::to_json).collect::<Vec<_>>(),
        });
        if c.state != ConfirmState::Waiting
            && let Some(o) = out.as_object_mut()
        {
            o.insert("result".into(), c.result.clone().unwrap_or(Value::Null));
        }
        Ok(out)
    },
};
