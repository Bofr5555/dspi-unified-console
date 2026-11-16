//! The phase-1 PRO commands (WP P1TM, PLAN-CLIENTS 5.9): snapshots, A/B,
//! offline files, document diffs, the limits profile, the installer's lock,
//! a guarded (safe) import and a status view.
//!
//! These run on the core (`DspiCore`), not on `dspi-session` as the rest of
//! the one-shot commands do: the snapshot store, the compare, the guard, the
//! limits and the lock live there, once for every client. Each command opens
//! its own core on the target, names this console's folders for the
//! snapshot store and the limits (`<data dir>/uconsole/snapshots` and
//! `.../limits`) before the device is opened, and turns confirmation tokens
//! on: a gated call (an import, a firmware write) runs only with the token a
//! confirmation minted (`--yes`, or the answer to the question).
//!
//! A one-shot command is one session of the core: an installer unlock lasts
//! for that command only, so `limits set` and `lock set-pin` ask for the PIN
//! themselves when one is set. With no PIN set nothing asks.
//!
//! Over DSPi Link the core opens its own DSPi Link session to the hub (WP
//! TMCORE): the hub judges every write against its own limits and lock as it
//! does for any client, its lock state is seen (`heldBy: hub`), and a guarded
//! import or recall is guarded by the hub too (P1HA, `hub: true`, the
//! `guard.hub_backstop` line).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

pub(crate) mod autoeq;
pub(crate) mod installer;
pub(crate) mod snapshots;

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use dspi_cmd::COMMAND;
use dspi_core::{DspiCore, DspiError, GuardPhase, GuardedApply};
use dspi_tui::shell::connect::{Target, printable};
use serde_json::{Value, json};

use crate::exit;

/// How often a waiting guard is polled.
const GUARD_POLL: Duration = Duration::from_millis(200);

/// The shortest and longest timeout `--guard` takes, in seconds (the
/// coordinator's 5 to 300 s; the core's default is 30).
pub(crate) const GUARD_MIN_S: u32 = 5;
pub(crate) const GUARD_MAX_S: u32 = 300;

/// Where the core keeps this console's snapshots and limits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Stores {
    pub snapshots: Option<PathBuf>,
    pub limits: Option<PathBuf>,
}

impl Stores {
    /// `<local data dir>/uconsole/{snapshots,limits}`, beside the hub's store.
    pub(crate) fn default_dirs() -> Self {
        dirs::data_local_dir()
            .map(|d| Self::under(&d.join(COMMAND)))
            .unwrap_or_default()
    }

    /// Both folders under `dir`.
    pub(crate) fn under(dir: &Path) -> Self {
        Self {
            snapshots: Some(dir.join("snapshots")),
            limits: Some(dir.join("limits")),
        }
    }
}

/// A core opened on the command's device.
pub(crate) struct Opened {
    pub core: Arc<DspiCore>,
    /// The device's serial, as its transport describes it.
    pub serial: String,
    // -- WP TMCORE --
    /// Keeps the device (and a DSPi Link session) open for the command.
    pub _handle: Arc<crate::core_route::CoreHandle>,
}

/// A core with this console's folders named, nothing open yet; confirmation
/// tokens are always required (P1TZ). The folders are named before a device is
/// opened, as `docs/integrator/installer-lock.md` asks.
pub(crate) fn prepare(stores: &Stores) -> Result<Arc<DspiCore>, u8> {
    let core = DspiCore::new();
    crate::core_route::apply_read_only(&core); // -- WP ROMODE --
    // -- P1TK -- every gated call this console makes carries a token; the core
    // requires them from the start (P1TZ), so there is nothing to turn on.
    let name = |dir: &Option<PathBuf>| dir.as_ref().map(|d| d.to_string_lossy().into_owned());
    if let Some(dir) = name(&stores.snapshots) {
        core.snapshot_set_dir(dir).map_err(|e| fail(&e))?;
    }
    if let Some(dir) = name(&stores.limits) {
        core.limits_set_dir(dir).map_err(|e| fail(&e))?;
    }
    Ok(core)
}

/// Open the target on a prepared core: the probe and the settings read, then
/// the lock state (from the hub when it holds one; nothing asked otherwise).
/// -- WP TMCORE -- a hub's device is the core's own DSPi Link session
/// (`core_route::open_core`): `heldBy: hub`, the hub's guard and backstop.
pub(crate) fn open(target: &Target, stores: &Stores) -> Result<Opened, u8> {
    let opened = crate::core_route::open_core(target, stores, &crate::prompt_pin)
        .map_err(crate::core_route::OpenFailure::report)?;
    Ok(Opened {
        core: Arc::clone(&opened.handle.core),
        serial: opened.handle.serial.clone(),
        _handle: opened.handle,
    })
}

// -- WP TMCOREF -- (review tmcore F1)
/// [`open`] for a command that writes to the device: a locked device is
/// unlocked first with the PIN given (`DSPI_INSTALLER_PIN`, `--installer-pin
/// -`), or, with `prompt`, asked for at the terminal (no echo).
pub(crate) fn open_writing(target: &Target, stores: &Stores, prompt: bool) -> Result<Opened, u8> {
    let pins = crate::core_route::PinSource::for_this_command()?;
    let opened = open(target, stores)?;
    crate::core_route::unlock_up_front(&opened.core, &pins, prompt)?;
    Ok(opened)
}

