//! `uconsole --plain` (WP UB11, UXP B11, owner D16): the interface as lines,
//! for a screen reader, a log or a script. No alternate screen, no colour,
//! no box, braille or meter glyph: a prompt, the shared grammar the `:` line
//! and the one-shot commands take, the shell's verbs on the same core, the
//! overview as sentences (`dspi_tui::plain`) and, after every change, the
//! echo line: the command that makes it, as the interface's echo line shows
//! it. Also what `uconsole` opens with no command when `TERM=dumb`.
//!
//! Every line of standard input is read by `pro::stdin_answers`'s one
//! reader, so an `ab`, a guard's countdown, a confirmation or the
//! installer's PIN asked in between take their answers from the same
//! stream and never swallow the next command.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::io::Write;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use dspi_cmd::{COMMAND, Command};
use dspi_proto::value::Value;
use dspi_session::{DeviceState, Session};
use dspi_tui::shell::connect::Target;

use crate::core_route::{self, Confirm, OpenedCore, PinSource};
use crate::{exit, pro};

/// How often the page that waits for a device looks for one.
const LOOK_EVERY: Duration = Duration::from_secs(1);

/// The prompt, when a person is at standard input.
const PROMPT: &str = "dspi> ";

/// The `measure` verbs that run here, as on the interface's `:` line.
const MEASURE_HERE: &[&str] = &["docs", "show", "export"];

/// The shell's verbs, run on this session's core (`main::shell_verb`).
const SHELL_VERBS: &[&str] = &[
    "snapshot", "ab", "diff", "lock", "limits", "status", "measure",
];

/// Whether this run is the plain mode: `--plain` with no command, or no
/// command at a terminal that cannot draw the interface (`TERM=dumb`).
/// With `TERM=dumb` and standard input not a terminal, the input is a
/// script, as before.
pub(crate) fn wanted(flags: &[&str], term: Option<&str>, stdin_is_terminal: bool) -> bool {
    let command = flags
        .first()
        .is_some_and(|f| !f.starts_with('-') || *f == "-f");
    // `--help`, `--version`, `--doctor`, `--fixture` …: not the interface
    let other = flags
        .iter()
        .any(|f| *f != "--lite" && crate::NOT_A_SCRIPT.contains(f));
    !command
        && !other
        && (flags.contains(&"--plain") || (term == Some("dumb") && stdin_is_terminal))
}

/// What a line asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Go,
    Quit,
}

/// `uconsole --plain`: open the device, greet, then one command per line
/// until `quit` or the end of the input. Answers the first failure's exit
/// code when standard input is not a person (a script), else 0.
pub(crate) fn run(target: &Target, flags: &[&str]) -> u8 {
    // The installer's PIN (`--installer-pin -`) is standard input's first
    // line: read before the shared reader takes the rest
    let pins = match PinSource::for_this_command() {
        Ok(p) => p,
        Err(code) => return code,
    };
    let interactive = pro::interactive();
    let (opened, target) = match open(target, interactive) {
        Ok(o) => o,
        Err(code) => return code,
    };
    let confirm = Confirm::for_command(crate::yes(flags));
    let c = match core_route::connected_plain(&opened, confirm, pins) {
        Ok(c) => c,
        Err(e) => return crate::fail_open(e),
    };
    let target = c.link.clone().map_or(target, Target::Link);
    let mut p = Plain {
        session: c.session,
        state: c.state,
        target,
        force: crate::force(flags),
        first_failure: exit::OK,
        quit_asked: false,
    };
    drop(opened);
    p.greet();
    loop {
        if interactive {
            print!("{PROMPT}");
            let _ = std::io::stdout().flush();
        }
        let Ok(line) = pro::stdin_answers().recv() else {
            p.end();
            break;
        };
        if p.line(&line) == Step::Quit {
            break;
        }
    }
    if interactive {
        exit::OK
    } else {
        p.first_failure
    }
}

/// Open `target` on a core; with no DSPi on the bus and a person at the
/// terminal, the waiting page's choices as lines.
fn open(target: &Target, interactive: bool) -> Result<(OpenedCore, Target), u8> {
    match core_route::open_core(target, &pro::Stores::default_dirs(), &crate::prompt_pin) {
        Ok(o) => Ok((o, target.clone())),
        Err(e) if e.is_no_device() && *target == (Target::Usb { serial: None }) && interactive => {
            wait_for_device()
        }
        Err(e) => Err(e.report()),
    }
}

