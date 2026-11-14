//! `uconsole snapshot`, `uconsole ab` and `uconsole open` (WP P1TM): the
//! core's snapshot store (P1SN), its A/B compare (P1AB) and offline
//! documents (P1OF).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::sync::mpsc::Receiver;

use dspi_cmd::COMMAND;
use dspi_core::{AbCompare, AbSide, DspiCore, SnapshotInfo, SnapshotRecall};
use dspi_tui::shell::connect::{Target, printable};
use serde_json::{Value, json};

use super::{
    Args, Stores, fail, guard_on_stdin, guard_seconds, interactive, open, print_diff, read_text,
    reply, stdin_answers, usage, write_text,
};
use crate::exit;

const SNAPSHOT_USAGE: &str = "snapshot takes save NAME [--note TEXT], list, recall ID [--guard \
                              SECONDS], diff ID [ID], export ID FILE|-, import FILE [--name NAME] \
                              or delete ID";

/// `uconsole snapshot ...`.
pub(crate) fn cmd_snapshot(target: &Target, flags: &[&str], json: bool) -> u8 {
    let args = match Args::parse(
        flags.get(1..).unwrap_or_default(),
        &["--note", "--name", "--guard"],
        &[],
    ) {
        Ok(a) => a,
        Err(e) => return usage(&e),
    };
    let Some((&verb, rest)) = args.pos.split_first() else {
        return usage(SNAPSHOT_USAGE);
    };
    if !matches!(
        verb,
        "save" | "list" | "recall" | "diff" | "export" | "import" | "delete"
    ) {
        return usage(SNAPSHOT_USAGE);
    }
    // A file to import is read before anything is opened.
    let imported = match (verb, rest) {
        ("import", [file]) => match read_text(file) {
            Ok(t) => Some(t),
            Err(c) => return c,
        },
        _ => None,
    };
    let opened = match open(target, &Stores::default_dirs()) {
        Ok(o) => o,
        Err(c) => return c,
    };
    let core = &opened.core;
    match (verb, rest) {
        ("save", [name]) => save(core, name, args.value("--note").unwrap_or(""), json),
        ("list", []) => list(core, json),
        ("recall", [id]) => match args.value("--guard").map(guard_seconds) {
            Some(Err(e)) => usage(&e),
            Some(Ok(secs)) => recall_guarded(core, id, secs, json),
            None => recall(core, id, json),
        },
        ("diff", [a]) => diff(core, a, None, json),
        ("diff", [a, b]) => diff(core, a, Some(b), json),
        ("export", [id, file]) => export(core, id, file, json),
        ("import", [file]) => import(
            core,
            imported.unwrap_or_default(),
            file,
            args.value("--name").unwrap_or(""),
            args.value("--note").unwrap_or(""),
            json,
        ),
        ("delete", [id]) => delete(core, id, json),
        _ => usage(SNAPSHOT_USAGE),
    }
}

/// The snapshot `wanted` names: its id, its number in `list` (from 1), or
/// its name when only one has it.
pub(crate) fn resolve(core: &DspiCore, wanted: &str) -> Result<SnapshotInfo, u8> {
    let all = core.snapshot_list(String::new());
    if let Some(s) = all.iter().find(|s| s.id == wanted) {
        return Ok(s.clone());
    }
    if let Ok(n) = wanted.parse::<usize>()
        && let Some(s) = n.checked_sub(1).and_then(|i| all.get(i))
    {
        return Ok(s.clone());
    }
    let named: Vec<&SnapshotInfo> = all.iter().filter(|s| s.name == wanted).collect();
    match named.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(usage(&format!(
            "no snapshot `{}` for this device; `{COMMAND} snapshot list` shows them",
            printable(wanted)
        ))),
        _ => Err(usage(&format!(
            "{} snapshots are called `{}`; use the number or id from `{COMMAND} snapshot list`",
            named.len(),
            printable(wanted)
        ))),
    }
}

fn info_json(s: &SnapshotInfo) -> Value {
    serde_json::to_value(s).unwrap_or(Value::Null)
}

