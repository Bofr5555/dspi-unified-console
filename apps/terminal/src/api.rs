//! -- WP P2TM -- `uconsole --hub HOST api <t> ['<json>' | -]`: any message of
//! the hub API (`docs/integrator/hub-api.md`, section 16), sent as this
//! console's DSPi Link session, and its answer printed.
//!
//! The hub judges it as every other request of this console: its role, the
//! limits, the lock, the voltage rule. `--read-only` lets the hub's reads out
//! only (the core's rule). No device is bound: a message names its device in
//! its own fields (`serial`), or `--device <serial>` fills that in.
//!
//! The answer goes to standard output: its fields as indented JSON, or with
//! `--json` the hub's whole `ok` object on one line. A refusal is one line on
//! standard error (with `--json`, `{"t":"err","code","msg"}` on standard
//! output), and the exit status says which kind ([`exit_for`]), so a script
//! branches on it without reading text.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::io::Read;

use dspi_cmd::COMMAND;
use dspi_cmd::api::{ApiFields, parse_api};
use dspi_core::{DspiCore, DspiError};
use dspi_tui::shell::connect::{self, LinkTarget, Target, printable};
use serde_json::{Map, Value, json};

use crate::exit;

/// Exit statuses of `api` past the shared ones (`exit`: 0 ok, 1 usage, 2 not
/// found, 3 refused, 4 the hub could not be reached or failed). Part of the
/// interface: scripts branch on them; hub-api.md section 16 lists them.
pub(crate) mod status {
    /// `unauthenticated`, `denied`: this console's role (or key) may not.
    pub const DENIED: u8 = 5;
    /// `offline`: the device is known but not attached; nothing was sent.
    pub const OFFLINE: u8 = 6;
    /// `limit_exceeded`, `installer_lock`, `image_refused`: the installer's
    /// limits or lock refused it; nothing was sent.
    pub const LIMIT: u8 = 7;
    /// `bad_request`, `unknown_type`, `unsupported`: the hub does not take
    /// this message as written, or does not run that family.
    pub const BAD_REQUEST: u8 = 8;
    /// `busy`, `rate_limited`, `locked`: try again later.
    pub const BUSY: u8 = 9;
}

/// Fields read from standard input at most (the hub's frame is 64 KiB).
const MAX_STDIN: u64 = 1 << 20;

fn usage_text() -> String {
    format!(
        "\
{COMMAND} --hub [NAME] api <type> ['<json>' | -]
{COMMAND} --link HOST[:PORT] api <type> ['<json>' | -]

Send one message of the hub API (docs/integrator/hub-api.md) and print the
answer. <type> is the message's `t` (inventory.list, param.get, ...); the JSON
object holds its fields, or `-` reads it from standard input. With
--device <serial> a message without `serial` or `handle` gets that serial.

    --json        the hub's whole answer, one line
    --read-only   send only the hub's reads

Exit status: 0 ok; 1 usage; 2 not_found, no_device; 3 another refusal
(conflict, confirmation_required, device, read-only); 4 the hub could not be
reached, timed out or failed (internal); 5 unauthenticated, denied;
6 offline; 7 limit_exceeded, installer_lock, image_refused; 8 bad_request,
unknown_type, unsupported; 9 busy, rate_limited, locked.

Examples:
    {COMMAND} --hub api inventory.list
    {COMMAND} --hub api param.get '{{\"serial\": \"E46058388B1A2E2C\", \"path\": \"vol.master\"}}'
    {COMMAND} --hub --device E46058388B1A2E2C api param.set '{{\"path\": \"vol.master\", \"value\": -20}}'
    {COMMAND} --hub api schedule.set - < rule.json"
    )
}

/// `uconsole api ...`; `flags` starts at the word `api`.
pub(crate) fn run(target: &Target, flags: &[&str], json: bool) -> u8 {
    if flags.iter().any(|f| matches!(*f, "--help" | "-h")) {
        println!("{}", usage_text());
        return exit::OK;
    }
    let device = match target {
        Target::Link(l) => l.device.as_deref(),
        _ => None,
    };
    let (t, fields) = match request(flags, device, &mut std::io::stdin().lock()) {
        Ok(r) => r,
        Err(m) => return usage(&m),
    };
    let Target::Link(l) = target else {
        return usage(&format!(
            "`api` talks to a hub: give --hub [NAME] or --link HOST[:PORT] (see `{COMMAND} api --help`)"
        ));
    };
    let core = DspiCore::new();
    crate::core_route::apply_read_only(&core); // -- WP ROMODE --
    let code = call(&core, l, &crate::prompt_pin, &t, fields, json);
    core.link_close();
    code
}