/// The waiting page (UXP A13) in lines: look for a board every second, or
/// open what the person types.
fn wait_for_device() -> Result<(OpenedCore, Target), u8> {
    println!("No DSPi found. Waiting for one (a board in BOOTSEL shows as RPI-RP2).");
    println!(
        "Type demo to try the demo, link ADDRESS to connect over DSPi Link (link alone looks \
         on the network), doctor to hear why it cannot see a device, or quit."
    );
    let mut prompted = false;
    loop {
        if !prompted {
            print!("{PROMPT}");
            let _ = std::io::stdout().flush();
            prompted = true;
        }
        let answer = pro::stdin_answers().recv_timeout(LOOK_EVERY);
        let typed = answer.is_ok();
        let target = match answer {
            Err(RecvTimeoutError::Disconnected) => return Err(exit::NO_DEVICE),
            Err(RecvTimeoutError::Timeout) => {
                if !dspi_transport::list_devices().is_ok_and(|d| !d.is_empty()) {
                    continue;
                }
                println!();
                Target::Usb { serial: None }
            }
            Ok(line) => {
                prompted = false;
                let words: Vec<&str> = line.split_whitespace().collect();
                match words.as_slice() {
                    [] => Target::Usb { serial: None },
                    ["demo" | "d"] => Target::Demo,
                    ["link" | "l", rest @ ..] if rest.len() <= 1 => {
                        match crate::link_target(rest.first().copied().unwrap_or("")) {
                            Ok(t) => t,
                            Err(e) => {
                                eprintln!("{COMMAND}: {e}");
                                continue;
                            }
                        }
                    }
                    ["doctor" | "?"] => {
                        print!("{}", crate::doctor::report(&Target::Usb { serial: None }).0);
                        continue;
                    }
                    ["quit" | "q" | "exit"] => return Err(exit::OK),
                    _ => {
                        println!("Type demo, link ADDRESS, doctor or quit; Enter looks again.");
                        continue;
                    }
                }
            }
        };
        match core_route::open_core(&target, &pro::Stores::default_dirs(), &crate::prompt_pin) {
            Ok(o) => return Ok((o, target)),
            // Gone again before it opened, or not there yet: keep waiting
            Err(e) if e.is_no_device() => {
                if typed {
                    println!("Still no DSPi.");
                }
            }
            Err(e) => eprintln!("{COMMAND}: {}: {}", target.via(), e.words()),
        }
    }
}

/// The plain mode's session.
struct Plain {
    session: Session,
    state: DeviceState,
    target: Target,
    force: bool,
    first_failure: u8,
    /// `quit` was typed once with something unsaved.
    quit_asked: bool,
}

impl Plain {
    fn failed(&mut self, code: u8) {
        if code != exit::OK && self.first_failure == exit::OK {
            self.first_failure = code;
        }
    }

    /// Re-read the device, as the interface does after a change.
    fn refresh(&mut self) {
        match self.session.snapshot() {
            Ok(b) => self.state.replace_bulk(b),
            Err(e) => eprintln!("{COMMAND}: {e}"),
        }
    }

    fn greet(&self) {
        let via = self.target.via();
        let serial = &self.state.caps.serial;
        let device = match (&self.target, via.is_empty()) {
            (Target::Demo, _) => "the demo device".to_owned(),
            (_, true) => format!("DSPi {serial} over USB"),
            (_, false) => format!("DSPi {serial} over {via}"),
        };
        println!(
            "{COMMAND} {}, plain mode. Connected to {device}.",
            env!("CARGO_PKG_VERSION")
        );
        for line in dspi_tui::plain::overview(&self.state, false) {
            println!("{line}");
        }
        println!("Type a command, help for the list, or quit.");
    }

    /// One line of input.
    fn line(&mut self, raw: &str) -> Step {
        let line = crate::strip_comment(raw).trim();
        if line.is_empty() {
            return Step::Go;
        }
        let tokens = dspi_cmd::tokenize(line);
        let words: Vec<&str> = tokens.iter().map(String::as_str).collect();
        let Some(first) = words.first().map(|w| w.to_ascii_lowercase()) else {
            return Step::Go;
        };
        if !matches!(first.as_str(), "quit" | "exit" | "q") {
            self.quit_asked = false;
        }
        let rest = words.get(1..).unwrap_or_default();
        match first.as_str() {
            "quit" | "exit" | "q" => return self.quit(),
            "help" | "?" => self.help(),
            "overview" | "o" => {
                self.refresh();
                let all = rest.first().is_some_and(|w| w.eq_ignore_ascii_case("all"));
                for l in dspi_tui::plain::overview(&self.state, all) {
                    println!("{l}");
                }
            }
            "show" => self.show(rest),
            "save" if rest.is_empty() => self.save(),
            "params" => {
                for l in crate::params_lines() {
                    println!("{l}");
                }
            }
            verb if SHELL_VERBS.contains(&verb) => self.shell_verb(&tokens),
            _ => self.grammar(&words),
        }
        Step::Go
    }

    fn quit(&mut self) -> Step {
        self.refresh();
        if self.state.has_unsaved_changes() && !self.quit_asked {
            self.quit_asked = true;
            println!(
                "{} Type save to keep them on the device, or quit again to leave them unsaved.",
                dspi_tui::plain::unsaved_sentence(&self.state)
            );
            return Step::Go;
        }
        Step::Quit
    }

    /// The end of the input.
    fn end(&mut self) {
        self.refresh();
        if self.state.has_unsaved_changes() {
            println!(
                "{} They are not saved to the device.",
                dspi_tui::plain::unsaved_sentence(&self.state)
            );
        }
    }