/// When a snapshot was taken, as local-free UTC text.
fn when(ms: i64) -> String {
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

fn save(core: &DspiCore, name: &str, note: &str, json: bool) -> u8 {
    match core.snapshot_capture(name.into(), note.into()) {
        Ok(s) => {
            if json {
                println!("{}", info_json(&s));
            } else {
                println!("Saved snapshot {}: {}", s.id, printable(&s.name));
            }
            exit::OK
        }
        Err(e) => fail(&e),
    }
}

fn list(core: &DspiCore, json: bool) -> u8 {
    let all = core.snapshot_list(String::new());
    if json {
        println!("{}", Value::Array(all.iter().map(info_json).collect()));
        return exit::OK;
    }
    if all.is_empty() {
        println!("No snapshots for this device.");
    }
    for (i, s) in all.iter().enumerate() {
        println!(
            "{:>3}  {}  {}  {}{}",
            i.saturating_add(1),
            when(s.saved_ms),
            s.id,
            printable(&s.name),
            if s.note.is_empty() {
                String::new()
            } else {
                format!("  ({})", printable(&s.note))
            }
        );
    }
    exit::OK
}

fn print_recall(r: &SnapshotRecall, json: bool) {
    if json {
        println!("{}", serde_json::to_string(r).unwrap_or_default());
        return;
    }
    println!("Recalled {}", printable(&r.name));
    for line in r.changes.iter().chain(&r.notes) {
        println!("  {}", printable(line));
    }
}

fn recall(core: &DspiCore, id: &str, json: bool) -> u8 {
    let s = match resolve(core, id) {
        Ok(s) => s,
        Err(c) => return c,
    };
    match core.snapshot_recall(s.id) {
        Ok(r) => {
            print_recall(&r, json);
            exit::OK
        }
        Err(e) => fail(&e),
    }
}

fn recall_guarded(core: &DspiCore, id: &str, secs: u32, json: bool) -> u8 {
    let s = match resolve(core, id) {
        Ok(s) => s,
        Err(c) => return c,
    };
    match core.apply_guarded_snapshot(s.id, secs) {
        Ok(g) => guard_on_stdin(core, g, json),
        Err(e) => fail(&e),
    }
}

/// One snapshot against the device (what recalling it would change), or
/// two against each other.
fn diff(core: &DspiCore, a: &str, b: Option<&str>, json: bool) -> u8 {
    let text = |id: &str| -> Result<String, u8> {
        let s = resolve(core, id)?;
        core.snapshot_export(s.id).map_err(|e| fail(&e))
    };
    let first = match text(a) {
        Ok(t) => t,
        Err(c) => return c,
    };
    let v = match b {
        Some(b) => match text(b) {
            Ok(second) => dspi_core::files::config_diff(first, second),
            Err(c) => return c,
        },
        None => super::preview_all(core, first),
    };
    match reply(&v) {
        Ok(v) => {
            print_diff(&v, json);
            exit::OK
        }
        Err(c) => c,
    }
}

fn export(core: &DspiCore, id: &str, file: &str, json: bool) -> u8 {
    let s = match resolve(core, id) {
        Ok(s) => s,
        Err(c) => return c,
    };
    let text = match core.snapshot_export(s.id.clone()) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    if let Err(c) = write_text(file, &text) {
        return c;
    }
    if file != "-" {
        if json {
            println!("{}", json!({ "ok": true, "path": file, "id": s.id }));
        } else {
            println!("Wrote {}: snapshot {}", printable(file), printable(&s.name));
        }
    }
    exit::OK
}

fn import(core: &DspiCore, text: String, file: &str, name: &str, note: &str, json: bool) -> u8 {
    match core.snapshot_import(text, name.into(), note.into()) {
        Ok(s) => {
            if json {
                println!("{}", info_json(&s));
            } else {
                println!(
                    "Filed {} as snapshot {}: {}",
                    printable(file),
                    s.id,
                    printable(&s.name)
                );
            }
            exit::OK
        }
        Err(e) => fail(&e),
    }
}

fn delete(core: &DspiCore, id: &str, json: bool) -> u8 {
    let s = match resolve(core, id) {
        Ok(s) => s,
        Err(c) => return c,
    };
    match core.snapshot_delete(s.id.clone()) {
        Ok(()) => {
            if json {
                println!("{}", json!({ "ok": true, "id": s.id }));
            } else {
                println!("Deleted snapshot {}: {}", s.id, printable(&s.name));
            }
            exit::OK
        }
        Err(e) => fail(&e),
    }
}

// ─── A/B ────────────────────────────────────────────────────────────────

const AB_USAGE: &str = "ab takes [A] B (snapshots; with only B, the device now is A) \
                        and --keep a|b to end on one side at once";

/// `uconsole ab [A] B [--keep a|b]`.
pub(crate) fn cmd_ab(target: &Target, flags: &[&str], json: bool) -> u8 {
    let args = match Args::parse(flags.get(1..).unwrap_or_default(), &["--keep"], &[]) {
        Ok(a) => a,
        Err(e) => return usage(&e),
    };
    let (a, b) = match args.pos.as_slice() {
        [b] => (None, *b),
        [a, b] => (Some(*a), *b),
        _ => return usage(AB_USAGE),
    };
    let keep = match args.value("--keep").map(AbSide::from_key) {
        Some(None) => return usage("--keep takes a or b"),
        Some(Some(side)) => Some(side),
        None => None,
    };
    let opened = match open(target, &Stores::default_dirs()) {
        Ok(o) => o,
        Err(c) => return c,
    };
    let show = interactive() && !json && keep.is_none();
    let answers = if keep.is_some() {
        None
    } else {
        Some(stdin_answers())
    };
    ab_on(&opened.core, a, b, keep, answers.as_ref(), show, json)
}

fn print_compare(c: &AbCompare, json: bool) {
    if json {
        println!("{}", serde_json::to_string(c).unwrap_or_default());
        return;
    }
    let side = |s: AbSide| match s {
        AbSide::A => format!("A ({})", printable(&c.a_name)),
        AbSide::B => format!("B ({})", printable(&c.b_name)),
    };
    match (c.active, c.side) {
        (true, Some(s)) => println!("On {}", side(s)),
        (false, Some(s)) => println!("Kept {}", side(s)),
        _ => println!("No compare running."),
    }
    if let Some(r) = &c.recall {
        for line in r.changes.iter().chain(&r.notes) {
            println!("  {}", printable(line));
        }
    }
}

/// Run a compare on `core`: start it, then end at once on `keep`, or follow
/// `answers` (`t` toggle, `d` diff, `a`/`b` keep that side, `q` or the end
/// of the input keep the side the device is on).
pub(crate) fn ab_on(
    core: &DspiCore,
    a: Option<&str>,
    b: &str,
    keep: Option<AbSide>,
    answers: Option<&Receiver<String>>,
    show: bool,
    json: bool,
) -> u8 {
    let id = |w: &str| resolve(core, w).map(|s| s.id);
    let a_id = match a.map(id).transpose() {
        Ok(a) => a.unwrap_or_default(),
        Err(c) => return c,
    };
    let b_id = match id(b) {
        Ok(b) => b,
        Err(c) => return c,
    };
    let mut c = match core.ab_start(a_id, b_id) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };
    print_compare(&c, json);
    if let Some(side) = keep {
        return finish(core, side, json);
    }
    let Some(answers) = answers else {
        return exit::OK;
    };
    loop {
        if show {
            eprint!("[t]oggle, [d]iff, keep [a] or [b], [q]uit on this side: ");
            let _ = std::io::Write::flush(&mut std::io::stderr());
        }
        let Ok(line) = answers.recv() else {
            // The input ended: the device stays on the side it is on.
            return finish(core, c.side.unwrap_or(AbSide::A), json);
        };
        match line.trim().to_ascii_lowercase().as_str() {
            "" | "t" | "toggle" => match core.ab_toggle() {
                Ok(now) => {
                    c = now;
                    print_compare(&c, json);
                }
                Err(e) => return fail(&e),
            },
            "d" | "diff" => match core.ab_diff().map_err(|e| fail(&e)).and_then(|t| reply(&t)) {
                Ok(v) => print_diff(&v, json),
                Err(code) => return code,
            },
            "a" => return finish(core, AbSide::A, json),
            "b" => return finish(core, AbSide::B, json),
            "q" | "quit" | "keep" => return finish(core, c.side.unwrap_or(AbSide::A), json),
            other => eprintln!("`{}`: type t, d, a, b or q.", printable(other)),
        }
    }
}

