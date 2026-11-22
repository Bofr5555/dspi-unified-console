//! WP MCP3: `uconsole mcp` as an agent host runs it, a child process on
//! pipes, against a listener this test runs: it pairs, keeps its token, and
//! relays both eras; with the app gone it says so and keeps going.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use dspi_core::{CoreSet, DeviceTarget};
use dspi_mcp::http::{Event, HttpConfig, HttpServer, Listener};
use dspi_mcp::{Server, ServerConfig};
use serde_json::{Value, json};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::current_exe()
        .unwrap()
        .with_file_name("test-scratch")
        .join(format!("mcp3-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `uconsole mcp --port <port>` over `lines`; (stdout lines, stderr).
fn relay(port: u16, config: &PathBuf, lines: &[Value]) -> (Vec<Value>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_uconsole"))
        .args(["mcp", "--port", &port.to_string()])
        .env("XDG_CONFIG_HOME", config)
        .env("HOME", config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    // One line at a time, each reply awaited, as a host does.
    let mut replies = Vec::new();
    let mut lines_out = stdout.lines();
    for l in lines {
        writeln!(stdin, "{l}").unwrap();
        stdin.flush().unwrap();
        if l.get("id").is_some() {
            let reply = lines_out.next().unwrap().unwrap();
            replies.push(serde_json::from_str(&reply).unwrap());
        }
    }
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{:?}", out.status);
    (replies, String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn wp_mcp3_uconsole_mcp_relays_an_agent_to_the_app() {
    let config = scratch("relay");
    let set = CoreSet::new();
    set.open(DeviceTarget::Demo).unwrap();
    let (tx, rx) = mpsc::channel::<u64>();
    let tx = Mutex::new(tx);
    let listener: Listener = Arc::new(move |e: &Event| {
        if let Event::PairingRequested { id, .. } = e {
            let _ = tx.lock().unwrap().send(*id);
        }
    });
    let http = Arc::new(
        HttpServer::start(
            Server::new(set, ServerConfig::default()),
            HttpConfig {
                port: 0,
                store_dir: Some(config.join("app")),
                ..HttpConfig::default()
            },
            Some(listener),
        )
        .unwrap(),
    );
    let port = http.port();
    let user = {
        let http = Arc::downgrade(&http);
        std::thread::spawn(move || {
            while let Ok(id) = rx.recv_timeout(Duration::from_secs(60)) {
                if let Some(h) = http.upgrade() {
                    h.pairing_answer(id, true).unwrap();
                }
            }
        })
    };
    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-11-25", "capabilities": {},
        "clientInfo": {"name": "claude-ai", "title": "Claude Desktop", "version": "1"}}});
    let meta = json!({"io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {}});
    let (replies, err) = relay(
        port,
        &config,
        &[
            init.clone(),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "dspi_list_devices", "arguments": {}}}),
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list", "params": {"_meta": meta}}),
        ],
    );
    assert_eq!(
        replies[0]["result"]["serverInfo"]["name"], "dspi-console",
        "{replies:?}\n{err}"
    );
    assert_eq!(
        replies[1]["result"]["isError"],
        json!(false),
        "{}",
        replies[1]
    );
    assert_eq!(replies[2]["result"]["resultType"], "complete");
    assert!(err.contains("Claude Desktop"), "{err}");
    let token_file = config.join("uconsole").join("mcp-token");
    let token = std::fs::read_to_string(&token_file).unwrap();
    assert!(token.starts_with("dspi-mcp-"));
    assert!(!err.contains(token.trim()), "the token is never printed");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&token_file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    assert_eq!(http.clients().len(), 1);

    // The second run uses the kept token: no new client.
    let (replies, _) = relay(port, &config, std::slice::from_ref(&init));
    assert!(replies[0].get("result").is_some());
    assert_eq!(http.clients().len(), 1);

    // The app gone: the agent is told, the relay goes on to the end.
    let Ok(http) = Arc::try_unwrap(http) else {
        panic!("still shared")
    };
    http.stop();
    drop(user);
    let (replies, _) = relay(
        port,
        &config,
        &[init, json!({"jsonrpc": "2.0", "id": 2, "method": "ping"})],
    );
    assert_eq!(replies.len(), 2);
    assert!(
        replies[1]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Start DSPi Console")
    );
}
