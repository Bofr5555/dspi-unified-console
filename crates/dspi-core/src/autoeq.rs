//! AutoEQ database upkeep: update, rebuild from GitHub, import, Reset to
//! Built-in (WP CG6, the macOS Console's `AutoEQManager`).
//!
//! The core carries no HTTP client. The host implements [`AutoEqFetch`] with its
//! own stack (URLSession, OkHttp, QNetworkAccessManager) and hands back whatever
//! the server answered; reading the status and the rate-limit headers, walking
//! GitHub's listings, parsing the profiles and writing the file all happen here,
//! in `dspi_session::autoeq`, so every client behaves the same.
//!
//! None of this needs a device, so these are free functions, not methods of
//! [`crate::DspiCore`]. `db_path` is the host's own file for the user's database:
//! mobile platforms have no shared configuration directory. The built-in database
//! stays with the host (this library does not bundle it), so "Reset to Built-in"
//! removes the user's file and the host goes back to its own copy.
//!
//! The file changes only after a success and in one step. A rate limit, a cancel,
//! or a rebuild where most downloads failed leaves it as it was.
//!
//! # Threads
//!
//! [`autoeq_rebuild_start`] returns at once and works on its own threads. The
//! other calls block on the file or, for [`autoeq_update`], the request: call them
//! off the UI thread.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use dspi_session::autoeq::{self as session, AutoEqError, FetchError};

use crate::{DspiError, Result};

// -- WP P1AQ -- preview and apply to any channel pair (PRO1 T6)
mod pair;
pub use pair::*;

/// One GET the host is asked to make.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoEqFetchRequest {
    /// The URL, already percent-encoded.
    pub url: String,
    /// The `Accept` header to send, if any.
    pub accept: Option<String>,
    /// The `User-Agent` header to send. GitHub refuses a request without one.
    pub user_agent: String,
}

/// What the server answered, whatever the status: a 403 or a 500 is an answer.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutoEqFetchResponse {
    /// The HTTP status, after following redirects.
    pub status: u16,
    /// The body, as received. Pass it for an error status too.
    pub body: Vec<u8>,
    /// The `Retry-After` header in seconds; `None` if absent or a date.
    pub retry_after_s: Option<u32>,
    /// The `X-RateLimit-Remaining` header.
    pub rate_limit_remaining: Option<u32>,
    /// The `X-RateLimit-Reset` header, in Unix seconds.
    pub rate_limit_reset: Option<u64>,
}

/// The host's HTTP GET.
///
/// `get` blocks until the server has answered, following redirects, and is
/// called on the rebuild's threads, up to 20 at once, so it must be safe to call
/// concurrently. It must not wait on the UI thread if that thread may call a
/// blocking cancel. Give it a timeout (the Terminal uses 30 s): a cancel waits for
/// the requests in flight.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
pub trait AutoEqFetch: Send + Sync {
    /// Make the request. An error means no answer came (no network, a timeout);
    /// use [`DspiError::Transfer`] with words for the user.
    fn get(&self, request: AutoEqFetchRequest) -> Result<AutoEqFetchResponse>;
}

/// What the "Update Database" dialog shows about the database in place.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoEqDatabaseInfo {
    /// A user database is at `db_path`, which is what decides whether "Reset to
    /// Built-in" is offered. True for a damaged file too, so it can be reset.
    pub has_user_database: bool,
    /// Why the file could not be read; `None` if it could, or there is none.
    pub problem: Option<String>,
    /// The file's format version; 0 with no readable database.
    pub version: i32,
    /// When it was generated, ISO 8601 as the file has it; empty if unknown. An
    /// imported file can say anything here: show it as text if it does not parse.
    pub generated_at: String,
    /// Profiles it holds.
    pub entry_count: u32,
}

impl From<&session::Database> for AutoEqDatabaseInfo {
    fn from(db: &session::Database) -> Self {
        AutoEqDatabaseInfo {
            has_user_database: true,
            problem: None,
            version: db.version,
            generated_at: db.generated_at.clone(),
            entry_count: count(db.entries.len()),
        }
    }
}