fn finish(core: &DspiCore, side: AbSide, json: bool) -> u8 {
    match core.ab_keep(side) {
        Ok(c) => {
            print_compare(&c, json);
            exit::OK
        }
        Err(e) => fail(&e),
    }
}

// ─── Offline documents ──────────────────────────────────────────────────

/// The interface, run on an offline session: the file's name to show,
/// the session and its first state. Returns the exit code.
pub(crate) type Interface<'a> =
    &'a mut dyn FnMut(&str, &mut dspi_session::Session, dspi_session::DeviceState) -> u8;

/// `uconsole open FILE`: the interface on a document, with no device; the
/// changes go back to the file when asked. Without a terminal (or with
/// `--json`), what the document opens as.
pub(crate) fn cmd_open(flags: &[&str], json: bool, tui: Interface<'_>) -> u8 {
    // The interface's own switches are the binary's to read.
    let args = match Args::parse(
        flags.get(1..).unwrap_or_default(),
        &["--theme"],
        &["--lite", "--no-animation"],
    ) {
        Ok(a) => a,
        Err(e) => return usage(&e),
    };
    let [file] = args.pos.as_slice() else {
        return usage("open takes one .dspipreset document");
    };
    if *file == "-" {
        return usage("open edits a file, so it needs one: name it");
    }
    let text = match read_text(file) {
        Ok(t) => t,
        Err(c) => return c,
    };
    let shown = std::path::Path::new(file)
        .file_name()
        .map_or_else(|| (*file).to_owned(), |n| n.to_string_lossy().into_owned());
    use std::io::IsTerminal;
    if json || !interactive() || !std::io::stdout().is_terminal() {
        return describe(&text, &shown, json);
    }
    let edited = match edit_offline(&text, &shown, tui) {
        Ok(e) => e,
        Err(c) => return c,
    };
    let Some(saved) = edited.changed else {
        println!("No changes to {}.", printable(file));
        return edited.code;
    };
    match super::ask(&format!(
        "Save {} change{} to {}? [y/N] ",
        saved.count,
        if saved.count == 1 { "" } else { "s" },
        printable(file)
    )) {
        Some(a) if matches!(a.to_ascii_lowercase().as_str(), "y" | "yes") => {
            if let Err(c) = write_text(file, &saved.text) {
                return c;
            }
            println!("Saved {}.", printable(file));
            edited.code
        }
        _ => {
            println!("Not saved; {} is as it was.", printable(file));
            edited.code
        }
    }
}

