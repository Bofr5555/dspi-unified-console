//! C functions: AutoEQ database upkeep (WP CG6), over `dspi_core::autoeq`.
//!
//! None of these takes a core handle: no device is involved. `db_path` is the
//! host's file for the user's database. The JSON replies are as in the parent
//! module's docs, plus on failure `{"code":<as below>, "retryAfterS":<seconds or
//! null>}`: `code` is the [`DspiError::AutoEq`] code (`rate_limited`,
//! `http_status`, `network`, `bad_content`, ...), or the parent's `io` or
//! `invalid`.
//!
//! # Short buffers
//!
//! [`dspi_autoeq_update`], [`dspi_autoeq_import`] and
//! [`dspi_autoeq_reset_to_builtin`] change a file, so they do not take the usual
//! ask-for-the-length-then-call-again route. With a null `buf`, or `len` under
//! [`AUTOEQ_REPLY_MIN`], they return that minimum and do nothing: no fetch, no file
//! change. With at least that much room they act, and the reply always fits: a
//! `generatedAt` or `error` text too long for `buf` comes back empty, the rest of the
//! reply intact. They never need calling twice.
//!
//! # The fetch callback
//!
//! The host's HTTP GET: [`AutoEqFetchCallback`]. It blocks until the server has
//! answered, following redirects, and hands over the answer with
//! [`dspi_autoeq_fetch_respond`], or says no answer came with
//! [`dspi_autoeq_fetch_fail`]. Returning without calling either counts as no
//! answer. A 403 or a 500 is an answer.
//!
//! It runs on the rebuild's threads, up to 20 at once, so it and `user_data` must
//! be safe to use concurrently. Give it a timeout. It must not depend on the thread
//! that calls [`dspi_autoeq_rebuild_free`]: a Qt host makes its own network manager
//! and event loop inside the callback rather than waiting on the GUI thread.

use std::ffi::{c_char, c_void};
use std::sync::Arc;

use dspi_core::{
    AutoEqFetch, AutoEqFetchRequest, AutoEqFetchResponse, AutoEqRebuild, DspiError, Result,
};

use super::{c_bytes, c_str, copy_text, json_reply};
use crate::guard;

/// The smallest reply buffer the functions that change a file act on (module docs).
pub const AUTOEQ_REPLY_MIN: u32 = 4096;

/// Where a fetch callback puts its answer. Valid only during the callback.
pub struct AutoEqFetchSink {
    answer: Option<Result<AutoEqFetchResponse>>,
}

/// A running rebuild, from [`dspi_autoeq_rebuild_start`].
pub struct AutoEqRebuildHandle {
    rebuild: Arc<AutoEqRebuild>,
}

/// The host's HTTP GET (module docs). `url` and `user_agent` are NUL-terminated
/// UTF-8 and always set; `accept` is null when no `Accept` header is wanted. All
/// three are valid only during the call. Send `User-Agent` as given: GitHub refuses
/// a request without one, and QNetworkAccessManager adds none.
pub type AutoEqFetchCallback = Option<
    extern "C" fn(
        user_data: *mut c_void,
        url: *const c_char,
        accept: *const c_char,
        user_agent: *const c_char,
        sink: *mut AutoEqFetchSink,
    ),
>;

/// A C callback and its `user_data`, as the core's fetch.
struct CFetch {
    callback: extern "C" fn(
        *mut c_void,
        *const c_char,
        *const c_char,
        *const c_char,
        *mut AutoEqFetchSink,
    ),
    user_data: *mut c_void,
}

// SAFETY: the callback runs on core threads by this module's documented contract
// (the caller's `user_data` must allow that); nothing here touches the pointer.
unsafe impl Send for CFetch {}
// SAFETY: as for Send: the pointer is only passed back to the caller.
unsafe impl Sync for CFetch {}

impl AutoEqFetch for CFetch {
    fn get(&self, request: AutoEqFetchRequest) -> Result<AutoEqFetchResponse> {
        let refused = |what| DspiError::InvalidArgument {
            msg: format!("the {what} has a NUL in it"),
        };
        let url = std::ffi::CString::new(request.url).map_err(|_| refused("URL"))?;
        let agent = std::ffi::CString::new(request.user_agent).map_err(|_| refused("agent"))?;
        let accept = request
            .accept
            .map(std::ffi::CString::new)
            .transpose()
            .map_err(|_| refused("accept header"))?;
        let mut sink = AutoEqFetchSink { answer: None };
        (self.callback)(
            self.user_data,
            url.as_ptr(),
            accept.as_ref().map_or(std::ptr::null(), |a| a.as_ptr()),
            agent.as_ptr(),
            &raw mut sink,
        );
        sink.answer.unwrap_or_else(|| {
            Err(DspiError::Transfer {
                msg: "the host gave no answer".to_string(),
            })
        })
    }
}

