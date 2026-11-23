//! WP INTF: FEAT1, FEAT2 and FEAT3 together. The overview's
//! `programme_sources` from FEAT1's installation field (a unit fed through
//! its slots, over USB and S/PDIF, the sender itself, a unit the description
//! does not reach, no installation), the tools' order after the three, and
//! the instructions, the guide and the tools naming only tools that exist.

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;

use dspi_core::{CoreSet, DeviceTarget, HubTarget};
use dspi_hub::{AccessMode, Hub, HubConfig, NetworkConfig, ServiceConfig};
use dspi_proto::ext::alink::AlinkRole;
use dspi_transport::FakeDevice;
use dspi_transport::fake::AlinkSetup;
use serde_json::{Value, json};

use crate::policy::{AgentPolicy, ConfirmMode};
use crate::{Server, ServerConfig};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::current_exe()
        .unwrap()
        .with_file_name("test-scratch")
        .join(format!("intf-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hub(name: &str, model: Option<Value>) -> Hub {
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
        ..HubConfig::in_process("INTF hub")
    })
    .unwrap();
    for (serial, role) in [
        ("IF-TX", AlinkRole::Sender),
        ("IF-A", AlinkRole::Receiver),
        ("IF-B", AlinkRole::Receiver),
        ("IF-C", AlinkRole::Receiver),
    ] {
        let setup = AlinkSetup {
            role,
            ..AlinkSetup::default()
        };
        hub.attach(
            Box::new(FakeDevice::demo_alink_as(setup).with_serial(serial)),
            0,
        )
        .unwrap();
    }
    if let Some(m) = model {
        hub.handle()
            .system_set_model(serde_json::from_value(m).unwrap())
            .unwrap();
    }
    hub
}

fn request(method: &str, mut params: Value) -> Value {
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "test-agent", "version": "1"},
    });
    json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params})
}

/// `serial` of `hub`'s `programme_sources`, as an agent reads the overview.
fn sources(hub: &Hub, serial: &str) -> Value {
    let port = hub.network().unwrap().port();
    let set = CoreSet::new();
    set.open(DeviceTarget::Hub(HubTarget {
        url: format!("ws://127.0.0.1:{port}/dspi/v1"),
        hub_id: String::new(),
        serial: serial.into(),
        client_name: "wp-intf".into(),
        app_version: "0".into(),
        credential: String::new(),
        tls_advertised: false,
    }))
    .unwrap();
    let mut s = server(&set);
    let c = s.connect();
    let r = s
        .handle_value(
            c,
            0,
            request(
                "tools/call",
                json!({"name": "dspi_get_overview", "arguments": {}}),
            ),
        )
        .unwrap();
    assert_eq!(r["result"]["isError"], json!(false), "{r}");
    r["result"]["structuredContent"]["programme"]["programme_sources"].clone()
}

fn server(set: &Arc<CoreSet>) -> Server {
    Server::new(
        Arc::clone(set),
        ServerConfig {
            policy: Arc::new(AgentPolicy::new(ConfirmMode::AskRisky)),
            ..ServerConfig::default()
        },
    )
}