/// A document as the interface left it.
pub(crate) struct Edited {
    /// The interface's exit code.
    pub code: u8,
    /// The document's new text and how many values differ from the file,
    /// when any do.
    pub changed: Option<Saved>,
}

pub(crate) struct Saved {
    pub text: String,
    pub count: usize,
}

/// Open `text` as the core's offline device (P1OF, `File: <shown>` in the
/// title), run `tui` on it, then read the document back: of the kind it was
/// opened as, compared with the file by the core's document diff.
pub(crate) fn edit_offline(text: &str, shown: &str, tui: Interface<'_>) -> Result<Edited, u8> {
    let opened = dspi_core::offline::document_device(text, shown).map_err(|e| fail(&e))?;
    for note in &opened.notes {
        eprintln!("{COMMAND}: {}", printable(note));
    }
    let mut c = dspi_tui::shell::connect::connected_on(Box::new(opened.device))
        .map_err(crate::fail_open)?;
    let code = tui(shown, &mut c.session, c.state.clone());
    let mut doc =
        dspi_session::preset_file::capture(&mut c.session, Some(opened.name)).map_err(|e| {
            eprintln!("{COMMAND}: the document could not be read back: {e}");
            exit::TRANSPORT
        })?;
    doc.meta.kind = Some(opened.kind.word().to_owned());
    let after = dspi_session::preset_file::write(&doc);
    let diff = reply(&dspi_core::files::config_diff(
        text.to_owned(),
        after.clone(),
    ))?;
    let count = super::diff_count(&diff);
    Ok(Edited {
        code,
        changed: (count > 0).then_some(Saved { text: after, count }),
    })
}

