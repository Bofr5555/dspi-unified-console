//! WP LK3: `esp-rig-tunnel`, a DSPi on the UART of the emulated ESP32 bridge.
//!
//! Connects to the emulator's UART (QEMU's `-serial unix:PATH,server`), reads
//! the bridge's request frames, answers each through a DSPi behind the
//! [`dspi_transport::Transport`] seam (the demo device, or the real one over
//! USB) and sends the answers and the device's notifications back at the pace
//! of a real 8N1 line at the chosen baud, with optional wire faults.
//! See tools/esp-rig/README.md.

mod device;
mod uart;

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use dspi_proto::frame::{Frame, status};
use dspi_transport::{FakeDevice, NotificationSource, Transport, UsbTransport};

use device::{Device, DeviceOptions};
use uart::{Faults, Line, Parsed, RequestParser, Rng, status_reply};

/// The DSPi's notification ring (`notify.c` `NOTIFY_RING_SIZE` 32, one slot
/// kept free): a consumer that falls this far behind loses its oldest entry.
const NOTIFY_RING: usize = 31;
/// How long to keep trying to reach the emulator's UART socket.
const CONNECT_FOR: Duration = Duration::from_secs(120);
/// The write loop's granularity: at 1 Mbaud, 100 bytes, inside the ESP32's
/// 128-byte receive FIFO.
const TICK: Duration = Duration::from_millis(1);

const USAGE: &str =
    "usage: esp-rig-tunnel --uart PATH [--baud N] [--device demo|demo-nosurfaces|usb|usb:SERIAL]
       [--allow-writes] [--no-notify] [--latency-us N] [--seed N] [--stats-every S] [--log PATH]
       [--flip-in P] [--drop-in P] [--flip-out P] [--drop-out P] [--drop-reply P]
  --uart       the emulator's UART socket (QEMU -serial unix:PATH,server=on)
  --baud       the line's rate, default 1000000
  --device     demo (default), demo-nosurfaces, usb (the one DSPi), usb:SERIAL
  --allow-writes  pass SETs to a USB device (refused with ERROR otherwise)
  --no-notify  report notify_enable 0 and send no notification frames
  --latency-us the DSPi's dispatch time per request, default 300
  --flip-* / --drop-*  per-byte bit-flip / loss probability; -in is bridge to DSPi,
               -out DSPi to bridge
  --drop-reply probability that a request runs but its answer is lost
  --log        one line per request: t_ms op kind span_us reply_bytes status";

struct Args {
    uart: String,
    baud: u32,
    device: String,
    allow_writes: bool,
    notify: bool,
    latency: Duration,
    seed: u64,
    stats_every: Option<Duration>,
    log: Option<String>,
    faults_in: Faults,
    faults_out: Faults,
    drop_reply: f64,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        uart: String::new(),
        baud: 1_000_000,
        device: "demo".into(),
        allow_writes: false,
        notify: true,
        latency: Duration::from_micros(300),
        seed: 1,
        stats_every: None,
        log: None,
        faults_in: Faults::default(),
        faults_out: Faults::default(),
        drop_reply: 0.0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{flag} needs a value"));
        let num = |v: String| {
            v.parse::<f64>()
                .map_err(|_| format!("{flag}: not a number: {v}"))
        };
        match flag.as_str() {
            "--uart" => a.uart = value()?,
            "--baud" => a.baud = num(value()?)? as u32,
            "--device" => a.device = value()?,
            "--allow-writes" => a.allow_writes = true,
            "--no-notify" => a.notify = false,
            "--latency-us" => a.latency = Duration::from_micros(num(value()?)? as u64),
            "--seed" => a.seed = num(value()?)? as u64,
            "--stats-every" => a.stats_every = Some(Duration::from_secs_f64(num(value()?)?)),
            "--log" => a.log = Some(value()?),
            "--flip-in" => a.faults_in.flip = num(value()?)?,
            "--drop-in" => a.faults_in.drop = num(value()?)?,
            "--flip-out" => a.faults_out.flip = num(value()?)?,
            "--drop-out" => a.faults_out.drop = num(value()?)?,
            "--drop-reply" => a.drop_reply = num(value()?)?,
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown flag {other}\n{USAGE}")),
        }
    }
    if a.uart.is_empty() {
        return Err(USAGE.into());
    }
    if !(9600..=1_000_000).contains(&a.baud) {
        return Err(format!("--baud {}: the DSPi takes 9600 to 1000000", a.baud));
    }
    Ok(a)
}

