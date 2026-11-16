//! WP P3WRO: the rollout bench of `uconsole --demo hub --rollout --with-rollout-bench`
//! (the web console's e2e tests drive it): the option's rules, and a rollout through the
//! real hub with the bench: a 1.1.N image comes back with the settings kept; any other
//! version fails the verify with the settings changed. No USB, no real device.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation
)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use dspi_hub::Role;
use serde_json::{Value, json};

use super::super::{Failure, HubCommand, RunOptions, Running, parse, start};
use super::*;

fn scratch(name: &str) -> PathBuf {
    // Beside the test binary, in the target directory: never on tmpfs.
    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("wp_p3wro")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_options(args: &[&str], demo: bool) -> Result<RunOptions, Failure> {
    match parse(args, demo, Some(PathBuf::from("/store")))? {
        HubCommand::Run(o) => Ok(o),
        other => panic!("{args:?}: {other:?}"),
    }
}

#[test]
fn wp_p3wro_bench_versions() {
    assert_eq!(patch_of("1.1.7"), Some(7));
    assert_eq!(patch_of("1.1.15"), Some(15));
    assert_eq!(patch_of("1.1.16"), None);
    assert_eq!(patch_of("2.0.0"), None);
    assert_eq!(patch_of(""), None);
}

#[test]
fn wp_p3wro_the_bench_is_a_demo_rollout_test_option() {
    let ok = run_options(&["--tls", "on", "--rollout", "--with-rollout-bench"], true).unwrap();
    assert!(ok.rollout_bench);
    assert!(
        !run_options(&["--tls", "on", "--rollout"], true)
            .unwrap()
            .rollout_bench
    );
    for (args, demo) in [
        (
            &["--tls", "on", "--rollout", "--with-rollout-bench"][..],
            false,
        ),
        (&["--with-rollout-bench"][..], true),
    ] {
        assert!(
            matches!(run_options(args, demo), Err(Failure::Usage(_))),
            "{args:?} demo {demo}"
        );
    }
    // It is for running a hub only.
    assert!(matches!(
        parse(
            &["clients", "--with-rollout-bench"],
            true,
            Some("/s".into())
        ),
        Err(Failure::Usage(_))
    ));
}

// ── An image (laid out as the firmware guard's own tests build them) ────────

const XIP: u32 = 0x1000_0000;

fn image(version: &str, stamp: &str) -> Vec<u8> {
    let entries: [(u16, u32, &str); 3] = [
        (0x5052, 0x0203_1C86, "DSPi"),
        (0x5052, 0x11A9_BC3A, version),
        (0x5344, 0x0B7A_4C15, stamp),
    ];
    let mut f = vec![0u8; 0x2000];
    let put =
        |f: &mut Vec<u8>, off: usize, v: u32| f[off..off + 4].copy_from_slice(&v.to_le_bytes());
    let a = |off: usize| XIP + off as u32;
    put(&mut f, 0x100, 0x7188_EBF2);
    put(&mut f, 0x104, a(0x200));
    put(&mut f, 0x108, a(0x200 + 4 * entries.len()));
    put(&mut f, 0x10C, a(0x180));
    put(&mut f, 0x110, 0xE71A_A390);
    for (i, (tag, id, text)) in entries.iter().enumerate() {
        let at = 0x400 + 16 * i;
        put(&mut f, 0x200 + 4 * i, a(at));
        f[at..at + 2].copy_from_slice(&6u16.to_le_bytes());
        f[at + 2..at + 4].copy_from_slice(&tag.to_le_bytes());
        put(&mut f, at + 4, *id);
        let s = 0x800 + 64 * i;
        put(&mut f, at + 8, a(s));
        f[s..s + text.len()].copy_from_slice(text.as_bytes());
    }
    let pages: Vec<&[u8]> = f.chunks(256).collect();
    let mut out = Vec::new();
    for (i, p) in pages.iter().enumerate() {
        let mut b = vec![0u8; 512];
        for (off, v) in [
            (0, 0x0A32_4655u32),
            (4, 0x9E5D_5157),
            (8, 0x2000),
            (12, XIP + 256 * i as u32),
            (16, 256),
            (20, i as u32),
            (24, pages.len() as u32),
            (28, 0xE48B_FF59),
            (508, 0x0AB1_6F30),
        ] {
            b[off..off + 4].copy_from_slice(&v.to_le_bytes());
        }
        b[32..32 + p.len()].copy_from_slice(p);
        out.extend(b);
    }
    out
}

// ── Through the hub ─────────────────────────────────────────────────────────

struct Admin<'a> {
    running: &'a Running,
    session: std::sync::Arc<dspi_hub::Session>,
}

