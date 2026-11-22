//! Names and the input source (WP MCP2b). Raw writes the core's hook does
//! not see (review RA, MCP0 item 3), so each has a static class:
//! - a channel name lives in the settings (saved with a preset): asked under
//!   "ask on everything" only (`policy::require_change`);
//! - the device's and the sources' names go to the device's flash at once
//!   (`FlashWrite`: asked by default);
//! - choosing another input source plays another signal (`Source`: asked by
//!   default).
//!
//! Every name an agent passes is cleaned first (`untrusted::name`: control
//! and direction characters dropped, cut to 32 bytes); the result carries
//! the name as written.

use serde_json::{Map, Value, json};

use crate::policy::{self, RiskKind};
use crate::registry::{Call, Tool, ToolClass, ToolError, ToolResult};
use crate::tools::edit::{channel, schema, u8_arg, with_target};
use crate::tools::read::{object_schema, state};
use crate::untrusted;

/// The tools, in `tools/list` order.
pub static TOOLS: &[&Tool] = &[
    &SET_CHANNEL_NAME,
    &SET_DEVICE_NAME,
    &SET_SOURCE_NAME,
    &SET_SOURCE,
];

/// The input sources the firmware knows (0..7; 7 is the audio link).
const SOURCES: u8 = 8;

fn name_arg(args: &Map<String, Value>) -> String {
    untrusted::name(args.get("name").and_then(Value::as_str).unwrap_or(""))
}

fn source_arg(args: &Map<String, Value>) -> Result<u8, ToolError> {
    u8_arg(args, "source")
        .filter(|s| *s < SOURCES)
        .ok_or_else(|| ToolError::new("`source` is an input source 0..7"))
}

fn name_prop() -> Value {
    json!({"type": "string", "description": "The name; cleaned and cut to 32 bytes"})
}

fn source_prop() -> Value {
    json!({"type": "integer", "minimum": 0, "maximum": SOURCES - 1,
           "description": "An input source: 0 USB, 1..6 the device's digital inputs, 7 the audio link"})
}

static SET_CHANNEL_NAME: Tool = Tool {
    name: "dspi_set_channel_name",
    title: "Name a channel",
    description: "Name an output, input or wire channel. The name is kept with the settings \
                  (saved with a preset).",
    class: ToolClass::Write,
    device: true,
    input_schema: || schema(with_target(json!({"name": name_prop()})), &["name"]),
    output_schema: object_schema,
    handler: set_channel_name,
};

fn set_channel_name(call: &mut Call<'_>, args: &Map<String, Value>) -> ToolResult {
    let st = state(call.core()?)?;
    let (ch, _, _) = channel(&st, args)?;
    let name = name_arg(args);
    policy::require_change(&call.judging, "Rename a channel", &[ch])?;
    let core = call.core()?;
    core.set_channel_name(ch, name)
        .map_err(|e| policy::core_error(&call.judging, e))?;
    // As the device keeps it (31 bytes).
    let kept = core
        .get_state()
        .channel_names
        .get(usize::from(ch))
        .map(|n| untrusted::name(n));
    Ok(json!({"ok": true, "device": call.device_text(), "channel": ch, "channel_name": kept}))
}

static SET_DEVICE_NAME: Tool = Tool {
    name: "dspi_set_device_name",
    title: "Name the device",
    description: "Rename the device (\"\" clears it). The device stores it in flash at once; \
                  it may be held for the user's confirmation.",
    class: ToolClass::Write,
    device: true,
    input_schema: || {
        let mut m = Map::new();
        m.insert("name".into(), name_prop());
        schema(m, &["name"])
    },
    output_schema: object_schema,
    handler: |call, args| {
        let name = name_arg(args);
        policy::require(
            &call.judging,
            RiskKind::FlashWrite,
            "Rename the device (flash)",
            &[],
        )?;
        call.core()?
            .set_device_name(name.clone())
            .map_err(|e| policy::core_error(&call.judging, e))?;
        Ok(json!({"ok": true, "device": call.device_text(), "device_name": name}))
    },
};

static SET_SOURCE_NAME: Tool = Tool {
    name: "dspi_set_source_name",
    title: "Name an input source",
    description: "Rename input source `source`. The device stores it in flash at once; it may be \
                  held for the user's confirmation.",
    class: ToolClass::Write,
    device: true,
    input_schema: || {
        let mut m = Map::new();
        m.insert("source".into(), source_prop());
        m.insert("name".into(), name_prop());
        schema(m, &["source", "name"])
    },
    output_schema: object_schema,
    handler: |call, args| {
        let source = source_arg(args)?;
        let name = name_arg(args);
        let what = format!("Rename input source {source} (flash)");
        policy::require(&call.judging, RiskKind::FlashWrite, &what, &[source])?;
        call.core()?
            .set_input_source_name(source, name.clone())
            .map_err(|e| policy::core_error(&call.judging, e))?;
        Ok(
            json!({"ok": true, "device": call.device_text(), "source": source,
                  "source_name": name}),
        )
    },
};

static SET_SOURCE: Tool = Tool {
    name: "dspi_set_source",
    title: "Choose the input source",
    description: "Switch the device to input source `source`. Another signal then plays: it may \
                  be held for the user's confirmation.",
    class: ToolClass::Write,
    device: true,
    input_schema: || {
        let mut m = Map::new();
        m.insert("source".into(), source_prop());
        schema(m, &["source"])
    },
    output_schema: object_schema,
    handler: |call, args| {
        let source = source_arg(args)?;
        let st = state(call.core()?)?;
        if st.input_source == source {
            return Ok(
                json!({"ok": true, "device": call.device_text(), "source": source,
                             "changed": false}),
            );
        }
        let what = format!("Switch to input source {source}");
        policy::require(&call.judging, RiskKind::Source, &what, &[source])?;
        call.core()?
            .set_input_source(source)
            .map_err(|e| policy::core_error(&call.judging, e))?;
        Ok(json!({"ok": true, "device": call.device_text(), "source": source, "changed": true}))
    },
};
