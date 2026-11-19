//! `uconsole info` and `uconsole raw --id` (WP PGTM, PLAN-PG wave PG-D).
//!
//! - `uconsole info`: the open device's protocol generation, the paged
//!   protocol's version, its page list, how its extension channel is reached
//!   and the board's name, from the core's `device_protocol()`, read once per
//!   connection (it costs one platform read). The interface's Settings > About
//!   shows the same rows ([`DeviceProtocolView::rows`], filled at connect by
//!   [`read_view`]).
//! - `uconsole raw --id 0x0550 <len> [wValue]` and `uconsole raw out --id
//!   0x0550 <wValue> [hex...]`: one paged GET or SET through the core's
//!   `raw_get_id` / `raw_set_id`, with the core's refusals in words (a page-0
//!   id, a 1.1.6 device, a command flagged as writing flash, restarting or
//!   bulk, a firmware result code by its `VC_ERR_*` name). `raw_set_id` reads
//!   `SYS.COMMAND_INFO` per call (PG-C merge note), which is one more
//!   transfer per SET.
//!
//! A paged device on a connection that says the generation but carries no page
//! discovery shows an empty page list, and the extension envelope is the
//! 1.1.6-shaped probe's (PG-C review carry-over; the note
//! [`dspi_tui::settings::about::NO_PAGE_LIST_NOTE`] says so). -- WP LINKFIX --
//! Our hub's in-process `LocalTransport` and a principal's `PrincipalTransport`
//! pass the attachment's discovery on, so they list the pages.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_cmd::COMMAND;
use dspi_core::{DeviceProtocol, DspiCore, DspiError, ExtEnvelope, ProtocolGeneration};
use dspi_proto::paged::registry;
use dspi_tui::settings::about::{DeviceProtocolView, ExtView, GenerationView};
use serde_json::json;

use crate::exit;

/// The core's record as the rows both `uconsole info` and the About page show.
pub(crate) fn view_of(p: &DeviceProtocol) -> DeviceProtocolView {
    DeviceProtocolView {
        generation: match p.generation {
            ProtocolGeneration::Legacy => GenerationView::Legacy,
            ProtocolGeneration::Paged => GenerationView::Paged,
            ProtocolGeneration::Unknown => GenerationView::Unknown,
        },
        protocol_major: p.protocol_major,
        protocol_minor: p.protocol_minor,
        pages: p.pages.iter().map(|pg| (pg.page, pg.revision)).collect(),
        ext: match p.ext_envelope {
            ExtEnvelope::NoChannel => ExtView::NoChannel,
            ExtEnvelope::Legacy => ExtView::Legacy,
            ExtEnvelope::Tunnel => ExtView::Tunnel,
        },
        board_id: p.board_id,
        board_name: p.board_name.clone(),
        // -- review PG-D1 -- (WP PGFIX) the alink sender is said
        alink_sender: p.device_kind == dspi_core::DeviceKind::AlinkSender,
    }
}

/// The open device's protocol, read now (one platform read): for a new
/// connection, once. `None` with nothing open; the interface then shows no
/// protocol rows.
pub(crate) fn read_view(core: &DspiCore) -> Option<DeviceProtocolView> {
    match core.device_protocol() {
        Ok(p) => Some(view_of(&p)),
        Err(e) => {
            log::debug!("device_protocol: {e}");
            None
        }
    }
}

/// `uconsole info`'s text: one row a line, the notes after them.
pub(crate) fn info_text(serial: &str, view: &DeviceProtocolView) -> String {
    let mut out = format!(
        "{:<12}{}\n",
        "Serial",
        dspi_tui::shell::connect::printable(serial)
    );
    for (label, value) in view.rows() {
        out.push_str(&format!("{label:<12}{value}\n"));
    }
    for note in view.notes() {
        out.push_str(&format!("\nNote: {note}\n"));
    }
    out
}

/// `uconsole info --json`: the core's record as it serialises, with the
/// serial and the notes.
pub(crate) fn info_json(serial: &str, p: &DeviceProtocol) -> serde_json::Value {
    let notes = view_of(p).notes();
    json!({
        "serial": serial,
        "protocol": p,
        "notes": notes,
    })
}

