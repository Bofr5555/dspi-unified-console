//! WP LK3: our client (the core over Link, in Android's order) against the
//! emulated ESP32 bridge of tools/esp-rig: time to the first live meter, meter
//! freezes, disconnects and re-reads while the meters run.
//!
//! A measurement, not a check: ignored by default, run by tools/esp-rig/rig.sh
//! inside the rig's network namespace. Only uses API that the pre-LK2 core
//! (4cdefa24) has too, so the same file builds both sides of the comparison.
//!
//!   LK3_URL=ws://10.77.0.2:11915/dspi/v1 LK3_MODE=android cargo test -p dspi-core \
//!       --features link --test wp_lk3_rig -- --ignored --nocapture
//!
//! - `LK3_MODE`: `seq` (0.12.0: the meter loop starts after every start-up
//!   read) or `android` (LK2: the meter loop runs from `link_select_device`).
//! - `LK3_SECONDS` (60): how long the meters run after the first one.
//! - `LK3_REREAD_S` (0 = never): a `fetch_all` re-read every this many seconds
//!   while the meters run, on its own thread as Android's FFI thread.
//! - `LK3_CLIENTS` (1): independent cores (apps) at once.
//! - `LK3_CS_CHAIN` (70): serial reads standing in for `cs_fetch` (LK2).
//! - `LK3_TRACE=1`: print what `fetch_status` returned once a second.
//! - `LK3_NO_READS=1`: no start-up reads at all, only the meters (to tell the
//!   reads' cost on the meters apart).

#![cfg(feature = "link")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use dspi_core::{DspiCore, LinkPhase, RtaRequest};

/// Android's meter period (`STATUS_POLL_MS`).
const STATUS_POLL: Duration = Duration::from_millis(60);
/// No live meter for longer than this is a freeze.
const FREEZE: Duration = Duration::from_secs(1);
/// `REQ_GET_PLATFORM`: a plain read every firmware answers.
const PLAIN_READ: u8 = 0x7F;
/// Give up on the first meter after this long.
const LIMIT: Duration = Duration::from_secs(300);

