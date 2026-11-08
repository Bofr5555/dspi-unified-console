//! `dspi` - terminal control for DSPi audio processors.
//!
//! M1 scope: connect, discover, and dump state. The TUI and the one-shot command
//! grammar arrive in later milestones; this binary exists now so that every
//! layer below it is exercised against real hardware rather than only the mock.

// -- WP TM15 --
// All CLI output goes through these. They shadow std's macros of the same names
// for everything below them in this file and in the modules declared below, so
// a new `println!` is covered without anyone having to remember. Keep them above
// every `mod` line: a macro_rules macro is only in scope after its definition.
macro_rules! println {
    () => { $crate::stdio::out(format_args!(""), true) };
    ($($arg:tt)*) => { $crate::stdio::out(format_args!($($arg)*), true) };
}
macro_rules! print {
    ($($arg:tt)*) => { $crate::stdio::out(format_args!($($arg)*), false) };
}
macro_rules! eprintln {
    () => { $crate::stdio::err(format_args!(""), true) };
    ($($arg:tt)*) => { $crate::stdio::err(format_args!($($arg)*), true) };
}
macro_rules! eprint {
    ($($arg:tt)*) => { $crate::stdio::err(format_args!($($arg)*), false) };
}

/// Standard output and error that outlive whoever is reading them.
///
/// std's macros panic when a write fails, which `dspi dump | head -5` makes
/// routine. Here a reader that has gone only ends the output: the command
/// finishes and exits with the status it would have had, so a device change
/// is never left half done by a closed pipe, and a refusal is still a refusal.
/// A loop that prints until stopped must check [`reader_gone`] to end.
mod stdio {
    use std::fmt::Arguments;
    use std::io::{ErrorKind, Write};
    use std::sync::atomic::{AtomicBool, Ordering};

    static READER_GONE: AtomicBool = AtomicBool::new(false);

    /// Has standard output's reader closed its end?
    pub fn reader_gone() -> bool {
        READER_GONE.load(Ordering::Relaxed)
    }

    fn write(out: &mut impl Write, args: Arguments<'_>, newline: bool) -> std::io::Result<()> {
        out.write_fmt(args)?;
        if newline {
            out.write_all(b"\n")?;
        }
        Ok(())
    }

    /// Write to standard output. Any failure other than a closed reader, such
    /// as a full disk, loses output someone wanted, so it ends the process and
    /// is reported like any other failed write to a file.
    pub fn out(args: Arguments<'_>, newline: bool) {
        // The test harness only captures what std's own macros print.
        if cfg!(test) {
            std::print!("{args}{}", if newline { "\n" } else { "" });
            return;
        }
        if reader_gone() {
            return;
        }
        let result = write(&mut std::io::stdout().lock(), args, newline);
        match result {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::BrokenPipe => {
                READER_GONE.store(true, Ordering::Relaxed);
            }
            Err(e) => {
                err(
                    format_args!("dspi: could not write to standard output: {e}"),
                    true,
                );
                std::process::exit(i32::from(super::exit::TRANSPORT));
            }
        }
    }

    /// Write to standard error. There is nowhere to report a failure of this,
    /// and the command's exit status is what a script branches on, so it stands.
    pub fn err(args: Arguments<'_>, newline: bool) {
        if cfg!(test) {
            std::eprint!("{args}{}", if newline { "\n" } else { "" });
            return;
        }
        let _ = write(&mut std::io::stderr().lock(), args, newline);
    }
}

// -- WP TM12 --
mod autoeq_update;
// -- WP TM10 --
mod doctor;
// -- WP TM9 --
mod link;
mod transfer;
// -- WP TM3 --
mod flash;
// -- WP TM11 --
mod hub;

use std::process::ExitCode;

use dspi_cmd::{Command, Context};
use dspi_proto::registry::{Kind, REGISTRY, by_path};
use dspi_proto::value::Value;

use dspi_session::ext_state::ExtState;
use dspi_session::{Outcome, Session};
use dspi_transport::{Transport, TransportError, list_devices};
use dspi_tui::shell::connect::{LinkTarget, OpenError, Target, printable};
// -- WP TM4 --
use dspi_tui::shell::read_only;

/// Exit codes are part of the interface: scripts branch on these.
mod exit {
    pub const OK: u8 = 0;
    pub const USAGE: u8 = 1;
    pub const NO_DEVICE: u8 = 2;
    /// The device accepted the write and then kept a different value.
    pub const REJECTED: u8 = 3;
    pub const TRANSPORT: u8 = 4;
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flags: Vec<&str> = args.iter().map(String::as_str).collect();

    // Where the device is comes off first, so the rest reads the same on every
    // link: `dspi --demo dump` and `dspi dump --demo` are one command.
    let (target, flags) = match parse_target(&flags) {
        Ok(parsed) => parsed,
        Err(e) => {
            eprintln!("dspi: {e}");
            return ExitCode::from(exit::USAGE);
        }
    };
    // -- WP TM26 -- `dspi --dry-run set ...` is `dspi set ... --dry-run`.
    let flags = command_first(flags);
    // -- WP TM11 -- Before a PIN is read or a `--hub` looked for: a hub runs here.
    // -- WP WB7 -- `web` is a hub too.
    if matches!(flags.first(), Some(&"hub" | &"web"))
        && let Some(code) = hub::refuse_target(&target)
    {
        return ExitCode::from(code);
    }
    let target = match resolve_pin(target, std::env::var("DSPI_PIN").ok(), read_stdin_line) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("dspi: {e}");
            return ExitCode::from(exit::USAGE);
        }
    };
    // -- WP TM9 --
    let target = match dspi_tui::shell::connect::locate_hub(target, &link::Person) {
        Ok(t) => t,
        Err(e) => return ExitCode::from(fail_open(e)),
    };
    let target = &target;

    let json = flags.contains(&"--json");
    let palette = match flag_value(&flags, "--theme") {
        Some(name) => match dspi_tui::theme::Palette::parse(name) {
            Some(p) => p,
            None => {
                eprintln!(
                    "dspi: no theme called `{name}`; try one of: {}",
                    dspi_tui::theme::Palette::NAMES.join(", ")
                );
                return ExitCode::from(exit::USAGE);
            }
        },
        // The config file's `theme`, then the calm palette.
        None => dspi_tui::settings::AppConfig::load()
            .theme
            .as_deref()
            .and_then(dspi_tui::theme::Palette::parse)
            .unwrap_or(dspi_tui::theme::Palette::Calm),
    };

    // A script beats every other reading of the arguments: `-f` is explicit,
    // and a non-terminal standard input with nothing else to do can only mean
    // someone is piping commands in.
    let script = flag_value(&flags, "-f").or_else(|| flag_value(&flags, "--file"));
    if script.is_some() {
        return ExitCode::from(cmd_script(target, script, &flags, json));
    }
    if wants_stdin_script(&flags, std::io::IsTerminal::is_terminal(&std::io::stdin())) {
        return ExitCode::from(cmd_script(target, None, &flags, json));
    }

    let code = match flags.first().copied() {
        Some("--help" | "-h") => {
            usage();
            exit::OK
        }
        Some("--version" | "-V") => {
            version();
            exit::OK
        }
        Some("list") => cmd_list(target, json),
        Some("params") => cmd_params(),
        Some("completions") => cmd_completions(&positional(&flags)),
        Some("doctor") | Some("--doctor") => doctor::run(target),
        Some("raw") => cmd_raw(target, &positional(&flags), json),
        Some("ext") => cmd_ext(
            target,
            &positional(&flags),
            json,
            flag_value(&flags, "--word"),
        ),
        Some("autoeq") => cmd_autoeq(target, &positional(&flags), &flags),
        Some("--install-udev") => doctor::install_udev(),
        Some("screenshot") => cmd_screenshot(target, &positional(&flags), &flags),
        // -- WP TM10 --
        Some("export") => transfer::cmd_export(target, &flags, json),
        Some("import") => transfer::cmd_import(target, &flags, json),
        Some("cs") => transfer::cmd_cs(target, &flags, json),
        // -- WP TM9 --
        Some("link") => link::run(&flags, json),
        // -- WP TM3 --
        Some("flash") => flash::cmd_flash(target, &flags),
        // -- WP TM11 --
        Some("hub") => hub::run(target, &flags, json),
        // -- WP WB7 --
        Some("web") => hub::run(target, &flags, json),
        Some("dump") => cmd_dump(target, json),
        Some("watch") => cmd_watch(target, json),
        // No arguments opens the interface; arguments run one command and exit.
        None => cmd_tui(target, &flags, palette),
        Some(other) if other.starts_with("--") => cmd_tui(target, &flags, palette),
        // Everything else is a command in the shared grammar, so the CLI and the
        // TUI's ':' line accept exactly the same syntax.
        Some(_) => cmd_run(target, &flags, json),
    };

    ExitCode::from(code)
}

/// The words that can follow `--midi` without being a port name: every command
/// and parameter. `--midi=NAME` is there for a port that shares a name with one.
fn is_command_word(word: &str) -> bool {
    /// What this binary adds to the shared grammar's verbs.
    const TERMINAL_VERBS: &[&str] = &[
        "ext",
        "autoeq",
        "screenshot",
        "export",
        "import",
        "watch",
        // -- WP TM10 --
        "cs",
        // -- WP TM9 --
        "link",
        // -- WP TM3 --
        "flash",
        // -- WP TM11 -- `dspi --hub hub` is the `hub` command, not a hub called "hub".
        "hub",
        // -- WP WB7 --
        "web",
    ];
    TERMINAL_VERBS.contains(&word)
        || dspi_cmd::VERBS.iter().any(|(n, _)| *n == word)
        || by_path(word).is_some()
}

// -- WP TM26 --
/// Switches that take no value and mean the same wherever they stand. Before
/// the command, they used to be read as the command itself, which opened the
/// interface (hw-1 D1).
const LEADING_SWITCHES: &[&str] = &["--json", "--quiet", "--dry-run", "--force", "--keep-going"];

/// The flags with the command first, as the dispatch reads them: leading
/// [`LEADING_SWITCHES`] move behind the first word that is not one, when that
/// word is a command or a flag that acts as one (`--version`). Anything else
/// stays as it was, so `dspi --json` alone still opens the interface.
fn command_first(flags: Vec<&str>) -> Vec<&str> {
    const FLAG_COMMANDS: &[&str] = &[
        "--help",
        "-h",
        "--version",
        "-V",
        "--doctor",
        "--install-udev",
    ];
    let lead = flags
        .iter()
        .take_while(|f| LEADING_SWITCHES.contains(f))
        .count();
    let Some(word) = flags.get(lead).copied() else {
        return flags;
    };
    if lead == 0 || (word.starts_with('-') && !FLAG_COMMANDS.contains(&word)) {
        return flags;
    }
    let mut out = Vec::with_capacity(flags.len());
    out.push(word);
    out.extend_from_slice(&flags[..lead]);
    out.extend_from_slice(&flags[lead + 1..]);
    out
}

/// Take the flags that say where the device is, leaving everything else in
/// order. Refuses a contradiction rather than choosing for the user.
fn parse_target<'a>(flags: &[&'a str]) -> Result<(Target, Vec<&'a str>), String> {
    let mut rest = Vec::with_capacity(flags.len());
    let mut device = None;
    let mut demo = false;
    let mut midi: Option<Option<String>> = None;
    let mut link: Option<String> = None;
    let mut pin = None;
    // -- WP TM9 --
    let mut hub: Option<Option<String>> = None;

    let mut it = flags.iter().copied().peekable();
    while let Some(f) = it.next() {
        // `--name=value`, for each flag that takes a value.
        let (name, inline) = match f.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v)),
            _ => (f, None),
        };
        let mut value = |what: &str| -> Result<String, String> {
            match inline {
                Some(v) => Ok(v.to_owned()),
                None => it
                    .next()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{name} needs {what}")),
            }
        };
        match name {
            "--device" => device = Some(value("a serial")?),
            "--pin" => pin = Some(value("the hub's PIN")?),
            "--link" => link = Some(value("a hub address, HOST[:PORT]")?),
            // -- WP TM9 --
            "--hub" => {
                hub = Some(match inline {
                    Some(v) => Some(v.to_owned()),
                    None => it
                        .next_if(|w| !w.starts_with('-') && !is_command_word(w))
                        .map(str::to_owned),
                });
            }
            "--demo" if inline.is_none() => demo = true,
            "--midi" => {
                midi = Some(match inline {
                    Some(v) => Some(v.to_owned()),
                    None => it
                        .next_if(|w| !w.starts_with('-') && !is_command_word(w))
                        .map(str::to_owned),
                });
            }
            _ => rest.push(f),
        }
    }

    // -- WP TM9 -- `hub` here and below.
    let chosen = [demo, midi.is_some(), link.is_some(), hub.is_some()]
        .iter()
        .filter(|c| **c)
        .count();
    if chosen > 1 {
        return Err("choose one of --demo, --midi, --link and --hub".into());
    }
    if pin.is_some() && link.is_none() && hub.is_none() {
        return Err("--pin only means something with --link or --hub".into());
    }
    if device.is_some() && (demo || midi.is_some()) {
        return Err("--device selects a USB device or a hub's device; \
                    use --midi <name> for a MIDI port"
            .into());
    }
    let target = if demo {
        Target::Demo
    } else if let Some(port) = midi {
        Target::Midi { port }
    } else if let Some(wanted) = hub {
        Target::Link(LinkTarget {
            device,
            pin,
            ..LinkTarget::find(wanted)
        })
    } else if let Some(address) = link {
        let parsed = LinkTarget::parse(&address)
            .ok_or_else(|| format!("`{address}` is not a hub address; try HOST[:PORT]"))?;
        Target::Link(LinkTarget {
            device,
            pin,
            ..parsed
        })
    } else {
        Target::Usb { serial: device }
    };
    Ok((target, rest))
}

