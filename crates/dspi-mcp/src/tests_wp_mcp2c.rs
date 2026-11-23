//! WP MCP2c: the audio link, Installation setup, hub, analysis, limits and
//! firmware tools, sans-IO. The hub-side tools run against a real
//! in-process hub with fake alink units (as dspi-hub's
//! `tests/wp_inst0_installation.rs`): a draft, validate and diff, then a
//! save that follows the user's setting and is bound to the draft asked
//! about; a conflict rebuilds the draft with the edits kept aside; a scene
//! recall is risky; the hub reads are read only and no admin tool is listed.
//! Also the review RB folds: an effect's new output and the upmixer's switch
//! are judged, and the relay renews its session only on the app's 404.

use std::path::PathBuf;
use std::sync::Arc;

use dspi_core::{CoreSet, CoreSetConfig, DeviceId, DeviceTarget, DspiCore, ParamValue};
use serde_json::{Value, json};

use crate::policy::{self, AgentPolicy, ConfirmMode, Judging, RiskKind, Verdict, WriteKey};
use crate::rate::RateLimit;
use crate::{ClientId, ReadOnlyPolicy, Server, ServerConfig};

const MODERN: &str = "2026-07-28";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::current_exe()
        .unwrap()
        .with_file_name("test-scratch")
        .join(format!("mcp2c-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn demo(name: &str) -> (Arc<CoreSet>, DeviceId, Arc<DspiCore>) {
    let set = CoreSet::new();
    set.set_config(CoreSetConfig {
        snapshot_dir: scratch(name).to_string_lossy().into_owned(),
        ..CoreSetConfig::default()
    })
    .unwrap();
    let id = set.open(DeviceTarget::Demo).unwrap();
    let core = set.core(&id).unwrap();
    (set, id, core)
}

fn request(tool: &str, args: Value) -> Value {
    let mut params = json!({"name": tool, "arguments": args});
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "test-agent", "version": "1"},
    });
    json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": params})
}

fn call(s: &mut Server, c: ClientId, tool: &str, args: Value) -> Value {
    let r = s.handle_value(c, 0, request(tool, args)).unwrap();
    r.get("result").cloned().unwrap_or_else(|| panic!("{r}"))
}

fn ok(s: &mut Server, c: ClientId, tool: &str, args: Value) -> Value {
    let r = call(s, c, tool, args);
    assert_eq!(r["isError"], json!(false), "{tool}: {r}");
    assert_ne!(
        r["structuredContent"]["status"],
        json!("awaiting_confirmation"),
        "{tool} was held: {r}"
    );
    r["structuredContent"].clone()
}

fn err(s: &mut Server, c: ClientId, tool: &str, args: Value) -> String {
    let r = call(s, c, tool, args);
    assert_eq!(r["isError"], json!(true), "{tool}: {r}");
    r["content"][0]["text"].as_str().unwrap().to_owned()
}

/// A held call's confirmation id.
fn held(s: &mut Server, c: ClientId, tool: &str, args: Value) -> u64 {
    let r = call(s, c, tool, args);
    assert_eq!(r["isError"], json!(false), "{tool}: {r}");
    let d = &r["structuredContent"];
    assert_eq!(d["status"], json!("awaiting_confirmation"), "{tool}: {r}");
    d["confirmation_id"].as_u64().unwrap()
}

fn risks(s: &Server, id: u64) -> Vec<String> {
    s.confirmation(id)
        .unwrap()
        .risks
        .iter()
        .map(|r| format!("{}: {}", r.kind.as_str(), r.what))
        .collect()
}

/// Answer `id`; what the re-run (or the refusal) gave the agent.
fn answer(s: &mut Server, c: ClientId, id: u64, allow: bool) -> Value {
    s.answer_confirmation(id, allow, 1_000).unwrap();
    ok(
        s,
        c,
        "dspi_get_confirmation",
        json!({"confirmation_id": id}),
    )
}

fn server(set: &Arc<CoreSet>, mode: ConfirmMode) -> Server {
    let config = ServerConfig {
        policy: Arc::new(AgentPolicy::new(mode)),
        read_limit: RateLimit {
            burst: 1000,
            per_s: 1.0,
        },
        write_limit: RateLimit {
            burst: 1000,
            per_s: 1.0,
        },
        ..ServerConfig::default()
    };
    Server::new(Arc::clone(set), config)
}

