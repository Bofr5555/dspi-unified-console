//! `dspi hub`: share this machine's DSPi devices on the network, with no window.
//!
//! A headless DSPi Link hub over `dspi_hub` (HUB.md; WP HB6 is the same hub behind the
//! C ABI, and this follows its rules). It runs in the foreground until Ctrl-C or
//! SIGTERM, so a service manager can own it: `packaging/service/` has a systemd user
//! unit, a launchd agent and a Windows scheduled task, with install notes.
//!
//! - **PIN pairing is the default.** `--open` (every connection an admin, no PIN) only
//!   when asked, with a warning. A PIN is shown only here, on this machine's terminal or
//!   in its log, and only while a window is open: `--pair` opens one at the start, and
//!   `pair` typed at a running hub opens another. It crosses the network in plain text.
//! - **Clients and roles** are commands: typed at a running hub on a terminal (they then
//!   reach the live sessions), or `dspi hub clients|role|revoke` with the hub stopped.
//!   A hub holds a lock file in its store, so those refuse while one runs: the store has
//!   one owner (HUB.md, HB1), and a second writer would put back what the hub changed.
//! - **A change of access mode revokes every token** (spec 6.2). Starting in the other
//!   mode is refused while that would forget paired clients, unless `--forget-clients`.
//! - **Sleep inhibit** while it runs, unless `--no-keep-awake`: see [`awake`].
//! - **Logs** go to standard error, `RUST_LOG` deciding (warnings by default), with
//!   `tungstenite` held at debug, as its trace level prints whole frames, PINs and
//!   tokens included (HB6).
//! - Owner rule (HB2), as it stands: a network client cannot raise the core voltage
//!   above 1.20 V, whatever its role. That is done on this machine.
//!
//! `dspi-hub` is built without `app`: there is no `DspiCore` here, and nothing holds a
//! device but the hub. `--demo` serves the built-in simulated device instead of USB.
//!
//! Owned by WP TM11 (docs/core-merge/PLAN-CLIENTS.md).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{BufRead, IsTerminal};
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime};

use dspi_hub::auth::AuthStore;
use dspi_hub::store::HubStore;
use dspi_hub::{
    AccessMode, DEFAULT_PORT, DeviceState, Hub, HubConfig, HubError, HubEvent, HubHandle,
    HubListener, NetworkConfig, NetworkControl, PairedClient, Role, SavedHub, UsbBus,
};
use dspi_transport::FakeDevice;
use dspi_tui::shell::connect::{Target, printable};
use serde_json::{Value, json};

use crate::exit;

/// What a hub is called when nobody has named it (as HB6's).
const DEFAULT_NAME: &str = "DSPi Hub";
/// Longest hub name kept, in characters: it goes to every client (as HB6's).
const MAX_NAME: usize = 64;
/// Held by a running hub, in its store.
const LOCK_FILE: &str = "dspi-hub.lock";
/// How often the foreground loop looks for a stop.
const TICK: Duration = Duration::from_millis(200);

/// Why a hub command did not do what it was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Failure {
    /// The command line, or a choice it needs to make explicitly.
    Usage(String),
    /// The machine: the store, the port, the bus.
    Failed(String),
}

impl Failure {
    fn report(self) -> u8 {
        match self {
            Failure::Usage(m) => {
                eprintln!("dspi hub: {m}");
                exit::USAGE
            }
            Failure::Failed(m) => {
                eprintln!("dspi hub: {m}");
                exit::TRANSPORT
            }
        }
    }
}

impl From<HubError> for Failure {
    fn from(e: HubError) -> Self {
        Failure::Failed(e.to_string())
    }
}

/// How `dspi hub run` starts.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RunOptions {
    /// `--name`; otherwise the stored one, otherwise [`DEFAULT_NAME`].
    name: Option<String>,
    port: u16,
    access: AccessMode,
    store: PathBuf,
    loopback: bool,
    /// Open a pairing window at the start.
    pair: bool,
    /// `--demo`: the simulated device, no bus.
    demo: bool,
    keep_awake: bool,
    forget_clients: bool,
}

/// One `dspi hub` command.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HubCommand {
    Help,
    Run(RunOptions),
    Clients {
        store: PathBuf,
    },
    Role {
        store: PathBuf,
        cid: u32,
        role: Role,
    },
    Revoke {
        store: PathBuf,
        cid: u32,
    },
}

/// `dspi hub ...`; `flags` starts at the word `hub`, without the target flags.
pub fn run(target: &Target, flags: &[&str], json: bool) -> u8 {
    if let Some(code) = refuse_target(target) {
        return code;
    }
    let demo = *target == Target::Demo;
    let command = match parse(flags.get(1..).unwrap_or_default(), demo, default_store()) {
        Ok(c) => c,
        Err(e) => return e.report(),
    };
    init_logger();
    let done = match command {
        HubCommand::Help => {
            usage();
            Ok(())
        }
        HubCommand::Run(opts) => serve(&opts, json),
        HubCommand::Clients { store } => offline_clients(&store, json),
        HubCommand::Role { store, cid, role } => {
            offline_edit(&store, cid, |auth| auth.set_role(cid, role))
                .map(|()| println!("client {cid} is now {}", role_name(role)))
        }
        HubCommand::Revoke { store, cid } => offline_edit(&store, cid, |auth| auth.revoke(cid))
            .map(|()| println!("client {cid} revoked: it must pair again")),
    };
    match done {
        Ok(()) => exit::OK,
        Err(e) => e.report(),
    }
}

/// A target a hub cannot run on, refused with its exit status. `main` asks before it
/// looks for a `--hub` on the network, so `dspi --hub hub` fails at once (review c4b).
pub fn refuse_target(target: &Target) -> Option<u8> {
    let why = match target {
        Target::Usb { serial: None } | Target::Demo => return None,
        Target::Usb { serial: Some(_) } => {
            "--device: a hub shares every DSPi on this machine's USB"
        }
        Target::Midi { .. } | Target::Link(_) => {
            "a hub shares this machine's USB devices; --midi, --link and --hub \
             reach a device elsewhere"
        }
    };
    Some(Failure::Usage(why.into()).report())
}