/// Say why a core call failed, in the notice table's words where it has a
/// row of its own, and the exit code a script branches on.
pub(crate) fn fail(e: &DspiError) -> u8 {
    // -- WP TMCOREF -- the lock's refusal: its banner, and the way this
    // Terminal takes the PIN
    if let DspiError::InstallerLock { code, .. } = e
        && (code == "locked" || code == "hub")
    {
        eprintln!(
            "{COMMAND}: {}",
            dspi_core::notice_for_key(dspi_core::LOCK_BANNER_NOTICE.into(), String::new()).reason
        );
        eprintln!(
            "      {}",
            crate::core_route::PinSource::action_for_this_command()
        );
        return error_exit(e);
    }
    // -- WP ROMODE -- a refusal of `--read-only`: the mode's row
    if let Some(n) = crate::core_route::client_read_only_notice(e) {
        eprintln!("{COMMAND}: {}", n.reason);
        eprintln!("      {}", n.action);
        return error_exit(e);
    }
    eprintln!("{COMMAND}: {e}");
    let notice = dspi_core::notice_for(e.clone());
    if explained(&notice.key) && !notice.action.is_empty() {
        eprintln!("      {}", notice.action);
    }
    error_exit(e)
}

// -- WP TMCOREF -- (review tmcore F3)
/// [`fail`], and with `--json` the refusal as `{ok: false, error, noticeKey}`
/// on standard output.
pub(crate) fn fail_json(e: &DspiError, json: bool) -> u8 {
    if json {
        let n = crate::core_route::client_read_only_notice(e) // -- WP ROMODE --
            .unwrap_or_else(|| dspi_core::notice_for(e.clone()));
        println!(
            "{}",
            json!({ "ok": false, "error": e.to_string(), "noticeKey": n.key })
        );
    }
    fail(e)
}

/// The notices whose action line adds to the error's own words.
pub(crate) fn explained(key: &str) -> bool {
    key.starts_with("installer_lock")
        || key.starts_with("limit")
        || matches!(key, "confirmation_required" | "image_refused")
}

/// The exit code for a core error: bad input, no device, a refusal, or a
/// transport failure.
pub(crate) fn error_exit(e: &DspiError) -> u8 {
    match e {
        DspiError::InvalidArgument { .. } | DspiError::Unsupported => exit::USAGE,
        DspiError::Disconnected => exit::NO_DEVICE,
        DspiError::ReadOnly
        | DspiError::Locked
        | DspiError::Denied
        | DspiError::Refused { .. }
        | DspiError::ConfirmationRequired { .. }
        | DspiError::LimitExceeded { .. }
        | DspiError::InstallerLock { .. }
        | DspiError::ImageRefused { .. }
        | DspiError::LimitsUnknown { .. }
        | DspiError::FirmwareGuardBlocked { .. }
        | DspiError::OvervoltWordRequired { .. } => exit::REJECTED,
        DspiError::Transfer { .. }
        | DspiError::Link { .. }
        | DspiError::Io { .. }
        | DspiError::AutoEq { .. } => exit::TRANSPORT,
    }
}

/// A reply of the core's JSON calls (`config_import`, `config_preview`,
/// `config_diff`, ...), or its refusal as an exit code.
pub(crate) fn reply(text: &str) -> Result<Value, u8> {
    let v: Value = serde_json::from_str(text).map_err(|e| {
        eprintln!("{COMMAND}: the core's reply could not be read: {e}");
        exit::TRANSPORT
    })?;
    if v.get("ok") == Some(&Value::Bool(true)) {
        return Ok(v);
    }
    let error = v.get("error").and_then(Value::as_str).unwrap_or("refused");
    eprintln!("{COMMAND}: {}", printable(error));
    let key = v.get("noticeKey").and_then(Value::as_str).unwrap_or("");
    Err(match key {
        "config.unreadable" | "config.nothing_selected" => exit::USAGE,
        "config.not_read" => exit::TRANSPORT,
        _ => exit::REJECTED,
    })
}

// ─── Arguments ──────────────────────────────────────────────────────────

/// A command's words after its name: the positional ones, the flags that
/// take a value, and the switches. A flag it does not know is an error,
/// never ignored: a misspelt one must not change what gets written.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Args<'a> {
    pub pos: Vec<&'a str>,
    values: Vec<(&'static str, &'a str)>,
    switches: Vec<&'static str>,
}

/// Switches every command here takes.
const COMMON: &[&str] = &["--json", "--quiet"];

impl<'a> Args<'a> {
    pub(crate) fn parse(
        args: &[&'a str],
        valued: &[&'static str],
        switches: &[&'static str],
    ) -> Result<Self, String> {
        let mut out = Self::default();
        let mut it = args.iter().copied();
        while let Some(arg) = it.next() {
            if !arg.starts_with("--") {
                out.pos.push(arg);
                continue;
            }
            let (flag, inline) = match arg.split_once('=') {
                Some((f, v)) => (f, Some(v)),
                None => (arg, None),
            };
            if let Some(name) = valued.iter().copied().find(|v| *v == flag) {
                let value = inline
                    .or_else(|| it.next())
                    .ok_or_else(|| format!("{name} needs a value"))?;
                out.values.push((name, value));
            } else if let Some(name) = switches.iter().chain(COMMON).copied().find(|s| *s == flag) {
                out.switches.push(name);
            } else {
                let mut known: Vec<&str> = valued.iter().chain(switches).copied().collect();
                known.extend_from_slice(COMMON);
                return Err(format!(
                    "`{flag}` is not an option here; this takes {}",
                    known.join(", ")
                ));
            }
        }
        Ok(out)
    }

    // -- WP P1EFIX --
    /// Whether the switch `name` was given.
    pub(crate) fn has(&self, name: &str) -> bool {
        self.switches.contains(&name)
    }

    pub(crate) fn value(&self, name: &str) -> Option<&'a str> {
        self.values
            .iter()
            .rev()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| *v)
    }
}

/// A usage error, said.
pub(crate) fn usage(e: &str) -> u8 {
    eprintln!("{COMMAND}: {e}");
    exit::USAGE
}

/// `--guard SECONDS`, held to 5..=300.
pub(crate) fn guard_seconds(value: &str) -> Result<u32, String> {
    let s: u32 = value
        .trim()
        .trim_end_matches('s')
        .parse()
        .map_err(|_| format!("--guard takes whole seconds, not `{value}`"))?;
    if !(GUARD_MIN_S..=GUARD_MAX_S).contains(&s) {
        return Err(format!(
            "--guard takes {GUARD_MIN_S} to {GUARD_MAX_S} seconds, not {s}"
        ));
    }
    Ok(s)
}

// -- WP P1EFIX -- (owner 2026-11-14) guarded by default over DSPi Link
/// The timeout a guarded path takes when `--guard` gives none: the core's
/// default, 30 s.
pub(crate) const GUARD_DEFAULT_S: u32 = 30;

/// Whether the command reaches its device through a DSPi Link hub
/// (`--link`), where a recall or an import is guarded by default.
pub(crate) fn over_dlink(target: &Target) -> bool {
    matches!(target, Target::Link(_))
}

/// The guard a recall or an import runs, in seconds (`None`: unguarded):
/// `--guard SECONDS` as given; `--no-guard` none; otherwise
/// [`GUARD_DEFAULT_S`] over DSPi Link and none locally (USB, MIDI, the
/// demo), as before. `--guard` with `--no-guard` is a usage error.
pub(crate) fn guard_choice(
    guard: Option<&str>,
    no_guard: bool,
    over_dlink: bool,
) -> Result<Option<u32>, String> {
    match (guard, no_guard) {
        (Some(_), true) => Err("--guard and --no-guard cannot both be given".into()),
        (Some(v), false) => guard_seconds(v).map(Some),
        (None, true) => Ok(None),
        (None, false) => Ok(over_dlink.then_some(GUARD_DEFAULT_S)),
    }
}

/// A file's text, or standard input's for `-`.
pub(crate) fn read_text(path: &str) -> Result<String, u8> {
    let read = if path == "-" {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s).map(|_| s)
    } else {
        std::fs::read_to_string(path)
    };
    read.map_err(|e| usage(&format!("cannot read {}: {e}", printable(path))))
}

