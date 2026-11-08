//! WP LK2: time from connecting over DSPi Link to the first live meter, with
//! Android's start-up reads including a `cs_fetch`-sized chain, against the
//! fake ESP32 bridge.
//!
//! A measurement, not a check: ignored by default. Run it under netem on `lo`
//! in a private network namespace (`~/.cache/dspi-build/work/wp-LK2/netem.sh`):
//!
//!   LK2_MODE=android cargo test -p dspi-core --features link \
//!       --test wp_lk2_meter_start -- --ignored --nocapture
//!
//! `LK2_MODE` picks the app's order:
//! - `seq` (default): Android 0.12.0, everything on one FFI thread; the meter
//!   loop starts after `fetch_all`, the probes and the device-changed hooks;
//! - `android`: WP LK2's order: the meter loop runs on its own thread from the
//!   moment `link_select_device` returns, while the FFI thread does the same
//!   reads as `seq` (the hooks at once, not deferred, as the worst case).
//!
//! The fake device has no control surfaces, so `cs_fetch` costs nothing;
//! [`CS_CHAIN`] serial reads stand in for it (`cs.rs`: about seventy round trips
//! on firmware that has them).

#![cfg(feature = "link")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use dspi_core::{DspiCore, RtaRequest};
use dspi_link::fakehub::{Auth, FakeHub, HubConfig};

/// The Android meter loop's period (`STATUS_POLL_MS`).
const STATUS_POLL: Duration = Duration::from_millis(60);
/// Give up after this long without a live reading.
const LIMIT: Duration = Duration::from_secs(300);
/// Serial reads standing in for `cs_fetch` on firmware with control surfaces.
const CS_CHAIN: usize = 70;
/// `REQ_GET_PLATFORM`: a plain read the fake device answers.
const PLAIN_READ: u8 = 0x7F;

fn phase(start: Instant, name: &str, hub: &FakeHub) {
    println!(
        "LK2 {name:<28} t={:>7.2}s commands={}",
        start.elapsed().as_secs_f64(),
        hub.log().len()
    );
}

/// `DspiBridgeImpl.deviceSelected` and the `deviceChangedHooks`, in order.
fn startup_reads(core: &DspiCore, start: Instant, hub: &FakeHub) {
    let _ = core.fetch_all();
    phase(start, "fetch_all", hub);
    core.rta_start();
    let _ = core.fetch_crossfeed_pair_map();
    let _ = core.fetch_crossfeed_pair_map();
    phase(start, "rta_start + pair map x2", hub);
    let _ = core.siggen_fetch_caps();
    let _ = core.siggen_get_config();
    let _ = core.siggen_get_status();
    let _ = core.cs_fetch();
    for _ in 0..CS_CHAIN {
        let _ = core.raw_get(PLAIN_READ, 0, 7);
    }
    core.rta_set_request(RtaRequest::default());
    phase(start, "hooks (siggen, cs chain)", hub);
}

/// One `fetch_status` per [`STATUS_POLL`] until a live reading: when it came
/// and how many calls it took.
fn meter_loop(core: &DspiCore, start: Instant) -> (Instant, u32) {
    let mut calls = 0u32;
    loop {
        std::thread::sleep(STATUS_POLL);
        calls += 1;
        match core.fetch_status() {
            Ok(s) if !s.stale => return (Instant::now(), calls),
            _ if start.elapsed() > LIMIT => panic!("no live meter within {LIMIT:?}"),
            _ => {}
        }
    }
}

#[test]
#[ignore = "measurement; run under netem (WP LK2)"]
fn wp_lk2_time_from_connect_to_first_live_meter() {
    let android = std::env::var("LK2_MODE").is_ok_and(|m| m == "android");
    let hub = FakeHub::start(HubConfig {
        auth: Auth::None,
        ..HubConfig::bridge()
    })
    .unwrap();
    let core = DspiCore::new();
    let start = Instant::now();
    core.link_connect(
        hub.url(),
        "wp-lk2".into(),
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
    let selected = Instant::now();

    let (first, calls) = if android {
        let done = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                startup_reads(&core, start, &hub);
                done.store(true, Ordering::SeqCst);
            });
            let first = meter_loop(&core, start);
            println!(
                "LK2 {:<28} t={:>7.2}s commands={} (start-up reads still running: {})",
                "first live meter",
                first.0.duration_since(start).as_secs_f64(),
                hub.log().len(),
                !done.load(Ordering::SeqCst)
            );
            first
        })
    } else {
        startup_reads(&core, start, &hub);
        let first = meter_loop(&core, start);
        println!(
            "LK2 {:<28} t={:>7.2}s commands={}",
            "first live meter",
            first.0.duration_since(start).as_secs_f64(),
            hub.log().len(),
        );
        first
    };
    println!(
        "LK2 summary mode={} first_meter={:.2}s after_select={:.2}s calls={calls} total_reads_done={:.2}s",
        if android { "android" } else { "seq" },
        first.duration_since(start).as_secs_f64(),
        first.duration_since(selected).as_secs_f64(),
        start.elapsed().as_secs_f64()
    );
    core.link_close();
    hub.stop();
}