#[cfg(feature = "link")]
fn names(s: &mut Server) -> Vec<String> {
    let c = s.connect();
    let mut params = json!({});
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    let r = s
        .handle_value(
            c,
            0,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": params}),
        )
        .unwrap();
    r["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

// ------------------------------------------------------- the tool list

/// The new tools, in `tools/list` order.
#[cfg(feature = "link")]
const MCP2C_TOOLS: &[&str] = &[
    "dspi_get_alink",
    "dspi_get_system",
    "dspi_system_set_delays",
    "dspi_system_set_usb_mode",
    "dspi_system_align",
    "dspi_system_scene_apply",
    "dspi_system_mute",
    "dspi_installation_view",
    "dspi_installation_draft",
    "dspi_installation_validate",
    "dspi_installation_diff",
    "dspi_installation_save",
    "dspi_list_hubs",
    "dspi_get_hub",
    "dspi_list_hub_units",
    "dspi_get_hub_groups",
    "dspi_clear_clips",
    "dspi_get_rta",
    "dspi_get_stats",
    "dspi_get_monitor",
    "dspi_monitor_clear",
    "dspi_get_events",
    "dspi_get_limits",
    "dspi_get_firmware",
];

/// The agent-proof list (MPLAN 3.2): words no tool name may hold.
#[cfg(feature = "link")]
const AGENT_PROOF: &[&str] = &[
    "flash",
    "bootloader",
    "picoboot",
    "factory",
    "vreg",
    "voltage",
    "pin",
    "lock",
    "unlock",
    "limits_set",
    "limits_apply",
    "limits_clear",
    "set_limits",
    "raw",
    "ext_",
    "notify",
    "cs_",
    "siggen",
    "identify",
    "hardware",
    "output_config",
    "read_only",
    "pair",
    "security",
    "forget",
    "api_key",
    "schedule",
    "mqtt",
    "outbound",
    "rollout",
    "group_set",
    "group_apply",
    "group_delete",
    "group_param",
    "hub_set",
    "hub_rename",
    "delete",
    "autoeq_update",
    "rebuild",
    "speaker_layout",
    "open_device",
    "close_device",
    "path",
];

// -- WP FEAT2 --
/// The tools the owner asked for by name on 2026-11-23 although they hold
/// such a word: opening and closing known devices (requests to the app) and
/// the schedule, read only. Exactly these.
#[cfg(feature = "link")]
const OWNER_NAMED: &[&str] = &[
    "dspi_open_device",
    "dspi_close_device",
    "dspi_list_schedules",
];

#[test]
#[cfg(feature = "link")]
fn wp_mcp2c_the_list_has_the_new_tools_read_only_hub_reads_and_no_agent_proof_tool() {
    let (set, _, _) = demo("list");
    let mut s = server(&set, ConfirmMode::AskRisky);
    let all = names(&mut s);
    // The snapshot: the new tools, in order, after MCP2b's.
    let at = all.iter().position(|n| n == "dspi_get_alink").unwrap();
    assert_eq!(&all[at..at + MCP2C_TOOLS.len()], MCP2C_TOOLS);
    assert_eq!(all[at - 1], "dspi_set_source", "after MCP2b's last");
    // No name twice.
    let mut sorted = all.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), all.len());
    // No agent-proof tool anywhere in the list.
    for n in all.iter().filter(|n| !OWNER_NAMED.contains(&n.as_str())) {
        for bad in AGENT_PROOF {
            assert!(!n.contains(bad), "{n} looks agent-proof ({bad})");
        }
    }
    // The hub reads are read only, the hub writes are not.
    let c = s.connect();
    let mut params = json!({});
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    let r = s
        .handle_value(
            c,
            0,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": params}),
        )
        .unwrap();
    let tools = r["result"]["tools"].as_array().unwrap();
    let read_only = |name: &str| {
        tools.iter().find(|t| t["name"] == json!(name)).unwrap()["annotations"]["readOnlyHint"]
            .as_bool()
            .unwrap()
    };
    for n in [
        "dspi_list_hubs",
        "dspi_get_hub",
        "dspi_list_hub_units",
        "dspi_get_hub_groups",
        "dspi_get_system",
        "dspi_get_alink",
        "dspi_installation_view",
        "dspi_installation_draft",
        "dspi_installation_validate",
        "dspi_installation_diff",
        "dspi_get_rta",
        "dspi_get_stats",
        "dspi_get_events",
        "dspi_get_limits",
        "dspi_get_firmware",
    ] {
        assert!(read_only(n), "{n} should be read only");
    }
    for n in [
        "dspi_installation_save",
        "dspi_system_set_delays",
        "dspi_system_set_usb_mode",
        "dspi_system_align",
        "dspi_system_scene_apply",
        "dspi_system_mute",
        "dspi_clear_clips",
        "dspi_monitor_clear",
    ] {
        assert!(!read_only(n), "{n} writes");
    }
}

