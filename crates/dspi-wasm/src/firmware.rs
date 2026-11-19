//! The board as an update target and the update check (WP PGFWX, PLAN-PG
//! sections 1.8 and 1.9), over `dspi_core`'s `firmware_target` and
//! `firmware_update_check`:
//!
//! - `firmwareTarget`: `{voltage: {requiredMv, source}, boardId (null: not
//!   read), lineage ("unknown", "upstream" or "bofr"), generation
//!   ("unknown", "legacy" or "paged")}`. Never a failure for the device:
//!   what cannot be read is unknown, and an unknown board is offered no
//!   image. Read it before the bootloader entry and keep it unchanged.
//! - `firmwareUpdateCheck {image: [bytes], target, typed}`: `{verdict:
//!   {allowed, decision, reason, message, confirmationPhrase, imageMv,
//!   requiredMv}, rule ("none", "noBootsel" or "lineageLoss"), imageLineage,
//!   imageGeneration, generationChange ("" when none)}`. Flash only when
//!   `verdict.allowed`. A missing `target` (or a field of it) is unknown, so
//!   no image; a target that is not one is code `invalid`. Needs no device.
//!
//! Both are reads: nothing is sent but the target's reads.
//!
//! -- WP PGFLASH -- The web has no firmware write (no `flashConfirmed`, no
//! `picobootFlashConfirmed`): a `target` given here only ever feeds the check.
//! Every write elsewhere goes through the core's write gate, which judges by
//! the target the core read itself.

use dspi_core::{DspiCore, FirmwareTarget};
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

#[derive(Deserialize)]
struct Check {
    #[serde(default)]
    image: Vec<u8>,
    #[serde(default)]
    target: FirmwareTarget,
    #[serde(default)]
    typed: String,
}

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "firmwareTarget" => Ok(tidy(&core.firmware_target())),
        "firmwareUpdateCheck" => serde_json::from_value::<Check>(p)
            .map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
            .map(|c| {
                tidy(&dspi_core::firmware_update_check(
                    c.image, &c.target, &c.typed,
                ))
            }),
        _ => return None,
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests_wp_pgfwx {
    use crate::calls::{call, session_new};
    use crate::guard;
    use serde_json::{Value, json};

    fn run(h: u32, method: &str, params: Value) -> Value {
        let out = call(
            h,
            json!({ "method": method, "params": params })
                .to_string()
                .as_bytes(),
        );
        serde_json::from_slice(&out).unwrap()
    }

    fn stock_image() -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/desktop/qt/firmware/DSPi-RP2350-v1.1.6.uf2");
        std::fs::read(path).unwrap()
    }

    #[test]
    fn wp_pgfwx_the_target_and_the_update_check_on_the_web() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        // Nothing open: nothing known, so no image.
        let none = run(h, "firmwareTarget", Value::Null);
        assert_eq!(none["ok"], true, "{none}");
        assert_eq!(none["value"]["boardId"], Value::Null, "{none}");
        let image = stock_image();
        let r = run(
            h,
            "firmwareUpdateCheck",
            json!({ "image": image, "target": none["value"] }),
        );
        assert_eq!(r["value"]["rule"], "noBootsel", "{r}");
        assert_eq!(r["value"]["verdict"]["allowed"], false, "{r}");
        // The demo is a Pico.
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
        let demo = run(h, "firmwareTarget", Value::Null);
        assert_eq!(demo["value"]["boardId"], 0, "{demo}");
        // A Pico on our firmware at the stock voltage: the lineage rule asks.
        let ours = json!({
            "voltage": { "requiredMv": 1150, "source": "statusBlock" },
            "boardId": 0, "lineage": "bofr", "generation": "legacy",
        });
        let r = run(
            h,
            "firmwareUpdateCheck",
            json!({ "image": image, "target": ours }),
        );
        assert_eq!(r["value"]["rule"], "lineageLoss", "{r}");
        assert_eq!(
            r["value"]["verdict"]["decision"], "needsConfirmation",
            "{r}"
        );
        let r = run(
            h,
            "firmwareUpdateCheck",
            json!({ "image": image, "target": ours, "typed": dspi_core::LINEAGE_PHRASE }),
        );
        assert_eq!(r["value"]["verdict"]["allowed"], true, "{r}");
        let r = run(
            h,
            "firmwareUpdateCheck",
            json!({ "image": [1, 2], "target": { "boardId": "x" } }),
        );
        assert_eq!(r["code"], "invalid", "{r}");
    }
}

// -- WP PGFLASH --
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_pgflash {
    use crate::calls::{call, session_new};
    use crate::guard;
    use serde_json::{Value, json};

    fn run(h: u32, method: &str, params: Value) -> Value {
        let out = call(
            h,
            json!({ "method": method, "params": params })
                .to_string()
                .as_bytes(),
        );
        serde_json::from_slice(&out).unwrap()
    }

    #[test]
    fn wp_pgflash_a_forged_target_on_the_web_reaches_no_write() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
        let image: Vec<u8> = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../apps/desktop/qt/firmware/DSPi-RP2350-v1.1.6.uf2"),
        )
        .unwrap();
        // A forged target makes the check say yes; that is all it can do.
        let forged = json!({
            "voltage": { "requiredMv": 1150, "source": "statusBlock" },
            "boardId": 0, "lineage": "upstream", "generation": "legacy",
        });
        let r = run(
            h,
            "firmwareUpdateCheck",
            json!({ "image": image, "target": forged }),
        );
        assert_eq!(r["value"]["verdict"]["allowed"], true, "{r}");
        // The web has no firmware write to take it to.
        for method in ["flashConfirmed", "picobootFlashConfirmed", "flashToDrive"] {
            let r = run(
                h,
                method,
                json!({ "image": image, "target": forged, "typed": "" }),
            );
            assert_eq!(
                (r["ok"].clone(), r["code"].clone()),
                (Value::Bool(false), "unknown_method".into()),
                "{method}: {r}"
            );
        }
    }
}
