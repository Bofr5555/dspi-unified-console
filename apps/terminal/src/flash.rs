//! `uconsole flash [FILE]`: put firmware on a board, behind the firmware guard.
//!
//! The order is the safety. Everything that can refuse does so while the board
//! still plays: the file is checked, the chip compared, the board's core voltage
//! read and judged, and any answer asked for, before the reboot. Each doubt
//! ends in nothing being done.
//!
//! - **The guard decides** ([`dspi_core::firmware_guard_check`], WP CG8 and CG14). It names
//!   the answer it wants: the phrase for a voltage too low or unknown, the word
//!   `OVERVOLT` for a raising image. `--force` only skips the ordinary "are you
//!   sure", and overrides neither. `--accept-core-voltage-risk` lets the question
//!   be asked; it answers nothing, and nothing on the command line can.
//! - **The board is read last thing before the reboot**, after every prompt, so
//!   an answer given minutes ago is judged against the board as it is now. An
//!   answer counts only for the risk it was asked about: if the re-read shows
//!   another one, that takes its own.
//! - **A drive that was already mounted** belongs to a board nobody can ask
//!   ([`dspi_core::firmware_guard_board_in_bootloader`]), even with a DSPi on the bus, since
//!   that DSPi need not be the board behind the drive. With `--device` naming a
//!   unit it is refused outright, as the file would not go where it was sent.
//! - **The demo device is not a board.** With `--demo` a drive is a board nobody
//!   can ask, like any mounted one, and since it may be a real path, the demo
//!   neither stands in for the board nor claims it came back.
//! - **USB only.** The drive has to appear here: a hub's device mounts on the
//!   hub's machine, and a MIDI port does not say which board is behind it.
//! - **It works read-only.** The device is opened through the same guard as
//!   `uconsole raw` ([`Target::open_raw`]), which lets the bootloader request through
//!   on purpose: another wire format, or an image that cannot be read at all, is
//!   when a board most needs this.
//! - **Writing is not success.** [`dspi_core::DspiCore::flash_confirmed`] (the
//!   guarded write, with its token) returning means the write ended, or the drive went away
//!   during it, which is assumed to be the board rebooting and which a pulled
//!   cable looks like too. Only the board
//!   coming back with the image's version is success.
//!
//! - **-- WP PGTM -- The update check decides** ([`firmware_update_check`] on
//!   the core's [`DspiCore::firmware_target`], PLAN-PG section 1.8): the
//!   voltage rule above, plus no image at all for a board without BOOTSEL (the
//!   DSPi-Zero, a board that could not be identified), and the typed phrase it
//!   names ([`LINEAGE_PHRASE`]) when this fork's firmware would give way to an
//!   image that may be upstream's. That phrase is asked on the terminal without
//!   `--accept-core-voltage-risk` (it is not a voltage risk), and is never typed
//!   for the person; a voltage phrase still needs the flag. A change of
//!   protocol generation (1.1.6 to 1.1.7 or back) is said before any question.
//!   Only `verdict.allowed` flashes.
//!
//! The device side goes through [`DspiCore`], the one owner of the guard's board
//! read; the core is otherwise for `firmware.rs`, and the rest of the terminal
//! stays on sessions.
//!
//! Owned by WP TM3 (docs/core-merge/PLAN-CLIENTS.md).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_cmd::COMMAND;
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use dspi_core::{
    BootloaderDrive, DspState, DspiCore, DspiError, FirmwareGuardBoard, FirmwareGuardDecision,
    FirmwareGuardVerdict, Uf2Info, bootloader_drives, info_platform, uf2_inspect,
};
// -- WP PGTM -- the update check (PGFWX's export) instead of the voltage rule alone
use dspi_core::{
    BundledFirmware,
    NOTICE_FIRMWARE_NOT_VERIFIED,
    notice_for_key,
    verify_bundled, // -- WP SECFIX --
};
use dspi_core::{
    FirmwareTarget, FirmwareUpdateRule, FirmwareUpdateVerdict, LINEAGE_PHRASE,
    firmware_update_check,
};
use dspi_proto::Platform;
use dspi_proto::packets::FirmwareVersion;
use dspi_transport::Transport;
use dspi_tui::shell::connect::{OpenError, Target, no_prompt, printable};
use serde_json::{Value, json};

use super::exit;