/// Where a script's PIN comes from, since a `--pin 1234` on the command line is
/// visible to every user in `ps` and stays in the shell's history: `--pin -`
/// reads a line of standard input, and `DSPI_PIN` is used when no `--pin` says
/// otherwise.
fn resolve_pin(
    target: Target,
    env: Option<String>,
    stdin: impl FnOnce() -> Option<String>,
) -> Result<Target, String> {
    let Target::Link(mut l) = target else {
        return Ok(target);
    };
    let usable = |p: String| {
        let p = p.trim().to_owned();
        (!p.is_empty()).then_some(p)
    };
    l.pin = match l.pin.take().as_deref() {
        Some("-") => Some(
            stdin()
                .and_then(usable)
                .ok_or("--pin - found no PIN on standard input")?,
        ),
        Some(p) => Some(p.to_owned()),
        None => env.and_then(usable),
    };
    Ok(Target::Link(l))
}

fn read_stdin_line() -> Option<String> {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).ok()?;
    Some(line)
}

/// Ask for a hub's PIN, when there is a person to ask.
fn prompt_pin(hub: &str, source: &str) -> Option<String> {
    use std::io::{IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        return None;
    }
    if source == "fixed" {
        eprint!("PIN for {hub}: ");
    } else {
        eprint!("Open pairing on {hub}, and type the PIN it shows: ");
    }
    std::io::stderr().flush().ok()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).ok()?;
    let pin = line.trim();
    (!pin.is_empty()).then(|| pin.to_owned())
}