/// A negative header value means the header was absent.
fn header<T: TryFrom<i64>>(value: i64) -> Option<T> {
    T::try_from(value).ok()
}

/// Answer a fetch: the HTTP status and body, as received, for any status.
/// `retry_after_s`, `rate_limit_remaining` and `rate_limit_reset` (Unix seconds) are
/// the `Retry-After`, `X-RateLimit-Remaining` and `X-RateLimit-Reset` headers, -1 if
/// absent. The body is copied. Does nothing for a null sink.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_autoeq_fetch_respond(
    sink: *mut AutoEqFetchSink,
    status: u16,
    body: *const u8,
    body_len: u32,
    retry_after_s: i64,
    rate_limit_remaining: i64,
    rate_limit_reset: i64,
) {
    if sink.is_null() {
        return;
    }
    guard((), || {
        let answer = AutoEqFetchResponse {
            status,
            body: c_bytes(body, body_len),
            retry_after_s: header(retry_after_s),
            rate_limit_remaining: header(rate_limit_remaining),
            rate_limit_reset: header(rate_limit_reset),
        };
        // SAFETY: a non-null sink is the one the callback was given, alive for the
        // call; that is the C caller's contract.
        unsafe { (*sink).answer = Some(Ok(answer)) };
    });
}

/// Say that no answer came (no network, a timeout), in words for the user.
/// Does nothing for a null sink.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_autoeq_fetch_fail(sink: *mut AutoEqFetchSink, message: *const c_char) {
    if sink.is_null() {
        return;
    }
    guard((), || {
        let msg = c_str(message).unwrap_or("no answer").to_owned();
        // SAFETY: as in dspi_autoeq_fetch_respond.
        unsafe { (*sink).answer = Some(Err(DspiError::Transfer { msg })) };
    });
}

/// A reply for the functions here: the parent's, plus the rate limit's wait.
fn reply<T: serde::Serialize>(r: Result<T>) -> String {
    let (code, msg, retry_after_s) = match r {
        Err(DspiError::AutoEq {
            code,
            msg,
            retry_after_s,
        }) => (code, msg, retry_after_s),
        other => return json_reply(other),
    };
    serde_json::json!({
        "ok": false,
        "code": code,
        "error": msg,
        "retryAfterS": retry_after_s,
    })
    .to_string()
}

/// Run a function that changes a file: nothing but the minimum for a null or short
/// buffer, and otherwise a reply that fits (module docs).
fn acting(buf: *mut c_char, len: u32, f: impl FnOnce() -> String) -> u32 {
    if buf.is_null() || len < AUTOEQ_REPLY_MIN {
        return AUTOEQ_REPLY_MIN;
    }
    copy_text(&fitted(f(), len), buf, len)
}

/// `text`, or if it does not fit `len` with its unbounded strings emptied. What is
/// left is numbers, flags and a short code, far under [`AUTOEQ_REPLY_MIN`].
fn fitted(text: String, len: u32) -> String {
    if text.len() < len as usize {
        return text;
    }
    let Ok(mut reply) = serde_json::from_str::<serde_json::Value>(&text) else {
        return text;
    };
    for field in ["generatedAt", "error", "problem"] {
        if let Some(value) = reply.get_mut(field).filter(|v| v.is_string()) {
            *value = serde_json::Value::String(String::new());
        }
    }
    reply.to_string()
}

/// A path argument, or the failure reply for one.
fn path_arg(db_path: *const c_char) -> std::result::Result<String, String> {
    c_str(db_path).map(str::to_owned).ok_or_else(|| {
        json_reply::<()>(Err(DspiError::InvalidArgument {
            msg: "db_path is null or not UTF-8".to_string(),
        }))
    })
}

/// The database at `db_path`, for the "Update Database" dialog: JSON {ok,
/// hasUserDatabase, problem, version, generatedAt, entryCount}. Reads the file; off
/// the UI thread. Returns the length needed.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_autoeq_database_info(
    db_path: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    guard(0, || {
        let text = path_arg(db_path).map_or_else(
            |refused| refused,
            |path| json_reply(Ok(dspi_core::autoeq_database_info(path))),
        );
        copy_text(&text, buf, len)
    })
}

/// Replace the database at `db_path` with a file's bytes, once they are known to be
/// a database with at least one profile; otherwise the file is left alone. JSON as
/// `dspi_autoeq_database_info`, or a failure. `buf` needs [`AUTOEQ_REPLY_MIN`] bytes;
/// with less, this returns that minimum and does nothing (module docs). Otherwise
/// returns the reply's length.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_autoeq_import(
    db_path: *const c_char,
    data: *const u8,
    data_len: u32,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    guard(0, || {
        acting(buf, len, || {
            path_arg(db_path).map_or_else(
                |refused| refused,
                |path| reply(dspi_core::autoeq_import(path, c_bytes(data, data_len))),
            )
        })
    })
}

