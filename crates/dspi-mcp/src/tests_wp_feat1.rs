//! WP FEAT1: the typed installation draft tools for routing and sources
//! (`dspi_installation_set_slots`, `_set_source`, `_set_zone_source`,
//! `_set_scene`, `_set_programme`) against a real in-process hub with fake
//! audio link units: each under the three confirmation settings (a draft edit
//! is never asked; the save follows MCP2c's rule and is bound to the draft),
//! a slot clash, a draft that changed after the save was asked about or on
//! the hub meanwhile, and what is live: a saved zone source is stored only
//! until the units' slots are written, a recalled source scene is live at
//! once.

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;

use dspi_core::{CoreSet, DeviceId, DeviceTarget, HubTarget};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, ServiceConfig};
use dspi_proto::ext::alink::AlinkRole;
use dspi_transport::FakeDevice;
use dspi_transport::fake::AlinkSetup;
use serde_json::{Value, json};

use crate::policy::{AgentPolicy, ConfirmMode};
use crate::rate::RateLimit;
use crate::{ClientId, Server, ServerConfig};

const MODERN: &str = "2026-07-28";

/// The tools, in `tools/list` order.
const FEAT1_TOOLS: &[&str] = &[
    "dspi_installation_set_slots",
    "dspi_installation_set_source",
    "dspi_installation_set_zone_source",
    "dspi_installation_set_scene",
    "dspi_installation_set_programme",
];

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::current_exe()
        .unwrap()
        .with_file_name("test-scratch")
        .join(format!("feat1-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn request(method: &str, params: Value) -> Value {
    let mut params = params;
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "test-agent", "version": "1"},
    });
    json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params})
}

fn call(s: &mut Server, c: ClientId, tool: &str, args: Value) -> Value {
    let r = s
        .handle_value(
            c,
            0,
            request("tools/call", json!({"name": tool, "arguments": args})),
        )
        .unwrap();
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
    let limit = RateLimit {
        burst: 1000,
        per_s: 1.0,
    };
    Server::new(
        Arc::clone(set),
        ServerConfig {
            policy: Arc::new(AgentPolicy::new(mode)),
            read_limit: limit,
            write_limit: limit,
            ..ServerConfig::default()
        },
    )
}

/// The site: rooms main and bar, zone `down` of main, source `radio` on the
/// sender's channels 2 and 3, A in main behind the sender, B in the bar
/// behind the fan-out.
fn site() -> Value {
    json!({
        "rooms": [{"id": "main", "name": "Main room"}, {"id": "bar", "name": "Bar"}],
        "zones": [{"id": "down", "name": "Downstairs", "rooms": ["main"]}],
        "senders": [{"id": "tx", "name": "Sender", "serial": "F1-TX"}],
        "sources": [{"id": "radio", "name": "Radio", "sender": "tx", "channels": [2, 3]}],
        "fanOuts": [{"id": "fan", "name": "Fan-out", "serial": "F1-FAN", "upstream": "tx", "output": 1}],
        "units": [
            {"id": "A", "name": "Main left", "serial": "F1-A", "room": "main", "upstream": "tx", "output": 0},
            {"id": "B", "name": "Bar", "serial": "F1-B", "room": "bar", "upstream": "fan"}
        ]
    })
}

/// An open hub (every session admin) with the four units attached.
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
        ..HubConfig::in_process("FEAT1 hub")
    })
    .unwrap();
    let at = |role, hops, hop_delay_frames| AlinkSetup {
        role,
        hops,
        hop_delay_frames,
        ..AlinkSetup::default()
    };
    for (serial, setup) in [
        ("F1-TX", at(AlinkRole::Sender, 0, 0)),
        ("F1-FAN", at(AlinkRole::FanOut, 0, 3)),
        ("F1-A", at(AlinkRole::Receiver, 0, 0)),
        ("F1-B", at(AlinkRole::Receiver, 1, 0)),
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
            serial: "F1-A".into(),
            client_name: "wp-feat1".into(),
            app_version: "0".into(),
            credential: String::new(),
            tls_advertised: false,
        }))
        .unwrap();
    (set, id)
}