fn flag_value<'a>(flags: &'a [&'a str], name: &str) -> Option<&'a str> {
    flags
        .iter()
        .position(|f| *f == name)
        .and_then(|i| flags.get(i + 1))
        .copied()
}

fn version() {
    println!(
        "dspi {} (protocol from DSPi {} @ {})",
        env!("CARGO_PKG_VERSION"),
        dspi_proto::generated::provenance::BRANCH,
        dspi_proto::generated::provenance::SHORT,
    );
    println!(
        "wire format V{}, {} vendor opcodes, for firmware {}",
        dspi_proto::generated::wire::WIRE_FORMAT_VERSION,
        dspi_proto::generated::ALL_OPCODES.len(),
        dspi_proto::packets::FirmwareVersion::expected(),
    );
    // -- WP TM22 --
    for line in dspi_tui::settings::about::credit_lines() {
        println!("{line}");
    }
}

fn usage() {
    println!(
        "\
dspi - terminal control for DSPi audio processors

USAGE:
    dspi                     open the interface
    dspi dump                connect and print the full device state
    dspi screenshot [w] [h] [screen]
                             render one frame of the interface as text
                             (--ansi for colour; --fixture, or no device,
                             draws the built-in example device)
    dspi watch               print the device's notifications as they arrive
    dspi list                list every connected DSPi
                             (--midi: the MIDI ports; --link: the hub's devices)
    dspi link scan           list the DSPi Link hubs on this network
                             (--timeout <seconds> to listen longer, --json)
    dspi params              list every parameter this build knows
    dspi get <path> [i..]    read one parameter
    dspi set <path> [i..] v  write one parameter, and confirm it
    dspi <path> [i..] v      the same, without the `set`
    dspi eq <ch> <band> <type> [freq] [q] [gain]
                             set a whole filter band in one transfer
    dspi export <file|->     write the device out; - is standard output.
                             .dspipreset or .json: the whole-device document;
                             otherwise, a Console-compatible filter file
                             (--format preset|filters|filters-json to choose;
                             `export - --json` is the whole-device document)
    dspi import <file|->     apply a document; - is standard input. Goes by
                             what the file holds, or --format
                             (--volume, --hardware to include those blocks)
                             (--map-legacy for pre-2026 channel names)
                             (--dry-run to report only, --json for the report)
    dspi cs export <file|->  write the control-surface configuration
    dspi cs import <file|->  apply one, live; bindings move pins, so only here
                             (--dry-run to report only, --save to store it)
    dspi flash [file]        install a .uf2 on this machine's USB-connected
                             board; with no file, only reboot it into its
                             bootloader. Asks first, or --force (--drive PATH
                             to name the drive, --dry-run to report only).
                             An image that risks the board's core voltage also
                             needs --accept-core-voltage-risk and the answer
                             it asks for typed; --force does not count
    dspi completions <shell> generate shell completions
    dspi doctor              diagnose connection problems
    dspi undo                put back the value the last change replaced
    dspi redo                re-apply the change undo reversed
    dspi raw <op> <len> [v]  read any vendor opcode and hex-dump the reply
    dspi raw out <op> <v> <hex...>
                             write any vendor opcode with a payload
    dspi raw war <op> <v>    issue a write-as-read and print its status byte
    dspi ext [list]          list the firmware's extension channel commands
    dspi ext xfeed-map [p0 p1 p2 p3]
                             read, or set, the crossfeed output pairs:
                             each L:R (output numbers), default, or 0xNN
    dspi ext vreg            read the core-voltage status
    dspi ext vreg set <V|default> | trial <V> | trial-end
             | auto on|off | clear
                             core-voltage operations (V: 1.20, 1250, 0x0E).
                             Raising it above 1.20 V from what the unit reports
                             now asks for the word OVERVOLT, or takes it from
                             --word OVERVOLT in a script; --force does not count
    dspi autoeq search <q>   find a headphone correction profile
    dspi autoeq apply <id>   apply one to the input channels
    dspi autoeq rebuild --yes
                             download every profile from GitHub again
    dspi autoeq update <url> replace the database with a download
    dspi autoeq import <f>   replace the database with a file
    dspi autoeq reset        go back to the built-in database
    dspi -f <script>         run one command per line, `#` starts a comment
    dspi < script            the same, from standard input
    dspi --install-udev      install the Linux udev rule (needs root)
    dspi hub [run]           share this machine's devices on the network, with
                             PIN pairing; clients, roles: dspi hub --help
    dspi web                 the web console on this machine, at
                             http://localhost:11916/console/ (dspi hub --local)
    dspi --version           show app and protocol versions, and credits

EXAMPLES:
    dspi get vol.user
    dspi set vol.user -18
    dspi get eq.freq 8 3
    dspi set bass.drive 12
    dspi eq usb.1 3 peak 2856 3.58 -8.6
    dspi ch.delay i2s.1.l 5
    dspi raw out 0x42 0 08 03 01 00
    printf 'vol.user -18\\nbass.on on\\n' | dspi

OPTIONS:
    --device <serial>        target a specific device (on USB or on a hub)
    --demo                   use the built-in simulated device
    --midi [PORT]            connect over USB MIDI; PORT is part of the port's
                             name (--midi=PORT if it could be read as a command)
    --link <host[:port]>     connect through a DSPi Link hub
    --hub [NAME]             the same, finding the hub on this network; NAME is
                             part of its name, its id or its address (--hub=NAME
                             if it could be read as a command). Several hubs and
                             no NAME: asks, or fails if there is nobody to ask
    --pin <pin>              the hub's PIN, when pairing without a terminal.
                             On the command line it shows in `ps` and in shell
                             history: for scripts, use `--pin -` (one line of
                             standard input) or the DSPI_PIN environment variable
                             (the token is kept in the config directory)
    --json                   machine-readable output
    --no-animation           no connect reveal or easing
    --lite                   reduce redraw rate, for a Pi or a slow link
    --fixture                open the interface on the built-in example
                             device (no hardware needed)
    --theme <name>           calm (default), console, amber, dark or mono
    --dry-run                report what would be written, write nothing
    --quiet                  do not echo each change
    --keep-going             in a script, carry on past a failing line
    --force                  agree to a change that needs a decision, such as
                             an output enable that turns another one off
"
    );
}

/// What a MIDI host or a hub can see, in the shape `list` prints for USB.
fn list_remote(target: &Target, json: bool) -> u8 {
    let listed = match target.list(&prompt_pin) {
        Ok(l) => l,
        Err(e) => return fail_open(e),
    };
    if listed.is_empty() {
        if json {
            println!("[]");
        } else {
            eprintln!("No DSPi found.");
        }
        return exit::NO_DEVICE;
    }
    if json {
        let arr: Vec<_> = listed
            .iter()
            .map(|d| serde_json::json!({ "serial": d.serial, "name": d.name, "detail": d.detail }))
            .collect();
        println!("{}", serde_json::Value::Array(arr));
    } else {
        for d in &listed {
            // `Target::list` has made these printable.
            println!("{}  {}  {}", d.name, d.serial, d.detail);
        }
    }
    exit::OK
}

fn cmd_list(target: &Target, json: bool) -> u8 {
    if !target.is_usb() {
        return list_remote(target, json);
    }
    let devices = match list_devices() {
        Ok(d) => d,
        Err(e) => return fail(e),
    };

    if devices.is_empty() {
        if json {
            println!("[]");
        } else {
            eprintln!("No DSPi found.");
        }
        return exit::NO_DEVICE;
    }

    if json {
        let arr: Vec<_> = devices
            .iter()
            .map(|d| {
                serde_json::json!({
                    "serial": d.serial,
                    "short": d.short_name(),
                    "bus": d.bus_id,
                    "address": d.address,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr).unwrap());
    } else {
        for d in &devices {
            println!("{}  bus {} addr {}", d.short_name(), d.bus_id, d.address);
        }
    }
    exit::OK
}

/// `dspi dump --json`'s document.
// -- WP TM4 --
fn dump_json(
    caps: &dspi_session::Capabilities,
    ext: &ExtState,
) -> Result<serde_json::Value, serde_json::Error> {
    let mut doc = serde_json::to_value(caps)?;
    if let Some(o) = doc.as_object_mut() {
        o.insert("extensions".into(), serde_json::json!(ext.supported));
        o.insert("bulk_source".into(), serde_json::json!(ext.bulk_source()));
        o.insert(
            "read_only".into(),
            serde_json::json!(read_only::is_read_only(caps)),
        );
    }
    Ok(doc)
}

fn cmd_dump(target: &Target, json: bool) -> u8 {
    // -- WP TM4 --
    // Through the connect path, so another wire format is described rather
    // than refused, and says that it is read-only.
    let mut session = match connect(target) {
        Ok(s) => s,
        Err(c) => return c,
    };
    // Absent is an answer: only the link failing is an error, and then the
    // rest of what this prints is already known.
    let ext = ExtState::probe(session.transport_mut()).unwrap_or_default();
    let caps = session.capabilities();

    if json {
        match dump_json(caps, &ext).and_then(|d| serde_json::to_string_pretty(&d)) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("dspi: could not serialise state: {e}");
                return exit::TRANSPORT;
            }
        }
        return exit::OK;
    }

    println!(
        "{}  {}  firmware {}  wire V{}",
        printable(&caps.serial),
        caps.platform.name(),
        printable(&caps.firmware),
        caps.wire_format
    );
    // -- WP TM4 --
    if let Some(banner) = read_only::banner(caps) {
        println!("{banner}");
    }
    println!(
        "{} channels: {} in, {} out, {} bands each",
        caps.num_channels, caps.num_inputs, caps.num_outputs, caps.max_bands
    );
    if let Some(b) = &caps.build_info {
        println!("build {}  {}", printable(&b.describe), printable(&b.date));
    }
    if let Some(p) = caps.active_preset {
        println!("active preset: {p}");
    }

    let label = dspi_tui::shell::connect::ext_label(&ext);
    if !label.is_empty() {
        println!("{label}");
    }

    println!("\nchannels");
    for c in &caps.channels {
        println!(
            "  {:>2}  {:<12}  {:<16}  {}",
            c.index,
            printable(&c.slug),
            printable(&c.name),
            if c.is_output { "output" } else { "input" }
        );
    }

    println!("\nfeatures");
    for f in &caps.features {
        println!(
            "  {}  {:<22}  {}",
            if f.present { "yes" } else { " no" },
            f.name,
            f.evidence
        );
    }

    if let Some(cs) = &caps.cs {
        println!(
            "\ncontrol surfaces: caps v{}, {} slots, {} nouns, {} IR commands",
            cs.caps_version, cs.max_bindings, cs.noun_count, cs.max_ir_commands
        );
    }
    if let Some(sg) = &caps.siggen {
        println!(
            "signal generator: {} types, {} outputs, multitone max {}",
            sg.type_count, sg.output_channels, sg.multitone_max
        );
    }

    exit::OK
}

/// Match a file's channel section to a channel on this device.
///
/// Index-keyed headers are authoritative. A name-keyed header is matched
/// against the device's own channel names.
///
/// Files written before the firmware's unified channel model use names like
/// "Master L" and "Sub" for a topology that no longer exists: there is no master
/// bus now, and inputs are first-class channels. Mapping those onto the current
/// model is a guess, right for a stereo setup and wrong for an eight-channel
/// one, so it happens only when the user asks for it with `--map-legacy`.
fn resolve_channel(
    bank: &dspi_session::filterfile::ChannelBank,
    caps: &dspi_session::Capabilities,
    map_legacy: bool,
) -> Option<u8> {
    if let Some(ix) = bank.index {
        let ch = if bank.is_output {
            caps.num_inputs.checked_add(ix)?
        } else {
            ix
        };
        return (ch < caps.num_channels).then_some(ch);
    }

    let want = bank.header.trim().to_ascii_lowercase();
    if let Some(c) = caps
        .channels
        .iter()
        .find(|c| c.name.to_ascii_lowercase() == want)
    {
        return Some(c.index);
    }

    if !map_legacy {
        return None;
    }
    legacy_channel(&want, caps)
}

/// The pre-V16 names, mapped onto the current model on request.
///
/// The old master pair becomes the first two inputs, and the old outputs become
/// the first outputs in order. Stated explicitly on import so the user can see
/// what was assumed.
fn legacy_channel(name: &str, caps: &dspi_session::Capabilities) -> Option<u8> {
    let out = |n: u8| {
        caps.num_inputs
            .checked_add(n)
            .filter(|c| *c < caps.num_channels)
    };
    match name {
        "master l" | "master left" => (caps.num_inputs > 0).then_some(0),
        "master r" | "master right" => (caps.num_inputs > 1).then_some(1),
        "out l" | "out left" => out(0),
        "out r" | "out right" => out(1),
        // The subwoofer was always the last output.
        "sub" | "pdm" | "pdm sub" => caps.num_channels.checked_sub(1),
        _ => None,
    }
}

/// Render one frame of the interface to stdout.
///
/// Useful for documentation and for checking a layout at a size you do not have
/// a terminal for, such as the 80x24 floor.
fn cmd_screenshot(target: &Target, args: &[&str], flags: &[&str]) -> u8 {
    let width: u16 = args.first().and_then(|a| a.parse().ok()).unwrap_or(120);
    let height: u16 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(40);
    let screen = args.get(2).copied().unwrap_or("overview");
    let ansi = flags.contains(&"--ansi");

    // Without a device (or with --fixture) the frame comes from the same
    // fixture the gallery and the tests use, so documentation needs no
    // hardware.
    let fixture = flags.contains(&"--fixture") || open(target).is_err();
    let (mut session, state, ext) = if fixture {
        let state = if screen == "settings" {
            dspi_tui::settings::demo::state()
        } else {
            dspi_tui::shell::fixture::state()
        };
        let mock = dspi_transport::MockTransport::new().answering_everything(vec![0u8; 64]);
        let Some(session) = Session::new(Box::new(mock), state.caps.clone()) else {
            return exit::TRANSPORT;
        };
        (session, state, None)
    } else {
        match target.connect(&prompt_pin) {
            Ok(c) => (c.session, c.state, Some(c.ext)),
            Err(e) => return fail_open(e),
        }
    };
    let theme = dspi_tui::Theme::new(
        dspi_tui::theme::Palette::Calm,
        if ansi {
            dspi_tui::theme::ColorDepth::detect()
        } else {
            dspi_tui::theme::ColorDepth::TrueColor
        },
        dspi_tui::perf::detect_glyphs(),
    );
    let mut live = dspi_tui::live::Live::new(
        state,
        theme,
        dspi_tui::perf::Performance::lite(),
        Box::new(dspi_tui::live::ConsoleScreens::new()),
    );
    if let Some(ext) = &ext {
        live.set_connection(target, ext);
    }
    live.refresh_presets(&mut session);
    live.tick(&mut session, None);
    if !live.show(&mut session, screen) {
        eprintln!("dspi: unknown screen {screen}; see `dspi screenshot --help`");
        return exit::USAGE;
    }
    live.tick(&mut session, None);
    let frame = if ansi {
        dspi_tui::render_frame_ansi(width, height, |area, buf| live.draw(area, buf))
    } else {
        dspi_tui::render_frame(width, height, |area, buf| live.draw(area, buf))
    };
    println!("{frame}");
    exit::OK
}

fn cmd_tui(target: &Target, flags: &[&str], palette: dspi_tui::theme::Palette) -> u8 {
    // -- WP TM26 -- Without a terminal there is no interface to draw, and
    // ratatui would panic trying (hw-1 D1).
    {
        use std::io::IsTerminal;
        if let Some(why) = no_terminal(
            std::io::stdin().is_terminal(),
            std::io::stdout().is_terminal(),
        ) {
            eprintln!("dspi: {why}");
            return exit::USAGE;
        }
    }
    let lite = flags.contains(&"--lite");
    let no_animation = flags.contains(&"--no-animation");

    // `--fixture` runs the interface on the built-in example device, the one
    // `screenshot` draws, so the terminal handling can be exercised on a
    // machine (or under Wine, or in CI) with no DSPi attached.
    if flags.contains(&"--fixture") {
        let state = dspi_tui::shell::fixture::state();
        let mock = dspi_transport::MockTransport::new().answering_everything(vec![0u8; 64]);
        let Some(mut session) = Session::new(Box::new(mock), state.caps.clone()) else {
            return exit::TRANSPORT;
        };
        return run_tui(&mut session, state, None, lite, no_animation, palette);
    }

    // One bulk read seeds everything the packet covers, so the first frame
    // shows the person's tuning rather than a flat line.
    let mut c = match target.connect(&prompt_pin) {
        Ok(c) => c,
        Err(e) => return fail_open(e),
    };
    // -- WP TM9 -- The target as reached: with the hub's id, for a reconnect.
    let reached = c.link.clone().map_or_else(|| target.clone(), Target::Link);
    let link = Some((&reached, &c.ext));
    run_tui(&mut c.session, c.state, link, lite, no_animation, palette)
}

// -- WP TM26 --
/// Why the interface cannot open, when standard input or output is not a
/// terminal.
fn no_terminal(stdin_is_terminal: bool, stdout_is_terminal: bool) -> Option<String> {
    let which = match (stdin_is_terminal, stdout_is_terminal) {
        (true, true) => return None,
        (false, true) => "standard input is",
        (true, false) => "standard output is",
        (false, false) => "standard input and output are",
    };
    Some(format!(
        "the interface needs a terminal, and {which} not one; \
         give a command to run instead (see `dspi --help`)"
    ))
}

fn run_tui(
    session: &mut Session,
    state: dspi_session::DeviceState,
    connection: Option<(&Target, &ExtState)>,
    lite: bool,
    no_animation: bool,
    palette: dspi_tui::theme::Palette,
) -> u8 {
    let theme = dspi_tui::Theme::new(
        palette,
        dspi_tui::theme::ColorDepth::detect(),
        dspi_tui::perf::detect_glyphs(),
    );
    // Every meter read is a round trip over a hub or a SysEx exchange.
    let slow = connection.is_some_and(|(t, _)| t.is_slow());
    let mut perf = if lite || slow {
        dspi_tui::perf::Performance::lite()
    } else {
        dspi_tui::perf::Performance::detect()
    };
    if no_animation {
        perf.animate = false;
    }
    let mut live = dspi_tui::live::Live::new(
        state,
        theme,
        perf,
        Box::new(dspi_tui::live::ConsoleScreens::new()),
    );
    if let Some((target, ext)) = connection {
        live.set_connection(target, ext);
    }

    match dspi_tui::live::run(live, session) {
        Ok(()) => exit::OK,
        Err(e) => {
            eprintln!("dspi: {e}");
            exit::TRANSPORT
        }
    }
}

/// Print notifications as the device pushes them, until interrupted.
///
/// This is the Console's Interrupt Monitor as a one-shot: turn the OS volume
/// slider and watch `user_volume` arrive with source `UAC1`. With `--json`,
/// one object per line.
fn cmd_watch(target: &Target, json: bool) -> u8 {
    let mut session = match connect(target) {
        Ok(s) => s,
        Err(c) => return c,
    };
    let source = match session.with_transport(|t| Ok(t.notifications())) {
        Ok(Some(s)) => s,
        _ => {
            eprintln!("dspi: this transport has no notification endpoint");
            return exit::TRANSPORT;
        }
    };
    let notes = match dspi_session::Notifications::start(source) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("dspi: cannot start the notification reader: {e}");
            return exit::TRANSPORT;
        }
    };
    if !json {
        println!(
            "watching {} (Ctrl-C to stop)",
            printable(&session.capabilities().serial)
        );
    }
    let (kind, own_session) = session
        .with_transport(|t| Ok((t.caps().kind, t.session_id())))
        .unwrap_or((dspi_transport::TransportCaps::USB.kind, None));
    let started = std::time::Instant::now();
    loop {
        // -- WP TM15 --
        // Nobody is reading and this never ends by itself. With a device that
        // stays quiet, this is only noticed at the first print after the close.
        if stdio::reader_gone() {
            return exit::OK;
        }
        if notes.is_disconnected() {
            eprintln!("dspi: device disconnected");
            return exit::TRANSPORT;
        }
        let Some(n) = notes.next(std::time::Duration::from_millis(500)) else {
            continue;
        };
        let t = started.elapsed().as_secs_f64();
        if json {
            let (name, detail) = describe_event(&n, kind, own_session);
            println!(
                "{{\"t\":{t:.3},\"seq\":{},\"lost\":{},\"event\":\"{name}\",\"detail\":\"{}\"}}",
                n.seq,
                n.lost,
                detail.replace('"', "'")
            );
        } else {
            let (name, detail) = describe_event(&n, kind, own_session);
            let lost = if n.lost { "  (packets lost)" } else { "" };
            println!("{t:8.3}  #{:<3}  {name:<18} {detail}{lost}", n.seq);
        }
    }
}

