//! WP P3CC: the hub's fleet families without a hub, and the readers of the
//! hub's answers. The calls against the real hub (each with and without its
//! cap, the roles, a rollout end to end with the fake writer) are in
//! `crates/dspi-hub/tests/wp_p3cc_core_calls.rs`.

use serde_json::json;

use super::*;
use crate::hub_server::{HUB_SERVER_READS, HubServerEvent};

fn group_target() -> HubFleetTarget {
    HubFleetTarget {
        group: "g1".into(),
        serials: Vec::new(),
    }
}

#[test]
fn wp_p3cc_off_a_hub_every_call_is_unsupported() {
    let core = DspiCore::new();
    let u = DspiError::Unsupported;
    assert_eq!(core.hub_groups().unwrap_err(), u);
    assert_eq!(core.hub_group_set(HubGroup::default()).unwrap_err(), u);
    assert_eq!(core.hub_group_delete("g1".into()).unwrap_err(), u);
    assert_eq!(
        core.hub_group_param_get(group_target(), "vol.master".into(), vec![])
            .unwrap_err(),
        u
    );
    assert_eq!(
        core.hub_group_param_set(
            group_target(),
            "vol.master".into(),
            vec![],
            ParamValue::Float(-20.0),
            vec![]
        )
        .unwrap_err(),
        u
    );
    assert_eq!(
        core.hub_group_preset_load(group_target(), Some(1), String::new(), vec![])
            .unwrap_err(),
        u
    );
    assert_eq!(
        core.hub_group_apply(
            group_target(),
            "S1-1".into(),
            String::new(),
            String::new(),
            true,
            0
        )
        .unwrap_err(),
        u
    );
    assert_eq!(core.hub_group_apply_confirm("a1".into()).unwrap_err(), u);
    assert_eq!(core.hub_group_apply_cancel("a1".into()).unwrap_err(), u);
    assert_eq!(core.hub_firmware_images().unwrap_err(), u);
    assert_eq!(
        core.hub_firmware_put("x.uf2".into(), vec![1, 2, 3])
            .unwrap_err(),
        u
    );
    assert_eq!(core.hub_firmware_delete("x.uf2".into()).unwrap_err(), u);
    assert_eq!(
        core.hub_rollout_start(group_target(), "x.uf2".into(), false)
            .unwrap_err(),
        u
    );
    assert_eq!(core.hub_rollouts(String::new()).unwrap_err(), u);
    assert_eq!(core.hub_rollout_cancel("ro1".into()).unwrap_err(), u);
}

#[test]
fn wp_p3cc_the_caller_s_mistakes_are_refused_before_anything() {
    let core = DspiCore::new();
    let invalid = |r: Result<()>| matches!(r, Err(DspiError::InvalidArgument { .. }));
    assert!(invalid(core.hub_group_delete(String::new())));
    assert!(invalid(
        core.hub_group_apply_confirm(String::new()).map(|_| ())
    ));
    assert!(invalid(
        core.hub_firmware_put("x.uf2".into(), Vec::new())
            .map(|_| ())
    ));
    assert!(invalid(
        core.hub_firmware_put("x.uf2".into(), vec![0; FIRMWARE_MAX_BYTES + 1])
            .map(|_| ())
    ));
    assert!(invalid(
        core.hub_firmware_put_base64("x.uf2".into(), "not base64!")
            .map(|_| ())
    ));
    // Targets: one of the two.
    let mut f = Map::new();
    assert!(address(HubFleetTarget::default(), &mut f).is_err());
    assert!(
        address(
            HubFleetTarget {
                group: "g1".into(),
                serials: vec!["A".into()]
            },
            &mut f
        )
        .is_err()
    );
    assert!(f.is_empty());
    address(group_target(), &mut f).unwrap();
    assert_eq!(Value::Object(f), json!({"group": "g1"}));
    let mut f = Map::new();
    address(
        HubFleetTarget {
            group: String::new(),
            serials: vec!["A".into(), "B".into()],
        },
        &mut f,
    )
    .unwrap();
    assert_eq!(Value::Object(f), json!({"targets": ["A", "B"]}));
    // A preset: slot or name.
    assert!(preset_ref(None, "").is_err());
    assert!(preset_ref(Some(1), "Live").is_err());
    assert_eq!(preset_ref(Some(3), "").unwrap(), json!(3));
    assert_eq!(preset_ref(None, "Live").unwrap(), json!("Live"));
    // A structured value is not a group write's.
    assert!(value_to_wire(&ParamValue::Bytes(vec![1])).is_err());
    assert_eq!(
        value_to_wire(&ParamValue::Float(-0.1)).unwrap(),
        json!(-0.1)
    );
    assert_eq!(value_to_wire(&ParamValue::Bool(true)).unwrap(), json!(true));
    assert_eq!(value_to_wire(&ParamValue::Trigger).unwrap(), Value::Null);
}

