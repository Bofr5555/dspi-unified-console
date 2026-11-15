//! `uconsole hub`: share this machine's DSPi devices on the network, with no window.
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
//!   reach the live sessions), or `uconsole hub clients|role|revoke` with the hub stopped.
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
//! -- WP WB7 -- **The web console.** `--web` serves the hub's own page at `/` and the web
//! console (`apps/web`'s built dist) at `/console/`, read from `--console DIR`, else from
//! `$DSPI_WEB_CONSOLE`, else from `web/` beside this binary or `../share/uconsole/web/` above
//! it. `uconsole web` (the same as `uconsole hub --local`) runs a hub on this machine only with
//! the console, so `http://localhost:PORT/console/` is a secure page: WebUSB and Link in
//! one browser tab. That hub holds no USB device unless `--share-usb`, so the browser can
//! take a DSPi over WebUSB; `--demo` shares the simulated device over Link.
//!
//! Owned by WP TM11 (docs/core-merge/PLAN-CLIENTS.md).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_cmd::COMMAND;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{BufRead, IsTerminal};
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime};

use dspi_hub::auth::{AuthError, AuthStore};
use dspi_hub::store::HubStore;
use dspi_hub::{
    AccessMode, DEFAULT_PORT, DeviceState, Hub, HubConfig, HubError, HubEvent, HubHandle,
    HubListener, NetworkConfig, NetworkControl, PairedClient, Role, SavedHub, UsbBus,
};
// -- WP P2TM --
use dspi_hub::{ClientKind, HealthConfig, HistoryConfig, MintedKey, ScheduleConfig};
use dspi_transport::FakeDevice;
// -- WP SW3 --
use dspi_proto::ext::alink::AlinkRole;
use dspi_transport::fake::{AlinkSetup, UsbDirectSetup};
use dspi_tui::shell::connect::{Target, printable};
use serde_json::{Value, json};

use crate::exit;

/// What a hub is called when nobody has named it (as HB6's).
const DEFAULT_NAME: &str = "DSPi Hub";
/// Longest hub name kept, in characters: it goes to every client (as HB6's).
const MAX_NAME: usize = 64;
/// Held by a running hub, in its store.
const LOCK_FILE: &str = "dspi-hub.lock";
// -- WP PAIRADM --
/// Shown with a pairing PIN: what the client becomes, and how admin is given.
const PAIRED_ROLE_NOTE: &str = concat!(
    "A client that pairs with this PIN gets at most the control role. To make it an ",
    "Admin, type `role <cid> admin` here (`clients` lists them), or run `",
    dspi_cmd::command!(),
    " hub role <cid> admin` with the hub stopped."
);
/// The refusal of `--pair-admin`, gone since 2026-11-14 (owner): no pairing
/// gives admin; the role is set on the hub's machine, and applies live.
const PAIR_ADMIN_GONE: &str = concat!(
    "--pair-admin is no longer supported: pairing gives at most the control role. ",
    "To make a console an Admin, pair it (--pair), then on this machine run `",
    dspi_cmd::command!(),
    " hub clients` and `",
    dspi_cmd::command!(),
    " hub role <cid> admin` with the hub stopped, or type `clients` and `role <cid> admin` ",
    "at the running hub's prompt, which applies at once"
);
// -- WP WB7 --
/// `uconsole web`'s port when nobody says: the standard one plus one, so it can run beside
/// a `uconsole hub`, and the same each time, so the page's origin (and with it the hub
/// tokens and the DSPis it was allowed on USB) stays the browser's.
const LOCAL_PORT: u16 = DEFAULT_PORT + 1;
/// The environment variable naming the web console's directory.
const CONSOLE_ENV: &str = "DSPI_WEB_CONSOLE";
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
                eprintln!("{COMMAND} hub: {m}");
                exit::USAGE
            }
            Failure::Failed(m) => {
                eprintln!("{COMMAND} hub: {m}");
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

/// How `uconsole hub run` starts.
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
    // -- WP WB7 --
    /// `--web`: a landing page at `/`, the console at `/console/`, the hub's own page
    /// (upstream's DSPi Link page) at `/hub/`.
    web: bool,
    /// `--console DIR`: where the console is; else [`find_console`] looks.
    console: Option<PathBuf>,
    /// `uconsole web`, `--local`: loopback, web on, the console required.
    local: bool,
    /// `--share-usb` with `--local`: the hub holds this machine's USB devices.
    share_usb: bool,
    // -- WP SW3 --
    /// `--demo --alink`: an installation of simulated devices on the audio
    /// link (a sender, a fan-out, four units; WP AUSB1: and two USB-direct
    /// units) with its system model.
    alink: bool,
    // -- WP HWLK1F --
    /// `--with-usb` (a test option, with `--demo --alink`): this machine's
    /// USB devices too, beside the simulated installation.
    with_usb: bool,
    /// `--with-plain-unit` (a test option, with `--demo --alink`): one
    /// more simulated DSPi, without the audio link, in the installation.
    with_plain_unit: bool,
    // -- WP AUSB2 --
    /// `--with-usb-lock-error` (a test option, with `--demo --alink`): the
    /// USB-direct unit Rear L/R's host is out of range from the start, so it
    /// is in lock error (muted, or unaligned as set).
    with_usb_lock_error: bool,
    // -- WP P2TM --
    /// `--rest`: the REST mirror under `/api/v1/`.
    rest: bool,
    /// `--data <dir>`: the services' folder; `None`: the store.
    data: Option<PathBuf>,
    /// Which phase-2 services run (`--no-<name>` turns one off).
    services: Services,
}

// -- WP P2TM --
/// The phase-2 services `uconsole hub` runs: every one unless turned off.
/// The parameter API, snapshots, statistics, MQTT and notifications out are
/// always registered (MQTT and notifications stay off until an Admin sets
/// them up).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Services {
    health: bool,
    inventory: bool,
    identify: bool,
    history: bool,
    alarms: bool,
    schedule: bool,
}

impl Default for Services {
    fn default() -> Self {
        Self {
            health: true,
            inventory: true,
            identify: true,
            history: true,
            alarms: true,
            schedule: true,
        }
    }
}

impl Services {
    /// Into the hub's switches, at their defaults.
    fn apply(self, config: &mut dspi_hub::ServiceConfig) {
        config.health = self.health.then(HealthConfig::default);
        config.inventory = self.inventory;
        config.identify = self.identify;
        config.history = self.history.then(HistoryConfig::default);
        config.alarms = self.alarms;
        config.schedule = self.schedule.then(ScheduleConfig::default);
    }
}

/// One `uconsole hub` command.
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
    // -- WP P2TM --
    ApiKeyNew {
        store: PathBuf,
        name: String,
        role: Role,
    },
    ApiKeyList {
        store: PathBuf,
    },
    ApiKeyRevoke {
        store: PathBuf,
        cid: u32,
    },
}

/// `uconsole hub ...`; `flags` starts at the word `hub`, without the target flags.
pub fn run(target: &Target, flags: &[&str], json: bool) -> u8 {
    if let Some(code) = refuse_target(target) {
        return code;
    }
    let demo = *target == Target::Demo;
    // -- WP WB7 -- `uconsole web` is `uconsole hub --local`, with a store of its own, so it can
    // run beside a `uconsole hub`.
    let local = flags.first() == Some(&"web");
    let mut args: Vec<&str> = flags.get(1..).unwrap_or_default().to_vec();
    let store = if local {
        args.insert(0, "--local");
        default_store().and_then(|d| d.parent().map(|p| p.join("web")))
    } else {
        default_store()
    };
    let command = match parse(&args, demo, store) {
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
        // -- WP P2TM --
        HubCommand::ApiKeyNew { store, name, role } => {
            offline_auth(&store, |auth| mint_key(auth, &name, role))
                .map(|key| print_minted(&key, json))
        }
        HubCommand::ApiKeyList { store } => offline_keys(&store, json),
        HubCommand::ApiKeyRevoke { store, cid } => {
            offline_auth(&store, |auth| revoke_key(auth, cid, |c| auth.revoke(c)))
                .map(|()| say_key_revoked(cid, json))
        }
    };
    match done {
        Ok(()) => exit::OK,
        Err(e) => e.report(),
    }
}

/// A target a hub cannot run on, refused with its exit status. `main` asks before it
/// looks for a `--hub` on the network, so `uconsole --hub hub` fails at once (review c4b).
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
{COMMAND} hub - share this machine's DSPi devices on the network (DSPi Link)

USAGE:
    {COMMAND} hub [run] [OPTIONS]  run a hub in the foreground until Ctrl-C or SIGTERM
    {COMMAND} --demo hub [...]     the same, sharing the built-in simulated device
    {COMMAND} [--demo] web         the web console on this machine ({COMMAND} hub --local)
    {COMMAND} hub clients          list the paired clients (hub stopped)
    {COMMAND} hub role <cid> viewer|control|admin
                              change a client's role (hub stopped)
    {COMMAND} hub revoke <cid>     forget a client; it must pair again (hub stopped)
    {COMMAND} hub api-key new <name> [viewer|control|admin]
                              mint an API key for REST (default control); the
                              key is shown once (hub stopped; while it runs,
                              type `api-key new ...` at its prompt)
    {COMMAND} hub api-key list     the API keys (hub stopped)
    {COMMAND} hub api-key revoke <cid>
                              revoke an API key (hub stopped)