/// Where a rebuild has got to, from [`AutoEqRebuild::poll`].
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoEqRebuildProgress {
    /// 0.0 to 1.0.
    pub fraction: f32,
    /// A line for the progress dialog; once finished, the result in words.
    pub status: String,
    /// The rebuild is over, one way or the other; the fields below are final.
    pub finished: bool,
    /// The database was written. It may still have parts missing: see
    /// `failed_profiles` and `failed_listings`. On a failure the file was left
    /// as it was.
    pub succeeded: bool,
    /// Once finished without success, why: `cancelled`, or an
    /// [`AutoEq`](DspiError::AutoEq) code; empty otherwise.
    pub code: String,
    /// With `code` `rate_limited`: seconds until it is worth trying again, when
    /// the server said.
    pub retry_after_s: Option<u32>,
    /// Profiles in the database written.
    pub entries: u32,
    /// Profiles that should have come and did not. Tell the person.
    pub failed_profiles: u32,
    /// Directory listings that failed, whose profiles were never seen. Tell the
    /// person.
    pub failed_listings: u32,
    /// Folders with no usable profile, which is normal upstream.
    pub skipped_profiles: u32,
}

/// A rebuild running on the core's threads. Poll it from a UI timer; dropping it
/// cancels it, without waiting for the requests in flight.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct AutoEqRebuild {
    handle: Mutex<session::RebuildHandle>,
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl AutoEqRebuild {
    /// The newest progress. Does not block.
    pub fn poll(&self) -> AutoEqRebuildProgress {
        let mut handle = self
            .handle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        progress_record(handle.poll())
    }

    /// Stop at the next batch of profiles; nothing is written. Does not block:
    /// keep polling until `finished`.
    pub fn cancel(&self) {
        self.handle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancel();
    }
}

impl AutoEqRebuild {
    /// Cancel, and block until the fetch is called no more. For the C ABI, whose
    /// callback's `user_data` must not be used after the handle is freed.
    pub fn cancel_and_wait(&self) {
        self.handle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancel_and_wait();
    }
}

/// The database at `db_path`: for the "Update Database" dialog. Reads the file.
/// Never fails: a file it cannot read is reported in
/// [`AutoEqDatabaseInfo::problem`].
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn autoeq_database_info(db_path: String) -> AutoEqDatabaseInfo {
    let path = Path::new(&db_path);
    if !path.exists() {
        return AutoEqDatabaseInfo::default();
    }
    match session::load_user_database(path) {
        Ok(db) => AutoEqDatabaseInfo::from(&db),
        Err(e) => AutoEqDatabaseInfo {
            has_user_database: true,
            problem: Some(e.to_string()),
            ..Default::default()
        },
    }
}

/// Replace the database at `db_path` with an imported file's bytes, which the host
/// reads from its own file picker. Refused, with the file left alone, unless they
/// are a database with at least one profile: [`DspiError::AutoEq`] with code
/// `bad_content`, or [`DspiError::Io`] if the file cannot be written.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn autoeq_import(db_path: String, data: Vec<u8>) -> Result<AutoEqDatabaseInfo> {
    let db = session::import_bytes(&data, Path::new(&db_path)).map_err(error)?;
    Ok(AutoEqDatabaseInfo::from(&db))
}

/// Download a ready-made database from `url` and replace the one at `db_path`, once
/// it is known to be one. Blocks for the request. Errors as for [`autoeq_import`],
/// and [`DspiError::AutoEq`] with a fetch's code.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn autoeq_update(
    db_path: String,
    url: String,
    fetch: Arc<dyn AutoEqFetch>,
) -> Result<AutoEqDatabaseInfo> {
    let db =
        session::update_from_url(&HostFetch(fetch), &url, Path::new(&db_path)).map_err(error)?;
    Ok(AutoEqDatabaseInfo::from(&db))
}

/// Reset to Built-in: remove the database at `db_path`, and reload the host's own
/// copy. True if there was one to remove.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn autoeq_reset_to_builtin(db_path: String) -> Result<bool> {
    session::remove_user_database(Path::new(&db_path)).map_err(error)
}

/// What a rebuild will reach for and write, to show before starting one. Nothing
/// touches the network unless someone reads this and says yes.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn autoeq_rebuild_warning(db_path: String) -> String {
    session::rebuild_warning_for(Path::new(&db_path))
}

/// Start rebuilding the database at `db_path` from the AutoEQ project on GitHub.
/// Returns at once. A full rebuild is thousands of requests, and unauthenticated
/// listings count against GitHub's limit of 60 an hour.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn autoeq_rebuild_start(db_path: String, fetch: Arc<dyn AutoEqFetch>) -> Arc<AutoEqRebuild> {
    let handle = session::rebuild_to(Box::new(HostFetch(fetch)), PathBuf::from(db_path));
    Arc::new(AutoEqRebuild {
        handle: Mutex::new(handle),
    })
}