/// `uconsole info`: open the device on a core, read its protocol once, print it.
pub(crate) fn cmd_info(target: &dspi_tui::shell::connect::Target, json: bool) -> u8 {
    let opened = match crate::core_route::open_core(
        target,
        &crate::pro::Stores::default_dirs(),
        &crate::prompt_pin,
    ) {
        Ok(o) => o,
        Err(e) => return e.report(),
    };
    let core = &opened.handle.core;
    let serial = opened.handle.serial.clone();
    match core.device_protocol() {
        Ok(p) => {
            if json {
                println!("{}", info_json(&serial, &p));
            } else {
                print!("{}", info_text(&serial, &view_of(&p)));
            }
            exit::OK
        }
        Err(e) => crate::pro::fail(&e),
    }
}

// ─── raw --id ───────────────────────────────────────────────────────────

/// What `raw --id` was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RawId {
    /// `raw --id ID LEN [wValue]`
    Get { id: u16, len: u16, value: u16 },
    /// `raw out --id ID wValue [hex...]`
    Set { id: u16, value: u16, data: Vec<u8> },
}

/// Whether the command line names a paged id (`--id X` or `--id=X`).
pub(crate) fn wants_id(flags: &[&str]) -> bool {
    flags.iter().any(|f| *f == "--id" || f.starts_with("--id="))
}

/// An id as a number (`0x0550`, `1360`) or as `PAGE.NAME` (`ROUTE.MASTER_VOLUME`).
pub(crate) fn parse_id(text: &str) -> Result<u16, String> {
    if let Some(c) = registry::by_name(text) {
        return Ok(c.id.0);
    }
    crate::parse_num(text)
        .and_then(|n| u16::try_from(n).ok())
        .ok_or_else(|| {
            format!(
                "`{}` is not a command id: give 0xPPCC (page and command) or PAGE.NAME",
                dspi_tui::shell::connect::printable(text)
            )
        })
}

/// `flags` start with the word `raw`.
pub(crate) fn parse_raw_id(flags: &[&str]) -> Result<RawId, String> {
    let mut id_text = None;
    let mut rest = Vec::with_capacity(flags.len());
    let mut it = flags.iter().copied();
    while let Some(f) = it.next() {
        if f == "--id" {
            id_text = Some(
                it.next()
                    .ok_or("--id needs a command id, e.g. --id 0x0550")?,
            );
        } else if let Some(v) = f.strip_prefix("--id=") {
            id_text = Some(v);
        } else {
            rest.push(f);
        }
    }
    let id = parse_id(id_text.ok_or("--id needs a command id")?)?;
    let args = crate::positional(&rest);
    let number = |s: Option<&&str>, what: &str| -> Result<u16, String> {
        let s = s.ok_or_else(|| format!("raw --id needs {what}"))?;
        crate::parse_num(s)
            .and_then(|n| u16::try_from(n).ok())
            .ok_or_else(|| format!("{what} must be a number up to 65535"))
    };
    match args.first().copied() {
        Some("out") => {
            let value = number(args.get(1), "a wValue after `out`")?;
            let data = crate::parse_hex(args.get(2..).unwrap_or_default())?;
            Ok(RawId::Set { id, value, data })
        }
        Some("war") => Err(
            "`raw war` is a 1.1.6 form: on paged firmware every GET only reads, and a SET \
             is `raw out --id`"
                .into(),
        ),
        _ => {
            let len = number(args.first(), "a length, e.g. `raw --id 0x0550 4`")?;
            let value = match args.get(1) {
                Some(_) => number(args.get(1), "wValue")?,
                None => 0,
            };
            Ok(RawId::Get { id, len, value })
        }
    }
}

/// The command's name for the words, `0x0550 ROUTE.MASTER_VOLUME_SAVED`.
fn id_words(id: u16) -> String {
    match registry::command(dspi_transport::CmdId(id)) {
        Some(c) => format!("0x{id:04X} {}", c.full_name()),
        None => format!("0x{id:04X}"),
    }
}

/// The core's refusal of a raw-id call in words, with nothing sent unless
/// the device itself refused. `None` for an error the general path says
/// (`pro::fail`: the lock, the read-only mode, the link).
pub(crate) fn refusal_words(id: u16, e: &DspiError) -> Option<String> {
    let name = id_words(id);
    match e {
        DspiError::InvalidArgument { msg } => Some(format!("{name} was not sent: {msg}")),
        DspiError::Unsupported => Some(format!(
            "{name} was not sent: this device speaks the 1.1.6 protocol, which has no pages \
             (or this connection does not say it is paged). Use `{COMMAND} raw <opcode>` with a \
             1.1.6 opcode"
        )),
        DspiError::Refused { status: 0 } => {
            Some(format!("the device refused {name}; it gave no result code"))
        }
        DspiError::Refused { status } => {
            let code = dspi_proto::paged::legacy::result_name(*status).map_or_else(
                || format!("result code {status}"),
                |n| format!("{n} ({status})"),
            );
            Some(format!("the device refused {name}: {code}"))
        }
        _ => None,
    }
}

