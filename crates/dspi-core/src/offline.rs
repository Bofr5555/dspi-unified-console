//! Offline editing and templates (WP P1OF, PRO1 W7).
//!
//! An installer prepares a system at the desk: [`DspiCore::open_document`]
//! opens a `.dspipreset` document with no device, as the demo device seeded
//! from it ([`dspi_transport::FakeDevice::from_document`]): the platform and
//! channel counts its `meta` names, every setting it holds. Every screen then
//! edits it as it would a device, undo included, and
//! [`DspiCore::save_document`] writes it back. Nothing touches USB.
//!
//! A session on a document says so: [`DspiCore::offline_document`] names the
//! file, and the clients show "File: <name>" in place of a serial. The
//! connection's serial is [`OFFLINE_SERIAL`].
//!
//! Templates are documents with `meta.kind = "template"`
//! ([`DspiCore::save_as_template`], from a device or a document alike).
//! [`template_list`] lists the ones this build ships (`assets/templates/`,
//! compiled in, so every platform has them) and the `.dspipreset` files of
//! kind template in the user's folder; [`template_text`] gives one's text, for
//! [`DspiCore::open_document`] or an import.
//!
//! Every reply is JSON, as the other document calls' (`files.rs`).

use serde_json::{Value, json};

use dspi_session::preset_file::MetaKind;
use dspi_transport::fake::DocumentSeed;
use dspi_transport::{Capture, FakeDevice};

use crate::{DspiCore, DspiError, Result, files, state};

/// The serial of a connection opened from a document.
pub const OFFLINE_SERIAL: &str = "OFFLINE-DOCUMENT";

/// The id prefix of a template this build ships.
const BUILTIN: &str = "builtin:";

/// The templates this build ships: id (after [`BUILTIN`]) and text. The
/// K340's awaits the owner's approval (OD-P1h); its `meta.status` says so.
const TEMPLATES: &[(&str, &str)] = &[(
    "k340",
    include_str!("../../../assets/templates/k340.dspipreset"),
)];

/// The document an offline connection was opened from.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Document {
    /// The file's name as the client gave it ("" when it has none).
    pub(crate) file_name: String,
    /// `meta.name`, else the file's name without its extension.
    pub(crate) name: String,
    /// `meta.kind`.
    pub(crate) kind: MetaKind,
}

/// The `meta` key holding a document's status line (a shipped template
/// awaiting approval says so here).
const STATUS_KEY: &str = "status";