/// How often the drive and the rebooted board are looked for.
const POLL: Duration = Duration::from_millis(250);
/// Polls before giving up on either: thirty seconds, as the interface's wait.
const POLLS: u32 = 120;
/// The most that is read of a file: a UF2 of a whole RP2350 flash window is
/// 64 MiB, and [`uf2_inspect`] refuses anything over that with a reason.
const MAX_FILE: u64 = 65 * 1024 * 1024;
/// A board can keep changing while its prompt waits, but not for ever.
const MAX_QUESTIONS: u32 = 3;
const INFO_FILE: &str = "INFO_UF2.TXT";

const FLAGS: &str = "--force, --accept-core-voltage-risk, --drive PATH, --dry-run, --quiet, --json";

/// What the command line asked for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Request {
    file: Option<PathBuf>,
    drive: Option<PathBuf>,
    force: bool,
    accept_risk: bool,
    dry_run: bool,
    quiet: bool,
    json: bool,
}

impl Request {
    /// `args` follow the word `flash`. A flag this does not know is an error,
    /// never ignored: a misspelt one must not change what gets written.
    fn parse(args: &[&str]) -> Result<Self, String> {
        let mut req = Self::default();
        let mut it = args.iter().copied();
        while let Some(arg) = it.next() {
            match arg {
                "--force" => req.force = true,
                "--accept-core-voltage-risk" => req.accept_risk = true,
                "--dry-run" => req.dry_run = true,
                "--quiet" => req.quiet = true,
                "--json" => req.json = true,
                "--drive" => {
                    let path = it.next().ok_or("--drive needs the drive's mount point")?;
                    req.drive = Some(PathBuf::from(path));
                }
                other if other.starts_with("--drive=") => {
                    req.drive = other.strip_prefix("--drive=").map(PathBuf::from);
                }
                other if other.starts_with('-') && other != "-" => {
                    return Err(format!(
                        "flash has no flag `{other}`; it takes {FLAGS}. A core-voltage answer is \
                         typed when asked, on standard input"
                    ));
                }
                file if req.file.is_none() => req.file = Some(PathBuf::from(file)),
                extra => return Err(format!("flash takes one file; `{extra}` is a second")),
            }
        }
        Ok(req)
    }

    /// Progress and warnings: never on standard output, which `--json` owns.
    fn note(&self, text: &str) {
        if !self.quiet {
            eprintln!("{text}");
        }
    }

    /// A refusal or failure: the words for a person, and for `--json` the same
    /// words as the one report on standard output. Every way out of this command
    /// reports once, so a script always has something to parse. Returns `code`.
    fn fail(&self, code: u8, message: &str) -> u8 {
        eprintln!("{COMMAND}: {message}");
        self.json_failure(message);
        code
    }

    fn json_failure(&self, message: &str) {
        if self.json {
            println!("{}", json!({ "flashed": false, "error": message }));
        }
    }

    /// The command's result on standard output.
    fn result(&self, text: &str, value: Value) {
        if self.json {
            println!("{value}");
        } else if !self.quiet {
            println!("{text}");
        }
    }
}

/// What the flow needs from outside itself. A trait because the point of this
/// command is what happens when a board reboots, which a test cannot arrange
/// for real.
trait World {
    /// The bootloader drives that are mounted now.
    fn drives(&mut self) -> Vec<BootloaderDrive>;
    /// Open `target` for this command, guarded on a read-only device.
    fn open(&mut self, target: &Target) -> Result<Box<dyn Transport>, OpenError>;
    /// The serials of the DSPis on the bus.
    fn serials(&mut self) -> Vec<String>;
    /// Ask a person; a line without its line ending, `None` if nobody answers.
    fn ask(&mut self, prompt: &str) -> Option<String>;
    /// Wait one [`POLL`].
    fn pause(&mut self);
}

struct RealWorld {
    /// `--drive`: this path and no other.
    chosen: Option<PathBuf>,
    /// The demo device has no bootloader, so it must never find a real one.
    demo: bool,
}

impl World for RealWorld {
    fn drives(&mut self) -> Vec<BootloaderDrive> {
        match &self.chosen {
            Some(path) => drive_at(path).into_iter().collect(),
            None if self.demo => Vec::new(),
            None => bootloader_drives(),
        }
    }

    fn open(&mut self, target: &Target) -> Result<Box<dyn Transport>, OpenError> {
        target.open_raw(&no_prompt)
    }

    fn serials(&mut self) -> Vec<String> {
        dspi_transport::list_devices()
            .map(|d| d.into_iter().map(|d| d.serial).collect())
            .unwrap_or_default()
    }

