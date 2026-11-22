//! `uconsole measure ...` (WP P5TM, PLAN-P5 OD-P5m): measurement documents
//! and the auto-tune from a document, on the console core.
//!
//! - `mic import FILE`, `target import FILE`: a microphone's calibration or a
//!   target curve made into its document (the core's `mic_import`,
//!   `target_import`) and filed: in this console's store
//!   (`<local data dir>/uconsole/measurements`), or the hub's over DSPi Link.
//! - `docs [--kind K]`, `show ID`, `export ID --output N`: the documents of
//!   both stores (the hub's copy wins), one in words, one output's response
//!   as REW text.
//! - `tune ID [--dry-run] [--guard S]`: the proposal in words (every write,
//!   what it was held to, what is withdrawn and why), then applied guarded as
//!   one undo step with a Keep prompt on the terminal (PLAN-P5 section 1.4
//!   steps 8 to 11). The guard's length is the core's (the planned verify
//!   plus a margin) unless `--guard` gives one. After a verify that did not
//!   pass, the first Keep is refused (`measure.tune.keep_unverified`) and the
//!   person is asked again: only a second Keep typed after that question
//!   keeps it; nothing here ever confirms on its own.
//! - `run` (the demo device only): a measurement with the demo's simulated
//!   microphone (P5DEMO), filed.
//!
//! **No live capture** (OD-P5m): the Terminal opens no microphone. The
//! summed check and the verify need one, so they run only with `--demo`'s
//! simulated microphone; elsewhere a proposal with delay, polarity or
//! crossover changes is not applied (the web and desktop consoles measure
//! and check), and an EQ-only tune is applied unverified, said so.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use dspi_cmd::COMMAND;
use dspi_core::{
    DspiCore, DspiError, GuardPhase, GuardedApply, MeasureLayout, MeasureOutput, MeasurePhase,
    MeasureSetup, MeasureState, MeasureTuneCheck, MeasureTuneKeep, MeasureTuneOptions,
    MeasureTuneProposal, MeasureTuneStage, MeasureTuneState, MeasureTuneVerify, MergedMeasureDoc,
};
use dspi_tui::shell::connect::{Target, printable};
use serde_json::{Value, json};

use crate::exit;
use crate::pro::{self, Args, Stores, fail, fail_json, usage};

const MEASURE_USAGE: &str = "measure takes mic import FILE [--name NAME], target import FILE \
     [--name NAME] [--use speakers|headphones] [--coupler iec60318_4|flatPlate|hats], docs \
     [--kind measurement|mic|target], show ID, export ID --output N [FILE|-], tune ID \
     [--dry-run] [--guard SECONDS], or run [--outputs 1,2,...] [--name NAME] (the demo device \
     only); ID is a document's id or name from `docs`, or a document's file";

/// How often a run or a guard is driven (the core asks for about 50 ms).
const TICK: Duration = Duration::from_millis(50);

/// The simulated microphone's seed: every demo run hears the same room.
const SIM_SEED: u64 = 0x5EED_D3E0;

/// The user volume the demo device is set to before it measures, dB (S2
/// refuses above -12 dB; the demo starts at 0 dB). Only the demo's.
const DEMO_VOLUME_DB: f32 = -20.0;

/// The notice key of the refused first Keep after a failed verify.
const KEEP_UNVERIFIED: &str = "measure.tune.keep_unverified";

/// This console's folder for measurement, microphone and target documents.
pub(crate) fn default_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join(COMMAND).join("measurements"))
}

/// Name the measurement folder on `core` (and, -- WP ROLE0 --, the speaker
/// layouts' folder beside it).
fn name_dir(core: &DspiCore, dir: Option<&Path>) -> Result<(), u8> {
    match dir {
        Some(d) => {
            if let Some(parent) = d.parent()
                && let Err(e) = core.speaker_layout_set_dir(
                    parent
                        .join("speaker-layouts")
                        .to_string_lossy()
                        .into_owned(),
                )
            {
                eprintln!("{COMMAND}: the speaker layouts' folder: {e}");
            }
            core.measure_set_dir(d.to_string_lossy().into_owned())
                .map_err(|e| fail(&e))
        }
        None => {
            eprintln!("{COMMAND}: this computer has no local data folder for documents");
            Err(exit::TRANSPORT)
        }
    }
}

// ─── The command ────────────────────────────────────────────────────────

