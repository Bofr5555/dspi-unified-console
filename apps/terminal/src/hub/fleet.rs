//! -- WP P3TM -- Groups, firmware images and the rollout at `uconsole hub`'s prompt, and
//! `uconsole hub firmware add|list` with the hub stopped (PLAN-P3 P3TM, `hub-api.md`
//! sections 16, 19 and 21).
//!
//! The prompt is the person at the hub's machine. Each command goes to the hub's services
//! as a principal named `Hub console` ([`super::PROMPT_PRINCIPAL`]), opened for that
//! command and closed after, through `HubHandle::open_console_principal`: nothing it sends
//! crosses a network, so the rollout's TLS-only family serves it. Reads go as a viewer;
//! `group set|delete`, `firmware add` and `rollout` as an admin (admin is given only on
//! the hub's machine, and this is it). Every rule of the services applies as to a
//! console: the rollout's checks, the firmware guard, the image store's checks.

use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use data_encoding::BASE64;
use dspi_hub::bus::{BusMessage, Subscription};
use dspi_hub::rollout::MAX_CHUNK_CHARS;
use dspi_hub::rollout::images::{self, MAX_IMAGE_BYTES};
use dspi_hub::{HubHandle, Role, Session};
use serde_json::{Value, json};

use super::{Failure, PROMPT_PRINCIPAL, RunOptions, Running, field, lock_store, unix};
use dspi_tui::shell::connect::printable;

/// Raw image bytes in one `fw.image.put` chunk: what [`MAX_CHUNK_CHARS`] base64
/// characters hold (36 KiB).
const CHUNK_BYTES: usize = MAX_CHUNK_CHARS / 4 * 3;
/// Rollout endings held for the prompt between two looks.
const ENDS_HELD: usize = 64;

/// A service's refusal: its `code` and the line to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Refused {
    pub code: String,
    pub line: String,
}

/// Print the prompt's lines.
pub(super) fn say_lines(lines: &[String]) {
    for l in lines {
        println!("{l}");
    }
}

/// A refusal as the prompt shows it.
fn refused_line(r: &Refused, json: bool) -> String {
    if json {
        json!({ "event": "error", "error": r.line, "code": r.code }).to_string()
    } else {
        r.line.clone()
    }
}

fn usage_line(m: &str, json: bool) -> Vec<String> {
    vec![refused_line(
        &Refused {
            code: "usage".into(),
            line: m.to_owned(),
        },
        json,
    )]
}

/// A session of the person at the hub's machine, as `role`: closed when dropped.
struct Console<'a> {
    hub: &'a HubHandle,
    session: Arc<Session>,
    next_id: u64,
}

impl<'a> Console<'a> {
    fn open(hub: &'a HubHandle, role: Role) -> Result<Self, Refused> {
        let session = hub
            .open_console_principal(role, PROMPT_PRINCIPAL)
            .map_err(|e| Refused {
                code: "internal".into(),
                line: format!("the hub: {e}"),
            })?;
        Ok(Self {
            hub,
            session,
            next_id: 1,
        })
    }

    /// The service's `ok` answer to `t` with `fields`, or its refusal.
    fn ask(&mut self, t: &str, fields: Value) -> Result<Value, Refused> {
        let mut msg = json!({ "t": t, "id": self.next_id });
        self.next_id = self.next_id.saturating_add(1);
        if let (Some(m), Value::Object(f)) = (msg.as_object_mut(), fields) {
            m.extend(f);
        }
        let Some(reply) = self.hub.service_request(&self.session, &msg) else {
            let line = if t.starts_with("fw.") {
                "the firmware rollout is off on this hub: start it with --rollout".to_owned()
            } else if t.starts_with("group.") {
                "groups are off on this hub (--no-groups)".to_owned()
            } else {
                format!("{t}: that service is off on this hub")
            };
            return Err(Refused {
                code: "unknown_type".into(),
                line,
            });
        };
        let v: Value = serde_json::from_str(&reply).map_err(|e| Refused {
            code: "internal".into(),
            line: format!("{t}: {e}"),
        })?;
        if v.get("t").and_then(Value::as_str) == Some("ok") {
            Ok(v)
        } else {
            Err(Refused {
                code: field(&v, "code"),
                line: format!("{t}: {} ({})", field(&v, "msg"), field(&v, "code")),
            })
        }
    }
}

impl Drop for Console<'_> {
    fn drop(&mut self) {
        self.hub.close_session(self.session.id());
    }
}