OPTIONS:
    --name <name>             the hub's name, kept for the next start
    --port <port>             the port to listen on (default {DEFAULT_PORT}; 0: any)
    --pair                    open a pairing window at the start and show its PIN.
                              A PIN gives at most the control role. Admin:
                              `{COMMAND} hub role <cid> admin` on this
                              machine, or `role <cid> admin` while it runs
    --open                    no PIN: every connection may do anything, including
                              anyone else on this network. PIN pairing is the default
    --forget-clients          agree to a change of --open that forgets every
                              paired client
    --web                     serve web pages to browsers on this network: the
                              web console at /console/, the hub's own page
                              (DSPi Link's) at /hub/, and a page at / linking both
    --console <dir>           the built web console (apps/web's dist); implies
                              --web. Else ${CONSOLE_ENV}, else web/ beside this
                              program or ../share/uconsole/web/ above it
    --local                   the same as `{COMMAND} web`: this machine only, with
                              the console at http://localhost:PORT/console/
                              (default port {LOCAL_PORT}), a secure page with
                              WebUSB and Link together. The hub holds no USB
                              device, so the browser can; --share-usb: it does
    --alink                   with --demo: an installation on the audio link instead
                              of the one device (a sender, a fan-out, four
                              units and two USB-direct units), and its system
                              model when none is kept
    --with-usb                test option, with --demo --alink: this
                              machine's USB DSPis too, to test a mixed
                              installation on hardware (add them to the model)
    --with-plain-unit         test option, with --demo --alink: one more
                              simulated DSPi without the audio link (unit P)
    --with-usb-lock-error     test option, with --demo --alink: the USB-direct
                              unit Rear L/R can't lock (its host out of range)
    --loopback                listen on this machine only, and do not advertise
    --no-keep-awake           let the machine sleep while sharing
    --store <dir>             where the hub keeps its id, clients and device names
                              (default: {})
    --json                    one JSON object per line
    --rest                    the REST mirror at /api/v1/ on the same port, for
                              API keys (`api-key new`); off by default
    --data <dir>              where the services keep their files (inventory,
                              history, alarms, schedules, snapshots, MQTT,
                              notifications); default: the store
    --no-health               no health poll of the devices (health.get)
    --no-inventory            no inventory of every device seen (inventory.*)
    --no-identify             no Identify (device.identify)
    --no-history              no shared event history (event.*)
    --no-alarms               no device alarms (alarm.*)
    --no-schedule             no schedules (schedule.*)

WHILE IT RUNS, on a terminal, type:
    pair                      open pairing for 2 minutes and show the PIN
    unpair                    close the pairing window
    clients                   the paired clients, and which are connected
    role <cid> <role>         change a client's role, live
    revoke <cid>              forget a client and disconnect it
    devices                   the devices being shared
    inventory                 every device the hub has seen, online or not
    alarms                    the alarms that are on
    schedule                  the schedule's rules and their next runs
    api-key new <name> [role] mint an API key (default control), shown once
    api-key list              the API keys
    api-key revoke <cid>      revoke an API key, at once
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
// -- WP P0E -- `uconsole diag` includes this store, its secrets removed.
pub(crate) fn default_store() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join(COMMAND).join("hub"))
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
    // -- WP WB7 --
    let (mut web, mut console, mut local, mut share_usb) = (false, None, false, false);
    // -- WP SW3 --
    let mut alink = false;
    // -- WP HWLK1F --
    let (mut with_usb, mut with_plain_unit) = (false, false);
    let mut with_usb_lock_error = false; // -- WP AUSB2 --
    // -- WP P2TM --
    let (mut rest_api, mut data, mut services) = (false, None, Services::default());
    let mut port_given = false;
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
                port_given = true;
                run_only.push("--port");
            }
            "--open" => access = AccessMode::Open,
            "--loopback" => loopback = true,
            "--pair" => pair = true,
            "--no-keep-awake" => keep_awake = false,
            "--forget-clients" => forget = true,
            // -- WP WB7 --
            "--web" => web = true,
            "--console" => {
                console = Some(PathBuf::from(value("the web console's directory")?));
                run_only.push("--console");
            }
            "--local" => local = true,
            "--share-usb" => share_usb = true,
            // -- WP SW3 --
            // -- WP ALINK1 -- `--alink`; `--audio-link`, its name before, is a hidden alias
            "--alink" | "--audio-link" => alink = true,
            // -- WP PAIRADM -- gone (owner, 2026-11-14): refused, never ignored
            "--pair-admin" => return Err(usage(PAIR_ADMIN_GONE.into())),
            "--with-usb" => with_usb = true,
            "--with-plain-unit" => with_plain_unit = true,
            "--with-usb-lock-error" => with_usb_lock_error = true,
            // -- WP P2TM --
            "--rest" => rest_api = true,
            "--data" => {
                data = Some(PathBuf::from(value("a directory")?));
                run_only.push("--data");
            }
            "--no-health" => services.health = false,
            "--no-inventory" => services.inventory = false,
            "--no-identify" => services.identify = false,
            "--no-history" => services.history = false,
            "--no-alarms" => services.alarms = false,
            "--no-schedule" => services.schedule = false,
            f if f.starts_with('-') => return Err(usage(format!("`{f}` is not a hub option"))),
            word => words.push(word),
        }
        if matches!(
            flag,
            "--open"
                | "--loopback"
                | "--pair"
                | "--no-keep-awake"
                | "--forget-clients"
                | "--web"
                | "--local"
                | "--share-usb"
                | "--alink"
                | "--audio-link"
                | "--with-usb"
                | "--with-plain-unit"
                | "--with-usb-lock-error"
                // -- WP P2TM --
                | "--rest"
                | "--no-health"
                | "--no-inventory"
                | "--no-identify"
                | "--no-history"
                | "--no-alarms"
                | "--no-schedule"
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
        let w = w.ok_or_else(|| {
            usage(format!(
                "which client? Give its number from `{COMMAND} hub clients`"
            ))
        })?;
        w.parse()
            .map_err(|_| usage(format!("`{w}` is not a client number")))
    };
    let command = match words.as_slice() {
        [] | ["run"] => {
            // -- WP WB7 --
            if share_usb && !local {
                return Err(usage(format!(
                    "--share-usb is for `{COMMAND} web`; a hub shares USB anyway"
                )));
            }
            if share_usb && demo {
                return Err(usage(
                    "--share-usb and --demo: the demo hub holds no USB device".into(),
                ));
            }
            // -- WP SW3 --
            if alink && !demo {
                return Err(usage(format!(
                    "--alink is for `{COMMAND} --demo hub`: it shares simulated devices"
                )));
            }
            // -- WP HWLK1F --
            for (given, flag) in [
                (with_usb, "--with-usb"),
                (with_plain_unit, "--with-plain-unit"),
                (with_usb_lock_error, "--with-usb-lock-error"),
            ] {
                if given && !alink {
                    return Err(usage(format!(
                        "{flag} is a test option of `{COMMAND} --demo hub --alink`"
                    )));
                }
            }
            if local && !port_given {
                port = LOCAL_PORT;
            }
            return Ok(HubCommand::Run(RunOptions {
                name,
                port,
                access,
                store: store()?,
                pair,
                demo,
                keep_awake,
                forget_clients: forget,
                // -- WP WB7 --
                web: web || local || console.is_some(),
                console,
                loopback: loopback || local,
                local,
                share_usb,
                // -- WP SW3 --
                alink,
                // -- WP HWLK1F --
                with_usb,
                with_plain_unit,
                with_usb_lock_error,
                // -- WP P2TM --
                rest: rest_api,
                data,
                services,
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
        // -- WP P2TM --
        ["api-key", "new", rest @ ..] if rest.len() <= 2 => HubCommand::ApiKeyNew {
            store: store()?,
            name: key_name(rest.first().copied())?,
            role: key_role(rest.get(1).copied())?,
        },
        ["api-key", "list"] => HubCommand::ApiKeyList { store: store()? },
        ["api-key", "revoke", rest @ ..] if rest.len() <= 1 => HubCommand::ApiKeyRevoke {
            store: store()?,
            cid: cid(rest.first())?,
        },
        _ => {
            return Err(usage(format!(
                "`{}` is not a hub command; see `{COMMAND} hub --help`",
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
    // -- WP WB7 -- where a browser finds the console, when served
    console_url: Option<String>,
    // -- WP P2TM -- `--rest`
    rest: bool,
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

    // -- WP WB7 -- Read whole before the port is bound: a wrong directory is said at once.
    let console = load_console(opts)?;
    let mut network = NetworkConfig::new(opts.store.clone());
    network.port = opts.port;
    network.access = opts.access;
    network.web = opts.web;
    network.console = console.clone();
    if opts.loopback {
        network.bind = IpAddr::V4(Ipv4Addr::LOCALHOST);
    }
    // -- WP TM26 -- The port is ours from here; clients wait until the device is in
    // (hw-1 M2: they were told "No DSPi found" for the first 0.3 s).
    network.hold_accepts = true;
    // -- WP P2TM -- the REST mirror, off unless asked
    network.rest = opts.rest;
    let mut config = HubConfig::in_process(&name);
    config.network = Some(network);
    // -- WP P2TM -- the phase-2 services, each on unless turned off
    if let Some(dir) = &opts.data {
        std::fs::create_dir_all(dir)
            .map_err(|e| Failure::Failed(format!("the data folder {}: {e}", dir.display())))?;
        config.data_dir = Some(dir.clone());
    }
    opts.services.apply(&mut config.services);
    // A scan only once the listener is in, below.
    config.initial_scan = false;
    // -- WP WB7 -- `uconsole web` leaves USB to the browser unless asked.
    if !opts.demo && (!opts.local || opts.share_usb) {
        config.bus = Some(Box::new(UsbBus::default()));
    }
    // -- WP HWLK1F -- a test option: the simulated installation and this machine's USB
    if opts.demo && opts.alink && opts.with_usb {
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
    let console_url = console.map(|_| {
        let host = if opts.loopback {
            "localhost"
        } else {
            "<this machine>"
        };
        format!(
            "http://{host}:{}{}",
            control.port(),
            dspi_hub::web::CONSOLE_PATH
        )
    });
    let running = Running {
        hub,
        handle,
        control,
        name,
        access: opts.access,
        console_url,
        rest: opts.rest, // -- WP P2TM --
        _awake: opts.keep_awake.then(awake::KeepAwake::begin).flatten(),
        _lock: lock,
    };
    // -- WP SW3 --
    if opts.alink {
        demo_alink(
            &running.handle,
            opts.with_plain_unit,
            opts.with_usb_lock_error,
        )?;
        // -- WP HWLK1F --
        if opts.with_usb {
            running.handle.scan_now();
        }
    } else if opts.demo {
        running
            .handle
            // -- WP P2CC3 -- with the diagnostics extension, so Identify works
            .attach(Box::new(demo_device()), 0)
            .map_err(|e| Failure::Failed(format!("the demo device did not attach: {e}")))?;
    } else {
        running.handle.scan_now();
    }
    // -- WP TM26 -- Attached, or found absent: either way, what a client sees now is
    // the truth.
    running.control.start_accepting();
    Ok(running)
}

// -- WP P2CC3 --
/// The demo device's own meter block (`FakeDevice::demo`'s).
const DEMO_STATUS: &[u8] = include_bytes!("../../../tests/fixtures/link/device/status_wvalue9.bin");

// -- WP P2CC3 --
/// `--demo hub`'s device: [`FakeDevice::demo`] as wp/FWX firmware
/// (`FakeOptions::diag_ext`: `EXT_IDENTIFY`, `EXT_RESET_INFO`, the clip
/// event), so the hub's Identify (`device.identify`) and health facts work on
/// it.
fn demo_device() -> FakeDevice {
    let dev = FakeDevice::new(
        dspi_transport::Capture::demo(),
        dspi_transport::FakeOptions {
            extensions: true,
            surfaces: true,
            analysis: true,
            diag_ext: true,
            ..dspi_transport::FakeOptions::default()
        },
    );
    dev.remote().set_status(DEMO_STATUS.to_vec());
    dev
}

// -- WP SW3 --
/// `--demo --alink`'s devices: (serial, place on the alink). The fan-out
/// re-emits 3 frames late; units B and C are behind it.
fn alink_rig() -> Vec<(&'static str, AlinkSetup)> {
    let at = |role, hops, hop_delay_frames| AlinkSetup {
        role,
        hops,
        hop_delay_frames,
        ..AlinkSetup::default()
    };
    vec![
        ("DEMO-ALINK-TX", at(AlinkRole::Sender, 0, 0)),
        ("DEMO-ALINK-FAN", at(AlinkRole::FanOut, 0, 3)),
        ("DEMO-ALINK-A", at(AlinkRole::Receiver, 0, 0)),
        ("DEMO-ALINK-B", at(AlinkRole::Receiver, 1, 0)),
        ("DEMO-ALINK-C", at(AlinkRole::Receiver, 1, 0)),
        ("DEMO-ALINK-D", at(AlinkRole::Receiver, 0, 0)),
    ]
}

// -- WP AUSB1 --
/// `--demo --alink`'s USB-direct units: (serial, place on the host). The
/// height unit's stream began 3 bus frames after the rear's, so aligning has a
/// start offset to equalise.
fn usb_rig() -> Vec<(&'static str, UsbDirectSetup)> {
    let at = |stream_lag_frames, origin_wraps| UsbDirectSetup {
        stream_lag_frames,
        origin_wraps,
        ..UsbDirectSetup::default()
    };
    vec![("DEMO-USB-REAR", at(0, 3)), ("DEMO-USB-HEIGHT", at(3, 11))]
}

// -- WP SW3 --
/// `--demo --alink`'s system model (`docs/integrator/audio-link.md`):
/// three rooms, a zone, a group, one source, the sender, the fan-out, four
/// units and four scenes, one of them admin-only (a test click).
fn alink_model() -> Value {
    let silent = |a: u8, b: u8| json!([a, b, 255, 255, 255, 255, 255, 255]);
    json!({
        "rooms": [
            {"id": "main", "name": "Main room"},
            {"id": "terrace", "name": "Terrace", "lipSyncUs": 0},
            {"id": "bar", "name": "Bar"}
        ],
        "zones": [{"id": "inside", "name": "Inside", "rooms": ["main", "bar"], "source": "music"}],
        "groups": [{"id": "fronts", "name": "Front of house", "units": ["A", "B"]}],
        "sources": [{"id": "music", "name": "Music", "sender": "tx", "channels": [0, 1, 2, 3, 4, 5, 6, 7]}],
        "senders": [{"id": "tx", "name": "Sender", "serial": "DEMO-ALINK-TX", "senderId": 66}],
        "fanOuts": [{"id": "fan", "name": "Fan-out", "serial": "DEMO-ALINK-FAN", "upstream": "tx", "output": 1}],
        "units": [
            {"id": "A", "name": "Main left", "serial": "DEMO-ALINK-A", "room": "main", "upstream": "tx",
             "output": 0, "slots": silent(0, 1),
             "speakers": [{"output": 0, "distanceM": 3.4}, {"output": 1, "distanceM": 2.9}]},
            {"id": "B", "name": "Main right", "serial": "DEMO-ALINK-B", "room": "main", "upstream": "fan",
             "slots": silent(0, 1)},
            {"id": "C", "name": "Terrace", "serial": "DEMO-ALINK-C", "room": "terrace", "upstream": "fan",
             "slots": silent(2, 3), "measuredLatencyUs": 120},
            {"id": "D", "name": "Bar", "serial": "DEMO-ALINK-D", "room": "bar", "upstream": "tx",
             "output": 2, "slots": silent(4, 5), "latencyUs": 40},
            // -- WP AUSB1 -- two USB-direct units on this machine's USB, phase-locked
            {"id": "R", "name": "Rear L/R", "serial": "DEMO-USB-REAR", "room": "main",
             "transport": "usb_direct", "usbMode": "phase_lock",
             "speakers": [{"output": 0, "distanceM": 3.1}, {"output": 1, "distanceM": 3.1}]},
            {"id": "H", "name": "Height", "serial": "DEMO-USB-HEIGHT", "room": "main",
             "transport": "usb_direct", "usbMode": "phase_lock",
             "speakers": [{"output": 0, "distanceM": 2.6}]}
        ],
        "scenes": [
            {"id": "evening", "name": "Evening", "actions": [{"target": "all", "op": "master_volume", "value": -1200}]},
            {"id": "quiet", "name": "Quiet inside", "actions": [
                {"target": "zone:inside", "op": "master_volume", "value": -3000},
                {"target": "room:terrace", "op": "mute"}
            ]},
            {"id": "closed", "name": "Closed", "systemMute": true, "actions": [{"target": "all", "op": "mute"}]},
            {"id": "click", "name": "Test click", "actions": [{"target": "unit:A", "op": "click", "arg": 0}]}
        ]
    })
}

// -- WP HWLK1F --
/// `--with-plain-unit`'s device: a simulated DSPi without the audio link.
const PLAIN_SERIAL: &str = "DEMO-PLAIN";

// -- WP HWLK1F --
/// `--with-plain-unit`'s unit in the demo model: in the bar, behind the sender
/// as the model has it, with a speaker; its firmware has no alink.
fn plain_unit_model() -> Value {
    json!({"id": "P", "name": "Plain unit", "serial": PLAIN_SERIAL, "room": "bar",
           "upstream": "tx", "speakers": [{"output": 0, "distanceM": 2.0}]})
}

// -- WP SW3 --
/// Attach `--demo --alink`'s devices and give the hub their model, unless
/// its store keeps one already (an installer's edits survive a restart).
fn demo_alink(handle: &HubHandle, plain_unit: bool, usb_lock_error: bool) -> Result<(), Failure> {
    for (serial, setup) in alink_rig() {
        handle
            .attach(
                Box::new(FakeDevice::demo_alink_as(setup).with_serial(serial)),
                0,
            )
            .map_err(|e| Failure::Failed(format!("{serial} did not attach: {e}")))?;
    }
    // -- WP AUSB1 -- the USB-direct units (`extension_channel_spec.md` 2.9)
    for (serial, setup) in usb_rig() {
        let dev = FakeDevice::demo_usb_direct(setup).with_serial(serial);
        // -- WP AUSB2 -- its host out of range: mode 2 in lock error
        if usb_lock_error && serial == "DEMO-USB-REAR" {
            dev.remote()
                .usb_direct_sim(dspi_transport::fake::USB_SIM_HOST_RANGE);
        }
        handle
            .attach(Box::new(dev), 0)
            .map_err(|e| Failure::Failed(format!("{serial} did not attach: {e}")))?;
    }
    // -- WP HWLK1F -- a unit whose firmware has no audio link, as a K340 on 470a127
    if plain_unit {
        handle
            .attach(Box::new(FakeDevice::demo().with_serial(PLAIN_SERIAL)), 0)
            .map_err(|e| Failure::Failed(format!("{PLAIN_SERIAL} did not attach: {e}")))?;
    }
    if handle.system_model().0.units.is_empty() {
        let mut value = alink_model();
        if plain_unit && let Some(units) = value.get_mut("units").and_then(Value::as_array_mut) {
            units.push(plain_unit_model());
        }
        let model = serde_json::from_value(value)
            .map_err(|e| Failure::Failed(format!("the demo's system model: {e}")))?;
        handle
            .system_set_model(model)
            .map_err(|e| Failure::Failed(format!("the demo's system model: {e}")))?;
    }
    Ok(())
}

// -- WP WB7 --
/// The web console to serve: `None` when it is not wanted, or (for a LAN hub's `--web`
/// alone) not found, which is said. `uconsole web` and `--console` need one.
fn load_console(opts: &RunOptions) -> Result<Option<Arc<dspi_hub::web::Console>>, Failure> {
    if !opts.web {
        return Ok(None);
    }
    let found = opts.console.clone().or_else(|| {
        let exe = std::env::current_exe().ok();
        find_console(
            std::env::var_os(CONSOLE_ENV).map(PathBuf::from),
            exe.as_deref(),
        )
    });
    let Some(dir) = found else {
        if opts.local {
            return Err(Failure::Usage(format!(
                "no web console found: give --console DIR (apps/web's built dist), \
                 or set {CONSOLE_ENV}"
            )));
        }
        eprintln!(
            "{COMMAND} hub: no web console found (--console DIR, {CONSOLE_ENV}): serving / and \
             /hub/ only"
        );
        return Ok(None);
    };
    let console = dspi_hub::web::Console::load(&dir).map_err(|e| Failure::Failed(e.to_string()))?;
    let (files, bytes) = console.size();
    log::info!(
        "{COMMAND} hub: web console from {}: {files} files, {bytes} bytes",
        dir.display()
    );
    Ok(Some(Arc::new(console)))
}

/// Where the console is when nobody says: `$DSPI_WEB_CONSOLE`, then `web/` beside the
/// program, then `share/uconsole/web/` in the prefix it is installed in. The first that has
/// an `index.html`.
fn find_console(env: Option<PathBuf>, exe: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = env {
        return Some(dir);
    }
    let bin = exe?.parent()?;
    let beside = bin.join("web");
    let shared = bin
        .parent()
        .map(|prefix| prefix.join("share").join(COMMAND).join("web"));
    [Some(beside), shared]
        .into_iter()
        .flatten()
        .find(|d| d.join("index.html").is_file())
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

/// `uconsole hub run`: start, serve until asked to stop, stop.
fn serve(opts: &RunOptions, json: bool) -> Result<(), Failure> {
    let stop = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        if let Err(e) = signal_hook::flag::register(signal, Arc::clone(&stop)) {
            log::warn!("{COMMAND} hub: signal {signal} not caught ({e}): it will stop abruptly");
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
        let _ = begin_pairing(&running, json);
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
/// A hub started in the background (`uconsole hub &`) still has the terminal as its
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
            log::warn!("{COMMAND} hub: no thread to read commands ({e}); Ctrl-C stops the hub");
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
            log::warn!("{COMMAND} hub: signal {signal} not ignored: in the background it may stop");
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
        ["pair"] => {
            let _ = begin_pairing(running, json);
        }
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
        // -- WP P2TM --
        ["inventory"] => inventory(running, json),
        ["alarms"] => alarms(running, json),
        ["schedule"] => schedule(running, json),
        ["api-key", "new", rest @ ..] if rest.len() <= 2 => {
            match key_name(rest.first().copied())
                .and_then(|name| Ok((name, key_role(rest.get(1).copied())?)))
                .and_then(|(name, role)| mint_key(running.control.auth(), &name, role))
            {
                Ok(key) => print_minted(&key, json),
                Err(Failure::Usage(m) | Failure::Failed(m)) => say(&m),
            }
        }
        ["api-key", "list"] => print_keys(&running.control.auth().clients(), json),
        ["api-key", "revoke", cid] => match cid.parse::<u32>() {
            Ok(cid) => match revoke_key(running.control.auth(), cid, |c| running.control.revoke(c))
            {
                Ok(()) => say_key_revoked(cid, json),
                Err(Failure::Usage(m) | Failure::Failed(m)) => say(&m),
            },
            Err(_) => say("api-key revoke <cid>"),
        },
        ["api-key", ..] => say("api-key new <name> [viewer|control|admin] | list | revoke <cid>"),
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
                // -- WP WB7 --
                "console": running.console_url,
                // -- WP P2TM --
                "rest": running.rest,
            })
        );
    } else {
        println!(
            "Sharing as \"{}\" on port {port}, {} (hub id {id})",
            printable(&running.name),
            access_words(running.access),
        );
        // -- WP WB7 --
        if let Some(url) = &running.console_url {
            println!("Web console: {url}");
        }
        // -- WP P2TM --
        if running.rest {
            println!("REST: /api/v1/ on port {port}, with an API key (`api-key new <name>`)");
        }
    }
}

/// Open a pairing window and show its PIN. -- WP PAIRADM -- It gives at most
/// the control role; admin is `role <cid> admin` (`{COMMAND} hub role`). The
/// window opened, for the tests.
fn begin_pairing(running: &Running, json: bool) -> Option<dspi_hub::auth::PairingWindow> {
    if running.access == AccessMode::Open {
        let m = "access is open: clients connect with no PIN";
        if json {
            println!("{}", json!({ "event": "error", "error": m }));
        } else {
            println!("{m}");
        }
        return None;
    }
    let now = Instant::now();
    match running.control.auth().begin_pairing(now) {
        Ok(window) => {
            let shown = window.clone();
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
            if !json {
                println!("{PAIRED_ROLE_NOTE}");
            }
            Some(shown)
        }
        Err(e) => {
            eprintln!("{COMMAND} hub: no pairing: {e}");
            None
        }
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
                    "kind": c.kind.wire_name(), // -- WP P2TM --
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
        // -- WP P2TM --
        let kind = if c.kind == ClientKind::Api {
            "  [API key]"
        } else {
            ""
        };
        println!(
            "{:>4}  {:<7}  {}  (last seen {seen}){live}{kind}",
            c.cid,
            role_name(c.role),
            printable(&c.name),
        );
    }
}

/// `uconsole hub clients`, with no hub running.
fn offline_clients(store: &Path, json: bool) -> Result<(), Failure> {
    match SavedHub::peek(store)? {
        Some(saved) => print_clients(&saved.clients, None, json),
        None if json => println!("{}", json!({ "event": "clients", "clients": [] })),
        None => println!("no hub has run with {} yet", store.display()),
    }
    Ok(())
}

/// `uconsole hub role|revoke`, with no hub running: through the hub's own store code,
/// in the mode it last ran in, so nothing else changes.
fn offline_edit(
    store: &Path,
    cid: u32,
    edit: impl FnOnce(&AuthStore) -> bool,
) -> Result<(), Failure> {
    offline_auth(store, |auth| {
        if edit(auth) {
            Ok(())
        } else {
            Err(Failure::Usage(format!("no paired client {cid}")))
        }
    })
}

/// -- WP P2TM -- [`offline_edit`]'s way into a stopped hub's store, for any
/// change of its clients (an API key minted or revoked).
fn offline_auth<T>(
    store: &Path,
    edit: impl FnOnce(&AuthStore) -> Result<T, Failure>,
) -> Result<T, Failure> {
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
    edit(&auth)
}

// -- WP P2TM --
/// Who the prompt's reads are, in the hub's labels.
const PROMPT_PRINCIPAL: &str = "Hub console";

/// A service's answer to `t` for the person at this hub's prompt: a Viewer
/// principal (the session-id rule), opened for this one read and closed.
fn ask_service(running: &Running, t: &str, fields: Value) -> Result<Value, String> {
    let mut msg = json!({ "t": t, "id": 1 });
    if let (Some(m), Value::Object(f)) = (msg.as_object_mut(), fields) {
        m.extend(f);
    }
    let session = running
        .handle
        .open_principal(Role::Viewer, None, PROMPT_PRINCIPAL)
        .map_err(|e| format!("{t}: {e}"))?;
    let reply = running.handle.service_request(&session, &msg);
    running.handle.close_session(session.id());
    let reply = reply.ok_or_else(|| format!("{t}: that service is off on this hub"))?;
    let v: Value = serde_json::from_str(&reply).map_err(|e| format!("{t}: {e}"))?;
    if v.get("t").and_then(Value::as_str) == Some("ok") {
        Ok(v)
    } else {
        Err(format!("{t}: {} ({})", field(&v, "msg"), field(&v, "code")))
    }
}

/// A string field of the hub's, fit for a terminal; "" when absent.
fn field(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => printable(s),
        Some(Value::Null) | None => String::new(),
        Some(other) => printable(&other.to_string()),
    }
}

/// The prompt's way to say a read failed.
fn say_error(m: &str, json: bool) {
    if json {
        println!("{}", json!({ "event": "error", "error": m }));
    } else {
        println!("{m}");
    }
}

/// `inventory` at the prompt: `inventory.list`, one page of 500.
fn inventory(running: &Running, json: bool) {
    let v = match ask_service(running, "inventory.list", json!({ "limit": 500 })) {
        Ok(v) => v,
        Err(m) => return say_error(&m, json),
    };
    let devices = v
        .get("devices")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if json {
        println!(
            "{}",
            json!({ "event": "inventory", "devices": devices, "total": v.get("total") })
        );
        return;
    }
    if devices.is_empty() {
        println!("no devices seen yet");
    }
    for d in &devices {
        let online = d.get("online").and_then(Value::as_bool) == Some(true);
        let place = field(d, "location");
        println!(
            "{}  {:<7}  {}  fw {}{}",
            field(d, "serial"),
            if online { "online" } else { "offline" },
            field(d, "name"),
            field(d, "fw"),
            if place.is_empty() {
                String::new()
            } else {
                format!("  at {place}")
            },
        );
    }
}

/// `alarms` at the prompt: the alarms that are on (`alarm.list`, active).
fn alarms(running: &Running, json: bool) {
    let v = match ask_service(running, "alarm.list", json!({ "state": "active" })) {
        Ok(v) => v,
        Err(m) => return say_error(&m, json),
    };
    let list = v
        .get("alarms")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if json {
        println!("{}", json!({ "event": "alarms", "alarms": list }));
        return;
    }
    if list.is_empty() {
        println!("no alarms on");
    }
    for a in &list {
        println!(
            "{}  {:<8}  {}  {}: {}  ({})",
            field(a, "id"),
            field(a, "severity"),
            field(a, "code"),
            field(a, "subject"),
            field(a, "message"),
            field(a, "state"),
        );
    }
}

/// `schedule` at the prompt: the rules (`schedule.list`).
fn schedule(running: &Running, json: bool) {
    let v = match ask_service(running, "schedule.list", json!({})) {
        Ok(v) => v,
        Err(m) => return say_error(&m, json),
    };
    let rules = v
        .get("rules")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if json {
        println!(
            "{}",
            json!({
                "event": "schedule",
                "rules": rules,
                "zone": v.get("zone"),
                "localTime": v.get("local_time"),
                "clockUnset": v.get("clock_unset"),
            })
        );
        return;
    }
    println!(
        "Time zone {}, local time {}",
        field(&v, "zone"),
        field(&v, "local_time")
    );
    if v.get("clock_unset").and_then(Value::as_bool) == Some(true) {
        println!("The clock is not set: no rule runs until it is");
    }
    if rules.is_empty() {
        println!("no rules");
    }
    for r in &rules {
        let on = r.get("enabled").and_then(Value::as_bool) != Some(false);
        let next = field(r, "next_run");
        let last = match r.get("last") {
            Some(l) if l.is_object() => {
                let ok = l.get("ok").and_then(Value::as_bool) == Some(true);
                format!(
                    ", last {} {}",
                    field(l, "at"),
                    if ok { "ok" } else { "failed" }
                )
            }
            _ => String::new(),
        };
        println!(
            "{:>4}  {}  {}, next {}{last}",
            field(r, "id"),
            field(r, "name"),
            if on { "on" } else { "off" },
            if next.is_empty() {
                "none".to_owned()
            } else {
                next
            },
        );
    }
}

/// An API key's name, as a client's: required, cleaned.
fn key_name(word: Option<&str>) -> Result<String, Failure> {
    let usage = || Failure::Usage("which name? api-key new <name> [viewer|control|admin]".into());
    let cleaned: String = word
        .ok_or_else(usage)?
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return Err(usage());
    }
    Ok(cleaned.to_owned())
}

/// An API key's role: control unless named.
fn key_role(word: Option<&str>) -> Result<Role, Failure> {
    match word {
        None => Ok(Role::Control),
        some => parse_role(some),
    }
}

/// `auth.mint` on this machine: an API key of `role`, shown once.
fn mint_key(auth: &AuthStore, name: &str, role: Role) -> Result<MintedKey, Failure> {
    auth.mint_api_key(name, role, false).map_err(|e| match e {
        AuthError::TableFull => {
            Failure::Failed("64 paired clients and API keys already: revoke one first".into())
        }
        e => Failure::Failed(format!("no API key made ({e}); the log says why")),
    })
}

fn print_minted(key: &MintedKey, json: bool) {
    if json {
        println!(
            "{}",
            json!({
                "event": "apiKey",
                "cid": key.cid,
                "name": key.name,
                "role": role_name(key.role),
                "kind": ClientKind::Api.wire_name(),
                "key": key.key,
            })
        );
        return;
    }
    println!(
        "API key {} \"{}\", {}:\n{}",
        key.cid,
        printable(&key.name),
        role_name(key.role),
        key.key
    );
    println!(
        "Shown this once: keep it now. Send it as `Authorization: Bearer <key>` to /api/v1/ \
         on this hub's port (with --rest). It crosses the network in plain text: use it on a \
         network you trust. Revoke it with `api-key revoke {}`.",
        key.cid
    );
}

/// The API keys among `clients`.
fn print_keys(clients: &[PairedClient], json: bool) {
    let keys: Vec<&PairedClient> = clients
        .iter()
        .filter(|c| c.kind == ClientKind::Api)
        .collect();
    if json {
        let list: Vec<Value> = keys
            .iter()
            .map(|c| {
                json!({
                    "cid": c.cid,
                    "name": c.name,
                    "role": role_name(c.role),
                    "created": unix(c.created),
                    "lastSeen": c.last_seen.map(unix),
                })
            })
            .collect();
        println!("{}", json!({ "event": "apiKeys", "keys": list }));
        return;
    }
    if keys.is_empty() {
        println!("no API keys");
    }
    for c in keys {
        let seen = c.last_seen.map_or_else(|| "never".to_owned(), ago);
        println!(
            "{:>4}  {:<7}  {}  (last used {seen})",
            c.cid,
            role_name(c.role),
            printable(&c.name)
        );
    }
}

/// `uconsole hub api-key list`, with no hub running.
fn offline_keys(store: &Path, json: bool) -> Result<(), Failure> {
    match SavedHub::peek(store)? {
        Some(saved) => print_keys(&saved.clients, json),
        None => print_keys(&[], json),
    }
    Ok(())
}

/// Revoke `cid` with `revoke` if it is an API key; a console is `revoke <cid>`'s.
fn revoke_key(auth: &AuthStore, cid: u32, revoke: impl FnOnce(u32) -> bool) -> Result<(), Failure> {
    match auth.clients().iter().find(|c| c.cid == cid).map(|c| c.kind) {
        Some(ClientKind::Api) if revoke(cid) => Ok(()),
        Some(ClientKind::Console) => Err(Failure::Usage(format!(
            "client {cid} is a console, not an API key: `revoke {cid}` forgets it"
        ))),
        _ => Err(Failure::Usage(format!("no API key {cid}"))),
    }
}

fn say_key_revoked(cid: u32, json: bool) {
    if json {
        println!("{}", json!({ "event": "apiKeyRevoked", "cid": cid }));
    } else {
        println!("API key {cid} revoked: it no longer works");
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
                concat!("--who=", dspi_cmd::command!(), " hub"),
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
                        "{} hub: warning: cannot keep the machine awake ({e}); \
                         sharing stops if it sleeps",
                        dspi_cmd::COMMAND
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
            // -- WP WB7 --
            web: false,
            console: None,
            local: false,
            share_usb: false,
            // -- WP SW3 --
            alink: false,
            // -- WP HWLK1F --
            with_usb: false,
            with_plain_unit: false,
            with_usb_lock_error: false,
            // -- WP P2TM --
            rest: false,
            data: None,
            services: Services::default(),
        }
    }

    /// The Terminal's own way to a hub, as `uconsole --link 127.0.0.1:PORT [--pin PIN]`.
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
            "{COMMAND} --hub hub: a hub is not run on another hub"
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

    /// Review c4b: `uconsole --hub hub` is refused before anything looks at the network.
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

    // -- WP TMCORE --
    /// The Terminal's write path on a hub's device: the core's own DSPi Link
    /// session (`core_route::open_core`), with the hub's guard, the hub's lock
    /// (`heldBy: hub`) and the interface's undo groups.
    mod tmcore {
        use super::*;
        use crate::core_route::{self, Confirm};
        use crate::pro::Stores;
        use dspi_proto::value::Value;
        use dspi_session::{RouteFailure, WriteError};

        fn line(s: &dspi_session::Session, path: &'static str, ix: &[u8], v: Value) -> String {
            dspi_cmd::format(
                &dspi_cmd::Command::Set {
                    path,
                    indices: ix.to_vec(),
                    value: v,
                },
                &crate::context_for(s),
            )
        }

        #[test]
        #[allow(clippy::too_many_lines)]
        fn wp_tmcore_a_hub_device_opens_as_a_core_dlink_session() {
            let store = scratch("tmcore");
            let running = start(&demo_options(&store), false).unwrap();
            let port = running.control.port();
            let pin = pin_of(&running);
            let stores = Stores::under(&store.join("console"));
            let open = |t: &Target| {
                core_route::open_core(t, &stores, &no_prompt)
                    .expect("the hub's device opens on a core")
            };

            // Paired with the PIN, the token kept, the device bound: the core's
            // own session, as `control`.
            let o = open(&client(port, Some(&pin), &store));
            let core = std::sync::Arc::clone(&o.handle.core);
            assert_eq!(core.link_hub_role(), "control");
            assert!(o.reached.as_ref().is_some_and(|l| l.hub_id.is_some()));
            assert!(core.apply_guard_default(), "the hub offers its guard");

            // A guarded recall is guarded by the hub too (P1HA): `hub: true`.
            let snap = core
                .snapshot_capture("TMCORE".into(), String::new())
                .unwrap();
            let g = core.apply_guarded_snapshot(snap.id.clone(), 30).unwrap();
            assert!(g.hub, "{g:?}");
            assert!(!crate::pro::backstop_line().is_empty());
            core.apply_confirm().unwrap();

            // The interface on it: a compound edit is one undo step.
            let mut c = core_route::connected_on(&o).unwrap();
            let theme = dspi_tui::Theme::console(
                dspi_tui::theme::ColorDepth::TrueColor,
                dspi_tui::theme::Glyphs::Braille,
            );
            let mut live = dspi_tui::live::Live::new(
                c.state,
                theme,
                dspi_tui::perf::Performance::default(),
                Box::new(dspi_tui::live::ConsoleScreens::with_config_path(
                    store.join("config.toml"),
                )),
            );
            let s = &mut c.session;
            let gain = |o: usize| core.get_state().output_gain_db[o];
            let (g0, g1) = (gain(0), gain(1));
            let block = format!(
                "{}\n{}\n# Pair",
                line(s, "out.gain", &[0], Value::Float(-5.0)),
                line(s, "out.gain", &[1], Value::Float(-6.0)),
            );
            live.run_commands(s, &block);
            assert!((gain(0) + 5.0).abs() < 0.05 && (gain(1) + 6.0).abs() < 0.05);
            live.run_command(s, "undo");
            assert!((gain(0) - g0).abs() < 0.05, "{}", gain(0));
            assert!((gain(1) - g1).abs() < 0.05, "{}", gain(1));
            drop(c.session);
            drop(live);
            drop(core);
            drop(o);

            // The installer, made admin, gives the hub a profile that locks the
            // front panel and a PIN.
            let cid = running.control.auth().clients()[0].cid;
            assert!(command(&running, &format!("role {cid} admin"), false));
            let o = open(&client(port, None, &store));
            let core = std::sync::Arc::clone(&o.handle.core);
            assert_eq!(core.link_hub_role(), "admin");
            let handle = core.link_devices()[0].handle;
            let profile: serde_json::Value = serde_json::from_str(
                &dspi_session::limits::LimitsProfile {
                    locked_pages: vec!["front".into()],
                    ..Default::default()
                }
                .to_document(),
            )
            .unwrap();
            let mut f = serde_json::Map::new();
            f.insert("handle".into(), handle.into());
            f.insert("profile".into(), profile);
            core.link_hub_request("limits.set", f).unwrap();
            let mut f = serde_json::Map::new();
            f.insert("handle".into(), handle.into());
            f.insert("pin".into(), "4711-door".into());
            core.link_hub_request("limits.pin", f).unwrap();
            drop(core);
            drop(o);

            // Back as `control`: the hub holds the lock, and a write past it is
            // refused with the banner's line.
            assert!(command(&running, &format!("role {cid} control"), false));
            let o = open(&client(port, None, &store));
            let lock = o.handle.core.installer_lock_state();
            assert_eq!(lock.held_by, "hub", "{lock:?}");
            assert!(lock.locked, "{lock:?}");
            let mut s =
                core_route::session_on(&o, Confirm::Refuse, core_route::PinSource::Nobody).unwrap();
            let e = s.write("out.gain", &[0], Value::Float(-1.0)).unwrap_err();
            let banner =
                dspi_core::notice_for_key(dspi_core::LOCK_BANNER_NOTICE.into(), String::new())
                    .reason;
            assert_eq!(e.to_string(), banner);
            assert!(matches!(
                e,
                WriteError::Routed {
                    kind: RouteFailure::Locked,
                    ..
                }
            ));
            // Inside the lock's scope, the write goes.
            assert!(s.write("vol.master", &[], Value::Float(-10.0)).is_ok());
            // -- WP TMCOREF -- (F2) the bootloader is held by the hub's lock too
            let mut sure =
                core_route::session_on(&o, Confirm::Given, core_route::PinSource::Nobody).unwrap();
            let e = sure
                .write("dev.bootloader", &[], Value::Trigger)
                .unwrap_err();
            assert_eq!(crate::write_exit(&e), crate::exit::REJECTED, "{e}");
            // -- WP TMCOREF -- (F1) the hub's PIN, given as a script gives it,
            // unlocks it for this command
            core_route::unlock_up_front(
                &o.handle.core,
                &core_route::PinSource::Env("4711-door".into()),
                true,
            )
            .unwrap();
            assert!(s.write("out.gain", &[0], Value::Float(-1.0)).is_ok());
            assert!(!command(&running, "quit", false));
        }
    }

    // -- WP HUBADMIN --
    /// `uconsole --link … limits get|set|clear`, `lock set-pin|clear-pin` and
    /// `mode pin` change the HUB's copy, as an Admin of the hub only.
    mod hubadmin {
        use super::*;
        use crate::core_route;
        use crate::pro::Stores;
        use crate::pro::installer::{self as inst, Pins};

        fn pins(new: Option<&'static str>) -> Pins<'static> {
            Pins {
                current: Box::new(|| None),
                new: Box::new(move || Ok(new.map(str::to_owned))),
            }
        }

        fn doc(max_gain_db: f32) -> String {
            dspi_session::limits::LimitsProfile {
                outputs: vec![dspi_session::limits::OutputLimits {
                    output: 0,
                    max_gain_db: Some(max_gain_db),
                    ..Default::default()
                }],
                locked_pages: vec!["eq".into()],
                ..Default::default()
            }
            .to_document()
        }

        #[test]
        fn wp_hubadmin_the_terminal_edits_the_hubs_copy_as_an_admin() {
            let store = scratch("hubadmin");
            let running = start(&demo_options(&store), false).unwrap();
            let port = running.control.port();
            let pin = pin_of(&running);
            let stores = Stores::under(&store.join("console"));
            let open = |t: &Target| {
                core_route::open_core(t, &stores, &no_prompt)
                    .expect("the hub's device opens on a core")
            };

            // Paired with the hub's PIN: Control, read-only here.
            let o = open(&client(port, Some(&pin), &store));
            let core = std::sync::Arc::clone(&o.handle.core);
            assert_eq!(core.link_hub_role(), "control");
            let t = inst::hub_copy(&core).expect("the hub's copy");
            assert!(!t.editable);
            assert_eq!(inst::hub_limits_show(&core, true), crate::exit::OK);
            assert_eq!(
                inst::hub_limits_set(&core, &t, doc(3.0), false),
                crate::exit::REJECTED
            );
            assert_eq!(
                inst::hub_pin_set(&core, &t, &mut pins(Some("4711-door")), false),
                crate::exit::REJECTED
            );
            assert_eq!(
                inst::mode_on(&core, Some(("control".into(), "simple".into())), false),
                crate::exit::REJECTED
            );
            assert!(!core.hub_limits_get().unwrap().pin_set, "nothing sent");

            // Made an Admin on the hub's machine, live: the next command
            // reads the role again, and the change goes.
            let cid = running.control.auth().clients()[0].cid;
            assert!(command(&running, &format!("role {cid} admin"), false));
            assert_eq!(
                inst::hub_limits_set(&core, &t, doc(3.0), false),
                crate::exit::OK
            );
            let st = core.hub_limits_get().unwrap();
            assert!(st.can_edit && !st.profile_document.is_empty(), "{st:?}");
            assert!(
                !core.limits_status().active,
                "the console's own copy untouched"
            );
            // A new PIN: from DSPI_NEW_INSTALLER_PIN, --new-pin - or the
            // prompt, never the command line; no current PIN for an Admin.
            assert_eq!(
                inst::hub_pin_set(&core, &t, &mut pins(Some("4711-door")), true),
                crate::exit::OK
            );
            assert!(core.hub_limits_get().unwrap().pin_set);
            assert_eq!(
                inst::hub_pin_set(&core, &t, &mut pins(None), false),
                crate::exit::USAGE
            );
            assert_eq!(
                inst::mode_on(&core, Some(("control".into(), "simple".into())), false),
                crate::exit::OK
            );
            assert!(
                core.hub_limits_get()
                    .unwrap()
                    .pinned_modes
                    .iter()
                    .any(|p| p.role == "control" && p.mode == "simple")
            );
            assert_eq!(
                inst::mode_on(&core, Some(("control".into(), "none".into())), false),
                crate::exit::OK
            );
            assert_eq!(
                inst::mode_on(&core, Some(("guest".into(), "simple".into())), false),
                crate::exit::USAGE
            );
            assert_eq!(inst::hub_pin_clear(&core, &t, false), crate::exit::OK);
            assert_eq!(inst::hub_limits_clear(&core, &t, false), crate::exit::OK);
            let st = core.hub_limits_get().unwrap();
            assert!(!st.pin_set && st.profile_document.is_empty(), "{st:?}");

            // -- WP HUBADMINF -- F1: demoted live on the hub's machine while
            // the core still holds "admin": the command reads the role again
            // and exits 3 before anything is sent.
            let t = inst::hub_copy(&core).unwrap();
            assert!(t.editable, "the core last heard admin");
            assert!(command(&running, &format!("role {cid} control"), false));
            assert_eq!(
                inst::hub_limits_set(&core, &t, doc(3.0), false),
                crate::exit::REJECTED
            );
            assert_eq!(
                inst::hub_pin_set(&core, &t, &mut pins(Some("4711-door")), false),
                crate::exit::REJECTED
            );
            assert_eq!(
                inst::mode_on(&core, Some(("control".into(), "simple".into())), false),
                crate::exit::REJECTED
            );
            let st = core.hub_limits_get().unwrap();
            assert!(
                !st.can_edit && !st.pin_set && st.profile_document.is_empty(),
                "nothing sent: {st:?}"
            );
            assert!(st.pinned_modes.iter().all(|p| p.mode.is_empty()), "{st:?}");
            assert!(!command(&running, "quit", false));
        }
    }
}

#[cfg(test)]
mod tests_wp_wb7 {
    use super::*;
    use std::io::{Read, Write};

    fn scratch(name: &str) -> PathBuf {
        // Beside the test binary, in the target directory: never on tmpfs.
        let dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("wp_wb7")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run_options(args: &[&str], demo: bool) -> RunOptions {
        match parse(args, demo, Some(PathBuf::from("/store"))).unwrap() {
            HubCommand::Run(o) => o,
            other => panic!("not a run: {other:?}"),
        }
    }

    /// A built console, as far as a hub can tell.
    fn console_dir() -> PathBuf {
        let dir = scratch("dist");
        std::fs::write(
            dir.join("index.html"),
            r#"<head><meta name="dspi-hub" content="" /></head>"#,
        )
        .unwrap();
        std::fs::write(dir.join("app.js"), "1").unwrap();
        dir
    }

    fn get(port: u16, path: &str) -> (String, String) {
        let mut tcp = std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(tcp, "GET {path} HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n").unwrap();
        let mut text = String::new();
        tcp.read_to_string(&mut text).unwrap();
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        (head.to_owned(), body.to_owned())
    }

    #[test]
    fn wp_wb7_local_is_loopback_with_the_console_on_its_own_port() {
        let o = run_options(&["--local"], false);
        assert!(o.local && o.loopback && o.web && !o.share_usb);
        assert_eq!(o.port, LOCAL_PORT);
        assert_eq!(o.access, AccessMode::Pin, "PIN pairing stays the default");
        let o = run_options(&["--local", "--port", "9000", "--share-usb"], false);
        assert_eq!((o.port, o.share_usb), (9000, true));
        let o = run_options(&["--local", "--console", "/srv/dist"], true);
        assert_eq!(o.console, Some(PathBuf::from("/srv/dist")));
        assert!(o.demo);
    }

    #[test]
    fn wp_wb7_web_and_console_on_a_lan_hub() {
        let o = run_options(&[], false);
        assert!(!o.web && o.console.is_none() && !o.local);
        let o = run_options(&["--web"], false);
        assert!(o.web && !o.loopback && !o.local);
        assert_eq!(o.port, DEFAULT_PORT);
        let o = run_options(&["--console=/srv/dist"], false);
        assert!(o.web, "--console implies --web");
    }

    #[test]
    fn wp_wb7_bad_web_lines_are_usage_errors() {
        let bad = |args: &[&str], demo: bool| {
            matches!(
                parse(args, demo, Some(PathBuf::from("/store"))),
                Err(Failure::Usage(_))
            )
        };
        assert!(bad(&["--share-usb"], false), "only with --local");
        assert!(
            bad(&["--local", "--share-usb"], true),
            "the demo holds no USB"
        );
        assert!(bad(&["--console"], false), "needs a directory");
        assert!(bad(&["clients", "--web"], false));
        assert!(bad(&["clients", "--console", "/x"], false));
    }

    #[test]
    fn wp_wb7_the_console_is_found_by_the_environment_or_beside_the_program() {
        let root = scratch("find");
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = bin.join(COMMAND);
        assert_eq!(find_console(None, Some(&exe)), None);
        let shared = root.join("share").join(COMMAND).join("web");
        std::fs::create_dir_all(&shared).unwrap();
        assert_eq!(
            find_console(None, Some(&exe)),
            None,
            "no index.html: not one"
        );
        std::fs::write(shared.join("index.html"), "x").unwrap();
        assert_eq!(find_console(None, Some(&exe)), Some(shared.clone()));
        let beside = bin.join("web");
        std::fs::create_dir_all(&beside).unwrap();
        std::fs::write(beside.join("index.html"), "x").unwrap();
        assert_eq!(find_console(None, Some(&exe)), Some(beside));
        let env = PathBuf::from("/from/env");
        assert_eq!(find_console(Some(env.clone()), Some(&exe)), Some(env));
        assert_eq!(find_console(None, None), None);
    }

    #[test]
    fn wp_wb7_dspi_web_serves_the_console_and_the_demo_on_loopback() {
        let store = scratch("local");
        let mut opts = run_options(&["--local", "--port", "0"], true);
        opts.store = store.clone();
        opts.keep_awake = false;
        opts.console = Some(console_dir());
        let running = start(&opts, false).unwrap();
        let port = running.control.port();
        assert_eq!(
            running.console_url.as_deref(),
            Some(format!("http://localhost:{port}/console/").as_str())
        );
        let (head, body) = get(port, "/console/");
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(
            body.contains(&format!(r#"content="ws://localhost:{port}""#)),
            "{body}"
        );
        assert!(head.contains("'wasm-unsafe-eval'"), "{head}");
        let (head, _) = get(port, "/");
        assert!(head.starts_with("HTTP/1.1 200"), "the hub's page: {head}");
        assert_eq!(running.handle.devices().len(), 1, "the demo device");
        drop(running);

        // Without a console `uconsole web` has nothing to serve, and says so.
        let mut none = opts.clone();
        none.console = Some(store.join("nowhere"));
        assert!(matches!(start(&none, false), Err(Failure::Failed(_))));
    }

    #[test]
    fn wp_wb7_a_lan_hubs_web_without_a_console_serves_its_page_only() {
        if std::env::var_os(CONSOLE_ENV).is_some() {
            return; // a console is named for the whole process
        }
        let mut opts = run_options(&["--web", "--port", "0", "--loopback"], true);
        opts.store = scratch("lan");
        opts.keep_awake = false;
        let running = start(&opts, false).unwrap();
        let port = running.control.port();
        assert!(get(port, "/").0.starts_with("HTTP/1.1 200"));
        assert!(get(port, "/console/").0.starts_with("HTTP/1.1 404"));
        assert_eq!(running.console_url, None);
    }
}

// -- WP SW3 --
#[cfg(test)]
mod tests_wp_sw3 {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        // Beside the test binary, in the target directory: never on tmpfs.
        let dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("wp_sw3")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn wp_sw3_audio_link_is_a_demo_hub_option() {
        let store = Some(PathBuf::from("/store"));
        // -- WP ALINK1 -- `--alink`, and `--audio-link` as its hidden alias
        for flag in ["--alink", "--audio-link"] {
            match parse(&[flag], true, store.clone()).unwrap() {
                HubCommand::Run(o) => assert!(o.alink && o.demo, "{flag}"),
                other => panic!("{other:?}"),
            }
            assert!(matches!(
                parse(&[flag], false, store.clone()),
                Err(Failure::Usage(_))
            ));
            assert!(matches!(
                parse(&["clients", flag], false, store.clone()),
                Err(Failure::Usage(_))
            ));
        }
    }

    #[test]
    fn wp_sw3_the_demo_installation_attaches_with_its_model() {
        let store = scratch("rig");
        let mut opts = tests_wp_tm11_options(&store);
        opts.alink = true;
        let running = start(&opts, false).unwrap();
        let (model, revision) = running.handle.system_model();
        assert_eq!(revision, 1);
        // -- WP AUSB1 -- four on the audio link, two USB-direct
        assert_eq!(model.units.len(), 6);
        assert_eq!(model.scenes.len(), 4);
        let status = running.handle.system_status();
        for (serial, _) in alink_rig() {
            assert!(
                status.observed.get(serial).is_some_and(|o| o.online),
                "{serial}"
            );
        }
        // -- WP AUSB1 -- the USB-direct units are phase-locked, the height one 3 frames late
        for (serial, _) in usb_rig() {
            let o = &status.observed[serial];
            assert_eq!(
                o.usb.as_ref().map(|u| u.state.as_str()),
                Some("phase_locked"),
                "{serial}"
            );
        }
        let rear = status
            .alignment
            .units
            .iter()
            .find(|p| p.unit == "R")
            .unwrap();
        assert_eq!(rear.start_offset_samples, Some(144));
        // A receiver reports its alink stats and the trim scan.
        let a = &status.observed["DEMO-ALINK-A"];
        assert_eq!(a.trim_scan.as_ref().map(|t| t.state.as_str()), Some("done"));
        assert!(a.stats.is_some());
        // -- fixup sw3x -- a sender holds one superframe apply at a time: the
        // status names the frame it holds, and a second is refused until then.
        let first = running
            .handle
            .system_apply_scene("evening", Role::Admin)
            .unwrap();
        assert_eq!(first.plan.path, "superframe");
        let held = running.handle.system_status().observed["DEMO-ALINK-TX"].pending_apply_frame;
        assert_eq!(held, Some(first.plan.frame));
        let second = running
            .handle
            .system_apply_scene("closed", Role::Admin)
            .unwrap();
        assert_eq!(second.plan.path, "superframe");
        // -- WP AUSB1 -- the USB-direct units are held back with it: no half scene
        assert!(
            second.writes.iter().all(|w| w.result == "refused"
                || (w.result == "skipped"
                    && w.reason.as_deref() == Some(dspi_hub::system::USB_HELD_BACK))),
            "{:?}",
            second.writes
        );
        assert_eq!(
            second
                .writes
                .iter()
                .filter(|w| w.result == "skipped")
                .count(),
            2,
            "{:?}",
            second.writes
        );
        drop(running);
        // A kept model is loaded, not replaced, at the next start.
        let running = start(&opts, false).unwrap();
        let (model, revision) = running.handle.system_model();
        assert_eq!((model.units.len(), revision), (6, 0));
        drop(running);
        let _ = std::fs::remove_dir_all(&store);
    }

    fn tests_wp_tm11_options(store: &Path) -> RunOptions {
        RunOptions {
            name: Some("SW3 test hub".into()),
            port: 0,
            access: AccessMode::Pin,
            store: store.to_owned(),
            loopback: true,
            pair: false,
            demo: true,
            keep_awake: false,
            forget_clients: false,
            web: false,
            console: None,
            local: false,
            share_usb: false,
            alink: false,
            // -- WP HWLK1F --
            with_usb: false,
            with_plain_unit: false,
            with_usb_lock_error: false,
            // -- WP P2TM --
            rest: false,
            data: None,
            services: Services::default(),
        }
    }
}

// -- WP HWLK1F --
#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests_wp_hwlk1f {
    use super::*;
    use std::net::Ipv4Addr;

    fn scratch(name: &str) -> PathBuf {
        // Beside the test binary, in the target directory: never on tmpfs.
        let dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("wp_hwlk1f")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run(args: &[&str], demo: bool) -> Result<RunOptions, Failure> {
        match parse(args, demo, Some(PathBuf::from("/store")))? {
            HubCommand::Run(o) => Ok(o),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn wp_hwlk1f_the_new_options_parse() {
        let o = run(&["--pair"], false).unwrap();
        assert!(o.pair);
        let o = run(&["--alink", "--with-usb", "--with-plain-unit"], true).unwrap();
        assert!(o.alink && o.with_usb && o.with_plain_unit && o.demo);
        // -- WP AUSB2 --
        let o = run(&["--alink", "--with-usb-lock-error"], true).unwrap();
        assert!(o.alink && o.with_usb_lock_error && !o.with_plain_unit);
        assert!(matches!(
            run(&["--with-usb-lock-error"], true),
            Err(Failure::Usage(_))
        ));
        for f in ["--with-usb", "--with-plain-unit"] {
            assert!(matches!(run(&[f], true), Err(Failure::Usage(_))), "{f}");
            assert!(matches!(run(&[f], false), Err(Failure::Usage(_))), "{f}");
        }
        let o = run(&["--alink"], true).unwrap();
        assert!(!o.with_usb && !o.with_plain_unit, "off unless given");
    }

    /// -- WP PAIRADM -- `--pair-admin` is gone: refused wherever it is given,
    /// with the way to admin (`hub role`), never ignored.
    #[test]
    fn wp_pairadm_pair_admin_is_refused_with_the_way_to_admin() {
        for args in [
            &["--pair-admin"][..],
            &["--pair", "--pair-admin"],
            &["--pair-admin", "--open"],
            &["run", "--pair-admin"],
            &["clients", "--pair-admin"],
        ] {
            match parse(args, false, Some(PathBuf::from("/s"))) {
                Err(Failure::Usage(m)) => {
                    assert!(m.contains("no longer supported"), "{m}");
                    assert!(m.contains(" hub role <cid> admin"), "{m}");
                    assert!(m.contains(" hub clients"), "{m}");
                }
                other => panic!("{args:?}: {other:?}"),
            }
        }
        assert!(parse(&["--pair-admin"], true, Some(PathBuf::from("/s"))).is_err());
    }

    fn options(store: &Path) -> RunOptions {
        RunOptions {
            name: Some("HWLK1F test hub".into()),
            port: 0,
            access: AccessMode::Pin,
            store: store.to_owned(),
            loopback: true,
            pair: false,
            demo: true,
            keep_awake: false,
            forget_clients: false,
            web: false,
            console: None,
            local: false,
            share_usb: false,
            alink: false,
            with_usb: false,
            with_plain_unit: false,
            with_usb_lock_error: false,
            // -- WP P2TM --
            rest: false,
            data: None,
            services: Services::default(),
        }
    }

    /// -- WP PAIRADM -- every window, `--pair`'s and the typed `pair`'s, gives
    /// at most control; `role <cid> admin` at the prompt makes it admin, live.
    #[test]
    fn wp_pairadm_pairing_gives_control_and_the_role_command_admin() {
        let store = scratch("pair");
        let running = start(&options(&store), false).unwrap();
        let peer = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let auth = running.control.auth();
        let w = begin_pairing(&running, true).unwrap();
        let a = auth
            .pair(&w.pin, "installer", Some(Role::Admin), peer, Instant::now())
            .unwrap();
        assert_eq!(a.role, Role::Control);
        assert!(command(&running, &format!("role {} admin", a.cid), true));
        assert_eq!(auth.role_of(a.cid), Some(Role::Admin));
        let w = begin_pairing(&running, true).unwrap();
        let c = auth
            .pair(&w.pin, "phone", Some(Role::Admin), peer, Instant::now())
            .unwrap();
        assert_eq!(c.role, Role::Control);
        drop(running);
        let _ = std::fs::remove_dir_all(&store);
    }

    /// `--with-plain-unit`: one more simulated DSPi without the audio link, in
    /// the model as unit P; it is seen as such and takes a scene at once.
    #[test]
    fn wp_hwlk1f_the_plain_unit_joins_the_demo_installation() {
        let store = scratch("plain");
        let mut opts = options(&store);
        opts.alink = true;
        opts.with_plain_unit = true;
        let running = start(&opts, false).unwrap();
        let (model, _) = running.handle.system_model();
        assert_eq!(model.units.len(), 7);
        assert_eq!(
            model.units.last().map(|u| u.serial.as_str()),
            Some(PLAIN_SERIAL)
        );
        let status = running.handle.system_status();
        let p = &status.observed[PLAIN_SERIAL];
        assert!(p.online && p.without_alink());
        assert!(
            status.observed["DEMO-ALINK-A"].alink.is_some()
                && !status.observed["DEMO-ALINK-A"].without_alink()
        );
        let r = running
            .handle
            .system_apply_scene("evening", Role::Admin)
            .unwrap();
        assert_eq!(r.plan.path, "superframe");
        assert!(
            r.writes
                .iter()
                .any(|w| w.element == "P" && w.result == "applied_now"),
            "{:?}",
            r.writes
        );
        assert!(!r.on_time);
        drop(running);
        let _ = std::fs::remove_dir_all(&store);
    }
}

// -- WP P2TM --
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests_wp_p2tm {
    use super::*;
    use crate::api::status;
    use crate::api::tests_wp_p2tm::api_call;
    use dspi_tui::shell::connect::LinkTarget;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    /// The demo device's serial (every fake's).
    const DEMO: &str = "DEMO0000DSPI0001";
    const WAIT: Duration = Duration::from_secs(20);

    fn scratch(name: &str) -> PathBuf {
        // Beside the test binary, in the target directory: never on tmpfs.
        let dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("wp_p2tm")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn parsed(args: &[&str]) -> Result<HubCommand, Failure> {
        parse(args, false, Some(PathBuf::from("/store")))
    }

    fn run_options(args: &[&str]) -> RunOptions {
        match parse(args, true, Some(PathBuf::from("/store"))) {
            Ok(HubCommand::Run(o)) => o,
            other => panic!("{args:?}: {other:?}"),
        }
    }

    fn options(store: &Path) -> RunOptions {
        let mut o = run_options(&["--port", "0", "--loopback", "--no-keep-awake"]);
        o.name = Some("P2TM test hub".into());
        o.store = store.to_owned();
        o
    }

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

    fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
        let start = Instant::now();
        while !cond() {
            assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// One HTTP request to the hub's port; the status code and the body.
    fn http(port: u16, method: &str, path: &str, key: Option<&str>) -> (u16, String) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(WAIT)).unwrap();
        let auth = key.map_or_else(String::new, |k| format!("Authorization: Bearer {k}\r\n"));
        let body = if method == "GET" {
            ""
        } else {
            "Content-Length: 0\r\n"
        };
        write!(
            s,
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{auth}{body}Connection: close\r\n\r\n"
        )
        .unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        let code = out
            .split(' ')
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        let body = out.split_once("\r\n\r\n").map_or("", |(_, b)| b).to_owned();
        (code, body)
    }

    #[test]
    fn wp_p2tm_every_service_is_on_by_default_and_each_turns_off() {
        let o = run_options(&[]);
        assert_eq!(o.services, Services::default());
        assert!(!o.rest, "REST is off by default");
        assert_eq!(o.data, None);
        let mut config = dspi_hub::ServiceConfig::default();
        o.services.apply(&mut config);
        assert!(config.health.is_some() && config.history.is_some() && config.schedule.is_some());
        assert!(config.inventory && config.identify && config.alarms && config.stats);

        let o = run_options(&[
            "--rest",
            "--data",
            "/data",
            "--no-health",
            "--no-inventory",
            "--no-identify",
            "--no-history",
            "--no-alarms",
            "--no-schedule",
        ]);
        assert!(o.rest);
        assert_eq!(o.data, Some(PathBuf::from("/data")));
        let mut config = dspi_hub::ServiceConfig::default();
        o.services.apply(&mut config);
        assert!(config.health.is_none() && config.history.is_none() && config.schedule.is_none());
        assert!(!config.inventory && !config.identify && !config.alarms);
        // `--data=DIR` too; each is for running a hub only.
        assert_eq!(run_options(&["--data=/d"]).data, Some(PathBuf::from("/d")));
        for bad in [
            &["clients", "--rest"][..],
            &["clients", "--no-alarms"],
            &["revoke", "1", "--data", "/d"],
            &["api-key", "list", "--rest"],
            &["--data"],
        ] {
            assert!(
                matches!(parsed(bad), Err(Failure::Usage(_))),
                "{bad:?}: {:?}",
                parsed(bad)
            );
        }
    }

    #[test]
    fn wp_p2tm_api_key_commands_parse() {
        let store = PathBuf::from("/store");
        assert_eq!(
            parsed(&["api-key", "new", "Home Assistant"]).unwrap(),
            HubCommand::ApiKeyNew {
                store: store.clone(),
                name: "Home Assistant".into(),
                role: Role::Control,
            }
        );
        assert_eq!(
            parsed(&["api-key", "new", "Dashboard", "viewer", "--store", "/s"]).unwrap(),
            HubCommand::ApiKeyNew {
                store: PathBuf::from("/s"),
                name: "Dashboard".into(),
                role: Role::Viewer,
            }
        );
        assert_eq!(
            parsed(&["api-key", "list"]).unwrap(),
            HubCommand::ApiKeyList {
                store: store.clone()
            }
        );
        assert_eq!(
            parsed(&["api-key", "revoke", "7"]).unwrap(),
            HubCommand::ApiKeyRevoke { store, cid: 7 }
        );
        for bad in [
            &["api-key"][..],
            &["api-key", "new"],
            &["api-key", "new", "  "],
            &["api-key", "new", "x", "boss"],
            &["api-key", "new", "x", "control", "more"],
            &["api-key", "revoke"],
            &["api-key", "revoke", "x"],
            &["api-key", "frob"],
        ] {
            assert!(matches!(parsed(bad), Err(Failure::Usage(_))), "{bad:?}");
        }
        assert!(parse(&["api-key", "list"], true, Some(PathBuf::from("/s"))).is_err());
    }

    /// The entry's first done criterion: `uconsole --hub HOST api ...` against
    /// an in-process hub with the demo device, and its exit codes.
    #[test]
    fn wp_p2tm_api_calls_the_hub_and_exits_by_its_code() {
        let store = scratch("api");
        let running = start(&options(&store), false).unwrap();
        let port = running.control.port();
        let pin = pin_of(&running);
        let target = client(port, Some(&pin), &store);
        let get = json!({"serial": DEMO, "path": "vol.master"});

        // Paired with the PIN: control. A read and a write.
        assert_eq!(api_call(&target, "param.get", get.clone()), exit::OK);
        assert_eq!(
            api_call(
                &target,
                "param.set",
                json!({"serial": DEMO, "path": "vol.master", "value": -30.0})
            ),
            exit::OK
        );
        assert_eq!(api_call(&target, "inventory.list", json!({})), exit::OK);
        assert_eq!(api_call(&target, "schedule.list", json!({})), exit::OK);
        assert_eq!(api_call(&target, "alarm.list", json!({})), exit::OK);
        assert_eq!(api_call(&target, "health.get", json!({})), exit::OK);
        // not_found, bad_request, unknown_type; hub.stats is the admin's.
        assert_eq!(
            api_call(
                &target,
                "param.get",
                json!({"serial": "NOPE", "path": "vol.master"})
            ),
            exit::NO_DEVICE
        );
        assert_eq!(
            api_call(&target, "param.get", json!({"path": "vol.master"})),
            status::BAD_REQUEST
        );
        assert_eq!(
            api_call(&target, "no.such_message", json!({})),
            status::BAD_REQUEST
        );
        assert_eq!(api_call(&target, "hub.stats", json!({})), status::DENIED);

        // denied: a viewer may read, not write.
        let cid = running.control.auth().clients()[0].cid;
        assert!(command(&running, &format!("role {cid} viewer"), false));
        assert_eq!(api_call(&target, "param.get", get.clone()), exit::OK);
        assert_eq!(
            api_call(
                &target,
                "param.set",
                json!({"serial": DEMO, "path": "vol.master", "value": -20.0})
            ),
            status::DENIED
        );

        // limit_exceeded: an admin sets a profile, control crosses it.
        assert!(command(&running, &format!("role {cid} admin"), false));
        assert_eq!(api_call(&target, "hub.stats", json!({})), exit::OK);
        let handle = running.handle.devices()[0].handle;
        assert_eq!(
            api_call(
                &target,
                "limits.set",
                json!({"handle": handle, "profile": {"masterVolumeMaxDb": -10.0}})
            ),
            exit::OK
        );
        assert!(command(&running, &format!("role {cid} control"), false));
        assert_eq!(
            api_call(
                &target,
                "param.set",
                json!({"serial": DEMO, "path": "vol.master", "value": -3.0})
            ),
            status::LIMIT
        );
        assert_eq!(
            api_call(
                &target,
                "param.set",
                json!({"serial": DEMO, "path": "vol.master", "value": -12.0})
            ),
            exit::OK
        );

        // offline: known to the hub, not attached; nothing sent.
        running.handle.detach(handle).unwrap();
        assert_eq!(api_call(&target, "param.get", get), status::OFFLINE);
        drop(running);
    }

    /// The prompt's `inventory`, `alarms` and `schedule`, and a service off.
    #[test]
    fn wp_p2tm_the_prompt_reads_the_inventory_alarms_and_schedule() {
        let store = scratch("prompt");
        let data = store.join("data");
        let mut o = options(&store.join("store"));
        o.data = Some(data.clone());
        let running = start(&o, false).unwrap();
        wait_until("the inventory has the demo device", || {
            ask_service(&running, "inventory.list", json!({})).is_ok_and(|v| {
                v["devices"]
                    .as_array()
                    .is_some_and(|d| d.iter().any(|d| d["serial"] == DEMO))
            })
        });
        let alarms = ask_service(&running, "alarm.list", json!({"state": "active"})).unwrap();
        assert!(alarms["alarms"].is_array(), "{alarms}");
        let rules = ask_service(&running, "schedule.list", json!({})).unwrap();
        assert!(
            rules["rules"].is_array() && rules["zone"].is_string(),
            "{rules}"
        );
        for line in ["inventory", "alarms", "schedule", "api-key list"] {
            assert!(command(&running, line, false), "{line}");
            assert!(command(&running, line, true), "{line} --json");
        }
        // The services keep their files in --data, not in the store.
        wait_until("inventory.json in the data folder", || {
            data.join("inventory.json").exists()
        });
        assert!(!store.join("store").join("inventory.json").exists());
        // Writes are not the prompt's reader's: it is a viewer.
        let denied = ask_service(&running, "inventory.forget", json!({"serial": DEMO}));
        assert!(denied.is_err_and(|e| e.contains("denied")));
        drop(running);

        let mut o = options(&scratch("prompt-off"));
        o.services.schedule = false;
        o.services.inventory = false;
        let running = start(&o, false).unwrap();
        let off = ask_service(&running, "schedule.list", json!({})).unwrap_err();
        assert!(off.contains("off"), "{off}");
        assert!(ask_service(&running, "inventory.list", json!({})).is_err());
        assert!(ask_service(&running, "alarm.list", json!({})).is_ok());
        assert!(command(&running, "schedule", false));
        drop(running);
    }

    /// The entry's second: an API key minted at the prompt works for REST,
    /// and stops working when revoked there.
    #[test]
    fn wp_p2tm_a_key_minted_at_the_prompt_works_for_rest() {
        let store = scratch("rest");
        let mut o = options(&store);
        o.rest = true;
        let running = start(&o, false).unwrap();
        let port = running.control.port();
        let key = mint_key(running.control.auth(), "Script", Role::Control).unwrap();
        assert_eq!((key.role, key.key.len()), (Role::Control, 43));
        assert!(command(&running, "api-key new Dashboard viewer", false));
        assert!(command(&running, "api-key new Bot", true));
        assert!(command(&running, "api-key list", false));
        let keys: Vec<_> = running
            .control
            .auth()
            .clients()
            .into_iter()
            .filter(|c| c.kind == ClientKind::Api)
            .collect();
        assert_eq!(keys.len(), 3);
        assert!(
            keys.iter()
                .any(|c| c.name == "Dashboard" && c.role == Role::Viewer)
        );
        assert!(
            keys.iter()
                .any(|c| c.name == "Bot" && c.role == Role::Control)
        );

        let path = format!("/api/v1/devices/{DEMO}/params/vol.master");
        let (code, body) = http(port, "GET", &path, Some(&key.key));
        assert_eq!(code, 200, "{body}");
        assert!(body.contains("\"value\""), "{body}");
        let (code, _) = http(port, "GET", "/api/v1/health", Some(&key.key));
        assert_eq!(code, 200);
        let (code, _) = http(port, "GET", &path, None);
        assert_eq!(code, 401);

        // A console is not an API key; an unknown cid is none.
        assert!(revoke_key(running.control.auth(), 9999, |_| true).is_err());
        assert!(command(
            &running,
            &format!("api-key revoke {}", key.cid),
            false
        ));
        assert!(
            !running
                .control
                .auth()
                .clients()
                .iter()
                .any(|c| c.cid == key.cid)
        );
        let (code, _) = http(port, "GET", &path, Some(&key.key));
        assert_eq!(code, 401, "a revoked key");
        drop(running);

        // Without --rest, /api/v1/ is not there.
        let running = start(&options(&store), false).unwrap();
        let bot = running
            .control
            .auth()
            .clients()
            .into_iter()
            .find(|c| c.name == "Bot")
            .unwrap();
        let (code, _) = http(running.control.port(), "GET", "/api/v1/health", None);
        assert_eq!(code, 404);
        // A console is refused by `api-key revoke`.
        let pin = pin_of(&running);
        let target = client(running.control.port(), Some(&pin), &store);
        assert_eq!(api_call(&target, "inventory.list", json!({})), exit::OK);
        let console = running
            .control
            .auth()
            .clients()
            .into_iter()
            .find(|c| c.kind == ClientKind::Console)
            .unwrap();
        let refused = revoke_key(running.control.auth(), console.cid, |c| {
            running.control.revoke(c)
        });
        assert!(matches!(refused, Err(Failure::Usage(m)) if m.contains("console")));
        assert!(
            running
                .control
                .auth()
                .clients()
                .iter()
                .any(|c| c.cid == console.cid)
        );
        assert!(
            running
                .control
                .auth()
                .clients()
                .iter()
                .any(|c| c.cid == bot.cid)
        );
        drop(running);
    }

    /// `uconsole hub api-key ...` with the hub stopped, through the store; refused
    /// while it runs.
    #[test]
    fn wp_p2tm_api_keys_are_minted_and_revoked_with_the_hub_stopped() {
        let store = scratch("offline");
        assert!(offline_auth(&store, |a| mint_key(a, "Early", Role::Viewer)).is_err());
        let mut o = options(&store);
        o.rest = true;
        let running = start(&o, false).unwrap();
        let busy = offline_auth(&store, |a| mint_key(a, "Now", Role::Viewer));
        assert!(busy.is_err(), "the hub holds its store");
        drop(running);

        let key = offline_auth(&store, |a| mint_key(a, "Cron", Role::Viewer)).unwrap();
        assert_eq!(key.role, Role::Viewer);
        assert!(offline_keys(&store, false).is_ok());
        let running = start(&o, false).unwrap();
        let port = running.control.port();
        let path = format!("/api/v1/devices/{DEMO}/params/vol.master");
        let (code, body) = http(port, "GET", &path, Some(&key.key));
        assert_eq!(code, 200, "{body}");
        // A viewer key may not write.
        let (code, _) = http(
            port,
            "POST",
            &format!("/api/v1/devices/{DEMO}/presets/1/load"),
            Some(&key.key),
        );
        assert_eq!(code, 403);
        drop(running);

        offline_auth(&store, |a| revoke_key(a, key.cid, |c| a.revoke(c))).unwrap();
        assert!(offline_auth(&store, |a| revoke_key(a, key.cid, |c| a.revoke(c))).is_err());
        let running = start(&o, false).unwrap();
        let (code, _) = http(running.control.port(), "GET", &path, Some(&key.key));
        assert_eq!(code, 401);
        drop(running);
    }
}

// -- WP P2CC3 --
/// `uconsole --demo hub`'s device has the diagnostics extension: the hub's
/// Identify works on it, and its meters are the demo's own.
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests_wp_p2cc3 {
    use super::*;
    use crate::api::tests_wp_p2tm::api_call;
    use dspi_transport::Transport;
    use dspi_tui::shell::connect::LinkTarget;

    #[test]
    fn wp_p2cc3_the_demo_hubs_device_answers_identify() {
        // The demo device's meters, unchanged.
        let mut ours = demo_device();
        let mut demo = FakeDevice::demo();
        let op = dspi_proto::generated::opcodes::REQ_GET_STATUS;
        let len = u16::try_from(DEMO_STATUS.len()).unwrap();
        assert_eq!(
            ours.control_in(op, 9, len).unwrap(),
            demo.control_in(op, 9, len).unwrap()
        );

        let store = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("wp_p2cc3")
            .join("identify");
        let _ = std::fs::remove_dir_all(&store);
        std::fs::create_dir_all(&store).unwrap();
        let mut o = match parse(
            &["--port", "0", "--loopback", "--no-keep-awake"],
            true,
            Some(PathBuf::from("/store")),
        ) {
            Ok(HubCommand::Run(o)) => o,
            other => panic!("{other:?}"),
        };
        o.name = Some("P2CC3 test hub".into());
        o.store = store.clone();
        let running = start(&o, false).unwrap();
        let pin = running
            .control
            .auth()
            .begin_pairing(Instant::now())
            .unwrap()
            .pin;
        let mut l = LinkTarget::parse(&format!("127.0.0.1:{}", running.control.port())).unwrap();
        l.pin = Some(pin);
        l.tokens = Some(store.join("link-tokens.toml"));
        let target = Target::Link(l);
        assert_eq!(
            api_call(
                &target,
                "device.identify",
                json!({"serial": "DEMO0000DSPI0001", "seconds": 1})
            ),
            exit::OK
        );
        assert!(!command(&running, "quit", false));
    }
}
