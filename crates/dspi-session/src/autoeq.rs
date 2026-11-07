//! AutoEQ headphone correction profiles.
//!
//! The database is bundled so the feature works offline and on a machine that
//! has never seen the Console, following the same resolution order the Console
//! uses: a user-supplied copy wins, otherwise the built-in one. That ordering is
//! what lets someone drop in a newer export without waiting for an app release.
//!
//! Bundling costs about 5 MB in the binary, which is most of its size. The
//! `bundled-autoeq` feature is on by default and can be turned off for a build
//! where that matters, such as an armv7 Raspberry Pi; the user-supplied path
//! still works without it.

use serde::{Deserialize, Serialize};

/// Where a user-supplied database is looked for, under the platform's config
/// directory.
pub const USER_DB_NAME: &str = "autoeq_database.json";

/// Where the favourite profiles are kept, beside the database.
///
/// The Console keeps its favourites in `UserDefaults` under
/// `AutoEQ.FavoriteProfiles`, which has no cross-platform equivalent; a small
/// JSON file of ids in the same directory as the database is the terminal's
/// version of the same list.
pub const FAVOURITES_NAME: &str = "autoeq_favourites.json";

#[cfg(feature = "bundled-autoeq")]
const BUNDLED: &str = include_str!("../autoeq/autoeq_database.json");

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Database {
    #[serde(default)]
    pub version: i32,
    #[serde(default)]
    pub generated_at: String,
    /// The count the file states, which is not necessarily the number of
    /// entries it carries; the Console writes it and reads neither.
    #[serde(default)]
    pub entry_count: i32,
    #[serde(default)]
    pub entries: Vec<Entry>,
    /// Which copy this came from, so a stale bundle is visible rather than
    /// silently assumed current.
    #[serde(skip)]
    pub origin: Origin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Origin {
    #[default]
    Bundled,
    User,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    #[serde(default)]
    pub manufacturer: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub form_factor: String,
    /// The trim the profile expects, which is almost always negative: the
    /// corrections boost, and without it the result clips.
    #[serde(default)]
    pub preamp: f32,
    #[serde(default)]
    pub filters: Vec<Filter>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Filter {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub freq: f32,
    #[serde(default)]
    pub q: f32,
    #[serde(default)]
    pub gain: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AutoEqError {
    #[error("no AutoEQ database available; supply one at {0}")]
    Missing(String),

    #[error("could not read the AutoEQ database: {0}")]
    Unreadable(String),

    #[error("could not write {path}: {reason}")]
    Unwritable { path: String, reason: String },

    /// A single download failed, or a rate limit stopped a rebuild.
    #[error("{0}")]
    Fetch(FetchError),

    /// The person stopped the rebuild. Nothing was written.
    #[error("Cancelled.")]
    Cancelled,

    /// The rebuild ended with no profile to write. Nothing was written.
    #[error("no profiles could be downloaded: {reason}")]
    NoProfiles { reason: String },

    /// Too much of the rebuild failed for the result to replace a database.
    /// Nothing was written.
    #[error("{failed} of {total} downloads failed, so the database was left as it was")]
    TooManyFailures { failed: usize, total: usize },

    /// The rebuild's thread died without a result.
    #[error("the rebuild stopped unexpectedly")]
    Stopped,
}

/// The directory the user's database and favourites live in.
///
/// The platform's configuration directory: `$XDG_CONFIG_HOME` or `~/.config`
/// on Linux, `~/Library/Application Support` on macOS, `%APPDATA%` on
/// Windows. Reading `HOME` by hand sent Windows users who have it set (Git
/// Bash, MSYS2, Cygwin) to `%HOME%\.config` instead of `%APPDATA%`.
pub fn config_dir() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("dspi-term")
}

/// Where a user-supplied database would live.
pub fn user_db_path() -> std::path::PathBuf {
    config_dir().join(USER_DB_NAME)
}

/// Whether a user-supplied database is in place, which is what decides whether
/// "Reset to Built-in" is offered.
pub fn has_user_database() -> bool {
    user_db_path().exists()
}

pub fn favourites_path() -> std::path::PathBuf {
    config_dir().join(FAVOURITES_NAME)
}

/// The favourite profile ids, oldest first. A missing or unreadable file is an
/// empty list: a favourites file is a convenience, never a reason to refuse to
/// open the browser.
pub fn load_favourites() -> Vec<String> {
    std::fs::read_to_string(favourites_path())
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok())
        .unwrap_or_default()
}

pub fn save_favourites(ids: &[String]) -> Result<(), AutoEqError> {
    let path = favourites_path();
    write_config(
        &path,
        &serde_json::to_string_pretty(ids).unwrap_or_default(),
    )
}

fn write_config(path: &std::path::Path, text: &str) -> Result<(), AutoEqError> {
    write_atomic(path, text.as_bytes())
}

/// Write beside the target, then rename over it: a crash or a full disk leaves
/// the old file whole, never half of the new one. The rename replaces an existing
/// file on Windows too.
fn write_atomic(path: &std::path::Path, data: &[u8]) -> Result<(), AutoEqError> {
    use std::io::Write as _;
    let failed = |e: std::io::Error, at: &std::path::Path| AutoEqError::Unwritable {
        path: at.display().to_string(),
        reason: e.to_string(),
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| failed(e, dir))?;
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp-{}", std::process::id()));
    let temp = path.with_file_name(name);
    let written = std::fs::File::create(&temp).and_then(|mut f| {
        f.write_all(data)?;
        f.sync_all()
    });
    if let Err(e) = written.and_then(|()| std::fs::rename(&temp, path)) {
        let _ = std::fs::remove_file(&temp);
        return Err(failed(e, path));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Applying a profile
// ---------------------------------------------------------------------------

/// One band a profile wants written, already mapped onto a firmware type.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedBand {
    /// 0-based index into the channel's PEQ bank.
    pub band: u8,
    pub filter_type: dspi_proto::FilterType,
    pub freq: f32,
    pub q: f32,
    pub gain_db: f32,
}

/// What applying a profile to one channel comes to, worked out before anything
/// is written so the caller can say what will not survive.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ApplyPlan {
    pub preamp_db: f32,
    pub bands: Vec<PlannedBand>,
    /// Bands to clear: everything the profile does not use. Without this,
    /// whatever was there before survives underneath the correction.
    pub cleared: Vec<u8>,
    /// Shapes this build does not know, named once each.
    pub unsupported: Vec<String>,
    /// Bands past the end of the device's bank.
    pub dropped: usize,
}

impl Entry {
    /// The Console's `displayName`.
    pub fn display_name(&self) -> String {
        if self.model.is_empty() {
            self.manufacturer.clone()
        } else {
            format!("{} {}", self.manufacturer, self.model)
        }
    }

    /// The Console's `sourceDisplayName`: the capsule's text.
    pub fn source_label(&self) -> &str {
        match self.source.as_str() {
            "oratory1990" => "oratory1990",
            "crinacle" => "Crinacle",
            "rtings" => "Rtings",
            "innerfidelity" => "InnerFidelity",
            "headphone.com" => "Headphone.com",
            other => other,
        }
    }

    /// Work out what applying this profile to a channel with `max_bands` bands
    /// would do.
    pub fn plan(&self, max_bands: u8) -> ApplyPlan {
        let mut plan = ApplyPlan {
            preamp_db: self.preamp,
            dropped: self.filters.len().saturating_sub(max_bands as usize),
            ..Default::default()
        };
        for (i, f) in self.filters.iter().enumerate() {
            if i as u8 >= max_bands {
                break;
            }
            match filter_type(&f.kind) {
                Some(kind) => plan.bands.push(PlannedBand {
                    band: i as u8,
                    filter_type: kind,
                    freq: f.freq,
                    q: f.q,
                    gain_db: f.gain,
                }),
                None => {
                    if !plan.unsupported.contains(&f.kind) {
                        plan.unsupported.push(f.kind.clone());
                    }
                }
            }
        }
        plan.cleared = (self.filters.len().min(max_bands as usize) as u8..max_bands).collect();
        plan
    }
}

/// Load the database, preferring a user-supplied copy.
pub fn load() -> Result<Database, AutoEqError> {
    let path = user_db_path();
    if path.exists() {
        return load_user_database(&path);
    }

    #[cfg(feature = "bundled-autoeq")]
    {
        let mut db: Database =
            serde_json::from_str(BUNDLED).map_err(|e| AutoEqError::Unreadable(e.to_string()))?;
        db.origin = Origin::Bundled;
        Ok(db)
    }
    #[cfg(not(feature = "bundled-autoeq"))]
    {
        Err(AutoEqError::Missing(path.display().to_string()))
    }
}

/// Load the database at `db_path`, for a host that keeps it somewhere of its own.
pub fn load_user_database(db_path: &std::path::Path) -> Result<Database, AutoEqError> {
    let text =
        std::fs::read_to_string(db_path).map_err(|e| AutoEqError::Unreadable(e.to_string()))?;
    let mut db: Database =
        serde_json::from_str(&text).map_err(|e| AutoEqError::Unreadable(e.to_string()))?;
    db.origin = Origin::User;
    Ok(db)
}

impl Database {
    /// Find profiles matching a query.
    ///
    /// Matching is on manufacturer, model and id together, so "sennheiser hd600"
    /// works as well as "hd600", and the terms may appear in any order.
    pub fn search(&self, query: &str) -> Vec<&Entry> {
        let terms: Vec<String> = query
            .split_whitespace()
            .map(|t| t.to_ascii_lowercase())
            .collect();
        if terms.is_empty() {
            return Vec::new();
        }

        let mut hits: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| {
                let hay = format!("{} {} {}", e.manufacturer, e.model, e.id).to_ascii_lowercase();
                terms.iter().all(|t| hay.contains(t))
            })
            .collect();

        // An exact model match should not be buried under longer names that
        // merely contain it.
        hits.sort_by_key(|e| (e.model.len(), e.id.clone()));
        hits
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }
}