#[test]
fn wp_p3cc_base64_both_ways() {
    for (raw, b64) in [
        (&b""[..], ""),
        (b"f", "Zg=="),
        (b"fo", "Zm8="),
        (b"foo", "Zm9v"),
        (b"foobar", "Zm9vYmFy"),
    ] {
        assert_eq!(base64(raw), b64);
        assert_eq!(unbase64(b64).unwrap(), raw);
    }
    let all: Vec<u8> = (0..=255).collect();
    assert_eq!(unbase64(&base64(&all)).unwrap(), all);
    for bad in ["Zg=", "Z===", "Zg==Zg==", "Zm9*", "Zm9v\u{e9}"] {
        assert!(unbase64(bad).is_none(), "{bad}");
    }
    // A chunk of image is 49 152 base64 characters.
    assert_eq!(base64(&vec![0; CHUNK_BYTES]).len(), 49_152);
}

#[test]
fn wp_p3cc_reads_the_hub_s_rows_and_records() {
    let row = row_from(
        "vol.master",
        &json!({"serial": "A", "ok": true, "value": -23.0, "clamped": true}),
    );
    assert!(row.ok && row.clamped);
    assert_eq!(row.value, Some(ParamValue::Float(-23.0)));
    assert_eq!(row.notice_key, "");
    let row = row_from(
        "vol.mute",
        &json!({"serial": "B", "ok": false, "code": "offline", "msg": "not attached"}),
    );
    assert_eq!(
        (row.code.as_str(), row.notice_key.as_str()),
        ("offline", "dlink.offline")
    );
    let row = row_from(
        "",
        &json!({"serial": "C", "ok": false, "code": "limit_exceeded", "limit": "vol.master"}),
    );
    assert_eq!(row.notice_key, "limit_exceeded");
    assert_eq!(row.limit, "vol.master");
    let row = row_from(
        "",
        &json!({"serial": "D", "ok": true, "image": true, "lines": ["x"], "guard": 17}),
    );
    assert_eq!((row.image, row.guard), (true, Some(17)));

    let r = rollout_from(&json!({
        "id": "ro1", "target": "group:g1", "state": "failed", "by": "Bench",
        "image": {"name": "a.uf2", "sha256": "ab", "firmware_version": "1.1.7"},
        "while_live": false, "started_at": "t0", "ended_at": null,
        "devices": [{"serial": "A", "state": "failed", "step": "find",
            "steps": [{"step": "check", "at": "t0", "ok": true, "text": "fine"},
                      {"step": "bootloader", "at": "t1", "ok": true, "text": "sent", "session": 4097},
                      {"step": "teleport", "at": "t2", "ok": false, "text": "?"}],
            "before": {"fw": "1.1.6", "ext_list": [1, 2], "board": {"required_mv": 1200,
                "source": "extCoreVreg"}, "vreg": {"live_mv": 1200, "stored_mv": 1200,
                "default_mv": 1200}, "export": "S1"},
            "after": null,
            "failed": {"step": "find", "code": null, "text": "no board"}},
          {"serial": "B", "state": "not_tried", "step": "", "steps": [], "before": null,
            "after": null, "failed": null}]
    }));
    assert_eq!(r.image_version, "1.1.7");
    assert_eq!(r.ended_at, "");
    let a = &r.devices[0];
    assert_eq!(a.steps[0].notice_key, "rollout.check");
    assert_eq!(a.steps[1].session, Some(4097));
    assert_eq!(a.steps[2].notice_key, NOTICE_ROLLOUT_UNKNOWN);
    let before = a.before.as_ref().unwrap();
    assert_eq!((before.required_mv, before.ext_list.len()), (1200, 2));
    assert_eq!(before.vreg.unwrap().live_mv, Some(1200));
    assert_eq!(a.failed.as_ref().unwrap().notice_key, "");
    assert_eq!(r.devices[1].state, "not_tried");
}

