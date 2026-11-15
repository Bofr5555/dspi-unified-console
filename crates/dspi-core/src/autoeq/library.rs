//! An AutoEQ database a host keeps itself (WP WB6K): the web, which has no
//! file system and no threads. The host stores the file's bytes where it can
//! (the page's storage) and hands them here; the checks, the search, the
//! favourites file and reading a download's answer are the same code every
//! client runs (`dspi_session::autoeq`, CG6).
//!
//! The download is the page's own HTTP GET: it makes the request and passes
//! the answer, whatever the status, to [`AutoEqLibrary::download`], which
//! reads it as [`autoeq_update`](crate::autoeq_update) reads its fetch's
//! answer (a rate limit, an error status, content that is no database).
//!
//! Not exported through UniFFI or C: the other clients keep their database as
//! a file and use the free functions of CG6.

use std::sync::Mutex;

use dspi_session::autoeq::{self as session, Database, Entry};

use super::{AutoEqDatabaseInfo, AutoEqFetchResponse, count, error};
use crate::Result;

/// One profile as a list shows it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoEqHit {
    /// The database's id: what favourites keep and [`AutoEqLibrary::entry_json`] takes.
    pub id: String,
    /// Manufacturer and model.
    pub name: String,
    /// Who measured it, as the Console names the source.
    pub source: String,
    /// `over-ear`, `in-ear` and so on, as the database has it; may be empty.
    pub form_factor: String,
    /// Filters in the profile.
    pub filters: u32,
    /// The trim the profile expects.
    pub preamp_db: f32,
}

impl From<&Entry> for AutoEqHit {
    fn from(e: &Entry) -> Self {
        AutoEqHit {
            id: e.id.clone(),
            name: e.display_name(),
            source: e.source_label().to_string(),
            form_factor: e.form_factor.clone(),
            filters: count(e.filters.len()),
            preamp_db: e.preamp,
        }
    }
}

/// A database held in memory, the built-in one or the user's.
#[derive(Debug, Clone)]
pub struct AutoEqLibrary {
    db: Database,
    user: bool,
}

impl AutoEqLibrary {
    /// The host's built-in database, from its bytes.
    ///
    /// Errors: [`DspiError::AutoEq`](crate::DspiError::AutoEq) `bad_content`
    /// unless they are a database with at least one profile.
    pub fn built_in(data: &[u8]) -> Result<Self> {
        let db = session::check_import(data).map_err(error)?;
        Ok(Self { db, user: false })
    }

    /// A database the person chose (Import File…), or the user's database
    /// as the host stored it. Errors as [`AutoEqLibrary::built_in`]: store
    /// the bytes only on `Ok`.
    pub fn import(data: &[u8]) -> Result<Self> {
        let db = session::check_import(data).map_err(error)?;
        Ok(Self { db, user: true })
    }

    /// The answer to the host's GET of `url` (Download…): the database it
    /// carries, once it is known to be one. Store the body only on `Ok`.
    ///
    /// Errors: [`DspiError::AutoEq`](crate::DspiError::AutoEq) with code
    /// `rate_limited` (and `retry_after_s` when the server said),
    /// `http_status` or `bad_content`.
    pub fn download(url: &str, response: AutoEqFetchResponse) -> Result<Self> {
        let answer = Answer(Mutex::new(Some(response)));
        let (db, _) = session::update_bytes(&answer, url).map_err(error)?;
        Ok(Self { db, user: true })
    }

    /// What the Update Database dialog shows about it.
    pub fn info(&self) -> AutoEqDatabaseInfo {
        AutoEqDatabaseInfo {
            has_user_database: self.user,
            ..AutoEqDatabaseInfo::from(&self.db)
        }
    }

    /// The profiles matching `query` (manufacturer, model and id, the terms in
    /// any order), at most `limit` of them, and how many matched in all. An
    /// empty query matches nothing.
    pub fn search(&self, query: &str, limit: usize) -> (Vec<AutoEqHit>, u32) {
        let hits = self.db.search(query);
        let total = count(hits.len());
        (
            hits.into_iter().take(limit).map(AutoEqHit::from).collect(),
            total,
        )
    }

    /// The profiles with these ids, in that order; an id the database does not
    /// hold is left out (a favourite from another database).
    pub fn hits(&self, ids: &[String]) -> Vec<AutoEqHit> {
        ids.iter()
            .filter_map(|id| self.db.get(id))
            .map(AutoEqHit::from)
            .collect()
    }

    /// One profile as the database has it, the JSON that
    /// [`AutoEqProfile::from_entry_json`](crate::AutoEqProfile::from_entry_json)
    /// takes; `None` for an id it does not hold.
    pub fn entry_json(&self, id: &str) -> Option<String> {
        self.db.get(id).and_then(|e| serde_json::to_string(e).ok())
    }
}