/// `uconsole measure ...`.
pub(crate) fn run(target: &Target, flags: &[&str], json: bool) -> u8 {
    let rest = flags.get(1..).unwrap_or_default();
    if rest.is_empty() || rest.iter().any(|f| matches!(*f, "--help" | "-h" | "help")) {
        println!("{}", help());
        return if rest.is_empty() {
            exit::USAGE
        } else {
            exit::OK
        };
    }
    let args = match Args::parse(
        rest,
        &[
            "--name",
            "--kind",
            "--output",
            "--guard",
            "--use",
            "--coupler",
            "--outputs",
            crate::core_route::PIN_FLAG,
        ],
        &["--dry-run"],
    ) {
        Ok(a) => a,
        Err(e) => return usage(&e),
    };
    let dir = default_dir();
    let dlink = pro::over_dlink(target);
    let name = args.value("--name").unwrap_or("");
    match args.pos.as_slice() {
        [kind @ ("mic" | "target"), "import", file] => {
            let text = match pro::read_text(file) {
                Ok(t) => t,
                Err(c) => return c,
            };
            let doc = if *kind == "mic" {
                dspi_core::mic_import(name.into(), file_name(file), text)
            } else {
                dspi_core::target_import(
                    name.into(),
                    text,
                    args.value("--use").unwrap_or("speakers").into(),
                    args.value("--coupler").unwrap_or("").into(),
                )
            };
            let doc = match doc {
                Ok(d) => d,
                Err(e) => return fail_json(&e, json),
            };
            with_store(target, dir.as_deref(), |core| {
                file_doc(core, doc, dlink, json)
            })
        }
        ["docs"] => with_store(target, dir.as_deref(), |core| {
            list(core, args.value("--kind").unwrap_or(""), json)
        }),
        ["show", id] => with_store(target, dir.as_deref(), |core| show(core, id, json)),
        ["export", id, rest @ ..] if rest.len() <= 1 => {
            let Some(output) = args.value("--output") else {
                return usage("export needs --output N (the output's number, from 1)");
            };
            let output = match output_number(output) {
                Ok(o) => o,
                Err(e) => return usage(&e),
            };
            let file = rest.first().copied().unwrap_or("-");
            with_store(target, dir.as_deref(), |core| {
                export(core, id, output, file, json)
            })
        }
        ["tune", id] => {
            let guard_s = match args.value("--guard").map(guard_seconds).transpose() {
                Ok(g) => g.unwrap_or(0),
                Err(e) => return usage(&e),
            };
            let opened = match pro::open_writing(target, &Stores::default_dirs(), true) {
                Ok(o) => o,
                Err(c) => return c,
            };
            if let Err(c) = name_dir(&opened.core, dir.as_deref()) {
                return c;
            }
            let options = TuneArgs {
                dry_run: args.has("--dry-run"),
                guard_s,
            };
            let show_countdown = pro::interactive() && !json;
            let answers = pro::stdin_answers();
            tune(&opened.core, id, &options, &answers, show_countdown, json)
        }
        ["run"] => {
            let outputs = match args.value("--outputs").map(output_list).transpose() {
                Ok(o) => o,
                Err(e) => return usage(&e),
            };
            let opened = match pro::open_writing(target, &Stores::default_dirs(), false) {
                Ok(o) => o,
                Err(c) => return c,
            };
            if let Err(c) = name_dir(&opened.core, dir.as_deref()) {
                return c;
            }
            let options = RunArgs {
                outputs,
                name: name.to_owned(),
                sweep_ms: 0,
            };
            measure_run(&opened.core, &options, pro::interactive() && !json, json)
        }
        _ => usage(MEASURE_USAGE),
    }
}

/// `uconsole measure --help`.
fn help() -> String {
    format!(
        "\
{COMMAND} measure: measurement documents and the auto-tune

    {COMMAND} measure mic import FILE [--name NAME]
        a measurement microphone's calibration file (miniDSP UMIK-1/UMIK-2
        text, Dayton Audio iMM-6C/UMM-6 text, REW calibration text or CSV)
        made into a microphone document and filed (over --link: on the hub)
    {COMMAND} measure target import FILE [--name NAME] [--use speakers|headphones]
        [--coupler iec60318_4|flatPlate|hats]
        a target curve (REW or CSV text) made into a target document and filed
    {COMMAND} measure docs [--kind measurement|mic|target]
        the filed documents, this computer's and (over --link) the hub's
    {COMMAND} measure show ID
        one document in words (--json: its text)
    {COMMAND} measure export ID --output N [FILE|-]
        output N's measured response as REW text (standard output by default)
    {COMMAND} measure tune ID [--dry-run] [--guard SECONDS]
        propose an auto-tune of the device from measurement ID: every change
        in words, with what it was held to and what is withdrawn and why.
        Then it is applied as one undo step with a way back: type keep, or
        revert; without keep it goes back when the guard runs out (by default
        the time the verify needs plus a margin; --guard 5 to {max}).
        --dry-run prints the proposal and writes nothing
    {COMMAND} --demo measure run [--outputs 1,2,...] [--name NAME]
        measure the demo device with its simulated microphone, and file it
        (speakers at the listening position in the demo's room: nothing
        plays, and no other device is measured here)

The Terminal opens no microphone. Delay, polarity and crossover changes need
the summed check, and a tune is verified by measuring again: here only the
demo's simulated microphone (--demo) can do either. Measure and tune a real
device in the web or desktop console; the Terminal applies an EQ-only tune
from a document, unverified.

ID is a document's id or its name from `docs`, or a document's file.",
        max = dspi_core::APPLY_GUARD_MAX_TIMEOUT_S
    )
}

/// `--guard SECONDS`: 5 to the guard's longest (590 s).
fn guard_seconds(value: &str) -> Result<u32, String> {
    let max = dspi_core::APPLY_GUARD_MAX_TIMEOUT_S;
    let s: u32 = value
        .trim()
        .trim_end_matches('s')
        .parse()
        .map_err(|_| format!("--guard takes whole seconds, not `{}`", printable(value)))?;
    if !(pro::GUARD_MIN_S..=max).contains(&s) {
        return Err(format!(
            "--guard takes {} to {max} seconds, not {s}",
            pro::GUARD_MIN_S
        ));
    }
    Ok(s)
}

/// An output's number as the words show it (from 1), as the core's index.
fn output_number(word: &str) -> Result<u8, String> {
    match word.trim().parse::<u8>() {
        Ok(n) if n >= 1 => Ok(n.saturating_sub(1)),
        _ => Err(format!(
            "an output is a number from 1, not `{}`",
            printable(word)
        )),
    }
}

/// `--outputs 1,2,3`.
fn output_list(list: &str) -> Result<Vec<u8>, String> {
    let out: Vec<u8> = list
        .split(',')
        .map(output_number)
        .collect::<Result<_, _>>()?;
    if out.is_empty() {
        return Err("--outputs needs at least one output".into());
    }
    Ok(out)
}

/// The file's own name, for a microphone document's `sourceName` (and its
/// angle: a UMIK file's name says 90°).
fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

/// Run `f` on a core with the stores named: over DSPi Link with the hub's
/// device open (the hub's documents too); otherwise with no device at all,
/// since this computer's documents need none.
fn with_store(target: &Target, dir: Option<&Path>, f: impl FnOnce(&DspiCore) -> u8) -> u8 {
    if pro::over_dlink(target) {
        let opened = match pro::open(target, &Stores::default_dirs()) {
            Ok(o) => o,
            Err(c) => return c,
        };
        if let Err(c) = name_dir(&opened.core, dir) {
            return c;
        }
        return f(&opened.core);
    }
    let core = match pro::prepare(&Stores::default_dirs()) {
        Ok(c) => c,
        Err(c) => return c,
    };
    if let Err(c) = name_dir(&core, dir) {
        return c;
    }
    f(&core)
}