/// Download a ready-made database from `url` and replace the one at `db_path`, once
/// it is known to be one. Blocks for the request, calling `callback` on this thread;
/// off the UI thread. JSON as `dspi_autoeq_import`. `buf` needs [`AUTOEQ_REPLY_MIN`]
/// bytes; with less, this returns that minimum and does nothing: no fetch, no file
/// change (module docs). Otherwise returns the reply's length.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_autoeq_update(
    db_path: *const c_char,
    url: *const c_char,
    callback: AutoEqFetchCallback,
    user_data: *mut c_void,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    guard(0, || {
        let invalid = |msg: &str| {
            json_reply::<()>(Err(DspiError::InvalidArgument {
                msg: msg.to_string(),
            }))
        };
        acting(buf, len, || {
            match (path_arg(db_path), c_str(url), callback) {
                (Err(refused), _, _) => refused,
                (_, None, _) => invalid("url is null or not UTF-8"),
                (_, _, None) => invalid("callback is null"),
                (Ok(path), Some(url), Some(callback)) => reply(dspi_core::autoeq_update(
                    path,
                    url.to_owned(),
                    Arc::new(CFetch {
                        callback,
                        user_data,
                    }),
                )),
            }
        })
    })
}

/// Reset to Built-in: remove the database at `db_path`. JSON {ok, value}: `value`
/// true if there was one. The host then goes back to its own bundled copy. `buf` needs
/// [`AUTOEQ_REPLY_MIN`] bytes; with less, this returns that minimum and does nothing
/// (module docs). Otherwise returns the reply's length.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_autoeq_reset_to_builtin(
    db_path: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    guard(0, || {
        acting(buf, len, || {
            path_arg(db_path).map_or_else(
                |refused| refused,
                |path| reply(dspi_core::autoeq_reset_to_builtin(path)),
            )
        })
    })
}

/// What a rebuild will reach for and write, as text to show before starting one.
/// Returns the length needed; 0 for a null or invalid `db_path`.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_autoeq_rebuild_warning(
    db_path: *const c_char,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    guard(0, || match c_str(db_path) {
        Some(path) => copy_text(
            &dspi_core::autoeq_rebuild_warning(path.to_owned()),
            buf,
            len,
        ),
        None => 0,
    })
}

/// Start rebuilding the database at `db_path` from GitHub, and return at once.
/// `callback` runs on the rebuild's threads (module docs), and it and `user_data` must
/// stay valid until `dspi_autoeq_rebuild_free` returns. Free the handle with that.
/// Null for a null or invalid `db_path` or a null callback.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_autoeq_rebuild_start(
    db_path: *const c_char,
    callback: AutoEqFetchCallback,
    user_data: *mut c_void,
) -> *mut AutoEqRebuildHandle {
    guard(std::ptr::null_mut(), || {
        let (Some(path), Some(callback)) = (c_str(db_path), callback) else {
            return std::ptr::null_mut();
        };
        let fetch = Arc::new(CFetch {
            callback,
            user_data,
        });
        let rebuild = dspi_core::autoeq_rebuild_start(path.to_owned(), fetch);
        Box::into_raw(Box::new(AutoEqRebuildHandle { rebuild }))
    })
}

/// The newest progress: JSON {ok, fraction, status, finished, succeeded, code,
/// retryAfterS, entries, failedProfiles, failedListings, skippedProfiles}. `succeeded`
/// means the database was written; on a failure the file was left as it was, and `code`
/// says why (`rate_limited`, `cancelled`, `no_profiles`, `too_many_failures`, ...).
/// Does not block. Returns the length needed; 0 for a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_autoeq_rebuild_poll(
    handle: *mut AutoEqRebuildHandle,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    if handle.is_null() {
        return 0;
    }
    guard(0, || {
        // SAFETY: a non-null handle comes from dspi_autoeq_rebuild_start and stays
        // valid until dspi_autoeq_rebuild_free; the C caller's contract.
        let rebuild = unsafe { &(*handle).rebuild };
        copy_text(&json_reply(Ok(rebuild.poll())), buf, len)
    })
}

/// Stop at the next batch of profiles; nothing is written. Does not block: keep
/// polling until `finished`. Does nothing for a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_autoeq_rebuild_cancel(handle: *mut AutoEqRebuildHandle) {
    if handle.is_null() {
        return;
    }
    guard((), || {
        // SAFETY: as in dspi_autoeq_rebuild_poll.
        unsafe { &(*handle).rebuild }.cancel();
    });
}

