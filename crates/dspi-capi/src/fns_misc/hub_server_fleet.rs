//! C functions: the hub's fleet families (WP P3CC), over
//! `dspi_core::hub_server::fleet` (`docs/integrator/hub-api.md` sections 19
//! to 21): groups, group writes and bulk apply, the firmware image store and
//! the rollout. Replies as in the parent module's docs: JSON, the records'
//! fields camelCase beside `"ok": true` (a list as `value`).
//!
//! A target (`target_json`) is `{"group": "g1"}` or `{"serials": ["A",
//! "B"]}`, exactly one of the two. Without the family's cap (or off a hub) a
//! call fails with `code` `unsupported` and sends nothing; the hub's refusals
//! as in `hub_server.rs` (a `fw.*` call over a plaintext session is `link`
//! with `dlink.tls_required`); in the client read-only mode each write is
//! `read_only`, nothing sent. Every call but `dspi_hub_rollout_step_notice`
//! is a round trip, a group request up to 300 s: off the UI thread.

use std::ffi::c_char;

use dspi_core::{
    DspiError, HubFleetTarget, HubGroup, HubOffset, HubPresetPick, ParamValue, Result,
};
use serde::de::DeserializeOwned;

use super::{arg, c_bytes, done_reply, json_reply, optional_text, text};
use crate::guard;

/// The `slot` of [`dspi_hub_group_preset_load`] when the preset is named.
const NO_SLOT: u8 = 0xFF;

/// `json` (JSON text) as `T`; `InvalidArgument` naming `name` when it does
/// not read. Null is `T`'s default when `optional`.
fn json_arg<T: DeserializeOwned + Default>(
    json: *const c_char,
    name: &str,
    optional: bool,
) -> Result<T> {
    if optional && json.is_null() {
        return Ok(T::default());
    }
    serde_json::from_str(arg(json, name)?).map_err(|e| DspiError::InvalidArgument {
        msg: format!("{name}: {e}"),
    })
}

fn target(json: *const c_char) -> Result<HubFleetTarget> {
    json_arg(json, "target_json", false)
}

fn owned(p: *const c_char, name: &str) -> Result<String> {
    Ok(optional_text(p, name)?.unwrap_or_default())
}

/// The hub's groups: JSON `{ok, value: [{id, name, note, members}]}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_groups(core: *mut crate::FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.hub_groups()))
}

/// Store a group (Admin): `group_json` `{id?, name, note?, members}`, no id
/// for a new one. JSON `{ok, group: {id, name, note, members}, created}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_group_set(
    core: *mut crate::FfiCore,
    group_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(
            json_arg::<HubGroup>(group_json, "group_json", false).and_then(|g| c.hub_group_set(g)),
        )
    })
}

/// Delete group `id` (Admin): JSON `{ok}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_group_delete(
    core: *mut crate::FfiCore,
    id: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        done_reply(owned(id, "id").and_then(|id| c.hub_group_delete(id)))
    })
}

/// Read `path` (`idx_json` its indices, a JSON list; null for none) on every
/// device of `target_json`: JSON `{ok, path, idx, results: [{serial, ok,
/// code, msg, noticeKey, value, choice, ...}]}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_group_param_get(
    core: *mut crate::FfiCore,
    target_json: *const c_char,
    path: *const c_char,
    idx_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let args = || -> Result<_> {
            Ok((
                target(target_json)?,
                arg(path, "path")?.to_owned(),
                json_arg::<Vec<u32>>(idx_json, "idx_json", true)?,
            ))
        };
        json_reply(args().and_then(|(t, p, i)| c.hub_group_param_get(t, p, i)))
    })
}

/// Write `value_json` (a parameter value: `{"float": -20}`, `{"bool": true}`,
/// ...) to `path` on every device of `target_json` (Control), with
/// `offsets_json` (null, or `[{serial, db}]`, dB paths only). JSON as
/// [`dspi_hub_group_param_get`]'s, rows with `clamped`.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn dspi_hub_group_param_set(
    core: *mut crate::FfiCore,
    target_json: *const c_char,
    path: *const c_char,
    idx_json: *const c_char,
    value_json: *const c_char,
    offsets_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let args =
            || -> Result<_> {
                let value: ParamValue = serde_json::from_str(arg(value_json, "value_json")?)
                    .map_err(|e| DspiError::InvalidArgument {
                        msg: format!("value_json: {e}"),
                    })?;
                Ok((
                    target(target_json)?,
                    arg(path, "path")?.to_owned(),
                    json_arg::<Vec<u32>>(idx_json, "idx_json", true)?,
                    value,
                    json_arg::<Vec<HubOffset>>(offsets_json, "offsets_json", true)?,
                ))
            };
        json_reply(args().and_then(|(t, p, i, v, o)| c.hub_group_param_set(t, p, i, v, o)))
    })
}

