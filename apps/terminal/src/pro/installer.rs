//! `uconsole limits` and `uconsole lock` (WP P1TM): the core's limits
//! profile (P1LM) and the installer's PIN and lock (P1LK).
//!
//! With no installer PIN set nothing asks for one and nothing is locked. With
//! one set, every change here (a profile set or cleared, a new PIN, the PIN
//! cleared) needs this console unlocked; one command is one session of the
//! core, so it asks for the PIN itself: on a terminal it prompts; in a
//! script it takes `DSPI_INSTALLER_PIN`, or `--installer-pin -` with the PIN
//! as a line of standard input. The recovery code works wherever the PIN
//! does. The lock is on this console, not on the DSPi (OD-P1d).
//!
//! -- WP HUBADMIN -- Over DSPi Link (`--link`) to a hub that keeps limits, the
//! hub's copy is the one: `limits get|set|clear` and `lock set-pin|clear-pin`
//! change the HUB's profile and PIN (Admin of the hub only; an Admin needs no
//! installer PIN), and `mode pin ROLE MODE` pins a role's mode on the hub.
//! Below Admin they show it and refuse a change, in the notice's words. The
//! console's own copy is changed only for a directly connected DSPi.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_cmd::COMMAND;
use dspi_core::hub_admin::{COPY_HUB, NOTICE_HUB_RECOVERY};
use dspi_core::{DspiCore, HubAdminState, InstallerTarget, LockState};
use dspi_tui::shell::connect::{Target, printable};
use serde_json::{Value, json};

use super::{Args, Stores, ask_secret, fail, interactive, lock_holder, open, read_text, usage};
use crate::exit;

/// The environment variable a script gives the installer's PIN in.
pub(crate) const PIN_ENV: &str = "DSPI_INSTALLER_PIN";
/// ... and a new one, for `lock set-pin`.
pub(crate) const NEW_PIN_ENV: &str = "DSPI_NEW_INSTALLER_PIN";

/// Where the PINs come from.
pub(crate) struct Pins<'a> {
    /// The installer's PIN or recovery code, when one is needed.
    pub current: Box<dyn FnMut() -> Option<String> + 'a>,
    /// A new PIN for `set-pin`; `Err` when the two typed differ.
    pub new: Box<dyn FnMut() -> Result<Option<String>, String> + 'a>,
}

fn usable(p: String) -> Option<String> {
    let p = p.trim_end_matches(['\r', '\n']).to_owned();
    (!p.trim().is_empty()).then_some(p)
}

fn stdin_line() -> Option<String> {
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => usable(line),
    }
}

/// `--installer-pin` and `--new-pin` take only `-`: a PIN on the command line
/// shows in `ps` and in the shell's history.
fn check_pin_flags(args: &Args<'_>) -> Result<(), u8> {
    for flag in ["--installer-pin", "--new-pin"] {
        if args.value(flag).is_some_and(|v| v != "-") {
            return Err(usage(&format!(
                "{flag} takes only - (the PIN as a line of standard input): on the command \
                 line it would show in `ps` and the shell's history; or set {}",
                if flag == "--new-pin" {
                    NEW_PIN_ENV
                } else {
                    PIN_ENV
                }
            )));
        }
    }
    Ok(())
}

impl Pins<'static> {
    /// The command line's sources: `-` (a line of standard input), the
    /// environment, then a person at the terminal.
    fn from_args(args: &Args<'_>, serial: &str) -> Self {
        let current_stdin = args.value("--installer-pin") == Some("-");
        let new_stdin = args.value("--new-pin") == Some("-");
        let serial = printable(serial);
        Pins {
            current: Box::new(move || {
                if current_stdin {
                    return stdin_line();
                }
                if let Some(p) = std::env::var(PIN_ENV).ok().and_then(usable) {
                    return Some(p);
                }
                ask_secret(&format!(
                    "Installer PIN (or recovery code) for {serial} on this console: "
                ))
                .and_then(usable)
            }),
            new: Box::new(move || {
                if new_stdin {
                    return Ok(stdin_line());
                }
                if let Some(p) = std::env::var(NEW_PIN_ENV).ok().and_then(usable) {
                    return Ok(Some(p));
                }
                if !interactive() {
                    return Ok(None);
                }
                let first = ask_secret("New installer PIN (4 to 64 characters): ").and_then(usable);
                let Some(first) = first else { return Ok(None) };
                match ask_secret("The same again: ").and_then(usable) {
                    Some(again) if again == first => Ok(Some(first)),
                    _ => Err("the two PINs differ; nothing was changed".into()),
                }
            }),
        }
    }
}