fn usage() {
    println!(
        "\
dspi hub - share this machine's DSPi devices on the network (DSPi Link)

USAGE:
    dspi hub [run] [OPTIONS]  run a hub in the foreground until Ctrl-C or SIGTERM
    dspi --demo hub [...]     the same, sharing the built-in simulated device
    dspi hub clients          list the paired clients (hub stopped)
    dspi hub role <cid> viewer|control|admin
                              change a client's role (hub stopped)
    dspi hub revoke <cid>     forget a client; it must pair again (hub stopped)

OPTIONS:
    --name <name>             the hub's name, kept for the next start
    --port <port>             the port to listen on (default {DEFAULT_PORT}; 0: any)
    --pair                    open a pairing window at the start and show its PIN
    --open                    no PIN: every connection may do anything, including
                              anyone else on this network. PIN pairing is the default
    --forget-clients          agree to a change of --open that forgets every
                              paired client
    --loopback                listen on this machine only, and do not advertise
    --no-keep-awake           let the machine sleep while sharing
    --store <dir>             where the hub keeps its id, clients and device names
                              (default: {})
    --json                    one JSON object per line

WHILE IT RUNS, on a terminal, type:
    pair                      open pairing for 2 minutes and show the PIN
    unpair                    close the pairing window
    clients                   the paired clients, and which are connected
    role <cid> <role>         change a client's role, live
    revoke <cid>              forget a client and disconnect it
    devices                   the devices being shared
    status                    the hub's address and access
    quit                      stop sharing

A PIN crosses the network in plain text: pair on a network you trust.
Clients on the network cannot raise a unit's core voltage above 1.20 V;
do that on this machine. Logs go to standard error (RUST_LOG).
",
        default_store().map_or_else(
            || "none here: give --store".to_owned(),
            |p| p.display().to_string()
        )
    );
}

/// Where a hub keeps its store when nobody says: the user's local data directory,
/// since the hub id and the token hashes belong to this machine.
fn default_store() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join("dspi").join("hub"))
}

/// The arguments after `hub`. `default_store` is passed in for the tests.
fn parse(args: &[&str], demo: bool, default_store: Option<PathBuf>) -> Result<HubCommand, Failure> {
    let usage = |m: String| Failure::Usage(m);
    let mut name = None;
    let mut port = DEFAULT_PORT;
    let mut store = None;
    let mut run_only: Vec<&str> = Vec::new();
    let mut access = AccessMode::Pin;
    let (mut loopback, mut pair, mut keep_awake, mut forget) = (false, false, true, false);
    let mut words = Vec::new();

    let mut it = args.iter().copied();
    while let Some(arg) = it.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v)),
            _ => (arg, None),
        };
        let mut value = |what: &str| -> Result<String, Failure> {
            match inline {
                Some(v) => Ok(v.to_owned()),
                None => it
                    .next()
                    .map(str::to_owned)
                    .ok_or_else(|| usage(format!("{flag} needs {what}"))),
            }
        };
        match flag {
            "--help" | "-h" => return Ok(HubCommand::Help),
            "--json" => {}
            "--store" => store = Some(PathBuf::from(value("a directory")?)),
            "--name" => {
                name = Some(clean_name(&value("a name")?)?);
                run_only.push("--name");
            }
            "--port" => {
                let text = value("a port number")?;
                port = text
                    .parse()
                    .map_err(|_| usage(format!("`{text}` is not a port number")))?;
                run_only.push("--port");
            }
            "--open" => access = AccessMode::Open,
            "--loopback" => loopback = true,
            "--pair" => pair = true,
            "--no-keep-awake" => keep_awake = false,
            "--forget-clients" => forget = true,
            f if f.starts_with('-') => return Err(usage(format!("`{f}` is not a hub option"))),
            word => words.push(word),
        }
        if matches!(
            flag,
            "--open" | "--loopback" | "--pair" | "--no-keep-awake" | "--forget-clients"
        ) {
            run_only.push(flag);
        }
    }

    let store = || {
        store
            .clone()
            .or_else(|| default_store.clone())
            .ok_or_else(|| usage("this system has no data directory: give --store".into()))
    };
    let cid = |w: Option<&&str>| -> Result<u32, Failure> {
        let w =
            w.ok_or_else(|| usage("which client? Give its number from `dspi hub clients`".into()))?;
        w.parse()
            .map_err(|_| usage(format!("`{w}` is not a client number")))
    };
    let command = match words.as_slice() {
        [] | ["run"] => {
            return Ok(HubCommand::Run(RunOptions {
                name,
                port,
                access,
                store: store()?,
                loopback,
                pair,
                demo,
                keep_awake,
                forget_clients: forget,
            }));
        }
        ["help"] => return Ok(HubCommand::Help),
        ["clients"] => HubCommand::Clients { store: store()? },
        ["role", rest @ ..] if rest.len() <= 2 => HubCommand::Role {
            store: store()?,
            cid: cid(rest.first())?,
            role: parse_role(rest.get(1).copied())?,
        },
        ["revoke", rest @ ..] if rest.len() <= 1 => HubCommand::Revoke {
            store: store()?,
            cid: cid(rest.first())?,
        },
        _ => {
            return Err(usage(format!(
                "`{}` is not a hub command; see `dspi hub --help`",
                printable(&words.join(" "))
            )));
        }
    };
    if let Some(flag) = run_only.first() {
        return Err(usage(format!(
            "{flag} is for running a hub, not this command"
        )));
    }
    if demo {
        return Err(usage(
            "--demo is for running a hub, not this command".into(),
        ));
    }
    Ok(command)
}