/// Unit A's slots as it plays them now.
fn live_slots(s: &mut Server, c: ClientId) -> Value {
    ok(s, c, "dspi_get_alink", json!({}))["slots"].clone()
}

const RADIO: [u8; 8] = [2, 3, 255, 255, 255, 255, 255, 255];

#[test]
fn wp_feat1_the_tools_are_listed_closed_read_only_and_bounded_as_the_core() {
    let hub = hub("list");
    let (set, _) = open_a(&hub);
    let mut s = server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let r = s
        .handle_value(c, 0, request("tools/list", json!({})))
        .unwrap();
    let tools = r["result"]["tools"].as_array().unwrap().clone();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    // After the measuring tools, so MCP2c's snapshot stays.
    let at = names.iter().position(|n| *n == FEAT1_TOOLS[0]).unwrap();
    assert_eq!(&names[at..at + FEAT1_TOOLS.len()], FEAT1_TOOLS);
    assert_eq!(names[at - 1], "dspi_get_measure_options"); // -- WP INTF -- FEAT2's
    let tool = |n: &str| {
        tools
            .iter()
            .find(|t| t["name"] == json!(n))
            .unwrap()
            .clone()
    };
    for n in FEAT1_TOOLS {
        let t = tool(n);
        // A draft edit sends nothing: read only.
        assert_eq!(t["annotations"]["readOnlyHint"], json!(true), "{n}");
        assert_eq!(
            t["inputSchema"]["additionalProperties"],
            json!(false),
            "{n}"
        );
    }
    // The bounds are the core's.
    let slots = &tool("dspi_installation_set_slots")["inputSchema"]["properties"]["slots"];
    assert_eq!(
        (slots["minItems"].clone(), slots["maxItems"].clone()),
        (json!(8), json!(8))
    );
    assert_eq!(slots["items"]["oneOf"][0]["maximum"], json!(63));
    assert_eq!(slots["items"]["oneOf"][1]["const"], json!(255));
    let ch = &tool("dspi_installation_set_source")["inputSchema"]["properties"]["channels"];
    assert_eq!(
        (ch["maxItems"].clone(), ch["items"]["maximum"].clone()),
        (json!(64), json!(63))
    );
    let scene = tool("dspi_installation_set_scene");
    let actions = &scene["inputSchema"]["properties"]["actions"];
    assert_eq!(actions["maxItems"], json!(32));
    let item = &actions["items"];
    assert_eq!(item["additionalProperties"], json!(false));
    assert_eq!(
        (
            item["properties"]["value"]["minimum"].clone(),
            item["properties"]["value"]["maximum"].clone(),
            item["properties"]["arg"]["maximum"].clone()
        ),
        (json!(-12800), json!(2047), json!(10)) // preset 10, as the app shows it
    );
    assert!(
        item["properties"]["op"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("source"))
    );
    let kinds = &tool("dspi_installation_set_programme")["inputSchema"]["properties"]["kind"];
    assert_eq!(
        kinds["enum"],
        json!(["computer_usb", "spdif", "analogue", "alink_sender"])
    );
    // A closed schema refuses what it does not name.
    assert!(
        err(
            &mut s,
            c,
            "dspi_installation_set_zone_source",
            json!({"zone_id": "down", "source_id": "radio", "now": true})
        )
        .contains("no argument `now`")
    );
    let e = err(
        &mut s,
        c,
        "dspi_installation_set_scene",
        json!({"actions": [{"target": "all", "op": "mute", "live": true}]}),
    );
    assert!(e.contains("does not have"), "{e}");
    hub.stop();
}

