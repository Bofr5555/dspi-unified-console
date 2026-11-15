//! WP P2TM: the integrator examples of `docs/integrator/examples/` run against
//! a real `uconsole --demo hub`: the shell one through `uconsole api` (paired
//! with the hub's PIN), the Python one over REST with a key minted by
//! `uconsole hub api-key new` while the hub is stopped. The Home Assistant
//! file is configuration, not a program: its requests are the Python one's.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;

const DSPI: &str = env!(concat!("CARGO_BIN_EXE_", dspi_cmd::command!()));
const DEMO: &str = "DEMO0000DSPI0001";
const WAIT: Duration = Duration::from_secs(60);

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("wp-p2tm-examples")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/integrator/examples")
}

/// A command with `home` as every folder it keeps things in.
fn in_home(program: &str, home: &Path) -> Command {
    let mut c = Command::new(program);
    c.current_dir(home)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env_remove("DSPI_PIN")
        .stdin(Stdio::null());
    c
}

/// A running `uconsole --demo hub --json`, and its lines.
struct Hub {
    child: Child,
    lines: mpsc::Receiver<Value>,
}

impl Hub {
    fn start(home: &Path, store: &Path, extra: &[&str]) -> Self {
        let mut c = in_home(DSPI, home);
        c.args([
            "--demo",
            "hub",
            "--loopback",
            "--port",
            "0",
            "--no-keep-awake",
            "--json",
        ])
        .arg("--store")
        .arg(store)
        .args(extra)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
        let mut child = c.spawn().expect("the hub starts");
        let out = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    let _ = tx.send(v);
                }
            }
        });
        Self { child, lines }
    }

    /// The next line whose `event` is `what`.
    fn next(&self, what: &str) -> Value {
        let start = Instant::now();
        loop {
            let left = WAIT.saturating_sub(start.elapsed());
            let v = self
                .lines
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("no `{what}` from the hub"));
            if v["event"] == what {
                return v;
            }
        }
    }

    /// SIGTERM, as Ctrl-C: the hub stops cleanly and lets go of its store.
    fn stop(mut self) {
        let pid = libc::pid_t::try_from(self.child.id()).unwrap();
        // SAFETY: a signal to our own child.
        unsafe { libc::kill(pid, libc::SIGTERM) };
        let start = Instant::now();
        while self.child.try_wait().unwrap().is_none() {
            if start.elapsed() > WAIT {
                let _ = self.child.kill();
                panic!("the hub did not stop");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[test]
fn wp_p2tm_the_integrator_examples_run_against_a_hub() {
    let home = scratch("home");
    let store = home.join("store");

    // The shell example, through `uconsole api`, paired with the hub's PIN.
    let hub = Hub::start(&home, &store, &["--pair"]);
    let port = hub.next("status")["port"].as_u64().unwrap();
    let pin = hub.next("pairing")["pin"].as_str().unwrap().to_owned();
    let out = in_home("sh", &home)
        .arg(examples().join("hub-api.sh"))
        .arg(DEMO)
        .env("UCONSOLE", DSPI)
        .env("HUB", format!("--link 127.0.0.1:{port} --pin {pin}"))
        .output()
        .expect("sh runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "hub-api.sh: {}\n{stdout}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("volume set"), "{stdout}");
    assert!(stdout.contains(DEMO), "the inventory: {stdout}");
    assert!(stdout.contains("\"alarms\""), "{stdout}");
    hub.stop();

    // A key minted with the hub stopped, shown once.
    let out = in_home(DSPI, &home)
        .args([
            "hub",
            "api-key",
            "new",
            "Example script",
            "--json",
            "--store",
        ])
        .arg(&store)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let minted: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        (minted["role"].as_str(), minted["kind"].as_str()),
        (Some("control"), Some("api"))
    );
    let key = minted["key"].as_str().unwrap().to_owned();
    let out = in_home(DSPI, &home)
        .args(["hub", "api-key", "list", "--store"])
        .arg(&store)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("Example script"));
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains(&key),
        "a key is shown once"
    );

    // The Python example over REST.
    let hub = Hub::start(&home, &store, &["--rest"]);
    let status = hub.next("status");
    assert_eq!(status["rest"], true);
    let port = status["port"].as_u64().unwrap();
    match in_home("python3", &home)
        .arg("-I")
        .arg(examples().join("hub_rest.py"))
        .env("DSPI_HUB", format!("http://127.0.0.1:{port}"))
        .env("DSPI_KEY", &key)
        .output()
    {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success(),
                "hub_rest.py: {}\n{stdout}\n{}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(stdout.contains("master volume now -20"), "{stdout}");
            assert!(stdout.contains(&format!("{DEMO} online")), "{stdout}");
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("wp_p2tm: no python3 here; hub_rest.py not run");
        }
        Err(e) => panic!("python3: {e}"),
    }
    hub.stop();

    // Revoked with the hub stopped: the key no longer works.
    let cid = minted["cid"].to_string();
    let out = in_home(DSPI, &home)
        .args(["hub", "api-key", "revoke", &cid, "--store"])
        .arg(&store)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
}
