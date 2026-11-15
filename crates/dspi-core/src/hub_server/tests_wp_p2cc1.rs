//! WP P2CC1: the typed calls for the hub's own families, against the fake hub
//! (`dspi-link`'s `fake-hub`, answering these families through its JSON
//! responder) and without one. The same calls against the real hub are in
//! `crates/dspi-hub/tests/wp_p2cc1_core_calls.rs`.

use serde_json::json;

use super::*;

fn info(id: &str, name: &str, saved_ms: i64) -> SnapshotInfo {
    SnapshotInfo {
        id: id.into(),
        name: name.into(),
        note: String::new(),
        serial: "S1".into(),
        saved_ms,
        platform: "RP2350".into(),
        input_channels: 2,
        output_channels: 9,
        wire_format: 32,
    }
}

#[test]
fn wp_p2cc1_merge_the_hub_wins_on_a_differing_id() {
    let same_l = info("0000000001000", "Same", 1000);
    let same_h = same_l.clone();
    let diff_l = info("0000000002000", "Mine", 2000);
    let diff_h = info("0000000002000", "Renamed on the hub", 2000);
    let only_l = info("0000000003000", "Local only", 3000);
    let only_h = info("0000000000500", "Hub only", 500);
    let rows = merge(
        vec![only_l.clone(), diff_l.clone(), same_l],
        vec![diff_h.clone(), same_h.clone(), only_h.clone()],
    );
    let got: Vec<(&str, &str, bool)> = rows
        .iter()
        .map(|r| (r.snapshot.name.as_str(), r.location.as_str(), r.local_copy))
        .collect();
    assert_eq!(
        got,
        [
            ("Hub only", WHERE_HUB, false),
            ("Same", WHERE_BOTH, false),
            // The hub's first, then this console's own, kept as a local copy
            ("Renamed on the hub", WHERE_HUB, false),
            ("Mine", WHERE_LOCAL, true),
            ("Local only", WHERE_LOCAL, false),
        ]
    );
    // `where` in JSON (C and wasm)
    let v = serde_json::to_value(&rows[1]).unwrap();
    assert_eq!(v["where"], "both", "{v}");
    assert_eq!(v["localCopy"], false);
    assert_eq!(v["snapshot"]["savedMs"], 1000);
}

#[test]
fn wp_p2cc1_the_wire_cap_refuses_locally_in_the_hubs_words() {
    let big = "x".repeat(SNAPSHOT_WIRE_MAX_BYTES);
    let doc = json!({"schemaVersion": 1, "meta": {"kind": "snapshot"}, "pad": big}).to_string();
    let len = doc.len();
    let e = wire_document(&doc).unwrap_err();
    assert_eq!(
        e,
        DspiError::Link {
            code: "conflict".into(),
            msg: format!(
                "a snapshot document is at most 61440 bytes on the DSPi Link (compact JSON); this one is {len}"
            ),
        }
    );
    assert_eq!(e.notice_key(), "dlink.conflict");
    // Measured compact: a pretty-printed text under the cap passes.
    let ok = json!({"meta": {"kind": "snapshot"}, "pad": "x".repeat(60_000)});
    let pretty = serde_json::to_string_pretty(&ok).unwrap();
    assert!(wire_document(&pretty).is_ok());
    assert!(matches!(
        wire_document("[1]"),
        Err(DspiError::InvalidArgument { .. })
    ));
}

#[test]
fn wp_p2cc1_the_hubs_codes_are_typed() {
    let link = |code: &str| DspiError::Link {
        code: code.into(),
        msg: "words".into(),
    };
    assert_eq!(hub_error(link("denied")), DspiError::Denied);
    assert_eq!(hub_error(link("unsupported")), DspiError::Unsupported);
    assert_eq!(hub_error(link("locked")), DspiError::Locked);
    assert!(matches!(
        hub_error(link("bad_request")),
        DspiError::InvalidArgument { .. }
    ));
    assert!(matches!(
        hub_error(link("limit_exceeded")),
        DspiError::LimitExceeded { .. }
    ));
    assert_eq!(
        hub_error(link("installer_lock")).notice_key(),
        "installer_lock.hub"
    );
    assert!(matches!(
        hub_error(link("image_refused")),
        DspiError::ImageRefused { .. }
    ));
    assert!(matches!(
        hub_error(link("confirmation_required")),
        DspiError::ConfirmationRequired { .. }
    ));
    for code in [
        HUB_ERR_NOT_FOUND,
        HUB_ERR_OFFLINE,
        HUB_ERR_CONFLICT,
        HUB_ERR_RATE_LIMITED,
        HUB_ERR_BUSY,
        HUB_ERR_DEVICE,
    ] {
        let e = hub_error(link(code));
        assert_eq!(e, link(code));
        // Each has its own words in the notice table.
        assert_eq!(e.notice_key(), format!("dlink.{code}"), "{code}");
    }
}