#[test]
fn wp_p3cc_the_events_reach_the_listener_s_record() {
    let e = HubServerEvent::from_hub(
        "group.changed",
        &json!({"v": 1, "hub": "h", "change": "forgot",
            "group": {"id": "g2", "name": "Bar", "note": null, "members": ["A"]}}),
    )
    .unwrap();
    assert_eq!(e.change, "forgot");
    let g = e.group.unwrap();
    assert_eq!((g.id.as_str(), g.note.as_str()), ("g2", ""));
    let e = HubServerEvent::from_hub(
        "fw.rollout.changed",
        &json!({"v": 1, "hub": "h", "serial": "A", "step": "flash",
            "rollout": {"id": "ro2", "state": "running", "devices": []}}),
    )
    .unwrap();
    assert_eq!(e.serial, "A");
    assert_eq!(e.rollout.unwrap().id, "ro2");
    assert!(e.group.is_none());
}

#[test]
fn wp_p3cc_each_step_has_its_notice_and_the_reads_are_listed() {
    for step in ROLLOUT_STEPS {
        let n = hub_rollout_step_notice(step.into(), "the hub's line".into());
        assert_eq!(n.key, format!("rollout.{step}"));
        assert_eq!(n.detail, "the hub's line");
    }
    assert_eq!(
        hub_rollout_step_notice("teleport".into(), String::new()).key,
        NOTICE_ROLLOUT_UNKNOWN
    );
    for read in [
        "group.list",
        "group.param.get",
        "fw.image.list",
        "fw.rollout.status",
    ] {
        assert!(HUB_SERVER_READS.contains(&read), "{read}");
    }
    for write in [
        "group.set",
        "group.param.set",
        "group.apply",
        "fw.image.put",
        "fw.rollout.start",
    ] {
        assert!(!HUB_SERVER_READS.contains(&write), "{write}");
    }
    // tls_required has its row.
    let e = hub_code(HUB_ERR_TLS_REQUIRED.into(), String::new());
    assert_eq!(e.notice_key(), "dlink.tls_required");
}

#[cfg(feature = "link")]
mod through_the_fake_hub {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use dspi_link::fakehub::{FakeHub, HubConfig, JsonResponder};
    use serde_json::Value;

    use super::*;

    const CAPS: [&str; 4] = [CAP_GROUPS, CAP_GROUP_PARAMS, CAP_GROUP_APPLY, CAP_ROLLOUT];
    const FAMILIES: [&str; 2] = ["group.", "fw."];

    fn hub(with_caps: bool) -> FakeHub {
        let mut cfg = HubConfig::default();
        if with_caps {
            cfg.caps.extend(CAPS.iter().map(|c| (*c).to_owned()));
        }
        FakeHub::start(cfg).unwrap()
    }

    fn bound(hub: &FakeHub) -> Arc<DspiCore> {
        let core = DspiCore::new();
        core.link_connect(
            hub.url(),
            "wp-p3cc".into(),
            "test".into(),
            String::new(),
            String::new(),
        )
        .unwrap();
        let start = Instant::now();
        while core.link_devices().is_empty() {
            assert!(start.elapsed() < Duration::from_secs(10), "no device list");
            std::thread::sleep(Duration::from_millis(5));
        }
        core
    }