/// Load a preset on every device of `target_json` (Control): by `slot` (0 to
/// 9; 0xFF when named) or `name` (null or "" when by slot); `map_json` null
/// or `[{serial, slot?, name}]`. JSON `{ok, value: [rows with slot, name]}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_group_preset_load(
    core: *mut crate::FfiCore,
    target_json: *const c_char,
    slot: u8,
    name: *const c_char,
    map_json: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    let slot = (slot != NO_SLOT).then_some(slot);
    text(core, buf, len, |c| {
        let args = || -> Result<_> {
            Ok((
                target(target_json)?,
                owned(name, "name")?,
                json_arg::<Vec<HubPresetPick>>(map_json, "map_json", true)?,
            ))
        };
        json_reply(args().and_then(|(t, n, m)| c.hub_group_preset_load(t, slot, n, m)))
    })
}

/// Apply one snapshot to every device of `target_json` (Control): exactly
/// one of `snapshot` (an id), `name` or `document` (its text) non-empty;
/// with `guard` one guarded apply per device (`timeout_ms`, 0 for 30 s).
/// JSON `{ok, run, confirmBy, msg, results: [rows with image, lines, notes,
/// guard, reverted]}`.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn dspi_hub_group_apply(
    core: *mut crate::FfiCore,
    target_json: *const c_char,
    snapshot: *const c_char,
    name: *const c_char,
    document: *const c_char,
    guard: bool,
    timeout_ms: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let args = || -> Result<_> {
            Ok((
                target(target_json)?,
                owned(snapshot, "snapshot")?,
                owned(name, "name")?,
                owned(document, "document")?,
            ))
        };
        json_reply(args().and_then(|(t, s, n, d)| c.hub_group_apply(t, s, n, d, guard, timeout_ms)))
    })
}

/// Keep every pending guard of bulk-apply run `run`: JSON as
/// [`dspi_hub_group_apply`]'s, one row per guard.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_group_apply_confirm(
    core: *mut crate::FfiCore,
    run: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(owned(run, "run").and_then(|r| c.hub_group_apply_confirm(r)))
    })
}

/// Revert every pending guard of run `run` now: JSON as
/// [`dspi_hub_group_apply_confirm`]'s, rows with `reverted`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_group_apply_cancel(
    core: *mut crate::FfiCore,
    run: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(owned(run, "run").and_then(|r| c.hub_group_apply_cancel(r)))
    })
}

/// The hub's firmware images (over TLS): JSON `{ok, images: [{name, size,
/// sha256, firmwareVersion, platform, stampMv, verified, addedAt, addedBy}],
/// maxImages, maxBytes}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_firmware_images(
    core: *mut crate::FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(c.hub_firmware_images()))
}

/// Put the `data_len` bytes at `data` in the hub's store as `name` (Admin,
/// over TLS; in chunks, never logged): JSON `{ok, image, created}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_firmware_put(
    core: *mut crate::FfiCore,
    name: *const c_char,
    data: *const u8,
    data_len: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    let bytes = c_bytes(data, data_len);
    text(core, buf, len, |c| {
        json_reply(arg(name, "name").and_then(|n| c.hub_firmware_put(n.to_owned(), bytes)))
    })
}

/// Delete image `name` from the hub's store (Admin): JSON `{ok}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_firmware_delete(
    core: *mut crate::FfiCore,
    name: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        done_reply(arg(name, "name").and_then(|n| c.hub_firmware_delete(n.to_owned())))
    })
}

/// Start a rollout of stored image `image` on `target_json` (one serial or
/// a group; Admin, over TLS), the mute check skipped with `while_live`:
/// JSON `{ok, id, target, image, imageSha256, imageVersion, by, whileLive,
/// state, startedAt, endedAt, devices: [...]}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_rollout_start(
    core: *mut crate::FfiCore,
    target_json: *const c_char,
    image: *const c_char,
    while_live: bool,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        let args = || -> Result<_> { Ok((target(target_json)?, arg(image, "image")?.to_owned())) };
        json_reply(args().and_then(|(t, i)| c.hub_rollout_start(t, i, while_live)))
    })
}

/// The hub's rollouts, newest first, or only `id` (null or "": all): JSON
/// `{ok, value: [records as dspi_hub_rollout_start's]}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_rollouts(
    core: *mut crate::FfiCore,
    id: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(owned(id, "id").and_then(|id| c.hub_rollouts(id)))
    })
}

/// Cancel rollout `id` before its current device's bootloader entry
/// (Admin): the record as it is now.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_rollout_cancel(
    core: *mut crate::FfiCore,
    id: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(owned(id, "id").and_then(|id| c.hub_rollout_cancel(id)))
    })
}