#[test]
fn wp_p2cc1_off_a_hub_the_calls_are_unsupported_and_identify_is_the_devices() {
    let core = DspiCore::new();
    let caps = core.hub_caps();
    assert!(!caps.on_hub && !caps.inventory && caps.caps.is_empty());
    assert_eq!(core.hub_inventory().unwrap_err(), DspiError::Unsupported);
    assert_eq!(core.hub_health().unwrap_err(), DspiError::Unsupported);
    assert_eq!(core.hub_stats().unwrap_err(), DspiError::Unsupported);
    assert_eq!(
        core.hub_snapshots_list(String::new()).unwrap_err(),
        DspiError::Unsupported
    );
    // Connected directly: the device's own identify, then its state (wp/FWX
    // firmware); firmware without it is unsupported, as `identify` says.
    let p = crate::ext::tests_wp_c8::probed(
        dspi_transport::Capture::demo().bulk,
        dspi_transport::FakeOptions {
            diag_ext: true,
            ..crate::ext::tests_wp_c8::k340_options()
        },
        crate::ext::tests_wp_c8::Faults::default(),
    );
    let got = p
        .core
        .hub_identify(5, None)
        .unwrap()
        .expect("the state read");
    assert!((4..=5).contains(&got.remaining_s), "{got:?}");
    assert!(matches!(
        p.core.hub_identify(5, Some(9)),
        Err(DspiError::InvalidArgument { .. })
    ));
    core.open_demo().unwrap();
    assert_eq!(
        core.hub_identify(5, None),
        core.identify(5, None).map(|()| None)
    );
    assert_eq!(
        core.hub_events_read(String::new(), 0, 10).unwrap_err(),
        DspiError::Unsupported
    );
    // The merged list: local only, no hub row, no notice.
    let merged = core.snapshot_list_merged(String::new());
    assert!(!merged.hub_listed && merged.hub_notice_key.is_empty());
}

#[test]
fn wp_p2cc1_events_are_read_from_the_hubs_messages() {
    let e = HubServerEvent::from_hub(
        "event.added",
        &json!({"v": 1, "hub": "h", "serial": "S1", "record": {
            "v": 1, "id": 7, "t": 1_700_000_000_000_i64, "kind": "identify", "serial": "S1",
            "text": "Identify", "session": 3, "by": "Bench Mac", "value": 5.0, "count": 1}}),
    )
    .unwrap();
    let r = e.record.unwrap();
    assert_eq!((r.id, r.by.as_str(), r.session), (7, "Bench Mac", Some(3)));
    // A kind this build does not know reads as `unknown`.
    let e = HubServerEvent::from_hub(
        "event.added",
        &json!({"serial": "S1", "record": {"id": 8, "t": 1, "kind": "teleport", "text": "?"}}),
    )
    .unwrap();
    assert_eq!(e.record.unwrap().kind, "unknown");
    let e = HubServerEvent::from_hub(
        "inventory.changed",
        &json!({"serial": "S1", "device": null}),
    )
    .unwrap();
    assert!(e.device.is_none());
    assert!(HubServerEvent::from_hub("limits.changed", &json!({})).is_none());
}

#[cfg(feature = "link")]
mod through_the_fake_hub {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use dspi_link::fakehub::{FakeHub, HubConfig, JsonResponder};

    use super::*;

    const SERIAL: &str = "0123456789ABCDEF";
    const FAMILY_CAPS: [&str; 6] = [
        CAP_INVENTORY,
        CAP_IDENTIFY,
        CAP_HEALTH,
        CAP_EVENTS,
        CAP_HUB_SNAPSHOTS,
        CAP_HUB_STATS,
    ];

    fn hub(with_caps: bool) -> FakeHub {
        let mut cfg = HubConfig::default();
        if with_caps {
            cfg.caps.extend(FAMILY_CAPS.iter().map(|c| (*c).to_owned()));
        }
        FakeHub::start(cfg).unwrap()
    }

    fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
        let start = Instant::now();
        while !f() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "timed out waiting for {what}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn bound(hub: &FakeHub) -> Arc<DspiCore> {
        let core = DspiCore::new();
        core.link_connect(
            hub.url(),
            "wp-p2cc1".into(),
            "test".into(),
            String::new(),
            String::new(),
        )
        .unwrap();
        wait_until("the device list", || !core.link_devices().is_empty());
        let d = core.link_devices().remove(0);
        core.link_select_device(d.handle, d.serial).unwrap();
        core.ensure_probed();
        core
    }

    /// The requests of these families the hub got.
    fn sent(hub: &FakeHub) -> Vec<Value> {
        hub.json_requests()
            .into_iter()
            .filter(|r| {
                let t = r["t"].as_str().unwrap_or("");
                [
                    "inventory.",
                    "health.",
                    "hub.",
                    "event.",
                    "snapshot.",
                    "device.identify",
                ]
                .iter()
                .any(|p| t.starts_with(p))
            })
            .collect()
    }

    fn entry(id: &str, name: &str, saved_ms: i64) -> Value {
        json!({"id": id, "name": name, "note": "", "serial": SERIAL, "savedMs": saved_ms,
               "platform": "RP2350", "inputChannels": 2, "outputChannels": 9, "wireFormat": 32})
    }

    /// Answers like the real hub's, from `hub-api.md`.
    fn answers(snapshots: Arc<Mutex<Vec<Value>>>) -> JsonResponder {
        Arc::new(move |r: &Value| {
            let t = r["t"].as_str()?;
            Some(match t {
                "inventory.list" => {
                    let offset = r["offset"].as_u64().unwrap_or(0);
                    let dev = |s: &str, online: bool| {
                        let handle = if online { json!(0) } else { Value::Null };
                        json!({"serial": s, "handle": handle,
                               "online": online, "name": "Bench", "platform": 1,
                               "fw": "1.2.3", "fw_beta": false, "outputs": 9, "board": "pico2",
                               "chip": "RP2350", "chip_rev": 2, "location": "Rack 2",
                               "note": null, "tags": ["stage"],
                               "first_seen": "2026-11-01T10:00:00Z", "last_seen": "2026-11-15T10:00:00Z",
                               "last_reset": {"reason": "watchdog", "prev_uptime_s": 60,
                                              "at": "2026-11-15T09:59:00Z"}})
                    };
                    if offset == 0 {
                        json!({"v": 1, "devices": [dev(SERIAL, true)], "total": 2, "next_offset": 1})
                    } else {
                        json!({"v": 1, "devices": [dev("OLD1", false)], "total": 2, "next_offset": null})
                    }
                }
                "health.get" => json!({"v": 1, "devices": [{"serial": SERIAL, "handle": 0,
                    "at": "2026-11-15T10:00:00Z", "cpu_pct": 41, "clip_mask": 3, "temp_c": 38.25,
                    "uptime_s": 600, "hang_count": 0, "unresponsive": false,
                    "reset": {"reason": "power_on"}}]}),
                "hub.stats" => json!({"v": 1, "sessions": 2, "uptime_s": 8812, "devices": [
                    {"handle": 0, "serial": SERIAL, "cmds": 18233, "errors": 2,
                     "avg_rtt_ms": 1.4, "notify_dropped": 0}]}),
                "device.identify" => {
                    if r.get("tone_output").and_then(Value::as_u64) == Some(99) {
                        json!({"code": "bad_request", "msg": "no output 99"})
                    } else if r["seconds"] == 7 {
                        json!({"code": "rate_limited", "msg": "one request per device per 2 s",
                               "retry_in_ms": 1500})
                    } else {
                        json!({"v": 1, "serial": r["serial"], "status": {"remaining_s": r["seconds"],
                            "last_tone": null, "chip": "RP2350", "chip_rev": 2, "board": "pico2",
                            "hw_rev": "B"}})
                    }
                }
                "event.list" => json!({"v": 1, "serial": r["serial"], "records": [
                    {"v": 1, "id": 4, "t": 1_700_000_000_000_i64, "kind": "notification",
                     "serial": SERIAL, "text": "Master volume -20 dB", "session": 5, "by": "Phone"}],
                    "more": false, "newest_id": 4}),
                "event.watch" => json!({"v": 1, "on": r["on"], "serial": r.get("serial")}),
                "snapshot.list" => {
                    json!({"v": 1, "serial": r["serial"], "snapshots": snapshots.lock().unwrap().clone()})
                }
                "snapshot.get" => {
                    let id = r["snapshot"].as_str().unwrap_or("");
                    if id == "missing" {
                        json!({"code": "not_found", "msg": "no snapshot missing"})
                    } else {
                        let doc = json!({"schemaVersion": 1, "meta": {"kind": "snapshot", "name": "From the hub"},
                            "channels": [{}], "snapshot": {"id": id, "serial": SERIAL, "savedMs": 4000}});
                        json!({"v": 1, "snapshot": entry(id, "From the hub", 4000), "document": doc.to_string()})
                    }
                }
                "snapshot.put" => {
                    let doc = &r["document"];
                    let id = doc["snapshot"]["id"].as_str().unwrap_or("").to_owned();
                    let mut held = snapshots.lock().unwrap();
                    let created = !held.iter().any(|e| e["id"] == id.as_str());
                    let e = entry(&id, doc["meta"]["name"].as_str().unwrap_or(""), 0);
                    if created {
                        held.push(e.clone());
                    }
                    json!({"v": 1, "snapshot": e, "created": created})
                }
                "snapshot.rename" => {
                    json!({"v": 1, "snapshot": entry(r["snapshot"].as_str()?, r["name"].as_str()?, 1)})
                }
                "snapshot.note" => {
                    json!({"v": 1, "snapshot": entry(r["snapshot"].as_str()?, "n", 1)})
                }
                "snapshot.delete" => json!({"code": "denied", "msg": "admin only"}),
                "snapshot.recall" => {
                    json!({"v": 1, "serial": r["serial"], "snapshot": r["snapshot"],
                    "name": "Show", "image": true, "lines": ["Master volume"], "notes": [],
                    "guard": 12})
                }
                _ => return None,
            })
        })
    }

    #[test]
    fn wp_p2cc1_without_the_caps_every_call_is_unsupported_and_sends_nothing() {
        let hub = hub(false);
        let core = bound(&hub);
        let caps = core.hub_caps();
        assert!(caps.on_hub, "{caps:?}");
        assert!(!caps.inventory && !caps.identify && !caps.health && !caps.events);
        assert!(!caps.hub_snapshots && !caps.hub_stats);
        let u = DspiError::Unsupported;
        assert_eq!(core.hub_inventory().unwrap_err(), u);
        assert_eq!(core.hub_identify(5, None).unwrap_err(), u);
        assert_eq!(core.hub_health().unwrap_err(), u);
        assert_eq!(core.hub_stats().unwrap_err(), u);
        assert_eq!(core.hub_events_read(String::new(), 0, 0).unwrap_err(), u);
        assert_eq!(core.hub_events_watch(true, String::new()).unwrap_err(), u);
        assert_eq!(core.hub_snapshots_list(String::new()).unwrap_err(), u);
        assert_eq!(core.hub_snapshots_get("a".into()).unwrap_err(), u);
        assert_eq!(
            core.hub_snapshots_put(String::new(), "{}".into())
                .unwrap_err(),
            u
        );
        assert_eq!(
            core.hub_snapshots_rename("a".into(), "b".into())
                .unwrap_err(),
            u
        );
        assert_eq!(
            core.hub_snapshots_note("a".into(), "b".into()).unwrap_err(),
            u
        );
        assert_eq!(core.hub_snapshots_delete("a".into()).unwrap_err(), u);
        assert_eq!(
            core.hub_snapshots_recall(String::new(), "a".into(), false, 0)
                .unwrap_err(),
            u
        );
        assert_eq!(core.hub_snapshots_copy_here("a".into()).unwrap_err(), u);
        assert_eq!(core.hub_snapshots_save("a".into()).unwrap_err(), u);
        let merged = core.snapshot_list_merged(String::new());
        assert!(!merged.hub_listed && merged.hub_notice_key.is_empty());
        assert!(sent(&hub).is_empty(), "{:?}", sent(&hub));
        core.link_close();
    }

    #[test]
    fn wp_p2cc1_each_call_with_the_cap_through_the_fake_hub() {
        let hub = hub(true);
        let held = Arc::new(Mutex::new(vec![entry("0000000000100", "Hub one", 100)]));
        hub.set_json_responder(Some(answers(Arc::clone(&held))));
        let core = bound(&hub);
        let caps = core.hub_caps();
        assert!(caps.inventory && caps.identify && caps.health && caps.events);
        assert!(caps.hub_snapshots && caps.hub_stats && !caps.param_api);

        // The inventory, read page by page.
        let devices = core.hub_inventory().unwrap();
        assert_eq!(devices.len(), 2, "{devices:?}");
        let d = &devices[0];
        assert_eq!(
            (d.serial.as_str(), d.handle, d.online),
            (SERIAL, Some(0), true)
        );
        assert_eq!(
            (d.fw_beta, d.chip_rev, d.location.as_str()),
            (Some(false), Some(2), "Rack 2")
        );
        assert_eq!(d.note, "");
        assert_eq!(d.tags, ["stage"]);
        assert_eq!(d.platform, "RP2350");
        let reset = d.last_reset.clone().unwrap();
        assert_eq!(
            (reset.reason.as_str(), reset.prev_uptime_s),
            ("watchdog", Some(60))
        );
        assert_eq!(devices[1].handle, None);
        let offsets: Vec<Value> = sent(&hub)
            .iter()
            .filter(|r| r["t"] == "inventory.list")
            .map(|r| r["offset"].clone())
            .collect();
        assert_eq!(offsets, [json!(0), json!(1)]);

        let health = core.hub_health().unwrap();
        assert_eq!(health[0].cpu_pct, Some(41));
        assert_eq!(health[0].temp_c, Some(38.25));
        assert_eq!(health[0].reset.as_ref().unwrap().prev_uptime_s, None);
        let stats = core.hub_stats().unwrap();
        assert_eq!((stats.sessions, stats.uptime_s), (2, 8812));
        assert_eq!(stats.devices[0].cmds, 18233);

        // Identify on a hub is the hub's message, for the open device.
        let st = core.hub_identify(5, Some(1)).unwrap().unwrap();
        assert_eq!((st.remaining_s, st.hw_rev.as_str()), (5, "B"));
        let ask = sent(&hub)
            .into_iter()
            .rfind(|r| r["t"] == "device.identify")
            .unwrap();
        assert_eq!(ask["serial"], SERIAL);
        assert_eq!(ask["tone_output"], 1);
        assert!(matches!(
            core.hub_identify(5, Some(99)).unwrap_err(),
            DspiError::InvalidArgument { .. }
        ));
        let e = core.hub_identify(7, None).unwrap_err();
        assert_eq!(e.notice_key(), "dlink.rate_limited", "{e:?}");

        let page = core.hub_events_read(String::new(), 0, 1000).unwrap();
        assert_eq!(page.serial, SERIAL);
        assert_eq!(page.records[0].by, "Phone");
        assert_eq!(page.records[0].session, Some(5));
        assert_eq!(page.newest_id, 4);
        let ask = sent(&hub)
            .into_iter()
            .rfind(|r| r["t"] == "event.list")
            .unwrap();
        assert_eq!(ask["limit"], 500, "capped: {ask}");
        core.hub_events_watch(true, String::new()).unwrap();

        // The hub's store.
        let list = core.hub_snapshots_list(String::new()).unwrap();
        assert_eq!(list[0].name, "Hub one");
        let got = core.hub_snapshots_get("0000000000100".into()).unwrap();
        assert_eq!(got.snapshot.saved_ms, 4000);
        assert!(got.document.contains("From the hub"));
        let e = core.hub_snapshots_get("missing".into()).unwrap_err();
        assert_eq!(
            e,
            DspiError::Link {
                code: "not_found".into(),
                msg: "no snapshot missing".into()
            }
        );
        let doc = json!({"schemaVersion": 1, "meta": {"kind": "snapshot", "name": "Put"},
            "channels": [{}], "snapshot": {"id": "0000000000200", "serial": SERIAL, "savedMs": 200}})
        .to_string();
        let put = core.hub_snapshots_put(String::new(), doc.clone()).unwrap();
        assert!(put.created);
        let ask = sent(&hub)
            .into_iter()
            .rfind(|r| r["t"] == "snapshot.put")
            .unwrap();
        // The serial its block names; the document as the JSON object.
        assert_eq!(ask["serial"], SERIAL);
        assert!(ask["document"].is_object());
        assert!(!core.hub_snapshots_put(String::new(), doc).unwrap().created);
        assert_eq!(
            core.hub_snapshots_rename("0000000000200".into(), "New".into())
                .unwrap()
                .name,
            "New"
        );
        core.hub_snapshots_note("0000000000200".into(), "n".into())
            .unwrap();
        assert_eq!(
            core.hub_snapshots_delete("0000000000200".into())
                .unwrap_err(),
            DspiError::Denied
        );
        let r = core
            .hub_snapshots_recall(String::new(), "0000000000100".into(), true, 5000)
            .unwrap();
        assert_eq!(
            (r.image, r.guard.as_str(), r.name.as_str()),
            (true, "12", "Show")
        );
        let ask = sent(&hub)
            .into_iter()
            .rfind(|r| r["t"] == "snapshot.recall")
            .unwrap();
        assert_eq!(
            (ask["guard"].clone(), ask["timeout_ms"].clone()),
            (json!(true), json!(5000))
        );

        // Too large: refused here, in the hub's words, and nothing sent.
        let before = sent(&hub).len();
        let big = json!({"meta": {"kind": "snapshot"}, "snapshot": {"serial": SERIAL},
            "pad": "x".repeat(SNAPSHOT_WIRE_MAX_BYTES)})
        .to_string();
        let e = core.hub_snapshots_put(String::new(), big).unwrap_err();
        assert!(
            matches!(&e, DspiError::Link { code, msg } if code == "conflict"
                && msg.starts_with("a snapshot document is at most 61440 bytes on the DSPi Link (compact JSON); this one is ")),
            "{e:?}"
        );
        assert_eq!(sent(&hub).len(), before);
        core.link_close();
    }

    #[test]
    fn wp_p2cc1_read_only_mode_refuses_every_hub_write_and_sends_nothing() {
        let hub = hub(true);
        hub.set_json_responder(Some(answers(Arc::new(Mutex::new(Vec::new())))));
        let core = bound(&hub);
        core.set_client_read_only(true).unwrap();
        let before = sent(&hub).len();
        let r = DspiError::ReadOnly;
        let doc = json!({"schemaVersion": 1, "meta": {"kind": "snapshot"}, "channels": [{}],
            "snapshot": {"id": "0000000000200", "serial": SERIAL}})
        .to_string();
        assert_eq!(core.hub_identify(5, None).unwrap_err(), r);
        assert_eq!(core.hub_snapshots_put(String::new(), doc).unwrap_err(), r);
        assert_eq!(
            core.hub_snapshots_rename("a".into(), "b".into())
                .unwrap_err(),
            r
        );
        assert_eq!(
            core.hub_snapshots_note("a".into(), "b".into()).unwrap_err(),
            r
        );
        assert_eq!(core.hub_snapshots_delete("a".into()).unwrap_err(), r);
        assert_eq!(
            core.hub_snapshots_recall(String::new(), "a".into(), false, 0)
                .unwrap_err(),
            r
        );
        // The hub's other writes (inventory.set/forget, hub.rename) are refused
        // by the same check; no typed call sends them yet.
        for t in ["inventory.set", "inventory.forget", "hub.rename"] {
            assert_eq!(
                core.link_hub_request(t, serde_json::Map::new())
                    .unwrap_err(),
                r,
                "{t}"
            );
        }
        assert_eq!(sent(&hub).len(), before, "{:?}", sent(&hub));
        // The reads go on.
        assert_eq!(core.hub_inventory().unwrap().len(), 2);
        core.hub_health().unwrap();
        core.hub_stats().unwrap();
        core.hub_events_read(String::new(), 0, 10).unwrap();
        core.hub_events_watch(true, String::new()).unwrap();
        core.hub_snapshots_list(String::new()).unwrap();
        core.hub_snapshots_get("x".into()).unwrap();
        core.link_close();
    }

    #[derive(Default)]
    struct Heard(Mutex<Vec<HubServerEvent>>);

    impl HubServerListener for Heard {
        fn on_hub_server_event(&self, event: HubServerEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[test]
    fn wp_p2cc1_the_hubs_events_reach_the_listener_and_the_queue() {
        let hub = hub(true);
        let core = bound(&hub);
        let heard = Arc::new(Heard::default());
        core.set_hub_server_listener(Some(heard.clone()));
        hub.send_event(json!({"t": "limits.changed", "v": 1, "serial": SERIAL}));
        hub.send_event(json!({"t": "event.added", "v": 1, "hub": "h", "serial": SERIAL,
            "record": {"v": 1, "id": 9, "t": 5, "kind": "identify", "text": "Identify", "by": "Phone"}}));
        hub.send_event(
            json!({"t": "inventory.changed", "v": 1, "hub": "h", "serial": SERIAL,
            "device": {"serial": SERIAL, "online": true, "handle": 0}}),
        );
        hub.send_event(
            json!({"t": "snapshot.changed", "v": 1, "hub": "h", "serial": SERIAL,
            "snapshot": "0000000000100", "change": "rename"}),
        );
        wait_until("three events", || heard.0.lock().unwrap().len() >= 3);
        let kinds: Vec<String> = heard
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.kind.clone())
            .collect();
        assert_eq!(
            kinds,
            ["event.added", "inventory.changed", "snapshot.changed"]
        );
        let queued = core.hub_server_events();
        assert_eq!(queued.len(), 3);
        assert_eq!(queued[0].record.as_ref().unwrap().by, "Phone");
        assert_eq!(queued[1].device.as_ref().unwrap().handle, Some(0));
        assert_eq!(
            (queued[2].snapshot.as_str(), queued[2].change.as_str()),
            ("0000000000100", "rename")
        );
        assert!(core.hub_server_events().is_empty());
        core.set_hub_server_listener(None);
        core.link_close();
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        // `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
        let dir = std::env::temp_dir()
            .join("wp-p2cc1")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn local_doc(name: &str, saved_ms: i64) -> String {
        json!({"schemaVersion": 1, "meta": {"kind": "snapshot", "name": name}, "channels": [{}],
               "snapshot": {"serial": SERIAL, "savedMs": saved_ms}})
        .to_string()
    }

    #[test]
    fn wp_p2cc1_the_merged_list_save_to_the_hub_and_copy_here() {
        let hub = hub(true);
        let held = Arc::new(Mutex::new(Vec::new()));
        hub.set_json_responder(Some(answers(Arc::clone(&held))));
        let core = bound(&hub);
        let dir = scratch("merge");
        core.snapshot_set_dir(dir.to_string_lossy().into_owned())
            .unwrap();
        let a = core
            .snapshot_import(local_doc("Both", 1000), String::new(), String::new())
            .unwrap();
        let b = core
            .snapshot_import(local_doc("Mine", 2000), String::new(), String::new())
            .unwrap();
        let c = core
            .snapshot_import(local_doc("Local only", 3000), String::new(), String::new())
            .unwrap();

        // "Save to the hub": the local document, under its own id.
        let put = core.hub_snapshots_save(a.id.clone()).unwrap();
        assert_eq!(put.snapshot.id, a.id);
        let ask = sent(&hub)
            .into_iter()
            .rfind(|r| r["t"] == "snapshot.put")
            .unwrap();
        assert_eq!(ask["document"]["snapshot"]["id"], a.id.as_str());

        // The hub's copy of `a` the same, of `b` renamed there, and one of its own.
        let to_json = |i: &SnapshotInfo| serde_json::to_value(i).unwrap();
        let mut b_hub = b.clone();
        b_hub.name = "Renamed on the hub".into();
        *held.lock().unwrap() = vec![
            to_json(&a),
            to_json(&b_hub),
            entry("0000000000500", "Hub only", 500),
        ];
        let merged = core.snapshot_list_merged(String::new());
        assert!(merged.hub_listed, "{merged:?}");
        let rows: Vec<(&str, &str, bool)> = merged
            .entries
            .iter()
            .map(|r| (r.snapshot.name.as_str(), r.location.as_str(), r.local_copy))
            .collect();
        assert_eq!(
            rows,
            [
                ("Hub only", "hub", false),
                ("Both", "both", false),
                ("Renamed on the hub", "hub", false),
                ("Mine", "local", true),
                ("Local only", "local", false),
            ]
        );
        assert_eq!(merged.entries[4].snapshot.id, c.id);

        // "Copy here": the hub's document filed here; nothing overwritten.
        let copied = core
            .hub_snapshots_copy_here("0000000000500".into())
            .unwrap();
        assert_eq!(copied.name, "From the hub");
        let mine = core.snapshot_list(SERIAL.into());
        assert_eq!(mine.len(), 4, "{mine:?}");
        assert!(
            mine.iter().any(|m| m.name == "Mine"),
            "the local copy stays"
        );

        // A hub that cannot answer: the local rows, with the reason.
        core.link_close();
        let merged = core.snapshot_list_merged(SERIAL.into());
        assert_eq!(merged.entries.len(), 4);
        assert!(merged.entries.iter().all(|r| r.location == "local"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
