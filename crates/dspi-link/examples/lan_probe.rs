//! -- WP DISC2 -- Hub discovery against the real network, for measuring what unit tests
//! cannot reach: Windows' DNS-SD service, or the embedded daemon, with real responders.
//! Kept from WP DISC1's probe (the browse and advertise parts) because the Windows source
//! can only be exercised on Windows, by hand or by a VM script.
//!
//! ```text
//! lan_probe browse SECS                     every change of the hub list, timestamped
//! lan_probe first HID CYCLES TIMEOUT_S      fresh browser per cycle: ms to first sight of HID
//! lan_probe cycles N                        N browser starts and stops; what the OS still holds
//! lan_probe adv NAME HID PORT SECS [EVERY]  advertise; every EVERY s bump `n`
//! lan_probe clock                           the wall clock for each line of stdin
//! ```
//! Lines go to stdout as `wall_ms rel_ms event ...`. `RUST_LOG` is honoured on stderr.

use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dspi_link::discovery::{
    BrowseListener, BrowseOptions, BrowseSnapshot, HubAdvert, HubBrowser, LanAdvertiser, LanSource,
};

fn wall() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

struct Tx(mpsc::Sender<BrowseSnapshot>);
impl BrowseListener for Tx {
    fn on_hubs_changed(&self, snapshot: BrowseSnapshot) {
        let _ = self.0.send(snapshot);
    }
}

fn line(t0: Instant, what: &str) {
    println!("{} {} {what}", wall(), t0.elapsed().as_millis());
}

fn describe(s: &BrowseSnapshot) -> String {
    let hubs: Vec<String> =
        s.hubs.iter().map(|h| format!("[{}|{}|n={}|{}]", h.name, h.id, h.device_count, h.url)).collect();
    format!("browsing={} error={:?} hubs={}", s.browsing, s.error, hubs.join(" "))
}

/// `DSPI_GRACE_MS` sets the removal grace, to see when the source heard a goodbye.
fn start(tx: mpsc::Sender<BrowseSnapshot>) -> HubBrowser {
    let mut options = BrowseOptions::default();
    if let Some(ms) = std::env::var("DSPI_GRACE_MS").ok().and_then(|v| v.parse().ok()) {
        options.removal_grace = Duration::from_millis(ms);
    }
    HubBrowser::start(Arc::new(LanSource), Some(Arc::new(Tx(tx))), options).expect("start")
}

/// What the OS still holds for us: dnsapi contexts on Windows, nothing to count elsewhere.
fn held() -> usize {
    #[cfg(windows)]
    return dspi_link::discovery::native_contexts();
    #[cfg(not(windows))]
    0
}

fn browse(secs: u64) {
    let t0 = Instant::now();
    let (tx, rx) = mpsc::channel();
    let mut b = start(tx);
    line(t0, "start");
    let end = t0 + Duration::from_secs(secs);
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        if let Ok(s) = rx.recv_timeout(left) {
            line(t0, &describe(&s));
        }
    }
    b.stop();
    line(t0, "end");
}