/// The message to send: its type and fields, `--device` filled in.
fn request(
    flags: &[&str],
    device: Option<&str>,
    stdin: &mut dyn Read,
) -> Result<(String, Map<String, Value>), String> {
    let words = crate::positional(flags);
    let call = parse_api(&words).map_err(|e| e.to_string())?;
    let text = match call.fields {
        ApiFields::None => None,
        ApiFields::Text(t) => Some(t.to_owned()),
        ApiFields::Stdin => {
            let mut s = String::new();
            stdin
                .take(MAX_STDIN)
                .read_to_string(&mut s)
                .map_err(|e| format!("standard input: {e}"))?;
            Some(s)
        }
    };
    let mut fields = match text {
        None => Map::new(),
        Some(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(m)) => m,
            Ok(_) => return Err("the fields must be one JSON object: {...}".into()),
            Err(e) => return Err(format!("the fields are not JSON: {e}")),
        },
    };
    // `t` and `id` are the client's to set.
    fields.remove("t");
    fields.remove("id");
    if let Some(serial) = device
        && !fields.contains_key("serial")
        && !fields.contains_key("handle")
    {
        fields.insert("serial".into(), Value::from(serial));
    }
    Ok((call.t.to_owned(), fields))
}

/// Open the hub session on `core`, send, print; the exit status.
fn call(
    core: &DspiCore,
    l: &LinkTarget,
    pin: connect::PinPrompt<'_>,
    t: &str,
    fields: Map<String, Value>,
    json: bool,
) -> u8 {
    if let Err(e) = crate::core_route::dlink_session(core, l, pin) {
        return e.report();
    }
    match core.link_hub_request(t, fields) {
        Ok(answer) => {
            print_answer(&answer, json);
            exit::OK
        }
        Err(e) => refused(&e, json),
    }
}

fn print_answer(answer: &Value, json: bool) {
    if json {
        println!("{answer}");
        return;
    }
    let mut body = answer.as_object().cloned().unwrap_or_default();
    for k in ["t", "id", "v"] {
        body.remove(k);
    }
    if body.is_empty() {
        println!("ok");
        return;
    }
    match serde_json::to_string_pretty(&Value::Object(body)) {
        Ok(text) => println!("{text}"),
        Err(_) => println!("{answer}"),
    }
}

/// Say why, and the exit status.
fn refused(e: &DspiError, json: bool) -> u8 {
    let (code, msg) = match e {
        DspiError::Link { code, msg } => (code.clone(), msg.clone()),
        DspiError::ReadOnly => ("read_only".to_owned(), e.to_string()),
        DspiError::Disconnected => ("closed".to_owned(), e.to_string()),
        other => ("refused".to_owned(), other.to_string()),
    };
    let status = match e {
        DspiError::Link { code, .. } => exit_for(code),
        DspiError::Disconnected => exit::TRANSPORT,
        // `--read-only`, or a refusal the core typed itself.
        other => crate::pro::error_exit(other),
    };
    if json {
        println!("{}", json!({ "t": "err", "code": code, "msg": msg }));
    } else {
        eprintln!("{COMMAND}: {} ({})", printable(&msg), printable(&code));
    }
    status
}

/// The exit status for a hub's `code` (hub-api.md section 1.6) or the
/// client's own (`timeout`, `closed`, `io`).
pub(crate) fn exit_for(code: &str) -> u8 {
    match code {
        "unauthenticated" | "denied" => status::DENIED,
        "offline" => status::OFFLINE,
        "limit_exceeded" | "installer_lock" | "image_refused" => status::LIMIT,
        "bad_request" | "unknown_type" | "unsupported" => status::BAD_REQUEST,
        "busy" | "rate_limited" | "locked" | "hub_busy" => status::BUSY,
        "not_found" | "no_device" => exit::NO_DEVICE,
        "internal" | "timeout" | "closed" | "io" | "handshake" | "protocol" => exit::TRANSPORT,
        _ => exit::REJECTED,
    }
}