/// A notice's title and reason as one line, its action on the next.
fn notice(key: &str) -> String {
    let n = dspi_core::notice_for_key(key.into(), String::new());
    if n.action.is_empty() {
        format!("{}: {}", n.title, n.reason)
    } else {
        format!("{}: {}\n  {}", n.title, n.reason, n.action)
    }
}

/// The lock's scope, in its notice's words (this console's, or the hub's).
fn scope(st: &LockState) -> String {
    match st.scope_notice_key.as_str() {
        "" => notice("installer_lock.scope"),
        key => notice(key),
    }
}

/// Unlock this console for the open device when a PIN is set and it is not
/// unlocked: the PIN (or the recovery code) from `pins`. Nothing is asked
/// with no PIN set.
pub(crate) fn ensure_unlocked(core: &DspiCore, pins: &mut Pins<'_>) -> Result<LockState, u8> {
    let st = core.installer_lock_state();
    if !st.pin_set || st.unlocked {
        return Ok(st);
    }
    if st.retry_after_ms > 0 {
        eprintln!("{COMMAND}: {}", notice("installer_lock.wait"));
        eprintln!("      Try again in {} s.", st.retry_after_ms.div_ceil(1000));
        return Err(exit::REJECTED);
    }
    let Some(pin) = (pins.current)() else {
        return Err(usage(&format!(
            "an installer PIN is set for this DSPi on this console, so this needs it: on a \
             terminal it asks; in a script, set {PIN_ENV} or pass --installer-pin - with the \
             PIN on standard input"
        )));
    };
    match core.installer_unlock(pin) {
        Ok(st) if st.unlocked => {
            if st.recovered {
                eprintln!(
                    "{COMMAND}: unlocked with the recovery code; set a new PIN with \
                     `{COMMAND} lock set-pin`"
                );
            }
            Ok(st)
        }
        Ok(_) => {
            eprintln!("{COMMAND}: {}", notice("installer_lock.wrong_pin"));
            Err(exit::REJECTED)
        }
        Err(e) => Err(fail(&e)),
    }
}

// ─── limits ─────────────────────────────────────────────────────────────

const LIMITS_USAGE: &str = "limits takes show (or get), set FILE or clear";

/// `uconsole limits {show,set FILE,clear}`.
pub(crate) fn cmd_limits(target: &Target, flags: &[&str], json: bool) -> u8 {
    let args = match Args::parse(
        flags.get(1..).unwrap_or_default(),
        &["--installer-pin"],
        &[],
    ) {
        Ok(a) => a,
        Err(e) => return usage(&e),
    };
    let profile = match args.pos.as_slice() {
        ["show" | "get"] | [] | ["clear"] => None,
        ["set", file] => match read_text(file) {
            Ok(t) => Some(t),
            Err(c) => return c,
        },
        _ => return usage(LIMITS_USAGE),
    };
    if let Err(c) = check_pin_flags(&args) {
        return c;
    }
    let opened = match open(target, &Stores::default_dirs()) {
        Ok(o) => o,
        Err(c) => return c,
    };
    let mut pins = Pins::from_args(&args, &opened.serial);
    // -- WP HUBADMIN -- a hub's device: the hub's copy
    if let Some(t) = hub_copy(&opened.core) {
        return match (args.pos.first().copied(), profile) {
            (Some("set"), Some(text)) => hub_limits_set(&opened.core, &t, text, json),
            (Some("clear"), _) => hub_limits_clear(&opened.core, &t, json),
            _ => hub_limits_show(&opened.core, json),
        };
    }
    match (args.pos.first().copied(), profile) {
        (Some("set"), Some(text)) => limits_set(&opened.core, text, &mut pins, json),
        (Some("clear"), _) => limits_clear(&opened.core, &mut pins, json),
        _ => limits_show(&opened.core, json),
    }
}

