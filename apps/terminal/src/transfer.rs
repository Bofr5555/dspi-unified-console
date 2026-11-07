//! `dspi export`, `dspi import` and `dspi cs export|import` (WP TM10).
//!
//! Three documents move through here: the whole-device `.dspipreset`, a filter
//! bank (the Console's text file or its JSON form) and the control-surface
//! configuration. A file name of `-` is standard input or output.
//!
//! **Choosing the format.** `--format` wins. Otherwise an export goes by the
//! name (`.dspipreset` and `.json` are the whole-device document, anything else
//! the filter text), and to `-` it is the filter text, or the whole-device
//! document with `--json`. An import goes by the content, because a name says
//! nothing on a pipe; a `.dspipreset` is only ever read as one, so that a
//! mistyped file gets the real reason.
//!
//! **`--json`.** What a command says about itself becomes one JSON object on
//! standard output. Writing a document to `-` keeps standard output for the
//! document alone, so the report goes to standard error.

use std::io::{Read, Write};

use dspi_session::Session;
use dspi_session::filterfile;
use dspi_session::preset_file::{
    self, ControlSurfaceDocument, CsApplyReport, CsFileError, DocumentKind, FilterDocument,
    FilterReport, MAX_DOCUMENT_BYTES,
};
use dspi_tui::shell::connect::Target;
use serde_json::json;

use crate::{connect, exit, positional, resolve_channel};

/// What a `--format` can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Preset,
    Filters,
    FiltersJson,
}

impl Format {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "preset" => Some(Self::Preset),
            "filters" => Some(Self::Filters),
            "filters-json" => Some(Self::FiltersJson),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Preset => "preset",
            Self::Filters => "filters",
            Self::FiltersJson => "filters-json",
        }
    }
}

/// `--format`'s value, and the flags without it, so that the value is not taken
/// for a file name.
fn take_format<'a>(flags: &[&'a str]) -> Result<(Option<Format>, Vec<&'a str>), String> {
    let mut rest = Vec::with_capacity(flags.len());
    let mut format = None;
    let mut it = flags.iter().copied();
    while let Some(f) = it.next() {
        let value = match f.strip_prefix("--format") {
            Some("") => Some(it.next().ok_or("--format needs a value")?),
            Some(v) => v.strip_prefix('='),
            None => None,
        };
        match value {
            Some(v) => {
                format = Some(Format::parse(v).ok_or_else(|| {
                    format!("no format called `{v}`; try preset, filters or filters-json")
                })?);
            }
            None => rest.push(f),
        }
    }
    Ok((format, rest))
}

/// Which format an export writes.
fn export_format(path: &str, chosen: Option<Format>, json: bool) -> Format {
    if let Some(f) = chosen {
        return f;
    }
    if path == "-" {
        return if json {
            Format::Preset
        } else {
            Format::Filters
        };
    }
    if path.ends_with(preset_file::FILE_EXTENSION) || path.ends_with(".json") {
        Format::Preset
    } else {
        Format::Filters
    }
}

/// Which document an import reads.
fn import_kind(path: &str, chosen: Option<Format>, text: &str) -> DocumentKind {
    match chosen {
        Some(Format::Preset) => DocumentKind::Preset,
        Some(Format::Filters) => DocumentKind::FilterText,
        Some(Format::FiltersJson) => DocumentKind::FilterJson,
        None if path.ends_with(preset_file::FILE_EXTENSION) => DocumentKind::Preset,
        None => preset_file::sniff(text),
    }
}

/// Read a document, refusing one over [`MAX_DOCUMENT_BYTES`] before it is
/// parsed: this is untrusted input, and `-` has no size to look at first.
fn read_document(path: &str) -> Result<String, String> {
    let name = if path == "-" { "standard input" } else { path };
    let limit = MAX_DOCUMENT_BYTES as u64 + 1;
    let mut bytes = Vec::new();
    let read = if path == "-" {
        std::io::stdin().lock().take(limit).read_to_end(&mut bytes)
    } else {
        std::fs::File::open(path).and_then(|f| f.take(limit).read_to_end(&mut bytes))
    };
    read.map_err(|e| format!("could not read {name}: {e}"))?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(format!(
            "{name} is over {} MB, which is no DSPi document",
            MAX_DOCUMENT_BYTES >> 20
        ));
    }
    String::from_utf8(bytes).map_err(|_| format!("{name} is not UTF-8 text"))
}

/// Write a document. A reader that closed standard output early has what it
/// wanted, as with `head`, so that is not a failure.
fn write_document(path: &str, text: &str) -> Result<(), String> {
    if path == "-" {
        let mut out = std::io::stdout().lock();
        return match out
            .write_all(text.as_bytes())
            .and_then(|()| out.write_all(b"\n"))
            .and_then(|()| out.flush())
        {
            Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => {
                Err(format!("could not write standard output: {e}"))
            }
            _ => Ok(()),
        };
    }
    std::fs::write(path, text).map_err(|e| format!("could not write {path}: {e}"))
}

/// One line of a command's own report, to standard output unless that holds the
/// document. These are TM15's macros: a closed reader ends the report and not
/// the command, and any other failure ends the process with [`exit::TRANSPORT`].
fn say(to_stderr: bool, line: &str) {
    if to_stderr {
        eprintln!("{line}");
    } else {
        println!("{line}");
    }
}