fn meta_str<'a>(doc: &'a Value, key: &str) -> &'a str {
    doc.get("meta")
        .and_then(|m| m.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn meta_mode(doc: &Value, key: &str, default: u8) -> u8 {
    doc.get("meta")
        .and_then(|m| m.get(key))
        .and_then(Value::as_u64)
        .and_then(|v| u8::try_from(v).ok())
        .unwrap_or(default)
}

/// `file_name` without a directory or its extension.
fn stem(file_name: &str) -> &str {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    match base.rsplit_once('.') {
        Some((s, _)) if !s.is_empty() => s,
        _ => base,
    }
}

/// The demo device a document describes: its platform and channel counts on
/// the demo's capture, then every setting it holds, as an import reads them.
/// Returns the seed and what reading the document reported.
fn seed_for(doc: &Value, name: String) -> Result<(DocumentSeed, Vec<String>)> {
    let shape = files::document_shape(doc);
    let (ins, outs) = (shape.num_input_channels, shape.num_output_channels);
    if ins == 0 || outs == 0 {
        return Err(DspiError::InvalidArgument {
            msg: "This file names no inputs or no outputs, so it describes no device.".into(),
        });
    }
    let mut capture = Capture::demo();
    // GET_PLATFORM: platform, major, packed version, outputs, ...
    if let Some(p) = capture.platform.get_mut(0) {
        *p = shape.platform_id;
    }
    if let Some(o) = capture.platform.get_mut(3) {
        *o = outs;
    }
    // The image's header: version, platform, channels, outputs, inputs.
    for (at, v) in [
        (1, shape.platform_id),
        (2, ins.saturating_add(outs)),
        (3, outs),
        (4, ins),
    ] {
        if let Some(b) = capture.bulk.get_mut(at) {
            *b = v;
        }
    }
    let mut base = state::default_state();
    state::apply_platform(&capture.platform, &mut base).map_err(transfer)?;
    state::decode_bulk(&capture.bulk, &mut base).map_err(transfer)?;
    let (built, notes) = files::document_onto(&base, doc);
    capture.bulk = state::encode_bulk(&built);
    let seed = DocumentSeed {
        capture,
        name,
        master_volume_mode: meta_mode(doc, "masterVolumeMode", 0),
        output_config_mode: meta_mode(doc, "outputConfigMode", 1),
    };
    Ok((seed, notes))
}

fn transfer(e: impl std::fmt::Display) -> DspiError {
    DspiError::Transfer {
        msg: format!("the document could not be laid out as a device: {e}"),
    }
}

fn platform_name(id: u8) -> String {
    dspi_proto::Platform::from_id(id).name()
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// Open the `.dspipreset` document `text` with no device, as the demo
    /// device seeded from it (PRO1 W7): the platform and channel counts its
    /// `meta` names (else the channels it lists), every setting it holds read
    /// as an import reads them, the rest the demo's. `file_name` is what the
    /// client shows ("File: <name>"); `meta.name`, else the file's name, names
    /// the running preset and the device. Replaces any open device; nothing
    /// touches USB. The settings are read (`fetch_all`) and are the baseline
    /// for unsaved changes; the undo history starts empty.
    ///
    /// Returns JSON `{ok: true, fileName, name, kind, platform, inputs,
    /// outputs, notes}`, `notes` what reading the document reported (values
    /// left alone or clamped, channels this shape lacks).
    ///
    /// Errors: [`DspiError::InvalidArgument`] for text that is not a document
    /// this build reads (the reason as a sentence) or one naming no inputs or
    /// outputs; [`DspiError::Transfer`] if the device could not be laid out
    /// or read. On an error nothing is open.
    pub fn open_document(&self, text: String, file_name: String) -> Result<String> {
        let doc = files::parse_document(&text).map_err(|msg| DspiError::InvalidArgument { msg })?;
        let name = match meta_str(&doc, "name") {
            "" => stem(&file_name).to_string(),
            n => n.to_string(),
        };
        let kind = files::doc_kind(&doc);
        let (seed, notes) = seed_for(&doc, name.clone())?;
        let dev = FakeDevice::from_document(seed).with_serial(OFFLINE_SERIAL);
        self.attach_probed(Box::new(dev), OFFLINE_SERIAL.into(), 0)?;
        {
            let mut inner = self.lock();
            if let Some(conn) = inner.conn.as_mut()
                && conn.serial == OFFLINE_SERIAL
            {
                conn.document = Some(Document {
                    file_name: file_name.clone(),
                    name: name.clone(),
                    kind,
                });
            }
        }
        if let Err(e) = self.fetch_all() {
            self.disconnect();
            return Err(e);
        }
        self.capture_baseline();
        let s = self.get_state();
        Ok(json!({
            "ok": true,
            "fileName": file_name,
            "name": name,
            "kind": kind.word(),
            "platform": platform_name(s.platform_id),
            "inputs": s.num_input_channels,
            "outputs": s.num_output_channels,
            "notes": notes,
        })
        .to_string())
    }

    /// Whether the open connection is a document ([`DspiCore::open_document`]),
    /// as JSON: `{offline: true, fileName, name, kind, platform}`, or
    /// `{offline: false}` for a device or none. No I/O.
    pub fn offline_document(&self) -> String {
        let inner = self.lock();
        match inner.conn.as_ref().and_then(|c| c.document.as_ref()) {
            Some(d) => json!({
                "offline": true,
                "fileName": d.file_name,
                "name": d.name,
                "kind": d.kind.word(),
                "platform": platform_name(inner.state.platform_id),
            })
            .to_string(),
            None => json!({ "offline": false }).to_string(),
        }
    }

    /// The open document as it now stands, a `.dspipreset` document of the
    /// kind it was opened as, under its name (`config_export`'s rules;
    /// `links` as there). The edits are then the baseline: nothing is unsaved.
    /// The client writes the text to the file.
    ///
    /// Errors: [`DspiError::InvalidArgument`] when the connection is not a
    /// document (export the configuration instead); otherwise
    /// `config_export`'s.
    pub fn save_document(&self, app_version: String, links: u8) -> Result<String> {
        let text = {
            let mut inner = self.lock();
            let doc = inner
                .conn
                .as_ref()
                .and_then(|c| c.document.clone())
                .ok_or_else(|| DspiError::InvalidArgument {
                    msg: "No document is open; export the configuration instead.".into(),
                })?;
            let (state, map) = files::current_document(&mut inner)?;
            files::export_document(&state, &doc.name, &app_version, links, map, doc.kind)
        };
        self.capture_baseline();
        Ok(text)
    }

    /// The open device or document as a template: a `.dspipreset` document
    /// with `meta.kind = "template"` named `name` (`config_export`'s rules;
    /// `links` as there). The client writes it to the user's template
    /// folder, where [`template_list`] finds it. Errors as `config_export`'s.
    pub fn save_as_template(&self, name: String, app_version: String, links: u8) -> Result<String> {
        self.ensure_probed();
        let mut inner = self.lock();
        let (state, map) = files::current_document(&mut inner)?;
        Ok(files::export_document(
            &state,
            &name,
            &app_version,
            links,
            map,
            MetaKind::Template,
        ))
    }
}

/// One listed template, from its text; `None` for a text that is not a
/// document this build reads or not a template.
fn describe(id: &str, text: &str, builtin: bool) -> Option<Value> {
    let doc = files::parse_document(text).ok()?;
    if files::doc_kind(&doc) != MetaKind::Template {
        return None;
    }
    let shape = files::document_shape(&doc);
    let name = match meta_str(&doc, "name") {
        "" => stem(id.trim_start_matches(BUILTIN)).to_string(),
        n => n.to_string(),
    };
    Some(json!({
        "id": id,
        "name": name,
        "builtin": builtin,
        "platform": platform_name(shape.platform_id),
        "inputs": shape.num_input_channels,
        "outputs": shape.num_output_channels,
        "savedUtc": meta_str(&doc, "savedUtc"),
        "status": meta_str(&doc, STATUS_KEY),
    }))
}

/// A user template's file name: a plain `.dspipreset` name, no directory.
/// (No user folder on the web.)
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn plain_file_name(id: &str) -> bool {
    !id.is_empty()
        && !id.contains(['/', '\\', '\0'])
        && id != "."
        && id != ".."
        && id.ends_with(".dspipreset")
}

/// The text of the user template `id` in `user_dir`, if it is one this build
/// may read: a plain file name, under [`files::MAX_FILE_BYTES`].
#[cfg(not(target_arch = "wasm32"))]
fn read_user(user_dir: &str, id: &str) -> Result<String> {
    if user_dir.is_empty() || !plain_file_name(id) {
        return Err(DspiError::InvalidArgument {
            msg: format!("There is no template \"{id}\"."),
        });
    }
    let path = std::path::Path::new(user_dir).join(id);
    let io = |e: std::io::Error| DspiError::Io {
        msg: format!("{}: {e}", path.display()),
    };
    let len = std::fs::metadata(&path).map_err(io)?.len();
    if usize::try_from(len).map_or(true, |l| l > files::MAX_FILE_BYTES) {
        return Err(DspiError::InvalidArgument {
            msg: "This file is too large to be a DSPi file (the limit is 4 MB).".into(),
        });
    }
    std::fs::read_to_string(&path).map_err(io)
}

/// No file system on the web: a user folder holds nothing.
#[cfg(target_arch = "wasm32")]
fn read_user(_user_dir: &str, id: &str) -> Result<String> {
    Err(DspiError::InvalidArgument {
        msg: format!("There is no template \"{id}\"."),
    })
}

/// The `.dspipreset` file names in `user_dir`, sorted; none for an empty or
/// unreadable folder.
#[cfg(not(target_arch = "wasm32"))]
fn user_files(user_dir: &str) -> Vec<String> {
    if user_dir.is_empty() {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(user_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| plain_file_name(n))
        .collect();
    names.sort();
    names
}

#[cfg(target_arch = "wasm32")]
fn user_files(_user_dir: &str) -> Vec<String> {
    Vec::new()
}

/// The templates there are, as a JSON array: the ones this build ships, then
/// the `.dspipreset` files of kind template in `user_dir` ("" for none; none
/// on the web), each `{id, name, builtin, platform, inputs, outputs,
/// savedUtc, status}`. `id` is what [`template_text`] takes: `builtin:<name>`
/// or the file's name. `status` is the template's own status line (a shipped
/// template awaiting approval says so), else "". A file that cannot be read,
/// is not a document or not a template is left out; listing never fails.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn template_list(user_dir: String) -> String {
    let mut list: Vec<Value> = TEMPLATES
        .iter()
        .filter_map(|(id, text)| describe(&format!("{BUILTIN}{id}"), text, true))
        .collect();
    for file in user_files(&user_dir) {
        if let Ok(text) = read_user(&user_dir, &file)
            && let Some(entry) = describe(&file, &text, false)
        {
            list.push(entry);
        }
    }
    Value::Array(list).to_string()
}

/// The text of template `id` from [`template_list`]: a shipped one, or a file
/// in `user_dir`, for [`DspiCore::open_document`] or an import.
///
/// Errors: [`DspiError::InvalidArgument`] for an id that is no template (a
/// user id must be a plain `.dspipreset` file name in `user_dir`) or a file
/// over 4 MB; [`DspiError::Io`] when the file cannot be read.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn template_text(id: String, user_dir: String) -> Result<String> {
    if let Some(name) = id.strip_prefix(BUILTIN) {
        return TEMPLATES
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, text)| (*text).to_string())
            .ok_or_else(|| DspiError::InvalidArgument {
                msg: format!("There is no template \"{id}\"."),
            });
    }
    read_user(&user_dir, &id)
}

#[cfg(test)]
mod tests_wp_p1of;