pub(crate) fn limits_show(core: &DspiCore, json: bool) -> u8 {
    let st = core.limits_status();
    let doc = core.limits_profile_document();
    let profile: Value = serde_json::from_str(&doc).unwrap_or(Value::Null);
    if json {
        println!("{}", json!({ "status": st, "profile": profile }));
        return exit::OK;
    }
    if !st.notice_key.is_empty() {
        println!("{}", notice(&st.notice_key));
        if !st.detail.is_empty() {
            println!("  {}", printable(&st.detail));
        }
    }
    if !st.active {
        println!("No limits profile for this device on this console.");
        return exit::OK;
    }
    println!("Limits profile in force for {}:", printable(&st.serial));
    let p = profile.get("limits").unwrap_or(&profile);
    let num = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64);
    for o in p
        .get("outputs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let mut parts = Vec::new();
        if let Some(g) = num(o, "maxGainDb") {
            parts.push(format!("gain at most {g:+.1} dB"));
        }
        if let Some(hz) = num(o, "highpassMinHz") {
            let slope = num(o, "highpassMinSlopeDb")
                .map(|s| format!(", at least {s} dB/oct"))
                .unwrap_or_default();
            parts.push(format!("a high-pass at or above {hz} Hz{slope}"));
        }
        match (num(o, "delayMinMs"), num(o, "delayMaxMs")) {
            (None, None) => {}
            (lo, hi) => parts.push(format!(
                "delay {} to {} ms",
                lo.map_or("0".into(), |v| v.to_string()),
                hi.map_or("any".into(), |v| v.to_string())
            )),
        }
        if let Some(c) = num(o, "limiterCeilingDb") {
            parts.push(format!("limiter on at or below {c:+.1} dBFS"));
        }
        let out = o.get("output").and_then(Value::as_u64).unwrap_or(0);
        println!("  Output {}: {}", out.saturating_add(1), parts.join("; "));
    }
    if let Some(b) = num(p, "maxPeqBoostDb") {
        println!("  EQ boost at most {b:+.1} dB");
    }
    if let Some(v) = num(p, "masterVolumeMaxDb") {
        println!("  Master volume at most {v:+.1} dB");
    }
    let pages: Vec<&str> = p
        .get("lockedPages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !pages.is_empty() {
        println!("  The lock covers: {}", printable(&pages.join(", ")));
    }
    for (k, label) in [("note", "Note"), ("contact", "Contact")] {
        if let Some(t) = p.get(k).and_then(Value::as_str).filter(|t| !t.is_empty()) {
            println!("  {label}: {}", printable(t));
        }
    }
    for held in &st.last_held {
        println!("  Last held: {}", printable(held));
    }
    println!("{}", notice("limits.scope"));
    exit::OK
}

pub(crate) fn limits_set(core: &DspiCore, text: String, pins: &mut Pins<'_>, json: bool) -> u8 {
    if let Err(c) = ensure_unlocked(core, pins) {
        return c;
    }
    match core.limits_apply_profile(text) {
        Ok(applied) => {
            if json {
                println!("{}", serde_json::to_string(&applied).unwrap_or_default());
                return exit::OK;
            }
            println!("Limits profile set.");
            for line in applied
                .lines
                .iter()
                .chain(&applied.held)
                .chain(&applied.notes)
            {
                println!("  {}", printable(line));
            }
            println!("{}", notice("limits.scope"));
            exit::OK
        }
        Err(e) => fail(&e),
    }
}

pub(crate) fn limits_clear(core: &DspiCore, pins: &mut Pins<'_>, json: bool) -> u8 {
    if let Err(c) = ensure_unlocked(core, pins) {
        return c;
    }
    match core.limits_clear() {
        Ok(()) => {
            if json {
                println!("{}", json!({ "ok": true }));
            } else {
                println!("Limits profile removed: this device runs without limits here.");
            }
            exit::OK
        }
        Err(e) => fail(&e),
    }
}