/// Write the device's configuration to a file or standard output.
pub(crate) fn cmd_export(target: &Target, flags: &[&str], json: bool) -> u8 {
    let (chosen, flags) = match take_format(flags) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("dspi: {e}");
            return exit::USAGE;
        }
    };
    let args = positional(&flags);
    let Some(&path) = args.first() else {
        eprintln!(
            "dspi: export needs a file name, or - for standard output, \
             e.g. `dspi export tuning.txt`"
        );
        return exit::USAGE;
    };
    let format = export_format(path, chosen, json);
    let mut session = match connect(target) {
        Ok(s) => s,
        Err(c) => return c,
    };
    let (text, summary) = export_text(&mut session, path, format);
    finish_export(path, format.name(), &text, summary, json)
}

/// A document's text, and what to say about it: a line and its JSON fields.
fn export_text(
    session: &mut Session,
    path: &str,
    format: Format,
) -> (String, (String, serde_json::Value)) {
    if format == Format::Preset {
        let name = (path != "-")
            .then(|| std::path::Path::new(path).file_stem())
            .flatten()
            .and_then(|s| s.to_str())
            .map(str::to_string);
        let doc = preset_file::capture(session, name);
        let line = format!(
            "{} channels, {} crosspoints",
            doc.channels.len(),
            doc.matrix.len()
        );
        let fields = json!({ "channels": doc.channels.len(), "crosspoints": doc.matrix.len() });
        return (preset_file::write(&doc), (line, fields));
    }
    let doc = preset_file::capture_filters(session);
    let bands: usize = doc.channels.iter().map(|c| c.peq.len()).sum();
    let line = format!("{} channels, {bands} bands", doc.channels.len());
    let fields = json!({ "channels": doc.channels.len(), "bands": bands });
    let text = if format == Format::FiltersJson {
        doc.write()
    } else {
        filterfile::write(&doc.to_text(), doc.saved_utc.as_deref().unwrap_or(""))
    };
    (text, (line, fields))
}

/// Write the text and report on it.
fn finish_export(
    path: &str,
    format: &str,
    text: &str,
    (line, fields): (String, serde_json::Value),
    json: bool,
) -> u8 {
    if let Err(e) = write_document(path, text) {
        eprintln!("dspi: {e}");
        return exit::TRANSPORT;
    }
    if path == "-" {
        return exit::OK;
    }
    if json {
        let mut report = json!({ "ok": true, "path": path, "format": format });
        if let (Some(r), Some(f)) = (report.as_object_mut(), fields.as_object()) {
            r.extend(f.clone());
        }
        say(false, &report.to_string());
    } else {
        say(false, &format!("Wrote {path}: {line}"));
    }
    exit::OK
}

/// Read a document from a file or standard input and apply it.
pub(crate) fn cmd_import(target: &Target, flags: &[&str], json: bool) -> u8 {
    let (chosen, flags) = match take_format(flags) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("dspi: {e}");
            return exit::USAGE;
        }
    };
    let args = positional(&flags);
    let Some(&path) = args.first() else {
        eprintln!("dspi: import needs a file name, or - for standard input");
        return exit::USAGE;
    };
    let text = match read_document(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("dspi: {e}");
            return exit::USAGE;
        }
    };
    match import_kind(path, chosen, &text) {
        DocumentKind::Preset => import_preset(target, &text, path, &flags, json),
        DocumentKind::FilterText => match filterfile::parse(&text) {
            Ok(f) => import_filters(target, &FilterDocument::from_text(&f), path, &flags, json),
            Err(e) => usage_error(&e.to_string()),
        },
        DocumentKind::FilterJson => match preset_file::parse_filters(&text) {
            Ok(d) => import_filters(target, &d, path, &flags, json),
            Err(e) => usage_error(&e.to_string()),
        },
        DocumentKind::ControlSurfaces => usage_error(
            "this is a control-surface document; `dspi cs import` applies those, \
             since they move pins",
        ),
    }
}

fn usage_error(e: &str) -> u8 {
    eprintln!("dspi: {e}");
    exit::USAGE
}

