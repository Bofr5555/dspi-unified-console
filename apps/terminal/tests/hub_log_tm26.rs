//! WP TM26: what `uconsole hub` logs (hw-1 M3), on a real hub process.
//!
//! A demo hub on loopback opens pairing; the Terminal pairs with the PIN, comes back
//! on its token and leaves, which is what makes tungstenite log its close frames. At
//! `RUST_LOG=info` nothing below info may appear; at `RUST_LOG=trace` the PIN and the
//! token may not, nor any tungstenite trace line.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const DSPI: &str = env!(concat!("CARGO_BIN_EXE_", dspi_cmd::command!()));
const WAIT: Duration = Duration::from_secs(60);

/// A scratch directory under the target directory, never on tmpfs.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("hub-log-tm26")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn dspi(home: &Path) -> Command {
    let mut c = Command::new(DSPI);
    c.current_dir(home)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env_remove("DSPI_PIN")
        .env_remove("RUST_LOG")
        .stdin(Stdio::null());
    c
}

/// The hub's `--json` events, one per line, read on a thread so a wait can time out.
fn events(stdout: ChildStdout) -> mpsc::Receiver<serde_json::Value> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Ok(v) = serde_json::from_str(&line) {
                let _ = tx.send(v);
            }
        }
    });
    rx
}

fn next_event(rx: &mpsc::Receiver<serde_json::Value>, kind: &str) -> serde_json::Value {
    loop {
        let v = rx
            .recv_timeout(WAIT)
            .unwrap_or_else(|_| panic!("no `{kind}` event from the hub"));
        if v["event"] == kind {
            return v;
        }
    }
}

struct Stop(Child);

impl Drop for Stop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Run a hub at `rust_log`, pair a client and reconnect it on its token. Returns the
/// hub's standard error, the PIN and the token.
fn session_at(name: &str, rust_log: &str) -> (String, String, String) {
    let home = scratch(name);
    let log_path = home.join("hub-stderr.log");
    let log = std::fs::File::create(&log_path).expect("log file");
    let mut child = dspi(&home)
        .args([
            "--demo",
            "hub",
            "run",
            "--loopback",
            "--port",
            "0",
            "--pair",
        ])
        .args(["--no-keep-awake", "--json", "--store"])
        .arg(home.join("store"))
        .env("RUST_LOG", rust_log)
        .stdout(Stdio::piped())
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn the hub");
    let rx = events(child.stdout.take().expect("stdout"));
    let hub = Stop(child);

    let port = next_event(&rx, "status")["port"].as_u64().expect("port");
    let pin = next_event(&rx, "pairing")["pin"]
        .as_str()
        .expect("pin")
        .to_owned();
    let address = format!("127.0.0.1:{port}");

    let client = scratch(&format!("{name}-client"));
    let paired = dspi(&client)
        .args(["--link", &address, "--pin", &pin, "list"])
        .output()
        .expect("spawn the client");
    assert_eq!(
        paired.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&paired.stderr)
    );
    let again = dspi(&client)
        .args(["--link", &address, "get", "vol.user"])
        .output()
        .expect("spawn the client");
    assert_eq!(
        again.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    let tokens = std::fs::read_to_string(client.join(concat!(
        "config/",
        dspi_cmd::command!(),
        "/link-tokens.toml"
    )))
    .expect("the client kept a token");
    let token = tokens
        .lines()
        .find_map(|l| l.split_once(" = \"").map(|(_, v)| v.trim_end_matches('"')))
        .expect("a token line")
        .to_owned();
    assert!(token.len() >= 16, "{tokens}");

    // The hub sees both clients go, which is when tungstenite logs.
    next_event(&rx, "clientDisconnected");
    next_event(&rx, "clientDisconnected");
    drop(hub);
    let stderr = std::fs::read_to_string(&log_path).expect("hub log");
    (stderr, pin, token)
}

/// `needle` as a whole word: not inside a longer run of digits or letters.
fn contains_word(hay: &str, needle: &str) -> bool {
    hay.match_indices(needle).any(|(i, _)| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + needle.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

#[test]
fn tm26_hub_at_info_logs_nothing_below_info() {
    let (stderr, _, _) = session_at("info", "info");
    // The same session that logs tungstenite's close frames at debug (below) did log.
    assert!(
        stderr.lines().any(|l| l.starts_with("[INFO")),
        "nothing at all:\n{stderr}"
    );
    for line in stderr.lines() {
        assert!(
            !line.starts_with("[DEBUG") && !line.starts_with("[TRACE"),
            "below info at RUST_LOG=info: {line}\n{stderr}"
        );
    }
}

#[test]
fn tm26_hub_at_trace_never_logs_the_pin_or_the_token() {
    let (stderr, pin, token) = session_at("trace", "trace");
    // The low levels are really on (nothing on this path logs at trace itself;
    // tungstenite's close frames come at debug), so the absence below means something.
    assert!(
        stderr.lines().any(|l| l.starts_with("[DEBUG tungstenite")),
        "no debug lines:\n{stderr}"
    );
    assert!(!contains_word(&stderr, &pin), "the PIN was logged");
    assert!(!stderr.contains(&token), "the token was logged");
    assert!(
        !stderr.lines().any(|l| l.starts_with("[TRACE tungstenite")),
        "tungstenite at trace"
    );
}