fn parse_role(word: Option<&str>) -> Result<Role, Failure> {
    match word {
        Some("viewer") => Ok(Role::Viewer),
        Some("control") => Ok(Role::Control),
        Some("admin") => Ok(Role::Admin),
        Some(other) => Err(Failure::Usage(format!(
            "`{other}` is not a role: viewer, control or admin"
        ))),
        None => Err(Failure::Usage(
            "which role? viewer, control or admin".into(),
        )),
    }
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Viewer => "viewer",
        Role::Control => "control",
        Role::Admin => "admin",
    }
}

/// A name every client can show: no control characters, at most [`MAX_NAME`].
fn clean_name(name: &str) -> Result<String, Failure> {
    let cleaned: String = name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect();
    if cleaned.trim().is_empty() {
        return Err(Failure::Usage("the hub's name is empty".into()));
    }
    Ok(cleaned.trim().to_owned())
}

/// The logger: standard error, `RUST_LOG`, warnings by default; `tungstenite` stops at
/// debug, since its trace level prints every WebSocket message whole, and those carry
/// PINs and tokens (HB6's `logger_builder`, for this binary's own logger).
///
/// -- WP TM26 -- A ceiling, not a level: `RUST_LOG` still decides, and tungstenite gets
/// the lower of what it asks for and debug. A `filter_module("tungstenite", Debug)`
/// raised it to debug at `RUST_LOG=info` (hw-1 M3).
struct CappedLogger(env_logger::Logger);

impl CappedLogger {
    fn new(mut builder: env_logger::Builder) -> Self {
        Self(builder.build())
    }

    fn filter(&self) -> log::LevelFilter {
        self.0.filter()
    }
}

/// tungstenite's own targets: the crate and its modules, not a crate that merely
/// starts with the same letters.
fn is_tungstenite(target: &str) -> bool {
    target == "tungstenite" || target.starts_with("tungstenite::")
}

impl log::Log for CappedLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        !(is_tungstenite(metadata.target()) && metadata.level() > log::Level::Debug)
            && self.0.enabled(metadata)
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            self.0.log(record);
        }
    }

    fn flush(&self) {
        self.0.flush();
    }
}

fn init_logger() {
    let env = env_logger::Env::default().default_filter_or("warn");
    let logger = CappedLogger::new(env_logger::Builder::from_env(env));
    let max = logger.filter();
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(max);
    }
}

/// The store's lock, held while a hub runs on it. Taking it creates the directory.
fn lock_store(store: &Path) -> Result<File, Failure> {
    std::fs::create_dir_all(store)
        .map_err(|e| Failure::Failed(format!("cannot create {}: {e}", store.display())))?;
    let path = store.join(LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| Failure::Failed(format!("cannot open {}: {e}", path.display())))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(Failure::Usage(format!(
            "a hub is running on {}: type the command at its terminal, or stop it first",
            store.display()
        ))),
        Err(TryLockError::Error(e)) => Err(Failure::Failed(format!(
            "cannot lock {}: {e}",
            path.display()
        ))),
    }
}

/// A running hub and what it holds. Dropping it stops the hub, then lets go of the
/// lock and the sleep inhibit.
struct Running {
    hub: Hub,
    handle: HubHandle,
    control: NetworkControl,
    name: String,
    access: AccessMode,
    _awake: Option<awake::KeepAwake>,
    _lock: File,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.hub.stop();
    }
}

/// Start sharing. Nothing is claimed until the port is bound and the listener is in,
/// and no client is let in until the device is (WP TM26).
fn start(opts: &RunOptions, json: bool) -> Result<Running, Failure> {
    let lock = lock_store(&opts.store)?;
    let saved = SavedHub::peek(&opts.store)?;
    let lost = saved
        .as_ref()
        .map_or(0, |s| s.tokens_lost_starting_in(opts.access));
    if lost > 0 && !opts.forget_clients {
        return Err(Failure::Usage(format!(
            "this hub last ran {}; starting it {} forgets its {lost} paired client(s), \
             which must then pair again. Add --forget-clients to go ahead",
            access_words(other_mode(opts.access)),
            access_words(opts.access),
        )));
    }
    let name = match &opts.name {
        Some(n) => n.clone(),
        None => saved
            .and_then(|s| s.name)
            .unwrap_or_else(|| DEFAULT_NAME.to_owned()),
    };

    let mut network = NetworkConfig::new(opts.store.clone());
    network.port = opts.port;
    network.access = opts.access;
    if opts.loopback {
        network.bind = IpAddr::V4(Ipv4Addr::LOCALHOST);
    }
    // -- WP TM26 -- The port is ours from here; clients wait until the device is in
    // (hw-1 M2: they were told "No DSPi found" for the first 0.3 s).
    network.hold_accepts = true;
    let mut config = HubConfig::in_process(&name);
    config.network = Some(network);
    // A scan only once the listener is in, below.
    config.initial_scan = false;
    if !opts.demo {
        config.bus = Some(Box::new(UsbBus::default()));
    }
    let hub = Hub::start(config)?;
    let handle = hub.handle();
    // Always there on a hub started with a network side.
    let control = hub.network().ok_or(HubError::Stopped)?;
    if opts.name.is_some() {
        control.auth().set_name(&name);
    }
    handle.set_listener(Some(Arc::new(Printer { json })));
    let running = Running {
        hub,
        handle,
        control,
        name,
        access: opts.access,
        _awake: opts.keep_awake.then(awake::KeepAwake::begin).flatten(),
        _lock: lock,
    };
    if opts.demo {
        running
            .handle
            .attach(Box::new(FakeDevice::demo()), 0)
            .map_err(|e| Failure::Failed(format!("the demo device did not attach: {e}")))?;
    } else {
        running.handle.scan_now();
    }
    // -- WP TM26 -- Attached, or found absent: either way, what a client sees now is
    // the truth.
    running.control.start_accepting();
    Ok(running)
}