/// The host's fetch, as the session's.
struct HostFetch(Arc<dyn AutoEqFetch>);

impl session::Fetch for HostFetch {
    fn get(
        &self,
        request: &session::FetchRequest,
    ) -> std::result::Result<session::FetchResponse, String> {
        let response = self
            .0
            .get(AutoEqFetchRequest {
                url: request.url.clone(),
                accept: request.accept.clone(),
                user_agent: request.user_agent.clone(),
            })
            // Words for the user: not "transfer failed: ", which a C or foreign
            // host's message would otherwise be wrapped in.
            .map_err(|e| match e {
                DspiError::Transfer { msg } => msg,
                other => other.to_string(),
            })?;
        Ok(session::FetchResponse {
            status: response.status,
            body: response.body,
            retry_after_s: response.retry_after_s,
            rate_limit_remaining: response.rate_limit_remaining,
            rate_limit_reset: response.rate_limit_reset,
        })
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The stable code of a failure, for [`DspiError::AutoEq`].
fn code(e: &AutoEqError) -> &'static str {
    match e {
        AutoEqError::Fetch(FetchError::RateLimited { .. }) => "rate_limited",
        AutoEqError::Fetch(FetchError::Status(_)) => "http_status",
        AutoEqError::Fetch(FetchError::Network(_)) => "network",
        AutoEqError::Fetch(FetchError::BadContent(_)) | AutoEqError::Unreadable(_) => "bad_content",
        AutoEqError::Missing(_) => "missing",
        AutoEqError::Unwritable { .. } => "write_failed",
        AutoEqError::Cancelled => "cancelled",
        AutoEqError::NoProfiles { .. } => "no_profiles",
        AutoEqError::TooManyFailures { .. } => "too_many_failures",
        AutoEqError::Stopped => "stopped",
    }
}

fn retry_after(e: &AutoEqError) -> Option<u32> {
    match e {
        AutoEqError::Fetch(FetchError::RateLimited { retry_after_s }) => *retry_after_s,
        _ => None,
    }
}

fn error(e: AutoEqError) -> DspiError {
    match e {
        AutoEqError::Unwritable { .. } => DspiError::Io { msg: e.to_string() },
        e => DspiError::AutoEq {
            code: code(&e).to_string(),
            msg: e.to_string(),
            retry_after_s: retry_after(&e),
        },
    }
}

fn progress_record(p: &session::RebuildProgress) -> AutoEqRebuildProgress {
    let mut record = AutoEqRebuildProgress {
        fraction: p.fraction,
        status: p.status.clone(),
        finished: p.done.is_some(),
        ..Default::default()
    };
    match &p.done {
        None => {}
        Some(Ok(summary)) => {
            record.succeeded = true;
            record.entries = count(summary.entries);
            record.failed_profiles = count(summary.failed_profiles);
            record.failed_listings = count(summary.failed_listings);
            record.skipped_profiles = count(summary.skipped_profiles);
        }
        Some(Err(e)) => {
            record.code = code(e).to_string();
            record.retry_after_s = retry_after(e);
        }
    }
    record
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg6 {
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    use super::*;

    const API: &str = "https://api.github.com/repos/jaakkopasanen/AutoEq/contents/results/";
    const RAW: &str = "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/master/results/";
    const DB: &str = r#"{"version":1,"generatedAt":"2026-01-02T03:04:05.000Z","entryCount":1,
        "entries":[{"id":"o/Sony A","manufacturer":"Sony","model":"A","filters":[]}]}"#;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::current_exe()
            .unwrap()
            .with_file_name("test-scratch")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("db.json")
    }

    fn text(p: &Path) -> String {
        p.display().to_string()
    }

    fn ok(status: u16, body: &str) -> Result<AutoEqFetchResponse> {
        Ok(AutoEqFetchResponse {
            status,
            body: body.as_bytes().to_vec(),
            ..Default::default()
        })
    }

    /// A scripted host: a URL nobody scripted is a 404.
    #[derive(Default)]
    struct Script {
        answers: HashMap<String, Result<AutoEqFetchResponse>>,
        requests: Mutex<Vec<AutoEqFetchRequest>>,
    }

    impl Script {
        fn on(mut self, url: &str, reply: Result<AutoEqFetchResponse>) -> Self {
            self.answers.insert(url.to_string(), reply);
            self
        }
    }