/// Write `text` to `path` (standard output for `-`): beside it first, then
/// renamed over it, so a failed write leaves the old file.
pub(crate) fn write_text(path: &str, text: &str) -> Result<(), u8> {
    if path == "-" {
        print!("{text}");
        return Ok(());
    }
    let target = Path::new(path);
    let tmp = target.with_extension("uconsole-part");
    let written = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, target));
    written.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        eprintln!("{COMMAND}: cannot write {}: {e}", printable(path));
        exit::TRANSPORT
    })
}

// ─── Answers ────────────────────────────────────────────────────────────

/// Whether a person is at standard input.
pub(crate) fn interactive() -> bool {
    std::io::stdin().is_terminal()
}

/// Standard input's lines as they come, from a reader thread, so a
/// countdown can run while nothing is typed. The channel ends with the input.
pub(crate) fn stdin_answers() -> Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

/// One line from a person at the terminal, after `question` on standard
/// error; `None` without a terminal or at the end of its input.
pub(crate) fn ask(question: &str) -> Option<String> {
    if !interactive() {
        return None;
    }
    eprint!("{question}");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim().to_owned()),
    }
}

// -- WP P1EFIX -- (review p1e-b F3)
/// A secret line (the installer PIN) from a person at the terminal, after
/// `question` on standard error, with nothing echoed as it is typed:
/// crossterm's raw mode, put back on every path. Backspace takes back a
/// character; Enter ends it; Ctrl+C, Ctrl+D or Esc gives `None`, as does no
/// terminal or a terminal that refuses raw mode.
pub(crate) fn ask_secret(question: &str) -> Option<String> {
    if !interactive() {
        return None;
    }
    eprint!("{question}");
    let _ = std::io::stderr().flush();
    let line = read_secret();
    eprintln!();
    line
}

/// Raw mode on for as long as it lives.
struct RawMode;

impl RawMode {
    fn on() -> Option<Self> {
        crossterm::terminal::enable_raw_mode()
            .ok()
            .map(|()| RawMode)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

fn read_secret() -> Option<String> {
    use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, read};
    let _raw = RawMode::on()?;
    let mut keys = Vec::new();
    loop {
        let Ok(Event::Key(k)) = read() else { continue };
        if k.kind == KeyEventKind::Release {
            continue;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Enter => break,
            KeyCode::Char('c' | 'd') if ctrl => return None,
            KeyCode::Esc => return None,
            code => keys = secret_edit(keys, code),
        }
    }
    Some(keys.into_iter().collect::<String>().trim().to_owned())
}

/// One key's edit of the secret typed so far: a character joins it,
/// Backspace takes the last back, anything else changes nothing.
fn secret_edit(mut keys: Vec<char>, code: crossterm::event::KeyCode) -> Vec<char> {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Char(c) => keys.push(c),
        KeyCode::Backspace => {
            keys.pop();
        }
        _ => {}
    }
    keys
}

/// A yes to `question`, or `--yes`.
pub(crate) fn confirmed(yes: bool, question: &str) -> Result<(), u8> {
    if yes {
        return Ok(());
    }
    match ask(&format!("{question} [y/N] ")) {
        Some(a) if matches!(a.to_ascii_lowercase().as_str(), "y" | "yes") => Ok(()),
        Some(_) => {
            eprintln!("Not confirmed. Nothing was changed.");
            Err(exit::REJECTED)
        }
        None => Err(usage(
            "this needs a confirmation: on a terminal it asks; in a script, add --yes",
        )),
    }
}

// ─── The guard ──────────────────────────────────────────────────────────

/// The countdown's sentence: the `guard.waiting` notice with the seconds.
fn countdown(remaining_ms: u32) -> String {
    let n = dspi_core::notice_for_key(dspi_core::NOTICE_GUARD_WAITING.into(), String::new());
    let s = remaining_ms.div_ceil(1000);
    format!("{} {}", n.title, n.reason.replace("{s}", &s.to_string()))
}

