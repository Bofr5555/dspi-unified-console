//! -- WP P2TM -- `api <t> [json | -]`: any message of the hub API
//! (`docs/integrator/hub-api.md`), sent as it is.
//!
//! Only the shape of the line is read here: the message type and where its
//! fields come from. The fields are JSON, which the binary that sends them
//! reads (this crate has no JSON). The TUI's `:` line does not send it: a hub
//! request is the shell's (`uconsole --hub HOST api ...`).

/// The longest message type the hub knows is 20 characters; this is plenty.
pub const MAX_TYPE_LEN: usize = 64;

/// What `api` was asked to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiCall<'a> {
    /// The message type, `t` (`param.get`, `schedule.list`).
    pub t: &'a str,
    /// Its fields.
    pub fields: ApiFields<'a>,
}

/// Where an [`ApiCall`]'s fields come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiFields<'a> {
    /// None: `{}`.
    None,
    /// This text, a JSON object.
    Text(&'a str),
    /// `-`: standard input, a JSON object.
    Stdin,
}

/// Why an `api` line is not one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiParseError {
    #[error("which message? `{cmd} api <type> ['<json>']`, e.g. `{cmd} api inventory.list`", cmd = crate::COMMAND)]
    NoType,
    #[error(
        "`{0}` is not a message type: lowercase letters, digits, `_` and `.`, at most {MAX_TYPE_LEN}"
    )]
    BadType(String),
    #[error("one JSON object after the type (quote it), or `-` to read it from standard input")]
    TooMany,
}

/// The words after `api`, with the flags already taken out.
pub fn parse_api<'a>(words: &[&'a str]) -> Result<ApiCall<'a>, ApiParseError> {
    let (t, rest) = words.split_first().ok_or(ApiParseError::NoType)?;
    if !is_type(t) {
        return Err(ApiParseError::BadType(printable(t)));
    }
    let fields = match rest {
        [] => ApiFields::None,
        ["-"] => ApiFields::Stdin,
        [text] => ApiFields::Text(text),
        _ => return Err(ApiParseError::TooMany),
    };
    Ok(ApiCall { t, fields })
}

/// `t` as the hub's messages spell it: `inventory.list`, `alarm.rules.get`.
fn is_type(t: &str) -> bool {
    !t.is_empty()
        && t.len() <= MAX_TYPE_LEN
        && t.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
        && !t.starts_with('.')
        && !t.ends_with('.')
        && !t.contains("..")
}

/// A word to quote back in an error: no control characters, not too long.
fn printable(word: &str) -> String {
    word.chars().filter(|c| !c.is_control()).take(80).collect()
}

#[cfg(test)]
mod tests_wp_p2tm {
    use super::*;

    #[test]
    fn wp_p2tm_api_lines_parse() {
        assert_eq!(
            parse_api(&["inventory.list"]),
            Ok(ApiCall {
                t: "inventory.list",
                fields: ApiFields::None
            })
        );
        assert_eq!(
            parse_api(&["param.get", r#"{"serial":"X","path":"vol.master"}"#]),
            Ok(ApiCall {
                t: "param.get",
                fields: ApiFields::Text(r#"{"serial":"X","path":"vol.master"}"#)
            })
        );
        assert_eq!(
            parse_api(&["schedule.set", "-"]).map(|c| c.fields),
            Ok(ApiFields::Stdin)
        );
        assert_eq!(
            parse_api(&["alarm.rules.get"]).map(|c| c.t),
            Ok("alarm.rules.get")
        );
    }

    #[test]
    fn wp_p2tm_bad_api_lines_are_refused() {
        assert_eq!(parse_api(&[]), Err(ApiParseError::NoType));
        for bad in [
            "",
            "Param.get",
            "param get",
            ".list",
            "list.",
            "a..b",
            "{\"t\":1}",
            "\u{7}x",
        ] {
            assert!(
                matches!(parse_api(&[bad]), Err(ApiParseError::BadType(_))),
                "{bad:?}"
            );
        }
        assert!(parse_api(&[&"x".repeat(MAX_TYPE_LEN + 1)]).is_err());
        assert_eq!(
            parse_api(&["param.get", "{}", "{}"]),
            Err(ApiParseError::TooMany)
        );
        // The message names no control character back.
        let Err(e) = parse_api(&["\u{1b}[2Jx"]) else {
            panic!("parsed")
        };
        assert!(!e.to_string().contains('\u{1b}'));
    }
}
