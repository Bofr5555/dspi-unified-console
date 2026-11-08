//! WP LK1: how long after connecting over DSPi Link the first live meter
//! reading arrives, replaying what the Android app (0.12.0) does on its one
//! FFI thread before its meter loop can run (`DspiBridgeImpl.deviceSelected`
//! and the `deviceChangedHooks`), against the fake ESP32 bridge.
//!
//! A measurement, not a check: ignored by default. Run it inside a network
//! namespace with `tc qdisc ... netem` on `lo` to model a lossy Wi-Fi link
//! (see `~/.cache/dspi-build/work/wp-LK1/netem.sh`):
//!
//!   cargo test -p dspi-core --features link --test wp_lk1_meter_start -- --ignored --nocapture
//!
//! It prints one `LK1 ...` line per phase and the number of tunnelled
//! commands the hub saw before the meters started.

#![cfg(feature = "link")]

use std::time::{Duration, Instant};

use dspi_core::{DspiCore, RtaRequest};
use dspi_link::fakehub::{Auth, FakeHub, HubConfig};

/// The Android meter loop's period (`STATUS_POLL_MS`).
const STATUS_POLL: Duration = Duration::from_millis(60);
/// Give up after this long without a live reading.
const LIMIT: Duration = Duration::from_secs(300);

fn phase(start: Instant, name: &str, hub: &FakeHub) {
    println!(
        "LK1 {name:<28} t={:>7.2}s commands={}",
        start.elapsed().as_secs_f64(),
        hub.log().len()
    );
}

#[test]
#[ignore = "measurement; run under netem (WP LK1)"]
fn wp_lk1_time_from_connect_to_first_live_meter() {
    let hub = FakeHub::start(HubConfig {
        auth: Auth::None,
        ..HubConfig::bridge()
    })
    .unwrap();
    let core = DspiCore::new();
    let start = Instant::now();
    core.link_connect(
        hub.url(),
        "wp-lk1".into(),
        "test".into(),
        String::new(),
        String::new(),
    )
    .unwrap();
    phase(start, "link_connect", &hub);
    while core.link_devices().is_empty() {
        std::thread::sleep(Duration::from_millis(10));
    }
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial).unwrap();
    phase(start, "link_select_device", &hub);

    // DspiBridgeImpl.deviceSelected, in order, then `connected = true`
    let _ = core.fetch_all();
    phase(start, "fetch_all", &hub);
    core.rta_start();
    let _ = core.fetch_crossfeed_pair_map();
    let _ = core.fetch_crossfeed_pair_map();
    phase(start, "rta_start + pair map x2", &hub);
    // deviceChangedHooks: siggen, control surfaces, the analyser's request
    let _ = core.siggen_fetch_caps();
    let _ = core.siggen_get_config();
    let _ = core.siggen_get_status();
    let _ = core.cs_fetch();
    core.rta_set_request(RtaRequest::default());
    phase(start, "hooks (siggen, cs_fetch)", &hub);

    // The meter loop: one fetch_status 60 ms after the last one ended
    let loop_start = Instant::now();
    let mut calls = 0u32;
    loop {
        std::thread::sleep(STATUS_POLL);
        calls += 1;
        match core.fetch_status() {
            Ok(s) if !s.stale => break,
            _ if start.elapsed() > LIMIT => panic!("no live meter within {LIMIT:?}"),
            _ => {}
        }
    }
    println!(
        "LK1 {:<28} t={:>7.2}s commands={} (meter loop {:.2}s, {calls} calls)",
        "first live meter",
        start.elapsed().as_secs_f64(),
        hub.log().len(),
        loop_start.elapsed().as_secs_f64()
    );
    let ops: Vec<String> = hub
        .log()
        .iter()
        .map(|(_, dir, req, val)| format!("{}{req:02X}/{val}", if *dir == 0 { "S" } else { "G" }))
        .collect();
    println!("LK1 commands: {}", ops.join(" "));
    core.link_close();
    hub.stop();
}