    fn ask(&mut self, prompt: &str) -> Option<String> {
        eprint!("{prompt}");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(n) if n > 0 => Some(without_line_ending(&line).to_owned()),
            _ => None,
        }
    }

    fn pause(&mut self) {
        std::thread::sleep(POLL);
    }
}

/// One line ending off, and nothing else: the guard compares exactly.
fn without_line_ending(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

/// What `Uf2Info::platform` and `BootloaderDrive::platform` use for "not an
/// RP2040 or RP2350".
const PLATFORM_UNKNOWN: i8 = -1;

/// A drive named on the command line, if its identity file is there. A file that
/// names no boot ROM gives [`PLATFORM_UNKNOWN`], so it is refused rather than
/// taken for a drive still on its way.
fn drive_at(path: &Path) -> Option<BootloaderDrive> {
    let mut info = Vec::new();
    std::fs::File::open(path.join(INFO_FILE))
        .ok()?
        .take(4096)
        .read_to_end(&mut info)
        .ok()?;
    let info = String::from_utf8_lossy(&info).into_owned();
    Some(BootloaderDrive {
        path: path.to_string_lossy().into_owned(),
        platform: info_platform(&info).unwrap_or(PLATFORM_UNKNOWN),
        info,
    })
}

/// `uconsole flash [FILE]`. `flags` start with the word `flash`.
pub fn cmd_flash(target: &Target, flags: &[&str]) -> u8 {
    let req = match Request::parse(flags.get(1..).unwrap_or_default()) {
        Ok(r) => r,
        Err(e) => {
            // No request to ask, and a script still wants its report.
            let req = Request {
                json: flags.contains(&"--json"),
                ..Request::default()
            };
            return req.fail(exit::USAGE, &e);
        }
    };
    let image = match &req.file {
        Some(file) => match read_image(&req, file) {
            Ok(i) => Some(i),
            Err(code) => return code,
        },
        None => None,
    };
    let mut world = RealWorld {
        chosen: req.drive.clone(),
        demo: matches!(target, Target::Demo),
    };
    run(target, &req, image.as_ref(), &mut world)
}

/// A firmware file, read once: what was checked is what gets written.
struct Image {
    bytes: Vec<u8>,
    info: Uf2Info,
    name: String,
    /// -- WP SECFIX -- the core's manifest on it, by its file name (SECFWV):
    /// anything but `Verified` is labelled, never refused for it here.
    bundled: BundledFirmware,
}

fn read_image(req: &Request, file: &Path) -> Result<Image, u8> {
    let name = printable(&file.display().to_string());
    let mut bytes = Vec::new();
    let read = std::fs::File::open(file)
        .and_then(|f| f.take(MAX_FILE.saturating_add(1)).read_to_end(&mut bytes));
    if let Err(e) = read {
        return Err(req.fail(exit::USAGE, &format!("could not read {name}: {e}")));
    }
    let info = uf2_inspect(bytes.clone());
    if !info.valid {
        let problem = printable(&info.problem);
        return Err(req.fail(
            exit::USAGE,
            &format!("{name} is not a firmware file: {problem}"),
        ));
    }
    let base = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let bundled = verify_bundled(bytes.clone(), base); // -- WP SECFIX --
    Ok(Image {
        bytes,
        info,
        name,
        bundled,
    })
}

fn run(target: &Target, req: &Request, image: Option<&Image>, world: &mut dyn World) -> u8 {
    match flow(target, req, image, world) {
        Ok(()) => exit::OK,
        Err(code) => code,
    }
}

fn flow(
    target: &Target,
    req: &Request,
    image: Option<&Image>,
    world: &mut dyn World,
) -> Result<(), u8> {
    if let Target::Midi { .. } | Target::Link(_) = target {
        return Err(req.fail(
            exit::USAGE,
            &format!(
                "flash needs the board on this machine's USB, since its bootloader drive has to \
                 appear here. {} cannot promise that",
                target.via()
            ),
        ));
    }
    let demo = matches!(target, Target::Demo);
    if demo && req.drive.is_none() {
        return Err(req.fail(
            exit::USAGE,
            "the demo device has no bootloader drive; name a test one with --drive",
        ));
    }

    let mounted = world.drives();
    if mounted.len() > 1 {
        return Err(req.fail(
            exit::USAGE,
            &format!(
                "{} bootloader drives are mounted ({}); say which with --drive",
                mounted.len(),
                drive_list(&mounted)
            ),
        ));
    }
    if demo && mounted.is_empty() {
        // The demo device never reboots, so no drive is on its way.
        return Err(req.fail(
            exit::USAGE,
            "the demo device has no bootloader, and no drive is mounted at --drive",
        ));
    }
    if let Some(drive) = mounted.first()
        && drive.platform == PLATFORM_UNKNOWN
    {
        return Err(req.fail(
            exit::USAGE,
            &format!(
                "{} is not the drive of an RP2040 or RP2350 bootloader. Nothing was changed",
                printable(&drive.path)
            ),
        ));
    }
    if let (Some(drive), Target::Usb { serial: Some(s) }) = (mounted.first(), target) {
        return Err(req.fail(
            exit::USAGE,
            &format!(
                "a bootloader drive is already mounted at {}, and nothing says it is {}. Flash \
                 it without --device, or eject it first",
                printable(&drive.path),
                printable(s)
            ),
        ));
    }
    match (image, mounted.first()) {
        (Some(image), Some(drive)) => onto_mounted(target, req, image, drive, world),
        (Some(image), None) => onto_device(target, req, image, world),
        (None, Some(drive)) => {
            let path = printable(&drive.path);
            req.result(
                &format!("A board is already in its bootloader, at {path}."),
                json!({ "flashed": false, "drive": path, "entered": false }),
            );
            Ok(())
        }
        (None, None) => reboot_only(target, req, world),
    }
}

fn drive_list(drives: &[BootloaderDrive]) -> String {
    drives
        .iter()
        .map(|d| printable(&d.path))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A board on the bus, with the core's guard read available.
struct Device {
    core: Arc<DspiCore>,
    serial: String,
    state: DspState,
}

fn open_device(target: &Target, world: &mut dyn World) -> Result<Device, OpenOutcome> {
    let transport = world.open(target).map_err(OpenOutcome::Open)?;
    let serial = transport.descriptor().serial.clone();
    let core = DspiCore::new();
    crate::core_route::apply_read_only(&core); // -- WP ROMODE --
    core.open_transport(transport, serial.clone(), 0)
        .map_err(OpenOutcome::Probe)?;
    let state = core.get_state();
    Ok(Device {
        core,
        serial,
        state,
    })
}

enum OpenOutcome {
    Open(OpenError),
    Probe(DspiError),
}

fn open_for_flash(req: &Request, target: &Target, world: &mut dyn World) -> Result<Device, u8> {
    open_device(target, world).map_err(|outcome| match outcome {
        OpenOutcome::Open(e) => {
            let absent = e.is_no_device();
            let message = e.to_string();
            // Says it on standard error, and picks the exit code.
            let code = super::fail_open(e);
            if absent {
                eprintln!(
                    "{COMMAND}: a board already in its bootloader needs its drive mounted first; \
                     nothing was changed"
                );
            }
            req.json_failure(&message);
            code
        }
        OpenOutcome::Probe(e) => req.fail(
            exit::TRANSPORT,
            &format!(
                "could not talk to the board: {e}. Holding BOOTSEL while plugging it in mounts \
                 its bootloader drive without its firmware's help"
            ),
        ),
    })
}

fn running(state: &DspState) -> String {
    FirmwareVersion::new(
        state.fw_major,
        state.fw_minor,
        state.fw_patch,
        state.fw_beta,
    )
    .to_string()
}

fn chip(platform: i8) -> String {
    u8::try_from(platform).map_or_else(|_| "unknown chip".into(), |p| Platform::from_id(p).name())
}

/// Before a reboot that would leave the board waiting for a file it cannot
/// take. A chip this build does not know (the STM32H723 has no UF2 bootloader)
/// is refused too.
fn check_chip(req: &Request, state: &DspState, image: &Image) -> Result<(), u8> {
    let board = Platform::from_id(state.platform_id);
    if let Platform::Unknown(_) = board {
        return Err(req.fail(
            exit::USAGE,
            &format!(
                "this board reports {}, which has no UF2 bootloader this build knows of. Nothing \
                 was changed",
                board.name()
            ),
        ));
    }
    if i16::from(state.platform_id) != i16::from(image.info.platform) {
        return Err(req.fail(
            exit::USAGE,
            &format!(
                "{} is firmware for the {}, and this board is an {}. Nothing was changed",
                image.name,
                chip(image.info.platform),
                board.name()
            ),
        ));
    }
    Ok(())
}

// -- WP SECFIX --
/// The label of a file that is not a bundled DSPi Console image (SECFWV): the
/// core's notice row `firmware.not_verified`, "Not a verified DSPi Console
/// image: ...". `None` for a verified one.
fn image_label(image: &Image) -> Option<String> {
    if image.bundled == BundledFirmware::Verified {
        return None;
    }
    let n = notice_for_key(NOTICE_FIRMWARE_NOT_VERIFIED.to_owned(), String::new());
    Some(format!("{}: {} {}", n.title, n.reason, n.action))
}

fn describe_image(req: &Request, image: &Image) {
    let version = match image.info.firmware_version.as_str() {
        "" => "names no DSPi version".to_owned(),
        v => format!("DSPi {}", printable(v)),
    };
    req.note(&format!(
        "Image: {}: {version}, {}, {} bytes",
        image.name,
        chip(image.info.platform),
        image.info.payload_bytes
    ));
    if let Some(label) = image_label(image) {
        req.note(&label); // -- WP SECFIX --
    }
}

fn volts(mv: u16) -> String {
    format!("{}.{:02} V", mv / 1000, (mv % 1000) / 10)
}

/// What was answered and what the guard thinks now.
struct Passed {
    /// The voltage rule's record of the target, for the guarded write.
    board: FirmwareGuardBoard,
    typed: String,
    verdict: FirmwareGuardVerdict,
    // -- WP PGTM --
    /// The board as the update check judged it, and its whole answer.
    target: FirmwareTarget,
    update: FirmwareUpdateVerdict,
}

// -- WP PGTM -- the update check: the voltage rule, BOOTSEL, the lineage
fn check(image: &Image, target: &FirmwareTarget, typed: &str) -> FirmwareUpdateVerdict {
    firmware_update_check(image.bytes.clone(), target, typed)
}

// -- WP PGTM --
/// A board the update check offers no image: said, with nothing done.
fn no_bootsel(req: &Request, update: &FirmwareUpdateVerdict) -> u8 {
    let message = format!("{} Nothing was changed", printable(&update.verdict.message));
    eprintln!("{COMMAND}: {message}");
    if req.json {
        println!(
            "{}",
            json!({ "flashed": false, "error": message, "update": update })
        );
    }
    exit::REJECTED
}

// -- WP PGTM --
/// Whether the answer wanted is the lineage phrase alone (no voltage risk),
/// which is asked without `--accept-core-voltage-risk`.
fn lineage_only(update: &FirmwareUpdateVerdict) -> bool {
    update.rule == FirmwareUpdateRule::LineageLoss
        && update.verdict.decision == FirmwareGuardDecision::NeedsConfirmation
        && update.verdict.confirmation_phrase == LINEAGE_PHRASE
}

fn yes(answer: Option<String>) -> bool {
    answer.is_some_and(|a| matches!(a.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
}

/// Report a guard refusal in both forms: `--json` readers get it too.
fn refuse(req: &Request, verdict: &FirmwareGuardVerdict, why: &str) -> u8 {
    let message = format!("{why}. Nothing was changed");
    eprintln!("{COMMAND}: {message}");
    if req.json {
        println!(
            "{}",
            json!({ "flashed": false, "error": message, "guard": verdict })
        );
    }
    exit::REJECTED
}

/// Every question there is, and the guard's last word on the board as `read`
/// finds it after them. Returns only when the guard allows the image. `read` runs
/// again after each answer, so what the caller holds is the latest reading.
fn gate(
    req: &Request,
    image: &Image,
    go_ahead: &str,
    world: &mut dyn World,
    read: &mut dyn FnMut() -> FirmwareTarget,
) -> Result<Passed, u8> {
    let mut target = read();
    let mut typed = String::new();
    let mut update = check(image, &target, &typed);
    // -- WP PGTM -- no image for a board without BOOTSEL, whatever is typed
    if update.rule == FirmwareUpdateRule::NoBootsel {
        return Err(no_bootsel(req, &update));
    }
    if !update.generation_change.is_empty() {
        req.note(&printable(&update.generation_change));
    }
    if req.dry_run {
        return dry_run(req, image, &target, &update);
    }
    let mut verdict = update.verdict.clone();
    if verdict.allowed && !req.force {
        if !yes(world.ask(go_ahead)) {
            return Err(refuse(req, &verdict, "not confirmed"));
        }
        target = read();
        update = check(image, &target, &typed);
        verdict = update.verdict.clone();
    }
    let mut questions: u32 = 0;
    while !verdict.allowed {
        // -- WP PGTM -- a board re-read as one without BOOTSEL
        if update.rule == FirmwareUpdateRule::NoBootsel {
            return Err(no_bootsel(req, &update));
        }
        if verdict.decision == FirmwareGuardDecision::Refused {
            return Err(req.fail(exit::USAGE, &printable(&verdict.message)));
        }
        eprintln!("{COMMAND}: {}", printable(&verdict.message));
        // -- WP PGTM -- the lineage phrase is not a voltage risk: asked without the flag
        if !req.accept_risk && !lineage_only(&update) {
            return Err(refuse(
                req,
                &verdict,
                "refused. --force does not override this; only \
                 --accept-core-voltage-risk and the typed answer it asks for do",
            ));
        }
        questions = questions.saturating_add(1);
        if questions > MAX_QUESTIONS {
            return Err(refuse(req, &verdict, "this board keeps changing"));
        }
        let prompt = format!("Type {} to go ahead: ", verdict.confirmation_phrase);
        let answer = world.ask(&prompt).unwrap_or_default();
        if answer != verdict.confirmation_phrase {
            return Err(refuse(req, &verdict, "that was not the answer"));
        }
        typed = answer;
        target = read();
        update = check(image, &target, &typed);
        verdict = update.verdict.clone();
    }
    Ok(Passed {
        board: target.voltage.clone(),
        typed,
        verdict,
        target,
        update,
    })
}

/// `--dry-run`: what the guard says, with no question asked and nothing done.
/// Always ends the flow, through `Err`: `exit::OK` is an exit code like the others.
fn dry_run(
    req: &Request,
    image: &Image,
    target: &FirmwareTarget,
    update: &FirmwareUpdateVerdict,
) -> Result<Passed, u8> {
    let verdict = &update.verdict;
    let text = if verdict.allowed {
        format!("{} would install. Nothing was changed.", image.name)
    } else if lineage_only(update) {
        // -- WP PGTM --
        format!(
            "{} would need `{}` typed: {} Nothing was changed.",
            image.name,
            verdict.confirmation_phrase,
            printable(&verdict.message)
        )
    } else {
        format!(
            "{} would need `{}` typed, with --accept-core-voltage-risk: {} Nothing was changed.",
            image.name,
            verdict.confirmation_phrase,
            printable(&verdict.message)
        )
    };
    req.result(
        &text,
        json!({
            "flashed": false,
            "dryRun": true,
            "board": target.voltage,
            "guard": verdict,
            "verified": image.bundled == BundledFirmware::Verified, // -- WP SECFIX --
            // -- WP PGTM --
            "target": target,
            "update": update,
        }),
    );
    Err(if verdict.allowed {
        exit::OK
    } else {
        exit::REJECTED
    })
}

/// The file on a drive nobody can ask about.
fn onto_mounted(
    target: &Target,
    req: &Request,
    image: &Image,
    drive: &BootloaderDrive,
    world: &mut dyn World,
) -> Result<(), u8> {
    let path = printable(&drive.path);
    if drive.platform != image.info.platform {
        return Err(req.fail(
            exit::USAGE,
            &format!(
                "{} is firmware for the {}, and the board at {path} is an {}. Nothing was changed",
                image.name,
                chip(image.info.platform),
                chip(drive.platform)
            ),
        ));
    }
    describe_image(req, image);
    req.note(&format!(
        "Board: already in its bootloader at {path}, so its core voltage cannot be asked."
    ));
    let go_ahead = format!("Install this image on the board at {path}? [y/N] ");
    let passed = gate(req, image, &go_ahead, world, &mut || {
        // -- WP PGTM -- a board in BOOTSEL: it has BOOTSEL, nothing else known
        FirmwareTarget::in_bootloader()
    })?;
    // Anything on the bus already is not the board that is about to arrive.
    let back = Back::New(world.serials().into_iter().collect());
    write(req, image, drive, &passed)?;
    if matches!(target, Target::Demo) {
        // A real path may just have been written, and no board can say it took.
        req.result(
            &format!("Written to {path}; no board to confirm with (demo)."),
            json!({
                "flashed": true,
                "drive": path,
                "demo": true,
                "imageVersion": printable(&image.info.firmware_version),
                "running": null,
                "confirmed": false,
                "board": passed.board,
                "guard": passed.verdict,
                "update": passed.update, // -- WP PGTM --
            }),
        );
        return Ok(());
    }
    confirm(target, req, image, drive, &passed, &back, world)
}

/// The file on the board behind a running device.
fn onto_device(
    target: &Target,
    req: &Request,
    image: &Image,
    world: &mut dyn World,
) -> Result<(), u8> {
    let device = open_for_flash(req, target, world)?;
    check_chip(req, &device.state, image)?;
    describe_image(req, image);
    req.note(&format!("Board: running {}.", running(&device.state)));
    req.note(&format!(
        "Updating can reset settings. Save them first if you have not: \
         {COMMAND} export backup.dspipreset"
    ));
    let core = Arc::clone(&device.core);
    let passed = gate(
        req,
        image,
        "Reboot this board into its bootloader and install this image? [y/N] ",
        world,
        &mut || core.firmware_target(), // -- WP PGTM --
    )?;
    if passed.board.required_mv > 0 {
        req.note(&format!(
            "Board: core voltage {}.",
            volts(passed.board.required_mv)
        ));
    }
    let back = Back::Known(target.pinned(&device.serial));
    enter_bootloader(req, &device)?;
    let drive = wait_for_drive(req, world)?;
    write(req, image, &drive, &passed)?;
    confirm(target, req, image, &drive, &passed, &back, world)
}

/// The reboot, and the connection let go of so nothing holds the device while
/// it leaves. There is no answer to wait for and a failure looks the same as
/// success: only the drive arriving says, and without one nothing gets written.
/// A reboot refused before it was sent (the device lock, WP CG24) is said at
/// once instead of waiting for a drive that cannot come (WP TM25).
fn enter_bootloader(req: &Request, device: &Device) -> Result<(), u8> {
    // -- WP P1TM -- gated (P1TK): asked already, so the token is minted here
    // (required by every core, P1TZ).
    let token = device
        .core
        .confirm_token(dspi_core::ConfirmOp::EnterBootloader);
    let sent = device.core.enter_bootloader_confirmed(Some(token));
    device.core.disconnect();
    sent.map_err(|e| {
        let (code, message) = not_rebooted(&e);
        req.fail(code, &message)
    })
}

/// The exit code and the words for a reboot refused before it was sent.
fn not_rebooted(e: &DspiError) -> (u8, String) {
    // Like a read-only refusal, not worth a retry when denied; another
    // console's lock or a lost link may pass later.
    let code = match e {
        DspiError::Denied | DspiError::ReadOnly => exit::REJECTED,
        _ => exit::TRANSPORT,
    };
    (
        code,
        format!("the board was not rebooted: {e}. Nothing was changed"),
    )
}

/// `uconsole flash` with no file: the board in its bootloader, for what comes next.
fn reboot_only(target: &Target, req: &Request, world: &mut dyn World) -> Result<(), u8> {
    let device = open_for_flash(req, target, world)?;
    req.note(&format!("Board: running {}.", running(&device.state)));
    // -- WP PGTM -- a board without BOOTSEL is not sent into a bootloader it lacks
    let board = device.core.firmware_target();
    if !board.has_bootsel() {
        let name = board.board().map_or(
            "a board that could not be identified",
            dspi_core::board_name,
        );
        return Err(req.fail(
            exit::REJECTED,
            &format!(
                "this device ({name}) has no BOOTSEL bootloader this app can update it \
                 through. Nothing was changed"
            ),
        ));
    }
    if req.dry_run {
        req.result(
            "This would reboot the board into its bootloader. Nothing was changed.",
            json!({ "flashed": false, "entered": false, "dryRun": true }),
        );
        return Ok(());
    }
    req.note(&format!(
        "Updating can reset settings. Save them first if you have not: \
         {COMMAND} export backup.dspipreset"
    ));
    if !req.force && !yes(world.ask("Reboot this board into its bootloader? [y/N] ")) {
        return Err(req.fail(exit::REJECTED, "not confirmed. Nothing was changed"));
    }
    enter_bootloader(req, &device)?;
    let drive = wait_for_drive(req, world)?;
    let path = printable(&drive.path);
    req.result(
        &format!(
            "The board is in its bootloader, at {path}. {COMMAND} flash FILE installs an image; it \
             will ask about core voltage, as a board there cannot say what it needs."
        ),
        json!({ "flashed": false, "drive": path, "entered": true }),
    );
    Ok(())
}

fn wait_for_drive(req: &Request, world: &mut dyn World) -> Result<BootloaderDrive, u8> {
    req.note("Waiting for the bootloader drive...");
    for _ in 0..POLLS {
        let mut drives = world.drives();
        if drives.len() > 1 {
            return Err(req.fail(
                exit::USAGE,
                &format!(
                    "{} bootloader drives appeared ({}), and nothing says which is this board. \
                     Nothing was written; flash the right one with --drive",
                    drives.len(),
                    drive_list(&drives)
                ),
            ));
        }
        if let Some(drive) = drives.pop() {
            return Ok(drive);
        }
        world.pause();
    }
    Err(req.fail(
        exit::TRANSPORT,
        "no bootloader drive appeared. Nothing was written. Look for it in your file manager and \
         pass it with --drive, or unplug the board and hold BOOTSEL while plugging it back in",
    ))
}

/// Through the guard, with the board record the answers were given against.
fn write(req: &Request, image: &Image, drive: &BootloaderDrive, passed: &Passed) -> Result<(), u8> {
    req.note(&format!("Writing to {}...", printable(&drive.path)));
    // -- WP P1TM -- the write is a gated call (P1TK): the person has said yes
    // (or --force), so its token is minted here, with the drive there and no
    // device open on this core.
    let core = DspiCore::new();
    crate::core_route::apply_read_only(&core); // -- WP ROMODE --
    let token = core.confirm_token(dspi_core::ConfirmOp::FirmwareFlash);
    let written = core.flash_confirmed(
        image.bytes.clone(),
        drive.clone(),
        passed.board.clone(),
        passed.typed.clone(),
        Some(token),
    );
    written.map_err(|e| {
        let code = match &e {
            DspiError::FirmwareGuardBlocked { .. } => exit::REJECTED,
            DspiError::InvalidArgument { .. } => exit::USAGE,
            _ => exit::TRANSPORT,
        };
        req.fail(
            code,
            &format!(
                "{e}. The board stays in its bootloader at {}; {COMMAND} flash FILE tries again, and \
                 will ask about core voltage, as a board there cannot say what it needs",
                printable(&drive.path)
            ),
        )
    })
}

/// Which device on the bus is the board that was just written.
enum Back {
    /// It can be named.
    Known(Target),
    /// The one that was not there before.
    New(BTreeSet<String>),
}

impl Back {
    /// The target to open now. `Ok(None)`: not back yet.
    fn target(&self, usb: &Target, world: &mut dyn World) -> Result<Option<Target>, String> {
        match self {
            Self::Known(t) => Ok(Some(t.clone())),
            Self::New(before) => {
                let fresh: Vec<String> = world
                    .serials()
                    .into_iter()
                    .filter(|s| !before.contains(s))
                    .collect();
                match fresh.as_slice() {
                    [] => Ok(None),
                    [one] => Ok(Some(usb.pinned(one))),
                    many => Err(format!(
                        "{} new DSPis appeared, so which is this board cannot be told",
                        many.len()
                    )),
                }
            }
        }
    }
}

fn confirm(
    target: &Target,
    req: &Request,
    image: &Image,
    drive: &BootloaderDrive,
    passed: &Passed,
    back: &Back,
    world: &mut dyn World,
) -> Result<(), u8> {
    let wanted = image.info.firmware_version.as_str();
    let shown = printable(wanted);
    let report = |running: Option<&str>, confirmed: bool| {
        json!({
            "flashed": true,
            "drive": printable(&drive.path),
            "imageVersion": shown,
            "running": running,
            "confirmed": confirmed,
            "board": passed.board,
            "guard": passed.verdict,
            "update": passed.update, // -- WP PGTM --
            "target": passed.target,
        })
    };
    req.note("Waiting for the board to come back...");
    let mut why = "it did not come back".to_owned();
    for _ in 0..POLLS {
        let candidate = match back.target(target, world) {
            Ok(c) => c,
            Err(e) => {
                why = e;
                break;
            }
        };
        let Some(device) = candidate.and_then(|t| open_device(&t, world).ok()) else {
            world.pause();
            continue;
        };
        let now = running(&device.state);
        if wanted.is_empty() {
            req.result(
                &format!(
                    "Written. The board is back, running {now}; the image names no version, so \
                     that it took cannot be confirmed."
                ),
                report(Some(now.as_str()), false),
            );
            return Ok(());
        }
        return match device.core.firmware_version_matches(wanted) {
            Ok(true) => {
                req.result(
                    &format!("Done. The board is back, running {now}."),
                    report(Some(now.as_str()), true),
                );
                Ok(())
            }
            _ => {
                eprintln!(
                    "{COMMAND}: the board came back running {now}, but the image was {shown}: it did \
                     not take"
                );
                if req.json {
                    println!("{}", report(Some(now.as_str()), false));
                }
                Err(exit::REJECTED)
            }
        };
    }
    eprintln!(
        "{COMMAND}: the write ended, but {why}, so this is not confirmed. Check \
         it with {COMMAND} list. If the drive is still there the image did not take"
    );
    if req.json {
        println!("{}", report(None, false));
    }
    Err(exit::TRANSPORT)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_tm3;