    fn sent(hub: &FakeHub) -> Vec<Value> {
        hub.json_requests()
            .into_iter()
            .filter(|r| {
                let t = r["t"].as_str().unwrap_or("");
                FAMILIES.iter().any(|p| t.starts_with(p))
            })
            .collect()
    }

    fn image() -> Value {
        json!({"name": "dspi.uf2", "size": 40000, "sha256": "ab", "firmware_version": "1.1.7",
               "platform": "RP2350", "stamp_mv": 1200, "verified": true})
    }

    fn rollout() -> Value {
        json!({"id": "ro1", "target": "A", "image": {"name": "dspi.uf2"}, "state": "running",
               "devices": [{"serial": "A", "state": "running", "step": "guard"}]})
    }

    /// Minimal answers of the real hub's shapes.
    fn answers() -> JsonResponder {
        Arc::new(|r: &Value| {
            let t = r["t"].as_str()?;
            let row = json!({"serial": "A", "ok": true, "value": -20.0});
            Some(match t {
                "group.list" => json!({"groups": [{"id": "g1", "name": "Bar", "members": ["A"]}]}),
                "group.set" => json!({"group": {"id": "g1", "name": "Bar", "members": ["A"]},
                                      "created": true}),
                "group.delete" => json!({}),
                "group.param.get" | "group.param.set" => {
                    json!({"path": "vol.master", "idx": [], "results": [row]})
                }
                "group.preset.load" => json!({"results": [{"serial": "A", "ok": true, "slot": 2}]}),
                "group.apply" | "group.apply.confirm" | "group.apply.cancel" => {
                    json!({"run": "a1", "results": [{"serial": "A", "ok": true}]})
                }
                "fw.image.list" => {
                    json!({"images": [image()], "max_images": 4, "max_bytes": 8_388_608})
                }
                "fw.image.put" => {
                    if r["last"] == true {
                        json!({"image": image(), "created": true})
                    } else {
                        json!({})
                    }
                }
                "fw.image.delete" => json!({}),
                "fw.rollout.start" | "fw.rollout.cancel" => json!({"rollout": rollout()}),
                "fw.rollout.status" => json!({"rollouts": [rollout()]}),
                _ => return None,
            })
        })
    }

    fn a() -> HubFleetTarget {
        HubFleetTarget {
            group: String::new(),
            serials: vec!["A".into()],
        }
    }

    /// Every call once; each answer's error, where it failed.
    fn every_call(core: &DspiCore) -> Vec<(&'static str, Option<DspiError>)> {
        fn e<T>(r: Result<T>) -> Option<DspiError> {
            r.err()
        }
        vec![
            ("group.list", e(core.hub_groups())),
            (
                "group.set",
                e(core.hub_group_set(HubGroup {
                    name: "Bar".into(),
                    members: vec!["A".into()],
                    ..HubGroup::default()
                })),
            ),
            ("group.delete", e(core.hub_group_delete("g1".into()))),
            (
                "group.param.get",
                e(core.hub_group_param_get(group_target(), "vol.master".into(), vec![])),
            ),
            (
                "group.param.set",
                e(core.hub_group_param_set(
                    a(),
                    "vol.master".into(),
                    vec![],
                    ParamValue::Float(-20.0),
                    vec![HubOffset {
                        serial: "A".into(),
                        db: -3.0,
                    }],
                )),
            ),
            (
                "group.preset.load",
                e(core.hub_group_preset_load(a(), Some(2), String::new(), vec![])),
            ),
            (
                "group.apply",
                e(core.hub_group_apply(
                    a(),
                    "s1".into(),
                    String::new(),
                    String::new(),
                    true,
                    60_000,
                )),
            ),
            (
                "group.apply.confirm",
                e(core.hub_group_apply_confirm("a1".into())),
            ),
            (
                "group.apply.cancel",
                e(core.hub_group_apply_cancel("a1".into())),
            ),
            ("fw.image.list", e(core.hub_firmware_images())),
            (
                "fw.image.put",
                e(core.hub_firmware_put("dspi.uf2".into(), vec![7u8; 40_000])),
            ),
            (
                "fw.image.delete",
                e(core.hub_firmware_delete("dspi.uf2".into())),
            ),
            (
                "fw.rollout.start",
                e(core.hub_rollout_start(a(), "dspi.uf2".into(), true)),
            ),
            ("fw.rollout.status", e(core.hub_rollouts(String::new()))),
            (
                "fw.rollout.cancel",
                e(core.hub_rollout_cancel("ro1".into())),
            ),
        ]
    }