// ─── lock ───────────────────────────────────────────────────────────────

const LOCK_USAGE: &str = "lock takes status, unlock, set-pin or clear-pin";

/// `uconsole lock {status,unlock,set-pin,clear-pin}`.
pub(crate) fn cmd_lock(target: &Target, flags: &[&str], json: bool) -> u8 {
    let args = match Args::parse(
        flags.get(1..).unwrap_or_default(),
        &["--installer-pin", "--new-pin"],
        &[],
    ) {
        Ok(a) => a,
        Err(e) => return usage(&e),
    };
    let verb = match args.pos.as_slice() {
        [] => "status",
        [v @ ("status" | "unlock" | "set-pin" | "clear-pin")] => v,
        _ => return usage(LOCK_USAGE),
    };
    if let Err(c) = check_pin_flags(&args) {
        return c;
    }
    let opened = match open(target, &Stores::default_dirs()) {
        Ok(o) => o,
        Err(c) => return c,
    };
    let mut pins = Pins::from_args(&args, &opened.serial);
    // -- WP HUBADMIN -- a hub's device: its PIN is the hub's
    if let Some(t) = hub_copy(&opened.core) {
        match verb {
            "set-pin" => return hub_pin_set(&opened.core, &t, &mut pins, json),
            "clear-pin" => return hub_pin_clear(&opened.core, &t, json),
            _ => {}
        }
    }
    lock_on(&opened.core, verb, &mut pins, json)
}

pub(crate) fn lock_on(core: &DspiCore, verb: &str, pins: &mut Pins<'_>, json: bool) -> u8 {
    match verb {
        "unlock" => unlock(core, pins, json),
        "set-pin" => set_pin(core, pins, json),
        "clear-pin" => clear_pin(core, pins, json),
        _ => {
            print_lock(&core.installer_lock_state(), json);
            exit::OK
        }
    }
}

fn print_lock(st: &LockState, json: bool) {
    if json {
        println!("{}", serde_json::to_string(st).unwrap_or_default());
        return;
    }
    if !st.pin_set {
        println!("{}", notice("installer_lock.no_pin"));
        return;
    }
    let state = if st.unlocked {
        "unlocked for this command"
    } else if st.locked {
        // -- WP TMCORE -- every write of this Terminal is held to it now
        "locked: only volume, mute and presets can be changed"
    } else {
        "set; the Installer settings need it"
    };
    println!("Installer PIN {}: {state}", lock_holder(&st.held_by));
    if !st.covers.is_empty() {
        println!("  Covers: {}", printable(&st.covers.join(", ")));
    }
    if st.wrong_tries > 0 {
        println!("  Wrong PINs since the last good one: {}", st.wrong_tries);
    }
    if st.retry_after_ms > 0 {
        println!("  Next try in {} s", st.retry_after_ms.div_ceil(1000));
    }
    for (label, text) in [("Note", &st.note), ("Contact", &st.contact)] {
        if !text.is_empty() {
            println!("  {label}: {}", printable(text));
        }
    }
    if !st.detail.is_empty() {
        println!("  {}", printable(&st.detail));
    }
    // -- WP TMCORE -- (was P1EFIX's gap line) every write of this Terminal
    // goes through the core now: `set`, `eq`, scripts and the interface too
    println!("{}", scope(st));
}

fn unlock(core: &DspiCore, pins: &mut Pins<'_>, json: bool) -> u8 {
    let st = core.installer_lock_state();
    if !st.pin_set {
        print_lock(&st, json);
        return exit::OK;
    }
    match ensure_unlocked(core, pins) {
        Ok(st) => {
            if json {
                print_lock(&st, true);
            } else {
                println!(
                    "The PIN is right. This command's unlock ends with it: `{COMMAND} limits` \
                     and `{COMMAND} lock` ask for the PIN themselves."
                );
                println!("{}", scope(&st));
            }
            exit::OK
        }
        Err(c) => c,
    }
}