    fn help(&self) {
        println!("The plain mode's words:");
        for (word, what) in dspi_tui::plain::HELP {
            println!("  {word:<10} {what}");
        }
        println!(
            "Any parameter or command the shell takes runs here too, as `{COMMAND} COMMAND` \
             would: vol.user -20, out.mute 2 on, eq 1 1 pk 1k 1.4 -3, undo. After each change \
             its command is said again."
        );
    }

    fn show(&mut self, words: &[&str]) {
        self.refresh();
        match dspi_tui::plain::channel_named(&self.state, words) {
            Some(ch) => println!("{}", dspi_tui::plain::channel_sentence(&self.state, ch)),
            None => {
                eprintln!("{COMMAND}: show takes a channel: in 1, out 2, or its number");
                self.failed(exit::USAGE);
            }
        }
    }

    /// Save to Device: `preset.save` on the active slot, as Ctrl-S does.
    fn save(&mut self) {
        match dspi_session::preset_copy::known_slot(self.state.caps.active_preset) {
            Some(slot) => self.execute(Command::Set {
                path: "preset.save",
                indices: vec![slot],
                value: Value::Trigger,
            }),
            None => {
                eprintln!(
                    "{COMMAND}: {}; save into one with preset.save N",
                    dspi_tui::screens::presets::ACTIVE_UNKNOWN
                );
                self.failed(exit::USAGE);
            }
        }
    }

    fn shell_verb(&mut self, tokens: &[String]) {
        let words: Vec<String> = tokens
            .iter()
            .enumerate()
            .map(|(i, w)| {
                if i == 0 {
                    w.to_ascii_lowercase()
                } else {
                    w.clone()
                }
            })
            .collect();
        if words.first().map(String::as_str) == Some("measure")
            && !words
                .get(1)
                .is_some_and(|w| MEASURE_HERE.contains(&w.as_str()))
        {
            eprintln!(
                "{COMMAND}: measure takes {} here; measuring and the auto-tune run as `{COMMAND} \
                 measure`",
                MEASURE_HERE.join(", ")
            );
            self.failed(exit::USAGE);
            return;
        }
        let code = crate::shell_verb(&words, Some(&self.target));
        self.failed(code);
        self.refresh();
    }

    fn grammar(&mut self, words: &[&str]) {
        let ctx = crate::context_for(&self.session);
        match dspi_cmd::parse(words, &ctx) {
            Ok(cmd) => self.execute(cmd),
            // the way on is in here (UXP C10), not "Try `uconsole params`"
            Err(dspi_cmd::ParseError::Unknown(word)) => {
                eprintln!(
                    "{COMMAND}: no parameter or command called `{word}`; help lists the plain \
                     mode's words, params every parameter"
                );
                self.failed(exit::USAGE);
            }
            Err(e) => {
                eprintln!("{COMMAND}: {e}");
                eprintln!("      help lists the plain mode's words, params every parameter");
                self.failed(exit::USAGE);
            }
        }
    }

    /// Run one command of the grammar; after a change, say it as the
    /// command that makes it (the echo line) and re-read the device.
    fn execute(&mut self, cmd: Command) {
        let ctx = crate::context_for(&self.session);
        let echo = dspi_cmd::format(&cmd, &ctx);
        let change = match &cmd {
            Command::Set { .. } | Command::SetBand { .. } => true,
            Command::Verb { name, .. } => matches!(name.as_str(), "undo" | "redo"),
            Command::Get { .. } => false,
        };
        let preset = match &cmd {
            Command::Set { path, indices, .. }
                if matches!(*path, "preset.save" | "preset.load") =>
            {
                indices.first().copied()
            }
            _ => None,
        };
        let echoed = matches!(cmd, Command::Set { .. } | Command::SetBand { .. });
        let code = crate::run(&mut self.session, cmd, false, true, self.force);
        self.failed(code);
        if code != exit::OK || !change {
            return;
        }
        if echoed {
            println!("{echo}");
        }
        self.refresh();
        if let Some(slot) = preset {
            self.state.caps.active_preset = Some(slot);
            self.state.mark_saved();
        }
    }
}

#[cfg(test)]
mod tests_wp_ub11 {
    use super::*;

    #[test]
    fn ub11_plain_is_the_flag_or_a_dumb_terminal_and_never_a_command() {
        assert!(wanted(&["--plain"], None, true));
        assert!(wanted(&["--plain", "--demo"], Some("xterm"), false));
        assert!(wanted(&[], Some("dumb"), true));
        assert!(wanted(&["--lite"], Some("dumb"), true));
        // a command runs as a command
        assert!(!wanted(&["set", "vol.user", "-3", "--plain"], None, true));
        assert!(!wanted(&["status"], Some("dumb"), true));
        // a script on standard input stays a script on a dumb terminal
        assert!(!wanted(&[], Some("dumb"), false));
        assert!(!wanted(&["-f", "x.dspi"], Some("dumb"), true));
        // the interface as before
        assert!(!wanted(&[], Some("xterm-256color"), true));
        assert!(!wanted(&[], None, true));
        // what is not the interface stays what it is
        assert!(!wanted(&["--help"], Some("dumb"), true));
        assert!(!wanted(&["--version", "--plain"], None, true));
    }
}
