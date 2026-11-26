//! The AutoEQ database on the web (WP WB6K, M31 and M32 over CG6). The page
//! keeps the file: the built-in one is served beside the console, the user's
//! is in the page's storage. It hands the text to `autoeqDatabaseLoad`, or
//! makes the Download's GET itself and hands the answer, whatever its status,
//! to `autoeqDatabaseDownload`; the core checks it either way
//! (`dspi_core::AutoEqLibrary`). The database then stays with the session for
//! `autoeqSearch`, `autoeqFavourites` and `autoeqEntry`, whose JSON is the
//! `profile` `autoeqPreview` and `autoeqApply` take.
//!
//! The favourites file is the page's too: `autoeqFavouritesText` turns the ids
//! into its text and `autoeqFavourites` reads it back, with the profiles the
//! database holds. Rebuild from GitHub is not on the web: its requests are the
//! core's own, made while a call runs, and the page has no blocking GET.

use std::cell::RefCell;
use std::collections::HashMap;

use dspi_core::{
    AutoEqFetchResponse, AutoEqLibrary, DspiError, autoeq_favourites_parse, autoeq_favourites_text,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::calls::{CallError, tidy};

thread_local! {
    /// Each session's database, by handle.
    static LIBRARIES: RefCell<HashMap<u32, AutoEqLibrary>> = RefCell::new(HashMap::new());
}

/// Forget session `h`'s database (the session was freed).
pub fn forget(h: u32) {
    LIBRARIES.with_borrow_mut(|l| l.remove(&h));
}

/// A database failure as the page's error: the core's AutoEQ code
/// (`bad_content`, `rate_limited`, `http_status`, ...) and its words.
fn failure(e: DspiError) -> CallError {
    match &e {
        DspiError::AutoEq { code, msg, .. } => CallError {
            code: match code.as_str() {
                "bad_content" => "bad_content",
                "rate_limited" => "rate_limited",
                "http_status" => "http_status",
                "network" => "network",
                _ => "autoeq",
            },
            message: msg.clone(),
            notice_key: Some(e.notice_key()),
            text: Some(e.user_text()), // -- WP INTE --
        },
        _ => CallError::from(e),
    }
}

fn keep(h: u32, lib: AutoEqLibrary) -> Value {
    let info = tidy(&lib.info());
    LIBRARIES.with_borrow_mut(|l| l.insert(h, lib));
    info
}

fn with<T>(h: u32, f: impl FnOnce(&AutoEqLibrary) -> T) -> Result<T, CallError> {
    LIBRARIES.with_borrow(|l| l.get(&h).map(f)).ok_or_else(|| {
        CallError::new(
            "no_database",
            "No AutoEQ database is loaded in this session",
        )
    })
}

/// `autoeqDatabaseLoad`'s parameters: the file's `text`; `user` true for the
/// person's own database (imported or downloaded), false for the built-in one.
#[derive(Debug, Deserialize)]
pub struct LoadParams {
    pub text: String,
    #[serde(default)]
    pub user: bool,
}

/// `autoeqDatabaseLoad`: check the text and keep it as the session's database:
/// `{hasUserDatabase, problem, version, generatedAt, entryCount}`. A text that
/// is no database (or one with no profiles) fails as `bad_content` and the
/// database before stays.
pub fn load(h: u32, p: LoadParams) -> Result<Value, CallError> {
    let lib = if p.user {
        AutoEqLibrary::import(p.text.as_bytes())
    } else {
        AutoEqLibrary::built_in(p.text.as_bytes())
    }
    .map_err(failure)?;
    Ok(keep(h, lib))
}

/// `autoeqDatabaseDownload`'s parameters: the address the page asked, and
/// what the server answered.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadParams {
    pub url: String,
    pub status: u16,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub retry_after_s: Option<u32>,
    #[serde(default)]
    pub rate_limit_remaining: Option<u32>,
    #[serde(default)]
    pub rate_limit_reset: Option<u64>,
}

/// `autoeqDatabaseDownload`: read the page's answer to its GET as the core
/// reads a download, and keep the database it carries: the info, as
/// `autoeqDatabaseLoad`. Fails as `rate_limited`, `http_status` or
/// `bad_content`, the database before staying; store the body only on success.
pub fn download(h: u32, p: DownloadParams) -> Result<Value, CallError> {
    let response = AutoEqFetchResponse {
        status: p.status,
        body: p.body.into_bytes(),
        retry_after_s: p.retry_after_s,
        rate_limit_remaining: p.rate_limit_remaining,
        rate_limit_reset: p.rate_limit_reset,
    };
    let lib = AutoEqLibrary::download(&p.url, response).map_err(failure)?;
    Ok(keep(h, lib))
}

/// `autoeqDatabaseInfo`: the session's database's info, null with none.
pub fn info(h: u32) -> Value {
    with(h, |l| tidy(&l.info())).unwrap_or(Value::Null)
}

/// `autoeqSearch`'s parameters.
#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_limit() -> u32 {
    50
}

/// `autoeqSearch`: `{hits: [{id, name, source, formFactor, filters,
/// preampDb}], total}`, at most `limit` hits (default 50, at most 500).
pub fn search(h: u32, p: SearchParams) -> Result<Value, CallError> {
    let limit = usize::try_from(p.limit.min(500)).unwrap_or(50);
    with(h, |l| {
        let (hits, total) = l.search(&p.query, limit);
        json!({ "hits": tidy(&hits), "total": total })
    })
}

/// `autoeqEntry`'s parameters.
#[derive(Debug, Deserialize)]
pub struct EntryParams {
    pub id: String,
}