/// Apply a whole-device document and report what happened.
///
/// Reading and validating come first, so that a bad file costs no connect and
/// touches nothing. Audio processing is always applied; volume levels and
/// hardware wiring only on request, as in the Console's import dialog.
fn import_preset(target: &Target, text: &str, path: &str, flags: &[&str], json: bool) -> u8 {
    let doc = match preset_file::parse(text) {
        Ok(d) => d,
        Err(e) => return usage_error(&e.to_string()),
    };

    if !json {
        say(
            false,
            &format!(
                "{path}: {}{}, {} channels",
                doc.meta.name.as_deref().unwrap_or("unnamed"),
                doc.meta
                    .platform
                    .as_deref()
                    .map(|p| format!(" from {p}"))
                    .unwrap_or_default(),
                doc.channels.len()
            ),
        );
        let features: Vec<&str> = [
            doc.psybass.is_some().then_some("psychoacoustic bass"),
            doc.upmix.is_some().then_some("upmixer"),
            doc.subharm.is_some().then_some("subharmonic synthesizer"),
            doc.tube.is_some().then_some("tube modeller"),
            doc.crossfeed
                .pair_map
                .is_some()
                .then_some("crossfeed pairs"),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !features.is_empty() {
            say(false, &format!("Carries: {}", features.join(", ")));
        }
    }

    let mut session = match connect(target) {
        Ok(s) => s,
        Err(c) => return c,
    };
    session.dry_run = flags.contains(&"--dry-run");

    let options = preset_file::ApplyOptions {
        audio_processing: true,
        volume_levels: flags.contains(&"--volume"),
        hardware_io: flags.contains(&"--hardware"),
    };
    let report = preset_file::apply(&mut session, &doc, options);

    // The pair links are the interface's, not the device's, and a one-shot
    // command has nowhere to keep them. Better said than silently lost.
    let links = doc.global.input_pair_linked.iter().any(|l| *l);
    let mut lines = report.lines(session.dry_run);
    if links {
        lines.push("Not applied: input pair links, which only the interface keeps.".into());
    }
    if json {
        say(
            false,
            &json!({
                "ok": true, "path": path, "format": "preset", "dryRun": session.dry_run,
                "channels": report.channels_applied, "bands": report.bands_applied,
                "crossoverBands": report.crossover_bands_applied,
                "crosspoints": report.crosspoints_applied,
                "missing": report.missing_channels, "skipped": report.skipped,
                "clean": report.is_clean(), "lines": lines,
            })
            .to_string(),
        );
    } else {
        say(false, "");
        for line in &lines {
            say(false, line);
        }
    }
    exit::OK
}

/// Apply a filter bank, band by band through the ordinary write path, so every
/// value gets the clamping, gating and readback of a typed command. Anything
/// the file asks for that this device cannot do is reported.
fn import_filters(
    target: &Target,
    doc: &FilterDocument,
    path: &str,
    flags: &[&str],
    json: bool,
) -> u8 {
    let mut session = match connect(target) {
        Ok(s) => s,
        Err(c) => return c,
    };
    session.dry_run = flags.contains(&"--dry-run");
    let (report, mapped, skipped) =
        apply_filters(&mut session, doc, flags.contains(&"--map-legacy"));
    let caps = session.capabilities();
    let legacy_hint = !flags.contains(&"--map-legacy")
        && skipped
            .iter()
            .any(|s| crate::legacy_channel(&s.to_ascii_lowercase(), caps).is_some());

    if json {
        say(
            false,
            &json!({
                "ok": true, "path": path, "format": "filters", "dryRun": session.dry_run,
                "bands": report.bands_applied, "crossoverBands": report.crossover_bands_applied,
                "preamps": report.preamps_applied, "refused": report.refused,
                "dropped": report.dropped, "mapped": mapped, "skipped": skipped,
            })
            .to_string(),
        );
        return exit::OK;
    }
    say(
        false,
        &format!(
            "{} {} bands{}{} from {path}",
            if session.dry_run {
                "Would apply"
            } else {
                "Applied"
            },
            report.bands_applied,
            match report.crossover_bands_applied {
                0 => String::new(),
                n => format!(", {n} crossover bands"),
            },
            match report.preamps_applied {
                0 => String::new(),
                n => format!(", {n} preamps"),
            },
        ),
    );
    if report.refused > 0 {
        say(false, &format!("Refused by the device: {}", report.refused));
    }
    if report.dropped > 0 {
        say(
            false,
            &format!("Past what this device has: {} entries", report.dropped),
        );
    }
    if !mapped.is_empty() {
        say(
            false,
            &format!("Mapped older channel names: {}", mapped.join(", ")),
        );
    }
    if !skipped.is_empty() {
        say(
            false,
            &format!("Skipped, no matching channel: {}", skipped.join(", ")),
        );
        if legacy_hint {
            say(
                false,
                "  These look like pre-2026 channel names. Re-run with --map-legacy \
                 to place them on the current channels.",
            );
        }
    }
    exit::OK
}

/// Place each section on a channel and apply it. Returns the report, the older
/// names that were mapped, and the sections with no matching channel.
fn apply_filters(
    session: &mut Session,
    doc: &FilterDocument,
    map_legacy: bool,
) -> (FilterReport, Vec<String>, Vec<String>) {
    let caps = session.capabilities().clone();
    let mut report = FilterReport::default();
    let mut mapped = Vec::new();
    let mut skipped = Vec::new();
    for section in &doc.channels {
        let bank = filterfile::ChannelBank {
            header: section.header.clone(),
            index: section.index,
            is_output: section.is_output,
            ..Default::default()
        };
        // A file written for a different device shape will name channels this
        // one does not have; say so rather than silently dropping them.
        let Some(channel) = resolve_channel(&bank, &caps, map_legacy) else {
            skipped.push(section.header.clone());
            continue;
        };
        if section.index.is_none()
            && !caps
                .channels
                .iter()
                .any(|c| c.name.eq_ignore_ascii_case(section.header.trim()))
        {
            mapped.push(format!(
                "{} → {}",
                section.header,
                caps.channels
                    .get(channel as usize)
                    .map(|c| c.name.as_str())
                    .unwrap_or("?")
            ));
        }
        preset_file::apply_filter_channel(session, channel, section, &mut report);
    }
    (report, mapped, skipped)
}

// ---------------------------------------------------------- control surfaces

/// `dspi cs export <file|->` and `dspi cs import <file|-> [--dry-run] [--save]`.
pub(crate) fn cmd_cs(target: &Target, flags: &[&str], json: bool) -> u8 {
    let args = positional(flags);
    match args.as_slice() {
        ["export", path] => cs_export(target, path, json),
        ["import", path] => cs_import(target, path, flags, json),
        _ => usage_error(
            "usage: dspi cs export <file|-> | dspi cs import <file|-> [--dry-run] [--save]",
        ),
    }
}

fn cs_failure(e: &CsFileError) -> u8 {
    eprintln!("dspi: {e}");
    match e {
        CsFileError::Unsupported => exit::USAGE,
        CsFileError::Device(_) => exit::TRANSPORT,
    }
}

fn cs_export(target: &Target, path: &str, json: bool) -> u8 {
    let mut session = match connect(target) {
        Ok(s) => s,
        Err(c) => return c,
    };
    let doc = match preset_file::capture_control_surfaces(&mut session) {
        Ok(d) => d,
        Err(e) => return cs_failure(&e),
    };
    let summary = cs_summary(&doc);
    finish_export(path, "control-surfaces", &doc.write(), summary, json)
}

/// What a document holds, for the report: the used records, not the slots.
fn cs_summary(doc: &ControlSurfaceDocument) -> (String, serde_json::Value) {
    let bindings = doc.bindings.iter().filter(|b| b.r#type != 0).count();
    let ir = doc.ir.iter().filter(|c| c.protocol != 0).count();
    let groups = doc.groups.iter().filter(|g| g.target_kind != 0).count();
    let macros = doc.macros.iter().filter(|m| !m.steps.is_empty()).count();
    (
        format!("{bindings} bindings, {ir} IR commands, {groups} groups, {macros} macros"),
        json!({ "bindings": bindings, "ir": ir, "groups": groups, "macros": macros }),
    )
}

fn cs_import(target: &Target, path: &str, flags: &[&str], json: bool) -> u8 {
    let text = match read_document(path) {
        Ok(t) => t,
        Err(e) => return usage_error(&e),
    };
    let doc = match preset_file::parse_control_surfaces(&text) {
        Ok(d) => d,
        Err(e) => return usage_error(&e.to_string()),
    };
    let mut session = match connect(target) {
        Ok(s) => s,
        Err(c) => return c,
    };
    session.dry_run = flags.contains(&"--dry-run");
    let want_save = flags.contains(&"--save") && !session.dry_run;
    run_cs_import(&mut session, &doc, want_save, json)
}

/// Apply, optionally store, report. A refused record or a failed link is a
/// non-zero exit, and nothing partial is stored.
fn run_cs_import(
    session: &mut Session,
    doc: &ControlSurfaceDocument,
    want_save: bool,
    json: bool,
) -> u8 {
    let report = match preset_file::apply_control_surfaces(session, doc) {
        Ok(r) => r,
        Err(e) => return cs_failure(&e),
    };
    let mut saved = false;
    let mut save_error = None;
    if want_save && report.is_clean() {
        match dspi_session::cs_model::apply(session, |s| s.save()) {
            Ok(()) => saved = true,
            Err(e) => save_error = Some(e.to_string()),
        }
    }
    print_cs_report(&report, session.dry_run, saved, save_error.as_deref(), json);
    if report.failed.is_some() || save_error.is_some() {
        exit::TRANSPORT
    } else if !report.refused.is_empty() {
        exit::REJECTED
    } else {
        exit::OK
    }
}

fn print_cs_report(
    report: &CsApplyReport,
    dry_run: bool,
    saved: bool,
    save_error: Option<&str>,
    json: bool,
) {
    let mut lines = report.lines(dry_run, saved);
    if let Some(e) = save_error {
        lines.push(format!("Could not save: {e}"));
    }
    if json {
        say(
            false,
            &json!({
                "ok": report.failed.is_none() && report.refused.is_empty() && save_error.is_none(),
                "dryRun": dry_run, "saved": saved,
                "bindings": report.bindings, "names": report.names, "ir": report.ir,
                "groups": report.groups, "macros": report.macros,
                "displayConfig": report.display_config, "pages": report.pages,
                "refused": report.refused, "beyond": report.beyond, "failed": report.failed,
                "lines": lines,
            })
            .to_string(),
        );
    } else {
        for line in &lines {
            say(false, line);
        }
    }
}

#[cfg(test)]
mod wp_tm10_tests {
    use super::*;
    use dspi_transport::FakeDevice;

    fn session_on(device: FakeDevice) -> Session {
        let mut t: Box<dyn dspi_transport::Transport> = Box::new(device);
        let caps = dspi_session::probe::probe(t.as_mut()).unwrap();
        Session::new(t, caps).unwrap()
    }

    fn demo() -> Session {
        session_on(FakeDevice::demo())
    }

    fn plain_demo() -> Session {
        session_on(FakeDevice::demo_without_surfaces())
    }

    const QT: &str = include_str!("../../../tests/fixtures/preset/qt-core-v32.dspipreset");
    const MAC: &str = include_str!("../../../tests/fixtures/preset/mac-swift-v32.dspipreset");

    #[test]
    fn wp_tm10_the_format_follows_the_flag_then_the_name() {
        use Format::*;
        for (path, chosen, json, want) in [
            ("x.json", None, false, Preset),
            ("x.dspipreset", None, false, Preset),
            ("x.txt", None, false, Filters),
            ("x.txt", None, true, Filters),
            ("-", None, false, Filters),
            ("-", None, true, Preset),
            ("x.json", Some(FiltersJson), false, FiltersJson),
            ("-", Some(Filters), true, Filters),
        ] {
            assert_eq!(export_format(path, chosen, json), want, "{path}");
        }
    }

    #[test]
    fn wp_tm10_format_is_taken_out_of_the_flags() {
        let (f, rest) = take_format(&["import", "--format", "preset", "x", "--json"]).unwrap();
        assert_eq!(f, Some(Format::Preset));
        assert_eq!(rest, vec!["import", "x", "--json"]);
        let (f, _) = take_format(&["import", "--format=filters-json", "x"]).unwrap();
        assert_eq!(f, Some(Format::FiltersJson));
        assert!(take_format(&["import", "--format"]).is_err());
        assert!(take_format(&["import", "--format", "yaml"]).is_err());
    }

    #[test]
    fn wp_tm10_an_import_goes_by_the_content() {
        assert_eq!(import_kind("x.txt", None, QT), DocumentKind::Preset);
        assert_eq!(import_kind("-", None, MAC), DocumentKind::Preset);
        assert_eq!(
            import_kind("x.json", None, "[USB 1]\n"),
            DocumentKind::FilterText
        );
        // A .dspipreset is one even when it is not, so the error is the real one.
        assert_eq!(
            import_kind("x.dspipreset", None, "[USB 1]\n"),
            DocumentKind::Preset
        );
        assert_eq!(
            import_kind("x", Some(Format::Filters), QT),
            DocumentKind::FilterText
        );
    }

    /// Every band of `doc` that names a channel on this device is what the
    /// device now holds, by that channel's own number.
    fn assert_bands(session: &mut Session, doc: &preset_file::PresetDocument) {
        let caps = session.capabilities().clone();
        let (placed, missing) = preset_file::place_channels(doc, caps.num_inputs, caps.num_outputs);
        assert!(missing.is_empty(), "{missing:?}");
        assert_eq!(placed.len(), caps.channels.len());
        for (r, block) in placed {
            let channel = r.unified(caps.num_inputs);
            assert_eq!(block.eq_channel, Some(channel as i32));
            for (b, want) in block.eq.iter().enumerate() {
                let got = session.read_band(channel, b as u8).unwrap();
                assert_eq!(
                    got.filter_type.to_raw() as i32,
                    want.r#type,
                    "{channel}.{b}"
                );
                assert_eq!(got.freq, want.freq_hz, "{channel}.{b}");
                assert_eq!(got.gain_db, want.gain, "{channel}.{b}");
                assert_eq!(got.bypass, want.bypass, "{channel}.{b}");
            }
        }
    }

    #[test]
    fn wp_tm10_the_mac_document_applies_and_comes_back_out() {
        let doc = preset_file::parse(MAC).unwrap();
        let mut s = plain_demo();
        let before = preset_file::capture(&mut s, None);
        assert_ne!(
            before.channels, doc.channels,
            "the fixture must move something"
        );

        let report = preset_file::apply(
            &mut s,
            &doc,
            preset_file::ApplyOptions {
                audio_processing: true,
                volume_levels: true,
                hardware_io: false,
            },
        );
        assert!(report.missing_channels.is_empty(), "{report:?}");
        assert_bands(&mut s, &doc);

        let out = preset_file::capture(&mut s, doc.meta.name.clone());
        for (a, b) in doc.channels.iter().zip(&out.channels) {
            assert_eq!(a.channel_id, b.channel_id);
            assert_eq!(a.input_index, b.input_index);
            assert_eq!(a.output_index, b.output_index);
            assert_eq!(a.name, b.name);
            assert_eq!(a.delay_ms, b.delay_ms, "{}", a.name);
            assert_eq!(a.eq, b.eq, "{}", a.name);
            assert_eq!(a.crossover, b.crossover, "{}", a.name);
            assert_eq!(a.gain_db, b.gain_db, "{}", a.name);
            assert_eq!(a.output_delay_ms, b.output_delay_ms, "{}", a.name);
        }
        // The fork's map survives the trip, through the extension channel.
        assert_eq!(out.crossfeed.pair_map, doc.crossfeed.pair_map);
        assert_eq!(out.crossfeed.enabled, doc.crossfeed.enabled);
        assert_eq!(
            out.crossfeed.output_pair_mask,
            doc.crossfeed.output_pair_mask
        );
        assert_eq!(out.loudness, doc.loudness);
        assert_eq!(out.global.input_preamps_db, doc.global.input_preamps_db);
        assert_eq!(out.global.user_volume_db, doc.global.user_volume_db);
        assert_eq!(out.matrix, doc.matrix);
    }

    #[test]
    fn wp_tm10_a_dry_run_does_not_touch_the_pair_map() {
        let doc = preset_file::parse(MAC).unwrap();
        let mut s = plain_demo();
        s.dry_run = true;
        preset_file::apply(&mut s, &doc, preset_file::ApplyOptions::default());
        s.dry_run = false;
        let out = preset_file::capture(&mut s, None);
        assert_eq!(out.crossfeed.pair_map, Some(vec![0, 0, 0, 0]));
    }

    /// The Terminal's export of the demo device and Qt's of the same capture
    /// say the same thing, key for key.
    #[test]
    fn wp_tm10_the_terminal_and_qt_export_the_same_document() {
        let mut s = plain_demo();
        let ours: serde_json::Value =
            serde_json::from_str(&preset_file::write(&preset_file::capture(&mut s, None))).unwrap();
        let qt: serde_json::Value = serde_json::from_str(QT).unwrap();
        let mut diffs = Vec::new();
        compare(&qt, &ours, "", &mut diffs);
        // What the two may fairly differ on: provenance, the app's own links,
        // and the documented gap below.
        diffs.retain(|d| {
            !d.starts_with(".meta.")
                && !d.starts_with(".global.inputPairLinked")
                && !KNOWN_DIFFERENCES.iter().any(|k| d.starts_with(k))
        });
        assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    }

    /// Hardware reads `FakeDevice` does not model, so `capture_io` has nothing
    /// to report on it, while Qt reads them from the bulk image: output pins and
    /// the I2S clock-pin mode stall. And the fake answers the DAC mute read that
    /// Qt's mirror marks as unsupported on this capture. A real device answers
    /// them all.
    const KNOWN_DIFFERENCES: &[&str] = &[
        ".io.outputPins",
        ".io.i2sBckPinSlave",
        ".io.i2sClockPinMode",
        ".io.dacHwMute",
    ];

    fn compare(a: &serde_json::Value, b: &serde_json::Value, at: &str, out: &mut Vec<String>) {
        use serde_json::Value as V;
        match (a, b) {
            (V::Object(x), V::Object(y)) => {
                for (k, v) in x {
                    match y.get(k) {
                        Some(w) => compare(v, w, &format!("{at}.{k}"), out),
                        None if v.is_null() => {}
                        None => out.push(format!("{at}.{k}: only in Qt's ({v})")),
                    }
                }
                for k in y.keys().filter(|k| !x.contains_key(*k)) {
                    out.push(format!("{at}.{k}: only in ours"));
                }
            }
            (V::Array(x), V::Array(y)) if x.len() == y.len() => {
                for (i, (v, w)) in x.iter().zip(y).enumerate() {
                    compare(v, w, &format!("{at}[{i}]"), out);
                }
            }
            (V::Number(x), V::Number(y)) => {
                let (x, y) = (x.as_f64().unwrap() as f32, y.as_f64().unwrap() as f32);
                if x != y {
                    out.push(format!("{at}: {x} vs {y}"));
                }
            }
            _ if a == b => {}
            _ => out.push(format!("{at}: {a} vs {b}")),
        }
    }

    // ------------------------------------------------ across the core (Qt's)

    fn normalise(text: &str) -> serde_json::Value {
        let mut v: serde_json::Value = serde_json::from_str(text).unwrap();
        v["meta"]["savedUtc"] = json!("");
        v
    }

    fn qt_core() -> std::sync::Arc<dspi_core::DspiCore> {
        let core = dspi_core::DspiCore::new();
        core.open_demo().unwrap();
        core
    }

    /// The committed Qt fixture is what the code Qt runs writes today, so it
    /// cannot go stale. `DSPI_TM10_REGEN=1` rewrites it.
    #[test]
    fn wp_tm10_the_qt_fixture_is_what_qts_core_writes() {
        let written = qt_core().config_export("Qt reference".into(), "1.0".into(), 0b0101);
        if std::env::var_os("DSPI_TM10_REGEN").is_some() {
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/preset/qt-core-v32.dspipreset"
            );
            std::fs::write(path, &written).unwrap();
            return;
        }
        assert_eq!(normalise(&written), normalise(QT));
    }

    /// A document from this client goes into Qt's importer and back out, and
    /// one from Qt's exporter goes into this client's.
    #[test]
    fn wp_tm10_documents_cross_between_the_terminal_and_qts_core() {
        // Mac document -> Terminal -> Qt.
        let mac = preset_file::parse(MAC).unwrap();
        let mut s = plain_demo();
        preset_file::apply(&mut s, &mac, preset_file::ApplyOptions::default());
        let text = preset_file::write(&preset_file::capture(&mut s, Some("x".into())));

        let core = qt_core();
        let reply: serde_json::Value = serde_json::from_str(&core.config_import(text, 0)).unwrap();
        assert_eq!(reply["ok"], true, "{reply}");

        // Qt -> Terminal: the same tuning, and the pair map.
        let back = preset_file::parse(&core.config_export("y".into(), "1.0".into(), 0)).unwrap();
        for (a, b) in mac.channels.iter().zip(&back.channels) {
            assert_eq!(a.eq, b.eq, "{}", a.name);
            assert_eq!(a.crossover, b.crossover, "{}", a.name);
        }
        assert_eq!(back.crossfeed.pair_map, mac.crossfeed.pair_map);
    }

    /// The Mac's document goes straight into Qt's importer too: this fixture
    /// and the Terminal agree with the same reader.
    #[test]
    fn wp_tm10_qts_importer_takes_the_mac_document() {
        let core = qt_core();
        let reply: serde_json::Value =
            serde_json::from_str(&core.config_import(MAC.into(), 0)).unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["links"], 1, "{reply}");
        let back = preset_file::parse(&core.config_export("y".into(), "1.0".into(), 0)).unwrap();
        let mac = preset_file::parse(MAC).unwrap();
        for (a, b) in mac.channels.iter().zip(&back.channels) {
            assert_eq!(a.eq, b.eq, "{}", a.name);
        }
        assert_eq!(back.crossfeed.pair_map, mac.crossfeed.pair_map);
    }

    #[test]
    fn wp_tm10_qts_document_applies_on_the_terminal() {
        let qt = preset_file::parse(QT).unwrap();
        assert_eq!(qt.crossfeed.pair_map, Some(vec![0, 0, 0, 0]));
        let mut s = plain_demo();
        let report = preset_file::apply(&mut s, &qt, preset_file::ApplyOptions::default());
        assert!(report.missing_channels.is_empty(), "{report:?}");
        assert_bands(&mut s, &qt);
    }

    // ---------------------------------------------------------------- filters

    #[test]
    fn wp_tm10_filter_json_keeps_crossovers_preamps_and_qp() {
        // A tuning with all three, on the device, exported.
        let mut s = plain_demo();
        let mac = preset_file::parse(MAC).unwrap();
        preset_file::apply(&mut s, &mac, preset_file::ApplyOptions::default());
        let mut doc = preset_file::capture_filters(&mut s);
        // A Linkwitz Transform, the one band that uses `qp`.
        doc.channels[0].peq[1] = preset_file::BandBlock {
            r#type: 11,
            freq_hz: 50.0,
            q: 0.75,
            gain: 100.0,
            qp: 0.5,
            bypass: false,
        };
        assert!(
            doc.channels
                .iter()
                .any(|c| c.crossover.iter().any(|b| b.r#type == 34))
        );
        assert!(doc.channels.iter().any(|c| c.preamp_db == Some(-3.0)));

        // Onto a clean device, through its JSON text.
        let text = doc.write();
        assert_eq!(preset_file::sniff(&text), DocumentKind::FilterJson);
        let parsed = preset_file::parse_filters(&text).unwrap();
        let mut clean = plain_demo();
        let (report, mapped, skipped) = apply_filters(&mut clean, &parsed, false);
        assert!(
            mapped.is_empty() && skipped.is_empty(),
            "{mapped:?} {skipped:?}"
        );
        assert_eq!(report.refused, 0);
        assert_eq!(report.preamps_applied, 8);
        assert_eq!(report.crossover_bands_applied, 9 * 4);
        assert_eq!(
            preset_file::capture_filters(&mut clean).channels,
            doc.channels
        );
    }

    #[test]
    fn wp_tm10_the_filter_text_keeps_crossovers_and_preamps_too() {
        let mut s = plain_demo();
        let mac = preset_file::parse(MAC).unwrap();
        preset_file::apply(&mut s, &mac, preset_file::ApplyOptions::default());
        let doc = preset_file::capture_filters(&mut s);
        let text = filterfile::write(&doc.to_text(), "now");

        let parsed = FilterDocument::from_text(&filterfile::parse(&text).unwrap());
        let mut clean = plain_demo();
        let (report, _, skipped) = apply_filters(&mut clean, &parsed, false);
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(report.refused, 0);
        let out = preset_file::capture_filters(&mut clean);
        for (a, b) in doc.channels.iter().zip(&out.channels) {
            assert_eq!(a.preamp_db, b.preamp_db, "{}", a.header);
            let used = |v: &[preset_file::BandBlock]| {
                v.iter()
                    .filter(|x| x.r#type != 0)
                    .map(|x| (x.r#type, x.freq_hz))
                    .collect::<Vec<_>>()
            };
            assert_eq!(used(&a.peq), used(&b.peq), "{}", a.header);
            assert_eq!(used(&a.crossover), used(&b.crossover), "{}", a.header);
        }
    }

    #[test]
    fn wp_tm10_the_shared_filter_files_apply() {
        for (name, text, map_legacy) in [
            (
                "dspi",
                include_str!("../../../tests/fixtures/DSPiFilters.txt"),
                true,
            ),
            (
                "macos",
                include_str!("../../../tests/fixtures/MacOSFilters.txt"),
                false,
            ),
        ] {
            let parsed = FilterDocument::from_text(&filterfile::parse(text).unwrap());
            let mut s = plain_demo();
            let (report, _, _) = apply_filters(&mut s, &parsed, map_legacy);
            assert!(report.bands_applied > 0, "{name}: {report:?}");
            assert_eq!(report.refused, 0, "{name}");
        }
    }

    // ------------------------------------------------------- control surfaces

    #[test]
    fn wp_tm10_cs_export_and_import_round_trip_on_the_demo_device() {
        let mut s = demo();
        let doc = preset_file::capture_control_surfaces(&mut s).unwrap();
        assert!(doc.bindings.iter().any(|b| b.r#type != 0));
        assert!(doc.macros.iter().any(|m| !m.steps.is_empty()));
        assert!(doc.groups.iter().any(|g| g.target_kind != 0));
        assert!(doc.display.is_some());

        // Nothing differs, so nothing is written.
        let same = preset_file::parse_control_surfaces(&doc.write()).unwrap();
        assert_eq!(same, doc);
        let r = preset_file::apply_control_surfaces(&mut s, &same).unwrap();
        assert_eq!(r, CsApplyReport::default());

        // A cleared device takes the whole document and reads back the same.
        let mut blank = doc.clone();
        blank
            .bindings
            .iter_mut()
            .for_each(|b| *b = Default::default());
        blank.bindings.iter_mut().for_each(|b| b.gpio1 = 0);
        blank.names.iter_mut().for_each(String::clear);
        blank.ir.iter_mut().for_each(|c| *c = Default::default());
        blank
            .groups
            .iter_mut()
            .for_each(|g| *g = Default::default());
        blank
            .macros
            .iter_mut()
            .for_each(|m| *m = Default::default());
        if let Some(d) = blank.display.as_mut() {
            d.config = Default::default();
            d.pages.iter_mut().for_each(|p| *p = Default::default());
        }
        let r = preset_file::apply_control_surfaces(&mut s, &blank).unwrap();
        assert!(r.is_clean(), "{r:?}");
        let cleared = preset_file::capture_control_surfaces(&mut s).unwrap();
        assert!(cleared.bindings.iter().all(|b| b.r#type == 0));

        let r = preset_file::apply_control_surfaces(&mut s, &doc).unwrap();
        assert!(r.is_clean(), "{r:?}");
        assert!(
            r.bindings > 0 && r.groups > 0 && r.macros > 0 && r.pages > 0,
            "{r:?}"
        );
        let mut back = preset_file::capture_control_surfaces(&mut s).unwrap();
        back.meta = doc.meta.clone();
        assert_eq!(back, doc);
    }

    #[test]
    fn wp_tm10_two_bindings_can_exchange_pins() {
        let mut s = demo();
        let doc = preset_file::capture_control_surfaces(&mut s).unwrap();
        let used: Vec<usize> = (0..doc.bindings.len())
            .filter(|&i| doc.bindings[i].r#type == 1)
            .collect();
        assert!(used.len() >= 2, "{used:?}");
        let (a, b) = (used[0], used[1]);
        let mut swapped = doc.clone();
        swapped.bindings[a].gpio0 = doc.bindings[b].gpio0;
        swapped.bindings[b].gpio0 = doc.bindings[a].gpio0;
        swapped.bindings[a].event = doc.bindings[b].event;
        swapped.bindings[b].event = doc.bindings[a].event;
        let r = preset_file::apply_control_surfaces(&mut s, &swapped).unwrap();
        assert!(r.is_clean(), "{r:?}");
        assert_eq!(r.bindings, 2);
    }

    #[test]
    fn wp_tm10_a_refused_binding_is_reported_and_the_rest_applies() {
        let mut s = demo();
        let doc = preset_file::capture_control_surfaces(&mut s).unwrap();
        let mut bad = doc.clone();
        let free = bad.bindings.iter().position(|b| b.r#type == 0).unwrap();
        // A pin and gesture another slot already holds.
        let holder = bad
            .bindings
            .iter()
            .find(|b| b.r#type == 1)
            .cloned()
            .unwrap();
        bad.bindings[free] = holder;
        bad.groups[0].name = "Renamed".into();
        let r = preset_file::apply_control_surfaces(&mut s, &bad).unwrap();
        assert_eq!(r.groups, 1);
        assert_eq!(r.refused.len(), 1, "{r:?}");
        assert!(r.refused[0].contains("already uses that pin"), "{r:?}");
        assert_eq!(run_cs_import(&mut s, &bad, true, true), exit::REJECTED);
    }

    #[test]
    fn wp_tm10_a_dry_run_writes_no_control_surfaces() {
        let mut s = demo();
        let doc = preset_file::capture_control_surfaces(&mut s).unwrap();
        let mut changed = doc.clone();
        changed.names[0] = "Changed".into();
        changed.groups[0].name = "Changed".into();
        s.dry_run = true;
        let r = preset_file::apply_control_surfaces(&mut s, &changed).unwrap();
        assert_eq!((r.names, r.groups), (1, 1));
        s.dry_run = false;
        let mut after = preset_file::capture_control_surfaces(&mut s).unwrap();
        after.meta = doc.meta.clone();
        assert_eq!(after, doc);
    }

    #[test]
    fn wp_tm10_more_than_the_device_has_is_reported() {
        let mut s = demo();
        let mut doc = preset_file::capture_control_surfaces(&mut s).unwrap();
        doc.bindings.push(preset_file::CsBindingBlock {
            r#type: 1,
            ..Default::default()
        });
        doc.macros[0].steps = vec![preset_file::CsStepBlock::default(); 200];
        let r = preset_file::apply_control_surfaces(&mut s, &doc).unwrap();
        assert_eq!(r.beyond.len(), 2, "{r:?}");
        assert_eq!(r.refused, Vec::<String>::new());
    }

    #[test]
    fn wp_tm10_a_device_without_surfaces_says_so() {
        let mut s = plain_demo();
        assert!(matches!(
            preset_file::capture_control_surfaces(&mut s),
            Err(CsFileError::Unsupported)
        ));
    }

    /// `dspi cs export` is a subset of Qt's `cs_snapshot`, key for key, so a
    /// Qt or Swift importer can hand it to `cs_apply` as it is.
    #[test]
    fn wp_tm10_the_cs_document_uses_the_snapshots_keys() {
        let mut s = demo();
        let doc: serde_json::Value = serde_json::from_str(
            &preset_file::capture_control_surfaces(&mut s)
                .unwrap()
                .write(),
        )
        .unwrap();

        let core = dspi_core::DspiCore::new();
        core.open_transport(Box::new(FakeDevice::demo()), "demo".into(), 0)
            .unwrap();
        core.cs_fetch().unwrap();
        let snap: serde_json::Value = serde_json::from_str(&core.cs_snapshot()).unwrap();

        assert_eq!(doc["bindings"], snap["bindings"]);
        assert_eq!(doc["ir"], snap["ir"]);
        assert_eq!(doc["groups"], snap["groups"]);
        assert_eq!(doc["macros"], snap["macros"]);
        assert_eq!(doc["display"]["config"], snap["display"]["config"]);
        assert_eq!(doc["display"]["pages"], snap["display"]["pages"]);
        assert_eq!(
            doc["names"].as_array().unwrap(),
            &snap["names"].as_array().unwrap()[..doc["names"].as_array().unwrap().len()]
        );
    }
}