/// Cancel if still running, wait until the callback can no longer be called, and free
/// the handle. Blocks for the requests in flight, so it deadlocks if the callback is
/// waiting on this thread; poll until `finished` first to avoid blocking at all. Do not
/// call from inside the callback. Null is ignored.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_autoeq_rebuild_free(handle: *mut AutoEqRebuildHandle) {
    if handle.is_null() {
        return;
    }
    guard((), || {
        // SAFETY: a non-null handle comes from dspi_autoeq_rebuild_start and is not
        // used again; the C caller's contract.
        let handle = unsafe { Box::from_raw(handle) };
        handle.rebuild.cancel_and_wait();
    });
}

// -- WP P1AQ -- AutoEQ preview and apply to any channel pair (PRO1 T6)

/// The profile and pair arguments, as the core takes them.
fn pair_args(profile: *const c_char) -> Result<dspi_core::AutoEqProfile> {
    super::arg(profile, "profile").and_then(dspi_core::AutoEqProfile::from_entry_json)
}

/// `apply_level` as the core takes it: negative the default (inputs yes,
/// outputs no), 0 no, positive yes.
fn level(apply_level: i8) -> Option<bool> {
    (apply_level >= 0).then_some(apply_level > 0)
}

/// What a profile would do to wire channels `left` and `right` (two inputs or
/// two outputs). `profile`: one entry of the AutoEQ database as JSON (`{id,
/// manufacturer, model, preamp, filters: [{type, freq, q, gain}]}`).
/// `apply_level`: whether the profile's preamp is written; negative the
/// default (yes on inputs, as their preamp; no on outputs, whose calibrated
/// gain is kept and the preamp only suggested), 0 no, 1 yes. JSON
/// `{ok, profileId, channels: [{channel, isOutput, levelDb, bands, crossover,
/// curve}], levelTarget, suggestedTrimDb, held, unsupported, dropped, notes}`
/// (`levelTarget` `input_preamp`, `output_gain` or `none`; `curve` at the
/// `dspi_curve_freq` points), or a failure. Sends nothing, so it may be called
/// again with a larger buffer.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_autoeq_preview(
    core: *mut crate::FfiCore,
    profile: *const c_char,
    left: u8,
    right: u8,
    apply_level: i8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    super::text(core, buf, len, |c| {
        json_reply(
            pair_args(profile).and_then(|p| c.autoeq_preview(p, left, right, level(apply_level))),
        )
    })
}