// -------------------------------------------------------- device reads

#[test]
fn wp_mcp2c_analysis_limits_firmware_and_alink_reads_on_a_demo() {
    let (set, _, core) = demo("reads");
    let mut s = server(&set, ConfirmMode::AskRisky);
    let c = s.connect();

    let stats = ok(&mut s, c, "dspi_get_stats", json!({}));
    assert!(stats.get("buffers").is_some(), "{stats}");
    let rta = ok(&mut s, c, "dspi_get_rta", json!({}));
    assert!(rta.get("supported").is_some(), "{rta}");
    let events = ok(&mut s, c, "dspi_get_events", json!({"limit": 5}));
    assert!(events["events"].is_array(), "{events}");
    let monitor = ok(&mut s, c, "dspi_get_monitor", json!({}));
    assert!(monitor["lines"].is_array(), "{monitor}");
    assert!(err(&mut s, c, "dspi_get_events", json!({"limit": 0})).contains("limit"));

    let limits = ok(&mut s, c, "dspi_get_limits", json!({}));
    assert_eq!(
        limits["confirmation_setting"],
        json!("ask_risky"),
        "{limits}"
    );
    assert_eq!(limits["limits"]["active"], json!(false), "{limits}");
    assert_eq!(limits["app_read_only"], json!(false));

    let fw = ok(&mut s, c, "dspi_get_firmware", json!({}));
    let st = core.get_state();
    assert_eq!(
        fw["firmware"],
        json!(format!("{}.{}.{}", st.fw_major, st.fw_minor, st.fw_patch)),
        "{fw}"
    );
    assert!(fw["bundled"].is_array() && fw["update_available"].is_boolean());

    // Not on a hub: the hub tools say so.
    assert!(err(&mut s, c, "dspi_get_hub", json!({})).contains("not on a DSPi Link hub"));
    assert_eq!(
        ok(&mut s, c, "dspi_list_hubs", json!({}))["hubs"],
        json!([])
    );

    // The demo has no audio link; a demo unit with one reports it.
    let none = ok(&mut s, c, "dspi_get_alink", json!({}));
    assert_eq!(none["alink"], json!(false), "{none}");
    let rx = set
        .open_with(DeviceId::demo(77), String::new(), |c| c.open_demo_alink())
        .unwrap();
    let a = ok(&mut s, c, "dspi_get_alink", json!({"device": rx.as_str()}));
    assert_eq!(a["alink"], json!(true), "{a}");
    assert_eq!(a["info"]["role"], json!("receiver"), "{a}");
    assert_eq!(a["slots"].as_array().map(Vec::len), Some(8), "{a}");
    assert!(a["status"].is_object() && a["phase"].is_object(), "{a}");
}

#[test]
fn wp_mcp2c_the_clears_follow_the_setting_and_the_read_only_policy_refuses() {
    let (set, _, _) = demo("clears");
    // Quiet: they pass under the default.
    let mut s = server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    ok(&mut s, c, "dspi_clear_clips", json!({}));
    ok(&mut s, c, "dspi_monitor_clear", json!({}));
    // Asked under "ask on everything".
    let mut s = server(&set, ConfirmMode::AskEverything);
    let c = s.connect();
    let id = held(&mut s, c, "dspi_clear_clips", json!({}));
    assert_eq!(risks(&s, id), ["change: Clear the clip indicators"]);
    assert_eq!(answer(&mut s, c, id, true)["status"], json!("allowed"));
    // The default server is read only.
    let mut s = Server::new(Arc::clone(&set), ServerConfig::default());
    let c = s.connect();
    assert!(err(&mut s, c, "dspi_clear_clips", json!({})).contains("does not let agents"));
    let _ = ReadOnlyPolicy;
}