/// The favourites file's bytes as ids; anything unreadable is no favourites.
pub fn autoeq_favourites_parse(data: &[u8]) -> Vec<String> {
    session::favourites_from_bytes(data)
}

/// The favourites file's text for these ids.
pub fn autoeq_favourites_text(ids: &[String]) -> String {
    session::favourites_text(ids)
}

/// The host's answer, given once.
struct Answer(Mutex<Option<AutoEqFetchResponse>>);

impl session::Fetch for Answer {
    fn get(
        &self,
        _request: &session::FetchRequest,
    ) -> std::result::Result<session::FetchResponse, String> {
        let r = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or_else(|| "no answer".to_string())?;
        Ok(session::FetchResponse {
            status: r.status,
            body: r.body,
            retry_after_s: r.retry_after_s,
            rate_limit_remaining: r.rate_limit_remaining,
            rate_limit_reset: r.rate_limit_reset,
        })
    }
}

#[cfg(test)]
mod tests_wp_wb6k {
    use super::*;
    use crate::DspiError;

    const DB: &str = r#"{"version":1,"generatedAt":"2026-01-02T03:04:05.000Z","entryCount":2,
        "entries":[{"id":"oratory1990/Sennheiser HD 600","manufacturer":"Sennheiser","model":"HD 600",
        "source":"oratory1990","formFactor":"over-ear","preamp":-6.2,
        "filters":[{"type":"PK","freq":105,"q":0.7,"gain":3}]},
        {"id":"crinacle/Sony WH-1000XM4","manufacturer":"Sony","model":"WH-1000XM4","source":"crinacle","filters":[]}]}"#;

    fn code(e: DspiError) -> String {
        match e {
            DspiError::AutoEq { code, .. } => code,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn wp_wb6k_a_kept_database_searches_and_gives_entries() {
        let lib = AutoEqLibrary::import(DB.as_bytes()).unwrap();
        let info = lib.info();
        assert!(info.has_user_database);
        assert_eq!(info.entry_count, 2);
        assert_eq!(info.generated_at, "2026-01-02T03:04:05.000Z");
        let (hits, total) = lib.search("600 sennheiser", 10);
        assert_eq!(total, 1);
        assert_eq!(hits[0].name, "Sennheiser HD 600");
        assert_eq!(hits[0].source, "oratory1990");
        assert_eq!(hits[0].filters, 1);
        assert_eq!(lib.search("", 10).1, 0);
        let entry = lib.entry_json("oratory1990/Sennheiser HD 600").unwrap();
        let p = crate::AutoEqProfile::from_entry_json(&entry).unwrap();
        assert_eq!(p.name, "Sennheiser HD 600");
        assert_eq!(p.filters.len(), 1);
        assert!(lib.entry_json("nope").is_none());
        let ids = vec!["crinacle/Sony WH-1000XM4".to_string(), "gone".to_string()];
        assert_eq!(lib.hits(&ids).len(), 1);
        assert!(
            !AutoEqLibrary::built_in(DB.as_bytes())
                .unwrap()
                .info()
                .has_user_database
        );
    }

    #[test]
    fn wp_wb6k_bad_files_and_answers_are_refused_in_the_cores_codes() {
        assert_eq!(
            code(AutoEqLibrary::import(b"{}").unwrap_err()),
            "bad_content"
        );
        assert_eq!(
            code(AutoEqLibrary::import(b"nope").unwrap_err()),
            "bad_content"
        );
        let ok = AutoEqFetchResponse {
            status: 200,
            body: DB.as_bytes().to_vec(),
            ..Default::default()
        };
        assert_eq!(
            AutoEqLibrary::download("https://example.invalid/db.json", ok)
                .unwrap()
                .info()
                .entry_count,
            2
        );
        let missing = AutoEqFetchResponse {
            status: 404,
            ..Default::default()
        };
        assert_eq!(
            code(AutoEqLibrary::download("u", missing).unwrap_err()),
            "http_status"
        );
        let limited = AutoEqFetchResponse {
            status: 429,
            retry_after_s: Some(120),
            ..Default::default()
        };
        match AutoEqLibrary::download("u", limited).unwrap_err() {
            DspiError::AutoEq {
                code,
                retry_after_s,
                ..
            } => {
                assert_eq!(code, "rate_limited");
                assert_eq!(retry_after_s, Some(120));
            }
            other => panic!("{other:?}"),
        }
        let html = AutoEqFetchResponse {
            status: 200,
            body: b"<html>".to_vec(),
            ..Default::default()
        };
        assert_eq!(
            code(AutoEqLibrary::download("u", html).unwrap_err()),
            "bad_content"
        );
    }

    #[test]
    fn wp_wb6k_favourites_round_trip() {
        let ids = vec!["a/b".to_string(), "c/d".to_string()];
        assert_eq!(
            autoeq_favourites_parse(autoeq_favourites_text(&ids).as_bytes()),
            ids
        );
        assert!(autoeq_favourites_parse(b"not json").is_empty());
    }
}