/// A one-line description of a notification, naming the bulk field it
/// touched where it can.
///
/// The source's words depend on the link it came over (PLAN §1.2).
fn describe_event(
    n: &dspi_session::Notification,
    kind: dspi_transport::TransportKind,
    own_session: Option<u16>,
) -> (&'static str, String) {
    use dspi_session::Event;
    use dspi_tui::shell::connect::attribution;
    let says = |source: dspi_session::Source| attribution(kind, own_session, source, n.origin);
    match &n.event {
        Event::Idle => ("idle", String::new()),
        Event::MasterVolume(db) => ("master_volume", format!("{db:.1} dB (legacy)")),
        Event::ParamChanged {
            offset,
            source,
            bytes,
        } => {
            let section = dspi_proto::wire::BulkPacket::section_at(*offset as usize).unwrap_or("?");
            let value = match bytes.len() {
                4 => format!(
                    "{}",
                    f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
                ),
                1 => format!("{}", bytes[0]),
                _ => hex_string(bytes),
            };
            (
                "param_changed",
                format!(
                    "{section}+{} = {value} [{}]",
                    *offset as usize - section_offset(section),
                    says(*source)
                ),
            )
        }
        Event::BulkInvalidated { source } => ("bulk_invalidated", says(*source).to_string()),
        Event::PresetLoaded { slot } => ("preset_loaded", format!("slot {}", slot + 1)),
        Event::InputFormat { channels } => ("input_format", format!("{channels} channels")),
        Event::SiggenState {
            state,
            reason,
            signal_type,
            channel,
        } => (
            "siggen_state",
            format!("state {state} reason {reason} type {signal_type} channel {channel}"),
        ),
        Event::AdatState {
            enabled,
            active,
            pin,
        } => (
            "adat_state",
            format!("enabled {enabled} active {active} pin {pin}"),
        ),
        Event::I2sSlaveState { state, rate_hz } => {
            ("i2s_slave_state", format!("state {state} {rate_hz} Hz"))
        }
        Event::IrLearn {
            state,
            protocol,
            code,
        } => (
            "cs_ir_learn",
            format!("state {state} protocol {protocol} code 0x{code:08X}"),
        ),
        Event::AdatInputState {
            state,
            rate_hz,
            clock_mode,
        } => (
            "adat_input_state",
            format!("state {state} {rate_hz} Hz clock {clock_mode}"),
        ),
        Event::CsAux {
            slot,
            state,
            level_q8,
            source,
        } => (
            "cs_aux",
            format!(
                "slot {slot} {} level {:.1}% [{}]",
                if *state != 0 { "on" } else { "off" },
                *level_q8 as f32 / 256.0,
                says(*source)
            ),
        ),
        Event::Unknown { id, bytes } => ("unknown", format!("0x{id:02X} {}", hex_string(bytes))),
    }
}

fn section_offset(name: &str) -> usize {
    dspi_proto::generated::SECTIONS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, o, _)| *o)
        .unwrap_or(0)
}

/// Search the AutoEQ database, or apply a profile.
fn cmd_autoeq(target: &Target, args: &[&str], flags: &[&str]) -> u8 {
    use dspi_session::autoeq;

    // -- WP TM12 -- Before the load: a database that will not load is what these mend.
    if let Some(code) = autoeq_update::run(args, flags) {
        return code;
    }

    let db = match autoeq::load() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("dspi: {e}");
            return exit::USAGE;
        }
    };

    match args.first().copied() {
        Some("search") => {
            let query = args[1..].join(" ");
            if query.is_empty() {
                eprintln!("dspi: autoeq search needs something to look for");
                return exit::USAGE;
            }
            let hits = db.search(&query);
            if hits.is_empty() {
                println!("Nothing matching \"{query}\".");
                return exit::OK;
            }
            for e in hits.iter().take(20) {
                println!(
                    "{:<44}{} bands, preamp {:+.1} dB",
                    e.id,
                    e.filters.len(),
                    e.preamp
                );
            }
            if hits.len() > 20 {
                println!("... and {} more; narrow the search.", hits.len() - 20);
            }
            exit::OK
        }

        Some("apply") => {
            let Some(id) = args.get(1) else {
                eprintln!("dspi: autoeq apply needs a profile id from `autoeq search`");
                return exit::USAGE;
            };
            let Some(entry) = db.get(id) else {
                eprintln!("dspi: no profile called `{id}`");
                return exit::USAGE;
            };

            let mut session = match connect(target) {
                Ok(s) => s,
                Err(c) => return c,
            };
            if flags.contains(&"--dry-run") {
                session.dry_run = true;
            }

            // Default to every input: a headphone correction belongs on what is
            // being listened to, and applying it to one side only would be worse
            // than not applying it.
            let caps = session.capabilities().clone();
            let channels: Vec<u8> = match args.get(2) {
                Some(name) => {
                    match caps
                        .channels
                        .iter()
                        .find(|c| c.slug == *name || c.name == *name)
                    {
                        Some(c) => vec![c.index],
                        None => {
                            eprintln!("dspi: no channel called `{name}`");
                            return exit::USAGE;
                        }
                    }
                }
                None => (0..caps.num_inputs).collect(),
            };

            let mut applied = 0usize;
            let mut unsupported: Vec<&str> = Vec::new();

            for ch in &channels {
                let _ = session.write("pre", &[*ch], dspi_proto::value::Value::Float(entry.preamp));

                for (i, f) in entry.filters.iter().enumerate() {
                    if i as u8 >= caps.max_bands {
                        break;
                    }
                    let Some(kind) = autoeq::filter_type(&f.kind) else {
                        if !unsupported.contains(&f.kind.as_str()) {
                            unsupported.push(&f.kind);
                        }
                        continue;
                    };
                    let packet = dspi_proto::value::EqParamPacket {
                        channel: *ch,
                        band: i as u8,
                        filter_type: kind,
                        bypass: false,
                        freq: f.freq,
                        q: f.q,
                        gain_db: f.gain,
                        qp: None,
                    };
                    if session.write_band(&packet).is_ok() {
                        applied += 1;
                    }
                }

                // Bands the profile does not use must be cleared, or whatever
                // was there before survives underneath the correction.
                for i in entry.filters.len()..caps.max_bands as usize {
                    let packet = dspi_proto::value::EqParamPacket {
                        channel: *ch,
                        band: i as u8,
                        filter_type: dspi_proto::FilterType::Flat,
                        bypass: false,
                        freq: 1000.0,
                        q: 0.707,
                        gain_db: 0.0,
                        qp: None,
                    };
                    let _ = session.write_band(&packet);
                }
            }

            println!(
                "{} {} to {} channel(s): {applied} bands, preamp {:+.1} dB",
                if session.dry_run {
                    "Would apply"
                } else {
                    "Applied"
                },
                entry.id,
                channels.len(),
                entry.preamp
            );
            if entry.filters.len() > caps.max_bands as usize {
                println!(
                    "Note: the profile has {} bands, this device has {}; the rest were dropped.",
                    entry.filters.len(),
                    caps.max_bands
                );
            }
            if !unsupported.is_empty() {
                println!(
                    "Shapes this build does not know: {}",
                    unsupported.join(", ")
                );
            }
            exit::OK
        }

        _ => {
            println!(
                "AutoEQ database: {} profiles, {} copy{}",
                db.entries.len(),
                match db.origin {
                    autoeq::Origin::User => "your",
                    autoeq::Origin::Bundled => "the built-in",
                },
                if db.generated_at.is_empty() {
                    String::new()
                } else {
                    format!(", generated {}", db.generated_at)
                }
            );
            println!("\n  dspi autoeq search <words>");
            println!("  dspi autoeq apply <id> [channel]");
            // -- WP TM12 --
            println!("  dspi autoeq rebuild --yes");
            println!("  dspi autoeq update <https url>");
            println!("  dspi autoeq import <file>");
            println!("  dspi autoeq reset");
            exit::OK
        }
    }
}

/// Issue any vendor opcode directly.
///
/// This is the escape hatch that means a firmware feature shipping before app
/// support is still reachable on the day it lands, and it is the fastest way to
/// see what a device actually returns when a decode looks wrong.
/// All three directions the protocol actually uses. An IN read is the default
/// because it is the safe one; `out` and `war` are spelled out because they
/// change the device.
/// The fork's extension channel (REQ_EXTENSION 0x00). Firmware without it
/// stalls every request, which is reported as "not supported", not a crash.
fn cmd_ext(target: &Target, args: &[&str], json: bool, word: Option<&str>) -> u8 {
    use dspi_session::ext::{self, ExtError, VregOp};

    // Parse before touching the device, so a typo costs no connect.
    enum Do {
        List,
        MapRead,
        MapWrite(ext::PairMap),
        Vreg,
        VregOp(VregOp),
    }
    let parsed: Result<Do, ExtError> = match args {
        [] | ["list"] => Ok(Do::List),
        ["xfeed-map"] => Ok(Do::MapRead),
        ["xfeed-map", pairs @ ..] => ext::PairMap::parse(pairs)
            .map(Do::MapWrite)
            .map_err(ExtError::from),
        ["vreg"] => Ok(Do::Vreg),
        ["vreg", "set", "default"] => Ok(Do::VregOp(VregOp::Set(0))),
        ["vreg", "set", v] => ext::parse_vreg(v)
            .map(|c| Do::VregOp(VregOp::Set(c)))
            .map_err(ExtError::from),
        ["vreg", "trial", v] => ext::parse_vreg(v)
            .map(|c| Do::VregOp(VregOp::Trial(c)))
            .map_err(ExtError::from),
        ["vreg", "trial-end"] => Ok(Do::VregOp(VregOp::TrialEnd)),
        ["vreg", "auto", "on"] => Ok(Do::VregOp(VregOp::Auto(true))),
        ["vreg", "auto", "off"] => Ok(Do::VregOp(VregOp::Auto(false))),
        ["vreg", "clear"] => Ok(Do::VregOp(VregOp::Clear)),
        _ => Err(ExtError::BadArgument(
            "usage: dspi ext [list] | xfeed-map [pairs] | vreg [set|trial|trial-end|auto|clear]"
                .into(),
        )),
    };
    let action = match parsed {
        Ok(a) => a,
        Err(e) => {
            eprintln!("dspi: {e}");
            return exit::USAGE;
        }
    };

    let mut session = match connect(target) {
        Ok(s) => s,
        Err(c) => return c,
    };
    let result = session
        .with_transport(|t| {
            Ok(match action {
                Do::List => ext::list(t).map(|ids| {
                    if json {
                        let v: Vec<_> = ids
                            .iter()
                            .map(|id| serde_json::json!({ "id": id, "name": ext::name(*id) }))
                            .collect();
                        serde_json::Value::Array(v).to_string()
                    } else if ids.is_empty() {
                        "extension channel present, no sub-commands".into()
                    } else {
                        ids.iter()
                            .map(|id| format!("0x{id:04X}  {}", ext::name(*id)))
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                }),
                Do::MapRead => ext::read_pair_map(t).map(|m| map_text(&m, json)),
                Do::MapWrite(m) => ext::write_pair_map(t, m).map(|m| map_text(&m, json)),
                Do::Vreg => ext::read_core_vreg(t).map(|s| {
                    let f = s.fields();
                    if json {
                        let o: serde_json::Map<_, _> = f
                            .into_iter()
                            .map(|(k, v)| (k.to_string(), serde_json::Value::String(v)))
                            .collect();
                        serde_json::Value::Object(o).to_string()
                    } else {
                        f.iter()
                            .map(|(k, v)| format!("{k:<14}{v}"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                }),
                Do::VregOp(op) => vreg_send(t, op, word, &mut prompt_overvolt_word).map(|()| {
                    if json {
                        "{\"ok\":true}".into()
                    } else {
                        format!("sent {op:?}; read `dspi ext vreg` to confirm")
                    }
                }),
            })
        })
        .unwrap_or_else(|e| Err(ExtError::Transport(e)));

    match result {
        Ok(text) => {
            println!("{text}");
            exit::OK
        }
        Err(e @ (ExtError::Unsupported { .. } | ExtError::Refused { .. })) => {
            eprintln!("dspi: {e}");
            exit::REJECTED
        }
        Err(e @ ExtError::NotApplied(_)) => {
            eprintln!("dspi: {e}");
            exit::REJECTED
        }
        Err(e @ ExtError::WordRequired { .. }) => {
            eprintln!("dspi: {e}");
            eprintln!(
                "      On a terminal it asks; in a script, pass --word {}. --force does not count.",
                dspi_session::overvolt::WORD
            );
            exit::REJECTED
        }
        Err(e @ ExtError::BadArgument(_)) => {
            eprintln!("dspi: {e}");
            exit::USAGE
        }
        Err(ExtError::Transport(e)) => fail(e),
    }
}

/// One core-voltage request (WP CG14). With `--word`, that is all it gets: a
/// script must not end up at a prompt. Without, a refusal for the word asks
/// through `ask`, once. `--force` plays no part.
fn vreg_send(
    t: &mut dyn dspi_transport::Transport,
    op: dspi_session::ext::VregOp,
    word: Option<&str>,
    ask: &mut dyn FnMut() -> Option<String>,
) -> Result<(), dspi_session::ext::ExtError> {
    use dspi_session::ext::{self, ExtError};
    match ext::core_vreg_op(t, op, word) {
        Err(ExtError::WordRequired { code }) if word.is_none() => match ask() {
            Some(typed) => ext::core_vreg_op(t, op, Some(&typed)),
            None => Err(ExtError::WordRequired { code }),
        },
        other => other,
    }
}

/// Ask for the word, when there is a person to ask. Only the line ending is
/// taken off: the match is exact.
fn prompt_overvolt_word() -> Option<String> {
    use std::io::{IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        return None;
    }
    eprint!(
        "This raises the core voltage above 1.20 V. Type {} to go ahead: ",
        dspi_session::overvolt::WORD
    );
    std::io::stderr().flush().ok()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).ok()?;
    Some(line.trim_end_matches(['\r', '\n']).to_owned())
}

fn map_text(m: &dspi_session::ext::PairMap, json: bool) -> String {
    if json {
        let pairs: Vec<_> = (0..4)
            .map(|p| {
                let (l, r) = m.pair(p);
                serde_json::json!({ "pair": p, "left": l, "right": r, "default": m.0[p] == 0 })
            })
            .collect();
        serde_json::json!({ "bytes": m.0, "pairs": pairs }).to_string()
    } else {
        format!(
            "{}\nbytes: {:02x} {:02x} {:02x} {:02x}",
            m, m.0[0], m.0[1], m.0[2], m.0[3]
        )
    }
}

fn cmd_raw(target: &Target, args: &[&str], json: bool) -> u8 {
    match args.first().copied() {
        Some("out") => raw_out(target, &args[1..], json),
        Some("war") => raw_war(target, &args[1..], json),
        _ => raw_in(target, args, json),
    }
}

fn parse_num(s: &str) -> Option<u32> {
    s.strip_prefix("0x")
        .and_then(|h| u32::from_str_radix(h, 16).ok())
        .or_else(|| s.parse().ok())
}

fn raw_in(target: &Target, args: &[&str], json: bool) -> u8 {
    let (Some(op_s), Some(len_s)) = (args.first(), args.get(1)) else {
        eprintln!("dspi: raw needs an opcode and a length, e.g. `dspi raw 0x87 32`");
        eprintln!("      an optional third argument is wValue.");
        return exit::USAGE;
    };
    let (Some(opcode), Some(len)) = (parse_num(op_s), parse_num(len_s)) else {
        eprintln!("dspi: opcode and length must be numbers");
        return exit::USAGE;
    };
    let value = args.get(2).and_then(|v| parse_num(v)).unwrap_or(0) as u16;

    // -- WP TM4 --
    let mut t = match target.open_raw(&prompt_pin) {
        Ok(t) => t,
        Err(e) => return fail_open(e),
    };

    match t.control_in(opcode as u8, value, len as u16) {
        Ok(d) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "opcode": opcode,
                        "wValue": value,
                        "length": d.len(),
                        "bytes": hex_string(&d),
                    })
                );
                return exit::OK;
            }
            for (i, chunk) in d.chunks(16).enumerate() {
                let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02X}")).collect();
                let ascii: String = chunk
                    .iter()
                    .map(|b| {
                        if b.is_ascii_graphic() {
                            *b as char
                        } else {
                            '.'
                        }
                    })
                    .collect();
                println!("{:04X}  {:<48}  {ascii}", i * 16, hex.join(" "));
            }
            println!("\n{} bytes", d.len());
            exit::OK
        }
        Err(e) => {
            eprintln!("dspi: {e}");
            transfer_exit(&e)
        }
    }
}