/// Every tool under each setting: a draft edit is never asked and sends
/// nothing; the save is asked under "ask on everything" and "risky", not
/// under "never ask"; the read-only policy refuses the save only.
#[test]
fn wp_feat1_each_tool_under_the_three_settings_and_the_save_follows_them() {
    for mode in [
        ConfirmMode::AskEverything,
        ConfirmMode::AskRisky,
        ConfirmMode::NeverAsk,
    ] {
        let hub = hub(&format!("modes-{mode:?}"));
        let (set, _) = open_a(&hub);
        let (_, rev0) = hub.handle().system_model();
        let mut s = server(&set, mode);
        let c = s.connect();

        let p = ok(
            &mut s,
            c,
            "dspi_installation_set_programme",
            json!({"name": "Plex on the HTPC", "kind": "alink_sender", "feeds_id": "tx", "inputs": [0, 1]}),
        );
        assert_eq!(p["element_id"], json!("Plex on the HTPC"), "{p}");
        assert!(p["live"].as_str().unwrap().contains("nothing was sent"));
        let src = ok(
            &mut s,
            c,
            "dspi_installation_set_source",
            json!({"source_id": "tv", "name": "TV", "sender_id": "tx", "channels": [4, 5]}),
        );
        assert_eq!(src["element_id"], json!("tv"));
        ok(
            &mut s,
            c,
            "dspi_installation_set_slots",
            json!({"unit_id": "B", "slots": [6, 7, 255, 255, 255, 255, 255, 255]}),
        );
        let z = ok(
            &mut s,
            c,
            "dspi_installation_set_zone_source",
            json!({"zone_id": "down", "source_id": "radio"}),
        );
        // The draft routes A; nothing is live yet.
        assert_eq!(z["routing"][0]["unit_id"], json!("A"), "{z}");
        assert_eq!(z["routing"][0]["slots_draft"], json!(RADIO), "{z}");
        assert_eq!(z["routing"][0]["zone_source_id"], json!("radio"), "{z}");
        assert_eq!(z["routing"][1]["unit_id"], json!("B"), "{z}");
        let sc = ok(
            &mut s,
            c,
            "dspi_installation_set_scene",
            json!({"scene_id": "tv-night", "name": "TV night", "actions": [
                {"target": "zone:down", "op": "source", "source_id": "tv"},
                {"target": "unit:A", "op": "master_volume", "value": -2500}
            ]}),
        );
        assert_eq!(sc["valid"], json!(true), "{sc}");
        assert_eq!(sc["edits"], json!(5), "{sc}");
        assert_eq!(
            hub.handle().system_model().1,
            rev0,
            "{mode:?}: nothing sent"
        );

        let saved = if mode == ConfirmMode::NeverAsk {
            ok(&mut s, c, "dspi_installation_save", json!({}))
        } else {
            let id = held(&mut s, c, "dspi_installation_save", json!({}));
            assert_eq!(
                risks(&s, id),
                [
                    "installation_save: Save the installation on the hub (6 change(s); every unit \
                  follows it; 2 unit(s) take other audio link channels at the next alignment)"
                ],
                "{mode:?}"
            );
            let w = answer(&mut s, c, id, true);
            assert_eq!(w["result"]["isError"], json!(false), "{w}");
            w["result"]["structuredContent"].clone()
        };
        assert_eq!(saved["revision"], json!(rev0 + 1), "{mode:?}: {saved}");
        assert!(
            saved["live"]
                .as_str()
                .unwrap()
                .contains("still play their old"),
            "{saved}"
        );
        let (m, _) = hub.handle().system_model();
        assert_eq!(m.zones[0].source.as_deref(), Some("radio"));
        assert_eq!(m.unit("A").unwrap().slots.as_deref(), Some(&RADIO[..]));
        assert_eq!(m.programme_sources[0].feeds, "tx");
        assert_eq!(
            m.scene("tv-night").unwrap().actions[0].source.as_deref(),
            Some("tv")
        );
        // Stored, not live: unit A still plays its own table.
        assert_ne!(live_slots(&mut s, c), json!(RADIO), "{mode:?}");

        // The read-only policy: drafting yes, saving no.
        let mut ro = Server::new(Arc::clone(&set), ServerConfig::default());
        let rc = ro.connect();
        ok(
            &mut ro,
            rc,
            "dspi_installation_set_zone_source",
            json!({"zone_id": "down", "source_id": "tv"}),
        );
        assert!(
            err(&mut ro, rc, "dspi_installation_save", json!({})).contains("does not let agents")
        );
        assert_eq!(hub.handle().system_model().1, rev0 + 1);
        hub.stop();
    }
}