fn set_pin(core: &DspiCore, pins: &mut Pins<'_>, json: bool) -> u8 {
    if let Err(c) = ensure_unlocked(core, pins) {
        return c;
    }
    let pin = match (pins.new)() {
        Ok(Some(p)) => p,
        Ok(None) => {
            return usage(&format!(
                "set-pin needs the new PIN: on a terminal it asks twice; in a script, set \
                 {NEW_PIN_ENV} or pass --new-pin - with the PIN on standard input"
            ));
        }
        Err(e) => return usage(&e),
    };
    match core.installer_pin_set(pin) {
        Ok(set) => {
            if json {
                println!("{}", serde_json::to_string(&set).unwrap_or_default());
                return exit::OK;
            }
            println!("Installer PIN set for this DSPi on this console.");
            println!("{}", notice("installer_lock.recovery"));
            println!("  Recovery code: {}", set.recovery_code);
            println!("{}", scope(&set.state));
            exit::OK
        }
        Err(e) => fail(&e),
    }
}

fn clear_pin(core: &DspiCore, pins: &mut Pins<'_>, json: bool) -> u8 {
    let st = core.installer_lock_state();
    if !st.pin_set {
        print_lock(&st, json);
        return exit::OK;
    }
    if let Err(c) = ensure_unlocked(core, pins) {
        return c;
    }
    match core.installer_pin_clear() {
        Ok(st) => {
            if json {
                print_lock(&st, true);
            } else {
                println!("Installer PIN cleared: nothing is locked on this console.");
            }
            exit::OK
        }
        Err(e) => fail(&e),
    }
}

// ─── the hub's copy (WP HUBADMIN) ───────────────────────────────────────

/// The Installer target when the open device's copy is the hub's.
pub(crate) fn hub_copy(core: &DspiCore) -> Option<InstallerTarget> {
    let t = core.installer_target();
    (t.copy == COPY_HUB).then_some(t)
}

/// Below Admin (or a session that is down): say why, before anything is sent.
fn hub_writable(t: &InstallerTarget) -> Result<(), u8> {
    if t.read_only_key.is_empty() {
        return Ok(());
    }
    eprintln!("{COMMAND}: {}", notice(&t.read_only_key));
    Err(if t.read_only_key == "disconnected" {
        exit::NO_DEVICE
    } else {
        exit::REJECTED
    })
}

/// The hub's state for a hub command: always read again right before a
/// change, so the role judged is the hub's live one, given or taken on the
/// hub's machine since this session paired or last read (-- WP HUBADMINF --
/// F1: a demoted Admin exits 3 before anything is sent).
fn hub_target(core: &DspiCore, t: &InstallerTarget) -> Result<InstallerTarget, u8> {
    if t.read_only_key == "disconnected" {
        return Ok(t.clone());
    }
    core.hub_limits_get().map_err(|e| fail(&e))?;
    Ok(core.installer_target())
}

fn print_hub(st: &HubAdminState) {
    let who = if st.can_edit {
        "an Admin: you may change it"
    } else {
        "read-only for this console's role"
    };
    println!(
        "The hub's copy for {} ({}; role {}):",
        printable(&st.serial),
        who,
        printable(&st.role)
    );
    println!(
        "  Installer PIN on the hub: {}",
        if !st.pin_set {
            "none"
        } else if st.locked {
            "set, locked"
        } else {
            "set"
        }
    );
    if st.wrong_tries > 0 {
        println!("  Wrong PINs since the last good one: {}", st.wrong_tries);
    }
    if st.retry_after_ms > 0 {
        println!("  Next try in {} s", st.retry_after_ms.div_ceil(1000));
    }
    for p in &st.pinned_modes {
        let mode = if p.mode.is_empty() {
            "the user's choice"
        } else {
            p.mode.as_str()
        };
        println!("  Mode for {}: {}", printable(&p.role), printable(mode));
    }
    for held in &st.held {
        println!("  Last held: {}", printable(held));
    }
}

