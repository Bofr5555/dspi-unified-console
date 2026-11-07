//! The hub the TypeScript and Swift clients run against (`run-clients.sh`). Ignored:
//! it serves until told to stop. Talks to the driver through files in `HB7_CONF_DIR`:
//!
//! | File | Written by | What |
//! |---|---|---|
//! | `hub.json` | here | `{"port", "hub_id", "pin"}`, once the hub is up |
//! | `cmd` | the driver | `<n> pin`, `<n> change`, `<n> revoke <client name>`, `<n> stop` |
//! | `ack` | here | `<n> <json>`: the new PIN, `revoked`, or `{}` |
//!
//! `HB7_BIND` (default `127.0.0.1`) and `HB7_PORT` (default any) place it;
//! `HB7_SERVE_SECS` (default 600) ends it if the driver never says stop.

use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use dspi_hub::server::ServerLimits;
use serde_json::{Value, json};

use crate::support::*;

fn write_atomic(dir: &Path, name: &str, text: &str) {
    let tmp = dir.join(format!(".{name}.tmp"));
    fs::write(&tmp, text).unwrap();
    fs::rename(&tmp, dir.join(name)).unwrap();
}

/// One command from the driver, and its answer. `None`: stop.
fn handle(b: &Bench, verb: &str, arg: &str) -> Option<Value> {
    Some(match verb {
        "pin" => json!({ "pin": b.pin() }),
        "change" => {
            b.knob();
            json!({})
        }
        "revoke" => {
            let revoked = b.client_id(arg).is_some_and(|cid| b.net.revoke(cid));
            json!({ "revoked": revoked })
        }
        "stop" => return None,
        other => json!({ "error": format!("unknown command {other}") }),
    })
}

#[test]
#[ignore = "serves other clients until told to stop: run-clients.sh"]
fn conformance_serve_for_other_clients() {
    let var = |name: &str| std::env::var(name).ok();
    let dir = PathBuf::from(var("HB7_CONF_DIR").expect("HB7_CONF_DIR"));
    fs::create_dir_all(&dir).unwrap();
    let _ = fs::remove_file(dir.join("cmd"));
    let _ = fs::remove_file(dir.join("ack"));
    let b = Bench::start(Setup {
        bind: var("HB7_BIND").map_or(LOOPBACK, |a| a.parse::<IpAddr>().unwrap()),
        port: var("HB7_PORT").map_or(0, |p| p.parse().unwrap()),
        // The clients' own timers are the defaults: so are the hub's.
        limits: ServerLimits::default(),
        ..Setup::pin("serve")
    });
    let hello = json!({ "port": b.port(), "hub_id": b.hub.hub_id(), "pin": b.pin() });
    write_atomic(&dir, "hub.json", &hello.to_string());
    println!("serving {} on port {}", b.hub.hub_id(), b.port());

    let secs = var("HB7_SERVE_SECS").map_or(600, |s| s.parse().unwrap());
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        let Ok(text) = fs::read_to_string(dir.join("cmd")) else {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        };
        let _ = fs::remove_file(dir.join("cmd"));
        let mut words = text.trim().splitn(3, ' ');
        let n = words.next().unwrap_or("0");
        let verb = words.next().unwrap_or("");
        let arg = words.next().unwrap_or("");
        println!("command {n}: {verb} {arg}");
        let answer = handle(&b, verb, arg);
        if answer.is_none() {
            b.hub.stop();
        }
        let answer = answer.unwrap_or_else(|| json!({}));
        write_atomic(&dir, "ack", &format!("{n} {answer}"));
        if b.hub.is_stopped() {
            // Long enough for the driver to read the answer before the files go.
            std::thread::sleep(Duration::from_millis(200));
            return;
        }
    }
    b.hub.stop();
    panic!("no stop from the driver in {secs} s");
}