/// Wait for Keep or Revert while the core times the guard: `keep` (or `k`,
/// `y`) and `revert` (`r`, `n`) from `answers`, one per line. Without an
/// answer (the input ended, or nobody typed) the guard reverts at its
/// timeout. Returns how it ended.
pub(crate) fn wait_guard(
    core: &DspiCore,
    started: GuardedApply,
    answers: &Receiver<String>,
    show: bool,
) -> GuardedApply {
    let mut g = started;
    let mut answers_open = true;
    let mut shown = None;
    while g.phase == GuardPhase::Waiting {
        if show {
            let s = g.remaining_ms.div_ceil(1000);
            if shown != Some(s) {
                shown = Some(s);
                eprint!("\r{} [k]eep, [r]evert: \x1b[K", countdown(g.remaining_ms));
                let _ = std::io::stderr().flush();
            }
        }
        let answer = if answers_open {
            match answers.recv_timeout(GUARD_POLL) {
                Ok(line) => Some(line),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => {
                    answers_open = false;
                    None
                }
            }
        } else {
            std::thread::sleep(GUARD_POLL);
            None
        };
        let acted = match answer.as_deref().map(|a| a.trim().to_ascii_lowercase()) {
            Some(a) if matches!(a.as_str(), "k" | "keep" | "y" | "yes") => {
                Some(core.apply_confirm())
            }
            Some(a) if matches!(a.as_str(), "r" | "revert" | "n" | "no") => {
                Some(core.apply_revert())
            }
            Some(other) => {
                if !other.is_empty() {
                    eprintln!("\nType keep or revert.");
                    shown = None;
                }
                None
            }
            None => None,
        };
        g = match acted {
            Some(Ok(state)) => state,
            Some(Err(_)) => core.apply_status(),
            None => core.apply_poll(),
        };
    }
    if show {
        eprintln!();
    }
    g
}

// -- WP TMCORE --
/// The `guard.hub_backstop` row's sentence: the hub puts the settings back
/// too if this console goes away.
pub(crate) fn backstop_line() -> String {
    dspi_core::notice_for_key(dspi_core::NOTICE_GUARD_HUB_BACKSTOP.into(), String::new()).reason
}

/// Report how a guard ended, and its exit code: kept is success, reverted
/// is a refusal, the rest a transport failure.
pub(crate) fn report_guard(g: &GuardedApply, json: bool) -> u8 {
    let code = match g.phase {
        GuardPhase::Kept => exit::OK,
        GuardPhase::Reverted => exit::REJECTED,
        _ => exit::TRANSPORT,
    };
    if json {
        println!("{}", serde_json::to_string(g).unwrap_or_default());
        return code;
    }
    let word = match g.phase {
        GuardPhase::Kept => "Kept",
        GuardPhase::Reverted => "Reverted",
        GuardPhase::RevertFailed => "Could not put the settings back",
        GuardPhase::NotWritten => "Not written",
        GuardPhase::Lost => "Connection lost",
        GuardPhase::Waiting | GuardPhase::Idle => "Ended",
    };
    let reason = match g.reason.as_str() {
        "" | "confirmed" | "reverted" => String::new(),
        r => format!(" ({r})"),
    };
    println!("{word}{reason}: {}", printable(&g.label));
    if !g.message.is_empty() && g.message != "Kept." {
        println!("{}", printable(&g.message));
    }
    for line in g.changes.iter().chain(&g.notes) {
        println!("  {}", printable(line));
    }
    code
}

/// Run a guarded apply's countdown on standard input and report it.
pub(crate) fn guard_on_stdin(core: &DspiCore, started: GuardedApply, json: bool) -> u8 {
    let show = interactive() && !json;
    if !json {
        for line in started.changes.iter().chain(&started.notes) {
            println!("  {}", printable(line));
        }
        if !show {
            eprintln!(
                "{COMMAND}: applied with a way back; answer keep or revert on standard input \
                 within {} s, or it goes back",
                started.remaining_ms.div_ceil(1000)
            );
        }
        // -- WP TMCORE -- the hub guards it too (P1HA): the core's row says so
        if started.hub {
            eprintln!("{COMMAND}: {}", backstop_line());
        }
    }
    let answers = stdin_answers();
    let ended = wait_guard(core, started, &answers, show);
    report_guard(&ended, json)
}

// ─── Diffs ──────────────────────────────────────────────────────────────

/// Print a diff reply (`config_diff`, `config_preview`, `ab_diff`): one line
/// per changed value, by block, then the summary.
pub(crate) fn print_diff(v: &Value, json: bool) {
    if json {
        println!("{v}");
        return;
    }
    let rows = v.get("rows").and_then(Value::as_array);
    let count = rows.map_or(0, Vec::len);
    if count == 0 {
        println!("No differences.");
    }
    let mut block = "";
    for row in rows.into_iter().flatten() {
        let field = |k: &str| row.get(k).and_then(Value::as_str).unwrap_or("");
        if field("block") != block {
            block = field("block");
            println!("[{block}]");
        }
        let item = match (field("section"), field("item")) {
            (s, "") => s.to_owned(),
            ("", i) => i.to_owned(),
            (s, i) => format!("{s} {i}"),
        };
        println!(
            "  {}: {} -> {}",
            printable(&item),
            printable(field("before")),
            printable(field("after"))
        );
    }
    for note in v
        .get("notes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        println!("Note: {}", printable(note));
    }
    if count > 0 {
        println!("{count} change{}", if count == 1 { "" } else { "s" });
    }
}

/// The number of rows in a diff reply.
pub(crate) fn diff_count(v: &Value) -> usize {
    v.get("rows").and_then(Value::as_array).map_or(0, Vec::len)
}