fn env_num(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn secs(start: Instant) -> f64 {
    start.elapsed().as_secs_f64()
}

/// `DspiBridgeImpl.deviceSelected` and the `deviceChangedHooks`, in order.
fn startup_reads(core: &DspiCore, chain: u64) {
    if std::env::var("LK3_NO_READS").is_ok_and(|v| v == "1") {
        return;
    }
    let _ = core.fetch_all();
    core.rta_start();
    let _ = core.fetch_crossfeed_pair_map();
    let _ = core.fetch_crossfeed_pair_map();
    let _ = core.siggen_fetch_caps();
    let _ = core.siggen_get_config();
    let _ = core.siggen_get_status();
    let _ = core.cs_fetch();
    for _ in 0..chain {
        let _ = core.raw_get(PLAIN_READ, 0, 7);
    }
    core.rta_set_request(RtaRequest::default());
}

#[derive(Default)]
struct Run {
    first_meter: Option<f64>,
    after_select: Option<f64>,
    select_at: Option<f64>,
    reads_done: Option<f64>,
    freezes: u32,
    longest_gap: Duration,
    drops: u32,
    rebinds: u32,
    reread: u32,
    reread_failed: u32,
    reread_max: Duration,
    reads_failed: u32,
    failed: Option<String>,
}

fn connect(core: &DspiCore, url: &str, name: &str) -> Result<(), String> {
    core.link_connect(
        url.to_owned(),
        name.to_owned(),
        "wp-lk3".into(),
        String::new(),
        String::new(),
    )
    .map_err(|e| format!("connect: {e}"))?;
    let until = Instant::now() + Duration::from_secs(30);
    while core.link_devices().is_empty() {
        if Instant::now() > until {
            return Err("no device in 30 s".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let d = core.link_devices().remove(0);
    core.link_select_device(d.handle, d.serial)
        .map_err(|e| format!("select: {e}"))
}

/// One app: connect, start-up reads and meters in `mode`'s order, then the
/// meters for `seconds` with re-reads, counting freezes and drops.
fn one_client(id: usize, url: &str, android: bool, seconds: u64, reread_s: u64, chain: u64) -> Run {
    let mut run = Run::default();
    let core = DspiCore::new();
    let start = Instant::now();
    if let Err(e) = connect(&core, url, &format!("lk3 rust c{id}")) {
        run.failed = Some(e);
        return run;
    }
    let selected = Instant::now();
    run.select_at = Some(secs(start));
    println!("LK3 c{id} t={:.2}s selected", secs(start));
    let reads_done = AtomicBool::new(false);
    let reads_done_at = std::sync::Mutex::new(None::<f64>);
    let stop = AtomicBool::new(false);
    let mut reread_times: Vec<(Duration, bool)> = Vec::new();
    std::thread::scope(|scope| {
        // The FFI thread: start-up reads (and, in seq, nothing else first),
        // then the periodic re-reads.
        let ffi = scope.spawn(|| {
            let mut times = Vec::new();
            startup_reads(&core, chain);
            reads_done.store(true, Ordering::SeqCst);
            if let Ok(mut at) = reads_done_at.lock() {
                *at = Some(secs(start));
            }
            println!("LK3 c{id} t={:.2}s start-up reads done", secs(start));
            if reread_s == 0 {
                return times;
            }
            let mut next = Instant::now() + Duration::from_secs(reread_s);
            while !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(50));
                if Instant::now() < next {
                    continue;
                }
                let t = Instant::now();
                let ok = core.fetch_all().is_ok();
                times.push((t.elapsed(), ok));
                next = Instant::now() + Duration::from_secs(reread_s);
            }
            times
        });
        if !android {
            while !reads_done.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        // The link watcher: `link_status` takes the core lock, so it runs on a
        // thread of its own (Android asks `isConnected` on its FFI thread
        // without waiting); it may block behind the start-up reads.
        let watcher = scope.spawn(|| {
            let (mut drops, mut rebinds, mut was_bound) = (0u32, 0u32, true);
            while !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(200));
                let st = core.link_status();
                let bound = st.phase == LinkPhase::Bound;
                if was_bound && !bound {
                    drops += 1;
                    println!(
                        "LK3 c{id} t={:.2}s session lost: {:?} code={} reason={}",
                        secs(start),
                        st.phase,
                        st.close_code,
                        st.close_reason
                    );
                }
                if !was_bound && bound {
                    rebinds += 1;
                    println!("LK3 c{id} t={:.2}s bound again", secs(start));
                }
                was_bound = bound;
            }
            (drops, rebinds)
        });
        // The meter loop: only `fetch_status`, as Android's poll.
        let mut last_live: Option<Instant> = None;
        let mut end = None::<Instant>;
        let trace = std::env::var("LK3_TRACE").is_ok_and(|v| v == "1");
        let mut next_trace = Instant::now();
        loop {
            std::thread::sleep(STATUS_POLL);
            let now = Instant::now();
            if run.first_meter.is_none() && start.elapsed() > LIMIT {
                run.failed = Some(format!("no live meter within {LIMIT:?}"));
                break;
            }
            if end.is_some_and(|e| now >= e) {
                break;
            }
            let got = core.fetch_status();
            if trace && now >= next_trace {
                next_trace = now + Duration::from_secs(1);
                println!(
                    "LK3 c{id} t={:.2}s trace fetch_status={}",
                    secs(start),
                    match &got {
                        Ok(s) => format!("ok stale={} age_ms={}", s.stale, s.age_ms),
                        Err(e) => format!("err {e}"),
                    }
                );
            }
            if got.is_ok_and(|s| !s.stale) {
                if run.first_meter.is_none() {
                    run.first_meter = Some(secs(start));
                    run.after_select = Some(selected.elapsed().as_secs_f64());
                    println!(
                        "LK3 c{id} t={:.2}s first live meter (start-up reads still running: {})",
                        secs(start),
                        !reads_done.load(Ordering::SeqCst)
                    );
                    end = Some(now + Duration::from_secs(seconds));
                }
                if let Some(prev) = last_live {
                    let gap = now - prev;
                    run.longest_gap = run.longest_gap.max(gap);
                    if gap > FREEZE {
                        run.freezes += 1;
                        println!(
                            "LK3 c{id} t={:.2}s meter freeze {:.1}s",
                            secs(start),
                            gap.as_secs_f64()
                        );
                    }
                }
                last_live = Some(now);
            }
        }
        stop.store(true, Ordering::SeqCst);
        reread_times = ffi.join().unwrap_or_default();
        (run.drops, run.rebinds) = watcher.join().unwrap_or_default();
    });
    run.reads_done = reads_done_at.lock().ok().and_then(|at| *at);
    for (d, ok) in reread_times {
        run.reread += 1;
        if ok {
            run.reread_max = run.reread_max.max(d);
        } else {
            run.reread_failed += 1;
        }
    }
    if core.raw_get(PLAIN_READ, 0, 7).is_err() {
        run.reads_failed += 1;
    }
    core.link_close();
    run
}

#[test]
#[ignore = "measurement against the emulated bridge (WP LK3, tools/esp-rig)"]
fn wp_lk3_meters_against_the_emulated_bridge() {
    let url = std::env::var("LK3_URL").unwrap_or_else(|_| "ws://10.77.0.2:11915/dspi/v1".into());
    let android = std::env::var("LK3_MODE").is_ok_and(|m| m == "android");
    let seconds = env_num("LK3_SECONDS", 60);
    let reread_s = env_num("LK3_REREAD_S", 0);
    let clients = env_num("LK3_CLIENTS", 1).max(1) as usize;
    let chain = env_num("LK3_CS_CHAIN", 70);
    let runs: Vec<Run> = std::thread::scope(|scope| {
        let handles: Vec<_> = (1..=clients)
            .map(|id| {
                let url = url.clone();
                scope.spawn(move || one_client(id, &url, android, seconds, reread_s, chain))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect()
    });
    for (i, r) in runs.iter().enumerate() {
        let f = |v: Option<f64>| v.map_or("never".to_owned(), |v| format!("{v:.2}"));
        println!(
            "LK3 summary client=rust mode={} c={} first_meter={} after_select={} select={} reads_done={} \
             freezes={} longest_gap_s={:.1} drops={} rebinds={} rereads={} reread_failed={} reread_max_s={:.2} \
             final_read_failed={} failed={}",
            if android { "android" } else { "seq" },
            i + 1,
            f(r.first_meter),
            f(r.after_select),
            f(r.select_at),
            f(r.reads_done),
            r.freezes,
            r.longest_gap.as_secs_f64(),
            r.drops,
            r.rebinds,
            r.reread,
            r.reread_failed,
            r.reread_max.as_secs_f64(),
            r.reads_failed,
            r.failed.as_deref().unwrap_or("no")
        );
    }
}
