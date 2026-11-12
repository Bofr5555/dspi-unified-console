//! C functions: the diagnostics bundle (WP P0E), over `dspi_core`'s
//! `diagnostics_save` and `diagnostics_file_name`.

use std::ffi::c_char;

use dspi_core::{DiagFile, DiagOptions, DspiError, Result};
use serde::Deserialize;

use super::{json_reply, optional_text, text};
use crate::FfiCore;
use crate::fns_device::arg;

/// The options' JSON (`dspi_diagnostics_save`).
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Options {
    app: String,
    hub_store_dir: String,
    files: Vec<File>,
}

#[derive(Deserialize)]
struct File {
    name: String,
    text: String,
}

fn options(json: Option<String>) -> Result<DiagOptions> {
    let o: Options = match json {
        Some(j) => serde_json::from_str(&j).map_err(|e| DspiError::InvalidArgument {
            msg: format!("the diagnostics options: {e}"),
        })?,
        None => Options::default(),
    };
    Ok(DiagOptions {
        app: o.app,
        hub_store_dir: o.hub_store_dir,
        files: o
            .files
            .into_iter()
            .map(|f| DiagFile {
                name: f.name,
                text: f.text,
            })
            .collect(),
    })
}

/// Write a diagnostics bundle (one zip: versions, the open device's dump,
/// stats and monitor lines, every event log, a hub store without its tokens
/// and secrets, OS and USB facts, the client's files) to `path`. Nothing is
/// sent anywhere. `options` is null or JSON: `{"app": "uConsole for DSPi 1.2.0",
/// "hubStoreDir": "<folder of hub-store.json>", "files": [{"name":
/// "app.log", "text": "..."}]}`, every key optional.
///
/// Reply (module docs): `{"ok": true, "files": <count>}`, or a failure
/// (`io` when the file cannot be written, `invalid` for a null `path` or bad
/// `options`). Writes on every call, whatever the buffer, so pass one of 1024
/// bytes rather than asking for the length first. Reads the device and
/// files: not on the UI thread.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_diagnostics_save(
    core: *mut FfiCore,
    path: *const c_char,
    options: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    let args = arg(path, "path").and_then(|p| {
        let o = optional_text(options, "options").and_then(self::options)?;
        Ok((p.to_owned(), o))
    });
    text(core, buf, len, |c| {
        json_reply(args.and_then(|(path, options)| {
            let names = c.diagnostics_save(path, options)?;
            Ok(serde_json::json!({ "files": names.len() }))
        }))
    })
}

/// A file name for a bundle saved now (`dspi-diagnostics-<UTC date and
/// time>Z.zip`), for a Save dialog. Returns the bytes needed, NUL included.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_diagnostics_file_name(
    core: *mut FfiCore,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    text(core, buf, len, |c| c.diagnostics_file_name())
}

#[cfg(test)]
mod tests_wp_p0e;