/// `uconsole diff A [B]`: two documents, or one against the device.
pub(crate) fn cmd_diff(target: &Target, flags: &[&str], json: bool) -> u8 {
    let args = match Args::parse(flags.get(1..).unwrap_or_default(), &[], &[]) {
        Ok(a) => a,
        Err(e) => return usage(&e),
    };
    let (a, b) = match args.pos.as_slice() {
        [a] => (*a, None),
        [a, b] => (*a, Some(*b)),
        _ => {
            return usage(
                "diff takes one document (against the device) or two \
                 (`diff before.dspipreset after.dspipreset`)",
            );
        }
    };
    let first = match read_text(a) {
        Ok(t) => t,
        Err(c) => return c,
    };
    let v = match b {
        Some(b) => {
            let second = match read_text(b) {
                Ok(t) => t,
                Err(c) => return c,
            };
            dspi_core::files::config_diff(first, second)
        }
        None => {
            let opened = match open(target, &Stores::default_dirs()) {
                Ok(o) => o,
                Err(c) => return c,
            };
            preview_all(&opened.core, first)
        }
    };
    match reply(&v) {
        Ok(v) => {
            print_diff(&v, json);
            exit::OK
        }
        Err(c) => c,
    }
}

/// What importing every block of `text` would change on the device.
pub(crate) fn preview_all(core: &DspiCore, text: String) -> String {
    use dspi_core::consts::{IMPORT_ALL_BLOCKS, IMPORT_BY_BLOCK};
    core.config_preview(text, IMPORT_BY_BLOCK | IMPORT_ALL_BLOCKS, u32::MAX)
}

// ─── Import by block, guarded ───────────────────────────────────────────

/// The import blocks `--only` names (P1DF's), as their `IMPORT_*` bits.
pub(crate) fn blocks(list: &str) -> Result<u32, String> {
    use dspi_core::consts::*;
    let mut bits = 0;
    for word in list.split(',').map(str::trim).filter(|w| !w.is_empty()) {
        bits |= match word.to_ascii_lowercase().as_str() {
            "eq" | "peq" => IMPORT_EQ,
            "crossover" | "xo" => IMPORT_CROSSOVER,
            "routing" | "matrix" => IMPORT_ROUTING,
            "outputs" => IMPORT_OUTPUTS,
            "limiters" => IMPORT_LIMITERS,
            "effects" => IMPORT_EFFECTS,
            "volumes" | "volume" => IMPORT_VOLUMES,
            "hardware" => IMPORT_HARDWARE,
            "names" => IMPORT_NAMES,
            "all" => IMPORT_ALL_BLOCKS,
            other => {
                return Err(format!(
                    "`{other}` is not a block; --only takes eq, crossover, routing, outputs, \
                     limiters, effects, volumes, hardware, names or all, separated by commas"
                ));
            }
        };
    }
    if bits == 0 {
        return Err("--only names no block".into());
    }
    Ok(bits)
}

/// How a `.dspipreset` import runs on the core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CoreImport {
    /// `IMPORT_*` block bits.
    pub blocks: u32,
    /// Seconds to wait for Keep before going back; `None` is unguarded.
    pub guard: Option<u32>,
    /// Show what would change, write nothing.
    pub preview: bool,
    /// Confirmed already (`--yes`).
    pub yes: bool,
}

/// The blocks an import takes when `--only` is not given: the audio
/// processing always, volumes and hardware on request (the old switches).
pub(crate) fn default_blocks(volume: bool, hardware: bool) -> u32 {
    use dspi_core::consts::*;
    let mut bits = IMPORT_ALL_BLOCKS & !(IMPORT_VOLUMES | IMPORT_HARDWARE);
    if volume {
        bits |= IMPORT_VOLUMES;
    }
    if hardware {
        bits |= IMPORT_HARDWARE;
    }
    bits
}

/// Import a whole-device document through the core: by block, with a
/// preview, a confirmation, and a way back when guarded.
pub(crate) fn import(target: &Target, text: String, plan: &CoreImport, json: bool) -> u8 {
    // -- WP TMCOREF -- a write: the installer's PIN when locked
    let opened = match open_writing(target, &Stores::default_dirs(), !plan.preview) {
        Ok(o) => o,
        Err(c) => return c,
    };
    if plan.guard.is_some() {
        return import_on(&opened.core, text, plan, json, guard_on_stdin);
    }
    import_on(&opened.core, text, plan, json, |_, _, _| exit::OK)
}

/// [`import`] on an open core; `guarded` runs a guard's countdown.
pub(crate) fn import_on(
    core: &DspiCore,
    text: String,
    plan: &CoreImport,
    json: bool,
    guarded: impl FnOnce(&DspiCore, GuardedApply, bool) -> u8,
) -> u8 {
    use dspi_core::consts::IMPORT_BY_BLOCK;
    let options = IMPORT_BY_BLOCK | plan.blocks;
    let preview = match reply(&core.config_preview(text.clone(), options, u32::MAX)) {
        Ok(v) => v,
        Err(c) => return c,
    };
    if plan.preview {
        print_diff(&preview, json);
        return exit::OK;
    }
    if diff_count(&preview) == 0 {
        if json {
            println!("{}", json!({ "ok": true, "changes": 0 }));
        } else {
            println!("No differences: nothing to apply.");
        }
        return exit::OK;
    }
    if !json {
        print_diff(&preview, false);
    }
    if let Some(secs) = plan.guard {
        // A guarded apply has its own way back: no token (P1TK).
        return match core.apply_guarded_import(text, options, u32::MAX, secs) {
            Ok(g) => guarded(core, g, json),
            Err(e) => fail_json(&e, json), // -- WP TMCOREF --
        };
    }
    if let Err(c) = confirmed(plan.yes, "Apply these changes to the device?") {
        return c;
    }
    let token = core.confirm_token(dspi_core::ConfirmOp::ConfigImport);
    let answer = core.config_import_confirmed(text, options, u32::MAX, Some(token));
    let done = match reply(&answer) {
        Ok(v) => v,
        Err(c) => {
            // -- WP TMCOREF -- the core's refusal, `ok: false`, on stdout too
            if json {
                println!("{answer}");
            }
            return c;
        }
    };
    if json {
        println!("{done}");
    } else {
        for line in done
            .get("lines")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            println!("{}", printable(line));
        }
        println!("Applied.");
    }
    exit::OK
}