/// `dspi raw out <op> <wValue> <hex bytes...>`
fn raw_out(target: &Target, args: &[&str], json: bool) -> u8 {
    let (Some(op_s), Some(val_s)) = (args.first(), args.get(1)) else {
        eprintln!("dspi: raw out needs an opcode, a wValue and the payload,");
        eprintln!("      e.g. `dspi raw out 0x42 0 08 03 01 00`");
        return exit::USAGE;
    };
    let (Some(opcode), Some(value)) = (parse_num(op_s), parse_num(val_s)) else {
        eprintln!("dspi: opcode and wValue must be numbers");
        return exit::USAGE;
    };
    let payload = match parse_hex(&args[2..]) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("dspi: {e}");
            return exit::USAGE;
        }
    };

    // WP CG14: this path could not carry the confirmation word.
    if dspi_session::overvolt::is_core_vreg_write(opcode as u8, value as u16) {
        eprintln!("dspi: core-voltage requests go through `dspi ext vreg`, not `raw out`");
        return exit::USAGE;
    }

    // -- WP TM4 --
    let mut t = match target.open_raw(&prompt_pin) {
        Ok(t) => t,
        Err(e) => return fail_open(e),
    };

    match t.control_out(opcode as u8, value as u16, &payload) {
        Ok(()) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "opcode": opcode,
                        "wValue": value,
                        "sent": payload.len(),
                        "bytes": hex_string(&payload),
                    })
                );
            } else {
                println!("Sent {} bytes to 0x{opcode:02X}.", payload.len());
            }
            exit::OK
        }
        Err(e) => {
            eprintln!("dspi: {e}");
            transfer_exit(&e)
        }
    }
}

/// `dspi raw war <op> <wValue>`
///
/// Write-as-read: a mutation issued as a one-byte IN with the parameters in
/// `wValue`. Several of the pin and clock setters are only reachable this way,
/// and the byte that comes back is the status code, not data.
fn raw_war(target: &Target, args: &[&str], json: bool) -> u8 {
    let Some(opcode) = args.first().and_then(|s| parse_num(s)) else {
        eprintln!("dspi: raw war needs an opcode and a wValue, e.g. `dspi raw war 0xC2 0x010E`");
        return exit::USAGE;
    };
    let value = args.get(1).and_then(|v| parse_num(v)).unwrap_or(0) as u16;

    // -- WP TM4 --
    let mut t = match target.open_raw(&prompt_pin) {
        Ok(t) => t,
        Err(e) => return fail_open(e),
    };

    match t.control_in(opcode as u8, value, 1) {
        Ok(d) => {
            let status = d.first().copied().unwrap_or(0);
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "opcode": opcode,
                        "wValue": value,
                        "status": status,
                    })
                );
            } else {
                println!("status 0x{status:02X}");
            }
            // The status byte is the device's verdict, so a non-zero one is a
            // refusal and a script needs to see it as one.
            if status == 0 {
                exit::OK
            } else {
                exit::REJECTED
            }
        }
        Err(e) => {
            eprintln!("dspi: {e}");
            transfer_exit(&e)
        }
    }
}

fn hex_string(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join("")
}

/// Hex bytes, however the user chose to space them: `DE AD BE EF`, `deadbeef`,
/// `0xDE,0xAD` all mean the same four or two bytes.
fn parse_hex(args: &[&str]) -> Result<Vec<u8>, String> {
    let joined: String = args
        .join("")
        .chars()
        .filter(|c| !matches!(c, ',' | ':' | '-' | '_' | ' '))
        .collect();
    let joined = joined.replace("0x", "").replace("0X", "");

    if joined.is_empty() {
        return Err("raw out needs at least one payload byte".into());
    }
    if !joined.len().is_multiple_of(2) {
        return Err(format!(
            "`{joined}` is {} hex digits; bytes come in pairs",
            joined.len()
        ));
    }
    (0..joined.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&joined[i..i + 2], 16)
                .map_err(|_| format!("`{}` is not a hex byte", &joined[i..i + 2]))
        })
        .collect()
}

fn cmd_completions(args: &[&str]) -> u8 {
    let Some(shell) = args.first() else {
        eprintln!(
            "dspi: which shell? one of: {}",
            dspi_cmd::shell::SHELLS.join(", ")
        );
        return exit::USAGE;
    };
    match dspi_cmd::shell::generate(shell) {
        Some(script) => {
            print!("{script}");
            exit::OK
        }
        None => {
            eprintln!(
                "dspi: no completions for `{shell}`; try one of: {}",
                dspi_cmd::shell::SHELLS.join(", ")
            );
            exit::USAGE
        }
    }
}

/// Parse with the shared grammar, then run it.
///
/// Parsing needs the device's channel names, so this connects first. That costs
/// a little latency on a bad command, but it means `dspi ch.delay i2s.1.l 5`
/// works with the names the device actually reports.
fn cmd_run(target: &Target, flags: &[&str], json: bool) -> u8 {
    let args = positional_all(flags);
    let mut s = match connect(target) {
        Ok(s) => s,
        Err(c) => return c,
    };
    if flags.contains(&"--dry-run") {
        s.dry_run = true;
    }

    let ctx = context_for(&s);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let cmd = match dspi_cmd::parse(&refs, &ctx) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("dspi: {e}");
            return exit::USAGE;
        }
    };

    run(&mut s, cmd, json, flags.contains(&"--quiet"), force(flags))
}

fn force(flags: &[&str]) -> bool {
    flags.contains(&"--force")
}

// ---------------------------------------------------------------------------
// Batching
// ---------------------------------------------------------------------------

/// Run a file, or standard input, one command per line.
///
/// The device is opened once for the whole script rather than once per line,
/// which is the point: a hundred `eq` lines against a fresh connection each
/// time would spend all of its time probing.
fn cmd_script(target: &Target, path: Option<&str>, flags: &[&str], json: bool) -> u8 {
    let text = match path {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("dspi: could not read {p}: {e}");
                return exit::USAGE;
            }
        },
        None => {
            let mut buf = String::new();
            if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf) {
                eprintln!("dspi: could not read standard input: {e}");
                return exit::USAGE;
            }
            buf
        }
    };

    let lines = script_lines(&text);
    if lines.is_empty() {
        return exit::OK;
    }

    let mut s = match connect(target) {
        Ok(s) => s,
        Err(c) => return c,
    };
    if flags.contains(&"--dry-run") {
        s.dry_run = true;
    }

    let ctx = context_for(&s);
    let quiet = flags.contains(&"--quiet");
    let keep_going = flags.contains(&"--keep-going");
    let force = force(flags);
    let mut first_failure = exit::OK;

    for (n, line) in lines.iter().enumerate() {
        let toks = dspi_cmd::tokenize(line);
        let refs: Vec<&str> = toks.iter().map(String::as_str).collect();
        let code = match dspi_cmd::parse(&refs, &ctx) {
            Ok(cmd) => run(&mut s, cmd, json, quiet, force),
            Err(e) => {
                eprintln!("dspi: line {}: {e}", n + 1);
                exit::USAGE
            }
        };

        if code != exit::OK {
            // The first failure is the one worth reporting: everything after it
            // may only have failed because of it.
            if first_failure == exit::OK {
                first_failure = code;
            }
            if !keep_going {
                eprintln!("dspi: stopped at line {} (`{line}`)", n + 1);
                return first_failure;
            }
        }
    }
    first_failure
}

/// The runnable lines of a script: comments and blank lines dropped.
fn script_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(strip_comment)
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Everything before an unquoted `#`.
///
/// Quotes matter: a preset called "Studio #2" is a name, not a comment.
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..i],
            _ => {}
        }
    }
    line
}

/// Flags that mean "do something other than run commands", so that a piped
/// standard input does not turn `dspi --version` into an empty script.
const NOT_A_SCRIPT: &[&str] = &[
    "--help",
    "-h",
    "--version",
    "-V",
    "--doctor",
    "--install-udev",
    "--lite",
    "--fixture",
];

/// Should this invocation read commands from standard input?
///
/// Only when there is nothing else to do and stdin is not a terminal, so
/// `dspi < script.dspi` and `generate | dspi` work while an interactive `dspi`
/// still opens the interface.
fn wants_stdin_script(flags: &[&str], stdin_is_terminal: bool) -> bool {
    !stdin_is_terminal
        && positional_all(flags).is_empty()
        && !flags.iter().any(|f| NOT_A_SCRIPT.contains(f))
}

fn context_for(s: &Session) -> Context {
    let caps = s.capabilities();
    Context {
        channel_slugs: caps.channels.iter().map(|c| c.slug.clone()).collect(),
        num_inputs: caps.num_inputs,
        num_outputs: caps.num_outputs,
        max_bands: caps.max_bands,
    }
}