    impl AutoEqFetch for Script {
        fn get(&self, request: AutoEqFetchRequest) -> Result<AutoEqFetchResponse> {
            self.requests.lock().unwrap().push(request.clone());
            self.answers
                .get(&request.url)
                .cloned()
                .unwrap_or_else(|| ok(404, ""))
        }
    }

    /// One source, one target, these headphones.
    fn github(headphones: &[(&str, Result<AutoEqFetchResponse>)]) -> Script {
        let names: Vec<String> = headphones
            .iter()
            .map(|(n, _)| format!(r#"{{"name":"{n}","type":"dir"}}"#))
            .collect();
        let mut script = Script::default()
            .on(
                &format!("{API}oratory1990"),
                ok(200, r#"[{"name":"t","type":"dir"}]"#),
            )
            .on(
                &format!("{API}oratory1990/t"),
                ok(200, &format!("[{}]", names.join(","))),
            );
        for (name, reply) in headphones {
            let h = name.replace(' ', "%20");
            let url = format!("{RAW}oratory1990/t/{h}/{h}%20ParametricEQ.txt");
            script = script.on(&url, reply.clone());
        }
        script
    }

    fn profile() -> Result<AutoEqFetchResponse> {
        ok(
            200,
            "Preamp: -6 dB\nFilter 1: ON PK Fc 105 Hz Gain 3 dB Q 0.7\n",
        )
    }

    fn run_to_end(rebuild: &AutoEqRebuild) -> AutoEqRebuildProgress {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let p = rebuild.poll();
            if p.finished {
                return p;
            }
            assert!(Instant::now() < deadline, "no result: {p:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_rebuild_with_a_partial_failure_reports_it_through_poll() {
        let db = scratch("core-cg6-partial");
        let fetch = github(&[
            ("Sony A", profile()),
            ("Sony B", profile()),
            ("Sony C", profile()),
            (
                "Sony D",
                Err(DspiError::Transfer {
                    msg: "timed out".into(),
                }),
            ),
            ("Sony E", ok(404, "")),
        ]);
        let p = run_to_end(&autoeq_rebuild_start(text(&db), Arc::new(fetch)));
        assert!(p.succeeded, "{p:?}");
        assert_eq!(p.code, "");
        assert_eq!(
            (p.entries, p.failed_profiles, p.skipped_profiles),
            (3, 1, 1)
        );
        assert_eq!(p.fraction, 1.0);

        let info = autoeq_database_info(text(&db));
        assert!(info.has_user_database);
        assert_eq!(info.entry_count, 3);
        assert_eq!(info.version, 1);
        assert!(info.generated_at.ends_with('Z'), "{info:?}");
    }

    #[test]
    fn a_rate_limited_rebuild_keeps_the_file_and_says_when() {
        let db = scratch("core-cg6-limit");
        std::fs::write(&db, DB).unwrap();
        let limit = Ok(AutoEqFetchResponse {
            status: 403,
            rate_limit_remaining: Some(0),
            retry_after_s: Some(1200),
            ..Default::default()
        });
        let fetch = Script::default().on(&format!("{API}oratory1990"), limit);
        let p = run_to_end(&autoeq_rebuild_start(text(&db), Arc::new(fetch)));
        assert!(!p.succeeded, "{p:?}");
        assert_eq!(p.code, "rate_limited");
        assert_eq!(p.retry_after_s, Some(1200));
        assert!(p.status.contains("about 20 minutes"), "{p:?}");
        assert_eq!(std::fs::read_to_string(&db).unwrap(), DB);
    }

    #[test]
    fn the_host_is_asked_with_the_headers_github_wants() {
        let db = scratch("core-cg6-headers");
        let fetch = Arc::new(github(&[("Sony A", profile())]));
        let hosted: Arc<dyn AutoEqFetch> = fetch.clone();
        run_to_end(&autoeq_rebuild_start(text(&db), hosted));
        let requests = fetch.requests.lock().unwrap();
        assert!(requests.iter().all(|r| !r.user_agent.is_empty()));
        let listing = requests.iter().find(|r| r.url.starts_with(API)).unwrap();
        assert_eq!(
            listing.accept.as_deref(),
            Some("application/vnd.github.v3+json")
        );
        let download = requests.iter().find(|r| r.url.starts_with(RAW)).unwrap();
        assert_eq!(download.accept, None);
    }

    #[test]
    fn cancel_reports_cancelled_and_cancel_and_wait_ends_the_calls() {
        let db = scratch("core-cg6-cancel");
        std::fs::write(&db, DB).unwrap();
        let rebuild = autoeq_rebuild_start(text(&db), Arc::new(Script::default()));
        rebuild.cancel();
        rebuild.cancel_and_wait();
        let p = rebuild.poll();
        assert!(p.finished, "{p:?}");
        // It may have reached its end before the cancel; offline either way.
        assert!(!p.succeeded, "{p:?}");
        assert_eq!(std::fs::read_to_string(&db).unwrap(), DB);
    }

    #[test]
    fn import_reports_the_database_and_refuses_what_is_not_one() {
        let db = scratch("core-cg6-import");
        let info = autoeq_import(text(&db), DB.as_bytes().to_vec()).unwrap();
        assert_eq!(info.entry_count, 1);
        assert_eq!(info.generated_at, "2026-01-02T03:04:05.000Z");
        assert_eq!(autoeq_database_info(text(&db)), info);

        let refused = autoeq_import(text(&db), b"{}".to_vec()).unwrap_err();
        let DspiError::AutoEq { code, .. } = refused else {
            panic!("{refused:?}");
        };
        assert_eq!(code, "bad_content");
        assert_eq!(std::fs::read_to_string(&db).unwrap(), DB);
    }

    #[test]
    fn a_file_that_cannot_be_written_is_an_io_error() {
        let db = scratch("core-cg6-io");
        // The target is a directory.
        std::fs::create_dir_all(&db).unwrap();
        let e = autoeq_import(text(&db), DB.as_bytes().to_vec()).unwrap_err();
        assert!(matches!(e, DspiError::Io { .. }), "{e:?}");
    }

    #[test]
    fn update_maps_each_failure_to_its_code() {
        let db = scratch("core-cg6-update");
        std::fs::write(&db, "old").unwrap();
        let url = "https://example.test/db.json";
        let update = |reply| {
            autoeq_update(
                text(&db),
                url.into(),
                Arc::new(Script::default().on(url, reply)),
            )
        };
        let code_of = |r: Result<AutoEqDatabaseInfo>| match r {
            Err(DspiError::AutoEq {
                code,
                retry_after_s,
                ..
            }) => (code, retry_after_s),
            other => panic!("{other:?}"),
        };

        let limit = Ok(AutoEqFetchResponse {
            status: 429,
            retry_after_s: Some(30),
            ..Default::default()
        });
        assert_eq!(code_of(update(limit)), ("rate_limited".into(), Some(30)));
        assert_eq!(code_of(update(ok(500, ""))), ("http_status".into(), None));
        assert_eq!(
            code_of(update(ok(200, "<html>"))),
            ("bad_content".into(), None)
        );
        let offline = Err(DspiError::Transfer {
            msg: "no route to host".into(),
        });
        let Err(DspiError::AutoEq { code, msg, .. }) = update(offline) else {
            panic!("expected a failure");
        };
        assert_eq!(code, "network");
        assert_eq!(msg, "no route to host", "the host's words, unwrapped");
        assert_eq!(std::fs::read_to_string(&db).unwrap(), "old");

        assert_eq!(update(ok(200, DB)).unwrap().entry_count, 1);
        assert_eq!(std::fs::read_to_string(&db).unwrap(), DB);
    }

    #[test]
    fn reset_removes_the_file_and_info_follows() {
        let db = scratch("core-cg6-reset");
        assert_eq!(
            autoeq_database_info(text(&db)),
            AutoEqDatabaseInfo::default()
        );
        assert_eq!(autoeq_reset_to_builtin(text(&db)), Ok(false));
        std::fs::write(&db, DB).unwrap();
        assert_eq!(autoeq_reset_to_builtin(text(&db)), Ok(true));
        assert!(!autoeq_database_info(text(&db)).has_user_database);
    }

    /// A damaged file still offers Reset to Built-in, which is the way out of it.
    #[test]
    fn a_damaged_file_is_reported_and_can_be_reset() {
        let db = scratch("core-cg6-damaged");
        std::fs::write(&db, "not json").unwrap();
        let info = autoeq_database_info(text(&db));
        assert!(info.has_user_database);
        assert!(info.problem.is_some(), "{info:?}");
        assert_eq!(autoeq_reset_to_builtin(text(&db)), Ok(true));
    }

    #[test]
    fn the_warning_names_the_hosts_and_the_file() {
        let w = autoeq_rebuild_warning("/data/app/db.json".into());
        assert!(w.contains("api.github.com"), "{w}");
        assert!(w.contains("/data/app/db.json"), "{w}");
    }
}