/// Connect and bind the fake bridge's device; `None` when that failed (the
/// failure is printed, and counts as a disconnect).
fn bound(hub: &FakeHub, core: &DspiCore) -> Option<()> {
    if let Err(e) = core.link_connect(
        hub.url(),
        "wp-lk2".into(),
        "test".into(),
        String::new(),
        String::new(),
    ) {
        println!("LK2 connect failed: {e}");
        return None;
    }
    let until = Instant::now() + Duration::from_secs(30);
    while core.link_devices().is_empty() {
        if Instant::now() > until {
            println!("LK2 no device list");
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let d = core.link_devices().remove(0);
    if let Err(e) = core.link_select_device(d.handle, d.serial) {
        println!("LK2 select failed: {e}");
        return None;
    }
    Some(())
}

/// Seconds from the env var `name`, or `default`.
fn secs(name: &str, default: u64) -> Duration {
    Duration::from_secs(
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default),
    )
}

/// LK1-c: on a lossy link, how often the session drops (leaves `Bound`) while
/// meters run and the app reads now and then. `LK2_SOAK_S` seconds (60).
#[test]
#[ignore = "measurement; run under netem (WP LK2)"]
fn wp_lk2_soak_disconnects() {
    let hub = FakeHub::start(HubConfig {
        auth: Auth::None,
        ..HubConfig::bridge()
    })
    .unwrap();
    let core = DspiCore::new();
    if bound(&hub, &core).is_none() {
        println!("LK2 soak drops=1 reads_failed=0 reads=0 (start failed)");
        return;
    }
    let _ = core.fetch_all();
    let soak = secs("LK2_SOAK_S", 60);
    let start = Instant::now();
    let (mut drops, mut reads, mut failed) = (0u32, 0u32, 0u32);
    let mut was_bound = true;
    let mut next_read = start;
    while start.elapsed() < soak {
        std::thread::sleep(STATUS_POLL);
        let _ = core.fetch_status();
        let is_bound = core.link_status().phase == dspi_core::LinkPhase::Bound;
        if was_bound && !is_bound {
            drops += 1;
            println!(
                "LK2 drop at t={:.1}s: {:?}",
                start.elapsed().as_secs_f64(),
                core.link_status().close_reason
            );
        }
        was_bound = is_bound;
        if is_bound && Instant::now() >= next_read {
            next_read = Instant::now() + Duration::from_millis(500);
            reads += 1;
            if core.raw_get(PLAIN_READ, 0, 7).is_err() {
                failed += 1;
            }
        }
    }
    println!("LK2 soak drops={drops} reads_failed={failed} reads={reads}");
    core.link_close();
    hub.stop();
}

/// LK1-c: a hub that goes silent (Wi-Fi or power gone, no FIN: the fake hub
/// stalls) with meters running and a read every 2 s: how long until the
/// session is given up (`Bound` left).
#[test]
#[ignore = "measurement; run under netem (WP LK2)"]
fn wp_lk2_time_to_notice_a_dead_hub() {
    let hub = FakeHub::start(HubConfig {
        auth: Auth::None,
        ..HubConfig::bridge()
    })
    .unwrap();
    let core = DspiCore::new();
    if bound(&hub, &core).is_none() {
        println!("LK2 dead-hub start failed");
        return;
    }
    let _ = core.fetch_all();
    // Meters live first
    let _ = meter_loop(&core, Instant::now());
    hub.stall(true);
    let dead = Instant::now();
    let mut next_read = dead;
    while core.link_status().phase == dspi_core::LinkPhase::Bound {
        if dead.elapsed() > Duration::from_secs(120) {
            println!("LK2 dead hub not noticed in 120 s");
            break;
        }
        std::thread::sleep(STATUS_POLL);
        let _ = core.fetch_status();
        if Instant::now() >= next_read {
            next_read = Instant::now() + Duration::from_secs(2);
            let _ = core.raw_get(PLAIN_READ, 0, 7);
        }
    }
    println!(
        "LK2 dead hub noticed after {:.1}s ({:?})",
        dead.elapsed().as_secs_f64(),
        core.link_status().close_reason
    );
    core.link_close();
    hub.stop();
}