/// One answer asked as `role`.
fn ask(running: &Running, role: Role, t: &str, fields: Value) -> Result<Value, Refused> {
    Console::open(&running.handle, role)?.ask(t, fields)
}

/// The prompt's P3TM commands; `None` for a line that is none of them.
pub(super) fn prompt(running: &Running, words: &[&str], json: bool) -> Option<Vec<String>> {
    let lines = match words {
        ["groups"] => groups(running, json),
        ["group", "set", rest @ ..] => group_set(running, rest, json),
        ["group", "delete", id] => group_delete(running, id, json),
        ["group", ..] => usage_line(
            "group set [<id>] <name> <serial>... | group delete <id>; `groups` lists them",
            json,
        ),
        ["firmware", "list"] => firmware_list(running, json),
        ["firmware", "add", file] => firmware_add(running, Path::new(file), json),
        ["firmware", ..] => usage_line("firmware list | firmware add <file.uf2>", json),
        ["rollout", "status"] => rollout_status(running, None, json),
        ["rollout", "status", id] => rollout_status(running, Some(id), json),
        ["rollout", "cancel", id] => rollout_cancel(running, id, json),
        ["rollout", target, image, flags @ ..]
            if flags.iter().all(|f| *f == "--while-live")
                && !matches!(*target, "status" | "cancel") =>
        {
            rollout_start(running, target, image, !flags.is_empty(), json)
        }
        ["rollout", ..] => usage_line(
            "rollout <serial|group:id> <image> [--while-live] | rollout status [<id>] | \
             rollout cancel <id>",
            json,
        ),
        _ => return None,
    };
    Some(lines)
}

// ── Groups ──────────────────────────────────────────────────────────────────

fn group_line(g: &Value) -> String {
    let members: Vec<String> = g
        .get("members")
        .and_then(Value::as_array)
        .map(|m| m.iter().filter_map(Value::as_str).map(printable).collect())
        .unwrap_or_default();
    let note = field(g, "note");
    format!(
        "{}  {}  ({} member{}){}{}",
        field(g, "id"),
        field(g, "name"),
        members.len(),
        if members.len() == 1 { "" } else { "s" },
        if members.is_empty() {
            String::new()
        } else {
            format!(": {}", members.join(", "))
        },
        if note.is_empty() {
            String::new()
        } else {
            format!("  ({note})")
        },
    )
}

fn groups(running: &Running, json: bool) -> Vec<String> {
    let v = match ask(running, Role::Viewer, "group.list", json!({})) {
        Ok(v) => v,
        Err(r) => return vec![refused_line(&r, json)],
    };
    let list = v
        .get("groups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if json {
        return vec![json!({ "event": "groups", "groups": list }).to_string()];
    }
    if list.is_empty() {
        return vec!["no groups".into()];
    }
    list.iter().map(group_line).collect()
}

