//! C functions: the hub's own families past DSPi Link (WP P2CC1), over
//! `dspi_core::hub_server` (`docs/integrator/hub-api.md`): the inventory,
//! Identify, health, statistics, the shared history and the hub's snapshot
//! store. Replies as in the parent module's docs: JSON, the records' fields
//! camelCase beside `"ok": true`, a list as `value`.
//!
//! Without the family's cap (or off a hub) a call fails with `code`
//! `unsupported` and sends nothing. The hub's refusals come as the core's
//! kinds (`denied`, `locked`, `invalid`, `limit_exceeded`, `installer_lock`,
//! `image_refused`, `confirmation_required`) or as `link` with `noticeKey`
//! `dlink.<code>` (`not_found`, `offline`, `conflict`, `rate_limited`,
//! `busy`, `device`). Every call but `dspi_hub_caps`, `dspi_hub_server_events`
//! and the listener is a round trip: off the UI thread.

use std::cell::Cell;
use std::ffi::{CString, c_char, c_void};
use std::sync::Arc;

use dspi_core::{HubServerEvent, HubServerListener};

use super::{arg, done_reply, json_reply, optional_text, text};
use crate::with_core;

/// The `tone_output` of [`dspi_hub_identify`] for no tone.
const IDENTIFY_NO_TONE: u8 = 0xFF;

/// Which of the hub's families this console's hub has: JSON `{ok, onHub,
/// inventory, identify, health, events, hubSnapshots, paramApi, hubStats,
/// caps}`. No I/O.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_caps(core: *mut crate::FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.hub_caps())))
}

/// Every DSPi the hub has seen: JSON `{ok, value: [{serial, handle, online,
/// name, platform, fw, fwBeta, outputs, board, hwRev, chip, chipRev,
/// location, note, tags, firstSeen, lastSeen, lastReset}]}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_inventory(core: *mut crate::FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.hub_inventory()))
}

/// Identify the open device for `seconds` (0 stops), with the quiet tone on
/// `tone_output` (0xFF: none; pass 0xFF unless the user asked for sound):
/// through the hub on a hub, the device's own `EXT_IDENTIFY` directly. JSON
/// `{ok, remainingS, lastToneOutput, chip, chipRev, chipId, board, hwRev}`,
/// `{ok, value: null}` when the state could not be read after the write, or
/// a failure (`link` with `dlink.rate_limited` within 2 s on a hub).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_identify(
    core: *mut crate::FfiCore,
    seconds: u8,
    tone_output: u8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    let tone = (tone_output != IDENTIFY_NO_TONE).then_some(tone_output);
    text(core, buf, len, |c| {
        json_reply(c.hub_identify(seconds, tone))
    })
}

/// One health record per online device: JSON `{ok, value: [{serial,
/// handle, at, cpuPct, clipMask, tempC, uptimeS, hangCount, unresponsive,
/// unresponsiveSince, reset}]}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_health(core: *mut crate::FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.hub_health()))
}

/// The hub's statistics (Admin of the hub): JSON `{ok, sessions, uptimeS,
/// devices: [{handle, serial, cmds, errors, avgRttMs, notifyDropped}]}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_stats(core: *mut crate::FfiCore, buf: *mut c_char, len: u32) -> u32 {
    text(core, buf, len, |c| json_reply(c.hub_stats()))
}

/// Device `serial`'s history on the hub (null or "" the open device), ids
/// above `after_id`, at most `limit` (0: the hub's 100; at most 500): JSON
/// `{ok, serial, records: [{id, timeMs, kind, serial, text, source, session,
/// by, channel, value, fw, uptimeS, lines, count}], more, newestId}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_events_read(
    core: *mut crate::FfiCore,
    serial: *const c_char,
    after_id: u64,
    limit: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(
            optional_text(serial, "serial")
                .and_then(|s| c.hub_events_read(s.unwrap_or_default(), after_id, limit)),
        )
    })
}

