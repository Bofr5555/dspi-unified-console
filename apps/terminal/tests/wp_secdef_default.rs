//! WP SECDEF (tests written by WP SECDOC): `uconsole hub` requires TLS by default
//! (OD-SECb, reconsidered 2026-11-15), and a hub store from before TLS upgrades
//! (PLAN-SEC 5, SECDEF).
//!
//! The default is a network hub's, so this one is not on `--loopback` (whose default
//! is `off`). It must not advertise on the machine's network, so it runs in a network
//! namespace of its own with only loopback in it: the outer test starts this test
//! binary again under `unshare -rn` (a user and a network namespace; no root), and
//! the inner test brings loopback up and runs the real `uconsole` there. Where
//! `unshare` cannot make one, the outer test says so and passes without running.
#![cfg(target_os = "linux")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;

const DSPI: &str = env!(concat!("CARGO_BIN_EXE_", dspi_cmd::command!()));
const DEMO: &str = "DEMO0000DSPI0001";
const WAIT: Duration = Duration::from_secs(60);
/// Set for the inner run, in its own network namespace.
const INNER: &str = "WP_SECDEF_INNER";

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("wp-secdef-default")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
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
    fn start(home: &Path, store: &Path, args: &[&str]) -> Self {
        let mut c = in_home(DSPI, home);
        c.args(["--demo", "hub", "--port", "0", "--no-keep-awake", "--json"])
            .arg("--store")
            .arg(store)
            .args(args)
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

    /// The next line whose `event` is `what`; every line before it is in `skipped`.
    fn next_noting(&self, what: &str, skipped: &mut Vec<Value>) -> Value {
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
            skipped.push(v);
        }
    }

    fn next(&self, what: &str) -> Value {
        self.next_noting(what, &mut Vec::new())
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

fn tcp(port: u16) -> TcpStream {
    let s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s
}

/// A plain HTTP request: the whole answer.
fn plain(port: u16, request: &str) -> Vec<u8> {
    let mut s = tcp(port);
    s.write_all(request.as_bytes()).unwrap();
    let mut all = Vec::new();
    let _ = s.read_to_end(&mut all);
    all
}

/// A plaintext WebSocket upgrade: the head, and the first frame's close code and
/// reason (a server's frames are not masked).
fn plain_upgrade(port: u16) -> (String, Option<(u16, String)>) {
    let all = plain(
        port,
        &format!(
            "GET /dspi/v1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: Upgrade\r\n\
             Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Sec-WebSocket-Protocol: dspi-link-1\r\n\r\n"
        ),
    );
    let at = all
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map_or(all.len(), |p| p + 4);
    let head = String::from_utf8_lossy(&all[..at]).into_owned();
    let frame = &all[at..];
    let close = (frame.len() >= 4 && frame[0] == 0x88 && frame[1] < 126).then(|| {
        let len = usize::from(frame[1]);
        let code = u16::from_be_bytes([frame[2], frame[3]]);
        let reason = String::from_utf8_lossy(&frame[4..(2 + len).min(frame.len())]).into_owned();
        (code, reason)
    });
    (head, close)
}

fn have(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// `curl -sS -w '\n%{http_code}' ARGS`: the body and the HTTP status (0: none).
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

/// Loopback up, in the namespace `unshare -rn` made (there it is down).
fn loopback_up() {
    // SAFETY: plain ioctls on a socket of our own, with an ifreq we own.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        assert!(fd >= 0, "a socket");
        let mut ifr: libc::ifreq = std::mem::zeroed();
        for (d, s) in ifr.ifr_name.iter_mut().zip(b"lo\0") {
            *d = *s as libc::c_char;
        }
        assert_eq!(
            libc::ioctl(fd, libc::SIOCGIFFLAGS as _, &mut ifr),
            0,
            "lo's flags"
        );
        ifr.ifr_ifru.ifru_flags |= libc::IFF_UP as libc::c_short;
        assert_eq!(libc::ioctl(fd, libc::SIOCSIFFLAGS as _, &ifr), 0, "lo up");
        libc::close(fd);
    }
}

#[test]
fn wp_secdef_uconsole_hub_requires_tls_by_default() {
    if std::env::var_os(INNER).is_some() {
        return;
    }
    let can = Command::new("unshare")
        .args(["-rn", "true"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !can {
        eprintln!("wp_secdef: no network namespace here (unshare -rn); the default not run");
        return;
    }
    let out = Command::new("unshare")
        .arg("-rn")
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "wp_secdef_inner_the_default_in_a_namespace",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(INNER, "1")
        .output()
        .unwrap();
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "the inner run: {text}");
    assert!(text.contains("1 passed"), "the inner test ran: {text}");
}

#[test]
#[ignore = "run by wp_secdef_uconsole_hub_requires_tls_by_default, in its own network namespace"]
fn wp_secdef_inner_the_default_in_a_namespace() {
    if std::env::var_os(INNER).is_none() {
        return;
    }
    loopback_up();
    let home = scratch("default");
    let store = home.join("store");

    // No --tls, no --loopback: the network default.
    let hub = Hub::start(&home, &store, &["--pair"]);
    let mut before = Vec::new();
    let status = hub.next_noting("status", &mut before);
    assert_eq!(status["tls"], "require", "the default: {status}");
    assert_eq!(status["upstreamCompat"], false, "{status}");
    let fingerprint = status["tlsFingerprint"].as_str().unwrap().to_owned();
    assert!(fingerprint.starts_with("SHA256:"), "{fingerprint}");
    assert!(
        before.iter().all(|v| v["event"] != "securityDowngraded"),
        "nothing downgraded: {before:?}"
    );
    let port = u16::try_from(status["port"].as_u64().unwrap()).unwrap();
    let pairing = hub.next("pairing");
    assert_eq!(pairing["fingerprint"], fingerprint.as_str());
    let pin = pairing["pin"].as_str().unwrap().to_owned();

    // Plaintext HTTP: 426, with what to open and the key to compare.
    let answer = String::from_utf8_lossy(&plain(
        port,
        &format!(
            "GET /dspi/v1/info HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        ),
    ))
    .into_owned();
    assert!(answer.starts_with("HTTP/1.1 426"), "{answer}");
    assert!(answer.contains(&fingerprint), "{answer}");

    // A plaintext WebSocket: upgraded, then closed 4003 with the reason.
    let (head, close) = plain_upgrade(port);
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    let (code, reason) = close.expect("a close frame");
    assert_eq!(code, 4003, "{reason}");
    assert!(reason.starts_with("tls_required"), "{reason}");

    // Our client pairs over TLS with the PIN, as any user would.
    let client = scratch("default-client");
    let out = in_home(DSPI, &client)
        .args([
            "--link",
            &format!("127.0.0.1:{port}"),
            "--pin",
            &pin,
            "list",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "pairing over TLS: {stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains(DEMO), "{stdout}");

    if have("curl") {
        let (body, code) = curl(&["-k", &format!("https://127.0.0.1:{port}/dspi/v1/info")]);
        assert_eq!(code, 200, "{body}");
        let info: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(info["tls"], true, "{info}");
        assert_eq!(info["upstream_compat"], false, "{info}");
    }
    hub.stop();
}

/// An API key minted before TLS has no `plaintext_ok`: in `on` it works over https
/// only, and with upstream compatibility over plaintext too. Loopback, so no
/// namespace is needed.
#[test]
fn wp_secdef_an_api_key_from_a_pre_tls_store() {
    if std::env::var_os(INNER).is_some() {
        return;
    }
    if !have("curl") {
        eprintln!("wp_secdef: no curl here; the API key path not run");
        return;
    }
    let home = scratch("api-key");
    let store = home.join("store");

    // A hub as it ran before TLS (loopback is `off` by default): no key in its store.
    let hub = Hub::start(&home, &store, &["--loopback"]);
    assert_eq!(hub.next("status")["tls"], "off");
    hub.stop();
    assert!(!store.join("tls").exists(), "no TLS key before TLS");
    let out = in_home(DSPI, &home)
        .args([
            "hub",
            "api-key",
            "new",
            "Old script",
            "viewer",
            "--json",
            "--store",
        ])
        .arg(&store)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let minted: Value = serde_json::from_slice(&out.stdout).unwrap();
    let key = minted["key"].as_str().unwrap().to_owned();
    let bearer = format!("Authorization: Bearer {key}");

    // Started with TLS on: the key is made, and the old API key is https only.
    let hub = Hub::start(&home, &store, &["--loopback", "--tls", "on", "--rest"]);
    let status = hub.next("status");
    assert_eq!(status["tls"], "on", "{status}");
    let port = status["port"].as_u64().unwrap();
    assert!(
        store.join("tls").join("key.pem").is_file(),
        "a key at the first TLS start"
    );
    let (body, code) = curl(&[
        "-H",
        &bearer,
        &format!("http://127.0.0.1:{port}/api/v1/health"),
    ]);
    assert_eq!(code, 426, "plaintext: {body}");
    assert!(body.contains("tls_required"), "{body}");
    let out = in_home(DSPI, &home)
        .args(["hub", "tls", "fingerprint", "--json", "--store"])
        .arg(&store)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let fp: Value = serde_json::from_slice(&out.stdout).unwrap();
    let pin = fp["pinnedpubkey"].as_str().unwrap().to_owned();
    let (body, code) = curl(&[
        "-k",
        "--pinnedpubkey",
        &pin,
        "-H",
        &bearer,
        &format!("https://127.0.0.1:{port}/api/v1/health"),
    ]);
    assert_eq!(code, 200, "https: {body}");
    hub.stop();

    // With upstream compatibility every key works over plaintext too (on a network
    // address the hub starts with the downgrade warning; here it is loopback).
    let hub = Hub::start(
        &home,
        &store,
        &["--loopback", "--upstream-compat", "--rest"],
    );
    let status = hub.next("status");
    assert_eq!(status["upstreamCompat"], true, "{status}");
    let port = status["port"].as_u64().unwrap();
    let (body, code) = curl(&[
        "-H",
        &bearer,
        &format!("http://127.0.0.1:{port}/api/v1/health"),
    ]);
    assert_eq!(code, 200, "compat, plaintext: {body}");
    hub.stop();
}