// ------------------------------------------------ the RB folds (E2, E3)

fn judging(mode: ConfirmMode) -> Judging {
    Judging {
        mode: Some(mode),
        ..Judging::default()
    }
}

#[test]
fn wp_mcp2c_an_effect_newly_on_an_output_is_louder_and_the_upmixer_is_an_enable() {
    let (set, _, core) = demo("masks");
    let mut st = core.get_state();
    st.psybass_enabled = true;
    st.psybass_output_mask = 0b011;
    let w = |path: &str, v: ParamValue| WriteKey::new(path, &[], v);

    // Outputs {0, 1} → {0, 1, 3}: louder on output 3 only.
    let v = policy::judge(
        &st,
        &[w("bass.mask", ParamValue::Mask(0b1011))],
        &judging(ConfirmMode::AskRisky),
    );
    let Verdict::Ask(r) = v else { panic!("{v:?}") };
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!((r[0].kind, r[0].output), (RiskKind::Louder, Some(3)));
    assert!(r[0].what.contains("Out 4"), "{}", r[0].what);
    // Taking an output away, or the mask of an effect that is off, passes.
    assert_eq!(
        policy::judge(
            &st,
            &[w("bass.mask", ParamValue::Mask(0b001))],
            &judging(ConfirmMode::AskRisky)
        ),
        Verdict::Pass
    );
    let mut off = st.clone();
    off.psybass_enabled = false;
    assert_eq!(
        policy::judge(
            &off,
            &[w("bass.mask", ParamValue::Mask(0b1111))],
            &judging(ConfirmMode::AskRisky)
        ),
        Verdict::Pass
    );
    // Never ask: passes.
    assert_eq!(
        policy::judge(
            &st,
            &[w("bass.mask", ParamValue::Mask(0b1011))],
            &judging(ConfirmMode::NeverAsk)
        ),
        Verdict::Pass
    );
    // The same for loudness, the subharmonic synth (its mask is a parameter).
    let mut l = st.clone();
    l.loudness_enabled = true;
    l.loudness_output_mask = 0b1;
    assert!(matches!(
        policy::judge(&l, &[w("loud.mask", ParamValue::Mask(0b11))], &judging(ConfirmMode::AskRisky)),
        Verdict::Ask(ref r) if r[0].output == Some(1)
    ));
    let mut sub = st.clone();
    if sub.subharm.len() < 2 {
        sub.subharm.resize(2, 0.0);
    }
    sub.subharm[0] = 1.0;
    sub.subharm[1] = 1.0;
    assert!(matches!(
        policy::judge(&sub, &[w("sub.mask", ParamValue::Float(5.0))], &judging(ConfirmMode::AskRisky)),
        Verdict::Ask(ref r) if r[0].output == Some(2)
    ));

    // The upmixer turned on: asked (an enable); turned off: passes.
    let mut up = st.clone();
    if up.upmix.is_empty() {
        up.upmix.push(0.0);
    }
    up.upmix[0] = 0.0;
    let v = policy::judge(
        &up,
        &[w("up.on", ParamValue::Bool(true))],
        &judging(ConfirmMode::AskRisky),
    );
    assert!(
        matches!(&v, Verdict::Ask(r) if r.len() == 1 && r[0].kind == RiskKind::Enable),
        "{v:?}"
    );
    up.upmix[0] = 1.0;
    assert_eq!(
        policy::judge(
            &up,
            &[w("up.on", ParamValue::Bool(false))],
            &judging(ConfirmMode::AskRisky)
        ),
        Verdict::Pass
    );

    // Through the tool on the device: psybass on for {0, 1} passes once
    // allowed, then adding output 3 is held, as review RB E2 asked.
    let mut s = server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let id = held(
        &mut s,
        c,
        "dspi_set_psybass",
        json!({"enabled": true, "outputs": [0, 1]}),
    );
    assert_eq!(answer(&mut s, c, id, true)["status"], json!("allowed"));
    assert!(core.get_state().psybass_enabled);
    let id = held(&mut s, c, "dspi_set_psybass", json!({"outputs": [0, 1, 3]}));
    assert_eq!(
        risks(&s, id),
        ["louder: Psychoacoustic bass now also on Out 4 (it adds level)"]
    );
    answer(&mut s, c, id, false);
    assert_eq!(core.get_state().psybass_output_mask, 0b011);
    ok(&mut s, c, "dspi_set_psybass", json!({"outputs": [0]}));
    assert_eq!(core.get_state().psybass_output_mask, 0b001);
}

