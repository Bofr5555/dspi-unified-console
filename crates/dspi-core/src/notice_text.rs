//! What to tell the user about an error (WP P1NT): one notice table for every
//! client, over `assets/notices/notices.json`.
//!
//! Every [`DspiError`] has a stable key ([`DspiError::notice_key`]): the C ABI's
//! failure kind (`disconnected`, `invalid`, ...; DSPi Link is `dlink`), refined to
//! `dlink.<code>` or `autoeq.<code>` where the table has a row for that code.
//! Keys are never reused. A client localises by the key and falls back to the
//! English here.
//!
//! Each row has a kind ([`NoticeKind`]: info, warning or error; there is no
//! success notice, a success shows in the change itself), a title, the reason
//! and what to do. The words are shared: no row names what the user is holding,
//! and a row may use CG10's placeholders (`crate::link_text`), resolved for the
//! platform this library was compiled for. A notice's `detail` is the error's own
//! words, for a detail line or a log.
//!
//! The asset is `{"notices": [{key, kind, title, reason, action, when?}]}`.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::DspiError;
use crate::link_text::{Platform, render_value};

const NOTICES_JSON: &str = include_str!("../../../assets/notices/notices.json");

/// The key of the row for a key the table does not have.
pub const NOTICE_KEY_UNKNOWN: &str = "unknown";

/// How a notice is shown.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    /// Nothing went wrong; something to know.
    Info,
    /// Not done, for a reason the user can change or wait out.
    Warning,
    /// Not done: something failed.
    Error,
}

/// What to tell the user about an error.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    /// Stable key, for localising: `disconnected`, `dlink.wrong_pin`.
    pub key: String,
    /// How to show it.
    pub kind: NoticeKind,
    /// A short title.
    pub title: String,
    /// Why it happened.
    pub reason: String,
    /// What to do.
    pub action: String,
    /// The error's own words (may be empty); not localised.
    pub detail: String,
}

/// One row of the asset.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    key: String,
    kind: NoticeKind,
    title: String,
    reason: String,
    action: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Table {
    notices: Vec<Row>,
}

/// The table for `platform`: every text resolved, every `when` applied. The
/// asset is valid (the tests read it on every platform); were it not, the table
/// is empty and every notice is the built-in fallback.
fn table_for(platform: Platform) -> Vec<Row> {
    serde_json::from_str::<Value>(NOTICES_JSON)
        .ok()
        .and_then(|v| render_value(v, platform))
        .and_then(|v| serde_json::from_value::<Table>(v).ok())
        .map(|t| t.notices)
        .unwrap_or_default()
}

static TABLE: LazyLock<Vec<Row>> = LazyLock::new(|| table_for(Platform::COMPILED));

/// The table has a row with this key.
pub(crate) fn has_row(key: &str) -> bool {
    TABLE.iter().any(|r| r.key == key)
}

fn notice(row: &Row, detail: String) -> Notice {
    Notice {
        key: row.key.clone(),
        kind: row.kind,
        title: row.title.clone(),
        reason: row.reason.clone(),
        action: row.action.clone(),
        detail,
    }
}

/// The row for `key`, or the `unknown` row. Only an asset that does not read
/// lacks that one: then a plain error notice in its place.
fn lookup(key: &str, detail: String) -> Notice {
    TABLE
        .iter()
        .find(|r| r.key == key)
        .or_else(|| TABLE.iter().find(|r| r.key == NOTICE_KEY_UNKNOWN))
        .map_or_else(
            || Notice {
                key: NOTICE_KEY_UNKNOWN.to_owned(),
                kind: NoticeKind::Error,
                title: "Something went wrong".to_owned(),
                reason: String::new(),
                action: String::new(),
                detail: detail.clone(),
            },
            |row| notice(row, detail.clone()),
        )
}

/// What to tell the user about `error`.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn notice_for(error: DspiError) -> Notice {
    lookup(&error.notice_key(), error.notice_detail())
}

/// The notice with this key (a failure reply's `noticeKey` in the C ABI and the
/// web binding), with `detail` as its detail. A key the table does not have
/// gives the `unknown` row.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn notice_for_key(key: String, detail: String) -> Notice {
    lookup(&key, detail)
}

/// The whole table, every key once and with no detail, for a client's
/// translation files.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn notice_texts() -> Vec<Notice> {
    TABLE.iter().map(|r| notice(r, String::new())).collect()
}

#[cfg(test)]
mod tests_wp_p1nt;