fn run(s: &mut Session, cmd: Command, json: bool, quiet: bool, force: bool) -> u8 {
    match cmd {
        Command::Get { path, indices } => {
            let d = by_path(path).expect("the parser only returns known paths");
            // A status or packet row is read at its own length and decoded;
            // a scalar read of one would be its first byte.
            match s.read_whole(path, &indices) {
                Ok(Some(r)) => {
                    if json {
                        println!("{}", serde_json::json!({ "path": path, "value": r.json }));
                    } else {
                        for line in &r.lines {
                            println!("{line}");
                        }
                    }
                    return exit::OK;
                }
                Ok(None) => {}
                Err(e) => {
                    eprintln!("dspi: {e}");
                    return write_exit(&e);
                }
            }
            match s.read(path, &indices) {
                Ok(v) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({ "path": path, "value": show(d.kind, &v) })
                        );
                    } else {
                        println!("{}", show(d.kind, &v));
                    }
                    exit::OK
                }
                Err(e) => {
                    eprintln!("dspi: {e}");
                    write_exit(&e)
                }
            }
        }

        Command::Set {
            path,
            indices,
            value,
        } => {
            let d = by_path(path).expect("the parser only returns known paths");

            // Turning an output on can need the other side of Core 1 turned
            // off, which is a decision rather than a retry, so it is only made
            // when the user says so.
            if force && path == "out.enable" && value.as_bool() == Some(true) {
                return match s.enable_output_confirmed(indices[0]) {
                    Ok(dspi_session::EnableOutcome::Rejected) => {
                        eprintln!("dspi: output {} did not come on.", indices[0]);
                        exit::REJECTED
                    }
                    Ok(_) => {
                        if !quiet {
                            println!("{path} {} on", indices[0]);
                        }
                        exit::OK
                    }
                    Err(e) => {
                        eprintln!("dspi: {e}");
                        write_exit(&e)
                    }
                };
            }

            match s.write(path, &indices, value.clone()) {
                Ok(Outcome::Rejected { sent, actual }) => {
                    eprintln!(
                        "dspi: {path} was not applied. Asked for {}, device kept {}.",
                        show(d.kind, &sent),
                        show(d.kind, &actual)
                    );
                    exit::REJECTED
                }
                Ok(outcome) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "path": path,
                                "value": show(d.kind, &value),
                                "outcome": format!("{outcome:?}"),
                            })
                        );
                    } else if !quiet {
                        println!("{path} = {}", show(d.kind, &value));
                    }
                    exit::OK
                }
                Err(e) => {
                    eprintln!("dspi: {e}");
                    if let dspi_session::WriteError::Core1Conflict { confirm, .. } = &e {
                        eprintln!("      Re-run with --force to {confirm}.");
                    }
                    write_exit(&e)
                }
            }
        }

        Command::SetBand {
            channel,
            band,
            filter_type,
            freq,
            q,
            gain,
            qp,
        } => {
            // One transfer for the whole band, which is how the firmware stores
            // it; setting fields one at a time would be six round trips.
            let packet = dspi_proto::value::EqParamPacket {
                channel,
                band,
                filter_type: dspi_proto::FilterType::from_raw(filter_type),
                bypass: false,
                freq,
                q,
                gain_db: gain,
                qp,
            };
            match s.write_band(&packet) {
                Ok(Outcome::Rejected { .. }) => {
                    eprintln!("dspi: the band was not applied as sent");
                    exit::REJECTED
                }
                Ok(_) => {
                    if !quiet {
                        println!(
                            "band {} on channel {channel} = {} {freq} Hz Q {q} {gain:+} dB",
                            band + 1,
                            dspi_proto::FilterType::from_raw(filter_type).label()
                        );
                    }
                    exit::OK
                }
                Err(e) => {
                    eprintln!("dspi: {e}");
                    write_exit(&e)
                }
            }
        }

        // `dspi undo` on a fresh process has nothing to undo, and says so:
        // the journal lives with the connection. The interface is where these
        // earn their keep; the CLI form exists so a script can walk one back
        // inside a batch.
        Command::Verb { name, .. } if name == "undo" => match s.undo() {
            Ok(None) => {
                println!("Nothing to undo.");
                exit::OK
            }
            Ok(Some(undone)) => {
                for reason in &undone.skipped {
                    println!("Skipped: {reason}.");
                }
                match undone.command {
                    Some(line) => println!("{line}"),
                    None => println!("Nothing to undo."),
                }
                exit::OK
            }
            Err(e) => {
                eprintln!("dspi: {e}");
                write_exit(&e)
            }
        },

        Command::Verb { name, .. } if name == "redo" => match s.redo() {
            Ok(None) => {
                println!("Nothing to redo.");
                exit::OK
            }
            Ok(Some(done)) => {
                if let Some(line) = done.command {
                    println!("{line}");
                }
                exit::OK
            }
            Err(e) => {
                eprintln!("dspi: {e}");
                write_exit(&e)
            }
        },

        Command::Verb { name, .. } => {
            eprintln!("dspi: `{name}` is not usable here");
            exit::USAGE
        }
    }
}

fn cmd_params() -> u8 {
    for line in params_lines() {
        println!("{line}");
    }
    exit::OK
}

/// One line per registry row: the address, its range and what it does.
///
/// The columns are 22 and 18 wide, and an entry longer than its column is
/// followed by two spaces rather than run into the next one: a long list of
/// choices read "all | percussive | sustainedWeight the sub...". Widening
/// the column to the longest entry instead would push every description
/// past 130 characters for the sake of the filter types.
fn params_lines() -> Vec<String> {
    let rows: Vec<(String, String, &str)> = REGISTRY
        .iter()
        .map(|d| {
            let args = match d.target.arity() {
                0 => "",
                1 => " <index>",
                _ => " <index> <index>",
            };
            let range = match d.kind {
                // -- WP CG37 -- the delay line's range, not the registry's 85 ms.
                Kind::Float { .. } if dspi_cmd::is_delay(d.path) => dspi_cmd::delay_range(),
                Kind::Float { unit, min, max } => {
                    format!("{min} to {max}{}", unit.suffix())
                }
                Kind::Int { unit, min, max } => format!("{min} to {max}{}", unit.suffix()),
                Kind::Bool => "on | off".into(),
                Kind::Choice(v) => v.iter().map(|(_, n)| *n).collect::<Vec<_>>().join(" | "),
                Kind::Trigger => "(action)".into(),
                _ => String::new(),
            };
            (format!("{}{args}", d.path), range, d.plain)
        })
        .collect();
    rows.into_iter()
        .map(|(a, r, plain)| {
            let aw = 22.max(a.len() + 2);
            let rw = 18.max(r.len() + 2);
            format!("{a:<aw$}{r:<rw$}{plain}")
        })
        .collect()
}

/// Every positional token, including the first, for the shared grammar.
fn positional_all(flags: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip = false;
    for f in flags {
        if skip {
            skip = false;
            continue;
        }
        if *f == "--device" || *f == "--word" {
            skip = true;
            continue;
        }
        if f.starts_with("--") {
            continue;
        }
        out.push((*f).to_string());
    }
    out
}

/// Positional arguments after the command, excluding flags and their values.
fn positional<'a>(flags: &'a [&'a str]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut skip = false;
    for (i, f) in flags.iter().enumerate().skip(1) {
        if skip {
            skip = false;
            continue;
        }
        if *f == "--device" || *f == "--word" {
            skip = true;
            continue;
        }
        if f.starts_with("--") {
            continue;
        }
        let _ = i;
        out.push(*f);
    }
    out
}

fn connect(target: &Target) -> Result<Session, u8> {
    target.open_session(&prompt_pin).map_err(fail_open)
}

/// Distinguish "you typed something wrong" from "the device is unhappy".
///
/// A script needs to tell these apart: bad input is worth reporting to a human,
/// a transport failure is worth retrying.
fn write_exit(e: &dspi_session::WriteError) -> u8 {
    use dspi_session::WriteError as W;
    match e {
        W::Value(_)
        | W::UnknownParam(_)
        | W::ReadOnly { .. }
        | W::WriteOnly { .. }
        | W::WrongArity { .. }
        | W::BadTarget(..)
        | W::PlatformRange { .. }
        | W::Unavailable { .. } => exit::USAGE,
        // Nothing was written and the old value stands, which is the same
        // thing a script needs to know about a silent rejection.
        W::Core1Conflict { .. } => exit::REJECTED,
        W::Transport(e) => transfer_exit(e),
        // -- WP TM20 -- Sent, and the device never said it finished.
        W::Unsettled { .. } => exit::TRANSPORT,
        // -- WP LK4 -- not sent; the device's capabilities are still being read: retry
        W::Unprobed { .. } => exit::TRANSPORT,
    }
}

/// The exit status for a transfer that failed.
// -- WP TM4 --
fn transfer_exit(e: &TransportError) -> u8 {
    match e {
        // Nothing was written and trying again will not change that: a
        // read-only device, or a hub that does not allow it. Unlike a transport
        // failure, not worth a script's retry.
        TransportError::Denied(_) => exit::REJECTED,
        _ => exit::TRANSPORT,
    }
}

/// Render a value using its parameter's kind, so a choice shows its name rather
/// than the wire number behind it.
fn show(kind: Kind, v: &Value) -> String {
    if let (Kind::Choice(variants), Some(n)) = (kind, v.as_u8())
        && let Some((_, name)) = variants.iter().find(|(raw, _)| *raw == n)
    {
        return (*name).to_string();
    }
    // A channel name, or any other text, is the device's to choose.
    printable(&v.display(kind.unit()))
}

fn open(target: &Target) -> Result<Box<dyn Transport>, OpenError> {
    target.open(&prompt_pin)
}

/// Report a failed open, with the exit code a script branches on.
fn fail_open(e: OpenError) -> u8 {
    match e {
        OpenError::Transport(t) => fail(t),
        // -- WP TM9 --
        other @ OpenError::NoHub { .. } => {
            eprintln!("dspi: {other}");
            exit::NO_DEVICE
        }
        // A hub without the device is a device that is not there; everything
        // else about opening is a transport failure.
        other => {
            eprintln!("dspi: {other}");
            if matches!(
                other,
                OpenError::NeedsPin { .. } | OpenError::PinRefused { .. }
            ) {
                exit::USAGE
            } else {
                exit::TRANSPORT
            }
        }
    }
}

fn fail(e: TransportError) -> u8 {
    eprintln!("dspi: {e}");
    match e {
        TransportError::NotFound | TransportError::SerialNotFound(_) => exit::NO_DEVICE,
        // -- WP TM4 --
        e => transfer_exit(&e),
    }
}

#[cfg(test)]
mod tests_wp_r1 {
    use super::*;