/// Whether `word` is a group's id as the hub makes them (`g1`, `g2`, ...).
fn is_group_id(word: &str) -> bool {
    word.strip_prefix('g')
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

fn group_set(running: &Running, words: &[&str], json: bool) -> Vec<String> {
    let (id, rest) = match words {
        [first, rest @ ..] if is_group_id(first) => (Some(*first), rest),
        _ => (None, words),
    };
    let Some((name, members)) = rest.split_first() else {
        return usage_line("group set [<id>] <name> <serial>...", json);
    };
    let mut group = json!({ "name": name, "members": members });
    if let (Some(id), Some(g)) = (id, group.as_object_mut()) {
        g.insert("id".into(), json!(id));
    }
    match ask(running, Role::Admin, "group.set", json!({ "group": group })) {
        Ok(v) if json => vec![
            json!({
                "event": "group",
                "group": v.get("group"),
                "created": v.get("created"),
            })
            .to_string(),
        ],
        Ok(v) => {
            let created = v.get("created").and_then(Value::as_bool) == Some(true);
            let g = v.get("group").cloned().unwrap_or(Value::Null);
            vec![format!(
                "group {}: {}",
                if created { "created" } else { "replaced" },
                group_line(&g)
            )]
        }
        Err(r) => vec![refused_line(&r, json)],
    }
}

fn group_delete(running: &Running, id: &str, json: bool) -> Vec<String> {
    match ask(running, Role::Admin, "group.delete", json!({ "group": id })) {
        Ok(v) if json => {
            vec![json!({ "event": "groupDeleted", "group": v.get("group") }).to_string()]
        }
        Ok(v) => vec![format!("group {} deleted", field(&v, "group"))],
        Err(r) => vec![refused_line(&r, json)],
    }
}

// ── Firmware images ─────────────────────────────────────────────────────────

/// One image as the prompt shows it, its label first among its facts.
pub(super) fn image_line(i: &Value) -> String {
    let verified = i.get("verified").and_then(Value::as_bool) == Some(true);
    let stamp = match i.get("stamp_mv").and_then(Value::as_u64) {
        Some(mv) => format!("{}.{:02} V", mv / 1000, mv % 1000 / 10),
        None => "unstamped".to_owned(),
    };
    let version = field(i, "firmware_version");
    let sha = field(i, "sha256");
    format!(
        "{}  {}  {}  fw {}  {}  {} bytes  sha256 {}  added by {}",
        field(i, "name"),
        if verified { "verified" } else { "NOT VERIFIED" },
        field(i, "platform"),
        if version.is_empty() { "?" } else { &version },
        stamp,
        field(i, "size"),
        sha.get(..16).unwrap_or(&sha),
        field(i, "added_by"),
    )
}

fn firmware_list(running: &Running, json: bool) -> Vec<String> {
    match ask(running, Role::Viewer, "fw.image.list", json!({})) {
        Ok(v) => images_lines(v.get("images"), json),
        Err(r) => vec![refused_line(&r, json)],
    }
}

fn images_lines(images: Option<&Value>, json: bool) -> Vec<String> {
    let list = images
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if json {
        return vec![json!({ "event": "firmwareImages", "images": list }).to_string()];
    }
    if list.is_empty() {
        return vec!["no firmware images stored".into()];
    }
    list.iter().map(image_line).collect()
}

/// What `firmware add` said: the image, and whether it was new.
fn added_line(image: &Value, created: bool, json: bool) -> String {
    if json {
        json!({ "event": "firmwareAdded", "image": image, "created": created }).to_string()
    } else if created {
        format!("added {}", image_line(image))
    } else {
        format!("already stored: {}", image_line(image))
    }
}

/// A file to put into the store: its name there (the file's own) and its bytes, read
/// whole up to the store's limit.
fn read_image(file: &Path) -> Result<(String, Vec<u8>), String> {
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| images::valid_name(n))
        .ok_or_else(|| {
            format!(
                "{}: the image's name in the store is the file's, which must be 1 to 64 \
                 characters of A-Z a-z 0-9 . - _ + ending in .uf2",
                printable(&file.display().to_string())
            )
        })?
        .to_owned();
    let shown = || printable(&file.display().to_string());
    let f = std::fs::File::open(file).map_err(|e| format!("{}: {e}", shown()))?;
    let limit = u64::try_from(MAX_IMAGE_BYTES).unwrap_or(u64::MAX);
    let mut bytes = Vec::new();
    f.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", shown()))?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(format!(
            "{}: an image is at most {MAX_IMAGE_BYTES} bytes",
            shown()
        ));
    }
    Ok((name, bytes))
}

/// `firmware add FILE` at the prompt: the upload a console makes (`fw.image.put` in
/// chunks), as an admin, so the image store's checks are the same and its `added_by`
/// is `Hub console`.
fn firmware_add(running: &Running, file: &Path, json: bool) -> Vec<String> {
    let refuse = |line: String| {
        vec![refused_line(
            &Refused {
                code: "bad_request".into(),
                line,
            },
            json,
        )]
    };
    let (name, bytes) = match read_image(file) {
        Ok(x) => x,
        Err(m) => return refuse(m),
    };
    let mut console = match Console::open(&running.handle, Role::Admin) {
        Ok(c) => c,
        Err(r) => return vec![refused_line(&r, json)],
    };
    let chunks: Vec<&[u8]> = if bytes.is_empty() {
        vec![&[][..]]
    } else {
        bytes.chunks(CHUNK_BYTES).collect()
    };
    let last = chunks.len().saturating_sub(1);
    for (seq, chunk) in chunks.iter().enumerate() {
        let put = json!({
            "name": name,
            "seq": seq,
            "data": BASE64.encode(chunk),
            "last": seq == last,
        });
        match console.ask("fw.image.put", put) {
            Ok(v) if seq == last => {
                let created = v.get("created").and_then(Value::as_bool) == Some(true);
                return vec![added_line(
                    v.get("image").unwrap_or(&Value::Null),
                    created,
                    json,
                )];
            }
            Ok(_) => {}
            Err(r) => return vec![refused_line(&r, json)],
        }
    }
    refuse("the upload ended without an answer".into())
}