// ─── The session path's limits ──────────────────────────────────────────

/// The limits profile this console keeps for the session's device (the
/// folder the core uses, P1LM), for the paths that still write through
/// `dspi-session` (a plain `import`, held by `preset_file::apply_limited`).
/// A profile that does not read says so; its last good copy is used.
pub(crate) fn console_limits(
    session: &mut dspi_session::Session,
) -> Option<dspi_session::limits::LimitsProfile> {
    let dir = Stores::default_dirs().limits?;
    let serial = session
        .with_transport(|t| Ok(t.descriptor().serial.clone()))
        .ok()?;
    let loaded = dspi_session::limits::LimitsStore::new(dir).load(&serial);
    let key = match loaded.state {
        dspi_session::limits::LoadState::FellBack { .. } => Some("limits.fell_back"),
        dspi_session::limits::LoadState::Unreadable { .. } => Some("limits.unreadable"),
        _ => None,
    };
    if let Some(key) = key {
        let n = dspi_core::notice_for_key(key.into(), String::new());
        eprintln!("{COMMAND}: {}: {}", n.title, n.reason);
    }
    loaded.profile
}

// ─── Status ─────────────────────────────────────────────────────────────

/// `uconsole status`: the connection, the device, the limits, the lock and
/// the audio link, in one view.
pub(crate) fn cmd_status(target: &Target, flags: &[&str], json: bool) -> u8 {
    if let Err(e) = Args::parse(flags.get(1..).unwrap_or_default(), &[], &[]) {
        return usage(&e);
    }
    let opened = match open(target, &Stores::default_dirs()) {
        Ok(o) => o,
        Err(c) => return c,
    };
    let mut v = status_of(&opened.core, &opened.serial, &target.via());
    // -- WP SECTM -- a hub session's encryption, and the badge without it
    if matches!(target, Target::Link(_)) {
        link_security_into(&mut v, &opened.core.link_security());
    }
    print_status(&v, json);
    exit::OK
}

/// The status view's facts, as JSON.
pub(crate) fn status_of(core: &DspiCore, serial: &str, via: &str) -> Value {
    let s = core.get_state();
    let platform = dspi_proto::Platform::from_id(s.platform_id).name();
    let firmware = format!("{}.{}.{}", s.fw_major, s.fw_minor, s.fw_patch);
    let limits = core.limits_status();
    let lock = core.installer_lock_state();
    let alink = match core.alink_info() {
        Ok(Some(info)) => {
            let st = core.fetch_alink_status().ok();
            json!({
                "role": info.role,
                "state": st.as_ref().map(|s| s.state.clone()),
                "senderId": st.as_ref().map(|s| s.sender_id),
                "hops": st.as_ref().map(|s| s.hops),
                "channels": st.as_ref().map(|s| s.channels),
                "frame": st.as_ref().map(|s| s.frame),
                "locks": st.as_ref().map(|s| s.lock_count),
                "losses": st.as_ref().map(|s| s.loss_count),
            })
        }
        _ => Value::Null,
    };
    let document: Value = serde_json::from_str(&core.offline_document()).unwrap_or(Value::Null);
    json!({
        "via": via,
        "serial": serial,
        "document": document,
        "platform": platform,
        "firmware": firmware,
        "inputs": s.num_input_channels,
        "outputs": s.num_output_channels,
        "limits": limits,
        "lock": lock,
        "alink": alink,
    })
}

// -- WP SECTM --
/// A hub session's `link` facts (`encrypted`, `hubKey`) into the status view,
/// and " (unsecured)" after its `via` when it has no TLS.
pub(crate) fn link_security_into(v: &mut Value, security: &dspi_core::LinkSecurity) {
    if let Some(via) = v.get("via").and_then(Value::as_str) {
        let via = dspi_tui::shell::connect::with_security(via, Some(security.encrypted));
        v["via"] = Value::String(via);
    }
    v["link"] = json!({
        "encrypted": security.encrypted,
        "hubKey": security.hub_key,
    });
}

fn print_status(v: &Value, json: bool) {
    if json {
        println!("{v}");
        return;
    }
    let s = |p: &str| {
        v.pointer(p)
            .map(|x| match x {
                Value::String(s) => printable(s),
                other => other.to_string(),
            })
            .unwrap_or_default()
    };
    let flag = |p: &str| v.pointer(p).and_then(Value::as_bool).unwrap_or(false);
    // -- WP SECTM -- the badge red on a terminal, and where the explanation is
    {
        let colour = dspi_tui::shell::connect::colour_allowed(std::io::stdout().is_terminal());
        println!(
            "Connection  {}",
            dspi_tui::shell::connect::paint_unsecured(&s("/via"), colour)
        );
        if v.pointer("/link/encrypted") == Some(&Value::Bool(false)) {
            println!("            not encrypted: see `{COMMAND} link security`");
        } else if !s("/link/hubKey").is_empty() {
            println!("Hub key     {}", s("/link/hubKey"));
        }
    }
    if flag("/document/offline") {
        println!("File        {}", s("/document/fileName"));
    } else {
        println!("Serial      {}", s("/serial"));
    }
    println!(
        "Device      {}, firmware {}, {} inputs, {} outputs",
        s("/platform"),
        s("/firmware"),
        s("/inputs"),
        s("/outputs")
    );
    let limits = if flag("/limits/active") {
        format!("in force ({})", s("/limits/state"))
    } else {
        "none".into()
    };
    println!("Limits      {limits}");
    let lock = if !flag("/lock/pinSet") {
        "no installer PIN set".to_string()
    } else if flag("/lock/locked") {
        format!("locked ({})", lock_holder(&s("/lock/heldBy")))
    } else {
        format!("PIN set, not locked ({})", lock_holder(&s("/lock/heldBy")))
    };
    println!("Lock        {lock}");
    if v.get("alink").is_some_and(|a| !a.is_null()) {
        println!(
            "Audio link  {}: {}, sender {}, {} hops, {} channels, locked {} times, lost {}",
            s("/alink/role"),
            s("/alink/state"),
            s("/alink/senderId"),
            s("/alink/hops"),
            s("/alink/channels"),
            s("/alink/locks"),
            s("/alink/losses")
        );
    } else {
        println!("Audio link  none");
    }
}

