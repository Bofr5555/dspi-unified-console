//! WP P3FRZ: the curl and Python examples of `docs/integrator/examples/` reach the fleet's
//! REST endpoints (`hub-api.md` section 14.2, frozen as v1) on a real `uconsole --demo hub
//! --rest --rollout` that requires TLS:
//! - `hub_curl.sh SERIAL GROUP` (pinned with `--pinnedpubkey`) lists the groups, sets a
//!   group's master volume with a per-member offset, and reads the firmware images and
//!   the rollouts;
//! - `hub_rest.py` (the hub's certificate as its only trust anchor) does the same and
//!   prints each member's row and each image's label;
//! - and what the freeze promises stays so: no rollout start and no image upload over
//!   REST (`405` with `Allow: GET`).
//!
//! The group is the hub's own file (`groups.json`, written with the hub stopped, as no
//! REST path makes one); the image goes in with `uconsole hub firmware add`.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;

const DSPI: &str = env!(concat!("CARGO_BIN_EXE_", dspi_cmd::command!()));
const DEMO: &str = "DEMO0000DSPI0001";
const IMAGE: &str = "dspi-1.1.7.uf2";
const WAIT: Duration = Duration::from_secs(60);

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("wp-p3frz-examples")
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

/// A running `uconsole --demo hub --loopback --tls require --rest --rollout --json`.
struct Hub {
    child: Child,
    lines: mpsc::Receiver<Value>,
}