/// Apply a profile to wire channels `left` and `right` as `dspi_autoeq_preview`
/// shows it (`apply_level` as there): the state before is captured as a snapshot and filed in the store
/// (`dspi_snapshot_set_dir`; `dspi_snapshot_export` gives its document), then
/// the writes are one undo step, held to the limits profile. The device changes
/// on every call, whatever the buffer, so the reply is short: JSON `{ok,
/// snapshotId, profileId, levelTarget, suggestedTrimDb, held, unsupported,
/// dropped, notes}`
/// (`snapshotId` "" with no folder named), or a failure. `buf` needs
/// [`AUTOEQ_REPLY_MIN`] bytes.
#[unsafe(no_mangle)]
pub extern "C" fn dspi_autoeq_apply(
    core: *mut crate::FfiCore,
    profile: *const c_char,
    left: u8,
    right: u8,
    apply_level: i8,
    buf: *mut c_char,
    len: u32,
) -> u32 {
    super::text(core, buf, len, |c| {
        match pair_args(profile).and_then(|p| c.autoeq_apply(p, left, right, level(apply_level))) {
            Ok(done) => {
                let mut notes = done.preview.notes;
                notes.extend(done.notes);
                serde_json::json!({
                    "ok": true,
                    "snapshotId": done.snapshot_id,
                    "profileId": done.preview.profile_id,
                    "levelTarget": done.preview.level_target,
                    "suggestedTrimDb": done.preview.suggested_trim_db,
                    "held": done.preview.held,
                    "unsupported": done.preview.unsupported,
                    "dropped": done.preview.dropped,
                    "notes": notes,
                })
                .to_string()
            }
            Err(e) => json_reply::<()>(Err(e)),
        }
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg6 {
    use std::ffi::{CStr, CString};
    use std::ptr::{null, null_mut};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use serde_json::Value;

    use super::*;

    const API: &str = "https://api.github.com/repos/jaakkopasanen/AutoEq/contents/results/";
    const RAW: &str = "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/master/results/";
    const DB: &str = r#"{"version":1,"generatedAt":"2026-01-02T03:04:05.000Z","entryCount":1,
        "entries":[{"id":"o/Sony A","manufacturer":"Sony","model":"A","filters":[]}]}"#;
    const PROFILE: &str = "Preamp: -6 dB\nFilter 1: ON PK Fc 105 Hz Gain 3 dB Q 0.7\n";

    fn scratch(name: &str) -> CString {
        let dir = std::env::current_exe()
            .unwrap()
            .with_file_name("test-scratch")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        CString::new(dir.join("db.json").display().to_string()).unwrap()
    }

    /// Read a function's JSON reply, with the room the ones that act require.
    fn json(f: impl Fn(*mut c_char, u32) -> u32) -> Value {
        let mut buf = vec![0 as c_char; AUTOEQ_REPLY_MIN as usize];
        let needed = f(buf.as_mut_ptr(), AUTOEQ_REPLY_MIN);
        assert!((1..=AUTOEQ_REPLY_MIN).contains(&needed), "{needed}");
        let text = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_str().unwrap();
        serde_json::from_str(text).unwrap()
    }

    /// What a scripted C host answers, and what it was asked.
    struct Host {
        calls: AtomicUsize,
        accepts: Mutex<Vec<Option<String>>>,
        agents: Mutex<Vec<String>>,
    }

    fn host() -> Host {
        Host {
            calls: AtomicUsize::new(0),
            accepts: Mutex::default(),
            agents: Mutex::default(),
        }
    }

    fn respond(sink: *mut AutoEqFetchSink, status: u16, body: &str) {
        dspi_autoeq_fetch_respond(sink, status, body.as_ptr(), body.len() as u32, -1, -1, -1);
    }

    /// The scripted C callback: one source, one target, two headphones, of which
    /// one download fails.
    extern "C" fn github(
        user_data: *mut c_void,
        url: *const c_char,
        accept: *const c_char,
        agent: *const c_char,
        sink: *mut AutoEqFetchSink,
    ) {
        let host = unsafe { &*user_data.cast::<Host>() };
        host.calls.fetch_add(1, Ordering::SeqCst);
        let url = unsafe { CStr::from_ptr(url) }.to_str().unwrap();
        let text = |p: *const c_char| unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_string();
        host.accepts
            .lock()
            .unwrap()
            .push((!accept.is_null()).then(|| text(accept)));
        host.agents.lock().unwrap().push(text(agent));
        if url == format!("{API}oratory1990") {
            respond(sink, 200, r#"[{"name":"t","type":"dir"}]"#);
        } else if url == format!("{API}oratory1990/t") {
            let names =
                ["Sony A", "Sony B", "Sony C"].map(|n| format!(r#"{{"name":"{n}","type":"dir"}}"#));
            respond(sink, 200, &format!("[{}]", names.join(",")));
        } else if url.starts_with(&format!("{RAW}oratory1990/t/Sony%20B")) {
            let why = CString::new("timed out").unwrap();
            dspi_autoeq_fetch_fail(sink, why.as_ptr());
        } else if url.starts_with(RAW) {
            respond(sink, 200, PROFILE);
        } else {
            respond(sink, 404, "");
        }
    }

    /// A rate limit on the first listing.
    extern "C" fn limited(
        _: *mut c_void,
        _: *const c_char,
        _: *const c_char,
        _: *const c_char,
        sink: *mut AutoEqFetchSink,
    ) {
        dspi_autoeq_fetch_respond(sink, 403, null(), 0, 90, 0, -1);
    }

    /// A host that returns without answering.
    extern "C" fn silent(
        _: *mut c_void,
        _: *const c_char,
        _: *const c_char,
        _: *const c_char,
        _: *mut AutoEqFetchSink,
    ) {
    }

    extern "C" fn serves_db(
        _: *mut c_void,
        _: *const c_char,
        _: *const c_char,
        _: *const c_char,
        sink: *mut AutoEqFetchSink,
    ) {
        respond(sink, 200, DB);
    }

    fn rebuild_to_end(handle: *mut AutoEqRebuildHandle) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let p = json(|b, l| dspi_autoeq_rebuild_poll(handle, b, l));
            if p["finished"] == true {
                return p;
            }
            assert!(Instant::now() < deadline, "no result: {p}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_rebuild_through_a_c_callback_reports_partial_failure() {
        let db = scratch("capi-cg6-rebuild");
        let host = host();
        let user = (&raw const host).cast_mut().cast::<c_void>();
        let handle = dspi_autoeq_rebuild_start(db.as_ptr(), Some(github), user);
        assert!(!handle.is_null());
        let p = rebuild_to_end(handle);
        assert_eq!(p["ok"], true);
        assert_eq!(p["succeeded"], true, "{p}");
        assert_eq!(p["entries"], 2, "{p}");
        assert_eq!(p["failedProfiles"], 1, "{p}");
        dspi_autoeq_rebuild_free(handle);

        let info = json(|b, l| dspi_autoeq_database_info(db.as_ptr(), b, l));
        assert_eq!(info["ok"], true);
        assert_eq!(info["hasUserDatabase"], true);
        assert_eq!(info["entryCount"], 2);

        // Listings ask for GitHub's media type, downloads for nothing; all three
        // strings reach C as given.
        let accepts = host.accepts.lock().unwrap();
        assert_eq!(
            accepts.iter().filter(|a| a.is_some()).count(),
            6,
            "five sources and a target: {accepts:?}"
        );
        assert_eq!(accepts.iter().filter(|a| a.is_none()).count(), 3);
        assert!(host.agents.lock().unwrap().iter().all(|a| !a.is_empty()));
    }

    #[test]
    fn a_rate_limit_through_a_c_callback_stops_and_says_when() {
        let db = scratch("capi-cg6-limit");
        std::fs::write(db.to_str().unwrap(), DB).unwrap();
        let handle = dspi_autoeq_rebuild_start(db.as_ptr(), Some(limited), null_mut());
        let p = rebuild_to_end(handle);
        assert_eq!(p["succeeded"], false, "{p}");
        assert_eq!(p["code"], "rate_limited", "{p}");
        assert_eq!(p["retryAfterS"], 90, "{p}");
        dspi_autoeq_rebuild_free(handle);
        assert_eq!(std::fs::read_to_string(db.to_str().unwrap()).unwrap(), DB);
    }

    #[test]
    fn a_host_that_does_not_answer_is_a_failure_not_a_hang() {
        let db = scratch("capi-cg6-silent");
        let url = CString::new("https://example.test/db.json").unwrap();
        let r = json(|b, l| {
            dspi_autoeq_update(db.as_ptr(), url.as_ptr(), Some(silent), null_mut(), b, l)
        });
        assert_eq!(r["ok"], false);
        assert_eq!(r["code"], "network");
    }

    #[test]
    fn free_waits_until_the_callback_can_no_longer_run() {
        let db = scratch("capi-cg6-free");
        let host = host();
        let user = (&raw const host).cast_mut().cast::<c_void>();
        let handle = dspi_autoeq_rebuild_start(db.as_ptr(), Some(github), user);
        dspi_autoeq_rebuild_cancel(handle);
        dspi_autoeq_rebuild_free(handle);
        let after = host.calls.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(host.calls.load(Ordering::SeqCst), after);
    }

    #[test]
    fn update_import_and_reset_reply_in_json() {
        let db = scratch("capi-cg6-files");
        let url = CString::new("https://example.test/db.json").unwrap();

        let r = json(|b, l| {
            dspi_autoeq_update(db.as_ptr(), url.as_ptr(), Some(limited), null_mut(), b, l)
        });
        assert_eq!(r["ok"], false);
        assert_eq!(r["code"], "rate_limited");
        assert_eq!(r["retryAfterS"], 90);

        let r = json(|b, l| {
            dspi_autoeq_update(db.as_ptr(), url.as_ptr(), Some(serves_db), null_mut(), b, l)
        });
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["entryCount"], 1);

        let r = json(|b, l| dspi_autoeq_import(db.as_ptr(), DB.as_ptr(), DB.len() as u32, b, l));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["generatedAt"], "2026-01-02T03:04:05.000Z");

        let r = json(|b, l| dspi_autoeq_import(db.as_ptr(), c"{}".as_ptr().cast(), 2, b, l));
        assert_eq!(r["ok"], false);
        assert_eq!(r["code"], "bad_content");
        assert_eq!(r["retryAfterS"], Value::Null);

        let r = json(|b, l| dspi_autoeq_reset_to_builtin(db.as_ptr(), b, l));
        assert_eq!(r["ok"], true);
        assert_eq!(r["value"], true);
        let r = json(|b, l| dspi_autoeq_reset_to_builtin(db.as_ptr(), b, l));
        assert_eq!(r["value"], false);
        let r = json(|b, l| dspi_autoeq_database_info(db.as_ptr(), b, l));
        assert_eq!(r["hasUserDatabase"], false);

        // The usual way of a text function: ask how long, then read.
        let needed = dspi_autoeq_rebuild_warning(db.as_ptr(), null_mut(), 0);
        let mut buf = vec![0 as c_char; needed as usize];
        assert_eq!(
            dspi_autoeq_rebuild_warning(db.as_ptr(), buf.as_mut_ptr(), needed),
            needed
        );
        let text = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_str().unwrap();
        assert!(text.contains("api.github.com"), "{text}");
    }

    /// The C ABI's rules: every pointer may be null, and nothing crashes for it.
    #[test]
    fn null_arguments_are_failures_not_crashes() {
        let p = c"some-path".as_ptr();
        let u = null_mut();
        let t = null_mut();
        assert!(dspi_autoeq_rebuild_start(null(), Some(limited), u).is_null());
        assert!(dspi_autoeq_rebuild_start(p, None, u).is_null());
        assert_eq!(dspi_autoeq_rebuild_poll(null_mut(), t, 0), 0);
        dspi_autoeq_rebuild_cancel(null_mut());
        dspi_autoeq_rebuild_free(null_mut());
        dspi_autoeq_fetch_respond(null_mut(), 200, null(), 0, -1, -1, -1);
        dspi_autoeq_fetch_fail(null_mut(), null());
        assert_eq!(dspi_autoeq_rebuild_warning(null(), t, 0), 0);
        for r in [
            json(|b, l| dspi_autoeq_database_info(null(), b, l)),
            json(|b, l| dspi_autoeq_import(null(), null(), 0, b, l)),
            json(|b, l| dspi_autoeq_reset_to_builtin(null(), b, l)),
            json(|b, l| dspi_autoeq_update(null(), p, Some(limited), u, b, l)),
            json(|b, l| dspi_autoeq_update(p, null(), Some(limited), u, b, l)),
            json(|b, l| dspi_autoeq_update(p, p, None, u, b, l)),
        ] {
            assert_eq!(r["ok"], false, "{r}");
            assert_eq!(r["code"], "invalid", "{r}");
        }
        // Null data is an empty file, which is refused like any other.
        let db = scratch("capi-cg6-null");
        let r = json(|b, l| dspi_autoeq_import(db.as_ptr(), null(), 0, b, l));
        assert_eq!(r["code"], "bad_content", "{r}");
    }

    /// Counts its calls; would replace the file if it ever got that far.
    extern "C" fn counting(
        user_data: *mut c_void,
        _: *const c_char,
        _: *const c_char,
        _: *const c_char,
        sink: *mut AutoEqFetchSink,
    ) {
        let calls = unsafe { &*user_data.cast::<AtomicUsize>() };
        calls.fetch_add(1, Ordering::SeqCst);
        respond(sink, 200, DB);
    }

    #[test]
    fn wp_cg6_a_short_buffer_fetches_nothing_and_changes_nothing() {
        const OLD: &str = "the old database";
        let db = scratch("capi-cg6-short");
        let path = db.to_str().unwrap();
        std::fs::write(path, OLD).unwrap();
        let url = c"https://example.test/db.json".as_ptr();
        let calls = AtomicUsize::new(0);
        let user = (&raw const calls).cast_mut().cast::<c_void>();
        let min = AUTOEQ_REPLY_MIN;

        // Every null or short shape, with a buffer that must stay untouched.
        let mut bytes = vec![0x55 as c_char; min as usize];
        let shapes = [
            (null_mut(), 0),
            (null_mut(), min),
            (null_mut(), 100_000),
            (bytes.as_mut_ptr(), 0),
            (bytes.as_mut_ptr(), 1),
            (bytes.as_mut_ptr(), min - 1),
        ];
        for (b, l) in shapes {
            let update = dspi_autoeq_update(db.as_ptr(), url, Some(counting), user, b, l);
            assert_eq!(update, min, "update, {l}");
            let import = dspi_autoeq_import(db.as_ptr(), DB.as_ptr(), DB.len() as u32, b, l);
            assert_eq!(import, min, "import, {l}");
            assert_eq!(
                dspi_autoeq_reset_to_builtin(db.as_ptr(), b, l),
                min,
                "reset, {l}"
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0, "a short buffer fetched");
        assert_eq!(std::fs::read_to_string(path).unwrap(), OLD);
        assert!(
            bytes.iter().all(|&b| b == 0x55),
            "a short buffer was written"
        );

        // With the minimum: exactly one fetch, one change, and no second call needed.
        let n = dspi_autoeq_update(
            db.as_ptr(),
            url,
            Some(counting),
            user,
            bytes.as_mut_ptr(),
            min,
        );
        assert!((1..=min).contains(&n), "{n}");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read_to_string(path).unwrap(), DB);
    }

    /// An imported file's `generatedAt` can be any length. The file has changed by
    /// then, so the reply still has to arrive.
    #[test]
    fn wp_cg6_a_reply_always_fits_the_minimum() {
        let db = scratch("capi-cg6-long");
        let long = "9".repeat(3 * AUTOEQ_REPLY_MIN as usize);
        let doc = DB.replace("2026-01-02T03:04:05.000Z", &long);
        let r = json(|b, l| dspi_autoeq_import(db.as_ptr(), doc.as_ptr(), doc.len() as u32, b, l));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["entryCount"], 1, "{r}");
        assert_eq!(r["generatedAt"], "", "{r}");
        assert_eq!(std::fs::read_to_string(db.to_str().unwrap()).unwrap(), doc);

        // A failure keeps its code when its words are dropped.
        let cut = fitted(
            json_reply::<()>(Err(DspiError::Io { msg: long })),
            AUTOEQ_REPLY_MIN,
        );
        assert!(cut.len() < AUTOEQ_REPLY_MIN as usize, "{cut}");
        let cut: Value = serde_json::from_str(&cut).unwrap();
        assert_eq!(
            (&cut["ok"], &cut["code"]),
            (&Value::Bool(false), &Value::from("io"))
        );

        // Room for the whole reply keeps the whole reply.
        let whole = r#"{"generatedAt":"2026","ok":true}"#.to_string();
        assert_eq!(fitted(whole.clone(), AUTOEQ_REPLY_MIN), whole);
    }

    #[test]
    fn the_header_declares_the_autoeq_functions() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let header = std::fs::read_to_string(dir.join("include/dspi_core.h")).unwrap();
        // cbindgen writes what it cannot resolve as an undefined struct.
        assert!(
            !header.contains("Option_"),
            "an unresolved type in the header"
        );
        for name in [
            "AUTOEQ_REPLY_MIN",
            "AutoEqFetchCallback",
            "AutoEqFetchSink",
            "AutoEqRebuildHandle",
            "dspi_autoeq_fetch_respond(",
            "dspi_autoeq_fetch_fail(",
            "dspi_autoeq_database_info(",
            "dspi_autoeq_import(",
            "dspi_autoeq_update(",
            "dspi_autoeq_reset_to_builtin(",
            "dspi_autoeq_rebuild_warning(",
            "dspi_autoeq_rebuild_start(",
            "dspi_autoeq_rebuild_poll(",
            "dspi_autoeq_rebuild_cancel(",
            "dspi_autoeq_rebuild_free(",
        ] {
            assert!(header.contains(name), "{name} is not in the header");
        }
    }
}

// -- WP P1AQ --
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests_wp_p1aq {
    use std::ffi::CString;
    use std::ptr::null;

    use serde_json::Value;

    use super::*;
    use crate::fns_device::{dspi_core_free, dspi_core_new};

    fn reply(f: impl Fn(*mut c_char, u32) -> u32) -> Value {
        let mut buf = vec![0u8; 1 << 20];
        let n = f(buf.as_mut_ptr().cast(), buf.len() as u32);
        assert!(n > 0 && (n as usize) <= buf.len(), "{n}");
        let text = std::ffi::CStr::from_bytes_until_nul(&buf).unwrap();
        serde_json::from_str(text.to_str().unwrap()).unwrap()
    }

    #[test]
    fn wp_p1aq_preview_and_apply_through_c() {
        let core = dspi_core_new();
        assert!(super::super::dspi_open_demo(core));
        let profile = CString::new(
            r#"{"id":"o/Sony A","manufacturer":"Sony","model":"A","preamp":-4,
                "filters":[{"type":"PK","freq":105,"q":0.7,"gain":3}]}"#,
        )
        .unwrap();
        let pv = reply(|b, l| dspi_autoeq_preview(core, profile.as_ptr(), 0, 1, -1, b, l));
        assert_eq!(pv["ok"], true, "{pv}");
        assert_eq!(pv["levelTarget"], "input_preamp");
        assert_eq!(pv["channels"][0]["bands"][0]["gain"], 3.0, "{pv}");

        let mut small = vec![0u8; AUTOEQ_REPLY_MIN as usize];
        let n = dspi_autoeq_apply(
            core,
            profile.as_ptr(),
            0,
            1,
            -1,
            small.as_mut_ptr().cast(),
            AUTOEQ_REPLY_MIN,
        );
        assert!(n > 0 && n <= AUTOEQ_REPLY_MIN, "a short reply: {n}");
        let text = std::ffi::CStr::from_bytes_until_nul(&small).unwrap();
        let done: Value = serde_json::from_str(text.to_str().unwrap()).unwrap();
        assert_eq!(done["ok"], true, "{done}");
        assert_eq!(done["snapshotId"], "");
        // SAFETY: a live handle from dspi_core_new, freed at the end.
        let preamp = unsafe { (*core).core.get_state().input_preamp_db[1] };
        assert_eq!(preamp, -4.0);

        // An output pair keeps its gain unless asked (1), then writes it.
        // SAFETY: as above.
        let out = unsafe { (*core).core.get_state().num_input_channels } + 2;
        let kept = reply(|b, l| dspi_autoeq_apply(core, profile.as_ptr(), out, out + 1, -1, b, l));
        assert_eq!(kept["levelTarget"], "none", "{kept}");
        assert_eq!(kept["suggestedTrimDb"], -4.0, "{kept}");
        let set = reply(|b, l| dspi_autoeq_apply(core, profile.as_ptr(), out, out + 1, 1, b, l));
        assert_eq!(set["levelTarget"], "output_gain", "{set}");
        // SAFETY: as above.
        let gain = unsafe { (*core).core.get_state().output_gain_db[2] };
        assert_eq!(gain, -4.0);

        let bad = reply(|b, l| dspi_autoeq_apply(core, profile.as_ptr(), 0, 0, -1, b, l));
        assert_eq!(bad["ok"], false);
        assert_eq!(bad["code"], "invalid", "{bad}");
        let none = reply(|b, l| dspi_autoeq_preview(core, null(), 0, 1, -1, b, l));
        assert_eq!(none["code"], "invalid", "{none}");
        dspi_core_free(core);
    }
}