/// Have the hub send each new history record of `serial` (null or "": every
/// device) as an event (`on`), or stop: `{ok}` or a failure. The records
/// come through [`dspi_hub_server_events`] and the listener.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_events_watch(
    core: *mut crate::FfiCore,
    on: bool,
    serial: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        done_reply(
            optional_text(serial, "serial")
                .and_then(|s| c.hub_events_watch(on, s.unwrap_or_default())),
        )
    })
}

/// The hub's `event.added`, `inventory.changed` and `snapshot.changed` since
/// the last call, oldest first: JSON `{ok, value: [{kind, serial, record,
/// device, snapshot, change}]}`. No I/O.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_server_events(
    core: *mut crate::FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| json_reply(Ok(c.hub_server_events())))
}

/// A hub event of these families: `kind` and the event as JSON (`{kind,
/// serial, record, device, snapshot, change}`), both valid during the call
/// only. Called on the Link session's supervisor thread with no core lock
/// held: post the work to the app's own thread and return. Do not call
/// `dspi_hub_server_set_listener` from it.
pub type HubServerCallback =
    extern "C" fn(kind: *const c_char, event_json: *const c_char, user_data: *mut c_void);

thread_local! {
    /// This thread is inside a [`HubServerCallback`].
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

struct CHubServer {
    callback: HubServerCallback,
    user_data: *mut c_void,
}

// SAFETY: the callback runs on the core's threads by the ABI's documented
// contract (the caller's `user_data` must allow that); nothing here touches
// the pointer.
unsafe impl Send for CHubServer {}
// SAFETY: as for Send: the pointer is only passed back to the caller.
unsafe impl Sync for CHubServer {}

impl HubServerListener for CHubServer {
    fn on_hub_server_event(&self, event: HubServerEvent) {
        let json = serde_json::to_string(&event).unwrap_or_default();
        // A string with an inner NUL is cut there, never a failure.
        let z = |s: &str| CString::new(s.split('\0').next().unwrap_or_default()).ok();
        let (Some(kind), Some(json)) = (z(&event.kind), z(&json)) else {
            return;
        };
        IN_CALLBACK.with(|c| c.set(true));
        (self.callback)(kind.as_ptr(), json.as_ptr(), self.user_data);
        IN_CALLBACK.with(|c| c.set(false));
    }
}

/// Set (or with a null `callback`, clear) the listener for the hub's events
/// of these families. False for a null core, or when called from the
/// callback itself.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_server_set_listener(
    core: *mut crate::FfiCore,
    callback: Option<
        extern "C" fn(kind: *const c_char, event_json: *const c_char, user_data: *mut c_void),
    >,
    user_data: *mut c_void,
) -> bool {
    if IN_CALLBACK.with(Cell::get) {
        log::warn!("dspi-capi: dspi_hub_server_set_listener called from its own callback");
        return false;
    }
    with_core(core, false, |h| {
        let listener = callback.map(|callback| {
            Arc::new(CHubServer {
                callback,
                user_data,
            }) as Arc<dyn HubServerListener>
        });
        h.core.set_hub_server_listener(listener);
        true
    })
}

/// Device `serial`'s snapshots on the hub (null or "" the open device):
/// JSON `{ok, value: [{id, name, note, serial, savedMs, platform,
/// inputChannels, outputChannels, wireFormat}]}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_snapshots_list(
    core: *mut crate::FfiCore,
    serial: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(
            optional_text(serial, "serial")
                .and_then(|s| c.hub_snapshots_list(s.unwrap_or_default())),
        )
    })
}

/// Snapshot `id` on the hub: JSON `{ok, snapshot: {entry}, document}` (the
/// document's compact text).
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_snapshots_get(
    core: *mut crate::FfiCore,
    id: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(id, "id").and_then(|i| c.hub_snapshots_get(i.to_owned())))
    })
}