fn usage(m: &str) -> u8 {
    eprintln!("{COMMAND} api: {m}");
    exit::USAGE
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
pub(crate) mod tests_wp_p2tm {
    use super::*;

    fn req(flags: &[&str], stdin: &str) -> Result<(String, Map<String, Value>), String> {
        request(flags, None, &mut stdin.as_bytes())
    }

    fn req_on(device: &str, flags: &[&str]) -> Map<String, Value> {
        request(flags, Some(device), &mut "".as_bytes()).unwrap().1
    }

    #[test]
    fn wp_p2tm_api_requests_are_read_from_the_line_or_standard_input() {
        let (t, f) = req(
            &[
                "api",
                "param.get",
                r#"{"serial":"S","path":"vol.master","id":9,"t":"x"}"#,
            ],
            "",
        )
        .unwrap();
        assert_eq!(t, "param.get");
        assert_eq!(
            Value::Object(f),
            json!({"serial": "S", "path": "vol.master"})
        );
        let (_, f) = req(&["api", "schedule.set", "-", "--json"], r#"{"rule": {}}"#).unwrap();
        assert_eq!(Value::Object(f), json!({"rule": {}}));
        let (_, f) = req(&["api", "inventory.list"], "").unwrap();
        assert!(f.is_empty());
        // `--device` fills in a serial the fields lack, never one they have.
        let f = req_on("D1", &["api", "param.get", r#"{"path":"vol.master"}"#]);
        assert_eq!(f["serial"], "D1");
        let f = req_on("D1", &["api", "param.get", r#"{"serial":"D2"}"#]);
        assert_eq!(f["serial"], "D2");
        let f = req_on("D1", &["api", "x.y", r#"{"handle":0}"#]);
        assert!(!f.contains_key("serial"));
    }

    #[test]
    fn wp_p2tm_bad_api_requests_are_usage_errors() {
        assert!(req(&["api"], "").is_err());
        assert!(req(&["api", "param.get", "[1]"], "").is_err());
        assert!(req(&["api", "param.get", "{nope"], "").is_err());
        assert!(req(&["api", "param.get", "-"], "7").is_err());
        assert!(req(&["api", "Param", "{}"], "").is_err());
        assert_eq!(
            run(&Target::Demo, &["api", "hub.stats"], false),
            exit::USAGE
        );
        assert_eq!(run(&Target::Demo, &["api", "--help"], false), exit::OK);
    }

    #[test]
    fn wp_p2tm_api_exit_statuses_follow_the_code() {
        for (code, want) in [
            ("denied", status::DENIED),
            ("unauthenticated", status::DENIED),
            ("offline", status::OFFLINE),
            ("limit_exceeded", status::LIMIT),
            ("installer_lock", status::LIMIT),
            ("bad_request", status::BAD_REQUEST),
            ("unknown_type", status::BAD_REQUEST),
            ("rate_limited", status::BUSY),
            ("locked", status::BUSY),
            ("not_found", exit::NO_DEVICE),
            ("timeout", exit::TRANSPORT),
            ("internal", exit::TRANSPORT),
            ("conflict", exit::REJECTED),
            ("something_new", exit::REJECTED),
        ] {
            assert_eq!(exit_for(code), want, "{code}");
        }
    }

    #[test]
    fn wp_p2tm_api_is_a_command_after_hub_not_a_hubs_name() {
        let (t, rest) = crate::parse_target(&["--hub", "api", "inventory.list"]).unwrap();
        let Target::Link(l) = t else {
            panic!("a hub target")
        };
        assert_eq!(l.find.unwrap().wanted, None);
        assert_eq!(rest, ["api", "inventory.list"]);
        let (t, _) =
            crate::parse_target(&["--hub", "Studio", "--device", "D1", "api", "x.y"]).unwrap();
        let Target::Link(l) = t else {
            panic!("a hub target")
        };
        assert_eq!(l.device.as_deref(), Some("D1"));
    }

    /// One `api` call as `uconsole --link 127.0.0.1:PORT api ...` makes it, on
    /// its own core; for hub.rs's tests against a running hub.
    pub(crate) fn api_call(target: &Target, t: &str, fields: Value) -> u8 {
        let Target::Link(l) = target else {
            panic!("not a link target")
        };
        let core = DspiCore::new();
        let Value::Object(fields) = fields else {
            panic!("not an object")
        };
        let code = call(&core, l, &connect::no_prompt, t, fields, true);
        core.link_close();
        code
    }
}