#[test]
fn wp_intf_the_overview_names_the_programme_sources_of_this_unit() {
    let hub = hub(
        "sources",
        Some(json!({
            "senders": [{"id": "tx", "name": "Sender", "serial": "IF-TX"}],
            "units": [
                {"id": "A", "name": "Left", "serial": "IF-A", "upstream": "tx",
                 "slots": [9, 2, 255, 3, 255, 255, 255, 255]},
                {"id": "B", "name": "Right", "serial": "IF-B", "upstream": "tx"},
                {"id": "C", "name": "Desk", "serial": "IF-C", "upstream": "",
                 "transport": "usb_direct"}
            ],
            "programmeSources": [
                {"id": "plex", "name": "Plex\u{202e} on the HTPC", "kind": "alink_sender",
                 "feeds": "tx", "inputs": [2, 3]},
                {"id": "tv", "name": "TV", "kind": "alink_sender", "feeds": "tx", "inputs": [6, 7]},
                {"id": "cd", "name": "CD", "kind": "spdif", "feeds": "A", "inputs": [0, 1]},
                {"id": "pc", "name": "Desk PC", "kind": "computer_usb", "feeds": "C", "inputs": [0, 1]}
            ]
        })),
    );
    let row = |id: &str, name: &str, kind: &str, feeds: &str, inputs: &[u8], unit: Value| {
        json!({"programme_id": id, "programme_name": name, "kind": kind, "feeds_id": feeds,
               "inputs": inputs, "unit_inputs": unit})
    };
    // A takes channels 2 and 3 on its inputs 1 and 3, none of the TV's; its
    // own S/PDIF box. The name is cleaned.
    let a = sources(&hub, "IF-A");
    let plex = a[0]["programme_name"].as_str().unwrap().to_owned();
    assert!(
        plex.starts_with("Plex") && !plex.contains('\u{202e}'),
        "{plex:?}"
    );
    assert_eq!(
        a,
        json!([
            row("plex", &plex, "alink_sender", "tx", &[2, 3], json!([1, 3])),
            row("cd", "CD", "spdif", "A", &[0, 1], json!([0, 1])),
        ])
    );
    // B's slots are its own (the installation does not say): both of the
    // sender's, on inputs unknown here.
    assert_eq!(
        sources(&hub, "IF-B"),
        json!([
            row("plex", &plex, "alink_sender", "tx", &[2, 3], Value::Null),
            row("tv", "TV", "alink_sender", "tx", &[6, 7], Value::Null),
        ])
    );
    // C is on USB: its computer only. The sender: what is fed into it.
    assert_eq!(
        sources(&hub, "IF-C"),
        json!([row(
            "pc",
            "Desk PC",
            "computer_usb",
            "C",
            &[0, 1],
            json!([0, 1])
        )])
    );
    let tx = sources(&hub, "IF-TX");
    assert_eq!(tx[0]["programme_id"], json!("plex"));
    assert_eq!(tx[1]["programme_id"], json!("tv"));
    assert_eq!(tx[0]["unit_inputs"], Value::Null);
    assert_eq!(tx.as_array().unwrap().len(), 2);
    hub.stop();
}

#[test]
fn wp_intf_a_described_installation_with_none_and_no_description_differ() {
    // An installation that describes nothing for this unit: an empty list.
    let described = hub(
        "none",
        Some(
            json!({"units": [{"id": "A", "name": "A", "serial": "IF-A", "upstream": "tx"}],
                    "senders": [{"id": "tx", "name": "Sender", "serial": "IF-TX"}]}),
        ),
    );
    assert_eq!(sources(&described, "IF-A"), json!([]));
    described.stop();
    // A hub with none stored: the hub's own answer, an empty one.
    let bare = hub("bare", None);
    let got = sources(&bare, "IF-A");
    assert!(got.is_null() || got == json!([]), "{got}");
    bare.stop();
}

/// The tools after the measuring ones, in `tools/list` order.
const AFTER_MEASURING: &[&str] = &[
    "dspi_installation_set_slots",
    "dspi_installation_set_source",
    "dspi_installation_set_zone_source",
    "dspi_installation_set_scene",
    "dspi_installation_set_programme",
    "dspi_system_set_volume",
    "dspi_list_schedules",
    "dspi_list_known_devices",
    "dspi_open_device",
    "dspi_close_device",
    "dspi_get_host_audio",
];

#[test]
fn wp_intf_the_tool_order_and_the_texts_name_only_tools_that_exist() {
    let set = CoreSet::new();
    let mut s = server(&set);
    let c = s.connect();
    let r = s
        .handle_value(c, 0, request("tools/list", json!({})))
        .unwrap();
    let tools = r["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    let at = names
        .iter()
        .position(|n| *n == "dspi_get_measure_options")
        .unwrap();
    assert_eq!(&names[at + 1..], AFTER_MEASURING);
    // Every tool the instructions, the guide or a description names exists
    // (`dspi_installation_set_*` is the family).
    let mut texts = vec![
        crate::protocol::INSTRUCTIONS.to_owned(),
        crate::resources::GUIDE.to_owned(),
    ];
    texts.extend(
        tools
            .iter()
            .map(|t| t["description"].as_str().unwrap().to_owned()),
    );
    for text in texts {
        for word in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
            let family = word.ends_with('_');
            if !word.starts_with("dspi_") {
                continue;
            }
            assert!(
                names
                    .iter()
                    .any(|n| *n == word || (family && n.starts_with(word))),
                "{word} is not a tool"
            );
        }
    }
    // The three read as one: what plays, how it is routed, what opens.
    for tool in AFTER_MEASURING {
        assert!(
            crate::resources::GUIDE.contains(tool)
                || tool.starts_with("dspi_installation_set_")
                    && crate::resources::GUIDE.contains("dspi_installation_set_*"),
            "the guide does not name {tool}"
        );
    }
}