/// Map an AutoEQ filter shape onto a firmware filter type.
///
/// An unrecognised shape returns `None` rather than being forced into a peaking
/// filter, because silently changing a correction's shape produces a curve that
/// is wrong in a way nobody can see.
pub fn filter_type(kind: &str) -> Option<dspi_proto::FilterType> {
    use dspi_proto::FilterType as F;
    Some(match kind.to_ascii_uppercase().as_str() {
        "PK" | "PEAKING" => F::Peaking,
        "LS" | "LOWSHELF" | "LSC" => F::LowShelf,
        "HS" | "HIGHSHELF" | "HSC" => F::HighShelf,
        "LP" | "LOWPASS" | "LPQ" => F::LowPass,
        "HP" | "HIGHPASS" | "HPQ" => F::HighPass,
        "NO" | "NOTCH" => F::Notch,
        _ => return None,
    })
}

/// The name the database file gives a firmware filter type, which is the
/// Console's `filterTypes` table read the other way round.
fn kind_name(t: dspi_proto::FilterType) -> Option<&'static str> {
    use dspi_proto::FilterType as F;
    Some(match t {
        F::Peaking => "peaking",
        F::LowShelf => "lowShelf",
        F::HighShelf => "highShelf",
        F::LowPass => "lowPass",
        F::HighPass => "highPass",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Update Database
// ---------------------------------------------------------------------------

/// The `Accept` header the GitHub contents API asks for.
pub const GITHUB_ACCEPT: &str = "application/vnd.github.v3+json";

/// The `User-Agent` a fetch sends: GitHub refuses a request without one, and
/// not every HTTP stack adds its own.
pub const FETCH_USER_AGENT: &str = "DSPi-Console";

/// Most bytes accepted as a database, on import or download. The bundled one is
/// about 5 MB.
pub const MAX_DATABASE_BYTES: usize = 64 * 1024 * 1024;

/// Profiles fetched at once, as the Console's batches do: one at a time, a
/// rebuild takes twenty times as long.
const BATCH_SIZE: usize = 20;

/// One GET the host is asked to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchRequest {
    pub url: String,
    /// The `Accept` header to send, if any.
    pub accept: Option<String>,
    /// The `User-Agent` header to send.
    pub user_agent: String,
}

impl FetchRequest {
    fn new(url: String, accept: Option<&str>) -> Self {
        FetchRequest {
            url,
            accept: accept.map(str::to_string),
            user_agent: FETCH_USER_AGENT.to_string(),
        }
    }
}

/// What the server answered, whatever the status. Reading the status and the
/// rate-limit headers is the core's job, so that every host treats them alike.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FetchResponse {
    pub status: u16,
    pub body: Vec<u8>,
    /// The `Retry-After` header, in seconds.
    pub retry_after_s: Option<u32>,
    /// The `X-RateLimit-Remaining` header.
    pub rate_limit_remaining: Option<u32>,
    /// The `X-RateLimit-Reset` header, in Unix seconds.
    pub rate_limit_reset: Option<u64>,
}

/// Fetching a URL, supplied by the host, so the core carries no HTTP client and
/// a rebuild can be driven by a test without a network.
///
/// `get` blocks its thread for the request and returns once the server has
/// answered, following redirects. It runs on the rebuild's own threads, up to
/// [`BATCH_SIZE`] at once, so it must be safe to call concurrently and should
/// have its own timeout: a cancel waits for the requests in flight. `Err` means
/// no answer came (no network, a timeout), in words for the user. Any answer at
/// all, a 403 or a 500 included, is `Ok`.
pub trait Fetch: Send + Sync {
    fn get(&self, request: &FetchRequest) -> Result<FetchResponse, String>;
}

/// Why a fetch gave nothing usable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    /// The server says to stop asking for now.
    #[error("{}", rate_limit_text(*.retry_after_s))]
    RateLimited {
        /// How long until it is worth trying again, when the server said.
        retry_after_s: Option<u32>,
    },

    #[error("the server answered with status {0}")]
    Status(u16),

    /// No answer came: the host's words.
    #[error("{0}")]
    Network(String),

    #[error("the download was not usable: {0}")]
    BadContent(String),
}

fn rate_limit_text(retry_after_s: Option<u32>) -> String {
    match retry_after_s.map(|s| s.div_ceil(60).max(1)) {
        Some(1) => "GitHub's rate limit was reached; try again in about a minute.".to_string(),
        Some(minutes) => {
            format!("GitHub's rate limit was reached; try again in about {minutes} minutes.")
        }
        None => "GitHub's rate limit was reached; try again later.".to_string(),
    }
}

/// A 200's body, or what the answer means instead.
///
/// GitHub signals its limits in two ways: 429, or 403 with
/// `X-RateLimit-Remaining: 0`, a `Retry-After`, or the words in the body (a host
/// that cannot pass headers still gets this right). A 403 with none of those is
/// an ordinary refusal: retrying later would not help.
fn read_response(response: FetchResponse, now: u64) -> Result<Vec<u8>, FetchError> {
    let FetchResponse {
        status,
        body,
        retry_after_s,
        rate_limit_remaining,
        rate_limit_reset,
    } = response;
    if status == 200 {
        return Ok(body);
    }
    let limited = status == 429
        || (status == 403
            && (rate_limit_remaining == Some(0)
                || retry_after_s.is_some()
                || String::from_utf8_lossy(&body)
                    .to_ascii_lowercase()
                    .contains("rate limit")));
    if !limited {
        return Err(FetchError::Status(status));
    }
    let until_reset =
        rate_limit_reset.map(|reset| u32::try_from(reset.saturating_sub(now)).unwrap_or(u32::MAX));
    Err(FetchError::RateLimited {
        retry_after_s: retry_after_s.or(until_reset),
    })
}

/// Make one request. A host callback that panics, which is how a foreign
/// exception arrives here, costs that one request: a rebuild thread that died
/// instead would leave the progress dialog waiting forever.
fn fetch_bytes(fetch: &dyn Fetch, request: &FetchRequest) -> Result<Vec<u8>, FetchError> {
    let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fetch.get(request)))
        .unwrap_or_else(|_| Err("the fetch callback failed".to_string()))
        .map_err(FetchError::Network)?;
    read_response(response, unix_now())
}

fn fetch_text(fetch: &dyn Fetch, request: &FetchRequest) -> Result<String, FetchError> {
    String::from_utf8(fetch_bytes(fetch, request)?)
        .map_err(|_| FetchError::BadContent("it was not text".to_string()))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// ISO 8601 in UTC with milliseconds, which is what the Console writes in
/// `generatedAt` and reads back for "Current database:". The days-to-date step is
/// Howard Hinnant's `civil_from_days`.
fn iso8601_utc(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64;
    let second_of_day = unix_seconds % 86_400;
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        second_of_day / 3600,
        second_of_day / 60 % 60,
        second_of_day % 60
    )
}

/// The AutoEQ sources, in the Console's priority order: when two sources
/// measure the same headphone the lower number wins.
const SOURCES: [(&str, &str, u8); 5] = [
    ("oratory1990", "oratory1990", 1),
    ("crinacle", "crinacle", 2),
    ("Rtings", "rtings", 3),
    ("Innerfidelity", "innerfidelity", 3),
    ("Headphone.com Legacy", "headphone.com", 4),
];

const REPO: &str = "jaakkopasanen/AutoEq";

/// What a finished rebuild came to. A rebuild can succeed with parts missing,
/// and this is how the person hears of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RebuildSummary {
    /// Profiles in the database written.
    pub entries: usize,
    /// Profiles that should have come and did not: no answer, a server error,
    /// or not text.
    pub failed_profiles: usize,
    /// Directory listings that failed the same way, so whatever they hold was
    /// never seen.
    pub failed_listings: usize,
    /// Folders with no usable profile in them (a 404, or nothing a filter could
    /// be read from). Normal in the upstream repository, and not a failure.
    pub skipped_profiles: usize,
}

impl std::fmt::Display for RebuildSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.entries)?;
        if self.failed_profiles > 0 || self.failed_listings > 0 {
            write!(
                f,
                " ({} profiles and {} folders could not be downloaded)",
                self.failed_profiles, self.failed_listings
            )?;
        }
        Ok(())
    }
}