// ------------------------------------------------------------- the relay

#[test]
#[cfg(feature = "http")]
fn wp_mcp2c_the_relay_renews_its_session_only_on_the_apps_404() {
    use crate::relay::tests_wp_mcp2c_session_gone as gone;
    // The app's "no such session" (http.rs `Response::error`).
    assert!(gone(
        br#"{"jsonrpc":"2.0","id":null,"error":{"code":-32000,"message":"no such session"}}"#
    ));
    // An unknown method's 404 is the agent's answer, not a lost session.
    assert!(!gone(
        br#"{"jsonrpc":"2.0","id":3,"error":{"code":-32601,"message":"no such method"}}"#
    ));
    assert!(!gone(b"not json"));
}

#[test]
#[cfg(feature = "http")]
fn wp_mcp2c_a_revoked_client_never_becomes_a_principal_again() {
    // A request authorised just before a revoke and served after it is
    // refused, not given a new principal (review RB, http.rs).
    assert!(!crate::http::tests_wp_mcp2c_revoked_becomes_a_principal());
}

// ------------------------------------------------------- against a hub

#[cfg(feature = "link")]
mod on_a_hub {
    use std::net::{IpAddr, Ipv4Addr};

    use dspi_core::HubTarget;
    use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, ServiceConfig};
    use dspi_proto::ext::alink::AlinkRole;
    use dspi_transport::FakeDevice;
    use dspi_transport::fake::AlinkSetup;

    use super::*;

    fn site() -> Value {
        json!({
            "rooms": [{"id": "main", "name": "Main room"}],
            "senders": [{"id": "tx", "name": "Sender", "serial": "M2C-TX"}],
            "fanOuts": [{"id": "fan", "name": "Fan-out", "serial": "M2C-FAN", "upstream": "tx", "output": 1}],
            "units": [
                {"id": "A", "name": "Main left", "serial": "M2C-A", "room": "main", "upstream": "tx", "output": 0}
            ],
            "scenes": [
                {"id": "closed", "name": "Closed", "systemMute": true, "actions": [{"target": "all", "op": "mute"}]},
                {"id": "open", "name": "Open \u{202e}gninrom", "systemMute": false, "actions": [{"target": "all", "op": "unmute"}]}
            ]
        })
    }

    /// An open hub (every session admin) with the inventory service, five
    /// alink units attached, the site holding the sender, the fan-out and A.
    fn hub(name: &str) -> Hub {
        let hub = Hub::start(HubConfig {
            network: Some(NetworkConfig {
                port: 0,
                access: AccessMode::Open,
                bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
                ..NetworkConfig::new(scratch(&format!("hub-{name}")))
            }),
            initial_scan: false,
            services: ServiceConfig {
                inventory: true,
                ..ServiceConfig::default()
            },
            ..HubConfig::in_process("MCP2c hub")
        })
        .unwrap();
        let at = |role, hops, hop_delay_frames| AlinkSetup {
            role,
            hops,
            hop_delay_frames,
            ..AlinkSetup::default()
        };
        for (serial, setup) in [
            ("M2C-TX", at(AlinkRole::Sender, 0, 0)),
            ("M2C-FAN", at(AlinkRole::FanOut, 0, 3)),
            ("M2C-A", at(AlinkRole::Receiver, 0, 0)),
            ("M2C-B", at(AlinkRole::Receiver, 1, 0)),
            ("M2C-C", at(AlinkRole::Receiver, 2, 0)),
        ] {
            hub.attach(
                Box::new(FakeDevice::demo_alink_as(setup).with_serial(serial)),
                0,
            )
            .unwrap();
        }
        hub.handle()
            .system_set_model(serde_json::from_value(site()).unwrap())
            .unwrap();
        hub
    }

    /// Unit A of `hub` open in a set, as the app opens a hub's unit.
    fn open_a(hub: &Hub) -> (Arc<CoreSet>, DeviceId) {
        let port = hub.network().unwrap().port();
        let set = CoreSet::new();
        let id = set
            .open(DeviceTarget::Hub(HubTarget {
                url: format!("ws://127.0.0.1:{port}/dspi/v1"),
                hub_id: String::new(),
                serial: "M2C-A".into(),
                client_name: "wp-mcp2c".into(),
                app_version: "0".into(),
                credential: String::new(),
                tls_advertised: false,
            }))
            .unwrap();
        (set, id)
    }

    fn edits() -> Value {
        json!([
            {"op": "addRoom", "id": "bar", "name": "Bar"},
            {"op": "addUnit", "serial": "M2C-B", "name": "Bar left", "room": "bar",
             "role": "receiver", "upstream": "fan"}
        ])
    }

    #[test]
    fn wp_mcp2c_installation_draft_validate_diff_then_save_follows_the_setting() {
        let hub = hub("inst");
        let (set, _) = open_a(&hub);
        let (_, revision) = hub.handle().system_model();
        let mut s = server(&set, ConfirmMode::AskRisky);
        let c = s.connect();

        let v = ok(&mut s, c, "dspi_installation_view", json!({}));
        assert_eq!(
            (v["may_edit"].clone(), v["inventory"].clone()),
            (json!(true), json!(true)),
            "{v}"
        );
        assert_eq!(v["revision"], json!(revision));
        assert_eq!(v["draft_open"], json!(false));
        let mut unseen: Vec<&str> = v["unseen"]
            .as_array()
            .unwrap()
            .iter()
            .map(|u| u["serial"].as_str().unwrap())
            .collect();
        unseen.sort_unstable();
        assert_eq!(unseen, ["M2C-B", "M2C-C"], "{v}");
        // Names only in *_name fields.
        assert!(v["site"]["rooms"][0].get("name").is_none(), "{v}");
        assert_eq!(v["site"]["rooms"][0]["display_name"], json!("Main room"));

        assert!(err(&mut s, c, "dspi_installation_validate", json!({})).contains("no draft"));

        // A bad edit changes nothing; it is named.
        let e = err(
            &mut s,
            c,
            "dspi_installation_draft",
            json!({"edits": [{"op": "addUnit", "serial": "M2C-A", "name": "Again", "role": "receiver", "upstream": "tx"}]}),
        );
        assert!(
            e.contains("edit 1") && e.contains("nothing was changed"),
            "{e}"
        );
        let e = err(
            &mut s,
            c,
            "dspi_installation_draft",
            json!({"edits": [{"op": "flashEverything"}]}),
        );
        assert!(e.contains("edit 1"), "{e}");

        let d = ok(
            &mut s,
            c,
            "dspi_installation_draft",
            json!({"edits": edits()}),
        );
        assert_eq!(d["edits"], json!(2), "{d}");
        assert_eq!(d["valid"], json!(true), "{d}");
        assert_eq!(
            d["changes"],
            json!(["Adds room Bar", "Adds unit Bar left"]),
            "{d}"
        );
        let val = ok(&mut s, c, "dspi_installation_validate", json!({}));
        assert_eq!(val["valid"], json!(true));
        let diff = ok(&mut s, c, "dspi_installation_diff", json!({}));
        assert_eq!(diff["changes"], d["changes"]);
        assert_eq!(hub.handle().system_model().1, revision, "nothing sent yet");

        // Save under the default: held, nothing sent; Deny keeps it so.
        let id = held(&mut s, c, "dspi_installation_save", json!({}));
        assert_eq!(
            risks(&s, id),
            [
                "installation_save: Save the installation on the hub (2 change(s); every unit follows it)"
            ]
        );
        assert_eq!(answer(&mut s, c, id, false)["status"], json!("denied"));
        assert_eq!(hub.handle().system_model().1, revision);

        // The grant is bound to the draft asked about: edited after the
        // question, Allow saves nothing.
        let id = held(&mut s, c, "dspi_installation_save", json!({}));
        ok(
            &mut s,
            c,
            "dspi_installation_draft",
            json!({"edits": [{"op": "renameElement", "id": "A", "name": "Front left"}]}),
        );
        let w = answer(&mut s, c, id, true);
        assert_eq!(w["result"]["isError"], json!(true), "{w}");
        assert_eq!(
            hub.handle().system_model().1,
            revision,
            "the edited draft was not saved"
        );

        // Asked again and allowed: one system.set at the revision.
        let id = held(&mut s, c, "dspi_installation_save", json!({}));
        assert!(
            risks(&s, id)[0].contains("3 change(s)"),
            "{:?}",
            risks(&s, id)
        );
        let w = answer(&mut s, c, id, true);
        assert_eq!(w["result"]["isError"], json!(false), "{w}");
        let (model, now) = hub.handle().system_model();
        assert_eq!(now, revision + 1);
        let b = model.unit("Bar left").unwrap();
        assert_eq!(
            (b.serial.as_str(), b.room.as_deref()),
            ("M2C-B", Some("bar"))
        );
        assert_eq!(model.unit("A").unwrap().name, "Front left");
        // Saved: the draft is gone.
        assert!(err(&mut s, c, "dspi_installation_diff", json!({})).contains("no draft"));

        // Never ask: nothing is asked (owner 2026-11-22).
        let mut s = server(&set, ConfirmMode::NeverAsk);
        let c = s.connect();
        ok(
            &mut s,
            c,
            "dspi_installation_draft",
            json!({"edits": [{"op": "addRoom", "name": "Office"}]}),
        );
        let saved = ok(&mut s, c, "dspi_installation_save", json!({}));
        assert_eq!(saved["revision"], json!(revision + 2), "{saved}");

        // The read-only policy refuses the save; reading and drafting it may.
        let mut s = Server::new(Arc::clone(&set), ServerConfig::default());
        let c = s.connect();
        ok(
            &mut s,
            c,
            "dspi_installation_draft",
            json!({"edits": [{"op": "addRoom", "name": "Hall"}]}),
        );
        assert!(
            err(&mut s, c, "dspi_installation_save", json!({})).contains("does not let agents")
        );
        assert_eq!(hub.handle().system_model().1, revision + 2);
    }

    #[test]
    fn wp_mcp2c_a_conflict_rebuilds_the_draft_and_keeps_the_edits_aside() {
        let hub = hub("conflict");
        let (set, _) = open_a(&hub);
        let mut s = server(&set, ConfirmMode::NeverAsk);
        let c = s.connect();
        ok(
            &mut s,
            c,
            "dspi_installation_draft",
            json!({"edits": edits()}),
        );

        // Someone else saves meanwhile: a room of their own.
        let (mut model, revision) = hub.handle().system_model();
        let mut theirs: Value = serde_json::to_value(&model).unwrap();
        theirs["rooms"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": "porch", "name": "Porch"}));
        model = serde_json::from_value(theirs).unwrap();
        hub.handle().system_set_model(model).unwrap();

        let e = err(&mut s, c, "dspi_installation_save", json!({}));
        assert!(
            e.contains("nothing was saved") && e.contains("2 edit(s)"),
            "{e}"
        );
        assert_eq!(hub.handle().system_model().1, revision + 1, "only theirs");
        // The fresh draft holds their change and none of ours.
        let d = ok(&mut s, c, "dspi_installation_draft", json!({}));
        assert_eq!(
            (d["edits"].clone(), d["edits_kept_aside"].clone()),
            (json!(0), json!(2)),
            "{d}"
        );
        assert_eq!(d["changes"], json!([]), "{d}");
        // Remade: judged again on the fresh draft, then saved; theirs stays.
        let d = ok(
            &mut s,
            c,
            "dspi_installation_draft",
            json!({"remake": true}),
        );
        assert_eq!(
            (d["edits"].clone(), d["edits_kept_aside"].clone()),
            (json!(2), json!(0)),
            "{d}"
        );
        ok(&mut s, c, "dspi_installation_save", json!({}));
        let (model, now) = hub.handle().system_model();
        assert_eq!(now, revision + 2);
        assert!(model.unit("Bar left").is_some());
        assert!(
            model.rooms.iter().any(|r| r.id == "porch"),
            "the other save stays"
        );
    }

    #[test]
    fn wp_mcp2c_scene_recall_is_risky_the_mute_and_the_system_reads() {
        let hub = hub("scene");
        let (set, id) = open_a(&hub);
        let mut s = server(&set, ConfirmMode::AskRisky);
        let c = s.connect();

        let sys = ok(&mut s, c, "dspi_get_system", json!({}));
        assert_eq!(sys["may_recall"], json!(true), "{sys}");
        let scenes = sys["scenes"].as_array().unwrap();
        assert_eq!(scenes.len(), 2, "{sys}");
        // A scene's name is a cleaned label: the bidi override is gone.
        assert_eq!(scenes[1]["display_name"], json!("Open gninrom"), "{sys}");
        assert!(
            sys["units"]
                .as_array()
                .unwrap()
                .iter()
                .all(|u| u.get("name").is_none())
        );

        // A recall is risky: held, the sheet names no scene by its name.
        let r = held(
            &mut s,
            c,
            "dspi_system_scene_apply",
            json!({"scene": "closed"}),
        );
        assert_eq!(
            risks(&s, r),
            ["system: Recall scene 1 of 2 on every unit it names"]
        );
        let w = answer(&mut s, c, r, true);
        assert_eq!(w["result"]["isError"], json!(false), "{w}");
        assert!(
            w["result"]["structuredContent"].get("taken").is_some(),
            "{w}"
        );
        let r = held(
            &mut s,
            c,
            "dspi_system_scene_apply",
            json!({"scene": "open"}),
        );
        assert_eq!(
            risks(&s, r),
            ["system: Recall scene 2 of 2 on every unit it names (it clears the system mute)"]
        );
        answer(&mut s, c, r, false);
        assert!(
            err(
                &mut s,
                c,
                "dspi_system_scene_apply",
                json!({"scene": "nope"})
            )
            .contains("no scene")
        );

        // Align is risky too; the delays are a quiet change.
        let r = held(&mut s, c, "dspi_system_align", json!({}));
        assert!(risks(&s, r)[0].starts_with("system: Write the audio link delays"));
        answer(&mut s, c, r, false);
        let rev = sys["revision"].as_u64().unwrap();
        let saved = ok(
            &mut s,
            c,
            "dspi_system_set_delays",
            json!({"revision": rev, "rooms": [{"room_id": "main", "lip_sync_us": 250}]}),
        );
        assert_eq!(saved["revision"], json!(rev + 1), "{saved}");
        let e = err(
            &mut s,
            c,
            "dspi_system_set_delays",
            json!({"revision": rev, "rooms": [{"room_id": "main", "lip_sync_us": 0}]}),
        );
        assert!(e.contains("changed on the hub meanwhile"), "{e}");
        let r = held(
            &mut s,
            c,
            "dspi_system_set_usb_mode",
            json!({"revision": rev + 1, "keep_usb_aligned": true}),
        );
        answer(&mut s, c, r, false);

        // The system mute: setting it is quiet, clearing it is an unmute.
        let m = ok(&mut s, c, "dspi_system_mute", json!({"muted": true}));
        assert_eq!(m["muted"], json!(true), "{m}");
        let r = held(&mut s, c, "dspi_system_mute", json!({"muted": false}));
        assert_eq!(
            risks(&s, r),
            ["unmute: Clear the system mute (every unit plays again)"]
        );
        answer(&mut s, c, r, true);

        // Never ask: a recall goes unasked.
        let mut s2 = server(&set, ConfirmMode::NeverAsk);
        let c2 = s2.connect();
        ok(
            &mut s2,
            c2,
            "dspi_system_scene_apply",
            json!({"scene": "closed"}),
        );

        // The hub reads.
        let hubs = ok(&mut s, c, "dspi_list_hubs", json!({}));
        let h = &hubs["hubs"][0];
        assert_eq!(h["devices"][0]["device"], json!(id.as_str()), "{hubs}");
        assert_eq!(h["role"], json!("admin"), "{hubs}");
        // A hub on this computer: nothing crosses the network.
        assert_eq!(h["encryption"], json!("local"), "{hubs}");
        let units = ok(&mut s, c, "dspi_list_hub_units", json!({}));
        assert!(
            units["units"]
                .as_array()
                .unwrap()
                .iter()
                .any(|u| u["serial"] == json!("M2C-C")),
            "{units}"
        );
        let got = ok(&mut s, c, "dspi_get_hub", json!({}));
        assert_eq!(got["role"], json!("admin"), "{got}");
        assert!(got["families"]["inventory"].as_bool().unwrap(), "{got}");
        // This hub has no groups (the parameter API's family): it says so.
        let e = err(&mut s, c, "dspi_get_hub_groups", json!({}));
        assert!(e.contains("does not offer"), "{e}");
    }
}