    fn parsed(flags: &[&'static str]) -> (Target, Vec<&'static str>) {
        parse_target(flags).unwrap()
    }

    #[test]
    fn wp_r1_no_flags_is_any_usb_device() {
        assert_eq!(parsed(&["dump"]).0, Target::Usb { serial: None });
        assert_eq!(parsed(&["dump"]).1, vec!["dump"]);
    }

    #[test]
    fn wp_r1_the_device_flag_comes_off_wherever_it_stands() {
        for flags in [&["--device", "X", "dump"][..], &["dump", "--device", "X"]] {
            let (t, rest) = parsed(flags);
            assert_eq!(
                t,
                Target::Usb {
                    serial: Some("X".into())
                }
            );
            assert_eq!(rest, vec!["dump"]);
        }
    }

    #[test]
    fn wp_r1_demo_before_a_command_leaves_the_command() {
        let (t, rest) = parsed(&["--demo", "get", "vol.user", "--json"]);
        assert_eq!(t, Target::Demo);
        assert_eq!(rest, vec!["get", "vol.user", "--json"]);
    }

    #[test]
    fn wp_r1_midi_takes_a_port_only_when_the_next_word_can_be_one() {
        let midi = |p: Option<&str>| Target::Midi {
            port: p.map(str::to_owned),
        };
        assert_eq!(parsed(&["--midi"]).0, midi(None));
        assert_eq!(parsed(&["--midi", "--json"]).0, midi(None));
        let (t, rest) = parsed(&["--midi", "dump"]);
        assert_eq!((t, rest), (midi(None), vec!["dump"]));
        let (t, rest) = parsed(&["--midi", "vol.user", "-18"]);
        assert_eq!((t, rest), (midi(None), vec!["vol.user", "-18"]));
        let (t, rest) = parsed(&["--midi", "Bench", "dump"]);
        assert_eq!((t, rest), (midi(Some("Bench")), vec!["dump"]));
        assert_eq!(parsed(&["--midi=dump", "dump"]).0, midi(Some("dump")));
    }

    #[test]
    fn wp_r1_link_reads_host_port_device_and_pin() {
        let (t, rest) = parsed(&["--link", "hub:99", "--device", "X", "--pin", "1", "dump"]);
        assert_eq!(rest, vec!["dump"]);
        let Target::Link(l) = t else {
            panic!("a hub target");
        };
        assert_eq!((l.host.as_str(), l.port), ("hub", 99));
        assert_eq!(l.device.as_deref(), Some("X"));
        assert_eq!(l.pin.as_deref(), Some("1"));
        let Target::Link(l) = parsed(&["--link=[fe80::1]"]).0 else {
            panic!("a hub target");
        };
        assert_eq!(l.host, "fe80::1");
        assert_eq!(l.port, dspi_link::DEFAULT_PORT);
    }

    #[test]
    fn wp_r1_contradictions_and_missing_values_are_refused() {
        for flags in [
            &["--demo", "--midi"][..],
            &["--demo", "--link", "hub"],
            &["--midi", "--link", "hub"],
            &["--pin", "1"],
            &["--device", "X", "--demo"],
            &["--device", "X", "--midi"],
            &["--link"],
            &["--device"],
            &["--link", "not an address"],
        ] {
            assert!(parse_target(flags).is_err(), "{flags:?}");
        }
    }

    // -- WP TM9 --
    #[test]
    fn wp_tm9_hub_takes_an_optional_name_as_midi_takes_a_port() {
        let wanted = |flags: &[&'static str]| {
            let (t, rest) = parsed(flags);
            let Target::Link(l) = t else {
                panic!("a hub target");
            };
            (l.find.unwrap().wanted, rest.len())
        };
        assert_eq!(wanted(&["--hub"]), (None, 0));
        assert_eq!(wanted(&["--hub", "dump"]), (None, 1));
        assert_eq!(wanted(&["--hub", "--lite"]), (None, 1));
        assert_eq!(wanted(&["--hub", "Studio"]), (Some("Studio".into()), 0));
        assert_eq!(
            wanted(&["--hub", "Studio", "dump"]),
            (Some("Studio".into()), 1)
        );
        assert_eq!(wanted(&["--hub=dump"]), (Some("dump".into()), 0));
        let Target::Link(l) = parsed(&["--hub", "--device", "X", "--pin", "1"]).0 else {
            panic!("a hub target");
        };
        assert_eq!(
            (l.device.as_deref(), l.pin.as_deref()),
            (Some("X"), Some("1"))
        );
        for flags in [
            &["--hub", "--demo"][..],
            &["--hub", "--midi"],
            &["--hub", "--link", "host"],
        ] {
            assert!(parse_target(flags).is_err(), "{flags:?}");
        }
    }

    #[test]
    fn wp_tm9_link_is_a_command_word_not_a_port_or_a_hub_name() {
        assert!(is_command_word("link"));
    }

    #[test]
    fn wp_r1_unrelated_flags_keep_their_order() {
        let (_, rest) = parsed(&["--theme", "calm", "--demo", "--lite"]);
        assert_eq!(rest, vec!["--theme", "calm", "--lite"]);
        let (_, rest) = parsed(&["--theme=amber"]);
        assert_eq!(rest, vec!["--theme=amber"]);
    }

    #[test]
    fn wp_r1_a_pin_comes_from_the_flag_stdin_or_the_environment() {
        let pin = |flags: &[&'static str], env: Option<&str>, stdin: Option<&str>| {
            let target = parsed(flags).0;
            resolve_pin(target, env.map(str::to_owned), || stdin.map(str::to_owned)).map(|t| {
                let Target::Link(l) = t else {
                    panic!("a hub target");
                };
                l.pin
            })
        };
        let hub = ["--link", "hub"];
        assert_eq!(pin(&hub, None, None), Ok(None));
        assert_eq!(pin(&hub, Some("12"), None), Ok(Some("12".into())));
        assert_eq!(pin(&hub, Some("  "), None), Ok(None));
        let flag = ["--link", "hub", "--pin", "34"];
        assert_eq!(pin(&flag, Some("12"), None), Ok(Some("34".into())));
        let dash = ["--link", "hub", "--pin", "-"];
        assert_eq!(pin(&dash, Some("12"), Some("56\n")), Ok(Some("56".into())));
        assert!(pin(&dash, Some("12"), None).is_err());
        assert!(pin(&dash, None, Some("\n")).is_err());
        // The environment means nothing off a hub.
        let usb = parsed(&["dump"]).0;
        assert_eq!(
            resolve_pin(usb.clone(), Some("12".into()), || None),
            Ok(usb)
        );
    }

    #[test]
    fn wp_r1_every_shared_verb_is_a_command_word() {
        for (verb, _) in dspi_cmd::VERBS {
            assert!(is_command_word(verb), "{verb}");
        }
        assert!(is_command_word("watch"));
        assert!(!is_command_word("Bench"));
    }

    #[test]
    fn wp_r1_a_command_runs_on_the_demo_device() {
        let target = Target::Demo;
        assert_eq!(cmd_dump(&target, true), exit::OK);
        assert_eq!(cmd_run(&target, &["get", "vol.user"], true), exit::OK);
    }

    #[test]
    fn wp_r1_a_refused_or_missing_pin_is_a_usage_failure() {
        let e = OpenError::NeedsPin {
            hub: "Hub".into(),
            pin_source: "window".into(),
        };
        assert_eq!(fail_open(e), exit::USAGE);
        let missing = OpenError::Transport(TransportError::NotFound);
        assert_eq!(fail_open(missing), exit::NO_DEVICE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- params -------------------------------------------------------------

    /// A long list of choices once ran straight into the description:
    /// "all | percussive | sustainedWeight the sub...".
    #[test]
    fn params_keep_two_spaces_between_columns() {
        let lines = params_lines();
        assert_eq!(lines.len(), REGISTRY.len());
        for (line, d) in lines.iter().zip(REGISTRY) {
            let address = line.split("  ").next().unwrap();
            assert!(address.starts_with(d.path), "{line}");
            assert!(line.ends_with(d.plain), "{line}");
            let before = &line[..line.len() - d.plain.len()];
            assert!(
                before.ends_with("  "),
                "no gap before the description: {line}"
            );
        }
        let sub = lines.iter().find(|l| l.starts_with("sub.select ")).unwrap();
        assert!(sub.contains("all | percussive | sustained  "), "{sub}");
        let rect = lines.iter().find(|l| l.contains("5y3")).unwrap();
        assert!(rect.contains("solid-state | gz34 | 5u4 | 5y3  "), "{rect}");
        // -- WP CG37 -- the delays show the line's range, not 85 ms.
        for path in ["ch.delay ", "out.delay "] {
            let l = lines.iter().find(|l| l.starts_with(path)).unwrap();
            assert!(l.contains("0 to 42 ms (21 on RP2040)  "), "{l}");
        }
    }

    // -- scripts ------------------------------------------------------------

    #[test]
    fn a_script_drops_blank_lines_and_comments() {
        let text = "\
# a tuning for the small room
vol.user -18

  bass.on on   # and a little weight
";
        assert_eq!(script_lines(text), vec!["vol.user -18", "bass.on on"]);
    }

    /// A `#` inside quotes is part of a name, not the start of a comment; a
    /// preset called "Studio #2" must survive.
    #[test]
    fn a_hash_inside_quotes_is_not_a_comment() {
        assert_eq!(
            strip_comment(r#"preset.name 3 "Studio #2""#).trim(),
            r#"preset.name 3 "Studio #2""#
        );
        assert_eq!(strip_comment("vol.user -18 # loud").trim(), "vol.user -18");
        assert_eq!(strip_comment("# nothing here").trim(), "");
    }

    // -- WP TM26 -- hw-1 D1.
    #[test]
    fn tm26_leading_switches_move_behind_the_command() {
        let f = |a: &[&'static str]| command_first(a.to_vec());
        assert_eq!(
            f(&["--dry-run", "set", "ch.delay", "i2s.1.l", "5"]),
            ["set", "--dry-run", "ch.delay", "i2s.1.l", "5"]
        );
        assert_eq!(
            f(&["--json", "--quiet", "get", "vol.user"]),
            ["get", "--json", "--quiet", "vol.user"]
        );
        assert_eq!(f(&["--json", "--version"]), ["--version", "--json"]);
        // Nothing to move, or no command to move it behind.
        assert_eq!(
            f(&["get", "--json", "vol.user"]),
            ["get", "--json", "vol.user"]
        );
        assert_eq!(f(&["--json"]), ["--json"]);
        assert_eq!(f(&["--json", "--lite"]), ["--json", "--lite"]);
        assert!(f(&[]).is_empty());
        // The command is found where the dispatch looks, and its arguments are
        // still the positional ones.
        let moved = f(&["--dry-run", "set", "vol.user", "-18"]);
        assert_eq!(moved.first(), Some(&"set"));
        assert_eq!(positional_all(&moved), ["set", "vol.user", "-18"]);
    }

    #[test]
    fn tm26_the_interface_refuses_without_a_terminal() {
        assert_eq!(no_terminal(true, true), None);
        for (i, o, which) in [
            (false, true, "standard input is not"),
            (true, false, "standard output is not"),
            (false, false, "standard input and output are not"),
        ] {
            let why = no_terminal(i, o).unwrap();
            assert!(
                why.contains(which) && why.contains("needs a terminal"),
                "{why}"
            );
        }
    }

    #[test]
    fn a_script_of_nothing_but_comments_has_no_lines() {
        assert!(script_lines("# one\n\n   # two\n").is_empty());
    }

    /// Every line has to be a line the parser accepts, or a script silently
    /// does something other than what it reads as.
    #[test]
    fn every_script_line_parses_with_the_shared_grammar() {
        let ctx = dspi_cmd::Context {
            channel_slugs: (0..17).map(|i| format!("ch.{i}")).collect(),
            num_inputs: 8,
            num_outputs: 9,
            max_bands: 10,
        };
        let text = "\
# set up the sub
vol.user -18
eq ch.0 3 peak 2856 3.58 -8.6
get out.gain 0
undo
";
        let lines = script_lines(text);
        assert_eq!(lines.len(), 4);
        for line in &lines {
            let toks = dspi_cmd::tokenize(line);
            let refs: Vec<&str> = toks.iter().map(String::as_str).collect();
            dspi_cmd::parse(&refs, &ctx).unwrap_or_else(|e| panic!("`{line}` does not parse: {e}"));
        }
    }

    /// Piping commands in is a script; asking for the version is not, even
    /// with standard input redirected from a file.
    #[test]
    fn only_an_otherwise_empty_invocation_reads_standard_input() {
        assert!(wants_stdin_script(&[], false));
        assert!(wants_stdin_script(&["--json"], false));
        assert!(wants_stdin_script(&["--device", "ABC"], false));

        // A terminal on standard input means a person, so open the interface.
        assert!(!wants_stdin_script(&[], true));
        // Anything with something else to do keeps doing it.
        assert!(!wants_stdin_script(&["--version"], false));
        assert!(!wants_stdin_script(&["--help"], false));
        assert!(!wants_stdin_script(&["--lite"], false));
        assert!(!wants_stdin_script(&["dump"], false));
        assert!(!wants_stdin_script(&["vol.user", "-18"], false));
    }

    // -- raw ----------------------------------------------------------------

    /// However the user spaced the bytes, they mean the same payload.
    #[test]
    fn hex_payloads_are_read_however_they_are_spaced() {
        let want = vec![0xDE, 0xAD, 0xBE, 0xEF];
        assert_eq!(parse_hex(&["DE", "AD", "BE", "EF"]).unwrap(), want);
        assert_eq!(parse_hex(&["deadbeef"]).unwrap(), want);
        assert_eq!(parse_hex(&["0xDE,0xAD", "0xBE:0xEF"]).unwrap(), want);
        assert_eq!(parse_hex(&["08", "00"]).unwrap(), vec![8, 0]);
    }

    /// A half-typed byte is a mistake worth naming, not something to pad.
    #[test]
    fn an_odd_number_of_hex_digits_is_refused() {
        let e = parse_hex(&["DEA"]).unwrap_err();
        assert!(e.contains("pairs"), "{e}");
        assert!(parse_hex(&[]).is_err());
        assert!(parse_hex(&["ZZ"]).is_err());
    }

    #[test]
    fn opcodes_are_accepted_in_hex_or_decimal() {
        assert_eq!(parse_num("0x42"), Some(0x42));
        assert_eq!(parse_num("66"), Some(66));
        assert_eq!(parse_num("0xFFFF"), Some(0xFFFF));
        assert_eq!(parse_num("nonsense"), None);
    }

    #[test]
    fn bytes_render_back_as_the_hex_they_came_from() {
        assert_eq!(hex_string(&[0xDE, 0xAD]), "DEAD");
        assert_eq!(hex_string(&[]), "");
    }

    // -- flags --------------------------------------------------------------

    #[test]
    fn flag_values_are_read_and_their_values_are_not_positional() {
        let flags = ["set", "vol.user", "-18", "--device", "ABC123", "--json"];
        assert_eq!(flag_value(&flags, "--device"), Some("ABC123"));
        assert_eq!(flag_value(&flags, "--nothing"), None);
        assert_eq!(positional_all(&flags), vec!["set", "vol.user", "-18"]);
    }

    #[test]
    fn force_is_off_unless_it_is_asked_for() {
        assert!(!force(&["out.enable", "8", "on"]));
        assert!(force(&["out.enable", "8", "on", "--force"]));
    }
}

// -- WP TM4 --
#[cfg(test)]
mod tests_wp_tm4 {
    use dspi_transport::mock::Direction;
    use dspi_transport::{FakeDevice, MockTransport};
    use dspi_tui::shell::connect::session_on;

    use super::*;

    /// A device of a wire format newer than this build, and its transfer log.
    pub(crate) fn newer_device() -> (Session, dspi_transport::mock::LogHandle) {
        let mut demo = FakeDevice::demo();
        let platform = demo
            .control_in_upto(dspi_proto::generated::opcodes::REQ_GET_PLATFORM, 0, 7)
            .unwrap();
        let mut image = demo.remote().image();
        image.extend_from_slice(&[0; 16]);
        image[0] = 33;
        let len = image.len() as u16;
        image[6..8].copy_from_slice(&len.to_le_bytes());
        let mock = MockTransport::new()
            .data(dspi_proto::generated::opcodes::REQ_GET_PLATFORM, platform)
            .window(
                dspi_proto::generated::opcodes::REQ_GET_ALL_PARAMS_CHUNK,
                image,
            )
            .answering_everything(vec![0u8; 512]);
        let log = mock.log_handle();
        (session_on(Box::new(mock)).unwrap(), log)
    }

    #[test]
    fn wp_tm4_dump_json_says_whether_the_device_is_read_only() {
        let ext = ExtState::default();
        let (session, _) = newer_device();
        let doc = dump_json(session.capabilities(), &ext).unwrap();
        assert_eq!(doc["read_only"], serde_json::json!(true));
        assert_eq!(doc["wire_format"], serde_json::json!(33));

        let writable = Target::Demo.open_session(&prompt_pin).unwrap();
        let doc = dump_json(writable.capabilities(), &ext).unwrap();
        assert_eq!(doc["read_only"], serde_json::json!(false));
    }

    #[test]
    fn wp_tm4_dump_still_works_on_the_demo_device() {
        assert_eq!(cmd_dump(&Target::Demo, true), exit::OK);
        assert_eq!(cmd_dump(&Target::Demo, false), exit::OK);
    }

    #[test]
    fn wp_tm4_a_set_on_a_read_only_device_is_rejected_and_sends_nothing() {
        let (mut session, log) = newer_device();
        let cmd = Command::Set {
            path: "vol.master",
            indices: Vec::new(),
            value: Value::Float(-3.0),
        };
        assert_eq!(run(&mut session, cmd, true, true, false), exit::REJECTED);
        let log = log.lock().unwrap();
        assert!(!log.iter().any(|e| e.direction == Direction::Out));
    }

    #[test]
    fn wp_tm4_export_on_a_writable_device_has_no_warning() {
        let dir = std::env::current_exe().unwrap();
        let path = dir.with_file_name("wp_tm4_export.dspipreset");
        let path = path.to_str().unwrap();
        assert_eq!(
            transfer::cmd_export(&Target::Demo, &["export", path], true),
            exit::OK
        );
        assert!(std::fs::metadata(path).is_ok(), "it wrote the file");
        let _ = std::fs::remove_file(path);

        // -- WP TM21 --
        let mut demo = Target::Demo.open_session(&prompt_pin).unwrap();
        let doc = dspi_session::preset_file::capture(&mut demo, None).unwrap();
        assert_eq!(read_only::export_warning(&doc, "x"), None);
    }

    #[test]
    fn wp_tm4_a_refusal_is_not_a_transport_failure() {
        assert_eq!(
            transfer_exit(&TransportError::Denied("no".into())),
            exit::REJECTED
        );
        assert_eq!(fail(TransportError::Denied("no".into())), exit::REJECTED);
        assert_eq!(
            transfer_exit(&TransportError::Disconnected),
            exit::TRANSPORT
        );
    }

    #[test]
    fn wp_tm4_raw_passes_on_a_writable_device() {
        assert_eq!(
            raw_out(&Target::Demo, &["0xD2", "0", "00", "00", "A0", "C1"], true),
            exit::OK
        );
    }
}

/// WP CG14: `dspi ext vreg` against the fake, reading what reached it.
#[cfg(test)]
mod tests_wp_cg14 {
    use super::*;
    use dspi_session::ext::{self, ExtError, VregOp};
    use dspi_transport::{Capture, FakeDevice, FakeOptions};

    fn unit_at_1v25() -> FakeDevice {
        let mut d = FakeDevice::new(
            Capture::demo(),
            FakeOptions {
                extensions: true,
                ..FakeOptions::default()
            },
        );
        ext::core_vreg_op(&mut d, VregOp::Set(0x0E), Some("OVERVOLT")).unwrap();
        d
    }

    fn stored(d: &mut FakeDevice) -> u8 {
        ext::read_core_vreg(d).unwrap().stored_sel
    }

    /// A prompt that must not be reached, for a path that has no person.
    fn no_prompt() -> Option<String> {
        panic!("asked");
    }

    #[test]
    fn wp_cg14_the_flag_carries_the_word_and_is_not_positional() {
        let flags = ["ext", "vreg", "set", "1.30", "--word", "OVERVOLT"];
        assert_eq!(flag_value(&flags, "--word"), Some("OVERVOLT"));
        assert_eq!(positional(&flags), vec!["vreg", "set", "1.30"]);

        let mut d = unit_at_1v25();
        vreg_send(&mut d, VregOp::Set(0x0F), Some("OVERVOLT"), &mut no_prompt).unwrap();
        assert_eq!(stored(&mut d), 0x0F);
    }

    #[test]
    fn wp_cg14_a_wrong_flag_is_refused_and_does_not_prompt() {
        for bad in ["overvolt", "OVERVOLT ", "--force", "yes"] {
            let mut d = unit_at_1v25();
            let r = vreg_send(&mut d, VregOp::Set(0x0F), Some(bad), &mut no_prompt);
            assert!(
                matches!(r, Err(ExtError::WordRequired { code: 0x0F })),
                "{bad}"
            );
            assert_eq!(stored(&mut d), 0x0E);
        }
    }

    #[test]
    fn wp_cg14_force_alone_is_never_enough() {
        // No terminal: nobody to ask, whatever the flags say.
        let flags = ["ext", "vreg", "set", "1.30", "--force"];
        assert!(force(&flags));
        assert_eq!(flag_value(&flags, "--word"), None);
        let mut d = unit_at_1v25();
        let r = vreg_send(
            &mut d,
            VregOp::Set(0x0F),
            flag_value(&flags, "--word"),
            &mut || None,
        );
        assert!(matches!(r, Err(ExtError::WordRequired { .. })));
        assert_eq!(stored(&mut d), 0x0E);
    }

    #[test]
    fn wp_cg14_the_prompt_asks_once_and_must_match_exactly() {
        let mut d = unit_at_1v25();
        let mut asked = 0;
        let r = vreg_send(&mut d, VregOp::Trial(0x0F), None, &mut || {
            asked += 1;
            Some("overvolt".into())
        });
        assert!(matches!(r, Err(ExtError::WordRequired { .. })));
        assert_eq!(asked, 1);
        assert!(!ext::read_core_vreg(&mut d).unwrap().trial_active);

        vreg_send(&mut d, VregOp::Trial(0x0F), None, &mut || {
            Some("OVERVOLT".into())
        })
        .unwrap();
        assert_eq!(ext::read_core_vreg(&mut d).unwrap().trial_vreg, 0x0F);
    }

    #[test]
    fn wp_cg14_raw_out_refuses_a_core_voltage_write_before_opening_anything() {
        // A target that cannot open: a usage error, not "no device", shows the
        // refusal came first. So nothing can have reached a device.
        let nobody = Target::Usb {
            serial: Some("CG14-NOT-THERE".into()),
        };
        for args in [
            &["0", "2", "00", "0F"][..],
            &["0x00", "0x0002", "01", "0F"],
            // The same request, spelled to survive the casts to u8 and u16.
            &["0x100", "2", "00", "0F"],
            &["0", "0x10002", "00", "0F"],
            &["0", "2"],
        ] {
            assert_eq!(raw_out(&nobody, args, false), exit::USAGE, "{args:?}");
            assert_eq!(raw_out(&Target::Demo, args, false), exit::USAGE, "{args:?}");
        }
        // The check is that narrow: the same target opens for anything else,
        // and the demo device takes another sub-command.
        assert_eq!(
            raw_out(&Target::Demo, &["0", "1", "00", "00", "00", "00"], false),
            exit::OK
        );
        assert_ne!(
            raw_out(&nobody, &["0", "1", "00", "00", "00", "00"], false),
            exit::USAGE
        );
    }

    #[test]
    fn wp_cg14_lowering_and_level_never_ask() {
        let mut d = unit_at_1v25();
        vreg_send(&mut d, VregOp::Set(0x0E), None, &mut no_prompt).unwrap();
        vreg_send(&mut d, VregOp::Set(0x0D), None, &mut || {
            panic!("1.25 V is still stored: this is lowering")
        })
        .unwrap();
        vreg_send(&mut d, VregOp::Clear, None, &mut no_prompt).unwrap();
    }
}

// -- WP CG37 --
#[cfg(test)]
mod tests_wp_cg37 {
    use super::*;

    /// `dspi set` takes a delay up to the line (42 ms on the demo's RP2350)
    /// and refuses past it or below 0 as bad input, before the wire.
    #[test]
    fn wp_cg37_set_holds_a_delay_to_the_line() {
        let mut s = Target::Demo.open_session(&prompt_pin).unwrap();
        assert_eq!(s.capabilities().platform, dspi_proto::Platform::Rp2350);
        let set = |path, i: u8, ms: f32| Command::Set {
            path,
            indices: vec![i],
            value: Value::Float(ms),
        };
        assert_eq!(
            run(&mut s, set("out.delay", 1, 42.0), false, true, false),
            exit::OK
        );
        assert_eq!(
            run(&mut s, set("out.delay", 1, 42.5), false, true, false),
            exit::USAGE
        );
        assert_eq!(
            run(&mut s, set("ch.delay", 0, 60.0), false, true, false),
            exit::USAGE
        );
        assert_eq!(
            run(&mut s, set("out.delay", 1, -1.0), false, true, false),
            exit::USAGE
        );
        let e = s.write("out.delay", &[1], Value::Float(42.5)).unwrap_err();
        assert_eq!(
            e.to_string(),
            "`out.delay` is limited to 42 ms on this platform"
        );
    }
}

// -- WP DL3 --
#[cfg(test)]
mod tests_wp_dl3 {
    use super::*;

    /// `dspi set out.delay 1 nan` is refused as bad input: the parser does not
    /// take the word, and a NaN that reaches the session some other way is
    /// refused there, before the wire, the stored delay unchanged.
    #[test]
    fn wp_dl3_set_refuses_a_delay_that_is_not_finite() {
        let mut s = Target::Demo.open_session(&prompt_pin).unwrap();
        let ctx = context_for(&s);
        for word in ["nan", "inf", "-inf"] {
            let args = ["set", "out.delay", "1", word];
            let e = dspi_cmd::parse(&args, &ctx).unwrap_err();
            assert_eq!(
                e.to_string(),
                format!("`{word}` is not valid for out.delay; expected a finite number")
            );
        }
        let before = s.read("out.delay", &[1]).unwrap();
        for f in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let cmd = Command::Set {
                path: "out.delay",
                indices: vec![1],
                value: Value::Float(f),
            };
            assert_eq!(run(&mut s, cmd, false, true, false), exit::USAGE);
        }
        assert_eq!(s.read("out.delay", &[1]).unwrap(), before);
    }
}