// ─── Documents ──────────────────────────────────────────────────────────

/// File a document: on the hub over DSPi Link, else in this console's store.
pub(crate) fn file_doc(core: &DspiCore, doc: String, on_hub: bool, json: bool) -> u8 {
    let filed = if on_hub {
        core.hub_measure_doc_put(doc).map(|p| (p.doc, "the hub"))
    } else {
        core.measure_doc_put(doc).map(|d| (d, "this computer"))
    };
    match filed {
        Ok((d, place)) => {
            if json {
                println!("{}", serde_json::to_value(&d).unwrap_or(Value::Null));
            } else {
                println!(
                    "Filed {} {} on {place}: {}",
                    kind_words(&d.kind),
                    d.id,
                    printable(&d.name)
                );
            }
            exit::OK
        }
        Err(e) => fail_json(&e, json),
    }
}

fn kind_words(kind: &str) -> &'static str {
    match kind {
        "measurement" => "measurement",
        "mic" => "microphone",
        "target" => "target",
        _ => "document",
    }
}

/// Every filed document (both stores, the hub's winning).
fn merged(core: &DspiCore, kind: &str) -> Result<Vec<MergedMeasureDoc>, u8> {
    match core.measure_docs_merged(kind.into(), String::new()) {
        Ok(m) => {
            if !m.hub_notice_key.is_empty() {
                let n = dspi_core::notice_for_key(m.hub_notice_key.clone(), String::new());
                eprintln!(
                    "{COMMAND}: the hub's documents are not listed: {}",
                    n.reason
                );
            }
            Ok(m.entries)
        }
        Err(e) => Err(fail(&e)),
    }
}

pub(crate) fn list(core: &DspiCore, kind: &str, json: bool) -> u8 {
    let all = match merged(core, kind) {
        Ok(a) => a,
        Err(c) => return c,
    };
    if json {
        println!("{}", serde_json::to_value(&all).unwrap_or(Value::Null));
        return exit::OK;
    }
    if all.is_empty() {
        println!("No documents filed.");
    }
    for e in &all {
        let d = &e.doc;
        let serial = if d.serial.is_empty() {
            String::new()
        } else {
            format!("  (DSPi {})", printable(&d.serial))
        };
        let place = match (e.location.as_str(), e.local_copy) {
            (_, true) => "  [this computer's copy]",
            ("hub", _) => "  [hub]",
            ("both", _) => "  [hub and here]",
            _ => "",
        };
        println!(
            "{:<11}  {}  {}  {}{serial}{place}",
            kind_words(&d.kind),
            when(d.saved_ms),
            d.id,
            printable(&d.name)
        );
        if !d.note.is_empty() {
            println!("             {}", printable(&d.note));
        }
    }
    exit::OK
}

/// A document's text and its id ("" for a file): `wanted` is a filed
/// document's id or its name (when only one has it), or a document's file.
pub(crate) fn resolve(core: &DspiCore, wanted: &str) -> Result<(String, String), u8> {
    if Path::new(wanted).is_file() {
        return pro::read_text(wanted).map(|t| (String::new(), t));
    }
    let all = merged(core, "")?;
    let found: Vec<&MergedMeasureDoc> = match all.iter().find(|e| e.doc.id == wanted) {
        Some(e) => vec![e],
        None => all.iter().filter(|e| e.doc.name == wanted).collect(),
    };
    let e = match found.as_slice() {
        [one] => *one,
        [] => {
            return Err(usage(&format!(
                "no document `{}`; `{COMMAND} measure docs` lists them",
                printable(wanted)
            )));
        }
        _ => {
            return Err(usage(&format!(
                "{} documents are called `{}`; use the id from `{COMMAND} measure docs`",
                found.len(),
                printable(wanted)
            )));
        }
    };
    let id = e.doc.id.clone();
    let text = if e.location == "local" || e.local_copy {
        core.measure_doc_get(id.clone())
    } else {
        core.hub_measure_doc_get(id.clone()).map(|h| h.document)
    };
    text.map(|t| (id, t)).map_err(|e| fail(&e))
}