#[test]
fn wp_feat1_a_slot_clash_is_refused_and_changes_nothing() {
    let hub = hub("clash");
    let (set, _) = open_a(&hub);
    let mut s = server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let z = ok(
        &mut s,
        c,
        "dspi_installation_set_zone_source",
        json!({"zone_id": "down", "source_id": "radio"}),
    );
    assert_eq!(z["edits"], json!(1));
    let e = err(
        &mut s,
        c,
        "dspi_installation_set_slots",
        json!({"unit_id": "A", "slots": [0, 1, 255, 255, 255, 255, 255, 255]}),
    );
    assert!(
        e.contains("slot clash") && e.contains("nothing was changed"),
        "{e}"
    );
    // A channel clash on the sender, and a source a unit cannot take.
    let e = err(
        &mut s,
        c,
        "dspi_installation_set_source",
        json!({"name": "TV", "sender_id": "tx", "channels": [3, 4]}),
    );
    assert!(e.contains("channel clash"), "{e}");
    let e = err(
        &mut s,
        c,
        "dspi_installation_set_source",
        json!({"source_id": "radio", "channels": [0, 1, 2, 3, 4, 5, 6, 7, 8]}),
    );
    assert!(e.contains("a unit takes 8"), "{e}");
    // Values outside the schema are named; nothing reaches the draft.
    let e = err(
        &mut s,
        c,
        "dspi_installation_set_slots",
        json!({"unit_id": "B", "slots": [64, 0, 0, 0, 0, 0, 0, 0]}),
    );
    assert!(e.contains("each a channel below 64 or 255"), "{e}");
    let d = ok(&mut s, c, "dspi_installation_draft", json!({}));
    assert_eq!(d["edits"], json!(1), "{d}");
    assert_eq!(d["changes"].as_array().unwrap().len(), 2, "{d}");
    // Changing the source's channels routes the zone again.
    let d = ok(
        &mut s,
        c,
        "dspi_installation_set_source",
        json!({"source_id": "radio", "channels": [10, 11]}),
    );
    assert_eq!(
        d["routing"][0]["slots_draft"],
        json!([10, 11, 255, 255, 255, 255, 255, 255]),
        "{d}"
    );
    hub.stop();
}

/// The grant is bound to the draft asked about: a typed edit after the
/// question saves nothing. Someone saving on the hub meanwhile rebuilds the
/// draft with the typed edits kept aside, remade and judged again.
#[test]
fn wp_feat1_a_draft_that_changed_since_it_was_read_is_not_saved() {
    let hub = hub("changed");
    let (set, _) = open_a(&hub);
    let (_, rev0) = hub.handle().system_model();
    let mut s = server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    ok(
        &mut s,
        c,
        "dspi_installation_set_zone_source",
        json!({"zone_id": "down", "source_id": "radio"}),
    );
    let id = held(&mut s, c, "dspi_installation_save", json!({}));
    ok(
        &mut s,
        c,
        "dspi_installation_set_programme",
        json!({"name": "Streamer", "kind": "spdif", "feeds_id": "B", "inputs": [1, 2]}),
    );
    let w = answer(&mut s, c, id, true);
    assert_eq!(w["result"]["isError"], json!(true), "{w}");
    assert_eq!(hub.handle().system_model().1, rev0, "nothing saved");

    // Someone else saves meanwhile.
    let (mut m, _) = hub.handle().system_model();
    m.rooms[1].name = "Cocktail bar".into();
    hub.handle().system_set_model(m).unwrap();
    let mut s = server(&set, ConfirmMode::NeverAsk);
    let c2 = s.connect();
    ok(
        &mut s,
        c2,
        "dspi_installation_set_zone_source",
        json!({"zone_id": "down", "source_id": "radio"}),
    );
    // Its draft was read after that save; one more save makes it stale.
    let (mut m, _) = hub.handle().system_model();
    m.rooms[0].name = "Lounge".into();
    hub.handle().system_set_model(m).unwrap();
    let e = err(&mut s, c2, "dspi_installation_save", json!({}));
    assert!(
        e.contains("nothing was saved") && e.contains("1 edit(s)"),
        "{e}"
    );
    let d = ok(
        &mut s,
        c2,
        "dspi_installation_draft",
        json!({"remake": true}),
    );
    assert_eq!(d["edits"], json!(1), "{d}");
    let saved = ok(&mut s, c2, "dspi_installation_save", json!({}));
    let (m, rev) = hub.handle().system_model();
    assert_eq!(saved["revision"], json!(rev));
    assert_eq!(m.zones[0].source.as_deref(), Some("radio"));
    assert_eq!(
        (m.rooms[0].name.as_str(), m.rooms[1].name.as_str()),
        ("Lounge", "Cocktail bar"),
        "the other saves stay"
    );
    hub.stop();
}