/// `autoeqEntry`: the profile as the database has it, for `autoeqPreview`'s
/// `profile`. `not_found` for an id it does not hold.
pub fn entry(h: u32, p: EntryParams) -> Result<Value, CallError> {
    let text = with(h, |l| l.entry_json(&p.id))?.ok_or_else(|| {
        CallError::new(
            "not_found",
            format!("{} is not in the AutoEQ database", p.id),
        )
    })?;
    serde_json::from_str(&text).map_err(|e| CallError::new("internal", e.to_string()))
}

/// `autoeqFavourites`'s parameters: the favourites file's text, as the page
/// keeps it ("" for none).
#[derive(Debug, Deserialize)]
pub struct FavouritesParams {
    #[serde(default)]
    pub text: String,
}

/// `autoeqFavourites`: `{ids, hits}`: the ids the file holds, and the profiles
/// of those the session's database holds (none without a database).
pub fn favourites(h: u32, p: FavouritesParams) -> Value {
    let ids = autoeq_favourites_parse(p.text.as_bytes());
    let hits = with(h, |l| tidy(&l.hits(&ids))).unwrap_or_else(|_| json!([]));
    json!({ "ids": ids, "hits": hits })
}

/// `autoeqFavouritesText`'s parameters.
#[derive(Debug, Deserialize)]
pub struct FavouritesTextParams {
    pub ids: Vec<String>,
}

/// `autoeqFavouritesText`: `{text}`, the favourites file for these ids.
pub fn favourites_text(p: FavouritesTextParams) -> Value {
    json!({ "text": autoeq_favourites_text(&p.ids) })
}

#[cfg(test)]
mod tests_wp_wb6k {
    use crate::calls::{call, session_free, session_new};
    use crate::guard;
    use serde_json::{Value, json};

    const DB: &str = r#"{"version":1,"generatedAt":"2026-01-02T03:04:05.000Z","entryCount":1,
        "entries":[{"id":"oratory1990/Sennheiser HD 600","manufacturer":"Sennheiser","model":"HD 600",
        "source":"oratory1990","preamp":-6,"filters":[{"type":"PK","freq":105,"q":0.7,"gain":3}]}]}"#;

    fn run(h: u32, method: &str, params: Value) -> Value {
        let out = call(
            h,
            json!({ "method": method, "params": params })
                .to_string()
                .as_bytes(),
        );
        serde_json::from_slice(&out).unwrap()
    }

    fn ok(h: u32, method: &str, params: Value) -> Value {
        let r = run(h, method, params);
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["value"].clone()
    }

    #[test]
    fn wp_wb6k_the_database_the_page_keeps() {
        guard::reset_for_test();
        let h = session_new().unwrap();
        assert_eq!(ok(h, "autoeqDatabaseInfo", Value::Null), Value::Null);
        assert_eq!(
            run(h, "autoeqSearch", json!({ "query": "hd" }))["code"],
            "no_database"
        );
        let info = ok(h, "autoeqDatabaseLoad", json!({ "text": DB }));
        assert_eq!(info["hasUserDatabase"], false);
        assert_eq!(info["entryCount"], 1);
        // A file that is no database is refused and the one before stays.
        assert_eq!(
            run(
                h,
                "autoeqDatabaseLoad",
                json!({ "text": "{}", "user": true })
            )["code"],
            "bad_content"
        );
        assert_eq!(
            ok(h, "autoeqDatabaseInfo", Value::Null)["hasUserDatabase"],
            false
        );

        let found = ok(h, "autoeqSearch", json!({ "query": "600 senn" }));
        assert_eq!(found["total"], 1);
        let id = found["hits"][0]["id"].as_str().unwrap().to_string();
        assert_eq!(found["hits"][0]["name"], "Sennheiser HD 600");
        let entry = ok(h, "autoeqEntry", json!({ "id": id }));
        assert_eq!(
            run(h, "autoeqEntry", json!({ "id": "x" }))["code"],
            "not_found"
        );

        // The entry is the profile autoeqPreview takes.
        ok(h, "open", json!({ "kind": "demo" }));
        let pv = ok(
            h,
            "autoeqPreview",
            json!({ "profile": entry, "left": 0, "right": 1 }),
        );
        assert_eq!(pv["levelTarget"], "input_preamp");

        // Favourites: the page's text, read back with what the database holds.
        let text = ok(h, "autoeqFavouritesText", json!({ "ids": [id, "gone/x"] }))["text"].clone();
        let fav = ok(h, "autoeqFavourites", json!({ "text": text }));
        assert_eq!(fav["ids"].as_array().unwrap().len(), 2);
        assert_eq!(fav["hits"].as_array().unwrap().len(), 1);
        assert_eq!(
            ok(h, "autoeqFavourites", json!({ "text": "" }))["ids"],
            json!([])
        );

        // The page's GET: its answer read as the core reads a download.
        let limited = run(
            h,
            "autoeqDatabaseDownload",
            json!({ "url": "https://example.invalid/db.json", "status": 403, "rateLimitRemaining": 0 }),
        );
        assert_eq!(limited["code"], "rate_limited", "{limited}");
        assert_eq!(
            run(
                h,
                "autoeqDatabaseDownload",
                json!({ "url": "u", "status": 500 })
            )["code"],
            "http_status"
        );
        assert_eq!(
            run(
                h,
                "autoeqDatabaseDownload",
                json!({ "url": "u", "status": 200, "body": "<html>" })
            )["code"],
            "bad_content"
        );
        let info = ok(
            h,
            "autoeqDatabaseDownload",
            json!({ "url": "u", "status": 200, "body": DB }),
        );
        assert_eq!(info["hasUserDatabase"], true);

        // Freed, its database goes with it.
        assert!(session_free(h).unwrap());
        let h2 = session_new().unwrap();
        assert_eq!(ok(h2, "autoeqDatabaseInfo", Value::Null), Value::Null);
    }
}
