//! WP SECDOC: the integrator examples of `docs/integrator/examples/` run against a
//! real `uconsole --demo hub` with TLS required (docs/integrator/hub-api.md section 2,
//! docs/integrator/security.md):
//! - the shell one through `uconsole api` over TLS, paired with the hub's PIN;
//! - curl (`hub_curl.sh`) pinned with `--pinnedpubkey`, and with `--cacert`;
//! - Python (`hub_rest.py`) with the exported certificate as its only trust anchor;
//! - Home Assistant: its configuration's requests, replayed as Home Assistant makes
//!   them (`verify_ssl: false`: no certificate check).
//!
//! And the refusals the docs promise: a wrong pin sends nothing, plaintext REST is
//! `426`, the system's roots do not trust a hub's self-signed certificate.
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
/// The serial and address the Home Assistant example is written for.
const HA_SERIAL: &str = "E46058388B1A2E2C";
const HA_HUB: &str = "https://hub.local:11915";
const WAIT: Duration = Duration::from_secs(60);

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("wp-secdoc-examples")
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

/// A running `uconsole --demo hub --loopback --tls require --json`, and its lines.
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
            "--tls",
            "require",
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

fn have(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// `curl -sS -w '\n%{http_code}' ARGS`: the body and the HTTP status (0: none, as
/// when the TLS handshake was refused).
fn curl(args: &[&str]) -> (String, u32) {
    let out = Command::new("curl")
        .args(["-sS", "-w", "\n%{http_code}"])
        .args(args)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    (body.to_owned(), code.trim().parse().unwrap_or(0))
}

/// One request of the Home Assistant configuration: method, URL, payload, and
/// whether it says `verify_ssl: false`.
#[derive(Debug, Default)]
struct HaRequest {
    method: String,
    url: String,
    payload: Option<String>,
    verify_ssl_off: bool,
}

/// The `url:` / `resource:` entries of a Home Assistant REST configuration, with the
/// `method`, `payload` and `verify_ssl` that follow each.
fn ha_requests(yaml: &str) -> Vec<HaRequest> {
    let quoted = |v: &str| v.trim().trim_matches('"').trim_matches('\'').to_owned();
    let mut out: Vec<HaRequest> = Vec::new();
    for line in yaml.lines().map(str::trim) {
        if line.starts_with('#') {
            continue;
        }
        if let Some(v) = line
            .strip_prefix("url:")
            .or_else(|| line.strip_prefix("resource:"))
        {
            out.push(HaRequest {
                method: "GET".into(),
                url: quoted(v),
                ..HaRequest::default()
            });
        } else if let Some(r) = out.last_mut() {
            if let Some(v) = line.strip_prefix("method:") {
                r.method = v.trim().to_ascii_uppercase();
            } else if let Some(v) = line.strip_prefix("payload:") {
                r.payload = Some(quoted(v));
            } else if line == "verify_ssl: false" {
                r.verify_ssl_off = true;
            }
        }
    }
    out
}

#[test]
fn wp_secdoc_the_integrator_examples_run_against_a_tls_hub() {
    let home = scratch("home");
    let store = home.join("store");

    // The shell example, through `uconsole api` over TLS, paired with the hub's PIN.
    let hub = Hub::start(&home, &store, &["--pair"]);
    let status = hub.next("status");
    assert_eq!(status["tls"], "require", "{status}");
    let port = status["port"].as_u64().unwrap();
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
    hub.stop();

    // On the hub's machine: a key, the certificate and the pin.
    let out = in_home(DSPI, &home)
        .args(["hub", "api-key", "new", "Examples", "--json", "--store"])
        .arg(&store)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let minted: Value = serde_json::from_slice(&out.stdout).unwrap();
    let key = minted["key"].as_str().unwrap().to_owned();
    let out = in_home(DSPI, &home)
        .args(["hub", "tls", "cert", "--store"])
        .arg(&store)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let pem = home.join("hub.pem");
    std::fs::write(&pem, &out.stdout).unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("-----BEGIN CERTIFICATE-----"));
    let out = in_home(DSPI, &home)
        .args(["hub", "tls", "fingerprint", "--json", "--store"])
        .arg(&store)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let fp: Value = serde_json::from_slice(&out.stdout).unwrap();
    let pinned = fp["pinnedpubkey"].as_str().unwrap().to_owned();
    assert!(pinned.starts_with("sha256//"), "{pinned}");

    let hub = Hub::start(&home, &store, &["--rest"]);
    let status = hub.next("status");
    assert_eq!(status["rest"], true);
    let port = status["port"].as_u64().unwrap();
    let https = format!("https://127.0.0.1:{port}");
    let bearer = format!("Authorization: Bearer {key}");

    if have("curl") {
        // The curl example, pinned.
        let out = in_home("sh", &home)
            .arg(examples().join("hub_curl.sh"))
            .arg(DEMO)
            .env("DSPI_HUB", &https)
            .env("DSPI_PIN", &pinned)
            .env("DSPI_KEY", &key)
            .output()
            .expect("sh runs");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "hub_curl.sh: {}\n{stdout}\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(stdout.contains("\"value\""), "{stdout}");
        assert!(stdout.contains("-20"), "the write: {stdout}");

        // Another pin: refused in the handshake, so nothing (no key) is sent.
        let wrong = format!("sha256//{}=", "A".repeat(43));
        let (body, code) = curl(&[
            "-k",
            "--pinnedpubkey",
            &wrong,
            "-H",
            &bearer,
            &format!("{https}/api/v1/health"),
        ]);
        assert_eq!(code, 0, "a wrong pin gets no answer: {body}");

        // --cacert with the exported certificate, by a name it carries.
        let (body, code) = curl(&[
            "--cacert",
            pem.to_str().unwrap(),
            "-H",
            &bearer,
            &format!("https://localhost:{port}/api/v1/health"),
        ]);
        assert_eq!(code, 200, "--cacert: {body}");
        // Without it, the system's roots do not trust a self-signed hub.
        let (_, code) = curl(&[
            "-H",
            &bearer,
            &format!("https://localhost:{port}/api/v1/health"),
        ]);
        assert_eq!(code, 0, "the system's roots refuse the hub's certificate");

        // Plaintext REST on a require hub: 426, the key not taken.
        let (body, code) = curl(&[
            "-H",
            &bearer,
            &format!("http://127.0.0.1:{port}/api/v1/health"),
        ]);
        assert_eq!(code, 426, "{body}");

        // Home Assistant's requests, as its configuration makes them.
        let yaml = std::fs::read_to_string(examples().join("home-assistant.yaml")).unwrap();
        let requests = ha_requests(&yaml);
        assert_eq!(requests.len(), 4, "{requests:?}");
        for r in &requests {
            assert!(r.url.starts_with(HA_HUB), "https only: {r:?}");
            assert!(r.verify_ssl_off, "the self-signed certificate: {r:?}");
            let url = r
                .url
                .replace(HA_HUB, &https)
                .replace(HA_SERIAL, DEMO)
                .replace("{{ slot | int }}", "0");
            let mut args = vec!["-k", "-X", r.method.as_str(), "-H", bearer.as_str()];
            let payload = r
                .payload
                .as_ref()
                .map(|p| p.replace("{{ volume | float }}", "-20.0"));
            if let Some(p) = &payload {
                args.extend(["-H", "Content-Type: application/json", "-d", p.as_str()]);
            }
            args.push(&url);
            let (body, code) = curl(&args);
            assert_eq!(code, 200, "{r:?}: {body}");
            let _: Value = serde_json::from_str(&body).unwrap_or_else(|e| panic!("{r:?}: {e}"));
        }
    } else {
        eprintln!("wp_secdoc: no curl here; hub_curl.sh and the Home Assistant requests not run");
    }

    // The Python example, with the hub's certificate as its only trust anchor.
    let python = |ca: Option<&Path>| {
        let mut c = in_home("python3", &home);
        c.arg("-I")
            .arg(examples().join("hub_rest.py"))
            .env("DSPI_HUB", &https)
            .env("DSPI_KEY", &key);
        if let Some(ca) = ca {
            c.env("DSPI_CA", ca);
        }
        c.output()
    };
    match python(Some(&pem)) {
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
            // Without DSPI_CA the system's roots refuse the hub, before the key is sent.
            let out = python(None).unwrap();
            assert!(!out.status.success(), "{out:?}");
            assert!(
                String::from_utf8_lossy(&out.stderr).contains("CERTIFICATE_VERIFY_FAILED"),
                "{out:?}"
            );
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("wp_secdoc: no python3 here; hub_rest.py not run");
        }
        Err(e) => panic!("python3: {e}"),
    }
    hub.stop();
}
