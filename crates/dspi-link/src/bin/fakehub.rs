//! `dspi-link-fakehub`: a fake DSPi Link hub on the LAN, for trying the app
//! without a Mac or an ESP32 bridge (docs/LINK-HIL-CHECKLIST.md).
//!
//!   dspi-link-fakehub [--bind 0.0.0.0:11915] [--auth none|window:PIN|fixed:PIN]
//!                     [--bridge] [--role viewer|control|admin] [--name NAME]
//!                     [--max-sessions N] [--no-board] [--rta]
//!
//! Faults, one command per line on stdin:
//!   stall | unstall          the network goes dark, or comes back (TCP stays up)
//!   drop                     cut every connection without a close frame
//!   restart                  reboot: connections drop; id and tokens survive
//!   reset ID                 factory reset to hub id ID: tokens gone
//!   token CODE | token ok    answer auth.token with err CODE, or normally
//!   list-fail CODE [N]       the next N (1) device.list answer err CODE
//!   list-silent [N]          the next N (1) device.list are never answered
//!   refuse STATUS | refuse ok   answer the upgrade with HTTP STATUS, or normally
//!   hold | release           hold replies back, then deliver them late
//!   dup on | dup off         send every frame twice
//!   board on | board off     attach or remove the board
//!   window                   open a new PIN window (a window PIN pairs one client)
//!
//! Advertise it for discovery with avahi on the same host, e.g.
//!   avahi-publish -s "Fake Hub" _dspi._tcp 11915 v=1 hid=<id> kind=console auth=none n=1
//! (the Android emulator cannot see mDNS: add 10.0.2.2:11915 by address).

use std::io::BufRead;
use std::time::Duration;

use dspi_link::fakehub::{Auth, DeviceListFault, FakeHub, HubConfig};

/// One stdin fault command; false when it is not one.
fn fault(hub: &FakeHub, line: &str) -> bool {
    let words: Vec<&str> = line.split_whitespace().collect();
    let times = |w: Option<&&str>| w.and_then(|n| n.parse().ok()).unwrap_or(1);
    match words.as_slice() {
        ["stall"] => hub.stall(true),
        ["unstall"] => hub.stall(false),
        ["drop"] => hub.drop_connections(),
        ["restart"] => hub.restart(),
        ["reset", id] => hub.factory_reset(id),
        ["token", "ok"] => hub.fail_tokens_with(None),
        ["token", code] => hub.fail_tokens_with(Some(code)),
        ["list-fail", code, rest @ ..] => {
            hub.fail_device_list(DeviceListFault::Error((*code).into()), times(rest.first()))
        }
        ["list-silent", rest @ ..] => hub.fail_device_list(DeviceListFault::Silent, times(rest.first())),
        ["refuse", "ok"] => hub.refuse_handshake(None),
        ["refuse", status] => match status.parse() {
            Ok(s) => hub.refuse_handshake(Some(s)),
            Err(_) => return false,
        },
        ["hold"] => hub.hold_replies(),
        ["release"] => hub.release_replies(),
        ["dup", "on"] => hub.duplicate_frames(true),
        ["dup", "off"] => hub.duplicate_frames(false),
        ["board", "on"] => hub.set_board_present(true),
        ["board", "off"] => hub.set_board_present(false),
        ["window"] => hub.reopen_window(),
        _ => return false,
    }
    true
}

fn main() {
    let mut cfg = HubConfig { bind: "0.0.0.0:11915".parse().unwrap(), ..HubConfig::default() };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_else(|| panic!("{a} needs a value"));
        match a.as_str() {
            "--bind" => cfg.bind = val().parse().expect("--bind host:port"),
            "--bridge" => {
                let bind = cfg.bind;
                cfg = HubConfig { bind, ..HubConfig::bridge() };
            }
            "--auth" => {
                let v = val();
                cfg.auth = match v.split_once(':') {
                    Some(("window", p)) => Auth::Window(p.into()),
                    Some(("fixed", p)) => Auth::Fixed(p.into()),
                    _ if v == "none" => Auth::None,
                    _ => panic!("--auth none|window:PIN|fixed:PIN"),
                }
            }
            "--role" => cfg.pin_role = val(),
            "--name" => cfg.name = val(),
            "--max-sessions" => cfg.max_sessions = Some(val().parse().expect("--max-sessions N")),
            "--no-board" => cfg.board_attached = false,
            // -- WP QD8 -- The device has an analyser.
            "--rta" => cfg.rta = true,
            other => panic!("unknown argument {other}"),
        }
    }
    let hub = FakeHub::start(cfg.clone()).expect("bind");
    println!(
        "fake DSPi Link hub \"{}\" ({}) on {} id {} auth {:?}",
        cfg.name,
        cfg.kind,
        hub.addr(),
        cfg.hub_id,
        cfg.auth
    );
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        if !line.trim().is_empty() && !fault(&hub, &line) {
            eprintln!("unknown fault: {line}");
        }
    }
    // stdin closed (run in the background): keep serving
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