fn other_mode(mode: AccessMode) -> AccessMode {
    match mode {
        AccessMode::Pin => AccessMode::Open,
        AccessMode::Open => AccessMode::Pin,
    }
}

fn access_words(mode: AccessMode) -> &'static str {
    match mode {
        AccessMode::Pin => "with PIN pairing",
        AccessMode::Open => "open (--open)",
    }
}

/// Prints the hub's events, on its event thread.
struct Printer {
    json: bool,
}

impl HubListener for Printer {
    fn on_event(&self, event: HubEvent) {
        let (kind, text) = match &event {
            HubEvent::DeviceOnline { handle, serial } => (
                "deviceOnline",
                format!("device {} online (handle {handle})", printable(serial)),
            ),
            HubEvent::DeviceOffline { handle, serial } => (
                "deviceOffline",
                format!("device {} offline (handle {handle})", printable(serial)),
            ),
            HubEvent::DeviceRemoved { handle, serial } => (
                "deviceRemoved",
                format!("device {} gone (handle {handle})", printable(serial)),
            ),
            HubEvent::SessionOpened { id } => (
                "clientConnected",
                format!("client connected (session {id})"),
            ),
            HubEvent::SessionClosed { id } => (
                "clientDisconnected",
                format!("client disconnected (session {id})"),
            ),
            HubEvent::Stopped => ("stopped", "hub stopped".to_owned()),
        };
        if self.json {
            let v = match event {
                HubEvent::DeviceOnline { handle, serial }
                | HubEvent::DeviceOffline { handle, serial }
                | HubEvent::DeviceRemoved { handle, serial } => {
                    json!({ "event": kind, "handle": handle, "serial": serial })
                }
                HubEvent::SessionOpened { id } | HubEvent::SessionClosed { id } => {
                    json!({ "event": kind, "session": id })
                }
                HubEvent::Stopped => json!({ "event": kind }),
            };
            println!("{v}");
        } else {
            println!("{text}");
        }
    }
}

