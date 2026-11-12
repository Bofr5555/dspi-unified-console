//! `uconsole diag`: write a diagnostics bundle (WP P0E, PRO1 E2).
//!
//! The bundle is the core's (`DspiCore::diagnostics_save`): versions, the
//! device's dump, stats and monitor lines, the event logs, the hub store with
//! its tokens and secrets removed, and the OS and USB facts. This command adds
//! what only the terminal knows: `uconsole doctor`'s report, and why the device
//! could not be opened when it could not. Nothing is sent anywhere; the zip is
//! written where the user says, and that is all.
//!
//! A device that cannot be opened does not stop the bundle: that is when one
//! is wanted most. The device is opened through the raw guard, as `flash`
//! does, so nothing in this command can write to it.

use std::path::PathBuf;

use dspi_cmd::COMMAND;
use dspi_core::{DiagFile, DiagOptions, DspiCore};
use dspi_tui::shell::connect::{Target, no_prompt, printable};

use crate::exit;

/// What `uconsole diag` was asked for.
#[derive(Debug, PartialEq)]
struct Request {
    /// Where the zip goes; the core's dated name in this folder when absent.
    out: Option<PathBuf>,
    /// The hub store's folder: `--store`, else `uconsole hub`'s default.
    store: Option<PathBuf>,
    /// A client's event log folder to include (`--events`).
    events: Option<PathBuf>,
}

fn parse(args: &[&str], default_store: Option<PathBuf>) -> Result<Request, String> {
    let mut out = None;
    let mut store = None;
    let mut events = None;
    let mut it = args.iter().copied();
    while let Some(arg) = it.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v)),
            _ => (arg, None),
        };
        let mut value = |what: &str| {
            inline
                .map(str::to_owned)
                .or_else(|| it.next().map(str::to_owned))
                .filter(|v| !v.is_empty())
                .ok_or_else(|| format!("{what} needs a folder"))
        };
        match flag {
            "--store" => store = Some(PathBuf::from(value("--store")?)),
            "--events" => events = Some(PathBuf::from(value("--events")?)),
            "--json" => {}
            f if f.starts_with('-') && f != "-" => {
                return Err(format!(
                    "`diag` takes --store, --events and --json, not `{f}`"
                ));
            }
            file if out.is_none() => out = Some(PathBuf::from(file)),
            extra => return Err(format!("`diag` takes one file name; `{extra}` is a second")),
        }
    }
    if out.as_deref() == Some(std::path::Path::new("-")) {
        return Err("a zip does not go to standard output: name a file".into());
    }
    Ok(Request {
        out,
        store: store.or(default_store),
        events,
    })
}

/// `uconsole diag [file] [--store DIR] [--events DIR] [--json]`.
pub fn run(target: &Target, flags: &[&str], json: bool) -> u8 {
    let args = flags.get(1..).unwrap_or_default();
    let req = match parse(args, crate::hub::default_store()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{COMMAND}: {e}");
            return exit::USAGE;
        }
    };

    let core = DspiCore::new();
    let mut files = Vec::new();
    if let Some(dir) = &req.events
        && let Err(e) = core.event_log_set_dir(dir.to_string_lossy().into_owned())
    {
        files.push(note(
            "events.txt",
            &format!("--events {}: {e}", dir.display()),
        ));
    }

    // Doctor first: it opens and lets go of each device itself.
    let (report, _) = crate::doctor::report(target);
    files.push(note("doctor.txt", &report));

    match target.open_raw(&no_prompt) {
        Ok(transport) => {
            let serial = transport.descriptor().serial.clone();
            if let Err(e) = core.open_transport(transport, serial, 0) {
                files.push(note(
                    "connect.txt",
                    &format!("opened, then the probe failed: {e}"),
                ));
            }
        }
        Err(e) => files.push(note("connect.txt", &format!("not opened: {e}"))),
    }

    let out = req
        .out
        .unwrap_or_else(|| PathBuf::from(core.diagnostics_file_name()));
    let options = DiagOptions {
        app: format!("{COMMAND} {}", env!("CARGO_PKG_VERSION")),
        hub_store_dir: req
            .store
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        files,
    };
    let saved = core.diagnostics_save(out.to_string_lossy().into_owned(), options);
    core.disconnect();
    match saved {
        Ok(names) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "path": out.to_string_lossy(), "files": names })
                );
            } else {
                println!(
                    "Wrote {} ({} files). Nothing was sent anywhere: attach it to a \
                     support request yourself if you want to.",
                    printable(&out.to_string_lossy()),
                    names.len()
                );
            }
            exit::OK
        }
        Err(e) => {
            eprintln!("{COMMAND}: {e}");
            exit::TRANSPORT
        }
    }
}

fn note(name: &str, text: &str) -> DiagFile {
    DiagFile {
        name: name.into(),
        text: format!("{text}\n"),
    }
}

#[cfg(test)]
mod tests_wp_p0e {
    use super::*;

    fn store() -> Option<PathBuf> {
        Some(PathBuf::from("/data/uconsole/hub"))
    }

    #[test]
    fn wp_p0e_parse_defaults_and_flags() {
        assert_eq!(
            parse(&[], store()).unwrap(),
            Request {
                out: None,
                store: store(),
                events: None
            }
        );
        let r = parse(
            &["b.zip", "--store", "/s", "--events=/e", "--json"],
            store(),
        )
        .unwrap();
        assert_eq!(r.out, Some(PathBuf::from("b.zip")));
        assert_eq!(r.store, Some(PathBuf::from("/s")));
        assert_eq!(r.events, Some(PathBuf::from("/e")));
    }

    #[test]
    fn wp_p0e_parse_refuses_what_it_cannot_do() {
        assert!(parse(&["a.zip", "b.zip"], None).is_err());
        assert!(parse(&["--store"], None).is_err());
        assert!(parse(&["--store="], None).is_err());
        assert!(parse(&["--upload"], None).is_err());
        assert!(parse(&["-"], None).is_err());
    }

    /// The whole command on the demo device, written where the test says.
    #[test]
    fn wp_p0e_diag_on_the_demo_writes_a_bundle_with_the_doctor_report() {
        let d = std::env::temp_dir()
            .join("wp-p0e-term")
            .join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let out = d.join("diag.zip");
        let out_s = out.to_string_lossy().into_owned();
        let empty_store = d.join("no-store").to_string_lossy().into_owned();
        let code = run(
            &Target::Demo,
            &["diag", &out_s, "--store", &empty_store],
            true,
        );
        assert_eq!(code, exit::OK);
        let zip = std::fs::read(&out).unwrap();
        let raw = String::from_utf8_lossy(&zip);
        for name in [
            "client/doctor.txt",
            "device/dump.json",
            "events/",
            "versions.json",
        ] {
            assert!(raw.contains(name), "{name}");
        }
        assert!(raw.contains("demo device"), "the doctor's demo check");
        assert!(!raw.contains("client/connect.txt"), "the demo opened");
        let _ = std::fs::remove_dir_all(&d);
    }
}