/// Say a raw-id failure, with the exit code a script branches on.
fn fail_raw_id(id: u16, e: &DspiError) -> u8 {
    match refusal_words(id, e) {
        Some(words) => {
            eprintln!("{COMMAND}: {words}");
            crate::pro::error_exit(e)
        }
        None => crate::pro::fail(e),
    }
}

/// Run `req` on `core`: the GET's reply, or `None` for a SET that went.
pub(crate) fn run_raw_id(core: &DspiCore, req: &RawId) -> Result<Option<Vec<u8>>, DspiError> {
    match req {
        RawId::Get { id, len, value } => core.raw_get_id(*id, *value, *len).map(Some),
        RawId::Set { id, value, data } => core.raw_set_id(*id, *value, data.clone()).map(|()| None),
    }
}

/// `uconsole raw [out] --id ...`. `flags` start with the word `raw`.
pub(crate) fn cmd_raw_id(
    target: &dspi_tui::shell::connect::Target,
    flags: &[&str],
    json: bool,
) -> u8 {
    let req = match parse_raw_id(flags) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{COMMAND}: {e}");
            return exit::USAGE;
        }
    };
    let id = match &req {
        RawId::Get { id, .. } | RawId::Set { id, .. } => *id,
    };
    let pins = match crate::core_route::PinSource::for_this_command() {
        Ok(p) => p,
        Err(c) => return c,
    };
    let opened = match crate::core_route::open_core(
        target,
        &crate::pro::Stores::default_dirs(),
        &crate::prompt_pin,
    ) {
        Ok(o) => o,
        Err(e) => return e.report(),
    };
    let core = &opened.handle.core;
    let mut done = run_raw_id(core, &req);
    // A SET the installer lock refused: the PIN, when one can be had, then once more
    if let Err(DspiError::InstallerLock { code, .. }) = &done
        && code == "locked"
        && matches!(req, RawId::Set { .. })
        && matches!(
            pins,
            crate::core_route::PinSource::Ask
                | crate::core_route::PinSource::Given(_)
                | crate::core_route::PinSource::Env(_)
        )
    {
        if let Err(c) = crate::core_route::unlock_up_front(core, &pins, true) {
            return c;
        }
        done = run_raw_id(core, &req);
    }
    match (done, &req) {
        (Ok(Some(bytes)), RawId::Get { value, .. }) => {
            if json {
                println!(
                    "{}",
                    json!({
                        "id": id,
                        "wValue": value,
                        "length": bytes.len(),
                        "bytes": crate::hex_string(&bytes),
                    })
                );
            } else {
                print!("{}", hex_dump(&bytes));
                println!("\n{} bytes from {}", bytes.len(), id_words(id));
            }
            exit::OK
        }
        (Ok(_), RawId::Set { value, data, .. }) => {
            if json {
                println!(
                    "{}",
                    json!({
                        "id": id,
                        "wValue": value,
                        "sent": data.len(),
                        "bytes": crate::hex_string(data),
                    })
                );
            } else {
                println!("Sent {} bytes to {}.", data.len(), id_words(id));
            }
            exit::OK
        }
        (Ok(_), RawId::Get { .. }) => exit::OK,
        (Err(e), _) => fail_raw_id(id, &e),
    }
}

/// 16 bytes a line, with their ASCII, as `uconsole raw` prints them.
fn hex_dump(bytes: &[u8]) -> String {
    let mut out = String::new();
    for (i, chunk) in bytes.chunks(16).enumerate() {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02X}")).collect();
        let ascii: String = chunk
            .iter()
            .map(|b| {
                if b.is_ascii_graphic() {
                    char::from(*b)
                } else {
                    '.'
                }
            })
            .collect();
        out.push_str(&format!(
            "{:04X}  {:<48}  {ascii}\n",
            i.saturating_mul(16),
            hex.join(" ")
        ));
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests_wp_pgtm;