/// What to tell the user about a rollout's `step` (`text` the hub's line,
/// null for none): JSON `{ok, key, kind, title, reason, action, detail}`.
/// No I/O; no core.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_rollout_step_notice(
    step: *const c_char,
    line: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    guard(0, || {
        let reply = owned(step, "step")
            .and_then(|s| owned(line, "text").map(|t| dspi_core::hub_rollout_step_notice(s, t)));
        crate::fns_device::copy_text(&json_reply(reply), buf, len)
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_p3cc {
    use std::ffi::{CString, c_char};

    use serde_json::Value;

    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};

    fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> Value {
        let mut buf = vec![0u8; 65536];
        let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
        assert!(needed as usize <= buf.len(), "{needed}");
        let end = buf.iter().position(|b| *b == 0).unwrap();
        serde_json::from_slice(&buf[..end]).unwrap()
    }

    /// The same words as the core's own calls, through C.
    #[test]
    fn wp_p3cc_the_fleet_calls_through_c() {
        let h = dspi_core_new();
        let s = |t: &str| CString::new(t).unwrap();
        let (group, target, path, value, id) = (
            s(r#"{"name": "Bar", "members": ["A"]}"#),
            s(r#"{"group": "g1"}"#),
            s("vol.master"),
            s(r#"{"float": -20}"#),
            s("g1"),
        );
        let offsets = s(r#"[{"serial": "A", "db": -3}]"#);
        let map = s(r#"[{"serial": "A", "slot": 2}]"#);
        let snap = s("S1");
        let image = s("dspi.uf2");
        let null = std::ptr::null();
        let bytes = [1u8, 2, 3];
        for r in [
            reply(|b, l| dspi_hub_groups(h, b, l)),
            reply(|b, l| dspi_hub_group_set(h, group.as_ptr(), b, l)),
            reply(|b, l| dspi_hub_group_delete(h, id.as_ptr(), b, l)),
            reply(|b, l| dspi_hub_group_param_get(h, target.as_ptr(), path.as_ptr(), null, b, l)),
            reply(|b, l| {
                dspi_hub_group_param_set(
                    h,
                    target.as_ptr(),
                    path.as_ptr(),
                    null,
                    value.as_ptr(),
                    offsets.as_ptr(),
                    b,
                    l,
                )
            }),
            reply(|b, l| {
                dspi_hub_group_preset_load(h, target.as_ptr(), 1, null, map.as_ptr(), b, l)
            }),
            reply(|b, l| {
                dspi_hub_group_apply(h, target.as_ptr(), snap.as_ptr(), null, null, true, 0, b, l)
            }),
            reply(|b, l| dspi_hub_group_apply_confirm(h, id.as_ptr(), b, l)),
            reply(|b, l| dspi_hub_group_apply_cancel(h, id.as_ptr(), b, l)),
            reply(|b, l| dspi_hub_firmware_images(h, b, l)),
            reply(|b, l| dspi_hub_firmware_put(h, image.as_ptr(), bytes.as_ptr(), 3, b, l)),
            reply(|b, l| dspi_hub_firmware_delete(h, image.as_ptr(), b, l)),
            reply(|b, l| dspi_hub_rollout_start(h, target.as_ptr(), image.as_ptr(), false, b, l)),
            reply(|b, l| dspi_hub_rollouts(h, null, b, l)),
            reply(|b, l| dspi_hub_rollout_cancel(h, id.as_ptr(), b, l)),
        ] {
            assert_eq!(r["ok"], false, "{r}");
            assert_eq!(r["code"], "unsupported", "{r}");
            assert_eq!(r["noticeKey"], "unsupported", "{r}");
        }
        // Arguments that do not read, or name nothing, are the caller's error.
        let bad = s("not json");
        for r in [
            reply(|b, l| dspi_hub_group_set(h, bad.as_ptr(), b, l)),
            reply(|b, l| dspi_hub_group_param_get(h, bad.as_ptr(), path.as_ptr(), null, b, l)),
            reply(|b, l| dspi_hub_group_delete(h, null, b, l)),
            reply(|b, l| dspi_hub_firmware_put(h, image.as_ptr(), std::ptr::null::<u8>(), 0, b, l)),
        ] {
            assert_eq!(r["code"], "invalid", "{r}");
        }
        let core_says = dspi_core::DspiCore::new()
            .hub_group_delete(String::new())
            .unwrap_err();
        let r = reply(|b, l| dspi_hub_group_delete(h, null, b, l));
        assert_eq!(r["error"], core_says.to_string(), "{r}");
        // The step's notice, no core.
        let step = s("flash");
        let n = reply(|b, l| dspi_hub_rollout_step_notice(step.as_ptr(), null, b, l));
        assert_eq!(n["key"], "rollout.flash", "{n}");
        assert_eq!(
            n["title"],
            dspi_core::hub_rollout_step_notice("flash".into(), String::new()).title
        );
        dspi_core_free(h);
    }
}