fn open_device(name: &str) -> Result<(Box<dyn Transport>, bool), String> {
    match name {
        "demo" => Ok((Box::new(FakeDevice::demo()), false)),
        "demo-nosurfaces" => Ok((Box::new(FakeDevice::demo_without_surfaces()), false)),
        "usb" => UsbTransport::open_only()
            .map(|t| (Box::new(t) as Box<dyn Transport>, true))
            .map_err(|e| e.to_string()),
        _ => match name.strip_prefix("usb:") {
            Some(serial) => UsbTransport::open_serial(serial)
                .map(|t| (Box::new(t) as Box<dyn Transport>, true))
                .map_err(|e| e.to_string()),
            None => Err(format!("unknown device {name}\n{USAGE}")),
        },
    }
}

fn connect(path: &str) -> Result<UnixStream, String> {
    let until = Instant::now() + CONNECT_FOR;
    loop {
        match UnixStream::connect(path) {
            Ok(s) => return Ok(s),
            Err(e) if Instant::now() > until => return Err(format!("{path}: {e}")),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The DSPi's transmit side, shared by the request loop (which queues) and
/// the writer thread (which puts due bytes on the socket).
struct Tx {
    line: Mutex<Line>,
    wake: Condvar,
    faults: Mutex<(Faults, Rng)>,
    stop: AtomicBool,
}

fn writer(tx: Arc<Tx>, mut sock: UnixStream) {
    let mut due = Vec::new();
    let mut out = Vec::new();
    while !tx.stop.load(Ordering::Relaxed) {
        let next = {
            let mut line = lock(&tx.line);
            line.due(Instant::now(), &mut due);
            if due.is_empty() && line.next_due().is_none() {
                let (line, _) = tx
                    .wake
                    .wait_timeout(line, Duration::from_millis(50))
                    .unwrap_or_else(PoisonError::into_inner);
                drop(line);
                continue;
            }
            line.next_due()
        };
        if !due.is_empty() {
            {
                let mut f = lock(&tx.faults);
                let (faults, rng) = &mut *f;
                faults.apply(rng, &due, &mut out);
            }
            if sock.write_all(&out).is_err() {
                tx.stop.store(true, Ordering::Relaxed);
                return;
            }
            due.clear();
            out.clear();
        }
        if let Some(next) = next {
            let wait = next.saturating_duration_since(Instant::now()).min(TICK);
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
        }
    }
}

/// The notifications the DSPi holds for its UART consumer, oldest first.
#[derive(Default)]
struct Ring {
    packets: Mutex<VecDeque<Vec<u8>>>,
    dropped: AtomicU64,
    received: AtomicU64,
}

fn notifier(ring: Arc<Ring>, mut source: Box<dyn NotificationSource>, stop: Arc<Tx>) {
    while !stop.stop.load(Ordering::Relaxed) {
        match source.read(Duration::from_millis(100)) {
            Ok(p) if p.is_empty() => {}
            Ok(p) => {
                ring.received.fetch_add(1, Ordering::Relaxed);
                let mut q = lock(&ring.packets);
                if q.len() >= NOTIFY_RING {
                    q.pop_front();
                    ring.dropped.fetch_add(1, Ordering::Relaxed);
                }
                q.push_back(p);
            }
            Err(e) => {
                eprintln!("esp-rig-tunnel: notifications ended: {e}");
                return;
            }
        }
    }
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    if let Err(e) = run(args) {
        eprintln!("esp-rig-tunnel: {e}");
        std::process::exit(1);
    }
}

fn run(mut args: Args) -> Result<(), String> {
    let (transport, is_usb) = open_device(&args.device)?;
    let source = if args.notify {
        transport.notifications()
    } else {
        None
    };
    let mut device = Device::new(
        transport,
        DeviceOptions {
            baud: args.baud,
            notify: args.notify,
            read_only: is_usb && !args.allow_writes,
        },
    );
    eprintln!(
        "esp-rig-tunnel: DSPi {} ({}) on {} at {} baud, notifications {}, writes {}",
        device.transport().descriptor().serial,
        args.device,
        args.uart,
        args.baud,
        if args.notify { "on" } else { "off" },
        if device.options.read_only {
            "refused"
        } else {
            "passed"
        }
    );
    let mut sock = connect(&args.uart)?;
    sock.set_read_timeout(Some(TICK))
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    let tx = Arc::new(Tx {
        line: Mutex::new(Line::new(args.baud, start)),
        wake: Condvar::new(),
        faults: Mutex::new((
            std::mem::take(&mut args.faults_out),
            Rng::new(args.seed ^ 0x5A),
        )),
        stop: AtomicBool::new(false),
    });
    let wsock = sock.try_clone().map_err(|e| e.to_string())?;
    let w = {
        let tx = Arc::clone(&tx);
        std::thread::spawn(move || writer(tx, wsock))
    };
    let ring = Arc::new(Ring::default());
    if let Some(source) = source {
        let (ring, tx) = (Arc::clone(&ring), Arc::clone(&tx));
        std::thread::spawn(move || notifier(ring, source, tx));
    }
    let mut log = match &args.log {
        Some(p) => Some(std::io::BufWriter::new(
            std::fs::File::create(p).map_err(|e| format!("{p}: {e}"))?,
        )),
        None => None,
    };

    let mut rng = Rng::new(args.seed);
    let mut parser = RequestParser::default();
    let mut buf = [0u8; 4096];
    let mut arrived = Vec::new();
    let (mut requests, mut lost_replies, mut bad_crc, mut sent_notes) = (0u64, 0u64, 0u64, 0u64);
    let mut longest_span = Duration::ZERO;
    let mut next_stats = args.stats_every.map(|d| start + d);
    let send = |frame: &Frame| {
        let bytes = frame.encode_uart();
        lock(&tx.line).send(&bytes, Instant::now());
        tx.wake.notify_one();
        bytes.len()
    };

    loop {
        if tx.stop.load(Ordering::Relaxed) {
            break;
        }
        match sock.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let now = Instant::now();
                arrived.clear();
                args.faults_in.apply(&mut rng, &buf[..n], &mut arrived);
                for &byte in &arrived {
                    let Some(parsed) = parser.push(byte, now) else {
                        continue;
                    };
                    requests += 1;
                    let span = parser
                        .started()
                        .map_or(Duration::ZERO, |s| now.saturating_duration_since(s));
                    longest_span = longest_span.max(span);
                    let (reply, op, kind) = match &parsed {
                        Parsed::BadCrc { is_get } => {
                            bad_crc += 1;
                            (Some(status_reply(*is_get, status::CRC_ERROR)), 0, "crc")
                        }
                        Parsed::Request(frame) => {
                            if !args.latency.is_zero() {
                                std::thread::sleep(args.latency);
                            }
                            let (op, kind) = match frame {
                                Frame::Get { request, .. } => (*request, "get"),
                                Frame::Set { request, .. } => (*request, "set"),
                                _ => (0, "?"),
                            };
                            (device.answer(frame), op, kind)
                        }
                    };
                    let (bytes, code) = match reply {
                        Some(_) if rng.chance(args.drop_reply) => {
                            lost_replies += 1;
                            (0, "lost")
                        }
                        Some(r) => {
                            let code = match &r {
                                Frame::GetResponse { status: s, .. }
                                | Frame::SetResponse { status: s } => status::name(*s),
                                _ => "?",
                            };
                            (send(&r), code)
                        }
                        None => (0, "none"),
                    };
                    if let Some(l) = log.as_mut() {
                        let _ = writeln!(
                            l,
                            "{:.1} 0x{op:02X} {kind} {} {bytes} {code}",
                            now.duration_since(start).as_secs_f64() * 1000.0,
                            span.as_micros()
                        );
                    }
                }
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(format!("reading the UART: {e}")),
        }
        // A notification goes out only at a frame boundary on an idle line
        // (spec 3.6, idle-priority rule).
        let now = Instant::now();
        if !parser.mid_frame() && lock(&tx.line).idle(now) {
            let packet = lock(&ring.packets).pop_front();
            if let Some(packet) = packet {
                send(&Frame::Notification { packet });
                sent_notes += 1;
            }
        }
        if let Some(at) = next_stats.filter(|&at| now >= at) {
            next_stats = Some(at + args.stats_every.unwrap_or(Duration::MAX));
            let s = &device.stats;
            let f = lock(&tx.faults);
            eprintln!(
                "esp-rig-tunnel t={:.0}s requests={requests} gets={} sets={} refused={} stalls={} no_answer={} \
                 lost_replies={lost_replies} bad_crc={bad_crc} parser_timeouts={} noise={} \
                 notes_sent={sent_notes} notes_dropped={} flips_in={} drops_in={} flips_out={} drops_out={} \
                 longest_request_span_us={}",
                now.duration_since(start).as_secs_f64(),
                s.gets,
                s.sets,
                s.refused_sets,
                s.stalls,
                s.timeouts,
                parser.timed_out,
                parser.noise,
                ring.dropped.load(Ordering::Relaxed),
                args.faults_in.flipped,
                args.faults_in.dropped,
                f.0.flipped,
                f.0.dropped,
                longest_span.as_micros()
            );
        }
        if let Some(l) = log.as_mut() {
            let _ = l.flush();
        }
    }
    tx.stop.store(true, Ordering::Relaxed);
    tx.wake.notify_one();
    let _ = w.join();
    eprintln!("esp-rig-tunnel: the emulator closed its UART after {requests} requests");
    Ok(())
}