/// What a document opens as: its name, kind, platform and channels, and
/// what reading it reported. Through the core's `open_document`.
pub(crate) fn describe(text: &str, file: &str, json: bool) -> u8 {
    let core = DspiCore::new();
    match core.open_document(text.to_owned(), file.to_owned()) {
        Ok(reply_text) => match reply(&reply_text) {
            Ok(v) => {
                if json {
                    println!("{v}");
                } else {
                    let s = |k: &str| {
                        v.get(k).map_or_else(String::new, |x| match x {
                            Value::String(s) => printable(s),
                            o => o.to_string(),
                        })
                    };
                    println!(
                        "File: {} ({} \"{}\"), {}, {} inputs, {} outputs",
                        s("fileName"),
                        s("kind"),
                        s("name"),
                        s("platform"),
                        s("inputs"),
                        s("outputs")
                    );
                    for note in v
                        .get("notes")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                    {
                        println!("  {}", printable(note));
                    }
                }
                exit::OK
            }
            Err(c) => c,
        },
        Err(e) => fail(&e),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::super::tests::{demo, exported_then_moved, gain, scratch};
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.05
    }

    #[test]
    fn wp_p1tm_snapshot_save_list_recall_diff_export_import_delete() {
        let (core, dir) = demo("snapshots");
        assert_eq!(list(&core, false), exit::OK);
        assert_eq!(save(&core, "Before the gig ✓", "a note", true), exit::OK);
        let all = core.snapshot_list(String::new());
        assert_eq!(all.len(), 1);
        let was = gain(&core);
        core.set_output_gain(0, -8.0).unwrap();

        // By number, by name and by id.
        assert_eq!(resolve(&core, "1").unwrap().id, all[0].id);
        assert_eq!(resolve(&core, "Before the gig ✓").unwrap().id, all[0].id);
        assert_eq!(resolve(&core, &all[0].id).unwrap().name, "Before the gig ✓");
        assert_eq!(resolve(&core, "nope").unwrap_err(), exit::USAGE);
        assert_eq!(list(&core, false), exit::OK);
        assert_eq!(list(&core, true), exit::OK);

        // Against the device: the gain moved.
        assert_eq!(diff(&core, "1", None, true), exit::OK);
        let v = reply(&super::super::preview_all(
            &core,
            core.snapshot_export(all[0].id.clone()).unwrap(),
        ))
        .unwrap();
        assert!(super::super::diff_count(&v) >= 1, "{v}");

        assert_eq!(recall(&core, "1", false), exit::OK);
        assert!(near(gain(&core), was));

        // Export, import as a second, diff the two: the same settings.
        let file = dir.join("one.dspipreset");
        let path = file.to_str().unwrap();
        assert_eq!(export(&core, "1", path, true), exit::OK);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("\"snapshot\""));
        assert_eq!(import(&core, text, path, "Copy", "", false), exit::OK);
        assert_eq!(core.snapshot_list(String::new()).len(), 2);
        assert_eq!(diff(&core, "1", Some("Copy"), false), exit::OK);
        let v = reply(&dspi_core::files::config_diff(
            core.snapshot_export(all[0].id.clone()).unwrap(),
            core.snapshot_export(resolve(&core, "Copy").unwrap().id)
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(super::super::diff_count(&v), 0, "{v}");

        assert_eq!(delete(&core, "Copy", true), exit::OK);
        assert_eq!(core.snapshot_list(String::new()).len(), 1);
        // A preset document is not a snapshot: refused, nothing filed.
        let preset = core.config_export("p".into(), "1".into(), 0).unwrap();
        assert_eq!(import(&core, preset, "p", "", "", false), exit::USAGE);
    }

    #[test]
    fn wp_p1tm_a_guarded_recall_reverts_when_told() {
        let (core, _dir) = demo("snapguard");
        let id = core.snapshot_capture("s".into(), String::new()).unwrap().id;
        let was = gain(&core);
        core.set_output_gain(0, -6.0).unwrap();
        let g = core.apply_guarded_snapshot(id, 30).unwrap();
        assert!(near(gain(&core), was));
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send("r".to_string()).unwrap();
        let ended = super::super::wait_guard(&core, g, &rx, false);
        assert_eq!(ended.phase, dspi_core::GuardPhase::Reverted);
        assert!(near(gain(&core), -6.0));
    }

    #[test]
    fn wp_p1tm_ab_toggles_diffs_and_keeps_a_side() {
        let (core, _dir) = demo("ab");
        let was = gain(&core);
        core.set_output_gain(0, -4.0).unwrap();
        core.snapshot_capture("B".into(), String::new()).unwrap();
        core.set_output_gain(0, was).unwrap();

        // With only B, the device now is A. Toggle, diff, toggle, keep B.
        let (tx, rx) = std::sync::mpsc::channel();
        for line in ["t", "d", "t", "b"] {
            tx.send(line.to_string()).unwrap();
        }
        assert_eq!(
            ab_on(&core, None, "B", None, Some(&rx), false, false),
            exit::OK
        );
        assert!(near(gain(&core), -4.0));
        assert!(!core.ab_state().active);

        // --keep b at once.
        core.set_output_gain(0, was).unwrap();
        assert_eq!(
            ab_on(&core, None, "B", Some(AbSide::B), None, false, true),
            exit::OK
        );
        assert!(near(gain(&core), -4.0));

        // The input ends: the device stays on the side it is on.
        core.set_output_gain(0, was).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send("toggle".to_string()).unwrap();
        drop(tx);
        assert_eq!(
            ab_on(&core, None, "B", None, Some(&rx), false, true),
            exit::OK
        );
        assert!(near(gain(&core), -4.0));
        assert!(!core.ab_state().active);
        assert_eq!(
            ab_on(&core, None, "missing", None, None, false, true),
            exit::USAGE
        );
    }

    #[test]
    fn wp_p1tm_an_offline_edit_saves_exactly_the_edit() {
        let (core, dir) = demo("offline");
        let (text, was) = exported_then_moved(&core, -3.0);
        let _ = was;

        // Nothing edited: nothing to save.
        let mut untouched =
            |_: &str, _: &mut dspi_session::Session, _: dspi_session::DeviceState| 0u8;
        let e = edit_offline(&text, "plan.dspipreset", &mut untouched).unwrap();
        assert!(e.changed.is_none());

        // One edit in the interface: one change, and the file it would
        // write differs from the original by exactly that.
        let mut shown = String::new();
        let mut edit =
            |name: &str, s: &mut dspi_session::Session, st: dspi_session::DeviceState| {
                shown = name.to_owned();
                assert!(st.connected);
                s.write("out.gain", &[0], dspi_proto::value::Value::Float(-12.5))
                    .unwrap();
                0u8
            };
        let e = edit_offline(&text, "plan.dspipreset", &mut edit).unwrap();
        assert_eq!(shown, "plan.dspipreset");
        let saved = e.changed.unwrap();
        assert_eq!(saved.count, 1);
        let v = reply(&dspi_core::files::config_diff(
            text.clone(),
            saved.text.clone(),
        ))
        .unwrap();
        assert_eq!(super::super::diff_count(&v), 1, "{v}");
        assert!(v.to_string().contains("-12.5"), "{v}");

        // The file reads back on a core as the same document kind.
        let f = dir.join("plan.dspipreset");
        std::fs::write(&f, &saved.text).unwrap();
        assert_eq!(describe(&saved.text, "plan.dspipreset", true), exit::OK);
        let _ = scratch;
    }

    #[test]
    fn wp_p1tm_dates_read_as_utc() {
        assert_eq!(when(0), "1970-01-01 00:00 UTC");
        assert_eq!(when(1_763_078_400_000), "2025-11-14 00:00 UTC");
    }
}