/// The data folder of a hub stopped: `--data`, else the store (as a running hub's).
fn data_dir<'a>(store: &'a Path, data: Option<&'a Path>) -> &'a Path {
    data.unwrap_or(store)
}

fn now_s() -> u64 {
    unix(SystemTime::now())
}

/// `uconsole hub firmware add FILE`, the hub stopped: the image store's own way in from
/// the hub's machine (`images::add_file`), with the upload's checks.
pub(super) fn offline_firmware_add(
    store: &Path,
    data: Option<&Path>,
    file: &Path,
    json: bool,
) -> Result<(), Failure> {
    let (name, bytes) = read_image(file).map_err(Failure::Usage)?;
    let _lock = lock_store(store)?;
    let added = images::add_file(data_dir(store, data), &name, &bytes, now_s()).map_err(|e| {
        let line = format!("{} ({})", e.msg, e.code);
        if e.code == "internal" {
            Failure::Failed(line)
        } else {
            Failure::Usage(line)
        }
    })?;
    println!(
        "{}",
        added_line(&added.image.to_json(), added.created, json)
    );
    Ok(())
}

/// `uconsole hub firmware list`, the hub stopped.
pub(super) fn offline_firmware_list(
    store: &Path,
    data: Option<&Path>,
    json: bool,
) -> Result<(), Failure> {
    let _lock = lock_store(store)?;
    let now = now_s();
    let list: Vec<Value> = images::ImageStore::open(data_dir(store, data), now)
        .list(now)
        .iter()
        .map(images::ImageInfo::to_json)
        .collect();
    say_lines(&images_lines(Some(&Value::Array(list)), json));
    Ok(())
}

// ── The rollout ─────────────────────────────────────────────────────────────

/// A record's lines: the rollout, then each device with where it is.
fn record_lines(r: &Value) -> Vec<String> {
    let image = r.get("image").cloned().unwrap_or(Value::Null);
    let ended = field(r, "ended_at");
    let mut out = vec![format!(
        "{}  {}  {}  image {} (fw {})  by {}  started {}{}",
        field(r, "id"),
        field(r, "state"),
        field(r, "target"),
        field(&image, "name"),
        field(&image, "firmware_version"),
        field(r, "by"),
        field(r, "started_at"),
        if ended.is_empty() {
            String::new()
        } else {
            format!(", ended {ended}")
        },
    )];
    let devices = r
        .get("devices")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for d in &devices {
        let failed = d.get("failed").filter(|f| f.is_object());
        let text = match failed {
            Some(f) => {
                let code = field(f, "code");
                format!(
                    "{}{}",
                    field(f, "text"),
                    if code.is_empty() {
                        String::new()
                    } else {
                        format!(" ({code})")
                    }
                )
            }
            None => d
                .get("steps")
                .and_then(Value::as_array)
                .and_then(|s| s.last())
                .map(|s| field(s, "text"))
                .unwrap_or_default(),
        };
        let settings = d
            .get("after")
            .map(|a| field(a, "settings"))
            .filter(|s| !s.is_empty())
            .map(|s| format!("; settings {s}"))
            .unwrap_or_default();
        out.push(format!(
            "    {}  {}  at {}: {text}{settings}",
            field(d, "serial"),
            field(d, "state"),
            field(d, "step"),
        ));
    }
    out
}

fn rollout_start(
    running: &Running,
    target: &str,
    image: &str,
    while_live: bool,
    json: bool,
) -> Vec<String> {
    let mut fields = json!({ "image": image, "while_live": while_live });
    if let Some(m) = fields.as_object_mut() {
        if target.starts_with(dspi_hub::groups::TARGET_PREFIX) {
            m.insert("group".into(), json!(target));
        } else {
            m.insert("serial".into(), json!(target));
        }
    }
    match ask(running, Role::Admin, "fw.rollout.start", fields) {
        Ok(v) => {
            let r = v.get("rollout").cloned().unwrap_or(Value::Null);
            if json {
                return vec![json!({ "event": "rolloutStarted", "rollout": r }).to_string()];
            }
            let mut out = vec![format!(
                "rollout {} started; it runs on, and each device's end is shown here \
                 (`rollout status {}` for the record)",
                field(&r, "id"),
                field(&r, "id"),
            )];
            out.extend(record_lines(&r));
            out
        }
        Err(mut r) => {
            if r.code == "confirmation_required" {
                r.line.push_str(
                    ". Add --while-live to update it unmuted: its audio stops for about 30 s",
                );
            }
            vec![refused_line(&r, json)]
        }
    }
}