fn first(hid: &str, cycles: u32, timeout: u64) {
    let mut times = Vec::new();
    let mut misses = 0;
    for i in 0..cycles {
        let t0 = Instant::now();
        let (tx, rx) = mpsc::channel();
        let mut b = start(tx);
        let end = t0 + Duration::from_secs(timeout);
        let mut found = None;
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(left) {
                Ok(s) if s.hubs.iter().any(|h| h.id.contains(hid)) => {
                    found = Some(t0.elapsed().as_millis());
                    break;
                }
                _ => {}
            }
        }
        b.stop();
        match found {
            Some(ms) => {
                println!("cycle {i} found {ms}");
                times.push(ms);
            }
            None => {
                println!("cycle {i} MISS");
                misses += 1;
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    std::thread::sleep(Duration::from_secs(6));
    times.sort_unstable();
    let pct = |p: usize| times.get((times.len().saturating_sub(1)) * p / 100).copied().unwrap_or(0);
    println!(
        "summary cycles={cycles} found={} miss={misses} min={} median={} p90={} max={} held={}",
        times.len(),
        pct(0),
        pct(50),
        pct(90),
        pct(100),
        held()
    );
}

fn cycles(n: u32) {
    let t0 = Instant::now();
    for i in 0..n {
        let (tx, _rx) = mpsc::channel();
        let mut b = start(tx);
        // Stopped at every stage: opening, resolving, settled.
        std::thread::sleep(Duration::from_millis(50 + u64::from(i % 7) * 250));
        b.stop();
    }
    line(t0, &format!("after {n} cycles held={}", held()));
    // `DSPI_IDLE_S`: how long to stay idle afterwards, for a look at the process.
    let idle = std::env::var("DSPI_IDLE_S").ok().and_then(|v| v.parse().ok()).unwrap_or(6);
    std::thread::sleep(Duration::from_secs(idle));
    line(t0, &format!("{idle} s later held={}", held()));
}

fn adv(name: &str, hid: &str, port: u16, secs: u64, every: Option<u64>) {
    let t0 = Instant::now();
    let mut advert = HubAdvert {
        name: name.into(),
        port,
        hub_id: hid.into(),
        kind: "console".into(),
        auth: "pin".into(),
        device_count: 1,
        serials: vec!["E66164084319392D".into()],
        web: true,
        path: None,
    };
    let mut a = LanAdvertiser::start(&advert).expect("advertise");
    line(t0, &format!("advertising n={}", advert.device_count));
    let end = t0 + Duration::from_secs(secs);
    let mut next = every.map(|e| Instant::now() + Duration::from_secs(e));
    let mut last = a.names();
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(100));
        let names = a.names();
        if names != last {
            line(t0, &format!("names {names:?}"));
            last = names;
        }
        if let (Some(at), Some(e)) = (next, every) {
            if Instant::now() >= at {
                advert.device_count += 1;
                let r = a.update(&advert);
                line(t0, &format!("update n={} {r:?}", advert.device_count));
                next = Some(at + Duration::from_secs(e));
            }
        }
    }
    line(t0, &format!("names {:?}", a.names()));
    drop(a);
    line(t0, &format!("withdrawn held={}", held()));
}

fn main() {
    env_logger_lite();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let num = |s: &str| s.parse::<u64>().expect("number");
    let small = |s: &str| u32::try_from(num(s)).expect("number");
    match a.as_slice() {
        ["browse", secs] => browse(num(secs)),
        ["clock"] => {
            use std::io::{BufRead, Write};
            for _ in std::io::stdin().lock().lines() {
                println!("{}", wall());
                let _ = std::io::stdout().flush();
            }
        }
        ["first", hid, n, timeout] => first(hid, small(n), num(timeout)),
        ["cycles", n] => cycles(small(n)),
        ["adv", name, hid, port, secs] => adv(name, hid, small(port) as u16, num(secs), None),
        ["adv", name, hid, port, secs, every] => adv(name, hid, small(port) as u16, num(secs), Some(num(every))),
        _ => {
            eprintln!("usage: see the source");
            std::process::exit(2)
        }
    }
}

/// `RUST_LOG=debug` (or any value) prints the crate's log lines to stderr, with no logger
/// crate to depend on.
fn env_logger_lite() {
    struct Stderr(log::LevelFilter);
    impl log::Log for Stderr {
        fn enabled(&self, m: &log::Metadata<'_>) -> bool {
            m.level() <= self.0
        }
        fn log(&self, r: &log::Record<'_>) {
            if self.enabled(r.metadata()) {
                eprintln!("{} {} {}: {}", wall(), r.level(), r.target(), r.args());
            }
        }
        fn flush(&self) {}
    }
    let level = match std::env::var("RUST_LOG").as_deref() {
        Ok("debug") => log::LevelFilter::Debug,
        Ok("trace") => log::LevelFilter::Trace,
        Ok(_) => log::LevelFilter::Info,
        Err(_) => return,
    };
    if log::set_logger(Box::leak(Box::new(Stderr(level)))).is_ok() {
        log::set_max_level(level);
    }
}