    #[test]
    fn wp_p3cc_without_the_caps_every_call_is_unsupported_and_sends_nothing() {
        let hub = hub(false);
        hub.set_json_responder(Some(answers()));
        let core = bound(&hub);
        let caps = core.hub_caps();
        assert!(caps.on_hub, "{caps:?}");
        assert!(!caps.groups && !caps.group_params && !caps.group_apply && !caps.rollout);
        for (t, e) in every_call(&core) {
            assert_eq!(e, Some(DspiError::Unsupported), "{t}");
        }
        assert!(sent(&hub).is_empty(), "{:?}", sent(&hub));
        core.link_close();
    }

    #[test]
    fn wp_p3cc_with_the_caps_each_call_sends_its_message() {
        let hub = hub(true);
        hub.set_json_responder(Some(answers()));
        let core = bound(&hub);
        let caps = core.hub_caps();
        assert!(caps.groups && caps.group_params && caps.group_apply && caps.rollout);
        for (t, e) in every_call(&core) {
            assert_eq!(e, None, "{t}");
        }
        let sent = sent(&hub);
        let types: Vec<&str> = sent.iter().filter_map(|r| r["t"].as_str()).collect();
        for t in [
            "group.list",
            "group.set",
            "group.delete",
            "group.param.get",
            "group.param.set",
            "group.preset.load",
            "group.apply",
            "group.apply.confirm",
            "group.apply.cancel",
            "fw.image.delete",
            "fw.rollout.start",
            "fw.rollout.status",
            "fw.rollout.cancel",
        ] {
            assert!(types.contains(&t), "{t} not sent: {types:?}");
        }
        let find = |t: &str| sent.iter().find(|r| r["t"] == t).unwrap().clone();
        let set = find("group.param.set");
        assert_eq!(set["targets"], json!(["A"]));
        assert_eq!(set["offsets"], json!({"A": -3.0}));
        assert_eq!(find("group.param.get")["group"], "g1");
        assert_eq!(find("group.preset.load")["slot"], 2);
        let apply = find("group.apply");
        assert_eq!(
            (apply["snapshot"].clone(), apply["guard"].clone()),
            (json!("s1"), json!(true))
        );
        assert_eq!(apply["timeout_ms"], 60_000);
        let start = find("fw.rollout.start");
        assert_eq!(
            (start["serial"].clone(), start["while_live"].clone()),
            (json!("A"), json!(true))
        );
        // The upload: the store read first, then 36 KiB chunks, in order, the
        // last marked; the bytes come back whole.
        let puts: Vec<&Value> = sent.iter().filter(|r| r["t"] == "fw.image.put").collect();
        assert_eq!(puts.len(), 2);
        assert_eq!(
            (puts[0]["seq"].clone(), puts[0]["last"].clone()),
            (json!(0), json!(false))
        );
        assert_eq!(
            (puts[1]["seq"].clone(), puts[1]["last"].clone()),
            (json!(1), json!(true))
        );
        assert!(
            puts.iter()
                .all(|p| p["data"].as_str().unwrap().len() <= 49_152)
        );
        let mut back = Vec::new();
        for p in &puts {
            back.extend(unbase64(p["data"].as_str().unwrap()).unwrap());
        }
        assert_eq!(back, vec![7u8; 40_000]);
        core.link_close();
    }
}