/// A scene with a source action, made and saved through the draft, then
/// recalled: risky under the default, and live at once on the zone's units.
#[test]
fn wp_feat1_a_recalled_source_scene_routes_at_once_and_says_so() {
    let hub = hub("recall");
    let (set, _) = open_a(&hub);
    let mut s = server(&set, ConfirmMode::AskRisky);
    let c = s.connect();
    let before = live_slots(&mut s, c);
    assert_ne!(before, json!(RADIO));
    let made = ok(
        &mut s,
        c,
        "dspi_installation_set_scene",
        json!({"name": "Radio \u{202e}on", "actions": [
            {"target": "zone:down", "op": "source", "source_id": "radio"}
        ]}),
    );
    // The new scene's id is from its (cleaned) name.
    let scene = made["element_id"].as_str().unwrap().to_owned();
    assert!(!scene.contains('\u{202e}'), "{scene}");
    // A source op on a room is the hub's refusal, here.
    let e = err(
        &mut s,
        c,
        "dspi_installation_set_scene",
        json!({"actions": [{"target": "room:main", "op": "source", "source_id": "radio"}]}),
    );
    assert!(e.contains("targets a zone"), "{e}");
    let id = held(&mut s, c, "dspi_installation_save", json!({}));
    answer(&mut s, c, id, true);
    // Saved, not recalled: nothing live moved.
    assert_eq!(live_slots(&mut s, c), before);

    let view = ok(&mut s, c, "dspi_get_system", json!({}));
    let at = view["scenes"]
        .as_array()
        .unwrap()
        .iter()
        .position(|x| x["id"] == json!(scene))
        .unwrap();
    let id = held(
        &mut s,
        c,
        "dspi_system_scene_apply",
        json!({"scene": scene}),
    );
    assert_eq!(
        risks(&s, id),
        [format!(
            "system: Recall scene {} of 1 on every unit it names",
            at + 1
        )]
    );
    let w = answer(&mut s, c, id, true);
    assert_eq!(w["result"]["isError"], json!(false), "{w}");
    let done = &w["result"]["structuredContent"];
    let message = done["note"].as_str().unwrap();
    assert!(
        message.contains("Zone Downstairs plays Radio (stored in the installation)")
            && message.contains("live on 1 of 1 unit(s)"),
        "{done}"
    );
    assert_eq!(done["taken"], json!(true), "{done}");
    // Live: unit A plays the radio's channels; stored: the zone plays it.
    assert_eq!(live_slots(&mut s, c), json!(RADIO));
    let (m, _) = hub.handle().system_model();
    assert_eq!(m.zones[0].source.as_deref(), Some("radio"));

    // The programme source and the scene's name are labels in the view.
    ok(
        &mut s,
        c,
        "dspi_installation_set_programme",
        json!({"programme_id": "tidal", "name": "Tidal \u{202e}streamer", "kind": "computer_usb",
               "feeds_id": "A", "inputs": [1, 2]}),
    );
    let d = ok(&mut s, c, "dspi_installation_diff", json!({}));
    let p = &d["site"]["programme_sources"][0];
    assert_eq!(p["display_name"], json!("Tidal streamer"), "{d}");
    assert!(p.get("name").is_none(), "{d}");
    hub.stop();
}