/// Where a lock is held, in words: on the hub, or on this console.
pub(crate) fn lock_holder(held_by: &str) -> &'static str {
    if held_by == "hub" {
        "on the hub"
    } else {
        "on this console"
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
pub(crate) mod tests {
    use super::*;

    /// A folder for one test's stores, beside the test binary (never tmpfs).
    pub(crate) fn scratch(name: &str) -> PathBuf {
        let dir = std::env::current_exe()
            .unwrap()
            .with_file_name(format!("wp_p1tm_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A prepared core on the demo device with stores in `scratch(name)`.
    pub(crate) fn demo(name: &str) -> (Arc<DspiCore>, PathBuf) {
        let dir = scratch(name);
        let core = prepare(&Stores::under(&dir)).unwrap();
        core.open_demo().unwrap();
        (core, dir)
    }

    #[test]
    fn wp_p1tm_options_are_parsed_and_unknown_ones_refused() {
        let a = Args::parse(
            &["x", "--guard", "30", "--yes", "--only=eq"],
            &["--guard", "--only"],
            &["--yes"],
        )
        .unwrap();
        assert_eq!(a.pos, ["x"]);
        assert_eq!(a.value("--guard"), Some("30"));
        assert_eq!(a.value("--only"), Some("eq"));
        assert_eq!(a.switches, ["--yes"]);
        assert!(Args::parse(&["--gaurd", "3"], &["--guard"], &[]).is_err());
        assert!(Args::parse(&["--guard"], &["--guard"], &[]).is_err());
    }

    #[test]
    fn wp_p1efix_guarded_by_default_over_dspi_link_only() {
        // Over DSPi Link: guarded by default, --no-guard opts out, --guard sets the time.
        assert_eq!(guard_choice(None, false, true), Ok(Some(GUARD_DEFAULT_S)));
        assert_eq!(guard_choice(None, true, true), Ok(None));
        assert_eq!(guard_choice(Some("60"), false, true), Ok(Some(60)));
        // Locally (USB, MIDI, the demo): unguarded unless asked, as before.
        assert_eq!(guard_choice(None, false, false), Ok(None));
        assert_eq!(guard_choice(None, true, false), Ok(None));
        assert_eq!(guard_choice(Some("10"), false, false), Ok(Some(10)));
        // Both at once, or a bad time: refused.
        assert!(guard_choice(Some("30"), true, true).is_err());
        assert!(guard_choice(Some("2"), false, true).is_err());
        let link = Target::Link(dspi_tui::shell::connect::LinkTarget::parse("127.0.0.1").unwrap());
        assert!(over_dlink(&link));
        assert!(!over_dlink(&Target::Demo));
        assert!(!over_dlink(&Target::Usb { serial: None }));
        let a = Args::parse(
            &["recall", "1", "--no-guard"],
            &["--guard"],
            &["--no-guard"],
        )
        .unwrap();
        assert!(a.has("--no-guard") && !a.has("--guard"));
    }

    #[test]
    fn wp_p1efix_a_secret_line_takes_back_with_backspace() {
        use crossterm::event::KeyCode;
        let keys = [
            KeyCode::Char('1'),
            KeyCode::Char('2'),
            KeyCode::Backspace,
            KeyCode::Char('4'),
            KeyCode::Left,
        ]
        .into_iter()
        .fold(Vec::new(), secret_edit);
        assert_eq!(keys.into_iter().collect::<String>(), "14");
        // No terminal in a test: nothing is asked, nothing read.
        assert_eq!(ask_secret("PIN: "), None);
    }

    #[test]
    fn wp_p1tm_guard_and_blocks_are_checked() {
        use dspi_core::consts::*;
        assert_eq!(guard_seconds("30"), Ok(30));
        assert_eq!(guard_seconds("5s"), Ok(5));
        assert!(guard_seconds("4").is_err() && guard_seconds("301").is_err());
        assert!(guard_seconds("soon").is_err());
        assert_eq!(blocks("eq, routing"), Ok(IMPORT_EQ | IMPORT_ROUTING));
        assert_eq!(blocks("all"), Ok(IMPORT_ALL_BLOCKS));
        assert!(blocks("eq,bass").is_err() && blocks(",").is_err());
        let d = default_blocks(false, false);
        assert_eq!(d & (IMPORT_VOLUMES | IMPORT_HARDWARE), 0);
        assert_eq!(default_blocks(true, true), IMPORT_ALL_BLOCKS);
    }

    #[test]
    fn wp_p1tm_errors_map_to_exit_codes() {
        assert_eq!(
            error_exit(&DspiError::InvalidArgument { msg: "x".into() }),
            exit::USAGE
        );
        assert_eq!(error_exit(&DspiError::Disconnected), exit::NO_DEVICE);
        assert_eq!(
            error_exit(&DspiError::InstallerLock {
                code: "locked".into(),
                msg: "x".into()
            }),
            exit::REJECTED
        );
        assert_eq!(
            error_exit(&DspiError::Io { msg: "x".into() }),
            exit::TRANSPORT
        );
    }

    /// Output 1's gain on the device.
    pub(crate) fn gain(core: &DspiCore) -> f32 {
        core.get_state().output_gain_db[0]
    }

    /// The device's document as it is now; then output 1's gain moved to
    /// `db` on the device, so importing the document puts it back. Returns
    /// the document and the gain it holds.
    pub(crate) fn exported_then_moved(core: &DspiCore, db: f32) -> (String, f32) {
        let text = core.config_export("t".into(), "1".into(), 0).unwrap();
        let was = gain(core);
        core.set_output_gain(0, db).unwrap();
        assert!((gain(core) - db).abs() < 0.05);
        assert!((was - db).abs() > 0.5, "pick a gain the demo is not at");
        (text, was)
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.05
    }

    #[test]
    fn wp_p1tm_a_by_block_import_previews_then_applies_with_a_token() {
        let (core, _dir) = demo("import");
        let (text, was) = exported_then_moved(&core, -7.0);
        let plan = CoreImport {
            blocks: blocks("outputs").unwrap(),
            guard: None,
            preview: true,
            yes: false,
        };
        // A preview writes nothing.
        assert_eq!(
            import_on(&core, text.clone(), &plan, true, |_, _, _| 9),
            exit::OK
        );
        assert!(near(gain(&core), -7.0));
        // Not confirmed, and nobody to ask: refused, nothing written.
        let plan = CoreImport {
            preview: false,
            ..plan
        };
        assert_eq!(
            import_on(&core, text.clone(), &plan, true, |_, _, _| 9),
            exit::USAGE
        );
        assert!(near(gain(&core), -7.0));
        // --yes mints the token the core requires (always, P1TZ).
        let plan = CoreImport { yes: true, ..plan };
        assert_eq!(import_on(&core, text, &plan, true, |_, _, _| 9), exit::OK);
        assert!(near(gain(&core), was), "{}", gain(&core));
    }

    #[test]
    fn wp_p1tm_a_block_left_out_keeps_the_devices_values() {
        let (core, _dir) = demo("blocks");
        let (text, _) = exported_then_moved(&core, -9.0);
        let plan = CoreImport {
            blocks: blocks("eq,routing").unwrap(),
            guard: None,
            preview: false,
            yes: true,
        };
        // The output gain is not in the blocks chosen.
        assert_eq!(import_on(&core, text, &plan, true, |_, _, _| 9), exit::OK);
        assert!(near(gain(&core), -9.0));
    }

    #[test]
    fn wp_p1tm_a_guarded_import_kept_stays_and_reverted_goes_back() {
        let (core, _dir) = demo("guard");
        let plan = CoreImport {
            blocks: blocks("outputs").unwrap(),
            guard: Some(30),
            preview: false,
            yes: false,
        };
        for (answer, phase, code) in [
            ("keep", GuardPhase::Kept, exit::OK),
            ("revert", GuardPhase::Reverted, exit::REJECTED),
        ] {
            let (text, was) = exported_then_moved(&core, -11.0);
            let (tx, rx) = std::sync::mpsc::channel();
            tx.send(answer.to_string()).unwrap();
            let got = import_on(&core, text, &plan, true, |c, g, json| {
                assert_eq!(g.phase, GuardPhase::Waiting);
                assert!(near(gain(c), was), "applied while waiting");
                let ended = wait_guard(c, g, &rx, false);
                assert_eq!(ended.phase, phase);
                report_guard(&ended, json)
            });
            assert_eq!(got, code, "{answer}");
            let expect = if phase == GuardPhase::Kept {
                was
            } else {
                -11.0
            };
            assert!(near(gain(&core), expect), "{answer}: {}", gain(&core));
        }
    }

    #[test]
    fn wp_p1tm_a_guard_with_no_answer_reverts_at_its_timeout() {
        let (core, _dir) = demo("timeout");
        let (text, was) = exported_then_moved(&core, -13.0);
        let options = dspi_core::consts::IMPORT_BY_BLOCK | blocks("outputs").unwrap();
        // The core takes a shorter timeout than the command's 5 s floor.
        let g = core
            .apply_guarded_import(text, options, u32::MAX, 1)
            .unwrap();
        assert!(near(gain(&core), was));
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        drop(tx); // the input ended: nobody will answer
        let ended = wait_guard(&core, g, &rx, false);
        assert_eq!(ended.phase, GuardPhase::Reverted);
        assert!(near(gain(&core), -13.0));
        assert_eq!(report_guard(&ended, true), exit::REJECTED);
    }

    #[test]
    fn wp_p1tm_two_documents_diff_without_a_device() {
        let (core, _dir) = demo("diff");
        let (a, _) = exported_then_moved(&core, -5.0);
        let b = core.config_export("t".into(), "1".into(), 0).unwrap();
        let v = reply(&dspi_core::files::config_diff(a.clone(), b)).unwrap();
        assert!(diff_count(&v) >= 1, "{v}");
        let same = reply(&dspi_core::files::config_diff(a.clone(), a.clone())).unwrap();
        assert_eq!(diff_count(&same), 0);
        // One document against the device: what importing it would change.
        let v = reply(&preview_all(&core, a)).unwrap();
        assert!(diff_count(&v) >= 1, "{v}");
        print_diff(&v, false);
    }

    #[test]
    fn wp_p1tm_status_names_the_device_limits_and_lock() {
        let (core, _dir) = demo("status");
        let v = status_of(&core, "DEMO", "demo");
        assert_eq!(v["via"], "demo");
        assert_eq!(v["lock"]["pinSet"], false);
        assert_eq!(v["limits"]["active"], false);
        assert!(v["inputs"].as_u64().unwrap() > 0);
        print_status(&v, false);
        // An audio-link unit says how its link stands.
        let core = prepare(&Stores::default()).unwrap();
        core.open_demo_alink().unwrap();
        let v = status_of(&core, "ALINK", "demo");
        assert!(v["alink"]["role"].is_string(), "{v}");
        print_status(&v, false);
    }
}
