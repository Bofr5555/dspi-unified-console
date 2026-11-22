//! WP MCP6: the server the official MCP conformance suite runs against
//! (`ci/mcp-conformance/run.sh`). Not a test of its own: it is ignored, and
//! the script runs it by name.
//!
//! It serves the real loopback listener ([`HttpServer`]) over the demo
//! device, under the policy the app starts agents with
//! ([`ConfirmMode::AskRisky`]), with one paired client. It writes
//! `{"port", "token"}` to `$MCP6_CONF_DIR/server.json` once it listens, and
//! serves until `$MCP6_CONF_DIR/stop` appears or `$MCP6_SERVE_SECS` (default
//! 900) have passed. `$MCP6_PORT` picks the port (default: any free one).
#![cfg(feature = "http")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dspi_core::{CoreSet, DeviceTarget};
use dspi_mcp::http::{HttpConfig, HttpLimits, HttpServer};
use dspi_mcp::{AgentPolicy, ConfirmMode, Server, ServerConfig};
use serde_json::json;

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[test]
#[ignore = "the conformance suite's server: ci/mcp-conformance/run.sh runs it"]
fn wp_mcp6_serve_for_the_conformance_suite() {
    let dir = PathBuf::from(std::env::var("MCP6_CONF_DIR").expect("MCP6_CONF_DIR"));
    let secs = env_u64("MCP6_SERVE_SECS", 900);
    let port = u16::try_from(env_u64("MCP6_PORT", 0)).expect("MCP6_PORT");

    let set = CoreSet::new();
    let demo = set.open(DeviceTarget::Demo).expect("the demo device");
    let config = ServerConfig {
        policy: Arc::new(AgentPolicy::new(ConfirmMode::AskRisky)),
        ..ServerConfig::default()
    };
    let http = HttpServer::start(
        Server::new(set, config),
        HttpConfig {
            port,
            store_dir: None,
            limits: HttpLimits::default(),
        },
        None,
    )
    .expect("the listener");
    http.with_server(|s| s.set_active_device(Some(demo)));
    let token = http.add_client("conformance").expect("a client").token;

    let tmp = dir.join("server.json.tmp");
    std::fs::write(
        &tmp,
        json!({"port": http.port(), "token": token}).to_string(),
    )
    .expect("server.json");
    std::fs::rename(&tmp, dir.join("server.json")).expect("server.json");
    eprintln!("wp_mcp6_serve: listening on {:?}", http.addrs());

    let stop = dir.join("stop");
    let until = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < until && !stop.exists() {
        std::thread::sleep(Duration::from_millis(200));
    }
    http.stop();
}