pub(crate) fn hub_limits_show(core: &DspiCore, json: bool) -> u8 {
    let st = match core.hub_limits_get() {
        Ok(st) => st,
        Err(e) => return fail(&e),
    };
    if json {
        let profile: Value = serde_json::from_str(&st.profile_document).unwrap_or(Value::Null);
        println!(
            "{}",
            json!({ "copy": COPY_HUB, "hub": st, "target": core.installer_target(), "profile": profile })
        );
        return exit::OK;
    }
    println!("{}", notice(dspi_core::hub_admin::NOTICE_HUB_COPY));
    print_hub(&st);
    if st.profile_document.is_empty() {
        println!("No limits profile for this device on the hub.");
    } else {
        println!("{}", printable(&st.profile_document));
    }
    if !st.can_edit {
        println!("{}", notice(dspi_core::hub_admin::NOTICE_HUB_ROLE));
    }
    exit::OK
}

pub(crate) fn hub_limits_set(core: &DspiCore, t: &InstallerTarget, text: String, json: bool) -> u8 {
    if let Err(c) = hub_target(core, t).and_then(|t| hub_writable(&t)) {
        return c;
    }
    match core.hub_limits_set(text) {
        Ok(set) => {
            if json {
                println!("{}", serde_json::to_string(&set).unwrap_or_default());
                return exit::OK;
            }
            println!("Limits profile set on the hub.");
            for line in &set.held {
                println!("  Held: {}", printable(line));
            }
            exit::OK
        }
        Err(e) => fail(&e),
    }
}

pub(crate) fn hub_limits_clear(core: &DspiCore, t: &InstallerTarget, json: bool) -> u8 {
    if let Err(c) = hub_target(core, t).and_then(|t| hub_writable(&t)) {
        return c;
    }
    match core.hub_limits_clear() {
        Ok(st) => {
            if json {
                println!("{}", serde_json::to_string(&st).unwrap_or_default());
            } else {
                println!(
                    "Limits profile removed on the hub: this device runs without the hub's limits."
                );
            }
            exit::OK
        }
        Err(e) => fail(&e),
    }
}

pub(crate) fn hub_pin_set(
    core: &DspiCore,
    t: &InstallerTarget,
    pins: &mut Pins<'_>,
    json: bool,
) -> u8 {
    if let Err(c) = hub_target(core, t).and_then(|t| hub_writable(&t)) {
        return c;
    }
    // An Admin needs no installer PIN: only the new one is asked.
    let pin = match (pins.new)() {
        Ok(Some(p)) => p,
        Ok(None) => {
            return usage(&format!(
                "set-pin needs the new PIN: on a terminal it asks twice; in a script, set \
                 {NEW_PIN_ENV} or pass --new-pin - with the PIN on standard input"
            ));
        }
        Err(e) => return usage(&e),
    };
    match core.hub_pin_set(pin) {
        Ok(set) => {
            if json {
                println!("{}", serde_json::to_string(&set).unwrap_or_default());
                return exit::OK;
            }
            println!("Installer PIN set for this DSPi on the hub.");
            println!("{}", notice(NOTICE_HUB_RECOVERY));
            println!("  Recovery code: {}", set.recovery_code);
            exit::OK
        }
        Err(e) => fail(&e),
    }
}

pub(crate) fn hub_pin_clear(core: &DspiCore, t: &InstallerTarget, json: bool) -> u8 {
    if let Err(c) = hub_target(core, t).and_then(|t| hub_writable(&t)) {
        return c;
    }
    match core.hub_pin_clear() {
        Ok(st) => {
            if json {
                println!("{}", serde_json::to_string(&st).unwrap_or_default());
            } else {
                println!("Installer PIN cleared on the hub: the hub's lock is off.");
            }
            exit::OK
        }
        Err(e) => fail(&e),
    }
}

// ─── mode (WP HUBADMIN) ─────────────────────────────────────────────────

const MODE_USAGE: &str = "mode takes status, or pin ROLE MODE (ROLE viewer, control or \
                          admin; MODE simple, advanced, installer or none)";