// -- WP INTI -- (review RF4)
/// A scene that loads a preset on unit A is an agent's preset load: refused
/// while A saves the master volume with each preset and for a unit this app
/// does not have open; otherwise A is silenced, the stored preset judged by
/// the tweeter rule after the hub loaded it, put back (its own slot, its
/// settings) when it lowered the tweeter, and its volume restored.
#[test]
fn wp_inti_a_scene_with_a_preset_load_is_judged_as_an_agents_load() {
    use crate::tests_wp_ffix::{LR4_HP, blank, confirm, hp, n_in};
    let hub = hub("scene-load");
    let mut model = site();
    model["scenes"] = json!([
        {"id": "test", "name": "Test", "actions": [{"target": "unit:A", "op": "preset_load", "arg": 8}]},
        {"id": "safe", "name": "Safe", "actions": [{"target": "unit:A", "op": "preset_load", "arg": 1}]},
        {"id": "at-bar", "name": "At the bar", "actions": [{"target": "unit:B", "op": "preset_load", "arg": 1}]}
    ]);
    hub.handle()
        .system_set_model(serde_json::from_value(model).unwrap())
        .unwrap();
    let (set, id) = open_a(&hub);
    let core = set.core(&id).unwrap();
    let tweeter = n_in(&core) + 1;
    let save = |slot: u8| {
        let token = core.confirm_token(dspi_core::ConfirmOp::SavePreset);
        core.save_preset_confirmed(slot, Some(token)).unwrap();
    };
    blank(&core, 1);
    core.set_matrix_route(0, 1, true, 0.0, false).unwrap();
    // Slot 9 (8): the tweeter at 850 Hz. Slot 2 (1): protected, the active one.
    core.set_crossover(tweeter, 0, hp(LR4_HP, 850.0)).unwrap();
    save(8);
    core.set_crossover(tweeter, 0, hp(LR4_HP, 3_000.0)).unwrap();
    save(1);
    core.set_master_volume(-12.0).unwrap();
    confirm(&core);
    let mut s = server(&set, ConfirmMode::NeverAsk);
    let c = s.connect();
    ok(&mut s, c, "dspi_get_system", json!({}));

    // Unit B is not open here: refused, nothing changed.
    let e = err(
        &mut s,
        c,
        "dspi_system_scene_apply",
        json!({"scene": "at-bar"}),
    );
    assert!(e.contains("does not have open"), "{e}");
    // A saves the master volume with each preset: refused before anything.
    core.set_master_volume_mode(1).unwrap();
    let e = err(
        &mut s,
        c,
        "dspi_system_scene_apply",
        json!({"scene": "test"}),
    );
    assert!(e.contains("saves the master volume"), "{e}");
    assert_eq!(core.get_state().active_preset_slot, 1);
    core.set_master_volume_mode(0).unwrap();
    core.set_master_volume(-12.0).unwrap();

    // The hub loads slot 9 on A (the fake units queue a frame's ops without
    // running them: the tool's seam loads it): the tweeter rule puts A back.
    crate::tools::library::HUB_LOADS.with(|h| h.set(true));
    let e = err(
        &mut s,
        c,
        "dspi_system_scene_apply",
        json!({"scene": "test"}),
    );
    assert!(e.contains("unit") && e.contains("put back"), "{e}");
    let st = core.get_state();
    assert_eq!(st.active_preset_slot, 1, "left on the scene's slot: {e}");
    assert!(
        (st.xover[usize::from(tweeter)][0].freq - 3_000.0).abs() < 1.0,
        "{}",
        st.xover[usize::from(tweeter)][0].freq
    );
    assert!(
        (st.master_volume_db - -12.0).abs() < 0.5,
        "{}",
        st.master_volume_db
    );
    // A protected preset passes, and A plays again.
    ok(
        &mut s,
        c,
        "dspi_system_scene_apply",
        json!({"scene": "safe"}),
    );
    let st = core.get_state();
    assert_eq!(st.active_preset_slot, 1);
    assert!(
        (st.master_volume_db - -12.0).abs() < 0.5,
        "{}",
        st.master_volume_db
    );
    crate::tools::library::HUB_LOADS.with(|h| h.set(false));
    hub.stop();
}