/// File snapshot `document` (its text) of `serial` (null or "": the one its
/// `snapshot` block names) on the hub: JSON `{ok, snapshot, created}`. A
/// document over 61 440 bytes compact fails here with `link`, `noticeKey`
/// `dlink.conflict` and the hub's words, and nothing is sent.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_snapshots_put(
    core: *mut crate::FfiCore,
    serial: *const c_char,
    document: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(optional_text(serial, "serial").and_then(|s| {
            arg(document, "document")
                .and_then(|d| c.hub_snapshots_put(s.unwrap_or_default(), d.to_owned()))
        }))
    })
}

/// Rename snapshot `id` on the hub: the entry's JSON, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_snapshots_rename(
    core: *mut crate::FfiCore,
    id: *const c_char,
    name: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(id, "id").and_then(|i| {
            arg(name, "name").and_then(|n| c.hub_snapshots_rename(i.to_owned(), n.to_owned()))
        }))
    })
}

/// Set snapshot `id`'s note on the hub: the entry's JSON, or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_snapshots_note(
    core: *mut crate::FfiCore,
    id: *const c_char,
    note: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(id, "id").and_then(|i| {
            arg(note, "note").and_then(|n| c.hub_snapshots_note(i.to_owned(), n.to_owned()))
        }))
    })
}

/// Delete snapshot `id` from the hub (Admin of the hub): `{ok}` or a
/// failure.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_snapshots_delete(
    core: *mut crate::FfiCore,
    id: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        done_reply(arg(id, "id").and_then(|i| c.hub_snapshots_delete(i.to_owned())))
    })
}

/// Recall snapshot `id` of the hub's store on `serial` (null or "" the open
/// device), with a guarded apply first when `guard` (`timeout_ms`, 0 for the
/// hub's 30 s): JSON `{ok, serial, snapshot, name, image, lines, notes,
/// guard}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_snapshots_recall(
    core: *mut crate::FfiCore,
    serial: *const c_char,
    id: *const c_char,
    guard: bool,
    timeout_ms: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(optional_text(serial, "serial").and_then(|s| {
            arg(id, "id").and_then(|i| {
                c.hub_snapshots_recall(s.unwrap_or_default(), i.to_owned(), guard, timeout_ms)
            })
        }))
    })
}

/// "Save to the hub": this console's snapshot `id` filed on the hub: JSON
/// `{ok, snapshot, created}`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_snapshots_save(
    core: *mut crate::FfiCore,
    id: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(id, "id").and_then(|i| c.hub_snapshots_save(i.to_owned())))
    })
}

/// "Copy here": the hub's snapshot `id` filed in this console's store: the
/// local entry's JSON.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_hub_snapshots_copy_here(
    core: *mut crate::FfiCore,
    id: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(arg(id, "id").and_then(|i| c.hub_snapshots_copy_here(i.to_owned())))
    })
}