/// The document's JSON, or a usage error.
fn parsed(text: &str) -> Result<Value, u8> {
    serde_json::from_str(text).map_err(|e| usage(&format!("not a document: {e}")))
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

fn word<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// An output's role as the tune's words say it.
fn role_words(role: &str) -> &str {
    match role {
        "full" => "full range",
        "sub" => "subwoofer",
        other => other,
    }
}

fn placement_words(p: &Value) -> String {
    match word(p, "mode") {
        "listeningPosition" => "the listening position".into(),
        "multiPosition" => format!(
            "{} positions around the listening position, averaged",
            num(p, "count").unwrap_or(0.0)
        ),
        "nearField" => "near-field (close to one driver)".into(),
        "headphones" => format!(
            "headphones on a coupler ({}), {} reseats",
            printable(word(p, "coupler")),
            num(p, "reseats").unwrap_or(1.0)
        ),
        other => printable(other),
    }
}

pub(crate) fn show(core: &DspiCore, wanted: &str, json: bool) -> u8 {
    let (id, text) = match resolve(core, wanted) {
        Ok(t) => t,
        Err(c) => return c,
    };
    // The kind's own reader: a damaged or foreign file is refused here.
    let info = match dspi_core::measure_doc_info(text.clone()) {
        Ok(i) => i,
        Err(e) => return fail_json(&e, json),
    };
    if json {
        println!("{}", text.trim_end());
        return exit::OK;
    }
    let v = match parsed(&text) {
        Ok(v) => v,
        Err(c) => return c,
    };
    let id = if id.is_empty() { info.id.clone() } else { id };
    println!(
        "{} {}: {}",
        capitalised(kind_words(&info.kind)),
        if id.is_empty() { "(not filed)" } else { &id },
        printable(&info.name)
    );
    match info.kind.as_str() {
        "measurement" => show_measurement(v.get("measurement").unwrap_or(&Value::Null)),
        "mic" => show_mic(v.get("mic").unwrap_or(&Value::Null)),
        _ => show_target(v.get("target").unwrap_or(&Value::Null)),
    }
    if !info.note.is_empty() {
        println!("  Note: {}", printable(&info.note));
    }
    exit::OK
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// A whole number of a document (validated by its reader) as an integer.
fn int(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn show_measurement(m: &Value) {
    println!(
        "  DSPi {} ({}), measured {}",
        printable(word(m, "serial")),
        printable(word(m, "platform")),
        when(int(m, "savedMs"))
    );
    println!(
        "  Placement: {}",
        placement_words(m.get("placement").unwrap_or(&Value::Null))
    );
    match m.get("mic") {
        Some(mic) => println!(
            "  Microphone: {}{}",
            printable(word(mic, "name")),
            if mic.get("approximate").and_then(Value::as_bool) == Some(true) {
                " (dB SPL approximate)"
            } else {
                ""
            }
        ),
        None => println!("  Microphone: no calibration (levels are relative)"),
    }
    let s = m.get("stimulus").unwrap_or(&Value::Null);
    println!(
        "  Stimulus: {} log sweep {} Hz to {} Hz, {:.1} s, {} passes, gap {} ms, {:.1} dBFS",
        printable(word(s, "kind")),
        num(s, "f1Hz").unwrap_or(0.0),
        num(s, "f2Hz").unwrap_or(0.0),
        num(s, "durationMs").unwrap_or(0.0) / 1000.0,
        int(s, "repeat"),
        int(s, "gapMs"),
        num(s, "levelDb").unwrap_or(0.0)
    );
    let c = m.get("capture").unwrap_or(&Value::Null);
    println!(
        "  Capture: {} Hz, the microphone's clock {:+.1} ppm",
        int(c, "rateHz"),
        num(c, "ratioPpm").unwrap_or(0.0)
    );
    if c.get("timingReliable").and_then(Value::as_bool) == Some(true) {
        println!(
            "  Timing: reliable (largest cycle residual {:.1} µs)",
            num(c, "maxResidualUs").unwrap_or(0.0)
        );
    } else {
        // In words, as every client shows it (P5-D).
        println!("  Timing: unreliable: no delay, polarity or crossover change is offered from it");
        for r in c
            .get("reasons")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            println!("    {}", printable(r.as_str().unwrap_or("")));
        }
    }
    println!("  Outputs:");
    for o in m
        .get("outputs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let speaker = word(o, "speaker");
        let arrival = match num(o, "arrivalMs") {
            Some(a) => format!(", arrival {a:.4} ms"),
            None => String::new(),
        };
        println!(
            "    Output {} ({}{}){arrival}",
            int(o, "output").saturating_add(1),
            if speaker.is_empty() {
                String::new()
            } else {
                format!("{} ", printable(speaker))
            },
            role_words(word(o, "role"))
        );
    }
    for t in m
        .get("tunes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        println!(
            "  Tuned {}: {} (before: snapshot {}, after: snapshot {})",
            when(int(t, "at")),
            if t.get("kept").and_then(Value::as_bool) == Some(true) {
                "kept"
            } else {
                "not kept"
            },
            printable(word(t, "snapshotBefore")),
            printable(word(t, "snapshotAfter"))
        );
    }
}

fn show_mic(m: &Value) {
    let serial = word(m, "serial");
    if !serial.is_empty() {
        println!("  Serial: {}", printable(serial));
    }
    println!(
        "  Format: {}, {}°",
        printable(word(m, "format")),
        int(m, "angle")
    );
    if let Some(s) = num(m, "sensitivityDb") {
        println!("  Sensitivity: {s:.3} dB");
    }
    match num(m, "splOffsetDb") {
        Some(o) => println!(
            "  dB SPL scale: {o:+.2} dB{}",
            if m.get("approximate").and_then(Value::as_bool) == Some(true) {
                " (approximate until a calibrator step)"
            } else {
                ""
            }
        ),
        None => println!("  dB SPL scale: none (relative levels)"),
    }
    let source = word(m, "sourceName");
    if !source.is_empty() {
        println!("  From: {}", printable(source));
    }
}

fn show_target(t: &Value) {
    let coupler = word(t, "coupler");
    println!(
        "  For: {}{}",
        printable(word(t, "use")),
        if coupler.is_empty() {
            String::new()
        } else {
            format!(" on a {} coupler", printable(coupler))
        }
    );
    let d = t.get("definition").unwrap_or(&Value::Null);
    match word(d, "type") {
        "builtin" => println!("  Built in: {}", printable(word(d, "id"))),
        _ => println!("  Imported points"),
    }
}

pub(crate) fn export(core: &DspiCore, wanted: &str, output: u8, file: &str, json: bool) -> u8 {
    let (_, text) = match resolve(core, wanted) {
        Ok(t) => t,
        Err(c) => return c,
    };
    let rew = match core.measurement_export_rew(text, output) {
        Ok(r) => r,
        Err(e) => return fail_json(&e, json),
    };
    if let Err(c) = pro::write_text(file, &rew) {
        return c;
    }
    if file != "-" {
        if json {
            println!("{}", json!({ "ok": true, "path": file }));
        } else {
            println!(
                "Wrote {}: output {} as REW text",
                printable(file),
                u16::from(output).saturating_add(1)
            );
        }
    }
    exit::OK
}

/// When, as UTC text.
fn when(ms: i64) -> String {
    if ms <= 0 {
        return "(no date)".into();
    }
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        rem.div_euclid(3600),
        rem.rem_euclid(3600).div_euclid(60)
    )
}

/// Days since 1970-01-01 as a calendar date (Howard Hinnant's algorithm).
#[allow(clippy::arithmetic_side_effects)]
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

// ─── The demo's simulated microphone ────────────────────────────────────