/// Where the rebuild has got to, for the progress dialog.
#[derive(Debug, Clone, PartialEq)]
pub struct RebuildProgress {
    pub fraction: f32,
    pub status: String,
    /// Set once, at the end: what was written, or why nothing was.
    pub done: Option<Result<RebuildSummary, AutoEqError>>,
}

/// A rebuild running on its own thread.
///
/// The interface polls [`RebuildHandle::poll`] each tick rather than blocking:
/// a full rebuild is thousands of requests and takes minutes, and an interface
/// that stops answering for minutes is broken however good the result is.
pub struct RebuildHandle {
    rx: std::sync::mpsc::Receiver<RebuildProgress>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    latest: RebuildProgress,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl RebuildHandle {
    /// The newest progress, or the last one seen if nothing new arrived.
    pub fn poll(&mut self) -> &RebuildProgress {
        loop {
            match self.rx.try_recv() {
                Ok(p) => self.latest = p,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // The thread sends `done` last, so without one it died.
                    if self.latest.done.is_none() {
                        self.latest.done = Some(Err(AutoEqError::Stopped));
                        self.latest.status = AutoEqError::Stopped.to_string();
                    }
                    break;
                }
            }
        }
        &self.latest
    }

    /// Stop at the next batch of profiles.
    pub fn cancel(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Cancel, and wait for the thread to end, so that the [`Fetch`] is called no
    /// more. Blocks for the requests in flight, so it deadlocks if the fetch is
    /// waiting on the calling thread; off the UI thread, then.
    pub fn cancel_and_wait(&mut self) {
        self.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for RebuildHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// What the rebuild will fetch, said in full before it is started.
///
/// Nothing in this program touches the network unless someone reads this and
/// says yes.
pub fn rebuild_warning() -> String {
    rebuild_warning_for(&user_db_path())
}

/// [`rebuild_warning`], for a database kept at `db_path`.
pub fn rebuild_warning_for(db_path: &std::path::Path) -> String {
    format!(
        "You are about to rebuild the AutoEQ database by downloading all profiles \
         from GitHub.\n\nThis requires an internet connection and may take several \
         minutes. It reads https://api.github.com/repos/{REPO}/contents to list the \
         profiles and https://raw.githubusercontent.com/{REPO} to download each one, \
         sends nothing about you or your device, and writes the result to {}.\n\nDo \
         you wish to proceed?",
        db_path.display()
    )
}

/// Start a rebuild of the user database on a background thread.
pub fn rebuild_from_github(fetch: Box<dyn Fetch>) -> RebuildHandle {
    rebuild_to(fetch, user_db_path())
}

/// Start a rebuild of the database at `db_path` on a background thread. The file
/// changes only when the rebuild succeeds, and in one step.
pub fn rebuild_to(fetch: Box<dyn Fetch>, db_path: std::path::PathBuf) -> RebuildHandle {
    let (tx, rx) = std::sync::mpsc::channel();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = std::sync::Arc::clone(&stop);
    let thread = std::thread::Builder::new()
        .name("dspi-autoeq-rebuild".into())
        .spawn(move || {
            let send = |fraction: f32, status: String, done: Option<RebuildDone>| {
                let _ = tx.send(RebuildProgress {
                    fraction,
                    status,
                    done,
                });
            };
            match run_rebuild(fetch.as_ref(), &flag, &db_path, &send) {
                Ok(s) => send(
                    1.0,
                    format!("Complete! {} profiles.", s.entries),
                    Some(Ok(s)),
                ),
                Err(e) => send(0.0, e.to_string(), Some(Err(e))),
            }
        });
    let (thread, done) = match thread {
        Ok(t) => (Some(t), None),
        Err(e) => (
            None,
            Some(Err(AutoEqError::Unwritable {
                path: "the rebuild's thread".to_string(),
                reason: e.to_string(),
            })),
        ),
    };
    RebuildHandle {
        rx,
        stop,
        latest: RebuildProgress {
            fraction: 0.0,
            status: "Connecting to GitHub...".into(),
            done,
        },
        thread,
    }
}

type RebuildDone = Result<RebuildSummary, AutoEqError>;

/// One profile the listing promised.
struct Wanted {
    folder: &'static str,
    name: &'static str,
    priority: u8,
    target: String,
    headphone: String,
}

/// The rebuild, in the Console's three phases, with one difference on purpose.
///
/// The Console drops whatever fails and writes the rest, so a rate limit quietly
/// replaces a good database with a fraction of one. Here:
/// - a rate limit stops it at once: everything after it would fail too;
/// - a profile or listing that fails on its own is left out and counted in the
///   [`RebuildSummary`];
/// - when more than half of the profiles or of the listings failed, or nothing
///   came at all, nothing is written.
///
/// What is on disk changes only at the very end, and in one step.
fn run_rebuild(
    fetch: &dyn Fetch,
    stop: &std::sync::atomic::AtomicBool,
    out: &std::path::Path,
    send: &dyn Fn(f32, String, Option<RebuildDone>),
) -> RebuildDone {
    let stopped = || stop.load(std::sync::atomic::Ordering::Relaxed);
    let mut summary = RebuildSummary::default();
    let mut last_error = None;
    send(0.0, "Connecting to GitHub...".into(), None);
    send(0.0, "Discovering profiles...".into(), None);

    let mut listings = 0usize;
    // A listing that failed on its own is `None`; a 404 is an empty folder.
    let mut list =
        |path: String, summary: &mut RebuildSummary| -> Result<Option<Vec<String>>, AutoEqError> {
            listings = listings.saturating_add(1);
            match list_dir(fetch, &path) {
                Ok(names) => Ok(Some(names)),
                Err(e @ FetchError::RateLimited { .. }) => Err(AutoEqError::Fetch(e)),
                Err(FetchError::Status(404)) => Ok(None),
                Err(e) => {
                    summary.failed_listings = summary.failed_listings.saturating_add(1);
                    last_error = Some(e);
                    Ok(None)
                }
            }
        };

    let mut wanted: Vec<Wanted> = Vec::new();
    for (folder, name, priority) in SOURCES {
        if stopped() {
            return Err(AutoEqError::Cancelled);
        }
        let Some(targets) = list(format!("results/{folder}"), &mut summary)? else {
            continue;
        };
        for target in targets {
            if stopped() {
                return Err(AutoEqError::Cancelled);
            }
            let path = format!("results/{folder}/{target}");
            let Some(headphones) = list(path, &mut summary)? else {
                continue;
            };
            for headphone in headphones {
                wanted.push(Wanted {
                    folder,
                    name,
                    priority,
                    target: target.clone(),
                    headphone,
                });
            }
        }
        send(0.0, format!("Found {} profiles...", wanted.len()), None);
    }
    if wanted.is_empty() {
        return Err(AutoEqError::NoProfiles {
            reason: last_error.map_or_else(
                || "GitHub listed no profiles".to_string(),
                |e: FetchError| e.to_string(),
            ),
        });
    }
    if summary.failed_listings.saturating_mul(2) > listings {
        return Err(AutoEqError::TooManyFailures {
            failed: summary.failed_listings,
            total: listings,
        });
    }

    let total = wanted.len();
    send(0.0, format!("Downloading 0 / {total} profiles..."), None);

    // Keyed by manufacturer and model so two sources measuring the same
    // headphone collapse to the better-regarded one. Results are read in listing
    // order, so which of two equals wins does not depend on network timing.
    let mut best: std::collections::BTreeMap<String, (Entry, u8)> =
        std::collections::BTreeMap::new();
    let mut done = 0usize;
    for batch in wanted.chunks(BATCH_SIZE) {
        if stopped() {
            return Err(AutoEqError::Cancelled);
        }
        for (profile, result) in batch.iter().zip(fetch_batch(fetch, batch)) {
            let text = match result {
                Ok(text) => text,
                Err(e @ FetchError::RateLimited { .. }) => return Err(AutoEqError::Fetch(e)),
                Err(FetchError::Status(404)) => {
                    summary.skipped_profiles = summary.skipped_profiles.saturating_add(1);
                    continue;
                }
                Err(_) => {
                    summary.failed_profiles = summary.failed_profiles.saturating_add(1);
                    continue;
                }
            };
            let Some(entry) =
                profile_entry(&text, profile.name, &profile.target, &profile.headphone)
            else {
                summary.skipped_profiles = summary.skipped_profiles.saturating_add(1);
                continue;
            };
            let key = format!(
                "{}/{}",
                entry.manufacturer.to_ascii_lowercase(),
                entry.model.to_ascii_lowercase()
            );
            match best.get(&key) {
                Some((_, p)) if *p <= profile.priority => {}
                _ => {
                    best.insert(key, (entry, profile.priority));
                }
            }
        }
        // Past half there is no way back; no reason to keep asking.
        if summary.failed_profiles.saturating_mul(2) > total {
            return Err(AutoEqError::TooManyFailures {
                failed: summary.failed_profiles,
                total,
            });
        }
        done = done.saturating_add(batch.len());
        send(
            done as f32 / total as f32,
            format!("Downloading {done} / {total} profiles..."),
            None,
        );
    }

    send(1.0, "Building database...".into(), None);
    let mut entries: Vec<Entry> = best.into_values().map(|(e, _)| e).collect();
    if entries.is_empty() {
        return Err(AutoEqError::NoProfiles {
            reason: "none of the profiles could be read".to_string(),
        });
    }
    entries.sort_by_key(|e| {
        (
            e.manufacturer.to_ascii_lowercase(),
            e.model.to_ascii_lowercase(),
        )
    });
    summary.entries = entries.len();
    let db = Database {
        version: 1,
        generated_at: iso8601_utc(unix_now()),
        entry_count: i32::try_from(entries.len()).unwrap_or(i32::MAX),
        entries,
        origin: Origin::User,
    };
    let text =
        serde_json::to_string_pretty(&db).map_err(|e| AutoEqError::Unreadable(e.to_string()))?;
    write_config(out, &text)?;
    Ok(summary)
}

/// Fetch a batch at once. The answers come back in the batch's order.
fn fetch_batch(fetch: &dyn Fetch, batch: &[Wanted]) -> Vec<Result<String, FetchError>> {
    let one = |p: &Wanted| {
        let url = format!(
            "https://raw.githubusercontent.com/{REPO}/master/results/{}/{}/{}/{}",
            encode(p.folder),
            encode(&p.target),
            encode(&p.headphone),
            encode(&format!("{} ParametricEQ.txt", p.headphone)),
        );
        fetch_text(fetch, &FetchRequest::new(url, None))
    };
    std::thread::scope(|scope| {
        let workers: Vec<_> = batch
            .iter()
            .map(|p| {
                let spawned = std::thread::Builder::new()
                    .name("dspi-autoeq-fetch".into())
                    .spawn_scoped(scope, move || one(p));
                // Out of threads: slower, not wrong.
                spawned.map_err(|_| p)
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| match worker {
                Ok(thread) => thread.join().unwrap_or_else(|_| {
                    Err(FetchError::Network("the fetch thread failed".to_string()))
                }),
                Err(p) => one(p),
            })
            .collect()
    })
}

/// The directory listing from the GitHub contents API, folder names only.
fn list_dir(fetch: &dyn Fetch, path: &str) -> Result<Vec<String>, FetchError> {
    let request = FetchRequest::new(
        format!(
            "https://api.github.com/repos/{REPO}/contents/{}",
            encode(path)
        ),
        Some(GITHUB_ACCEPT),
    );
    let text = fetch_text(fetch, &request)?;
    let items: Vec<serde_json::Value> = serde_json::from_str(&text)
        .map_err(|e| FetchError::BadContent(format!("GitHub answered something else: {e}")))?;
    Ok(items
        .into_iter()
        .filter(|i| i.get("type").and_then(|t| t.as_str()) == Some("dir"))
        .filter_map(|i| {
            i.get("name")
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())
        })
        .collect())
}

/// Percent-encode a path; the separators stay as they are, which is what
/// `urlPathAllowed` does. Every byte outside the plain set goes, so a headphone
/// name in another script or with a `%` in it still makes a valid URL.
fn encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(char::from(byte));
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Turn one downloaded `ParametricEQ.txt` into a database entry.
///
/// The file is REW's format, which the filter-file reader already speaks, so
/// there is one parser for both rather than two that can disagree.
pub fn profile_entry(text: &str, source: &str, target: &str, headphone: &str) -> Option<Entry> {
    let file = crate::filterfile::parse(text).ok()?;
    let bank = file.channels.first()?;
    let filters: Vec<Filter> = bank
        .peq
        .iter()
        .filter_map(|b| {
            Some(Filter {
                kind: kind_name(b.filter_type)?.into(),
                freq: b.freq,
                q: b.q,
                gain: b.gain_db,
            })
        })
        .collect();
    if filters.is_empty() {
        return None;
    }
    let (manufacturer, model) = split_name(headphone);
    Some(Entry {
        id: format!("{source}/{headphone}"),
        manufacturer,
        model,
        source: source.to_string(),
        form_factor: form_factor(target),
        preamp: bank.preamp_db.unwrap_or(0.0),
        filters,
    })
}

/// The Console's `detectFormFactor`, which reads it off the target folder.
fn form_factor(target: &str) -> String {
    let lower = target.to_ascii_lowercase();
    if lower.contains("in-ear") || lower.contains("in_ear") || lower.contains("iem") {
        "in-ear".into()
    } else if lower.contains("earbud") {
        "earbud".into()
    } else {
        "over-ear".into()
    }
}

/// The Console's `parseHeadphoneName`: a known manufacturer wins over the
/// first-word split, so "Dan Clark Audio Aeon" does not become manufacturer
/// "Dan".
fn split_name(folder: &str) -> (String, String) {
    const KNOWN: [&str; 66] = [
        "AKG",
        "Audio-Technica",
        "Audeze",
        "Bang & Olufsen",
        "Beats",
        "Beyerdynamic",
        "Bose",
        "Campfire Audio",
        "Dan Clark Audio",
        "Denon",
        "FiiO",
        "Final",
        "Focal",
        "Grado",
        "HarmonicDyne",
        "HIFIMAN",
        "JBL",
        "Koss",
        "Massdrop",
        "Meze",
        "Moondrop",
        "Philips",
        "Pioneer",
        "Sennheiser",
        "Shure",
        "Sony",
        "SteelSeries",
        "STAX",
        "Tin HiFi",
        "V-MODA",
        "ZMF",
        "64 Audio",
        "7Hz",
        "Anker",
        "Apple",
        "AFUL",
        "BLON",
        "CCA",
        "Dunu",
        "Empire Ears",
        "Etymotic",
        "FatFreq",
        "Hidizs",
        "HiBy",
        "iBasso",
        "JVC",
        "KZ",
        "Letshuoer",
        "Linsoul",
        "Noble Audio",
        "QKZ",
        "Samsung",
        "See Audio",
        "Simgot",
        "SoftEars",
        "Tangzu",
        "Thieaudio",
        "Tinhifi",
        "Tripowin",
        "TRN",
        "Truthear",
        "Unique Melody",
        "Westone",
        "Yanyin",
        "BGVP",
        "CCZ",
    ];
    let lower = folder.to_ascii_lowercase();
    for mfr in KNOWN {
        let m = mfr.to_ascii_lowercase();
        if lower == m {
            return (mfr.into(), String::new());
        }
        if let Some(rest) = lower.strip_prefix(&format!("{m} ")) {
            let _ = rest;
            let model = folder[mfr.len()..].trim().to_string();
            return (mfr.into(), model);
        }
    }
    match folder.split_once(' ') {
        Some((a, b)) => (a.into(), b.into()),
        None => (folder.into(), String::new()),
    }
}

/// Check a database's bytes: it has to be one, with something in it. An empty
/// one is refused because every field is optional, so `{}` would otherwise
/// replace a working database with nothing.
fn validate(data: &[u8]) -> Result<Database, AutoEqError> {
    if data.len() > MAX_DATABASE_BYTES {
        return Err(AutoEqError::Unreadable(format!(
            "it is over {} MB",
            MAX_DATABASE_BYTES / (1024 * 1024)
        )));
    }
    let db: Database =
        serde_json::from_slice(data).map_err(|e| AutoEqError::Unreadable(e.to_string()))?;
    if db.entries.is_empty() {
        return Err(AutoEqError::Unreadable("it has no profiles".to_string()));
    }
    Ok(db)
}

/// Replace the database at `db_path` with these bytes, once they are known to be
/// a database. For a host whose file picker gives bytes rather than a path.
pub fn import_bytes(data: &[u8], db_path: &std::path::Path) -> Result<Database, AutoEqError> {
    let mut db = validate(data)?;
    write_atomic(db_path, data)?;
    db.origin = Origin::User;
    Ok(db)
}

/// Replace the user database with a file the person chose.
pub fn import_database(path: &std::path::Path) -> Result<usize, AutoEqError> {
    let data = std::fs::read(path).map_err(|e| AutoEqError::Unreadable(e.to_string()))?;
    Ok(import_bytes(&data, &user_db_path())?.entries.len())
}

/// Download a ready-made database (the Console's `updateDatabase(from:)`) and
/// replace the one at `db_path`, once the download is known to be a database.
/// Blocks for the request.
pub fn update_from_url(
    fetch: &dyn Fetch,
    url: &str,
    db_path: &std::path::Path,
) -> Result<Database, AutoEqError> {
    let data = fetch_bytes(fetch, &FetchRequest::new(url.to_string(), None))
        .map_err(AutoEqError::Fetch)?;
    import_bytes(&data, db_path)
}

/// Remove the database at `db_path`, which is what going back to the built-in
/// one comes to. True if there was one.
pub fn remove_user_database(db_path: &std::path::Path) -> Result<bool, AutoEqError> {
    match std::fs::remove_file(db_path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(AutoEqError::Unwritable {
            path: db_path.display().to_string(),
            reason: e.to_string(),
        }),
    }
}

/// Throw the user database away and go back to the bundled one.
pub fn reset_to_builtin() -> Result<usize, AutoEqError> {
    remove_user_database(&user_db_path())?;
    Ok(load()?.entries.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "bundled-autoeq")]
    #[test]
    fn the_bundled_database_loads_and_is_populated() {
        let db = load().expect("the bundled database should always load");
        assert!(
            db.entries.len() > 1000,
            "only {} entries; the bundle looks truncated",
            db.entries.len()
        );
        assert_eq!(db.origin, Origin::Bundled);
    }

    #[cfg(feature = "bundled-autoeq")]
    #[test]
    fn a_well_known_headphone_is_findable() {
        let db = load().unwrap();
        let hits = db.search("hd600");
        assert!(
            !hits.is_empty(),
            "the HD 600 should be in any AutoEQ export"
        );

        let e = hits[0];
        assert!(
            !e.filters.is_empty(),
            "a profile with no filters is useless"
        );
        // Corrections boost, so the trim is essentially always negative.
        assert!(e.preamp <= 0.0, "preamp should be a cut, got {}", e.preamp);
    }

    #[cfg(feature = "bundled-autoeq")]
    #[test]
    fn every_filter_shape_in_the_database_is_understood() {
        let db = load().unwrap();
        let mut unknown = std::collections::BTreeSet::new();
        for e in &db.entries {
            for f in &e.filters {
                if filter_type(&f.kind).is_none() {
                    unknown.insert(f.kind.clone());
                }
            }
        }
        assert!(
            unknown.is_empty(),
            "shapes the mapper does not know: {unknown:?}"
        );
    }

    #[cfg(feature = "bundled-autoeq")]
    #[test]
    fn search_terms_may_arrive_in_any_order() {
        let db = load().unwrap();
        let a = db.search("sennheiser hd600").len();
        let b = db.search("hd600 sennheiser").len();
        assert_eq!(a, b);
    }

    #[test]
    fn filter_shapes_map_to_firmware_types() {
        use dspi_proto::FilterType as F;
        assert_eq!(filter_type("PK"), Some(F::Peaking));
        assert_eq!(filter_type("lowshelf"), Some(F::LowShelf));
        assert_eq!(filter_type("HSC"), Some(F::HighShelf));
    }

    /// Forcing an unknown shape into a peaking filter would produce a curve
    /// that is wrong in a way nobody can see.
    #[test]
    fn an_unknown_shape_is_refused_rather_than_guessed() {
        assert_eq!(filter_type("WOBBLE"), None);
        assert_eq!(filter_type(""), None);
    }

    #[test]
    fn the_user_path_is_somewhere_sensible() {
        let p = user_db_path();
        assert!(p.ends_with(USER_DB_NAME));
        assert!(p.to_string_lossy().contains("dspi-term"));
        assert!(favourites_path().ends_with(FAVOURITES_NAME));
    }

    fn entry(filters: Vec<(&str, f32)>) -> Entry {
        Entry {
            id: "oratory1990/Sennheiser HD 600".into(),
            manufacturer: "Sennheiser".into(),
            model: "HD 600".into(),
            source: "oratory1990".into(),
            form_factor: "over-ear".into(),
            preamp: -6.5,
            filters: filters
                .into_iter()
                .map(|(kind, freq)| Filter {
                    kind: kind.into(),
                    freq,
                    q: 0.7,
                    gain: 3.0,
                })
                .collect(),
        }
    }

    /// Bands the profile does not use have to be cleared, or whatever was on
    /// the channel before survives underneath the correction.
    #[test]
    fn a_plan_clears_the_bands_the_profile_does_not_use() {
        let plan = entry(vec![("peaking", 100.0), ("lowShelf", 105.0)]).plan(10);
        assert_eq!(plan.preamp_db, -6.5);
        assert_eq!(plan.bands.len(), 2);
        assert_eq!(plan.bands[1].filter_type, dspi_proto::FilterType::LowShelf);
        assert_eq!(plan.cleared, (2u8..10).collect::<Vec<_>>());
        assert_eq!(plan.dropped, 0);
        assert!(plan.unsupported.is_empty());
    }

    #[test]
    fn a_plan_reports_what_will_not_fit_or_is_not_understood() {
        let mut e = entry(vec![("peaking", 100.0); 12]);
        e.filters[1].kind = "wobble".into();
        let plan = e.plan(10);
        assert_eq!(plan.dropped, 2, "12 bands into a bank of 10");
        assert_eq!(plan.bands.len(), 9, "the unknown shape is not invented");
        assert_eq!(plan.unsupported, vec!["wobble".to_string()]);
    }

    #[test]
    fn an_entry_names_itself_the_way_the_console_does() {
        let e = entry(vec![("peaking", 100.0)]);
        assert_eq!(e.display_name(), "Sennheiser HD 600");
        assert_eq!(e.source_label(), "oratory1990");
        let mut bare = e.clone();
        bare.model = String::new();
        assert_eq!(bare.display_name(), "Sennheiser");
        bare.source = "crinacle".into();
        assert_eq!(bare.source_label(), "Crinacle");
    }

    #[test]
    fn a_headphone_folder_splits_on_a_known_manufacturer_first() {
        assert_eq!(
            split_name("Dan Clark Audio Aeon 2"),
            ("Dan Clark Audio".into(), "Aeon 2".into())
        );
        assert_eq!(
            split_name("Sennheiser HD 600"),
            ("Sennheiser".into(), "HD 600".into())
        );
        assert_eq!(
            split_name("Wobbletron X1"),
            ("Wobbletron".into(), "X1".into())
        );
        assert_eq!(split_name("Nameless"), ("Nameless".into(), "".into()));
    }

    #[test]
    fn the_form_factor_comes_off_the_target_folder() {
        assert_eq!(form_factor("harman_in-ear_2019v2"), "in-ear");
        assert_eq!(form_factor("harman_earbud_2019"), "earbud");
        assert_eq!(form_factor("harman_over-ear_2018"), "over-ear");
    }

    /// A downloaded profile is REW's format, so the filter-file reader is the
    /// one parser for both.
    #[test]
    fn a_downloaded_profile_becomes_an_entry() {
        let text = "\
Preamp: -6.5 dB
Filter 1: ON PK Fc 105 Hz Gain 3.0 dB Q 0.70
Filter 2: ON LSC Fc 105 Hz Gain 5.5 dB Q 0.70
Filter 3: ON None
";
        let e = profile_entry(
            text,
            "oratory1990",
            "harman_over-ear_2018",
            "Sennheiser HD 600",
        )
        .expect("a profile with filters");
        assert_eq!(e.id, "oratory1990/Sennheiser HD 600");
        assert_eq!(e.manufacturer, "Sennheiser");
        assert_eq!(e.form_factor, "over-ear");
        assert_eq!(e.preamp, -6.5);
        assert_eq!(e.filters.len(), 2);
        assert_eq!(e.filters[1].kind, "lowShelf");
        // And it reads back through the same mapper the browser applies with.
        assert_eq!(
            filter_type(&e.filters[1].kind),
            Some(dspi_proto::FilterType::LowShelf)
        );
        assert!(profile_entry("nothing here", "s", "t", "h").is_none());
    }

    /// A rebuild is the one thing in this program that touches the network, so
    /// the warning has to say exactly what it will reach for.
    #[test]
    fn the_rebuild_warning_names_what_it_fetches() {
        let w = rebuild_warning();
        assert!(w.contains("api.github.com"), "{w}");
        assert!(w.contains("raw.githubusercontent.com"), "{w}");
        assert!(w.contains("sends nothing about you or your device"), "{w}");
        assert!(w.contains("Do you wish to proceed?"), "{w}");
    }

    const API: &str = "https://api.github.com/repos/jaakkopasanen/AutoEq/contents/results/";
    const RAW: &str = "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/master/results/";
    const PROFILE: &str = "Preamp: -6.5 dB\nFilter 1: ON PK Fc 105 Hz Gain 3.0 dB Q 0.70\n";

    fn answer(status: u16, body: &str) -> Result<FetchResponse, String> {
        Ok(FetchResponse {
            status,
            body: body.as_bytes().to_vec(),
            ..Default::default()
        })
    }

    fn dirs(names: &[&str]) -> String {
        let items: Vec<String> = names
            .iter()
            .map(|n| format!(r#"{{"name":"{n}","type":"dir"}}"#))
            .collect();
        format!("[{}]", items.join(","))
    }

    /// A scripted GitHub: every answer is written down in advance, and a URL
    /// nobody scripted is a 404, as it would be there. Records what was asked.
    #[derive(Default)]
    struct Script {
        answers: std::collections::HashMap<String, Result<FetchResponse, String>>,
        /// Answers every download with this, whatever was scripted.
        downloads: Option<Result<FetchResponse, String>>,
        asked: std::sync::Mutex<Vec<FetchRequest>>,
    }

    impl Script {
        fn on(mut self, url: String, reply: Result<FetchResponse, String>) -> Self {
            self.answers.insert(url, reply);
            self
        }

        /// One source with one target holding these headphones.
        fn listing(self, folder: &str, headphones: &[&str]) -> Self {
            let source = folder.replace(' ', "%20");
            self.on(
                format!("{API}{source}"),
                answer(200, &dirs(&["t_over-ear"])),
            )
            .on(
                format!("{API}{source}/t_over-ear"),
                answer(200, &dirs(headphones)),
            )
        }

        /// A healthy download of one headphone.
        fn profile(self, folder: &str, headphone: &str) -> Self {
            let url = raw_url(folder, headphone);
            self.on(url, answer(200, PROFILE))
        }

        fn asked_urls(&self) -> Vec<String> {
            self.asked
                .lock()
                .unwrap()
                .iter()
                .map(|r| r.url.clone())
                .collect()
        }
    }

    fn raw_url(folder: &str, headphone: &str) -> String {
        let (s, h) = (folder.replace(' ', "%20"), headphone.replace(' ', "%20"));
        format!("{RAW}{s}/t_over-ear/{h}/{h}%20ParametricEQ.txt")
    }

    impl Fetch for Script {
        fn get(&self, request: &FetchRequest) -> Result<FetchResponse, String> {
            self.asked.lock().unwrap().push(request.clone());
            if request.url.starts_with(RAW)
                && let Some(reply) = &self.downloads
            {
                return reply.clone();
            }
            self.answers
                .get(&request.url)
                .cloned()
                .unwrap_or_else(|| answer(404, "Not Found"))
        }
    }

    /// A directory for a test's files, beside the test binary in the target
    /// directory: the system's temp dir may be a tmpfs. Starts empty.
    fn test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::current_exe()
            .unwrap()
            .with_file_name("test-scratch")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Rebuild into a scratch directory that already holds a database, and give
    /// back the result and that file's text afterwards.
    fn rebuild_over_existing(name: &str, fetch: &dyn Fetch) -> (RebuildDone, String) {
        let dir = test_dir(name);
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join(USER_DB_NAME);
        std::fs::write(&out, "the old database").unwrap();
        let stop = std::sync::atomic::AtomicBool::new(false);
        let result = run_rebuild(fetch, &stop, &out, &|_, _, _| {});
        let text = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        (result, text)
    }

    #[test]
    fn a_rebuild_walks_the_listing_and_reports_progress() {
        let fetch = Script::default()
            .listing("oratory1990", &["Sennheiser HD 600"])
            .profile("oratory1990", "Sennheiser HD 600");
        let stop = std::sync::atomic::AtomicBool::new(false);
        let seen = std::sync::Mutex::new(Vec::new());
        // The rebuild writes wherever it is told, so it is pointed at a scratch
        // file rather than the real user database.
        let dir = test_dir("autoeq");
        let out = dir.join(USER_DB_NAME);

        let result = run_rebuild(&fetch, &stop, &out, &|f, s, _| {
            seen.lock().unwrap().push((f, s));
        });

        let written = std::fs::read_to_string(&out).ok();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(
            result,
            Ok(RebuildSummary {
                entries: 1,
                ..Default::default()
            }),
            "one headphone, from one source"
        );
        let lines: Vec<String> = seen
            .lock()
            .unwrap()
            .iter()
            .map(|(_, s)| s.clone())
            .collect();
        assert_eq!(lines[0], "Connecting to GitHub...");
        for line in [
            "Discovering profiles...",
            "Found 1 profiles...",
            "Downloading 1 / 1 profiles...",
            "Building database...",
        ] {
            assert!(lines.contains(&line.to_string()), "{line}: {lines:?}");
        }

        let written = written.expect("the rebuild writes the user database");
        assert!(written.contains("\"entryCount\": 1"), "{written}");
        assert!(written.contains("Sennheiser"), "{written}");
        let back: Database = serde_json::from_str(&written).unwrap();
        assert_eq!(back.entries[0].filters[0].kind, "peaking");
    }

    #[test]
    fn a_cancelled_rebuild_says_so_rather_than_writing_half_a_database() {
        let stop = std::sync::atomic::AtomicBool::new(true);
        let out = test_dir("autoeq-cancelled").join("db.json");
        let result = run_rebuild(&Script::default(), &stop, &out, &|_, _, _| {});
        assert_eq!(result, Err(AutoEqError::Cancelled));
        assert_eq!(AutoEqError::Cancelled.to_string(), "Cancelled.");
        assert!(!out.exists());
    }

    #[test]
    fn wp_cg6_github_rate_limit_signals_are_recognised() {
        let now = 1_000;
        let limited = |r: FetchResponse| match read_response(r, now) {
            Err(FetchError::RateLimited { retry_after_s }) => Some(retry_after_s),
            _ => None,
        };
        let base = FetchResponse::default();
        // The primary limit: a 403 with the counter at zero.
        let primary = FetchResponse {
            status: 403,
            rate_limit_remaining: Some(0),
            rate_limit_reset: Some(1_600),
            ..base.clone()
        };
        assert_eq!(limited(primary), Some(Some(600)));
        // A secondary limit: Retry-After, which wins over the reset time.
        let secondary = FetchResponse {
            status: 403,
            retry_after_s: Some(60),
            rate_limit_reset: Some(1_600),
            ..base.clone()
        };
        assert_eq!(limited(secondary), Some(Some(60)));
        // 429, with nothing else to go on.
        let plain = FetchResponse {
            status: 429,
            ..base.clone()
        };
        assert_eq!(limited(plain), Some(None));
        // A host that cannot pass headers: GitHub's words.
        let words = FetchResponse {
            status: 403,
            body: b"{\"message\":\"API rate limit exceeded for 1.2.3.4\"}".to_vec(),
            ..base.clone()
        };
        assert_eq!(limited(words), Some(None));
        // A reset already past is not a negative wait.
        let past = FetchResponse {
            status: 429,
            rate_limit_reset: Some(10),
            ..base.clone()
        };
        assert_eq!(limited(past), Some(Some(0)));

        // A plain refusal is not a rate limit: waiting would not help.
        let refused = FetchResponse {
            status: 403,
            body: b"Forbidden".to_vec(),
            rate_limit_remaining: Some(41),
            ..base.clone()
        };
        assert_eq!(read_response(refused, now), Err(FetchError::Status(403)));
        let ok = FetchResponse {
            status: 200,
            body: b"x".to_vec(),
            ..base
        };
        assert_eq!(read_response(ok, now), Ok(b"x".to_vec()));
    }

    #[test]
    fn wp_cg6_the_rate_limit_message_says_when() {
        let in_s = |s| {
            AutoEqError::Fetch(FetchError::RateLimited {
                retry_after_s: Some(s),
            })
            .to_string()
        };
        assert!(in_s(600).contains("about 10 minutes"), "{}", in_s(600));
        assert!(in_s(0).contains("about a minute"), "{}", in_s(0));
    }

    /// The trap the Console falls into: it treats a rate limit like any other
    /// failed listing, and replaces the database with whatever it got.
    #[test]
    fn wp_cg6_a_rate_limit_while_listing_stops_and_keeps_the_database() {
        let limit = Ok(FetchResponse {
            status: 403,
            retry_after_s: Some(120),
            ..Default::default()
        });
        let fetch = Script::default()
            .listing("oratory1990", &["Sony WH-1000XM4"])
            .profile("oratory1990", "Sony WH-1000XM4")
            .on(format!("{API}crinacle"), limit);
        let (result, on_disk) = rebuild_over_existing("cg6-limit-list", &fetch);
        assert_eq!(
            result,
            Err(AutoEqError::Fetch(FetchError::RateLimited {
                retry_after_s: Some(120)
            }))
        );
        assert_eq!(on_disk, "the old database");
        // It stopped there: not one more listing, and no downloads.
        let asked = fetch.asked_urls();
        assert_eq!(
            asked.last().unwrap(),
            &format!("{API}crinacle"),
            "{asked:?}"
        );
        assert!(!asked.iter().any(|u| u.starts_with(RAW)), "{asked:?}");
    }

    #[test]
    fn wp_cg6_a_rate_limit_while_downloading_stops_and_keeps_the_database() {
        let many: Vec<String> = (0..50).map(|i| format!("Acme H{i}")).collect();
        let names: Vec<&str> = many.iter().map(String::as_str).collect();
        let mut fetch = Script::default().listing("oratory1990", &names);
        fetch.downloads = Some(Ok(FetchResponse {
            status: 429,
            ..Default::default()
        }));
        let (result, on_disk) = rebuild_over_existing("cg6-limit-raw", &fetch);
        assert_eq!(
            result,
            Err(AutoEqError::Fetch(FetchError::RateLimited {
                retry_after_s: None
            }))
        );
        assert_eq!(on_disk, "the old database");
        let downloads = fetch
            .asked_urls()
            .iter()
            .filter(|u| u.starts_with(RAW))
            .count();
        assert_eq!(downloads, BATCH_SIZE, "stops after the first batch");
    }

    /// Scattered failures leave out what failed, write the rest, and say so.
    #[test]
    fn wp_cg6_a_partial_failure_writes_the_rest_and_reports_it() {
        let fetch = Script::default()
            .listing(
                "oratory1990",
                &["Sony A", "Sony B", "Sony C", "Sony D", "Sony E"],
            )
            .profile("oratory1990", "Sony A")
            .profile("oratory1990", "Sony B")
            // A server error and a dropped connection: failures.
            .on(raw_url("oratory1990", "Sony C"), answer(500, "oops"))
            .on(raw_url("oratory1990", "Sony D"), Err("timed out".into()))
            // Sony E has no scripted answer: a 404, which is not a failure.
            // Another source lists fine but holds a profile with no filters,
            // and a third cannot be listed at all.
            .listing("crinacle", &["Sony F"])
            .on(
                raw_url("crinacle", "Sony F"),
                answer(200, "no filters here"),
            )
            .on(format!("{API}Rtings"), answer(502, "Bad Gateway"));
        let dir = test_dir("cg6-partial");
        let out = dir.join(USER_DB_NAME);
        let stop = std::sync::atomic::AtomicBool::new(false);
        let result = run_rebuild(&fetch, &stop, &out, &|_, _, _| {}).unwrap();
        let back = load_user_database(&out).unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(
            result,
            RebuildSummary {
                entries: 2,
                failed_profiles: 2,
                failed_listings: 1,
                skipped_profiles: 2,
            }
        );
        assert_eq!(back.entries.len(), 2);
        assert_eq!(back.entry_count, 2);
        assert_eq!(back.entries[0].model, "A");
        assert_eq!(leftovers.len(), 1, "no temporary file stays behind");
        assert!(result.to_string().contains("could not be downloaded"));
        assert!(!back.generated_at.is_empty(), "the Console shows this date");
    }

    #[test]
    fn wp_cg6_a_clean_summary_says_only_the_count() {
        let clean = RebuildSummary {
            entries: 7,
            skipped_profiles: 3,
            ..Default::default()
        };
        assert_eq!(clean.to_string(), "7");
    }

    #[test]
    fn wp_cg6_when_most_downloads_fail_the_database_is_kept() {
        let fetch = Script::default()
            .listing("oratory1990", &["Sony A", "Sony B", "Sony C"])
            .profile("oratory1990", "Sony A")
            .on(raw_url("oratory1990", "Sony B"), answer(503, ""))
            .on(raw_url("oratory1990", "Sony C"), Err("no route".into()));
        let (result, on_disk) = rebuild_over_existing("cg6-most-fail", &fetch);
        assert_eq!(
            result,
            Err(AutoEqError::TooManyFailures {
                failed: 2,
                total: 3
            })
        );
        assert_eq!(on_disk, "the old database");
    }

    #[test]
    fn wp_cg6_when_most_listings_fail_the_database_is_kept() {
        // Five sources and one target: four of the six listings fail.
        let fetch = Script::default()
            .listing("oratory1990", &["Sony A"])
            .profile("oratory1990", "Sony A")
            .on(format!("{API}crinacle"), answer(500, ""))
            .on(format!("{API}Rtings"), answer(500, ""))
            .on(format!("{API}Innerfidelity"), answer(500, ""))
            .on(format!("{API}Headphone.com%20Legacy"), answer(500, ""));
        let (result, on_disk) = rebuild_over_existing("cg6-listings", &fetch);
        assert_eq!(
            result,
            Err(AutoEqError::TooManyFailures {
                failed: 4,
                total: 6
            })
        );
        assert_eq!(on_disk, "the old database");
    }

    /// Offline: the person hears the network's words, not "no profiles".
    #[test]
    fn wp_cg6_with_no_network_the_reason_is_the_networks() {
        struct Offline;
        impl Fetch for Offline {
            fn get(&self, _: &FetchRequest) -> Result<FetchResponse, String> {
                Err("could not resolve api.github.com".into())
            }
        }
        let (result, on_disk) = rebuild_over_existing("cg6-offline", &Offline);
        assert_eq!(
            result,
            Err(AutoEqError::NoProfiles {
                reason: "could not resolve api.github.com".into()
            })
        );
        assert_eq!(on_disk, "the old database");
    }

    #[test]
    fn wp_cg6_a_listing_with_nothing_readable_writes_nothing() {
        // Everything lists, nothing can be read: no profiles, not an empty file.
        let fetch = Script::default().listing("oratory1990", &["Sony A"]);
        let (result, on_disk) = rebuild_over_existing("cg6-empty", &fetch);
        assert!(
            matches!(result, Err(AutoEqError::NoProfiles { .. })),
            "{result:?}"
        );
        assert_eq!(on_disk, "the old database");
        // A listing that is not a listing counts as failed, not as empty.
        let fetch = Script::default().on(format!("{API}oratory1990"), answer(200, "<html>"));
        let (result, _) = rebuild_over_existing("cg6-html", &fetch);
        let Err(AutoEqError::NoProfiles { reason }) = result else {
            panic!("{result:?}");
        };
        assert!(
            reason.contains("GitHub answered something else"),
            "{reason}"
        );
    }

    #[test]
    fn wp_cg6_the_better_source_wins_a_duplicate() {
        let fetch = Script::default()
            .listing("crinacle", &["Sony A"])
            .listing("oratory1990", &["Sony A"])
            .profile("crinacle", "Sony A")
            .profile("oratory1990", "Sony A");
        let dir = test_dir("cg6-dup");
        let out = dir.join(USER_DB_NAME);
        let stop = std::sync::atomic::AtomicBool::new(false);
        run_rebuild(&fetch, &stop, &out, &|_, _, _| {}).unwrap();
        let back = load_user_database(&out).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(back.entries.len(), 1);
        assert_eq!(back.entries[0].source, "oratory1990");
        assert_eq!(back.entries[0].id, "oratory1990/Sony A");
    }

    #[test]
    fn wp_cg6_the_host_is_told_which_headers_to_send() {
        let fetch = Script::default()
            .listing("oratory1990", &["Sony A"])
            .profile("oratory1990", "Sony A");
        let _ = rebuild_over_existing("cg6-headers", &fetch);
        let asked = fetch.asked.lock().unwrap();
        for request in asked.iter() {
            assert_eq!(request.user_agent, FETCH_USER_AGENT, "{request:?}");
            let accept = request.accept.as_deref();
            if request.url.starts_with(RAW) {
                assert_eq!(accept, None, "{request:?}");
            } else {
                assert_eq!(accept, Some(GITHUB_ACCEPT), "{request:?}");
            }
        }
        assert!(asked.len() >= 3);
    }

    #[test]
    fn wp_cg6_urls_are_encoded_for_any_name() {
        assert_eq!(
            encode("results/Headphone.com Legacy/a#b?c+d&e"),
            "results/Headphone.com%20Legacy/a%23b%3Fc%2Bd%26e"
        );
        assert_eq!(encode("100%"), "100%25");
        assert_eq!(encode("水"), "%E6%B0%B4");
    }

    #[test]
    fn wp_cg6_downloads_run_in_batches_of_twenty() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Slow {
            script: Script,
            now: AtomicUsize,
            peak: AtomicUsize,
        }
        impl Fetch for Slow {
            fn get(&self, request: &FetchRequest) -> Result<FetchResponse, String> {
                if !request.url.starts_with(RAW) {
                    return self.script.get(request);
                }
                let n = self.now.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(n, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(20));
                self.now.fetch_sub(1, Ordering::SeqCst);
                answer(200, PROFILE)
            }
        }
        let many: Vec<String> = (0..45).map(|i| format!("Acme H{i:02}")).collect();
        let names: Vec<&str> = many.iter().map(String::as_str).collect();
        let fetch = Slow {
            script: Script::default().listing("oratory1990", &names),
            now: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        };
        let (result, _) = rebuild_over_existing("cg6-batches", &fetch);
        assert_eq!(result.unwrap().entries, 45);
        let peak = fetch.peak.load(Ordering::SeqCst);
        assert!((2..=BATCH_SIZE).contains(&peak), "peak {peak}");
    }

    /// A foreign exception reaches Rust as a panic in the callback. It costs
    /// that request, not the rebuild, on the rebuild's thread or a batch's.
    #[test]
    fn wp_cg6_a_panicking_fetch_is_a_failed_request() {
        struct Panics(Script);
        impl Fetch for Panics {
            fn get(&self, request: &FetchRequest) -> Result<FetchResponse, String> {
                assert!(!request.url.contains("crinacle"), "listing panic");
                assert!(!request.url.contains("Sony%20B"), "download panic");
                self.0.get(request)
            }
        }
        let fetch = Panics(
            Script::default()
                .listing("oratory1990", &["Sony A", "Sony B", "Sony C"])
                .profile("oratory1990", "Sony A")
                .profile("oratory1990", "Sony C"),
        );
        let (result, _) = rebuild_over_existing("cg6-panic", &fetch);
        assert_eq!(
            result,
            Ok(RebuildSummary {
                entries: 2,
                failed_profiles: 1,
                failed_listings: 1,
                skipped_profiles: 0,
            })
        );
    }

    /// Cancelling between batches: the requests in flight finish, no more start.
    #[test]
    fn wp_cg6_a_cancel_stops_at_the_next_batch() {
        struct CancelsAfterFirst {
            script: Script,
            stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        }
        impl Fetch for CancelsAfterFirst {
            fn get(&self, request: &FetchRequest) -> Result<FetchResponse, String> {
                if request.url.starts_with(RAW) {
                    self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
                    self.script.asked.lock().unwrap().push(request.clone());
                    return answer(200, PROFILE);
                }
                self.script.get(request)
            }
        }
        let many: Vec<String> = (0..50).map(|i| format!("Acme H{i}")).collect();
        let names: Vec<&str> = many.iter().map(String::as_str).collect();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fetch = CancelsAfterFirst {
            script: Script::default().listing("oratory1990", &names),
            stop: std::sync::Arc::clone(&stop),
        };
        let dir = test_dir("cg6-cancel");
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join(USER_DB_NAME);
        std::fs::write(&out, "the old database").unwrap();
        let result = run_rebuild(&fetch, &stop, &out, &|_, _, _| {});
        let on_disk = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(result, Err(AutoEqError::Cancelled));
        assert_eq!(on_disk, "the old database");
        let downloads = fetch
            .script
            .asked_urls()
            .iter()
            .filter(|u| u.starts_with(RAW))
            .count();
        assert_eq!(downloads, BATCH_SIZE);
    }

    #[test]
    fn wp_cg6_a_rebuild_on_its_thread_reports_through_poll() {
        let dir = test_dir("cg6-handle");
        let out = dir.join(USER_DB_NAME);
        let fetch = Script::default()
            .listing("oratory1990", &["Sony A"])
            .profile("oratory1990", "Sony A");
        let mut handle = rebuild_to(Box::new(fetch), out.clone());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let done = loop {
            let progress = handle.poll().clone();
            if let Some(done) = progress.done {
                assert_eq!(progress.status, "Complete! 1 profiles.");
                assert_eq!(progress.fraction, 1.0);
                break done;
            }
            assert!(std::time::Instant::now() < deadline, "no result");
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(done.unwrap().entries, 1);
        handle.cancel_and_wait();
        assert_eq!(load_user_database(&out).unwrap().entries.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wp_cg6_a_thread_that_died_is_reported_rather_than_waited_for() {
        let (tx, rx) = std::sync::mpsc::channel();
        drop(tx);
        let mut handle = RebuildHandle {
            rx,
            stop: Default::default(),
            latest: RebuildProgress {
                fraction: 0.5,
                status: "Downloading".into(),
                done: None,
            },
            thread: None,
        };
        assert_eq!(handle.poll().done, Some(Err(AutoEqError::Stopped)));
    }

    #[test]
    fn wp_cg6_cancel_and_wait_ends_the_fetch_calls() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Counts(std::sync::Arc<AtomicUsize>, Script);
        impl Fetch for Counts {
            fn get(&self, request: &FetchRequest) -> Result<FetchResponse, String> {
                self.0.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(5));
                if request.url.starts_with(RAW) {
                    return answer(200, PROFILE);
                }
                self.1.get(request)
            }
        }
        let many: Vec<String> = (0..500).map(|i| format!("Acme H{i}")).collect();
        let names: Vec<&str> = many.iter().map(String::as_str).collect();
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let script = Script::default().listing("oratory1990", &names);
        let dir = test_dir("cg6-wait");
        let mut handle = rebuild_to(
            Box::new(Counts(std::sync::Arc::clone(&calls), script)),
            dir.join(USER_DB_NAME),
        );
        handle.cancel_and_wait();
        let after = calls.load(Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(calls.load(Ordering::SeqCst), after, "still being called");
        assert_eq!(handle.poll().done, Some(Err(AutoEqError::Cancelled)));
        assert!(!dir.join(USER_DB_NAME).exists());
    }

    const SMALL_DB: &str = r#"{"version":1,"generatedAt":"2026-01-02T03:04:05.000Z","entryCount":1,
        "entries":[{"id":"o/Sony A","manufacturer":"Sony","model":"A","source":"o",
        "formFactor":"over-ear","preamp":-1.5,"filters":[{"type":"peaking","freq":100,"q":1,"gain":2}]}]}"#;

    #[test]
    fn wp_cg6_import_replaces_the_database_only_with_a_database() {
        let dir = test_dir("cg6-import");
        let out = dir.join(USER_DB_NAME);
        let db = import_bytes(SMALL_DB.as_bytes(), &out).unwrap();
        assert_eq!(db.entries.len(), 1);
        assert_eq!(db.origin, Origin::User);
        assert_eq!(std::fs::read_to_string(&out).unwrap(), SMALL_DB);
        assert_eq!(
            load_user_database(&out).unwrap().generated_at,
            db.generated_at
        );

        // None of these may touch what is there. `{}` parses, since every field
        // has a default, and is exactly what must not replace a database.
        for bad in [
            &b"not json"[..],
            b"{}",
            b"[]",
            b"{\"entries\":[]}",
            b"\xff\xfe",
        ] {
            let refused = import_bytes(bad, &out);
            assert!(
                matches!(refused, Err(AutoEqError::Unreadable(_))),
                "{refused:?}"
            );
            assert_eq!(std::fs::read_to_string(&out).unwrap(), SMALL_DB);
        }
        let huge = vec![b' '; MAX_DATABASE_BYTES + 1];
        assert!(matches!(
            import_bytes(&huge, &out),
            Err(AutoEqError::Unreadable(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wp_cg6_update_from_url_checks_the_download() {
        let dir = test_dir("cg6-update");
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join(USER_DB_NAME);
        std::fs::write(&out, "the old database").unwrap();
        let url = "https://example.test/db.json";
        let with = |reply| Script::default().on(url.to_string(), reply);
        let kept = || std::fs::read_to_string(&out).unwrap() == "the old database";

        let r = update_from_url(&with(answer(500, "")), url, &out);
        assert_eq!(r.unwrap_err(), AutoEqError::Fetch(FetchError::Status(500)));
        assert!(kept());
        let limit = Ok(FetchResponse {
            status: 429,
            retry_after_s: Some(9),
            ..Default::default()
        });
        let r = update_from_url(&with(limit), url, &out);
        assert_eq!(
            r.unwrap_err(),
            AutoEqError::Fetch(FetchError::RateLimited {
                retry_after_s: Some(9)
            })
        );
        assert!(kept());
        let r = update_from_url(&with(Err("offline".into())), url, &out);
        assert_eq!(
            r.unwrap_err(),
            AutoEqError::Fetch(FetchError::Network("offline".into()))
        );
        assert!(kept());
        let r = update_from_url(&with(answer(200, "<html>")), url, &out);
        assert!(matches!(r, Err(AutoEqError::Unreadable(_))), "{r:?}");
        assert!(kept());

        let db = update_from_url(&with(answer(200, SMALL_DB)), url, &out).unwrap();
        assert_eq!(db.entries[0].id, "o/Sony A");
        assert_eq!(std::fs::read_to_string(&out).unwrap(), SMALL_DB);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wp_cg6_removing_the_database_is_not_an_error_when_it_is_absent() {
        let dir = test_dir("cg6-reset");
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join(USER_DB_NAME);
        std::fs::write(&out, SMALL_DB).unwrap();
        assert_eq!(remove_user_database(&out), Ok(true));
        assert!(!out.exists());
        assert_eq!(remove_user_database(&out), Ok(false));
        // A directory in its place cannot be removed as a file.
        std::fs::create_dir_all(&out).unwrap();
        assert!(matches!(
            remove_user_database(&out),
            Err(AutoEqError::Unwritable { .. })
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wp_cg6_a_failed_write_leaves_nothing_behind() {
        let dir = test_dir("cg6-write");
        // The rename fails: the target is a directory.
        let out = dir.join(USER_DB_NAME);
        std::fs::create_dir_all(&out).unwrap();
        assert!(matches!(
            write_atomic(&out, b"x"),
            Err(AutoEqError::Unwritable { .. })
        ));
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(names.len(), 1, "the temporary file was removed");
    }

    #[test]
    fn wp_cg6_generated_at_is_the_consoles_format() {
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601_utc(1_709_164_800), "2024-02-29T00:00:00.000Z");
        assert_eq!(iso8601_utc(1_735_689_599), "2024-12-31T23:59:59.000Z");
        assert_eq!(iso8601_utc(1_793_837_045), "2026-11-05T00:04:05.000Z");
    }

    #[test]
    fn wp_cg6_the_consoles_whole_manufacturer_list_is_known() {
        for (folder, mfr) in [
            ("Westone W40", "Westone"),
            ("Yanyin Canon", "Yanyin"),
            ("BGVP DM6", "BGVP"),
            ("CCZ Melody", "CCZ"),
        ] {
            assert_eq!(split_name(folder).0, mfr);
        }
        assert_eq!(split_name("Dan Clark Audio").0, "Dan Clark Audio");
    }

    #[test]
    fn wp_cg6_the_warning_names_the_file_it_writes() {
        let w = rebuild_warning_for(std::path::Path::new("/somewhere/db.json"));
        assert!(w.contains("/somewhere/db.json"), "{w}");
        assert!(w.contains("api.github.com"), "{w}");
    }
}