impl Hub {
    fn start(home: &Path, store: &Path) -> Self {
        let mut c = in_home(DSPI, home);
        c.args([
            "--demo",
            "hub",
            "--loopback",
            "--tls",
            "require",
            "--rest",
            "--rollout",
            "--port",
            "0",
            "--no-keep-awake",
            "--json",
        ])
        .arg("--store")
        .arg(store)
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

/// `uconsole ARGS --store STORE` on the hub's machine, the hub stopped: its stdout.
fn on_the_hub_machine(home: &Path, store: &Path, args: &[&str]) -> Vec<u8> {
    let out = in_home(DSPI, home)
        .args(args)
        .arg("--store")
        .arg(store)
        .output()
        .unwrap();
    assert!(out.status.success(), "{args:?}: {out:?}");
    out.stdout
}

// ── An image (laid out as the firmware guard's own tests build them) ─────────

const XIP: u32 = 0x1000_0000;
const RP2350: u32 = 0xE48B_FF59;
const BI_MARKER_START: u32 = 0x7188_EBF2;
const BI_MARKER_END: u32 = 0xE71A_A390;
const BI_TYPE_ID_AND_STRING: u16 = 6;
const TAG_RP: u16 = 0x5052;
const TAG_DS: u16 = 0x5344;
const ID_NAME: u32 = 0x0203_1C86;
const ID_VERSION: u32 = 0x11A9_BC3A;
const ID_STAMP: u32 = 0x0B7A_4C15;

fn block(target: u32, page: &[u8], no: u32, total: u32) -> Vec<u8> {
    let mut b = vec![0u8; 512];
    for (off, v) in [
        (0, 0x0A32_4655u32),
        (4, 0x9E5D_5157),
        (8, 0x2000),
        (12, target),
        (16, 256),
        (20, no),
        (24, total),
        (28, RP2350),
        (508, 0x0AB1_6F30),
    ] {
        b[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
    b[32..32 + page.len()].copy_from_slice(page);
    b
}

/// An RP2350 DSPi image of `version`, stamped `stamp` mV (not in any release manifest).
fn image(version: &str, stamp: &str) -> Vec<u8> {
    let entries: Vec<(u16, u32, String)> = vec![
        (TAG_RP, ID_NAME, "DSPi".into()),
        (TAG_RP, ID_VERSION, version.into()),
        (TAG_DS, ID_STAMP, stamp.into()),
    ];
    let mut f = vec![0u8; 0x2000];
    let put =
        |f: &mut Vec<u8>, off: usize, v: u32| f[off..off + 4].copy_from_slice(&v.to_le_bytes());
    let a = |off: usize| XIP + off as u32;
    put(&mut f, 0x100, BI_MARKER_START);
    put(&mut f, 0x104, a(0x200));
    put(&mut f, 0x108, a(0x200 + 4 * entries.len()));
    put(&mut f, 0x10C, a(0x180));
    put(&mut f, 0x110, BI_MARKER_END);
    for (i, (tag, id, text)) in entries.iter().enumerate() {
        let at = 0x400 + 16 * i;
        put(&mut f, 0x200 + 4 * i, a(at));
        f[at..at + 2].copy_from_slice(&BI_TYPE_ID_AND_STRING.to_le_bytes());
        f[at + 2..at + 4].copy_from_slice(&tag.to_le_bytes());
        put(&mut f, at + 4, *id);
        let s = 0x800 + 64 * i;
        put(&mut f, at + 8, a(s));
        f[s..s + text.len()].copy_from_slice(text.as_bytes());
    }
    let pages: Vec<&[u8]> = f.chunks(256).collect();
    let mut out = Vec::new();
    for (i, p) in pages.iter().enumerate() {
        out.extend(block(XIP + 256 * i as u32, p, i as u32, pages.len() as u32));
    }
    out
}

#[test]
fn wp_p3frz_the_examples_reach_the_fleet_endpoints() {
    let home = scratch("home");
    let store = home.join("store");

    // A first start makes the store, its TLS key and the demo device's inventory record.
    let hub = Hub::start(&home, &store);
    let status = hub.next("status");
    assert_eq!(status["tls"], "require", "{status}");
    assert_eq!(status["rest"], true, "{status}");
    hub.stop();

    // On the hub's machine: a key (control), the certificate and the pin; a group of the
    // demo device; an image in the store.
    let minted: Value = serde_json::from_slice(&on_the_hub_machine(
        &home,
        &store,
        &["hub", "api-key", "new", "Fleet examples", "--json"],
    ))
    .unwrap();
    let key = minted["key"].as_str().unwrap().to_owned();
    let pem = home.join("hub.pem");
    std::fs::write(
        &pem,
        on_the_hub_machine(&home, &store, &["hub", "tls", "cert"]),
    )
    .unwrap();
    let fp: Value = serde_json::from_slice(&on_the_hub_machine(
        &home,
        &store,
        &["hub", "tls", "fingerprint", "--json"],
    ))
    .unwrap();
    let pinned = fp["pinnedpubkey"].as_str().unwrap().to_owned();
    std::fs::write(
        store.join("groups.json"),
        format!(
            "{{\"v\":1,\"next_id\":2,\"groups\":[\n\
             {{\"id\":\"g1\",\"name\":\"Racks\",\"members\":[\"{DEMO}\"]}}\n]}}\n"
        ),
    )
    .unwrap();
    let file = home.join(IMAGE);
    std::fs::write(&file, image("1.1.7", "1200")).unwrap();
    on_the_hub_machine(
        &home,
        &store,
        &["hub", "firmware", "add", file.to_str().unwrap()],
    );

    let hub = Hub::start(&home, &store);
    let status = hub.next("status");
    let port = status["port"].as_u64().unwrap();
    let https = format!("https://127.0.0.1:{port}");

    if have("curl") {
        let out = in_home("sh", &home)
            .arg(examples().join("hub_curl.sh"))
            .args([DEMO, "g1"])
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
        assert!(stdout.contains("\"Racks\""), "the groups: {stdout}");
        assert!(stdout.contains("\"results\""), "the group write: {stdout}");
        // -20 dB with the member's -2 dB offset.
        assert!(stdout.contains("-22"), "the offset value: {stdout}");
        assert!(stdout.contains(IMAGE), "the firmware images: {stdout}");
        assert!(stdout.contains("\"rollouts\""), "the rollouts: {stdout}");
        assert!(!stdout.contains("no firmware rollout"), "{stdout}");

        // No rollout start and no image upload over REST, whatever the key.
        for path in ["/api/v1/rollouts", "/api/v1/firmware"] {
            let out = Command::new("curl")
                .args(["-sS", "-k", "--pinnedpubkey", &pinned, "-X", "POST", "-i"])
                .args(["-H", &format!("Authorization: Bearer {key}")])
                .args(["-H", "Content-Type: application/json", "-d", "{}"])
                .arg(format!("{https}{path}"))
                .output()
                .unwrap();
            let head = String::from_utf8_lossy(&out.stdout).to_ascii_lowercase();
            assert!(head.starts_with("http/1.1 405"), "POST {path}: {head}");
            assert!(head.contains("allow: get"), "POST {path}: {head}");
        }
    } else {
        eprintln!("wp_p3frz: no curl here; hub_curl.sh not run");
    }

    match in_home("python3", &home)
        .arg("-I")
        .arg(examples().join("hub_rest.py"))
        .env("DSPI_HUB", &https)
        .env("DSPI_KEY", &key)
        .env("DSPI_CA", &pem)
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
            assert!(
                stdout.contains(&format!("group g1 Racks members {DEMO}")),
                "{stdout}"
            );
            assert!(stdout.contains(&format!("group g1 {DEMO} ok")), "{stdout}");
            assert!(
                stdout.contains(&format!("firmware {IMAGE} RP2350 1.1.7 NOT VERIFIED")),
                "{stdout}"
            );
            assert!(stdout.contains("firmware images 1"), "{stdout}");
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("wp_p3frz: no python3 here; hub_rest.py not run");
        }
        Err(e) => panic!("python3: {e}"),
    }
    hub.stop();
}