/// The snapshots of `serial` (null or "" the open device) on both sides:
/// JSON `{ok, entries: [{snapshot, where, localCopy}], hubListed,
/// hubNoticeKey}` (`where`: `local`, `hub` or `both`). Never fails but for a
/// null core.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_snapshot_list_merged(
    core: *mut crate::FfiCore,
    serial: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| {
        json_reply(
            optional_text(serial, "serial").map(|s| c.snapshot_list_merged(s.unwrap_or_default())),
        )
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_p2cc1 {
    use std::ffi::{CString, c_char};

    use serde_json::Value;

    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};
    use crate::fns_misc::dspi_open_demo;

    fn reply(f: impl FnOnce(*mut c_char, u32) -> u32) -> Value {
        let mut buf = vec![0u8; 65536];
        let needed = f(buf.as_mut_ptr().cast(), buf.len() as u32);
        assert!(needed as usize <= buf.len(), "{needed}");
        let end = buf.iter().position(|b| *b == 0).unwrap();
        serde_json::from_slice(&buf[..end]).unwrap()
    }

    extern "C" fn heard(_kind: *const c_char, _json: *const c_char, _user: *mut c_void) {}

    /// The same words as the core's own call, through C.
    #[test]
    fn wp_p2cc1_the_hub_calls_through_c() {
        let h = dspi_core_new();
        let caps = reply(|b, l| dspi_hub_caps(h, b, l));
        assert_eq!(caps["ok"], true, "{caps}");
        assert_eq!(caps["onHub"], false);
        assert_eq!(caps["hubSnapshots"], false);
        let id = CString::new("0000000000001").unwrap();
        for r in [
            reply(|b, l| dspi_hub_inventory(h, b, l)),
            reply(|b, l| dspi_hub_health(h, b, l)),
            reply(|b, l| dspi_hub_stats(h, b, l)),
            reply(|b, l| dspi_hub_events_read(h, std::ptr::null(), 0, 0, b, l)),
            reply(|b, l| dspi_hub_events_watch(h, true, std::ptr::null(), b, l)),
            reply(|b, l| dspi_hub_snapshots_list(h, std::ptr::null(), b, l)),
            reply(|b, l| dspi_hub_snapshots_get(h, id.as_ptr(), b, l)),
            reply(|b, l| dspi_hub_snapshots_delete(h, id.as_ptr(), b, l)),
            reply(|b, l| {
                dspi_hub_snapshots_recall(h, std::ptr::null(), id.as_ptr(), false, 0, b, l)
            }),
            reply(|b, l| dspi_hub_snapshots_copy_here(h, id.as_ptr(), b, l)),
        ] {
            assert_eq!(r["ok"], false, "{r}");
            assert_eq!(r["code"], "unsupported", "{r}");
            assert_eq!(r["noticeKey"], "unsupported", "{r}");
        }
        // The wire cap: the hub's words, before anything else.
        let big = format!(
            r#"{{"meta":{{"kind":"snapshot"}},"pad":"{}"}}"#,
            "x".repeat(dspi_core::hub_server::SNAPSHOT_WIRE_MAX_BYTES)
        );
        let doc = CString::new(big.clone()).unwrap();
        let r = reply(|b, l| dspi_hub_snapshots_put(h, std::ptr::null(), doc.as_ptr(), b, l));
        let core_says = dspi_core::DspiCore::new()
            .hub_snapshots_put(String::new(), big)
            .unwrap_err();
        assert_eq!(r["code"], "link", "{r}");
        assert_eq!(r["noticeKey"], "dlink.conflict", "{r}");
        assert_eq!(r["error"], core_says.to_string(), "{r}");
        assert!(
            r["error"]
                .as_str()
                .unwrap()
                .contains("a snapshot document is at most 61440 bytes on the DSPi Link (compact JSON); this one is "),
            "{r}"
        );
        assert!(dspi_open_demo(h));
        // Identify off a hub is the device's own: the demo has no EXT_IDENTIFY.
        let r = reply(|b, l| dspi_hub_identify(h, 5, 0xFF, b, l));
        assert_eq!(r["code"], "unsupported", "{r}");
        let r = reply(|b, l| dspi_snapshot_list_merged(h, std::ptr::null(), b, l));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["entries"], Value::Array(Vec::new()), "{r}");
        assert_eq!(r["hubListed"], false, "{r}");
        let r = reply(|b, l| dspi_hub_server_events(h, b, l));
        assert_eq!(r["value"], Value::Array(Vec::new()), "{r}");
        assert!(dspi_hub_server_set_listener(
            h,
            Some(heard),
            std::ptr::null_mut()
        ));
        assert!(dspi_hub_server_set_listener(h, None, std::ptr::null_mut()));
        assert!(!dspi_hub_server_set_listener(
            std::ptr::null_mut(),
            None,
            std::ptr::null_mut()
        ));
        dspi_core_free(h);
    }
}