/// On the demo device: its simulated microphone on, and its user volume
/// where a measurement may start (S2). Returns whether it is on; nothing
/// is changed on any other connection (the core refuses it there anyway).
fn demo_microphone(core: &DspiCore, json: bool) -> bool {
    if !core.measure_sim_info().available {
        return false;
    }
    if core.get_state().user_volume_db > DEMO_VOLUME_DB {
        match core.set_user_volume(DEMO_VOLUME_DB) {
            // The read-only mode refuses the apply too, in its own words.
            Err(DspiError::ReadOnly) => {}
            Err(e) => eprintln!("{COMMAND}: the demo's volume: {e}"),
            Ok(_) if json => {}
            Ok(_) => {
                println!("Demo device: the user volume set to {DEMO_VOLUME_DB:.0} dB to measure.");
            }
        }
    }
    match core.measure_sim_set(true, SIM_SEED) {
        Ok(i) if i.on => {
            if !json {
                println!("Microphone: {} (the demo's room).", i.label);
            }
            true
        }
        Ok(_) => false,
        Err(e) => {
            eprintln!("{COMMAND}: the simulated microphone: {e}");
            false
        }
    }
}

/// Words for a session's progress.
fn session_words(st: &MeasureState) -> String {
    let phase = match st.phase {
        MeasurePhase::PreRoll => "listening to the room",
        MeasurePhase::Ramp => "finding the level",
        MeasurePhase::Run => "measuring",
        MeasurePhase::Analysing => "analysing",
        MeasurePhase::WaitPosition => "waiting for the next position",
        _ => "",
    };
    let output = st
        .output
        .map(|o| format!(", output {}", u16::from(o).saturating_add(1)))
        .unwrap_or_default();
    format!(
        "run {} of {}: {phase}{output} ({:.0} %)",
        st.run.max(1),
        st.runs.max(1),
        st.progress.clamp(0.0, 1.0) * 100.0
    )
}

/// Show a progress line on standard error, in place.
fn progress(show: bool, line: &str) {
    if show {
        eprint!("\r{line}\x1b[K");
        let _ = std::io::stderr().flush();
    }
}

fn progress_end(show: bool) {
    if show {
        eprintln!();
    }
}

/// What `measure run` measures.
#[derive(Debug, Clone, Default)]
pub(crate) struct RunArgs {
    /// The outputs (0-based); `None`: every output of the layout.
    pub outputs: Option<Vec<u8>>,
    /// The document's name ("" names it by the date).
    pub name: String,
    /// One sweep, ms (0: the core's default).
    pub sweep_ms: u32,
}

// -- WP ROLE0 --
/// The layout to measure and tune with: the proposal's outputs and feeds,
/// with each output's role, speaker and group from the device's stored
/// speaker layout (outputs it leaves out are left out). With no stored
/// layout, the proposal as it is, and a warning that its roles are guessed.
pub(crate) fn roles_layout(core: &DspiCore) -> Result<MeasureLayout, DspiError> {
    let mut layout = core.measure_layout_proposal()?;
    match core.speaker_layout()? {
        Some(stored) => layout
            .outputs
            .retain_mut(|o| match stored.output(o.output) {
                Some(r) => {
                    o.role = r.role;
                    o.speaker = r.speaker.clone();
                    o.group = r.group;
                    true
                }
                None => false,
            }),
        None => eprintln!(
            "{COMMAND}: warning: no speaker layout is stored for this DSPi: the roles are \
             guessed from its filters; confirm them in Settings › Speakers"
        ),
    }
    Ok(layout)
}

/// `uconsole --demo measure run`: a walked measurement of the layout's
/// outputs with the demo's simulated microphone, filed in this console's
/// store. Nothing anywhere plays: the room is arithmetic.
pub(crate) fn measure_run(core: &DspiCore, a: &RunArgs, show: bool, json: bool) -> u8 {
    if !core.measure_sim_info().available {
        return usage(
            "the Terminal has no microphone: it measures only the demo device (--demo), with \
             its simulated microphone; measure a DSPi in the web or desktop console",
        );
    }
    let layout = match roles_layout(core) {
        Ok(l) => l,
        Err(e) => return fail_json(&e, json),
    };
    let outputs: Vec<MeasureOutput> = layout
        .outputs
        .iter()
        .filter(|o| a.outputs.as_ref().is_none_or(|w| w.contains(&o.output)))
        .map(|o| MeasureOutput {
            output: o.output,
            role: o.role,
            speaker: o.speaker.clone(),
            group: o.group,
            inverted: false,
        })
        .collect();
    if outputs.is_empty() {
        return usage("none of those outputs plays a programme input now");
    }
    if !demo_microphone(core, json) {
        eprintln!("{COMMAND}: the simulated microphone is not available");
        return exit::TRANSPORT;
    }
    let setup = MeasureSetup {
        outputs,
        sweep_ms: a.sweep_ms,
        gap_ms: if a.sweep_ms > 0 { 300 } else { 0 },
        name: a.name.clone(),
        ..MeasureSetup::default()
    };
    match core.measure_preflight(setup.clone()) {
        Ok(p) => {
            for w in &p.warnings {
                eprintln!("{COMMAND}: {}", printable(&w.message));
            }
            if !p.clear {
                for b in &p.blockers {
                    eprintln!("{COMMAND}: not measured: {}", printable(&b.message));
                }
                return exit::REJECTED;
            }
        }
        Err(e) => return fail_json(&e, json),
    }
    if let Err(e) = core.measure_start(setup) {
        return fail_json(&e, json);
    }
    let st = loop {
        let st = core.measure_tick();
        if !st.active {
            break st;
        }
        progress(show, &format!("Measuring: {}", session_words(&st)));
        std::thread::sleep(TICK);
    };
    progress_end(show);
    if st.phase != MeasurePhase::Done {
        let why = st
            .issue
            .as_ref()
            .map(|i| i.message.clone())
            .unwrap_or_else(|| "the run ended without a result".into());
        let _ = core.measure_clear();
        eprintln!("{COMMAND}: not measured: {}", printable(&why));
        return exit::REJECTED;
    }
    let text = core.measure_result();
    let _ = core.measure_clear();
    match text {
        Ok(t) => file_doc(core, t, false, json),
        Err(e) => fail_json(&e, json),
    }
}