fn rollout_status(running: &Running, id: Option<&str>, json: bool) -> Vec<String> {
    let fields = id.map_or_else(|| json!({}), |id| json!({ "rollout": id }));
    let v = match ask(running, Role::Viewer, "fw.rollout.status", fields) {
        Ok(v) => v,
        Err(r) => return vec![refused_line(&r, json)],
    };
    let list = v
        .get("rollouts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if json {
        return vec![json!({ "event": "rollouts", "rollouts": list }).to_string()];
    }
    if list.is_empty() {
        return vec!["no rollouts".into()];
    }
    list.iter().flat_map(record_lines).collect()
}

fn rollout_cancel(running: &Running, id: &str, json: bool) -> Vec<String> {
    match ask(
        running,
        Role::Admin,
        "fw.rollout.cancel",
        json!({ "rollout": id }),
    ) {
        Ok(v) => {
            let r = v.get("rollout").cloned().unwrap_or(Value::Null);
            if json {
                vec![json!({ "event": "rolloutCancelled", "rollout": r }).to_string()]
            } else {
                record_lines(&r)
            }
        }
        Err(r) => vec![refused_line(&r, json)],
    }
}

/// What the prompt hears of the rollout: each device's end.
fn rollout_ended(m: &BusMessage) -> bool {
    matches!(m, BusMessage::RolloutEnded { .. })
}

/// Listen for each device's end of a rollout, from now on.
pub(super) fn watch_rollouts(hub: &HubHandle) -> Subscription {
    hub.bus()
        .subscribe_filtered("uconsole hub prompt", ENDS_HELD, rollout_ended)
}

/// The rollout endings since the last look, one line each.
pub(super) fn rollout_ends(running: &Running, json: bool) -> Vec<String> {
    let Some(sub) = &running.rollout_ends else {
        return Vec::new();
    };
    let mut out = Vec::new();
    while let Some(m) = sub.try_recv() {
        let BusMessage::RolloutEnded {
            serial,
            ok,
            step,
            text,
            firmware_version,
        } = &*m
        else {
            continue;
        };
        out.push(if json {
            json!({
                "event": "rolloutEnded",
                "serial": serial,
                "ok": ok,
                "step": step,
                "text": text,
                "firmwareVersion": firmware_version,
            })
            .to_string()
        } else if *ok {
            format!(
                "rollout: {} updated to firmware {}: {}",
                printable(serial),
                printable(firmware_version),
                printable(text)
            )
        } else {
            format!(
                "rollout: {} FAILED at {}: {} (alarm rollout_failed raised)",
                printable(serial),
                printable(step),
                printable(text)
            )
        });
    }
    out
}

/// The large warning of `--allow-unverified-firmware`.
pub(super) const UNVERIFIED_WARNING: &str = "\
WARNING: --allow-unverified-firmware. This hub flashes firmware images the release does
not list: an admin's console can put any image it uploads on the DSPis on this machine's
USB (the firmware guard still refuses an image the board's core voltage does not allow).
Use it on a bench, never at a venue.";

/// At the start, with `--rollout`: that it is on, and the warning when unverified images
/// are allowed (on standard error too with `--json`, so it is never only in a log).
pub(super) fn rollout_at_start(opts: &RunOptions, json: bool) {
    if !opts.rollout {
        return;
    }
    if json {
        println!(
            "{}",
            json!({
                "event": "rollout",
                "allowUnverifiedFirmware": opts.allow_unverified_firmware,
            })
        );
        if opts.allow_unverified_firmware {
            eprintln!("{UNVERIFIED_WARNING}");
        }
        return;
    }
    println!(
        "Firmware rollout: on, for admins over TLS only ({})",
        if opts.allow_unverified_firmware {
            "any valid image"
        } else {
            "verified images only"
        }
    );
    if opts.allow_unverified_firmware {
        println!("{UNVERIFIED_WARNING}");
    }
}

#[cfg(test)]
#[path = "tests_wp_p3tm.rs"]
mod tests_wp_p3tm;