impl Admin<'_> {
    fn ask(&self, msg: Value) -> Value {
        let reply = self
            .running
            .handle
            .service_request(&self.session, &msg)
            .expect("the service answers");
        serde_json::from_str(&reply).unwrap()
    }

    fn put(&self, name: &str, bytes: &[u8]) -> Value {
        let b64 = data_encoding::BASE64.encode(bytes);
        self.ask(
            json!({"t": "fw.image.put", "id": 1, "name": name, "seq": 0, "data": b64,
            "last": true}),
        )
    }

    /// `fw.rollout.start`; a `busy` in the moment after the last rollout's record ended
    /// (the hub frees its slot just after announcing the end) is asked again.
    fn start(&self, msg: Value) -> Value {
        let begun = Instant::now();
        loop {
            let v = self.ask(msg.clone());
            if v["code"] != "busy" || begun.elapsed() > Duration::from_secs(5) {
                return v;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The rollout once it has ended.
    fn ended(&self, id: &str) -> Value {
        let start = Instant::now();
        loop {
            let v = self.ask(json!({"t": "fw.rollout.status", "id": 2, "rollout": id}));
            let r = v["rollouts"][0].clone();
            if r["state"] != "running" {
                return r;
            }
            assert!(start.elapsed() < Duration::from_secs(90), "{r}");
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[test]
fn wp_p3wro_a_rollout_on_the_bench_keeps_or_resets_the_settings() {
    let dir = scratch("bench");
    let mut o = run_options(
        &[
            "--port",
            "0",
            "--loopback",
            "--no-keep-awake",
            "--tls",
            "on",
            "--rollout",
            "--allow-unverified-firmware",
            "--with-rollout-bench",
        ],
        true,
    )
    .unwrap();
    o.store = dir.join("store");
    let running = start(&o, false).unwrap();
    let admin = Admin {
        running: &running,
        session: running
            .handle
            .open_console_principal(Role::Admin, "P3WRO bench test")
            .unwrap(),
    };
    // The bench DSPi is on the hub, on "USB", beside the demo.
    let list = admin.ask(json!({"t": "inventory.list", "id": 3}));
    let serials: Vec<&str> = list["devices"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["serial"].as_str())
        .collect();
    assert!(serials.contains(&BENCH_SERIAL), "{list}");
    let put = admin.put("dspi-1.1.7.uf2", &image("1.1.7", "1200"));
    assert_eq!(put["t"], "ok", "{put}");
    let put = admin.put("dspi-2.0.0.uf2", &image("2.0.0", "1200"));
    assert_eq!(put["t"], "ok", "{put}");
    // Muted first (a change to its settings), then a good image: back with them kept.
    let mute = admin.ask(json!({"t": "param.set", "id": 4, "serial": BENCH_SERIAL,
        "path": "vol.mute", "value": true}));
    assert_eq!(mute["t"], "ok", "{mute}");
    let v = admin.start(
        json!({"t": "fw.rollout.start", "id": 5, "serial": BENCH_SERIAL,
        "image": "dspi-1.1.7.uf2"}),
    );
    assert_eq!(v["t"], "ok", "{v}");
    let r = admin.ended(v["rollout"]["id"].as_str().unwrap());
    assert_eq!(r["state"], "ok", "{r}");
    let d = &r["devices"][0];
    assert_eq!(d["after"]["fw"], "1.1.7", "{d}");
    assert_eq!(d["after"]["settings"], "unchanged", "{d}");
    assert_eq!(d["after"]["vreg"]["stored_mv"], 1200, "{d}");
    // Another version: back on 1.1.7 with other settings (10 dB lower).
    let v = admin.start(
        json!({"t": "fw.rollout.start", "id": 6, "serial": BENCH_SERIAL,
        "image": "dspi-2.0.0.uf2"}),
    );
    assert_eq!(v["t"], "ok", "{v}");
    let r = admin.ended(v["rollout"]["id"].as_str().unwrap());
    assert_eq!(r["state"], "failed", "{r}");
    let d = &r["devices"][0];
    assert_eq!(d["failed"]["step"], "verify", "{d}");
    assert_eq!(d["after"]["settings"], "changed", "{d}");
    assert!(
        d["before"]["export"]
            .as_str()
            .is_some_and(|e| !e.is_empty()),
        "{d}"
    );
    // The export recalled (as the web's "Recall the export"), the next rollout finds the
    // settings as they were before the botched one.
    let export = d["before"]["export"].as_str().unwrap().to_owned();
    let back = admin.ask(
        json!({"t": "snapshot.recall", "id": 7, "serial": BENCH_SERIAL,
        "snapshot": export}),
    );
    assert_eq!(back["t"], "ok", "{back}");
    let v = admin.start(
        json!({"t": "fw.rollout.start", "id": 8, "serial": BENCH_SERIAL,
        "image": "dspi-1.1.7.uf2", "while_live": true}),
    );
    assert_eq!(v["t"], "ok", "{v}");
    let r = admin.ended(v["rollout"]["id"].as_str().unwrap());
    assert_eq!(r["state"], "ok", "{r}");
    assert_eq!(r["devices"][0]["after"]["settings"], "unchanged", "{r}");
    running.hub.stop();
}
