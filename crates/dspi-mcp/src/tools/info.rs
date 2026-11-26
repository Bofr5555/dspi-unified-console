//! Limits and firmware, read only (WP MCP2c, MPLAN 3.2 "Limits and
//! installer", "Firmware"). Editing the limits profile, the installer PIN,
//! locking and unlocking, flashing, the bootloader, PICOBOOT and a factory
//! reset are agent-proof: no tool does them.

use dspi_core::{FirmwareGeneration, bundled_images, platform_shape};
use serde_json::{Value, json};

use crate::policy;
use crate::registry::{Tool, ToolClass};
use crate::tools::hub::record;
use crate::tools::read::{empty_schema, object_schema, put};
use crate::untrusted;

/// The tools, in `tools/list` order.
pub static TOOLS: &[&Tool] = &[&GET_LIMITS, &GET_FIRMWARE];

static GET_LIMITS: Tool = Tool {
    name: "dspi_get_limits",
    title: "Read the limits",
    description: "What holds this agent back on the device: the installer's limits profile (in \
                  force or not, and its ranges), the installer lock (and what passes while it is \
                  locked), whether the app has the device read-only, the \
                  confirmation setting the user chose for agents (ask_everything, ask_risky, \
                  never_ask) and what agents may do. The limits and the user's speaker setup \
                  are refused, not asked about, whatever the setting. Only the user edits them.",
    class: ToolClass::Read,
    device: true,
    input_schema: empty_schema,
    output_schema: object_schema,
    handler: |call, _| {
        let core = call.core()?;
        let status = core.limits_status();
        let mut profile = if status.active {
            serde_json::from_str::<Value>(&core.limits_profile_document())
                .map_or(Value::Null, untrusted::scrub_json)
        } else {
            Value::Null
        };
        // -- WP FIN015 -- (REVF S3) each limited output from 1, as the agent
        // counts it (the file keeps 0)
        for o in profile
            .pointer_mut("/limits/outputs")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if let Some(n) = o.get("output").and_then(Value::as_u64) {
                put(o, "output", json!(n.saturating_add(1)));
            }
        }
        let mode = policy::lock(&call.judging).mode.map(|m| m.as_str());
        Ok(json!({
            "device": call.device_text(),
            "limits": record(serde_json::to_value(&status), &[("detail", "note"), ("last_held", "notes")]),
            "profile": profile,
            "installer_lock": policy::lock_json(&core.installer_lock_state()), // -- WP FUNC2c --
            "app_read_only": core.client_read_only(),
            "confirmation_setting": mode,
            "agent_may": call.agent_may(), // -- WP FEAT2 --
        }))
    },
};

/// `a.b.c` as numbers, for comparing versions; `None` for another shape.
fn version(text: &str) -> Option<Vec<u32>> {
    let core = text.split(['-', '+', ' ']).next()?;
    core.split('.').map(|p| p.parse().ok()).collect()
}

static GET_FIRMWARE: Tool = Tool {
    name: "dspi_get_firmware",
    title: "Read the firmware",
    description: "The device's firmware: version, beta flag, protocol generation (legacy 1.1.6 \
                  or paged 1.1.7+), the platform, and the images this app bundles for that \
                  platform with whether one is newer. Updating is the user's, in the app: no \
                  tool flashes.",
    class: ToolClass::Read,
    device: true,
    input_schema: empty_schema,
    output_schema: object_schema,
    handler: |call, _| {
        let core = call.core()?;
        let connected = core.is_connected();
        let st = core.get_state();
        let running = format!("{}.{}.{}", st.fw_major, st.fw_minor, st.fw_patch);
        let platform = platform_shape(st.platform_id).name;
        let generation = record(
            serde_json::to_value(FirmwareGeneration::of_version(&running)),
            &[],
        );
        let mine = version(&running);
        let images: Vec<Value> = bundled_images()
            .into_iter()
            .filter(|i| i.platform == platform)
            .map(|i| {
                let newer = matches!((version(&i.version), &mine), (Some(v), Some(m)) if v > *m);
                json!({
                    "image_name": untrusted::name(&i.name),
                    "version": untrusted::name(&i.version),
                    "lineage": record(serde_json::to_value(i.lineage), &[]),
                    "generation": record(serde_json::to_value(i.generation), &[]),
                    "newer": newer,
                })
            })
            .collect();
        let update_available =
            connected && images.iter().any(|i| i.get("newer") == Some(&json!(true)));
        Ok(json!({
            "device": call.device_text(),
            "connected": connected,
            "firmware": if connected { Value::from(running) } else { Value::Null },
            "firmware_beta": st.fw_beta,
            "generation": generation,
            "platform": platform,
            "bundled": images,
            "update_available": update_available,
        }))
    },
};