// ─── The tune ───────────────────────────────────────────────────────────

/// What `measure tune` was told.
#[derive(Debug, Clone, Default)]
pub(crate) struct TuneArgs {
    /// Print the proposal and write nothing.
    pub dry_run: bool,
    /// The guard's length, s; 0: the core's (the planned verify plus a
    /// margin).
    pub guard_s: u32,
}

/// The layout to tune: the device's, limited to the outputs `doc` measured.
fn layout_for(core: &DspiCore, doc: &Value) -> Result<MeasureLayout, DspiError> {
    let measured: Vec<u8> = doc
        .pointer("/measurement/outputs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|o| o.get("output").and_then(Value::as_u64))
        .filter_map(|o| u8::try_from(o).ok())
        .collect();
    let mut layout = roles_layout(core)?;
    layout.outputs.retain(|o| measured.contains(&o.output));
    if layout.outputs.is_empty() {
        return Err(DspiError::InvalidArgument {
            msg: "none of the measured outputs plays a programme input now".into(),
        });
    }
    Ok(layout)
}

/// Print a proposal: every write in words, what it was held to, what is
/// withdrawn and why, the warnings and the maths' remarks.
fn print_proposal(p: &MeasureTuneProposal, json: bool) {
    if json {
        println!("{}", serde_json::to_value(p).unwrap_or(Value::Null));
        return;
    }
    println!(
        "Auto-tune of DSPi {} from \"{}\"",
        printable(&p.serial),
        printable(&p.measurement_name)
    );
    if !p.timing_reliable {
        println!(
            "  Timing unreliable: the measurement's timing is not trusted, so no delay, polarity \
             or crossover change is offered."
        );
    }
    for w in &p.warnings {
        println!("  Warning: {}", printable(&w.message));
    }
    if p.writes.is_empty() {
        println!("  No change proposed.");
    } else {
        println!("  Changes ({}):", p.writes.len());
        for w in &p.writes {
            println!("    {}", printable(&w.words));
        }
    }
    if !p.held.is_empty() {
        println!("  Held to:");
        for h in &p.held {
            println!("    {}", printable(h));
        }
    }
    if !p.withdrawn.is_empty() {
        println!("  Withdrawn:");
        for w in &p.withdrawn {
            println!("    {}", printable(&w.message));
        }
    }
    match p.sum_check {
        MeasureTuneCheck::Pending => println!(
            "  The summed check has not run: the delay, polarity and crossover changes are \
             applied only after it passed."
        ),
        MeasureTuneCheck::Passed => println!("  The summed check passed."),
        MeasureTuneCheck::Failed => println!(
            "  The summed check failed: the timing changes it did not confirm are withdrawn."
        ),
        MeasureTuneCheck::NotNeeded => {}
    }
    if !p.notes.is_empty() {
        println!("  Notes:");
        for n in &p.notes {
            println!("    {}", printable(n));
        }
    }
}

/// Drive the tune's runs (a summed check or a verify) to their end.
fn drive_runs(core: &DspiCore, what: &str, show: bool) -> MeasureTuneState {
    loop {
        let st = core.measure_tune_tick();
        if !st.running {
            progress_end(show);
            return st;
        }
        progress(
            show,
            &format!(
                "{what}: {} of {}; {}",
                st.run.max(1),
                st.runs.max(1),
                session_words(&st.session)
            ),
        );
        std::thread::sleep(TICK);
    }
}

/// `uconsole measure tune ID`: propose, print, and unless `--dry-run`
/// check, apply guarded, verify where a microphone is, and ask Keep.
/// `answers` are the person's lines (standard input).
pub(crate) fn tune(
    core: &DspiCore,
    wanted: &str,
    a: &TuneArgs,
    answers: &Receiver<String>,
    show: bool,
    json: bool,
) -> u8 {
    let (id, text) = if wanted.trim_start().starts_with('{') {
        (String::new(), wanted.to_owned())
    } else {
        match resolve(core, wanted) {
            Ok(t) => t,
            Err(c) => return c,
        }
    };
    let doc = match parsed(&text) {
        Ok(v) => v,
        Err(c) => return c,
    };
    let layout = match layout_for(core, &doc) {
        Ok(l) => l,
        Err(e) => return fail_json(&e, json),
    };
    // A filed measurement goes by its id, so Keep adds the tune's row to it.
    let measurement = if id.is_empty() { text } else { id };
    let p = match core.measure_tune_propose(measurement, layout, MeasureTuneOptions::default()) {
        Ok(p) => p,
        Err(e) => return fail_json(&e, json),
    };
    print_proposal(&p, json);
    if p.writes.is_empty() || a.dry_run {
        if a.dry_run && !json && !p.writes.is_empty() {
            println!("Nothing written (--dry-run).");
        }
        let _ = core.measure_tune_stop();
        return exit::OK;
    }
    let microphone = demo_microphone(core, json);
    let mut p = p;
    if p.sum_check == MeasureTuneCheck::Pending {
        if !microphone {
            eprintln!(
                "{COMMAND}: not applied: {}",
                dspi_core::notice_for_key("measure.tune.sum_check_needed".into(), String::new())
                    .reason
            );
            eprintln!(
                "      The Terminal has no microphone to run it (only --demo's simulated one): \
                 tune this device in the web or desktop console."
            );
            let _ = core.measure_tune_stop();
            return exit::REJECTED;
        }
        if let Err(e) = core.measure_tune_sum_check(p.id) {
            let _ = core.measure_tune_stop();
            return refused(&e, json);
        }
        let st = drive_runs(core, "Summed check", show);
        if st.stage != MeasureTuneStage::Proposed {
            let why = st.issue.map(|i| i.message).unwrap_or_default();
            eprintln!("{COMMAND}: the summed check stopped: {}", printable(&why));
            let _ = core.measure_tune_stop();
            return exit::REJECTED;
        }
        match core.measure_tune_proposal() {
            Some(q) => p = q,
            None => {
                eprintln!("{COMMAND}: the summed check left no proposal");
                return exit::TRANSPORT;
            }
        }
        if !json {
            println!("After the summed check:");
        }
        print_proposal(&p, json);
        if p.writes.is_empty() {
            let _ = core.measure_tune_stop();
            return exit::OK;
        }
    }
    apply_and_keep(core, &p, a.guard_s, microphone, answers, show, json)
}

/// A refusal of the tune's calls, said: a refusal of the device's state (the
/// preflight again, a write the limits or the lock refused, the summed check
/// not passed) is a refusal, never a usage error.
fn refused(e: &DspiError, json: bool) -> u8 {
    let code = fail_json(e, json);
    if matches!(e, DspiError::InvalidArgument { .. }) {
        exit::REJECTED
    } else {
        code
    }
}

/// Apply `p` guarded (`guard_s` 0: the core's length), verify it when there
/// is a microphone, and wait for Keep or Revert on `answers`.
pub(crate) fn apply_and_keep(
    core: &DspiCore,
    p: &MeasureTuneProposal,
    guard_s: u32,
    verify: bool,
    answers: &Receiver<String>,
    show: bool,
    json: bool,
) -> u8 {
    let g = match core.measure_tune_apply(p.id, guard_s) {
        Ok(g) => g,
        Err(e) => {
            let st = core.measure_tune_state();
            if !json && st.stage == MeasureTuneStage::Reverted {
                eprintln!("{COMMAND}: a change was refused, so the tune was put back:");
            }
            // -- WP REVERTW -- and when it was not wholly put back, say so
            if !json && st.stage == MeasureTuneStage::RevertIncomplete {
                eprintln!("{COMMAND}: a change was refused, and the tune was not wholly put back:");
                if let Some(i) = &st.issue {
                    eprintln!("      {}", printable(&i.message));
                    pro::revert_notice_lines(&i.key);
                }
            }
            let _ = core.measure_tune_stop();
            return refused(&e, json);
        }
    };
    let st = core.measure_tune_state();
    if !json {
        println!(
            "Applied {} changes as one undo step \"{}\", with a way back: it goes back in {} s \
             unless kept.",
            p.writes.len(),
            printable(&g.label),
            g.remaining_ms.div_ceil(1000)
        );
        if !st.snapshot_before_id.is_empty() {
            println!(
                "The device before it is snapshot {}.",
                st.snapshot_before_id
            );
        }
        if g.hub {
            eprintln!("{COMMAND}: {}", pro::backstop_line());
        }
    }
    if verify {
        if let Err(e) = core.measure_tune_verify() {
            eprintln!("{COMMAND}: not verified: {e}");
        } else if !json {
            println!("Verifying: measuring the tuned outputs again ...");
        }
    } else if !json {
        println!(
            "Not verified: the Terminal has no microphone to measure the result. Keep it only \
             if it sounds right."
        );
    }
    if !show && !json {
        eprintln!(
            "{COMMAND}: answer keep or revert on standard input within {} s, or it goes back",
            g.remaining_ms.div_ceil(1000)
        );
    }
    let (end, keep) = keep_or_revert(core, answers, show, json);
    report(&end, keep.as_ref(), json)
}

/// What the Keep prompt drives (the core; a stand-in in tests).
pub(crate) trait TuneDriver {
    fn tick(&self) -> MeasureTuneState;
    fn keep(&self) -> Result<MeasureTuneKeep, DspiError>;
    fn revert(&self) -> Result<GuardedApply, DspiError>;
}

impl TuneDriver for DspiCore {
    fn tick(&self) -> MeasureTuneState {
        self.measure_tune_tick()
    }
    fn keep(&self) -> Result<MeasureTuneKeep, DspiError> {
        self.measure_tune_keep()
    }
    fn revert(&self) -> Result<GuardedApply, DspiError> {
        self.measure_tune_revert()
    }
}

/// The refused first Keep after a failed verify.
fn keep_unverified(e: &DspiError, st: &MeasureTuneState) -> bool {
    e.to_string().contains(KEEP_UNVERIFIED)
        || st.issue.as_ref().is_some_and(|i| i.key == KEEP_UNVERIFIED)
}

// -- WP P5HOLD -- (review P5-D D3)
/// The notice key of a Keep refused while the verify runs.
const KEEP_DURING_VERIFY: &str = "measure.tune.keep_during_verify";

/// The countdown's sentence (the `guard.waiting` row).
fn countdown(remaining_ms: u32) -> String {
    let n = dspi_core::notice_for_key(dspi_core::NOTICE_GUARD_WAITING.into(), String::new());
    let s = remaining_ms.div_ceil(1000);
    format!("{} {}", n.title, n.reason.replace("{s}", &s.to_string()))
}

fn print_verify(v: &MeasureTuneVerify, json: bool) {
    if json {
        println!("{}", json!({ "verify": v }));
        return;
    }
    println!(
        "Verify: {}{}",
        if v.passed {
            "passed"
        } else {
            "did NOT pass: the measurement does not agree with the prediction"
        },
        if v.timing_reliable {
            ""
        } else {
            " (timing unreliable)"
        }
    );
    for o in &v.outputs {
        match o.rms_db {
            Some(r) => println!(
                "  Output {}: {r:.2} dB RMS from the prediction",
                u16::from(o.output).saturating_add(1)
            ),
            None => println!(
                "  Output {}: too few trusted points to compare",
                u16::from(o.output).saturating_add(1)
            ),
        }
    }
    for s in &v.sums {
        if let Some(r) = s.rms_db {
            println!(
                "  {} summed: {r:.2} dB RMS from the prediction",
                printable(&s.speaker)
            );
        }
    }
    for t in &v.transitions {
        if let Some(r) = t.verify_rms_db {
            println!(
                "  {}: outputs {} and {} played together, {r:.2} dB RMS from the prediction",
                printable(&t.speaker),
                u16::from(t.low_output).saturating_add(1),
                u16::from(t.high_output).saturating_add(1)
            );
        }
    }
    for n in &v.notes {
        println!("  {}", printable(n));
    }
}

/// Wait for keep or revert, one per line from `answers`, while the core
/// times the guard and runs the verify. Without an answer the guard goes
/// back at its timeout. **A refused first Keep after a failed verify** is
/// said, the lines already typed are dropped, and the person is asked
/// again: only a keep typed after that question is the confirmation (the
/// core counts it from a second after the refusal).
pub(crate) fn keep_or_revert(
    driver: &dyn TuneDriver,
    answers: &Receiver<String>,
    show: bool,
    json: bool,
) -> (MeasureTuneState, Option<MeasureTuneKeep>) {
    let mut answers_open = true;
    let mut verify_shown = false;
    let mut shown: Option<String> = None;
    loop {
        let st = driver.tick();
        if !matches!(
            st.stage,
            MeasureTuneStage::Applied | MeasureTuneStage::Verifying | MeasureTuneStage::Verified
        ) || st.guard.phase != GuardPhase::Waiting
        {
            progress_end(show && shown.is_some());
            return (st, None);
        }
        if !verify_shown
            && !st.running
            && let Some(v) = &st.verify
        {
            verify_shown = true;
            progress_end(show && shown.take().is_some());
            print_verify(v, json);
        }
        if show {
            let line = if st.running {
                format!(
                    "Verifying: {} of {}; {} - {}",
                    st.run.max(1),
                    st.runs.max(1),
                    session_words(&st.session),
                    countdown(st.guard.remaining_ms)
                )
            } else {
                format!("{} [k]eep, [r]evert:", countdown(st.guard.remaining_ms))
            };
            if shown.as_ref() != Some(&line) {
                progress(true, &format!("{line} "));
                shown = Some(line);
            }
        }
        let answer = if answers_open {
            match answers.recv_timeout(TICK) {
                Ok(line) => Some(line),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => {
                    answers_open = false;
                    None
                }
            }
        } else {
            std::thread::sleep(TICK);
            None
        };
        match answer.as_deref().map(|a| a.trim().to_ascii_lowercase()) {
            Some(a) if matches!(a.as_str(), "k" | "keep" | "y" | "yes") => match driver.keep() {
                Ok(k) => {
                    progress_end(show && shown.is_some());
                    let st = driver.tick();
                    return (st, Some(k));
                }
                Err(e) => {
                    progress_end(show && shown.take().is_some());
                    if keep_unverified(&e, &driver.tick()) {
                        let n = dspi_core::notice_for_key(KEEP_UNVERIFIED.into(), String::new());
                        eprintln!("{COMMAND}: not kept: {}", n.title);
                        eprintln!("      {}", n.reason);
                        eprintln!("      Type keep again to keep it anyway, or revert.");
                        // What was typed before this question is not its answer.
                        while answers.try_recv().is_ok() {}
                    } else if e.to_string().contains(KEEP_DURING_VERIFY) {
                        // -- WP P5HOLD -- the verdict first; a keep typed
                        // before it is not the answer.
                        let n = dspi_core::notice_for_key(KEEP_DURING_VERIFY.into(), String::new());
                        eprintln!("{COMMAND}: not kept yet: {}", n.title);
                        eprintln!("      {}", n.action);
                        while answers.try_recv().is_ok() {}
                    } else {
                        eprintln!("{COMMAND}: not kept: {e}");
                    }
                }
            },
            Some(a) if matches!(a.as_str(), "r" | "revert" | "n" | "no") => {
                if let Err(e) = driver.revert() {
                    eprintln!("{COMMAND}: {e}");
                }
            }
            Some(other) if !other.is_empty() => {
                progress_end(show && shown.take().is_some());
                eprintln!("Type keep or revert.");
            }
            Some(_) | None => {}
        }
    }
}

/// Say how the tune ended, and its exit code: kept is success, reverted a
/// refusal, the rest a transport failure.
fn report(st: &MeasureTuneState, keep: Option<&MeasureTuneKeep>, json: bool) -> u8 {
    let guard = keep.map(|k| &k.guard).unwrap_or(&st.guard);
    let code = match guard.phase {
        // -- WP REVERTW -- a revert not fully successful is never 0
        _ if st.stage == MeasureTuneStage::RevertIncomplete => exit::TRANSPORT,
        GuardPhase::Kept => exit::OK,
        GuardPhase::Reverted => exit::REJECTED,
        _ => exit::TRANSPORT,
    };
    if json {
        let keep = keep.map(|k| {
            json!({
                "snapshotBeforeId": k.snapshot_before_id,
                "snapshotAfterId": k.snapshot_after_id,
                "measurementId": k.measurement_id,
                "notes": k.notes,
            })
        });
        println!(
            "{}",
            json!({ "ok": code == exit::OK, "stage": st.stage, "guard": guard, "keep": keep })
        );
        return code;
    }
    match keep {
        Some(k) if guard.phase == GuardPhase::Kept => {
            println!("Kept: {}", printable(&guard.label));
            if !k.snapshot_before_id.is_empty() {
                println!("  Before: snapshot {}", k.snapshot_before_id);
            }
            if !k.snapshot_after_id.is_empty() {
                println!("  After: snapshot {}", k.snapshot_after_id);
            }
            if !k.snapshot_before_id.is_empty() && !k.snapshot_after_id.is_empty() {
                println!(
                    "  Compare them with: {COMMAND} ab {} {}",
                    k.snapshot_before_id, k.snapshot_after_id
                );
            }
            for n in &k.notes {
                println!("  {}", printable(n));
            }
        }
        _ => {
            let _ = pro::report_guard(guard, false);
            // -- WP REVERTW -- the tune's own word when the guard has none
            // (a Revert another console's takeover left kept)
            if st.stage == MeasureTuneStage::RevertIncomplete
                && guard.revert_notice.is_empty()
                && let Some(i) = &st.issue
            {
                println!("  {}", printable(&i.message));
                pro::revert_notice_lines(&i.key);
            }
        }
    }
    code
}

#[cfg(test)]
#[path = "measure/tests_wp_p5tm.rs"]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests_wp_p5tm;
