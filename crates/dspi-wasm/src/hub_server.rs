//! The hub's own families on the web (WP P2CC1), over `dspi_core::hub_server`
//! (`docs/integrator/hub-api.md`), through this session's DSPi Link:
//!
//! - `hubCaps`: which families the hub has (no I/O).
//! - `hubInventory`, `hubHealth`, `hubStats`: reads.
//! - `hubIdentify {seconds, toneOutput?}`: the open device, through the hub.
//! - `hubEventsRead {serial?, afterId?, limit?}`, `hubEventsWatch {on, serial?}`.
//! - `hubServerEvents`: the hub's `event.added`, `inventory.changed` and
//!   `snapshot.changed` since the last call (also handed out by `linkTick` as
//!   `hubServerEvents`).
//! - `hubSnapshotsList {serial?}`, `hubSnapshotsGet {id}`, `hubSnapshotsPut
//!   {serial?, text}` (over 61 440 bytes compact is refused here with the
//!   hub's words, nothing sent), `hubSnapshotsRename {id, name}`,
//!   `hubSnapshotsNote {id, note}`, `hubSnapshotsDelete {id}`,
//!   `hubSnapshotsRecall {serial?, id, guard?, timeoutMs?}`, and
//!   `snapshotListMerged {serial?}` (the web keeps no store of its own: the
//!   hub's rows only).
//!
//! Without the family's cap every call answers `unsupported` and sends
//! nothing. The hub's refusals carry the core's `noticeKey` (`dlink.not_found`,
//! `dlink.offline`, `dlink.conflict`, ...).

use dspi_core::DspiCore;
use serde::Deserialize;
use serde_json::Value;

use crate::calls::{CallError, tidy};

type CallResult = Result<Value, CallError>;

fn params<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, CallError> {
    // No parameters is every field's default.
    let v = if v.is_null() {
        Value::Object(Default::default())
    } else {
        v
    };
    serde_json::from_value(v).map_err(|e| CallError::new("invalid", format!("bad parameters: {e}")))
}

fn record<T: serde::Serialize>(r: dspi_core::Result<T>) -> CallResult {
    Ok(tidy(&r?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Identify {
    seconds: u8,
    #[serde(default)]
    tone_output: Option<u8>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct EventsRead {
    serial: String,
    after_id: u64,
    limit: u32,
}

#[derive(Deserialize)]
struct Watch {
    on: bool,
    #[serde(default)]
    serial: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Serial {
    serial: String,
}

#[derive(Deserialize)]
struct Id {
    id: String,
}

#[derive(Deserialize)]
struct Put {
    #[serde(default)]
    serial: String,
    text: String,
}

#[derive(Deserialize)]
struct Rename {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct Note {
    id: String,
    note: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Recall {
    #[serde(default)]
    serial: String,
    id: String,
    #[serde(default)]
    guard: bool,
    #[serde(default)]
    timeout_ms: u32,
}

/// The calls of this module, `None` for another method.
pub fn dispatch(core: &DspiCore, method: &str, p: Value) -> Option<CallResult> {
    Some(match method {
        "hubCaps" => Ok(tidy(&core.hub_caps())),
        // -- WP FEATUI -- who works on the hub's devices (the `presence` family)
        "hubPresenceList" => record(core.hub_presence_list()),
        "hubInventory" => record(core.hub_inventory()),
        "hubHealth" => record(core.hub_health()),
        "hubStats" => record(core.hub_stats()),
        "hubIdentify" => {
            params::<Identify>(p).and_then(|x| record(core.hub_identify(x.seconds, x.tone_output)))
        }
        "hubEventsRead" => params::<EventsRead>(p)
            .and_then(|x| record(core.hub_events_read(x.serial, x.after_id, x.limit))),
        "hubEventsWatch" => params::<Watch>(p)
            .and_then(|x| record(core.hub_events_watch(x.on, x.serial).map(|()| Value::Null))),
        "hubServerEvents" => Ok(tidy(&core.hub_server_events())),
        "hubSnapshotsList" => {
            params::<Serial>(p).and_then(|x| record(core.hub_snapshots_list(x.serial)))
        }
        "hubSnapshotsGet" => params::<Id>(p).and_then(|x| record(core.hub_snapshots_get(x.id))),
        "hubSnapshotsPut" => {
            params::<Put>(p).and_then(|x| record(core.hub_snapshots_put(x.serial, x.text)))
        }
        "hubSnapshotsRename" => {
            params::<Rename>(p).and_then(|x| record(core.hub_snapshots_rename(x.id, x.name)))
        }
        "hubSnapshotsNote" => {
            params::<Note>(p).and_then(|x| record(core.hub_snapshots_note(x.id, x.note)))
        }
        "hubSnapshotsDelete" => params::<Id>(p)
            .and_then(|x| record(core.hub_snapshots_delete(x.id).map(|()| Value::Null))),
        "hubSnapshotsRecall" => params::<Recall>(p)
            .and_then(|x| record(core.hub_snapshots_recall(x.serial, x.id, x.guard, x.timeout_ms))),
        "snapshotListMerged" => {
            params::<Serial>(p).map(|x| tidy(&core.snapshot_list_merged(x.serial)))
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests_wp_p2cc1 {
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

    /// The same words as the core's own calls, on the web.
    #[test]
    fn wp_p2cc1_the_hub_calls_on_the_web() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        let caps = run(h, "hubCaps", Value::Null);
        assert_eq!(caps["value"]["onHub"], false, "{caps}");
        for (m, p) in [
            ("hubInventory", Value::Null),
            ("hubHealth", Value::Null),
            ("hubStats", Value::Null),
            ("hubEventsRead", Value::Null),
            ("hubEventsWatch", json!({"on": true})),
            ("hubSnapshotsList", Value::Null),
            ("hubSnapshotsGet", json!({"id": "1"})),
            ("hubSnapshotsRename", json!({"id": "1", "name": "n"})),
            ("hubSnapshotsNote", json!({"id": "1", "note": "n"})),
            ("hubSnapshotsDelete", json!({"id": "1"})),
            ("hubSnapshotsRecall", json!({"id": "1"})),
        ] {
            let r = run(h, m, p);
            assert_eq!(r["ok"], false, "{m}: {r}");
            assert_eq!(r["code"], "unsupported", "{m}: {r}");
            assert_eq!(r["noticeKey"], "unsupported", "{m}: {r}");
        }
        let big = json!({"meta": {"kind": "snapshot"},
            "pad": "x".repeat(dspi_core::hub_server::SNAPSHOT_WIRE_MAX_BYTES)})
        .to_string();
        let core_says = dspi_core::DspiCore::new()
            .hub_snapshots_put(String::new(), big.clone())
            .unwrap_err();
        let r = run(h, "hubSnapshotsPut", json!({"text": big}));
        assert_eq!(r["code"], "link", "{r}");
        assert_eq!(r["noticeKey"], "dlink.conflict", "{r}");
        assert_eq!(r["error"], core_says.to_string(), "{r}");
        assert_eq!(run(h, "open", json!({ "kind": "demo" }))["ok"], true);
        let r = run(h, "snapshotListMerged", Value::Null);
        assert_eq!(r["value"]["entries"], json!([]), "{r}");
        assert_eq!(r["value"]["hubListed"], false, "{r}");
        let r = run(h, "hubServerEvents", Value::Null);
        assert_eq!(r["value"], json!([]), "{r}");
        let r = run(h, "hubIdentify", json!({"seconds": 5}));
        assert_eq!(
            r["code"], "unsupported",
            "the demo has no EXT_IDENTIFY: {r}"
        );
    }
}