/// `dspi hub run`: start, serve until asked to stop, stop.
fn serve(opts: &RunOptions, json: bool) -> Result<(), Failure> {
    let stop = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        if let Err(e) = signal_hook::flag::register(signal, Arc::clone(&stop)) {
            log::warn!("dspi hub: signal {signal} not caught ({e}): it will stop abruptly");
        }
    }
    let running = start(opts, json)?;
    status(&running, json);
    let tokens_revoked = running.control.auth().tokens_revoked_at_start();
    if tokens_revoked > 0 && !json {
        println!("{tokens_revoked} paired client(s) forgotten: the access mode changed");
    }
    if running.access == AccessMode::Open && !json {
        println!(
            "warning: open access. Anyone who can reach this machine can control every \
             device, with no PIN"
        );
    }
    if opts.pair {
        begin_pairing(&running, json);
    }

    let typed = std::io::stdin().is_terminal().then(read_lines).flatten();
    if typed.is_some() && !json {
        println!("Type `help` for commands, `quit` to stop sharing.");
    }
    while !stop.load(Ordering::Relaxed) {
        let Some(lines) = &typed else {
            std::thread::sleep(TICK);
            continue;
        };
        match lines.recv_timeout(TICK) {
            Ok(line) => {
                if !command(&running, &line, json) {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            // End of input on a terminal: Ctrl-D.
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    drop(running);
    Ok(())
}

/// The lines typed at the terminal, read on a thread of their own, which a blocked
/// read keeps until the process ends. `None` if that thread could not start.
///
/// A hub started in the background (`dspi hub &`) still has the terminal as its
/// standard input, and a read from there would stop the whole process with SIGTTIN
/// (review c4b). So SIGTTIN is ignored, which turns such a read into an error, and an
/// error is tried again a little later rather than taken for the end of input: brought
/// to the foreground with `fg`, the hub takes commands again.
fn read_lines() -> Option<mpsc::Receiver<String>> {
    ignore_background_read();
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("dspi-hub-stdin".into())
        .spawn(move || pump_lines(std::io::stdin().lock(), &tx, RETRY_READ));
    match spawned {
        Ok(_) => Some(rx),
        Err(e) => {
            log::warn!("dspi hub: no thread to read commands ({e}); Ctrl-C stops the hub");
            None
        }
    }
}

/// How long a read that failed (a background job's) waits to be tried again.
const RETRY_READ: Duration = Duration::from_secs(1);

/// Send each line of `input` until its end, or until nobody listens. A failed read is
/// tried again after `retry`.
fn pump_lines(mut input: impl BufRead, tx: &mpsc::Sender<String>, retry: Duration) {
    loop {
        let mut line = String::new();
        match input.read_line(&mut line) {
            Ok(0) => return,
            Ok(_) => {
                let line = line.trim_end_matches(['\n', '\r']).to_owned();
                if tx.send(line).is_err() {
                    return;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => std::thread::sleep(retry),
        }
    }
}

/// Have a read from the terminal while in the background fail with EIO instead of
/// stopping the process (POSIX: a process that ignores SIGTTIN gets EIO). SIGTTOU too,
/// so that `stty tostop` does not stop a background hub at its next line of output.
#[cfg(unix)]
fn ignore_background_read() {
    for signal in [libc::SIGTTIN, libc::SIGTTOU] {
        // SAFETY: sets a disposition to SIG_IGN, which runs no code of ours.
        if unsafe { libc::signal(signal, libc::SIG_IGN) } == libc::SIG_ERR {
            log::warn!("dspi hub: signal {signal} not ignored: in the background it may stop");
        }
    }
}

#[cfg(not(unix))]
fn ignore_background_read() {}

/// One typed line. `false` to stop.
fn command(running: &Running, line: &str, json: bool) -> bool {
    let words: Vec<&str> = line.split_whitespace().collect();
    let say = |m: &str| {
        if json {
            println!("{}", json!({ "event": "error", "error": m }));
        } else {
            println!("{m}");
        }
    };
    match words.as_slice() {
        [] => {}
        ["quit" | "stop" | "exit"] => return false,
        ["help"] => usage(),
        ["status"] => status(running, json),
        ["pair"] => begin_pairing(running, json),
        ["unpair"] => {
            running.control.auth().cancel_pairing();
            say("pairing closed");
        }
        ["devices"] => devices(running, json),
        ["clients"] => print_clients(
            &running.control.auth().clients(),
            Some(&|cid| running.control.is_online(cid)),
            json,
        ),
        ["role", cid, role] => match (cid.parse::<u32>(), parse_role(Some(role))) {
            (Ok(cid), Ok(role)) if running.control.set_role(cid, role) => {
                say(&format!("client {cid} is now {}", role_name(role)));
            }
            (Ok(cid), Ok(_)) => say(&format!("no paired client {cid}")),
            _ => say("role <cid> viewer|control|admin"),
        },
        ["revoke", cid] => match cid.parse::<u32>() {
            Ok(cid) if running.control.revoke(cid) => {
                say(&format!("client {cid} revoked: it must pair again"));
            }
            Ok(cid) => say(&format!("no paired client {cid}")),
            Err(_) => say("revoke <cid>"),
        },
        _ => say(&format!(
            "`{}`: not a hub command; type `help`",
            printable(line.trim())
        )),
    }
    true
}

fn status(running: &Running, json: bool) {
    let port = running.control.port();
    let id = running.handle.hub_id();
    if json {
        println!(
            "{}",
            json!({
                "event": "status",
                "hubId": id,
                "name": running.name,
                "port": port,
                "access": running.access.wire_name(),
                "connections": running.control.connections(),
                "tokensRevoked": running.control.auth().tokens_revoked_at_start(),
            })
        );
    } else {
        println!(
            "Sharing as \"{}\" on port {port}, {} (hub id {id})",
            printable(&running.name),
            access_words(running.access),
        );
    }
}

fn begin_pairing(running: &Running, json: bool) {
    if running.access == AccessMode::Open {
        let m = "access is open: clients connect with no PIN";
        if json {
            println!("{}", json!({ "event": "error", "error": m }));
        } else {
            println!("{m}");
        }
        return;
    }
    let now = Instant::now();
    match running.control.auth().begin_pairing(now) {
        Ok(window) => {
            let secs = window.expires.saturating_duration_since(now).as_secs();
            if json {
                println!(
                    "{}",
                    json!({ "event": "pairing", "pin": window.pin, "expiresInSecs": secs })
                );
            } else {
                println!(
                    "Pairing open for {secs} s. PIN: {}  (it crosses the network in plain \
                     text: pair on a network you trust)",
                    window.pin
                );
            }
        }
        Err(e) => eprintln!("dspi hub: no pairing: {e}"),
    }
}

fn devices(running: &Running, json: bool) {
    let devices = running.handle.devices();
    if json {
        let list: Vec<Value> = devices
            .iter()
            .map(|d| {
                json!({
                    "handle": d.handle,
                    "serial": d.identity.serial,
                    "name": d.name,
                    "online": d.state == DeviceState::Online,
                })
            })
            .collect();
        println!("{}", json!({ "event": "devices", "devices": list }));
        return;
    }
    if devices.is_empty() {
        println!("no devices");
    }
    for d in &devices {
        let state = match d.state {
            DeviceState::Online => "online",
            DeviceState::Offline => "offline",
        };
        println!(
            "{:>3}  {}  {state:<7}  {}",
            d.handle,
            printable(&d.identity.serial),
            printable(&d.name)
        );
    }
}

/// How long ago, roughly.
fn ago(then: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(then)
        .unwrap_or_default()
        .as_secs();
    match secs {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86_400),
    }
}

fn unix(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// The client list; `online` only for a running hub.
fn print_clients(clients: &[PairedClient], online: Option<&dyn Fn(u32) -> bool>, json: bool) {
    if json {
        let list: Vec<Value> = clients
            .iter()
            .map(|c| {
                json!({
                    "cid": c.cid,
                    "name": c.name,
                    "role": role_name(c.role),
                    "created": unix(c.created),
                    "lastSeen": c.last_seen.map(unix),
                    // Only a running hub knows.
                    "online": online.map(|f| f(c.cid)),
                })
            })
            .collect();
        println!("{}", json!({ "event": "clients", "clients": list }));
        return;
    }
    if clients.is_empty() {
        println!("no paired clients");
    }
    for c in clients {
        let seen = c.last_seen.map_or_else(|| "never".to_owned(), ago);
        let live = match online {
            Some(f) if f(c.cid) => "  connected",
            _ => "",
        };
        println!(
            "{:>4}  {:<7}  {}  (last seen {seen}){live}",
            c.cid,
            role_name(c.role),
            printable(&c.name),
        );
    }
}

/// `dspi hub clients`, with no hub running.
fn offline_clients(store: &Path, json: bool) -> Result<(), Failure> {
    match SavedHub::peek(store)? {
        Some(saved) => print_clients(&saved.clients, None, json),
        None if json => println!("{}", json!({ "event": "clients", "clients": [] })),
        None => println!("no hub has run with {} yet", store.display()),
    }
    Ok(())
}

/// `dspi hub role|revoke`, with no hub running: through the hub's own store code,
/// in the mode it last ran in, so nothing else changes.
fn offline_edit(
    store: &Path,
    cid: u32,
    edit: impl FnOnce(&AuthStore) -> bool,
) -> Result<(), Failure> {
    let Some(saved) = SavedHub::peek(store)? else {
        return Err(Failure::Usage(format!(
            "no hub has run with {} yet",
            store.display()
        )));
    };
    let _lock = lock_store(store)?;
    let mode = if saved.access.as_deref() == Some(AccessMode::Open.wire_name()) {
        AccessMode::Open
    } else {
        AccessMode::Pin
    };
    let auth = AuthStore::new(HubStore::load(store)?, mode)?;
    if edit(&auth) {
        Ok(())
    } else {
        Err(Failure::Usage(format!("no paired client {cid}")))
    }
}

/// Keeping the machine awake while the hub shares, released when dropped.
///
/// Each OS's own way, with nothing to install: on Linux `systemd-inhibit`, which
/// holds a logind sleep lock for as long as its `cat` reads our pipe, so the lock
/// goes with this process however it ends; on macOS `caffeinate -i -w <pid>`, which
/// ends with this process; on Windows `SetThreadExecutionState`, for the thread
/// that serves. Elsewhere, or if refused (no logind, as in a container), a warning
/// and the hub runs anyway.
mod awake {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::process::{Child, Command, Stdio};

    /// The Linux inhibitor's command line.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(super) fn linux_command() -> (&'static str, [&'static str; 5]) {
        (
            "systemd-inhibit",
            [
                "--what=sleep:idle",
                "--who=dspi hub",
                "--why=Sharing DSPi devices on the network",
                "--mode=block",
                "cat",
            ],
        )
    }

    pub struct KeepAwake {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        child: Child,
    }

    impl KeepAwake {
        /// `None` with a warning when it cannot be had.
        pub fn begin() -> Option<Self> {
            match Self::try_begin() {
                Ok(k) => Some(k),
                Err(e) => {
                    eprintln!(
                        "dspi hub: warning: cannot keep the machine awake ({e}); \
                         sharing stops if it sleeps"
                    );
                    None
                }
            }
        }

        #[cfg(target_os = "linux")]
        fn try_begin() -> Result<Self, String> {
            let (program, args) = linux_command();
            let mut child = Command::new(program)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("{program}: {e}"))?;
            // A refusal (no logind, no permission) ends it at once.
            std::thread::sleep(std::time::Duration::from_millis(300));
            match child.try_wait() {
                Ok(None) => Ok(Self { child }),
                Ok(Some(status)) => Err(format!("{program} refused ({status})")),
                Err(e) => Err(format!("{program}: {e}")),
            }
        }

        #[cfg(target_os = "macos")]
        fn try_begin() -> Result<Self, String> {
            let pid = std::process::id().to_string();
            let child = Command::new("/usr/bin/caffeinate")
                .args(["-i", "-w", &pid])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("caffeinate: {e}"))?;
            Ok(Self { child })
        }

        #[cfg(windows)]
        fn try_begin() -> Result<Self, String> {
            use windows_sys::Win32::System::Power::{
                ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
            };
            // SAFETY: takes flags by value and touches no memory of ours.
            let previous = unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
            if previous == 0 {
                return Err("SetThreadExecutionState failed".into());
            }
            Ok(Self {})
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        fn try_begin() -> Result<Self, String> {
            Err("not supported on this system".into())
        }
    }

    impl Drop for KeepAwake {
        fn drop(&mut self) {
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                // On Linux closing the pipe would end `cat` too; a kill is quicker.
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
            #[cfg(windows)]
            {
                use windows_sys::Win32::System::Power::{ES_CONTINUOUS, SetThreadExecutionState};
                // SAFETY: as in `try_begin`. Same thread: `Running` lives on the one
                // that serves.
                unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_tm11 {
    use super::*;
    use dspi_tui::shell::connect::{LinkTarget, OpenError, no_prompt};

    fn scratch(name: &str) -> PathBuf {
        // Beside the test binary, in the target directory: never on tmpfs.
        let dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("wp_tm11")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn parsed(args: &[&str]) -> Result<HubCommand, Failure> {
        parse(args, false, Some(PathBuf::from("/store")))
    }

    fn demo_options(store: &Path) -> RunOptions {
        RunOptions {
            name: Some("TM11 test hub".into()),
            port: 0,
            access: AccessMode::Pin,
            store: store.to_owned(),
            loopback: true,
            pair: false,
            demo: true,
            keep_awake: false,
            forget_clients: false,
        }
    }

    /// The Terminal's own way to a hub, as `dspi --link 127.0.0.1:PORT [--pin PIN]`.
    fn client(port: u16, pin: Option<&str>, tokens: &Path) -> Target {
        let mut l = LinkTarget::parse(&format!("127.0.0.1:{port}")).unwrap();
        l.pin = pin.map(str::to_owned);
        l.tokens = Some(tokens.join("link-tokens.toml"));
        Target::Link(l)
    }

    fn pin_of(running: &Running) -> String {
        running
            .control
            .auth()
            .begin_pairing(Instant::now())
            .unwrap()
            .pin
    }

    #[test]
    fn wp_tm11_a_bare_hub_runs_with_pin_pairing_on_the_standard_port() {
        let HubCommand::Run(o) = parsed(&[]).unwrap() else {
            panic!("not a run");
        };
        assert_eq!(o.access, AccessMode::Pin, "PIN pairing is the default");
        assert_eq!(o.port, DEFAULT_PORT);
        assert_eq!(o.store, PathBuf::from("/store"));
        assert!(o.keep_awake && !o.loopback && !o.pair && !o.demo && !o.forget_clients);
        assert_eq!(parsed(&["run"]).unwrap(), parsed(&[]).unwrap());
    }

    #[test]
    fn wp_tm11_run_options_parse() {
        let HubCommand::Run(o) = parse(
            &[
                "--name",
                " Studio ",
                "--port=0",
                "--open",
                "--loopback",
                "--pair",
                "--no-keep-awake",
                "--forget-clients",
                "--store",
                "/s",
                "--json",
            ],
            true,
            None,
        )
        .unwrap() else {
            panic!("not a run");
        };
        assert_eq!(o.name.as_deref(), Some("Studio"));
        assert_eq!(o.port, 0);
        assert_eq!(o.access, AccessMode::Open);
        assert_eq!(o.store, PathBuf::from("/s"));
        assert!(o.loopback && o.pair && o.demo && o.forget_clients && !o.keep_awake);
    }

    #[test]
    fn wp_tm11_client_commands_parse() {
        assert_eq!(
            parsed(&["clients"]).unwrap(),
            HubCommand::Clients {
                store: "/store".into()
            }
        );
        assert_eq!(
            parsed(&["role", "3", "viewer"]).unwrap(),
            HubCommand::Role {
                store: "/store".into(),
                cid: 3,
                role: Role::Viewer
            }
        );
        assert_eq!(
            parsed(&["revoke", "7", "--store", "/x"]).unwrap(),
            HubCommand::Revoke {
                store: "/x".into(),
                cid: 7
            }
        );
        assert_eq!(parsed(&["--help"]).unwrap(), HubCommand::Help);
        assert_eq!(parsed(&["help"]).unwrap(), HubCommand::Help);
    }

    #[test]
    fn wp_tm11_bad_hub_command_lines_are_usage_errors() {
        for bad in [
            &["role", "3", "owner"][..],
            &["role", "x", "admin"],
            &["role"],
            &["revoke"],
            &["revoke", "1", "2"],
            &["clients", "--open"],
            &["revoke", "1", "--port", "9"],
            &["--port", "99999"],
            &["--name", "\u{7}  "],
            &["--bogus"],
            &["pair"],
            &["--store"],
        ] {
            assert!(
                matches!(parsed(bad), Err(Failure::Usage(_))),
                "{bad:?} should be refused"
            );
        }
        assert!(matches!(
            parse(&["clients"], true, None),
            Err(Failure::Usage(_))
        ));
        assert!(
            matches!(parse(&[], false, None), Err(Failure::Usage(_))),
            "no data directory and no --store"
        );
    }

    /// TM9's `--hub [NAME]` takes the next word as a name unless it is a command:
    /// `hub` must stay the command.
    #[test]
    fn wp_tm11_hub_the_command_does_not_clash_with_the_hub_flag() {
        let (target, rest) = crate::parse_target(&["--hub", "hub"]).unwrap();
        let Target::Link(l) = target else {
            panic!("--hub is a link");
        };
        assert_eq!(l.find.unwrap().wanted, None);
        assert_eq!(rest, ["hub"]);

        let (target, rest) = crate::parse_target(&["hub", "--port", "9", "--demo"]).unwrap();
        assert_eq!(target, Target::Demo);
        assert_eq!(rest, ["hub", "--port", "9"]);
        assert_eq!(
            run(&Target::Link(LinkTarget::find(None)), &["hub"], false),
            exit::USAGE,
            "dspi --hub hub: a hub is not run on another hub"
        );
    }

    /// Review c4b: a read error (a background job's EIO) is not the end of input; the
    /// lines after it still arrive, and the end ends it.
    #[test]
    fn wp_tm11b_a_failed_read_is_tried_again_not_taken_for_the_end() {
        struct Flaky(Vec<std::io::Result<&'static [u8]>>);
        impl std::io::Read for Flaky {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.0.is_empty() {
                    return Ok(0);
                }
                let chunk = self.0.remove(0)?;
                buf[..chunk.len()].copy_from_slice(chunk);
                Ok(chunk.len())
            }
        }
        let eio = || Err(std::io::Error::from_raw_os_error(5));
        let input = Flaky(vec![eio(), Ok(b"clients\n"), eio(), eio(), Ok(b"quit\r\n")]);
        let (tx, rx) = mpsc::channel();
        pump_lines(std::io::BufReader::new(input), &tx, Duration::ZERO);
        drop(tx);
        assert_eq!(rx.iter().collect::<Vec<_>>(), ["clients", "quit"]);
    }

    /// Review c4b: `dspi --hub hub` is refused before anything looks at the network.
    #[test]
    fn wp_tm11b_a_hub_on_another_target_is_refused_up_front() {
        assert_eq!(refuse_target(&Target::Usb { serial: None }), None);
        assert_eq!(refuse_target(&Target::Demo), None);
        for t in [
            Target::Link(LinkTarget::find(None)),
            Target::Midi { port: None },
            Target::Usb {
                serial: Some("X".into()),
            },
        ] {
            assert_eq!(refuse_target(&t), Some(exit::USAGE), "{t:?}");
        }
    }

    /// HB6's rule for this binary's own logger: `RUST_LOG=trace` gets everything
    /// except tungstenite's frame dumps, PINs and tokens in them. -- WP TM26 -- And a
    /// ceiling only: tungstenite never gets more than was asked for (hw-1 M3).
    #[test]
    fn wp_tm11_the_logger_holds_back_the_frame_dumps() {
        use log::{Level, Log, Metadata};
        let logger_for = |asked: &str| {
            let mut b = env_logger::Builder::new();
            b.parse_filters(asked);
            CappedLogger::new(b)
        };
        let at = |logger: &CappedLogger, target, level| {
            logger.enabled(&Metadata::builder().target(target).level(level).build())
        };
        for asked in [
            "trace",
            "tungstenite=trace",
            "info,tungstenite::protocol=trace",
        ] {
            let logger = logger_for(asked);
            assert!(
                !at(&logger, "tungstenite::protocol", Level::Trace),
                "{asked}"
            );
            assert!(
                at(&logger, "tungstenite::protocol", Level::Debug),
                "{asked}"
            );
        }
        let logger = logger_for("trace");
        assert!(at(&logger, "dspi_hub::server", Level::Trace));
        assert!(
            at(&logger, "tungstenite_like", Level::Trace),
            "not tungstenite"
        );
        // -- WP TM26 -- Below debug, what was asked for stands.
        for (asked, top) in [
            ("info", Level::Info),
            ("warn", Level::Warn),
            ("error", Level::Error),
            ("debug,tungstenite=warn", Level::Warn),
        ] {
            let logger = logger_for(asked);
            for level in [
                Level::Error,
                Level::Warn,
                Level::Info,
                Level::Debug,
                Level::Trace,
            ] {
                assert_eq!(
                    at(&logger, "tungstenite::protocol", level),
                    level <= top,
                    "{asked}: tungstenite at {level}"
                );
            }
        }
        let logger = logger_for("off");
        assert!(!at(&logger, "tungstenite::protocol", Level::Error));
    }

    #[test]
    fn wp_tm11_linux_sleep_inhibit_holds_sleep_while_reading_our_pipe() {
        let (program, args) = awake::linux_command();
        assert_eq!(program, "systemd-inhibit");
        assert!(args.contains(&"--what=sleep:idle"));
        assert!(args.contains(&"--mode=block"));
        assert_eq!(args.last(), Some(&"cat"), "ends with our stdin");
    }

    /// The done criterion: a Link client, the Terminal's own, pairs with a PIN, reads
    /// the device, comes back on its token, and a revoke shuts it out.
    #[test]
    fn wp_tm11_a_link_client_pairs_connects_and_is_revoked() {
        let store = scratch("connects");
        let running = start(&demo_options(&store), false).unwrap();
        let port = running.control.port();
        assert_ne!(port, 0);
        assert_eq!(running.handle.hub_id().len(), 36);

        // No PIN, no entry.
        let refused = client(port, None, &store).open_session(&no_prompt);
        assert!(
            matches!(refused, Err(OpenError::NeedsPin { .. })),
            "{:?}",
            refused.err()
        );

        let pin = pin_of(&running);
        let connected = client(port, Some(&pin), &store)
            .connect(&no_prompt)
            .unwrap();
        assert!(connected.link.is_some(), "reached through the hub");
        assert!(!connected.session.capabilities().channels.is_empty());
        drop(connected);

        let clients = running.control.auth().clients();
        assert_eq!(clients.len(), 1);
        let cid = clients[0].cid;
        assert_eq!(clients[0].name, "DSPi Terminal");

        // Back on the stored token: no PIN.
        let listed = client(port, None, &store).list(&no_prompt).unwrap();
        assert_eq!(listed.len(), 1, "the demo device");

        assert!(command(&running, &format!("role {cid} viewer"), false));
        assert_eq!(running.control.auth().role_of(cid), Some(Role::Viewer));
        assert!(command(&running, &format!("revoke {cid}"), false));
        assert!(running.control.auth().clients().is_empty());
        let after = client(port, None, &store).open_session(&no_prompt);
        assert!(after.is_err(), "a revoked token is refused");
        assert!(!command(&running, "quit", false));
    }

    /// One owner of the store: the offline commands wait for the hub to stop, then
    /// change what it will start with.
    #[test]
    fn wp_tm11_offline_commands_refuse_a_running_store_and_work_on_a_stopped_one() {
        let store = scratch("offline");
        assert!(matches!(
            offline_edit(&store.join("never"), 1, |_| true),
            Err(Failure::Usage(_))
        ));
        let running = start(&demo_options(&store), false).unwrap();
        let port = running.control.port();
        let pin = pin_of(&running);
        client(port, Some(&pin), &store)
            .open_session(&no_prompt)
            .unwrap();
        let cid = running.control.auth().clients()[0].cid;

        let busy = offline_edit(&store, cid, |a| a.set_role(cid, Role::Viewer));
        assert!(matches!(busy, Err(Failure::Usage(m)) if m.contains("a hub is running")));
        assert!(
            matches!(start(&demo_options(&store), false), Err(Failure::Usage(_))),
            "two hubs on one store"
        );
        offline_clients(&store, true).unwrap();
        drop(running);

        offline_edit(&store, cid, |a| a.set_role(cid, Role::Control)).unwrap();
        let saved = SavedHub::peek(&store).unwrap().unwrap();
        assert_eq!(saved.clients[0].role, Role::Control);
        assert_eq!(
            saved.access.as_deref(),
            Some("pin"),
            "the mode is left alone"
        );
        assert!(matches!(
            offline_edit(&store, 999, |a| a.revoke(999)),
            Err(Failure::Usage(_))
        ));
        offline_edit(&store, cid, |a| a.revoke(cid)).unwrap();
        assert!(SavedHub::peek(&store).unwrap().unwrap().clients.is_empty());
    }

    /// Switching to open access forgets every paired client: refused unless asked for.
    #[test]
    fn wp_tm11_a_change_of_access_that_forgets_clients_needs_asking() {
        let store = scratch("mode");
        let running = start(&demo_options(&store), false).unwrap();
        let pin = pin_of(&running);
        client(running.control.port(), Some(&pin), &store)
            .open_session(&no_prompt)
            .unwrap();
        drop(running);

        let mut open = demo_options(&store);
        open.access = AccessMode::Open;
        let refused = start(&open, false).err().unwrap();
        assert!(matches!(&refused, Failure::Usage(m) if m.contains("--forget-clients")));
        assert_eq!(SavedHub::peek(&store).unwrap().unwrap().clients.len(), 1);

        open.forget_clients = true;
        let running = start(&open, false).unwrap();
        assert_eq!(running.control.auth().tokens_revoked_at_start(), 1);
        // Open: anyone connects, with no PIN.
        client(running.control.port(), None, &store.join("other"))
            .open_session(&no_prompt)
            .unwrap();
    }

    #[test]
    fn wp_tm11_the_name_is_kept_for_the_next_start() {
        let store = scratch("name");
        drop(start(&demo_options(&store), false).unwrap());
        let mut unnamed = demo_options(&store);
        unnamed.name = None;
        let running = start(&unnamed, false).unwrap();
        assert_eq!(running.name, "TM11 test hub");
        assert_eq!(running.handle.name(), "TM11 test hub");
    }
}