/// `uconsole --link … mode {status,pin ROLE MODE}`: the hub's pinned modes.
pub(crate) fn cmd_mode(target: &Target, flags: &[&str], json: bool) -> u8 {
    let args = match Args::parse(flags.get(1..).unwrap_or_default(), &[], &[]) {
        Ok(a) => a,
        Err(e) => return usage(&e),
    };
    let pin = match args.pos.as_slice() {
        [] | ["status"] => None,
        ["pin", role, mode] => Some(((*role).to_owned(), (*mode).to_owned())),
        ["pin", role] => Some(((*role).to_owned(), "none".to_owned())),
        _ => return usage(MODE_USAGE),
    };
    let opened = match open(target, &Stores::default_dirs()) {
        Ok(o) => o,
        Err(c) => return c,
    };
    mode_on(&opened.core, pin, json)
}

pub(crate) fn mode_on(core: &DspiCore, pin: Option<(String, String)>, json: bool) -> u8 {
    let Some(t) = hub_copy(core) else {
        eprintln!(
            "{COMMAND}: {}",
            notice(dspi_core::hub_admin::NOTICE_NO_LIMITS)
        );
        return exit::REJECTED;
    };
    let st = match pin {
        None => core.hub_limits_get(),
        Some((role, mode)) => {
            if let Err(c) = hub_target(core, &t).and_then(|t| hub_writable(&t)) {
                return c;
            }
            let mode = if mode == "none" { String::new() } else { mode };
            core.hub_mode_pin(role, mode)
        }
    };
    match st {
        Ok(st) => {
            if json {
                println!(
                    "{}",
                    json!({ "role": st.role, "pinnedModes": st.pinned_modes })
                );
            } else {
                for p in &st.pinned_modes {
                    let mode = if p.mode.is_empty() {
                        "the user's choice"
                    } else {
                        p.mode.as_str()
                    };
                    println!("Mode for {}: {}", printable(&p.role), printable(mode));
                }
            }
            exit::OK
        }
        Err(e) => fail(&e),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::super::tests::{gain, scratch};
    use super::super::{Stores, prepare};
    use super::*;
    use std::sync::Arc;

    /// A core on the demo device with stores in `dir`: a new command.
    fn command(dir: &std::path::Path) -> Arc<DspiCore> {
        let core = prepare(&Stores::under(dir)).unwrap();
        core.open_demo().unwrap();
        core
    }

    fn never() -> Pins<'static> {
        Pins {
            current: Box::new(|| panic!("asked for a PIN with none set")),
            new: Box::new(|| panic!("asked for a new PIN")),
        }
    }

    fn giving(current: Option<&'static str>, new: Option<&'static str>) -> Pins<'static> {
        Pins {
            current: Box::new(move || current.map(str::to_owned)),
            new: Box::new(move || Ok(new.map(str::to_owned))),
        }
    }

    fn profile(max_gain_db: f32) -> String {
        dspi_session::limits::LimitsProfile {
            outputs: vec![dspi_session::limits::OutputLimits {
                output: 0,
                max_gain_db: Some(max_gain_db),
                ..Default::default()
            }],
            note: "Hand-over 2026-11".into(),
            ..Default::default()
        }
        .to_document()
    }

    #[test]
    fn wp_p1tm_no_pin_asks_nothing_and_limits_apply() {
        let dir = scratch("nopin");
        let core = command(&dir);
        assert_eq!(lock_on(&core, "status", &mut never(), false), exit::OK);
        assert_eq!(lock_on(&core, "unlock", &mut never(), false), exit::OK);
        assert_eq!(lock_on(&core, "clear-pin", &mut never(), true), exit::OK);

        // Narrowing below the device's value holds it to the limit at once.
        core.set_output_gain(0, 6.0).unwrap();
        assert_eq!(
            limits_set(&core, profile(0.0), &mut never(), false),
            exit::OK
        );
        assert!(gain(&core) <= 0.05, "{}", gain(&core));
        assert!(core.limits_status().active);
        // A write past it is refused.
        let e = core.set_output_gain(0, 3.0).unwrap_err();
        assert_eq!(super::super::error_exit(&e), exit::REJECTED);
        assert_eq!(limits_show(&core, false), exit::OK);
        assert_eq!(limits_show(&core, true), exit::OK);

        // The next command finds it in the folder.
        drop(core);
        let core = command(&dir);
        assert!(core.limits_status().active);
        assert_eq!(limits_clear(&core, &mut never(), false), exit::OK);
        assert!(!core.limits_status().active);
        assert_eq!(limits_show(&core, false), exit::OK);
    }

    #[test]
    fn wp_p1tm_a_pin_set_makes_limits_and_lock_changes_ask_for_it() {
        let dir = scratch("pin");
        let core = command(&dir);
        assert_eq!(
            lock_on(
                &core,
                "set-pin",
                &mut giving(None, Some("4711-door")),
                false
            ),
            exit::OK
        );
        drop(core);

        // A new command: locked out of the installer's changes.
        let core = command(&dir);
        let st = core.installer_lock_state();
        assert!(st.pin_set && !st.unlocked);
        // No PIN to be had: refused as a usage error, nothing changed.
        assert_eq!(
            limits_set(&core, profile(-3.0), &mut giving(None, None), false),
            exit::USAGE
        );
        assert!(!core.limits_status().active);
        // A wrong one: refused.
        assert_eq!(
            limits_set(
                &core,
                profile(-3.0),
                &mut giving(Some("1234-no"), None),
                false
            ),
            exit::REJECTED
        );
        assert!(!core.limits_status().active);
        // The right one.
        assert_eq!(
            limits_set(
                &core,
                profile(-3.0),
                &mut giving(Some("4711-door"), None),
                true
            ),
            exit::OK
        );
        assert!(core.limits_status().active);
        assert_eq!(lock_on(&core, "status", &mut never(), false), exit::OK);
        drop(core);

        // Changing the PIN needs the old one; the PIN prompt is asked first.
        let core = command(&dir);
        assert_eq!(
            lock_on(
                &core,
                "set-pin",
                &mut giving(Some("wrong-pin"), Some("5555-new")),
                false
            ),
            exit::REJECTED
        );
        assert_eq!(
            lock_on(
                &core,
                "set-pin",
                &mut giving(Some("4711-door"), Some("5555-new")),
                true
            ),
            exit::OK
        );
        drop(core);

        // Unlock checks it; clear-pin needs it; then nothing asks again.
        let core = command(&dir);
        assert_eq!(
            lock_on(&core, "unlock", &mut giving(Some("4711-door"), None), false),
            exit::REJECTED
        );
        drop(core);
        let core = command(&dir);
        assert_eq!(
            lock_on(
                &core,
                "clear-pin",
                &mut giving(Some("5555-new"), None),
                false
            ),
            exit::OK
        );
        drop(core);
        let core = command(&dir);
        assert!(!core.installer_lock_state().pin_set);
        assert_eq!(limits_clear(&core, &mut never(), false), exit::OK);
    }

    #[test]
    fn wp_p1tm_a_pin_is_never_taken_from_the_command_line() {
        let ok = Args::parse(
            &["--installer-pin", "-"],
            &["--installer-pin", "--new-pin"],
            &[],
        )
        .unwrap();
        assert_eq!(check_pin_flags(&ok), Ok(()));
        for bad in [["--installer-pin", "1234"], ["--new-pin", "5678"]] {
            let a = Args::parse(&bad, &["--installer-pin", "--new-pin"], &[]).unwrap();
            assert_eq!(check_pin_flags(&a), Err(exit::USAGE));
        }
    }

    #[test]
    fn wp_p1tm_the_recovery_code_unlocks_in_place_of_the_pin() {
        let dir = scratch("recovery");
        let code = {
            let core = command(&dir);
            core.installer_pin_set("2468-gate".into())
                .unwrap()
                .recovery_code
        };
        let core = command(&dir);
        let mut pins = Pins {
            current: Box::new(move || Some(code.to_lowercase())),
            new: Box::new(|| Ok(None)),
        };
        assert_eq!(lock_on(&core, "unlock", &mut pins, false), exit::OK);
        assert!(core.installer_lock_state().recovered);
    }
}
